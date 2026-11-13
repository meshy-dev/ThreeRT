//! threert-render: native renderer for ThreeRT games (no browser, no engine). A game bundle (game code + the scene
//! runtime) runs in the embedded JS engine and builds the scene through the engine API; the renderer draws it.
//!
//!   threert-render render <bundle.js> -o out.png [--assets DIR] [--room id] [--seed n] [--state state.json]
//!                         [--camera '{"position":[x,y,z],"target":[x,y,z],"fov":deg}'] [--spp N] [--stats out.json]
//!                         [--preset name] [--width W --height H] [--time s] [--no-denoise] [--override JSON] [--presets DIR]
//!                         [--runtime threert-runtime.js (the game is then an ES module importing it)]
//!                         [--hdr out.raw] [--signals PREFIX] (linear RGB f32: the image; its diffuse / specular /
//!                         emission apart, PREFIX-{diffuse,specular,emission}.raw)
//!   threert-render serve [--presets DIR]      # JSON-lines jobs on stdin, keeps the GPU device and game runtimes warm
//!   threert-render play | live <bundle.js>    # native window / headless real-time (view.rs, live/)
//!   threert-render setup <bundle.js>          # setup cost in the embedded runtime
//!   threert-render stats <bundle.js> [--assets DIR] [--room id] [--seed n]
//!                         # scene characterisation without a GPU: objects, geometries,
//!                         # materials, textures, triangles, emitters
//!   threert-render edit-check <bundle.js> --events '[{"type":..,"payload":..}]'
//!   threert-render info                       # print adapter info

mod net;

use threert_render::script::ScriptHost;
use threert_render::{compare, default_presets_dir, denoise, gpu, live, post, prepare, scene, view};

use anyhow::{anyhow, Context, Result};
use glam::Vec3;
use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::Instant;

struct Job {
    game: GameJob,
    out: PathBuf,
    spp: Option<u32>,
    stats: Option<PathBuf>,
    denoise: Option<bool>,
    overrides: Option<serde_json::Value>,
    /// Viewpoint instead of the game camera's: {position, target, fov?}.
    camera: Option<serde_json::Value>,
    /// Render preset / image size / scene time (animated materials, rain) instead of the game's.
    preset: Option<String>,
    time: Option<f32>,
    size: (Option<u32>, Option<u32>),
    hdr: Option<PathBuf>,
    /// `--signals PREFIX`: PREFIX-diffuse.raw / -specular.raw / -emission.raw, the image's linear signals apart as the
    /// live denoiser splits them (RGB f32, output size; live snapshots to `*.signals` write the same).
    signals: Option<PathBuf>,
}

/// The game state to render: a bundle's scene for a room, at its initial state or a room's committed state.
struct GameJob {
    /// The game: a bundle (game + runtime in one script), or with `runtime` the game's ES module.
    bundle: PathBuf,
    /// The runtime ES module (threert-runtime.js) the game module imports.
    runtime: Option<PathBuf>,
    /// Asset root the scene's model / texture paths resolve against.
    assets: PathBuf,
    room: String,
    seed: Option<u32>,
    state: Option<serde_json::Value>,
    seq: i64,
}

impl GameJob {
    fn from_json(v: &serde_json::Value) -> Result<GameJob> {
        Ok(GameJob {
            bundle: PathBuf::from(v["game"].as_str().context("job: game bundle path required")?),
            runtime: v["runtime"].as_str().map(PathBuf::from),
            assets: PathBuf::from(v["assets"].as_str().unwrap_or(".")),
            room: v["room"].as_str().unwrap_or("local").to_string(),
            seed: v["seed"].as_u64().map(|s| s as u32),
            state: (!v["state"].is_null()).then(|| v["state"].clone()),
            seq: v["seq"].as_i64().unwrap_or(0),
        })
    }
}

/// Game runtimes of a warm `serve`: one per bundle version / room / seed, set up once, then only given new states.
#[derive(Default)]
struct Hosts {
    hosts: HashMap<String, (ScriptHost, serde_json::Value)>,
}

impl Hosts {
    /// The runtime with the job's scene: setup on first use, then the job's state (or the initial one) applied.
    fn scene(&mut self, job: &GameJob) -> Result<(&ScriptHost, f64)> {
        let version = |p: &Path| -> Result<String> {
            let m = std::fs::metadata(p).with_context(|| p.display().to_string())?;
            Ok(format!("{}|{}|{:?}", p.display(), m.len(), m.modified().ok()))
        };
        let runtime = job.runtime.as_deref().map(version).transpose()?;
        let key = format!("{}|{runtime:?}|{}|{:?}", version(&job.bundle)?, job.room, job.seed);
        let t = Instant::now();
        if !self.hosts.contains_key(&key) {
            if self.hosts.len() >= 8 {
                self.hosts.clear();
            }
            let code = std::fs::read_to_string(&job.bundle)?;
            let host = match &job.runtime {
                Some(r) => ScriptHost::from_modules(&code, &std::fs::read_to_string(r)?)?,
                None => ScriptHost::new(&code)?,
            };
            host.setup(&job.room, job.seed)?;
            // A native voxel level the scene code asked for (`voxelLevel`), loaded now that the asset root is known: the
            // offline reference of the live view's level (ROADMAP § 6 step 4). Its changes are the setup's, dropped.
            let realized = host.voxels.borrow_mut().realize(&mut host.world.borrow_mut(), &mut host.physics.borrow_mut(), &job.assets)?;
            if let Some(r) = realized {
                eprintln!("voxels: {r}");
                host.world.borrow_mut().take_changes();
            }
            let (initial, _) = host.state()?;
            self.hosts.insert(key.clone(), (host, initial));
        }
        let (host, initial) = &self.hosts[&key];
        host.set_state(job.state.as_ref().unwrap_or(initial), job.seq)?;
        Ok((host, t.elapsed().as_secs_f64() * 1000.0))
    }
}

/// Files the scene reads that are not under the asset root yet (a render node fetches them and retries).
#[derive(Debug)]
struct MissingFiles(Vec<String>);

impl std::fmt::Display for MissingFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "missing files under the asset root: {}", self.0.join(", "))
    }
}

impl std::error::Error for MissingFiles {}

/// `--camera {position, target, fov?}` (fov in degrees, vertical).
fn override_camera(c: &mut scene::CameraData, v: &serde_json::Value) -> Result<()> {
    let v3 = |k: &str| -> Result<Vec3> {
        let a: [f32; 3] = serde_json::from_value(v[k].clone()).with_context(|| format!("--camera: {k} must be [x, y, z]"))?;
        Ok(Vec3::from(a))
    };
    let (pos, target) = (v3("position")?, v3("target")?);
    let fwd = (target - pos).try_normalize().ok_or_else(|| anyhow!("--camera: position == target"))?;
    let right = fwd.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
    (c.position, c.forward, c.right, c.up) = (pos, fwd, right, right.cross(fwd));
    if let Some(fov) = v["fov"].as_f64() {
        c.tan_half_fov = (fov.to_radians() as f32 * 0.5).tan();
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str).unwrap_or("help");
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let presets = flag("--presets").map(PathBuf::from).unwrap_or_else(default_presets_dir);
    match cmd {
        "info" => {
            let g = gpu::Gpu::new()?;
            println!("{}", serde_json::json!({ "adapter": g.adapter_info.name, "hwrt": g.hwrt, "backend": format!("{:?}", g.adapter_info.backend), "hwrt": g.hwrt, "device_type": format!("{:?}", g.adapter_info.device_type), "driver": g.adapter_info.driver }));
        }
        "render" => {
            let t0 = Instant::now();
            let g = gpu::Gpu::new()?;
            let init_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let job = Job {
                game: GameJob {
                    bundle: PathBuf::from(args.get(2).filter(|a| !a.starts_with("-")).ok_or_else(|| anyhow!("game bundle required"))?),
                    runtime: flag("--runtime").map(PathBuf::from),
                    assets: PathBuf::from(flag("--assets").unwrap_or_else(|| ".".into())),
                    room: flag("--room").unwrap_or_else(|| "local".into()),
                    seed: flag("--seed").map(|s| s.parse()).transpose()?,
                    state: flag("--state").map(|f| -> Result<serde_json::Value> { Ok(serde_json::from_slice(&std::fs::read(f)?)?) }).transpose()?,
                    seq: 0,
                },
                out: PathBuf::from(flag("-o").unwrap_or_else(|| "out.png".into())),
                spp: flag("--spp").map(|s| s.parse()).transpose()?,
                stats: flag("--stats").map(PathBuf::from),
                denoise: if args.iter().any(|a| a == "--no-denoise") { Some(false) } else { None },
                overrides: flag("--override").map(|s| serde_json::from_str(&s)).transpose()?,
                camera: flag("--camera").map(|s| serde_json::from_str(&s)).transpose()?,
                preset: flag("--preset"),
                time: flag("--time").map(|s| s.parse()).transpose()?,
                size: (flag("--width").map(|s| s.parse()).transpose()?, flag("--height").map(|s| s.parse()).transpose()?),
                hdr: flag("--hdr").map(PathBuf::from),
                signals: flag("--signals").map(PathBuf::from),
            };
            let stats = render(&g, &job, &presets, init_ms, &mut Hosts::default())?;
            println!("{}", serde_json::to_string(&stats)?);
        }
        "compare" => {
            // --region u0,v0,u1,v1 (0..1, v down): only that part of the images.
            let region = flag("--region").map(|r| r.split(',').map(|v| v.parse::<f32>()).collect::<Result<Vec<_>, _>>()).transpose()?.map(|v| [v[0], v[1], v[2], v[3]]);
            let r = compare::compare_region(Path::new(&args[2]), Path::new(&args[3]), flag("--sheet").as_deref().map(Path::new), region)?;
            println!("{}", serde_json::to_string(&r)?);
        }
        "edge-width" => {
            // threert-render edge-width image.png x0 x1 y0 y1 [channel 0|1|2]: blur across a vertical edge (compare::edge_width).
            let n = |i: usize| -> Result<u32> { Ok(args.get(i).ok_or_else(|| anyhow!("edge-width image x0 x1 y0 y1 [channel]"))?.parse()?) };
            let ch = args.get(7).map(|c| c.parse()).transpose()?.unwrap_or(0);
            println!("{}", compare::edge_width(Path::new(&args[2]), [n(3)?, n(4)?], [n(5)?, n(6)?], ch)?);
        }
        "ae" => {
            // threert-render ae linear.exr <ev100 it was exposed at> [lowPercent highPercent compensation minEv maxEv]
            let img = image::open(&args[2])?.to_rgb32f();
            let k = 2f32.powf(args[3].parse::<f32>()?);
            let px: Vec<Vec3> = img.pixels().map(|p| Vec3::new(p.0[0], p.0[1], p.0[2]) * k).collect();
            let num = |i: usize, d: f32| args.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(d);
            let mut pre: threert_render::desc::Preset = serde_json::from_str(&std::fs::read_to_string(presets.join("cinematic.json"))?)?;
            pre.post.auto_exposure.low_percent = num(4, 70.0);
            pre.post.auto_exposure.high_percent = num(5, 97.0);
            pre.post.auto_exposure.histogram_log_min = -10.0;
            pre.post.auto_exposure.histogram_log_max = 20.0;
            let exp = threert_render::desc::Exposure::Auto { compensation: num(6, 0.6), min_ev100: num(7, -1.5), max_ev100: num(8, 3.0) };
            let (_, ev) = post::exposure_scale(&post::Image { w: img.width() as usize, h: img.height() as usize, px }, &exp, &pre.post);
            println!("{{\"ev100\":{ev}}}");
        }
        "rain-stats" => {
            // threert-render rain-stats --on a.png,b.png,... --off c.png,... [--region u0,v0,u1,v1] (compare::rain_stats).
            let list = |k: &str| flag(k).map(|v| v.split(',').map(String::from).collect::<Vec<_>>()).ok_or_else(|| anyhow!("rain-stats --on a.png,.. --off b.png,.. [--region u0,v0,u1,v1]"));
            let (on, off) = (list("--on")?, list("--off")?);
            let region = flag("--region").map(|r| r.split(',').map(|v| v.parse::<f32>()).collect::<Result<Vec<_>, _>>()).transpose()?.map(|v| [v[0], v[1], v[2], v[3]]);
            let on: Vec<&Path> = on.iter().map(Path::new).collect();
            let off: Vec<&Path> = off.iter().map(Path::new).collect();
            println!("{}", compare::rain_stats(&on, &off, region)?);
        }
        "compare-hdr" => {
            // threert-render compare-hdr reference.exr ours.raw <ev100> [--heat heat.png]
            let ev: f32 = args.get(4).ok_or_else(|| anyhow!("ev100 required"))?.parse()?;
            let r = compare::compare_hdr(Path::new(&args[2]), Path::new(&args[3]), ev, flag("--heat").as_deref().map(Path::new))?;
            println!("{}", serde_json::to_string(&r)?);
        }
        "view" | "play" => {
            // The window is the app of the game's host channel: its `net.*` messages are this binary's sockets and
            // HTTP (src/net.rs; a build without the `net` feature answers that it has none).
            view::run(view::parse_args(&args, &presets)?, Box::new(net::Net::new()))?;
        }
        "setup" => {
            // Setup cost of a game bundle in the embedded runtime (as on a render node).
            let t0 = Instant::now();
            let host = ScriptHost::new(&std::fs::read_to_string(args.get(2).ok_or_else(|| anyhow!("game bundle required"))?)?)?;
            let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let setup_ms = host.setup(&flag("--room").unwrap_or_else(|| "local".into()), flag("--seed").map(|v| v.parse()).transpose()?)?;
            let w = host.world.borrow();
            println!("{}", serde_json::json!({ "engine": "quickjs", "bundle_load_ms": load_ms, "setup_ms": setup_ms, "objects": w.objects.len(), "geometries": w.geometries.len(), "materials": w.materials.len(), "textures": w.textures.len(), "info": host.info }));
        }
        "stats" => {
            // Scene characterisation without a GPU: the world is built in the embedded runtime, then the CPU
            // scene build + prepare count the geometry and the emitters (the same `prepare_as` the render and
            // live paths run, with the hardware-traversal flags). No adapter, no device, no upload.
            let t0 = Instant::now();
            let host = ScriptHost::new(&std::fs::read_to_string(args.get(2).ok_or_else(|| anyhow!("game bundle required"))?)?)?;
            let load_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let assets = PathBuf::from(flag("--assets").unwrap_or_else(|| ".".into()));
            let setup_ms = host.setup(&flag("--room").unwrap_or_else(|| "local".into()), flag("--seed").map(|v| v.parse()).transpose()?)?;
            let w = host.world.borrow();
            let tb = Instant::now();
            let mut sc = scene::build(&w, &assets, &presets, None)?;
            let prep = threert_render::prepare_as(true, false, &mut sc, &mut vec![]);
            let prepare_ms = tb.elapsed().as_secs_f64() * 1000.0;
            // Same counts the render stats report (scene.rs `stats`): flat + instanced triangles.
            let triangles = sc.stats.get("triangles").map_or(sc.tris.len() as u64, |t| *t as u64);
            println!("{}", serde_json::json!({ "engine": "quickjs", "bundle_load_ms": load_ms, "setup_ms": setup_ms, "prepare_ms": prepare_ms, "objects": w.objects.len(), "geometries": w.geometries.len(), "materials": w.materials.len(), "textures": w.textures.len(), "triangles": triangles, "flat_triangles": sc.tris.len(), "instances": sc.instances.len(), "instanced_triangles": sc.stats.get("instanced_triangles").copied().unwrap_or(0.0) as u64, "emitters": prep.emitter_tris.len(), "info": host.info }));
        }
        "edit-check" => {
            // threert-render edit-check <bundle.js> [--assets DIR] --events '[{"type":..,"payload":..}, ..]'
            // The events' parameter edits, applied in place, must leave the GPU tables exactly as a fresh build of the
            // edited world makes them.
            let g = gpu::Gpu::new()?;
            let assets = PathBuf::from(flag("--assets").unwrap_or_else(|| ".".into()));
            let host = ScriptHost::new(&std::fs::read_to_string(args.get(2).ok_or_else(|| anyhow!("game bundle required"))?)?)?;
            host.setup("local", None)?;
            let build = |w: &threert_render::world::World| -> Result<(scene::Scene, threert_render::Prepared)> {
                let mut sc = scene::build(w, &assets, &presets, None)?;
                let prep = prepare(&g, &mut sc, &mut vec![]);
                Ok((sc, prep))
            };
            let (a, mut pa) = build(&host.world.borrow_mut())?;
            host.world.borrow_mut().take_changes();
            let mut e = threert_render::tables::Tables::new(&a, std::mem::take(&mut pa.emitter_tris), pa.light_tree.take());
            let events: Vec<serde_json::Value> = serde_json::from_str(&flag("--events").unwrap_or_else(|| "[]".into()))?;
            for ev in &events {
                host.event(ev["type"].as_str().context("event: type")?, &ev["payload"])?;
            }
            // Settled: the frame script shows the resulting state without transitions.
            let (state, seq) = host.state()?;
            host.set_state(&state, seq)?;
            let mut w = host.world.borrow_mut();
            let changes = w.take_changes();
            let applied = e.apply(&w, &changes, 0.0)?;
            let (b, pb) = build(&w)?;
            // Same sampling distribution: entries switched off (zero weight) only exist on the edited side, so only the
            // entries are compared, not the guide table after them (indices into the entries).
            let bits = |v: &[[f32; 4]], n: usize| v[..n].iter().filter(|e| e[2] > 0.0).flat_map(|e| e.map(f32::to_bits)).collect::<Vec<_>>();
            let (ea, _, _) = threert_render::emitter_cdf(&e.emitter_tris, &e.materials, &e.objects, e.stride, [0.0; 4], threert_render::EmitterCull::default());
            let power = |o: &[[f32; 4]]| o.iter().step_by(e.stride).map(|p| p[3].to_bits()).collect::<Vec<_>>();
            let checks = [
                ("materials", bytemuck::cast_slice::<_, u8>(&e.materials) == bytemuck::cast_slice::<_, u8>(&b.materials)),
                ("lights", bytemuck::cast_slice::<_, u8>(&e.lights) == bytemuck::cast_slice::<_, u8>(&b.lights)),
                ("object power", power(&e.objects) == power(&b.objects)),
                ("emitters", bits(&ea, e.emitter_tris.len()) == bits(&pb.emitters, pb.emitter_tris.len())),
                ("wetness / rain", (e.wetness, e.rain) == (b.wetness, b.rain)),
            ];
            println!("{}", serde_json::json!({ "structural": applied.is_none(), "checks": checks.iter().map(|(k, ok)| (k.to_string(), *ok)).collect::<std::collections::BTreeMap<_, _>>() }));
            if applied.is_none() || checks.iter().any(|(_, ok)| !ok) {
                std::process::exit(1);
            }
        }
        "live" => {
            live::run(&presets, live::parse_opts(&args)?)?;
        }
        "montage" => {
            // threert-render montage out.jpg --width 640 a.png b.png c.png  (horizontal strip)
            let out = PathBuf::from(&args[2]);
            let width: u32 = flag("--width").map(|w| w.parse()).transpose()?.unwrap_or(640);
            let inputs: Vec<&String> = args[3..].iter().filter(|a| !a.starts_with("--") && a.as_str() != width.to_string()).collect();
            let imgs: Vec<image::RgbImage> = inputs.iter().map(|p| image::open(p).map(|i| i.to_rgb8())).collect::<Result<_, _>>()?;
            let h = imgs.iter().map(|i| (i.height() as f32 * width as f32 / i.width() as f32) as u32).max().unwrap_or(1);
            let mut sheet = image::RgbImage::new(width * imgs.len() as u32 + 4 * (imgs.len() as u32 - 1), h);
            for (k, img) in imgs.iter().enumerate() {
                let r = image::imageops::resize(img, width, h, image::imageops::FilterType::Lanczos3);
                image::imageops::replace(&mut sheet, &r, (k as u32 * (width + 4)) as i64, 0);
            }
            sheet.save(&out)?;
        }
        "serve" => {
            // One job per stdin line: {id, game: bundle path (or game module + runtime: runtime module), assets, room,
            // seed, state?, seq?, out, spp?, denoise?, overrides?, camera?, preset?, width?, height?, time?, hdr? (raw f32
            // RGB linear radiance, nits), live? (frames of the real-time renderer instead, `render_live`)}; one result line
            // each: {id, ok, stats} or
            // {id, ok: false, error, missing?: [files to put under the asset root before retrying]}.
            let t0 = Instant::now();
            let g = gpu::Gpu::new()?;
            let mut init_ms = t0.elapsed().as_secs_f64() * 1000.0;
            let mut hosts = Hosts::default();
            // The real-time renderer owns its device: a second one, made on the first live job.
            let mut live_gpu: Option<gpu::Gpu> = None;
            eprintln!("threert-render serving on stdin ({} / {:?}), init {init_ms:.0} ms", g.adapter_info.name, g.adapter_info.backend);
            println!("{}", serde_json::json!({ "ready": true, "adapter": g.adapter_info.name, "init_ms": init_ms }));
            for line in std::io::stdin().lock().lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let v: serde_json::Value = serde_json::from_str(&line)?;
                let res = GameJob::from_json(&v).and_then(|game| {
                    let job = Job {
                        game,
                        out: PathBuf::from(v["out"].as_str().unwrap_or("out.png")),
                        spp: v["spp"].as_u64().map(|x| x as u32),
                        stats: None,
                        denoise: v["denoise"].as_bool(),
                        overrides: v.get("overrides").filter(|o| !o.is_null()).cloned(),
                        camera: v.get("camera").filter(|c| !c.is_null()).cloned(),
                        preset: v["preset"].as_str().map(String::from),
                        time: v["time"].as_f64().map(|t| t as f32),
                        size: (v["width"].as_u64().map(|x| x as u32), v["height"].as_u64().map(|x| x as u32)),
                        hdr: v["hdr"].as_str().map(PathBuf::from),
                        signals: v["signals"].as_str().map(PathBuf::from),
                    };
                    match v["live"].as_u64() {
                        Some(frames) => render_live(&mut live_gpu, &job, &presets, frames as u32, &mut hosts),
                        None => render(&g, &job, &presets, init_ms, &mut hosts),
                    }
                });
                init_ms = 0.0;
                match res {
                    Ok(s) => println!("{}", serde_json::json!({ "id": v["id"], "ok": true, "stats": s })),
                    Err(e) => {
                        let missing = e.downcast_ref::<MissingFiles>().map(|m| m.0.clone());
                        println!("{}", serde_json::json!({ "id": v["id"], "ok": false, "error": format!("{e:#}"), "missing": missing }));
                    }
                }
            }
        }
        _ => {
            eprintln!("usage: threert-render render <bundle.js> -o out.png [--assets DIR] [--state f.json] [--camera JSON] [--spp N] | play | live | serve | setup | edit-check | info");
            std::process::exit(2);
        }
    }
    Ok(())
}


/// A `serve` job with `live: frames`: the real-time renderer (radiance cache, temporal and denoising passes) runs that
/// many frames at 100 % scale on the scene's camera, the game's frame script (if any) before each at 60 fps; `hdr` gets
/// its linear radiance, `out` the displayed image.
fn render_live(slot: &mut Option<gpu::Gpu>, job: &Job, presets: &Path, frames: u32, hosts: &mut Hosts) -> Result<serde_json::Value> {
    let t0 = Instant::now();
    let (host, _) = hosts.scene(&job.game)?;
    // `time`: the scene time of the last frame (frames run at 60 fps up to it). The frame script's clock starts where the
    // frames do, 0 without a `time`: a cached host keeps the clock of its last job (a still at 41 s, the end of a live
    // run), and the scene would start with that moment's walkers, rain and smoke.
    let start = job.time.map_or(0.0, |t| t - frames.max(1) as f32 / 60.0);
    if host.has_frame() {
        host.seek(start)?;
    }
    // The physics world as it is where the frames start (as `render` has it at its scene time); the frames step it on.
    host.settle_physics((start.max(0.0) * 60.0).round() as u32);
    let mut w = host.world.borrow_mut();
    w.take_changes();
    let missing: Vec<String> = w.files().into_iter().filter(|f| !job.game.assets.join(f).is_file()).collect();
    if !missing.is_empty() {
        return Err(MissingFiles(missing).into());
    }
    let (width, height) = (job.size.0.unwrap_or(w.settings.width), job.size.1.unwrap_or(w.settings.height));
    let g = match slot.take() {
        Some(g) => g,
        None => gpu::Gpu::new()?,
    };
    let mut core = live::LiveCore::new(g, &w, &job.game.assets, presets, width, height)?;
    core.set_scale(1.0);
    drop(w);
    // A frame script runs at 60 fps: the renderer's frame time is the same (motion blur, exposure adaptation).
    if host.has_frame() {
        core.fixed_dt = Some(1.0 / 60.0);
    }
    if job.time.is_some() {
        core.set_time(start);
    }
    for _ in 0..frames.max(1) {
        core.advance_time(1.0 / 60.0);
        let swayed = host.world.borrow_mut().apply_sway(core.scene_time());
        // (`frames`: also a game of world handlers, and a scene with bodies: its listeners get their events.)
        if host.frames() {
            host.frame(1.0 / 60.0)?;
        }
        let physics = host.advance_physics(1.0 / 60.0);
        if host.frames() || swayed || physics {
            core.commit(&mut host.world.borrow_mut())?;
        }
        core.render(1, |_, _| {});
    }
    let hdr = core.read_hdr()?;
    if let Some(p) = &job.hdr {
        std::fs::write(p, bytemuck::cast_slice(&hdr))?;
    }
    let ev100 = core.read_ev100()?;
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    *slot = Some(core.g);
    Ok(serde_json::json!({ "width": width, "height": height, "frames": frames, "ev100": ev100, "timings_ms": { "total_ms": ms } }))
}

/// PREFIX-diffuse.raw, -specular.raw, -emission.raw (RGB f32 each).
fn write_signals(prefix: &Path, diffuse: &[f32], specular: &[f32], emission: &[f32]) -> Result<()> {
    if let Some(parent) = prefix.parent() {
        std::fs::create_dir_all(parent)?;
    }
    for (name, v) in [("diffuse", diffuse), ("specular", specular), ("emission", emission)] {
        std::fs::write(format!("{}-{name}.raw", prefix.display()), bytemuck::cast_slice(v))?;
    }
    Ok(())
}

fn render(g: &gpu::Gpu, job: &Job, presets: &Path, init_ms: f64, hosts: &mut Hosts) -> Result<serde_json::Value> {
    let t_total = Instant::now();
    let mut timings: Vec<(String, f64)> = vec![("gpu_init_ms".into(), init_ms)];
    let (host, script_ms) = hosts.scene(&job.game)?;
    // A frame script draws a timeline: the render shows the game at the job's scene time (0 without one), as live
    // shows it after running that long. Before, a still showed the frame script at its last run (setup: time 0) while
    // shaders and sway took the job's time: the Bistro C slice's walkers, smoke and rain stood at 0 s in every frame.
    if host.has_frame() {
        host.seek(job.time.unwrap_or(0.0))?;
    }
    // The physics world as it is after the job's scene time (60 ticks a second from the setup's world).
    host.settle_physics((job.time.unwrap_or(0.0).max(0.0) * 60.0).round() as u32);
    timings.push(("script_ms".into(), script_ms));
    let mut sc = {
        let mut w = host.world.borrow_mut();
        w.take_changes();
        let missing: Vec<String> = w.files().into_iter().filter(|f| !job.game.assets.join(f).is_file()).collect();
        if !missing.is_empty() {
            return Err(MissingFiles(missing).into());
        }
        let saved = w.settings.clone();
        if let Some(p) = &job.preset {
            w.settings.preset = p.clone();
        }
        w.settings.time = job.time.unwrap_or(w.settings.time);
        let t = w.settings.time;
        w.apply_sway(t);
        let sc = scene::build(&w, &job.game.assets, presets, job.overrides.as_ref());
        w.settings = saved;
        sc?
    };
    sc.width = job.size.0.unwrap_or(sc.width);
    sc.height = job.size.1.unwrap_or(sc.height);
    if let Some(c) = &job.camera {
        override_camera(&mut sc.camera, c)?;
    }
    timings.push(("load_ms".into(), sc.stats["load_ms"]));

    let prep = prepare(g, &mut sc, &mut timings);
    let preset = sc.preset.clone();
    let spp = job.spp.unwrap_or(preset.native.spp);
    let mut params = prep.params;
    if job.signals.is_some() {
        params.motion[1] = 1.0;
    }
    let frame = g.render(&sc, &prep.tri_verts, prep.groups, bytemuck::cast_slice(&prep.nodes), &prep.emitters, &prep.env, &prep.env_cdf, &prep.energy, params, spp, &mut timings)?;
    if let Some(prefix) = &job.signals {
        // The raw (undenoised) means: diffuse = color less specular and emission.
        let diffuse: Vec<f32> = frame.color.iter().zip(&frame.specular).zip(&frame.emission).flat_map(|((c, s), e)| [0, 1, 2].map(|k| (c[k] - s[k] - e[k]).max(0.0))).collect();
        let flat = |v: &[[f32; 3]]| v.iter().flatten().copied().collect::<Vec<f32>>();
        write_signals(prefix, &diffuse, &flat(&frame.specular), &flat(&frame.emission))?;
    }

    let t = Instant::now();
    let (w, h) = (frame.width as usize, frame.height as usize);
    let color: Vec<Vec3> = frame.color.iter().map(|c| Vec3::from(*c)).collect();
    let denoise = job.denoise.unwrap_or(preset.native.denoise);
    // Exposure is needed up front so OIDN knows the display scale of the HDR input.
    let (exposure, ev100) = post::exposure_scale(&post::Image { w, h, px: color.clone() }, &sc.camera.exposure, &preset.post);
    let mut denoiser = "none";
    let color = if denoise {
        let albedo: Vec<Vec3> = frame.albedo.iter().map(|c| Vec3::from(*c)).collect();
        let normal: Vec<Vec3> = frame.normal.iter().map(|c| Vec3::from(*c).normalize_or_zero()).collect();
        match denoise::oidn(w, h, &color, &albedo, &normal, exposure) {
            Some(c) => {
                denoiser = "oidn";
                c
            }
            None => {
                denoiser = "atrous";
                denoise::denoise(w, h, &color, &albedo, &normal, &frame.variance, 4)
            }
        }
    } else {
        color
    };
    timings.push(("denoise_ms".into(), t.elapsed().as_secs_f64() * 1000.0));
    // Translucent particles over the converged image (gpu.rs `render_translucent`).
    let color: Vec<Vec3> = if frame.overlay.is_empty() { color } else { color.iter().zip(&frame.overlay).map(|(c, o)| *c * o[3] + Vec3::new(o[0], o[1], o[2])).collect() };

    let t = Instant::now();
    let img = post::Image { w, h, px: color };
    // Local exposure's middle grey follows the exposure compensation (UE: auto bias; manual: bias = -EV100).
    let compensation = match &sc.camera.exposure {
        threert_render::desc::Exposure::Auto { compensation, .. } => 2f32.powf(*compensation),
        threert_render::desc::Exposure::Manual { ev100 } => 2f32.powf(-ev100),
    };
    let rgba = post::finish(&img, exposure, compensation, &preset.post);
    timings.push(("post_ms".into(), t.elapsed().as_secs_f64() * 1000.0));
    if let Some(hdr) = &job.hdr {
        let raw: Vec<f32> = img.px.iter().flat_map(|c| [c.x, c.y, c.z]).collect();
        std::fs::write(hdr, bytemuck::cast_slice(&raw))?;
    }

    let t = Instant::now();
    if let Some(parent) = job.out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let ext = job.out.extension().and_then(|e| e.to_str()).unwrap_or("png").to_ascii_lowercase();
    if ext == "jpg" || ext == "jpeg" {
        let rgb: Vec<u8> = rgba.chunks(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        let f = std::io::BufWriter::new(std::fs::File::create(&job.out)?);
        image::codecs::jpeg::JpegEncoder::new_with_quality(f, 88).encode(&rgb, w as u32, h as u32, image::ExtendedColorType::Rgb8)?;
    } else {
        image::save_buffer(&job.out, &rgba, w as u32, h as u32, image::ColorType::Rgba8)?;
    }
    timings.push(("encode_ms".into(), t.elapsed().as_secs_f64() * 1000.0));
    timings.push(("total_ms".into(), t_total.elapsed().as_secs_f64() * 1000.0 + init_ms));

    let trace_ms = timings.iter().find(|t| t.0 == "trace_ms").map(|t| t.1).unwrap_or(0.0);
    let stats = serde_json::json!({
        "scene": sc.name,
        "out": job.out,
        "width": w, "height": h, "spp": spp,
        "triangles": sc.stats.get("triangles").map_or(sc.tris.len(), |t| *t as usize), "instances": sc.instances.len(),
        "instanced_triangles": sc.stats.get("instanced_triangles").copied().unwrap_or(0.0) as u64, "flat_triangles": sc.tris.len(), "bvh_nodes": prep.nodes.len(), "emitters": prep.emitter_tris.len(), "lights": sc.lights.len(), "textures": sc.textures.len(),
        "adapter": g.adapter_info.name, "backend": format!("{:?}", g.adapter_info.backend), "hwrt": g.hwrt,
        "ev100": ev100, "denoise": denoiser, "preset": preset.name,
        "samples_per_sec": (w * h) as f64 * spp as f64 / (trace_ms / 1000.0).max(1e-9),
        "timings_ms": timings.iter().map(|(k, v)| (k.clone(), serde_json::json!((v * 10.0).round() / 10.0))).collect::<serde_json::Map<_, _>>(),
    });
    if let Some(p) = &job.stats {
        std::fs::write(p, serde_json::to_vec_pretty(&stats)?)?;
    }
    Ok(stats)
}
