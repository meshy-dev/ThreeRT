//! GPU-only bridge on the Metal command buffer owned and submitted by wgpu. The input attachments
//! and output view keep their textures alive. All resource accesses use Metal tracked hazards.
use super::*;
use std::ffi::c_void;
use std::ptr::NonNull;
use wgpu::hal::api::Metal;
use wgpu::TextureFormat::{self, *};

const INPUTS: &[TextureFormat] = &[Rgba16Float, R32Float, Rg16Float, R8Unorm];
const LAYERS: &[TextureFormat] = &[Rgba16Float, R8Unorm];
const GUIDES: &[TextureFormat] = &[Rgba8Unorm, Rgba8Unorm, Rgba16Float, R16Float, R16Float];

unsafe extern "C" {
    fn threert_fx_create(
        device: *const c_void,
        denoise: bool,
        dynamic: bool,
        iw: u32,
        ih: u32,
        ow: u32,
        oh: u32,
        scales: *mut f32,
    ) -> *mut c_void;
    fn threert_fx_release(scaler: *mut c_void);
    fn threert_fx_bind(scaler: *mut c_void, denoise: bool, inputs: *const *const c_void, output: *const c_void);
    fn threert_fx_encode(
        scaler: *mut c_void,
        denoise: bool,
        cmd: *const c_void,
        reset: bool,
        iw: u32,
        ih: u32,
        jx: f32,
        jy: f32,
        view: *const f32,
        projection: *const f32,
    );
}

struct Scaler(NonNull<c_void>);
// A retained Metal object can move with its LiveCore. Only an exclusive borrow of that core
// binds or encodes the scaler, and command-buffer completion holds its own Objective-C retain.
unsafe impl Send for Scaler {}
impl Drop for Scaler {
    fn drop(&mut self) {
        unsafe { threert_fx_release(self.0.as_ptr()) };
    }
}

struct Pipelines {
    inputs: wgpu::RenderPipeline,
    denoised_inputs: wgpu::RenderPipeline,
    guides: wgpu::RenderPipeline,
    output: wgpu::ComputePipeline,
}

impl Pipelines {
    fn new(g: &gpu::Gpu) -> Self {
        let d = &g.device;
        let src = [targets::live_source(), include_str!("../../shaders/metalfx.wgsl")].concat();
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("MetalFX resources"),
            source: wgpu::ShaderSource::Wgsl(src.into()),
        });
        let render = |name, formats: &[TextureFormat]| {
            let attachments: Vec<_> = formats.iter().map(|f| Some((*f).into())).collect();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("fx_vertex"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(name),
                    targets: &attachments,
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: g.cache(),
            })
        };
        let output = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("MetalFX output"),
            layout: None,
            module: &module,
            entry_point: Some("fx_output_to_post"),
            compilation_options: Default::default(),
            cache: g.cache(),
        });
        Self {
            inputs: render("fx_inputs", INPUTS),
            denoised_inputs: render("fx_denoised_inputs", &[INPUTS, LAYERS].concat()),
            guides: render("fx_guides", GUIDES),
            output,
        }
    }
}

struct Resources {
    scaler: Scaler,
    inputs: Vec<wgpu::TextureView>,
    output: wgpu::TextureView,
    base_group: wgpu::BindGroup,
    guide_group: Option<wgpu::BindGroup>,
    post_group: wgpu::BindGroup,
    output_group: wgpu::BindGroup,
    out_size: (u32, u32),
    fresh: bool,
    exposure: f32,
    dynamic_range: Option<[f32; 2]>,
}

fn attachment(view: &wgpu::TextureView) -> Option<wgpu::RenderPassColorAttachment<'_>> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            store: wgpu::StoreOp::Store,
        },
    })
}

impl Resources {
    fn new(
        g: &gpu::Gpu,
        targets: &Targets,
        pipes: &Pipelines,
        scaler: Scaler,
        denoise: bool,
        input: (u32, u32),
        out: (u32, u32),
        dynamic_range: Option<[f32; 2]>,
    ) -> Self {
        let d = &g.device;
        let texture = |label, format, (width, height), extra| {
            let t = d.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | extra,
                view_formats: &[],
            });
            let view = t.create_view(&Default::default());
            // The view owns t, and bind retains the MTLTexture in the scaler.
            let raw = unsafe { t.as_hal::<Metal>() }.expect("Metal device");
            (view, std::ptr::from_ref(raw.raw_handle()).cast::<c_void>())
        };
        let formats = INPUTS
            .iter()
            .chain(if denoise { LAYERS } else { &[] })
            .chain(if denoise { GUIDES } else { &[] });
        let (inputs, raw): (Vec<_>, Vec<_>) = formats
            .map(|f| {
                texture(
                    "MetalFX input",
                    *f,
                    if dynamic_range.is_some() { out } else { input },
                    wgpu::TextureUsages::empty(),
                )
            })
            .unzip();
        let (output, raw_output) = texture("MetalFX output", Rgba16Float, out, wgpu::TextureUsages::STORAGE_BINDING);
        unsafe { threert_fx_bind(scaler.0.as_ptr(), denoise, raw.as_ptr(), raw_output) };
        let bind = |l: wgpu::BindGroupLayout, names: &[&str]| {
            let entries: Vec<_> = names
                .iter()
                .map(|&n| wgpu::BindGroupEntry {
                    binding: layout::binding(n).binding,
                    resource: match n {
                        "L" => targets.params.as_entire_binding(),
                        "sample_px" => targets.accum.as_entire_binding(),
                        _ => targets.buf(n).binding(),
                    },
                })
                .collect();
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("MetalFX signals"),
                layout: &l,
                entries: &entries,
            })
        };
        let base_group = if denoise {
            bind(
                pipes.denoised_inputs.get_bind_group_layout(0),
                &["L", "fb", "sample_px", "sample_vel"],
            )
        } else {
            bind(pipes.inputs.get_bind_group_layout(0), &["L", "fb", "sample_vel"])
        };
        let guide_group = denoise.then(|| bind(pipes.guides.get_bind_group_layout(0), &["L", "sample_px", "refl_in"]));
        let post_group = bind(pipes.output.get_bind_group_layout(0), &["L", "fa", "sb", "sample_vel"]);
        let output_group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MetalFX HDR"),
            layout: &pipes.output.get_bind_group_layout(1),
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&output),
            }],
        });
        Self {
            scaler,
            inputs,
            output,
            base_group,
            guide_group,
            post_group,
            output_group,
            out_size: out,
            fresh: true,
            exposure: 0.0,
            dynamic_range,
        }
    }
}

#[derive(Default)]
pub(super) struct MetalFx {
    key: Option<(&'static [bool], [u32; 2], [u32; 2], bool)>,
    pipes: Option<Pipelines>,
    resources: Option<Resources>,
}

impl MetalFx {
    pub(super) fn invalidate(&mut self) {
        self.key = None;
        self.resources = None;
    }

    pub(super) fn prepare(
        &mut self,
        g: &gpu::Gpu,
        targets: &Targets,
        mode: &Reconstruction,
        size: ReconstructionSize,
    ) -> Option<Active> {
        let candidates: &'static [bool] = match mode {
            Reconstruction::Auto => &[true, false],
            Reconstruction::Temporal => &[false],
            Reconstruction::Denoised => &[true],
            _ => return None,
        };
        let ReconstructionSize { input, output: out, dynamic } = size;
        let key = Some((candidates, input, out, dynamic));
        if self.key != key && dynamic {
            if let (Some((prev, _, prev_out, true)), Some([min, max])) =
                (self.key, self.resources.as_ref().and_then(|r| r.dynamic_range))
            {
                let fits = |i: u32, o: u32| i >= (o as f32 / max).floor() as u32 && i <= (o as f32 / min).ceil() as u32;
                if candidates == prev && out == prev_out && fits(input[0], out[0]) && fits(input[1], out[1]) {
                    self.key = key;
                }
            }
        }
        if self.key != key {
            self.invalidate();
            self.key = key;
            for &denoise in candidates {
                // Release the HAL guard before making further calls on the wgpu device.
                let mut scales = [1.0f32; 2];
                let dynamic_inputs = dynamic && !denoise;
                let raw = unsafe { g.device.as_hal::<Metal>() }.and_then(|dev| {
                    let ptr = std::ptr::from_ref(&**dev.raw_device()).cast();
                    NonNull::new(unsafe {
                        threert_fx_create(ptr, denoise, dynamic_inputs, input[0], input[1], out[0], out[1], scales.as_mut_ptr())
                    })
                });
                if let Some(raw) = raw {
                    let scaler = Scaler(raw);
                    let pipes = self.pipes.get_or_insert_with(|| Pipelines::new(g));
                    self.resources = Some(Resources::new(
                        g,
                        targets,
                        pipes,
                        scaler,
                        denoise,
                        (input[0], input[1]),
                        (out[0], out[1]),
                        dynamic_inputs.then_some(scales),
                    ));
                    break;
                }
            }
        }
        self.resources.as_ref().map(|r| {
            if r.guide_group.is_some() { Active::MetalFxDenoised } else { Active::MetalFx }
        })
    }

    pub(super) fn encode(
        &mut self,
        device: &wgpu::Device,
        e: &mut wgpu::CommandEncoder,
        cam: Camera,
        aspect: f32,
        jitter: (f32, f32),
        exposure: f32,
        reset: bool,
    ) -> [wgpu::CommandBuffer; 2] {
        let (_, [iw, ih], _, _) = self.key.expect("prepared MetalFX");
        let r = self.resources.as_mut().expect("prepared MetalFX");
        let p = self.pipes.as_ref().unwrap();
        if r.fresh {
            // Mark the external writer's destination initialized in wgpu as well. Otherwise wgpu can
            // insert its lazy zero-initialization immediately before the first read of the MetalFX output.
            drop(e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("MetalFX output init"),
                color_attachments: &[attachment(&r.output)],
                ..Default::default()
            }));
        }
        let mut pack = |views: &[wgpu::TextureView], pipe: &wgpu::RenderPipeline, group: &wgpu::BindGroup| {
            let attachments: Vec<_> = views.iter().map(attachment).collect();
            let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("MetalFX inputs"),
                color_attachments: &attachments,
                ..Default::default()
            });
            pass.set_scissor_rect(0, 0, iw, ih);
            pass.set_viewport(0.0, 0.0, iw as f32, ih as f32, 0.0, 1.0);
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        };
        if let Some(group) = &r.guide_group {
            let (base, guides) = r.inputs.split_at(INPUTS.len() + LAYERS.len());
            pack(base, &p.denoised_inputs, &r.base_group);
            pack(guides, &p.guides, group);
        } else {
            pack(&r.inputs, &p.inputs, &r.base_group);
        }
        let view = glam::Mat4::look_to_rh(cam.pos, cam.fwd, cam.up).to_cols_array();
        let proj = glam::Mat4::perspective_infinite_reverse_rh(2.0 * cam.tan_half_fov.atan(), aspect, 0.01).to_cols_array();
        // All wgpu passes are closed. This scaler only encodes on the borrowed command buffer. Its textures
        // are ordinary tracked Metal resources on that device. wgpu retains them through the surrounding passes.
        // wgpu 30 permits only one encoding API per encoder. Submit all three command buffers together,
        // in order on the same queue. This keeps wgpu resources alive without a CPU fence or readback.
        let before = std::mem::replace(e, device.create_command_encoder(&Default::default())).finish();
        let mut native = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("MetalFX") });
        unsafe {
            native.as_hal_mut::<Metal, _, _>(|encoder| {
                let cmd = encoder.and_then(|e| e.raw_command_buffer()).expect("Metal command buffer");
                threert_fx_encode(
                    r.scaler.0.as_ptr(),
                    r.guide_group.is_some(),
                    std::ptr::from_ref(cmd).cast(),
                    reset || r.fresh || exposure != r.exposure,
                    iw,
                    ih,
                    0.5 - jitter.0,
                    0.5 - jitter.1,
                    view.as_ptr(),
                    proj.as_ptr(),
                );
            });
        }
        r.fresh = false;
        r.exposure = exposure;
        let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("MetalFX to post"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&p.output);
        pass.set_bind_group(0, &r.post_group, &[]);
        pass.set_bind_group(1, &r.output_group, &[]);
        pass.dispatch_workgroups(r.out_size.0.div_ceil(8), r.out_size.1.div_ceil(8), 1);
        [before, native.finish()]
    }
}
