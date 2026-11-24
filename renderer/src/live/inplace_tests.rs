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
    // (A mesh of two sections behind the camera: an instance's rows are not at its record's index, here and for
    // everything an edit places after.)
    w.set_geometry(9, Geometry { groups: vec![[0, 3, 0], [3, 3, 1]], ..quad(0.05) }).unwrap();
    w.set_object(9, "two".into(), None, Kind::Mesh { geometry: 9, material: 1, own_blas: false, materials: vec![1, 1] }, true, false).unwrap();
    w.set_transform(9, Vec3::new(0.0, 0.0, 9.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
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

/// Whether rows `at .. at + n` are the room's to give again (a hole holds them, or they are past what is in use): what
/// an object an edit placed gives back when it goes.
fn rows_free(c: &LiveCore, at: u32, n: u32) -> bool {
    let rows = &c.sc.edit.rows;
    at as usize >= rows.end || rows.free.iter().any(|(&start, &len)| start <= at as usize && (at + n) as usize <= start + len)
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

/// Whether the rays see the instance at `record`: by its mask above the triangles, or its unit.
fn seen_by_rays(c: &LiveCore, record: u32) -> bool {
    match (&c.sc.up.accel, &c.sc.up.units) {
        (Some(a), _) => a.tlas[a.inst_base + record as usize].as_ref().is_some_and(|t| t.mask != 0),
        (None, Some(u)) => u.slots.get(record as usize).copied().flatten().is_some_and(|x| !x.hidden),
        _ => false,
    }
}

/// Part of a GPU buffer, read back.
fn read_back<T: bytemuck::Pod>(c: &LiveCore, b: &wgpu::Buffer, at: u64, bytes: u64) -> Vec<T> {
    let to = c.g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let mut e = c.g.device.create_command_encoder(&Default::default());
    e.copy_buffer_to_buffer(b, at, &to, 0, bytes);
    c.g.queue.submit([e.finish()]);
    to.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    c.g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let words = bytemuck::pod_collect_to_vec(&to.slice(..).get_mapped_range().unwrap());
    words
}

/// Triangles as the kernel reads them (through their indices): a triangle's corners (9 numbers), their normals (9),
/// their UVs (6), its first vertex's tangent (4; zeros: none), its row (4 words), and its corners' positions of the
/// previous frame (9; a mesh that deforms).
#[derive(Default)]
struct Triangles {
    p: Vec<f32>,
    n: Vec<f32>,
    uv: Vec<f32>,
    t: Vec<f32>,
    rows: Vec<u32>,
    prev: Vec<f32>,
}

/// `n` triangles from `first` as the GPU has them, their vertices counted from `base`; their previous positions from
/// `prev` on (None: not read). Software traversal: each one's leaf is held to the triangle its indices name (v0, e1,
/// e2, bit for bit), and `rows` has the leaves' ids as a row (a mesh's: its section, -, -, its index there).
fn triangles_on_gpu(c: &LiveCore, first: u64, n: u64, base: u32, prev: Option<u32>) -> Triangles {
    use crate::heap::Stream;
    let up = &c.sc.up;
    let index: Vec<u32> = read_back(c, &up.words.buffer, up.words.at(Stream::Indices) + first * 12, n * 12);
    let vertices: Vec<crate::indexed::Vertex> = read_back(c, &up.vertices, 0, up.vertices.size());
    let mut rows = vec![];
    if !c.g.hwrt {
        let size = crate::indexed::LEAF_BYTES as u64;
        let leaves: Vec<crate::indexed::Leaf> = read_back(c, &up.bg_parts.leaves, first * size, n * size);
        let bits = |l: &crate::indexed::Leaf| l.map(|r| r.map(f32::to_bits));
        for (j, (l, tri)) in leaves.iter().zip(index.chunks_exact(3)).enumerate() {
            let p = |k: usize| glam::Vec3A::from_slice(&vertices[(base + tri[k]) as usize][0][..3]);
            let ids = [l[0][3].to_bits(), l[1][3].to_bits()];
            assert_eq!(bits(l), bits(&crate::indexed::leaf([p(0), p(1), p(2)], ids)), "triangle {}: its leaf is the triangle its indices name", first + j as u64);
            rows.extend([ids[0], 0, 0, ids[1]]);
        }
    }
    let before: Vec<crate::indexed::Vertex> = prev.map_or(vec![], |_| read_back(c, &up.prev_vertices, 0, up.prev_vertices.size()));
    let mut out = Triangles { rows, ..Default::default() };
    for tri in index.chunks_exact(3) {
        for &i in tri {
            let v = &vertices[(base + i) as usize];
            out.p.extend(&v[0][..3]);
            out.n.extend(&v[1][..3]);
            out.uv.extend([v[0][3], v[1][3]]);
            out.t.extend(v[2]);
            if let Some(p) = prev {
                out.prev.extend(&before[(p + i) as usize][0][..3]);
            }
        }
    }
    out
}

/// A skinned model's mesh as the CPU skin gives it for a pose, in the same form: a normal where the model has one
/// (else zeros: the triangle's own is made where it is hit), a tangent a corner where the model has them.
fn skinned_on_cpu(c: &LiveCore, sk: &scene::Skinned, a: &crate::desc::Animation) -> Triangles {
    let tris = scene::model_gltf(&sk.uri, Path::new("..")).unwrap().triangles(glam::Mat4::IDENTITY, None, Some(a)).unwrap();
    let (model, index) = &c.sc.skins[sk.source as usize];
    let has = |j: usize, k: usize, bit: u32| model[index[j][k] as usize].t[3].to_bits() & bit != 0;
    let mut out = Triangles::default();
    for (j, &(k, section, rank)) in sk.order.iter().enumerate() {
        let q = &tris[k as usize];
        for corner in 0..3 {
            out.p.extend(q.p[corner].to_array());
            out.n.extend(if has(j, corner, crate::skin::CORNER_NORMAL) { q.n[corner].to_array() } else { [0.0; 3] });
            out.uv.extend(q.uv[corner].to_array());
            out.t.extend(match q.tangent.filter(|_| has(j, corner, crate::skin::CORNER_TANGENT)) {
                Some(t) => [t[corner].0.x, t[corner].0.y, t[corner].0.z, t[corner].1],
                None => [0.0; 4],
            });
        }
        out.rows.extend([section, 0, 0, rank]);
    }
    out
}

/// Numbers within a rounding of each other.
fn same_numbers(what: &str, got: &[f32], want: &[f32], stride: usize) {
    assert_eq!(got.len(), want.len(), "{what}: its length");
    for (i, (g, x)) in got.iter().zip(want).enumerate() {
        assert!((g - x).abs() <= 2e-5 * x.abs().max(1.0), "{what}: number {} of triangle {}: {g} ({:#x}), the CPU's {x} ({:#x})", i % stride, i / stride, g.to_bits(), x.to_bits());
    }
}

/// A posed skinned model's mesh on the GPU against the CPU skin of `now` (corners, normals, UVs, tangents), its
/// vertices' previous positions against that of `before`, and the mesh as the CPU has it when asked (`name`: its
/// node). Its record has where its vertices and their previous positions are, and that its mesh deforms. Which
/// section a triangle is in and its index there: under software traversal the tree's rows, word for word; under
/// hardware traversal the triangle is its section's first (its instance's row's) and its index on. The CPU's corners.
fn skinned_is_the_cpus(what: &str, c: &LiveCore, sk: &scene::Skinned, name: &str, now: &crate::desc::Animation, before: &crate::desc::Animation) -> Vec<f32> {
    let (inst, record, mesh) = (&c.sc.instances[sk.instance as usize], &c.sc.gpu_instances[sk.instance as usize], &c.sc.meshes[sk.mesh as usize]);
    assert_eq!((record.n[2][3].to_bits(), record.info[2], inst.verts), (inst.prev, mesh.vfirst, mesh.vfirst), "{what}: the record has where the mesh's vertices and their previous positions are");
    let got = triangles_on_gpu(c, record.info[0] as u64, sk.order.len() as u64, inst.verts, Some(inst.prev));
    let want = skinned_on_cpu(c, sk, now);
    same_numbers(&format!("{what}: the corners"), &got.p, &want.p, 9);
    same_numbers(&format!("{what}: the normals"), &got.n, &want.n, 9);
    same_numbers(&format!("{what}: the UVs"), &got.uv, &want.uv, 6);
    same_numbers(&format!("{what}: the tangents"), &got.t, &want.t, 12);
    assert!(record.n[1][3].to_bits() & scene::INST_DEFORMS != 0 && inst.bits & scene::INST_DEFORMS != 0, "{what}: the record says its mesh deforms");
    if c.g.hwrt {
        for (j, &(_, section, rank)) in sk.order.iter().enumerate() {
            assert_eq!(c.sc.rows[(inst.rows + section) as usize][1] + rank, record.info[0] + j as u32, "{what}: triangle {j} is its section's first and its index on (section {section}, index {rank})");
        }
    } else {
        assert_eq!(got.rows, want.rows, "{what}: the tree's rows (a triangle's section, -, -, its index in the section)");
    }
    same_numbers(&format!("{what}: the previous frame's positions"), &got.prev, &skinned_on_cpu(c, sk, before).p, 9);
    let asked: Vec<f32> = c.node_tris(name).iter().flatten().flat_map(|p| p.to_array()).collect();
    let placed: Vec<f32> = want.p.chunks_exact(3).flat_map(|p| inst.world.transform_point3(Vec3::new(p[0], p[1], p[2])).to_array()).collect();
    same_numbers(&format!("{what}: the mesh as the CPU has it when asked"), &asked, &placed, 9);
    want.p
}

/// An object's mesh on the GPU, read through its triangles' indices from its instance's first vertex, is the mesh
/// the CPU has (its corners, bit for bit); its record and its mesh agree on where its vertices are.
fn mesh_is_the_cpus(what: &str, c: &LiveCore, id: u32) {
    let i = c.sc.tables.retained.instance(id).expect("an instance").instance as usize;
    let (inst, record) = (&c.sc.instances[i], &c.sc.gpu_instances[i]);
    let m = c.sc.meshes[inst.mesh as usize];
    assert_eq!((record.info[2], inst.verts), (m.vfirst, m.vfirst), "{what}: the record has the mesh's first vertex");
    let got = triangles_on_gpu(c, record.info[0] as u64, m.count as u64, inst.verts, None);
    let want: Vec<f32> = c.sc.pool_tris[m.first as usize..(m.first + m.count) as usize].iter().flatten().flat_map(|p| p.to_array()).collect();
    assert_eq!(got.p, want, "{what}: the mesh's corners through its indices");
}

/// The world-space triangles that move, on the GPU through their indices, are the CPU's (`LoadedScene::dynamic`).
fn moving_triangles_are_the_cpus(what: &str, c: &LiveCore) {
    let (first, rows, _, corners) = &c.sc.dynamic;
    let got = triangles_on_gpu(c, *first as u64, rows.len() as u64 / 3, 0, None);
    assert_eq!(got.p, corners.iter().flatten().copied().collect::<Vec<f32>>(), "{what}: the moving triangles' corners through their indices");
}

/// Mean absolute difference of two pictures, in 8-bit steps.
fn apart(a: &[u8], b: &[u8]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as f64).sum::<f64>() / a.len() as f64
}

/// The mean of a picture's colour bytes (BGRA8, 96 pixels a row) over the pixels x0..x1 of the rows y0..y1.
fn region_mean(p: &[u8], [x0, y0, x1, y1]: [usize; 4]) -> f64 {
    assert_eq!(p.len(), 96 * 54 * 4);
    let sum: f64 = (y0..y1).flat_map(|y| (x0..x1).map(move |x| (y * 96 + x) * 4)).map(|k| p[k] as f64 + p[k + 1] as f64 + p[k + 2] as f64).sum();
    sum / (3 * (x1 - x0) * (y1 - y0)) as f64
}

/// A world of a white wall (2 m, at z = 0), a black sky and a camera before the wall at `z`, with three materials:
/// 1 white, 2 black, 3 as given.
fn wall_world(z: f32, third: serde_json::Value) -> World {
    let json = |v: serde_json::Value| serde_json::from_value::<crate::desc::Material>(v).unwrap();
    let plain = |c: f32| serde_json::json!({ "name": "m", "color": [c, c, c], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true });
    let mut w = World::default();
    w.set_environment(crate::desc::Environment { sky: sky(serde_json::json!({ "type": "color", "color": [0.0, 0.0, 0.0], "intensity": 0, "skyLightIntensity": 0 })), ..w.environment.clone() });
    w.set_material(1, json(plain(0.8)));
    w.set_material(2, json(plain(0.0)));
    w.set_material(3, json(third));
    w.set_geometry(1, Geometry { positions: vec![-1.0, -1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0, -1.0, 1.0, 0.0], normals: [0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] }).unwrap();
    w.set_object(10, "wall".into(), None, Kind::Mesh { geometry: 1, material: 1, own_blas: false, materials: vec![] }, true, false).unwrap();
    w.set_object(4, "camera".into(), None, Kind::Camera(camera(50.0, None, manual(0.0))), true, false).unwrap();
    w.set_transform(4, Vec3::new(0.0, 0.0, z), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    w.set_settings(Settings { camera: Some(4), ..Default::default() });
    w
}

/// Triangles as a geometry of corners of their own, with one normal.
fn soup(t: &[[[f32; 3]; 3]], normal: [f32; 3], groups: Vec<[u32; 3]>) -> Geometry {
    Geometry { positions: t.iter().flatten().flatten().copied().collect(), normals: normal.repeat(3 * t.len()), uvs: vec![0.0; 6 * t.len()], indices: (0..3 * t.len() as u32).collect(), groups, colors: vec![] }
}

/// A wall with a dark triangle and three emitting ones close before it, at right angles to each other (one parallel
/// to the wall, one upright, one level): one mesh of two sections (the emitters its second), or with `apart` a mesh
/// each. (At right angles: light sampling picks between emitters of one material by power, so a ray's weight for
/// one read as another's differs by their cosines alone. Three: whichever entry a wrong index names, one of the
/// first two is not it.)
fn emitters_world(apart: bool) -> World {
    let dark = [[-0.95, -0.95, 0.05], [-0.8, -0.95, 0.05], [-0.8, -0.8, 0.05]];
    let lit = [[[-0.4, -0.1, 0.12], [-0.1, -0.1, 0.12], [-0.25, 0.16, 0.12]], [[0.3, -0.65, 0.02], [0.3, -0.35, 0.02], [0.3, -0.5, 0.3]], [[0.4, 0.6, 0.02], [0.8, 0.6, 0.02], [0.6, 0.6, 0.3]]];
    let mut w = wall_world(2.6, serde_json::json!({ "name": "glow", "color": [0, 0, 0], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [1, 1, 1], "emissiveIntensity": 1.5, "doubleSided": true }));
    if apart {
        for (k, (t, material)) in [(dark, 2), (lit[0], 3), (lit[1], 3), (lit[2], 3)].into_iter().enumerate() {
            w.set_geometry(20 + k as u32, soup(&[t], [0.0, 0.0, 1.0], vec![])).unwrap();
            w.set_object(20 + k as u32, format!("part{k}"), None, Kind::Mesh { geometry: 20 + k as u32, material, own_blas: false, materials: vec![] }, true, false).unwrap();
        }
    } else {
        w.set_geometry(20, soup(&[dark, lit[0], lit[1], lit[2]], [0.0, 0.0, 1.0], vec![[0, 3, 0], [3, 9, 1]])).unwrap();
        w.set_object(20, "lamps".into(), None, Kind::Mesh { geometry: 20, material: 2, own_blas: false, materials: vec![2, 3] }, true, false).unwrap();
    }
    w
}

/// The wall beside `emitters_world`'s first two emitters, in a picture of 96 x 54: x0, y0, x1, y1.
const BESIDE_EMITTERS: [[usize; 4]; 2] = [[34, 19, 50, 35], [49, 33, 61, 44]];

/// Emitters in a section that is not its mesh's first, close before a wall: the wall beside two of them is lit as
/// by the same triangles a mesh each (the same rays: the two pictures are one). A light sample is told its emitter's section, whose row has the material that emits; a surface's own ray
/// that meets one finds it in that section.
#[test]
fn emitters_in_a_meshs_second_section_light_as_meshes_of_their_own() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let (mut one, mut each) = match (open(&emitters_world(false)), open(&emitters_world(true))) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return eprintln!("skipped: {e}"),
    };
    let e = &one.sc.tables.emitter_tris;
    assert!(e.len() == 3 && e.iter().all(|e| e.inst != crate::NO_INST && e.part == 1), "the mesh's three emitters, of its second section");
    let (pa, pb) = (picture(&mut one, 96), picture(&mut each, 96));
    for region in BESIDE_EMITTERS {
        let (a, b) = (region_mean(&pa, region), region_mean(&pb, region));
        assert!(b > 30.0 && (a / b - 1.0).abs() < 0.01, "the wall beside an emitter ({region:?}): {a:.1} of 255 as one mesh's second section, {b:.1} as meshes of their own");
    }
}

/// Offline, a surface's own ray that meets an emitter weighs it against the light sample that could have picked it,
/// by that triangle's entry of the light list: its section's first and its index on (under software traversal's
/// units its tree's leaf has the index). The same wall, lit alike by the emitters as one section and as a mesh each
/// (a mesh of one triangle: index 0).
#[test]
fn a_ray_that_meets_an_emitter_weighs_it_by_that_triangles_entry() {
    let g = match gpu::Gpu::new() {
        Ok(g) => g,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    let lit = |apart: bool| {
        let mut sc = scene::build(&emitters_world(apart), Path::new(".."), &crate::default_presets_dir(), None).unwrap();
        (sc.width, sc.height) = (96, 54);
        // (Under software traversal with units, as a live view has the scene.)
        let prep = crate::prepare_as(g.hwrt, true, &mut sc, &mut vec![]);
        let frame = g.render(&sc, prep.groups, bytemuck::cast_slice(&prep.nodes), &prep.emitters, &prep.gpu_emitter_tris, &prep.emitter_runs, crate::ltree::nodes_of(prep.light_tree.as_ref()), &prep.env, &prep.env_cdf, &prep.energy, prep.params, 64, &mut vec![]).unwrap();
        BESIDE_EMITTERS.map(|[x0, y0, x1, y1]| (y0..y1).flat_map(|y| (x0..x1).map(move |x| y * 96 + x)).map(|k| frame.color[k].iter().sum::<f32>() as f64).sum::<f64>() / (3 * (x1 - x0) * (y1 - y0)) as f64)
    };
    let (one, each) = (lit(false), lit(true));
    for ((a, b), region) in one.into_iter().zip(each).zip(BESIDE_EMITTERS) {
        assert!(b > 0.01 && (a / b - 1.0).abs() < 0.003, "the wall beside an emitter ({region:?}): {a:.4} as one mesh's second section, {b:.4} as meshes of their own");
    }
}

/// The light a surface's own ray brings back from what it meets is that surface's from a view's first frames on,
/// while the cache is cold and such a pixel is left to the kernel's second pass: a wall lit by nothing but a patch
/// under a spot light is lit alike with the patch a mesh of its own, the second section of a mesh whose first is
/// black, and (hardware traversal, which draws particles) a particle that follows a black one. (The record a pixel
/// is left by says which section its ray met; a triangle no instance places is its own row.)
#[test]
fn a_first_frames_bounce_is_off_the_section_the_ray_met() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let quad = |x: f32, z: f32, h: f32| [[[x - h, -0.7, z + h], [x + h, -0.7, z + h], [x + h, -0.7, z - h]], [[x - h, -0.7, z + h], [x + h, -0.7, z - h], [x - h, -0.7, z - h]]];
    let (far, patch) = (quad(-3.0, 3.0, 0.05), quad(0.0, 0.75, 0.5));
    // (`how` 0: the patch a mesh of its own; 1: a mesh's second section; 2: a particle.)
    let scene = |how: u32| {
        let mut w = wall_world(3.0, serde_json::json!({ "name": "patch", "color": [0.8, 0.8, 0.8], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true }));
        let spot = serde_json::from_value(serde_json::json!({ "type": "spot", "color": [1, 1, 1], "castShadow": true, "intensity": 60, "radius": 0, "range": 0, "angle": 0.25, "penumbra": 0.2 })).unwrap();
        w.set_object(21, "spot".into(), None, Kind::Light(spot), true, false).unwrap();
        w.set_transform(21, Vec3::new(0.0, 1.4, 0.75), glam::Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2), Vec3::ONE).unwrap();
        match how {
            0 => {
                for (k, (t, material)) in [(far, 2), (patch, 3)].into_iter().enumerate() {
                    w.set_geometry(30 + k as u32, soup(&t, [0.0, 1.0, 0.0], vec![])).unwrap();
                    w.set_object(30 + k as u32, format!("part{k}"), None, Kind::Mesh { geometry: 30 + k as u32, material, own_blas: false, materials: vec![] }, true, false).unwrap();
                }
            }
            1 => {
                w.set_geometry(30, soup(&[far, patch].concat(), [0.0, 1.0, 0.0], vec![[0, 6, 0], [6, 6, 1]])).unwrap();
                w.set_object(30, "floor".into(), None, Kind::Mesh { geometry: 30, material: 2, own_blas: false, materials: vec![2, 3] }, true, false).unwrap();
            }
            _ => {
                for (id, name, material, at) in [(30, "far", 2, "vec3<f32>(-3.0, -0.7, 3.0), vec3<f32>(0.05, 0.0, 0.0), vec3<f32>(0.0, 0.0, -0.05)"), (31, "patch", 3, "vec3<f32>(0.0, -0.7, 0.75), vec3<f32>(0.5, 0.0, 0.0), vec3<f32>(0.0, 0.0, -0.5)")] {
                    w.set_particle_fn(name, format!("fn pf_{name}(i: PIn, p: array<vec4<f32>, 8>) -> POut {{ return POut({at}, true, vec4<f32>(0.0)); }}"), String::new());
                    w.set_object(id, name.into(), None, Kind::Particles { count: 1, emitter: name.into(), params: vec![], material, emitter_area: 0.0 }, true, false).unwrap();
                }
            }
        }
        w
    };
    // (The wall above the patch: lit by nothing but what the patch sends back.)
    let region = [39, 22, 57, 38];
    let mut lit = vec![];
    for how in 0..3 {
        let mut live = match open(&scene(how)) {
            Ok(c) if how == 2 && !c.g.hwrt => break,
            Ok(c) => c,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        lit.push(region_mean(&picture(&mut live, 2), region));
    }
    assert!(lit[0] > 20.0 && lit[1..].iter().all(|x| (x / lit[0] - 1.0).abs() < 0.1), "the wall above the patch after two frames, of 255: {lit:.1?} (the patch a mesh, a mesh's second section, a particle)");
}

/// A particle of a cutout's material is tested by its own material (its triangle's row has it): one whose opacity is
/// under its material's alpha test is not there, before a wall the sun lights, after a particle of another material.
#[test]
fn a_cutout_particle_is_tested_by_its_own_material() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let scene = |cut: bool| {
        let mut w = wall_world(3.0, serde_json::json!({ "name": "cut", "color": [0, 0, 0], "opacity": 0.3, "alphaTest": 0.5, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true }));
        let sun = serde_json::from_value(serde_json::json!({ "type": "directional", "color": [1.0, 1.0, 1.0], "castShadow": true, "intensity": 1.0, "angularDiameter": 0.5 })).unwrap();
        w.set_object(20, "sun".into(), None, Kind::Light(sun), true, false).unwrap();
        for (id, name, material, at) in [(30, "solid", 1, "vec3<f32>(-0.8, -0.8, 0.3), vec3<f32>(0.05, 0.0, 0.0), vec3<f32>(0.0, 0.05, 0.0)"), (31, "cut", 3, "vec3<f32>(0.0, 0.0, 0.5), vec3<f32>(0.5, 0.0, 0.0), vec3<f32>(0.0, 0.5, 0.0)")] {
            if cut || id == 30 {
                w.set_particle_fn(name, format!("fn pf_{name}(i: PIn, p: array<vec4<f32>, 8>) -> POut {{ return POut({at}, true, vec4<f32>(0.0)); }}"), String::new());
                w.set_object(id, name.into(), None, Kind::Particles { count: 1, emitter: name.into(), params: vec![], material, emitter_area: 0.0 }, true, false).unwrap();
            }
        }
        w
    };
    let (mut with, mut without) = match (open(&scene(true)), open(&scene(false))) {
        (Ok(a), Ok(b)) if a.g.hwrt => (a, b),
        (Ok(_), Ok(_)) => return eprintln!("skipped: no hardware RT"),
        (Err(e), _) | (_, Err(e)) => return eprintln!("skipped: {e}"),
    };
    // (The wall's middle, behind the particle.)
    let region = [40, 19, 56, 35];
    let (a, b) = (region_mean(&picture(&mut with, 16), region), region_mean(&picture(&mut without, 16), region));
    assert!(b > 30.0 && (a / b - 1.0).abs() < 0.05, "the wall behind the particle: {a:.1} of 255 with it, {b:.1} without");
}

/// An emitting particle lights both its sides (a rect light's card alone lights one, and whether an emitter is one
/// its own row's material says): a wall behind it is lit alike with the particle's face or its back to it, in a
/// scene whose first triangles are a light's card.
#[test]
fn an_emitting_particle_lights_both_its_sides_beside_a_lights_card() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let scene = |turned: bool| {
        let mut w = wall_world(3.0, serde_json::json!({ "name": "glow", "color": [0, 0, 0], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [1, 1, 1], "emissiveIntensity": 3, "doubleSided": true }));
        // (A card far behind the camera, turned away.)
        let card = serde_json::from_value(serde_json::json!({ "type": "rect", "color": [1, 1, 1], "castShadow": true, "intensity": 0.05, "width": 0.2, "height": 0.2, "range": 0 })).unwrap();
        w.set_object(21, "card".into(), None, Kind::Light(card), true, false).unwrap();
        w.set_transform(21, Vec3::new(0.0, 0.0, 9.0), glam::Quat::from_rotation_y(std::f32::consts::PI), Vec3::ONE).unwrap();
        let v = if turned { -0.15 } else { 0.15 };
        w.set_particle_fn("glow", format!("fn pf_glow(i: PIn, p: array<vec4<f32>, 8>) -> POut {{ return POut(vec3<f32>(0.0, 0.0, 0.3), vec3<f32>(0.15, 0.0, 0.0), vec3<f32>(0.0, {v}, 0.0), true, vec4<f32>(0.0)); }}"), String::new());
        w.set_object(31, "glow".into(), None, Kind::Particles { count: 1, emitter: "glow".into(), params: vec![], material: 3, emitter_area: 0.09 }, true, false).unwrap();
        w
    };
    let (mut face, mut back) = match (open(&scene(false)), open(&scene(true))) {
        (Ok(a), Ok(b)) if a.g.hwrt => (a, b),
        (Ok(_), Ok(_)) => return eprintln!("skipped: no hardware RT"),
        (Err(e), _) | (_, Err(e)) => return eprintln!("skipped: {e}"),
    };
    let e = &face.sc.tables.emitter_tris;
    assert!(e.first().is_some_and(|e| !e.nominal) && e.iter().any(|e| e.nominal), "the light list: a card's triangles first, then the particle's");
    // (The wall beside the particle, as the camera sees it.)
    let region = [35, 20, 43, 34];
    let (a, b) = (region_mean(&picture(&mut face, 64), region), region_mean(&picture(&mut back, 64), region));
    assert!(b > 20.0 && (a / b - 1.0).abs() < 0.05, "the wall beside the particle: {a:.1} of 255 with its back to the wall, {b:.1} with its face");
}

/// Primary visibility by rasterization (`Raster`): a path starts at the triangle the pass drew at its pixel, which is
/// the one its camera ray hits: the picture is the traced one's. The pass draws world-space triangles and is off in a
/// scene with instances, which every mesh of a world is: a scene of rect lights' cards alone.
#[test]
fn a_rasterized_first_hit_is_the_traced_one() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let mut w = World::default();
    // (Turned to face the camera: a card lights the side of its node's -z.)
    for (id, at, turn) in [(21, Vec3::new(-0.9, 0.2, 0.0), 3.6), (22, Vec3::new(0.8, -0.3, 1.0), 2.5)] {
        let card = serde_json::from_value(serde_json::json!({ "type": "rect", "color": [1, 0.6, 0.3], "castShadow": true, "intensity": 3, "width": 1.2, "height": 0.8, "range": 0 })).unwrap();
        w.set_object(id, format!("card{id}"), None, Kind::Light(card), true, false).unwrap();
        w.set_transform(id, at, glam::Quat::from_rotation_y(turn), Vec3::ONE).unwrap();
    }
    w.set_object(4, "camera".into(), None, Kind::Camera(camera(50.0, None, manual(0.0))), true, false).unwrap();
    w.set_transform(4, Vec3::new(0.0, 0.0, 5.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    w.set_settings(Settings { camera: Some(4), ..Default::default() });
    let (mut traced, mut drawn) = match (open(&w), open(&w)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return eprintln!("skipped: {e}"),
    };
    assert!(!drawn.sc.instanced && drawn.sc.tris.len() == 4 && traced.raster.is_none(), "a scene of four world-space triangles, traced unless asked");
    drawn.raster = Some(super::frame::Raster::new(&drawn.g, drawn.w, drawn.h));
    (drawn.kernel_bg, drawn.tl_kernel_bg) = super::kernel_bind_groups(&drawn.g, drawn.raster.as_ref(), &drawn.targets);
    let (a, b) = (picture(&mut traced, 64), picture(&mut drawn, 64));
    let lit = a.chunks_exact(4).filter(|p| p[0] > 100).count();
    assert!(lit > a.len() / 4 / 50, "the cards are in the picture ({lit} pixels)");
    // (0 steps as measured; 4.9 with each index read one word on.)
    assert!(apart(&a, &b) < 0.5, "the picture is {:.2} steps from the traced one", apart(&a, &b));
}

/// A mesh changed in place to triangles of more vertices (a crease: its two triangles share no corner any more) takes
/// another run of vertices, and the mesh that lies after it keeps its own; changed back, it stays where it is. Each
/// is read back through its indices, and the picture is a build's.
#[test]
fn a_mesh_changed_in_place_to_more_vertices_takes_another_run() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: [0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
    let creased = |s: f32| Geometry {
        positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, -s, 0.0, s, s, 0.0, -s, s, 0.0],
        normals: [[0.0, 0.0, 1.0].repeat(3), [0.6, 0.0, 0.8].repeat(3)].concat(),
        uvs: vec![0.0; 12],
        indices: vec![0, 1, 2, 3, 4, 5],
        groups: vec![],
        colors: vec![],
    };
    let mut w = world();
    w.set_geometry(31, quad(0.3)).unwrap();
    w.set_geometry(32, quad(0.2)).unwrap();
    for (id, geometry, x) in [(40, 31, -0.5), (41, 32, 0.5)] {
        w.set_object(id, format!("o{id}"), None, Kind::Mesh { geometry, material: 1, own_blas: true, materials: vec![] }, true, false).unwrap();
        w.set_transform(id, Vec3::new(x, 0.6, 0.8), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    }
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let run = |c: &LiveCore, id: u32| {
        let m = c.sc.meshes[c.sc.instances[c.sc.tables.retained.instance(id).unwrap().instance as usize].mesh as usize];
        (m.vfirst, m.vcount)
    };
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 32), picture(&mut fresh, 32));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
    };
    let (a, b) = (run(&live, 40), run(&live, 41));
    assert_eq!((a.1, b), (4, (a.0 + 4, 4)), "two meshes of four vertices, one after the other");
    w.set_geometry(31, creased(0.3)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "creased: in place ({:?})", live.fallback);
    let creased_run = run(&live, 40);
    assert!(creased_run.1 == 6 && creased_run.0 >= b.0 + b.1 && run(&live, 41) == b, "another run of six vertices, past the other mesh's ({creased_run:?})");
    mesh_is_the_cpus("the creased mesh", &live, 40);
    mesh_is_the_cpus("the mesh after it", &live, 41);
    against_a_build("creased", &mut live, &w);
    w.set_geometry(31, quad(0.25)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "flat again: in place ({:?})", live.fallback);
    assert_eq!(run(&live, 40), creased_run, "four vertices fit where six were");
    mesh_is_the_cpus("the mesh, flat again", &live, 40);
    mesh_is_the_cpus("the mesh after it, still", &live, 41);
    against_a_build("flat again", &mut live, &w);
}

/// Two movable rect lights' cards (world-space triangles that move, one card after the other; a still card's
/// vertices lie before theirs): each moved alone is written where it lies, the other's vertices as they were; the
/// motion vectors at its pixels are its step back (its previous positions are its own); the picture is a build's.
/// (Hardware traversal: under software traversal a world-space triangle that moves is a rebuild's.)
#[test]
fn a_moved_card_beside_another_is_written_where_it_lies() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let turn = glam::Quat::from_rotation_y(3.3);
    let mut w = world();
    let mut at = [Vec3::new(0.0, -0.8, 1.2), Vec3::new(-0.8, 0.8, 1.2), Vec3::new(0.8, 0.8, 1.2)];
    for (k, id) in [20, 21, 22].into_iter().enumerate() {
        let card = serde_json::from_value(serde_json::json!({ "type": "rect", "color": [1, 0.7, 0.4], "castShadow": true, "intensity": 2, "width": 0.4, "height": 0.3, "range": 0 })).unwrap();
        w.set_object(id + 100, format!("card{id}"), None, Kind::Light(card), true, k > 0).unwrap();
        w.set_transform(id + 100, at[k], turn, Vec3::ONE).unwrap();
    }
    let mut live = match open(&w) {
        Ok(c) if c.g.hwrt => c,
        Ok(_) => return eprintln!("skipped: no hardware RT"),
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    assert!(live.sc.dynamic.1.len() == 12 && live.sc.dynamic_verts.0 > 0, "two cards that move, four triangles, after vertices that do not");
    moving_triangles_are_the_cpus("as loaded", &live);
    picture(&mut live, 2);
    for (k, to) in [(2, Vec3::new(0.5, 0.5, 1.4)), (1, Vec3::new(-0.4, 0.3, 1.0)), (2, Vec3::new(0.9, 0.9, 1.1))] {
        let (what, before, step) = (format!("card {k} moved"), live.sc.dynamic.3.clone(), at[k] - to);
        at[k] = to;
        w.set_transform(120 + k as u32, to, turn, Vec3::ONE).unwrap();
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        let moved = (0..4).filter(|&t| live.sc.dynamic.3[t * 3..t * 3 + 3] != before[t * 3..t * 3 + 3]).collect::<Vec<_>>();
        assert!(moved == [0, 1] || moved == [2, 3], "{what}: its two triangles alone ({moved:?})");
        moving_triangles_are_the_cpus(&what, &live);
        // The frame after: where the card is seen, the way back to where it was.
        live.render(1, |_, _| {});
        let m = &live.targets.kernel.motion;
        let vectors: Vec<Vec3> = read_back::<f32>(&live, &m.buffer, m.offset, m.size).chunks_exact(4).filter(|p| p[3] > 0.5).map(|p| Vec3::new(p[0], p[1], p[2])).filter(|v| v.length() > 1e-4).collect();
        assert!(vectors.len() > 10 && vectors.iter().all(|v| (*v - step).length() < 2e-4), "{what}: {} pixels moved, each by the card's step back {step} (the first: {:?})", vectors.len(), vectors.first());
        let mut fresh = open(&w).unwrap();
        forget(&mut live);
        let (a, b) = (picture(&mut live, 32), picture(&mut fresh, 32));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
    }
}

/// A particle that drifts, beside a system that stands still: its vertices' previous positions are the pass's last
/// (the motion vectors at its pixels are its step back, and nothing else moves), and the pass leaves the vertices'
/// UVs as the build made them.
#[test]
fn a_particle_that_moves_has_its_last_positions_and_keeps_its_uvs() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let mut w = world();
    w.set_particle_fn("drift", "fn pf_drift(i: PIn, p: array<vec4<f32>, 8>) -> POut { return POut(vec3<f32>(-0.6 + 0.6 * i.time, 0.5, 1.0), vec3<f32>(0.3, 0.0, 0.0), vec3<f32>(0.0, 0.3, 0.0), true, vec4<f32>(0.0)); }".into(), String::new());
    // (Another system before it, two quads that stand still: the drifting one's vertices and their previous positions
    // are not the scene's first that move.)
    w.set_particle_fn("still", "fn pf_still(i: PIn, p: array<vec4<f32>, 8>) -> POut { return POut(vec3<f32>(-0.6 + 0.5 * f32(i.index), -0.5, 1.0), vec3<f32>(0.1, 0.0, 0.0), vec3<f32>(0.0, 0.1, 0.0), true, vec4<f32>(0.0)); }".into(), String::new());
    w.set_object(17, "still".into(), None, Kind::Particles { count: 2, emitter: "still".into(), params: vec![], material: 1, emitter_area: 0.0 }, true, false).unwrap();
    w.set_object(18, "drift".into(), None, Kind::Particles { count: 1, emitter: "drift".into(), params: vec![], material: 1, emitter_area: 0.01 }, true, false).unwrap();
    let mut live = match open(&w) {
        Ok(c) if c.g.hwrt => c,
        Ok(_) => return eprintln!("skipped: no hardware RT"),
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    // (The vertices that move: four a particle, the systems' meshes.)
    let vertices = |c: &LiveCore| -> (Vec<[f32; 3]>, Vec<[f32; 2]>) {
        let (first, n) = (c.sc.dynamic_verts.0 as u64, c.sc.dynamic_verts.1 as u64);
        let size = crate::indexed::VERTEX_BYTES as u64;
        let v: Vec<crate::indexed::Vertex> = read_back(c, &c.sc.up.vertices, first * size, n * size);
        (v.iter().map(|v| [v[0][0], v[0][1], v[0][2]]).collect(), v.iter().map(|v| [v[0][3], v[1][3]]).collect())
    };
    let built = vertices(&live);
    assert!(built.1.len() == 12 && built.1.chunks(4).all(|q| q == [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]), "four vertices a particle (its mesh's quad, in the order the pass writes it), with the build's UVs: {:?}", built.1);
    live.fixed_dt = Some(1.0 / 60.0);
    for k in 0..4 {
        live.advance_time(1.0 / 60.0);
        live.render(1, |_, _| {});
        // (A pass's first run has no last frame: the previous positions are the positions, and nothing moves.)
        if k == 0 {
            let m = &live.targets.kernel.motion;
            let moved = read_back::<f32>(&live, &m.buffer, m.offset, m.size).chunks_exact(4).filter(|p| p[3] > 0.5 && Vec3::new(p[0], p[1], p[2]).length() > 1e-4).count();
            assert_eq!(moved, 0, "the first frame: no pixel moves");
        }
    }
    let now = vertices(&live);
    assert!(now.0 != built.0 && now.1 == built.1, "the pass moved the vertices and left their UVs ({:?} -> {:?})", built.1, now.1);
    let (m, step) = (&live.targets.kernel.motion, Vec3::new(-0.6 / 60.0, 0.0, 0.0));
    let vectors: Vec<Vec3> = read_back::<f32>(&live, &m.buffer, m.offset, m.size).chunks_exact(4).filter(|p| p[3] > 0.5).map(|p| Vec3::new(p[0], p[1], p[2])).filter(|v| v.length() > 1e-4).collect();
    assert!(vectors.len() > 20 && vectors.iter().all(|v| (*v - step).length() < 2e-4), "{} pixels moved, each by the particle's step back {step} (the first: {:?})", vectors.len(), vectors.first());
}

/// A stateful GPU particle that drifts, as the one above: its pass keeps its vertices' last positions too, a vertex
/// each, and leaves their UVs.
#[test]
fn a_gpu_particle_that_moves_has_its_last_positions_and_keeps_its_uvs() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let mut w = world();
    let emit = "fn pe_drift(i: EIn, p: array<vec4<f32>, 8>) -> PState { var s: PState; s.pos = i.at; s.life = 1000.0; s.color = i.color; s.size = 0.3; s.vel = vec3<f32>(0.6, 0.0, 0.0); return s; }";
    w.set_particle_program("drift", emit.into(), "fn pu_drift(s: PState, i: UIn, p: array<vec4<f32>, 8>) -> PState { return s; }".into(), String::new());
    // (Another system before it, two quads that stand still: the drifting one's vertices and their previous positions
    // are not the scene's first that move.)
    w.set_particle_fn("still", "fn pf_still(i: PIn, p: array<vec4<f32>, 8>) -> POut { return POut(vec3<f32>(-0.6 + 0.5 * f32(i.index), -0.5, 1.0), vec3<f32>(0.1, 0.0, 0.0), vec3<f32>(0.0, 0.1, 0.0), true, vec4<f32>(0.0)); }".into(), String::new());
    w.set_object(17, "still".into(), None, Kind::Particles { count: 2, emitter: "still".into(), params: vec![], material: 1, emitter_area: 0.0 }, true, false).unwrap();
    w.set_object(18, "drift".into(), None, Kind::GpuParticles { capacity: 1, program: "drift".into(), params: vec![], material: 1, emitter_area: 0.01, rate: 0.0, collide: Default::default() }, true, false).unwrap();
    let mut live = match open(&w) {
        Ok(c) if c.g.hwrt => c,
        Ok(_) => return eprintln!("skipped: no hardware RT"),
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    // (The game spawns one: the next commit takes it.)
    w.spawn_particles(18, &[-0.6, 0.5, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0]).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "a spawn: in place ({:?})", live.fallback);
    let vertices = |c: &LiveCore| -> (Vec<[f32; 3]>, Vec<[f32; 2]>) {
        let (first, n) = (c.sc.dynamic_verts.0 as u64, c.sc.dynamic_verts.1 as u64);
        let size = crate::indexed::VERTEX_BYTES as u64;
        let v: Vec<crate::indexed::Vertex> = read_back(c, &c.sc.up.vertices, first * size, n * size);
        (v.iter().map(|v| [v[0][0], v[0][1], v[0][2]]).collect(), v.iter().map(|v| [v[0][3], v[1][3]]).collect())
    };
    let built = vertices(&live);
    assert!(built.1.len() == 12 && built.1.chunks(4).all(|q| q == [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]]), "four vertices a particle (its mesh's quad, in the order the pass writes it), with the build's UVs: {:?}", built.1);
    live.fixed_dt = Some(1.0 / 60.0);
    for k in 0..6 {
        live.advance_time(1.0 / 60.0);
        live.render(1, |_, _| {});
        // (A pass's first run has no last frame: the previous positions are the positions, and nothing moves.)
        if k == 0 {
            let m = &live.targets.kernel.motion;
            let moved = read_back::<f32>(&live, &m.buffer, m.offset, m.size).chunks_exact(4).filter(|p| p[3] > 0.5 && Vec3::new(p[0], p[1], p[2]).length() > 1e-4).count();
            assert_eq!(moved, 0, "the first frame: no pixel moves");
        }
    }
    let now = vertices(&live);
    assert!(now.0 != built.0 && now.1 == built.1, "the pass moved the vertices and left their UVs ({:?} -> {:?})", built.1, now.1);
    let (m, step) = (&live.targets.kernel.motion, Vec3::new(-0.6 / 60.0, 0.0, 0.0));
    let vectors: Vec<Vec3> = read_back::<f32>(&live, &m.buffer, m.offset, m.size).chunks_exact(4).filter(|p| p[3] > 0.5).map(|p| Vec3::new(p[0], p[1], p[2])).filter(|v| v.length() > 1e-4).collect();
    assert!(vectors.len() > 20 && vectors.iter().all(|v| (*v - step).length() < 2e-4), "{} pixels moved, each by the particle's step back {step} (the first: {:?})", vectors.len(), vectors.first());
}

/// A skinned model's primitive whose file has no normals is shaded by its triangles' own, which the kernel makes where
/// a triangle is hit: offline, a camera before the largest such triangle of a posed model sees that triangle's normal.
/// (Under either traversal: the model's mesh an instance's, or a unit's.)
#[test]
fn a_skinned_primitive_without_normals_is_shaded_by_its_triangles() {
    let g = match gpu::Gpu::new() {
        Ok(g) => g,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    let mut w = world();
    let pose = crate::desc::Animation { clip: "bend".into(), time: 0.4, r#loop: true, nodes: Default::default() };
    w.set_object(31, "m31".into(), None, Kind::Model { uri: "tests/render/data/skin-blend.glb".into(), nodes: None, animation: Some(pose), functions: vec![] }, true, false).unwrap();
    w.set_transform(31, Vec3::new(0.0, 6.0, 2.0), glam::Quat::from_rotation_y(0.3), Vec3::splat(2.5)).unwrap();
    let built = |w: &World| {
        let mut sc = scene::build(w, Path::new(".."), &crate::default_presets_dir(), None).unwrap();
        (sc.width, sc.height) = (96, 54);
        // (Under software traversal with units, as a live view has the scene: without them the model is flattened
        // into world-space triangles with the CPU's normals.)
        let prep = crate::prepare_as(g.hwrt, true, &mut sc, &mut vec![]);
        (sc, prep)
    };
    // The mesh's triangles whose three vertices have no normal, as the instance places them: the largest.
    let (centre, normal, edge) = {
        let (sc, prep) = built(&w);
        let m = sc.meshes.iter().position(|m| m.deforms).expect("a deforming mesh");
        let (im, world, x) = (sc.meshes[m], sc.instances.iter().find(|i| i.mesh as usize == m).unwrap().world, &sc.indexed);
        let first = prep.groups.iter().sum::<usize>() + im.first as usize;
        let bare = x.indices[first..first + im.count as usize].iter().filter(|t| t.iter().all(|&v| x.vertices[(im.vfirst + v) as usize][1][..3] == [0.0; 3]));
        let placed = bare.map(|t| t.map(|v| world.transform_point3(Vec3::from_slice(&x.vertices[(im.vfirst + v) as usize][0][..3]))));
        let p = placed.max_by(|a, b| (a[1] - a[0]).cross(a[2] - a[0]).length().total_cmp(&(b[1] - b[0]).cross(b[2] - b[0]).length())).expect("triangles without normals");
        ((p[0] + p[1] + p[2]) / 3.0, (p[1] - p[0]).cross(p[2] - p[0]).normalize(), (p[1] - p[0]).length().max((p[2] - p[0]).length()))
    };
    w.set_transform(4, centre + normal * edge, glam::Quat::from_rotation_arc(Vec3::NEG_Z, -normal), Vec3::ONE).unwrap();
    let (sc, prep) = built(&w);
    let frame = g.render(&sc, prep.groups, bytemuck::cast_slice(&prep.nodes), &prep.emitters, &prep.gpu_emitter_tris, &prep.emitter_runs, crate::ltree::nodes_of(prep.light_tree.as_ref()), &prep.env, &prep.env_cdf, &prep.energy, prep.params, 1, &mut vec![]).unwrap();
    let seen = Vec3::from(frame.normal[27 * 96 + 48]);
    assert!(seen.dot(normal).abs() > 0.999 && (seen.length() - 1.0).abs() < 1e-3, "the picture's middle has the normal {seen}; the triangle's is {normal}");
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
    // A commit that rebuilds (a rect light with a card: its triangles are a build's to place) and brings another
    // preset with it.
    w.set_object(13, "card".into(), None, Kind::Light(serde_json::from_value(serde_json::json!({ "type": "rect", "color": [1, 1, 1], "castShadow": true, "intensity": 50, "width": 0.2, "height": 0.2, "visible": true })).unwrap()), true, false).unwrap();
    w.set_settings(Settings { preset: "cinematic".into(), overrides: Some(serde_json::json!({ "post": { "vignette": 0.3 } })), ..w.settings.clone() });
    assert!(live.commit(&mut w).unwrap(), "a rect light's card: a rebuild");
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
    let (verts, cache, out) = (live.sc.up.vertices.clone(), live.targets.kernel.cache.val.buffer.clone(), live.targets.out.clone());
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
        assert!(live.sc.up.vertices == verts, "{size:?}: the scene's data is as it was");
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
        // (Scene code hides by scaling to 0: the instance stands at a point where no ray sees it, and comes back.)
        ("the movable mesh scaled to nothing", Box::new(move |w| w.set_transform(13, at(0.2, 0.2, 0.7), glam::Quat::IDENTITY, Vec3::ZERO).unwrap())),
        ("the movable mesh back at its size", Box::new(move |w| w.set_transform(13, at(-0.1, 0.3, 0.8), glam::Quat::IDENTITY, Vec3::ONE).unwrap())),
    ];
    for (what, edit) in &edits {
        edit(&mut w);
        let builds = live.top_builds;
        let rebuilt = live.commit(&mut w).unwrap();
        // (Software traversal places some of these another way, and may rebuild: the picture holds either way.)
        assert!(!(rebuilt && live.g.hwrt), "{what}: in place ({:?})", live.fallback);
        assert_eq!(live.top_builds - builds, u32::from(!rebuilt), "{what}: one build above the triangles a commit, whatever moved");
        assert!(live.sc.instances.iter().all(|i| i.world.is_finite() && i.world.determinant() != 0.0), "{what}: every instance has a size above the triangles");
        let mut fresh = open(&w).unwrap();
        forget(&mut live);
        let (a, b) = (picture(&mut live, 24), picture(&mut fresh, 24));
        // (0.02 to 0.08 steps as measured; 0.9 after a move with nothing built above the triangles.)
        assert!(apart(&a, &b) < 0.2, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
    }
}

/// An instanced object whose copies emit is gone from the picture once it is removed (its copies were flattened once,
/// and nothing took them out).
#[test]
fn an_emitting_instanced_object_removed_is_gone() {
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
    use crate::heap::Stream;
    let read = |b: &wgpu::Buffer, at: u64, bytes: u64| -> Vec<u8> { read_back(c, b, at, bytes) };
    let (tables, up) = (&c.sc.tables, &c.sc.up);
    let size = std::mem::size_of::<crate::GpuEmitterTri>();
    let list = read(&up.heap.buffer, up.heap.at(Table::EmitterTris), (tables.emitter_tris.len() * size) as u64);
    // (The triangles as the kernel reads them: three indices each; the vertices; the rows of those that have one.)
    let vertices: Vec<crate::indexed::Vertex> = read_back(c, &up.vertices, 0, up.vertices.size());
    let index: Vec<u32> = read_back(c, &up.words.buffer, up.words.at(Stream::Indices), up.words.cap(Stream::Indices));
    let tri_rows: Vec<[u32; 4]> = read_back(c, &up.words.buffer, up.words.at(Stream::Triangles), up.words.cap(Stream::Triangles) / 16 * 16);
    // (Software traversal: the trees' leaves, whose ids say a mesh's triangle's section and its index there.)
    let leaves: Vec<crate::indexed::Leaf> = if c.g.hwrt { vec![] } else { read_back(c, &up.bg_parts.leaves, 0, up.bg_parts.leaves.size() / crate::indexed::LEAF_BYTES as u64 * crate::indexed::LEAF_BYTES as u64) };
    let codes = crate::emitter_codes(&tables.emitter_tris, &crate::emitter_order(&tables.emitter_tris, tables.light_tree.as_ref()), tables.light_tree.as_ref());
    // (The CDF as the kernel reads it: an emitter that is not the tree's is found by the entry its code names.)
    let cdf = read(&up.heap.buffer, up.heap.at(Table::Emitters), (tables.emitter_tris.len() * 16) as u64);
    let record_size = std::mem::size_of::<scene::GpuInstance>();
    let records = read(&up.heap.buffer, up.heap.at(Table::Instances), (c.sc.gpu_instances.len() * record_size) as u64);
    let gpu_rows = read(&up.words.buffer, up.words.at(crate::heap::Stream::Rows), up.words.cap(crate::heap::Stream::Rows));
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
        // (A mesh's indices count from its instance's first vertex; a world-space triangle's from the first of all.)
        let base = if e.inst == crate::NO_INST { 0 } else { c.sc.instances[e.inst as usize].verts };
        let p: [glam::Vec3A; 3] = std::array::from_fn(|k| glam::Vec3A::from_slice(&vertices[(base + index[e.tri as usize * 3 + k]) as usize][0][..3]));
        if e.inst == crate::NO_INST {
            // (A triangle no instance places has a row: its material, its object, its flags, its emitter + 1. The
            // list's record names the row: the triangle's own.)
            assert_eq!(r, crate::GpuEmitterTri::of(e, p, code & !crate::EMITTER_MOVES), "{what}: emitter {i} of the list against its triangle {}", e.tri);
            assert_eq!((r.part, tri_rows[e.tri as usize][3]), (e.tri, i as u32 + 1), "{what}: emitter {i}'s record names its triangle's row, and the row the emitter");
        } else {
            // (The record as the GPU has it.)
            let (inst, record) = (&c.sc.instances[e.inst as usize], bytemuck::pod_read_unaligned::<scene::GpuInstance>(&records[e.inst as usize * record_size..][..record_size]));
            let placed = [inst.world.transform_point3(p[0].into()), inst.world.transform_vector3((p[1] - p[0]).into()), inst.world.transform_vector3((p[2] - p[0]).into())];
            assert_eq!(record.info[2], inst.verts, "{what}: instance {}'s record has its mesh's first vertex", e.inst);
            assert_eq!((r.tri, r.inst, r.code, r.part), (e.tri, e.inst, code, e.part), "{what}: emitter {i}'s words (its triangle, its instance, its code, its section)");
            for (got, want) in [r.v0, r.e1, r.e2].into_iter().zip(placed) {
                assert!((glam::Vec3::from(got) - want).length() <= 1e-4 * (1.0 + want.length()), "{what}: emitter {i}'s triangle {got:?}, its instance places it at {want:?}");
            }
            // Its section's row as the GPU has it says the material and where the section's emitters start, the
            // triangle its index among them: its place after the section's first triangle (hardware traversal), or
            // what its tree's leaf says (software traversal: a tree has a mesh's triangles in its own order).
            let at = (record.info[3] + e.part) as usize;
            let row: scene::Row = bytemuck::pod_read_unaligned(&gpu_rows[at * 16..][..16]);
            assert_eq!(row, c.sc.rows[at], "{what}: row {at} on the GPU is the scene's");
            assert_eq!(record.info[3], inst.rows, "{what}: instance {}'s record has its rows", e.inst);
            assert!(inst.first_emitter <= i as u32, "{what}: emitter {i} is of instance {}'s run, from {}", e.inst, inst.first_emitter);
            let rank = if c.g.hwrt {
                e.tri.wrapping_sub(row[1])
            } else {
                let l = &leaves[e.tri as usize];
                assert_eq!(l[0][3].to_bits(), e.part, "{what}: triangle {}'s leaf has its section", e.tri);
                l[1][3].to_bits()
            };
            assert_eq!((row[2].wrapping_add(rank), scene::row_material(&row)), (i as u32, e.mat), "{what}: triangle {} of instance {} finds its emitter, of its row's material", e.tri, e.inst);
        }
        alive += 1;
    }
    assert!(alive > 0, "{what}: the list has emitters");
}

/// A glass mesh added in place is drawn as a build draws it, whether the scene had glass on an instance or not: a
/// mesh that is all glass needs nothing of the kernel (its instances are out of a shadow ray's way by their mask).
/// And a material of an instance's triangles that turns glass rebuilds.
#[test]
fn a_glass_mesh_added_is_drawn_as_a_build_draws_it() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    for has in [true, false] {
        let mut w = world();
        w.set_material(7, serde_json::from_value(serde_json::json!({ "name": "glass", "color": [0.5, 0.5, 0.5], "opacity": 1, "metalness": 0, "roughness": 0, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true, "transmission": 1, "ior": 1.5 })).unwrap());
        let mesh = |geometry, material| Kind::Mesh { geometry, material, own_blas: true, materials: vec![] };
        // (A mesh with its own BLAS: the scene has instances either way.)
        w.set_object(14, "own".into(), None, mesh(3, 1), true, false).unwrap();
        w.set_transform(14, Vec3::new(-0.6, 0.5, 0.7), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        if has {
            w.set_object(18, "glass".into(), None, mesh(3, 7), true, false).unwrap();
            w.set_transform(18, Vec3::new(0.8, -0.7, 0.4), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        }
        let mut live = match open(&w) {
            Ok(c) if c.g.hwrt => c,
            Ok(_) => return eprintln!("skipped: no hardware RT"),
            Err(e) => return eprintln!("skipped: {e}"),
        };
        w.take_changes();
        assert_eq!(live.sc.up.kernel.features & gpu::F_IGLASS, 0, "no mesh has glass among other triangles");
        let before = picture(&mut live, 24);
        // A pane before the wall, in the sun's and the lamp's light.
        w.set_object(19, "pane".into(), None, mesh(2, 7), true, false).unwrap();
        w.set_transform(19, Vec3::new(0.1, 0.0, 0.9), glam::Quat::from_rotation_y(0.4), Vec3::splat(0.5)).unwrap();
        assert!(!live.commit(&mut w).unwrap(), "glass on an instance already: {has}: in place ({:?})", live.fallback);
        let pane = live.sc.instances.iter().find(|i| i.name == "pane").expect("the pane is an instance");
        assert!(!pane.cast_shadow, "an instance of a mesh that is all glass casts no shadow");
        let mut fresh = open(&w).unwrap();
        forget(&mut live);
        let (a, b) = (picture(&mut live, 64), picture(&mut fresh, 64));
        assert!(apart(&before, &b) > 0.5, "the pane is in the picture ({:.2} steps)", apart(&before, &b));
        assert!(apart(&a, &b) < 0.5, "glass on an instance already: {has}: the picture is {:.2} steps from a build's", apart(&a, &b));
        // A material of instances' triangles that turns glass (the mesh of its own, and the wall), and is plain
        // again: their rows and their masks say so, in place.
        w.set_material(1, serde_json::from_value(serde_json::json!({ "name": "m", "color": [0.8, 0.4, 0.8], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [1, 1, 1], "emissiveIntensity": 0, "doubleSided": true, "transmission": 1, "ior": 1.5 })).unwrap());
        assert!(!live.commit(&mut w).unwrap(), "a material of instances' triangles turned glass: in place ({:?})", live.fallback);
        let wall = live.sc.instances.iter().find(|i| i.name == "wall").expect("the wall is an instance");
        assert!(!wall.cast_shadow && wall.bits & scene::INST_GLASS != 0, "the wall is glass: it casts no shadow");
        let mut fresh = open(&w).unwrap();
        forget(&mut live);
        let (a, b) = (picture(&mut live, 64), picture(&mut fresh, 64));
        assert!(apart(&a, &b) < 0.5, "a material turned glass: the picture is {:.2} steps from a build's", apart(&a, &b));
        w.set_material(1, serde_json::from_value(serde_json::json!({ "name": "m", "color": [0.8, 0.4, 0.8], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [1, 1, 1], "emissiveIntensity": 0, "doubleSided": true })).unwrap());
        assert!(!live.commit(&mut w).unwrap(), "the material plain again: in place ({:?})", live.fallback);
        let wall = live.sc.instances.iter().find(|i| i.name == "wall").expect("the wall is an instance");
        assert!(wall.cast_shadow && wall.bits & scene::INST_GLASS == 0, "the wall casts its shadow again");
        let mut fresh = open(&w).unwrap();
        forget(&mut live);
        let (c, b) = (picture(&mut live, 64), picture(&mut fresh, 64));
        assert!(apart(&c, &b) < 0.5, "the material plain again: the picture is {:.2} steps from a build's", apart(&c, &b));
        assert!(apart(&a, &c) > 0.5, "glass or not is in the picture ({:.2} steps)", apart(&a, &c));
    }
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
        let mut run = 0;
        for (what, edit) in &edits {
            let what = format!("an emitting instanced object {what} in place, {} a light tree", if tree { "with" } else { "without" });
            edit(&mut w);
            assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
            // Its copies' rows: a run of the room's, the room's again once it is removed.
            match live.sc.tables.retained.instanced(15) {
                Some(place) => {
                    run = live.sc.instances[place.first as usize].rows;
                    assert!((0..3).all(|i| live.sc.instances[place.first as usize + i].rows == run + i as u32) && !rows_free(&live, run, 3), "{what}: a row a copy, in a run");
                }
                None => assert!(rows_free(&live, run, 3), "{what}: its rows are the room's again"),
            }
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

/// Power edits of emitters (a game flashing its glowing bodies every frame) write only the light list's codes that
/// changed (`LoadedScene::codes_sent`: each code is a write of its own, and rewriting a list of 133 k moving emitters
/// cost a frame 470 ms). After several, an emitter moved between them, and two materials of still meshes that start
/// emitting one after the other (emitters added in place, their records made with code 0), every emitter on the GPU
/// has its code: with emitters that move (their codes stay), and in a scene whose emitters all stand still, where the
/// light tree has a slot for the added ones and the second material's regrows it (the first one's codes change).
#[test]
fn power_edits_keep_the_light_lists_codes() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let late = |emissive: f32| serde_json::from_value(serde_json::json!({ "name": "late", "color": [0.2, 0.2, 0.2], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [1.0, 0.6, 0.3], "emissiveIntensity": emissive, "doubleSided": true })).unwrap();
    for (tree, moving) in [(true, true), (false, true), (true, false)] {
        let what = |s: &str| format!("{s}, {}, {} a light tree", if moving { "emitters that move" } else { "still emitters" }, if tree { "with" } else { "without" });
        let mut w = world();
        w.set_material(6, late(0.0));
        w.set_material(7, late(0.0));
        // Two that emit (moving or not), and two still meshes of the materials that emit later, of their own BLASes
        // (the scene edits meshes: a tree without moving emitters has a slot for emitters added in place).
        for (id, x, material, movable) in [(13u32, 0.35f32, 5u32, moving), (14, -0.35, 5, moving), (15, 0.0, 6, false), (16, -0.7, 7, false)] {
            w.set_object(id, format!("mesh{id}"), None, Kind::Mesh { geometry: 3, material, own_blas: !movable, materials: vec![] }, true, movable).unwrap();
            w.set_transform(id, Vec3::new(x, -0.45, 0.6), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        }
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
        assert_eq!(live.sc.tables.light_tree.as_ref().map(|t| t.dynamic.is_some()), tree.then_some(!moving), "{}: the tree's slot", what("as loaded"));
        list_is_its_triangles(&what("as loaded"), &live);
        for (k, power) in [3.0f32, 0.5, 2.0].into_iter().enumerate() {
            if k > 0 {
                w.set_material(5 + k as u32, late(30.0));
            }
            w.set_power(13, power).unwrap();
            if moving {
                w.set_transform(14, Vec3::new(-0.35 + 0.1 * k as f32, -0.45, 0.6), glam::Quat::from_rotation_z(0.3 * k as f32), Vec3::ONE).unwrap();
            }
            live.commit(&mut w).unwrap();
            live.land_powers_now();
            list_is_its_triangles(&what(&format!("power {power}{}", if k > 0 { " and a material that starts emitting" } else { "" })), &live);
        }
        let t = &live.sc.tables;
        let emitting = t.emitter_tris.iter().filter(|e| crate::emitter_power(e, &t.materials, &t.objects, t.stride) > 0.0).count();
        assert!(emitting >= 8, "{}: the late materials' emitters were added ({emitting} emit)", what("at the end"));
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
    // An emitter that moves: a movable mesh is an instance of its mesh, its emitters its instance's.
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
        assert!(live.sc.tables.emitter_tris.iter().any(|e| e.inst != crate::NO_INST && e.moves), "the mover's emitters are its instance's");
        assert!(live.sc.up.fills.moved.is_none(), "no triangle is left for the CPU to move");
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
    let slide = |time| Some(crate::desc::Animation { clip: "slide".into(), time, r#loop: true, nodes: Default::default() });
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
        assert_eq!(of_instances, 8, "the lamp's two, and two a placement of the model, each its instance's");
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
        assert!(live.sc.tables.emitter_tris.iter().filter(|e| e.inst != crate::NO_INST && !e.moves).count() == 4, "the lamp's and the still model's do not");
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
            let size = std::mem::size_of::<crate::GpuEmitterTri>() as u64;
            let to = c.g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let mut e = c.g.device.create_command_encoder(&Default::default());
            e.copy_buffer_to_buffer(&c.sc.up.heap.buffer, at, &to, 0, size);
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

/// A material that starts emitting in place lights the instances of the meshes that have it as a build lights them: a
/// mesh of that material alone that two objects share, one of them movable (and moved afterwards: its new emitters
/// follow it), a mesh of two materials of which the other emitted already (its instance's run of the light list
/// grows), and a mesh added in place of a material nothing had. With and without a light tree. After each: no rebuild,
/// the light list is its triangles, the weights and the picture are a build's. A material an instanced object has too
/// rebuilds, one added in place as well.
#[test]
fn a_material_that_starts_emitting_lights_its_meshes_instances_as_a_build_does() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let late = |emissive: f32| serde_json::from_value(serde_json::json!({ "name": "late", "color": [0.2, 0.2, 0.2], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [1.0, 0.6, 0.3], "emissiveIntensity": emissive, "doubleSided": true })).unwrap();
    for tree in [false, true] {
        let mut w = world();
        // (The sun away: the meshes will light the scene.)
        w.set_visible(20, false).unwrap();
        w.set_material(6, late(0.0));
        // Sixteen quads in a row in one geometry, four to a group, the groups by turns of the lamp's material, which
        // emits, and of the late one; the geometry has the quads out of the row's order (so that a mesh tree orders a
        // section's triangles its own way under software traversal).
        let (mut positions, mut indices) = (vec![], vec![]);
        for q in 0..16u32 {
            let x = -0.48 + 0.06 * ((q * 7) % 16) as f32;
            positions.extend([x, -0.15, 0.0, x + 0.06, -0.15, 0.0, x + 0.06, 0.15, 0.0, x, 0.15, 0.0]);
            indices.extend([0, 1, 2, 0, 2, 3].map(|i| 4 * q + i));
        }
        let groups = (0..4u32).map(|k| [24 * k, 24, k % 2]).collect();
        // (And the same quads without groups, for the mesh added in place below: one section of 32 triangles.)
        w.set_geometry(8, Geometry { positions: positions.clone(), normals: [0.0, 0.0, 1.0].repeat(64), uvs: vec![0.0; 128], indices: indices.clone(), groups: vec![], colors: vec![] }).unwrap();
        w.set_geometry(7, Geometry { positions, normals: [0.0, 0.0, 1.0].repeat(64), uvs: vec![0.0; 128], indices, groups, colors: vec![] }).unwrap();
        let mesh = |geometry, material, materials| Kind::Mesh { geometry, material, own_blas: false, materials };
        w.set_object(13, "one".into(), None, mesh(3, 6, vec![]), true, false).unwrap();
        w.set_transform(13, Vec3::new(-0.6, -0.5, 0.8), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_object(33, "third".into(), None, mesh(3, 6, vec![]), true, false).unwrap();
        w.set_transform(33, Vec3::new(-0.1, -0.1, 1.2), glam::Quat::from_rotation_y(-0.4), Vec3::ONE).unwrap();
        w.set_object(14, "other".into(), None, mesh(3, 6, vec![]), true, true).unwrap();
        w.set_transform(14, Vec3::new(0.5, -0.5, 0.9), glam::Quat::from_rotation_y(0.5), Vec3::ONE).unwrap();
        w.set_object(15, "two".into(), None, mesh(7, 5, vec![5, 6]), true, false).unwrap();
        w.set_transform(15, Vec3::new(0.0, 0.6, 0.7), glam::Quat::from_rotation_x(-0.3), Vec3::ONE).unwrap();
        // A material a mesh has and an instanced object too, and one nothing has yet; a mesh the scene says it edits
        // (room for one added in place).
        w.set_material(7, late(0.0));
        w.set_material(8, late(0.0));
        w.set_material(9, late(0.0));
        w.set_object(32, "nine".into(), None, mesh(3, 9, vec![]), true, false).unwrap();
        w.set_transform(32, Vec3::new(-0.9, -0.1, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_object(26, "both".into(), None, mesh(3, 7, vec![]), true, false).unwrap();
        w.set_transform(26, Vec3::new(-0.9, 0.6, 0.6), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_object(27, "copies".into(), None, Kind::Instanced { geometry: 3, procedural: false, materials: vec![7], capacity: 1 }, true, false).unwrap();
        w.set_instances(27, 1, Some(glam::Mat4::from_translation(Vec3::new(0.9, 0.6, 0.6)).to_cols_array().to_vec()), None, None).unwrap();
        w.set_object(28, "own".into(), None, Kind::Mesh { geometry: 3, material: 1, own_blas: true, materials: vec![] }, true, false).unwrap();
        w.set_transform(28, Vec3::new(0.9, -0.1, 0.4), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        if tree {
            strip(&mut w);
        }
        let mut live = match open(&w) {
            Ok(c) => c,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        w.take_changes();
        picture(&mut live, 4);
        let with = if tree { "with" } else { "without" };
        let mesh_of = |c: &LiveCore, id: u32| c.sc.instances[c.sc.tables.retained.instance(id).expect("a mesh is an instance").instance as usize].mesh;
        assert_eq!(mesh_of(&live, 13), mesh_of(&live, 14), "the two objects share their mesh");
        let two = mesh_of(&live, 15) as usize;
        let emitters_of = |c: &LiveCore, id: u32| {
            let i = &c.sc.instances[c.sc.tables.retained.instance(id).unwrap().instance as usize];
            let m = c.sc.meshes[i.mesh as usize];
            scene::rows_emitters(m.sections(&c.sc.sections), &c.sc.rows[i.rows as usize..][..m.sect_count as usize], &c.sc.tables.materials)
        };
        assert_eq!((live.sc.meshes[two].count, live.sc.meshes[two].sect_count, emitters_of(&live, 15)), (32, 4, 16), "the mesh of two materials, a section a group: one emits");
        // (A third object of the shared mesh, hidden when the material starts: it has its emitters when it is shown.)
        w.set_visible(33, false).unwrap();
        assert!(!live.commit(&mut w).unwrap(), "a mesh hidden, {with} a light tree: in place ({:?})", live.fallback);
        w.set_material(6, late(30.0));
        let what = format!("a material started emitting in place, {with} a light tree");
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        assert_eq!((emitters_of(&live, 15), emitters_of(&live, 13), emitters_of(&live, 14)), (32, 2, 2), "{what}: the instances' emitters");
        list_is_its_triangles(&what, &live);
        let moving = |c: &LiveCore, id: u32| {
            let r = c.sc.tables.retained.instance(id).unwrap().instance;
            c.sc.tables.emitter_tris.iter().filter(|e| e.inst == r && e.area > 0.0).map(|e| e.moves).collect::<Vec<_>>()
        };
        assert_eq!((moving(&live, 13), moving(&live, 14)), (vec![false; 2], vec![true; 2]), "{what}: the movable one's emitters move");
        lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
        w.set_visible(33, true).unwrap();
        let what = format!("a mesh hidden when its material started emitting, shown, {with} a light tree");
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        list_is_its_triangles(&what, &live);
        lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
        // (A mesh of many triangles added in place: under software traversal its tree orders them its own way.)
        w.set_object(29, "added".into(), None, Kind::Mesh { geometry: 8, material: 8, own_blas: true, materials: vec![] }, true, false).unwrap();
        w.set_transform(29, Vec3::new(-0.2, 0.1, 1.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        assert!(!live.commit(&mut w).unwrap(), "a mesh added, {with} a light tree: in place ({:?})", live.fallback);
        w.set_material(8, late(20.0));
        let what = format!("the material of a mesh added in place started emitting, {with} a light tree");
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        list_is_its_triangles(&what, &live);
        lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
        // A material an instanced object added in place has too: its copy's emitters, as the meshes'.
        if live.g.hwrt {
            w.set_object(31, "more".into(), None, Kind::Instanced { geometry: 3, procedural: false, materials: vec![9], capacity: 1 }, true, false).unwrap();
            w.set_instances(31, 1, Some(glam::Mat4::from_translation(Vec3::new(0.4, 0.2, 1.1)).to_cols_array().to_vec()), None, None).unwrap();
            assert!(!live.commit(&mut w).unwrap(), "an instanced object added, {with} a light tree: in place ({:?})", live.fallback);
            w.set_material(9, late(30.0));
            let what = format!("a material an instanced object added in place has too started emitting, {with} a light tree");
            assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
            list_is_its_triangles(&what, &live);
            lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
        }
        // One an instanced object of the build has too: its copies', a copy that is not shown at no weight.
        w.set_material(7, late(30.0));
        let what = format!("a material an instanced object has too started emitting, {with} a light tree");
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        list_is_its_triangles(&what, &live);
        lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
        w.set_transform(14, Vec3::new(0.2, -0.3, 1.2), glam::Quat::from_rotation_z(0.6), Vec3::ONE).unwrap();
        let what = format!("the movable one of them moved, {with} a light tree");
        assert!(!(live.commit(&mut w).unwrap() && live.g.hwrt), "{what}: in place ({:?})", live.fallback);
        list_is_its_triangles(&what, &live);
        // (A moving emitter's centre is not kept on the CPU: the picture alone.)
        forget(&mut live);
        let (a, b) = (picture(&mut live, 64), picture(&mut open(&w).unwrap(), 64));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
    }
}

/// A mesh hidden or shown by its `visible` is drawn and lit as a build draws it, in place: a still mesh, one that
/// emits, one hidden when the scene was built, a group of two, a movable one moved while it was hidden. With and
/// without a light tree. After each: no rebuild, the light list is its triangles, the weights and the picture are a
/// build's. So for what an edit places unseen: a mesh whose geometry changed while it was hidden, one added hidden,
/// one added without size.
#[test]
fn meshes_hidden_and_shown_in_place_are_drawn_as_a_build_draws_them() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: [0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
    for tree in [false, true] {
        let mut w = world();
        let mesh = |geometry, material, own_blas| Kind::Mesh { geometry, material, own_blas, materials: vec![] };
        let place = |w: &mut World, id: u32, name: &str, parent: Option<u32>, kind: Kind, movable: bool, at: [f32; 3]| {
            w.set_object(id, name.into(), parent, kind, true, movable).unwrap();
            w.set_transform(id, Vec3::from(at), glam::Quat::from_rotation_y(0.3), Vec3::ONE).unwrap();
        };
        // A lamp hidden at the build, a group of two panels, a movable panel, a mesh the scene edits (a geometry of
        // its own).
        place(&mut w, 13, "late", None, mesh(3, 5, false), false, [-0.6, 0.6, 1.2]);
        // (Stretched: the area it has when it is shown is not its mesh's own.)
        w.set_transform(13, Vec3::new(-0.6, 0.6, 1.2), glam::Quat::from_rotation_y(0.3), Vec3::new(1.6, 0.6, 1.0)).unwrap();
        w.set_visible(13, false).unwrap();
        w.set_object(14, "group".into(), None, Kind::Group, true, false).unwrap();
        place(&mut w, 15, "a", Some(14), mesh(3, 1, false), false, [0.5, -0.5, 0.6]);
        place(&mut w, 16, "b", Some(14), mesh(3, 1, false), false, [0.0, -0.6, 0.9]);
        place(&mut w, 21, "mover", None, mesh(3, 1, false), true, [0.7, 0.5, 0.5]);
        w.set_geometry(8, quad(0.2)).unwrap();
        place(&mut w, 22, "own", None, mesh(8, 1, true), false, [-0.7, -0.5, 0.9]);
        if tree {
            strip(&mut w);
        }
        let mut live = match open(&w) {
            Ok(c) => c,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        w.take_changes();
        picture(&mut live, 4);
        let with = if tree { "with" } else { "without" };
        let hidden = |c: &LiveCore, id: u32| {
            let i = &c.sc.instances[c.sc.tables.retained.instance(id).expect("a mesh is an instance").instance as usize];
            (i.hidden, i.masked)
        };
        assert_eq!((hidden(&live, 13), hidden(&live, 12)), ((true, true), (false, false)), "a mesh hidden at the build is an instance no ray sees");
        let emitters = |c: &LiveCore, id: u32| {
            let (t, r) = (&c.sc.tables, c.sc.tables.retained.instance(id).unwrap().instance);
            t.emitter_tris.iter().filter(|e| e.inst == r && e.area > 0.0).map(|e| (crate::emitter_power(e, &t.materials, &t.objects, t.stride) > 0.0, e.moves)).collect::<Vec<_>>()
        };
        assert_eq!((emitters(&live, 13), emitters(&live, 12)), (vec![(false, true); 2], vec![(true, false); 2]), "its emitters weigh nothing, and are the moving subtree's when it is shown");
        type Edit = (&'static str, Box<dyn Fn(&mut World)>);
        let show = |id: u32, on: bool| -> Box<dyn Fn(&mut World)> { Box::new(move |w| w.set_visible(id, on).unwrap()) };
        let edits: Vec<Edit> = vec![
            ("a panel hidden", show(11, false)),
            ("the panel shown", show(11, true)),
            ("the lamp hidden", show(12, false)),
            ("the lamp shown", show(12, true)),
            ("a lamp hidden at the build shown", show(13, true)),
            ("that lamp hidden again", show(13, false)),
            ("a group of two hidden", show(14, false)),
            ("the group shown", show(14, true)),
            ("a movable mesh hidden", show(21, false)),
            ("the movable mesh moved while hidden", Box::new(|w| w.set_transform(21, Vec3::new(-0.1, 0.2, 1.3), glam::Quat::from_rotation_z(0.4), Vec3::ONE).unwrap())),
            ("the movable mesh shown where it moved to", show(21, true)),
        ];
        for (what, edit) in &edits {
            let what = format!("{what}, {with} a light tree");
            edit(&mut w);
            let rebuilt = live.commit(&mut w).unwrap();
            // (Software traversal may rebuild for a move: the picture holds either way.)
            assert!(!rebuilt || (!live.g.hwrt && what.contains("moved")), "{what}: in place ({:?})", live.fallback);
            if live.sc.tables.emitter_tris.iter().any(|e| e.area > 0.0) {
                list_is_its_triangles(&what, &live);
            }
            // (Where each instance stands, hidden or not: its record's too.)
            for id in [11, 12, 13, 15, 16, 21] {
                let r = live.sc.tables.retained.instance(id).unwrap().instance as usize;
                let (at, rec) = (w.world_matrix(id), live.sc.gpu_instances[r].m);
                assert!(live.sc.instances[r].world.abs_diff_eq(at, 1e-5), "{what}: object {id}'s instance stands where the object does");
                let rows = at.transpose().to_cols_array();
                assert!(rec.iter().flatten().zip(&rows[..12]).all(|(a, b)| (a - b).abs() <= 1e-5), "{what}: object {id}'s record stands where the object does");
            }
            lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
        }
        assert_eq!((hidden(&live, 13), hidden(&live, 11), hidden(&live, 21)), ((true, true), (false, false), (false, false)), "{with} a light tree: as the world has them");
        // An edit places a mesh that is hidden, or has no size, as the build does: an instance no ray sees. Its geometry
        // changed while it was hidden (its old mesh is left without a user), a mesh added hidden, one added without size:
        // each comes back in place. A hidden mesh removed takes its mesh's user with it.
        if live.g.hwrt {
            let mesh_of = |c: &LiveCore, id: u32| c.sc.instances[c.sc.tables.retained.instance(id).unwrap().instance as usize].mesh;
            w.set_visible(22, false).unwrap();
            assert!(!live.commit(&mut w).unwrap(), "a mesh the scene edits hidden, {with} a light tree: in place ({:?})", live.fallback);
            let its = mesh_of(&live, 22);
            w.set_geometry(8, quad(0.35)).unwrap();
            place(&mut w, 23, "added hidden", None, mesh(3, 5, false), false, [0.3, 0.6, 1.0]);
            w.set_visible(23, false).unwrap();
            w.set_object(24, "added without size".into(), None, mesh(3, 1, false), true, false).unwrap();
            w.set_transform(24, Vec3::new(0.6, -0.1, 0.8), glam::Quat::IDENTITY, Vec3::ZERO).unwrap();
            let what = format!("a hidden mesh's geometry changed, a mesh added hidden, one added without size, {with} a light tree");
            assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
            assert_eq!((hidden(&live, 22), hidden(&live, 23), hidden(&live, 24)), ((true, true), (true, true), (true, true)), "{what}: instances no ray sees");
            // (As many triangles as before: the mesh is written again where it was, and has its one user.)
            assert_eq!((mesh_of(&live, 22), live.sc.edit.users.get(&its).copied().unwrap_or(0)), (its, 1), "{what}: its mesh has its one user");
            lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
            let back: Vec<Edit> = vec![
                ("a mesh shown after its geometry changed while hidden", show(22, true)),
                ("a mesh added hidden shown", show(23, true)),
                ("a mesh added without size given one", Box::new(|w| w.set_transform(24, Vec3::new(0.6, -0.1, 0.8), glam::Quat::from_rotation_y(-0.3), Vec3::ONE).unwrap())),
                ("the added lamp hidden again", show(23, false)),
            ];
            for (what, edit) in &back {
                let what = format!("{what}, {with} a light tree");
                edit(&mut w);
                assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
                if live.sc.tables.emitter_tris.iter().any(|e| e.area > 0.0) {
                    list_is_its_triangles(&what, &live);
                }
                lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
            }
            let its = mesh_of(&live, 22);
            w.set_visible(22, false).unwrap();
            assert!(!live.commit(&mut w).unwrap(), "hidden once more, {with} a light tree: in place ({:?})", live.fallback);
            w.remove(22);
            let what = format!("a hidden mesh removed, {with} a light tree");
            assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
            assert_eq!(live.sc.edit.users.get(&its).copied().unwrap_or(0), 0, "{what}: its mesh has no user left");
            lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
        }
    }
}

/// A still mesh's first move is its instance's, in place, and it moves in place from then on: one that emits (its
/// emitters are moving ones from now on), a plain one, a group of two, and a lamp built without size that is given
/// one. With and without a light tree. After each: no rebuild, the light list is its triangles, the picture is a
/// build's.
#[test]
fn a_still_mesh_moves_in_place_from_its_first_move() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    for tree in [false, true] {
        let mut w = world();
        let mesh = |geometry, material| Kind::Mesh { geometry, material, own_blas: false, materials: vec![] };
        w.set_object(14, "group".into(), None, Kind::Group, true, false).unwrap();
        for (id, at) in [(15, Vec3::new(0.5, -0.5, 0.6)), (16, Vec3::new(0.0, -0.6, 0.9))] {
            w.set_object(id, format!("o{id}"), Some(14), mesh(3, 1), true, false).unwrap();
            w.set_transform(id, at, glam::Quat::from_rotation_y(0.3), Vec3::ONE).unwrap();
        }
        // (A lamp built without size: an instance no ray sees, until it is given one.)
        w.set_object(18, "seed".into(), None, mesh(3, 5), true, false).unwrap();
        w.set_transform(18, Vec3::new(-0.7, -0.5, 1.1), glam::Quat::IDENTITY, Vec3::ZERO).unwrap();
        if tree {
            strip(&mut w);
        }
        let mut live = match open(&w) {
            Ok(c) => c,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        w.take_changes();
        picture(&mut live, 4);
        let with = if tree { "with" } else { "without" };
        let moves = |c: &LiveCore, id: u32| {
            let i = c.sc.tables.retained.instance(id).expect("a mesh is an instance");
            (i.moves, c.sc.tables.emitter_tris.iter().filter(|e| e.inst == i.instance && e.area > 0.0).map(|e| e.moves).collect::<Vec<_>>())
        };
        assert_eq!((moves(&live, 12), moves(&live, 11).0, moves(&live, 15).0), ((false, vec![false; 2]), false, false), "still meshes, as built");
        let seed = &live.sc.instances[live.sc.tables.retained.instance(18).expect("a mesh without size is an instance").instance as usize];
        assert!(seed.hidden && seed.masked && seed.world.determinant() != 0.0, "built without size: unseen, at a point");
        type Edit = (&'static str, Box<dyn Fn(&mut World)>);
        let to = |id: u32, at: [f32; 3], turn: f32| -> Box<dyn Fn(&mut World)> { Box::new(move |w| w.set_transform(id, Vec3::from(at), glam::Quat::from_rotation_z(turn), Vec3::ONE).unwrap()) };
        let edits: Vec<Edit> = vec![
            ("the still lamp moved", to(12, [0.3, 0.5, 1.3], 0.4)),
            ("the lamp moved again", to(12, [-0.2, 0.4, 1.4], -0.2)),
            ("a still panel moved", to(11, [-0.6, 0.2, 0.7], 0.3)),
            ("a still group of two moved", to(14, [0.1, 0.15, 0.2], 0.2)),
            ("the group moved again", to(14, [-0.1, 0.1, 0.1], 0.0)),
            ("a lamp built without size given its size", to(18, [-0.7, -0.5, 1.1], 0.3)),
        ];
        for (what, edit) in &edits {
            let what = format!("{what}, {with} a light tree");
            edit(&mut w);
            let builds = live.top_builds;
            let rebuilt = live.commit(&mut w).unwrap();
            // (Software traversal without units has no instance to move, and rebuilds: the picture holds either way.)
            assert!(!(rebuilt && live.g.hwrt), "{what}: in place ({:?})", live.fallback);
            assert_eq!(live.top_builds - builds, u32::from(!rebuilt), "{what}: one build above the triangles");
            list_is_its_triangles(&what, &live);
            forget(&mut live);
            let (a, b) = (picture(&mut live, 64), picture(&mut open(&w).unwrap(), 64));
            assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        }
        if live.g.hwrt {
            assert_eq!((moves(&live, 12), moves(&live, 11).0, moves(&live, 15).0, moves(&live, 16).0), ((true, vec![true; 2]), true, true, true), "{with} a light tree: they move from now on");
        }
    }
}

/// Meshes the scene did not say it edits change in place as those it did: a geometry two of them use replaced (one
/// emits), one re-created with another geometry, a movable one added and moved, a still one added and moved, the
/// movable one hidden, moved and shown, one removed, a group of two removed.
/// With and without a light tree. After each: no rebuild, the light list is its triangles, the weights and the picture
/// are a build's. (Hardware traversal: its room for edits grows from nothing.)
#[test]
fn meshes_change_in_place_whether_or_not_the_scene_said_so() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: [0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
    for tree in [false, true] {
        let mut w = world();
        let mesh = |geometry, material| Kind::Mesh { geometry, material, own_blas: false, materials: vec![] };
        w.set_object(14, "group".into(), None, Kind::Group, true, false).unwrap();
        for (id, at) in [(15, Vec3::new(0.5, -0.5, 0.6)), (16, Vec3::new(0.0, -0.6, 0.9))] {
            w.set_object(id, format!("o{id}"), Some(14), mesh(3, 1), true, false).unwrap();
            w.set_transform(id, at, glam::Quat::from_rotation_y(0.3), Vec3::ONE).unwrap();
        }
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
        assert!(!live.sc.tables.retained.has_own_instances(), "the scene said of no mesh that it edits it");
        type Edit = (&'static str, Box<dyn Fn(&mut World)>);
        let edits: Vec<Edit> = vec![
            ("a geometry two meshes use replaced", Box::new(move |w| w.set_geometry(3, quad(0.4)).unwrap())),
            ("a mesh re-created with another geometry", Box::new(move |w| w.set_object(10, "wall".into(), None, mesh(3, 1), true, false).unwrap())),
            (
                "a movable mesh added",
                Box::new(move |w| {
                    w.set_object(13, "mover".into(), None, mesh(3, 1), true, true).unwrap();
                    w.set_transform(13, Vec3::new(-0.7, 0.6, 0.8), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
                }),
            ),
            ("the added mesh moved", Box::new(|w| w.set_transform(13, Vec3::new(-0.3, 0.7, 1.0), glam::Quat::from_rotation_z(0.5), Vec3::ONE).unwrap())),
            // (What an edit placed moves as what the build placed does: a still one from its first move, a hidden one
            // while it is hidden.)
            (
                "a still mesh added",
                Box::new(move |w| {
                    w.set_object(18, "still".into(), None, mesh(3, 1), true, false).unwrap();
                    w.set_transform(18, Vec3::new(0.7, 0.6, 0.6), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
                }),
            ),
            ("the still added mesh moved", Box::new(|w| w.set_transform(18, Vec3::new(0.6, 0.3, 0.9), glam::Quat::from_rotation_z(-0.3), Vec3::ONE).unwrap())),
            ("the added movable mesh hidden", Box::new(|w| w.set_visible(13, false).unwrap())),
            ("the added movable mesh moved while hidden", Box::new(|w| w.set_transform(13, Vec3::new(-0.6, 0.2, 1.2), glam::Quat::IDENTITY, Vec3::ONE).unwrap())),
            ("the added movable mesh shown", Box::new(|w| w.set_visible(13, true).unwrap())),
            (
                "the lamp removed",
                Box::new(|w| {
                    w.remove(12);
                }),
            ),
            (
                "a group of two meshes removed",
                Box::new(|w| {
                    w.remove(14);
                }),
            ),
        ];
        for (what, edit) in &edits {
            let what = format!("{what}, {} a light tree", if tree { "with" } else { "without" });
            edit(&mut w);
            assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
            if live.sc.tables.emitter_tris.iter().any(|e| e.area > 0.0) {
                list_is_its_triangles(&what, &live);
            }
            lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
        }
    }
}

/// A still model whose instance is all of it (tests/render/data/panel.glb: a grey quad and a quad that only emits) is
/// hidden, shown, removed and moved in place as a mesh is. With and without a light tree. After each: no rebuild, the
/// light list is its triangles, the picture is a build's (and the weights, until one has moved).
#[test]
fn a_still_model_is_hidden_removed_and_moved_in_place() {
    let presets = crate::default_presets_dir();
    let open_at_root = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let model = || Kind::Model { uri: "tests/render/data/panel.glb".into(), nodes: None, animation: None, functions: vec![] };
    for tree in [false, true] {
        let mut w = world();
        // (The lamp away: the models' panels light the scene.)
        w.set_visible(12, false).unwrap();
        for (id, at) in [(30, Vec3::new(-0.6, -0.5, 0.6)), (31, Vec3::new(0.5, 0.4, 0.7)), (32, Vec3::new(0.0, 0.6, 0.5))] {
            w.set_object(id, format!("m{id}"), None, model(), true, false).unwrap();
            w.set_transform(id, at, glam::Quat::from_rotation_y(0.4), Vec3::splat(0.05)).unwrap();
        }
        if tree {
            strip(&mut w);
        }
        let mut live = match open_at_root(&w) {
            Ok(c) => c,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        w.take_changes();
        picture(&mut live, 4);
        let with = if tree { "with" } else { "without" };
        if live.g.hwrt {
            assert!(matches!(&live.sc.tables.retained.proxies[&30].draw, scene::Draw::Instance(i) if !i.moves && matches!(i.mesh, scene::MeshKind::Model)), "a still model's proxy has its instance");
        }
        type Edit = (&'static str, bool, Box<dyn Fn(&mut World)>);
        let edits: Vec<Edit> = vec![
            ("a still model hidden", true, Box::new(|w| w.set_visible(30, false).unwrap())),
            ("the model shown", true, Box::new(|w| w.set_visible(30, true).unwrap())),
            (
                "a still model removed",
                true,
                Box::new(|w| {
                    w.remove(32);
                }),
            ),
            ("a still model moved", false, Box::new(|w| w.set_transform(31, Vec3::new(0.3, 0.2, 1.0), glam::Quat::from_rotation_y(-0.3), Vec3::splat(0.05)).unwrap())),
            ("the model moved again", false, Box::new(|w| w.set_transform(31, Vec3::new(0.1, 0.3, 1.1), glam::Quat::from_rotation_z(0.3), Vec3::splat(0.05)).unwrap())),
        ];
        for (what, weights, edit) in &edits {
            let what = format!("{what}, {with} a light tree");
            edit(&mut w);
            let rebuilt = live.commit(&mut w).unwrap();
            assert!(!(rebuilt && live.g.hwrt), "{what}: in place ({:?})", live.fallback);
            list_is_its_triangles(&what, &live);
            let mut fresh = open_at_root(&w).unwrap();
            if *weights {
                lit_as_a_build(&what, &mut live, &mut fresh);
            } else {
                // (A moving emitter's centre is not kept on the CPU: the picture alone.)
                forget(&mut live);
                let (a, b) = (picture(&mut live, 64), picture(&mut fresh, 64));
                assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
            }
        }
    }
}

/// Objects of one geometry share its mesh whatever their materials: one that reads the geometry's vertex colours
/// and one that does not, a cutout that lets everything through and one that lets nothing, glass, one that emits. At
/// the build; one more placed in place; an object given another material, which keeps the mesh (none is made, no
/// triangle written). Each time the picture is a build's, and the picture of the same world with a geometry an
/// object, so a mesh an object; it is not the picture of the objects all with one material, nor of the plain one
/// reading the colours too.
#[test]
fn objects_of_one_geometry_share_its_mesh_whatever_their_materials() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let json = |v: serde_json::Value| serde_json::from_value::<crate::desc::Material>(v).unwrap();
    let plain = |extra: serde_json::Value| {
        let mut m = serde_json::json!({ "name": "m", "color": [0.9, 0.9, 0.9], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true });
        m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        json(m)
    };
    // (30 + i: the geometry of object 40 + i where each has its own.)
    let tile = || Geometry { positions: vec![-0.4, -0.4, 0.0, 0.4, -0.4, 0.0, 0.4, 0.4, 0.0, -0.4, 0.4, 0.0], normals: [0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0] };
    let mats = [61, 62, 63, 64, 65, 5];
    // (`variant` 1: the objects all of the plain material; 2: the plain material reads the colours too.)
    let scene = |own: bool, variant: u32| {
        let mut w = world();
        w.set_material(61, plain(serde_json::json!({ "vertexColors": true })));
        w.set_material(62, plain(serde_json::json!({ "vertexColors": variant == 2 })));
        w.set_material(63, plain(serde_json::json!({ "opacity": 0.3, "alphaTest": 0.5 })));
        w.set_material(64, plain(serde_json::json!({ "opacity": 0.9, "alphaTest": 0.5, "color": [0.2, 0.6, 0.9] })));
        w.set_material(65, plain(serde_json::json!({ "color": [0.5, 0.9, 0.5], "roughness": 0, "transmission": 1, "ior": 1.5 })));
        for (i, mat) in mats.iter().enumerate() {
            let geometry = 30 + if own { i as u32 } else { 0 };
            w.set_geometry(geometry, tile()).unwrap();
            let id = 40 + i as u32;
            w.set_object(id, format!("tile{i}"), None, Kind::Mesh { geometry, material: if variant == 1 { 62 } else { *mat }, own_blas: i == 0, materials: vec![] }, true, false).unwrap();
            w.set_transform(id, Vec3::new(-2.25 + 0.9 * i as f32, 0.55, 0.6), glam::Quat::from_rotation_y(0.2), Vec3::ONE).unwrap();
        }
        w
    };
    let mesh_of = |c: &LiveCore, id: u32| c.sc.instances[c.sc.tables.retained.instance(id).expect("a mesh is an instance").instance as usize].mesh;
    let mut w = scene(false, 0);
    let mut live = match open(&w) {
        Ok(c) if c.g.hwrt => c,
        Ok(_) => return eprintln!("skipped: no hardware RT"),
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let shared = mesh_of(&live, 40);
    assert!((41..46).all(|id| mesh_of(&live, id) == shared), "the six objects of one geometry share its mesh");
    assert!(live.sc.meshes[shared as usize].colored && live.sc.meshes[shared as usize].sect_count == 1);
    let features = live.sc.up.kernel.features;
    assert_eq!(features & (gpu::F_MASKED | gpu::F_VCOLOR), gpu::F_MASKED | gpu::F_VCOLOR);
    let placed = |c: &LiveCore, id: u32| c.sc.instances[c.sc.tables.retained.instance(id).unwrap().instance as usize].clone();
    assert!(!placed(&live, 44).cast_shadow && placed(&live, 43).cast_shadow, "glass alone casts no shadow");
    list_is_its_triangles("objects of one geometry, at the build", &live);
    let mut apart_world = open(&scene(true, 0)).unwrap();
    let meshes: std::collections::HashSet<u32> = (40..46).map(|id| mesh_of(&apart_world, id)).collect();
    assert_eq!(meshes.len(), 6, "a geometry an object: a mesh an object");
    let (a, b) = (picture(&mut live, 64), picture(&mut apart_world, 64));
    assert!(apart(&a, &b) < 0.5, "one mesh for six materials: the picture is {:.2} steps from six meshes'", apart(&a, &b));
    let c = picture(&mut open(&scene(false, 1)).unwrap(), 64);
    assert!(apart(&a, &c) > 0.5, "the materials are in the picture ({:.2} steps from the objects all of one)", apart(&a, &c));
    let c = picture(&mut open(&scene(false, 2)).unwrap(), 64);
    assert!(apart(&a, &c) > 0.5, "the plain material reads no colour of the mesh ({:.2} steps from where it does)", apart(&a, &c));
    // One more of the geometry, glass, placed in place: the mesh it shares, nothing made.
    let (meshes, triangles) = (live.sc.meshes.len(), live.sc.pool_tris.len());
    let place = |w: &mut World, id: u32, material: u32, at: Vec3| {
        w.set_object(id, format!("tile{}", id - 40), None, Kind::Mesh { geometry: 30, material, own_blas: false, materials: vec![] }, true, false).unwrap();
        w.set_transform(id, at, glam::Quat::from_rotation_y(0.2), Vec3::ONE).unwrap();
    };
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        list_is_its_triangles(what, live);
        let mut fresh = open(w).unwrap();
        lit_as_a_build(what, live, &mut fresh);
    };
    place(&mut w, 46, 65, Vec3::new(0.2, -0.5, 0.9));
    assert!(!live.commit(&mut w).unwrap(), "an object of the geometry placed: in place ({:?})", live.fallback);
    assert_eq!((mesh_of(&live, 46), live.sc.meshes.len(), live.sc.pool_tris.len()), (shared, meshes, triangles), "it shares the mesh: none made");
    against_a_build("an object of the geometry placed in place", &mut live, &w);
    // Another material for an object: its row, the one it had (the build's, or the room's for the one placed above).
    // The plain one emits now, the one that emitted is a cutout, the glass one placed in place is plain.
    for (id, material, at) in [(41, 5, Vec3::new(-1.35, 0.55, 0.6)), (45, 64, Vec3::new(2.25, 0.55, 0.6)), (46, 62, Vec3::new(0.2, -0.5, 0.9))] {
        let row = placed(&live, id).rows;
        place(&mut w, id, material, at);
        let what = format!("object {id} given material {material}");
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        assert_eq!((mesh_of(&live, id), live.sc.meshes.len(), live.sc.pool_tris.len()), (shared, meshes, triangles), "{what}: it keeps the mesh, none made");
        assert_eq!(placed(&live, id).rows, row, "{what}: it keeps its row");
        assert_eq!(scene::row_material(&live.sc.rows[row as usize]), live.sc.tables.retained.materials[&material], "{what}: its row names it");
        against_a_build(&what, &mut live, &w);
    }
    // A geometry with colours that no object read at the build: its mesh was made without them, and an object that
    // reads them, placed later, has them by a rebuild.
    let mut w = world();
    w.set_material(61, plain(serde_json::json!({ "vertexColors": true })));
    w.set_material(62, plain(serde_json::json!({})));
    w.set_geometry(30, tile()).unwrap();
    w.set_object(40, "tile0".into(), None, Kind::Mesh { geometry: 30, material: 62, own_blas: true, materials: vec![] }, true, false).unwrap();
    w.set_transform(40, Vec3::new(-0.6, 0.55, 0.6), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    // (And a coloured mesh in the scene: the kernel has vertex colours.)
    w.set_geometry(31, tile()).unwrap();
    w.set_object(41, "tile1".into(), None, Kind::Mesh { geometry: 31, material: 61, own_blas: false, materials: vec![] }, true, false).unwrap();
    w.set_transform(41, Vec3::new(0.9, -0.6, 0.6), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = open(&w).unwrap();
    w.take_changes();
    assert!(!live.sc.meshes[mesh_of(&live, 40) as usize].colored && live.sc.meshes[mesh_of(&live, 41) as usize].colored);
    place(&mut w, 42, 61, Vec3::new(0.0, 0.55, 0.6));
    assert!(live.commit(&mut w).unwrap(), "colours asked of a mesh made without them: a rebuild");
    assert!(live.sc.meshes[mesh_of(&live, 42) as usize].colored && mesh_of(&live, 42) == mesh_of(&live, 40));
    against_a_build("an object that reads its geometry's colours placed", &mut live, &w);
}

/// Glass among an instance's other sections lets the sky's and the sun's light through as an instance that is all
/// glass does: a pane and its frame as one mesh of two materials, against the two as a mesh each, before a wall, in
/// a scene with a cutout and in one without (the kernel then tests no hit for a cutout, and still steps through the
/// glass). A shadow ray steps through the first's glass by its row and never meets the second's (its mask); a
/// sample of the sky weighs the same against the surface's own rays, which pass the glass either way (glass is no
/// cutout's hole), so the pictures are the same.
#[test]
fn glass_among_an_instances_sections_is_lit_through_as_glass_alone() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let json = |v: serde_json::Value| serde_json::from_value::<crate::desc::Material>(v).unwrap();
    let quads = |q: &[[f32; 4]], groups: Vec<[u32; 3]>| {
        let (mut positions, mut indices) = (vec![], vec![]);
        for (k, &[x0, y0, x1, y1]) in q.iter().enumerate() {
            positions.extend([x0, y0, 0.0, x1, y0, 0.0, x1, y1, 0.0, x0, y1, 0.0]);
            indices.extend([0, 1, 2, 0, 2, 3].map(|i| 4 * k as u32 + i));
        }
        Geometry { normals: [0.0, 0.0, 1.0].repeat(4 * q.len()), uvs: vec![0.0; 8 * q.len()], positions, indices, groups, colors: vec![] }
    };
    let (pane, frame) = ([-1.0, -1.0, 1.0, 0.6], [-1.0, 0.6, 1.0, 1.0]);
    // (`parts` 0: the pane and its frame one mesh; 1: a mesh each; 2: neither.)
    let scene = |parts: u32, cutout: bool| {
        let mut w = world();
        w.set_material(71, json(serde_json::json!({ "name": "pane", "color": [0.6, 0.9, 0.6], "opacity": 1, "metalness": 0, "roughness": 0, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true, "transmission": 1, "ior": 1.5 })));
        w.set_material(72, json(serde_json::json!({ "name": "frame", "color": [0.05, 0.05, 0.05], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true })));
        w.set_material(73, json(serde_json::json!({ "name": "grille", "color": [0.5, 0.5, 0.5], "opacity": 0.9, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true, "alphaTest": 0.5 })));
        // A sky of one colour, and no lamp beside it, so that the sky is sampled as a light (the rays the wall's surface
        // samples find it too); a cutout in a corner.
        w.set_environment(crate::desc::Environment { sky: sky(serde_json::json!({ "type": "color", "color": [1.0, 1.0, 1.0], "intensity": 0.3, "skyLightIntensity": 1 })), ..w.environment.clone() });
        w.set_material(5, json(serde_json::json!({ "name": "off", "color": [0.2, 0.2, 0.2], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true })));
        if cutout {
            w.set_object(13, "grille".into(), None, Kind::Mesh { geometry: 3, material: 73, own_blas: false, materials: vec![] }, true, false).unwrap();
            w.set_transform(13, Vec3::new(-0.8, -0.8, 0.9), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        }
        let at = |w: &mut World, id: u32| w.set_transform(id, Vec3::new(0.0, 0.0, 0.4), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        match parts {
            0 => {
                w.set_geometry(50, quads(&[pane, frame], vec![[0, 6, 0], [6, 6, 1]])).unwrap();
                w.set_object(30, "window".into(), None, Kind::Mesh { geometry: 50, material: 71, own_blas: false, materials: vec![71, 72] }, true, false).unwrap();
                at(&mut w, 30);
            }
            1 => {
                for (id, geometry, q, material) in [(30, 51, pane, 71), (31, 52, frame, 72)] {
                    w.set_geometry(geometry, quads(&[q], vec![])).unwrap();
                    w.set_object(id, format!("part{id}"), None, Kind::Mesh { geometry, material, own_blas: false, materials: vec![] }, true, false).unwrap();
                    at(&mut w, id);
                }
            }
            _ => {}
        }
        w
    };
    for cutout in [true, false] {
        let mut one = match open(&scene(0, cutout)) {
            Ok(c) if c.g.hwrt => c,
            Ok(_) => return eprintln!("skipped: no hardware RT"),
            Err(e) => return eprintln!("skipped: {e}"),
        };
        let window = one.sc.instances.iter().find(|i| i.name == "window").expect("the window is an instance").clone();
        assert_eq!((window.bits, window.cast_shadow, one.sc.meshes[window.mesh as usize].sect_count), (scene::INST_GLASS, true, 2), "glass and a frame: one mesh, which casts");
        let wanted = gpu::F_IGLASS | if cutout { gpu::F_MASKED } else { 0 };
        assert_eq!(one.sc.up.kernel.features & (wanted | gpu::F_MASKED), wanted);
        assert_eq!(one.sc.base.counts2[1], 1, "the sky is sampled as a light");
        let mut two = open(&scene(1, cutout)).unwrap();
        assert!(!two.sc.instances.iter().find(|i| i.name == "part30").unwrap().cast_shadow, "the pane alone casts no shadow");
        let (a, b) = (picture(&mut one, 96), picture(&mut two, 96));
        assert!(apart(&a, &b) < 0.5, "a pane among its mesh's sections, with a cutout in the scene: {cutout}: the picture is {:.2} steps from the pane's as a mesh of its own", apart(&a, &b));
        let c = picture(&mut open(&scene(2, cutout)).unwrap(), 96);
        assert!(apart(&a, &c) > 0.5, "the window is in the picture ({:.2} steps from none)", apart(&a, &c));
    }
}

/// A model of several materials placed in place has its rows from the room: as many as its mesh's sections, the
/// file's materials, each where its section's triangles start, on the GPU as here, its record naming their first.
/// It is drawn as a build draws it. Removed, it gives the rows back, and the next one placed takes them.
#[test]
fn a_model_of_several_materials_placed_in_place_has_its_rows() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    // (An opaque, a cutout and a glass quad: three sections.)
    let model = || Kind::Model { uri: "tests/render/data/frame-pane.glb".into(), nodes: None, animation: None, functions: vec![] };
    let mut w = world();
    w.set_object(30, "first".into(), None, model(), true, false).unwrap();
    w.set_transform(30, Vec3::new(-0.5, 0.3, 0.8), glam::Quat::IDENTITY, Vec3::splat(0.5)).unwrap();
    // (A mesh the scene says it edits: room for what is placed.)
    w.set_object(28, "own".into(), None, Kind::Mesh { geometry: 3, material: 1, own_blas: true, materials: vec![] }, true, false).unwrap();
    w.set_transform(28, Vec3::new(0.9, -0.1, 0.4), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let built = live.sc.rows.len();
    let placed = |c: &LiveCore, id: u32| c.sc.instances[c.sc.tables.retained.instance(id).expect("a model is an instance").instance as usize].clone();
    let read = |c: &LiveCore, b: &wgpu::Buffer, at: u64, bytes: u64| -> Vec<u8> {
        let to = c.g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: bytes, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut e = c.g.device.create_command_encoder(&Default::default());
        e.copy_buffer_to_buffer(b, at, &to, 0, bytes);
        c.g.queue.submit([e.finish()]);
        to.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        c.g.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let bytes = to.slice(..).get_mapped_range().unwrap().to_vec();
        bytes
    };
    let mut check = |live: &mut LiveCore, w: &World, id: u32, what: &str| -> u32 {
        let (first, it) = (placed(live, 30), placed(live, id));
        let im = live.sc.meshes[it.mesh as usize];
        assert_eq!((it.mesh, im.sect_count), (first.mesh, 3), "{what}: an instance of the model's mesh, of three sections");
        assert!(it.rows as usize >= built, "{what}: its rows are the room's");
        let rows = |i: &scene::Instance| live.sc.rows[i.rows as usize..][..3].to_vec();
        assert_eq!((rows(&it), it.bits, it.cast_shadow), (rows(&first), first.bits, first.cast_shadow), "{what}: the mesh's rows, as the first has them");
        let up = &live.sc.up;
        let on_gpu: Vec<scene::Row> = bytemuck::pod_collect_to_vec(&read(live, &up.words.buffer, up.words.at(crate::heap::Stream::Rows) + it.rows as u64 * 16, 48));
        assert_eq!(on_gpu, rows(&it), "{what}: its rows on the GPU");
        let record = live.sc.tables.retained.instance(id).unwrap().instance as u64;
        let size = std::mem::size_of::<scene::GpuInstance>() as u64;
        let rec: scene::GpuInstance = bytemuck::pod_read_unaligned(&read(live, &up.heap.buffer, up.heap.at(Table::Instances) + record * size, size));
        assert_eq!(rec.info[3], it.rows, "{what}: its record names its rows");
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 64), picture(&mut fresh, 64));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        it.rows
    };
    let place = |w: &mut World, id: u32, at: Vec3| {
        w.set_object(id, format!("m{id}"), None, model(), true, false).unwrap();
        w.set_transform(id, at, glam::Quat::from_rotation_y(0.3), Vec3::splat(0.5)).unwrap();
    };
    place(&mut w, 31, Vec3::new(0.5, -0.3, 0.9));
    assert!(!live.commit(&mut w).unwrap(), "a model placed: in place ({:?})", live.fallback);
    let rows = check(&mut live, &w, 31, "a model placed in place");
    w.remove(31);
    assert!(!live.commit(&mut w).unwrap(), "the model removed: in place ({:?})", live.fallback);
    assert!(rows_free(&live, rows, 3), "the model removed: its rows are the room's again");
    place(&mut w, 32, Vec3::new(0.2, 0.5, 1.0));
    assert!(!live.commit(&mut w).unwrap(), "another placed: in place ({:?})", live.fallback);
    assert_eq!(check(&mut live, &w, 32, "another placed after the first was removed"), rows, "it takes the rows the first gave back");
}

/// A model with glass among its sections, in a scene where no instance of it casts a shadow: the kernel was built
/// without what has a shadow ray pass such glass. The first instance that casts is placed in place and asks for it
/// (`F_IGLASS`): the picture is a build's.
#[test]
fn the_first_caster_with_glass_among_its_sections_comes_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let model = || Kind::Model { uri: "tests/render/data/frame-pane.glb".into(), nodes: None, animation: None, functions: vec![] };
    let mut w = world();
    w.set_object(30, "first".into(), None, model(), false, false).unwrap();
    w.set_transform(30, Vec3::new(-0.5, 0.3, 0.8), glam::Quat::IDENTITY, Vec3::splat(0.5)).unwrap();
    // (A mesh the scene says it edits: room for what is placed.)
    w.set_object(28, "own".into(), None, Kind::Mesh { geometry: 3, material: 1, own_blas: true, materials: vec![] }, true, false).unwrap();
    w.set_transform(28, Vec3::new(0.9, -0.1, 0.4), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    assert_eq!(live.sc.up.kernel.features & gpu::F_IGLASS, 0, "no caster has glass among its sections");
    // (Upright before the wall, its pane 1 m wide: the pane's shadow is the sun's light through it.)
    w.set_object(31, "caster".into(), None, model(), true, false).unwrap();
    w.set_transform(31, Vec3::new(0.5, 0.0, 0.5), glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_2), Vec3::splat(2.0)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "in place ({:?})", live.fallback);
    let mut fresh = open(&w).unwrap();
    forget(&mut live);
    let (a, b) = (picture(&mut live, 64), picture(&mut fresh, 64));
    assert!(apart(&a, &b) < 0.5, "the picture is {:.2} steps from a build's", apart(&a, &b));
    assert_ne!(live.sc.up.kernel.features & gpu::F_IGLASS, 0, "the kernel passes its glass");
    // So for a mesh of several materials, glass one of them, added where none like it cast a shadow.
    let mut w = world();
    w.set_material(65, serde_json::from_value(serde_json::json!({ "name": "m", "color": [0.5, 0.9, 0.5], "opacity": 1, "metalness": 0, "roughness": 0, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true, "transmission": 1, "ior": 1.5 })).unwrap());
    w.set_object(28, "own".into(), None, Kind::Mesh { geometry: 3, material: 1, own_blas: true, materials: vec![] }, true, false).unwrap();
    w.set_transform(28, Vec3::new(0.9, -0.1, 0.4), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = open(&w).unwrap();
    w.take_changes();
    assert_eq!(live.sc.up.kernel.features & gpu::F_IGLASS, 0, "no caster has glass among its sections");
    w.set_geometry(30, Geometry { positions: vec![-1.0, -0.5, 0.0, 0.0, -0.5, 0.0, 1.0, -0.5, 0.0, -1.0, 0.5, 0.0, 0.0, 0.5, 0.0, 1.0, 0.5, 0.0], normals: [0.0, 0.0, 1.0].repeat(6), uvs: vec![0.0; 12], indices: vec![0, 1, 4, 0, 4, 3, 1, 2, 5, 1, 5, 4], groups: vec![[0, 6, 0], [6, 6, 1]], colors: vec![] }).unwrap();
    w.set_object(31, "pane".into(), None, Kind::Mesh { geometry: 30, material: 1, own_blas: false, materials: vec![1, 65] }, true, false).unwrap();
    w.set_transform(31, Vec3::new(0.0, 0.0, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "in place ({:?})", live.fallback);
    let mut fresh = open(&w).unwrap();
    forget(&mut live);
    let (a, b) = (picture(&mut live, 64), picture(&mut fresh, 64));
    assert!(apart(&a, &b) < 0.5, "a mesh with glass among its sections: the picture is {:.2} steps from a build's", apart(&a, &b));
    assert_ne!(live.sc.up.kernel.features & gpu::F_IGLASS, 0, "the kernel passes its glass");
}

/// A mesh of several materials (a geometry group each) is edited in place as one of a single material is: added
/// (its mesh made with its sections), given other materials (its rows: nothing made), another of its geometry added
/// (the mesh shared), its geometry changed (the same sections: where it lies; others: a new mesh), removed (its rows
/// the room's again). Each time the light list, the weights and the picture are a build's.
#[test]
fn a_mesh_of_several_materials_is_edited_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let plain = |extra: serde_json::Value| {
        let mut m = serde_json::json!({ "name": "m", "color": [0.9, 0.9, 0.9], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true });
        m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value::<crate::desc::Material>(m).unwrap()
    };
    // Bands side by side, `w` wide each, a group for each run of `groups` of them.
    let bands = |w: f32, groups: &[u32]| {
        let n: u32 = groups.iter().sum();
        let positions: Vec<f32> = (0..=n).flat_map(|i| { let x = (i as f32 - n as f32 / 2.0) * w; [x, -0.4, 0.0, x, 0.4, 0.0] }).collect();
        let indices: Vec<u32> = (0..n).flat_map(|i| [2 * i, 2 * i + 2, 2 * i + 3, 2 * i, 2 * i + 3, 2 * i + 1]).collect();
        let mut at = 0;
        let groups = groups.iter().enumerate().map(|(k, &g)| { at += g * 6; [at - g * 6, g * 6, k as u32] }).collect();
        Geometry { normals: [0.0, 0.0, 1.0].repeat(2 * (n as usize + 1)), uvs: vec![0.0; 4 * (n as usize + 1)], positions, indices, groups, colors: vec![] }
    };
    let mut w = world();
    w.set_material(62, plain(serde_json::json!({})));
    w.set_material(63, plain(serde_json::json!({ "opacity": 0.3, "alphaTest": 0.5 })));
    w.set_material(64, plain(serde_json::json!({ "opacity": 0.9, "alphaTest": 0.5, "color": [0.2, 0.6, 0.9] })));
    w.set_material(65, plain(serde_json::json!({ "color": [0.5, 0.9, 0.5], "roughness": 0, "transmission": 1, "ior": 1.5 })));
    // (At the build: a caster with a cutout and glass among its sections, so the kernel has both; a mesh the scene
    // says it edits, for the room.)
    w.set_geometry(30, bands(0.3, &[1, 1, 1])).unwrap();
    w.set_object(40, "bands".into(), None, Kind::Mesh { geometry: 30, material: 62, own_blas: true, materials: vec![62, 63, 65] }, true, false).unwrap();
    w.set_transform(40, Vec3::new(-0.9, 0.6, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let placed = |c: &LiveCore, id: u32| c.sc.instances[c.sc.tables.retained.instance(id).expect("a mesh is an instance").instance as usize].clone();
    let rows_of = |c: &LiveCore, id: u32| { let i = placed(c, id); c.sc.rows[i.rows as usize..][..c.sc.meshes[i.mesh as usize].sect_count as usize].to_vec() };
    let materials = |c: &LiveCore, id: u32| rows_of(c, id).iter().map(scene::row_material).collect::<Vec<u32>>();
    let of = |c: &LiveCore, ids: &[u32]| ids.iter().map(|k| c.sc.tables.retained.materials[k]).collect::<Vec<u32>>();
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        list_is_its_triangles(what, live);
        let mut fresh = open(w).unwrap();
        lit_as_a_build(what, live, &mut fresh);
    };
    let place = |w: &mut World, id: u32, geometry: u32, materials: &[u32], at: Vec3| {
        w.set_object(id, format!("bands{id}"), None, Kind::Mesh { geometry, material: materials[0], own_blas: false, materials: materials.to_vec() }, true, false).unwrap();
        w.set_transform(id, at, glam::Quat::from_rotation_y(0.2), Vec3::ONE).unwrap();
    };
    // Added: an emitter, a cutout, a plain band.
    w.set_geometry(31, bands(0.25, &[1, 1, 1])).unwrap();
    place(&mut w, 41, 31, &[5, 64, 62], Vec3::new(0.3, -0.5, 0.9));
    assert!(!live.commit(&mut w).unwrap(), "a mesh of three materials added: in place ({:?})", live.fallback);
    let (mesh, first) = (placed(&live, 41).mesh, placed(&live, 41).rows);
    assert_eq!((live.sc.meshes[mesh as usize].sect_count, materials(&live, 41)), (3, of(&live, &[5, 64, 62])), "its mesh has three sections, its rows their materials");
    against_a_build("a mesh of three materials added in place", &mut live, &w);
    // Other materials: its rows where they are, no mesh made, no triangle written. The emitter is another section's.
    let (meshes, triangles) = (live.sc.meshes.len(), live.sc.pool_tris.len());
    place(&mut w, 41, 31, &[62, 65, 5], Vec3::new(0.3, -0.5, 0.9));
    assert!(!live.commit(&mut w).unwrap(), "its materials changed: in place ({:?})", live.fallback);
    assert_eq!((placed(&live, 41).mesh, placed(&live, 41).rows, live.sc.meshes.len(), live.sc.pool_tris.len()), (mesh, first, meshes, triangles), "it keeps its mesh and its rows: nothing made");
    assert_eq!(materials(&live, 41), of(&live, &[62, 65, 5]));
    against_a_build("a mesh of three materials given others", &mut live, &w);
    // Another object of the geometry, with its own materials: the mesh shared.
    place(&mut w, 42, 31, &[64, 5, 62], Vec3::new(-0.2, 0.55, 1.2));
    assert!(!live.commit(&mut w).unwrap(), "another of the geometry: in place ({:?})", live.fallback);
    assert_eq!((placed(&live, 42).mesh, live.sc.meshes.len()), (mesh, meshes), "it shares the mesh");
    against_a_build("another object of a geometry of three materials", &mut live, &w);
    // The geometry changed, its sections as they were: the mesh rewritten where it lies.
    w.set_geometry(31, bands(0.2, &[1, 1, 1])).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "its geometry changed: in place ({:?})", live.fallback);
    assert_eq!((placed(&live, 41).mesh, placed(&live, 42).mesh, live.sc.meshes.len()), (mesh, mesh, meshes), "the same sections: the mesh where it lies");
    against_a_build("a geometry of three materials changed, its sections as they were", &mut live, &w);
    // Other sections (two groups, the first of two bands): a new mesh, the rows as many as it has now.
    w.set_geometry(31, bands(0.2, &[2, 1])).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "its geometry's groups changed: in place ({:?})", live.fallback);
    let mesh = placed(&live, 41).mesh;
    assert_eq!((live.sc.meshes[mesh as usize].sect_count, placed(&live, 42).mesh, materials(&live, 41), materials(&live, 42)), (2, mesh, of(&live, &[62, 65]), of(&live, &[64, 5])), "two sections: two rows an object");
    against_a_build("a geometry whose groups changed", &mut live, &w);
    // Three sections again, then four: a freed mesh's sections are the next one's only where they are as many.
    for groups in [&[1, 1, 1][..], &[1, 1, 1, 1]] {
        w.set_geometry(31, bands(0.2, groups)).unwrap();
        let what = format!("a geometry of {} groups after one of fewer", groups.len());
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        assert_eq!(live.sc.meshes[placed(&live, 41).mesh as usize].sect_count as usize, groups.len(), "{what}: as many sections");
        against_a_build(&what, &mut live, &w);
    }
    // Removed: their rows are the room's again.
    let (a, b) = (placed(&live, 41).rows, placed(&live, 42).rows);
    w.remove(41);
    w.remove(42);
    assert!(!live.commit(&mut w).unwrap(), "the two removed: in place ({:?})", live.fallback);
    assert!(rows_free(&live, a, 4) && rows_free(&live, b, 4), "their rows are the room's again");
    against_a_build("meshes of several materials removed", &mut live, &w);
}

/// A material that becomes a cutout, or stops being one, in place: the instances with a row of it say so (a mesh, a
/// mesh of several materials, an instanced object's copies), and the picture is a build's. In a scene without a
/// cutout the kernel has no alpha test: the first one is a rebuild's. A material created after the build, of any
/// class (a cutout, glass, one that emits), is added in place with the mesh that is drawn with it.
#[test]
fn a_materials_alpha_test_comes_and_goes_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let plain = |extra: serde_json::Value| {
        let mut m = serde_json::json!({ "name": "m", "color": [0.9, 0.9, 0.9], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true });
        m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value::<crate::desc::Material>(m).unwrap()
    };
    let two = || Geometry { positions: vec![-0.5, -0.3, 0.0, 0.0, -0.3, 0.0, 0.5, -0.3, 0.0, -0.5, 0.3, 0.0, 0.0, 0.3, 0.0, 0.5, 0.3, 0.0], normals: [0.0, 0.0, 1.0].repeat(6), uvs: vec![0.0; 12], indices: vec![0, 1, 4, 0, 4, 3, 1, 2, 5, 1, 5, 4], groups: vec![[0, 6, 0], [6, 6, 1]], colors: vec![] };
    // (`cutout`: a material of the scene is a cutout at the build, so the kernel has the alpha test.)
    let scene = |cutout: bool| {
        let mut w = world();
        // (Opacity 0.3: with an alpha test at 0.5, nothing of it is there.)
        w.set_material(61, plain(serde_json::json!({ "opacity": 0.3, "color": [0.2, 0.6, 0.9] })));
        w.set_material(62, plain(serde_json::json!({})));
        w.set_material(63, plain(serde_json::json!({ "opacity": 0.9, "alphaTest": if cutout { 0.5 } else { 0.0 } })));
        w.set_object(40, "tile".into(), None, Kind::Mesh { geometry: 3, material: 61, own_blas: true, materials: vec![] }, true, false).unwrap();
        w.set_transform(40, Vec3::new(-0.6, 0.5, 0.6), glam::Quat::IDENTITY, Vec3::splat(1.6)).unwrap();
        w.set_geometry(30, two()).unwrap();
        w.set_object(41, "pair".into(), None, Kind::Mesh { geometry: 30, material: 62, own_blas: false, materials: vec![62, 61] }, true, false).unwrap();
        w.set_transform(41, Vec3::new(0.5, 0.5, 0.7), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_object(42, "copies".into(), None, Kind::Instanced { geometry: 3, procedural: false, materials: vec![61], capacity: 2 }, true, false).unwrap();
        let at = |x: f32| glam::Mat4::from_translation(Vec3::new(x, -0.5, 0.8)).to_cols_array();
        w.set_instances(42, 2, Some([at(-0.4), at(0.4)].concat()), None, None).unwrap();
        w.set_object(43, "other".into(), None, Kind::Mesh { geometry: 3, material: 63, own_blas: false, materials: vec![] }, true, false).unwrap();
        w.set_transform(43, Vec3::new(1.2, -0.5, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w
    };
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| -> Vec<u8> {
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 64), picture(&mut fresh, 64));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        a
    };
    let mut w = scene(true);
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let bits = |c: &LiveCore, name: &str| c.sc.instances.iter().filter(|i| i.name == name && (!i.hidden || i.masked)).map(|i| i.bits & scene::INST_CUTOUT).collect::<Vec<u32>>();
    let before = picture(&mut live, 64);
    w.set_material(61, plain(serde_json::json!({ "opacity": 0.3, "color": [0.2, 0.6, 0.9], "alphaTest": 0.5 })));
    assert!(!live.commit(&mut w).unwrap(), "a material becomes a cutout: in place ({:?})", live.fallback);
    assert_eq!((bits(&live, "tile"), bits(&live, "pair"), bits(&live, "copies")), (vec![scene::INST_CUTOUT], vec![scene::INST_CUTOUT], vec![scene::INST_CUTOUT; 2]), "their records say they have a cutout");
    let holes = against_a_build("a material becomes a cutout", &mut live, &w);
    assert!(apart(&before, &holes) > 0.5, "what was drawn with it is gone ({:.2} steps)", apart(&before, &holes));
    w.set_material(61, plain(serde_json::json!({ "opacity": 0.3, "color": [0.2, 0.6, 0.9] })));
    assert!(!live.commit(&mut w).unwrap(), "it stops being one: in place ({:?})", live.fallback);
    assert_eq!((bits(&live, "tile"), bits(&live, "pair"), bits(&live, "copies")), (vec![0], vec![0], vec![0; 2]), "their records say they have none");
    let back = against_a_build("a material stops being a cutout", &mut live, &w);
    assert!(apart(&before, &back) < 0.5, "it is drawn again ({:.2} steps)", apart(&before, &back));
    // Materials made after the build, of every class, and a mesh drawn with them.
    w.set_material(71, plain(serde_json::json!({ "opacity": 0.3, "alphaTest": 0.5 })));
    w.set_material(72, plain(serde_json::json!({ "color": [0.5, 0.9, 0.5], "roughness": 0, "transmission": 1, "ior": 1.5 })));
    w.set_material(73, plain(serde_json::json!({ "color": [0.0, 0.0, 0.0], "emissive": [1.0, 0.6, 0.2], "emissiveIntensity": 30 })));
    for (i, material) in [71, 72, 73].into_iter().enumerate() {
        w.set_object(50 + i as u32, format!("late{i}"), None, Kind::Mesh { geometry: 3, material, own_blas: false, materials: vec![] }, true, false).unwrap();
        w.set_transform(50 + i as u32, Vec3::new(-1.0 + i as f32, 0.0, 1.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    }
    assert!(!live.commit(&mut w).unwrap(), "materials of every class made after the build, and meshes of them: in place ({:?})", live.fallback);
    list_is_its_triangles("a material that emits, made after the build", &live);
    lit_as_a_build("materials of every class made after the build", &mut live, &mut open(&w).unwrap());
    // Without a cutout at the build, the first is a rebuild's.
    let mut w = scene(false);
    let mut live = open(&w).unwrap();
    w.take_changes();
    assert_eq!(live.sc.up.kernel.features & gpu::F_MASKED, 0, "no cutout: the kernel has no alpha test");
    w.set_material(61, plain(serde_json::json!({ "opacity": 0.3, "color": [0.2, 0.6, 0.9], "alphaTest": 0.5 })));
    live.commit(&mut w).unwrap();
    against_a_build("the scene's first cutout", &mut live, &w);
    assert_ne!(live.sc.up.kernel.features & gpu::F_MASKED, 0, "the kernel has it now");
}

/// An instanced object given another material, in place: every copy's row names it (a copy that is not shown too),
/// with its class, and the copies' emitters go or are made with it. Plain, then one that emits, a third copy shown,
/// a cutout, glass, plain again: each time the light list, the weights and the picture are a build's.
#[test]
fn an_instanced_object_given_another_material_has_it_in_its_copies_rows() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let plain = |extra: serde_json::Value| {
        let mut m = serde_json::json!({ "name": "m", "color": [0.9, 0.9, 0.9], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true });
        m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value::<crate::desc::Material>(m).unwrap()
    };
    let mut w = world();
    w.set_material(62, plain(serde_json::json!({ "color": [0.2, 0.6, 0.9] })));
    w.set_material(63, plain(serde_json::json!({ "opacity": 0.3, "alphaTest": 0.5 })));
    w.set_material(65, plain(serde_json::json!({ "color": [0.5, 0.9, 0.5], "roughness": 0, "transmission": 1, "ior": 1.5 })));
    let copies = |w: &mut World, material: u32| w.set_object(42, "copies".into(), None, Kind::Instanced { geometry: 3, procedural: false, materials: vec![material], capacity: 3 }, true, false).unwrap();
    let at = |x: f32, y: f32| glam::Mat4::from_translation(Vec3::new(x, y, 0.8)).to_cols_array();
    // (`by`: the copies moved that far along x.)
    let placed = |w: &mut World, count: u32, by: f32| w.set_instances(42, count, Some([at(-0.5 + by, -0.5), at(0.4 + by, -0.5), at(by, 0.5)].concat()), None, None).unwrap();
    let shown = |w: &mut World, count: u32| placed(w, count, 0.0);
    copies(&mut w, 62);
    shown(&mut w, 2);
    // (A cutout at the build: the kernel has the alpha test.)
    w.set_object(43, "other".into(), None, Kind::Mesh { geometry: 3, material: 63, own_blas: false, materials: vec![] }, true, false).unwrap();
    w.set_transform(43, Vec3::new(1.3, 0.6, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let named = |c: &LiveCore, material: u32| {
        let place = *c.sc.tables.retained.instanced(42).expect("the copies are instances");
        (0..3).all(|i| scene::row_material(&c.sc.rows[c.sc.instances[place.first as usize + i].rows as usize]) == c.sc.tables.retained.materials[&material])
    };
    let mut last = picture(&mut live, 64);
    for (material, count, what) in [(5, 2, "one that emits"), (5, 3, "the third copy shown"), (63, 3, "a cutout"), (65, 3, "glass"), (62, 3, "a plain one again")] {
        copies(&mut w, material);
        shown(&mut w, count);
        let what = format!("an instanced object given {what}");
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        assert!(named(&live, material), "{what}: its copies' rows name it, the one not shown too");
        if live.sc.tables.emitter_tris.iter().any(|e| e.area > 0.0) {
            list_is_its_triangles(&what, &live);
        }
        lit_as_a_build(&what, &mut live, &mut open(&w).unwrap());
        let now = picture(&mut live, 64);
        assert!(apart(&last, &now) > 0.5, "{what}: it is in the picture ({:.2} steps)", apart(&last, &now));
        last = now;
    }
    // Another material and another place in one commit: the emitters are where the copies are now.
    copies(&mut w, 5);
    placed(&mut w, 3, 0.35);
    let what = "an instanced object given a material that emits and moved at once";
    assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
    list_is_its_triangles(what, &live);
    lit_as_a_build(what, &mut live, &mut open(&w).unwrap());
    copies(&mut w, 62);
    assert!(!live.commit(&mut w).unwrap(), "plain again: in place ({:?})", live.fallback);
    // Its material starts emitting while a copy is not shown: that copy's emitters weigh nothing.
    shown(&mut w, 2);
    assert!(!live.commit(&mut w).unwrap(), "a copy hidden: in place ({:?})", live.fallback);
    w.set_material(62, plain(serde_json::json!({ "color": [0.2, 0.6, 0.9], "emissive": [1.0, 0.7, 0.4], "emissiveIntensity": 25 })));
    let what = "the copies' material starts emitting, a copy not shown";
    assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
    list_is_its_triangles(what, &live);
    lit_as_a_build(what, &mut live, &mut open(&w).unwrap());
}

/// What still changes by a rebuild: a material of world-space triangles. Particles' (dynamic triangles: their class
/// is theirs, and they get no emitters in place) when it starts emitting or becomes a cutout; one that still
/// triangles have (the build groups them by whether they are cutouts) when it becomes a cutout. The same change of
/// a material that only instances' rows name is in place.
#[test]
fn a_material_of_world_space_triangles_changes_by_a_rebuild() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let plain = |extra: serde_json::Value| {
        let mut m = serde_json::json!({ "name": "m", "color": [0.9, 0.9, 0.9], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true });
        m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value::<crate::desc::Material>(m).unwrap()
    };
    let mut w = world();
    w.set_material(62, plain(serde_json::json!({})));
    w.set_material(63, plain(serde_json::json!({ "opacity": 0.9, "alphaTest": 0.5 })));
    w.set_material(64, plain(serde_json::json!({ "opacity": 0.9 })));
    w.set_particle_fn("spark", "fn pf_spark(i: PIn, p: array<vec4<f32>, 8>) -> POut { return POut(vec3<f32>(0.3, 0.2, 1.0), vec3<f32>(0.05, 0.0, 0.0), vec3<f32>(0.0, 0.05, 0.0), true, vec4<f32>(0.0)); }".into(), String::new());
    w.set_object(18, "sparks".into(), None, Kind::Particles { count: 3, emitter: "spark".into(), params: vec![], material: 62, emitter_area: 0.01 }, true, false).unwrap();
    // (A cutout at the build: the kernel has the alpha test. And a mesh of a material of its own.)
    w.set_object(43, "cut".into(), None, Kind::Mesh { geometry: 3, material: 63, own_blas: false, materials: vec![] }, true, false).unwrap();
    w.set_transform(43, Vec3::new(1.2, -0.5, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    w.set_object(44, "tile".into(), None, Kind::Mesh { geometry: 3, material: 64, own_blas: false, materials: vec![] }, true, false).unwrap();
    w.set_transform(44, Vec3::new(-1.0, -0.5, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let index = |c: &LiveCore, id: u32| c.sc.tables.retained.materials[&id];
    assert!(live.sc.tables.moving_mats.contains(&index(&live, 62)) && !live.sc.tables.moving_mats.contains(&index(&live, 64)), "the particles' material is dynamic triangles', the tile's is not");
    // The tile's material, which rows alone name: a cutout and back, in place.
    for (test, what) in [(0.5, "becomes a cutout"), (0.0, "is none again")] {
        w.set_material(64, plain(serde_json::json!({ "opacity": 0.9, "alphaTest": test })));
        assert!(!live.commit(&mut w).unwrap(), "a material of rows {what}: in place ({:?})", live.fallback);
    }
    // The same, were it still triangles' too: a rebuild.
    let k = index(&live, 64);
    live.sc.tables.static_mats.insert(k);
    w.set_material(64, plain(serde_json::json!({ "opacity": 0.9, "alphaTest": 0.5 })));
    assert!(live.commit(&mut w).unwrap(), "a material of still triangles becomes a cutout: a rebuild");
    // The particles' material: it starts emitting, it becomes a cutout.
    w.set_material(62, plain(serde_json::json!({ "emissive": [1.0, 0.8, 0.6], "emissiveIntensity": 20 })));
    assert!(live.commit(&mut w).unwrap(), "the particles' material starts emitting: a rebuild");
    w.set_material(62, plain(serde_json::json!({ "emissive": [1.0, 0.8, 0.6], "emissiveIntensity": 20, "opacity": 0.9, "alphaTest": 0.5 })));
    assert!(live.commit(&mut w).unwrap(), "the particles' material becomes a cutout: a rebuild");
}

/// A material that turns glass takes its instances' shadows with it, in place, and gives them back when it is
/// plain again: a screen before the wall in the sun, of a material of its own. The picture is a build's each time.
#[test]
fn a_material_turned_glass_stops_casting_its_shadow_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let screen = |glass: bool| {
        let mut m = serde_json::json!({ "name": "m", "color": [0.9, 0.9, 0.9], "opacity": 1, "metalness": 0, "roughness": if glass { 0.0 } else { 1.0 }, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true });
        if glass {
            m.as_object_mut().unwrap().extend(serde_json::json!({ "transmission": 1, "ior": 1.5 }).as_object().unwrap().clone());
        }
        serde_json::from_value::<crate::desc::Material>(m).unwrap()
    };
    let mut w = world();
    w.set_material(66, screen(false));
    w.set_object(45, "screen".into(), None, Kind::Mesh { geometry: 2, material: 66, own_blas: false, materials: vec![] }, true, false).unwrap();
    w.set_transform(45, Vec3::new(0.3, 0.0, 0.5), glam::Quat::IDENTITY, Vec3::splat(0.5)).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let casts = |c: &LiveCore| c.sc.instances.iter().find(|i| i.name == "screen").expect("the screen is an instance").cast_shadow;
    let mut last = picture(&mut live, 64);
    for glass in [true, false, true] {
        w.set_material(66, screen(glass));
        let what = if glass { "the screen's material turns glass" } else { "it is plain again" };
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        assert_eq!(casts(&live), !glass, "{what}: whether it casts a shadow");
        let mut fresh = open(&w).unwrap();
        forget(&mut live);
        let (a, b) = (picture(&mut live, 64), picture(&mut fresh, 64));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        assert!(apart(&last, &a) > 0.5, "{what}: it is in the picture ({:.2} steps)", apart(&last, &a));
        last = a;
    }
}

/// A posed rigid model with nodes its clip leaves alone (tests/render/data/panel-slider.glb: a panel that emits on
/// a node 5 m along x, a grey quad on a node the clip moves) is posed and moved in place: a new pose moves the
/// node's instance alone; one moved takes the part left alone with it, whose emitters are moving ones from that
/// first move (still ones, in the light tree, until then), movable object or not. Each time the light list is its
/// triangles and the picture a build's. Under software traversal too, where its parts are units.
/// A posed rigid model given another clip of its file, which moves another node (tests/render/data/two-clips.glb), is
/// posed in place: its parts are the nodes any clip moves, so they are the same ones.
#[test]
fn a_posed_rigid_model_given_another_clip_is_posed_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let pose = |clip: &str, time| Some(crate::desc::Animation { clip: clip.into(), time, r#loop: true, nodes: Default::default() });
    let mut w = world();
    w.set_object(32, "posed".into(), None, Kind::Model { uri: "tests/render/data/two-clips.glb".into(), nodes: None, animation: pose("raise-left", 0.5), functions: vec![] }, true, false).unwrap();
    w.set_transform(32, Vec3::new(-0.2, 0.1, 0.6), glam::Quat::IDENTITY, Vec3::splat(0.12)).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let parts = |c: &LiveCore| {
        let r = c.sc.tables.retained.rigid(32).expect("a posed rigid model");
        (r.instances.iter().map(|&(node, i)| (node, c.sc.instances[i as usize].world)).collect::<Vec<_>>(), r.still.map(|i| c.sc.instances[i as usize].world))
    };
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        let mut fresh = open(w).unwrap();
        assert_eq!(parts(live), parts(&fresh), "{what}: its parts and where they stand, as a build's");
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
    };
    let loaded = parts(&live);
    let at = |p: &(Vec<(usize, glam::Mat4)>, Option<glam::Mat4>), node: usize| p.0.iter().find(|q| q.0 == node).expect("the node's part").1;
    assert_eq!((loaded.0.len(), loaded.1.is_some()), (2, true), "a part each node a clip moves, and the node none does");
    against_a_build("a posed rigid model, as loaded", &mut live, &w);
    w.set_animation(32, pose("raise-right", 0.75)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "another clip of the file: in place ({:?})", live.fallback);
    let other = parts(&live);
    assert!(at(&other, 1) != at(&loaded, 1) && at(&other, 2) != at(&loaded, 2) && other.1 == loaded.1, "the first clip's node is back where the file has it, the other's is raised, the third stays");
    against_a_build("another clip of the file", &mut live, &w);
}

/// A pose that moves other nodes than the one its parts were made for (it names a node the clip leaves alone): the
/// parts are a build's to make again. In place the named node would stay where the part it is merged into stands.
#[test]
fn a_posed_rigid_model_whose_new_pose_moves_other_nodes_is_rebuilt() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let pose = |time, nodes: &[(&str, [f32; 7])]| Some(crate::desc::Animation { clip: "slide".into(), time, r#loop: true, nodes: nodes.iter().map(|(n, v)| (n.to_string(), v.to_vec())).collect() });
    let model = |a| Kind::Model { uri: "tests/render/data/panel-slider.glb".into(), nodes: None, animation: a, functions: vec![] };
    let mut w = world();
    w.set_object(32, "posed".into(), None, model(pose(0.25, &[])), true, false).unwrap();
    w.set_transform(32, Vec3::new(-0.6, 0.2, 0.6), glam::Quat::IDENTITY, Vec3::splat(0.08)).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        list_is_its_triangles(what, live);
        let mut fresh = open(w).unwrap();
        let parts = |c: &LiveCore| c.sc.tables.retained.rigid(32).map(|r| (r.instances.iter().map(|&(node, i)| (node, c.sc.instances[i as usize].world)).collect::<Vec<_>>(), r.still.map(|i| c.sc.instances[i as usize].world)));
        assert_eq!(parts(live), parts(&fresh), "{what}: its parts and where they stand, as a build's");
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        a
    };
    let before = against_a_build("a posed rigid model, as loaded", &mut live, &w);
    // The panel, a node the clip leaves alone, named by the pose: lifted and turned.
    let lifted = [("panel", [2.0, 4.0, 0.0, 0.0, 0.0, 0.38268343, 0.92387953])];
    w.set_animation(32, pose(0.25, &lifted)).unwrap();
    assert!(live.commit(&mut w).unwrap(), "a pose that names a node the clip leaves alone: a rebuild");
    let named = against_a_build("a pose that names a node the clip leaves alone", &mut live, &w);
    assert!(apart(&before, &named) > 0.5, "the named node's pose is in the picture ({:.2} steps)", apart(&before, &named));
    // The same nodes named, posed on: in place.
    w.set_animation(32, pose(0.75, &[("panel", [3.0, 2.0, 0.0, 0.0, 0.0, 0.0, 1.0])])).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the same nodes posed on: in place ({:?})", live.fallback);
    against_a_build("the same nodes posed on", &mut live, &w);
    // Another placed with the same node named: the scene's parts, in place; posed on in place; named no more, a rebuild.
    w.set_object(33, "second".into(), None, model(pose(0.5, &lifted)), true, false).unwrap();
    w.set_transform(33, Vec3::new(0.5, -0.4, 0.7), glam::Quat::IDENTITY, Vec3::splat(0.08)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "another with the same node named: in place ({:?})", live.fallback);
    w.set_animation(33, pose(0.9, &lifted)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "it, posed on: in place ({:?})", live.fallback);
    against_a_build("another with the same node named, posed on", &mut live, &w);
    w.set_animation(33, pose(0.9, &[])).unwrap();
    assert!(live.commit(&mut w).unwrap(), "it, its node named no more: a rebuild");
    against_a_build("another, its node named no more", &mut live, &w);
    // Named no more: a rebuild again.
    w.set_animation(32, pose(0.75, &[])).unwrap();
    assert!(live.commit(&mut w).unwrap(), "a pose that names the node no more: a rebuild");
    against_a_build("a pose that names the node no more", &mut live, &w);
}

#[test]
fn a_posed_rigid_model_is_posed_and_moved_in_place_with_the_nodes_its_clip_leaves_alone() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let slide = |time| Some(crate::desc::Animation { clip: "slide".into(), time, r#loop: true, nodes: Default::default() });
    let model = |time| Kind::Model { uri: "tests/render/data/panel-slider.glb".into(), nodes: None, animation: slide(time), functions: vec![] };
    let size = Vec3::splat(0.08);
    let mut w = world();
    w.set_object(32, "posed".into(), None, model(0.25), true, false).unwrap();
    w.set_transform(32, Vec3::new(-1.3, 0.6, 0.5), glam::Quat::IDENTITY, size).unwrap();
    w.set_object(33, "movable".into(), None, model(0.25), true, true).unwrap();
    w.set_transform(33, Vec3::new(0.2, -0.7, 0.6), glam::Quat::IDENTITY, size).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let rigid = |c: &LiveCore, id: u32| c.sc.tables.retained.rigid(id).expect("a posed rigid model").clone();
    let (a, b) = (rigid(&live, 32), rigid(&live, 33));
    assert_eq!((live.sc.meshes.len(), live.sc.instances[a.still.unwrap() as usize].mesh, live.sc.instances[a.instances[0].1 as usize].mesh), (live.sc.meshes.len(), live.sc.instances[b.still.unwrap() as usize].mesh, live.sc.instances[b.instances[0].1 as usize].mesh), "the two placements share their parts' meshes");
    let moves = |c: &LiveCore, record: u32| c.sc.tables.emitter_tris.iter().filter(|e| e.inst == record).map(|e| e.moves).collect::<Vec<bool>>();
    assert_eq!((moves(&live, a.still.unwrap()), moves(&live, b.still.unwrap())), (vec![false; 2], vec![false; 2]), "the panels' emitters are still ones as loaded");
    list_is_its_triangles("posed rigid models, as loaded", &live);
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        list_is_its_triangles(what, live);
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        a
    };
    let before = against_a_build("posed rigid models, as loaded", &mut live, &w);
    // A new pose: the node's instance, and no other.
    w.set_animation(32, slide(0.75)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "a new pose: in place ({:?})", live.fallback);
    assert!(live.inst_moving.contains(&a.instances[0].1) && !live.inst_moving.contains(&a.still.unwrap()), "the node the clip moves is moved, the part it leaves alone is not");
    against_a_build("a posed rigid model's new pose", &mut live, &w);
    // The movable one moved, and turned: its panel with it.
    w.set_transform(33, Vec3::new(-0.6, -0.2, 0.9), glam::Quat::from_rotation_z(0.5), size).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "a movable posed rigid model moved: in place ({:?})", live.fallback);
    assert!(live.inst_moving.contains(&b.still.unwrap()), "the part its clip leaves alone moves with the object");
    // (Its emitters: a new run of moving ones, the run it had at no weight.)
    let lit = |c: &LiveCore, record: u32| c.sc.tables.emitter_tris.iter().filter(|e| e.inst == record && e.area > 0.0).map(|e| e.moves).collect::<Vec<bool>>();
    assert_eq!((lit(&live, b.still.unwrap()), moves(&live, b.still.unwrap()).len(), lit(&live, a.still.unwrap())), (vec![true; 2], 4, vec![false; 2]), "the moved panel's emitters are moving ones now, the other's are not");
    let moving = live.sc.tables.retained.moving_records();
    assert!(moving.contains(&b.still.unwrap()) && !moving.contains(&a.still.unwrap()), "the moved panel is among the records an edit moves, the other is not");
    let moved = against_a_build("a movable posed rigid model moved", &mut live, &w);
    assert!(apart(&before, &moved) > 0.5, "the move is in the picture ({:.2} steps)", apart(&before, &moved));
    // Posed on where it stands now: the node alone again.
    w.set_animation(33, slide(0.6)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "posed where it stands now: in place ({:?})", live.fallback);
    assert!(!live.inst_moving.contains(&b.still.unwrap()), "the part left alone stays where the move put it");
    against_a_build("a moved posed rigid model posed on", &mut live, &w);
    // Moved and posed at once.
    w.set_transform(33, Vec3::new(0.4, 0.5, 0.7), glam::Quat::from_rotation_y(0.3), size).unwrap();
    w.set_animation(33, slide(0.1)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "moved and posed at once: in place ({:?})", live.fallback);
    assert_eq!(moves(&live, b.still.unwrap()).len(), 4, "a second move makes no new run of emitters");
    against_a_build("a posed rigid model moved and posed at once", &mut live, &w);
    // One that is not movable, moved: its panel as the movable one's.
    w.set_transform(32, Vec3::new(-1.0, -0.3, 0.8), glam::Quat::IDENTITY, size).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "a posed rigid model that is not movable, moved: in place ({:?})", live.fallback);
    assert_eq!(lit(&live, a.still.unwrap()), vec![true; 2], "its panel's emitters are moving ones now");
    against_a_build("a posed rigid model that is not movable, moved", &mut live, &w);
}

/// A posed rigid model under groups is not moved by a new pose, though the build and an update multiply its
/// transforms in another order (their products differ in their last bits here): the part its clip leaves alone stays
/// where it is, its emitters still ones.
#[test]
fn a_posed_rigid_model_under_groups_is_not_moved_by_a_new_pose() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let slide = |time| Some(crate::desc::Animation { clip: "slide".into(), time, r#loop: true, nodes: Default::default() });
    let mut w = world();
    w.set_object(50, "a".into(), None, Kind::Group, true, false).unwrap();
    w.set_transform(50, Vec3::new(0.13, -0.27, 0.31), glam::Quat::from_euler(glam::EulerRot::XYZ, 0.37, -0.53, 0.71), Vec3::new(1.1, 0.9, 1.3)).unwrap();
    w.set_object(51, "b".into(), Some(50), Kind::Group, true, false).unwrap();
    w.set_transform(51, Vec3::new(-0.21, 0.17, 0.09), glam::Quat::from_euler(glam::EulerRot::XYZ, -0.29, 0.61, 0.23), Vec3::new(0.7, 1.2, 0.8)).unwrap();
    w.set_object(32, "posed".into(), Some(51), Kind::Model { uri: "tests/render/data/panel-slider.glb".into(), nodes: None, animation: slide(0.25), functions: vec![] }, true, false).unwrap();
    w.set_transform(32, Vec3::new(-0.9, 0.3, 0.4), glam::Quat::from_rotation_y(0.4), Vec3::splat(0.08)).unwrap();
    let local = |id: u32| w.objects[&id].local();
    assert_ne!((local(50) * local(51)) * local(32), w.world_matrix(32), "the test's transforms: the two products differ");
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let still = live.sc.tables.retained.rigid(32).expect("a posed rigid model").still.expect("it has nodes left alone");
    w.set_animation(32, slide(0.75)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "a new pose: in place ({:?})", live.fallback);
    assert!(!live.sc.tables.retained.rigid(32).unwrap().still_moves && !live.inst_moving.contains(&still), "the object did not move: the part left alone is not placed again");
    let moves: Vec<bool> = live.sc.tables.emitter_tris.iter().filter(|e| e.inst == still).map(|e| e.moves).collect();
    assert_eq!(moves, vec![false; 2], "its emitters are still ones");
    list_is_its_triangles("a posed rigid model under groups, posed again", &live);
    let mut fresh = open(&w).unwrap();
    forget(&mut live);
    let (a, b) = (picture(&mut live, 48), picture(&mut fresh, 48));
    assert!(apart(&a, &b) < 0.5, "the picture is {:.2} steps from a build's", apart(&a, &b));
}

/// A posed rigid model is hidden, shown and removed in place, under either traversal. Hidden, its parts are
/// instances no ray sees, their emitters at no weight, and it is posed and moved on; shown, it stands where it was
/// moved to, as its clip has it now; removed, its instances are no object's. Each time the light list is its
/// triangles and the picture a build's.
#[test]
fn a_posed_rigid_model_is_hidden_shown_and_removed_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let slide = |time| Some(crate::desc::Animation { clip: "slide".into(), time, r#loop: true, nodes: Default::default() });
    let model = |time| Kind::Model { uri: "tests/render/data/panel-slider.glb".into(), nodes: None, animation: slide(time), functions: vec![] };
    let size = Vec3::splat(0.08);
    let mut w = world();
    w.set_object(32, "posed".into(), None, model(0.25), true, false).unwrap();
    w.set_transform(32, Vec3::new(-1.3, 0.6, 0.5), glam::Quat::IDENTITY, size).unwrap();
    w.set_object(33, "movable".into(), None, model(0.25), true, true).unwrap();
    w.set_transform(33, Vec3::new(0.2, -0.7, 0.6), glam::Quat::IDENTITY, size).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    assert!(live.sc.up.kernel.features & gpu::F_DEFORM == 0, "no mesh deforms here: the kernel has none of that");
    let records = |c: &LiveCore, id: u32| c.sc.tables.retained.rigid(id).expect("a posed rigid model").records();
    let (a, b) = (records(&live, 32), records(&live, 33));
    let seen = |c: &LiveCore, of: &[u32]| of.iter().map(|&i| (c.sc.instances[i as usize].hidden, c.sc.instances[i as usize].masked)).collect::<Vec<_>>();
    let lit = |c: &LiveCore, of: &[u32]| c.sc.tables.emitter_tris.iter().filter(|e| of.contains(&e.inst) && crate::emitter_power(e, &c.sc.tables.materials, &c.sc.tables.objects, c.sc.tables.stride) > 0.0).count();
    let rays = |c: &LiveCore, of: &[u32]| of.iter().map(|&i| seen_by_rays(c, i)).collect::<Vec<bool>>();
    // (The panel's two triangles: 32 m2 each in the model, 0.08 x 0.08 of that as it is placed.)
    let areas = |c: &LiveCore, of: &[u32]| c.sc.tables.emitter_tris.iter().filter(|e| of.contains(&e.inst) && e.area > 0.0).map(|e| (e.area * 1e4).round() / 1e4).collect::<Vec<f32>>();
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        list_is_its_triangles(what, live);
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        a
    };
    let before = against_a_build("posed rigid models, as loaded", &mut live, &w);
    assert_eq!((lit(&live, &a), lit(&live, &b)), (2, 2), "each panel's two emitters weigh something");
    // Hidden: instances no ray sees, emitters at no weight.
    w.set_visible(33, false).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "a posed rigid model hidden: in place ({:?})", live.fallback);
    assert_eq!((seen(&live, &b), lit(&live, &b), seen(&live, &a), lit(&live, &a)), (vec![(true, true); 2], 0, vec![(false, false); 2], 2), "its parts are unseen, the other model's are not");
    assert_eq!((rays(&live, &b), rays(&live, &a)), (vec![false; 2], vec![true; 2]), "no ray sees its parts");
    assert_eq!((areas(&live, &b), areas(&live, &a)), (vec![32.0; 2], vec![0.2048; 2]), "an unseen instance's emitters have their mesh's own area, as a hidden mesh's");
    let hidden = against_a_build("a posed rigid model hidden", &mut live, &w);
    assert!(apart(&before, &hidden) > 0.5, "it is gone from the picture ({:.2} steps)", apart(&before, &hidden));
    // Posed while hidden, then moved while hidden: nothing of it is drawn.
    w.set_animation(33, slide(0.8)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "posed while hidden: in place ({:?})", live.fallback);
    assert_eq!(rays(&live, &b), vec![false; 2], "posed while hidden: no ray sees its parts");
    w.set_transform(33, Vec3::new(-0.5, -0.1, 0.9), glam::Quat::from_rotation_z(0.5), size).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "moved while hidden: in place ({:?})", live.fallback);
    assert_eq!((seen(&live, &b), lit(&live, &b)), (vec![(true, true); 2], 0));
    let still = against_a_build("a hidden posed rigid model posed and moved", &mut live, &w);
    assert!(apart(&hidden, &still) < 0.5, "nothing of it is in the picture ({:.2} steps)", apart(&hidden, &still));
    // Shown: where it was moved to, as its clip has it now.
    w.set_visible(33, true).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "shown again: in place ({:?})", live.fallback);
    assert_eq!((seen(&live, &b), lit(&live, &b)), (vec![(false, false); 2], 2), "its parts are seen, its emitters weigh something again");
    assert_eq!((rays(&live, &b), areas(&live, &b)), (vec![true; 2], vec![0.2048; 2]), "the rays see them, their emitters as they are placed");
    let shown = against_a_build("a posed rigid model shown where it was moved to while hidden", &mut live, &w);
    assert!(apart(&hidden, &shown) > 0.5 && apart(&before, &shown) > 0.5, "it is in the picture, and not where it was ({:.2}, {:.2} steps)", apart(&hidden, &shown), apart(&before, &shown));
    // The one that is not movable, hidden and shown where it stands.
    w.set_visible(32, false).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the other hidden: in place ({:?})", live.fallback);
    w.set_visible(32, true).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the other shown: in place ({:?})", live.fallback);
    let again = against_a_build("a posed rigid model hidden and shown where it stands", &mut live, &w);
    assert!(apart(&shown, &again) < 0.5, "as before it was hidden ({:.2} steps)", apart(&shown, &again));
    // Removed: its instances are no object's, its emitters at no weight.
    w.remove(33);
    assert!(!live.commit(&mut w).unwrap(), "a posed rigid model removed: in place ({:?})", live.fallback);
    assert!(live.sc.tables.retained.rigid(33).is_none() && seen(&live, &b) == vec![(true, false); 2] && lit(&live, &b) == 0, "its parts are no object's");
    let removed = against_a_build("a posed rigid model removed", &mut live, &w);
    assert!(apart(&again, &removed) > 0.5, "it is gone from the picture ({:.2} steps)", apart(&again, &removed));
    assert_eq!(rays(&live, &b), vec![false; 2], "no ray sees what was its parts");
    // The other posed on after: its own parts alone.
    w.set_animation(32, slide(0.6)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the other posed on: in place ({:?})", live.fallback);
    against_a_build("the other model posed on after the removal", &mut live, &w);
    // And removed, never having moved: the part its clip leaves alone goes with the node.
    w.remove(32);
    assert!(!live.commit(&mut w).unwrap(), "the one that never moved removed: in place ({:?})", live.fallback);
    assert_eq!((rays(&live, &a), lit(&live, &a)), (vec![false; 2], 0), "no ray sees what was its parts, its emitters weigh nothing");
    against_a_build("a posed rigid model that never moved removed", &mut live, &w);
}

/// A model the scene has is placed again in place, with its emitters: a still one (tests/render/data/panel.glb), and
/// a posed rigid one (panel-slider.glb), a part a record of the room in a run, each an instance of the mesh the
/// build's placement has. The one placed is then posed, moved, hidden, shown and removed as one a build placed; its
/// records and rows are the room's again, and the next one placed takes them. Each time the light list is its
/// triangles and the picture a build's.
#[test]
fn a_model_with_emitters_is_placed_again_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let slide = |time| Some(crate::desc::Animation { clip: "slide".into(), time, r#loop: true, nodes: Default::default() });
    let still = || Kind::Model { uri: "tests/render/data/panel.glb".into(), nodes: None, animation: None, functions: vec![] };
    let posed = |time| Kind::Model { uri: "tests/render/data/panel-slider.glb".into(), nodes: None, animation: slide(time), functions: vec![] };
    let mut w = world();
    w.set_object(30, "still".into(), None, still(), true, false).unwrap();
    w.set_transform(30, Vec3::new(-1.4, 0.7, 0.4), glam::Quat::from_rotation_y(0.4), Vec3::splat(0.05)).unwrap();
    w.set_object(32, "posed".into(), None, posed(0.25), true, false).unwrap();
    w.set_transform(32, Vec3::new(-1.3, -0.6, 0.5), glam::Quat::IDENTITY, Vec3::splat(0.08)).unwrap();
    // (A mesh the scene says it edits: room for what is placed.)
    w.set_object(28, "own".into(), None, Kind::Mesh { geometry: 3, material: 1, own_blas: true, materials: vec![] }, true, false).unwrap();
    w.set_transform(28, Vec3::new(1.4, -0.7, 0.4), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let pictured = |what: &str, live: &mut LiveCore, w: &World| {
        list_is_its_triangles(what, live);
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        a
    };
    let lit = |c: &LiveCore, of: &[u32]| c.sc.tables.emitter_tris.iter().filter(|e| of.contains(&e.inst) && crate::emitter_power(e, &c.sc.tables.materials, &c.sc.tables.objects, c.sc.tables.stride) > 0.0).count();
    let before = pictured("as loaded", &mut live, &w);
    // The still model again.
    w.set_object(31, "still again".into(), None, still(), true, false).unwrap();
    w.set_transform(31, Vec3::new(1.2, 0.7, 0.5), glam::Quat::from_rotation_y(-0.3), Vec3::splat(0.05)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "a still model with emitters placed again: in place ({:?})", live.fallback);
    let (first, again) = (*live.sc.tables.retained.instance(30).unwrap(), *live.sc.tables.retained.instance(31).expect("it is an instance"));
    assert_eq!((live.sc.instances[again.instance as usize].mesh, lit(&live, &[again.instance])), (live.sc.instances[first.instance as usize].mesh, 2), "an instance of the model's mesh, its two emitters its own");
    list_is_its_triangles("a still model with emitters placed again", &live);
    lit_as_a_build("a still model with emitters placed again", &mut live, &mut open(&w).unwrap());
    // The posed rigid model again, elsewhere in its clip.
    let place = |w: &mut World, id: u32, time: f32, at: Vec3| {
        w.set_object(id, format!("posed{id}"), None, posed(time), true, false).unwrap();
        w.set_transform(id, at, glam::Quat::from_rotation_z(0.3), Vec3::splat(0.08)).unwrap();
    };
    // (One whose pose names a node moves other nodes: other parts, a build's to make.)
    let named = crate::desc::Animation { clip: "slide".into(), time: 0.75, r#loop: true, nodes: [("panel".to_string(), vec![0.0, 0.0, 0.0, 1.0])].into() };
    w.set_object(35, "named".into(), None, Kind::Model { uri: "tests/render/data/panel-slider.glb".into(), nodes: None, animation: Some(named), functions: vec![] }, true, false).unwrap();
    w.set_transform(35, Vec3::new(0.2, 0.3, 0.8), glam::Quat::from_rotation_z(0.3), Vec3::splat(0.08)).unwrap();
    assert!(live.commit(&mut w).unwrap(), "a model whose pose names a node the clip leaves alone: a rebuild");
    w.remove(35);
    live.commit(&mut w).unwrap();
    place(&mut w, 33, 0.75, Vec3::new(0.2, 0.3, 0.8));
    assert!(!live.commit(&mut w).unwrap(), "a posed rigid model placed again: in place ({:?})", live.fallback);
    let (built, r) = (live.sc.tables.retained.rigid(32).unwrap().clone(), live.sc.tables.retained.rigid(33).expect("a posed rigid model").clone());
    let (records, mesh) = (r.records(), |c: &LiveCore, i: u32| c.sc.instances[i as usize].mesh);
    assert_eq!((r.instances.len(), mesh(&live, r.still.unwrap()), mesh(&live, r.instances[0].1)), (1, mesh(&live, built.still.unwrap()), mesh(&live, built.instances[0].1)), "its parts are instances of the meshes the build's placement has");
    let run: Vec<u32> = (0..2).map(|j| records.iter().min().unwrap() + j).collect();
    assert!(records.iter().all(|&i| live.sc.edit.in_room(i as usize) && run.contains(&i)), "a part a record of the room, in a run ({records:?})");
    assert_eq!((lit(&live, &[r.still.unwrap()]), r.moving()), (2, vec![r.instances[0].1]), "its panel's emitters are its own, still ones");
    assert!(live.sc.tables.emitter_tris.iter().filter(|e| e.inst == r.still.unwrap()).all(|e| !e.moves), "they are no moving emitters");
    list_is_its_triangles("a posed rigid model placed again", &live);
    let mut fresh = open(&w).unwrap();
    // (Each part where a build places it: its node, and where its instance stands.)
    let parts = |c: &LiveCore| {
        let r = c.sc.tables.retained.rigid(33).unwrap();
        let mut of: Vec<(usize, Vec<i64>)> = r.instances.iter().map(|&(n, i)| (n, i)).chain(r.still.map(|i| (scene::STILL_NODES, i))).map(|(n, i)| (n, c.sc.instances[i as usize].world.to_cols_array().iter().map(|x| (x * 1e4).round() as i64).collect())).collect();
        of.sort();
        of
    };
    assert_eq!(parts(&live), parts(&fresh), "each part stands where a build places it");
    assert!(records.iter().all(|&i| seen_by_rays(&live, i)), "the rays see its parts");
    lit_as_a_build("a posed rigid model placed again", &mut live, &mut fresh);
    let placed = pictured("a posed rigid model placed again", &mut live, &w);
    assert!(apart(&before, &placed) > 0.5, "the models placed are in the picture ({:.2} steps)", apart(&before, &placed));
    // Posed on, moved, hidden and shown, as one the build placed.
    w.set_animation(33, slide(0.4)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the one placed posed on: in place ({:?})", live.fallback);
    assert!(!live.sc.tables.retained.rigid(33).unwrap().still_moves, "a new pose is no move: its panel stays, its emitters still ones");
    pictured("the one placed posed on", &mut live, &w);
    w.set_transform(33, Vec3::new(-0.3, 0.1, 1.0), glam::Quat::from_rotation_z(-0.4), Vec3::splat(0.08)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the one placed moved: in place ({:?})", live.fallback);
    // (Every model's row of the object table as a build's: the moved one's pivot in its own row, no other's touched.
    // Its world bounds' size apart: a move in place keeps the size the object was placed with, for every kind.)
    let object_rows = |c: &LiveCore| {
        let st = c.sc.tables.stride;
        [30u32, 31, 32, 33].map(|id| {
            c.sc.tables.retained.proxies[&id].object.map(|o| {
                let mut row = c.sc.tables.objects[o as usize * st..(o as usize + 1) * st].to_vec();
                row[1] = [0.0; 4];
                row
            })
        })
    };
    assert_eq!(object_rows(&live), object_rows(&open(&w).unwrap()), "the one placed moved: each model's row of the object table, as a build's");
    let moved = pictured("the one placed moved", &mut live, &w);
    assert!(apart(&placed, &moved) > 0.5, "the move is in the picture ({:.2} steps)", apart(&placed, &moved));
    w.set_visible(33, false).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the one placed hidden: in place ({:?})", live.fallback);
    pictured("the one placed hidden", &mut live, &w);
    w.set_visible(33, true).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the one placed shown: in place ({:?})", live.fallback);
    let shown = pictured("the one placed shown", &mut live, &w);
    assert!(apart(&moved, &shown) < 0.5, "as before it was hidden ({:.2} steps)", apart(&moved, &shown));
    // Removed: its records and rows are the room's again, and the next one placed takes the records.
    let rows: Vec<(u32, u32)> = records.iter().map(|&i| (live.sc.instances[i as usize].rows, live.sc.meshes[live.sc.instances[i as usize].mesh as usize].sect_count)).collect();
    w.remove(33);
    assert!(!live.commit(&mut w).unwrap(), "the one placed removed: in place ({:?})", live.fallback);
    assert!(rows.iter().all(|&(at, n)| rows_free(&live, at, n)) && lit(&live, &records) == 0, "its rows are the room's again, its emitters weigh nothing");
    pictured("the one placed removed", &mut live, &w);
    // (Made visible in the commit that adds it: placed as it is, by the edit that places it.)
    place(&mut w, 34, 0.5, Vec3::new(0.4, -0.2, 0.9));
    w.set_visible(34, false).unwrap();
    w.set_visible(34, true).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "another placed after: in place ({:?})", live.fallback);
    let mut next = live.sc.tables.retained.rigid(34).unwrap().records();
    next.sort_unstable();
    assert_eq!(next, run, "it takes the records the first gave back");
    list_is_its_triangles("another posed rigid model placed after one was removed", &live);
    lit_as_a_build("another posed rigid model placed after one was removed", &mut live, &mut open(&w).unwrap());
}

/// A posed rigid model with triangles flattened beside its parts (tests/render/data/panel-decal.glb: a cutout on
/// exactly an emitter's vertices leaves the mesh with it) is hidden, and removed, by a rebuild: an instance's mask
/// hides no world-space triangle. The picture is a build's.
#[test]
fn a_posed_rigid_model_with_flattened_triangles_is_hidden_and_removed_by_a_rebuild() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let model = || Kind::Model { uri: "tests/render/data/panel-decal.glb".into(), nodes: None, animation: Some(crate::desc::Animation { clip: "slide".into(), time: 0.25, r#loop: true, nodes: Default::default() }), functions: vec![] };
    let scene = || {
        let mut w = world();
        w.set_object(32, "posed".into(), None, model(), true, false).unwrap();
        w.set_transform(32, Vec3::new(-0.4, 0.3, 0.6), glam::Quat::IDENTITY, Vec3::splat(0.08)).unwrap();
        w
    };
    let pictured = |what: &str, live: &mut LiveCore, w: &World| {
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        a
    };
    let mut w = scene();
    let mut live = match open(&w) {
        Ok(c) if c.g.hwrt => c,
        Ok(_) => return eprintln!("skipped: no hardware RT"),
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    assert!(!live.sc.tables.retained.rigid(32).expect("a posed rigid model").rest.mats.is_empty(), "the model has triangles flattened beside its part");
    let before = picture(&mut live, 48);
    w.set_visible(32, false).unwrap();
    assert!(live.commit(&mut w).unwrap(), "hidden: a rebuild");
    let hidden = pictured("a posed rigid model with flattened triangles hidden", &mut live, &w);
    assert!(apart(&before, &hidden) > 0.5, "it is gone from the picture ({:.2} steps)", apart(&before, &hidden));
    let mut w = scene();
    let mut live = open(&w).unwrap();
    w.take_changes();
    w.remove(32);
    assert!(live.commit(&mut w).unwrap(), "removed: a rebuild");
    let removed = pictured("a posed rigid model with flattened triangles removed", &mut live, &w);
    assert!(apart(&hidden, &removed) < 0.5, "as when it was hidden ({:.2} steps)", apart(&hidden, &removed));
    // Placed again: what is flattened beside its part is a build's to make.
    let mut w = scene();
    // (A mesh the scene says it edits: room for what is placed.)
    w.set_object(28, "own".into(), None, Kind::Mesh { geometry: 3, material: 1, own_blas: true, materials: vec![] }, true, false).unwrap();
    w.set_transform(28, Vec3::new(1.4, -0.7, 0.4), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = open(&w).unwrap();
    w.take_changes();
    let one = picture(&mut live, 48);
    w.set_object(33, "again".into(), None, model(), true, false).unwrap();
    w.set_transform(33, Vec3::new(0.6, -0.4, 0.8), glam::Quat::IDENTITY, Vec3::splat(0.08)).unwrap();
    assert!(live.commit(&mut w).unwrap(), "placed again: a rebuild");
    let two = pictured("a posed rigid model with flattened triangles placed again", &mut live, &w);
    assert!(apart(&one, &two) > 0.5, "the second is in the picture ({:.2} steps)", apart(&one, &two));
}

/// Posed skinned models are each a mesh of their own, whatever their poses (tests/render/data/arm.glb: a skinned strip,
/// a dark half and a light half that a joint swings): two in the same pose are two meshes, posed again in place with
/// each triangle still in its section, and one of them given another pose alone is in place too (a pose two shared
/// was one mesh, and that was a rebuild). The picture is a build's.
#[test]
fn posed_skinned_models_are_meshes_of_their_own() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let bend = |time| Some(crate::desc::Animation { clip: "bend".into(), time, r#loop: true, nodes: Default::default() });
    let mut w = world();
    for (id, x) in [(30, -0.7), (31, 0.7)] {
        w.set_object(id, format!("arm{id}"), None, Kind::Model { uri: "tests/render/data/arm.glb".into(), nodes: None, animation: bend(0.0), functions: vec![] }, true, false).unwrap();
        w.set_transform(id, Vec3::new(x, -1.0, 0.8), glam::Quat::IDENTITY, Vec3::splat(2.0)).unwrap();
    }
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let of = |c: &LiveCore, id: u32| c.sc.tables.retained.skinned.iter().find(|s| s.object == id).expect("a posed skinned model").clone();
    let (a, b) = (of(&live, 30), of(&live, 31));
    assert_eq!((live.sc.tables.retained.skinned.len(), a.mesh != b.mesh, a.source == b.source, live.sc.meshes[a.mesh as usize].sect_count), (2, true, true, 2), "two meshes of two sections, skinned from the model's one copy of its corners");
    assert!(live.sc.up.kernel.features & gpu::F_DEFORM != 0, "the kernel finds a deforming mesh's previous vertices by its instance");
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 64), picture(&mut fresh, 64));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        a
    };
    let loaded = against_a_build("as loaded", &mut live, &w);
    for id in [30, 31] {
        w.set_animation(id, bend(0.75)).unwrap();
    }
    assert!(!live.commit(&mut w).unwrap(), "both posed again: in place ({:?})", live.fallback);
    let both = against_a_build("both posed again", &mut live, &w);
    assert!(apart(&loaded, &both) > 0.5, "the pose is in the picture ({:.2} steps)", apart(&loaded, &both));
    // One alone, and the other's mesh keeps its pose.
    w.set_animation(31, bend(0.2)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "one of two in the same pose posed alone: in place ({:?})", live.fallback);
    assert_eq!((of(&live, 30).animation.time, of(&live, 31).animation.time), (0.75, 0.2));
    let one = against_a_build("one posed alone", &mut live, &w);
    assert!(apart(&both, &one) > 0.5, "its pose is in the picture ({:.2} steps)", apart(&both, &one));
}

/// A posed skinned model is hidden and shown in place: hidden, no ray sees its instance and a new pose skins
/// nothing; shown, it is skinned as it is posed then, where it stands. The picture is a build's each time, under
/// either traversal.
#[test]
fn a_posed_skinned_model_is_hidden_and_shown_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let bend = |time| Some(crate::desc::Animation { clip: "bend".into(), time, r#loop: true, nodes: Default::default() });
    let mut w = world();
    for (id, x) in [(30, -0.7), (31, 0.7)] {
        w.set_object(id, format!("arm{id}"), None, Kind::Model { uri: "tests/render/data/arm.glb".into(), nodes: None, animation: bend(0.0), functions: vec![] }, true, true).unwrap();
        w.set_transform(id, Vec3::new(x, -1.0, 0.8), glam::Quat::IDENTITY, Vec3::splat(2.0)).unwrap();
    }
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let of = |c: &LiveCore, id: u32| c.sc.tables.retained.skinned.iter().find(|s| s.object == id).expect("a posed skinned model").clone();
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 64), picture(&mut fresh, 64));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        a
    };
    let loaded = against_a_build("as loaded", &mut live, &w);
    let record = of(&live, 31).instance;
    w.set_visible(31, false).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "hidden: in place ({:?})", live.fallback);
    assert!(!seen_by_rays(&live, record) && seen_by_rays(&live, of(&live, 30).instance), "no ray sees the hidden one; the other is seen");
    let hidden = against_a_build("hidden", &mut live, &w);
    assert!(apart(&loaded, &hidden) > 0.5, "it is out of the picture ({:.2} steps)", apart(&loaded, &hidden));
    // Posed and moved while hidden: nothing is skinned; the other is posed as ever.
    w.set_animation(31, bend(0.8)).unwrap();
    w.set_animation(30, bend(0.5)).unwrap();
    w.set_transform(31, Vec3::new(0.5, -0.8, 0.9), glam::Quat::from_rotation_y(0.4), Vec3::splat(2.0)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "posed and moved while hidden: in place ({:?})", live.fallback);
    assert_eq!((of(&live, 31).animation.time, of(&live, 30).animation.time), (0.0, 0.5), "the hidden one's mesh keeps the pose it had; the other's has the new one");
    against_a_build("posed and moved while hidden", &mut live, &w);
    // Shown: skinned as it is posed now, where it stands now.
    w.set_visible(31, true).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "shown: in place ({:?})", live.fallback);
    assert!(seen_by_rays(&live, record), "the rays see it again");
    assert_eq!(of(&live, 31).animation.time, 0.8, "its mesh has the pose it was given while hidden");
    let shown = against_a_build("shown", &mut live, &w);
    assert!(apart(&hidden, &shown) > 0.5, "it is in the picture again ({:.2} steps)", apart(&hidden, &shown));
    // Hidden and shown again with nothing between: as it was.
    w.set_visible(31, false).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "hidden again: in place ({:?})", live.fallback);
    w.set_visible(31, true).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "shown again: in place ({:?})", live.fallback);
    let again = against_a_build("shown again", &mut live, &w);
    assert!(apart(&shown, &again) < 0.5, "as before it was hidden ({:.2} steps)", apart(&shown, &again));
}

/// A posed skinned model of a model the scene has is added in place: a mesh of its own in the room, skinned as the
/// build skins one, its previous vertices its own, a job of the pass. It is then posed, moved, hidden, shown and
/// removed as one a build placed, and the next one added takes what it had. Many added at once make every room grow.
/// Its motion (the kernel finds a deforming mesh's previous vertices through its instance) is a built one's. One of a
/// model the scene has not, or in a pose that mirrors otherwise, is a rebuild's; so is any under software traversal.
#[test]
fn a_posed_skinned_model_is_added_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let pose = |clip: &str, time| crate::desc::Animation { clip: clip.into(), time, r#loop: true, nodes: Default::default() };
    let bend = |time| pose("bend", time);
    let file = |name: &str| format!("tests/render/data/{name}.glb");
    let model = |name: &str, a: crate::desc::Animation| Kind::Model { uri: file(name), nodes: None, animation: Some(a), functions: vec![] };
    let mut w = world();
    for (id, name, x) in [(30, "arm", -1.2), (31, "skin-blend", 1.3)] {
        w.set_object(id, format!("m{id}"), None, model(name, bend(0.0)), true, false).unwrap();
        w.set_transform(id, Vec3::new(x, -0.8, 0.8), glam::Quat::IDENTITY, Vec3::splat(1.2)).unwrap();
    }
    // (A movable rect light's card: world-space triangles that move, whose previous vertices lie before the meshes'.)
    let card = serde_json::from_value(serde_json::json!({ "type": "rect", "color": [1, 1, 1], "castShadow": true, "intensity": 1, "width": 0.3, "height": 0.3, "range": 0 })).unwrap();
    w.set_object(22, "card".into(), None, Kind::Light(card), true, true).unwrap();
    w.set_transform(22, Vec3::new(-2.4, 1.3, 0.6), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    // (The build's frames counted from the live view's: a frame's samples go by its number, and a picture of many
    // thin models is two thirds of a step from itself at another count, more than a model that is not drawn.)
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        let mut fresh = open(w).unwrap();
        fresh.frame = live.frame;
        forget(live);
        let (a, b) = (picture(live, 64), picture(&mut fresh, 64));
        assert!(apart(&a, &b) < 0.35, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        a
    };
    let loaded = against_a_build("as loaded", &mut live, &w);
    let add = |w: &mut World, id: u32, name: &str, a: crate::desc::Animation, at: Vec3| {
        w.set_object(id, format!("m{id}"), None, model(name, a), true, false).unwrap();
        w.set_transform(id, at, glam::Quat::from_rotation_y(0.3), Vec3::splat(1.2)).unwrap();
    };
    if !live.g.hwrt {
        // (With a mesh the scene says it edits, so that there is room for triangles: the model is still a rebuild's.)
        w.set_object(28, "own".into(), None, Kind::Mesh { geometry: 3, material: 1, own_blas: true, materials: vec![] }, true, false).unwrap();
        w.set_transform(28, Vec3::new(1.4, -0.7, 0.4), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        live = open(&w).unwrap();
        w.take_changes();
        assert!(live.sc.edit.tris.cap > live.sc.edit.tris.end + 8, "the room has triangles to give");
        let loaded = against_a_build("as loaded, with a mesh to edit", &mut live, &w);
        add(&mut w, 32, "arm", bend(0.3), Vec3::new(-0.2, -0.8, 0.9));
        assert!(live.commit(&mut w).unwrap(), "added under software traversal: a rebuild");
        let added = against_a_build("added under software traversal", &mut live, &w);
        assert!(apart(&loaded, &added) > 0.5, "it is in the picture ({:.2} steps)", apart(&loaded, &added));
        return;
    }
    let read = |c: &LiveCore, b: &wgpu::Buffer, at: u64, bytes: u64| -> Vec<f32> { read_back(c, b, at, bytes) };
    let of = |c: &LiveCore, id: u32| c.sc.tables.retained.skinned.iter().position(|s| s.object == id && s.mesh != u32::MAX).expect("a posed skinned model");
    let cpu = |c: &LiveCore, id: u32, a: &crate::desc::Animation| skinned_on_cpu(c, &c.sc.tables.retained.skinned[of(c, id)], a).p;
    // A model's mesh on the GPU against the CPU skin of `now`; its previous positions against that of `before`.
    let check = |what: &str, c: &LiveCore, id: u32, now: &crate::desc::Animation, before: &crate::desc::Animation| {
        skinned_is_the_cpus(what, c, &c.sc.tables.retained.skinned[of(c, id)], &format!("m{id}"), now, before);
    };
    let built = live.sc.tables.retained.skinned.len();
    let prev_size = live.sc.up.prev_vertices.size();
    {
        let i = &live.sc.instances[live.sc.tables.retained.skinned[of(&live, 31)].instance as usize];
        assert!(live.sc.dynamic.1.len() == 6 && i.prev == i.verts - live.sc.dynamic_verts.0 && i.prev >= 6, "a mesh's previous positions lie after the card's six vertices'");
    }

    // One of a model the scene has, in a pose of its own.
    add(&mut w, 32, "arm", bend(0.3), Vec3::new(-0.2, -0.8, 0.9));
    assert!(!live.commit(&mut w).unwrap(), "added: in place ({:?})", live.fallback);
    let (g, t) = (of(&live, 32), of(&live, 30));
    let (sk, its) = (live.sc.tables.retained.skinned[g].clone(), live.sc.tables.retained.skinned[t].clone());
    assert_eq!((g, sk.source, sk.mesh != its.mesh, live.sc.meshes[sk.mesh as usize].deforms), (built, its.source, true, true), "a mesh of its own that deforms, skinned from the model's one copy of its corners, the next of the skinned");
    assert!(live.sc.edit.in_room(sk.instance as usize) && live.sc.instances[sk.instance as usize].prev as usize >= live.sc.edit.prev.base && seen_by_rays(&live, sk.instance), "its record and its previous vertices are the room's, and the rays see it");
    assert!(live.sc.up.prev_vertices.size() > prev_size, "the previous vertices' buffer was made again, larger");
    check("added", &live, 32, &bend(0.3), &bend(0.3));
    check("the built one beside it", &live, 30, &bend(0.0), &bend(0.0));
    let added = against_a_build("added", &mut live, &w);
    assert!(apart(&loaded, &added) > 0.5, "it is in the picture ({:.2} steps)", apart(&loaded, &added));

    // Its motion at a new pose is a built one's: the same world built, both posed on in place, a frame of each.
    let mut w2 = w.clone();
    let mut fresh = open(&w2).unwrap();
    w2.take_changes();
    for id in [30, 31, 32] {
        w.set_animation(id, Some(bend(0.8))).unwrap();
        w2.set_animation(id, Some(bend(0.8))).unwrap();
    }
    assert!(!live.commit(&mut w).unwrap() && !fresh.commit(&mut w2).unwrap(), "posed on: in place ({:?}, {:?})", live.fallback, fresh.fallback);
    check("posed on", &live, 32, &bend(0.8), &bend(0.3));
    check("the built one posed on", &live, 30, &bend(0.8), &bend(0.0));
    check("the other built one posed on", &live, 31, &bend(0.8), &bend(0.0));
    // (The card did not move: its previous vertices are its own, whatever the meshes' did.)
    let card = 6 * crate::indexed::VERTEX_BYTES as u64;
    assert_eq!(read(&live, &live.sc.up.prev_vertices, 0, card), read(&live, &live.sc.up.vertices, live.sc.dynamic_verts.0 as u64 * crate::indexed::VERTEX_BYTES as u64, card), "the card's previous vertices are its vertices");
    let motion = |c: &mut LiveCore| -> Vec<f32> {
        (c.fixed_dt, c.frame, c.reset) = (Some(1.0 / 60.0), 16, true);
        c.render(1, |_, _| {});
        let m = &c.targets.kernel.motion;
        read(c, &m.buffer, m.offset, m.size)
    };
    let (a, b) = (motion(&mut live), motion(&mut fresh));
    let moving = |m: &[f32]| m.chunks_exact(4).filter(|p| p[3] > 0.5 && (p[0].abs() + p[1].abs() + p[2].abs()) > 1e-3).count();
    assert!(moving(&a) > 20 && moving(&a) == moving(&b), "pixels that moved: {} here, {} in a build", moving(&a), moving(&b));
    let off = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
    assert!(off < 1e-3, "the motion vectors are a build's (off by {off} at most)");
    // No vector is longer than the furthest a corner of a model went (a kernel that takes another triangle's previous
    // vertices gives vectors as long as the mesh; another mesh's, anything), and some are nearly as long.
    let reach = |c: &LiveCore, id: u32, before: &crate::desc::Animation| -> f32 {
        let world = c.sc.instances[c.sc.tables.retained.skinned[of(c, id)].instance as usize].world;
        let corners = |v: &[f32]| -> Vec<Vec3> { v.chunks_exact(3).map(Vec3::from_slice).collect() };
        corners(&cpu(c, id, &bend(0.8))).iter().zip(corners(&cpu(c, id, before))).map(|(x, y)| world.transform_vector3(*x - y).length()).fold(0.0, f32::max)
    };
    let far = [(30, bend(0.0)), (31, bend(0.0)), (32, bend(0.3))].iter().map(|(id, before)| reach(&live, *id, before)).fold(0.0f32, f32::max);
    let longest = a.chunks_exact(4).filter(|p| p[3] > 0.5).map(|p| Vec3::new(p[0], p[1], p[2]).length()).fold(0.0f32, f32::max);
    assert!(longest <= far * 1.001 + 1e-4 && longest > far * 0.2, "the longest motion vector is {longest}; the furthest a corner went, {far}");
    drop(fresh);

    // Moved, hidden and shown, as one a build placed.
    w.set_transform(32, Vec3::new(0.2, -0.7, 1.0), glam::Quat::from_rotation_y(-0.4), Vec3::splat(1.2)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the one added moved: in place ({:?})", live.fallback);
    assert_eq!(live.sc.instances[sk.instance as usize].world, w.world_matrix(32), "its instance stands where it was moved to");
    let moved = against_a_build("the one added moved", &mut live, &w);
    w.set_visible(32, false).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the one added hidden: in place ({:?})", live.fallback);
    assert!(!seen_by_rays(&live, sk.instance), "no ray sees it");
    against_a_build("the one added hidden", &mut live, &w);
    w.set_visible(32, true).unwrap();
    w.set_animation(32, Some(bend(0.5))).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the one added shown and posed: in place ({:?})", live.fallback);
    check("shown and posed", &live, 32, &bend(0.5), &bend(0.8));
    let shown = against_a_build("the one added shown", &mut live, &w);
    assert!(apart(&moved, &shown) > 0.2, "its new pose is in the picture ({:.2} steps)", apart(&moved, &shown));

    // Many at once, of both models (more triangles than the room has): every room grows again, and each is skinned as
    // the build skins it.
    let rooms_size = |c: &LiveCore| (c.sc.up.vertices.size(), c.sc.up.words.cap(crate::heap::Stream::Indices));
    let (size, ids) = (rooms_size(&live), (100..140).collect::<Vec<u32>>());
    for (k, &id) in ids.iter().enumerate() {
        add(&mut w, id, if k % 2 == 0 { "skin-blend" } else { "arm" }, bend(0.1 + 0.02 * k as f32), Vec3::new(-1.5 + 0.075 * k as f32, 0.3, 0.6));
    }
    assert!(!live.commit(&mut w).unwrap(), "40 added at once: in place ({:?})", live.fallback);
    assert!(rooms_size(&live).0 > size.0 && rooms_size(&live).1 > size.1, "the vertices' buffer and the triangles' indices were made again, larger ({size:?} -> {:?})", rooms_size(&live));
    for (k, &id) in ids.iter().enumerate() {
        let a = bend(0.1 + 0.02 * k as f32);
        check(&format!("one of 40 added ({id})"), &live, id, &a, &a);
    }
    // (Frames were drawn since its last pose: its previous vertices are that pose's too.)
    check("the first added, after the rooms grew", &live, 32, &bend(0.5), &bend(0.5));
    against_a_build("40 added at once", &mut live, &w);
    // Every other one hidden, and shown again.
    for &id in ids.iter().step_by(2) {
        w.set_visible(id, false).unwrap();
    }
    assert!(!live.commit(&mut w).unwrap(), "20 of them hidden: in place ({:?})", live.fallback);
    against_a_build("20 of them hidden", &mut live, &w);
    for &id in ids.iter().step_by(2) {
        w.set_visible(id, true).unwrap();
    }
    assert!(!live.commit(&mut w).unwrap(), "shown again: in place ({:?})", live.fallback);
    // (A model shown has its power back, and what a power edit touched keeps a short history for half a second of the
    // scene's clock, which a game's frames advance and a test's do not: past it.)
    for _ in 0..6 {
        live.advance_time(0.1);
    }
    against_a_build("the 20 shown again", &mut live, &w);
    for &id in &ids {
        w.set_animation(id, Some(bend(0.9))).unwrap();
    }
    assert!(!live.commit(&mut w).unwrap(), "the 40 posed on: in place ({:?})", live.fallback);
    for (k, &id) in ids.iter().enumerate() {
        check(&format!("one of 40 posed on ({id})"), &live, id, &bend(0.9), &bend(0.1 + 0.02 * k as f32));
    }
    against_a_build("the 40 posed on", &mut live, &w);

    // Removed: its slot among the skinned, its record, its triangles and its previous vertices are the room's again,
    // and the next one added takes them.
    let (mesh, prev, tri, vertices) = (sk.mesh, live.sc.instances[sk.instance as usize].prev, live.sc.gpu_instances[sk.instance as usize].info[0], live.sc.meshes[sk.mesh as usize].vcount);
    let first_vertex = live.sc.instances[sk.instance as usize].verts;
    w.remove(32);
    assert!(!live.commit(&mut w).unwrap(), "the one added removed: in place ({:?})", live.fallback);
    assert!(live.sc.tables.retained.skinned[g].mesh == u32::MAX && live.sc.edit.free_skinned == vec![g as u32] && live.sc.edit.dead_meshes.contains(&mesh) && !seen_by_rays(&live, sk.instance), "its slot and its mesh are free, no ray sees it");
    assert!(live.sc.edit.prev.free.iter().any(|(&at, &n)| at <= prev as usize && prev as usize + vertices as usize <= at + n), "its previous vertices are the room's again");
    against_a_build("the one added removed", &mut live, &w);
    add(&mut w, 34, "arm", bend(0.6), Vec3::new(0.0, -0.8, 0.9));
    assert!(!live.commit(&mut w).unwrap(), "the next added: in place ({:?})", live.fallback);
    let next = live.sc.tables.retained.skinned[of(&live, 34)].clone();
    assert_eq!((of(&live, 34), next.mesh, next.instance, live.sc.instances[next.instance as usize].prev, live.sc.gpu_instances[next.instance as usize].info[0]), (g, mesh, sk.instance, prev, tri), "it has the slot, the mesh, the record, the previous vertices and the triangles of the one removed");
    assert_eq!(live.sc.instances[next.instance as usize].verts, first_vertex, "and its vertices");
    check("the next added", &live, 34, &bend(0.6), &bend(0.6));
    w.set_animation(34, Some(bend(0.2))).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the next added posed on: in place ({:?})", live.fallback);
    check("the next added posed on", &live, 34, &bend(0.2), &bend(0.6));
    against_a_build("the next added posed on", &mut live, &w);

    // One the build placed removed, and another of its model added: the build's mesh stays, the model is still the scene's.
    w.remove(30);
    add(&mut w, 35, "arm", bend(0.4), Vec3::new(-1.2, -0.8, 0.8));
    assert!(!live.commit(&mut w).unwrap(), "a built one removed and another of its model added: in place ({:?})", live.fallback);
    check("another of a removed one's model", &live, 35, &bend(0.4), &bend(0.4));
    against_a_build("a built one removed and another added", &mut live, &w);

    // More of them, in one commit that makes no room grow but the skinning pass's own: the pass reads its new table.
    let (grows, rooms, more) = (live.sc.edit.grows, live.sc.up.skin.as_ref().unwrap().rooms(), (200..224).collect::<Vec<u32>>());
    for (k, &id) in more.iter().enumerate() {
        add(&mut w, id, "arm", bend(0.33), Vec3::new(-2.8 + 0.1 * k as f32, -1.6, 0.9));
    }
    assert!(!live.commit(&mut w).unwrap(), "24 more added: in place ({:?})", live.fallback);
    assert!(live.sc.edit.grows == grows && live.sc.up.skin.as_ref().unwrap().rooms() != rooms, "no room grew but the pass's ({:?} -> {:?})", rooms, live.sc.up.skin.as_ref().unwrap().rooms());
    for &id in &more {
        w.set_animation(id, Some(bend(0.7))).unwrap();
    }
    assert!(!live.commit(&mut w).unwrap(), "the 24 posed on: in place ({:?})", live.fallback);
    for &id in &more {
        check(&format!("one of 24 more posed on ({id})"), &live, id, &bend(0.7), &bend(0.33));
    }
    for &id in &more {
        w.remove(id);
    }
    assert!(!live.commit(&mut w).unwrap(), "the 24 removed: in place ({:?})", live.fallback);
    against_a_build("24 more added, posed and removed", &mut live, &w);

    // Previous vertices kept mesh by mesh. Two more, one after the other in the room; the first posed and, before a
    // frame is drawn, removed, a mesh of two triangles and a third model taking what it had: the third's previous
    // vertices are where the first's were, its triangles are not.
    let place = |c: &LiveCore, id: u32| {
        let i = c.sc.tables.retained.instance(id).unwrap().instance as usize;
        (c.sc.gpu_instances[i].info[0], c.sc.instances[i].prev)
    };
    // (A model's vertices: fewer than three a triangle.)
    let nv = live.sc.meshes[live.sc.tables.retained.skinned[of(&live, 35)].mesh as usize].vcount;
    assert!(nv > 0 && nv < 12, "the model's four triangles share vertices ({nv})");
    add(&mut w, 50, "arm", bend(0.1), Vec3::new(-2.6, -0.2, 0.7));
    add(&mut w, 51, "arm", bend(0.1), Vec3::new(-2.1, -0.2, 0.7));
    assert!(!live.commit(&mut w).unwrap(), "two more: in place ({:?})", live.fallback);
    let ((ta, pa), (tb, pb)) = (place(&live, 50), place(&live, 51));
    assert_eq!((tb, pb), (ta + 4, pa + nv), "the two lie one after the other");
    picture(&mut live, 2);
    w.set_animation(50, Some(bend(0.5))).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the first posed: in place ({:?})", live.fallback);
    w.remove(50);
    w.set_geometry(8, Geometry { positions: vec![-0.1, -0.1, 0.0, 0.1, -0.1, 0.0, 0.1, 0.1, 0.0, -0.1, 0.1, 0.0], normals: [0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] }).unwrap();
    w.set_object(49, "two".into(), None, Kind::Mesh { geometry: 8, material: 1, own_blas: false, materials: vec![] }, true, false).unwrap();
    w.set_transform(49, Vec3::new(-2.6, 0.6, 0.7), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    add(&mut w, 52, "arm", bend(0.2), Vec3::new(-2.6, -0.2, 0.7));
    assert!(!live.commit(&mut w).unwrap(), "the first removed, a mesh and a third added: in place ({:?})", live.fallback);
    assert_eq!(place(&live, 52), (tb + 4, pa), "the third: its triangles after the second's, its previous vertices where the first's were");
    picture(&mut live, 4);
    check("the third, frames on (the first's pose is no one's to keep)", &live, 52, &bend(0.2), &bend(0.2));
    // The second and the third posed; then, before a frame is drawn, the third alone again: the second's previous
    // vertices are its pose's (it stands), the third's the pose between.
    w.set_animation(51, Some(bend(0.6))).unwrap();
    w.set_animation(52, Some(bend(0.7))).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "both posed: in place ({:?})", live.fallback);
    w.set_animation(52, Some(bend(0.9))).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the third posed again: in place ({:?})", live.fallback);
    check("the second, which stands", &live, 51, &bend(0.6), &bend(0.6));
    check("the third, posed twice", &live, 52, &bend(0.9), &bend(0.7));
    against_a_build("previous vertices kept mesh by mesh", &mut live, &w);

    // A rebuild's: one whose pose mirrors other primitives than the scene's; one of a model the scene has not (by its nodes).
    add(&mut w, 36, "skin-blend", pose("flip", 0.9), Vec3::new(0.6, 0.4, 0.7));
    assert!(live.commit(&mut w).unwrap(), "one whose pose mirrors otherwise: a rebuild");
    against_a_build("one whose pose mirrors otherwise", &mut live, &w);
    w.set_object(37, "m37".into(), None, Kind::Model { uri: file("skin-blend"), nodes: Some(vec!["strip".into()]), animation: Some(bend(0.2)), functions: vec![] }, true, false).unwrap();
    w.set_transform(37, Vec3::new(-0.6, 0.5, 0.7), glam::Quat::IDENTITY, Vec3::splat(1.2)).unwrap();
    assert!(live.commit(&mut w).unwrap(), "one of a model the scene has not: a rebuild");
    against_a_build("one of a model the scene has not", &mut live, &w);
}

/// In a scene with particle systems the rooms for edits do not grow (the particle passes hold the scene's buffers),
/// and a skinned model added needs room for its previous vertices: a rebuild's, as a mesh larger than the room is
/// there. Posed on and removed, the build's are in place as anywhere.
#[test]
fn a_posed_skinned_model_added_beside_particles_is_a_rebuilds() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let bend = |time| Some(crate::desc::Animation { clip: "bend".into(), time, r#loop: true, nodes: Default::default() });
    let arm = |time| Kind::Model { uri: "tests/render/data/arm.glb".into(), nodes: None, animation: bend(time), functions: vec![] };
    let mut w = world();
    w.set_particle_fn("spark", "fn pf_spark(i: PIn, p: array<vec4<f32>, 8>) -> POut { return POut(vec3<f32>(0.3, 0.2, 1.0), vec3<f32>(0.05, 0.0, 0.0), vec3<f32>(0.0, 0.05, 0.0), true, vec4<f32>(0.0)); }".into(), String::new());
    w.set_object(18, "sparks".into(), None, Kind::Particles { count: 3, emitter: "spark".into(), params: vec![], material: 5, emitter_area: 0.01 }, true, false).unwrap();
    // (A mesh the scene says it edits: room for triangles and records, which a model's instance would take.)
    w.set_object(28, "own".into(), None, Kind::Mesh { geometry: 3, material: 1, own_blas: true, materials: vec![] }, true, false).unwrap();
    w.set_transform(28, Vec3::new(1.4, -0.7, 0.4), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    for (id, x) in [(30, -0.9), (31, 0.0)] {
        w.set_object(id, format!("arm{id}"), None, arm(0.0), true, false).unwrap();
        w.set_transform(id, Vec3::new(x, -1.0, 0.8), glam::Quat::IDENTITY, Vec3::splat(2.0)).unwrap();
    }
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    if !live.g.hwrt {
        return eprintln!("skipped: no hardware ray tracing");
    }
    assert!(live.sc.up.particles.is_some() && live.sc.edit.tris.cap >= live.sc.edit.tris.end + 4, "a scene with a particle pass, and room for a model's triangles");
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        let mut fresh = open(w).unwrap();
        fresh.frame = live.frame;
        forget(live);
        let (a, b) = (picture(live, 64), picture(&mut fresh, 64));
        assert!(apart(&a, &b) < 0.35, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
    };
    w.set_animation(30, bend(0.6)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "a built one posed: in place ({:?})", live.fallback);
    against_a_build("a built one posed", &mut live, &w);
    w.set_object(32, "arm32".into(), None, arm(0.3), true, false).unwrap();
    w.set_transform(32, Vec3::new(0.9, -1.0, 0.8), glam::Quat::IDENTITY, Vec3::splat(2.0)).unwrap();
    assert!(live.commit(&mut w).unwrap(), "one added beside particles: a rebuild");
    assert!(live.fallback.as_deref().is_some_and(|f| f.contains("previous vertices")), "for its previous vertices' room ({:?})", live.fallback);
    against_a_build("one added beside particles", &mut live, &w);
    w.remove(31);
    assert!(!live.commit(&mut w).unwrap(), "one removed beside particles: in place ({:?})", live.fallback);
    against_a_build("one removed beside particles", &mut live, &w);
}

/// A posed skinned model is removed in place: its instance is no object's and no ray sees it, the others are posed
/// on as ever, and the picture is a build's. One that is hidden is removed in place too. Under either traversal.
#[test]
fn a_posed_skinned_model_is_removed_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let bend = |time| Some(crate::desc::Animation { clip: "bend".into(), time, r#loop: true, nodes: Default::default() });
    let mut w = world();
    for (id, x) in [(30, -0.9), (31, 0.0), (32, 0.9)] {
        w.set_object(id, format!("arm{id}"), None, Kind::Model { uri: "tests/render/data/arm.glb".into(), nodes: None, animation: bend(0.0), functions: vec![] }, true, false).unwrap();
        w.set_transform(id, Vec3::new(x, -1.0, 0.8), glam::Quat::IDENTITY, Vec3::splat(2.0)).unwrap();
    }
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let against_a_build = |what: &str, live: &mut LiveCore, w: &World| {
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 64), picture(&mut fresh, 64));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        a
    };
    let loaded = against_a_build("as loaded", &mut live, &w);
    let record = |c: &LiveCore, id: u32| c.sc.tables.retained.instance(id).expect("a posed skinned model").instance;
    let (r30, r31, r32) = (record(&live, 30), record(&live, 31), record(&live, 32));
    w.remove(31);
    assert!(!live.commit(&mut w).unwrap(), "removed: in place ({:?})", live.fallback);
    assert!(!seen_by_rays(&live, r31) && seen_by_rays(&live, r30) && seen_by_rays(&live, r32), "no ray sees the one removed; the others are seen");
    assert!(live.sc.tables.retained.instance(31).is_none() && live.pick(0.5, 0.5).is_none_or(|p| p.0 != "arm31"), "it is no object's, and a click does not find it");
    let removed = against_a_build("removed", &mut live, &w);
    assert!(apart(&loaded, &removed) > 0.5, "it is out of the picture ({:.2} steps)", apart(&loaded, &removed));
    // The others posed and moved on, in place.
    w.set_animation(30, bend(0.7)).unwrap();
    w.set_transform(32, Vec3::new(0.3, -0.9, 0.9), glam::Quat::from_rotation_y(0.4), Vec3::splat(2.0)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "the others posed and moved: in place ({:?})", live.fallback);
    let on = against_a_build("the others posed and moved", &mut live, &w);
    assert!(apart(&removed, &on) > 0.5, "that is in the picture ({:.2} steps)", apart(&removed, &on));
    // One hidden, then removed: in place both.
    w.set_visible(32, false).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "hidden: in place ({:?})", live.fallback);
    w.remove(32);
    assert!(!live.commit(&mut w).unwrap(), "a hidden one removed: in place ({:?})", live.fallback);
    assert!(!seen_by_rays(&live, r32) && live.sc.tables.retained.instance(32).is_none(), "no ray sees it, it is no object's");
    against_a_build("a hidden one removed", &mut live, &w);
    // The last, posed in the commit that removes it: in place, nothing left of it.
    w.set_animation(30, bend(0.2)).unwrap();
    w.remove(30);
    assert!(!live.commit(&mut w).unwrap(), "posed and removed at once: in place ({:?})", live.fallback);
    assert!(!seen_by_rays(&live, r30), "no ray sees it");
    against_a_build("all removed", &mut live, &w);
}

/// The skinning pass writes what the CPU skin writes. Two models (tests/render/data/skin-blend.glb: 72 triangles,
/// vertices two joints hold, weights that do not sum to one, a primitive with tangents and one without normals, a
/// mesh on a node beside the skin that mirrors it, a joint turned about an axis that is neither the normals' nor the
/// tangents', its materials in an order a mesh laid out by material does not keep; and arm.glb), two of each, each
/// a mesh of its own, in a scene with world-space triangles before the meshes': after each new pose the GPU's
/// triangles, shading attributes and BLAS input of each mesh are its model's triangles for the pose, the previous
/// frame's vertices are the pose before, and the CPU has the mesh as it is now when asked. One posed alone leaves
/// the others' as they were. Also after the scene's buffers were made again larger, and a pose that mirrors other
/// primitives than the mesh was made for is a rebuild's. Under software traversal there is no BLAS input: each mesh's
/// tree keeps its model's shape and has the pose's boxes, and the top tree its unit by them. The motion vectors of a
/// pose are no longer than its corners went, under either.
#[test]
fn the_skinning_pass_writes_what_the_cpu_skin_writes() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let pose = |clip: &str, time| crate::desc::Animation { clip: clip.into(), time, r#loop: true, nodes: Default::default() };
    let bend = |time| pose("bend", time);
    let mut w = world();
    for (id, uri, x) in [(30, "skin-blend", -0.5), (31, "skin-blend", 0.9), (32, "arm", -1.5), (33, "arm", 1.6)] {
        w.set_object(id, format!("m{id}"), None, Kind::Model { uri: format!("tests/render/data/{uri}.glb"), nodes: None, animation: Some(bend(0.0)), functions: vec![] }, true, false).unwrap();
        w.set_transform(id, Vec3::new(x, -0.6, 0.8), glam::Quat::from_rotation_y(0.3), Vec3::splat(1.1)).unwrap();
    }
    // (A rect light's card, far below: world-space triangles, which come before the meshes' in the GPU's order.)
    let card = serde_json::from_value(serde_json::json!({ "type": "rect", "color": [1, 1, 1], "castShadow": true, "intensity": 1, "width": 0.2, "height": 0.2, "range": 0 })).unwrap();
    w.set_object(21, "card".into(), None, Kind::Light(card), true, false).unwrap();
    w.set_transform(21, Vec3::new(0.0, -60.0, 0.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let hw = live.g.hwrt;
    assert!(live.sc.edit.pool_first > 0, "a mesh's first triangle on the GPU is not its first in the pool");
    {
        let g = live.sc.tables.retained.skinned.iter().find(|g| g.uri.ends_with("skin-blend.glb")).expect("the model's mesh");
        assert!(g.order.iter().enumerate().any(|(at, &(k, _, _))| at as u32 != k), "the mesh has the model's triangles in another order (by material)");
    }
    // (A mesh, its instance and its object, for each deforming mesh of the scene.)
    let deforming = |c: &LiveCore| c.sc.tables.retained.skinned.iter().map(|g| (g.uri.clone(), g.order.clone(), g.instance, g.object)).collect::<Vec<_>>();
    assert_eq!(deforming(&live).len(), 4, "four models, each a mesh of its own");
    let read = |c: &LiveCore, b: &wgpu::Buffer, at: u64, bytes: u64| -> Vec<f32> { read_back(c, b, at, bytes) };
    // What the CPU skin gives a mesh for a pose: its triangles' corners, nine numbers a triangle.
    let cpu = |c: &LiveCore, g: usize, a: &crate::desc::Animation| skinned_on_cpu(c, &c.sc.tables.retained.skinned[g], a).p;
    // Software traversal: a mesh's tree on the GPU has its model's tree's shape (the links, word for word) and the
    // pose's boxes: a leaf's its triangles' (`corners`: the CPU skin's, nine numbers a triangle), an interior node's
    // its two children's. The mesh's bounds are the root's, and the top tree's leaf of its unit is those, placed.
    let tree = |what: &str, c: &LiveCore, g: usize, corners: &[f32]| {
        let Some(units) = &c.sc.up.units else { return };
        let sk = &c.sc.tables.retained.skinned[g];
        let (t, shape) = (units.trees[sk.mesh as usize].expect("a unit's tree"), &c.sc.skin_trees[sk.source as usize]);
        assert!(t.len as usize == shape.len() && t.len > 0, "{what}: its tree is as long as its model's ({} nodes, {})", t.len, shape.len());
        let words = read(c, &c.sc.up.bg_parts.nodes, t.root as u64 * 32, t.len as u64 * 32);
        let nodes: &[crate::bvh::GpuNode] = bytemuck::cast_slice(&words);
        let (mut leaves, mut seen) = (0, vec![false; corners.len() / 9]);
        for (k, (n, m)) in nodes.iter().zip(shape).enumerate() {
            assert_eq!((n.left_or_first, n.count), (m.left_or_first, m.count), "{what}: node {k} is linked as its model's");
            let (lo, hi) = (Vec3::from(n.bmin), Vec3::from(n.bmax));
            if n.count > 0 {
                let own = &corners[n.left_or_first as usize * 9..(n.left_or_first + n.count) as usize * 9];
                let (a, b) = own.chunks_exact(3).fold((Vec3::splat(f32::INFINITY), Vec3::splat(f32::NEG_INFINITY)), |(a, b), p| (a.min(Vec3::from_slice(p)), b.max(Vec3::from_slice(p))));
                assert!((lo - a).abs().max_element() <= 2e-5 * a.abs().max_element().max(1.0) && (hi - b).abs().max_element() <= 2e-5 * b.abs().max_element().max(1.0), "{what}: leaf {k}'s box is {lo} .. {hi}; its triangles', {a} .. {b}");
                (n.left_or_first..n.left_or_first + n.count).for_each(|i| seen[i as usize] = true);
                leaves += 1;
            } else {
                let (x, y) = (nodes[n.left_or_first as usize], nodes[n.left_or_first as usize + 1]);
                assert_eq!((lo, hi), (Vec3::from(x.bmin).min(y.bmin.into()), Vec3::from(x.bmax).max(y.bmax.into())), "{what}: node {k}'s box is its children's");
            }
        }
        // (A leaf holds 16 triangles at most: the larger model's tree has interior nodes.)
        assert!((leaves > 1 || seen.len() <= 16) && seen.iter().all(|&s| s), "{what}: every triangle is in a leaf ({leaves} leaves)");
        assert_eq!((Vec3::from(t.lo), Vec3::from(t.hi)), (nodes[0].bmin.into(), nodes[0].bmax.into()), "{what}: the mesh's bounds are its root's");
        let unit = crate::units::UnitBox::new(sk.instance, c.sc.instances[sk.instance as usize].world, t.lo, t.hi);
        let leaf = units.kept.nodes.iter().find(|n| n.count > 0 && n.left_or_first == sk.instance).expect("the unit's leaf of the top tree");
        assert_eq!((leaf.bmin, leaf.bmax), (unit.lo.into(), unit.hi.into()), "{what}: the top tree has the unit by its mesh's bounds, placed");
    };
    let check_one = |what: &str, c: &LiveCore, g: usize, now: &crate::desc::Animation, before: &crate::desc::Animation| {
        let sk = &c.sc.tables.retained.skinned[g];
        let what = format!("{what}, {} (object {})", sk.uri, sk.object);
        let corners = skinned_is_the_cpus(&what, c, sk, &format!("m{}", sk.object), now, before);
        tree(&what, c, g, &corners);
    };
    let check = |what: &str, c: &LiveCore, now: &crate::desc::Animation, before: &crate::desc::Animation| {
        for g in 0..deforming(c).len() {
            check_one(what, c, g, now, before);
        }
    };
    let all = |w: &mut World, a: &crate::desc::Animation| {
        for id in [30, 31, 32, 33] {
            w.set_animation(id, Some(a.clone())).unwrap();
        }
    };
    let (first, second, third) = (bend(0.0), bend(0.75), bend(0.4));
    let loaded: Vec<Vec<f32>> = (0..4).map(|g| cpu(&live, g, &first)).collect();
    for (what, now, before) in [("a new pose", &second, &first), ("the next", &third, &second)] {
        all(&mut w, now);
        assert!(!live.commit(&mut w).unwrap(), "{what}: in place ({:?})", live.fallback);
        check(what, &live, now, before);
    }
    assert!((0..4).all(|g| cpu(&live, g, &third).iter().zip(&loaded[g]).any(|(a, b)| (a - b).abs() > 0.05)), "the pose moves every mesh");
    // One posed alone: its mesh, and the others' as they were (their previous frame's vertices their own pose now).
    let alone = bend(0.15);
    w.set_animation(31, Some(alone.clone())).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "one posed alone: in place ({:?})", live.fallback);
    for g in 0..4 {
        if deforming(&live)[g].3 == 31 {
            check_one("one posed alone", &live, g, &alone, &third);
        } else {
            check_one("the others, left alone", &live, g, &third, &third);
        }
    }
    w.set_animation(31, Some(third.clone())).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "and back: in place ({:?})", live.fallback);
    // The picture is a build's.
    let pictured = |what: &str, live: &mut LiveCore, w: &World| {
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 64), picture(&mut fresh, 64));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
    };
    pictured("posed in place", &mut live, &w);
    // Moved alone (a model the scene did not call movable): its instance, in place, and the picture a build's.
    w.set_transform(31, Vec3::new(1.1, -0.4, 0.9), glam::Quat::from_rotation_y(0.5), Vec3::splat(1.1)).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "a posed model moved: in place ({:?})", live.fallback);
    let stands = |c: &LiveCore, id: u32| c.sc.instances[c.sc.tables.retained.instance(id).unwrap().instance as usize].world;
    assert_eq!(stands(&live, 31), w.world_matrix(31), "its instance stands where the model was moved to");
    pictured("a posed model moved", &mut live, &w);
    // Posed in the same frame as moved: in place, the mesh skinned, the instance moved.
    w.set_transform(30, Vec3::new(-0.9, -0.5, 0.9), glam::Quat::from_rotation_y(-0.2), Vec3::splat(1.1)).unwrap();
    all(&mut w, &bend(0.9));
    assert!(!live.commit(&mut w).unwrap(), "posed and moved at once: in place ({:?})", live.fallback);
    check("posed and moved at once", &live, &bend(0.9), &third);
    assert_eq!(stands(&live, 30), w.world_matrix(30), "its instance stands where the model was moved to");
    pictured("posed and moved at once", &mut live, &w);
    // The scene's buffers made again, larger (a mesh of more triangles and more vertices than the room for edits has,
    // added in place, far below the view): the pass writes the new ones.
    let rooms_size = |c: &LiveCore| (c.sc.up.vertices.size(), c.sc.up.words.cap(crate::heap::Stream::Indices));
    let size = rooms_size(&live);
    let n = (((live.sc.edit.verts.cap - live.sc.edit.verts.end) as f64).sqrt() as u32 + 1).max(200);
    let positions: Vec<f32> = (0..=n).flat_map(|y| (0..=n).flat_map(move |x| [x as f32, y as f32, 0.0])).collect();
    let indices: Vec<u32> = (0..n).flat_map(|y| (0..n).flat_map(move |x| { let k = y * (n + 1) + x; [k, k + 1, k + n + 2, k, k + n + 2, k + n + 1] })).collect();
    let corners = ((n + 1) * (n + 1)) as usize;
    w.set_geometry(7, Geometry { normals: [0.0, 0.0, 1.0].repeat(corners), uvs: vec![0.0; corners * 2], positions, indices, groups: vec![], colors: vec![] }).unwrap();
    w.set_object(40, "sheet".into(), None, Kind::Mesh { geometry: 7, material: 1, own_blas: false, materials: vec![] }, true, false).unwrap();
    w.set_transform(40, Vec3::new(0.0, -80.0, 0.0), glam::Quat::IDENTITY, Vec3::splat(0.001)).unwrap();
    // (Software traversal's room does not grow: a rebuild's there.)
    assert_eq!(live.commit(&mut w).unwrap(), !hw, "a large mesh added: in place under hardware traversal ({:?})", live.fallback);
    assert!(!hw || (rooms_size(&live).0 > size.0 && rooms_size(&live).1 > size.1), "the vertices' buffer and the triangles' indices were made again, larger ({size:?} -> {:?})", rooms_size(&live));
    if !hw {
        picture(&mut live, 2);
    }
    // (Under software traversal this is a build's: the meshes, and their trees, as the build makes them for a pose.)
    check("after a large mesh was added", &live, &bend(0.9), &bend(0.9));
    all(&mut w, &bend(0.25));
    assert!(!live.commit(&mut w).unwrap(), "posed after the buffers grew: in place ({:?})", live.fallback);
    check("posed after the buffers grew", &live, &bend(0.25), &bend(0.9));
    // The motion of that pose: no vector is longer than the furthest a corner of a model went (a kernel that takes
    // another triangle's previous vertices gives vectors as long as the mesh), and some are nearly as long.
    {
        (live.fixed_dt, live.frame, live.reset) = (Some(1.0 / 60.0), 16, true);
        live.render(1, |_, _| {});
        let m = &live.targets.kernel.motion;
        let moved = read(&live, &m.buffer, m.offset, m.size);
        let corners = |v: &[f32]| -> Vec<Vec3> { v.chunks_exact(3).map(Vec3::from_slice).collect() };
        let far = (0..4).map(|g| {
            let world = live.sc.instances[deforming(&live)[g].2 as usize].world;
            corners(&cpu(&live, g, &bend(0.25))).iter().zip(corners(&cpu(&live, g, &bend(0.9)))).map(|(x, y)| world.transform_vector3(*x - y).length()).fold(0.0f32, f32::max)
        }).fold(0.0f32, f32::max);
        let vectors: Vec<f32> = moved.chunks_exact(4).filter(|p| p[3] > 0.5).map(|p| Vec3::new(p[0], p[1], p[2]).length()).collect();
        let longest = vectors.iter().copied().fold(0.0f32, f32::max);
        assert!(vectors.iter().filter(|&&l| l > 1e-3).count() > 20 && longest <= far * 1.001 + 1e-4 && longest > far * 0.2, "{} pixels moved; the longest motion vector is {longest}; the furthest a corner went, {far}", vectors.iter().filter(|&&l| l > 1e-3).count());
    }
    pictured("posed after the buffers grew", &mut live, &w);
    // A pose that mirrors other primitives than the mesh was made for (the clip `flip` turns the badge's node the
    // other way round): its triangles are in another order, a build's to make.
    for id in [30, 31] {
        w.set_animation(id, Some(pose("flip", 0.9))).unwrap();
    }
    assert!(live.commit(&mut w).unwrap(), "a pose that mirrors other primitives: a rebuild");
    pictured("a pose that mirrors other primitives", &mut live, &w);
}

/// Models the game poses node by node (tests/render/data/arm.glb's joint `tip`, no clip): each is a pose of its own, even
/// in the same pose as another, so posing them apart is in place; the picture is a build's.
#[test]
fn models_posed_node_by_node_are_posed_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    let turn = |a: f32| {
        let q = glam::Quat::from_rotation_z(a);
        Some(crate::desc::Animation { clip: String::new(), time: 0.0, r#loop: true, nodes: [("tip".to_string(), q.to_array().to_vec())].into() })
    };
    let mut w = world();
    for (id, x) in [(30, -0.7), (31, 0.7)] {
        w.set_object(id, format!("arm{id}"), None, Kind::Model { uri: "tests/render/data/arm.glb".into(), nodes: None, animation: turn(0.0), functions: vec![] }, true, false).unwrap();
        w.set_transform(id, Vec3::new(x, -1.0, 0.8), glam::Quat::IDENTITY, Vec3::splat(2.0)).unwrap();
    }
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    assert_eq!(live.sc.tables.retained.skinned.len(), 2, "each a mesh of its own, though in the same pose");
    let before = picture(&mut live, 64);
    for (id, a) in [(30, 0.9), (31, -1.2)] {
        w.set_animation(id, turn(a)).unwrap();
    }
    assert!(!live.commit(&mut w).unwrap(), "posed apart in place ({:?})", live.fallback);
    let mut fresh = open(&w).unwrap();
    forget(&mut live);
    let (a, b) = (picture(&mut live, 64), picture(&mut fresh, 64));
    assert!(apart(&a, &b) < 0.5, "the picture is {:.2} steps from a build's", apart(&a, &b));
    assert!(apart(&before, &a) > 0.5, "the new poses are in the picture ({:.2} steps)", apart(&before, &a));
}

/// A scene whose only volume is hidden has its media passes made when it is loaded: the frame that shows the volume
/// compiles nothing (an idle fireball's first fire).
#[test]
fn a_hidden_volumes_passes_are_made_at_load() {
    let presets = crate::default_presets_dir();
    let mut w = world();
    w.set_volume_fn("f", "o.extinction = 1.0;".into(), String::new());
    w.set_object(40, "fog".into(), None, Kind::Volume { function: "f".into(), params: vec![], shape: 0, majorant: 1.0, g: 0.0, homogeneous: true, animated: false, grid: None }, true, false).unwrap();
    w.set_visible(40, false).unwrap();
    let mut live = match gpu::Gpu::new().and_then(|g| LiveCore::new(g, &w, Path::new("."), &presets, 96, 54)) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    assert!(live.sc.up.kernel.features & gpu::F_MEDIA != 0, "a hidden volume is the scene's medium");
    let made = |c: &LiveCore| {
        let mut v: Vec<&'static str> = c.g.kernel_passes_made().into_iter().filter(|e| e.starts_with("media")).collect();
        v.sort_unstable();
        v
    };
    assert_eq!(made(&live), ["media_composite", "media_train"], "made when the scene was loaded");
    picture(&mut live, 2);
    w.set_visible(40, true).unwrap();
    assert!(!live.commit(&mut w).unwrap(), "shown in place ({:?})", live.fallback);
    picture(&mut live, 2);
    assert_eq!(made(&live), ["media_composite", "media_train"], "the frame that shows it makes none");
}

/// A bright HDR panel catches clipping before reconstruction, stale bindings after resize, and
/// carrying temporal history through a cut. MetalFX methods run only where the SDK supports them.
#[test]
fn reconstruction_keeps_hdr_emitters_and_resets_on_a_cut() {
    let Ok(gpu) = gpu::Gpu::new() else { return };
    let mut w = world();
    w.set_material(5, serde_json::from_value(serde_json::json!({
        "name": "HDR panel", "color": [0, 0, 0], "opacity": 1, "metalness": 0, "roughness": 1,
        "emissive": [1, 0.25, 0.5], "emissiveIntensity": 1800, "doubleSided": true,
    })).unwrap());
    w.set_transform(12, Vec3::new(0.0, 0.0, 1.0), glam::Quat::IDENTITY, Vec3::new(5.0, 0.5, 1.0)).unwrap();
    w.set_camera_desc(4, camera(50.0, None, manual(6.0))).unwrap();
    let mut core = LiveCore::new(gpu, &w, Path::new("."), &crate::default_presets_dir(), 320, 180).unwrap();
    let scene_cam = core.cam;
    let center = |c: &LiveCore| {
        let hdr = c.read_hdr().unwrap();
        assert!(hdr.iter().all(|v| v.is_finite() && *v >= 0.0));
        let p = ((c.h / 2 * c.w + c.w / 2) * 3) as usize;
        [hdr[p], hdr[p + 1], hdr[p + 2]]
    };
    let methods = [
        Method::Fsr,
        Method::Off,
        #[cfg(metalfx)]
        Method::MetalFx,
        #[cfg(metalfx)]
        Method::MetalFxDenoised,
    ];
    for method in methods {
        core.set_scale(0.5);
        core.set_reconstruction(method.clone());
        core.set_camera(scene_cam);
        for _ in 0..16 { core.render(1, |_, _| {}); }
        if core.reconstruction() == Method::Builtin {
            assert!(!matches!(method, Method::Fsr | Method::Off));
            eprintln!("{} unsupported on this device: {:?}", method.name(), core.reconstruction_status().skipped);
            continue;
        }
        for (v, reference) in center(&core).into_iter().zip([1800.0, 450.0, 900.0]) {
            assert!((v / reference - 1.0).abs() < 0.2, "{}: {v} of {reference}", method.name());
        }
        core.set_scale(0.0);
        for scale in [0.5, 0.75, 0.625] {
            core.render_scale = scale;
            core.render(1, |_, _| {});
            assert_eq!(core.reconstruction(), method);
            assert!(center(&core)[0] > 1000.0, "{}: missing dynamic input at {scale}", method.name());
        }
        core.set_camera(Camera::look_at(scene_cam.pos, scene_cam.pos + Vec3::Z, scene_cam.tan_half_fov));
        core.render(1, |_, _| {});
        // (The view after the turn sees surfaces the panel lights a little, ~4 here; history carried over reads ~1800.)
        assert!(center(&core).iter().all(|v| *v < 100.0), "{}: bright history after cut", method.name());
        core.resize(257, 161).unwrap();
        core.set_camera(scene_cam);
        core.render(1, |_, _| {});
        assert!(center(&core)[0] > 1000.0, "{}: missing output after resize", method.name());
        core.resize(320, 180).unwrap();
    }
}

/// A bright emitter keeps its level and hue through MetalFX, as through the built-in path, from 1800 to 120 000 nits
/// (at EV 6: 28 to 1900 times display white). The denoised scaler given lighting near display white kept them only to
/// ~30x: Zero Day's orange LEDs (~125x) came out yellow and its frame at 0.68 of the offline reference's luminance; at
/// 20 000 nits this read 0.82 of the level and green / red 0.29, at 120 000, 0.22 and 0.71 (metalfx.rs `white_units`).
#[test]
fn metalfx_keeps_bright_emitters_level_and_hue() {
    #[cfg(metalfx)]
    let methods = [Method::MetalFx, Method::MetalFxDenoised];
    #[cfg(not(metalfx))]
    let methods: [Method; 0] = [];
    for nits in [1800.0f32, 20000.0, 120000.0] {
        let Ok(gpu) = gpu::Gpu::new() else { return };
        let mut w = world();
        w.set_material(5, serde_json::from_value(serde_json::json!({
            "name": "HDR panel", "color": [0, 0, 0], "opacity": 1, "metalness": 0, "roughness": 1,
            "emissive": [1, 0.25, 0.5], "emissiveIntensity": nits, "doubleSided": true,
        })).unwrap());
        w.set_transform(12, Vec3::new(0.0, 0.0, 1.0), glam::Quat::IDENTITY, Vec3::new(5.0, 0.5, 1.0)).unwrap();
        w.set_camera_desc(4, camera(50.0, None, manual(6.0))).unwrap();
        let mut core = LiveCore::new(gpu, &w, Path::new("."), &crate::default_presets_dir(), 320, 180).unwrap();
        for method in methods.clone() {
            core.set_scale(0.5);
            core.set_reconstruction(method.clone());
            for _ in 0..48 { core.render(1, |_, _| {}); }
            if core.reconstruction() == Method::Builtin {
                eprintln!("{} unsupported on this device: {:?}", method.name(), core.reconstruction_status().skipped);
                continue;
            }
            // The panel's middle (a block: one pixel's upscaling does not decide it).
            let hdr = core.read_hdr().unwrap();
            let mut c = [0.0f32; 3];
            for y in core.h / 2 - 5..core.h / 2 + 5 {
                for x in core.w / 2 - 10..core.w / 2 + 10 {
                    let p = ((y * core.w + x) * 3) as usize;
                    for k in 0..3 { c[k] += hdr[p + k] / 200.0; }
                }
            }
            let what = format!("{} at {nits} nits: {c:?}", method.name());
            assert!((c[0] / nits - 1.0).abs() < 0.05, "{what}: level");
            assert!((c[1] / c[0] / 0.25 - 1.0).abs() < 0.03 && (c[2] / c[0] / 0.5 - 1.0).abs() < 0.03, "{what}: hue");
        }
    }
}

/// A request is a preference list: the first method this device runs is used, and the status says why each one
/// before it did not. A typed quality preset is the vendors' exact ratio.
#[test]
fn reconstruction_takes_the_first_method_that_runs_and_says_why() {
    assert_eq!(Reconstruction::parse("metalfx-denoised, builtin").unwrap(), Reconstruction::Prefer(vec![Method::MetalFxDenoised, Method::Builtin]));
    assert_eq!(Reconstruction::from_json(&serde_json::json!(["dlss-rr", "fsr"])).unwrap().name(), "dlss-rr,fsr");
    assert_eq!(Reconstruction::parse("auto").unwrap(), Reconstruction::Auto);
    assert!(Reconstruction::parse("dlss-fg").is_err());
    let r = RenderingRequest::from_json(&serde_json::json!({ "quality": "ultra-performance", "latency": "on", "frameGeneration": "2x" })).unwrap();
    assert_eq!(((1920.0 * r.quality.unwrap().scale().unwrap()) as f32).round(), 640.0, "a third of 1080p's width is DLSS's 640");
    assert!(RenderingRequest::from_json(&serde_json::json!({ "upscaler": "dlss" })).is_err(), "unknown settings are errors");

    let Ok(g) = gpu::Gpu::new() else { return };
    let mut live = LiveCore::new(g, &world(), Path::new("."), &crate::default_presets_dir(), 97, 55).unwrap();
    live.view_mode = 0;
    let control = RenderingControl::default();
    live.attach_rendering_control(control.clone());
    control.request(RenderingRequest { reconstruction: Some(Reconstruction::parse("custom:none,fsr").unwrap()), ..r });
    live.render(1, |_, _| {});
    assert_eq!(live.reconstruction(), Method::Fsr);
    let st = control.status();
    assert_eq!(st["reconstruction"]["active"], "fsr");
    assert_eq!(st["reconstruction"]["denoise"], "builtin");
    assert_eq!(st["reconstruction"]["fallback"], "unavailable");
    assert_eq!(st["reconstruction"]["skipped"][0]["method"], "custom:none");
    assert_eq!(st["quality"]["scale"], 1.0 / 3.0f32);
    assert_eq!(st["latency"]["active"], "on");
    assert_eq!(live.frames_in_flight(), 1);
    assert_eq!(st["frameGeneration"]["fallback"], "unavailable", "no frame generation yet, and it says so");
}

/// A test provider: on prepare, a pipeline that writes (4, 8, 12) into the output; the frame after, it fails.
struct ConstantOnce(Option<wgpu::ComputePipeline>);

impl ConstantOnce {
    fn prepare(&mut self, g: &gpu::Gpu) {
        let d = &g.device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(r#"
                @group(0) @binding(0) var image: texture_storage_2d<rgba16float, write>;
                @compute @workgroup_size(8,8) fn put_color(@builtin(global_invocation_id) id: vec3<u32>) {
                    if (all(id.xy < textureDimensions(image))) {
                        textureStore(image, id.xy, vec4<f32>(4.0, 8.0, 12.0, 1.0));
                    }
                }
            "#.into()),
        });
        self.0 = Some(d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: None, layout: None, module: &module, entry_point: Some("put_color"),
            compilation_options: Default::default(), cache: None,
        }));
    }

    fn encode(&mut self, f: &ReconstructionFrame<'_>, e: &mut wgpu::CommandEncoder) -> Result<Option<wgpu::CommandBuffer>> {
        let Some(pipe) = self.0.take() else { anyhow::bail!("test provider rejected the frame") };
        let bg = f.gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipe.get_bind_group_layout(0),
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(f.output) }],
        });
        let mut pass = e.begin_compute_pass(&Default::default());
        pass.set_pipeline(&pipe);
        pass.set_bind_group(0, &bg, &[]);
        pass.dispatch_workgroups(f.size.output[0].div_ceil(8), f.size.output[1].div_ceil(8), 1);
        Ok(None)
    }
}

impl UpscaleProvider for ConstantOnce {
    fn prepare(&mut self, g: &gpu::Gpu, size: ReconstructionSize) -> Result<Prepared> { ConstantOnce::prepare(self, g); Ok(Prepared::ready(size)) }
    fn encode(&mut self, f: ReconstructionFrame<'_>, e: &mut wgpu::CommandEncoder) -> Result<Option<wgpu::CommandBuffer>> { ConstantOnce::encode(self, &f, e) }
}

#[test]
fn reconstruction_falls_back_in_the_frame_a_custom_provider_fails() {
    let Ok(g) = gpu::Gpu::new() else { return };
    let mut live = LiveCore::new(g, &world(), Path::new("."), &crate::default_presets_dir(), 97, 55).unwrap();
    live.view_mode = 0;
    live.set_scale(0.5);
    let control = RenderingControl::default();
    live.attach_rendering_control(control.clone());
    live.register_upscaler("custom:test", ConstantOnce(None)).unwrap();
    control.request(RenderingRequest { reconstruction: Some(Reconstruction::parse("custom:test").unwrap()), ..Default::default() });
    live.render(1, |_, _| {});
    let p = ((live.h / 2 * live.w + live.w / 2) * 3) as usize;
    assert_eq!(&live.read_hdr().unwrap()[p..p+3], &[128.0, 256.0, 384.0]);
    assert_eq!(control.status()["reconstruction"]["active"], "custom:test");
    assert_eq!(control.status()["reconstruction"]["upscale"], "custom:test");

    live.render(1, |_, _| {});
    let hdr = live.read_hdr().unwrap();
    assert!(hdr[p] > 0.01 && hdr[p] < 32.0, "expected scene radiance after failure, got {}", hdr[p]);
    assert_eq!(control.status()["reconstruction"], serde_json::json!({
        "requested": "custom:test", "active": "builtin", "denoise": "builtin", "upscale": "builtin", "fallback": "provider-error",
        "skipped": [{ "method": "custom:test", "reason": "provider-error", "detail": "test provider rejected the frame" }],
    }));
}

/// A joint denoiser gets the noisy signal and every guide, at the input's size in the contract's formats, and the
/// built-in denoiser does not run.
#[test]
fn a_joint_denoiser_gets_the_noisy_signal_and_the_guides() {
    struct Joint(ConstantOnce, std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl JointProvider for Joint {
        fn prepare(&mut self, g: &gpu::Gpu, size: ReconstructionSize) -> Result<Prepared> { self.0.prepare(g); Ok(Prepared::ready(size)) }
        fn encode(&mut self, f: ReconstructionFrame<'_>, j: JointGuides<'_>, e: &mut wgpu::CommandEncoder) -> Result<Option<wgpu::CommandBuffer>> {
            let seen = [f.color, f.depth, f.motion, f.reactive, j.overlay, j.bypass, j.diffuse_albedo, j.specular_albedo, j.normal, j.roughness, j.specular_hit_distance]
                .map(|v| { let t = v.texture(); format!("{:?} {}x{}", t.format(), t.width(), t.height()) });
            *self.1.lock().unwrap() = seen.into_iter().chain([f.radiance_unit.to_string()]).collect();
            if self.0.0.is_none() { self.0.prepare(f.gpu); }
            self.0.encode(&f, e)
        }
    }
    let Ok(g) = gpu::Gpu::new() else { return };
    let mut w = world();
    w.set_camera_desc(4, camera(50.0, None, manual(6.0))).unwrap();
    let mut live = LiveCore::new(g, &w, Path::new("."), &crate::default_presets_dir(), 96, 54).unwrap();
    live.view_mode = 0;
    live.set_scale(0.5);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
    live.register_joint_denoiser("custom:joint", Joint(ConstantOnce(None), seen.clone())).unwrap();
    live.set_reconstruction(Method::parse("custom:joint").unwrap());
    live.render(1, |_, _| {});
    let st = live.reconstruction_status();
    assert_eq!((st.denoise.as_str(), st.upscale.as_str()), ("custom:joint", "custom:joint"));
    let seen = seen.lock().unwrap().clone();
    let unit: f32 = seen[11].parse().unwrap();
    assert_eq!(seen[..11], ["Rgba16Float 48x27", "R32Float 48x27", "Rg16Float 48x27", "R8Unorm 48x27", "Rgba16Float 48x27",
        "R8Unorm 48x27", "Rgba8Unorm 48x27", "Rgba8Unorm 48x27", "Rgba16Float 48x27", "R16Float 48x27", "R16Float 48x27"]);
    let p = ((live.h / 2 * live.w + live.w / 2) * 3) as usize;
    // The output's (4, 8, 12), in the frame's unit, back in nits.
    let hdr = live.read_hdr().unwrap();
    assert!((hdr[p] / (4.0 * unit) - 1.0).abs() < 0.01 && (hdr[p + 2] / (12.0 * unit) - 1.0).abs() < 0.01, "{:?} at {unit}", &hdr[p..p + 3]);
}

/// Dynamic resolution below a provider's range does not lose it: the governor goes up to the lowest scale the method
/// asked for takes (MetalFX: a third), and from there the method runs.
#[cfg(metalfx)]
#[test]
fn dynamic_resolution_rises_to_the_range_of_the_method_asked_for() {
    let Ok(g) = gpu::Gpu::new() else { return };
    let mut live = LiveCore::new(g, &world(), Path::new("."), &crate::default_presets_dir(), 640, 360).unwrap();
    live.view_mode = 0;
    live.set_scale(0.0);
    live.min_scale = 0.2;
    live.render_scale = 0.25;
    live.set_reconstruction(Method::MetalFxDenoised);
    live.render(1, |_, _| {});
    let st = live.reconstruction_status();
    if st.skipped.first().is_some_and(|s| !s.2.contains("takes outputs")) {
        eprintln!("MetalFX denoised unsupported here: {:?}", st.skipped);
        return;
    }
    assert_eq!(live.reconstruction(), Method::Builtin, "4x is outside MetalFX's range: {:?}", st.skipped);
    let floor = live.reconstructor.scale_floor().expect("a floor from MetalFX's range");
    assert!((floor - 1.0 / 3.0).abs() < 0.01, "{floor}");
    // A frame far over its time: the governor would go down, and goes to the floor instead.
    (live.gpu_ms, live.gpu_frame, live.frame) = (200.0, 1000, 1021);
    live.govern_resolution(60);
    assert!(live.render_scale >= floor, "{} under {floor}", live.render_scale);
    live.render(1, |_, _| {});
    assert_eq!(live.reconstruction(), Method::MetalFxDenoised, "{:?}", live.reconstruction_status().skipped);
}

/// A world texture of `width` x `height` pixels: a checker of `a` and `b`, `cell` pixels a square.
fn checker(width: u32, height: u32, cell: u32, a: [u8; 3], b: [u8; 3]) -> crate::world::Texture {
    let rgba: Vec<u8> = (0..width * height).flat_map(|k| if (k % width / cell + k / width / cell) % 2 == 0 { [a[0], a[1], a[2], 255] } else { [b[0], b[1], b[2], 255] }).collect();
    crate::world::Texture { source: crate::world::TextureSource::Pixels { width, height, rgba: std::sync::Arc::new(rgba) }, srgb: true, wrap: true, repeat: [1.0, 1.0], offset: [0.0, 0.0] }
}

/// By world material: the keys of the textures its slots name in the pages (base, normal, roughness, metalness,
/// emissive, occlusion), None where it names none. Where in the pages a texture lies, and its number, are a view's own.
fn textures_named(c: &LiveCore) -> std::collections::BTreeMap<u32, Vec<Option<String>>> {
    let keys = c.g.pages_state().0;
    let t = &c.sc.tables;
    t.retained.materials.iter().map(|(id, &mi)| {
        let m = &t.materials[mi as usize];
        (*id, [m.tex0[0], m.tex0[1], m.tex0[2], m.tex0[3], m.tex1[0], m.tex1[1]].iter().map(|&s| (s >= 0).then(|| keys[s as usize].0.clone())).collect())
    }).collect()
}

/// Textures come and change in place, in a scene that had none: a material given a map, a new material with one, a
/// second texture of pixels the pages have (the same slot), one of those two replaced (another slot: the other
/// keeps its pixels), a texture's pixels replaced by as many (over the old ones, in their slot) and by a picture of
/// another size (another slot), a texture wider than the pages (they get a longer side and a layer more, what they
/// held copied there). After each: no rebuild, every material names the textures a build of the edited world
/// names, and the picture is that build's. A texture set again as it is changes nothing.
#[test]
fn textures_come_and_change_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let mat = |color: f32, maps: serde_json::Value| -> crate::desc::Material {
        let mut m = serde_json::json!({ "name": "m", "color": [color, color, color], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true });
        m.as_object_mut().unwrap().extend(maps.as_object().unwrap().clone());
        serde_json::from_value(m).unwrap()
    };
    let mut w = world();
    // (The wall and the panel with UVs over them; the panel a mesh the scene edits, with a material of its own.)
    let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: [0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
    w.set_geometry(2, quad(1.0)).unwrap();
    w.set_geometry(3, quad(0.25)).unwrap();
    w.set_material(6, mat(0.9, serde_json::json!({})));
    w.set_object(11, "panel".into(), None, Kind::Mesh { geometry: 3, material: 6, own_blas: true, materials: vec![] }, true, false).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    assert_eq!((live.sc.up.kernel.features & gpu::F_TEX, live.g.pages_state().0.len()), (0, 0), "a scene without a texture");
    let mut last = picture(&mut live, 24);
    let mut check = |what: &str, live: &mut LiveCore, w: &mut World| {
        assert!(!live.commit(w).unwrap(), "{what}: in place ({:?})", live.fallback);
        let mut fresh = open(w).unwrap();
        assert_eq!(textures_named(live), textures_named(&fresh), "{what}: the textures the materials name");
        assert_eq!(live.sc.up.kernel.features, fresh.sc.up.kernel.features, "{what}: the kernel's features");
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        assert!(apart(&last, &b) > 0.2, "{what}: the picture is {:.2} steps from the one before", apart(&last, &b));
        last = b;
    };
    // The wall's material given a map.
    w.set_texture(30, checker(64, 40, 8, [255, 40, 40], [40, 40, 255])).unwrap();
    w.set_material(1, mat(0.8, serde_json::json!({ "map": 30 })));
    check("a material given a map", &mut live, &mut w);
    assert_ne!(live.sc.up.kernel.features & gpu::F_TEX, 0);
    let slot = |live: &LiveCore, texture: u32| live.sc.tables.retained.textures.get(&(texture, true)).copied();
    let (keys, side, layers, places) = live.g.pages_state();
    assert_eq!((keys.len(), side, layers, slot(&live, 30)), (1, 256, 1, Some(0)), "the first texture: slot 0 of one page");
    // A new material with a map of its own, on the panel.
    w.set_texture(31, checker(100, 60, 10, [30, 220, 30], [240, 240, 30])).unwrap();
    w.set_material(8, mat(0.9, serde_json::json!({ "map": 31 })));
    w.set_object(11, "panel".into(), None, Kind::Mesh { geometry: 3, material: 8, own_blas: true, materials: vec![] }, true, false).unwrap();
    check("a new material with a map", &mut live, &mut w);
    assert_eq!((live.g.pages_state().0.len(), live.g.pages_state().3[0]), (2, places[0]), "a second texture beside the first, which stays where it is");
    // Another texture of the same pixels, on a tile of its own: the slot the pages have for them.
    w.set_texture(33, checker(100, 60, 10, [30, 220, 30], [240, 240, 30])).unwrap();
    w.set_material(9, mat(0.9, serde_json::json!({ "map": 33 })));
    w.set_object(13, "tile".into(), None, Kind::Mesh { geometry: 3, material: 9, own_blas: true, materials: vec![] }, true, false).unwrap();
    w.set_transform(13, Vec3::new(0.45, -0.5, 0.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    check("a texture of pixels the pages have", &mut live, &mut w);
    assert_eq!((live.g.pages_state().0.len(), slot(&live, 31), slot(&live, 33)), (2, Some(1), Some(1)), "one slot for both");
    // The first of the two given other pixels: the slot is the other's too, so it takes another.
    w.set_texture(31, checker(100, 60, 10, [230, 30, 200], [20, 20, 20])).unwrap();
    check("one of two textures in a slot replaced", &mut live, &mut w);
    assert_eq!((live.g.pages_state().0.len(), slot(&live, 31), slot(&live, 33)), (3, Some(2), Some(1)), "another slot; the other texture keeps its own");
    // The same texture again: nothing to do. Its pixels replaced by as many: over the old ones.
    w.set_texture(30, checker(64, 40, 8, [255, 40, 40], [40, 40, 255])).unwrap();
    assert!(!w.changes.any(), "a texture set again as it is changes nothing");
    w.set_texture(30, checker(64, 40, 8, [20, 230, 230], [230, 20, 230])).unwrap();
    check("a texture's pixels replaced", &mut live, &mut w);
    assert_eq!((live.g.pages_state().0.len(), slot(&live, 30)), (3, Some(0)), "written over the old ones, in their slot");
    assert_ne!(live.g.pages_state().0[0].0, keys[0].0, "the slot holds the new pixels' key");
    // A picture of another size: another slot; the material names it.
    w.set_texture(30, checker(128, 128, 16, [250, 250, 250], [10, 10, 10])).unwrap();
    check("a texture replaced by one of another size", &mut live, &mut w);
    assert_eq!((live.g.pages_state().0.len(), slot(&live, 30)), (4, Some(3)));
    let (_, side, layers, before) = live.g.pages_state();
    assert!(side == 256 && layers == 2 && before[3].layer == 1, "the first page was full: the wall's texture lies on a second ({layers} layers of {side})");
    // A texture wider than the pages, on the panel: a longer side and a layer more, what they held copied there
    // (the wall's and the tile's are drawn from the copy).
    w.set_texture(32, checker(2048, 2048, 256, [250, 120, 20], [20, 120, 250])).unwrap();
    w.set_material(8, mat(0.9, serde_json::json!({ "map": 32 })));
    check("a texture wider than the pages", &mut live, &mut w);
    let (keys, side, layers, now) = live.g.pages_state();
    assert_eq!((keys.len(), side, layers, now[4].layer, now[4].size), (5, 2048, 3, 2, (2016, 2016)), "the pages grew by a side and a layer");
    assert_eq!(now[..4], before[..], "the textures there stay where they are");
    // A material that drops its map.
    w.set_material(1, mat(0.8, serde_json::json!({})));
    check("a material without its map", &mut live, &mut w);
}

/// By model file and material of it (`Retained::asset_materials`): the keys of the textures its slots name in the
/// pages, as `textures_named` has a world's materials'.
fn model_textures_named(c: &LiveCore) -> std::collections::BTreeMap<(String, usize), Vec<Option<String>>> {
    let keys = c.g.pages_state().0;
    let t = &c.sc.tables;
    t.retained.asset_materials.iter().map(|(k, &mi)| {
        let m = &t.materials[mi as usize];
        (k.clone(), [m.tex0[0], m.tex0[1], m.tex0[2], m.tex0[3], m.tex1[0], m.tex1[1]].iter().map(|&s| (s >= 0).then(|| keys[s as usize].0.clone())).collect())
    }).collect()
}

/// A still model the scene has not got comes in place (tests/render/data/textured-quads.glb: a quad with a picture
/// and a plain one, two materials of the file): its materials are entries after the scene's, its picture is placed
/// in the pages, its mesh is made in the room; no rebuild, and the view draws what a build of the edited world
/// draws. Another of it is another instance of that mesh. With the last of them gone the mesh is given back, and the
/// model comes again in place, its materials and its picture the ones there were.
#[test]
fn a_model_the_scene_has_not_got_comes_in_place() {
    let presets = crate::default_presets_dir();
    let open_at_root = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new(".."), &presets, 96, 54));
    const FILE: &str = "tests/render/data/textured-quads.glb";
    let model = || Kind::Model { uri: FILE.into(), nodes: None, animation: None, functions: vec![] };
    let mut w = world();
    let mut live = match open_at_root(&w) {
        Ok(c) if c.g.hwrt => c,
        Ok(_) => return eprintln!("skipped: no hardware RT"),
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let mut last = picture(&mut live, 24);
    let (materials, meshes) = (live.sc.tables.materials.len(), live.sc.meshes.len());
    assert_eq!((live.sc.up.kernel.features & gpu::F_TEX, live.g.pages_state().0.len()), (0, 0), "a scene without a texture");
    let mut check = |what: &str, live: &mut LiveCore, w: &mut World| {
        assert!(!live.commit(w).unwrap(), "{what}: in place ({:?})", live.fallback);
        let mut fresh = open_at_root(w).unwrap();
        // (A material of a model that has gone stays in the view: it names what a build's names where a build has it.)
        let named = model_textures_named(live);
        for (k, v) in model_textures_named(&fresh) {
            assert_eq!(named.get(&k), Some(&v), "{what}: the textures the model's material {k:?} names");
        }
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        assert!(apart(&last, &b) > 0.5, "{what}: the picture is {:.2} steps from the one before", apart(&last, &b));
        last = b;
    };
    let place = |w: &mut World, id: u32, at: Vec3| {
        w.set_object(id, format!("quads{id}"), None, model(), true, false).unwrap();
        w.set_transform(id, at, glam::Quat::from_rotation_y(0.3), Vec3::ONE).unwrap();
    };
    place(&mut w, 40, Vec3::new(-0.3, 0.2, 0.7));
    check("a model the scene has not got", &mut live, &mut w);
    let key = (FILE.to_string(), None);
    let mesh = *live.sc.tables.retained.model_mesh.get(&key).expect("the model's mesh");
    assert_eq!((live.sc.tables.materials.len(), live.sc.meshes.len(), live.g.pages_state().0.len()), (materials + 2, meshes + 1, 1), "two materials, a mesh and a picture more");
    assert_ne!(live.sc.up.kernel.features & gpu::F_TEX, 0, "the kernel's variant that samples textures");
    assert_eq!(live.sc.meshes[mesh as usize].sections(&live.sc.sections).len(), 2, "a section a material of the file");
    // Another of it: an instance of that mesh; nothing else is made.
    place(&mut w, 41, Vec3::new(0.3, -0.5, 0.9));
    check("another of the model", &mut live, &mut w);
    assert_eq!((live.sc.tables.materials.len(), live.sc.meshes.len(), live.sc.edit.users.get(&mesh)), (materials + 2, meshes + 1, Some(&2)));
    // Both gone: the mesh is given back.
    w.remove(40);
    w.remove(41);
    check("the models removed", &mut live, &mut w);
    assert!(!live.sc.tables.retained.model_mesh.contains_key(&key) && !live.sc.tables.retained.mesh_rows.contains_key(&mesh), "the mesh is no model's any more");
    // It comes again: its mesh made again (in the slot the other left), its materials and its picture there still.
    place(&mut w, 42, Vec3::new(0.2, 0.1, 0.8));
    check("the model again", &mut live, &mut w);
    assert_eq!((live.sc.tables.materials.len(), live.sc.meshes.len(), live.g.pages_state().0.len()), (materials + 2, meshes + 1, 1), "nothing made twice");
    // Another model the scene has not got, one of whose materials only emits (tests/render/data/panel.glb): its
    // emitters light the scene as a build's (the lamp away: the panel is what lights it).
    w.set_visible(12, false).unwrap();
    w.set_object(43, "panel43".into(), None, Kind::Model { uri: "tests/render/data/panel.glb".into(), nodes: None, animation: None, functions: vec![] }, true, false).unwrap();
    w.set_transform(43, Vec3::new(-0.5, -0.4, 0.6), glam::Quat::from_rotation_y(0.4), Vec3::splat(0.05)).unwrap();
    check("a model that emits", &mut live, &mut w);
    list_is_its_triangles("a model that emits", &live);
}

/// A kernel variant the device has not is made on another thread while the frames go on: the frame after the edit
/// that brings a feature (a material that is glass) runs the kernel the view had, and the view takes the scene's
/// once it is made. In lockstep a frame is its world's alone: it makes its kernel itself.
#[test]
fn a_new_kernel_variant_is_made_while_the_frames_go_on() {
    let presets = crate::default_presets_dir();
    let mat = |extra: serde_json::Value| -> crate::desc::Material {
        let mut m = serde_json::json!({ "name": "m", "color": [0.5, 0.5, 0.5], "opacity": 1, "metalness": 0, "roughness": 0.2, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true });
        m.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value(m).unwrap()
    };
    let mut w = world();
    let mut live = match gpu::Gpu::new().and_then(|g| LiveCore::new(g, &w, Path::new("."), &presets, 96, 54)) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    live.render(1, |_, _| {});
    let key = |k: &gpu::Kernel| (k.features, k.hash);
    let had = key(live.kernel.as_ref().expect("the view's kernel"));
    assert_eq!((had, had.0 & (gpu::F_GLASS | gpu::F_COAT)), (key(&live.sc.up.kernel), 0), "the scene's kernel, without glass or a clear coat");
    w.set_material(7, mat(serde_json::json!({ "transmission": 1 })));
    assert!(!live.commit(&mut w).unwrap(), "a new material: in place ({:?})", live.fallback);
    let wanted = key(&live.sc.up.kernel);
    assert_ne!(wanted.0 & gpu::F_GLASS, 0, "the scene's kernel has glass now");
    live.render(1, |_, _| {});
    assert_eq!(key(live.kernel.as_ref().unwrap()), had, "the frame after the edit ran the kernel the view had");
    let t = Instant::now();
    while key(live.kernel.as_ref().unwrap()) != wanted {
        assert!(t.elapsed().as_secs() < 120, "the scene's kernel was never taken");
        live.render(1, |_, _| {});
    }
    // In lockstep the next frame runs the scene's kernel, whatever it has to make for it.
    live.fixed_dt = Some(1.0 / 60.0);
    w.set_material(8, mat(serde_json::json!({ "clearcoat": 1 })));
    assert!(!live.commit(&mut w).unwrap(), "another new material: in place ({:?})", live.fallback);
    assert_ne!(live.sc.up.kernel.features & gpu::F_COAT, 0);
    live.render(1, |_, _| {});
    assert_eq!(key(live.kernel.as_ref().unwrap()), key(&live.sc.up.kernel), "in lockstep the frame ran the scene's kernel");
}

/// Material functions come and change in place, in a scene that had none: a function and a new material of it on an
/// instance (the kernel's code, `F_FUNC`, and the placement of a function's inputs by an instance, `F_IFUNC`), the
/// function's code changed, a second function whose name sorts before the first (a build numbers them by name, the
/// view in the order they came), a material that drops its function. After each: no rebuild, the kernel's features
/// are a build's, and the picture is that build's.
#[test]
fn material_functions_come_and_change_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let mat = |function: Option<&str>| -> crate::desc::Material {
        let shader = function.map(|id| serde_json::json!({ "id": id, "params": [], "textures": [] }));
        serde_json::from_value(serde_json::json!({ "name": "m", "color": [0.8, 0.8, 0.8], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true, "shader": shader })).unwrap()
    };
    let function = |name: &str, rgb: &str| format!("fn mf_{name}(i: MatIn, m: Material, base: MatOut) -> MatOut {{\n    var o = base;\n    o.base = vec3<f32>({rgb});\n    return o;\n}}\n");
    let mut w = world();
    w.set_material(6, mat(None));
    w.set_object(11, "panel".into(), None, Kind::Mesh { geometry: 3, material: 6, own_blas: true, materials: vec![] }, true, false).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    assert_eq!(live.sc.up.kernel.features & (gpu::F_FUNC | gpu::F_IFUNC), 0, "a scene without a function");
    let mut last = picture(&mut live, 24);
    let mut check = |what: &str, live: &mut LiveCore, w: &mut World| {
        assert!(!live.commit(w).unwrap(), "{what}: in place ({:?})", live.fallback);
        let mut fresh = open(w).unwrap();
        // (An instance of a function's material: under hardware traversal and with units. A view keeps the feature
        // once it has it.)
        let unkept = !(gpu::F_IFUNC | gpu::F_FUNC);
        assert_eq!(live.sc.up.kernel.features & unkept, fresh.sc.up.kernel.features & unkept, "{what}: the kernel's features");
        assert_eq!(fresh.sc.up.kernel.features & !live.sc.up.kernel.features, 0, "{what}: the view's kernel has what a build's has");
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        assert!(apart(&last, &b) > 0.5, "{what}: the picture is {:.2} steps from the one before", apart(&last, &b));
        last = b;
    };
    // A function, and a new material of it on the wall.
    w.set_shader("zz_red", function("zz_red", "0.9, 0.05, 0.05"), String::new());
    w.set_material(7, mat(Some("zz_red")));
    w.set_object(10, "wall".into(), None, Kind::Mesh { geometry: 2, material: 7, own_blas: true, materials: vec![] }, true, false).unwrap();
    check("a function and a material of it", &mut live, &mut w);
    assert_ne!(live.sc.up.kernel.features & gpu::F_FUNC, 0, "the kernel has functions");
    let first = live.sc.up.kernel.hash;
    // The same function again: nothing. Other code: another kernel.
    w.set_shader("zz_red", function("zz_red", "0.9, 0.05, 0.05"), String::new());
    assert!(!w.changes.any(), "a function set again as it is changes nothing");
    w.set_shader("zz_red", function("zz_red", "0.05, 0.05, 0.9"), String::new());
    check("the function's code changed", &mut live, &mut w);
    assert_ne!(live.sc.up.kernel.hash, first, "another module");
    // A second function, before the first by name: the panel's material given it.
    w.set_shader("aa_green", function("aa_green", "0.05, 0.9, 0.05"), String::new());
    w.set_material(6, mat(Some("aa_green")));
    check("a second function, on a material the scene had", &mut live, &mut w);
    assert_eq!((live.sc.tables.retained.shaders.get("zz_red"), live.sc.tables.retained.shaders.get("aa_green")), (Some(&0), Some(&1)), "numbered in the order they came");
    // A material that drops its function.
    w.set_material(7, mat(None));
    check("a material without its function", &mut live, &mut w);
}

/// Lights come, change and go in place: a point light with a shadow, a spot light without, the point light without
/// its shadow (another class: the table is sorted again), a rect light without a card, the point light removed, the
/// directional light removed, every light removed; a group that comes and goes draws nothing. After each: no
/// rebuild, the view's state (the light table, the kernel's counts of it) is a build's of the edited world, and so
/// is the picture.
#[test]
fn lights_come_and_go_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let light = |v: serde_json::Value| -> crate::desc::Light { serde_json::from_value(v).unwrap() };
    let point = |shadow: bool| light(serde_json::json!({ "type": "point", "color": [1.0, 0.6, 0.2], "castShadow": shadow, "intensity": 400.0, "radius": 0.02, "range": 0.0 }));
    let mut w = world();
    // (The lamp away: the lights light the scene.)
    w.set_visible(12, false).unwrap();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    let last = std::cell::RefCell::new(picture(&mut live, 24));
    let check = |what: &str, live: &mut LiveCore, w: &mut World, lights: usize| {
        assert!(!live.commit(w).unwrap(), "{what}: in place ({:?})", live.fallback);
        assert_eq!((live.sc.tables.lights.len(), live.sc.base.counts[0]), (lights, lights as u32), "{what}: the table's lights and the kernel's count of them");
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        same_state(what, live, &fresh);
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        assert!(apart(&last.borrow(), &b) > 0.1, "{what}: the picture is {:.2} steps from the one before", apart(&last.borrow(), &b));
        *last.borrow_mut() = b;
    };
    w.set_object(40, "point".into(), None, Kind::Light(point(true)), true, false).unwrap();
    w.set_transform(40, Vec3::new(-0.5, 0.3, 0.8), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    check("a point light", &mut live, &mut w, 2);
    w.set_object(41, "spot".into(), None, Kind::Light(light(serde_json::json!({ "type": "spot", "color": [0.2, 0.5, 1.0], "castShadow": false, "intensity": 900.0, "radius": 0.02, "range": 0.0, "angle": 0.5, "penumbra": 0.3 }))), true, false).unwrap();
    w.set_transform(41, Vec3::new(0.5, -0.2, 1.5), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    check("a spot light without a shadow", &mut live, &mut w, 3);
    w.set_light(40, point(false)).unwrap();
    assert!(!w.changes.structural, "a light of another class: its entry's change");
    check("the point light without its shadow", &mut live, &mut w, 3);
    w.set_object(42, "rect".into(), None, Kind::Light(light(serde_json::json!({ "type": "rect", "color": [1.0, 1.0, 1.0], "castShadow": true, "intensity": 3000.0, "width": 0.4, "height": 0.4, "visible": false }))), true, false).unwrap();
    w.set_transform(42, Vec3::new(0.0, 0.7, 1.2), glam::Quat::from_rotation_x(-0.8), Vec3::ONE).unwrap();
    check("a rect light without a card", &mut live, &mut w, 4);
    // A group that comes and goes: nothing to do.
    w.set_object(50, "group".into(), None, Kind::Group, true, false).unwrap();
    assert!(!w.changes.structural && !live.commit(&mut w).unwrap(), "a new group: nothing rebuilt");
    w.remove(50);
    assert!(!w.changes.structural && !live.commit(&mut w).unwrap(), "the group removed: nothing rebuilt");
    w.remove(40);
    check("the point light removed", &mut live, &mut w, 3);
    w.remove(20);
    check("the directional light removed", &mut live, &mut w, 2);
    w.remove(41);
    w.remove(42);
    check("every light removed", &mut live, &mut w, 0);
    // One again, in a scene without a light.
    w.set_object(43, "point".into(), None, Kind::Light(point(true)), true, false).unwrap();
    w.set_transform(43, Vec3::new(0.3, 0.3, 0.9), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    check("a light in a scene without one", &mut live, &mut w, 1);
    // As many shadow-casting local lights as the kernel resamples (the light tiles): they come with a rebuild; one
    // more and one fewer among them are in place again.
    for k in 0..gpu::RIS_EXACT as u32 {
        w.set_object(60 + k, format!("p{k}"), None, Kind::Light(point(true)), true, false).unwrap();
        w.set_transform(60 + k, Vec3::new(-0.8 + 0.2 * k as f32, -0.6, 0.7), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
    }
    if gpu::light_tiles_on() {
        assert!(live.commit(&mut w).unwrap() && live.sc.up.kernel.features & gpu::F_LISTS != 0, "the light tiles come with a rebuild ({:?})", live.fallback);
        *last.borrow_mut() = picture(&mut live, 24);
        w.set_object(70, "more".into(), None, Kind::Light(point(true)), true, false).unwrap();
        w.set_transform(70, Vec3::new(0.0, 0.7, 0.8), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        check("a light among those resampled", &mut live, &mut w, gpu::RIS_EXACT + 2);
        w.remove(62);
        check("one of them removed", &mut live, &mut w, gpu::RIS_EXACT + 1);
    }
}

/// A translucent particle's material given another function, in place: the translucency pass has the functions of
/// its particles' materials alone, so the kernel is made again with the one the material has now. First a function
/// the scene had (an opaque material's: no function's code changes), then one that comes with the edit. After
/// each: no rebuild, and the picture is a build's of the edited world.
#[test]
fn a_translucent_particles_material_takes_another_function_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let glow = |function: &str| -> crate::desc::Material {
        serde_json::from_value(serde_json::json!({ "name": "glow", "color": [0.5, 0.5, 0.5], "opacity": 0.9, "transparent": true, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true,
            "shader": { "id": function, "params": [], "textures": [] } })).unwrap()
    };
    let function = |name: &str, rgb: &str| format!("fn mf_{name}(i: MatIn, m: Material, base: MatOut) -> MatOut {{\n    var o = base;\n    o.base = vec3<f32>({rgb});\n    o.emissive = vec3<f32>({rgb}) * 40.0;\n    return o;\n}}\n");
    let mut w = wall_world(3.0, serde_json::json!({ "name": "m", "color": [0.5, 0.5, 0.5], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true, "shader": { "id": "fb", "params": [], "textures": [] } }));
    let sun = serde_json::from_value(serde_json::json!({ "type": "directional", "color": [1.0, 1.0, 1.0], "castShadow": true, "intensity": 1.0, "angularDiameter": 0.5 })).unwrap();
    w.set_object(20, "sun".into(), None, Kind::Light(sun), true, false).unwrap();
    w.set_shader("fa", function("fa", "0.9, 0.05, 0.05"), String::new());
    w.set_shader("fb", function("fb", "0.05, 0.9, 0.05"), String::new());
    // (A small mesh of the opaque material with `fb`, out of the particle's way: the scene has both functions.)
    w.set_object(11, "tile".into(), None, Kind::Mesh { geometry: 1, material: 3, own_blas: false, materials: vec![] }, true, false).unwrap();
    w.set_transform(11, Vec3::new(-0.8, -0.8, 0.2), glam::Quat::IDENTITY, Vec3::splat(0.1)).unwrap();
    w.set_material(5, glow("fa"));
    w.set_particle_fn("card", "fn pf_card(i: PIn, p: array<vec4<f32>, 8>) -> POut { return POut(vec3<f32>(0.0, 0.0, 0.5), vec3<f32>(0.5, 0.0, 0.0), vec3<f32>(0.0, 0.5, 0.0), true, vec4<f32>(0.0)); }".into(), String::new());
    w.set_object(30, "card".into(), None, Kind::Particles { count: 1, emitter: "card".into(), params: vec![], material: 5, emitter_area: 0.0 }, true, false).unwrap();
    let mut live = match open(&w) {
        Ok(c) if c.g.hwrt => c,
        Ok(_) => return eprintln!("skipped: no hardware RT"),
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    assert!(live.sc.up.translucent.is_some(), "the particle is the translucency pass's");
    let mut last = picture(&mut live, 24);
    let mut check = |what: &str, live: &mut LiveCore, w: &mut World| {
        assert!(!live.commit(w).unwrap(), "{what}: in place ({:?})", live.fallback);
        let mut fresh = open(w).unwrap();
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        assert!(apart(&last, &b) > 0.5, "{what}: the picture is {:.2} steps from the one before", apart(&last, &b));
        last = b;
    };
    let before = live.sc.up.kernel.hash;
    w.set_material(5, glow("fb"));
    assert!(!w.changes.functions && !w.changes.structural, "a material given a function the scene has: the material's change alone");
    check("a function the scene had", &mut live, &mut w);
    assert_ne!(live.sc.up.kernel.hash, before, "another kernel: the translucency pass's functions are others");
    w.set_shader("fc", function("fc", "0.05, 0.05, 0.9"), String::new());
    w.set_material(5, glow("fc"));
    check("a function that comes with the edit", &mut live, &mut w);
}

/// Volumes come, change and go in place, in a scene that had none: a box of haze (a volume function the scene had
/// not, the scene's first medium), a homogeneous glowing sphere whose function's name sorts before the first's (a
/// build numbers them by name, the view in the order they came), a third (the table grows), the first set again
/// with other parameters, moved where it stood still, the second hidden (the first set again beside it) and shown, each removed. After each: no
/// rebuild, the kernel's count of media and its features are a build's of the edited world, the media are that
/// build's nodes, and so is the picture.
#[test]
fn volumes_come_and_go_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let function = |name: &str, body: &str| format!("fn vf_{name}(i: VIn, p: array<vec4<f32>, 8>) -> VOut {{\n    var o = VOut(0.0, vec3<f32>(1.0), vec3<f32>(0.0));\n    {body}\n    return o;\n}}\n");
    let volume = |function: &str, shape: u32, majorant: f32, homogeneous: bool, params: Vec<f32>| Kind::Volume { function: function.into(), params, shape, majorant, g: 0.0, homogeneous, animated: false, grid: None };
    let mut w = world();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    assert_eq!((live.sc.up.kernel.features & gpu::F_MEDIA, live.sc.base.time[3]), (0, 0.0), "a scene without a medium");
    let last = std::cell::RefCell::new(picture(&mut live, 24));
    let check = |what: &str, live: &mut LiveCore, w: &mut World, nodes: &[u32]| {
        live.lit_changed = 0;
        assert!(!live.commit(w).unwrap(), "{what}: in place ({:?})", live.fallback);
        assert!(live.live_lighting(), "{what}: the lighting changed, as after a rebuild (the histories are held short)");
        let mut fresh = open(w).unwrap();
        assert_eq!((&live.sc.up.media_nodes[..], live.sc.base.time[3]), (nodes, nodes.len() as f32), "{what}: the media and the kernel's count of them");
        assert_eq!((&fresh.sc.up.media_nodes[..], fresh.sc.base.time[3]), (nodes, nodes.len() as f32), "{what}: a build's");
        assert_eq!(*live.sc.up.media_hidden.borrow(), *fresh.sc.up.media_hidden.borrow(), "{what}: which are hidden");
        let media = gpu::F_MEDIA | gpu::F_HMEDIA | gpu::F_GRID;
        assert_eq!(live.sc.up.kernel.features & media, fresh.sc.up.kernel.features & media, "{what}: the kernel's media features");
        forget(live);
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        assert!(apart(&last.borrow(), &b) > 0.1, "{what}: the picture is {:.2} steps from the one before", apart(&last.borrow(), &b));
        *last.borrow_mut() = b;
    };
    // The scene's first medium, of a function it had not.
    w.set_volume_fn("haze", function("haze", "o.extinction = p[0].x; o.albedo = vec3<f32>(0.9, 0.6, 0.3);"), String::new());
    w.set_object(40, "haze".into(), None, volume("haze", 0, 4.0, false, vec![2.5]), true, false).unwrap();
    w.set_transform(40, Vec3::new(-0.3, 0.0, 1.2), glam::Quat::IDENTITY, Vec3::splat(0.9)).unwrap();
    check("the scene's first volume", &mut live, &mut w, &[40]);
    assert_ne!(live.sc.up.kernel.features & gpu::F_MEDIA, 0);
    // A homogeneous sphere that glows; its function before the first by name.
    w.set_volume_fn("aglow", function("aglow", "o.extinction = 1.0; o.albedo = vec3<f32>(0.2); o.emission = vec3<f32>(0.2, 1.5, 3.0);"), String::new());
    w.set_object(41, "glow".into(), None, volume("aglow", 1, 1.0, true, vec![]), true, false).unwrap();
    w.set_transform(41, Vec3::new(0.6, -0.3, 1.0), glam::Quat::IDENTITY, Vec3::splat(0.7)).unwrap();
    check("a second volume, homogeneous", &mut live, &mut w, &[40, 41]);
    assert_ne!(live.sc.up.kernel.features & gpu::F_HMEDIA, 0);
    // A third: more than the table had room for (it is another buffer now, which the kernel reads).
    let room = live.sc.up.heap.cap(crate::heap::Table::Media);
    w.set_object(42, "more haze".into(), None, volume("haze", 0, 4.0, false, vec![3.0]), true, false).unwrap();
    w.set_transform(42, Vec3::new(0.0, 0.55, 1.6), glam::Quat::IDENTITY, Vec3::splat(0.5)).unwrap();
    check("a third volume", &mut live, &mut w, &[40, 41, 42]);
    assert!(live.sc.up.heap.cap(crate::heap::Table::Media) > room, "the table grew");
    w.remove(42);
    check("the third volume removed", &mut live, &mut w, &[40, 41]);
    // The first set again: thinner.
    w.set_object(40, "haze".into(), None, volume("haze", 0, 4.0, false, vec![0.6]), true, false).unwrap();
    assert!(!w.changes.structural, "a volume set again: the media table's");
    check("the first volume set again", &mut live, &mut w, &[40, 41]);
    // Moved, where it stood still.
    w.set_transform(40, Vec3::new(0.2, 0.4, 1.4), glam::Quat::from_rotation_y(0.5), Vec3::splat(0.9)).unwrap();
    assert!(!w.changes.structural && w.changes.promoted.is_empty(), "a still volume's move: the media table's");
    check("the first volume moved", &mut live, &mut w, &[40, 41]);
    // Hidden and shown: its entry keeps its place.
    w.set_visible(41, false).unwrap();
    check("the second volume hidden", &mut live, &mut w, &[40, 41]);
    // The first set again beside it: the table is made whole, the hidden one without its shape still.
    w.set_object(40, "haze".into(), None, volume("haze", 0, 4.0, false, vec![1.8]), true, false).unwrap();
    check("a volume set again beside a hidden one", &mut live, &mut w, &[40, 41]);
    w.set_visible(41, true).unwrap();
    check("the second volume shown", &mut live, &mut w, &[40, 41]);
    // The function's code changed.
    w.set_volume_fn("haze", function("haze", "o.extinction = p[0].x * 3.0; o.albedo = vec3<f32>(0.3, 0.6, 0.9);"), String::new());
    check("a volume function's code changed", &mut live, &mut w, &[40, 41]);
    // Each removed.
    w.remove(40);
    check("the first volume removed", &mut live, &mut w, &[41]);
    w.remove(41);
    check("every volume removed", &mut live, &mut w, &[]);
    assert_eq!(live.sc.up.kernel.features & (gpu::F_MEDIA | gpu::F_HMEDIA), 0, "no medium: the kernel without its media code");
}

/// Density grids come, change and go in place with the volumes that name them, in a scene that had none: a volume
/// of a grid the scene had not (the atlas made), a second grid of other cells (the atlas made again, both stacked),
/// the first with another size, new data; a grid with a program (the grids' pass made), its parameters (the same
/// grid still: nothing is made again), another grid beside it and the program's code changed (what it had simulated
/// stays), a program whose name comes first (the programs' numbers move), a simulated grid's volume moved (its
/// program runs in the box as it is now), a second and a third volume of one grid and the first of them removed
/// each time (the grid is the next one's, its program runs in that one's box), every one removed. After each: no rebuild; the
/// grids, the atlas, the media and the kernel's media features are a build's of the edited world, and so is the
/// picture.
#[test]
fn density_grids_come_and_change_in_place() {
    let presets = crate::default_presets_dir();
    let open = |w: &World| gpu::Gpu::new().and_then(|g| LiveCore::new(g, w, Path::new("."), &presets, 96, 54));
    let volume = |grid: u32, density: f32| Kind::Volume { function: "smoke".into(), params: vec![density], shape: 0, majorant: 8.0, g: 0.0, homogeneous: false, animated: false, grid: Some(grid) };
    let place = |w: &mut World, id: u32, grid: u32, density: f32, at: Vec3, size: f32| {
        w.set_object(id, format!("smoke {id}"), None, volume(grid, density), true, false).unwrap();
        w.set_transform(id, at, glam::Quat::IDENTITY, Vec3::splat(size)).unwrap();
    };
    let mut w = world();
    let mut live = match open(&w) {
        Ok(c) => c,
        Err(e) => return eprintln!("skipped: {e}"),
    };
    w.take_changes();
    assert_eq!(live.g.grids_state(&live.sc.up).0, vec![], "a scene without a grid");
    let last = std::cell::RefCell::new(picture(&mut live, 24));
    // (The commit alone: what the grids hold before a frame runs their programs.)
    let commit = |what: &str, live: &mut LiveCore, w: &mut World| {
        // (An edit of the scene, not a grid's new data alone, which is uploaded: the lighting changed, as after a
        // rebuild, and the histories are held short.)
        let edit = w.changes.any();
        live.lit_changed = 0;
        assert!(!live.commit(w).unwrap(), "{what}: in place ({:?})", live.fallback);
        assert_eq!(live.live_lighting(), edit, "{what}: whether the lighting changed");
    };
    let check = |what: &str, live: &mut LiveCore, w: &mut World, nodes: &[u32]| {
        commit(what, live, w);
        let mut fresh = open(w).unwrap();
        assert_eq!(live.g.grids_state(&live.sc.up), fresh.g.grids_state(&fresh.sc.up), "{what}: the grids and their atlas");
        assert_eq!((&live.sc.up.media_nodes[..], live.sc.base.time[3]), (nodes, nodes.len() as f32), "{what}: the media and the kernel's count of them");
        assert_eq!(&fresh.sc.up.media_nodes[..], nodes, "{what}: a build's");
        let media = gpu::F_MEDIA | gpu::F_HMEDIA | gpu::F_GRID;
        assert_eq!(live.sc.up.kernel.features & media, fresh.sc.up.kernel.features & media, "{what}: the kernel's media features");
        assert_eq!(gpu::Gpu::has_grid_sim(&live.sc.up), gpu::Gpu::has_grid_sim(&fresh.sc.up), "{what}: whether the grids' pass runs");
        // (The programs run to where they stay in both, then the pictures: a build's grids start from their data.)
        for c in [&mut *live, &mut fresh] {
            picture(c, 48);
            forget(c);
        }
        let (a, b) = (picture(live, 48), picture(&mut fresh, 48));
        assert!(apart(&a, &b) < 0.5, "{what}: the picture is {:.2} steps from a build's", apart(&a, &b));
        assert!(apart(&last.borrow(), &b) > 0.1, "{what}: the picture is {:.2} steps from the one before", apart(&last.borrow(), &b));
        *last.borrow_mut() = b;
    };
    let program = |name: &str, body: &str| format!("fn gp_{name}(s_in: Cell, i: CIn, p: array<vec4<f32>, 8>) -> Cell {{\n    var s = s_in;\n    {body}\n    return s;\n}}\n");
    let cells = |dims: [u32; 3], f: &dyn Fn([u32; 3]) -> f32| -> Vec<f32> { (0..dims[2]).flat_map(|z| (0..dims[1]).flat_map(move |y| (0..dims[0]).map(move |x| [x, y, z]))).map(f).collect() };
    // The scene's first grid: dense in its left half.
    w.set_volume_fn("smoke", "fn vf_smoke(i: VIn, p: array<vec4<f32>, 8>) -> VOut {\n    var o = VOut(0.0, vec3<f32>(0.9, 0.6, 0.3), vec3<f32>(0.0));\n    o.extinction = p[0].x * vgrid(i, i.lp).x;\n    return o;\n}\n".into(), String::new());
    w.set_grid(1, [4, 4, 4], 1, Some(&cells([4, 4, 4], &|c| (c[0] < 2) as u32 as f32)), None).unwrap();
    place(&mut w, 40, 1, 4.0, Vec3::new(-0.5, 0.0, 1.2), 0.9);
    assert!(!w.changes.structural, "a new grid and its volume: the media table's");
    check("the scene's first grid", &mut live, &mut w, &[40]);
    assert_ne!(live.sc.up.kernel.features & gpu::F_GRID, 0);
    assert_eq!(live.g.grid_cell(&live.sc.up, 1, [1, 2, 3]), [1.0, 0.0, 0.0, 0.0], "its data");
    assert_eq!(live.g.grid_cell(&live.sc.up, 1, [2, 2, 3]), [0.0; 4]);
    // A second, of other cells: the atlas is wider now, the first below it.
    w.set_grid(2, [8, 2, 2], 1, Some(&[1.0; 32]), None).unwrap();
    place(&mut w, 41, 2, 3.0, Vec3::new(0.6, -0.3, 1.0), 0.7);
    check("a second grid", &mut live, &mut w, &[40, 41]);
    assert_eq!(live.g.grids_state(&live.sc.up).1, [8, 4, 6], "an atlas of both");
    assert_eq!(live.g.grid_cell(&live.sc.up, 1, [1, 2, 3]), [1.0, 0.0, 0.0, 0.0], "the first's data in the new atlas");
    // The first with another size: dense in its upper half.
    w.set_grid(1, [2, 6, 2], 1, Some(&cells([2, 6, 2], &|c| (c[1] >= 3) as u32 as f32)), None).unwrap();
    check("a grid of another size", &mut live, &mut w, &[40, 41]);
    // New data: uploaded, nothing made again.
    w.set_grid(2, [8, 2, 2], 1, Some(&[0.1; 32]), None).unwrap();
    assert!(!w.changes.media && w.grid_dirty.contains(&2));
    check("new data", &mut live, &mut w, &[40, 41]);
    // A grid with a program: it fills up to its first parameter.
    w.set_grid_program("fill", program("fill", "s.a.x = clamp(s.a.x + 0.05, p[0].y, p[0].x);"), String::new());
    w.set_grid(3, [4, 4, 4], 1, None, Some(("fill".into(), vec![1.0, 0.0]))).unwrap();
    place(&mut w, 42, 3, 3.0, Vec3::new(0.1, 0.5, 1.4), 0.6);
    check("a grid with a program", &mut live, &mut w, &[40, 41, 42]);
    assert!(gpu::Gpu::has_grid_sim(&live.sc.up));
    assert_eq!(live.g.grid_cell(&live.sc.up, 3, [1, 1, 1])[0], 1.0, "filled");
    // Its parameters: the same grid still.
    w.set_grid(3, [4, 4, 4], 1, None, Some(("fill".into(), vec![2.0, 0.0]))).unwrap();
    commit("a program's parameters", &mut live, &mut w);
    assert_eq!(live.g.grid_cell(&live.sc.up, 3, [1, 1, 1])[0], 1.0, "what it had simulated stays");
    check("a program's parameters", &mut live, &mut w, &[40, 41, 42]);
    assert_eq!(live.g.grid_cell(&live.sc.up, 3, [1, 1, 1])[0], 2.0, "filled to the new parameter");
    // Another grid beside it: all are made again, and what the program had simulated goes with its grid.
    w.set_grid(4, [2, 2, 2], 1, Some(&[1.0; 8]), None).unwrap();
    place(&mut w, 43, 4, 4.0, Vec3::new(-0.6, -0.5, 1.6), 0.5);
    commit("another grid beside a simulated one", &mut live, &mut w);
    assert_eq!(live.g.grid_cell(&live.sc.up, 3, [1, 1, 1])[0], 2.0, "what it had simulated is in the new atlas");
    check("another grid beside a simulated one", &mut live, &mut w, &[40, 41, 42, 43]);
    // The program's code: it empties now (down to its second parameter).
    w.set_grid_program("fill", program("fill", "s.a.x = clamp(s.a.x - 0.05, p[0].y, p[0].x);"), String::new());
    assert!(!w.changes.structural, "a grid program's code: the media table's");
    commit("a program's code", &mut live, &mut w);
    assert_eq!(live.g.grid_cell(&live.sc.up, 3, [1, 1, 1])[0], 2.0, "what it had simulated stays");
    check("a program's code", &mut live, &mut w, &[40, 41, 42, 43]);
    assert_eq!(live.g.grid_cell(&live.sc.up, 3, [1, 1, 1])[0], 0.0, "emptied");
    // A program whose name comes first (the others' numbers move), and the grid's floor raised: it is its own
    // program that runs on it still.
    w.set_grid_program("an_earlier", program("an_earlier", "s.a.x = 7.0;"), String::new());
    w.set_grid(3, [4, 4, 4], 1, None, Some(("fill".into(), vec![2.0, 1.5]))).unwrap();
    check("a program whose name comes first", &mut live, &mut w, &[40, 41, 42, 43]);
    assert_eq!(live.g.grid_cell(&live.sc.up, 3, [1, 1, 1])[0], 1.5, "its own program");
    // A program that reads the world (dense where x > 0.3 m), then its volume moved, where it stood still.
    let row = |live: &LiveCore| [0, 1, 2, 3].map(|x| live.g.grid_cell(&live.sc.up, 3, [x, 1, 1])[0]);
    w.set_grid_program("fill", program("fill", "s.a.x = select(p[0].y, p[0].x, i.p.x > 0.3);"), String::new());
    w.set_grid(3, [4, 4, 4], 1, None, Some(("fill".into(), vec![2.0, 0.0]))).unwrap();
    check("a program that reads the world", &mut live, &mut w, &[40, 41, 42, 43]);
    assert_eq!(row(&live), [0.0, 0.0, 0.0, 2.0], "the box's last quarter is past x = 0.3");
    w.set_transform(42, Vec3::new(0.6, 0.5, 1.4), glam::Quat::IDENTITY, Vec3::splat(0.6)).unwrap();
    assert!(!w.changes.structural && w.changes.promoted.is_empty(), "a still volume's move: the media table's");
    check("a simulated grid's volume moved", &mut live, &mut w, &[40, 41, 42, 43]);
    assert_eq!(row(&live), [2.0; 4], "its program runs in the box as it is now");
    // A second volume of the simulated grid, then the first of them removed: the grid is the other's now, and its
    // program runs in the other's box (half of it past x = 0.3).
    place(&mut w, 44, 3, 3.0, Vec3::new(0.3, -0.55, 1.8), 0.5);
    check("a second volume of one grid", &mut live, &mut w, &[40, 41, 42, 43, 44]);
    assert_eq!(row(&live), [2.0; 4], "the first's box still");
    w.remove(42);
    commit("the first volume of a grid removed", &mut live, &mut w);
    assert_eq!(row(&live), [2.0; 4], "what it had simulated stays");
    check("the first volume of a grid removed", &mut live, &mut w, &[40, 41, 43, 44]);
    assert_eq!(row(&live), [0.0, 0.0, 2.0, 2.0], "the other's box");
    // A third volume of it after the second, and the second removed: the grids lie as they did, and the grid is the
    // third's (its box is all past x = 0.3).
    place(&mut w, 45, 3, 3.0, Vec3::new(0.75, 0.0, 1.9), 0.4);
    check("a third volume of one grid", &mut live, &mut w, &[40, 41, 43, 44, 45]);
    assert_eq!(row(&live), [0.0, 0.0, 2.0, 2.0], "the second's box still");
    w.remove(44);
    check("a grid's volume removed, the grids lying as they did", &mut live, &mut w, &[40, 41, 43, 45]);
    assert_eq!(row(&live), [2.0; 4], "the third's box");
    // The simulated grid's last volume removed: no program runs.
    w.remove(45);
    check("the simulated grid's volume removed", &mut live, &mut w, &[40, 41, 43]);
    assert!(!gpu::Gpu::has_grid_sim(&live.sc.up));
    // Every one removed.
    for id in [40, 41, 43] {
        w.remove(id);
    }
    check("every volume removed", &mut live, &mut w, &[]);
    assert_eq!((live.g.grids_state(&live.sc.up).0, live.sc.up.kernel.features & (gpu::F_MEDIA | gpu::F_GRID)), (vec![], 0), "no grid: the kernel without its media code");
}
