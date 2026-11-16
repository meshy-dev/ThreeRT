//! An edit the live view takes in place leaves it as a build of the edited world does: no rebuild, the same
//! parameters and kernel variant, the same picture. (GPU tests: skipped without a GPU.)

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

/// A view's state: the kernel's parameters whole (as words; not the camera's four vectors, which the frame writes from
/// the view), and what else these edits reach: the lights' table and the sun, the sky's map, what the passes after
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
    let camera = (std::mem::offset_of!(gpu::Params, cam_up) + 16) / 4;
    assert_eq!((std::mem::offset_of!(gpu::Params, cam_pos), camera), (0, 16), "the camera's vectors are the parameters' first");
    (bytemuck::cast_slice::<_, u32>(bytemuck::bytes_of(&s.base))[camera..].to_vec(), rest)
}

/// Holds a view that took an edit in place to one built from the edited world.
fn same_state(what: &str, live: &LiveCore, fresh: &LiveCore) {
    let ((a, ra), (b, rb)) = (state(live), state(fresh));
    let words: Vec<usize> = (0..a.len()).filter(|&k| a[k] != b[k]).map(|k| k + 16).collect();
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
}
