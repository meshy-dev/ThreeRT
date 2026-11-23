//! glTF models, posed: a clip sampled at a time (node translation / rotation / scale channels with step, linear or
//! cubic-spline interpolation) and linear blend skinning on the CPU (UE: a skeletal mesh with an animation sequence).
//! `Model::triangles` yields a model's triangles in world space for a pose, in the same order every time, so a posed
//! model can be re-skinned in place (`tables.rs`, `LiveCore::commit`). A mesh that deforms is skinned by the GPU
//! (shaders/skin.wgsl) from the same triangles as the file has them (`skin_source`) and a pose's matrices (`palette`).

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

impl Prim {
    /// A primitive with UVs and without tangents has them made (tangents.rs): a corner each, so a vertex whose
    /// corners come out with different tangents is a vertex for each afterwards (its position, normal, UV, joints
    /// and weights with it), as a file with tangents has them. A triangle the algorithm gives no frame has none (a
    /// hand of 0); a primitive none of whose triangles has one stays as it is. Without normals: by each triangle's
    /// own.
    fn make_tangents(&mut self) {
        let face = |k: usize| [0, 1, 2].map(|c| self.indices[k * 3 + c] as usize);
        let made = crate::tangents::generate(self.indices.len() / 3, |k, c| {
            let v = face(k);
            let normal = match &self.normals {
                Some(n) => n[v[c]],
                None => (self.positions[v[1]] - self.positions[v[0]]).cross(self.positions[v[2]] - self.positions[v[0]]).normalize_or_zero(),
            };
            (self.positions[v[c]].to_array(), normal.to_array(), self.uvs[v[c]].to_array())
        });
        if made.iter().all(|t| t.is_none()) {
            return;
        }
        // (A vertex of the file and a tangent: a vertex. `from`: the file's vertex each is.)
        let mut seen: HashMap<(u32, [u32; 4]), u32> = HashMap::new();
        let (mut from, mut tangents, mut indices) = (vec![], vec![], Vec::with_capacity(self.indices.len()));
        for (k, t) in made.iter().enumerate() {
            for c in 0..3 {
                let (v, tangent) = (self.indices[k * 3 + c], t.map_or([0.0; 4], |t| t[c]));
                indices.push(*seen.entry((v, tangent.map(f32::to_bits))).or_insert_with(|| {
                    from.push(v as usize);
                    tangents.push(tangent);
                    from.len() as u32 - 1
                }));
            }
        }
        fn pick<T: Copy>(of: &[T], from: &[usize]) -> Vec<T> {
            from.iter().map(|&v| of[v]).collect()
        }
        self.positions = pick(&self.positions, &from);
        self.uvs = pick(&self.uvs, &from);
        self.normals = self.normals.as_ref().map(|n| pick(n, &from));
        self.skin = self.skin.as_ref().map(|(j, w)| (pick(j, &from), pick(w, &from)));
        (self.tangents, self.indices) = (Some(tangents), indices);
    }
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
    /// Nodes by name (the first of a name), for poses that name them (`desc::Animation::nodes`).
    names: HashMap<String, usize>,
    clips: Mutex<HashMap<String, Arc<Clip>>>,
}

/// A triangle of a posed model, in world space.
pub struct ModelTri {
    pub p: [Vec3; 3],
    pub n: [Vec3; 3],
    pub uv: [Vec2; 3],
    pub tangent: Option<crate::scene::CornerTangents>,
    /// glTF material index.
    pub material: Option<usize>,
}

/// A corner of a model's triangle as the skinning pass reads it (skin.wgsl `Corner`): where it is in the file, and
/// the matrices of the model's palette (`Model::palette`) that place it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SkinCorner {
    /// Position; w: the first two palette indices (16 bits each).
    pub p: [f32; 4],
    /// Normal; w: the other two.
    pub n: [f32; 4],
    /// Tangent; w: `CORNER_NORMAL` | `CORNER_TANGENT` (bits): which of the two the file has.
    pub t: [f32; 4],
    /// The four weights, summing to one.
    pub w: [f32; 4],
}

pub const CORNER_NORMAL: u32 = 1;
pub const CORNER_TANGENT: u32 = 2;

/// A palette matrix: the three rows of an affine transform.
pub type PaletteRows = [[f32; 4]; 3];

fn palette_rows(m: Mat4) -> PaletteRows {
    let t = m.transpose().to_cols_array_2d();
    [t[0], t[1], t[2]]
}

/// A corner placed by a palette, as skin.wgsl places it: its position, and its normal and tangent (zero where the
/// file has none). The four matrices blended by the weights; the normal by the blend's inverse transpose.
pub fn skin_corner(c: &SkinCorner, palette: &[PaletteRows]) -> (Vec3, Vec3, Vec3) {
    let (a, b) = (c.p[3].to_bits(), c.n[3].to_bits());
    let joints = [a & 0xffff, a >> 16, b & 0xffff, b >> 16];
    let mut rows = [glam::Vec4::ZERO; 3];
    for (j, w) in joints.iter().zip(c.w) {
        for (r, m) in rows.iter_mut().zip(palette[*j as usize]) {
            *r += glam::Vec4::from(m) * w;
        }
    }
    let (p, n, t) = (Vec3::new(c.p[0], c.p[1], c.p[2]), Vec3::new(c.n[0], c.n[1], c.n[2]), Vec3::new(c.t[0], c.t[1], c.t[2]));
    let r = rows.map(|r| r.truncate());
    let det = r[0].dot(r[1].cross(r[2]));
    let normal = (Vec3::new(r[1].cross(r[2]).dot(n), r[2].cross(r[0]).dot(n), r[0].cross(r[1]).dot(n)) * det.signum()).normalize_or_zero();
    (Vec3::new(rows[0].dot(p.extend(1.0)), rows[1].dot(p.extend(1.0)), rows[2].dot(p.extend(1.0))), normal, Vec3::new(r[0].dot(t), r[1].dot(t), r[2].dot(t)).normalize_or_zero())
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
                let uvs: Option<Vec<Vec2>> = r.read_tex_coords(0).map(|t| t.into_f32().map(Vec2::from).collect());
                let mut p = Prim {
                    node: ni,
                    material: prim.material().index(),
                    normals: r.read_normals().map(|it| it.map(Vec3::from).collect()),
                    tangents: r.read_tangents().map(|t| t.collect()),
                    indices: r.read_indices().map(|i| i.into_u32().collect()).unwrap_or_else(|| (0..n as u32).collect()),
                    positions,
                    skin,
                    uvs: uvs.clone().unwrap_or_else(|| vec![Vec2::ZERO; n]),
                };
                if uvs.is_some() && p.tangents.is_none() {
                    p.make_tangents();
                }
                prims.push(p);
            }
        }
        let mut names = HashMap::new();
        for n in doc.nodes() {
            if let Some(name) = n.name() {
                names.entry(name.to_string()).or_insert(n.index());
            }
        }
        Ok(Model { gltf, prims, parents, names, clips: Mutex::new(HashMap::new()) })
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
        if let Some(a) = pose.filter(|a| !a.clip.is_empty()) {
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
        // The nodes the game poses itself, over the clip.
        for (name, v) in pose.map(|a| &a.nodes).into_iter().flatten() {
            let &i = self.names.get(name).ok_or_else(|| anyhow!("pose: no node \"{name}\""))?;
            let (t, r) = match v.len() {
                4 => (None, [v[0], v[1], v[2], v[3]]),
                7 => (Some(Vec3::new(v[0], v[1], v[2])), [v[3], v[4], v[5], v[6]]),
                n => return Err(anyhow!("pose: node \"{name}\" has {n} numbers (4: a rotation, 7: a translation and a rotation)")),
            };
            let q = Quat::from_array(r);
            if !q.is_finite() || q.length_squared() < 1e-12 || t.is_some_and(|t| !t.is_finite()) {
                return Err(anyhow!("pose: node \"{name}\" is not a transform ({v:?})"));
            }
            let l = &mut local[i];
            l.1 = q.normalize();
            if let Some(t) = t {
                l.0 = t;
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

    /// Per node: whether a pose may move it (a channel of any clip of the file, or a node this pose names, is it or one
    /// of its ancestors). Any clip, not the pose's alone: the model is the same parts whichever clip it plays.
    pub fn moving_nodes(&self, pose: &desc::Animation) -> Result<Vec<bool>> {
        let mut direct = vec![false; self.parents.len()];
        if !pose.clip.is_empty() {
            self.clip(&pose.clip)?;
        }
        for ch in self.gltf.0.animations().flat_map(|a| a.channels()) {
            if ch.target().property() != gltf::animation::Property::MorphTargetWeights {
                direct[ch.target().node().index()] = true;
            }
        }
        for name in pose.nodes.keys() {
            direct[*self.names.get(name).ok_or_else(|| anyhow!("pose: no node \"{name}\""))?] = true;
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

    /// Where each matrix of the palette is: the first of each skin's joints, each node that holds a primitive without
    /// a skin, and the palette's length. Entry 0 is the identity (a vertex no joint holds).
    fn palette_layout(&self) -> (Vec<usize>, HashMap<usize, usize>, usize) {
        let mut at = 1;
        let mut skins = vec![];
        for skin in self.gltf.0.skins() {
            skins.push(at);
            at += skin.joints().count();
        }
        let mut nodes = HashMap::new();
        for p in self.prims.iter().filter(|p| p.skin.is_none()) {
            nodes.entry(p.node).or_insert_with(|| {
                at += 1;
                at - 1
            });
        }
        (skins, nodes, at)
    }

    /// The matrices that place the model's vertices in its own space for `pose` (None = rest pose): what
    /// `skin_source`'s corners index. A skin's joints (joint x inverse bind), then the nodes of primitives without a
    /// skin.
    pub fn palette(&self, pose: Option<&desc::Animation>) -> Result<Vec<PaletteRows>> {
        let (doc, bufs, _) = &self.gltf;
        let globals = self.globals(pose)?;
        let (_, nodes, len) = self.palette_layout();
        let mut out = vec![palette_rows(Mat4::IDENTITY); len];
        let mut at = 1;
        for skin in doc.skins() {
            let ibm: Vec<Mat4> = skin.reader(|b| Some(&bufs[b.index()])).read_inverse_bind_matrices().map(|it| it.map(|m| Mat4::from_cols_array_2d(&m)).collect()).unwrap_or_default();
            for (k, j) in skin.joints().enumerate() {
                out[at] = palette_rows(globals[j.index()] * ibm.get(k).copied().unwrap_or(Mat4::IDENTITY));
                at += 1;
            }
        }
        for (node, k) in nodes {
            out[k] = palette_rows(globals[node]);
        }
        Ok(out)
    }

    /// The palette's length.
    pub fn palette_len(&self) -> usize {
        self.palette_layout().2
    }

    /// Per primitive: whether `pose` places it mirrored (a node's transform; a skinned one never is). `triangles`
    /// keeps such a primitive's winding front-facing, so the corners of `skin_source` are in a pose's order: they
    /// serve the poses that mirror the same primitives.
    pub fn mirrored(&self, pose: Option<&desc::Animation>) -> Result<Vec<bool>> {
        let globals = self.globals(pose)?;
        let nodes: Vec<gltf::Node> = self.gltf.0.nodes().collect();
        Ok(self.prims.iter().map(|p| !(p.skin.is_some() && nodes[p.node].skin().is_some()) && globals[p.node].determinant() < 0.0).collect())
    }

    /// The model's triangles as the skinning pass reads them, three corners each, in `triangles`' order for `pose`
    /// (restricted to the mesh nodes named in `only`, as there): skinned by `palette`'s matrices they are `triangles`
    /// in the model's own space, for every pose that mirrors the primitives `pose` does (`mirrored`).
    pub fn skin_source(&self, only: Option<&[String]>, pose: Option<&desc::Animation>) -> Result<Vec<[SkinCorner; 3]>> {
        let (doc, _, _) = &self.gltf;
        let mirrors = self.mirrored(pose)?;
        let nodes: Vec<gltf::Node> = doc.nodes().collect();
        let (skins, of_node, _) = self.palette_layout();
        let mut out = vec![];
        for (p, &mirrored) in self.prims.iter().zip(&mirrors) {
            let node = &nodes[p.node];
            if only.is_some_and(|names| !node.name().is_some_and(|nm| names.iter().any(|x| x == nm))) {
                continue;
            }
            let skin = p.skin.as_ref().zip(node.skin());
            let corner = |v: usize, tangent: Option<[f32; 4]>| {
                let (joints, weights) = match &skin {
                    Some(((joints, weights), skin)) => {
                        let (j, w) = (joints[v], weights[v]);
                        let sum = w[0] + w[1] + w[2] + w[3];
                        let count = skin.joints().count();
                        if sum > 0.0 {
                            // (A weight that is not positive holds nothing; a joint the skin has not is the identity.)
                            let held = |c: usize| w[c] > 0.0;
                            ([0, 1, 2, 3].map(|c| if held(c) && (j[c] as usize) < count { skins[skin.index()] + j[c] as usize } else { 0 }), [0, 1, 2, 3].map(|c| if held(c) { w[c] / sum } else { 0.0 }))
                        } else {
                            ([0; 4], [1.0, 0.0, 0.0, 0.0])
                        }
                    }
                    None => ([of_node[&p.node], 0, 0, 0], [1.0, 0.0, 0.0, 0.0]),
                };
                let pos = p.positions[v];
                let n = p.normals.as_ref().map(|n| n[v]);
                let flags = if n.is_some() { CORNER_NORMAL } else { 0 } | if tangent.is_some() { CORNER_TANGENT } else { 0 };
                let (n, t) = (n.unwrap_or(Vec3::ZERO), tangent.unwrap_or([0.0; 4]));
                SkinCorner {
                    p: [pos.x, pos.y, pos.z, f32::from_bits(joints[0] as u32 | (joints[1] as u32) << 16)],
                    n: [n.x, n.y, n.z, f32::from_bits(joints[2] as u32 | (joints[3] as u32) << 16)],
                    t: [t[0], t[1], t[2], f32::from_bits(flags)],
                    w: weights,
                }
            };
            // (A node that mirrors: the winding stays front-facing, as `triangles` has it.)
            for t in p.indices.chunks_exact(3) {
                let (a, b, c) = if mirrored { (t[0], t[2], t[1]) } else { (t[0], t[1], t[2]) };
                // (A hand of 0: the vertex has none.)
                let tangent = |v: u32| p.tangents.as_ref().map(|tv| tv[v as usize]).filter(|t| t[3] != 0.0);
                out.push([corner(a as usize, tangent(a)), corner(b as usize, tangent(b)), corner(c as usize, tangent(c))]);
            }
        }
        Ok(out)
    }

    /// Node transforms relative to the glTF scene root for `pose` (None = rest pose).
    pub fn node_globals(&self, pose: Option<&desc::Animation>) -> Result<Vec<Mat4>> {
        self.globals(pose)
    }

    /// A rigid model's triangles of the nodes `keep` admits, with each one's node: in world space for `pose`
    /// (`world` x the node's global transform), or in the node's own space (`local`: the caller places them per node).
    /// Same order as `triangles`. Mirrored placements keep the winding front-facing in world space, as `triangles`
    /// does; in local space a node's triangles are as its mesh has them (an instance that places it mirrored says so
    /// itself: `scene::INST_MIRRORED`).
    pub fn node_triangles(&self, world: Mat4, pose: Option<&desc::Animation>, keep: &dyn Fn(usize) -> bool, local: bool) -> Result<Vec<(usize, ModelTri)>> {
        let globals = self.globals(pose)?;
        let mut out = vec![];
        for p in self.prims.iter().filter(|p| keep(p.node)) {
            let m = if local { Mat4::IDENTITY } else { world * globals[p.node] };
            let nm = Mat3::from_mat4(m).inverse().transpose();
            let pos: Vec<Vec3> = p.positions.iter().map(|v| m.transform_point3(*v)).collect();
            let nrm: Option<Vec<Vec3>> = p.normals.as_ref().map(|n| n.iter().map(|v| (nm * *v).normalize_or_zero()).collect());
            // (A mirroring placement: the winding stays front-facing, and a tangent's frame changes hand.)
            let flip = !local && m.determinant() < 0.0;
            for t in p.indices.chunks_exact(3) {
                let (a, b, c) = if flip { (t[0], t[2], t[1]) } else { (t[0], t[1], t[2]) };
                let (a, b, c) = (a as usize, b as usize, c as usize);
                let tp = [pos[a], pos[b], pos[c]];
                let gn = (tp[1] - tp[0]).cross(tp[2] - tp[0]).normalize_or_zero();
                let n = nrm.as_ref().map(|n| [n[a], n[b], n[c]]).unwrap_or([gn; 3]);
                let tangent = p.tangents.as_ref().map(|tv| [a, b, c].map(|v| (m.transform_vector3(Vec3::new(tv[v][0], tv[v][1], tv[v][2])).normalize_or_zero(), if flip { -tv[v][3] } else { tv[v][3] })));
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
        // (A skinned primitive's matrix a vertex, where it has tangents: they follow their joints as its normals do.)
        let mut blends: Vec<Mat4> = vec![];
        for p in &self.prims {
            let node = &nodes[p.node];
            if only.is_some_and(|names| !node.name().is_some_and(|nm| names.iter().any(|x| x == nm))) {
                continue;
            }
            blends.clear();
            let (pos, nrm): (Vec<Vec3>, Option<Vec<Vec3>>) = match (&p.skin, node.skin()) {
                (Some((joints, weights)), Some(skin)) => {
                    // glTF: a skinned mesh ignores its node's transform; joints place it (relative to the scene root).
                    let mats = skins.entry(skin.index()).or_insert_with(|| {
                        let ibm: Vec<Mat4> = skin.reader(|b| Some(&bufs[b.index()])).read_inverse_bind_matrices().map(|it| it.map(|m| Mat4::from_cols_array_2d(&m)).collect()).unwrap_or_default();
                        skin.joints().enumerate().map(|(k, j)| world * globals[j.index()] * ibm.get(k).copied().unwrap_or(Mat4::IDENTITY)).collect()
                    });
                    let mut pos = Vec::with_capacity(p.positions.len());
                    let mut nrm = p.normals.as_ref().map(|n| Vec::with_capacity(n.len()));
                    blends.clear();
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
                        if p.tangents.is_some() {
                            blends.push(m);
                        }
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
                    [a, b, c].map(|v| {
                        let by = blends.get(v).copied().unwrap_or(m);
                        (by.transform_vector3(Vec3::new(tv[v][0], tv[v][1], tv[v][2])).normalize_or_zero(), if mirrored { -tv[v][3] } else { tv[v][3] })
                    })
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
        let pose = desc::Animation { clip: "move".into(), time: 0.5, r#loop: false, nodes: Default::default() };
        assert_eq!(m.moving_nodes(&pose).unwrap(), vec![true, true, false]);
        let g = m.node_globals(Some(&pose)).unwrap();
        assert!((g[1].transform_point3(Vec3::ZERO) - Vec3::new(1.0, 1.0, 0.0)).length() < 1e-6);
        // Static part in world space (the object at x = 10), the moving part in its node's space.
        let world = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        let stat = m.node_triangles(world, Some(&pose), &|n| n == 2, false).unwrap();
        assert_eq!(stat.len(), 1);
        assert_eq!(stat[0].0, 2);
        assert!((stat[0].1.p[1] - Vec3::new(11.0, 0.0, 5.0)).length() < 1e-6);
        let local = m.node_triangles(Mat4::IDENTITY, None, &|n| n != 2, true).unwrap();
        assert_eq!(local.len(), 1);
        assert_eq!(local[0].0, 1);
        assert_eq!(local[0].1.p, [Vec3::ZERO, Vec3::X, Vec3::Y]);
        // Placed: world x the node's global = what `triangles` gives for the pose.
        let posed = m.triangles(world, None, Some(&pose)).unwrap();
        let w = world * g[1];
        assert!(posed.iter().any(|t| (0..3).all(|k| (t.p[k] - w.transform_point3(local[0].1.p[k])).length() < 1e-5)));
    }

    /// A primitive with UVs and without tangents has them made, a corner each. Two quads along an edge, the texture
    /// mirrored across it (the edge's two vertices have one UV for both sides): afterwards each of the two is a
    /// vertex a side, with its side's tangent (u's direction there) and hand (the bitangent points up the image on
    /// both), and with the file's position, normal, UV, joints and weights. One whose UVs have no area keeps none.
    #[test]
    fn a_primitive_without_tangents_has_them_made_a_corner_each() {
        let at = [(-1.0, 0.0), (0.0, 0.0), (0.0, 1.0), (-1.0, 1.0), (1.0, 0.0), (1.0, 1.0)];
        let prim = |uv: &dyn Fn(f32, f32) -> Vec2| Prim {
            node: 0,
            material: None,
            positions: at.iter().map(|&(x, y)| Vec3::new(x, y, 0.0)).collect(),
            normals: Some(vec![Vec3::Z; 6]),
            uvs: at.iter().map(|&(x, y)| uv(x, y)).collect(),
            tangents: None,
            indices: vec![0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2],
            skin: Some(((0..6).map(|k| [k, 0, 0, 0]).collect(), (0..6).map(|k| [1.0 - k as f32 * 0.1, k as f32 * 0.1, 0.0, 0.0]).collect())),
        };
        let mut p = prim(&|x, y| Vec2::new(x.abs(), 1.0 - y));
        p.make_tangents();
        let tangents = p.tangents.as_ref().expect("tangents");
        assert_eq!((p.positions.len(), tangents.len(), p.indices.len()), (8, 8, 12), "the edge's two vertices twice");
        let (joints, weights) = p.skin.as_ref().unwrap();
        for (k, tri) in p.indices.chunks_exact(3).enumerate() {
            let side = if k < 2 { -1.0 } else { 1.0 };
            for &v in tri {
                let (v, from) = (v as usize, joints[v as usize][0] as usize);
                assert_eq!(tangents[v], [side, 0.0, 0.0, side], "triangle {k}: its side's tangent and hand");
                let up = Vec3::Z.cross(Vec3::new(tangents[v][0], tangents[v][1], tangents[v][2])) * tangents[v][3];
                assert_eq!(up, Vec3::Y, "triangle {k}: the bitangent points up the image");
                assert_eq!((p.positions[v], p.normals.as_ref().unwrap()[v], p.uvs[v]), (Vec3::new(at[from].0, at[from].1, 0.0), Vec3::Z, Vec2::new(at[from].0.abs(), 1.0 - at[from].1)), "triangle {k}: the file's vertex {from}");
                assert_eq!(weights[v], [1.0 - from as f32 * 0.1, from as f32 * 0.1, 0.0, 0.0], "triangle {k}: its weights");
            }
        }
        // (The file's triangles, in its order: each corner the vertex it was.)
        assert_eq!(p.indices.iter().map(|&v| joints[v as usize][0]).collect::<Vec<_>>(), [0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2]);
        let mut flat = prim(&|_, _| Vec2::splat(0.5));
        flat.make_tangents();
        assert!(flat.tangents.is_none() && flat.positions.len() == 6, "UVs without area: no tangents, the primitive as it was");
    }

    /// An authored tangent's frame changes hand where a placement mirrors (the winding is swapped there too); in a
    /// node's own space it is as authored (the instance that places the node says whether it mirrors).
    #[test]
    fn a_mirrored_placement_changes_an_authored_tangents_hand() {
        let mut m = Model::new(gltf::import_slice(RIGID.as_bytes()).unwrap()).unwrap();
        for p in &mut m.prims {
            p.tangents = Some(vec![[1.0, 0.0, 0.0, 1.0]; p.positions.len()]);
        }
        let hand = |t: &ModelTri| t.tangent.unwrap()[0].1;
        let straight = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0));
        let mirrored = straight * Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0));
        assert!(m.triangles(straight, None, None).unwrap().iter().all(|t| hand(t) == 1.0));
        assert!(m.triangles(mirrored, None, None).unwrap().iter().all(|t| hand(t) == -1.0));
        assert!(m.node_triangles(mirrored, None, &|_| true, false).unwrap().iter().all(|(_, t)| hand(t) == -1.0));
        assert!(m.node_triangles(Mat4::IDENTITY, None, &|_| true, true).unwrap().iter().all(|(_, t)| hand(t) == 1.0));
    }

    /// Which nodes are parts does not depend on the clip that plays (tests/render/data/two-clips.glb: a clip for the
    /// node "left", another for "right", none for "base").
    #[test]
    fn the_nodes_any_clip_moves_are_moving_whichever_plays() {
        let m = Model::new(gltf::import("../tests/render/data/two-clips.glb").unwrap()).unwrap();
        let pose = |clip: &str| desc::Animation { clip: clip.into(), time: 0.5, r#loop: true, nodes: Default::default() };
        assert_eq!(m.moving_nodes(&pose("raise-left")).unwrap(), vec![false, true, true]);
        assert_eq!(m.moving_nodes(&pose("raise-right")).unwrap(), vec![false, true, true]);
        assert_eq!(m.moving_nodes(&pose("")).unwrap(), vec![false, true, true]);
        assert!(m.moving_nodes(&pose("raise-both")).is_err(), "no such clip");
    }

    /// The skinning pass's inputs give the triangles the CPU skin gives: a model's corners placed by its palette are
    /// `triangles` in the model's own space, for a skinned strip of two materials (tests/render/data/arm.glb) and for
    /// primitives on nodes (`RIGID`), at rest and posed.
    #[test]
    fn corners_placed_by_the_palette_are_the_models_triangles() {
        let mut arm = Model::new(gltf::import("../tests/render/data/arm.glb").unwrap()).unwrap();
        // (Vertices two joints hold, weights that do not sum to one, a primitive with tangents and one without
        // normals, a mesh on a node beside the skin that mirrors it; a joint turned about an axis that is neither
        // the normals' nor the tangents'.)
        let mut blend = Model::new(gltf::import("../tests/render/data/skin-blend.glb").unwrap()).unwrap();
        let mut rigid = Model::new(gltf::import_slice(RIGID.as_bytes()).unwrap()).unwrap();
        // (Every vertex a tangent that is no other's, so a corner that took another vertex's is seen.)
        for m in [&mut arm, &mut blend, &mut rigid] {
            for p in &mut m.prims {
                p.tangents = Some((0..p.positions.len()).map(|i| Vec3::new(1.0, 0.1 * i as f32, 0.03 * i as f32).normalize().extend(1.0).to_array()).collect());
            }
        }
        let pose = |clip: &str, time| desc::Animation { clip: clip.into(), time, r#loop: true, nodes: Default::default() };
        let mut seen = 0;
        let bends = || vec![None, Some(pose("bend", 0.3)), Some(pose("bend", 0.8))];
        for (m, poses) in [(&arm, bends()), (&blend, bends()), (&rigid, vec![None, Some(pose("move", 0.5))])] {
            let source = m.skin_source(None, None).unwrap();
            assert!(poses.iter().all(|a| m.mirrored(a.as_ref()).unwrap() == m.mirrored(None).unwrap()), "no pose here mirrors a primitive");
            for a in &poses {
                let (tris, palette) = (m.triangles(Mat4::IDENTITY, None, a.as_ref()).unwrap(), m.palette(a.as_ref()).unwrap());
                assert_eq!(source.len(), tris.len());
                for (corners, t) in source.iter().zip(&tris) {
                    for (k, c) in corners.iter().enumerate() {
                        let (p, n, tg) = skin_corner(c, &palette);
                        assert!((p - t.p[k]).length() < 1e-5, "position {p} against {}", t.p[k]);
                        if c.t[3].to_bits() & CORNER_NORMAL != 0 {
                            assert!((n - t.n[k]).length() < 1e-5, "normal {n} against {}", t.n[k]);
                        }
                        let tc = t.tangent.expect("a tangent a corner");
                        assert!((tg - tc[k].0).length() < 1e-5, "tangent {tg} against {}", tc[k].0);
                        seen += 1;
                    }
                }
            }
        }
        assert_eq!(seen, 36 + 648 + 12, "corners compared");
        // A vertex whose tangent has no hand has none: its corners do not say they have one, the others do.
        rigid.prims[0].tangents.as_mut().unwrap()[0] = [0.0; 4];
        let says: Vec<bool> = rigid.skin_source(None, None).unwrap().iter().flatten().map(|c| c.t[3].to_bits() & CORNER_TANGENT != 0).collect();
        let without = rigid.prims.iter().enumerate().map(|(k, p)| if k == 0 { p.indices.iter().filter(|&&v| v == 0).count() } else { 0 }).sum::<usize>();
        assert_eq!((says.iter().filter(|s| !**s).count(), without > 0), (without, true), "the corners at the vertex without a tangent");
        rigid.prims[0].tangents.as_mut().unwrap()[0] = [1.0, 0.0, 0.0, 1.0];
        // A model's triangles a node (`node_triangles`) are `triangles`', their tangents a corner each too.
        for a in [None, Some(pose("move", 0.5))] {
            let (by_node, whole) = (rigid.node_triangles(Mat4::IDENTITY, a.as_ref(), &|_| true, false).unwrap(), rigid.triangles(Mat4::IDENTITY, None, a.as_ref()).unwrap());
            assert_eq!(by_node.len(), whole.len());
            for ((_, t), w) in by_node.iter().zip(&whole) {
                assert_eq!((t.p, t.tangent.unwrap()), (w.p, w.tangent.unwrap()));
                assert!(t.tangent.unwrap()[0].0 != t.tangent.unwrap()[1].0, "a tangent a corner");
            }
        }
        assert_eq!((blend.palette_len(), blend.skin_source(None, None).unwrap().len()), (4, 72), "the identity, two joints and the badge's node; 72 triangles");
        // The badge's node mirrors it: its corners are in the order that keeps it front-facing, and a pose that does
        // not mirror it has them in another.
        // (Primitives in the order the model emits them: the badge's node first.)
        assert_eq!(blend.mirrored(None).unwrap(), vec![true, false, false]);
        assert_eq!(blend.mirrored(Some(&pose("flip", 0.9))).unwrap(), vec![false, false, false]);
        assert!(blend.skin_source(None, None).unwrap() != blend.skin_source(None, Some(&pose("flip", 0.9))).unwrap());
        // A blended vertex's normal is not its matrix's: the matrix of two joints is no rotation.
        let turned = blend.palette(Some(&pose("bend", 0.8))).unwrap();
        let by_the_matrix = |c: &SkinCorner| {
            let (a, b) = (c.p[3].to_bits(), c.n[3].to_bits());
            let mut rows = [glam::Vec4::ZERO; 3];
            for (j, w) in [a & 0xffff, a >> 16, b & 0xffff, b >> 16].iter().zip(c.w) {
                for (r, m) in rows.iter_mut().zip(turned[*j as usize]) {
                    *r += glam::Vec4::from(m) * w;
                }
            }
            let n = Vec3::new(c.n[0], c.n[1], c.n[2]);
            Vec3::new(rows[0].truncate().dot(n), rows[1].truncate().dot(n), rows[2].truncate().dot(n)).normalize_or_zero()
        };
        assert!(blend.skin_source(None, None).unwrap().iter().flatten().any(|c| c.t[3].to_bits() & CORNER_NORMAL != 0 && (skin_corner(c, &turned).1 - by_the_matrix(c)).length() > 0.01), "the fixture tells a normal by the inverse transpose from one by the matrix");
        // A pose bends the strip: its corners are not where the file has them.
        let (source, bent) = (arm.skin_source(None, None).unwrap(), arm.palette(Some(&pose("bend", 0.8))).unwrap());
        assert_eq!(bent.len(), arm.palette_len());
        assert!(source.iter().flatten().any(|c| (skin_corner(c, &bent).0 - Vec3::new(c.p[0], c.p[1], c.p[2])).length() > 0.05), "the pose moves corners");
    }

    #[test]
    fn nodes_posed_by_name_over_the_clip() {
        let m = Model::new(gltf::import_slice(RIGID.as_bytes()).unwrap()).unwrap();
        let q = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        // A rotation over the clip's translation; a translation and a rotation; a node the clip does not move.
        let pose = |nodes: &[(&str, Vec<f32>)]| desc::Animation { clip: "move".into(), time: 0.5, r#loop: false, nodes: nodes.iter().map(|(k, v)| (k.to_string(), v.clone())).collect() };
        let g = m.node_globals(Some(&pose(&[("arm", q.to_array().to_vec())]))).unwrap();
        assert!((g[1].transform_point3(Vec3::ZERO) - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-5, "hand turned about the arm, the arm where the clip has it");
        let mut tr = vec![3.0, 0.0, 0.0];
        tr.extend(Quat::IDENTITY.to_array());
        let g = m.node_globals(Some(&pose(&[("arm", tr), ("base", q.to_array().to_vec())]))).unwrap();
        assert!((g[1].transform_point3(Vec3::ZERO) - Vec3::new(4.0, 0.0, 0.0)).length() < 1e-5, "the translation replaces the clip's");
        assert!((g[2].transform_point3(Vec3::X) - Vec3::new(0.0, 1.0, 5.0)).length() < 1e-5);
        let a = pose(&[("base", q.to_array().to_vec())]);
        assert_eq!(m.moving_nodes(&a).unwrap(), vec![true, true, true], "a posed node moves");
        // Without a clip: the file's transforms and the named nodes.
        let a = desc::Animation { clip: String::new(), time: 0.0, r#loop: true, nodes: [("arm".to_string(), q.to_array().to_vec())].into() };
        assert!((m.node_globals(Some(&a)).unwrap()[1].transform_point3(Vec3::ZERO) - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-5);
        assert_eq!(m.moving_nodes(&a).unwrap(), vec![true, true, false]);
        assert!(m.node_globals(Some(&pose(&[("elbow", q.to_array().to_vec())]))).is_err(), "no such node");
        assert!(m.node_globals(Some(&pose(&[("arm", vec![0.0, 0.0, 0.0])]))).is_err(), "3 numbers");
        assert!(m.node_globals(Some(&pose(&[("arm", vec![0.0; 4])]))).is_err(), "not a rotation");
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
            let a = crate::desc::Animation { clip: clip.clone(), time: k as f32 * 0.016, r#loop: true, nodes: Default::default() };
            n = m.triangles(glam::Mat4::IDENTITY, None, Some(&a)).unwrap().len();
        }
        eprintln!("{n} triangles: {:.3} ms per pose", t0.elapsed().as_secs_f64() * 1000.0 / 100.0);
        let a = crate::desc::Animation { clip, time: 0.3, r#loop: true, nodes: Default::default() };
        let tris = m.triangles(glam::Mat4::IDENTITY, None, Some(&a)).unwrap();
        let t0 = web_time::Instant::now();
        let mut x = 0.0;
        for _ in 0..100 {
            for t in &tris {
                x += crate::scene::tri_attr(t.n, t.uv, t.tangent, 0).t[0];
            }
        }
        eprintln!("tri_attr: {:.3} ms per pose ({x})", t0.elapsed().as_secs_f64() * 1000.0 / 100.0);
    }
}
