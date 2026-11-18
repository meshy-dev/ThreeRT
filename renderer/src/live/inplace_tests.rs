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
            "an instanced object's copy mirrored",
            Box::new(move |w| {
                let mirror = glam::Mat4::from_translation(Vec3::new(-0.2, -0.5, 0.4)) * glam::Mat4::from_scale(Vec3::new(-1.0, 1.5, 1.0));
                w.set_instances(15, 2, Some([mirror.to_cols_array(), glam::Mat4::from_translation(Vec3::new(0.6, 0.6, 0.8)).to_cols_array()].concat()), None, None).unwrap()
            }),
        ),
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

/// An instanced object the build flattened (an emitting material: its copies are triangles of the static groups)
/// is gone from the picture once it is removed: nothing takes it out in place, so the scene is rebuilt.
#[test]
fn a_flattened_instanced_object_removed_is_gone() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let mut w = world();
    w.set_object(15, "copies".into(), None, Kind::Instanced { geometry: 3, procedural: false, materials: vec![5], capacity: 2 }, true, false).unwrap();
    let copies = [glam::Mat4::from_translation(Vec3::new(-0.6, -0.5, 0.4)).to_cols_array(), glam::Mat4::from_translation(Vec3::new(0.5, 0.6, 0.8)).to_cols_array()].concat();
    w.set_instances(15, 2, Some(copies), None, None).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let with = picture(&mut live, 24);
    w.remove(15);
    live.commit(&mut w).unwrap();
    let mut fresh = open(&w).unwrap();
    forget(&mut live);
    let (a, b) = (picture(&mut live, 24), picture(&mut fresh, 24));
    assert!(apart(&with, &b) > 1.0, "the copies were in the picture ({:.2} steps)", apart(&with, &b));
    assert!(apart(&a, &b) < 0.2, "removed: the picture is {:.2} steps from a build's without it", apart(&a, &b));
}

/// An instanced object an edit added in place keeps each copy's own data when its node's data changes, as one the
/// build placed does: the objects table's entries of its copies against a build's.
#[test]
fn an_instanced_object_added_in_place_keeps_its_copies_data() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let mut w = world();
    // (A mesh with its own BLAS: the scene has instances, so another instanced mesh is added in place.)
    w.set_object(14, "own".into(), None, Kind::Mesh { geometry: 3, material: 1, own_blas: true, materials: vec![] }, true, false).unwrap();
    w.set_transform(14, Vec3::new(-0.6, 0.5, 0.7), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = match open(&w) {
        Ok(c) if c.g.hwrt => c,
        Ok(_) => return eprintln!("skipped: no hardware RT"),
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    w.set_object(17, "copies".into(), None, Kind::Instanced { geometry: 3, procedural: false, materials: vec![1], capacity: 2 }, true, false).unwrap();
    let copies = [glam::Mat4::from_translation(Vec3::new(-0.6, -0.5, 0.4)).to_cols_array(), glam::Mat4::from_translation(Vec3::new(0.5, 0.6, 0.8)).to_cols_array()].concat();
    w.set_instances(17, 2, Some(copies), None, Some(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0])).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "added in place ({:?})", live.fallback);
    w.set_custom(17, [9.0; 4]).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "its node's data: in place");
    let fresh = open(&w).unwrap();
    let entries = |c: &LiveCore| {
        let (place, st) = (*c.sc.tables.retained.instanced(17).expect("placed as instances"), c.sc.tables.stride);
        (0..place.capacity as usize).map(|i| c.sc.tables.objects[(place.object as usize + i) * st + 2]).collect::<Vec<_>>()
    };
    assert_eq!(entries(&live), entries(&fresh), "each copy's data, in place against a build's");
    assert_eq!(entries(&fresh), [[1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0]]);
}

/// What the kernel's light sampling reads against the triangles it stands for, on the GPU: every emitter of the list
/// has the light tree's path code for it, and one that is alive its triangle's rows (an instance's: its mesh's
/// triangle placed by the instance, to rounding), its instance and moving flag, and a ray that hits its triangle
/// finds it (the attributes' number, from the instance's first).
fn list_is_its_triangles(what: &str, c: &LiveCore) {
    let read = |b: &wgpu::Buffer, at: u64, bytes: u64| -> Vec<u8> {
        let to = c.g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut e = c.g.device.create_command_encoder(&Default::default());
        e.copy_buffer_to_buffer(b, at, &to, 0, bytes);
        c.g.queue.submit([e.finish()]);
        to.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        c.g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = to.slice(..).get_mapped_range().unwrap().to_vec();
        bytes
    };
    let (tables, up) = (&c.sc.tables, &c.sc.up);
    let size = std::mem::size_of::<crate::GpuEmitterTri>();
    let list = read(&up.heap.buffer, up.heap.at(Table::EmitterTris), (tables.emitter_tris.len() * size) as u64);
    let (verts, attrs) = (read(&up.verts, 0, up.verts.size()), read(&up.attrs, 0, up.attrs.size()));
    let codes = crate::emitter_codes(&tables.emitter_tris, &crate::emitter_order(&tables.emitter_tris, tables.light_tree.as_ref()), tables.light_tree.as_ref());
    // (The CDF as the kernel reads it: an emitter that is not the tree's is found by the entry its code names.)
    let cdf = read(&up.heap.buffer, up.heap.at(Table::Emitters), (tables.emitter_tris.len() * 16) as u64);
    let mut alive = 0;
    for (i, e) in tables.emitter_tris.iter().enumerate() {
        let r: crate::GpuEmitterTri = bytemuck::pod_read_unaligned(&list[i * size..][..size]);
        let code = codes[i];
        assert_eq!(r.code, code, "{what}: emitter {i}'s code (its path in the tree, or its entry in the CDF)");
        if code & !crate::EMITTER_MOVES < 1 << crate::ltree::PATH_BITS {
            let entry: [u32; 4] = bytemuck::pod_read_unaligned(&cdf[(code & !crate::EMITTER_MOVES) as usize * 16..][..16]);
            assert_eq!(entry[0], i as u32, "{what}: the CDF entry emitter {i}'s code names is its own");
        }
        if e.area <= 0.0 {
            continue;
        }
        let rows: Vec<[f32; 4]> = (0..3).map(|k| bytemuck::pod_read_unaligned(&verts[(e.tri as usize * 3 + k) * 16..][..16])).collect();
        let a: scene::GpuTriAttr = bytemuck::pod_read_unaligned(&attrs[e.tri as usize * 96..][..96]);
        if e.inst == crate::NO_INST {
            assert_eq!(r, crate::GpuEmitterTri::new(e, &rows, code & !crate::EMITTER_MOVES), "{what}: emitter {i} of the list against its triangle {}", e.tri);
            assert_eq!(a.x[3].to_bits(), i as u32 + 1, "{what}: triangle {} names its emitter", e.tri);
        } else {
            let (inst, record) = (&c.sc.instances[e.inst as usize], &c.sc.gpu_instances[e.inst as usize]);
            let at = |k: usize| glam::Vec3::new(rows[k][0], rows[k][1], rows[k][2]);
            let placed = [inst.world.transform_point3(at(0)), inst.world.transform_vector3(at(1)), inst.world.transform_vector3(at(2))];
            assert_eq!((r.tri, r.inst, r.code), (e.tri, e.inst, code), "{what}: emitter {i}'s words");
            for (got, want) in [r.v0, r.e1, r.e2].into_iter().zip(placed) {
                assert!((glam::Vec3::from(got) - want).length() <= 1e-4 * (1.0 + want.length()), "{what}: emitter {i}'s triangle {got:?}, its instance places it at {want:?}");
            }
            assert_eq!((record.n[2][3].to_bits(), record.n[2][3].to_bits() + a.x[3].to_bits()), (inst.first_emitter, i as u32 + 1), "{what}: triangle {} of instance {} finds its emitter", e.tri, e.inst);
        }
        alive += 1;
    }
    assert!(alive > 0, "{what}: the list has emitters");
}

/// An instanced object's copies that emit, changed in place, are lit as a build lights them: a copy moved, made
/// larger, shown by the count, hidden by a matrix scaled to 0 and by the count, the node's power changed, the object
/// removed; with and without a light tree. After each: no rebuild, the light list is its triangles, the emitters'
/// weights are a build's, and so is the picture; and the tree's moving subtree is no wider for a copy that is not
/// shown (one waits 300 m away), nor for an object that was removed.
#[test]
fn emitting_copies_changed_in_place_are_lit_as_a_build_lights_them() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    for tree in [false, true] {
        let mut w = world();
        // (The sun and the lamp away: the copies light the scene.)
        w.set_visible(20, false).unwrap();
        w.set_visible(12, false).unwrap();
        w.set_object(15, "copies".into(), None, Kind::Instanced { geometry: 3, procedural: false, materials: vec![5], capacity: 3 }, true, false).unwrap();
        let at = |x: f32, y: f32, z: f32, s: f32| glam::Mat4::from_scale_rotation_translation(Vec3::splat(s), glam::Quat::from_rotation_y(0.3), Vec3::new(x, y, z)).to_cols_array();
        let set = |w: &mut World, count: u32, m: [[f32; 16]; 3]| w.set_instances(15, count, Some(m.concat()), None, None).unwrap();
        // (The third copy waits far away, past the count.)
        let (b, c, far) = (at(0.5, 0.5, 1.0, 0.4), at(0.0, 0.0, 1.4, 0.4), at(300.0, 0.0, 1.4, 0.4));
        set(&mut w, 2, [at(-0.6, -0.4, 0.8, 0.4), b, far]);
        // (Another such object, one copy, stays: what is left in the tree's moving subtree when the first is removed.)
        w.set_object(16, "another".into(), None, Kind::Instanced { geometry: 3, procedural: false, materials: vec![5], capacity: 1 }, true, false).unwrap();
        w.set_instances(16, 1, Some(at(0.7, -0.6, 0.9, 0.3).to_vec()), None, None).unwrap();
        if tree {
            strip(&mut w);
        }
        let mut live = match open(&w) {
            Ok(c) => c,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        w.take_changes();
        if !live.g.hwrt {
            return;
        }
        assert!(live.sc.tables.retained.instanced(15).is_some_and(|p| p.mesh.is_some()), "the copies are instances");
        assert_eq!(live.sc.tables.light_tree.as_ref().is_some_and(|t| t.moving_count > 0), tree, "the copies' emitters are in the tree's moving subtree");
        list_is_its_triangles("emitting copies, as loaded", &live);
        // A copy that is not shown bounds nothing in the tree: the moving subtree holds the shown ones (within 2 m).
        let reach = |c: &LiveCore| c.sc.tables.light_tree.as_ref().and_then(|t| t.moving_sphere()).map_or(0.0, |s| s.1);
        assert!(reach(&live) < 10.0, "as loaded: the moving subtree reaches {} m", reach(&live));
        picture(&mut live, 4);
        type Edit = (&'static str, Box<dyn Fn(&mut World)>);
        let zero = glam::Mat4::from_scale(Vec3::ZERO).to_cols_array();
        let edits: Vec<Edit> = vec![
            ("a copy moved", Box::new(move |w| set(w, 2, [at(-0.2, -0.5, 1.1, 0.4), b, far]))),
            ("a copy three times larger", Box::new(move |w| set(w, 2, [at(-0.2, -0.5, 1.1, 1.2), b, far]))),
            ("a third copy shown", Box::new(move |w| set(w, 3, [at(-0.2, -0.5, 1.1, 1.2), b, c]))),
            ("a copy scaled to 0", Box::new(move |w| set(w, 3, [zero, b, c]))),
            ("two copies past the count", Box::new(move |w| set(w, 1, [at(-0.6, 0.4, 0.9, 0.8), b, c]))),
            ("the node's power", Box::new(move |w| w.set_power(15, 0.25).unwrap())),
            (
                "the object removed",
                Box::new(move |w| {
                    w.remove(15);
                }),
            ),
        ];
        for (what, edit) in &edits {
            let what = format!("{what}, {} a light tree", if tree { "with" } else { "without" });
            edit(&mut w);
            assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
            // (Removed without a tree's strip: no emitter is left to hold.)
            if live.sc.tables.emitter_tris.iter().any(|e| e.area > 0.0) {
                list_is_its_triangles(&what, &live);
            }
            // (The two objects' copies are a metre apart; the one left when the first is removed is 0.1 m wide.)
            if tree {
                let alone = what.starts_with("the object removed");
                assert!(reach(&live) < if alone { 0.3 } else { 10.0 } && (alone || reach(&live) > 0.3), "{what}: the moving subtree reaches {} m", reach(&live));
            }
            lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
        }
    }
}

/// A view that took edits in place against one built from the edited world: the emitters that weigh something are as
/// many, each weighs what a build's does where a build's is, the kernel's total weight is a build's, and so is the
/// picture after 64 frames.
fn lit_as_a_build(what: &str, live: &mut LiveCore, fresh: &mut LiveCore) {
    let weights = |c: &LiveCore| {
        let t = &c.sc.tables;
        let mut v: Vec<(f32, [f32; 3])> = t.emitter_tris.iter().map(|e| (crate::emitter_power(e, &t.materials, &t.objects, t.stride), e.c)).filter(|(p, _)| *p > 0.0).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        v
    };
    let (wl, wf) = (weights(live), weights(fresh));
    assert_eq!(wl.len(), wf.len(), "{what}: as many emitters weigh something as in a build");
    for (l, f) in wl.iter().zip(&wf) {
        assert!((l.0 - f.0).abs() <= 1e-5 * f.0 && (Vec3::from(l.1) - Vec3::from(f.1)).length() <= 1e-4, "{what}: an emitter weighs {l:?}, a build's {f:?}");
    }
    let (tl, tf) = (live.sc.base.misc[1], fresh.sc.base.misc[1]);
    assert!((tl - tf).abs() <= 1e-4 * tf.abs(), "{what}: the kernel's total weight {tl}, a build's {tf}");
    forget(live);
    let (a, b) = (picture(live, 64), picture(fresh, 64));
    assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
}

/// An instanced object whose copies emit, added in place, is lit as a build lights it; so it is after its copies
/// changed in place (one moved, one more shown) and after it was removed. With and without a light tree. After each:
/// no rebuild, the light list is its triangles, the weights and the picture are a build's.
#[test]
fn an_emitting_instanced_object_added_in_place_is_lit_as_a_build_lights_it() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    for tree in [false, true] {
        let mut w = world();
        // (The sun and the lamp away: the copies will light the scene. A mesh with its own BLAS: the scene has
        // instances, so an instanced mesh is added in place.)
        w.set_visible(20, false).unwrap();
        w.set_visible(12, false).unwrap();
        w.set_object(14, "own".into(), None, Kind::Mesh { geometry: 3, material: 1, own_blas: true, materials: vec![] }, true, false).unwrap();
        w.set_transform(14, Vec3::new(-0.6, 0.5, 0.7), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        if tree {
            strip(&mut w);
        }
        let mut live = match open(&w) {
            Ok(c) if c.g.hwrt => c,
            Ok(_) => return eprintln!("skipped: no hardware RT"),
            Err(e) => return eprintln!("skipped: {e}"),
        };
        w.take_changes();
        picture(&mut live, 4);
        let at = |x: f32, y: f32, z: f32, s: f32| glam::Mat4::from_scale_rotation_translation(Vec3::splat(s), glam::Quat::from_rotation_y(0.3), Vec3::new(x, y, z)).to_cols_array();
        let set = |w: &mut World, count: u32, m: [[f32; 16]; 3]| w.set_instances(15, count, Some(m.concat()), None, None).unwrap();
        let (b, c) = (at(0.5, 0.5, 1.0, 0.4), at(0.0, 0.0, 1.4, 0.4));
        type Edit = (&'static str, Box<dyn Fn(&mut World)>);
        let edits: Vec<Edit> = vec![
            (
                "added",
                Box::new(move |w| {
                    w.set_object(15, "copies".into(), None, Kind::Instanced { geometry: 3, procedural: false, materials: vec![5], capacity: 3 }, true, false).unwrap();
                    set(w, 2, [at(-0.6, -0.4, 0.8, 0.4), b, c]);
                }),
            ),
            ("a copy moved, a third shown", Box::new(move |w| set(w, 3, [at(-0.2, -0.5, 1.1, 0.8), b, c]))),
            (
                "removed",
                Box::new(move |w| {
                    w.remove(15);
                }),
            ),
        ];
        for (what, edit) in &edits {
            let what = format!("an emitting instanced object {what} in place, {} a light tree", if tree { "with" } else { "without" });
            // (Its material covers instances' triangles once it is added: `Tables::fixed_mats`.)
            let covers = |c: &LiveCore| c.sc.tables.fixed_mats.contains(&c.sc.tables.retained.materials[&5]);
            let before = covers(&live);
            edit(&mut w);
            assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
            assert!(covers(&live) && (before || what.contains("added")), "{what}: its material is among those of instances' triangles");
            if live.sc.tables.emitter_tris.iter().any(|e| e.area > 0.0) {
                list_is_its_triangles(&what, &live);
            }
            lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
        }
    }
}

/// 300 small emitting triangles: enough for a light tree.
fn strip(w: &mut World) {
    let (mut positions, mut indices) = (vec![], vec![]);
    for q in 0..150u32 {
        let x = -1.5 + q as f32 * 0.02;
        positions.extend([x, -0.9, 0.2, x + 0.008, -0.9, 0.2, x + 0.008, -0.892, 0.2, x, -0.892, 0.2]);
        indices.extend([0, 1, 2, 0, 2, 3].map(|k| q * 4 + k));
    }
    w.set_geometry(6, Geometry { normals: [0.0, 0.0, 1.0].repeat(600), uvs: vec![0.0; 1200], positions, indices, groups: vec![], colors: vec![] }).unwrap();
    w.set_object(17, "strip".into(), None, Kind::Mesh { geometry: 6, material: 5, own_blas: false, materials: vec![] }, true, false).unwrap();
}

/// An emitter that changes size in place is lit as a build lights it: a movable model (its instance is scaled) and a
/// movable mesh (the CPU moves its triangles), the scene's main light, three times larger and three times smaller,
/// with and without a light tree. The weights it was given at load are not what its triangles are now.
#[test]
fn an_emitter_scaled_in_place_is_lit_as_a_build_lights_it() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    for (is_model, tree, grow) in [(true, false, true), (true, true, true), (false, false, true), (false, true, true), (true, false, false), (false, false, false)] {
        let what = format!("a movable {} {} in place, {} a light tree", if is_model { "model" } else { "mesh" }, if grow { "three times larger" } else { "three times smaller" }, if tree { "with" } else { "without" });
        let mut w = world();
        // (The sun and the lamp away: the emitter lights the scene.)
        w.set_visible(20, false).unwrap();
        w.set_visible(12, false).unwrap();
        let (small, large) = if is_model { (0.05f32, 0.15f32) } else { (0.4, 1.2) };
        let (from, to) = if grow { (small, large) } else { (large, small) };
        if is_model {
            w.set_object(31, "movable".into(), None, Kind::Model { uri: "tests/render/data/panel.glb".into(), nodes: None, animation: None, functions: vec![] }, true, true).unwrap();
            w.set_power(31, 60.0).unwrap();
        } else {
            w.set_object(31, "movable".into(), None, Kind::Mesh { geometry: 3, material: 5, own_blas: false, materials: vec![] }, true, true).unwrap();
            w.set_power(31, 1.5).unwrap();
        }
        w.set_transform(31, Vec3::new(0.5, -0.5, 0.6), glam::Quat::IDENTITY, Vec3::splat(from)).unwrap();
        if tree {
            strip(&mut w);
        }
        let mut live = match open(&w) {
            Ok(c) => c,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        w.take_changes();
        if !live.g.hwrt {
            return;
        }
        assert_eq!(live.sc.tables.light_tree.is_some(), tree, "{what}");
        picture(&mut live, 4);
        w.set_transform(31, Vec3::new(0.2, -0.2, 0.9), glam::Quat::from_rotation_z(0.5), Vec3::splat(to)).unwrap();
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        let mut fresh = open(&w).unwrap();
        forget(&mut live);
        let (a, b) = (picture(&mut live, 64), picture(&mut fresh, 64));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
    }
}

/// The light list's copies of its emitters' triangles follow them: as loaded, after an emitter the CPU moves has
/// moved (in the light tree's moving subtree, and without a tree), after an emitting mesh a body drives has moved,
/// after a model's placement or its clip has moved its emitters, after a particle's pass has placed it, and after
/// the edits that make emitters in place (a mesh added, given other triangles and removed, with the tree's slot for
/// them; a material that starts emitting): the moves and edits also against a build's picture.
#[test]
fn the_light_list_holds_its_triangles() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    // An emitter the CPU moves.
    for tree in [true, false] {
        let mut w = world();
        w.set_object(13, "mover".into(), None, Kind::Mesh { geometry: 3, material: 5, own_blas: false, materials: vec![] }, true, true).unwrap();
        w.set_transform(13, Vec3::new(0.35, -0.45, 0.6), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        if tree {
            strip(&mut w);
        }
        let mut live = match open(&w) {
            Ok(c) => c,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        w.take_changes();
        assert_eq!(live.sc.tables.light_tree.as_ref().is_some_and(|t| t.moving_count > 0), tree, "the mover is in the tree's moving subtree");
        assert_eq!(live.sc.up.fills.moved.is_some(), live.g.hwrt, "the mover's emitters are filled on the GPU");
        list_is_its_triangles("a moving emitter, as loaded", &live);
        w.set_transform(13, Vec3::new(0.1, -0.3, 0.9), glam::Quat::from_rotation_z(0.5), Vec3::ONE).unwrap();
        let rebuilt = live.commit(&mut w).unwrap();
        assert!(!(rebuilt && live.g.hwrt), "the mover moved in place ({:?})", live.fallback);
        list_is_its_triangles("a moving emitter, moved", &live);
    }
    // An emitting mesh a body drives: an instance of its mesh, whose emitters follow it.
    let mut w = world();
    w.set_object(13, "driven".into(), None, Kind::Mesh { geometry: 3, material: 5, own_blas: true, materials: vec![] }, true, false).unwrap();
    w.set_driven(13).unwrap();
    w.set_transform(13, Vec3::new(-0.6, 0.5, 0.7), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = open(&w).unwrap();
    w.take_changes();
    assert!(live.sc.tables.emitter_tris.iter().any(|e| e.inst != crate::NO_INST && e.moves), "the driven mesh's emitters are its instance's");
    list_is_its_triangles("a driven emitting mesh, as loaded", &live);
    assert!(w.place_driven(13, Vec3::new(-0.2, 0.3, 1.0), glam::Quat::from_rotation_z(-0.4)));
    let rebuilt = live.commit(&mut w).unwrap();
    assert!(!(rebuilt && live.g.hwrt), "the driven mesh moved in place ({:?})", live.fallback);
    list_is_its_triangles("a driven emitting mesh, moved", &live);
    // A model's emitters (tests/render/data/panel.glb: a grey quad, then a quad that only emits: its emitting triangles
    // are not its mesh's first): a placement that stays, one that is movable and moves, and one posed by a clip that
    // moves its node (a rigid model: the node is an instance), each against a build's picture.
    let open_at_root = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let model = |animation| Kind::Model { uri: "tests/render/data/panel.glb".into(), nodes: None, animation, functions: vec![] };
    let slide = |time| Some(crate::desc::Animation { clip: "slide".into(), time, r#loop: true });
    let small = Vec3::splat(0.05);
    let mut w = world();
    w.set_object(30, "still".into(), None, model(None), true, false).unwrap();
    w.set_transform(30, Vec3::new(-0.6, -0.5, 0.6), glam::Quat::from_rotation_y(0.4), small).unwrap();
    w.set_object(31, "movable".into(), None, model(None), true, true).unwrap();
    w.set_transform(31, Vec3::new(0.5, -0.5, 0.6), glam::Quat::IDENTITY, small).unwrap();
    w.set_object(32, "posed".into(), None, model(slide(0.25)), true, false).unwrap();
    w.set_transform(32, Vec3::new(0.0, 0.6, 0.4), glam::Quat::IDENTITY, small).unwrap();
    let mut live = open_at_root(&w).unwrap();
    w.take_changes();
    if live.g.hwrt {
        let of_instances = live.sc.tables.emitter_tris.iter().filter(|e| e.inst != crate::NO_INST).count();
        assert_eq!(of_instances, 6, "two emitters a placement of the model, each its instance's");
        // (An instance an edit moves in place has moving emitters: no light LOD from a centroid that goes stale, the
        // light tree's moving subtree. One that stays has not.)
        let moves = |id: u32| {
            let records: Vec<u32> = match &live.sc.tables.retained.proxies[&id].draw {
                scene::Draw::Instance(i) => vec![i.instance],
                scene::Draw::Rigid(r) => r.instances.iter().map(|&(_, i)| i).collect(),
                _ => vec![],
            };
            let of: Vec<bool> = live.sc.tables.emitter_tris.iter().filter(|e| records.contains(&e.inst)).map(|e| e.moves).collect();
            (of.len(), of.iter().all(|m| *m))
        };
        assert_eq!((moves(31), moves(32)), ((2, true), (2, true)), "the movable and the posed model's emitters move");
        assert!(live.sc.tables.emitter_tris.iter().filter(|e| e.inst != crate::NO_INST && !e.moves).count() == 2, "the still model's do not");
        list_is_its_triangles("a model's emitters, as loaded", &live);
        type Move = (&'static str, Box<dyn Fn(&mut World)>);
        let moves: Vec<Move> = vec![
            ("a movable model moved", Box::new(move |w| w.set_transform(31, Vec3::new(0.2, -0.2, 0.9), glam::Quat::from_rotation_z(0.5), small).unwrap())),
            ("a posed model's node moved by its clip", Box::new(move |w| w.set_animation(32, slide(0.75)).unwrap())),
        ];
        for (what, edit) in &moves {
            edit(&mut w);
            assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
            list_is_its_triangles(what, &live);
            let mut fresh = open_at_root(&w).unwrap();
            forget(&mut live);
            let (a, b) = (picture(&mut live, 24), picture(&mut fresh, 24));
            assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        }
    }
    // A light-sampled particle: collapsed as loaded, placed by its pass in the first frame.
    let mut w = world();
    w.set_particle_fn("spark", "fn pf_spark(i: PIn, p: array<vec4<f32>, 8>) -> POut { return POut(vec3<f32>(0.3, 0.2, 1.0), vec3<f32>(0.05, 0.0, 0.0), vec3<f32>(0.0, 0.05, 0.0), true, vec4<f32>(0.0)); }".into(), String::new());
    w.set_object(18, "sparks".into(), None, Kind::Particles { count: 3, emitter: "spark".into(), params: vec![], material: 5, emitter_area: 0.01 }, true, false).unwrap();
    let mut live = open(&w).unwrap();
    if live.g.hwrt {
        let spark = live.sc.tables.emitter_tris.iter().position(|e| e.nominal).expect("the particles are emitters");
        let side = |c: &LiveCore| {
            let at = c.sc.up.heap.at(Table::EmitterTris) + (spark * std::mem::size_of::<crate::GpuEmitterTri>()) as u64;
            let to = c.g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 48, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let mut e = c.g.device.create_command_encoder(&Default::default());
            e.copy_buffer_to_buffer(&c.sc.up.heap.buffer, at, &to, 0, 48);
            c.g.queue.submit([e.finish()]);
            to.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            c.g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
            let r: crate::GpuEmitterTri = bytemuck::pod_read_unaligned(&to.slice(..).get_mapped_range().unwrap());
            Vec3::from(r.e1).length()
        };
        list_is_its_triangles("a particle, as loaded", &live);
        assert_eq!(side(&live), 0.0, "a particle is collapsed until its pass places it");
        picture(&mut live, 1);
        list_is_its_triangles("a particle, placed", &live);
        assert!(side(&live) > 0.01, "the list has the particle where its pass placed it ({})", side(&live));
    }
    // Emitters made in place, in a scene with the tree's slot for them (one that has a mesh of its own: its geometry
    // can be edited).
    let mut w = world();
    strip(&mut w);
    w.set_material(7, serde_json::from_value(serde_json::json!({ "name": "m", "color": [0.5, 0.5, 0.5], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true })).unwrap());
    w.set_object(14, "own".into(), None, Kind::Mesh { geometry: 3, material: 7, own_blas: true, materials: vec![] }, true, false).unwrap();
    w.set_transform(14, Vec3::new(-0.6, 0.5, 0.7), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = open(&w).unwrap();
    w.take_changes();
    list_is_its_triangles("as loaded", &live);
    if !live.g.hwrt {
        return;
    }
    assert!(live.sc.tables.light_tree.as_ref().is_some_and(|t| t.dynamic.is_some()), "the tree has the slot");
    type Edit = (&'static str, Box<dyn Fn(&mut World)>);
    let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: [0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
    let edits: Vec<Edit> = vec![
        (
            "an emitting mesh added",
            Box::new(move |w| {
                w.set_geometry(8, quad(0.25)).unwrap();
                w.set_object(16, "added".into(), None, Kind::Mesh { geometry: 8, material: 5, own_blas: true, materials: vec![] }, true, false).unwrap();
                w.set_transform(16, Vec3::new(0.0, 0.65, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
            }),
        ),
        ("the added mesh's triangles changed", Box::new(move |w| w.set_geometry(8, quad(0.15)).unwrap())),
        ("a material starts emitting", Box::new(|w| w.set_material(1, serde_json::from_value(serde_json::json!({ "name": "m", "color": [0.8, 0.4, 0.8], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [1, 1, 1], "emissiveIntensity": 2, "doubleSided": true })).unwrap()))),
        ("the added mesh removed", Box::new(|w| drop(w.remove(16)))),
    ];
    for (what, edit) in &edits {
        let before = live.sc.tables.emitter_tris.len();
        let added = live.sc.tables.retained.instance(16).map(|i| i.instance);
        edit(&mut w);
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        list_is_its_triangles(what, &live);
        // (An instance that is gone lights nothing: its run of the list has no weight.)
        if live.sc.tables.retained.instance(16).is_none() {
            let left: Vec<f32> = live.sc.tables.emitter_tris.iter().filter(|e| Some(e.inst) == added).map(|e| e.area).collect();
            assert!(!left.is_empty() && left.iter().all(|a| *a == 0.0), "{what}: the removed instance's emitters keep no weight ({left:?})");
        }
        // (A mesh given other triangles takes its entries back; the other edits that add emitters make new ones.)
        let grew = live.sc.tables.emitter_tris.len() > before;
        assert_eq!(grew, matches!(*what, "an emitting mesh added" | "a material starts emitting"), "{what}: the list's length");
        // (The list agrees with the instances; that the instances are where a build puts them is the picture's to say.)
        let mut fresh = open(&w).unwrap();
        forget(&mut live);
        let (a, b) = (picture(&mut live, 24), picture(&mut fresh, 24));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
    }
}
