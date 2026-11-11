//! ThreeRT in a browser (docs/RUNTIME.md "The engine in a browser"): the engine's library as a wasm32 module. The
//! page's own JavaScript runs the game (the same bundle the native runtime loads in QuickJS: game code, the scene
//! binding and the node host); this module holds what the native process holds around the script: the engine's world,
//! the voxels, the physics, and the live view, on WebGPU.
//!
//! Two sets of exports:
//!  - The engine API, one export for each native function of `globalThis.__engine` (renderer/src/script.rs
//!    `install_engine`), under the same name, doing the same thing (renderer/src/api.rs is the code of both). A
//!    descriptor crosses as JSON text, an array as a typed array copied into the module's memory. www/engine.js makes
//!    `__engine` of them.
//!  - The session, what `GameSession` and the native window do around the script: `start` (the GPU, the view of the
//!    world the setup built), `frameBegin` / `frameEnd` around the node host's `frame`, `click` (the pick and the
//!    event's payload), `commit`.
//!
//! Nothing here blocks: the GPU comes from futures (`Gpu::new_async`), a frame is recorded and submitted, and the
//! browser presents the canvas when the page's callback returns.

use glam::Vec3;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::path::Path;
use threert_render::live::{Camera, LiveCore};
use threert_render::physics::PhysicsWorld;
use threert_render::voxel::VoxelWorld;
use threert_render::world::World;
use threert_render::{api, files, gpu};
use wasm_bindgen::prelude::*;

/// What scene code edits through `__engine`: as the native `ScriptHost`'s.
#[derive(Default)]
struct Engine {
    world: World,
    voxels: VoxelWorld,
    physics: Option<PhysicsWorld>,
}

/// The live view and the canvas it is shown on.
struct View {
    core: LiveCore,
    surface: wgpu::Surface<'static>,
    blit: wgpu::RenderPipeline,
    blit_bg: wgpu::BindGroup,
    /// The frame script's clock (`f.time`): each frame's dt clamped to [0, 0.1] s, in f64 (script.rs `GameClock`).
    clock: f64,
    /// Of the node host's `describe()`: the game has a frame script; it has world handlers.
    has_frame: bool,
    has_on: bool,
    /// The camera the scene had when the view last followed it.
    scene_cam: Camera,
    fps: u32,
}

thread_local! {
    static ENGINE: RefCell<Engine> = RefCell::default();
    static VIEW: RefCell<Option<View>> = const { RefCell::new(None) };
}

fn thrown(e: api::Error) -> JsValue {
    match e {
        api::Error::Type(m) => js_sys::TypeError::new(&m).into(),
        api::Error::Other(e) => js_sys::Error::new(&format!("{e:#}")).into(),
    }
}

fn failed(e: impl std::fmt::Display) -> JsValue {
    js_sys::Error::new(&format!("{e:#}")).into()
}

/// A descriptor: the JSON text www/engine.js made of scene code's object.
fn desc(what: &str, json: &str) -> Result<Value, JsValue> {
    serde_json::from_str(json).map_err(|e| js_sys::TypeError::new(&format!("engine.{what}: {e}")).into())
}

fn engine<T>(f: impl FnOnce(&mut Engine) -> T) -> T {
    ENGINE.with_borrow_mut(f)
}

// ---- The engine API (script.rs `install_engine`, one for one) ----

#[wasm_bindgen]
pub fn physics(op: &str, args: &str) -> Result<String, JsValue> {
    let a = desc("physics", args)?;
    engine(|e| api::physics(&mut e.physics, &mut e.world, op, &a)).map_err(thrown)
}

#[wasm_bindgen(js_name = physicsPoses)]
pub fn physics_poses() -> Vec<f32> {
    engine(|e| api::physics_poses(&e.physics).to_vec())
}

#[wasm_bindgen(js_name = voxelLevel)]
pub fn voxel_level(d: &str) -> Result<(), JsValue> {
    let d = desc("voxelLevel", d)?;
    engine(|e| api::voxel_level(&mut e.voxels, &d)).map_err(thrown)
}

#[wasm_bindgen(js_name = voxelExplode)]
pub fn voxel_explode(x: f64, y: f64, z: f64, r: f64) -> Result<String, JsValue> {
    engine(|e| api::voxel_explode(&mut e.voxels, &mut e.world, x, y, z, r)).map_err(thrown)
}

#[wasm_bindgen(js_name = voxelEdit)]
pub fn voxel_edit(d: &str) -> Result<String, JsValue> {
    let d = desc("voxelEdit", d)?;
    engine(|e| api::voxel_edit(&mut e.voxels, &mut e.world, &d)).map_err(thrown)
}

#[wasm_bindgen(js_name = voxelHash)]
pub fn voxel_hash() -> String {
    engine(|e| api::voxel_hash(&e.voxels))
}

#[wasm_bindgen(js_name = voxelDebris)]
pub fn voxel_debris() -> Vec<f32> {
    engine(|e| e.voxels.take_debris())
}

#[wasm_bindgen(js_name = voxelFrame)]
pub fn voxel_frame(dt: f64) -> Result<String, JsValue> {
    engine(|e| api::voxel_frame(&mut e.voxels, &mut e.world, dt)).map_err(thrown)
}

#[wasm_bindgen]
pub fn geometry(id: u32, p: &[f32], n: &[f32], uv: &[f32], idx: &[u32], groups: Option<Vec<u32>>, colors: Option<Vec<f32>>) -> Result<(), JsValue> {
    engine(|e| api::geometry(&mut e.world, id, p, n, uv, idx, groups.as_deref(), colors.as_deref())).map_err(thrown)
}

#[wasm_bindgen]
pub fn texture(id: u32, d: &str, data: Option<Vec<u8>>) -> Result<(), JsValue> {
    let d = desc("texture", d)?;
    engine(|e| api::texture(&mut e.world, id, &d, data.as_deref())).map_err(thrown)
}

#[wasm_bindgen]
pub fn shader(name: &str, code: String, helpers: String) {
    engine(|e| e.world.set_shader(name, code, helpers))
}

#[wasm_bindgen(js_name = particleFunction)]
pub fn particle_function(name: &str, code: String, helpers: String) {
    engine(|e| e.world.set_particle_fn(name, code, helpers))
}

#[wasm_bindgen(js_name = particleProgram)]
pub fn particle_program(name: &str, emit: String, update: String, helpers: String) {
    engine(|e| e.world.set_particle_program(name, emit, update, helpers))
}

#[wasm_bindgen(js_name = particlesSpawn)]
pub fn particles_spawn(id: u32, data: &[f32]) -> Result<(), JsValue> {
    engine(|e| e.world.spawn_particles(id, data)).map_err(failed)
}

#[wasm_bindgen(js_name = gridProgram)]
pub fn grid_program(name: &str, code: String, helpers: String) {
    engine(|e| e.world.set_grid_program(name, code, helpers))
}

#[wasm_bindgen(js_name = densityGrid)]
pub fn density_grid(id: u32, d: &str, data: Option<Vec<f32>>) -> Result<(), JsValue> {
    let d = desc("densityGrid", d)?;
    engine(|e| api::density_grid(&mut e.world, id, &d, data.as_deref())).map_err(thrown)
}

#[wasm_bindgen(js_name = gridSources)]
pub fn grid_sources(id: u32, data: &[f32]) -> Result<(), JsValue> {
    engine(|e| e.world.add_grid_sources(id, data)).map_err(failed)
}

#[wasm_bindgen(js_name = volumeFunction)]
pub fn volume_function(name: &str, code: String, helpers: String) {
    engine(|e| e.world.set_volume_fn(name, code, helpers))
}

#[wasm_bindgen(js_name = geometryFunction)]
pub fn geometry_function(name: &str, code: String, helpers: String) {
    engine(|e| e.world.set_geometry_fn(name, code, helpers))
}

#[wasm_bindgen]
pub fn procedural(id: u32, function: String, boxes: Vec<f32>, data: Vec<u32>) -> Result<(), JsValue> {
    engine(|e| e.world.set_proc_geometry(id, function, boxes, data)).map_err(failed)
}

#[wasm_bindgen(js_name = proceduralAsset)]
pub fn procedural_asset(id: u32, function: String, uri: String) -> Result<(), JsValue> {
    engine(|e| e.world.set_proc_asset(id, function, uri)).map_err(failed)
}

#[wasm_bindgen]
pub fn material(id: u32, d: &str) -> Result<(), JsValue> {
    let d = desc("material", d)?;
    engine(|e| api::material(&mut e.world, id, &d)).map_err(thrown)
}

#[wasm_bindgen]
pub fn object(id: u32, d: &str) -> Result<(), JsValue> {
    let d = desc("object", d)?;
    engine(|e| api::object(&mut e.world, id, &d)).map_err(thrown)
}

#[wasm_bindgen]
pub fn transform(id: u32, t: &[f32]) -> Result<(), JsValue> {
    engine(|e| api::transform(&mut e.world, id, t)).map_err(thrown)
}

#[wasm_bindgen]
pub fn instances(id: u32, count: u32, m: Option<Vec<f32>>, c: Option<Vec<f32>>, d: Option<Vec<f32>>) -> Result<(), JsValue> {
    engine(|e| e.world.set_instances(id, count, m, c, d)).map_err(failed)
}

#[wasm_bindgen]
pub fn visible(id: u32, v: bool) -> Result<(), JsValue> {
    engine(|e| e.world.set_visible(id, v)).map_err(failed)
}

#[wasm_bindgen]
pub fn power(id: u32, p: f64) -> Result<(), JsValue> {
    engine(|e| e.world.set_power(id, p as f32)).map_err(failed)
}

#[wasm_bindgen(js_name = customData)]
pub fn custom_data(id: u32, v: &[f64]) -> Result<(), JsValue> {
    engine(|e| api::custom_data(&mut e.world, id, v)).map_err(thrown)
}

/// (`d`: JSON, `null` for none.)
#[wasm_bindgen]
pub fn sway(id: u32, d: &str) -> Result<(), JsValue> {
    let d = desc("sway", d)?;
    engine(|e| api::sway(&mut e.world, id, Some(&d).filter(|d| !d.is_null()))).map_err(thrown)
}

#[wasm_bindgen]
pub fn light(id: u32, d: &str) -> Result<(), JsValue> {
    let d = desc("light", d)?;
    engine(|e| api::light(&mut e.world, id, &d)).map_err(thrown)
}

#[wasm_bindgen]
pub fn camera(id: u32, d: &str) -> Result<(), JsValue> {
    let d = desc("camera", d)?;
    engine(|e| api::camera(&mut e.world, id, &d)).map_err(thrown)
}

/// (`d`: JSON, `null` for none.)
#[wasm_bindgen]
pub fn animation(id: u32, d: &str) -> Result<(), JsValue> {
    let d = desc("animation", d)?;
    engine(|e| api::animation(&mut e.world, id, Some(&d).filter(|d| !d.is_null()))).map_err(thrown)
}

#[wasm_bindgen]
pub fn remove(id: u32) {
    engine(|e| api::remove(&mut e.world, &mut e.physics, id))
}

#[wasm_bindgen]
pub fn environment(d: &str) -> Result<(), JsValue> {
    let d = desc("environment", d)?;
    engine(|e| api::environment(&mut e.world, &d)).map_err(thrown)
}

#[wasm_bindgen]
pub fn settings(d: &str) -> Result<(), JsValue> {
    let d = desc("settings", d)?;
    engine(|e| api::settings(&mut e.world, &d));
    Ok(())
}

// ---- The session ----

/// The canvas' pixels from the view's output (sRGB BGRA8 per pixel, live.wgsl `out_px`), bilinear to the canvas' size:
/// the native window's blit (renderer/src/view.rs).
const BLIT: &str = r#"
override W: f32 = 1280.0;
override H: f32 = 720.0;
override SW: f32 = 1280.0;
override SH: f32 = 720.0;
@group(0) @binding(0) var<storage, read> px: array<u32>;

@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn texel(x: i32, y: i32) -> vec4<f32> {
    let c = px[u32(clamp(y, 0, i32(H) - 1)) * u32(W) + u32(clamp(x, 0, i32(W) - 1))];
    return vec4<f32>(f32((c >> 16u) & 255u), f32((c >> 8u) & 255u), f32(c & 255u), 255.0) / 255.0;
}

@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(W, H);
    let uv = p.xy / vec2<f32>(SW, SH);
    let q = uv * dims - 0.5;
    let f = fract(q);
    let i = vec2<i32>(floor(q));
    let a = mix(texel(i.x, i.y), texel(i.x + 1, i.y), f.x);
    let b = mix(texel(i.x, i.y + 1), texel(i.x + 1, i.y + 1), f.x);
    return mix(a, b, f.y);
}
"#;

/// A panic's message on the page's console (the module is unusable after one).
fn hook() {
    std::panic::set_hook(Box::new(|info| web_sys::console::error_1(&format!("threert: {info}").into())));
}

/// A file the scene reads (a model, a texture, a preset), fetched by the page before `start`: the module has no file
/// system (renderer/src/files.rs).
#[wasm_bindgen]
pub fn provide(path: &str, bytes: Vec<u8>) {
    files::provide(Path::new(path), bytes);
}

/// Opens the live view of the world the game's setup built (the node host's `setup` ran before this) on `canvas`, at
/// `width` x `height` (the view's own pixels; the canvas may be larger). `info`: the node host's `describe()`.
/// `scale`: the traced resolution's share of the view's, 0 for dynamic resolution toward `fps`. Resolves to what it
/// opened, as JSON: the adapter, the device's limits that matter here, the times of the load.
#[wasm_bindgen]
pub async fn start(canvas: web_sys::HtmlCanvasElement, width: u32, height: u32, info: String, scale: f32, fps: u32) -> Result<String, JsValue> {
    hook();
    let t0 = js_sys::Date::now();
    let info: Value = serde_json::from_str(&info).map_err(failed)?;
    let g = gpu::Gpu::new_async().await.map_err(failed)?;
    let t_gpu = js_sys::Date::now();
    let surface = g.instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone())).map_err(failed)?;
    let caps = surface.get_capabilities(&g.adapter);
    // The view's output is sRGB-encoded bytes already: shown through a format that does not encode again.
    let format = caps.formats.iter().copied().find(|f| !f.is_srgb()).ok_or_else(|| failed("the canvas has no format"))?;
    let (sw, sh) = (canvas.width().max(1), canvas.height().max(1));
    surface.configure(
        &g.device,
        &wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: sw,
            height: sh,
            present_mode: wgpu::PresentMode::Fifo,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: Default::default(),
        },
    );
    let limits = g.device.limits();
    let adapter = g.adapter_info.clone();
    let timestamps = g.timestamps;
    // As `GameSession::start`: the setup's changes are the scene the view is built from, not edits of it; a voxel
    // level the scene asked for is loaded now.
    let mut core = engine(|e| {
        e.world.take_changes();
        if e.voxels.realize(&mut e.world, Path::new(""))?.is_some() {
            e.world.take_changes();
        }
        LiveCore::new(g, &e.world, Path::new(""), Path::new(""), width, height)
    })
    .map_err(failed)?;
    core.dynamic_res = scale <= 0.0;
    if scale > 0.0 {
        core.render_scale = scale.clamp(threert_render::live::MIN_SCALE, 1.0);
    }
    let d = &core.g.device;
    let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("blit"), source: wgpu::ShaderSource::Wgsl(BLIT.into()) });
    let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("blit"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    });
    let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
    let blit = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("blit"),
        layout: Some(&pl),
        vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            targets: &[Some(format.into())],
            compilation_options: wgpu::PipelineCompilationOptions { constants: &[("W", width as f64), ("H", height as f64), ("SW", sw as f64), ("SH", sh as f64)], ..Default::default() },
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    });
    let blit_bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("blit"), layout: &layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: core.targets.out.as_entire_binding() }] });
    let out = json!({
        "adapter": { "name": adapter.name, "vendor": adapter.vendor, "device": adapter.device, "backend": format!("{:?}", adapter.backend), "type": format!("{:?}", adapter.device_type) },
        "format": format!("{format:?}"),
        "timestamps": timestamps,
        "merged": core.g.merged,
        "loop_guard": gpu::loop_guard(),
        "limits": { "maxStorageBuffersPerShaderStage": limits.max_storage_buffers_per_shader_stage, "maxBufferSize": limits.max_buffer_size, "maxStorageBufferBindingSize": limits.max_storage_buffer_binding_size },
        "triangles": core.triangles,
        "ms": { "gpu": t_gpu - t0, "view": js_sys::Date::now() - t_gpu },
        "load": core.load_timings().iter().map(|(k, v)| (k.clone(), json!(v))).collect::<serde_json::Map<_, _>>(),
    });
    let on = |k: &str| info[k].as_array().is_some_and(|a| !a.is_empty());
    let scene_cam = core.scene_camera();
    VIEW.set(Some(View { core, surface, blit, blit_bg, clock: 0.0, has_frame: info["frame"].as_bool().unwrap_or(false), has_on: on("on"), scene_cam, fps: fps.max(1) }));
    Ok(out.to_string())
}

fn view<T>(f: impl FnOnce(&mut View) -> Result<T, JsValue>) -> Result<T, JsValue> {
    VIEW.with_borrow_mut(|v| f(v.as_mut().ok_or_else(|| failed("no view: `start` has not resolved"))?))
}

/// The view shows the scene's camera: when scene code moved it (or made another the scene's), the view follows.
fn follow(v: &mut View) {
    let cam = v.core.scene_camera();
    if cam != v.scene_cam {
        v.scene_cam = cam;
        v.core.set_camera(cam);
    }
}

/// Applies what scene code changed to the view (`LiveCore::commit`): true if that rebuilt the scene.
fn commit_view(v: &mut View) -> Result<bool, JsValue> {
    let rebuilt = engine(|e| v.core.commit(&mut e.world)).map_err(failed)?;
    follow(v);
    Ok(rebuilt)
}

/// A frame begins, `dt` seconds after the last (`GameSession::frame` up to the frame script): the scene's time, wind
/// sway. True if the page is to run the node host's `frame` now (script.rs `ScriptHost::frames`: a frame script,
/// world handlers, or a physics world whose events are delivered once a frame).
#[wasm_bindgen(js_name = frameBegin)]
pub fn frame_begin(dt: f32) -> Result<bool, JsValue> {
    view(|v| {
        v.core.advance_time(dt);
        let frames = v.has_frame || v.has_on || engine(|e| e.physics.is_some());
        if frames && v.has_frame {
            v.clock += (dt as f64).clamp(0.0, 0.1);
            v.core.set_time(v.clock as f32);
        }
        let t = v.core.scene_time();
        engine(|e| e.world.apply_sway(t));
        Ok(frames)
    })
}

/// The frame's end: the physics' ticks (after the frame script: its impulses and targets act in this frame's ticks),
/// what the script and the bodies changed committed, the frame rendered and shown. Returns the frame's numbers as
/// JSON (ms): the physics (ticks, step, sync, bodies, moved), the commit and whether it rebuilt the scene, the
/// rebuild, recording the frame, the GPU's time for the last frame that was timed and its trace pass, the traced
/// resolution's scale; and `guard_trips`, the kernel's invocations the loop guard has stopped since the view opened
/// (renderer/src/merged.rs `guard`; read back a frame or two late): anything but 0 means frames with wrong pixels.
#[wasm_bindgen(js_name = frameEnd)]
pub fn frame_end(dt: f32) -> Result<String, JsValue> {
    view(|v| {
        let now = web_time_now;
        let p0 = now();
        engine(|e| {
            if let Some(p) = e.physics.as_mut() {
                p.advance(&mut e.world, dt as f64);
            }
        });
        let p1 = now();
        let rebuilt = commit_view(v)?;
        let p2 = now();
        let tex = match v.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => return Err(failed(format!("the canvas' texture: {other:?}"))),
        };
        let target = tex.texture.create_view(&Default::default());
        let (blit, bg) = (&v.blit, &v.blit_bg);
        v.core.render(1, |e, _| {
            let mut rp = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &target, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rp.set_pipeline(blit);
            rp.set_bind_group(0, bg, &[]);
            rp.draw(0..3, 0..1);
        });
        v.core.g.queue.present(tex);
        let p3 = now();
        v.core.govern_resolution(v.fps);
        let physics = engine(|e| e.physics.as_mut().map(|p| p.take_stats()));
        Ok(json!({
            "physics": physics.map(|s| json!({ "ticks": s.ticks, "step": s.step_ms, "sync": s.sync_ms, "bodies": s.bodies, "moved": s.moved })),
            "physics_ms": p1 - p0,
            "commit_ms": p2 - p1,
            "rebuilt": rebuilt,
            "rebuild_ms": v.core.rebuild_ms,
            "render_ms": p3 - p2,
            "gpu_ms": v.core.gpu_ms,
            "trace_ms": v.core.gpu_trace_ms,
            "scale": v.core.render_scale,
            "frame": v.core.frame,
            "fallback": v.core.fallback,
            "guard_trips": v.core.guard_trips,
        })
        .to_string())
    })
}

/// The page's clock, ms (`performance.now()`: what `web_time::Instant` reads).
fn web_time_now() -> f64 {
    thread_local! { static T0: web_time::Instant = web_time::Instant::now(); }
    T0.with(|t| t.elapsed().as_secs_f64() * 1e3)
}

/// A click at (u, v) of the view: the object under it as the `click` event's payload, JSON (`GameSession::click`:
/// `{ target, point, player, ray: { origin, direction }, instanceId? }`), or nothing where the click meets nothing.
/// The page sends it to the node host (`event("click", payload)`), then calls `commit`.
#[wasm_bindgen]
pub fn click(u: f32, v: f32) -> Result<Option<String>, JsValue> {
    view(|view| {
        let Some((target, p, sub)) = view.core.pick(u, v) else { return Ok(None) };
        let r = |x: f32| (x * 1e4).round() / 1e4;
        let (o, d): (Vec3, Vec3) = view.core.ray(u, v);
        let mut payload = json!({ "target": target, "point": [r(p.x), r(p.y), r(p.z)], "player": "you", "ray": { "origin": o.to_array(), "direction": d.to_array() } });
        if let Some(i) = sub {
            payload["instanceId"] = i.into();
        }
        Ok(Some(payload.to_string()))
    })
}

/// Applies what an event's handlers changed to the view, `seq` the event's sequence number (`GameSession::event`).
/// True if that rebuilt the scene.
#[wasm_bindgen]
pub fn commit(seq: f64) -> Result<bool, JsValue> {
    view(|v| {
        let rebuilt = commit_view(v)?;
        v.core.seq = seq as i64;
        Ok(rebuilt)
    })
}

/// The physics world's hash (16 hex digits; "" without a world): the same calls and ticks give the same hash natively
/// and here.
#[wasm_bindgen(js_name = physicsHash)]
pub fn physics_hash() -> String {
    engine(|e| e.physics.as_ref().map_or(String::new(), |p| format!("{:016x}", p.hash())))
}
