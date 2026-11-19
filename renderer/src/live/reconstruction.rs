//! Live HDR reconstruction selection and the texture contract for external providers.
use super::{fsr, gpu, targets, Camera, Result, Targets};
use std::sync::{Arc, Mutex};
use wgpu::{CommandBuffer, CommandEncoder, TextureView};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Reconstruction {
    #[default]
    Builtin,
    Auto,
    Temporal,
    Denoised,
    Dlss,
    Fsr,
    Custom(Arc<str>),
}

impl Reconstruction {
    pub fn name(&self) -> &str {
        match self {
            Self::Builtin => "builtin",
            Self::Auto => "auto",
            Self::Temporal => "metalfx",
            Self::Denoised => "metalfx-denoised",
            Self::Dlss => "dlss",
            Self::Fsr => "fsr",
            Self::Custom(id) => id,
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "builtin" => Self::Builtin,
            "auto" => Self::Auto,
            "metalfx" => Self::Temporal,
            "metalfx-denoised" => Self::Denoised,
            "dlss" => Self::Dlss,
            "fsr" => Self::Fsr,
            _ if s.strip_prefix("custom:").is_some_and(|id| {
                (1..=48).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            }) => Self::Custom(Arc::from(s)),
            _ => anyhow::bail!("unknown reconstruction '{s}'; use builtin, auto, dlss, fsr, metalfx, metalfx-denoised or custom:<id>"),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconstructionSize {
    pub input: [u32; 2],
    pub output: [u32; 2],
    pub dynamic: bool,
}

/// Textures are on the live device, exact input/output size, top-left origin. They are borrowed for this encode.
pub struct ReconstructionFrame<'a> {
    pub gpu: &'a gpu::Gpu,
    pub size: ReconstructionSize,
    /// RGBA16Float, nonnegative scene-linear HDR in units of `radiance_unit` nits, alpha 1.
    pub color: &'a TextureView,
    /// R32Float, reversed infinite Z (near = 0.01 m, sky = 0).
    pub depth: &'a TextureView,
    /// RG16Float, current-to-previous displacement in input pixels; no jitter in the motion.
    pub motion: &'a TextureView,
    /// R8Unorm, 0 = stable, 1 = fully reactive.
    pub reactive: &'a TextureView,
    /// RGBA16Float storage texture. Write the full image in the same HDR units as `color`.
    pub output: &'a TextureView,
    pub radiance_unit: f32,
    /// Sample position minus pixel centre, in input pixels (+Y down).
    pub jitter: [f32; 2],
    /// Discard prior history, including after cuts, mode changes and attachment recreation.
    pub reset: bool,
    pub camera: Camera,
    pub aspect: f32,
}

/// A live upscaler installed by the embedding host. ThreeRT still performs its own denoising and post effects.
pub trait ReconstructionProvider: Send {
    /// Called on first use or when the selected mode or dimensions change. Return false for unsupported sizes/devices.
    fn prepare(&mut self, gpu: &gpu::Gpu, size: ReconstructionSize) -> Result<bool>;

    /// Record on `encoder`, or return a native command buffer ordered after it and before ThreeRT reads `output`.
    /// A native provider must record resource transitions on `encoder` and keep its resources alive through GPU use.
    /// On error the encoder must remain usable; the live frame falls back to the built-in reconstruction.
    fn encode(&mut self, frame: ReconstructionFrame<'_>, encoder: &mut CommandEncoder) -> Result<Option<CommandBuffer>>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Active {
    #[default]
    Builtin,
    #[cfg(target_os = "macos")]
    MetalFx,
    #[cfg(target_os = "macos")]
    MetalFxDenoised,
    Fsr,
    #[cfg(all(feature = "dlss", target_arch = "x86_64", any(target_os = "linux", target_os = "windows")))]
    Dlss,
    Custom(usize),
}

impl Active {
    pub(super) fn denoises(self) -> bool {
        #[cfg(target_os = "macos")]
        { self == Self::MetalFxDenoised }
        #[cfg(not(target_os = "macos"))]
        { false }
    }

    pub(super) fn skips(self, pass: &str) -> bool {
        self != Self::Builtin && (matches!(pass, "lock_carry" | "track" | "judge" | "reject" | "upscale")
            || self.denoises() && (matches!(pass, "refl" | "dtemporal" | "stemporal") || pass.starts_with("atrous") || pass.starts_with("sfilt")))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fallback {
    Unavailable,
    DiagnosticView,
    ProviderError,
}

impl Fallback {
    fn name(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::DiagnosticView => "diagnostic-view",
            Self::ProviderError => "provider-error",
        }
    }
}

#[derive(Default)]
struct ControlState {
    preference: Option<Reconstruction>,
    pending: bool,
    requested: Reconstruction,
    active: Option<Reconstruction>,
    fallback: Option<Fallback>,
}

/// Local live-view controls shared with a game runtime. Calls before the first frame can queue a selection.
#[derive(Clone, Default)]
pub struct ReconstructionControl(Arc<Mutex<ControlState>>);

impl ReconstructionControl {
    pub fn request(&self, mode: Reconstruction) {
        let mut s = self.0.lock().unwrap();
        s.preference = Some(mode);
        s.pending = true;
    }

    pub fn status(&self) -> serde_json::Value {
        let s = self.0.lock().unwrap();
        serde_json::json!({
            "requested": s.requested().name(),
            "active": s.active.as_ref().map(Reconstruction::name),
            "fallback": s.fallback.map(Fallback::name),
        })
    }
}

impl ControlState {
    fn requested(&self) -> &Reconstruction {
        if self.pending { self.preference.as_ref().unwrap() } else { &self.requested }
    }
}

#[derive(Default)]
pub(super) struct Reconstructor {
    pub(super) requested: Reconstruction,
    pub(super) active: Active,
    key: Option<(ReconstructionSize, bool)>,
    fallback: Option<Fallback>,
    control: Option<ReconstructionControl>,
    reported: bool,
    #[cfg(target_os = "macos")]
    metalfx: super::metalfx::MetalFx,
    #[cfg(all(feature = "dlss", target_arch = "x86_64", any(target_os = "linux", target_os = "windows")))]
    dlss: super::dlss::Dlss,
    fsr: Option<fsr::Fsr>,
    custom: Vec<(Reconstruction, Box<dyn ReconstructionProvider>)>,
    // Fields drop in declaration order: providers release GPU work before their input/output textures.
    textures: Option<ProviderTextures>,
    pipes: Option<ProviderPipelines>,
}

impl Reconstructor {
    pub(super) fn set(&mut self, mode: Reconstruction) -> bool {
        if self.requested == mode { return false; }
        self.requested = mode;
        self.key = None;
        self.reported = false;
        true
    }

    pub(super) fn attach(&mut self, control: ReconstructionControl) -> bool {
        let mut s = control.0.lock().unwrap();
        let changed = s.preference.clone().is_some_and(|mode| self.set(mode));
        s.pending = false;
        s.requested = self.requested.clone();
        s.active = None;
        s.fallback = None;
        drop(s);
        self.reported = false;
        self.control = Some(control);
        changed
    }

    pub(super) fn set_from_host(&mut self, mode: Reconstruction) -> bool {
        let changed = self.set(mode);
        if let Some(c) = &self.control {
            let mut s = c.0.lock().unwrap();
            s.pending = false;
            s.requested = self.requested.clone();
        }
        changed
    }

    pub(super) fn register(&mut self, id: &str, provider: Box<dyn ReconstructionProvider>) -> Result<()> {
        let mode = Reconstruction::parse(id)?;
        anyhow::ensure!(matches!(mode, Reconstruction::Custom(_)), "provider id must use the custom: namespace");
        anyhow::ensure!(!self.custom.iter().any(|(m, _)| *m == mode), "duplicate reconstruction provider '{id}'");
        if self.requested == mode { self.key = None; }
        self.custom.push((mode, provider));
        Ok(())
    }

    pub(super) fn invalidate(&mut self) {
        self.key = None;
        self.fsr = None;
        self.textures = None;
        #[cfg(target_os = "macos")]
        self.metalfx.invalidate();
    }

    pub(super) fn mode(&self) -> &Reconstruction {
        match self.active {
            Active::Builtin => &Reconstruction::Builtin,
            #[cfg(target_os = "macos")]
            Active::MetalFx => &Reconstruction::Temporal,
            #[cfg(target_os = "macos")]
            Active::MetalFxDenoised => &Reconstruction::Denoised,
            Active::Fsr => &Reconstruction::Fsr,
            #[cfg(all(feature = "dlss", target_arch = "x86_64", any(target_os = "linux", target_os = "windows")))]
            Active::Dlss => &Reconstruction::Dlss,
            Active::Custom(i) => &self.custom[i].0,
        }
    }

    pub(super) fn fallback(&self) -> Option<&'static str> { self.fallback.map(Fallback::name) }

    /// Resolve before tracing, so denoising and jitter are selected for the same frame.
    pub(super) fn prepare(&mut self, g: &gpu::Gpu, targets: &Targets, size: ReconstructionSize, diagnostic: bool) -> bool {
        let pending = self.control.as_ref().and_then(|c| {
            let mut s = c.0.lock().unwrap();
            if !std::mem::take(&mut s.pending) { return None; }
            s.requested = s.preference.as_ref().unwrap().clone();
            Some(s.requested.clone())
        });
        let changed_request = pending.is_some_and(|mode| self.set(mode));
        if self.key == Some((size, diagnostic)) { return changed_request; }
        self.key = Some((size, diagnostic));
        let old = (self.active, self.fallback);
        self.fallback = None;
        self.active = if diagnostic {
            if self.requested != Reconstruction::Builtin { self.fallback = Some(Fallback::DiagnosticView); }
            Active::Builtin
        } else {
            self.select(g, targets, size)
        };
        if (self.active, self.fallback) != old { self.reported = false; }
        changed_request || self.active != old.0
    }

    fn select(&mut self, g: &gpu::Gpu, targets: &Targets, size: ReconstructionSize) -> Active {
        use Reconstruction::*;
        if self.requested != Builtin {
            let limit = g.device.limits().max_texture_dimension_2d;
            if size.output.into_iter().any(|side| side > limit) {
                if self.requested != Auto { self.fallback = Some(Fallback::Unavailable); }
                return Active::Builtin;
            }
        }
        match &self.requested {
            Builtin => return Active::Builtin,
            Fsr => {
                self.fsr.get_or_insert_with(|| fsr::Fsr::new(g, targets, size.output));
                return Active::Fsr;
            }
            Auto | Temporal | Denoised => {
                #[cfg(target_os = "macos")]
                if let Some(active) = self.metalfx.prepare(g, targets, &self.requested, size) {
                    return active;
                }
            }
            Dlss => {}
            Custom(_) => {
                if let Some(i) = self.custom.iter().position(|(mode, _)| *mode == self.requested) {
                    match self.custom[i].1.prepare(g, size) {
                        Ok(true) => { self.prepare_textures(g, targets, size); return Active::Custom(i); }
                        Ok(false) => {}
                        Err(e) => { self.provider_error(&e); return Active::Builtin; }
                    }
                }
            }
        }
        #[cfg(all(feature = "dlss", target_arch = "x86_64", any(target_os = "linux", target_os = "windows")))]
        if matches!(self.requested, Auto | Dlss) {
            match self.dlss.prepare(g, size) {
                Ok(true) => { self.prepare_textures(g, targets, size); return Active::Dlss; }
                Ok(false) => {}
                Err(e) => { self.provider_error(&e); return Active::Builtin; }
            }
        }
        if self.requested != Auto { self.fallback = Some(Fallback::Unavailable); }
        Active::Builtin
    }

    fn prepare_textures(&mut self, g: &gpu::Gpu, targets: &Targets, size: ReconstructionSize) {
        if self.textures.as_ref().is_none_or(|t| t.size != (size.input, size.output)) {
            let pipes = self.pipes.get_or_insert_with(|| ProviderPipelines::new(g));
            self.textures = Some(ProviderTextures::new(g, targets, pipes, size));
        }
    }

    fn provider_error(&mut self, e: &anyhow::Error) {
        eprintln!("live: {} reconstruction failed: {e:#}; using builtin", self.requested.name());
        self.active = Active::Builtin;
        self.fallback = Some(Fallback::ProviderError);
        self.reported = false;
    }

    pub(super) fn publish(&mut self) {
        if self.reported { return; }
        if let Some(c) = &self.control {
            let mut s = c.0.lock().unwrap();
            s.requested = self.requested.clone();
            s.active = Some(self.mode().clone());
            s.fallback = self.fallback;
        }
        self.reported = true;
    }

    pub(super) fn encode(&mut self, g: &gpu::Gpu, e: &mut CommandEncoder, size: ReconstructionSize,
        cam: Camera, aspect: f32, jitter: (f32, f32), exposure: f32, reset: bool,
    ) -> Result<Option<[CommandBuffer; 2]>> {
        let provider = match self.active {
            Active::Builtin => return Ok(None),
            Active::Fsr => { self.fsr.as_ref().unwrap().encode(e); return Ok(None); }
            #[cfg(target_os = "macos")]
            Active::MetalFx | Active::MetalFxDenoised => {
                return Ok(Some(self.metalfx.encode(&g.device, e, cam, aspect, jitter, exposure, reset)));
            }
            #[cfg(all(feature = "dlss", target_arch = "x86_64", any(target_os = "linux", target_os = "windows")))]
            Active::Dlss => &mut self.dlss as &mut dyn ReconstructionProvider,
            Active::Custom(i) => &mut *self.custom[i].1,
        };
        let textures = self.textures.as_mut().expect("prepared reconstruction textures");
        let pipes = self.pipes.as_ref().unwrap();
        textures.pack(e, pipes);
        let dispatch = provider.encode(ReconstructionFrame {
            gpu: g, size, color: &textures.inputs[0], depth: &textures.inputs[1], motion: &textures.inputs[2],
            reactive: &textures.inputs[3], output: &textures.output, jitter: [jitter.0 - 0.5, jitter.1 - 0.5],
            radiance_unit: 32.0 / exposure.clamp(0.00001, 64.0), reset: reset || textures.fresh || textures.exposure != exposure,
            camera: cam, aspect,
        }, e);
        let native = match dispatch {
            Ok(cb) => cb,
            Err(e) => { self.provider_error(&e); return Err(e); }
        };
        let before_post = native.map(|native| [std::mem::replace(e, g.device.create_command_encoder(&Default::default())).finish(), native]);
        textures.fresh = false;
        textures.exposure = exposure;
        let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("reconstruction to post"), ..Default::default() });
        pass.set_pipeline(&pipes.output);
        pass.set_bind_group(0, &textures.post_group, &[]);
        pass.set_bind_group(1, &textures.output_group, &[]);
        pass.dispatch_workgroups(size.output[0].div_ceil(8), size.output[1].div_ceil(8), 1);
        Ok(before_post)
    }
}

const INPUT_FORMATS: [wgpu::TextureFormat; 4] = [wgpu::TextureFormat::Rgba16Float, wgpu::TextureFormat::R32Float, wgpu::TextureFormat::Rg16Float, wgpu::TextureFormat::R8Unorm];

struct ProviderPipelines {
    pack: wgpu::RenderPipeline,
    output: wgpu::ComputePipeline,
}

impl ProviderPipelines {
    fn new(g: &gpu::Gpu) -> Self {
        let d = &g.device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reconstruction signals"),
            source: wgpu::ShaderSource::Wgsl([targets::live_source(), include_str!("../../shaders/metalfx.wgsl")].concat().into()),
        });
        let pack = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("reconstruction inputs"), layout: None,
            vertex: wgpu::VertexState { module: &module, entry_point: Some("fx_vertex"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fx_inputs"), compilation_options: Default::default(), targets: &INPUT_FORMATS.map(|f| Some(f.into())) }),
            primitive: Default::default(), depth_stencil: None, multisample: Default::default(), multiview_mask: None, cache: g.cache(),
        });
        let output = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("reconstruction output"), layout: None, module: &module,
            entry_point: Some("fx_output_to_post"), compilation_options: Default::default(), cache: g.cache(),
        });
        Self { pack, output }
    }
}

struct ProviderTextures {
    size: ([u32; 2], [u32; 2]),
    inputs: [TextureView; 4],
    output: TextureView,
    input_group: wgpu::BindGroup,
    post_group: wgpu::BindGroup,
    output_group: wgpu::BindGroup,
    fresh: bool,
    exposure: f32,
}

impl ProviderTextures {
    fn new(g: &gpu::Gpu, targets: &Targets, pipes: &ProviderPipelines, size: ReconstructionSize) -> Self {
        use wgpu::TextureUsages as Usage;
        let d = &g.device;
        let view = |label, format, [width, height]: [u32; 2], extra| d.create_texture(&wgpu::TextureDescriptor {
            label: Some(label), format,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            usage: Usage::RENDER_ATTACHMENT | Usage::TEXTURE_BINDING | extra, view_formats: &[],
        }).create_view(&Default::default());
        let inputs = INPUT_FORMATS.map(|f| view("reconstruction input", f, size.input, Usage::empty()));
        let output = view("reconstruction HDR", INPUT_FORMATS[0], size.output, Usage::STORAGE_BINDING | Usage::COPY_DST);
        let bind = |layout: wgpu::BindGroupLayout, entries: &[wgpu::BindGroupEntry<'_>]| {
            d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("reconstruction signals"), layout: &layout, entries })
        };
        let buffer = |name| wgpu::BindGroupEntry {
            binding: super::layout::binding(name).binding,
            resource: if name == "L" { targets.params.as_entire_binding() } else { targets.buf(name).binding() },
        };
        let input_group = bind(pipes.pack.get_bind_group_layout(0), &[buffer("L"), buffer("fb"), buffer("sample_vel")]);
        let post_group = bind(pipes.output.get_bind_group_layout(0), &[buffer("L"), buffer("fa"), buffer("sb"), buffer("sample_vel")]);
        let output_group = bind(pipes.output.get_bind_group_layout(1), &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&output) }]);
        Self { size: (size.input, size.output), inputs, output, input_group, post_group, output_group, fresh: true, exposure: 0.0 }
    }

    fn pack(&self, e: &mut CommandEncoder, pipes: &ProviderPipelines) {
        let attachment = |view| Some(wgpu::RenderPassColorAttachment {
            view, depth_slice: None, resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
        });
        if self.fresh {
            // Initialize before a native writer, so wgpu doesn't clear its output on the first read.
            drop(e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reconstruction output init"), color_attachments: &[attachment(&self.output)], ..Default::default()
            }));
        }
        let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("reconstruction inputs"), color_attachments: &self.inputs.each_ref().map(attachment), ..Default::default()
        });
        pass.set_pipeline(&pipes.pack);
        pass.set_bind_group(0, &self.input_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
