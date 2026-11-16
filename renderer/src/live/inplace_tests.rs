//! An edit the live view takes in place leaves it as a build of the edited world does: no rebuild, the same
//! parameters and kernel variant, the same picture. So does another size. (GPU tests: skipped without a GPU.)

use super::*;
use crate::world::{Geometry, Kind, Settings};
use glam::Vec3;

/// A wall, a panel before it and an emissive quad that lights both, a directional light (20), no sky; camera 4.
fn world() -> World {
    let mut w = World::default();
    let mat = |color: f32, emissive: f32| serde_json::from_value(serde_json::json!({ "name": "m", "color": [color, color * 0.5, color], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [1, 1, 1], "emissiveIntensity": emissive, "doubleSided": true })).unwrap();
    w.set_material(1, mat(0.8, 0.0));
    w.set_material(5, mat(0.0, 40.0));
    let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: [0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
    w.set_geometry(2, quad(1.0)).unwrap();
    w.set_geometry(3, quad(0.25)).unwrap();
    let mesh = |geometry, material| Kind::Mesh { geometry, material, own_blas: false, materials: vec![] };
    w.set_object(10, "wall".into(), None, mesh(2, 1), true, false).unwrap();
    w.set_object(11, "panel".into(), None, mesh(3, 1), true, false).unwrap();
    w.set_transform(11, Vec3::new(-0.4, 0.0, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    w.set_object(12, "lamp".into(), None, mesh(3, 5), true, false).unwrap();
    w.set_transform(12, Vec3::new(0.6, 0.3, 1.5), glam::Quat::from_rotation_y(2.6), Vec3::ONE).unwrap();
    let sun = serde_json::from_value(serde_json::json!({ "type": "directional", "color": [1.0, 0.9, 0.8], "castShadow": true, "intensity": 2.0, "angularDiameter": 0.5 })).unwrap();
    w.set_object(20, "sun".into(), None, Kind::Light(sun), true, false).unwrap();
    w.set_transform(20, Vec3::ZERO, glam::Quat::from_rotation_x(-0.5) * glam::Quat::from_rotation_y(0.4), Vec3::ONE).unwrap();
    w.set_object(4, "camera".into(), None, Kind::Camera(camera(50.0, None, manual(0.0))), true, false).unwrap();
    w.set_transform(4, Vec3::new(0.0, 0.0, 5.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    w.set_settings(Settings { camera: Some(4), ..Default::default() });
    w
}

fn sky(v: serde_json::Value) -> crate::desc::Sky {
    serde_json::from_value(v).unwrap()
}

/// An atmosphere whose sun is light 20.
fn atmosphere(dynamic: bool) -> serde_json::Value {
    serde_json::json!({ "type": "atmosphere", "sun": 20, "groundAlbedo": [0.3, 0.3, 0.3], "rayleighScale": 1, "mieScale": 1, "mieAnisotropy": 0.8, "skyLightIntensity": 1, "dynamic": dynamic })
}

fn manual(ev: f32) -> serde_json::Value {
    serde_json::json!({ "mode": "manual", "ev100": ev })
}

fn camera(fov: f32, dof: Option<f32>, exposure: serde_json::Value) -> crate::desc::Camera {
    let mut c = serde_json::json!({ "fov": fov, "near": 0.1, "far": 100, "exposure": exposure });
    if let Some(focus) = dof {
        c["dof"] = serde_json::json!({ "focusDistance": focus, "fStop": 1.4, "sensorWidth": 36 });
    }
    serde_json::from_value(c).unwrap()
}

/// A view's state: the kernel's parameters whole (as words; not the camera's four vectors and the size, which the frame
/// writes from the view), and what else these edits reach: the lights' table and the sun, the sky's map, what the passes after
/// the kernel take. (After a frame: the frame writes the light LOD's reference.)
fn state(c: &LiveCore) -> (Vec<u32>, String) {
    let (s, v, sky) = (&c.sc, &c.view, &c.env.sky);
    let rest = format!(
        "features {:#x} | sun {:?} lights {:?} | environment {:?} sky {:?} {} {} | exposure {} {:?} | {:?} {:?} | post {} {:?} {} {:?} {:?} | {}",
        s.up.kernel.features, s.tables.retained.sun, bytemuck::cast_slice::<_, u32>(&s.tables.lights),
        // (Not the altitude the sky was baked for: a view keeps its sky when its camera moves, a build bakes anew.)
        c.env.settings, sky.map, sky.floor, sky.cover,
        v.exposure, v.exposure_settings, v.camera, v.dof,
        c.post_key, c.bloom, c.vignette, c.post, c.blur, c.scene_name
    );
    assert_eq!((std::mem::offset_of!(gpu::Params, cam_pos), std::mem::offset_of!(gpu::Params, size) + 16), (0, FRAME_WORDS * 4), "the camera's vectors and the size are the parameters' first");
    (bytemuck::cast_slice::<_, u32>(bytemuck::bytes_of(&s.base))[FRAME_WORDS..].to_vec(), rest)
}

/// The parameters' words the frame writes from the view: the camera's four vectors, the size.
const FRAME_WORDS: usize = 20;

/// Holds a view that took an edit in place to one built from the edited world.
fn same_state(what: &str, live: &LiveCore, fresh: &LiveCore) {
    let ((a, ra), (b, rb)) = (state(live), state(fresh));
    let words: Vec<usize> = (0..a.len()).filter(|&k| a[k] != b[k]).map(|k| k + FRAME_WORDS).collect();
    assert!(words.is_empty(), "{what}: the kernel's parameters differ from a build's at words {words:?}");
    assert_eq!(ra, rb, "{what}: the view's state against a build's");
}

/// The output (BGRA8) after `frames` frames in lockstep.
fn picture(core: &mut LiveCore, frames: u32) -> Vec<u8> {
    core.fixed_dt = Some(1.0 / 60.0);
    for _ in 0..frames {
        core.render(1, |_, _| {});
    }
    let (d, out) = (&core.g.device, &core.targets.out);
    let to = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: out.size(), usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut e = d.create_command_encoder(&Default::default());
    e.copy_buffer_to_buffer(out, 0, &to, 0, out.size());
    core.g.queue.submit([e.finish()]);
    to.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    d.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let bytes = to.slice(..).get_mapped_range().unwrap().to_vec();
    bytes
}

/// The view without its histories (the radiance cache emptied, the frames' histories reset), as a new view is.
fn forget(core: &mut LiveCore) {
    if let Some(c) = &core.cache {
        let mut e = core.g.device.create_command_encoder(&Default::default());
        c.bufs.clear(&mut e);
        core.g.queue.submit([e.finish()]);
    }
    core.reset = true;
}

/// Mean absolute difference of two pictures, in 8-bit steps.
fn apart(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as f64).sum::<f64>() / a.len() as f64
}

/// Each edit, on one view that takes them one after another, against a view built from the world as it is then.
#[test]
fn edits_in_place_leave_what_a_build_does() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let mut w = world();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let mut before = picture(&mut live, 24);
    let fog = |density: f32, volumetric: bool| -> crate::desc::Fog {
        let mut f = serde_json::json!({ "density": density, "heightFalloff": 0.05, "height": 0.0, "color": [0.4, 0.5, 0.7], "intensity": 1.0 });
        if volumetric {
            f["volumetric"] = serde_json::json!({ "scattering": 0.9 });
        }
        serde_json::from_value(f).unwrap()
    };
    type Edit = (&'static str, bool, Box<dyn Fn(&mut World)>);
    let edits: Vec<Edit> = vec![
        ("a longer lens", true, Box::new(|w| w.set_camera_desc(4, camera(25.0, None, manual(0.0))).unwrap())),
        ("a depth of field", false, Box::new(|w| w.set_camera_desc(4, camera(25.0, Some(4.5), manual(0.0))).unwrap())),
        ("a manual exposure", true, Box::new(|w| w.set_camera_desc(4, camera(25.0, Some(4.5), manual(1.5))).unwrap())),
        ("auto exposure", false, Box::new(|w| w.set_camera_desc(4, camera(25.0, Some(4.5), serde_json::json!({ "mode": "auto", "compensation": 0.5, "minEv100": -3, "maxEv100": 4 }))).unwrap())),
        ("manual again, no depth of field", false, Box::new(|w| w.set_camera_desc(4, camera(25.0, None, manual(0.0))).unwrap())),
        ("fog", true, Box::new(move |w| w.set_environment(crate::desc::Environment { fog: Some(fog(0.25, false)), ..w.environment.clone() }))),
        ("volumetric fog", false, Box::new(move |w| w.set_environment(crate::desc::Environment { fog: Some(fog(0.25, true)), ..w.environment.clone() }))),
        ("no fog", true, Box::new(|w| w.set_environment(crate::desc::Environment { fog: None, ..w.environment.clone() }))),
        ("a background", true, Box::new(|w| w.set_environment(crate::desc::Environment { background: Some([0.2, 0.6, 0.1]), ..w.environment.clone() }))),
        ("no background", true, Box::new(|w| w.set_environment(crate::desc::Environment { background: None, ..w.environment.clone() }))),
        ("a sky of one colour", true, Box::new(|w| w.set_environment(crate::desc::Environment { sky: sky(serde_json::json!({ "type": "color", "color": [0.3, 0.5, 1.0], "intensity": 0.5, "skyLightIntensity": 1 })), ..w.environment.clone() }))),
        ("a gradient sky (a map where there was none)", true, Box::new(|w| w.set_environment(crate::desc::Environment { sky: sky(serde_json::json!({ "type": "gradient", "zenith": [0.1, 0.2, 0.9], "horizon": [0.9, 0.5, 0.2], "ground": [0.1, 0.1, 0.1], "intensity": 0.8, "skyLightIntensity": 1 })), ..w.environment.clone() }))),
        ("an atmosphere with its sun", true, Box::new(|w| w.set_environment(crate::desc::Environment { sky: sky(atmosphere(false)), ..w.environment.clone() }))),
        ("the atmosphere as a dynamic sky (a map of another size)", false, Box::new(|w| w.set_environment(crate::desc::Environment { sky: sky(atmosphere(true)), ..w.environment.clone() }))),
        ("the sun turned (a sun's move)", true, Box::new(|w| w.set_transform(20, Vec3::ZERO, glam::Quat::from_rotation_x(-0.9) * glam::Quat::from_rotation_y(-0.3), Vec3::ONE).unwrap())),
        ("one colour again (the sun is a light of its own)", true, Box::new(|w| w.set_environment(crate::desc::Environment { sky: sky(serde_json::json!({ "type": "color", "color": [1.0, 1.0, 1.0], "intensity": 0.2, "skyLightIntensity": 1 })), ..w.environment.clone() }))),
        ("the light turned and made an atmosphere's sun at once", true, Box::new(|w| {
            w.set_transform(20, Vec3::ZERO, glam::Quat::from_rotation_x(-0.3) * glam::Quat::from_rotation_y(0.8), Vec3::ONE).unwrap();
            w.set_environment(crate::desc::Environment { sky: sky(atmosphere(false)), ..w.environment.clone() })
        })),
        ("no sky", true, Box::new(|w| w.set_environment(crate::desc::Environment { sky: sky(serde_json::json!({ "type": "color", "color": [0, 0, 0], "intensity": 0, "skyLightIntensity": 1 })), ..w.environment.clone() }))),
        ("the preset's overrides", false, Box::new(|w| w.set_settings(Settings { overrides: Some(serde_json::json!({ "native": { "maxBounces": 1, "energyCompensation": 0.0, "clampIndirect": 3.0 }, "post": { "vignette": 0.9 } })), name: "renamed".into(), ..w.settings.clone() }))),
        ("another preset", false, Box::new(|w| w.set_settings(Settings { preset: "preview".into(), overrides: None, ..w.settings.clone() }))),
    ];
    for (what, seen, edit) in &edits {
        edit(&mut w);
        let t = std::time::Instant::now();
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place");
        eprintln!("{what}: committed in {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
        let mut fresh = open(&w).unwrap();
        // (With no history, as the new view has none: the radiance cache and the frames' histories hold the light as
        // it was for a while, here as after a rebuild; fog gone was still 5 steps off after 264 frames.)
        let now = picture(&mut live, 2);
        forget(&mut live);
        let (a, b) = (picture(&mut live, 24), picture(&mut fresh, 24));
        assert!(apart(&a, &b) < 2.0, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        same_state(what, &live, &fresh);
        if *seen {
            assert!(apart(&now, &before) > 1.0, "{what}: the picture did not change at once ({:.2} steps)", apart(&now, &before));
        }
        before = a;
    }
    // Another camera becomes the scene's: its place, its lens and its exposure.
    w.set_object(6, "second".into(), None, Kind::Camera(camera(70.0, None, manual(-1.0))), true, false).unwrap();
    w.set_transform(6, Vec3::new(1.0, 0.5, 4.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    live.commit(&mut w).unwrap();
    w.set_settings(Settings { camera: Some(6), ..w.settings.clone() });
    assert!(!live.commit(&mut w).unwrap(), "another camera: in place");
    let mut fresh = open(&w).unwrap();
    (picture(&mut live, 1), picture(&mut fresh, 1));
    same_state("another camera", &live, &fresh);
    assert!(live.cam == fresh.cam, "the view is at the scene's camera");
    // A view moved away from the scene's camera stays there through a depth of field or an exposure; a lens takes it back.
    let away = Camera { pos: live.cam.pos + Vec3::new(0.5, 0.0, 0.0), ..live.cam };
    live.cam = away;
    w.set_camera_desc(6, camera(70.0, Some(3.0), manual(2.0))).unwrap();
    assert!(!live.commit(&mut w).unwrap() && live.cam == away && live.view.dof.is_some(), "a depth of field and an exposure: the view stays");
    w.set_camera_desc(6, camera(40.0, Some(3.0), manual(2.0))).unwrap();
    assert!(!live.commit(&mut w).unwrap() && live.cam == live.view.camera && live.cam != away, "a lens: the view takes the scene's camera");
    // A commit that rebuilds (another mesh) and brings another preset with it.
    w.set_object(13, "more".into(), None, Kind::Mesh { geometry: 3, material: 1, own_blas: false, materials: vec![] }, true, false).unwrap();
    w.set_settings(Settings { preset: "cinematic".into(), overrides: Some(serde_json::json!({ "post": { "vignette": 0.3 } })), ..w.settings.clone() });
    assert!(live.commit(&mut w).unwrap(), "another mesh: a rebuild");
    let mut fresh = open(&w).unwrap();
    (picture(&mut live, 1), picture(&mut fresh, 1));
    same_state("a rebuild with another preset", &live, &fresh);
}

/// A view at another size, against a view built at that size: the same state and picture, with the scene's data and
/// the radiance cache it had; an edit and a rebuild after it as in a view of that size; a size the device does not
/// hold refused.
#[test]
fn a_resized_view_is_a_view_built_at_that_size() {
    let presets = crate::default_presets_dir();
    let open = |w: &World, (x, y): (u32, u32)| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, x, y));
    let mut w = world();
    let mut live = match open(&w, (96, 54)) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    picture(&mut live, 8);
    let (verts, cache, out) = (live.sc.up.verts.clone(), live.targets.kernel.cache.val.buffer.clone(), live.targets.out.clone());
    live.resize(96, 54).unwrap();
    assert!(live.targets.out == out && !live.reset, "the same size: the view as it was");
    let against = |what: &str, live: &mut LiveCore, w: &World| {
        let size = (live.w, live.h);
        let mut fresh = open(w, size).unwrap();
        forget(live);
        let (a, b) = (picture(live, 24), picture(&mut fresh, 24));
        assert_eq!(a.len(), (size.0 * size.1 * 4) as usize, "{what}: the output's size");
        // (0.02 to 0.13 steps as measured; 9 with the scene bound to the old size's samples.)
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        same_state(what, live, &fresh);
    };
    // (Smaller than a tile of 8 too.)
    for size in [(160, 90), (54, 96), (97, 55), (5, 3), (1, 1), (96, 54)] {
        live.resize(size.0, size.1).unwrap();
        assert!(live.reset && (live.w, live.h) == size, "{size:?}: the histories start again");
        assert!(live.sc.up.verts == verts, "{size:?}: the scene's data is as it was");
        assert!(live.g.merged || live.targets.kernel.cache.val.buffer == cache, "{size:?}: the radiance cache is as it was");
        against(&format!("{size:?}"), &mut live, &w);
    }
    // An edit in place, then a rebuild, in a view that was resized.
    live.resize(128, 72).unwrap();
    w.set_camera_desc(4, camera(25.0, Some(4.5), manual(1.0))).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "a lens after another size: in place");
    against("a lens after another size", &mut live, &w);
    live.rebuild(&w).unwrap();
    against("a rebuild after another size", &mut live, &w);
    // More pixels than a buffer holds.
    assert!(live.resize(1 << 20, 1 << 20).is_err() && (live.w, live.h) == (128, 72), "a size the device does not hold is refused");
    against("a size refused", &mut live, &w);
    // A display of another density: dynamic resolution starts again from its points' resolution, a fixed scale stays.
    live.set_scale(0.0);
    live.set_display_scale(2.0);
    assert!(live.dynamic_res && live.render_scale == 0.5, "dynamic resolution on a display of two pixels a point");
    live.set_scale(0.75);
    live.set_display_scale(1.0);
    assert!(!live.dynamic_res && live.render_scale == 0.75, "a fixed scale through another density");
}

/// Moves and geometry edits a view takes in place, alone and several in one commit, against a view built from the
/// world as it is then: the same picture, so the acceleration structures hold what moved, built once a commit
/// (`LiveCore::build_top`). Each kind of object that
/// moves its own way: triangles re-transformed (a movable mesh), an instance's transform (a mesh a body drives), an
/// instanced object's copies, a mesh added and removed in place.
#[test]
fn moves_in_place_leave_what_a_build_does() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let mut w = world();
    let mesh = |own_blas| Kind::Mesh { geometry: 3, material: 1, own_blas, materials: vec![] };
    w.set_object(13, "mover".into(), None, mesh(false), true, true).unwrap();
    w.set_transform(13, Vec3::new(0.35, -0.45, 0.6), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    w.set_object(14, "driven".into(), None, mesh(true), true, false).unwrap();
    w.set_driven(14).unwrap();
    w.set_transform(14, Vec3::new(-0.6, 0.5, 0.7), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    w.set_object(15, "copies".into(), None, Kind::Instanced { geometry: 3, procedural: false, materials: vec![1], capacity: 2 }, true, false).unwrap();
    let copies = |x: f32| [glam::Mat4::from_translation(Vec3::new(x, -0.5, 0.4)).to_cols_array(), glam::Mat4::from_translation(Vec3::new(0.6, 0.6, 0.8)).to_cols_array()].concat();
    w.set_instances(15, 2, Some(copies(-0.7)), None, None).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let at = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
    type Edit = (&'static str, Box<dyn Fn(&mut World)>);
    let edits: Vec<Edit> = vec![
        ("a movable mesh moved", Box::new(move |w| w.set_transform(13, at(0.1, -0.3, 0.9), glam::Quat::from_rotation_z(0.5), Vec3::ONE).unwrap())),
        ("a driven mesh moved", Box::new(move |w| assert!(w.place_driven(14, at(-0.2, 0.3, 1.0), glam::Quat::from_rotation_z(-0.4))))),
        ("an instanced object's copy moved", Box::new(move |w| w.set_instances(15, 2, Some(copies(-0.2)), None, None).unwrap())),
        (
            "all three moved at once",
            Box::new(move |w| {
                w.set_transform(13, at(0.5, -0.1, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
                assert!(w.place_driven(14, at(-0.7, 0.1, 0.6), glam::Quat::IDENTITY));
                w.set_instances(15, 2, Some(copies(-0.9)), None, None).unwrap();
            }),
        ),
        (
            "a mesh added, the driven one moved",
            Box::new(move |w| {
                w.set_object(16, "added".into(), None, mesh(true), true, false).unwrap();
                w.set_transform(16, at(0.0, 0.65, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
                assert!(w.place_driven(14, at(-0.3, -0.1, 1.1), glam::Quat::IDENTITY));
            }),
        ),
        (
            "the added mesh removed, the movable one moved",
            Box::new(move |w| {
                w.remove(16);
                w.set_transform(13, at(0.2, 0.2, 0.7), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
            }),
        ),
    ];
    for (what, edit) in &edits {
        edit(&mut w);
        let builds = live.top_builds;
        let rebuilt = live.commit(&mut w).unwrap();
        // (Software traversal places some of these another way, and may rebuild: the picture holds either way.)
        assert!(!(rebuilt && live.g.hwrt), "{what}: in place ({:?})", live.fallback);
        assert_eq!(live.top_builds - builds, u32::from(!rebuilt), "{what}: one build above the triangles a commit, whatever moved");
        let mut fresh = open(&w).unwrap();
        forget(&mut live);
        let (a, b) = (picture(&mut live, 24), picture(&mut fresh, 24));
        // (0.02 to 0.08 steps as measured; 0.9 after a move with nothing built above the triangles.)
        assert!(apart(&a, &b) < 0.2, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
    }
}
