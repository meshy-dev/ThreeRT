//! glTF models, posed: a clip sampled at a time (node translation / rotation / scale channels with step, linear or
//! cubic-spline interpolation) and linear blend skinning on the CPU (UE: a skeletal mesh with an animation sequence).
//! `Model::triangles` yields a model's triangles in world space for a pose, in the same order every time, so a posed
//! model can be re-skinned in place (`tables.rs`, `LiveCore::commit`).

use crate::desc;
use anyhow::{anyhow, Result};
use glam::{Mat3, Mat4, Quat, Vec2, Vec3};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub type Gltf = (gltf::Document, Vec<gltf::buffer::Data>, Vec<gltf::image::Data>);

/// One triangle primitive, decoded once.
struct Prim {
    node: usize,
    material: Option<usize>,
    positions: Vec<Vec3>,
    normals: Option<Vec<Vec3>>,
    uvs: Vec<Vec2>,
    tangents: Option<Vec<[f32; 4]>>,
    indices: Vec<u32>,
    /// Joint indices (into the node's skin) and weights, for skinned primitives.
    skin: Option<(Vec<[u16; 4]>, Vec<[f32; 4]>)>,
}

#[derive(Clone, Copy)]
enum Path {
    Translation,
    Rotation,
    Scale,
}

struct Channel {
    node: usize,
    path: Path,
    interpolation: gltf::animation::Interpolation,
    times: Vec<f32>,
    /// xyz (translation / scale) or xyzw (rotation); cubic splines store (in-tangent, value, out-tangent) per key.
    values: Vec<[f32; 4]>,
}

/// An animation clip.
pub struct Clip {
    channels: Vec<Channel>,
    pub duration: f32,
}

/// A glTF file with its primitives decoded and its clips (on first use).
pub struct Model {
    pub gltf: Gltf,
    /// In the order the renderer emits them (depth first, as before posing existed).
    prims: Vec<Prim>,
    parents: Vec<Option<usize>>,
    clips: Mutex<HashMap<String, Arc<Clip>>>,
}

/// A triangle of a posed model, in world space.
pub struct ModelTri {
    pub p: [Vec3; 3],
    pub n: [Vec3; 3],
    pub uv: [Vec2; 3],
    pub tangent: Option<(Vec3, f32)>,
    /// glTF material index.
    pub material: Option<usize>,
}

impl Model {
    pub fn new(gltf: Gltf) -> Result<Model> {
        let (doc, bufs, _) = &gltf;
        let scene = doc.default_scene().or_else(|| doc.scenes().next()).ok_or_else(|| anyhow!("glb without scene"))?;
        let mut parents = vec![None; doc.nodes().len()];
        for n in doc.nodes() {
            for c in n.children() {
                parents[c.index()] = Some(n.index());
            }
        }
        // Same depth-first order as before posing existed (stack, last root first), so static renders are unchanged.
        let mut order = vec![];
        let mut stack: Vec<gltf::Node> = scene.nodes().collect();
        while let Some(node) = stack.pop() {
            stack.extend(node.children());
            order.push(node.index());
        }
        let mut prims = vec![];
        for &ni in &order {
            let node = doc.nodes().nth(ni).unwrap();
            let Some(mesh) = node.mesh() else { continue };
            for prim in mesh.primitives() {
                if prim.mode() != gltf::mesh::Mode::Triangles {
                    continue;
                }
                let r = prim.reader(|b| Some(&bufs[b.index()]));
                let positions: Vec<Vec3> = r.read_positions().ok_or_else(|| anyhow!("no positions"))?.map(Vec3::from).collect();
                let n = positions.len();
                let skin = match (node.skin(), r.read_joints(0), r.read_weights(0)) {
                    (Some(_), Some(j), Some(w)) => Some((j.into_u16().collect(), w.into_f32().collect())),
                    _ => None,
                };
                prims.push(Prim {
                    node: ni,
                    material: prim.material().index(),
                    normals: r.read_normals().map(|it| it.map(Vec3::from).collect()),
                    uvs: r.read_tex_coords(0).map(|t| t.into_f32().map(Vec2::from).collect()).unwrap_or_else(|| vec![Vec2::ZERO; n]),
                    tangents: r.read_tangents().map(|t| t.collect()),
                    indices: r.read_indices().map(|i| i.into_u32().collect()).unwrap_or_else(|| (0..n as u32).collect()),
                    positions,
                    skin,
                });
            }
        }
        Ok(Model { gltf, prims, parents, clips: Mutex::new(HashMap::new()) })
    }

    /// Whether the file can be posed (has skins or animations).
    pub fn animated(&self) -> bool {
        self.gltf.0.animations().len() > 0 || self.gltf.0.skins().len() > 0
    }

    pub fn clip_names(&self) -> Vec<String> {
        self.gltf.0.animations().enumerate().map(|(i, a)| a.name().map_or_else(|| format!("#{i}"), str::to_string)).collect()
    }

    fn clip(&self, name: &str) -> Result<Arc<Clip>> {
        if let Some(c) = self.clips.lock().unwrap().get(name) {
            return Ok(c.clone());
        }
        let (doc, bufs, _) = &self.gltf;
        let anim = doc
            .animations()
            .enumerate()
            .find(|(i, a)| a.name() == Some(name) || format!("#{i}") == name)
            .map(|(_, a)| a)
            .ok_or_else(|| anyhow!("no animation clip \"{name}\" (clips: {})", self.clip_names().join(", ")))?;
        let mut channels = vec![];
        let mut duration = 0f32;
        for ch in anim.channels() {
            let r = ch.reader(|b| Some(&bufs[b.index()]));
            let (Some(inputs), Some(outputs)) = (r.read_inputs(), r.read_outputs()) else { continue };
            let times: Vec<f32> = inputs.collect();
            use gltf::animation::util::ReadOutputs;
            let (path, values): (Path, Vec<[f32; 4]>) = match outputs {
                ReadOutputs::Translations(t) => (Path::Translation, t.map(|v| [v[0], v[1], v[2], 0.0]).collect()),
                ReadOutputs::Rotations(q) => (Path::Rotation, q.into_f32().collect()),
                ReadOutputs::Scales(s) => (Path::Scale, s.map(|v| [v[0], v[1], v[2], 0.0]).collect()),
                ReadOutputs::MorphTargetWeights(_) => continue,
            };
            duration = duration.max(times.last().copied().unwrap_or(0.0));
            channels.push(Channel { node: ch.target().node().index(), path, interpolation: ch.sampler().interpolation(), times, values });
        }
        let c = Arc::new(Clip { channels, duration });
        self.clips.lock().unwrap().insert(name.to_string(), c.clone());
        Ok(c)
    }

    /// Node transforms relative to the glTF scene root: rest pose, or the clip at the pose's time.
    fn globals(&self, pose: Option<&desc::Animation>) -> Result<Vec<Mat4>> {
        let doc = &self.gltf.0;
        let mut local: Vec<(Vec3, Quat, Vec3)> = doc
            .nodes()
            .map(|n| {
                let (t, r, s) = n.transform().decomposed();
                (Vec3::from(t), Quat::from_array(r), Vec3::from(s))
            })
            .collect();
        if let Some(a) = pose {
            let clip = self.clip(&a.clip)?;
            let t = if clip.duration <= 0.0 { 0.0 } else if a.r#loop { a.time.rem_euclid(clip.duration) } else { a.time.clamp(0.0, clip.duration) };
            for ch in &clip.channels {
                let v = sample(ch, t);
                let l = &mut local[ch.node];
                match ch.path {
                    Path::Translation => l.0 = Vec3::new(v[0], v[1], v[2]),
                    Path::Rotation => l.1 = Quat::from_array(v).normalize(),
                    Path::Scale => l.2 = Vec3::new(v[0], v[1], v[2]),
                }
            }
        }
        let mut global: Vec<Option<Mat4>> = vec![None; local.len()];
        fn resolve(i: usize, local: &[(Vec3, Quat, Vec3)], parents: &[Option<usize>], global: &mut Vec<Option<Mat4>>) -> Mat4 {
            if let Some(m) = global[i] {
                return m;
            }
            let (t, r, s) = local[i];
            let m = Mat4::from_scale_rotation_translation(s, r, t);
            let m = match parents[i] {
                Some(p) => resolve(p, local, parents, global) * m,
                None => m,
            };
            global[i] = Some(m);
            m
        }
        Ok((0..local.len()).map(|i| resolve(i, &local, &self.parents, &mut global)).collect())
    }

    /// Whether no primitive is skinned: every triangle moves with its node alone (rigid node animation).
    pub fn rigid(&self) -> bool {
        self.prims.iter().all(|p| p.skin.is_none())
    }

    /// Per node: whether clip `clip` moves it (a channel targets it or one of its ancestors).
    pub fn moving_nodes(&self, clip: &str) -> Result<Vec<bool>> {
        let c = self.clip(clip)?;
        let mut direct = vec![false; self.parents.len()];
        for ch in &c.channels {
            direct[ch.node] = true;
        }
        Ok((0..self.parents.len())
            .map(|i| {
                let mut k = Some(i);
                while let Some(j) = k {
                    if direct[j] {
                        return true;
                    }
                    k = self.parents[j];
                }
                false
            })
            .collect())
    }

    /// Node transforms relative to the glTF scene root for `pose` (None = rest pose).
    pub fn node_globals(&self, pose: Option<&desc::Animation>) -> Result<Vec<Mat4>> {
        self.globals(pose)
    }

    /// A rigid model's triangles of the nodes `keep` admits, with each one's node: in world space for `pose`
    /// (`world` x the node's global transform), or in the node's own space (`local`: the caller places them per node).
    /// Same order as `triangles`. Mirrored placements keep the winding front-facing in world space, as `triangles`
    /// does; in local space `mirrored(node)` says which nodes will be placed mirrored.
    pub fn node_triangles(&self, world: Mat4, pose: Option<&desc::Animation>, keep: &dyn Fn(usize) -> bool, local: bool, mirrored: &dyn Fn(usize) -> bool) -> Result<Vec<(usize, ModelTri)>> {
        let globals = self.globals(pose)?;
        let mut out = vec![];
        for p in self.prims.iter().filter(|p| keep(p.node)) {
            let m = if local { Mat4::IDENTITY } else { world * globals[p.node] };
            let nm = Mat3::from_mat4(m).inverse().transpose();
            let pos: Vec<Vec3> = p.positions.iter().map(|v| m.transform_point3(*v)).collect();
            let nrm: Option<Vec<Vec3>> = p.normals.as_ref().map(|n| n.iter().map(|v| (nm * *v).normalize_or_zero()).collect());
            let flip = if local { mirrored(p.node) } else { m.determinant() < 0.0 };
            for t in p.indices.chunks_exact(3) {
                let (a, b, c) = if flip { (t[0], t[2], t[1]) } else { (t[0], t[1], t[2]) };
                let (a, b, c) = (a as usize, b as usize, c as usize);
                let tp = [pos[a], pos[b], pos[c]];
                let gn = (tp[1] - tp[0]).cross(tp[2] - tp[0]).normalize_or_zero();
                let n = nrm.as_ref().map(|n| [n[a], n[b], n[c]]).unwrap_or([gn; 3]);
                let tangent = p.tangents.as_ref().map(|tv| {
                    let tt = tv[a];
                    (m.transform_vector3(Vec3::new(tt[0], tt[1], tt[2])).normalize_or_zero(), tt[3])
                });
                out.push((p.node, ModelTri { p: tp, n, uv: [p.uvs[a], p.uvs[b], p.uvs[c]], tangent, material: p.material }));
            }
        }
        Ok(out)
    }

    /// The model's triangles in world space (`world` = the object's transform), posed by `pose` (None = rest pose),
    /// restricted to the mesh nodes named in `only`.
    pub fn triangles(&self, world: Mat4, only: Option<&[String]>, pose: Option<&desc::Animation>) -> Result<Vec<ModelTri>> {
        let (doc, bufs, _) = &self.gltf;
        let globals = self.globals(pose)?;
        let nodes: Vec<gltf::Node> = doc.nodes().collect();
        let mut skins: HashMap<usize, Vec<Mat4>> = HashMap::new();
        let mut out = vec![];
        for p in &self.prims {
            let node = &nodes[p.node];
            if only.is_some_and(|names| !node.name().is_some_and(|nm| names.iter().any(|x| x == nm))) {
                continue;
            }
            let (pos, nrm): (Vec<Vec3>, Option<Vec<Vec3>>) = match (&p.skin, node.skin()) {
                (Some((joints, weights)), Some(skin)) => {
                    // glTF: a skinned mesh ignores its node's transform; joints place it (relative to the scene root).
                    let mats = skins.entry(skin.index()).or_insert_with(|| {
                        let ibm: Vec<Mat4> = skin.reader(|b| Some(&bufs[b.index()])).read_inverse_bind_matrices().map(|it| it.map(|m| Mat4::from_cols_array_2d(&m)).collect()).unwrap_or_default();
                        skin.joints().enumerate().map(|(k, j)| world * globals[j.index()] * ibm.get(k).copied().unwrap_or(Mat4::IDENTITY)).collect()
                    });
                    let mut pos = Vec::with_capacity(p.positions.len());
                    let mut nrm = p.normals.as_ref().map(|n| Vec::with_capacity(n.len()));
                    for (v, (j, w)) in joints.iter().zip(weights).enumerate() {
                        let sum = w[0] + w[1] + w[2] + w[3];
                        let k = if sum > 0.0 { 1.0 / sum } else { 0.0 };
                        let mut m = Mat4::ZERO;
                        for c in 0..4 {
                            if w[c] > 0.0 {
                                m += mats.get(j[c] as usize).copied().unwrap_or(world) * (w[c] * k);
                            }
                        }
                        if sum <= 0.0 {
                            m = world;
                        }
                        pos.push(m.transform_point3(p.positions[v]));
                        if let (Some(out), Some(src)) = (nrm.as_mut(), &p.normals) {
                            out.push((Mat3::from_mat4(m).inverse().transpose() * src[v]).normalize_or_zero());
                        }
                    }
                    (pos, nrm)
                }
                _ => {
                    let m = world * globals[p.node];
                    let nm = Mat3::from_mat4(m).inverse().transpose();
                    (p.positions.iter().map(|v| m.transform_point3(*v)).collect(), p.normals.as_ref().map(|n| n.iter().map(|v| (nm * *v).normalize_or_zero()).collect()))
                }
            };
            let m = if p.skin.is_some() { world } else { world * globals[p.node] };
            let mirrored = m.determinant() < 0.0;
            for t in p.indices.chunks_exact(3) {
                let (a, b, c) = if mirrored { (t[0], t[2], t[1]) } else { (t[0], t[1], t[2]) };
                let (a, b, c) = (a as usize, b as usize, c as usize);
                let tp = [pos[a], pos[b], pos[c]];
                let gn = (tp[1] - tp[0]).cross(tp[2] - tp[0]).normalize_or_zero();
                let n = nrm.as_ref().map(|n| [n[a], n[b], n[c]]).unwrap_or([gn; 3]);
                let tangent = p.tangents.as_ref().map(|tv| {
                    let tt = tv[a];
                    (m.transform_vector3(Vec3::new(tt[0], tt[1], tt[2])).normalize_or_zero(), tt[3])
                });
                out.push(ModelTri { p: tp, n, uv: [p.uvs[a], p.uvs[b], p.uvs[c]], tangent, material: p.material });
            }
        }
        Ok(out)
    }
}

/// A channel's value at time `t` (clamped to its keys).
fn sample(ch: &Channel, t: f32) -> [f32; 4] {
    use gltf::animation::Interpolation;
    let ts = &ch.times;
    let cubic = ch.interpolation == Interpolation::CubicSpline;
    let value = |k: usize| if cubic { ch.values[k * 3 + 1] } else { ch.values[k] };
    if ts.is_empty() {
        return [0.0, 0.0, 0.0, 1.0];
    }
    if t <= ts[0] || ts.len() == 1 {
        return value(0);
    }
    let last = ts.len() - 1;
    if t >= ts[last] {
        return value(last);
    }
    let k = ts.partition_point(|&x| x <= t) - 1;
    let dt = ts[k + 1] - ts[k];
    let u = if dt > 0.0 { (t - ts[k]) / dt } else { 0.0 };
    let lerp = |a: [f32; 4], b: [f32; 4], u: f32| [0, 1, 2, 3].map(|i| a[i] + (b[i] - a[i]) * u);
    match (ch.interpolation, ch.path) {
        (Interpolation::Step, _) => value(k),
        (Interpolation::Linear, Path::Rotation) => Quat::from_array(value(k)).slerp(Quat::from_array(value(k + 1)), u).to_array(),
        (Interpolation::Linear, _) => lerp(value(k), value(k + 1), u),
        (Interpolation::CubicSpline, _) => {
            // Hermite: p(u) = (2u³-3u²+1) v0 + (u³-2u²+u) dt·b0 + (-2u³+3u²) v1 + (u³-u²) dt·a1
            let (v0, b0, a1, v1) = (ch.values[k * 3 + 1], ch.values[k * 3 + 2], ch.values[(k + 1) * 3], ch.values[(k + 1) * 3 + 1]);
            let (u2, u3) = (u * u, u * u * u);
            let r = [0, 1, 2, 3].map(|i| (2.0 * u3 - 3.0 * u2 + 1.0) * v0[i] + (u3 - 2.0 * u2 + u) * dt * b0[i] + (-2.0 * u3 + 3.0 * u2) * v1[i] + (u3 - u2) * dt * a1[i]);
            if matches!(ch.path, Path::Rotation) {
                Quat::from_array(r).normalize().to_array()
            } else {
                r
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A rigid model: node "arm" (animated: 0 -> 2 m up over 1 s) holding "hand" (a triangle, 1 m along x), and a
    /// static "base" (the same triangle 5 m along z).
    const RIGID: &str = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0,2]}],"nodes":[{"name":"arm","children":[1]},{"name":"hand","mesh":0,"translation":[1,0,0]},{"name":"base","mesh":0,"translation":[0,0,5]}],"meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1}]}],"buffers":[{"byteLength":76,"uri":"data:application/octet-stream;base64,AAAAAAAAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAgD8AAAAAAAABAAIAAAAAAAAAAACAPwAAAAAAAAAAAAAAAAAAAAAAAABAAAAAAA=="}],"bufferViews":[{"buffer":0,"byteOffset":0,"byteLength":36},{"buffer":0,"byteOffset":36,"byteLength":6},{"buffer":0,"byteOffset":44,"byteLength":8},{"buffer":0,"byteOffset":52,"byteLength":24}],"accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]},{"bufferView":1,"componentType":5123,"count":3,"type":"SCALAR"},{"bufferView":2,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]},{"bufferView":3,"componentType":5126,"count":2,"type":"VEC3"}],"animations":[{"name":"move","channels":[{"sampler":0,"target":{"node":0,"path":"translation"}}],"samplers":[{"input":2,"output":3,"interpolation":"LINEAR"}]}]}"#;

    #[test]
    fn rigid_split_moves_the_animated_subtree() {
        let m = Model::new(gltf::import_slice(RIGID.as_bytes()).unwrap()).unwrap();
        assert!(m.rigid());
        assert_eq!(m.moving_nodes("move").unwrap(), vec![true, true, false]);
        let pose = desc::Animation { clip: "move".into(), time: 0.5, r#loop: false };
        let g = m.node_globals(Some(&pose)).unwrap();
        assert!((g[1].transform_point3(Vec3::ZERO) - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-6);
        // Static part in world space (the object at x = 10), the moving part in its node's space.
        let world = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        let stat = m.node_triangles(world, Some(&pose), &|n| n == 2, false, &|_| false).unwrap();
        assert_eq!(stat.len(), 1);
        assert_eq!(stat[0].0, 2);
        assert!((stat[0].1.p[1] - Vec3::new(11.0, 0.0, 5.0)).length() < 1e-6);
        let local = m.node_triangles(Mat4::IDENTITY, None, &|n| n != 2, true, &|_| false).unwrap();
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].0, 1);
        assert_eq!(local[0].1.p, [Vec3::ZERO, Vec3::X, Vec3::Y]);
        // Placed: world x the node's global = what `triangles` gives for the pose.
        let posed = m.triangles(world, None, Some(&pose)).unwrap();
        let w = world * g[1];
        assert!(posed.iter().any(|t| (0..3).all(|k| (t.p[k] - w.transform_point3(local[0].1.p[k])).length() < 1e-5)));
        // A node placed mirrored keeps its winding front-facing once placed.
        let flipped = m.node_triangles(Mat4::IDENTITY, None, &|n| n == 1, true, &|_| true).unwrap();
        assert_eq!(flipped[0].1.p, [Vec3::ZERO, Vec3::Y, Vec3::X]);
    }

    fn ch(interpolation: gltf::animation::Interpolation, path: Path, times: Vec<f32>, values: Vec<[f32; 4]>) -> Channel {
        Channel { node: 0, path, interpolation, times, values }
    }

    #[test]
    fn samples_keys_and_interpolates() {
        use gltf::animation::Interpolation::*;
        let t = ch(Linear, Path::Translation, vec![0.0, 1.0, 3.0], vec![[0.0; 4], [2.0, 0.0, 0.0, 0.0], [2.0, 4.0, 0.0, 0.0]]);
        assert_eq!(sample(&t, -1.0)[0], 0.0);
        assert_eq!(sample(&t, 0.5)[0], 1.0);
        assert_eq!(sample(&t, 2.0)[1], 2.0);
        assert_eq!(sample(&t, 9.0)[1], 4.0);
        let s = ch(Step, Path::Scale, vec![0.0, 1.0], vec![[1.0; 4], [3.0; 4]]);
        assert_eq!(sample(&s, 0.99)[0], 1.0);
        let q90 = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2).to_array();
        let r = ch(Linear, Path::Rotation, vec![0.0, 1.0], vec![Quat::IDENTITY.to_array(), q90]);
        let half = Quat::from_array(sample(&r, 0.5));
        assert!(half.angle_between(Quat::from_rotation_y(std::f32::consts::FRAC_PI_4)) < 1e-5, "slerp");
        // cubic spline with zero tangents eases between values
        let c = ch(CubicSpline, Path::Translation, vec![0.0, 2.0], vec![[0.0; 4], [0.0; 4], [0.0; 4], [0.0; 4], [4.0, 0.0, 0.0, 0.0], [0.0; 4]]);
        assert!((sample(&c, 1.0)[0] - 2.0).abs() < 1e-6);
    }
}

#[cfg(test)]
mod bench {
    #[test]
    #[ignore]
    fn skinning_cost() {
        let path = std::env::var("THREERT_SKIN_GLB").unwrap();
        let m = super::Model::new(gltf::import(&path).unwrap()).unwrap();
        let clip = m.clip_names()[0].clone();
        let t0 = web_time::Instant::now();
        let mut n = 0;
        for k in 0..100 {
            let a = crate::desc::Animation { clip: clip.clone(), time: k as f32 * 0.016, r#loop: true };
            n = m.triangles(glam::Mat4::IDENTITY, None, Some(&a)).unwrap().len();
        }
        eprintln!("{n} triangles: {:.3} ms per pose", t0.elapsed().as_secs_f64() * 1000.0 / 100.0);
        let a = crate::desc::Animation { clip, time: 0.3, r#loop: true };
        let tris = m.triangles(glam::Mat4::IDENTITY, None, Some(&a)).unwrap();
        let t0 = web_time::Instant::now();
        let mut x = 0.0;
        for _ in 0..100 {
            for t in &tris {
                x += crate::scene::tri_attr(t.p, t.n, t.uv, t.tangent, 0).x[0];
            }
        }
        eprintln!("tri_attr: {:.3} ms per pose ({x})", t0.elapsed().as_secs_f64() * 1000.0 / 100.0);
    }
}
