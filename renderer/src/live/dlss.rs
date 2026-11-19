//! Vulkan NGX Super Resolution / DLAA at the live HDR reconstruction boundary.

use super::reconstruction::{ReconstructionFrame, ReconstructionProvider, ReconstructionSize};
use crate::gpu;
use anyhow::{Context, Result};
use dlss_wgpu::super_resolution::{DlssSuperResolution, DlssSuperResolutionExposure, DlssSuperResolutionRenderParameters};
use dlss_wgpu::{DlssError, DlssFeatureFlags, DlssPerfQualityMode as Mode};
use wgpu::{CommandBuffer, CommandEncoder, TextureFormat, TextureUsages, TextureView};

#[derive(Default)]
pub(super) struct Dlss {
    state: Option<State>,
}

struct State {
    // The wrapper keeps NGX and the device alive and waits for GPU use before releasing this context.
    context: DlssSuperResolution,
    mode: Mode,
    input: [u32; 2],
    reset: bool,
}

impl State {
    fn supports(&self, size: ReconstructionSize) -> bool {
        self.context.upscaled_resolution() == size.output
            && (self.mode == Mode::Dlaa) == (size.input == size.output)
            && in_range(&self.context, size.input)
    }
}

fn in_range(context: &DlssSuperResolution, input: [u32; 2]) -> bool {
    let r = context.render_resolution_range();
    // RangeInclusive<[u32; 2]>::contains would compare the dimensions lexicographically.
    (0..2).all(|i| r.start()[i] <= input[i] && input[i] <= r.end()[i])
}

impl ReconstructionProvider for Dlss {
    fn prepare(&mut self, gpu: &gpu::Gpu, size: ReconstructionSize) -> Result<bool> {
        let Some(sdk) = &gpu.dlss_sdk else { return Ok(false) };
        if (0..2).any(|i| size.output[i] < 32 || size.input[i] == 0 || size.input[i] > size.output[i]) {
            return Ok(false);
        }
        if let Some(state) = &mut self.state {
            if state.supports(size) && (size.dynamic || state.input == size.input) {
                state.input = size.input;
                return Ok(true);
            }
        }

        // NGX initializes each preset at its own optimal size and reports the actual permitted input range.
        // Probe in order of distance from the nominal ratios; those ratios are not validation bounds.
        let mut scaled = [(Mode::Quality, 667), (Mode::Balanced, 588), (Mode::Performance, 500), (Mode::UltraPerformance, 333)];
        let native = [(Mode::Dlaa, 1000)];
        let modes = if size.input == size.output {
            native.as_slice()
        } else {
            scaled.sort_unstable_by_key(|&(_, scale)| (size.input[0] as u64 * 1000).abs_diff(size.output[0] as u64 * scale));
            scaled.as_slice()
        };
        let flags = DlssFeatureFlags::HighDynamicRange | DlssFeatureFlags::LowResolutionMotionVectors
            | DlssFeatureFlags::InvertedDepth | DlssFeatureFlags::AutoExposure;
        for &(mode, _) in modes {
            if let Some(state) = &mut self.state {
                if state.mode == mode && state.context.upscaled_resolution() == size.output {
                    if state.supports(size) {
                        state.input = size.input;
                        state.reset = true;
                        return Ok(true);
                    }
                    continue;
                }
            }
            let context = match DlssSuperResolution::new(size.output, mode, flags, sdk.clone(), &gpu.device, &gpu.queue) {
                Ok(context) => context,
                Err(DlssError::FeatureNotSupported | DlssError::InvalidParameters | DlssError::UnsupportedParameter) => continue,
                Err(err) => return Err(err).with_context(|| format!("create DLSS {mode:?} for {:?}", size.output)),
            };
            if in_range(&context, size.input) {
                self.state = Some(State { context, mode, input: size.input, reset: true });
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn encode(&mut self, frame: ReconstructionFrame<'_>, encoder: &mut CommandEncoder) -> Result<Option<CommandBuffer>> {
        let state = self.state.as_mut().context("DLSS context was not prepared")?;
        anyhow::ensure!(state.input == frame.size.input && state.supports(frame.size), "DLSS render dimensions were not prepared");
        let sampled = TextureUsages::TEXTURE_BINDING;
        let checks = [
            (frame.color, "HDR color", TextureFormat::Rgba16Float, frame.size.input, sampled),
            (frame.depth, "depth", TextureFormat::R32Float, frame.size.input, sampled),
            (frame.motion, "motion", TextureFormat::Rg16Float, frame.size.input, sampled),
            (frame.reactive, "reactive mask", TextureFormat::R8Unorm, frame.size.input, sampled),
            // Some Vulkan NGX paths also clear the output using vkCmdClearColorImage.
            (frame.output, "output", TextureFormat::Rgba16Float, frame.size.output, TextureUsages::STORAGE_BINDING | TextureUsages::COPY_DST),
        ];
        for (view, name, format, size, usage) in checks {
            check_texture(view, name, format, size, usage)?;
        }
        // NGX wants the shift of the projected image, opposite to the displacement of the sample point.
        let [x, y] = frame.jitter;
        let params = DlssSuperResolutionRenderParameters {
            color: frame.color, depth: frame.depth, motion_vectors: frame.motion,
            exposure: DlssSuperResolutionExposure::Automatic, bias: Some(frame.reactive), dlss_output: frame.output,
            reset: state.reset || frame.reset, jitter_offset: [-x, -y],
            partial_texture_size: Some(frame.size.input), motion_vector_scale: None,
        };
        state.reset = true;
        // The wrapper records barriers on `encoder`. Submit the returned buffer immediately after it, in the same submit.
        let native = state.context.render(params, encoder, &frame.gpu.adapter).context("encode DLSS evaluation")?;
        state.reset = false;
        Ok(Some(native))
    }
}

fn check_texture(view: &TextureView, name: &str, format: TextureFormat, [w, h]: [u32; 2], usage: TextureUsages) -> Result<()> {
    let t = view.texture();
    anyhow::ensure!(t.format() == format && t.width() == w && t.height() == h && t.usage().contains(usage),
        "invalid DLSS {name} texture: expected {w}x{h} {format:?} with {usage:?}");
    Ok(())
}
