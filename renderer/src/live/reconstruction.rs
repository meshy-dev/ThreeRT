//! Live reconstruction: the path tracer's few samples a pixel, at the render resolution, made into a clean image at
//! the output resolution. That is two jobs, in this order:
//!
//! 1. **denoise**: the samples' noise (too few rays a pixel);
//! 2. **upscale**: the image to the output size, anti-aliased (too few pixels), from the history of earlier frames.
//!
//! ThreeRT does both itself (`builtin`: live.wgsl `refl` .. `sfilt2`, then `lock_carry` .. `upscale`). Vendor
//! technology takes over either the second job or both, and that is the only distinction the engine makes:
//!
//! - an **upscaler** ([`UpscaleProvider`]: DLSS Super Resolution and DLAA, the MetalFX temporal scaler, XeSS SR,
//!   FSR 2+, ...) reads ThreeRT's *denoised* image and replaces the upscale stage;
//! - a **joint denoiser** ([`JointProvider`]: DLSS Ray Reconstruction, the MetalFX temporal denoised scaler, ...)
//!   reads the *noisy* signal, with the surface guides it needs to tell lighting from texture (albedos, normal,
//!   roughness, specular hit distance), and replaces both. One network that sees the raw signal keeps detail a
//!   separate denoiser would have averaged away before the upscaler saw it, and keeps one history instead of two.
//!
//! The two read different signals, so they are two contracts ([`ReconstructionFrame`], plus [`JointGuides`] for a
//! joint denoiser), not variants of one. The engine owns the contracts: it packs the signals into textures and takes
//! the result back into its post chain, so a vendor's code is an adapter of a few calls.
//!
//! A request ([`Reconstruction`]) is an ordered list of methods ([`Method`]); the first this device runs is used.
//! The status says which ran, what each stage is, and why each method before it did not (nothing falls back
//! silently).
use super::{fsr, gpu, targets, Camera, Result, Targets};
use std::sync::Arc;
use wgpu::{CommandBuffer, CommandEncoder, TextureView};

/// One way of reconstructing the frame: who denoises and who upscales.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// ThreeRT's denoiser, then its temporal upscaler.
    Builtin,
    /// Neither: each frame's samples as traced (the signal a joint denoiser receives), brought to the output size
    /// without history. A view of the input, not a way to ship.
    Off,
    /// ThreeRT's denoiser, then FidelityFX Super Resolution 1 (spatial: no history).
    Fsr,
    /// ThreeRT's denoiser, then the MetalFX temporal scaler (macOS 13+, iOS 16+).
    MetalFx,
    /// The MetalFX temporal denoised scaler: both jobs (macOS 26+ on Apple silicon, iOS 26+).
    MetalFxDenoised,
    /// ThreeRT's denoiser, then DLSS Super Resolution (DLAA at scale 1; NVIDIA RTX, feature `dlss`).
    Dlss,
    /// DLSS Ray Reconstruction: both jobs (NVIDIA RTX, feature `dlss`).
    DlssRr,
    /// A provider the embedding host registered under `custom:<id>` (either contract).
    Custom(Arc<str>),
}

impl Method {
    pub fn name(&self) -> &str {
        match self {
            Self::Builtin => "builtin",
            Self::Off => "off",
            Self::Fsr => "fsr",
            Self::MetalFx => "metalfx",
            Self::MetalFxDenoised => "metalfx-denoised",
            Self::Dlss => "dlss",
            Self::DlssRr => "dlss-rr",
            Self::Custom(id) => id,
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim() {
            "builtin" => Self::Builtin,
            "off" => Self::Off,
            "fsr" => Self::Fsr,
            "metalfx" => Self::MetalFx,
            "metalfx-denoised" => Self::MetalFxDenoised,
            "dlss" => Self::Dlss,
            "dlss-rr" => Self::DlssRr,
            s if s.strip_prefix("custom:").is_some_and(|id| {
                (1..=48).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            }) => Self::Custom(Arc::from(s)),
            s => anyhow::bail!("unknown reconstruction '{s}'; use auto, builtin, off, fsr, metalfx, metalfx-denoised, dlss, dlss-rr or custom:<id>"),
        })
    }

    /// The methods this build could run on its platform, in the order the viewer's key cycles them.
    pub fn platform() -> Vec<Method> {
        let mut m = vec![Self::Builtin];
        #[cfg(metalfx)]
        m.extend([Self::MetalFxDenoised, Self::MetalFx]);
        #[cfg(dlss)]
        m.extend([Self::DlssRr, Self::Dlss]);
        m.extend([Self::Fsr, Self::Off]);
        m
    }
}

/// A reconstruction request: the methods to try, in order of preference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reconstruction {
    /// The best this device runs: a joint denoiser, else an upscaler, else ThreeRT's own.
    Auto,
    /// These, in order; ThreeRT's own after the last if none of them runs.
    Prefer(Vec<Method>),
}

impl Default for Reconstruction {
    fn default() -> Self { Self::Prefer(vec![Method::Builtin]) }
}

impl From<Method> for Reconstruction {
    fn from(m: Method) -> Self { Self::Prefer(vec![m]) }
}

impl Reconstruction {
    /// `auto`, one method, or a comma-separated list (`metalfx-denoised,metalfx,builtin`).
    pub fn parse(s: &str) -> Result<Self> {
        if s.trim() == "auto" { return Ok(Self::Auto); }
        let methods = s.split(',').filter(|m| !m.trim().is_empty()).map(Method::parse).collect::<Result<Vec<_>>>()?;
        anyhow::ensure!(!methods.is_empty(), "empty reconstruction request");
        Ok(Self::Prefer(methods))
    }

    /// A string as in [`Self::parse`] or an array of method names.
    pub fn from_json(v: &serde_json::Value) -> Result<Self> {
        match v {
            serde_json::Value::String(s) => Self::parse(s),
            serde_json::Value::Array(a) => {
                let names = a.iter().map(|m| m.as_str().ok_or_else(|| anyhow::anyhow!("reconstruction methods are strings"))).collect::<Result<Vec<_>>>()?;
                Self::parse(&names.join(","))
            }
            _ => anyhow::bail!("reconstruction is a method name, a list of them, or \"auto\""),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Self::Auto => "auto".into(),
            Self::Prefer(m) => m.iter().map(Method::name).collect::<Vec<_>>().join(","),
        }
    }

    /// The methods tried, in order. Each ends with `builtin`, which always runs.
    fn chain(&self) -> Vec<Method> {
        let mut chain = match self {
            Self::Auto => {
                #[allow(unused_mut)]
                let mut m = vec![];
                #[cfg(metalfx)]
                m.extend([Method::MetalFxDenoised, Method::MetalFx]);
                #[cfg(dlss)]
                m.extend([Method::DlssRr, Method::Dlss]);
                m
            }
            Self::Prefer(m) => m.clone(),
        };
        if !chain.contains(&Method::Builtin) { chain.push(Method::Builtin); }
        chain
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconstructionSize {
    /// The traced image (the render resolution).
    pub input: [u32; 2],
    /// The view.
    pub output: [u32; 2],
    /// Dynamic resolution: `input` changes from frame to frame.
    pub dynamic: bool,
}

/// What a provider's `prepare` answers.
pub enum Prepared {
    /// It runs at this size. Its input textures are `capacity` (at least `size.input`; larger for a provider that
    /// takes a varying input in fixed textures); each frame's input is their top-left `size.input`.
    Ready { capacity: [u32; 2] },
    /// It does not run here; the reason goes to the status (e.g. "input 639 x 360 is below DLSS's 640 x 360").
    Unsupported(String),
}

impl Prepared {
    /// Ready, with input textures exactly the input's size.
    pub fn ready(size: ReconstructionSize) -> Self { Self::Ready { capacity: size.input } }
}

/// The signals both contracts share. Textures are on the live device, top-left origin, valid for this encode only.
pub struct ReconstructionFrame<'a> {
    pub gpu: &'a gpu::Gpu,
    pub size: ReconstructionSize,
    /// RGBA16Float, scene-linear HDR radiance in units of `radiance_unit` (nonnegative, alpha 1). An upscaler gets
    /// ThreeRT's denoised image; a joint denoiser the traced lighting, noisy, without [`JointGuides::overlay`].
    pub color: &'a TextureView,
    /// R32Float, reversed infinite Z (near = 0.01 m, sky = 0), the projection of `view_to_clip`.
    pub depth: &'a TextureView,
    /// RG16Float, current-to-previous displacement in input pixels; no jitter in it.
    pub motion: &'a TextureView,
    /// R8Unorm, 0 = stable, 1 = fully reactive (history unusable: animated shading, media).
    pub reactive: &'a TextureView,
    /// RGBA16Float storage texture of `size.output`: write the full image, in the same units as `color`.
    pub output: &'a TextureView,
    /// Nits per unit of `color`.
    pub radiance_unit: f32,
    /// Sample position minus pixel centre, in input pixels (+Y down).
    pub jitter: [f32; 2],
    /// Discard history: cuts, mode changes, recreated textures, an exposure change.
    pub reset: bool,
    pub camera: Camera,
    pub aspect: f32,
    /// Column-major, right-handed, the camera at the origin looking down -Z.
    pub world_to_view: [f32; 16],
    /// Column-major, reversed infinite Z with near 0.01 m (`depth`'s projection), unjittered.
    pub view_to_clip: [f32; 16],
}

/// What a joint denoiser reads besides [`ReconstructionFrame`], at the same size.
pub struct JointGuides<'a> {
    /// RGBA16Float, radiance that has no noise to remove (emitters and sky seen directly, or through a mirror or
    /// clear glass), in `radiance_unit`; alpha 1 where the pixel is entirely that. Composite over the result.
    pub overlay: &'a TextureView,
    /// R8Unorm, 1 = do not denoise this pixel (the overlay's).
    pub bypass: &'a TextureView,
    /// RGBA8Unorm, the diffuse reflectance seen at the pixel (the virtual surface's through a mirror).
    pub diffuse_albedo: &'a TextureView,
    /// RGBA8Unorm, the specular reflectance (directional albedo of the specular lobe).
    pub specular_albedo: &'a TextureView,
    /// RGBA16Float, world-space unit normal (xyz).
    pub normal: &'a TextureView,
    /// R16Float, linear roughness, 0 = mirror.
    pub roughness: &'a TextureView,
    /// R16Float, metres from the surface to what its specular ray hit (0 where unknown).
    pub specular_hit_distance: &'a TextureView,
}

/// Replaces the upscale stage; reads ThreeRT's denoised image. ThreeRT still denoises and runs its post chain.
pub trait UpscaleProvider: Send {
    /// On first use and when the size changes.
    fn prepare(&mut self, gpu: &gpu::Gpu, size: ReconstructionSize) -> Result<Prepared>;

    /// Record on `encoder`, or return a native command buffer ordered after it and before ThreeRT reads `output`.
    /// A native provider records its resource transitions on `encoder` and keeps its resources alive through GPU use.
    /// On error the encoder stays usable; the frame falls back to ThreeRT's own.
    fn encode(&mut self, frame: ReconstructionFrame<'_>, encoder: &mut CommandEncoder) -> Result<Option<CommandBuffer>>;

    /// GPU time (ms) of work outside `encoder` (a native command buffer), last finished frame: added to the frame's,
    /// which the dynamic-resolution governor steers by.
    fn untimed_gpu_ms(&self) -> f32 { 0.0 }

    /// The render scales (input / output, a side) it takes at the output last prepared, when it knows them (also after
    /// answering `Unsupported`): dynamic resolution stays within them while it is the method asked for.
    fn scale_range(&self) -> Option<[f32; 2]> { None }

    /// The view was resized: release size-dependent state.
    fn invalidate(&mut self) {}
}

/// Replaces both stages; reads the noisy signal and the guides. ThreeRT's denoiser does not run.
pub trait JointProvider: Send {
    fn prepare(&mut self, gpu: &gpu::Gpu, size: ReconstructionSize) -> Result<Prepared>;
    /// As [`UpscaleProvider::encode`].
    fn encode(&mut self, frame: ReconstructionFrame<'_>, guides: JointGuides<'_>, encoder: &mut CommandEncoder) -> Result<Option<CommandBuffer>>;
    fn untimed_gpu_ms(&self) -> f32 { 0.0 }
    /// As [`UpscaleProvider::scale_range`].
    fn scale_range(&self) -> Option<[f32; 2]> { None }
    fn invalidate(&mut self) {}
    /// Display whites a unit of `color` and of the overlay is (`radiance_unit` = this / exposure, in nits per unit):
    /// 1 = lighting near display white, the scale a denoiser is commonly trained on. Upscalers take 32.
    fn white_units(&self) -> f32 { 1.0 }
}

enum Provider {
    Upscale(Box<dyn UpscaleProvider>),
    Joint(Box<dyn JointProvider>),
}

impl Provider {
    fn joint(&self) -> bool { matches!(self, Self::Joint(_)) }
    fn prepare(&mut self, g: &gpu::Gpu, size: ReconstructionSize) -> Result<Prepared> {
        match self { Self::Upscale(p) => p.prepare(g, size), Self::Joint(p) => p.prepare(g, size) }
    }
    fn untimed_gpu_ms(&self) -> f32 {
        match self { Self::Upscale(p) => p.untimed_gpu_ms(), Self::Joint(p) => p.untimed_gpu_ms() }
    }
    fn white_units(&self) -> f32 {
        match self { Self::Upscale(_) => 32.0, Self::Joint(p) => p.white_units() }
    }
    fn invalidate(&mut self) {
        match self { Self::Upscale(p) => p.invalidate(), Self::Joint(p) => p.invalidate() }
    }
    fn scale_range(&self) -> Option<[f32; 2]> {
        match self { Self::Upscale(p) => p.scale_range(), Self::Joint(p) => p.scale_range() }
    }
}

/// What ran: the pipeline's configuration for a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Active {
    #[default]
    Builtin,
    Off,
    Fsr,
    /// `providers[i]`, an upscaler.
    Upscaler(usize),
    /// `providers[i]`, a joint denoiser.
    Joint(usize),
}

/// The built-in denoiser's passes (live/layout.rs `PASSES`).
fn denoise_pass(pass: &str) -> bool {
    matches!(pass, "refl" | "dtemporal" | "stemporal") || pass.starts_with("atrous") || pass.starts_with("sfilt")
}

/// The built-in temporal upscaler's passes.
fn upscale_pass(pass: &str) -> bool {
    matches!(pass, "lock_carry" | "track" | "judge" | "reject" | "upscale")
}

impl Active {
    /// The trace leaves its raw signal (no demodulation, the whole specular path) for something other than
    /// ThreeRT's denoiser: `compose` writes it as it is (`L.mode.w`, the kernel's `P.motion.y`).
    pub(super) fn raw_signal(self) -> bool { matches!(self, Self::Off | Self::Joint(_)) }

    /// The kernel computes the joint denoiser's surface guides (pathtrace.wgsl `surface_guides`).
    pub(super) fn guides(self) -> bool { matches!(self, Self::Joint(_)) }

    /// Nothing of an earlier frame is used (the reset lane stays set: `L.size.w`).
    pub(super) fn history_free(self) -> bool { self == Self::Off }

    /// No sub-pixel jitter: nothing accumulates it.
    pub(super) fn unjittered(self) -> bool { matches!(self, Self::Off | Self::Fsr) }

    /// A provider takes the upscale stage's place (its passes do not run; it does, where `upscale` would).
    pub(super) fn external(self) -> bool { matches!(self, Self::Fsr | Self::Upscaler(_) | Self::Joint(_)) }

    pub(super) fn skips(self, pass: &str) -> bool {
        (self.external() && upscale_pass(pass)) || (self.raw_signal() && denoise_pass(pass))
    }
}

/// Why a method in the request did not run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// Not in this build or on this platform, or no such provider registered.
    Unavailable,
    /// The device, the OS or this size is outside what it supports.
    Unsupported,
    /// It failed (at prepare or encode).
    ProviderError,
    /// A view mode (base colour, normals, ...) is on: those are ThreeRT's own buffers.
    DiagnosticView,
}

impl Skip {
    pub fn name(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Unsupported => "unsupported",
            Self::ProviderError => "provider-error",
            Self::DiagnosticView => "diagnostic-view",
        }
    }
}

/// Which method ran, each stage's implementation, and why the methods before it did not.
#[derive(Clone, Debug, Default)]
pub struct ReconstructionStatus {
    pub requested: String,
    /// None before the first frame.
    pub active: Option<String>,
    /// "builtin", "none" or the joint denoiser's method.
    pub denoise: String,
    /// "builtin", "none" or the provider's method.
    pub upscale: String,
    /// The first skipped method's reason, unless the request was `auto`.
    pub fallback: Option<Skip>,
    /// Method, reason, detail: each method tried before the one that ran.
    pub skipped: Vec<(String, Skip, String)>,
}

impl ReconstructionStatus {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "requested": self.requested, "active": self.active, "denoise": self.denoise, "upscale": self.upscale,
            "fallback": self.fallback.map(Skip::name),
            "skipped": self.skipped.iter().map(|(m, s, d)| serde_json::json!({ "method": m, "reason": s.name(), "detail": d })).collect::<Vec<_>>(),
        })
    }
}

pub(super) struct Reconstructor {
    pub(super) requested: Reconstruction,
    pub(super) active: Active,
    key: Option<(ReconstructionSize, bool)>,
    skipped: Vec<(Method, Skip, String)>,
    /// The lowest render scale dynamic resolution may go to: the range of the method that runs, or of the first one
    /// asked for that did not run for the scale alone (the governor raises the scale until it does).
    scale_floor: Option<f32>,
    fsr: Option<fsr::Fsr>,
    providers: Vec<(Method, Provider)>,
    // Fields drop in declaration order: providers release GPU work before their input/output textures.
    textures: Option<ProviderTextures>,
    pipes: Option<ProviderPipelines>,
}

impl Default for Reconstructor {
    fn default() -> Self {
        #[allow(unused_mut)]
        let mut providers: Vec<(Method, Provider)> = vec![];
        #[cfg(metalfx)]
        providers.extend([
            (Method::MetalFx, Provider::Upscale(Box::new(super::metalfx::MetalFx::upscaler()))),
            (Method::MetalFxDenoised, Provider::Joint(Box::new(super::metalfx::MetalFx::denoiser()))),
        ]);
        #[cfg(dlss)]
        providers.extend([
            (Method::Dlss, Provider::Upscale(Box::new(super::dlss::Dlss::default()))),
            (Method::DlssRr, Provider::Joint(Box::new(super::dlss::DlssRr::default()))),
        ]);
        Self { requested: Reconstruction::default(), active: Active::Builtin, key: None, skipped: vec![], scale_floor: None, fsr: None, providers, textures: None, pipes: None }
    }
}

impl Reconstructor {
    /// A new request (the frame after resolves it). False if it is the current one.
    pub(super) fn set(&mut self, r: Reconstruction) -> bool {
        if self.requested == r { return false; }
        self.requested = r;
        self.key = None;
        true
    }

    fn register(&mut self, id: &str, provider: Provider) -> Result<()> {
        let m = Method::parse(id)?;
        anyhow::ensure!(matches!(m, Method::Custom(_)), "a provider's id is custom:<id>");
        anyhow::ensure!(!self.providers.iter().any(|(k, _)| *k == m), "a reconstruction provider '{id}' is registered already");
        self.key = None;
        self.providers.push((m, provider));
        Ok(())
    }

    pub(super) fn register_upscaler(&mut self, id: &str, p: Box<dyn UpscaleProvider>) -> Result<()> { self.register(id, Provider::Upscale(p)) }
    pub(super) fn register_joint(&mut self, id: &str, p: Box<dyn JointProvider>) -> Result<()> { self.register(id, Provider::Joint(p)) }

    pub(super) fn invalidate(&mut self) {
        self.key = None;
        self.fsr = None;
        self.textures = None;
        for (_, p) in &mut self.providers { p.invalidate(); }
    }

    pub(super) fn method(&self) -> Method {
        match self.active {
            Active::Builtin => Method::Builtin,
            Active::Off => Method::Off,
            Active::Fsr => Method::Fsr,
            Active::Upscaler(i) | Active::Joint(i) => self.providers[i].0.clone(),
        }
    }

    pub(super) fn status(&self) -> ReconstructionStatus {
        let m = self.method();
        let (denoise, upscale) = match self.active {
            Active::Builtin => ("builtin".into(), "builtin".into()),
            Active::Off => ("none".into(), "none".into()),
            Active::Fsr | Active::Upscaler(_) => ("builtin".into(), m.name().into()),
            Active::Joint(_) => (m.name().into(), m.name().into()),
        };
        ReconstructionStatus {
            requested: self.requested.name(),
            active: self.key.is_some().then(|| m.name().to_string()),
            denoise, upscale,
            fallback: (self.requested != Reconstruction::Auto).then(|| self.skipped.first().map(|s| s.1)).flatten(),
            skipped: self.skipped.iter().map(|(m, s, d)| (m.name().to_string(), *s, d.clone())).collect(),
        }
    }

    /// Resolve before tracing, so the trace's signals, denoising and jitter are this frame's. True if the
    /// configuration changed (history is reset).
    pub(super) fn prepare(&mut self, g: &gpu::Gpu, targets: &Targets, size: ReconstructionSize, diagnostic: bool) -> bool {
        if self.key == Some((size, diagnostic)) { return false; }
        self.key = Some((size, diagnostic));
        let old = self.active;
        self.skipped.clear();
        self.active = if diagnostic {
            if let Some(m) = self.requested.chain().into_iter().find(|m| *m != Method::Builtin) {
                self.skipped.push((m, Skip::DiagnosticView, "a view mode shows ThreeRT's own buffers".into()));
            }
            Active::Builtin
        } else {
            self.select(g, targets, size)
        };
        self.active != old
    }

    /// The lowest render scale the governor may go to for this reconstruction (None: its own floor).
    pub(super) fn scale_floor(&self) -> Option<f32> { self.scale_floor }

    fn select(&mut self, g: &gpu::Gpu, targets: &Targets, size: ReconstructionSize) -> Active {
        self.scale_floor = None;
        let range_of = |s: &Self, m: &Method| s.providers.iter().find(|(k, _)| k == m).and_then(|(_, p)| p.scale_range());
        let scale = size.input[0] as f32 / size.output[0].max(1) as f32;
        for m in self.requested.chain() {
            match self.try_method(g, targets, &m, size) {
                Ok(active) => {
                    if size.dynamic && self.scale_floor.is_none() {
                        self.scale_floor = range_of(self, &m).map(|r| r[0]);
                    }
                    return active;
                }
                Err((skip, detail)) => {
                    // (Below the range of a method asked for before the one that runs: the governor goes up to it.)
                    if size.dynamic && self.scale_floor.is_none() && skip == Skip::Unsupported {
                        self.scale_floor = range_of(self, &m).filter(|r| scale < r[0] && r[0] <= 1.0).map(|r| r[0]);
                    }
                    if skip == Skip::ProviderError { eprintln!("live: {} reconstruction failed: {detail}", m.name()); }
                    self.skipped.push((m, skip, detail));
                }
            }
        }
        Active::Builtin
    }

    fn try_method(&mut self, g: &gpu::Gpu, targets: &Targets, m: &Method, size: ReconstructionSize) -> std::result::Result<Active, (Skip, String)> {
        match m {
            Method::Builtin => return Ok(Active::Builtin),
            Method::Off => return Ok(Active::Off),
            _ => {}
        }
        let limit = g.device.limits().max_texture_dimension_2d;
        if size.output.into_iter().any(|side| side > limit) {
            return Err((Skip::Unsupported, format!("the view is larger than this device's {limit}-pixel textures")));
        }
        if *m == Method::Fsr {
            self.fsr.get_or_insert_with(|| fsr::Fsr::new(g, targets, size.output));
            return Ok(Active::Fsr);
        }
        let Some(i) = self.providers.iter().position(|(k, _)| k == m) else {
            return Err((Skip::Unavailable, match m {
                Method::MetalFx | Method::MetalFxDenoised => "MetalFX runs on macOS and iOS only".into(),
                Method::Dlss | Method::DlssRr => "this build has no DLSS (feature `dlss`, x86-64 Windows or Linux)".into(),
                _ => format!("no provider is registered as {}", m.name()),
            }));
        };
        match self.providers[i].1.prepare(g, size) {
            Ok(Prepared::Ready { capacity }) => {
                let joint = self.providers[i].1.joint();
                self.prepare_textures(g, targets, joint, size, capacity);
                Ok(if joint { Active::Joint(i) } else { Active::Upscaler(i) })
            }
            Ok(Prepared::Unsupported(why)) => Err((Skip::Unsupported, why)),
            Err(e) => Err((Skip::ProviderError, format!("{e:#}"))),
        }
    }

    fn prepare_textures(&mut self, g: &gpu::Gpu, targets: &Targets, joint: bool, size: ReconstructionSize, capacity: [u32; 2]) {
        let capacity = [capacity[0].max(size.input[0]), capacity[1].max(size.input[1])];
        if self.textures.as_ref().is_none_or(|t| t.key != (joint, capacity, size.output)) {
            let pipes = self.pipes.get_or_insert_with(|| ProviderPipelines::new(g));
            self.textures = Some(ProviderTextures::new(g, targets, pipes, joint, capacity, size.output));
        }
    }

    /// Display whites a unit of the active provider's input is (its `radiance_unit` = this / exposure); the shader's
    /// `rc_radiance_unit` takes the same (`L.mode.y`).
    pub(super) fn white_units(&self) -> f32 {
        match self.active {
            Active::Upscaler(i) | Active::Joint(i) => self.providers[i].1.white_units(),
            _ => 32.0,
        }
    }

    /// GPU time (ms) the active provider spends outside the frame timer's passes.
    pub(super) fn untimed_gpu_ms(&self) -> f32 {
        match self.active {
            Active::Upscaler(i) | Active::Joint(i) => self.providers[i].1.untimed_gpu_ms(),
            _ => 0.0,
        }
    }

    /// A provider failed while encoding: ThreeRT's own from this frame on (until the request or size changes).
    fn provider_error(&mut self, e: &anyhow::Error) {
        let m = self.method();
        eprintln!("live: {} reconstruction failed: {e:#}; using builtin", m.name());
        self.skipped.push((m, Skip::ProviderError, format!("{e:#}")));
        self.active = Active::Builtin;
    }

    /// In place of the upscale stage. Returns the command buffers to submit before the rest of the frame when the
    /// provider recorded native work.
    pub(super) fn encode(&mut self, g: &gpu::Gpu, e: &mut CommandEncoder, size: ReconstructionSize,
        cam: Camera, aspect: f32, jitter: (f32, f32), exposure: f32, reset: bool,
    ) -> Result<Option<[CommandBuffer; 2]>> {
        let i = match self.active {
            Active::Builtin | Active::Off => return Ok(None),
            Active::Fsr => { self.fsr.as_ref().unwrap().encode(e); return Ok(None); }
            Active::Upscaler(i) | Active::Joint(i) => i,
        };
        let textures = self.textures.as_mut().expect("prepared reconstruction textures");
        let pipes = self.pipes.as_ref().unwrap();
        textures.pack(e, pipes, size.input);
        let unit = self.providers[i].1.white_units() / exposure.clamp(0.00001, 64.0);
        let world_to_view = glam::Mat4::look_to_rh(cam.pos, cam.fwd, cam.up).to_cols_array();
        let view_to_clip = glam::Mat4::perspective_infinite_reverse_rh(2.0 * cam.tan_half_fov.atan(), aspect, 0.01).to_cols_array();
        let [color, depth, motion, reactive, ..] = &textures.inputs[..] else { unreachable!() };
        let frame = ReconstructionFrame {
            gpu: g, size, color, depth, motion, reactive, output: &textures.output, radiance_unit: unit,
            jitter: [jitter.0 - 0.5, jitter.1 - 0.5], reset: reset || textures.fresh || exposure_jumped(textures.exposure, exposure),
            camera: cam, aspect, world_to_view, view_to_clip,
        };
        let dispatch = match &mut self.providers[i].1 {
            Provider::Upscale(p) => p.encode(frame, e),
            Provider::Joint(p) => {
                let [_, _, _, _, overlay, bypass, diffuse_albedo, specular_albedo, normal, roughness, specular_hit_distance] = &textures.inputs[..] else { unreachable!() };
                p.encode(frame, JointGuides { overlay, bypass, diffuse_albedo, specular_albedo, normal, roughness, specular_hit_distance }, e)
            }
        };
        let native = match dispatch {
            Ok(cb) => cb,
            Err(err) => { self.provider_error(&err); return Err(err); }
        };
        let before_post = native.map(|native| [std::mem::replace(e, g.device.create_command_encoder(&Default::default())).finish(), native]);
        textures.fresh = false;
        if textures.exposure == 0.0 || exposure_jumped(textures.exposure, exposure) {
            textures.exposure = exposure;
        }
        let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("reconstruction to post"), ..Default::default() });
        pass.set_pipeline(&pipes.output);
        pass.set_bind_group(0, &textures.post_group, &[]);
        pass.set_bind_group(1, &textures.output_group, &[]);
        pass.dispatch_workgroups(size.output[0].div_ceil(8), size.output[1].div_ceil(8), 1);
        Ok(before_post)
    }
}

/// Whether the exposure moved far enough from the one the provider's history started at (`textures.exposure`) for
/// that history to be dropped: more than `EXPOSURE_RESET` stops. The inputs are in units of display white
/// (`radiance_unit` = white / exposure), so the history is in the units of the exposure it was made at; a drift of a
/// fraction of a stop (auto exposure adapting, as it does every frame in a dim room) only scales it a little, which
/// the provider's own blend takes up within a few frames. Resetting on every change, as before, threw the history
/// away every frame under auto exposure: MetalFX-denoised then showed each frame's raw samples (EarthRT's machiya, a
/// room lit through a garden opening: sparkle everywhere after 200 frames).
fn exposure_jumped(from: f32, to: f32) -> bool {
    from <= 0.0 || to <= 0.0 || (to / from).log2().abs() > EXPOSURE_RESET
}

/// Stops of exposure change past which a provider's history is dropped (`exposure_jumped`).
const EXPOSURE_RESET: f32 = 1.0;

use wgpu::TextureFormat::*;
/// [`ReconstructionFrame`]'s inputs, then [`JointGuides`]' (shaders/reconstruction.wgsl `rc_*_inputs`, `rc_guides`).
const SIGNALS: [wgpu::TextureFormat; 4] = [Rgba16Float, R32Float, Rg16Float, R8Unorm];
const JOINT_LAYERS: [wgpu::TextureFormat; 2] = [Rgba16Float, R8Unorm];
const GUIDES: [wgpu::TextureFormat; 5] = [Rgba8Unorm, Rgba8Unorm, Rgba16Float, R16Float, R16Float];

struct ProviderPipelines {
    upscale_inputs: wgpu::RenderPipeline,
    joint_inputs: wgpu::RenderPipeline,
    guides: wgpu::RenderPipeline,
    output: wgpu::ComputePipeline,
}

impl ProviderPipelines {
    fn new(g: &gpu::Gpu) -> Self {
        let d = &g.device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reconstruction signals"),
            source: wgpu::ShaderSource::Wgsl([targets::live_source(), include_str!("../../shaders/reconstruction.wgsl")].concat().into()),
        });
        let render = |entry: &str, formats: &[wgpu::TextureFormat]| {
            let targets: Vec<_> = formats.iter().map(|f| Some((*f).into())).collect();
            d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry), layout: None,
                vertex: wgpu::VertexState { module: &module, entry_point: Some("rc_vertex"), buffers: &[], compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(entry), compilation_options: Default::default(), targets: &targets }),
                primitive: Default::default(), depth_stencil: None, multisample: Default::default(), multiview_mask: None, cache: g.cache(),
            })
        };
        let output = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("reconstruction output"), layout: None, module: &module,
            entry_point: Some("rc_output_to_post"), compilation_options: Default::default(), cache: g.cache(),
        });
        Self {
            upscale_inputs: render("rc_upscale_inputs", &SIGNALS),
            joint_inputs: render("rc_joint_inputs", &[&SIGNALS[..], &JOINT_LAYERS].concat()),
            guides: render("rc_guides", &GUIDES),
            output,
        }
    }
}

struct ProviderTextures {
    /// Joint, input capacity, output size.
    key: (bool, [u32; 2], [u32; 2]),
    /// SIGNALS, then (joint) JOINT_LAYERS and GUIDES.
    inputs: Vec<TextureView>,
    output: TextureView,
    input_group: wgpu::BindGroup,
    guide_group: Option<wgpu::BindGroup>,
    post_group: wgpu::BindGroup,
    output_group: wgpu::BindGroup,
    fresh: bool,
    exposure: f32,
}

impl ProviderTextures {
    fn new(g: &gpu::Gpu, targets: &Targets, pipes: &ProviderPipelines, joint: bool, capacity: [u32; 2], output_size: [u32; 2]) -> Self {
        use wgpu::TextureUsages as Usage;
        let d = &g.device;
        let view = |label, format, [width, height]: [u32; 2], extra| d.create_texture(&wgpu::TextureDescriptor {
            label: Some(label), format,
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2,
            usage: Usage::RENDER_ATTACHMENT | Usage::TEXTURE_BINDING | extra, view_formats: &[],
        }).create_view(&Default::default());
        let formats: Vec<_> = if joint { [&SIGNALS[..], &JOINT_LAYERS, &GUIDES].concat() } else { SIGNALS.to_vec() };
        let inputs = formats.into_iter().map(|f| view("reconstruction input", f, capacity, Usage::empty())).collect();
        let output = view("reconstruction HDR", Rgba16Float, output_size, Usage::STORAGE_BINDING | Usage::COPY_DST);
        let bind = |layout: wgpu::BindGroupLayout, names: &[&str]| {
            let entries: Vec<_> = names.iter().map(|&n| wgpu::BindGroupEntry {
                binding: super::layout::binding(n).binding,
                resource: match n {
                    "L" => targets.params.as_entire_binding(),
                    "sample_px" => targets.accum.as_entire_binding(),
                    _ => targets.buf(n).binding(),
                },
            }).collect();
            d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("reconstruction signals"), layout: &layout, entries: &entries })
        };
        let input_group = if joint {
            bind(pipes.joint_inputs.get_bind_group_layout(0), &["L", "fb", "sample_px", "sample_vel"])
        } else {
            bind(pipes.upscale_inputs.get_bind_group_layout(0), &["L", "fb", "sample_vel"])
        };
        let guide_group = joint.then(|| bind(pipes.guides.get_bind_group_layout(0), &["L", "sample_px", "refl_in"]));
        let post_group = bind(pipes.output.get_bind_group_layout(0), &["L", "fa", "sb", "sample_vel"]);
        let output_group = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("reconstruction HDR"), layout: &pipes.output.get_bind_group_layout(1),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&output) }],
        });
        Self { key: (joint, capacity, output_size), inputs, output, input_group, guide_group, post_group, output_group, fresh: true, exposure: 0.0 }
    }

    /// The signals, into the top-left `input` of the textures.
    fn pack(&self, e: &mut CommandEncoder, pipes: &ProviderPipelines, input: [u32; 2]) {
        fn attachment(view: &TextureView) -> Option<wgpu::RenderPassColorAttachment<'_>> {
            Some(wgpu::RenderPassColorAttachment {
                view, depth_slice: None, resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })
        }
        if self.fresh {
            // Initialize before a native writer, so wgpu doesn't clear its output on the first read.
            drop(e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reconstruction output init"), color_attachments: &[attachment(&self.output)], ..Default::default()
            }));
        }
        let mut draw = |views: &[TextureView], pipe: &wgpu::RenderPipeline, group: &wgpu::BindGroup| {
            let attachments: Vec<_> = views.iter().map(attachment).collect();
            let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("reconstruction inputs"), color_attachments: &attachments, ..Default::default()
            });
            pass.set_scissor_rect(0, 0, input[0], input[1]);
            pass.set_viewport(0.0, 0.0, input[0] as f32, input[1] as f32, 0.0, 1.0);
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        };
        match &self.guide_group {
            Some(guides) => {
                let (base, rest) = self.inputs.split_at(SIGNALS.len() + JOINT_LAYERS.len());
                draw(base, &pipes.joint_inputs, &self.input_group);
                draw(rest, &pipes.guides, guides);
            }
            None => draw(&self.inputs, &pipes.upscale_inputs, &self.input_group),
        }
    }
}
