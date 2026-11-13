//! Real-time renderer (LiveCore), modelled on UE5's real-time path with hardware RT: every frame traces 1+ spp
//! (primary visibility, GI, shadows, reflections), reprojects and accumulates history (converging to the offline
//! reference while the camera holds still), denoises and applies the UE post chain. Drawn by the native window viewer
//! (`view`) and the playground client; `live` runs it headless at a fixed frame rate for benchmarks:
//!
//!   threert-render live <game bundle.js> [--assets DIR] [--room id] [--width 1280 --height 720 --fps 60 --bounces 4
//!                       --scale 1|0.6|auto --display-scale 1|2|3] [--frame-log frames.jsonl]
//!
//! `--scale` is the render scale (UE screen percentage): the frame is traced, denoised and composed at that fraction
//! of the output resolution and the temporal upscaler takes it to the output; `auto` = dynamic resolution.
//! `--display-scale`: the output is a display's physical pixels, that many a point (a Retina window: 2), as the
//! window host's is; dynamic resolution then starts from the points' resolution and goes down to half of it.
//!
//! stdin (JSON lines): {"cmd":"camera","pos":[x,y,z],"target":[x,y,z]} | {"cmd":"event","type":t,"payload":{..}}
//!                     | {"cmd":"click","u":u,"v":v} | {"cmd":"reset"} | {"cmd":"time","t":s} (set the scene time)
//!                     | {"cmd":"snapshot","path":p,"after":k (history length ≥ k),"frames":n (the n-th frame from now, default 1)}
//!                       (a path ending in .raw: the linear radiance, RGB f32 at output size; ending in .signals:
//!                       its diffuse / specular / emission apart, <stem>-{diffuse,specular,emission}.raw, as
//!                       `render --signals` writes them)
//!                     | {"cmd":"sync","id":s,"frames":n} (stderr "live: sync s" after the n-th frame from now)
//!                     | {"cmd":"lockstep","dt":s | "on":false} (render only the frames steps ask for, at frame time s)
//!                     | {"cmd":"camera",...,"frames":n,"sync":id | "snapshot":p} | {"cmd":"frames","n":n,"sync":id | "snapshot":p}
//!                       (a step: camera, frames, and the sync answer / snapshot at its last frame, in one command)
//!                     | {"cmd":"scale","scale":s (0 = auto)} | {"cmd":"quit"}
//!
//! The files: this one holds `LiveCore` itself (its settings, camera, resolution governor and readbacks) and what the
//! others share; `targets` the live passes' pipelines, screen-size buffers and bind group; `load` a scene loaded and
//! the workers of its sky and emitters; `edits` the world's edits applied in place; `frame` one frame (the trace, the
//! live passes, the caches' training, the GPU timing); `session` the headless runner (native only).

use crate::gpu::Buf;
use crate::world::World;
use crate::{gpu, post, scene, tables};
// (The headless runner, `session`, native only, is what uses all of it.)
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use anyhow::{anyhow, Result};
use glam::Vec3;
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use std::time::Duration;
use web_time::Instant;

mod edits;
mod frame;
mod layout;
mod load;
#[cfg(not(target_arch = "wasm32"))]
mod session;
mod targets;
#[cfg(test)]
pub(crate) use targets::live_source;
pub(crate) use targets::Targets;
#[cfg(not(target_arch = "wasm32"))]
pub use session::{parse_cmd, parse_launch, parse_opts, run, save_bgra};
use edits::*;
use frame::*;
use load::*;
use targets::*;

pub struct LiveOpts {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bounces: u32,
    /// Render scale (traced / output resolution); None = dynamic resolution.
    pub scale: Option<f32>,
    /// Game bundle (game code + runtime) that builds and drives the scene in-process.
    pub bundle: PathBuf,
    /// Asset root its model / texture paths resolve against.
    pub assets: PathBuf,
    pub room: String,
    /// JSON line per frame (wall interval, CPU work, GPU time, scale, history): hitch statistics (tools/bench-hitches.ts).
    /// `interval` runs from the end of one frame's work to the next's, so it carries the variation of the work itself;
    /// `start` runs from one frame's start to the next's, which the pacer sets: the frame rate a player sees.
    pub frame_log: Option<PathBuf>,
    /// The output's pixels a point of its display (1: the output is in points).
    pub display_scale: f32,
    /// `--launch`: the `params` of the host channel's `launch` message (docs/RUNTIME.md "The host channel").
    pub launch: Option<serde_json::Value>,
}

/// Lowest dynamic-resolution scale (UE r.DynamicRes.MinScreenPercentage default) of a view whose pixels are the
/// display's points; a host whose view is in the display's physical pixels divides it by the display's scale factor
/// (`LiveCore::min_scale`), so the fewest rays a window traces do not depend on the display's density.
pub const MIN_SCALE: f32 = 0.5;
/// The lowest render scale any view takes (a view of 3 pixels a point at `MIN_SCALE`, rounded down).
pub const SCALE_FLOOR: f32 = 0.125;

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct CamU {
    pos: [f32; 4],
    fwd: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct LiveParams {
    cur: CamU,
    prev: CamU,
    size: [u32; 4],
    misc: [f32; 4],
    misc2: [f32; 4],
    mode: [u32; 4],
    post: [f32; 4],
    /// Auto exposure (UE histogram): low / high fraction, histogram log2 min / max.
    ae: [f32; 4],
    /// Compensation (2^bias), min / max average luminance, mode (0 manual, 1 auto).
    ae2: [f32; 4],
    /// Speed up / down (EV/s), frame time (s), snap to target (1).
    ae3: [f32; 4],
    /// Local exposure: highlight contrast, shadow contrast, blurred-luminance blend, detail strength.
    lx: [f32; 4],
    /// Traced (internal) width, height.
    isz: [u32; 4],
    /// Bloom stage radii (texels of each stage's level) and tints, stages 0..3 then 4, 5.
    br: [f32; 4],
    br2: [f32; 4],
    bt: [f32; 4],
    bt2: [f32; 4],
    /// Depth of field: CoC radius of infinity (output px; 0 = off), focus distance (m), max radius (px), -.
    dof: [f32; 4],
    /// This frame's sub-pixel jitter (every camera ray's: x, y in 0 .. 1 of a traced pixel), -, -.
    jit: [f32; 4],
    /// Which half of the lasting buffers is this frame's (live.wgsl `this_half`; the other the previous frame's), -.
    state: [u32; 4],
}

/// Auto / local exposure settings of the loaded scene (UE PostProcessVolume semantics).
#[derive(Clone, Copy, Default)]
struct ExposureSettings {
    ae: [f32; 4],
    ae2: [f32; 4],
    speed: [f32; 2],
    lx: [f32; 4],
}

impl ExposureSettings {
    fn new(exp: &crate::desc::Exposure, pp: &crate::desc::PostPreset) -> Self {
        let a = &pp.auto_exposure;
        let le = &pp.local_exposure;
        let (ae2, comp) = match exp {
            crate::desc::Exposure::Manual { ev100 } => ([2f32.powf(-ev100), 0.0, 0.0, 0.0], 2f32.powf(-ev100)),
            crate::desc::Exposure::Auto { compensation, min_ev100, max_ev100 } => {
                let c = 2f32.powf(*compensation);
                ([c, 0.18 * 2f32.powf(*min_ev100), 0.18 * 2f32.powf(*max_ev100), 1.0], c)
            }
        };
        let _ = comp;
        ExposureSettings {
            ae: [a.low_percent.clamp(1.0, 99.0) / 100.0, a.high_percent.clamp(1.0, 99.0) / 100.0, a.histogram_log_min, a.histogram_log_max.max(a.histogram_log_min + 1.0)],
            ae2,
            speed: [a.speed_up.max(0.01), a.speed_down.max(0.01)],
            lx: [le.highlight_contrast, le.shadow_contrast, le.blurred_luminance_blend, le.detail_strength],
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct Camera {
    pub pos: Vec3,
    pub fwd: Vec3,
    pub up: Vec3,
    pub right: Vec3,
    pub tan_half_fov: f32,
}

impl Camera {
    pub fn look_at(pos: Vec3, target: Vec3, tan_half_fov: f32) -> Camera {
        let fwd = (target - pos).normalize();
        let right = fwd.cross(Vec3::Y).normalize();
        Camera { pos, fwd, up: right.cross(fwd), right, tan_half_fov }
    }
    fn uniform(&self, aspect: f32) -> CamU {
        CamU {
            pos: [self.pos.x, self.pos.y, self.pos.z, self.tan_half_fov],
            fwd: [self.fwd.x, self.fwd.y, self.fwd.z, aspect],
            right: [self.right.x, self.right.y, self.right.z, 0.0],
            up: [self.up.x, self.up.y, self.up.z, 0.0],
        }
    }
}

pub enum Cmd {
    Camera(Camera),
    /// An event for the in-process game (type, payload).
    Event(String, serde_json::Value),
    /// A click at (u, v) for the in-process game (picks the node under it).
    Click(f32, f32),
    /// Messages of the host channel for the in-process game (`{"cmd":"host","messages":[{"t":...}, ...]}`: what an
    /// app would send, its input among them; docs/RUNTIME.md "The host channel").
    Host(Vec<serde_json::Value>),
    /// Snapshot (path) of the first frame whose history length reaches `after` (None = any) and that is at least the
    /// `frames`-th frame from now (None = the next).
    Snapshot(PathBuf, Option<u32>, Option<u32>),
    /// Answer `live: sync <id>` on stderr once the n-th frame from now is rendered (benchmarks step the camera frame by
    /// frame, so what they measure does not depend on how many frames a busy machine renders per second).
    Sync(String, u32),
    /// Lockstep (benchmarks): with Some(frame time, s) the runner renders only the frames `Step`s ask for, at that frame
    /// time (motion blur, exposure, frame script); None = real time again.
    Lockstep(Option<f32>),
    /// A step: an optional camera, `frames` frames to render (lockstep), and a sync answer / snapshot at the last of them,
    /// registered with the camera in one command so neither can land a frame late.
    Step { cam: Option<Camera>, frames: u32, sync: Option<String>, snapshot: Option<PathBuf> },
    /// Drop all accumulated history (benchmarks measure convergence from here).
    Reset,
    /// Set the scene time, s (benchmarks: with lockstep, frame k from here renders at t + k x its frame time; frame
    /// time 0 holds it there).
    Time(f32),
    /// View mode (see LiveCore::view_mode).
    Mode(u32),
    Bounces(u32),
    /// Max samples per frame (adaptive below it).
    SppCap(u32),
    /// Viewer status text (e.g. the game score), shown in the window title.
    Status(String),
    /// Render scale (0 = dynamic resolution).
    Scale(f32),
    Quit,
}

/// The real-time renderer: scene + screen targets + camera/history state. Records one frame at a time; the output
/// (sRGB BGRA8 per pixel) lands in `targets.out`. Used by the headless runner, the native window viewer and the playground.
pub struct LiveCore {
    /// The game's particle spawns since the last frame (`World::particle_spawns`), for the GPU particle pass.
    particle_spawns: std::collections::BTreeMap<u32, Vec<f32>>,
    /// The sources the game gave its density grids since the last frame (`World::grid_sources`), for the grid pass.
    grid_sources: std::collections::BTreeMap<u32, Vec<f32>>,
    pub g: gpu::Gpu,
    passes: Passes,
    pub targets: Targets,
    sc: LoadedScene,
    presets: PathBuf,
    /// Asset root (models, file textures).
    assets: PathBuf,
    pub w: u32,
    pub h: u32,
    /// Committed state sequence number the image shows (set by the game session).
    pub seq: i64,
    aspect: f32,
    pub cam: Camera,
    prev_cam: Camera,
    pub frame: u32,
    pub history_len: f32,
    reset: bool,
    /// Bloom stage radii / tints (`bloom_stages`).
    bloom: ([f32; 6], [f32; 6]),
    vignette: f32,
    /// Chromatic aberration, film grain, bloom threshold, longest motion blur (output pixels).
    post: [f32; 4],
    /// Motion blur (UE MotionBlurAmount, MotionBlurTargetFPS; amount 0 = off).
    blur: (f32, f32),
    /// Last submission of each frame still on the GPU (at most two frames in flight).
    inflight: std::collections::VecDeque<wgpu::SubmissionIndex>,
    pub gpu_ms: f32,
    /// The frame `gpu_ms` timed (it lags by the frames in flight).
    gpu_frame: u32,
    /// That frame's trace pass alone (the path-tracing kernel: intersection and shading), ms; 0 without timestamps.
    /// Frame log `trace`: the per-frame cost a geometry representation changes, without THREERT_PROFILE's stalls.
    pub gpu_trace_ms: f32,
    /// This frame's geometry edits applied in place (`edit_geometry`: CPU ms, its BLAS / TLAS builds submitted), and
    /// scene rebuilds (ms) and, when edits fell back to one, why: summed over the frame's commits (a game event's and the
    /// frame script's), reset when the frame is logged. Fallbacks since the viewport opened.
    /// Frame log `edit`, `rebuild`, `fallback`, `fallbacks`.
    pub edit_ms: f32,
    /// CPU time of this frame's in-place instance updates (`update_instanced`), ms.
    pub instanced_ms: f32,
    pub edit_split: EditSplit,
    pub rebuild_ms: f32,
    pub fallback: Option<String>,
    pub fallbacks: u32,
    /// Path-tracing bounces for real-time frames (the offline preset uses more).
    pub bounces: u32,
    /// Ray tracing (true). Off (`set_rays`), the kernel traces the camera ray of each pixel and no other: no shadows,
    /// no bounce light, reflections of the sky only; analytic lights and emissive surfaces light unoccluded. The
    /// fallback for GPUs that trace too slowly (no hardware ray queries), and the "what tracing adds" comparison on
    /// those that do: the same scene, materials, exposure and post either way.
    rays: bool,
    /// 0 lit, 1 base color, 2 world normal, 3 lit without denoiser, 4 raw sample, 5 history length, 6 diffuse,
    /// 7 specular, 8 deterministic direct light (THREERT_VIEW_MODE at start: e.g. bench-temporal per signal).
    pub view_mode: u32,
    /// Lockstep frame time (s; benchmarks): motion blur and exposure adaptation use it instead of the wall clock.
    pub fixed_dt: Option<f32>,
    /// A/B switch (playground key N): motion blur off whatever the scene's post settings say.
    pub motion_blur_off: bool,
    pub triangles: usize,
    pub scene_name: String,
    /// Scene time, s: what `advance_time` added up, the dt each frame script gets, so it is the game's clock (`f.time`)
    /// and a burst fired at a game time starts then; lockstep runs repeat exactly. Animated shading, the particle pass
    /// and power edits' time stamps use it (`shader_time`).
    time: f32,
    /// Time between rendered frames (auto exposure adaptation), s.
    frame_dt: f32,
    last_frame: Instant,
    /// The post preset the LUT / bloom / grain settings were built from (scenes may override it).
    post_key: String,
    /// Traced / output resolution (UE screen percentage); the temporal pass upsamples below 1.
    pub render_scale: f32,
    /// Adjust `render_scale` to the frame budget (`govern_resolution`).
    pub dynamic_res: bool,
    /// Dynamic resolution's lowest scale: `MIN_SCALE` over the display's scale factor.
    pub min_scale: f32,
    /// The view's pixels a point of its display (`set_display_scale`): dynamic resolution's steps are a point's.
    display_scale: f32,
    /// Dynamic resolution: consecutive measured frames well over the frame time, and the first frame at the current
    /// scale.
    over_budget: (u32, u32),
    /// When this view was made, and when its render scale was last checked (`govern_resolution` by the clock).
    born: Instant,
    governed: Instant,
    /// THREERT_PROFILE=1: GPU time per pass (timestamp queries), printed every 2 s.
    profile: Option<Profile>,
    /// GPU time per frame without stalls (timestamp queries; None: measured by draining the queue twice a second).
    timer: Option<FrameTimer>,
    /// Rasterized primary visibility (None: the kernel traces camera rays).
    raster: Option<Raster>,
    /// World-space radiance cache (pathtrace.wgsl; None: THREERT_CACHE=0, paths trace every bounce).
    cache: Option<RadianceCache>,
    /// Kernel group 1: primary visibility + radiance cache + what the live passes read of a trace.
    kernel_bg: wgpu::BindGroup,
    /// Span (relative to the dynamic tail) re-transformed by the last in-place update whose previous-frame vertices
    /// (`Uploaded::prev_verts`) still hold the frame before it: synced to the current ones next frame.
    prev_span: Option<(usize, usize)>,
    /// An in-place update happened this frame (it keeps `prev_verts` itself).
    reposed: bool,
    /// Dynamic triangles moved in this commit: the light tree's moving subtree is refit once at its end.
    emitters_moved: bool,
    /// Frames since the last reset or camera cut (`COLD_FALLBACK_FRAMES`).
    cold_frames: u32,
    /// Likewise for the deforming meshes of shared poses (`repose_groups`): the span they took (relative to the dynamic
    /// tail, past its end) and whether this frame re-skinned any.
    pose_span: Option<(usize, usize)>,
    pose_reposed: bool,
    /// Movable instances moved by the last update (their records' previous-frame rows differ), and whether it was this
    /// frame's.
    inst_moving: Vec<u32>,
    inst_moved: bool,
    /// The previous frame's traced size (its traced-grid diffuse history; 0 before the first frame).
    prev_isz: (u32, u32),
    /// Frame of the last lighting edit (power, light / material parameters, environment): the temporal pass looks
    /// for changed lighting (anti-lag) only shortly after one.
    lit_changed: u32,
    /// The scene has light-sampled particles (emitter table entries with a nominal area).
    spark_lights: bool,
    /// Samples per pixel of the last frame rendered (the kernel's sample buffer holds their sums).
    shown_spp: u32,
    /// CPU time of the last commit (ms): skinning and re-transforming moving objects (`reskin`, rigid instance poses), and
    /// uploading them with the dynamic BLAS / TLAS rebuild submitted (frame log `skin`, `upload`).
    pub cpu_skin_ms: f32,
    pub cpu_upload_ms: f32,
    /// THREERT_PROFILE_BLAS=1: GPU time of the last dynamic BLAS / TLAS rebuild (ms; frame log `blas`), measured by draining
    /// the queue before it and waiting for it after (it stalls the frame: measurement runs only). The pass profile has
    /// no timestamps for it (Metal cannot write them between acceleration-structure builds).
    pub gpu_blas_ms: f32,
    /// CPU time the last `render` spent waiting for the oldest frame in flight (ms; frame log `wait`): GPU-bound frames.
    pub cpu_wait_ms: f32,
    /// The cloud cover the sky and the sun light were made with (`move_sun`).
    sky_cover: f32,
    /// The loop guard (merged.rs `guard`), if a kernel of this session may have it: the invocations it has stopped at
    /// its limit since the view was made, as last read back (a frame or two behind, without waiting). Above zero:
    /// frames with wrong pixels, a traversal that did not end on the scene's data. None without the guard.
    pub guard_trips: Option<u32>,
    /// Its readback: the buffer and its state (0 free, 1 mapping, 2 mapped), as the frame timer's.
    guard_read: Option<(wgpu::Buffer, Arc<AtomicU32>)>,
}

/// THREERT_CACHE_FLAGS: experiment switches of the radiance cache (pathtrace.wgsl `cfl`), 0 = defaults: 1 = no cold
/// fallback (cold voxels read as they are), 2 = no hit lighting (reflection hits use the cache's total), 4 = no
/// training tiles, 8 = the exact light loop at reflection hits. THREERT_CACHE=0 turns the cache off.
/// Frames after a reset or a camera cut during which pixels whose radiance-cache voxel is cold trace a fresh estimate
/// (pathtrace.wgsl, cache flag 1 off). Later the cold voxels left are mostly where bounce rays land off screen,
/// scattered, sparsely trained: their traces cost each SIMD group nearly a whole extra trace (Bistro's path 0.7-1.2
/// ms, the crowd's square 2.3 ms) for ~1 % of the image (bench-crowd ΔE 13.1 -> 13.3).
const COLD_FALLBACK_FRAMES: u32 = 30;

/// Ceiling for one live lighting sample, in display units at the current exposure (live.wgsl `sample_clamp`): fireflies
/// far above white. 64 until 2026-11-09, when it cut the light of small strong lights that the denoisers now keep: the
/// Bistro crowd in motion (lockstep at 10 s) diffuse 0.87 -> 0.90 of our offline render, specular 0.83 -> 0.89, the
/// pavement under the headlights 0.71 -> 0.77. 1024 adds nothing more.
const SAMPLE_CEILING: f32 = 256.0;

/// THREERT_GOVERN_CLOCK=1|0 (default 1 on Android): dynamic resolution also checks by the clock, so a view whose
/// frames are far slower than 60 a second comes down to its frame rate in seconds (`LiveCore::govern_resolution`).
fn govern_by_clock() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("THREERT_GOVERN_CLOCK").map(|v| v == "1").unwrap_or(cfg!(target_os = "android")))
}

fn cache_flags() -> u32 {
    static F: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("THREERT_CACHE_FLAGS").ok().and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// THREERT_FREEZE_TIME=1: animation holds still (shader time 0, the frame script gets dt 0: signs, screens, flicker groups),
/// so frames taken at different times compare (tools/bench-ghosting.ts).
fn freeze_time() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("THREERT_FREEZE_TIME").is_ok_and(|v| v == "1"))
}

/// Seconds of animated shading (sign flicker, screens, ripples) and particles: the scene time; 0 under THREERT_FREEZE_TIME.
fn shader_time(time: f32) -> f32 {
    if freeze_time() { 0.0 } else { time }
}

/// THREERT_EXPOSURE_SNAP=1: auto exposure jumps to its target every frame (no adaptation lag), so a frame taken while the
/// view moves has the exposure a settled view of that pose would have (tools/bench-ghosting.ts).
fn exposure_snap() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("THREERT_EXPOSURE_SNAP").is_ok_and(|v| v == "1"))
}

/// UE's physical CoC model (DiaphragmDOFUtils.cpp, FPhysicalCocModel::Compile): focal length from the sensor height
/// (sensor width / aspect) and the vertical FOV, the CoC of infinity on the sensor f^2 / (N (focus - f)), as a radius in
/// output pixels; max radius r.DOF.Kernel.Max{Fore,Back}groundRadius = 2.5 % of the width. Off without a focus distance.
fn dof_params(dof: Option<&crate::desc::Dof>, tan_half_fov: f32, w: u32, h: u32) -> [f32; 4] {
    let Some(d) = dof.filter(|d| d.focus_distance > 0.0 && d.f_stop > 0.0 && d.sensor_width > 0.0) else { return [0.0; 4] };
    let aspect = w as f32 / h as f32;
    let sensor_h = d.sensor_width / aspect;
    let f = 0.5 * sensor_h / tan_half_fov;
    let diameter = f * f / (d.f_stop * (d.focus_distance * 1000.0 - f).max(1e-3));
    [diameter * 0.5 / sensor_h / aspect * w as f32, d.focus_distance, 0.025 * w as f32, 0.0]
}

/// Light LOD (`emitter_lod`): emitters within this distance of the camera keep their sampling weight, farther ones
/// lose it as (d0 / d)^2.
const LIGHT_LOD_D0: f32 = 10.0;

/// Emitters farther than this from the light LOD's reference are not proposed (`emitter_lod`); BSDF rays bring their
/// light.
const EMITTER_CULL: f32 = 60.0;

/// The cull's reach inside the view cone (`EmitterCull`), in cull radii: emitters ahead of the camera are kept out to
/// this.
const EMITTER_CULL_VIEW: f32 = 2.0;
/// The view cone: the frustum's half-diagonal plus this margin (degrees); the CDF is rebuilt when the camera turns by
/// LIGHT_LOD_TURN, so the frustum stays inside the cone between rebuilds.
const EMITTER_CULL_MARGIN: f32 = 20.0;
const LIGHT_LOD_TURN: f32 = 15.0;



/// Halton(base) of index i (sub-pixel jitter sequence).
fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0f32, 0.0f32);
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

/// History caps of the cache: samples (noise), frames (lag; shorter for a while after a lighting edit).
const CACHE_SAMPLES: f32 = 256.0;
const CACHE_FRAMES: f32 = 32.0;
const CACHE_FRAMES_LIT: f32 = 4.0;
/// The most frames a pixel's history holds while the scene's lighting changes: short enough to follow the change by
/// itself (a real-time denoiser's bound: NRD's default is 30), so no edit has to cut it. Against main's rules (24
/// frames for good in a scene with spark lights, every pixel cut to 16 at a lighting edit, an on / off anti-lag), dE
/// against the offline render or the converged frame: the Bistro slice half a second after its explosion 6.04 for
/// 6.58, at rest 2.82 for 2.79; Kabukichō 2 s and 4 s after its signs close 6.70 and 2.92 for 6.56 and 2.82. At 64
/// frames the explosion read 5.72 and the signs 7.82 and 3.37.
const HISTORY_LIVE: f32 = 32.0;
/// The most once the scene has settled: holding still, the frame converges on the offline render.
const HISTORY_FRAMES: f32 = 4096.0;

/// THREERT_TL_HIDE (diagnostics): the translucency pass trains the volume voxels but draws nothing, so frames show what is
/// behind translucent particles; `odd` hides them on odd frames only (rain on / off pairs from one run).
fn translucent_hidden(frame: u32) -> bool {
    static F: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    match F.get_or_init(|| std::env::var("THREERT_TL_HIDE").ok().map(|v| v == "odd")) {
        None => false,
        Some(false) => true,
        Some(true) => frame % 2 == 1,
    }
}

impl LiveCore {
    /// Advances the scene time by a frame's dt (the frame script's: at most 0.1 s).
    pub fn advance_time(&mut self, dt: f32) {
        self.time += dt.clamp(0.0, 0.1);
    }

    /// Sets the scene time: a game session's frame script clock (`GameSession::frame`).
    pub fn set_time(&mut self, t: f32) {
        self.time = t;
    }

    /// The scene time animation sees (0 under THREERT_FREEZE_TIME).
    pub fn scene_time(&self) -> f32 {
        shader_time(self.time)
    }

    /// A live view of the engine's world; asset files resolve against `assets`.
    pub fn new(g: gpu::Gpu, world: &World, assets: &Path, presets: &Path, w: u32, h: u32) -> Result<LiveCore> {
        let passes = Passes::new(&g);
        // The scene's post preset replaces this LUT below (apply_post).
        let cache_on = !std::env::var("THREERT_CACHE").is_ok_and(|v| v == "0");
        // (Without the cache the kernel binds one of one entry.)
        let targets = Targets::new(&g, w, h, &vec![0u16; (LUT * LUT * LUT * 4) as usize], if cache_on { gpu::CACHE_SIZE } else { 1 });
        let (sc, pp) = load_scene(&g, world, assets, presets, w, h, &targets, &passes)?;
        let cam = sc.scene_camera;
        let (triangles, scene_name) = (sc.triangles, sc.name.clone());
        let profile = (g.timestamps && std::env::var("THREERT_PROFILE").is_ok()).then(|| {
            // trace, the post passes, cache resolve, then the particle pass, the BLAS rebuild, the translucency pass and
            // the volume training.
            let n = (passes.pipes.len() as u64 + 6) * 2;
            Profile {
                queries: g.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("profile"), ty: wgpu::QueryType::Timestamp, count: n as u32 }),
                resolve: g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: n * 8, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
                read: g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: n * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
                sums: vec![0.0; passes.pipes.len() + 6],
                frames: 0,
                grid: (
                    g.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("profile grid"), ty: wgpu::QueryType::Timestamp, count: 6 }),
                    g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 2 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
                    g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 48, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
                ),
                grid_sum: 0.0,
                media_sum: 0.0,
            }
        });
        let timer = (g.timestamps && profile.is_none()).then(|| FrameTimer::new(&g.device));
        let raster = std::env::var("THREERT_RASTER").is_ok_and(|v| v == "1").then(|| Raster::new(&g, w, h));
        let cache = cache_on.then(|| RadianceCache { bufs: targets.kernel.cache.clone(), resolve: g.cache_resolve_pipeline(&targets.kernel) });
        let kernel_bg = gpu::vis_bind_group(&g.device, &g.vis_layout, raster.as_ref().map_or(&g.no_vis_view, |r| &r.vis), &targets.kernel);
        // (Read whenever a kernel of this session may have the guard: under software traversal a scene's first unit
        // brings it, `gpu::loop_guard_of`.)
        let guard_read = (gpu::loop_guard_of(!g.hwrt).is_some() && targets.kernel.guard.is_some())
            .then(|| (g.device.create_buffer(&wgpu::BufferDescriptor { label: Some("loop guard read"), size: 4, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }), Arc::new(AtomicU32::new(0))));
        let mut core = LiveCore { particle_spawns: Default::default(), grid_sources: Default::default(), g, passes, targets, sc, presets: presets.to_path_buf(), assets: assets.to_path_buf(), w, h, seq: 0, aspect: w as f32 / h as f32, cam, prev_cam: cam, frame: 0, history_len: 0.0, reset: true, bloom: ([0.0; 6], [0.0; 6]), vignette: 0.0, post: [0.0; 4], blur: (0.0, 0.0), inflight: Default::default(), gpu_ms: 0.0, gpu_frame: 0, gpu_trace_ms: 0.0, edit_ms: 0.0, instanced_ms: 0.0, edit_split: EditSplit::default(), rebuild_ms: 0.0, fallback: None, fallbacks: 0, bounces: 4, rays: true, view_mode: std::env::var("THREERT_VIEW_MODE").ok().and_then(|v| v.parse().ok()).unwrap_or(0), fixed_dt: None, motion_blur_off: false, triangles, scene_name, time: 0.0, frame_dt: 1.0 / 60.0, last_frame: Instant::now(), post_key: String::new(), render_scale: 1.0, dynamic_res: false, min_scale: MIN_SCALE, display_scale: 1.0, over_budget: (0, 0), born: Instant::now(), governed: Instant::now(), profile, timer, raster, cache, kernel_bg, prev_span: None, reposed: false, emitters_moved: false, cold_frames: 0, pose_span: None, pose_reposed: false, inst_moving: vec![], inst_moved: false, prev_isz: (0, 0), lit_changed: 0, spark_lights: false, shown_spp: 1, cpu_skin_ms: 0.0, cpu_upload_ms: 0.0, gpu_blas_ms: 0.0, cpu_wait_ms: 0.0, sky_cover: 0.0, guard_trips: guard_read.as_ref().map(|_| 0), guard_read };
        core.spark_lights = core.sc.tables.emitter_tris.iter().any(|e| e.nominal);
        core.sky_cover = core.sc.tables.cloud_cover;
        core.apply_post(&pp);
        Ok(core)
    }

    pub fn scene_camera(&self) -> Camera {
        self.sc.scene_camera
    }

    /// `guard_trips` for the frame log: None while the scene's kernel has no guard. Natively with nothing set that is
    /// a scene without units (`gpu::loop_guard_of`): under software traversal the first unit brings the guard.
    pub fn guard_count(&self) -> Option<u32> {
        self.guard_trips.filter(|_| gpu::loop_guard_of(self.sc.up.kernel.features & gpu::F_UNITS != 0).is_some())
    }

    /// The edit room as the frame log reports it (`room`): triangles, records and object entries in use and held (GPU
    /// order counts, holes not in use), its growths, and the GPU bytes of the buffers edits write and of the triangle
    /// BLASes the live meshes need (estimated from the build's bytes per triangle), MB.
    pub fn room_stats(&self) -> serde_json::Value {
        let (e, up) = (&self.sc.edit, &self.sc.up);
        // Software traversal's (it does not grow): the same counts, and of the node buffer the mesh trees' range (in
        // use, held) and the units the top tree has against the most its range holds.
        if let Some(u) = up.units.as_ref() {
            return serde_json::json!({
                "tris": [e.tris.used(), e.tris.cap], "records": [e.records.used(), e.records.cap], "objects": [e.objects.used(), e.objects.cap],
                "holes": e.tris.free.len(), "grows": e.grows, "nodes": [u.layout.mesh_cap as usize - u.layout.free_nodes(), u.layout.mesh_cap], "units": [u.count, u.layout.max_units()],
            });
        }
        let Some(a) = up.accel.as_ref() else { return serde_json::Value::Null };
        let buffers = [&up.verts, &up.attrs, &up.prev_verts, &up.instances, &up.objects, &up.cdf, &up.emitters, &a.verts].iter().map(|b| b.size()).sum::<u64>();
        let live: u64 = self.sc.meshes.iter().enumerate().filter(|(k, _)| e.users.get(&(*k as u32)).copied().unwrap_or(0) > 0).map(|(_, m)| m.count as u64).sum::<u64>() + e.pool_first as u64;
        let mb = |b: f64| (b / (1 << 20) as f64 * 10.0).round() / 10.0;
        serde_json::json!({
            "tris": [e.tris.used(), e.tris.cap], "records": [e.records.used(), e.records.cap], "objects": [e.objects.used(), e.objects.cap],
            "holes": e.tris.free.len(), "grows": e.grows, "buffers_mb": mb(buffers as f64), "blas_mb": mb(live as f64 * a.blas_per_tri),
        })
    }

    /// Software traversal's top tree so far (the frame log's `top`): how many times it was built, kept with leaves
    /// taken out and put in, and kept with its boxes brought up to date, and the ms all of that took. Null without
    /// units.
    pub fn top_stats(&self) -> serde_json::Value {
        self.sc.up.units.as_ref().map_or(serde_json::Value::Null, |u| serde_json::json!([u.kept.builds, u.kept.changes, u.kept.refits, (u.kept.ms * 1e3).round() / 1e3]))
    }

    pub fn rebuild(&mut self, world: &World) -> Result<()> {
        let (s, pp) = load_scene(&self.g, world, &self.assets, &self.presets, self.w, self.h, &self.targets, &self.passes)?;
        self.apply_post(&pp);
        self.triangles = s.triangles;
        self.scene_name = s.name.clone();
        // The scene's camera changed (a new lens rebuilds, and a move in the same commit comes with it): the view takes it.
        // Otherwise the view stays where it is.
        if s.scene_camera != self.sc.scene_camera {
            self.cam = s.scene_camera;
        }
        // What the grid programs had simulated goes with the grids that are still the same.
        self.g.carry_grids(&self.sc.up, &s.up);
        self.sc = s;
        self.spark_lights = self.sc.tables.emitter_tris.iter().any(|e| e.nominal);
        self.inst_moving.clear();
        self.prev_span = None;
        self.pose_span = None;
        self.emitters_moved = false;
        self.lit_changed = self.frame.max(1);
        Ok(())
    }

    /// Rebuilds the tonemap LUT and bloom / vignette / fringe / grain settings when the scene's post preset changed.
    fn apply_post(&mut self, pp: &crate::desc::PostPreset) {
        let key = format!("{pp:?}");
        if key == self.post_key {
            return;
        }
        let lut = tonemap_lut(pp);
        self.g.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.targets.lut_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&lut),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(LUT * 8), rows_per_image: Some(LUT) },
            wgpu::Extent3d { width: LUT, height: LUT, depth_or_array_layers: LUT },
        );
        self.bloom = bloom_stages(pp, self.w, self.h);
        self.vignette = pp.vignette;
        // THREERT_NO_GRAIN=1: no film grain (a fresh random pattern every frame; comparisons between frames see it as error).
        let grain = if std::env::var("THREERT_NO_GRAIN").is_ok_and(|v| v == "1") { 0.0 } else { pp.film_grain };
        // THREERT_MOTION_BLUR=1: UE's default motion blur whatever the preset says; =amount,max,targetFps: those.
        let mb = match std::env::var("THREERT_MOTION_BLUR").ok().as_deref() {
            Some("1") => crate::desc::MotionBlur::default(),
            Some(v) => match v.split(',').map(|x| x.trim().parse::<f32>()).collect::<Result<Vec<_>, _>>().as_deref() {
                Ok([amount, max, target_fps]) => crate::desc::MotionBlur { amount: *amount, max: *max, target_fps: *target_fps },
                _ => pp.motion_blur.clone(),
            },
            None => pp.motion_blur.clone(),
        };
        self.post = [pp.chromatic_aberration, grain, pp.bloom.threshold, mb.max / 100.0 * self.w as f32];
        self.blur = (if mb.max > 0.0 { mb.amount.max(0.0) } else { 0.0 }, mb.target_fps);
        self.post_key = key;
    }

    /// Switches to a different world: camera from it, history cleared.
    pub fn open(&mut self, world: &World, assets: &Path) -> Result<()> {
        self.assets = assets.to_path_buf();
        self.rebuild(world)?;
        if let Some(c) = &self.cache {
            let mut e = self.g.device.create_command_encoder(&Default::default());
            c.bufs.clear(&mut e);
            self.g.queue.submit([e.finish()]);
        }
        self.cam = self.sc.scene_camera;
        self.prev_cam = self.cam;
        self.reset = true;
        Ok(())
    }

    /// The view is a display's physical pixels, `k` a point: dynamic resolution, from the points' resolution (the rays
    /// a view in points starts with) down to `MIN_SCALE` of it. A fixed scale is set after it (`set_scale`).
    pub fn set_display_scale(&mut self, k: f32) {
        let k = k.max(1.0);
        self.display_scale = k;
        self.min_scale = (MIN_SCALE / k).max(SCALE_FLOOR);
        self.render_scale = 1.0 / k;
        self.dynamic_res = true;
    }

    /// The scene's lighting is changing: something moved or was edited in the last 45 frames, or the scene has spark
    /// lights (light-sampled particles: they move every frame, and no edit says so).
    fn live_lighting(&self) -> bool {
        self.spark_lights || (self.lit_changed > 0 && self.frame.wrapping_sub(self.lit_changed) < 45)
    }

    /// Fixed render scale (`s` in SCALE_FLOOR ..= 1), or dynamic resolution (`s` = 0).
    pub fn set_scale(&mut self, s: f32) {
        self.dynamic_res = s <= 0.0;
        if s > 0.0 {
            self.render_scale = s.clamp(SCALE_FLOOR, 1.0);
        }
    }

    pub fn reset_history(&mut self) {
        self.reset = true;
    }

    /// Ray tracing on or off (see `rays`). The history starts over: the two modes light a pixel differently.
    pub fn set_rays(&mut self, on: bool) {
        if self.rays != on {
            self.rays = on;
            self.reset = true;
        }
    }

    pub fn rays(&self) -> bool {
        self.rays
    }

    pub fn set_camera(&mut self, mut c: Camera) {
        if c.tan_half_fov <= 0.0 {
            c.tan_half_fov = self.sc.tan_half_fov;
        }
        self.cam = c;
    }

    /// Renders one frame with `spp` samples per pixel; returns the command buffers to submit after the samples.
    /// Path-trace dispatches are submitted inside (each needs its own uniform contents).
    /// Traced resolution at the current render scale.
    pub fn internal_size(&self) -> (u32, u32) {
        let s = self.render_scale.clamp(SCALE_FLOOR, 1.0);
        (((self.w as f32 * s).round() as u32).clamp(8, self.w), ((self.h as f32 * s).round() as u32).clamp(8, self.h))
    }

    /// Dynamic resolution (UE r.DynamicRes): twice a second, move the render scale toward the frame budget (traced
    /// pixels ~ GPU time), within [`min_scale`, 1], in damped steps with hysteresis so it does not oscillate.
    /// Two frames in a row well over the frame time (x 1.25) act at once (UE
    /// r.DynamicRes.MaxConsecutiveOverbudgetGPUFrameCount = 2): turning from a light view at a raised scale toward a
    /// heavy one otherwise cost up to 30 frames at 35-48 ms (the 3 x 3 district's drive, its far end). Just over the
    /// frame time waits for the regular check, as before. After a change, only frames rendered at the new scale count:
    /// GPU time lags the scale by the frames in flight, and acting on the old scale's time overshot to the floor.
    ///
    /// "Twice a second" and "the first 2 s" are counted in frames (every 30th, the first 120), which is the same thing
    /// at 60 frames a second. Where frames are much slower (`govern_by_clock`: a phone's GPU, on which a scene opens
    /// at its full scale at 6 frames a second) they are also read off the clock: counted in frames alone the first
    /// check that may act at once came after 20 s and the regular one every 5 s, and a game opened as a slide show
    /// for half a minute (Adreno 740, OpenStrikeRT: 25 s to its frame rate; 3 s with the clock).
    pub fn govern_resolution(&mut self, fps: u32) {
        if !self.dynamic_res || self.gpu_ms <= 0.0 {
            return;
        }
        if (self.gpu_frame.wrapping_sub(self.over_budget.1) as i32) < 0 {
            return;
        }
        let clock = govern_by_clock();
        // Not in the first 2 s: those frames run while caches fill (Kabukichō's first frames at 0.7 take ~47 ms), and
        // the regular check settles the start as before.
        let warm = self.frame >= 120 || (clock && self.born.elapsed().as_secs_f32() >= 2.0);
        self.over_budget.0 = if warm && self.gpu_ms > 1250.0 / fps as f32 && self.render_scale > self.min_scale { self.over_budget.0 + 1 } else { 0 };
        let urgent = self.over_budget.0 >= 2;
        // (By the clock only when 30 frames take longer than 0.75 s: at 40 frames a second and above the frame count
        // comes first, as it always did.)
        let due = self.frame % 30 == 1 || (clock && self.governed.elapsed().as_secs_f32() >= 0.75);
        if !due && !urgent {
            return;
        }
        self.governed = Instant::now();
        self.over_budget.0 = 0;
        let before = self.render_scale;
        let budget = 1000.0 / fps as f32 * 0.9;
        let target = (self.render_scale * (budget / self.gpu_ms).sqrt()).clamp(self.min_scale, 1.0);
        // The hysteresis (0.03) does not hold the scale just short of either end: 0.525 with the floor as the target
        // kept the district's return drive at 21-22 ms.
        let to_end = (target == 1.0 && self.render_scale < 1.0 && self.gpu_ms < budget * 0.8) || (target == self.min_scale && self.render_scale > self.min_scale);
        // (The hysteresis, the steps and the grid of scales are a view in points'; on a display of k pixels a point the
        // same share of the rays is 1 / k of the scale. Left absolute, a Retina view held 0.325 at 17.3 ms: its target,
        // 0.30, was inside the hysteresis.)
        let k = self.display_scale;
        if (target - self.render_scale).abs() > 0.03 / k || to_end {
            // Down fast when far over budget (a heavy scene just opened), up slowly; straight to the target when urgent.
            let step = if urgent { target - self.render_scale } else { (target - self.render_scale).clamp(if self.gpu_ms > budget * 1.5 { -0.3 } else { -0.15 } / k, 0.1 / k) };
            self.render_scale = ((self.render_scale + step) * 40.0 * k).round() / (40.0 * k);
        }
        if self.render_scale != before {
            // `frame` is already the next frame's index (render advanced it): the first one at the new scale.
            self.over_budget.1 = self.frame;
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// The last frame's linear HDR radiance (nits, before exposure and post), RGB per output pixel. Waits for the GPU.
    pub fn read_hdr(&self) -> Result<Vec<f32>> {
        let d = &self.g.device;
        let size = self.w as u64 * self.h as u64 * 16;
        let read = d.create_buffer(&wgpu::BufferDescriptor { label: Some("hdr read"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut e = d.create_command_encoder(&Default::default());
        let fa = self.targets.buf("fa");
        e.copy_buffer_to_buffer(&fa.buffer, fa.offset, &read, 0, size);
        self.g.queue.submit([e.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        d.poll(wgpu::PollType::wait_indefinitely())?;
        let px = bytemuck::cast_slice::<u8, [f32; 4]>(&read.slice(..).get_mapped_range().unwrap()).iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        Ok(px)
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// The last frame's signals apart, linear RGB f32 per output pixel: diffuse (the composed colour less the rest, so
    /// with the sun's direct light, which view mode 6 leaves out), specular (the specular denoiser's output: mode 7) and
    /// emission (what the camera ray saw directly). They are the render resolution's (the frame as `compose` hands it to
    /// the upscaler), each output pixel taking the traced sample under it; not with depth of field and motion blur both
    /// on (the composed colour's buffer is theirs then). Waits for the GPU.
    pub fn read_signals(&self) -> Result<[Vec<f32>; 3]> {
        let d = &self.g.device;
        let (w, h) = (self.w as usize, self.h as usize);
        let (iw, ih) = (self.prev_isz.0 as usize, self.prev_isz.1 as usize);
        let (n, tn) = ((w * h) as u64, (iw * ih) as u64);
        let t = &self.targets;
        let read = |buffer: &wgpu::Buffer, offset: u64, size: u64| -> Result<Vec<u8>> {
            let r = d.create_buffer(&wgpu::BufferDescriptor { label: Some("signals read"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let mut e = d.create_command_encoder(&Default::default());
            e.copy_buffer_to_buffer(buffer, offset, &r, 0, size);
            self.g.queue.submit([e.finish()]);
            r.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            d.poll(wgpu::PollType::wait_indefinitely())?;
            let v = r.slice(..).get_mapped_range()?.to_vec();
            Ok(v)
        };
        let half = |b: u32, hi: bool| half::f16::from_bits(if hi { (b >> 16) as u16 } else { b as u16 }).to_f32();
        let fb = t.buf("fb");
        let composed = read(&fb.buffer, fb.offset, tn * 16)?;
        let spec_pp = t.buf("spec_pp");
        let spec = read(&spec_pp.buffer, spec_pp.offset + n * 8, tn * 8)?; // spec_pp slot 1: (rgb, history) as two pairs of f16
        let samples = read(&self.sc.up.accum, 0, tn * 80)?; // the kernel's samples (live.wgsl `Pixel`): emit at 48
        let (composed, spec, samples): (&[[f32; 4]], &[[u32; 2]], &[[f32; 20]]) = (bytemuck::cast_slice(&composed), bytemuck::cast_slice(&spec), bytemuck::cast_slice(&samples));
        let (mut dif, mut spe, mut emi) = (Vec::with_capacity(w * h * 3), Vec::with_capacity(w * h * 3), Vec::with_capacity(w * h * 3));
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        for y in 0..h {
            for x in 0..w {
                // live.wgsl `sample_under`.
                let q = (((y as f32 + 0.5) * ih as f32 / h as f32) as usize).min(ih - 1) * iw + (((x as f32 + 0.5) * iw as f32 / w as f32) as usize).min(iw - 1);
                // The media before the surface (live.wgsl `medium_tr`): the specular as the frame shows it.
                let tr = (samples[q][11].fract() * 2.0).clamp(0.0, 1.0);
                let s = [half(spec[q][0], false), half(spec[q][0], true), half(spec[q][1], false)].map(|v| v * tr);
                for c in 0..3 {
                    let e = finite(samples[q][12 + c] / self.shown_spp as f32);
                    spe.push(s[c]);
                    emi.push(e);
                    dif.push((composed[q][c] - s[c] - e).max(0.0));
                }
            }
        }
        Ok([dif, spe, emi])
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// The exposure the last frame was shown at, EV100 (auto exposure: its adapted value).
    pub fn read_ev100(&self) -> Result<f32> {
        if self.sc.exposure_settings.ae2[3] <= 0.5 {
            return Ok(-self.sc.exposure.log2());
        }
        let d = &self.g.device;
        let read = d.create_buffer(&wgpu::BufferDescriptor { label: Some("ae read"), size: 16, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut e = d.create_command_encoder(&Default::default());
        let ae = self.targets.buf("ae_state");
        e.copy_buffer_to_buffer(&ae.buffer, ae.offset, &read, 0, 16);
        self.g.queue.submit([e.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        d.poll(wgpu::PollType::wait_indefinitely())?;
        let s = bytemuck::cast_slice::<u8, f32>(&read.slice(..).get_mapped_range().unwrap())[0];
        Ok(-s.max(1e-30).log2())
    }

    /// Phases of the last scene (re)load, ms.
    pub fn load_timings(&self) -> &[(String, f64)] {
        &self.sc.timings
    }

    pub fn aspect(&self) -> f32 {
        self.aspect
    }

    /// Primary ray through (u, v) of the image (0..1, v down), as the path tracer generates it.
    pub fn ray(&self, u: f32, v: f32) -> (Vec3, Vec3) {
        let c = &self.cam;
        let d = c.fwd + c.right * ((u * 2.0 - 1.0) * c.tan_half_fov * self.aspect) + c.up * ((1.0 - v * 2.0) * c.tan_half_fov);
        (c.pos, d.normalize())
    }

    /// Closest surface under (u, v): (node id, world-space hit point). CPU ray cast over the scene triangles, and over
    /// each instance's model-space mesh with the ray in its space (same ray parameter t).
    /// The object under view point (u, v): its name, the hit point, and for an instanced object the instance's index.
    pub fn pick(&self, u: f32, v: f32) -> Option<(String, Vec3, Option<u32>)> {
        let (o, d) = self.ray(u, v);
        let (o, d) = (glam::Vec3A::from(o), glam::Vec3A::from(d));
        // Moller-Trumbore, two-sided: the ray parameter of the closest hit in `tris` beyond `near`.
        let closest = |tris: &[[glam::Vec3A; 3]], o: glam::Vec3A, d: glam::Vec3A| {
            let mut best = (f32::INFINITY, usize::MAX);
            for (i, [a, b, c]) in tris.iter().enumerate() {
                let (e1, e2) = (*b - *a, *c - *a);
                let p = d.cross(e2);
                let det = e1.dot(p);
                if det.abs() < 1e-12 {
                    continue;
                }
                let inv = 1.0 / det;
                let s = o - *a;
                let bu = s.dot(p) * inv;
                if !(0.0..=1.0).contains(&bu) {
                    continue;
                }
                let q = s.cross(e1);
                let bv = d.dot(q) * inv;
                if bv < 0.0 || bu + bv > 1.0 {
                    continue;
                }
                let t = e2.dot(q) * inv;
                if t > 1e-4 && t < best.0 {
                    best = (t, i);
                }
            }
            best
        };
        let (mut t, i) = closest(&self.sc.tris, o, d);
        let mut node = self.sc.tri_nodes.iter().find(|(_, a, b)| (*a as usize..*b as usize).contains(&i)).map(|n| n.0.as_str());
        let mut sub = self.sc.tri_instances.iter().find(|(a, b, _)| (*a as usize..*b as usize).contains(&i)).map(|r| r.2);
        let added = |want_mesh: bool| self.sc.edit.added.values().filter_map(move |p| match p {
            Picked::Mesh(i) if want_mesh => Some(Ok(i)),
            Picked::Proc(q) if !want_mesh => Some(Err(q)),
            _ => None,
        });
        for inst in self.sc.instances.iter().filter(|i| !i.hidden).chain(added(true).filter_map(|x| x.ok())) {
            let m = self.sc.meshes[inst.mesh as usize];
            let inv = inst.world.inverse();
            let (lo, ld) = (inv.transform_point3a(o), inv.transform_vector3a(d));
            // Slab test against the mesh bounds before its triangles.
            let (t0, t1) = ((m.lo - lo) / ld, (m.hi - lo) / ld);
            let (near, far) = (t0.min(t1).max_element(), t0.max(t1).min_element());
            if far < near.max(0.0) || near > t {
                continue;
            }
            let (ti, _) = closest(&self.sc.pool_tris[m.first as usize..(m.first + m.count) as usize], lo, ld);
            if ti < t {
                t = ti;
                node = Some(inst.name.as_str());
                sub = inst.sub;
            }
        }
        // Procedural geometry: its boxes (the geometry function runs on the GPU only).
        for p in self.sc.procs.iter().filter(|p| !p.hidden).chain(added(false).filter_map(|x| x.err())) {
            let inv = p.world.inverse();
            let (lo, ld) = (inv.transform_point3a(o), inv.transform_vector3a(d));
            let slab = |a: glam::Vec3A, b: glam::Vec3A| {
                let (t0, t1) = ((a - lo) / ld, (b - lo) / ld);
                let (near, far) = (t0.min(t1).max_element().max(1e-4), t0.max(t1).min_element());
                (near <= far).then_some(near)
            };
            if slab(p.lo.into(), p.hi.into()).is_none_or(|near| near > t) {
                continue;
            }
            for b in p.blob.boxes.chunks(6) {
                if let Some(near) = slab(glam::Vec3A::new(b[0], b[1], b[2]), glam::Vec3A::new(b[3], b[4], b[5])).filter(|&near| near < t) {
                    t = near;
                    node = Some(p.name.as_str());
                    sub = p.sub;
                }
            }
        }
        Some((node?.to_string(), Vec3::from(o + d * t), sub))
    }

    /// World-space triangles of a node (empty for lights without geometry, cameras, hidden nodes).
    pub fn node_tris(&self, id: &str) -> Vec<[glam::Vec3A; 3]> {
        // World-space triangles (an instance's glass / emissive part among them), then its instanced mesh.
        let mut out: Vec<[glam::Vec3A; 3]> = scene::node_ranges(&self.sc.tri_nodes, id).flat_map(|r| self.sc.tris[r].iter().copied()).collect();
        // A model's first instance; every drawn instance of an instanced object.
        let mut of_node = self.sc.instances.iter().filter(|i| i.name == id && !i.hidden);
        let first = of_node.next();
        for inst in first.into_iter().chain(of_node.filter(|i| i.sub.is_some())) {
            let m = self.sc.meshes[inst.mesh as usize];
            out.extend(self.sc.pool_tris[m.first as usize..(m.first + m.count) as usize].iter().map(|t| inst.tri(t)));
        }
        // Objects added in place (Bugbot on #131: `pick` saw them, this did not).
        for p in self.sc.edit.added.values() {
            if let Picked::Mesh(inst) = p {
                if inst.name != id {
                    continue;
                }
                let m = self.sc.meshes[inst.mesh as usize];
                out.extend(self.sc.pool_tris[m.first as usize..(m.first + m.count) as usize].iter().map(|t| inst.tri(t)));
            }
        }
        out
    }

    /// Frame metadata (camera pose for picking, convergence, timings).
    pub fn meta(&self) -> serde_json::Value {
        let c = &self.cam;
        serde_json::json!({
            "seq": self.seq, "frame": self.frame, "history": self.history_len, "gpu_ms": self.gpu_ms,
            "scene": self.scene_name, "triangles": self.triangles, "bounces": self.bounces, "mode": self.view_mode, "width": self.w, "height": self.h, "scale": self.render_scale,
            "cam": { "pos": c.pos.to_array(), "fwd": c.fwd.to_array(), "up": c.up.to_array(), "right": c.right.to_array(), "tan": c.tan_half_fov, "aspect": self.aspect },
        })
    }
}

/// Adaptive samples per frame: fill ~75% of the frame budget with path tracing.
pub fn next_spp(spp: u32, gpu_ms: f32, fps: u32) -> u32 {
    if gpu_ms <= 0.0 {
        return spp;
    }
    let budget = 750.0 / fps as f32;
    let per_sample = gpu_ms / spp as f32;
    let target = (budget / per_sample).floor() as u32;
    target.clamp(1, 16).min(spp + 1).max(spp.saturating_sub(2).max(1))
}

#[cfg(not(target_arch = "wasm32"))]
/// `--scale`: a fraction (0.25 .. 1) or `auto` (dynamic resolution).
pub fn parse_scale(v: Option<&str>, default: Option<f32>) -> Result<Option<f32>> {
    match v {
        None => Ok(default),
        Some("auto") => Ok(None),
        Some(v) => v.parse::<f32>().map(|s| Some(s.clamp(SCALE_FLOOR, 1.0))).map_err(|_| anyhow!("--scale must be a number or auto")),
    }
}

#[cfg(test)]
mod timer_tests {
    use super::*;

    #[test]
    fn a_view_in_physical_pixels_starts_with_dynamic_resolution() {
        let opts = |extra: &[&str]| parse_opts(&["threert-render", "live", "game.js"].iter().chain(extra).map(|a| a.to_string()).collect::<Vec<_>>()).unwrap();
        assert_eq!(opts(&[]).scale, Some(1.0));
        assert_eq!(opts(&["--display-scale", "2"]).scale, None);
        assert_eq!(opts(&["--display-scale", "2", "--scale", "0.2"]).scale, Some(0.2));
        assert_eq!(opts(&["--scale", "auto"]).scale, None);
    }

    #[test]
    fn a_post_pass_from_another_frame_counts_by_its_own_length() {
        // Two frames of the crowd scene (ns, period 1): frame 453 as read, and frame 454, whose slot read frame 458's
        // post pair (74.5 ms after its trace began).
        let t453 = [35929577531333, 35929585495833, 35929585700583, 35929591610875, 0, 0, 0, 0];
        let t454 = [35929593765500, 35929601614666, 35929668312875, 35929673973041, 0, 0, 0, 0];
        let ms = |t: [u64; 8]| frame_gpu_ms(&t, 1, 1.0).unwrap();
        assert_eq!(ms(t453), (t453[3] - t453[0]) as f32 * 1e-6);
        assert!((ms(t454) - (7.849166 + 5.660166)).abs() < 1e-3, "{}", ms(t454));
        // The first pass is the frame's start; nothing timed, no measurement.
        assert_eq!(frame_gpu_ms(&[0, 0, 0, 0, 0, 0, 0, 0], 1, 1.0), None);
        let particles_first = [2_000_000, 9_000_000, 9_200_000, 14_000_000, 0, 0, 1_000_000, 1_900_000];
        assert_eq!(frame_gpu_ms(&particles_first, 1, 1.0), Some(13.0));
        // 3 samples: two more traces (untimed) between the first and the post pass, a 16.2 ms gap: the span. The post
        // pair of frame 458 under 3 samples: still out of place (trace 7.85 ms, gap 66.7 ms), counted as the passes.
        let three = [1_000_000, 9_000_000, 25_200_000, 31_000_000, 0, 0, 0, 0];
        assert_eq!(frame_gpu_ms(&three, 3, 1.0), Some(30.0));
        assert!((frame_gpu_ms(&t454, 3, 1.0).unwrap() - (3.0 * 7.849166 + 5.660166)).abs() < 1e-3);
    }
}

#[cfg(test)]
mod instanced_tests {
    use super::*;
    use crate::world::{Geometry, Kind};
    use glam::Mat4;

    /// Clicks pick what is drawn (ROADMAP § 6 step 5): an object added in place is picked, one removed in place no
    /// longer is, and a rebuild agrees. (Skipped without a GPU with hardware RT.)
    #[test]
    fn picks_follow_in_place_edits() {
        let g = match gpu::Gpu::new() {
            Ok(g) if g.hwrt => g,
            other => return eprintln!("skipped: {}", other.map_or_else(|e| e.to_string(), |_| "no hardware RT".into())),
        };
        let mut w = World::default();
        w.set_material(1, serde_json::from_value(serde_json::json!({ "name": "m", "color": [0.5, 0.5, 0.5], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true })).unwrap());
        let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: vec![0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
        w.set_geometry(2, quad(1.0)).unwrap();
        w.set_geometry(3, quad(0.5)).unwrap();
        let mesh = |geometry| Kind::Mesh { geometry, material: 1, own_blas: true, materials: vec![] };
        w.set_object(10, "a".into(), None, mesh(2), true, false).unwrap();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(4, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_transform(4, Vec3::new(0.0, 0.0, 5.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_settings(crate::world::Settings { camera: Some(4), ..Default::default() });
        let mut core = LiveCore::new(g, &w, Path::new("."), &crate::default_presets_dir(), 64, 36).unwrap();
        w.take_changes();
        let picked = |core: &LiveCore| core.pick(0.5, 0.5).map(|p| p.0);
        assert_eq!(picked(&core).as_deref(), Some("a"));
        // Added in place, in front of it: picked.
        w.set_object(11, "b".into(), None, mesh(3), true, false).unwrap();
        w.set_transform(11, Vec3::new(0.0, 0.0, 1.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        assert!(!core.commit(&mut w).unwrap(), "added in place");
        assert_eq!(picked(&core).as_deref(), Some("b"));
        assert_eq!(core.node_tris("b").len(), 2, "its triangles under its name");
        // Removed in place: the one behind it is picked again; both removed: nothing.
        w.remove(11);
        assert!(!core.commit(&mut w).unwrap(), "removed in place");
        assert_eq!(picked(&core).as_deref(), Some("a"));
        assert!(core.node_tris("b").is_empty(), "removed: none");
        w.remove(10);
        assert!(!core.commit(&mut w).unwrap(), "removed in place");
        assert_eq!(picked(&core), None);
        core.rebuild(&w).unwrap();
        assert_eq!(picked(&core), None, "a rebuild agrees");
    }

    /// A mesh a rigid body drives, added while the game runs with a material the scene did not have (ROADMAP § 6 step
    /// 7): placed in place, and its body's moves are its instance's transform, with no rebuild; a rebuild places it the
    /// same and it goes on moving in place. (Skipped without a GPU with hardware RT.)
    #[test]
    fn driven_meshes_added_in_place_move_as_instances() {
        let g = match gpu::Gpu::new() {
            Ok(g) if g.hwrt => g,
            other => return eprintln!("skipped: {}", other.map_or_else(|e| e.to_string(), |_| "no hardware RT".into())),
        };
        let mut w = World::default();
        let mat = |r: f32| serde_json::from_value(serde_json::json!({ "name": "m", "color": [r, r, r], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true })).unwrap();
        w.set_material(1, mat(0.5));
        let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: vec![0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
        w.set_geometry(2, quad(1.0)).unwrap();
        w.set_geometry(3, quad(0.5)).unwrap();
        let mesh = |geometry, material| Kind::Mesh { geometry, material, own_blas: true, materials: vec![] };
        w.set_object(10, "a".into(), None, mesh(2, 1), true, false).unwrap();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(4, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_transform(4, Vec3::new(0.0, 0.0, 5.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_settings(crate::world::Settings { camera: Some(4), ..Default::default() });
        let mut core = LiveCore::new(g, &w, Path::new("."), &crate::default_presets_dir(), 64, 36).unwrap();
        w.take_changes();
        let picked = |core: &LiveCore, u: f32| core.pick(u, 0.5).map(|p| p.0);
        // The box and its material arrive together, as scene code adds them; its body is told of it right after.
        let materials = core.sc.tables.materials.len();
        w.set_material(5, mat(0.9));
        w.set_object(11, "b".into(), None, mesh(3, 5), true, false).unwrap();
        w.set_driven(11).unwrap();
        w.set_transform(11, Vec3::new(0.0, 0.0, 1.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        assert!(!core.commit(&mut w).unwrap(), "added in place, its material too");
        assert_eq!(core.sc.tables.materials.len(), materials + 1);
        assert_eq!(picked(&core, 0.5).as_deref(), Some("b"));
        // Its body moves it 3 m to the side (u 0.95 of the image, 4 m from the camera): no rebuild, its record moved
        // with the rows it had the frame before.
        let record = core.sc.edit.placed[&11].1;
        assert!(w.place_driven(11, Vec3::new(3.0, 0.0, 1.0), glam::Quat::IDENTITY));
        assert!(!core.commit(&mut w).unwrap(), "moved as an instance");
        assert_eq!(core.fallbacks, 0);
        assert_eq!((picked(&core, 0.5).as_deref(), picked(&core, 0.95).as_deref()), (Some("a"), Some("b")));
        let r = core.sc.gpu_instances[record];
        assert_eq!((r.m[0][3], r.p[0][3]), (3.0, 0.0), "moved: last frame's rows kept");
        // A rebuild places it as an instance too, and it moves on in place.
        core.rebuild(&w).unwrap();
        assert!(core.sc.tables.retained.instance_of.contains_key(&11) && core.sc.edit.placed.is_empty());
        assert_eq!((picked(&core, 0.5).as_deref(), picked(&core, 0.95).as_deref()), (Some("a"), Some("b")));
        assert!(w.place_driven(11, Vec3::new(0.0, 0.0, 1.0), glam::Quat::IDENTITY));
        assert!(!core.commit(&mut w).unwrap(), "moved as an instance");
        assert_eq!((picked(&core, 0.5).as_deref(), picked(&core, 0.95)), (Some("b"), None));
        // Removed in place.
        w.remove(11);
        assert!(!core.commit(&mut w).unwrap(), "removed in place");
        assert_eq!(picked(&core, 0.5).as_deref(), Some("a"));
    }

    /// An instanced mesh's moved instances keep their previous-frame rows (motion vectors) through the commit that moves
    /// them, and catch up at the next commit that does not move them. (Skipped without a GPU with hardware RT.)
    #[test]
    fn moved_instances_keep_their_motion() {
        let g = match gpu::Gpu::new() {
            Ok(g) if g.hwrt => g,
            other => return eprintln!("skipped: {}", other.map_or_else(|e| e.to_string(), |_| "no hardware RT".into())),
        };
        let mut w = World::default();
        let mat = |r: f32| serde_json::from_value(serde_json::json!({ "name": "m", "color": [r, r, r], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": false })).unwrap();
        w.set_material(1, mat(0.5));
        w.set_geometry(2, Geometry { positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], normals: vec![0.0, 0.0, 1.0].repeat(3), uvs: vec![0.0; 6], indices: vec![0, 1, 2], groups: vec![], colors: vec![] }).unwrap();
        w.set_object(3, "copies".into(), None, Kind::Instanced { geometry: 2, procedural: false, materials: vec![1], capacity: 2 }, true, false).unwrap();
        let at = |x: f32| [Mat4::IDENTITY.to_cols_array(), Mat4::from_translation(Vec3::new(x, 0.0, 0.0)).to_cols_array()].concat();
        w.set_instances(3, 2, Some(at(1.0)), None, None).unwrap();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(4, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_transform(4, Vec3::new(0.0, 0.0, 5.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_settings(crate::world::Settings { camera: Some(4), ..Default::default() });
        let mut core = LiveCore::new(g, &w, Path::new("."), &crate::default_presets_dir(), 64, 36).unwrap();
        w.take_changes();
        let first = core.sc.tables.retained.instanced[&3].first as usize;
        w.set_instances(3, 2, Some(at(2.0)), None, None).unwrap();
        assert!(!core.commit(&mut w).unwrap(), "in place");
        let r = core.sc.gpu_instances[first + 1];
        assert_eq!((r.m[0][3], r.p[0][3]), (2.0, 1.0), "moved: last frame's rows kept");
        w.set_material(1, mat(0.6));
        core.commit(&mut w).unwrap();
        let r = core.sc.gpu_instances[first + 1];
        assert_eq!(r.p, r.m, "not moved since: caught up");
        // Movable, moved and given new instances in one commit: updated once (twice would lose the motion).
        w.set_object(3, "copies".into(), None, Kind::Instanced { geometry: 2, procedural: false, materials: vec![1], capacity: 2 }, true, true).unwrap();
        assert!(core.commit(&mut w).unwrap(), "made movable: rebuilt");
        let first = core.sc.tables.retained.instanced[&3].first as usize;
        w.set_transform(3, Vec3::new(0.0, 1.0, 0.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_instances(3, 2, Some(at(3.0)), None, None).unwrap();
        let c = w.changes.clone();
        assert!(c.moved.contains(&3) && c.instanced.contains(&3), "{c:?}");
        assert!(!core.commit(&mut w).unwrap(), "in place");
        let r = core.sc.gpu_instances[first + 1];
        assert_eq!((r.m[0][3], r.m[1][3], r.p[0][3], r.p[1][3]), (3.0, 1.0, 2.0, 0.0), "moved: last frame's rows kept");
    }

    /// The uniform tables (`gpu::GpuOptions::uniform_tables`: what a device is opened with on Android), on this
    /// machine's GPU: a lit scene with an instanced mesh renders offline to the default device's image, bit for bit;
    /// live, the four uniform buffers hold what their tables hold when a frame is traced, and after an edit in place
    /// what the tables hold then. (Skipped without a GPU.)
    #[test]
    fn the_uniform_tables_render_what_the_tables_do() {
        let open = |uniform: bool| gpu::Gpu::new_with(gpu::GpuOptions { uniform_tables: Some(uniform), ..Default::default() });
        let (plain, uniform) = match (open(false), open(true)) {
            (Ok(a), Ok(b)) => (a, b),
            (Err(e), _) | (_, Err(e)) => return eprintln!("skipped: {e}"),
        };
        if uniform.merged {
            return eprintln!("skipped: the merged layout has no uniform tables");
        }
        assert!(uniform.uniform_tables && !plain.uniform_tables);
        let mut w = World::default();
        let mat = |color: f32, emissive: f32| serde_json::from_value(serde_json::json!({ "name": "m", "color": [color, color * 0.5, color], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [1, 1, 1], "emissiveIntensity": emissive, "doubleSided": true })).unwrap();
        w.set_material(1, mat(0.8, 0.0));
        w.set_material(5, mat(0.0, 40.0));
        let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: [0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
        w.set_geometry(2, quad(1.0)).unwrap();
        w.set_geometry(3, quad(0.25)).unwrap();
        let mesh = |geometry, material, own_blas| Kind::Mesh { geometry, material, own_blas, materials: vec![] };
        // A wall, a small movable panel in front of it, and an emissive quad that lights both from the side.
        w.set_object(10, "wall".into(), None, mesh(2, 1, false), true, false).unwrap();
        w.set_object(11, "panel".into(), None, mesh(3, 1, true), true, true).unwrap();
        w.set_transform(11, Vec3::new(-0.4, 0.0, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_object(12, "lamp".into(), None, mesh(3, 5, false), true, false).unwrap();
        w.set_transform(12, Vec3::new(0.6, 0.3, 1.5), glam::Quat::from_rotation_y(2.6), Vec3::ONE).unwrap();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(4, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_transform(4, Vec3::new(0.0, 0.0, 5.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_settings(crate::world::Settings { camera: Some(4), ..Default::default() });
        let presets = crate::default_presets_dir();

        // Offline: the same bytes.
        let offline = |g: &gpu::Gpu| {
            let mut sc = crate::scene::build(&w, Path::new("."), &presets, None).unwrap();
            (sc.width, sc.height) = (64, 36);
            let prep = crate::prepare(g, &mut sc, &mut vec![]);
            let frame = g.render(&sc, &prep.tri_verts, prep.groups, bytemuck::cast_slice(&prep.nodes), &prep.emitters, &prep.env, &prep.env_cdf, &prep.energy, prep.params, 16, &mut vec![]).unwrap();
            frame.color.iter().flatten().map(|v| v.to_bits()).collect::<Vec<u32>>()
        };
        let (a, b) = (offline(&plain), offline(&uniform));
        let lit = a.iter().filter(|v| f32::from_bits(**v) > 0.0).count();
        assert!(lit > a.len() / 16, "the scene is lit ({lit} of {} numbers)", a.len());
        let differ = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        assert_eq!(differ, 0, "{differ} of {} numbers differ between the storage tables and the uniform ones", a.len());
        drop(plain);

        // Live: the uniform buffers follow the tables.
        let mut core = LiveCore::new(uniform, &w, Path::new("."), &presets, 64, 36).unwrap();
        w.take_changes();
        let read = |core: &LiveCore, buf: &wgpu::Buffer, bytes: u64| -> Vec<u8> {
            let d = &core.g.device;
            let to = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let mut e = d.create_command_encoder(&Default::default());
            e.copy_buffer_to_buffer(buf, 0, &to, 0, bytes);
            core.g.queue.submit([e.finish()]);
            to.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            d.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let bytes = to.slice(..).get_mapped_range().unwrap().to_vec();
            bytes
        };
        // Each table's bytes and its uniform copy's, over what both hold.
        let pairs = |core: &LiveCore| -> Vec<(Vec<u8>, Vec<u8>)> {
            let up = &core.sc.up;
            let copies = up.uniform.as_ref().expect("the uniform tables' buffers");
            [&up.bg_parts.texinfo, &up.objects, &up.instances, &up.media].into_iter().zip(copies).map(|(t, c)| {
                let n = t.size().min(c.size()) & !3;
                (read(core, t, n), read(core, c, n))
            }).collect()
        };
        core.render(1, |_, _| {});
        let first = pairs(&core);
        for (k, (table, copy)) in first.iter().enumerate() {
            assert!(!table.is_empty() && table == copy, "table {k} and its uniform buffer differ after a frame");
        }
        assert!(first[1].0.iter().any(|b| *b != 0), "the objects' table has the scene's objects");
        // The panel moves in place: its object's entry and its instance's record change, and the next frame's
        // uniform buffers have them.
        w.set_transform(11, Vec3::new(0.3, 0.2, 0.75), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        assert!(!core.commit(&mut w).unwrap(), "moved in place");
        core.render(1, |_, _| {});
        let moved = pairs(&core);
        for (k, (table, copy)) in moved.iter().enumerate() {
            assert!(table == copy, "table {k} and its uniform buffer differ after an edit");
        }
        assert!(moved[1].1 != first[1].1 || moved[2].1 != first[2].1, "the move reached the uniform buffers");
    }
}
