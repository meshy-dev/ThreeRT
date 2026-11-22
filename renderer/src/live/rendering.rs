//! The live view's rendering settings, by the gap each one closes between what a frame can afford and what is shown:
//!
//! | setting | the gap | ThreeRT's own | vendor technology |
//! |---|---|---|---|
//! | `quality` | how many rays: the render resolution (the input every stage below works from) | dynamic resolution | (none: a budget, not a technique) |
//! | `reconstruction` | noise and pixels (live/reconstruction.rs) | denoiser + temporal upscaler | DLSS SR / RR, MetalFX, FSR, XeSS |
//! | `frameGeneration` | frames: more shown than rendered | none | DLSS FG, FSR FG, XeSS FG, MetalFX frame interpolation |
//! | `latency` | input to photon | at most one frame queued | Reflex, Anti-Lag 2, XeLL |
//!
//! Each is a request ("what I want"), resolved each frame against the device to what runs, and read back as a
//! status: a request this device cannot run is not an error, and is never silent. Neural techniques are not a
//! setting of their own: a neural denoiser or upscaler is a reconstruction method; a neural radiance cache would be
//! a lighting option and neural texture compression an asset format.
//!
//! These are device-local view settings (a player's graphics options, a host's default), not scene state: a game
//! may request them (`rendering.set` in @threert/scene), the viewer's flags and keys override them.
use super::{Method, Reconstruction, ReconstructionStatus, Result};
use std::sync::{Arc, Mutex};

/// The render resolution, as a fraction of the output's side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Quality {
    /// Dynamic resolution: the governor keeps the frame within its time.
    Auto,
    /// 1: every output pixel traced (reconstruction is anti-aliasing: DLAA, MetalFX at 1x).
    Native,
    /// 2/3.
    Quality,
    /// 0.58.
    Balanced,
    /// 1/2.
    Performance,
    /// 1/3.
    UltraPerformance,
    /// A fraction of its own (0.25 ..= 1).
    Scale(f32),
}

impl Quality {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim() {
            "auto" | "dynamic" => Self::Auto,
            "native" => Self::Native,
            "quality" => Self::Quality,
            "balanced" => Self::Balanced,
            "performance" => Self::Performance,
            "ultra-performance" => Self::UltraPerformance,
            s => match s.parse::<f32>() {
                Ok(v) if v > 0.0 && v <= 1.0 => Self::Scale(v),
                Ok(v) if v == 0.0 => Self::Auto,
                _ => anyhow::bail!("unknown quality '{s}'; use auto, native, quality, balanced, performance, ultra-performance or a scale in (0, 1]"),
            },
        })
    }

    pub fn from_json(v: &serde_json::Value) -> Result<Self> {
        match v {
            serde_json::Value::String(s) => Self::parse(s),
            serde_json::Value::Number(n) => Self::parse(&n.to_string()),
            _ => anyhow::bail!("quality is a name or a scale"),
        }
    }

    pub fn name(&self) -> String {
        match self {
            Self::Auto => "auto".into(),
            Self::Native => "native".into(),
            Self::Quality => "quality".into(),
            Self::Balanced => "balanced".into(),
            Self::Performance => "performance".into(),
            Self::UltraPerformance => "ultra-performance".into(),
            Self::Scale(s) => format!("{s}"),
        }
    }

    /// The render scale, or None for dynamic resolution. (The presets are the vendors' ratios exactly: a third of
    /// 1920 is 640, the smallest input DLSS takes for 1080p, where a typed 0.333 gives 639.)
    pub fn scale(&self) -> Option<f32> {
        match self {
            Self::Auto => None,
            Self::Native => Some(1.0),
            Self::Quality => Some(2.0 / 3.0),
            Self::Balanced => Some(0.58),
            Self::Performance => Some(0.5),
            Self::UltraPerformance => Some(1.0 / 3.0),
            Self::Scale(s) => Some(*s),
        }
    }
}

/// Frames shown per frame rendered. No provider is integrated yet: a request other than `off` reports itself
/// unavailable and the view shows each rendered frame once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameGeneration {
    Off,
    /// What the device does best (nothing yet).
    Auto,
    /// 2 to 4 frames shown per frame rendered.
    Multiply(u8),
}

impl FrameGeneration {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim() {
            "off" => Self::Off,
            "auto" => Self::Auto,
            "2x" => Self::Multiply(2),
            "3x" => Self::Multiply(3),
            "4x" => Self::Multiply(4),
            s => anyhow::bail!("unknown frame generation '{s}'; use off, auto, 2x, 3x or 4x"),
        })
    }

    pub fn name(&self) -> String {
        match self { Self::Off => "off".into(), Self::Auto => "auto".into(), Self::Multiply(n) => format!("{n}x") }
    }
}

/// How far the CPU may run ahead of the GPU.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Latency {
    /// Two frames queued (throughput first).
    Off,
    /// `on` while frames are generated (a generated frame needs it), else `off`.
    Auto,
    /// One frame queued: the next frame's input is read as late as the GPU allows.
    On,
}

impl Latency {
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s.trim() {
            "off" => Self::Off,
            "auto" => Self::Auto,
            "on" => Self::On,
            s => anyhow::bail!("unknown latency '{s}'; use off, auto or on"),
        })
    }

    pub fn name(&self) -> &'static str {
        match self { Self::Off => "off", Self::Auto => "auto", Self::On => "on" }
    }
}

/// A request: each field set is changed, each left out kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderingRequest {
    pub quality: Option<Quality>,
    pub reconstruction: Option<Reconstruction>,
    pub frame_generation: Option<FrameGeneration>,
    pub latency: Option<Latency>,
}

impl RenderingRequest {
    /// `{"quality": "performance", "reconstruction": ["metalfx-denoised", "builtin"], "frameGeneration": "off",
    /// "latency": "auto"}`, any of them.
    pub fn from_json(v: &serde_json::Value) -> Result<Self> {
        let o = v.as_object().ok_or_else(|| anyhow::anyhow!("rendering settings are an object"))?;
        let str_of = |k: &str| o.get(k).map(|v| v.as_str().ok_or_else(|| anyhow::anyhow!("{k} is a string")));
        for k in o.keys() {
            anyhow::ensure!(matches!(k.as_str(), "quality" | "reconstruction" | "frameGeneration" | "latency" | "cmd"),
                "unknown rendering setting '{k}'; use quality, reconstruction, frameGeneration or latency");
        }
        Ok(Self {
            quality: o.get("quality").map(Quality::from_json).transpose()?,
            reconstruction: o.get("reconstruction").map(Reconstruction::from_json).transpose()?,
            frame_generation: str_of("frameGeneration").transpose()?.map(FrameGeneration::parse).transpose()?,
            latency: str_of("latency").transpose()?.map(Latency::parse).transpose()?,
        })
    }

    fn merge(&mut self, r: RenderingRequest) {
        if r.quality.is_some() { self.quality = r.quality; }
        if r.reconstruction.is_some() { self.reconstruction = r.reconstruction; }
        if r.frame_generation.is_some() { self.frame_generation = r.frame_generation; }
        if r.latency.is_some() { self.latency = r.latency; }
    }
}

/// The settings as requested and as they ran (the last frame).
#[derive(Clone, Debug)]
pub struct RenderingStatus {
    pub quality: Quality,
    /// The render scale the last frame ran at, and whether the governor sets it.
    pub scale: f32,
    pub dynamic: bool,
    pub reconstruction: ReconstructionStatus,
    pub frame_generation: FrameGeneration,
    pub latency: Latency,
    pub frames_in_flight: u32,
}

impl RenderingStatus {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "quality": { "requested": self.quality.name(), "scale": self.scale, "dynamic": self.dynamic },
            "reconstruction": self.reconstruction.json(),
            "frameGeneration": {
                "requested": self.frame_generation.name(), "active": "off",
                "fallback": matches!(self.frame_generation, FrameGeneration::Multiply(_)).then_some("unavailable"),
            },
            "latency": {
                "requested": self.latency.name(), "active": if self.frames_in_flight == 1 { "on" } else { "off" },
                "framesInFlight": self.frames_in_flight,
            },
        })
    }
}

#[derive(Default)]
struct ControlState {
    pending: Option<RenderingRequest>,
    status: Option<RenderingStatus>,
}

/// Settings shared with a game runtime or a browser page: requests queue for the next frame, the status is the last
/// frame's. Requests made before the view's first frame apply to it.
#[derive(Clone, Default)]
pub struct RenderingControl(Arc<Mutex<ControlState>>);

impl RenderingControl {
    pub fn request(&self, r: RenderingRequest) {
        let mut s = self.0.lock().unwrap();
        s.pending.get_or_insert_with(Default::default).merge(r);
    }

    pub(super) fn take(&self) -> Option<RenderingRequest> { self.0.lock().unwrap().pending.take() }

    pub(super) fn publish(&self, status: RenderingStatus) { self.0.lock().unwrap().status = Some(status); }

    /// The last frame's settings, with requests not yet applied shown as requested. Null before the first frame.
    pub fn status(&self) -> serde_json::Value {
        let s = self.0.lock().unwrap();
        let Some(mut st) = s.status.clone() else { return serde_json::Value::Null };
        if let Some(p) = &s.pending {
            if let Some(q) = p.quality { st.quality = q; }
            if let Some(r) = &p.reconstruction { st.reconstruction.requested = r.name(); }
            if let Some(f) = p.frame_generation { st.frame_generation = f; }
            if let Some(l) = p.latency { st.latency = l; }
        }
        st.json()
    }
}

/// The viewer's key: the next of this platform's methods after `m`.
pub fn next_method(m: &Method) -> Method {
    let all = Method::platform();
    let i = all.iter().position(|k| k == m).map_or(0, |i| (i + 1) % all.len());
    all[i].clone()
}

impl RenderingRequest {
    /// `--quality`, `--reconstruction`, `--frame-generation`, `--latency` (`flag` returns a flag's value).
    pub fn from_flags(flag: impl Fn(&str) -> Option<String>) -> Result<Self> {
        Ok(Self {
            quality: flag("--quality").map(|v| Quality::parse(&v)).transpose()?,
            reconstruction: flag("--reconstruction").map(|v| Reconstruction::parse(&v)).transpose()?,
            frame_generation: flag("--frame-generation").map(|v| FrameGeneration::parse(&v)).transpose()?,
            latency: flag("--latency").map(|v| Latency::parse(&v)).transpose()?,
        })
    }
}
