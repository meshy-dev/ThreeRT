//! ThreeRT Playground: a native macOS editor client for ThreeRT scenes and games. One window, one wgpu device:
//!
//!   * PocketJS UI (Solid TSX in playground/ui, Unreal-Editor-style chrome): toolbar, outliner, details, content
//!     browser, output log, acceptance; laid out and drawn by PocketJS's wgpu renderer into the window,
//!   * the threert real-time renderer (LiveCore: HW-RT path tracing, temporal accumulation, denoise, UE post) drawn
//!     into a PocketJS compositor surface (the viewport) inside that layout, with UE5 reference A/B,
//!   * the scene itself: the opened scene / game bundle runs in the renderer's embedded JS runtime (GameSession: its
//!     code builds the scene, edits and gameplay change it in place, its frame script animates it in Play),
//!   * a Node backend (tools/playground-backend.ts): catalog, editor model (selection, details, undo) and the Play
//!     flow, on top of the session's outline / inspect / edit / event; JSON lines over stdio.
//!
//!   threert-playground [--scene <id>] [--tour | --uitest] [--root <repo>] [--ui <dir with main.js/main.pak>]
//!
//! Viewport: LMB drag orbit · RMB drag look (+WASD/QE fly) · MMB / Shift-drag pan · wheel or pinch dolly · click select
//! (in Play: click interacts) · F focus selection · Home scene camera · G hide outline · Alt+P play/stop ·
//! Esc stop / deselect · Cmd+Z / Cmd+Shift+Z undo / redo.

mod output;
mod ui;
mod viewport;

use threert_render::gpu;
use threert_render::live::{next_spp, Camera, LiveCore};
use threert_render::script::GameSession;
use anyhow::{Context, Result, anyhow};
use glam::Vec3;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use viewport::{Compare, Viewport};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Fullscreen, Theme, Window, WindowId};

const FPS: u32 = 60;
const RENDER: (u32, u32) = (1280, 720);
const THUMB: (u32, u32) = (320, 180);

struct Args {
    root: PathBuf,
    ui_dir: PathBuf,
    node: String,
    scene: Option<String>,
    /// Backend automation flags passed through (--tour, --uitest).
    extra: Vec<String>,
}

fn parse_args() -> Result<Args> {
    let argv: Vec<String> = std::env::args().collect();
    let flag = |k: &str| argv.iter().position(|a| a == k).and_then(|i| argv.get(i + 1)).cloned();
    let exe = std::env::current_exe().unwrap_or_default();
    let root = flag("--root").or_else(|| std::env::var("THREERT_ROOT").ok()).map(PathBuf::from).or_else(|| exe.ancestors().find(|a| a.join("tools/playground-backend.ts").exists()).map(Path::to_path_buf)).context("repo root not found (pass --root)")?;
    // Inside an .app bundle the UI ships in Contents/Resources/ui; in the source tree it is playground/ui/dist.
    let bundled = exe.parent().map(|p| p.join("../Resources/ui")).filter(|p| p.join("main.js").exists());
    let ui_dir = flag("--ui").map(PathBuf::from).or(bundled).unwrap_or_else(|| root.join("playground/ui/dist"));
    let node = flag("--node")
        .or_else(|| std::env::var("THREERT_NODE").ok())
        .or_else(|| exe.parent().and_then(|p| std::fs::read_to_string(p.join("../Resources/node-path")).ok()).map(|s| s.trim().to_string()))
        .unwrap_or_else(|| "node".into());
    let extra = argv.iter().filter(|a| matches!(a.as_str(), "--tour" | "--uitest")).cloned().collect();
    Ok(Args { root, ui_dir, node, scene: flag("--scene"), extra })
}

// ------------------------------------------------------------------------------------------------ backend link

struct Backend {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Value>,
}

impl Backend {
    fn spawn(a: &Args) -> Result<Backend> {
        let mut cmd = Command::new(&a.node);
        cmd.arg(a.root.join("tools/playground-backend.ts")).current_dir(&a.root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        if let Some(s) = &a.scene {
            cmd.args(["--scene", s]);
        }
        for f in &a.extra {
            cmd.arg(f);
        }
        let mut child = cmd.spawn().with_context(|| format!("spawn backend ({} tools/playground-backend.ts)", a.node))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let v = serde_json::from_str::<Value>(&line).unwrap_or_else(|_| json!({ "type": "log", "level": "info", "source": "backend", "text": line }));
                if tx.send(v).is_err() {
                    break;
                }
            }
            let _ = tx.send(json!({ "type": "exited" }));
        });
        Ok(Backend { child, stdin, rx })
    }
    fn send(&mut self, v: &Value) {
        let _ = writeln!(self.stdin, "{v}");
        let _ = self.stdin.flush();
    }
}

// ------------------------------------------------------------------------------------------------ camera

/// Editor camera: a pivot, a direction (yaw, pitch) from the pivot to the eye and a distance. Orbit turns around
/// the pivot, look turns around the eye (UE RMB), pan / fly move both.
#[derive(Clone, Copy)]
struct CamCtl {
    pivot: Vec3,
    yaw: f32,
    pitch: f32,
    dist: f32,
}

impl CamCtl {
    fn from_camera(c: &Camera, dist: f32) -> CamCtl {
        let dir = -c.fwd;
        CamCtl { pivot: c.pos + c.fwd * dist, yaw: dir.x.atan2(dir.z), pitch: dir.y.clamp(-1.0, 1.0).asin(), dist }
    }
    fn dir(&self) -> Vec3 {
        let cp = self.pitch.cos();
        Vec3::new(self.yaw.sin() * cp, self.pitch.sin(), self.yaw.cos() * cp)
    }
    fn pos(&self) -> Vec3 {
        self.pivot + self.dir() * self.dist
    }
    fn camera(&self, tan: f32) -> Camera {
        Camera::look_at(self.pos(), self.pivot, tan)
    }
    fn orbit(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.005;
        self.pitch = (self.pitch + dy * 0.004).clamp(-1.5, 1.5);
    }
    fn look(&mut self, dx: f32, dy: f32) {
        let eye = self.pos();
        self.yaw -= dx * 0.003;
        self.pitch = (self.pitch + dy * 0.003).clamp(-1.5, 1.5);
        self.pivot = eye - self.dir() * self.dist;
    }
    fn pan(&mut self, dx: f32, dy: f32, cam: &Camera) {
        let k = self.dist * 0.0015;
        self.pivot += -cam.right * dx * k + cam.up * dy * k;
    }
    fn dolly(&mut self, amount: f32) {
        self.dist = (self.dist * (-amount * 0.002).exp()).clamp(0.05, 500.0);
    }
}

/// Ease in-out (cubic): the curve for every camera / split animation.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) * 0.5 }
}

/// A camera move in progress (UE-style smooth focus, back to the scene camera, game camera changes).
struct Flight {
    from: Camera,
    to: CamCtl,
    /// Land exactly on this camera (the scene camera: pixel-aligned with the UE5 references).
    exact: Option<Camera>,
    t0: Instant,
    dur: f32,
}

impl Flight {
    fn t(&self) -> f32 {
        self.t0.elapsed().as_secs_f32() / self.dur
    }
    fn pose(&self, tan: f32) -> Camera {
        let to = self.exact.unwrap_or_else(|| self.to.camera(tan));
        let e = ease(self.t());
        let pos = self.from.pos.lerp(to.pos, e);
        let fwd = self.from.fwd.lerp(to.fwd, e).try_normalize().unwrap_or(to.fwd);
        Camera::look_at(pos, pos + fwd, tan)
    }
}

/// The split line sweeping in when a UE5 comparison opens (1 -> 0.5) or out before returning to Live.
struct Wipe {
    from: f32,
    to: f32,
    /// Starts after the camera has landed on the scene camera.
    t0: Instant,
    dur: f32,
    then_live: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Drag {
    Orbit,
    Look,
    Pan,
    Split,
}

struct Press {
    button: MouseButton,
    start: (f32, f32),
    moved: bool,
    drag: Drag,
}

// ------------------------------------------------------------------------------------------------ app state

struct State {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// Moved into LiveCore when the backend opens the first scene.
    pending_gpu: Option<gpu::Gpu>,
    core: Option<LiveCore>,
    /// The opened scene / game, running in the embedded JS runtime.
    session: Option<GameSession>,
    /// Play in Editor: the frame script runs and input goes to the game.
    playing: bool,
    /// Asset root (the repo) scene paths resolve against.
    assets: PathBuf,
    adapter: String,
    hwrt: bool,
    presets: PathBuf,
    ui: ui::PocketUi,
    vp: Viewport,
    present: wgpu::RenderPipeline,
    present_layout: wgpu::BindGroupLayout,
    last_target: Option<Arc<output::ProgramTarget>>,
    thumbs: Vec<(String, wgpu::TextureView)>,
    backend: Option<Backend>,

    cam: CamCtl,
    /// The live camera is exactly the scene camera until the user moves it (pixel-aligned UE5 A/B).
    cam_exact: bool,
    flight: Option<Flight>,
    wipe: Option<Wipe>,
    /// When the current selection was made (the outline fades in).
    selected_at: Instant,
    cursor: (f32, f32),
    press: Option<Press>,
    mods: ModifiersState,
    keys: HashSet<String>,
    viewport_focus: bool,

    spp: u32,
    spp_cap: u32,
    max_bounces: u32,
    /// Auto: the frame governor may lower bounces (down to 2) to hold 60 fps. Fixed: exactly `max_bounces`.
    auto_bounces: bool,
    frames: u32,
    stats_t: Instant,
    fps: f32,
    last: Instant,
    /// Content clock: frames are due every 1/FPS s regardless of the panel's refresh rate.
    next_frame: Instant,
    cpu_ms: f32,
    ui_ms: f32,
    reload_ms: f32,
    stats_due: bool,

    scene_id: String,
    selection: Option<String>,
    selection_dirty: bool,
    /// Automation: save the composed window to this path after the next frame.
    screenshot: Option<PathBuf>,
    quit: bool,
}

struct App {
    args: Args,
    state: Option<State>,
    error: Option<anyhow::Error>,
}

const PRESENT_WGSL: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(src, vec2<i32>(p.xy), 0);
}
"#;

/// UE5 Lumen reference thumbnails for the content browser (compositor surfaces `thumb:<id>`).
fn load_thumbs(root: &Path) -> Vec<(String, image::RgbaImage)> {
    let Ok(text) = std::fs::read_to_string(root.join("out/bundles/catalog.json")) else { return vec![] };
    let Ok(cat) = serde_json::from_str::<Value>(&text) else { return vec![] };
    cat["scenes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s["id"].as_str()?;
            let img = image::open(s["references"]["lumen"].as_str()?).ok()?;
            Some((id.to_string(), image::imageops::thumbnail(&img.to_rgba8(), THUMB.0, THUMB.1)))
        })
        .collect()
}

impl App {
    fn init(&mut self, el: &ActiveEventLoop) -> Result<()> {
        let a = &self.args;
        let (mw, mh) = el.primary_monitor().map(|m| { let s = m.size().to_logical::<f64>(m.scale_factor()); (s.width, s.height) }).unwrap_or((1728.0, 1117.0));
        let attrs = Window::default_attributes()
            .with_title("ThreeRT Playground")
            .with_inner_size(LogicalSize::new(1640f64.min(mw * 0.94), 1000f64.min(mh * 0.9)))
            .with_min_inner_size(LogicalSize::new(1100, 700))
            .with_theme(Some(Theme::Dark));
        let window = Arc::new(el.create_window(attrs)?);
        let t0 = Instant::now();
        let g = gpu::Gpu::new()?;
        let surface = g.instance.create_surface(window.clone())?;
        let caps = surface.get_capabilities(&g.adapter);
        // Renderer and UI both produce sRGB-encoded bytes: present through a non-sRGB format.
        let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).unwrap_or(caps.formats[0]);
        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        };
        surface.configure(&g.device, &config);
        let (device, queue) = (g.device.clone(), g.queue.clone());
        let ugpu = output::Gpu { device: device.clone(), queue: queue.clone() };
        let t_gpu = t0.elapsed().as_secs_f64() * 1e3;

        let thumbs = load_thumbs(&a.root);
        let mut surfaces = vec!["viewport".to_string()];
        surfaces.extend(thumbs.iter().map(|(id, _)| format!("thumb:{id}")));
        let bundle = std::fs::read_to_string(a.ui_dir.join("main.js")).with_context(|| format!("{}/main.js (run `bun playground/build.ts`)", a.ui_dir.display()))?;
        let pak = std::fs::read(a.ui_dir.join("main.pak")).with_context(|| format!("{}/main.pak", a.ui_dir.display()))?;
        let t1 = Instant::now();
        let ui = ui::PocketUi::new(&ugpu, &bundle, &pak, (config.width, config.height), scale, &surfaces)?;
        let t_ui = t1.elapsed().as_secs_f64() * 1e3;
        let thumbs = thumbs.into_iter().map(|(id, img)| (format!("thumb:{id}"), viewport::upload_rgba(&device, &queue, "thumbnail", &img))).collect::<Vec<_>>();

        let present_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            }],
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("present"), source: wgpu::ShaderSource::Wgsl(PRESENT_WGSL.into()) });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&present_layout)], immediate_size: 0 });
        let present = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), targets: &[Some(format.into())], compilation_options: Default::default() }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let vp = Viewport::new(&device, &queue);
        let backend = Backend::spawn(a)?;
        let (adapter, hwrt) = (g.adapter_info.name.clone(), g.hwrt);
        let presets = a.root.join("presets");
        let mut s = State {
            window, surface, config, device, queue, pending_gpu: Some(g), core: None, session: None, playing: false, assets: a.root.clone(), adapter, hwrt, presets,
            ui, vp, present, present_layout, last_target: None, thumbs, backend: Some(backend),
            cam: CamCtl { pivot: Vec3::ZERO, yaw: 0.0, pitch: 0.0, dist: 3.0 }, cam_exact: true, flight: None, wipe: None, selected_at: Instant::now(),
            cursor: (0.0, 0.0), press: None, mods: ModifiersState::empty(), keys: HashSet::new(), viewport_focus: false,
            spp: 1, spp_cap: 16, max_bounces: 4, auto_bounces: true, frames: 0, stats_t: Instant::now(), fps: 0.0, last: Instant::now(), next_frame: Instant::now(),
            cpu_ms: 0.0, ui_ms: 0.0, reload_ms: 0.0, stats_due: true,
            scene_id: String::new(), selection: None, selection_dirty: false, screenshot: None, quit: false,
        };
        let host = json!({
            "kind": "host", "adapter": s.adapter, "hwrt": s.hwrt, "render": [RENDER.0, RENDER.1], "fps": FPS,
            "thumbs": s.thumbs.iter().map(|(n, _)| n.trim_start_matches("thumb:")).collect::<Vec<_>>(),
        });
        s.ui.push(&host);
        s.log("info", "host", &format!("{} ({}), wgpu device {t_gpu:.0} ms, PocketJS UI boot {t_ui:.0} ms, {}x{} @{scale}x", s.adapter, if s.hwrt { "hardware ray tracing" } else { "BVH ray tracing" }, size.width, size.height));
        let hello = json!({ "type": "hello", "adapter": s.adapter, "hwrt": s.hwrt });
        s.send(&hello);
        self.state = Some(s);
        Ok(())
    }
}

impl State {
    fn send(&mut self, v: &Value) {
        if let Some(b) = &mut self.backend {
            b.send(v);
        }
    }

    fn log(&mut self, level: &str, source: &str, text: &str) {
        eprintln!("[{source}] {text}");
        self.ui.push(&json!({ "kind": "log", "level": level, "source": source, "text": text }));
    }

    fn scale(&self) -> f32 {
        self.window.scale_factor() as f32
    }

    /// Image-space (u, v) of a logical window point, if it lies on the rendered image.
    fn image_uv(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let core = self.core.as_ref()?;
        let [vx, vy, _, _] = self.ui.surface_rect("viewport")?;
        let s = self.scale();
        let r = self.vp.image_rect(core.aspect());
        let (u, v) = (((x - vx) * s - r[0]) / r[2], ((y - vy) * s - r[1]) / r[3]);
        ((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)).then_some((u, v))
    }

    fn camera_locked(&self) -> bool {
        self.vp.compare != Compare::Live
    }

    /// Back to the scene's own camera (the pose the UE5 references were rendered from); `fly` animates the move.
    fn home_camera(&mut self, fly: bool) {
        let Some(core) = &mut self.core else { return };
        let c = core.scene_camera();
        let dist = core.pick(0.5, 0.5).map(|(_, p)| (p - c.pos).length()).unwrap_or(3.0).max(0.2);
        let to = CamCtl::from_camera(&c, dist);
        if fly && core.cam != c {
            self.flight = Some(Flight { from: core.cam, to, exact: Some(c), t0: Instant::now(), dur: 0.45 });
        } else {
            self.flight = None;
            core.set_camera(c);
            self.cam = to;
            self.cam_exact = true;
        }
    }

    /// User camera input takes over from an animated move at its current pose.
    fn take_camera(&mut self) {
        if let (Some(f), Some(core)) = (self.flight.take(), &self.core) {
            let pose = f.pose(core.cam.tan_half_fov);
            let dist = f.to.dist.max(0.2);
            self.cam = CamCtl::from_camera(&pose, dist);
        }
        self.cam_exact = false;
    }

    fn focus_selection(&mut self) {
        let (Some(core), Some(id)) = (&self.core, &self.selection) else { return };
        let tris = core.node_tris(id);
        if tris.is_empty() {
            return;
        }
        let (mut lo, mut hi) = (Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY));
        for t in &tris {
            for v in t {
                lo = lo.min(Vec3::from(*v));
                hi = hi.max(Vec3::from(*v));
            }
        }
        let radius = ((hi - lo).length() * 0.5).max(0.05);
        let from = core.cam;
        let mut to = CamCtl::from_camera(&from, self.cam.dist);
        to.pivot = (lo + hi) * 0.5;
        to.dist = radius / from.tan_half_fov * 1.15;
        self.flight = Some(Flight { from, to, exact: None, t0: Instant::now(), dur: 0.5 });
        self.cam_exact = false;
        if self.camera_locked() {
            self.vp.compare = Compare::Live;
            self.wipe = None;
        }
    }

    /// Opens a scene / game bundle: setup in the embedded runtime, then the live view (the first one creates the
    /// renderer). `keepCamera`: re-open of the same scene (Stop, Revert) keeping the user's viewpoint.
    fn open_scene(&mut self, m: &Value) -> Result<()> {
        let bundle = PathBuf::from(m["bundle"].as_str().context("open: bundle")?);
        let keep = m["keepCamera"].as_bool().unwrap_or(false) && self.core.is_some();
        let t = Instant::now();
        self.session = None;
        self.playing = false;
        let session = match &mut self.core {
            Some(core) => {
                let cam = core.cam;
                let s = GameSession::open(&bundle, "playground", &self.assets, core)?;
                if keep {
                    core.set_camera(cam);
                }
                s
            }
            None => {
                let g = self.pending_gpu.take().context("gpu")?;
                let (s, mut core) = GameSession::start(&bundle, "playground", &self.assets, g, &self.presets, RENDER.0, RENDER.1)?;
                core.bounces = self.max_bounces;
                core.dynamic_res = true;
                self.core = Some(core);
                s
            }
        };
        let core = self.core.as_mut().unwrap();
        if !keep {
            core.bounces = 4;
            self.max_bounces = 4;
        }
        self.spp = 1;
        self.reload_ms = t.elapsed().as_secs_f32() * 1e3;
        self.scene_id = m["id"].as_str().unwrap_or_default().to_string();
        let opened = json!({
            "type": "opened", "id": self.scene_id, "info": session.host.info, "outline": session.host.outline()?, "state": session.host.state()?.0,
            "triangles": core.triangles, "setupMs": session.setup_ms, "ms": self.reload_ms,
        });
        self.session = Some(session);
        self.send(&opened);
        let refs = [m["refs"]["lumen"].as_str(), m["refs"]["pathtracer"].as_str(), m["refs"]["three"].as_str()];
        for e in self.vp.set_references(&self.device, &self.queue, refs) {
            self.log("warn", "renderer", &e);
        }
        if !self.vp.has_reference(self.vp.compare) {
            self.vp.compare = Compare::Live;
        }
        self.selection = None;
        self.selection_dirty = true;
        if !keep {
            self.wipe = None;
            self.home_camera(false);
        }
        let (tris, name) = (self.core.as_ref().unwrap().triangles, self.core.as_ref().unwrap().scene_name.clone());
        self.window.set_title(&format!("ThreeRT Playground — {name}"));
        let phases = self.core.as_ref().unwrap().load_timings().iter().map(|(k, v)| format!("{} {v:.0}", k.trim_end_matches("_ms"))).collect::<Vec<_>>().join(", ");
        self.log("info", "renderer", &format!("opened {} ({tris} triangles) in {:.0} ms ({phases})", self.scene_id, self.reload_ms));
        self.stats_due = true;
        Ok(())
    }

    /// The scene changed shape (rebuild): selection outline and stats follow.
    fn rebuilt(&mut self, t: Instant) {
        self.reload_ms = t.elapsed().as_secs_f32() * 1e3;
        self.selection_dirty = true;
    }

    fn inspect(&mut self, node: &str) -> Result<()> {
        let Some(s) = &self.session else { return Ok(()) };
        let msg = json!({ "type": "inspected", "node": node, "value": serde_json::from_str::<Value>(&s.host.inspect(node)?)? });
        self.send(&msg);
        Ok(())
    }

    fn backend_message(&mut self, m: Value) -> Result<()> {
        match m["type"].as_str().unwrap_or("") {
            "open" => {
                if let Err(e) = self.open_scene(&m) {
                    let msg = json!({ "type": "opened", "id": m["id"], "error": format!("{e:#}") });
                    self.send(&msg);
                    return Err(e);
                }
            }
            "edit" => {
                // Details panel edit of the live scene (editor world); the backend keeps the undo stack.
                let (Some(s), Some(core)) = (&mut self.session, &mut self.core) else { return Ok(()) };
                let node = m["node"].as_str().unwrap_or_default().to_string();
                let path = m["path"].as_str().unwrap_or_default();
                let t = Instant::now();
                let res = s.edit(core, &node, path, &m["value"]);
                let outline = (path == "node.visible").then(|| s.host.outline()).transpose()?;
                match res {
                    Ok(rebuilt) => {
                        if rebuilt {
                            self.rebuilt(t);
                        }
                    }
                    Err(e) => self.log("warn", "editor", &format!("{e:#}")),
                }
                if let Some(rows) = outline {
                    self.send(&json!({ "type": "outline", "rows": rows }));
                }
                if m["inspect"].as_bool().unwrap_or(true) {
                    self.inspect(&node)?;
                }
            }
            "inspect" => self.inspect(m["node"].as_str().unwrap_or_default())?,
            "play" => self.playing = self.session.is_some(),
            "stop" => self.playing = false,
            "event" => {
                // A game event (Play): its handler commits, the frame script shows the new state.
                let (Some(s), Some(core)) = (&mut self.session, &mut self.core) else { return Ok(()) };
                let kind = m["event"].as_str().unwrap_or_default().to_string();
                let t = Instant::now();
                let cam = core.scene_camera();
                let reply = match s.event(core, &kind, m["payload"].clone()) {
                    Ok(e) => {
                        let (state, _) = s.host.state()?;
                        json!({ "type": "tx", "event": kind, "label": m["label"], "log": e.log, "seq": e.seq, "ms": e.ms, "state": state })
                    }
                    Err(e) => json!({ "type": "tx", "event": kind, "label": m["label"], "error": format!("{e:#}") }),
                };
                if core.scene_camera() != cam {
                    // The game moved its camera (a viewpoint switch): the viewport follows.
                    self.home_camera(true);
                }
                if t.elapsed().as_secs_f32() > 0.05 {
                    self.rebuilt(t);
                }
                self.selection_dirty = true;
                self.send(&reply);
            }
            "select" => {
                let sel = m["node"].as_str().map(str::to_string);
                if sel != self.selection {
                    self.selected_at = Instant::now();
                }
                self.selection = sel;
                self.selection_dirty = true;
            }
            "focus" => self.focus_selection(),
            "ui" => {
                let mut v = m["value"].clone();
                v["kind"] = "app".into();
                self.ui.push(&v);
            }
            "log" => {
                let v = json!({ "kind": "log", "level": m["level"].as_str().unwrap_or("info"), "source": m["source"].as_str().unwrap_or("backend"), "text": m["text"] });
                self.ui.push(&v);
            }
            "host" => self.host_command(&m["cmd"]),
            "screenshot" => self.screenshot = m["path"].as_str().map(PathBuf::from),
            "quit" => self.quit = true,
            "exited" => {
                self.backend = None;
                self.log("error", "host", "backend exited; scene editing and gameplay are unavailable");
            }
            _ => {}
        }
        Ok(())
    }

    /// Commands the renderer handles itself (from the UI, the keyboard, or a backend tour); the rest go to the backend.
    fn host_command(&mut self, c: &Value) {
        let name = c["name"].as_str().unwrap_or("");
        let core = self.core.as_mut();
        match (name, core) {
            ("viewMode", Some(core)) => core.view_mode = c["mode"].as_u64().unwrap_or(0) as u32 % 6,
            // A/B switch for review (docs/REVIEW-21-26.md): motion blur.
            ("toggleMotionBlur", Some(core)) => {
                core.motion_blur_off = !core.motion_blur_off;
                let on = !core.motion_blur_off;
                self.log("info", "viewport", if on { "Motion blur ON (scene setting; key N toggles)" } else { "Motion blur OFF (key N toggles)" });
            }
            ("bounces", Some(core)) => {
                // 0 = auto (governed, up to 4); 1..8 = fixed.
                let v = c["value"].as_u64().unwrap_or(0) as u32;
                self.auto_bounces = v == 0;
                self.max_bounces = if v == 0 { 4 } else { v.min(8) };
                core.bounces = self.max_bounces;
            }
            ("sppCap", _) => self.spp_cap = c["value"].as_u64().unwrap_or(16).clamp(1, 16) as u32,
            ("resetHistory", Some(core)) => core.reset_history(),
            ("compare", _) => {
                let mode = c["mode"].as_str().and_then(Compare::parse).unwrap_or(Compare::Live);
                if self.vp.has_reference(mode) {
                    let was = self.vp.compare;
                    if mode == Compare::Live {
                        // Sweep the reference out, then hand the viewport back.
                        if was != Compare::Live {
                            self.wipe = Some(Wipe { from: self.vp.split, to: 1.0, t0: Instant::now(), dur: 0.3, then_live: true });
                        }
                    } else {
                        self.vp.compare = mode;
                        if was == Compare::Live || self.wipe.as_ref().is_some_and(|w| w.then_live) {
                            // Fly to the scene camera, then sweep the UE5 reference in from the right.
                            self.home_camera(true);
                            let delay = if self.flight.is_some() { 0.45 } else { 0.0 };
                            self.vp.split = 1.0;
                            self.wipe = Some(Wipe { from: 1.0, to: 0.5, t0: Instant::now() + Duration::from_secs_f32(delay), dur: 0.55, then_live: false });
                        }
                    }
                } else {
                    self.log("warn", "renderer", &format!("no UE5 reference for {} ({})", self.scene_id, mode.name()));
                }
            }
            ("split", _) => {
                self.wipe = None;
                self.vp.split = c["value"].as_f64().unwrap_or(0.5).clamp(0.02, 0.98) as f32;
            }
            ("outline", _) => self.vp.outline = c["on"].as_bool().unwrap_or(!self.vp.outline),
            ("homeCamera", _) => self.home_camera(true),
            ("focus", _) => self.focus_selection(),
            ("fullscreen", _) => {
                let fs = self.window.fullscreen().is_none().then_some(Fullscreen::Borderless(None));
                self.window.set_fullscreen(fs);
            }
            ("quit", _) => self.quit = true,
            // Automation (tours / UI tests): the same input path as the mouse.
            ("uiWheel", _) => self.ui.push(&json!({ "kind": "wheel", "x": c["x"], "y": c["y"], "dy": c["dy"] })),
            ("uiClick", _) | ("uiDrag", _) => {
                let Some([x, y, w, h]) = self.ui.control_bounds(c["control"].as_str().unwrap_or("")) else {
                    let msg = json!({ "type": "uiResult", "control": c["control"], "ok": false });
                    self.send(&msg);
                    return;
                };
                let (cx, cy) = (x + w * 0.5, y + h * 0.5);
                self.pointer_move(cx, cy);
                self.pointer_button(MouseButton::Left, true);
                let dx = c["dx"].as_f64().unwrap_or(0.0) as f32;
                for k in 1..=8 {
                    self.pointer_move(cx + dx * k as f32 / 8.0, cy);
                }
                self.pointer_button(MouseButton::Left, false);
                let msg = json!({ "type": "uiResult", "control": c["control"], "ok": true, "at": [cx, cy] });
                self.send(&msg);
            }
            ("windowSize", _) => {
                let _ = self.window.request_inner_size(LogicalSize::new(c["w"].as_f64().unwrap_or(1400.0), c["h"].as_f64().unwrap_or(880.0)));
            }
            ("viewportClickNode", _) => {
                // Click the middle of an object's on-screen bounds (UI tests).
                let Some(core) = &self.core else { return };
                let tris = core.node_tris(c["node"].as_str().unwrap_or_default());
                if tris.is_empty() {
                    return;
                }
                let center = tris.iter().flat_map(|t| t.iter().map(|v| Vec3::from(*v))).fold(Vec3::ZERO, |a, v| a + v) / (tris.len() * 3) as f32;
                let cam = core.cam;
                let d = center - cam.pos;
                let z = d.dot(cam.fwd);
                if z <= 0.0 {
                    return;
                }
                let u = (d.dot(cam.right) / z / (cam.tan_half_fov * core.aspect()) + 1.0) * 0.5;
                let v = (1.0 - d.dot(cam.up) / z / cam.tan_half_fov) * 0.5;
                self.host_command(&json!({ "name": "viewportClick", "u": u, "v": v }));
                return;
            }
            ("viewportClick", _) => {
                let (Some(core), Some([vx, vy, _, _])) = (&self.core, self.ui.surface_rect("viewport")) else { return };
                let (s, r) = (self.scale(), self.vp.image_rect(core.aspect()));
                let (u, v) = (c["u"].as_f64().unwrap_or(0.5) as f32, c["v"].as_f64().unwrap_or(0.5) as f32);
                let (x, y) = (vx + (r[0] + u * r[2]) / s, vy + (r[1] + v * r[3]) / s);
                self.pointer_move(x, y);
                self.pointer_button(MouseButton::Left, true);
                self.pointer_button(MouseButton::Left, false);
            }
            _ => {
                let msg = json!({ "type": "cmd", "cmd": c });
                self.send(&msg);
                return;
            }
        }
        self.stats_due = true;
    }

    // -------------------------------------------------------------------------------------------- input

    fn pointer_move(&mut self, x: f32, y: f32) {
        let (dx, dy) = (x - self.cursor.0, y - self.cursor.1);
        self.cursor = (x, y);
        let Some(p) = &mut self.press else {
            self.ui.pointer(json!({ "kind": "move", "x": x, "y": y }));
            return;
        };
        if (x - p.start.0).hypot(y - p.start.1) > 3.0 {
            p.moved = true;
        }
        if !p.moved {
            return;
        }
        let drag = p.drag;
        if drag == Drag::Split {
            if let Some((u, _)) = self.image_uv(x, y.max(0.0)).or_else(|| self.image_uv(x, self.cursor.1)) {
                self.wipe = None;
                self.vp.split = u.clamp(0.02, 0.98);
                self.stats_due = true;
            }
            return;
        }
        if self.camera_locked() {
            return;
        }
        self.take_camera();
        let cam = self.core.as_ref().map(|c| c.cam);
        match (drag, cam) {
            (Drag::Orbit, _) => self.cam.orbit(dx, dy),
            (Drag::Look, _) => self.cam.look(dx, dy),
            (Drag::Pan, Some(cam)) => self.cam.pan(dx, dy, &cam),
            _ => return,
        }
        self.cam_exact = false;
    }

    fn pointer_button(&mut self, button: MouseButton, down: bool) {
        let (x, y) = self.cursor;
        if down {
            if self.press.is_some() {
                return;
            }
            if self.ui.over("viewport", x, y) {
                let split = self.vp.compare != Compare::Live;
                let drag = match button {
                    MouseButton::Left if split => Drag::Split,
                    MouseButton::Left if self.mods.shift_key() => Drag::Pan,
                    MouseButton::Left => Drag::Orbit,
                    MouseButton::Right if self.mods.shift_key() => Drag::Pan,
                    MouseButton::Right => Drag::Look,
                    _ => Drag::Pan,
                };
                self.press = Some(Press { button, start: (x, y), moved: false, drag });
                self.viewport_focus = true;
                self.ui.pointer(json!({ "kind": "cancel" }));
                return;
            }
            self.viewport_focus = false;
        } else if let Some(p) = self.press.take_if(|p| p.button == button) {
            if !p.moved && button == MouseButton::Left {
                self.click(x, y);
            }
            return;
        }
        if button == MouseButton::Left {
            self.ui.pointer(json!({ "kind": "button", "x": x, "y": y, "down": down }));
        }
    }

    /// Viewport click: pick the surface under the cursor; the backend selects it (editor) or sends it to the game (Play).
    fn click(&mut self, x: f32, y: f32) {
        let Some((u, v)) = self.image_uv(x, y) else { return };
        let Some(core) = &self.core else { return };
        let hit = core.pick(u, v);
        let msg = match hit {
            Some((node, p)) => json!({ "type": "pick", "node": node, "point": [p.x, p.y, p.z], "shift": self.mods.shift_key() }),
            None => json!({ "type": "pick", "node": null }),
        };
        self.send(&msg);
    }

    fn key(&mut self, name: String, down: bool, repeat: bool) {
        if !down {
            self.keys.remove(&name);
            return;
        }
        self.keys.insert(name.clone());
        if repeat {
            return;
        }
        let (cmd, alt, shift) = (self.mods.super_key(), self.mods.alt_key(), self.mods.shift_key());
        let c = |n: &str| json!({ "name": n });
        match name.as_str() {
            "Z" if cmd && shift => self.host_command(&c("redo")),
            "Z" | "Y" if cmd => self.host_command(&c(if name == "Z" { "undo" } else { "redo" })),
            "Q" | "W" if cmd => self.quit = true,
            "P" if alt || cmd => self.host_command(&c("togglePlay")),
            "Escape" => self.host_command(&c("escape")),
            "F" if !cmd => self.focus_selection(),
            "G" if !cmd => self.host_command(&json!({ "name": "outline" })),
            "Home" => self.home_camera(true),
            "F11" => self.host_command(&c("fullscreen")),
            "1" | "2" | "3" | "4" | "5" | "6" if alt => self.host_command(&json!({ "name": "viewMode", "mode": name.parse::<u32>().unwrap() - 1 })),
            "N" if !cmd => self.host_command(&c("toggleMotionBlur")),
            _ if !cmd => {
                // Game input while playing (the backend ignores keys when no game runs).
                let msg = json!({ "type": "key", "key": name });
                self.send(&msg);
            }
            _ => {}
        }
    }

    // -------------------------------------------------------------------------------------------- frame

    fn frame(&mut self) -> Result<()> {
        // Content (UI + path tracing) runs at 60 fps; faster panels re-present the last composed frame.
        let t_frame = Instant::now();
        let period = Duration::from_secs_f64(1.0 / FPS as f64);
        if t_frame + Duration::from_millis(2) < self.next_frame {
            return self.reshow();
        }
        self.next_frame = if t_frame > self.next_frame + period { t_frame + period } else { self.next_frame + period };
        let dt = self.last.elapsed().as_secs_f32().min(0.1);
        self.last = t_frame;

        let mut msgs = vec![];
        if let Some(b) = &self.backend {
            while let Ok(m) = b.rx.try_recv() {
                msgs.push(m);
            }
        }
        // Coalesce scene reloads (edits while scrubbing): only the newest version is loaded.
        let last_reload = msgs.iter().rposition(|m| m["type"] == "reload");
        for (i, m) in msgs.into_iter().enumerate() {
            if m["type"] == "reload" && Some(i) != last_reload {
                continue;
            }
            if let Err(e) = self.backend_message(m) {
                self.log("error", "renderer", &format!("{e:#}"));
            }
        }

        let t_ui = Instant::now();
        if self.stats_due || self.stats_t.elapsed() >= Duration::from_millis(250) {
            self.push_stats();
        }
        for c in self.ui.advance()? {
            if c["type"] == "cmd" {
                self.host_command(&c);
            }
        }
        self.ui_ms = t_ui.elapsed().as_secs_f32() * 1e3;

        // Fly (UE: hold RMB + WASD/QE; also when the viewport has focus).
        if (self.viewport_focus || self.press.as_ref().is_some_and(|p| p.drag == Drag::Look)) && !self.camera_locked() {
            if let Some(core) = &self.core {
                let speed = self.cam.dist.max(0.5) * 0.9 * dt * if self.mods.shift_key() { 3.0 } else { 1.0 };
                let (f, r) = (core.cam.fwd, core.cam.right);
                let mut v = Vec3::ZERO;
                for (k, d) in [("W", f), ("S", -f), ("D", r), ("A", -r), ("E", Vec3::Y), ("Q", -Vec3::Y)] {
                    if self.keys.contains(k) && !self.mods.super_key() {
                        v += d;
                    }
                }
                if v != Vec3::ZERO {
                    self.take_camera();
                    self.cam.pivot += v * speed;
                }
            }
        }

        // Viewport texture at the node's exact physical size.
        let scale = self.scale();
        let rect = self.ui.surface_rect("viewport");
        let logical = rect.map(|[_, _, w, h]| (w.ceil().max(1.0) as u32, h.ceil().max(1.0) as u32));
        if let Some((lw, lh)) = logical {
            self.vp.ensure_target(&self.device, (((lw as f32) * scale).round() as u32, ((lh as f32) * scale).round() as u32));
        }

        if let Some(w) = &self.wipe {
            let now = Instant::now();
            let t = if now < w.t0 { 0.0 } else { (now - w.t0).as_secs_f32() / w.dur };
            self.vp.split = w.from + (w.to - w.from) * ease(t);
            if t >= 1.0 {
                if w.then_live {
                    self.vp.compare = Compare::Live;
                    self.vp.split = 0.5;
                }
                self.wipe = None;
            }
            self.stats_due = true;
        }
        self.vp.outline_alpha = ease(self.selected_at.elapsed().as_secs_f32() / 0.2);
        // Scene time (animated shading, particles) runs every frame, editing or playing. In Play the frame script's
        // clock sets it (GameSession::frame), so a burst the script fires at `f.time` starts then.
        if let Some(core) = &mut self.core {
            core.advance_time(dt);
        }
        if let (true, Some(session), Some(core)) = (self.playing, &mut self.session, &mut self.core) {
            // Play: the game's frame script (its Tick) animates the scene; the viewport follows the game camera.
            let cam = core.scene_camera();
            let t = Instant::now();
            match session.frame(core, dt) {
                Ok(rebuilt) => {
                    if rebuilt {
                        self.rebuilt(t);
                    }
                }
                Err(e) => {
                    self.playing = false;
                    self.log("error", "game", &format!("frame script stopped: {e:#}"));
                }
            }
            if self.core.as_ref().is_some_and(|c| c.scene_camera() != cam) {
                self.home_camera(false);
            }
        }
        self.vp.outline_alpha = ease(self.selected_at.elapsed().as_secs_f32() / 0.2);
        if let Some(core) = &mut self.core {
            let tan = core.cam.tan_half_fov;
            if let Some(f) = self.flight.take_if(|f| f.t() >= 1.0) {
                self.cam = f.to;
                self.cam_exact = f.exact.is_some();
                core.set_camera(f.exact.unwrap_or_else(|| f.to.camera(tan)));
            } else if let Some(f) = &self.flight {
                core.set_camera(f.pose(tan));
            } else if !self.cam_exact {
                core.set_camera(self.cam.camera(tan));
            }
            if self.selection_dirty {
                let tris = self.selection.as_deref().map(|id| core.node_tris(id));
                self.vp.set_selection(&self.device, tris.as_deref());
                self.selection_dirty = false;
            }
            self.vp.prepare(&self.queue, (core.w, core.h), &core.cam, core.aspect());
            let (vp, device, aspect) = (&self.vp, &self.device, core.aspect());
            core.render(self.spp, |e, out| vp.encode(device, e, out, aspect));
            // Frame governor (UE dynamic resolution first): render scale toward the budget, full resolution before extra
            // samples per frame; at the lowest scale and 1 spp still over budget, trade bounces (and give them back).
            core.govern_resolution(FPS);
            self.spp = if core.render_scale < 1.0 { 1 } else { next_spp(self.spp, core.gpu_ms, FPS).min(self.spp_cap) };
            if self.auto_bounces && core.frame % 30 == 1 {
                let budget = 1000.0 / FPS as f32;
                if self.spp == 1 && core.render_scale <= core.min_scale && core.gpu_ms > budget * 0.92 && core.bounces > 2 {
                    core.bounces -= 1;
                } else if core.gpu_ms < budget * 0.55 && core.bounces < self.max_bounces {
                    core.bounces += 1;
                }
            }
        } else {
            let mut e = self.device.create_command_encoder(&Default::default());
            self.vp.clear(&mut e);
            self.queue.submit([e.finish()]);
        }

        // UI (+ viewport and thumbnails as compositor surfaces) -> window.
        if let (Some(t), Some(l)) = (&self.vp.target, logical) {
            let view = t.view.clone();
            self.ui.bind_surface("viewport", &view, l);
        }
        for (name, view) in &self.thumbs {
            self.ui.bind_surface(name, view, (THUMB.0 / 2, THUMB.1 / 2));
        }
        let mut e = self.device.create_command_encoder(&Default::default());
        let target = self.ui.encode(&self.device, &mut e)?;
        self.queue.submit([e.finish()]);
        self.last_target = Some(target.clone());
        self.reshow()?;

        if let Some(path) = self.screenshot.take() {
            save_target(&self.device, &self.queue, &target, &path)?;
            let msg = json!({ "type": "screenshot", "path": path });
            self.send(&msg);
        }

        self.cpu_ms = t_frame.elapsed().as_secs_f32() * 1e3;
        self.frames += 1;
        Ok(())
    }

    fn push_stats(&mut self) {
        let el = self.stats_t.elapsed().as_secs_f32();
        if el >= 0.25 {
            self.fps = self.frames as f32 / el;
            self.frames = 0;
            self.stats_t = Instant::now();
        }
        self.stats_due = false;
        let vp = self.vp.target.as_ref().map_or((0, 0), |t| t.size);
        let mut st = json!({
            "kind": "stats", "fps": self.fps, "cpuMs": self.cpu_ms, "uiMs": self.ui_ms, "spp": self.spp, "sppCap": self.spp_cap,
            "maxBounces": self.max_bounces, "autoBounces": self.auto_bounces, "compare": self.vp.compare.name(), "split": self.vp.split, "outline": self.vp.outline,
            "locked": self.camera_locked(), "sceneCamera": self.cam_exact, "vp": [vp.0, vp.1], "reloadMs": self.reload_ms,
            "refs": [self.vp.has_reference(Compare::SplitLumen), self.vp.has_reference(Compare::SplitPathTracer), self.vp.has_reference(Compare::SplitThree)],
            "win": [self.ui.size.0 as f32 / self.ui.scale, self.ui.size.1 as f32 / self.ui.scale],
        });
        if let Some(core) = &self.core {
            let m = core.meta();
            for k in ["gpu_ms", "history", "bounces", "mode", "triangles", "frame", "width", "height", "scale", "scene"] {
                st[k] = m[k].clone();
            }
            st["cam"] = json!(core.cam.pos.to_array());
        }
        self.ui.push(&st);
        let fwd = json!({ "type": "stats", "fps": self.fps, "gpu_ms": st["gpu_ms"], "history": st["history"], "spp": self.spp, "cpu_ms": self.cpu_ms, "ui_ms": self.ui_ms, "win": st["win"], "compare": st["compare"], "mode": st["mode"] });
        self.send(&fwd);
    }

    /// Presents the last composed window image (every vsync; content frames replace it at 60 fps).
    fn reshow(&mut self) -> Result<()> {
        let Some(target) = &self.last_target else { return Ok(()) };
        let tex = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Occluded | wgpu::CurrentSurfaceTexture::Timeout => return Ok(()),
            _ => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
        };
        if target.size != (self.config.width, self.config.height) {
            return Ok(()); // resized; the next content frame recomposes at the new size
        }
        let view = tex.texture.create_view(&Default::default());
        let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &self.present_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&target.view) }] });
        let mut e = self.device.create_command_encoder(&Default::default());
        {
            let mut rp = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(&self.present);
            rp.set_bind_group(0, &bg, &[]);
            rp.draw(0..3, 0..1);
        }
        self.queue.submit([e.finish()]);
        self.window.pre_present_notify();
        self.queue.present(tex);
        Ok(())
    }
}

fn save_target(d: &wgpu::Device, q: &wgpu::Queue, target: &output::ProgramTarget, path: &Path) -> Result<()> {
    let (w, h) = target.size;
    let stride = (w * 4).div_ceil(256) * 256;
    let buf = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: (stride * h) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut e = d.create_command_encoder(&Default::default());
    e.copy_texture_to_buffer(
        target.texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(stride), rows_per_image: Some(h) } },
        wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
    );
    q.submit([e.finish()]);
    buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    d.poll(wgpu::PollType::wait_indefinitely())?;
    let data = buf.slice(..).get_mapped_range()?.to_vec();
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h {
        px.extend_from_slice(&data[(row * stride) as usize..(row * stride + w * 4) as usize]);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    image::RgbaImage::from_raw(w, h, px).ok_or_else(|| anyhow!("screenshot buffer"))?.save(path)?;
    Ok(())
}

fn key_name(k: &Key) -> Option<String> {
    Some(match k {
        Key::Character(c) => c.to_uppercase(),
        Key::Named(n) => match n {
            NamedKey::Escape => "Escape".into(),
            NamedKey::Space => "Space".into(),
            NamedKey::Home => "Home".into(),
            NamedKey::Enter => "Enter".into(),
            NamedKey::Delete | NamedKey::Backspace => "Delete".into(),
            NamedKey::ArrowLeft => "Left".into(),
            NamedKey::ArrowRight => "Right".into(),
            NamedKey::ArrowUp => "Up".into(),
            NamedKey::ArrowDown => "Down".into(),
            NamedKey::F11 => "F11".into(),
            _ => return None,
        },
        _ => return None,
    })
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.state.is_none() {
            if let Err(e) = self.init(el) {
                self.error = Some(e);
                el.exit();
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let Some(s) = self.state.as_mut() else { return };
        let scale = s.scale();
        match event {
            WindowEvent::CloseRequested => s.quit = true,
            WindowEvent::Resized(size) => {
                if (size.width.max(1), size.height.max(1)) == (s.config.width, s.config.height) {
                    return;
                }
                s.config.width = size.width.max(1);
                s.config.height = size.height.max(1);
                s.surface.configure(&s.device, &s.config);
                if let Err(e) = s.ui.resize((s.config.width, s.config.height), scale) {
                    eprintln!("resize: {e:#}");
                }
            }
            WindowEvent::ScaleFactorChanged { .. } => {
                let size = s.window.inner_size();
                let _ = s.ui.resize((size.width.max(1), size.height.max(1)), scale);
            }
            WindowEvent::Focused(false) => {
                s.keys.clear();
                s.press = None;
                s.ui.pointer(json!({ "kind": "cancel" }));
            }
            WindowEvent::ModifiersChanged(m) => s.mods = m.state(),
            WindowEvent::CursorMoved { position, .. } => s.pointer_move(position.x as f32 / scale, position.y as f32 / scale),
            WindowEvent::MouseInput { state, button, .. } => s.pointer_button(button, state == ElementState::Pressed),
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 40.0,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / scale,
                };
                if s.ui.over("viewport", s.cursor.0, s.cursor.1) {
                    if !s.camera_locked() {
                        s.take_camera();
                        s.cam.dolly(dy * scale);
                    }
                } else {
                    // Panels scroll in the UI (outliner, details).
                    s.ui.push(&json!({ "kind": "wheel", "x": s.cursor.0, "y": s.cursor.1, "dy": dy }));
                }
            }
            // Trackpad pinch: dolly toward the pivot (delta > 0 = magnify).
            WindowEvent::PinchGesture { delta, .. } => {
                if s.ui.over("viewport", s.cursor.0, s.cursor.1) && !s.camera_locked() {
                    s.take_camera();
                    s.cam.dist = (s.cam.dist * (-(delta as f32) * 1.6).exp()).clamp(0.05, 500.0);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(name) = key_name(&event.logical_key) {
                    s.key(name, event.state == ElementState::Pressed, event.repeat);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = s.frame() {
                    s.log("error", "host", &format!("frame: {e:#}"));
                }
            }
            _ => {}
        }
        if s.quit {
            if let Some(mut b) = s.backend.take() {
                b.send(&json!({ "type": "quit" }));
                let t = Instant::now();
                while t.elapsed() < Duration::from_millis(500) && matches!(b.child.try_wait(), Ok(None)) {
                    std::thread::sleep(Duration::from_millis(10));
                }
                let _ = b.child.kill();
            }
            el.exit();
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if let Some(s) = &self.state {
            s.window.request_redraw();
            el.set_control_flow(ControlFlow::Poll);
        }
    }
}

/// PocketJS reports guest (JS) errors through `log`; print warnings and errors to stderr.
struct StderrLog;
impl log::Log for StderrLog {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) {
            eprintln!("[{}] {}: {}", r.level(), r.target(), r.args());
        }
    }
    fn flush(&self) {}
}

fn main() -> Result<()> {
    let _ = log::set_logger(&StderrLog).map(|()| log::set_max_level(log::LevelFilter::Warn));
    let args = parse_args()?;
    let el = EventLoop::new()?;
    let mut app = App { args, state: None, error: None };
    el.run_app(&mut app)?;
    if let Some(e) = app.error {
        return Err(e);
    }
    Ok(())
}
