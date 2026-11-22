//! The scene's geometry as the GPU holds it: vertices kept once, a triangle three indices and a row. The build still
//! makes triangles with corners of their own (`Scene::tris`, `attrs`); what they share is found here, scope by scope:
//! a mesh's triangles share within the mesh, the still world-space triangles among themselves, and what a pass or the
//! CPU writes whole each frame (the dynamic world-space triangles, particles) has three vertices a triangle.

use crate::scene::GpuTriAttr;
use glam::Vec3A;
use std::collections::HashMap;
use std::hash::Hash;

/// `GpuTriAttr::x[0]` (bits): the triangle's tangent is its first corner's own (a file's, a particle's), not one made
/// from the triangle's UVs.
pub const OWN_TANGENT: u32 = 1;

/// A vertex, one record: (position, u), (normal, v), (tangent, hand). A tangent of zeros: the vertex has none. What a
/// hit reads of each of its triangle's three; an acceleration structure is built from the records' positions.
pub type Vertex = [[f32; 4]; 3];
pub const VERTEX_BYTES: usize = std::mem::size_of::<Vertex>();

/// The row of a triangle no instance places (a light's card, a particle's, a flattened scene's): its material, its
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

/// What a vertex's tangent slot is so far: no triangle's first corner yet; the first corner of triangles without a
/// tangent of their own (it stays zeros); the first corner of triangles with this one.
#[derive(Clone, Copy, PartialEq)]
enum Slot {
    Open,
    Without,
    Own([u32; 4]),
}

/// A triangle's corner `c` out of its attributes: (normal, uv).
fn corner(a: &GpuTriAttr, c: usize) -> ([f32; 3], [f32; 2]) {
    match c {
        0 => ([a.n0[0], a.n0[1], a.n0[2]], [a.n0[3], a.n1[3]]),
        1 => ([a.n1[0], a.n1[1], a.n1[2]], [a.n2[3], a.uv[0]]),
        _ => ([a.n2[0], a.n2[1], a.n2[2]], [a.uv[1], a.uv[2]]),
    }
}

/// The tangent a triangle's first corner carries, if it has one of its own.
fn own_tangent(a: &GpuTriAttr) -> Option<[f32; 4]> {
    (a.x[0].to_bits() & OWN_TANGENT != 0).then_some(a.t)
}

/// `n` triangles as shared vertices. `vertex(k, c)`: corner c of triangle k: what tells it from every other vertex,
/// and its (position, u) and (normal, v). `tangent(k)`: triangle k's own tangent, which its first vertex is to
/// carry. Two corners are one vertex where their keys are equal; a triangle's first corner is besides a vertex
/// whose tangent slot is its own (the same tangent, or none for both).
fn share<K: Hash + Eq>(n: usize, vertex: impl Fn(usize, usize) -> (K, [f32; 4], [f32; 4]), tangent: impl Fn(usize) -> Option<[f32; 4]>) -> Scope {
    let mut s = Scope { indices: Vec::with_capacity(n), ..Default::default() };
    let mut slots: Vec<Slot> = vec![];
    // (A key's vertices: one, but where triangles disagree on the tangent of a first corner.)
    let mut seen: HashMap<K, Vec<u32>> = HashMap::with_capacity(n);
    // (A triangle with a corner that is not finite: three times one vertex at the origin, a triangle without size,
    // which no ray meets. The BLASes are built from these positions.)
    let mut nowhere = None;
    for k in 0..n {
        if (0..3).any(|c| vertex(k, c).1[..3].iter().any(|x| !x.is_finite())) {
            let v = *nowhere.get_or_insert_with(|| {
                s.vertices.push([[0.0; 4]; 3]);
                slots.push(Slot::Without);
                slots.len() as u32 - 1
            });
            s.indices.push([v; 3]);
            continue;
        }
        let mut row = [0u32; 3];
        for (c, at) in row.iter_mut().enumerate() {
            let (key, p, nv) = vertex(k, c);
            let want = match (c, tangent(k)) {
                (0, Some(t)) => Slot::Own(t.map(f32::to_bits)),
                (0, None) => Slot::Without,
                _ => Slot::Open,
            };
            let list = seen.entry(key).or_default();
            let found = match want {
                Slot::Open => list.first().copied(),
                _ => list.iter().copied().find(|&v| slots[v as usize] == want).or_else(|| list.iter().copied().find(|&v| slots[v as usize] == Slot::Open)),
            };
            let v = found.unwrap_or_else(|| {
                s.vertices.push([p, nv, [0.0; 4]]);
                slots.push(Slot::Open);
                list.push(slots.len() as u32 - 1);
                slots.len() as u32 - 1
            });
            if want != Slot::Open {
                slots[v as usize] = want;
                if let Slot::Own(t) = want {
                    s.vertices[v as usize][2] = t.map(f32::from_bits);
                }
            }
            *at = v;
        }
        s.indices.push(row);
    }
    s
}

/// A scope's triangles as shared vertices: corners with the same position, normal and UV (bit for bit) are one
/// vertex. (The attributes' tangent: see `share`.)
pub fn shared(tris: &[[Vec3A; 3]], attrs: &[GpuTriAttr]) -> Scope {
    share(
        tris.len(),
        |k, c| {
            let (p, (n, uv)) = (tris[k][c], corner(&attrs[k], c));
            let key = [p.x, p.y, p.z, n[0], n[1], n[2], uv[0], uv[1]].map(f32::to_bits);
            (key, [p.x, p.y, p.z, uv[0]], [n[0], n[1], n[2], uv[1]])
        },
        |k| own_tangent(&attrs[k]),
    )
}

/// A skinned mesh's triangles as shared vertices: corners are one vertex where the model's corners they are skinned
/// from are the same (`source`, a triangle's three in the mesh's order: position, normal, tangent, joints and
/// weights, bit for bit) and their UVs are. A vertex's values are the build's pose's. One whose model has no normal
/// has a normal of zeros (its triangle's is made where it is hit), and none has a tangent that is not its own.
pub fn shared_skinned(source: &[[crate::skin::SkinCorner; 3]], tris: &[[Vec3A; 3]], attrs: &[GpuTriAttr]) -> Scope {
    share(
        tris.len(),
        |k, c| {
            let (p, (n, uv), from) = (tris[k][c], corner(&attrs[k], c), source[k][c]);
            let mut key = [0u32; 18];
            key[..16].copy_from_slice(bytemuck::cast_slice(&[from]));
            (key[16], key[17]) = (uv[0].to_bits(), uv[1].to_bits());
            let n = if from.t[3].to_bits() & crate::skin::CORNER_NORMAL != 0 { n } else { [0.0; 3] };
            (key, [p.x, p.y, p.z, uv[0]], [n[0], n[1], n[2], uv[1]])
        },
        |k| (source[k][0].t[3].to_bits() & crate::skin::CORNER_TANGENT != 0).then_some(attrs[k].t),
    )
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
        let tangent = own_tangent(a).unwrap_or([0.0; 4]);
        // (One with a corner that is not finite: its three vertices at the origin, as `share` has it.)
        let finite = t.iter().all(|p| p.is_finite());
        for (c, p) in t.iter().enumerate() {
            let (n, uv) = corner(a, c);
            let p = if finite { *p } else { Vec3A::ZERO };
            s.vertices.push([[p.x, p.y, p.z, uv[0]], [n[0], n[1], n[2], uv[1]], tangent]);
        }
        s.indices.push([k as u32 * 3, k as u32 * 3 + 1, k as u32 * 3 + 2]);
    }
    s
}

/// Triangles written whole as their vertices alone (their indices do not change): `corners`, three a triangle, and
/// their attributes; the vertices, as `apart` has them.
pub fn apart_vertices(corners: &[[f32; 3]], attrs: &[GpuTriAttr]) -> Vec<Vertex> {
    let mut out = Vec::with_capacity(corners.len());
    for (t, a) in corners.chunks_exact(3).zip(attrs) {
        let tangent = own_tangent(a).unwrap_or([0.0; 4]);
        let finite = t.iter().flatten().all(|x| x.is_finite());
        for (c, p) in t.iter().enumerate() {
            let (n, uv) = corner(a, c);
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
    /// The meshes' first triangle: those before it are in world space. And the translucent particles' first (they
    /// lie after the meshes') and how many.
    pub pool_first: u32,
    pub translucent_tris: (u32, u32),
    /// By mesh (`Scene::meshes`): its first vertex and how many it has.
    pub meshes: Vec<(u32, u32)>,
    /// The world-space triangles' vertices: the first so many (the still ones', then those that move).
    pub flat: u32,
    /// The translucent particles' first vertex (three a triangle, after every mesh's).
    pub translucent: u32,
    /// By skin source (`Scene::skin_sources`): the model's vertices a mesh of it is skinned from (`source_vertices`)
    /// and its triangles' indices, which every mesh of the source has. Empty: no mesh is skinned from it.
    pub skins: Vec<(Vec<crate::skin::SkinCorner>, Vec<[u32; 3]>)>,
    /// The vertices that move (the dynamic world-space triangles', then the deforming meshes'): the first and how
    /// many. Their previous frame's positions are kept in this order.
    pub dynamic: (u32, u32),
}

impl Indexed {
    /// The rows the GPU holds: the world-space triangles', then the translucent particles' (found from `pool_first`
    /// on: the meshes' triangles lie between and have none).
    pub fn gpu_rows(&self) -> Vec<TriRow> {
        let (first, n) = (self.translucent_tris.0 as usize, self.translucent_tris.1 as usize);
        self.rows[..(self.pool_first as usize).min(self.rows.len())].iter().chain(self.rows.get(first..first + n).unwrap_or(&[])).copied().collect()
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
    /// UVs and tangent (its first vertex's: zeros where that has none).
    pub fn triangle(&self, k: usize, base: u32) -> ([[f32; 3]; 3], [[f32; 3]; 3], [[f32; 2]; 3], [f32; 4]) {
        let v = self.indices[k].map(|i| (base + i) as usize);
        let [p, n] = [0, 1].map(|r| v.map(|i| [self.vertices[i][r][0], self.vertices[i][r][1], self.vertices[i][r][2]]));
        let uv = v.map(|i| [self.vertices[i][0][3], self.vertices[i][1][3]]);
        (p, n, uv, self.vertices[v[0]][2])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec3};

    /// A sheet of `n` x `n` quads bent into a wave, as the build makes its triangles: every corner its own, with
    /// the normal and UV of its place on the sheet. `own`: each triangle has its first corner's tangent (a file's);
    /// else one made from its UVs.
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
                    attrs.push(crate::scene::tri_attr(c.map(|c| c.0), c.map(|c| c.1), c.map(|c| c.2), own.then_some((c[0].3, -1.0)), 3));
                }
            }
        }
        (tris, attrs)
    }

    /// The scope's triangles are the build's, corner for corner and bit for bit: positions, normals, UVs, and the
    /// tangent of a triangle that has its own on its first vertex (zeros there for one that has none).
    fn same(what: &str, s: &Scope, tris: &[[Vec3A; 3]], attrs: &[GpuTriAttr]) {
        let all = Indexed { vertices: s.vertices.clone(), indices: s.indices.clone(), ..Default::default() };
        assert_eq!(s.indices.len(), tris.len(), "{what}: a row of indices a triangle");
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        for (k, (t, a)) in tris.iter().zip(attrs).enumerate() {
            let (p, n, uv, tangent) = all.triangle(k, 0);
            for c in 0..3 {
                let (wn, wuv) = corner(a, c);
                assert_eq!((bits(&p[c]), bits(&n[c]), bits(&uv[c])), (bits(&t[c].to_array()), bits(&wn), bits(&wuv)), "{what}: triangle {k}, corner {c}");
            }
            assert_eq!(bits(&tangent), bits(&own_tangent(a).unwrap_or([0.0; 4])), "{what}: triangle {k}'s tangent on its first vertex");
        }
    }

    /// Triangles that the build made from a sheet of (n + 1)^2 vertices come back as that many, whether each has a
    /// tangent of its own (a file's: its first corner's) or none; and each triangle read through its indices is the
    /// build's. Written whole, they have three vertices each.
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

    /// Two triangles along an edge share its two corners; with another UV on one's side (a texture's seam) or another
    /// normal (a crease) each keeps its own.
    #[test]
    fn a_seam_or_a_crease_keeps_its_vertices() {
        let p = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(1.0, 1.0, 0.0)];
        let tris = vec![[p[0], p[1], p[2]].map(Vec3A::from), [p[2], p[1], p[3]].map(Vec3A::from)];
        let uv = |v: Vec3| Vec2::new(v.x, v.y);
        for (what, shift, normal, vertices) in [("one sheet", Vec2::ZERO, Vec3::Z, 4), ("a seam", Vec2::new(0.5, 0.0), Vec3::Z, 6), ("a crease", Vec2::ZERO, Vec3::new(0.6, 0.0, 0.8), 6)] {
            let attrs = vec![
                crate::scene::tri_attr([p[0], p[1], p[2]], [Vec3::Z; 3], [p[0], p[1], p[2]].map(uv), None, 3),
                crate::scene::tri_attr([p[2], p[1], p[3]], [normal; 3], [p[2], p[1], p[3]].map(|v| uv(v) + shift), None, 3),
            ];
            let s = shared(&tris, &attrs);
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

    /// Where triangles disagree about a first corner's tangent the corner is a vertex for each: a sheet whose
    /// triangles each have a tangent of their own that is no other's, one with and without a tangent by turns, and
    /// one where a corner is first met as a second corner and takes its tangent later.
    #[test]
    fn a_first_corner_has_its_triangles_tangent() {
        let (tris, mut attrs) = sheet(6, true);
        for (k, a) in attrs.iter_mut().enumerate() {
            a.t = [k as f32 + 1.0, 0.5, 0.25, 1.0];
        }
        let s = shared(&tris, &attrs);
        same("a tangent a triangle", &s, &tris, &attrs);
        // (Every vertex of the sheet but two corners is the first corner of one triangle or two.)
        assert!(s.vertices.len() > 7 * 7 && s.vertices.len() <= 7 * 7 + 6 * 6, "{} vertices", s.vertices.len());
        let (tris, mut attrs) = sheet(6, true);
        for (k, a) in attrs.iter_mut().enumerate() {
            if k % 2 == 1 {
                a.x[0] = 0.0;
            }
        }
        let s = shared(&tris, &attrs);
        same("with and without by turns", &s, &tris, &attrs);
        // Two triangles of one corner each way round: the corner is the first's second and the second's first.
        let p = [Vec3A::ZERO, Vec3A::X, Vec3A::Y];
        let a = |own: bool| crate::scene::tri_attr(p.map(Vec3::from), [Vec3::Z; 3], [Vec2::ZERO, Vec2::X, Vec2::Y], own.then_some((Vec3::X, 1.0)), 0);
        // (The same triangle begun at its second corner.)
        let turned = |a: GpuTriAttr| GpuTriAttr { n0: [a.n1[0], a.n1[1], a.n1[2], a.n2[3]], n1: [a.n2[0], a.n2[1], a.n2[2], a.uv[0]], n2: [a.n0[0], a.n0[1], a.n0[2], a.uv[1]], uv: [a.uv[2], a.n0[3], a.n1[3], a.uv[3]], ..a };
        let (tris, attrs) = (vec![p, [p[1], p[2], p[0]]], vec![a(false), turned(a(true))]);
        let s = shared(&tris, &attrs);
        same("a corner met as a second corner first", &s, &tris, &attrs);
        assert_eq!(s.vertices.len(), 3, "three vertices: the shared corner took its tangent when a triangle began at it");
    }
}
