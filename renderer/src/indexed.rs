//! The scene's geometry as the GPU holds it: vertices kept once, a triangle three indices and a row. The build still
//! makes triangles with corners of their own (`Scene::tris`, `attrs`); what they share is found here, scope by scope:
//! a mesh's triangles share within the mesh, the still world-space triangles among themselves, what the CPU writes
//! whole each frame (the dynamic world-space triangles) has three vertices a triangle, and a particle system's
//! mesh four a quad.

use crate::scene::GpuTriAttr;
use glam::Vec3A;
use std::collections::HashMap;
use std::hash::Hash;

/// A vertex, one record: (position, u), (normal, v), (tangent, hand). A hand of 0: the vertex has no tangent. What a
/// hit reads of each of its triangle's three; an acceleration structure is built from the records' positions.
pub type Vertex = [[f32; 4]; 3];
pub const VERTEX_BYTES: usize = std::mem::size_of::<Vertex>();

/// The row of a triangle no instance places (what the build flattens): its material, its
/// object, its flags (`scene::TRI_*`), its emitter entry + 1. A mesh's triangle needs none on the GPU: a ray's hit
/// says which section of its mesh it is in, and the rest is its instance's. (The build has one for every triangle,
/// a mesh's with its section and its index there: `Indexed::rows`.)
pub type TriRow = [u32; 4];

/// A triangle as a software tree keeps it in its leaves, the tree's own copy (hardware traversal's acceleration
/// structures keep theirs): (v0, id), (v1 - v0, id), (v2 - v0, -), so a candidate's test reads one place. The ids:
/// a world-space triangle's flags (`scene::TRI_*`), -; a mesh's section, and its index in the section. Three vec4s a
/// triangle, in the triangles' order (a tree's leaves name runs of it).
pub type Leaf = [[f32; 4]; 3];
pub const LEAF_BYTES: usize = std::mem::size_of::<Leaf>();

/// A triangle's leaf, of its corners.
pub fn leaf(p: [glam::Vec3A; 3], ids: [u32; 2]) -> Leaf {
    let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
    [[p[0].x, p[0].y, p[0].z, f32::from_bits(ids[0])], [e1.x, e1.y, e1.z, f32::from_bits(ids[1])], [e2.x, e2.y, e2.z, 0.0]]
}

/// A scope's triangles as vertices and indices that count from the scope's first vertex.
#[derive(Clone, Debug, Default)]
pub struct Scope {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<[u32; 3]>,
}

/// A triangle's corner `c` out of its attributes: (normal, uv, tangent).
fn corner(a: &GpuTriAttr, c: usize) -> ([f32; 3], [f32; 2], [f32; 4]) {
    match c {
        0 => ([a.n0[0], a.n0[1], a.n0[2]], [a.n0[3], a.n1[3]], a.t),
        1 => ([a.n1[0], a.n1[1], a.n1[2]], [a.n2[3], a.uv[0]], a.t1),
        _ => ([a.n2[0], a.n2[1], a.n2[2]], [a.uv[1], a.uv[2]], a.t2),
    }
}

/// `n` triangles as shared vertices. `vertex(k, c)`: corner c of triangle k: what tells it from every other vertex,
/// and its record. Two corners are one vertex where their keys are equal.
/// A hasher for keys of plain words (vertices' bits, triangles' corners), several times SipHash's speed: FxHash's mix
/// (rustc's). No map keyed with it is iterated, and its keys come from the scene, not from an adversary.
#[derive(Default, Clone, Copy)]
pub struct FastHash(u64);
impl FastHash {
    #[inline]
    fn add(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}
impl std::hash::Hasher for FastHash {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for c in bytes.chunks(8) {
            let mut w = [0u8; 8];
            w[..c.len()].copy_from_slice(c);
            self.add(u64::from_le_bytes(w));
        }
    }
    fn write_u32(&mut self, x: u32) {
        self.add(x as u64);
    }
    fn write_u64(&mut self, x: u64) {
        self.add(x);
    }
    fn write_usize(&mut self, x: usize) {
        self.add(x as u64);
    }
}
pub type FastMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<FastHash>>;
pub type FastSet<K> = std::collections::HashSet<K, std::hash::BuildHasherDefault<FastHash>>;

fn share<K: Hash + Eq>(n: usize, vertex: impl Fn(usize, usize) -> (K, Vertex)) -> Scope {
    let mut s = Scope { indices: Vec::with_capacity(n), ..Default::default() };
    let mut seen: FastMap<K, u32> = FastMap::with_capacity_and_hasher(n, Default::default());
    // (A triangle with a corner that is not finite: three times one vertex at the origin, a triangle without size,
    // which no ray meets. The BLASes are built from these positions.)
    let mut nowhere = None;
    for k in 0..n {
        if (0..3).any(|c| vertex(k, c).1[0][..3].iter().any(|x| !x.is_finite())) {
            let v = *nowhere.get_or_insert_with(|| {
                s.vertices.push([[0.0; 4]; 3]);
                s.vertices.len() as u32 - 1
            });
            s.indices.push([v; 3]);
            continue;
        }
        let mut row = [0u32; 3];
        for (c, at) in row.iter_mut().enumerate() {
            let (key, record) = vertex(k, c);
            *at = *seen.entry(key).or_insert_with(|| {
                s.vertices.push(record);
                s.vertices.len() as u32 - 1
            });
        }
        s.indices.push(row);
    }
    s
}

/// A scope's triangles as shared vertices: corners with the same position, normal, UV and tangent (bit for bit) are
/// one vertex.
pub fn shared(tris: &[[Vec3A; 3]], attrs: &[GpuTriAttr]) -> Scope {
    share(tris.len(), |k, c| {
        let (p, (n, uv, t)) = (tris[k][c], corner(&attrs[k], c));
        let key = [p.x, p.y, p.z, n[0], n[1], n[2], uv[0], uv[1], t[0], t[1], t[2], t[3]].map(f32::to_bits);
        (key, [[p.x, p.y, p.z, uv[0]], [n[0], n[1], n[2], uv[1]], t])
    })
}

/// A skinned mesh's triangles as shared vertices: corners are one vertex where the model's corners they are skinned
/// from are the same (`source`, a triangle's three in the mesh's order: position, normal, tangent, joints and
/// weights, bit for bit), their UVs are and their tangents' hands. A vertex's values are the build's pose's. One
/// whose model has no normal has a normal of zeros (its triangle's is made where it is hit).
pub fn shared_skinned(source: &[[crate::skin::SkinCorner; 3]], tris: &[[Vec3A; 3]], attrs: &[GpuTriAttr]) -> Scope {
    share(tris.len(), |k, c| {
        let (p, (n, uv, t), from) = (tris[k][c], corner(&attrs[k], c), source[k][c]);
        let mut key = [0u32; 19];
        key[..16].copy_from_slice(bytemuck::cast_slice(&[from]));
        (key[16], key[17], key[18]) = (uv[0].to_bits(), uv[1].to_bits(), t[3].to_bits());
        let n = if from.t[3].to_bits() & crate::skin::CORNER_NORMAL != 0 { n } else { [0.0; 3] };
        let t = if from.t[3].to_bits() & crate::skin::CORNER_TANGENT != 0 { t } else { [0.0; 4] };
        (key, [[p.x, p.y, p.z, uv[0]], [n[0], n[1], n[2], uv[1]], t])
    })
}

/// The model's vertices a skinned mesh's are skinned from: for each vertex `scope` found (`shared_skinned`), the
/// corner of the model that was it first.
pub fn source_vertices(source: &[[crate::skin::SkinCorner; 3]], scope: &Scope) -> Vec<crate::skin::SkinCorner> {
    let mut out = vec![crate::skin::SkinCorner::default(); scope.vertices.len()];
    let mut seen = vec![false; out.len()];
    for (tri, from) in scope.indices.iter().zip(source) {
        for (c, &v) in tri.iter().enumerate() {
            if !std::mem::replace(&mut seen[v as usize], true) {
                out[v as usize] = from[c];
            }
        }
    }
    out
}

/// A scope's triangles with three vertices each: what is written whole, a triangle at a time.
pub fn apart(tris: &[[Vec3A; 3]], attrs: &[GpuTriAttr]) -> Scope {
    let mut s = Scope::default();
    for (k, (t, a)) in tris.iter().zip(attrs).enumerate() {
        // (One with a corner that is not finite: its three vertices at the origin, as `share` has it.)
        let finite = t.iter().all(|p| p.is_finite());
        for (c, p) in t.iter().enumerate() {
            let (n, uv, tangent) = corner(a, c);
            let p = if finite { *p } else { Vec3A::ZERO };
            s.vertices.push([[p.x, p.y, p.z, uv[0]], [n[0], n[1], n[2], uv[1]], tangent]);
        }
        s.indices.push([k as u32 * 3, k as u32 * 3 + 1, k as u32 * 3 + 2]);
    }
    s
}

/// A particle system's quads (two triangles each, the second the first's first and third corners and its own
/// third) as four vertices a quad, in the order the particle pass writes them.
pub fn quads(tris: &[[Vec3A; 3]], attrs: &[GpuTriAttr]) -> Scope {
    let mut s = Scope::default();
    for (q, (t, a)) in tris.chunks_exact(2).zip(attrs.chunks_exact(2)).enumerate() {
        for (k, c) in [(0, 0), (0, 1), (0, 2), (1, 2)] {
            let (p, (n, uv, tangent)) = (t[k][c], corner(&a[k], c));
            s.vertices.push([[p.x, p.y, p.z, uv[0]], [n[0], n[1], n[2], uv[1]], tangent]);
        }
        let v = q as u32 * 4;
        s.indices.extend([[v, v + 1, v + 2], [v, v + 2, v + 3]]);
    }
    s
}

/// Triangles written whole as their vertices alone (their indices do not change): `corners`, three a triangle, and
/// their attributes; the vertices, as `apart` has them.
pub fn apart_vertices(corners: &[[f32; 3]], attrs: &[GpuTriAttr]) -> Vec<Vertex> {
    let mut out = Vec::with_capacity(corners.len());
    for (t, a) in corners.chunks_exact(3).zip(attrs) {
        let finite = t.iter().flatten().all(|x| x.is_finite());
        for (c, p) in t.iter().enumerate() {
            let (n, uv, tangent) = corner(a, c);
            let p = if finite { *p } else { [0.0; 3] };
            out.push([[p[0], p[1], p[2], uv[0]], [n[0], n[1], n[2], uv[1]], tangent]);
        }
    }
    out
}

/// The scene's geometry: every scope's vertices one after another, the triangles in the GPU's order.
#[derive(Clone, Debug, Default)]
pub struct Indexed {
    pub vertices: Vec<Vertex>,
    /// A triangle's three vertices: a mesh's count from the mesh's first vertex, a world-space triangle's from 0.
    pub indices: Vec<[u32; 3]>,
    pub rows: Vec<TriRow>,
    /// Software traversal: every triangle as its tree's leaf has it. Empty under hardware traversal.
    pub leaves: Vec<Leaf>,
    /// The meshes' first triangle: those before it are in world space.
    pub pool_first: u32,
    /// By mesh (`Scene::meshes`): its first vertex and how many it has.
    pub meshes: Vec<(u32, u32)>,
    /// The world-space triangles' vertices: the first so many (the still ones', then those that move).
    pub flat: u32,
    /// By skin source (`Scene::skin_sources`): the model's vertices a mesh of it is skinned from (`source_vertices`)
    /// and its triangles' indices, which every mesh of the source has. Empty: no mesh is skinned from it.
    pub skins: Vec<(Vec<crate::skin::SkinCorner>, Vec<[u32; 3]>)>,
    /// The vertices that move (the dynamic world-space triangles', then the deforming meshes'): the first and how
    /// many. Their previous frame's positions are kept in this order.
    pub dynamic: (u32, u32),
}

impl Indexed {
    /// The rows the GPU holds: the world-space triangles' (a mesh's triangles have none).
    pub fn gpu_rows(&self) -> &[TriRow] {
        &self.rows[..(self.pool_first as usize).min(self.rows.len())]
    }

    /// A scope's vertices after those there are, its triangles after those there are, their indices raised by `by`
    /// (0: they count from the scope's first vertex). The scope's first vertex.
    pub fn push(&mut self, s: Scope, by: u32) -> u32 {
        let first = self.vertices.len() as u32;
        self.vertices.extend(s.vertices);
        self.indices.extend(s.indices.iter().map(|i| i.map(|v| v + by)));
        first
    }

    /// Triangle `k` as the build had it, its vertices counted from `base`: its corners, and its attributes' normals,
    /// UVs and tangents (a hand of 0 where it has none).
    #[allow(clippy::type_complexity)]
    pub fn triangle(&self, k: usize, base: u32) -> ([[f32; 3]; 3], [[f32; 3]; 3], [[f32; 2]; 3], [[f32; 4]; 3]) {
        let v = self.indices[k].map(|i| (base + i) as usize);
        let [p, n] = [0, 1].map(|r| v.map(|i| [self.vertices[i][r][0], self.vertices[i][r][1], self.vertices[i][r][2]]));
        let uv = v.map(|i| [self.vertices[i][0][3], self.vertices[i][1][3]]);
        (p, n, uv, v.map(|i| self.vertices[i][2]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec3};

    /// A sheet of `n` x `n` quads bent into a wave, as the build makes its triangles: every corner its own, with
    /// the normal and UV of its place on the sheet, and with `own` its tangent there.
    fn sheet(n: u32, own: bool) -> (Vec<[Vec3A; 3]>, Vec<GpuTriAttr>) {
        let at = |x: u32, y: u32| {
            let (u, v) = (x as f32 / n as f32, y as f32 / n as f32);
            let p = Vec3::new(u * 2.0, (u * 5.0).sin() * 0.3, v * 2.0);
            let normal = Vec3::new(-(u * 5.0).cos() * 0.75, 1.0, 0.0).normalize();
            let tangent = Vec3::new(1.0, (u * 5.0).cos() * 0.75, 0.0).normalize();
            (p, normal, Vec2::new(u, v), tangent)
        };
        let (mut tris, mut attrs) = (vec![], vec![]);
        for y in 0..n {
            for x in 0..n {
                for q in [[(x, y), (x + 1, y), (x + 1, y + 1)], [(x + 1, y + 1), (x, y + 1), (x, y)]] {
                    let c = q.map(|(x, y)| at(x, y));
                    tris.push(c.map(|c| Vec3A::from(c.0)));
                    attrs.push(crate::scene::tri_attr(c.map(|c| c.1), c.map(|c| c.2), own.then_some(c.map(|c| (c.3, -1.0))), 3));
                }
            }
        }
        (tris, attrs)
    }

    /// The scope's triangles are the build's, corner for corner and bit for bit: positions, normals, UVs, tangents
    /// (zeros for a triangle that has none).
    fn same(what: &str, s: &Scope, tris: &[[Vec3A; 3]], attrs: &[GpuTriAttr]) {
        let all = Indexed { vertices: s.vertices.clone(), indices: s.indices.clone(), ..Default::default() };
        assert_eq!(s.indices.len(), tris.len(), "{what}: a row of indices a triangle");
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        for (k, (t, a)) in tris.iter().zip(attrs).enumerate() {
            let (p, n, uv, tangent) = all.triangle(k, 0);
            for c in 0..3 {
                let (wn, wuv, wt) = corner(a, c);
                assert_eq!((bits(&p[c]), bits(&n[c]), bits(&uv[c]), bits(&tangent[c])), (bits(&t[c].to_array()), bits(&wn), bits(&wuv), bits(&wt)), "{what}: triangle {k}, corner {c}");
            }
        }
    }

    /// Triangles that the build made from a sheet of (n + 1)^2 vertices come back as that many, with tangents or
    /// without; and each triangle read through its indices is the build's. Written whole, they have three vertices
    /// each.
    #[test]
    fn a_scopes_triangles_share_their_vertices() {
        for own in [false, true] {
            let (tris, attrs) = sheet(12, own);
            let s = shared(&tris, &attrs);
            same(&format!("own tangents {own}"), &s, &tris, &attrs);
            assert_eq!(s.vertices.len(), 13 * 13, "own tangents {own}: the sheet's vertices, each once");
            let a = apart(&tris, &attrs);
            same(&format!("apart, own tangents {own}"), &a, &tris, &attrs);
            assert_eq!(a.vertices.len(), tris.len() * 3);
        }
    }

    /// Two triangles along an edge share its two corners; with another UV on one's side (a texture's seam), another
    /// normal (a crease), another tangent, or the other hand (a mirrored texture's seam), each keeps its own.
    #[test]
    fn a_seam_or_a_crease_keeps_its_vertices() {
        let p = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0)];
        let tris = vec![[p[0], p[1], p[2]].map(Vec3A::from), [p[2], p[1], p[3]].map(Vec3A::from)];
        let uv = |v: Vec3| Vec2::new(v.x, v.y);
        let one = Some([(Vec3::X, 1.0); 3]);
        for (what, shift, normal, tangent, vertices) in [
            ("one sheet", Vec2::ZERO, Vec3::Z, one, 4),
            ("a seam", Vec2::new(0.5, 0.0), Vec3::Z, one, 6),
            ("a crease", Vec2::ZERO, Vec3::new(0.6, 0.0, 0.8), one, 6),
            ("another tangent", Vec2::ZERO, Vec3::Z, Some([(Vec3::Y, 1.0); 3]), 6),
            ("the other hand", Vec2::ZERO, Vec3::Z, Some([(Vec3::X, -1.0); 3]), 6),
            ("no tangent beside one", Vec2::ZERO, Vec3::Z, None, 6),
        ] {
            let attrs = vec![
                crate::scene::tri_attr([Vec3::Z; 3], [p[0], p[1], p[2]].map(uv), one, 3),
                crate::scene::tri_attr([normal; 3], [p[2], p[1], p[3]].map(|v| uv(v) + shift), tangent, 3),
            ];
            let s = shared(&tris, &attrs);
            same(what, &s, &tris, &attrs);
            assert_eq!(s.vertices.len(), vertices, "{what}: its vertices");
        }
    }

    /// A skinned mesh's corners are one vertex where they are skinned from the same corner of the model and have the
    /// same UV and the same hand: two triangles along an edge whose corners there are the model's same two, one
    /// vertex each; with the other hand on one side, each side its own.
    #[test]
    fn a_skinned_meshs_corners_share_by_their_models_corner_and_their_hand() {
        let p = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0)];
        let (order, uv) = ([[0, 1, 2], [2, 1, 3]], |v: Vec3| Vec2::new(v.x, v.y));
        let tris: Vec<[Vec3A; 3]> = order.iter().map(|o| o.map(|i| Vec3A::from(p[i]))).collect();
        let from = |i: usize| crate::skin::SkinCorner { p: [p[i].x, p[i].y, p[i].z, 0.0], n: [0.0, 0.0, 1.0, 0.0], t: [1.0, 0.0, 0.0, f32::from_bits(crate::skin::CORNER_NORMAL | crate::skin::CORNER_TANGENT)], w: [1.0, 0.0, 0.0, 0.0] };
        let source: Vec<[crate::skin::SkinCorner; 3]> = order.iter().map(|o| o.map(from)).collect();
        for (what, hand, vertices) in [("one hand", 1.0, 4), ("the other hand on one side", -1.0, 6)] {
            let attrs = vec![
                crate::scene::tri_attr([Vec3::Z; 3], order[0].map(|i| uv(p[i])), Some([(Vec3::X, 1.0); 3]), 3),
                crate::scene::tri_attr([Vec3::Z; 3], order[1].map(|i| uv(p[i])), Some([(Vec3::X, hand); 3]), 3),
            ];
            let s = shared_skinned(&source, &tris, &attrs);
            same(what, &s, &tris, &attrs);
            assert_eq!(s.vertices.len(), vertices, "{what}: its vertices");
        }
    }

    /// A triangle with a corner that is not finite is three times one vertex at the origin (no ray meets it; a BLAS is
    /// built from these positions), shared or apart; the others are as they were.
    #[test]
    fn a_triangle_that_is_not_finite_has_no_size() {
        let (mut tris, attrs) = sheet(4, false);
        tris[5][1] = Vec3A::new(f32::NAN, 0.0, 0.0);
        tris[9][2] = Vec3A::new(0.0, f32::INFINITY, 0.0);
        for s in [shared(&tris, &attrs), apart(&tris, &attrs)] {
            assert!(s.vertices.iter().flat_map(|v| v[0]).all(|x| x.is_finite()), "every position is finite");
            for k in [5, 9] {
                let p = s.indices[k].map(|v| s.vertices[v as usize][0]);
                assert!(p[0][..3] == [0.0; 3] && p[0][..3] == p[1][..3] && p[1][..3] == p[2][..3], "triangle {k}: {p:?}");
            }
            let all = Indexed { vertices: s.vertices.clone(), indices: s.indices.clone(), ..Default::default() };
            for (k, t) in tris.iter().enumerate().filter(|(k, _)| ![5, 9].contains(k)) {
                assert_eq!(all.triangle(k, 0).0, t.map(|p| p.to_array()), "triangle {k}'s corners");
            }
        }
        let corners: Vec<[f32; 3]> = tris.iter().flatten().map(|p| p.to_array()).collect();
        assert!(apart_vertices(&corners, &attrs).iter().flat_map(|v| v[0]).all(|x| x.is_finite()));
    }
}
