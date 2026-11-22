//! NVIDIA DLSS through Vulkan NGX (dlss_wgpu): Super Resolution / DLAA as an upscaler, Ray Reconstruction as a joint
//! denoiser (live/reconstruction.rs has the two contracts).

use super::reconstruction::{JointGuides, JointProvider, Prepared, ReconstructionFrame, ReconstructionSize, UpscaleProvider};
use crate::gpu;
use anyhow::{Context, Result};
use dlss_wgpu::ray_reconstruction::{
    DlssRayReconstruction, DlssRayReconstructionDepthMode, DlssRayReconstructionRenderParameters,
    DlssRayReconstructionRoughnessMode, DlssRayReconstructionSpecularGuide, DlssRayReconstructionTransparencyOverlay,
};
use dlss_wgpu::super_resolution::{DlssSuperResolution, DlssSuperResolutionExposure, DlssSuperResolutionRenderParameters};
use dlss_wgpu::{DlssError, DlssFeatureFlags, DlssPerfQualityMode as Mode};
use std::ops::RangeInclusive;
use wgpu::{CommandBuffer, CommandEncoder, TextureFormat, TextureUsages, TextureView};

/// What both DLSS features share: a context made for one output size and quality mode, which accepts a range of
/// input sizes.
trait NgxFeature: Sized {
    fn create(gpu: &gpu::Gpu, sdk: &gpu::DlssSdk, output: [u32; 2], mode: Mode) -> Result<Self, DlssError>;
    fn output(&self) -> [u32; 2];
    fn range(&self) -> RangeInclusive<[u32; 2]>;
}

impl NgxFeature for DlssSuperResolution {
    fn create(gpu: &gpu::Gpu, sdk: &gpu::DlssSdk, output: [u32; 2], mode: Mode) -> Result<Self, DlssError> {
        let flags = DlssFeatureFlags::HighDynamicRange | DlssFeatureFlags::LowResolutionMotionVectors
            | DlssFeatureFlags::InvertedDepth | DlssFeatureFlags::AutoExposure;
        DlssSuperResolution::new(output, mode, flags, sdk.clone(), &gpu.device, &gpu.queue)
    }
    fn output(&self) -> [u32; 2] { self.upscaled_resolution() }
    fn range(&self) -> RangeInclusive<[u32; 2]> { self.render_resolution_range() }
}

impl NgxFeature for DlssRayReconstruction {
    fn create(gpu: &gpu::Gpu, sdk: &gpu::DlssSdk, output: [u32; 2], mode: Mode) -> Result<Self, DlssError> {
        // (Ray Reconstruction has no auto exposure: its input is in units of display white, `radiance_unit`.)
        let flags = DlssFeatureFlags::HighDynamicRange | DlssFeatureFlags::LowResolutionMotionVectors | DlssFeatureFlags::InvertedDepth;
        DlssRayReconstruction::new(output, mode, flags, DlssRayReconstructionRoughnessMode::Unpacked,
            DlssRayReconstructionDepthMode::Hardware, sdk.clone(), &gpu.device, &gpu.queue)
    }
    fn output(&self) -> [u32; 2] { self.upscaled_resolution() }
    fn range(&self) -> RangeInclusive<[u32; 2]> {
        // NGX reports one render resolution for Ray Reconstruction; dynamic resolution recreates it.
        let r = self.render_resolution();
        r..=r
    }
}

fn contains(r: &RangeInclusive<[u32; 2]>, input: [u32; 2]) -> bool {
    // RangeInclusive<[u32; 2]>::contains would compare the dimensions lexicographically.
    (0..2).all(|i| r.start()[i] <= input[i] && input[i] <= r.end()[i])
}

struct State<C> {
    // The wrapper keeps NGX and the device alive and waits for GPU use before releasing this context.
    context: C,
    mode: Mode,
    input: [u32; 2],
    reset: bool,
}

/// The context for a size: the current one if it accepts it, else one per quality mode, nearest the input's
/// ratio first (the modes' nominal ratios are not bounds: NGX reports each one's range).
struct Feature<C> {
    name: &'static str,
    state: Option<State<C>>,
}

impl<C: NgxFeature> Feature<C> {
    fn prepare(&mut self, gpu: &gpu::Gpu, available: bool, size: ReconstructionSize) -> Result<Prepared> {
        let Some(sdk) = gpu.dlss_sdk.as_ref().filter(|_| available) else {
            return Ok(Prepared::Unsupported(if gpu.dlss_sdk.is_none() {
                "NGX is not running (an NVIDIA RTX GPU on Vulkan, THREERT_DLSS=1, nvngx_dlss.dll beside the program)".into()
            } else {
                format!("NGX reports no {} on this device or driver", self.name)
            }));
        };
        if (0..2).any(|i| size.output[i] < 32 || size.input[i] == 0 || size.input[i] > size.output[i]) {
            return Ok(Prepared::Unsupported(format!("{} needs an input of 1 x 1 up to the output, and an output of 32 x 32 or more", self.name)));
        }
        let native = size.input == size.output;
        let fits = |s: &State<C>| s.context.output() == size.output && (s.mode == Mode::Dlaa) == native && contains(&s.context.range(), size.input);
        if let Some(s) = &mut self.state {
            if fits(s) && (size.dynamic || s.input == size.input) {
                s.input = size.input;
                return Ok(Prepared::ready(size));
            }
        }
        let mut scaled = [(Mode::Quality, 667), (Mode::Balanced, 588), (Mode::Performance, 500), (Mode::UltraPerformance, 333)];
        let modes = if native {
            &[(Mode::Dlaa, 1000)][..]
        } else {
            scaled.sort_unstable_by_key(|&(_, k)| (size.input[0] as u64 * 1000).abs_diff(size.output[0] as u64 * k));
            &scaled[..]
        };
        let mut ranges = vec![];
        for &(mode, _) in modes {
            let context = match C::create(gpu, sdk, size.output, mode) {
                Ok(c) => c,
                Err(DlssError::FeatureNotSupported | DlssError::InvalidParameters | DlssError::UnsupportedParameter) => continue,
                Err(e) => return Err(e).with_context(|| format!("create {} {mode:?} for {:?}", self.name, size.output)),
            };
            let r = context.range();
            if contains(&r, size.input) {
                self.state = Some(State { context, mode, input: size.input, reset: true });
                return Ok(Prepared::ready(size));
            }
            ranges.push(format!("{mode:?} {}x{}..{}x{}", r.start()[0], r.start()[1], r.end()[0], r.end()[1]));
        }
        Ok(Prepared::Unsupported(format!("an input of {}x{} is outside {}'s inputs for {}x{} ({})",
            size.input[0], size.input[1], self.name, size.output[0], size.output[1], ranges.join(", "))))
    }

    fn state(&mut self, frame: &ReconstructionFrame<'_>) -> Result<&mut State<C>> {
        let s = self.state.as_mut().with_context(|| format!("{} was not prepared", self.name))?;
        anyhow::ensure!(s.input == frame.size.input && s.context.output() == frame.size.output, "{} was prepared for another size", self.name);
        Ok(s)
    }
}

fn check(view: &TextureView, name: &str, format: TextureFormat, [w, h]: [u32; 2], usage: TextureUsages) -> Result<()> {
    let t = view.texture();
    anyhow::ensure!(t.format() == format && t.width() >= w && t.height() >= h && t.usage().contains(usage),
        "invalid DLSS {name} texture: expected {w}x{h} or larger, {format:?} with {usage:?}");
    Ok(())
}

fn check_frame(f: &ReconstructionFrame<'_>) -> Result<()> {
    let sampled = TextureUsages::TEXTURE_BINDING;
    check(f.color, "colour", TextureFormat::Rgba16Float, f.size.input, sampled)?;
    check(f.depth, "depth", TextureFormat::R32Float, f.size.input, sampled)?;
    check(f.motion, "motion", TextureFormat::Rg16Float, f.size.input, sampled)?;
    check(f.reactive, "reactive mask", TextureFormat::R8Unorm, f.size.input, sampled)?;
    // Some Vulkan NGX paths also clear the output using vkCmdClearColorImage.
    check(f.output, "output", TextureFormat::Rgba16Float, f.size.output, TextureUsages::STORAGE_BINDING | TextureUsages::COPY_DST)
}

/// DLSS Super Resolution, DLAA at scale 1.
pub(super) struct Dlss(Feature<DlssSuperResolution>);

impl Default for Dlss {
    fn default() -> Self { Self(Feature { name: "DLSS Super Resolution", state: None }) }
}

impl UpscaleProvider for Dlss {
    fn prepare(&mut self, gpu: &gpu::Gpu, size: ReconstructionSize) -> Result<Prepared> { self.0.prepare(gpu, true, size) }

    fn encode(&mut self, frame: ReconstructionFrame<'_>, encoder: &mut CommandEncoder) -> Result<Option<CommandBuffer>> {
        check_frame(&frame)?;
        let state = self.0.state(&frame)?;
        // NGX wants the shift of the projected image, opposite to the displacement of the sample point.
        let [x, y] = frame.jitter;
        let params = DlssSuperResolutionRenderParameters {
            color: frame.color, depth: frame.depth, motion_vectors: frame.motion,
            exposure: DlssSuperResolutionExposure::Automatic, bias: Some(frame.reactive), dlss_output: frame.output,
            reset: state.reset || frame.reset, jitter_offset: [-x, -y],
            partial_texture_size: Some(frame.size.input), motion_vector_scale: None,
        };
        // The wrapper records barriers on `encoder`. Submit the returned buffer immediately after it, in the same submit.
        let native = state.context.render(params, encoder, &frame.gpu.adapter).context("encode DLSS Super Resolution")?;
        state.reset = false;
        Ok(Some(native))
    }
}

/// DLSS Ray Reconstruction.
pub(super) struct DlssRr(Feature<DlssRayReconstruction>);

impl Default for DlssRr {
    fn default() -> Self { Self(Feature { name: "DLSS Ray Reconstruction", state: None }) }
}

impl JointProvider for DlssRr {
    fn prepare(&mut self, gpu: &gpu::Gpu, size: ReconstructionSize) -> Result<Prepared> { self.0.prepare(gpu, gpu.dlss_rr, size) }

    fn encode(&mut self, frame: ReconstructionFrame<'_>, g: JointGuides<'_>, encoder: &mut CommandEncoder) -> Result<Option<CommandBuffer>> {
        check_frame(&frame)?;
        let sampled = TextureUsages::TEXTURE_BINDING;
        check(g.overlay, "overlay", TextureFormat::Rgba16Float, frame.size.input, sampled)?;
        check(g.diffuse_albedo, "diffuse albedo", TextureFormat::Rgba8Unorm, frame.size.input, sampled)?;
        check(g.specular_albedo, "specular albedo", TextureFormat::Rgba8Unorm, frame.size.input, sampled)?;
        check(g.normal, "normal", TextureFormat::Rgba16Float, frame.size.input, sampled)?;
        check(g.roughness, "roughness", TextureFormat::R16Float, frame.size.input, sampled)?;
        check(g.specular_hit_distance, "specular hit distance", TextureFormat::R16Float, frame.size.input, sampled)?;
        let state = self.0.state(&frame)?;
        let [x, y] = frame.jitter;
        let params = DlssRayReconstructionRenderParameters {
            diffuse_albedo: g.diffuse_albedo, specular_albedo: g.specular_albedo, normals: g.normal, roughness: Some(g.roughness),
            color: frame.color, depth: frame.depth, motion_vectors: frame.motion,
            // NGX's matrices are row-vector (v * M) and row-major: the same 16 floats as a column-vector matrix
            // stored column-major (glam's `to_cols_array`).
            specular_guide: DlssRayReconstructionSpecularGuide::SpecularHitDistance {
                texture_view: g.specular_hit_distance, world_to_view_rows_array: frame.world_to_view, view_to_clip_rows_array: frame.view_to_clip,
            },
            // The overlay is premultiplied: its alpha is 1 where it is the whole pixel, and 0 (with no colour) elsewhere.
            transparency_overlay: Some(DlssRayReconstructionTransparencyOverlay::Premultiplied(g.overlay)),
            color_before_transparency: None, screen_space_subsurface_scattering_guide: None, depth_of_field_guide: None,
            responsivity_mask: Some(frame.reactive), alpha: None, dlss_output_alpha: None, dlss_output: frame.output,
            reset: state.reset || frame.reset, jitter_offset: [-x, -y],
            partial_texture_size: Some(frame.size.input), motion_vector_scale: None,
        };
        let native = state.context.render(params, encoder, &frame.gpu.adapter).context("encode DLSS Ray Reconstruction")?;
        state.reset = false;
        Ok(Some(native))
    }
}
