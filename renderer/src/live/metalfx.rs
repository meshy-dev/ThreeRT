//! Apple MetalFX: the temporal scaler as an upscaler, the temporal denoised scaler (macOS 26) as a joint denoiser
//! (live/reconstruction.rs has the two contracts). The engine packs the signals; this binds its textures to the
//! scaler (metalfx.m) and encodes the scaler into a Metal command buffer that wgpu submits between its own.
use super::reconstruction::{JointGuides, JointProvider, Prepared, ReconstructionFrame, ReconstructionSize, UpscaleProvider};
use super::{gpu, Result};
use std::ffi::c_void;
use std::ptr::NonNull;
use wgpu::hal::api::Metal;

unsafe extern "C" {
    /// NULL with `why`: 1 = no such scaler on this OS or device, 2 = the input's scale is outside `scales`
    /// (min, max output / input), 3 = the scaler failed to build.
    fn threert_fx_create(device: *const c_void, denoise: bool, dynamic: bool, iw: u32, ih: u32, ow: u32, oh: u32, scales: *mut f32, why: *mut i32) -> *mut c_void;
    fn threert_fx_release(scaler: *mut c_void);
    fn threert_fx_gpu_ms() -> f32;
    /// `inputs`: the frame's colour, depth, motion, reactive; then (denoise) overlay, bypass, diffuse albedo,
    /// specular albedo, normal, roughness, specular hit distance.
    fn threert_fx_bind(scaler: *mut c_void, denoise: bool, inputs: *const *const c_void, output: *const c_void);
    fn threert_fx_encode(scaler: *mut c_void, denoise: bool, cmd: *const c_void, reset: bool, iw: u32, ih: u32, jx: f32, jy: f32, view: *const f32, projection: *const f32);
}

struct Scaler(NonNull<c_void>);
// A retained Metal object can move with its LiveCore. Only an exclusive borrow of that core binds or encodes the
// scaler, and command-buffer completion holds its own Objective-C retain.
unsafe impl Send for Scaler {}
impl Drop for Scaler {
    fn drop(&mut self) { unsafe { threert_fx_release(self.0.as_ptr()) }; }
}

/// One MetalFX scaler, made for a size (and, for dynamic resolution, a range of input scales).
pub(super) struct MetalFx {
    denoise: bool,
    scaler: Option<Scaler>,
    /// Input, output, dynamic.
    key: Option<([u32; 2], [u32; 2], bool)>,
    /// Dynamic resolution (the plain scaler only): inputs in textures of the output's size, within these scales.
    range: Option<[f32; 2]>,
    /// A new scaler: no history yet.
    fresh: bool,
    /// The render scales it takes (input / output), as the system reported them at the last prepare.
    scales: Option<[f32; 2]>,
}

impl MetalFx {
    pub(super) fn upscaler() -> Self { Self { denoise: false, scaler: None, key: None, range: None, fresh: true, scales: None } }
    pub(super) fn denoiser() -> Self { Self { denoise: true, scaler: None, key: None, range: None, fresh: true, scales: None } }

    fn name(&self) -> &'static str {
        if self.denoise { "the MetalFX temporal denoised scaler" } else { "the MetalFX temporal scaler" }
    }

    fn prepare(&mut self, gpu: &gpu::Gpu, size: ReconstructionSize) -> Prepared {
        let ReconstructionSize { input, output, dynamic } = size;
        let key = Some((input, output, dynamic));
        // The plain scaler takes a varying input within the range it was made for.
        let dynamic_inputs = dynamic && !self.denoise;
        if self.key != key && dynamic_inputs && self.scaler.is_some() {
            if let (Some((_, prev_out, true)), Some([min, max])) = (self.key, self.range) {
                let fits = |i: u32, o: u32| i >= (o as f32 / max).floor() as u32 && i <= (o as f32 / min).ceil() as u32;
                if output == prev_out && fits(input[0], output[0]) && fits(input[1], output[1]) {
                    self.key = key;
                }
            }
        }
        if self.key != key || self.scaler.is_none() {
            self.scaler = None;
            self.key = None;
            let mut scales = [1.0f32; 2];
            let mut why = 0i32;
            // (Release the HAL guard before further calls on the wgpu device.)
            let raw = unsafe { gpu.device.as_hal::<Metal>() }.and_then(|dev| {
                let ptr = std::ptr::from_ref(&**dev.raw_device()).cast();
                NonNull::new(unsafe { threert_fx_create(ptr, self.denoise, dynamic_inputs, input[0], input[1], output[0], output[1], scales.as_mut_ptr(), &mut why) })
            });
            // (MetalFX reports output / input; a render scale is its inverse.)
            self.scales = (why == 0 || why == 2).then(|| [1.0 / scales[1].max(1e-3), 1.0 / scales[0].max(1e-3)]);
            let Some(raw) = raw else {
                return Prepared::Unsupported(match why {
                    1 => format!("this device has no {} (macOS {} or iOS {} or later, Apple silicon)", self.name(), if self.denoise { 26 } else { 13 }, if self.denoise { 26 } else { 16 }),
                    2 => format!("{} takes outputs {:.2}x to {:.2}x its input; {}x{} to {}x{} is {:.2}x", self.name(), scales[0], scales[1],
                        input[0], input[1], output[0], output[1], output[0] as f32 / input[0] as f32),
                    _ => format!("{} could not be made for {}x{} to {}x{}", self.name(), input[0], input[1], output[0], output[1]),
                });
            };
            self.scaler = Some(Scaler(raw));
            self.key = key;
            self.range = dynamic_inputs.then_some(scales);
            self.fresh = true;
        }
        Prepared::Ready { capacity: if self.range.is_some() { output } else { input } }
    }

    fn encode(&mut self, f: &ReconstructionFrame<'_>, guides: Option<&JointGuides<'_>>) -> Result<Option<wgpu::CommandBuffer>> {
        let scaler = self.scaler.as_ref().ok_or_else(|| anyhow::anyhow!("MetalFX was not prepared"))?.0.as_ptr();
        // The textures' Metal objects; the scaler retains what it is given.
        let raw = |v: &wgpu::TextureView| -> Result<*const c_void> {
            let t = v.texture();
            let hal = unsafe { t.as_hal::<Metal>() }.ok_or_else(|| anyhow::anyhow!("not a Metal texture"))?;
            Ok(std::ptr::from_ref(hal.raw_handle()).cast::<c_void>())
        };
        let mut views = vec![f.color, f.depth, f.motion, f.reactive];
        if let Some(g) = guides {
            views.extend([g.overlay, g.bypass, g.diffuse_albedo, g.specular_albedo, g.normal, g.roughness, g.specular_hit_distance]);
        }
        let inputs = views.into_iter().map(raw).collect::<Result<Vec<_>>>()?;
        let output = raw(f.output)?;
        unsafe { threert_fx_bind(scaler, self.denoise, inputs.as_ptr(), output) };
        // wgpu 30 permits one encoding API per encoder: the scaler gets a command buffer of its own, submitted after
        // the encoder that packed its inputs and before the one that reads its output, on the same queue (Metal's
        // tracked hazards order the accesses; wgpu keeps the textures alive through the submission).
        let mut native = f.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("MetalFX") });
        let [iw, ih] = f.size.input;
        unsafe {
            native.as_hal_mut::<Metal, _, _>(|encoder| {
                let cmd = encoder.and_then(|e| e.raw_command_buffer()).expect("Metal command buffer");
                // MetalFX's jitter is the shift of the projected image: opposite to the sample's.
                threert_fx_encode(scaler, self.denoise, std::ptr::from_ref(cmd).cast(), f.reset || self.fresh, iw, ih,
                    -f.jitter[0], -f.jitter[1], f.world_to_view.as_ptr(), f.view_to_clip.as_ptr());
            });
        }
        self.fresh = false;
        Ok(Some(native.finish()))
    }
}

impl UpscaleProvider for MetalFx {
    fn prepare(&mut self, gpu: &gpu::Gpu, size: ReconstructionSize) -> Result<Prepared> { Ok(MetalFx::prepare(self, gpu, size)) }
    fn encode(&mut self, frame: ReconstructionFrame<'_>, _: &mut wgpu::CommandEncoder) -> Result<Option<wgpu::CommandBuffer>> {
        MetalFx::encode(self, &frame, None)
    }
    fn untimed_gpu_ms(&self) -> f32 { if self.scaler.is_some() { unsafe { threert_fx_gpu_ms() } } else { 0.0 } }
    fn scale_range(&self) -> Option<[f32; 2]> { self.scales }
    fn invalidate(&mut self) { self.scaler = None; self.key = None; }
}

impl JointProvider for MetalFx {
    fn prepare(&mut self, gpu: &gpu::Gpu, size: ReconstructionSize) -> Result<Prepared> { Ok(MetalFx::prepare(self, gpu, size)) }
    fn encode(&mut self, frame: ReconstructionFrame<'_>, guides: JointGuides<'_>, _: &mut wgpu::CommandEncoder) -> Result<Option<wgpu::CommandBuffer>> {
        MetalFx::encode(self, &frame, Some(&guides))
    }
    fn untimed_gpu_ms(&self) -> f32 { if self.scaler.is_some() { unsafe { threert_fx_gpu_ms() } } else { 0.0 } }
    fn scale_range(&self) -> Option<[f32; 2]> { self.scales }
    fn invalidate(&mut self) { self.scaler = None; self.key = None; }
}
