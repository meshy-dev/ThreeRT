//! A voxel object as triangles: its exposed faces (a set voxel next to an empty one), merged greedily into rectangles of
//! one material per face plane, two triangles each. The same rectangles as packages/voxel `greedyMesh` (rows scanned
//! in the same order, extended along u then w), found by bit operations: per axis, each column's voxels are a bitmask,
//! its faces `solid & !neighbour` a shift away.

use crate::grid::VoxelGrid;
use std::collections::BTreeMap;

/// One material's triangles (object space: voxel (0, 0, 0)'s corner at the mesh's origin), 4 vertices per rectangle.
/// Merged by class (`merge_by_class`): `material` is the class, `colors` each vertex's voxel colour (linear RGB).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshPart {
    pub material: u8,
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub indices: Vec<u32>,
    pub colors: Vec<f32>,
}

/// A mesh's parts (one per material) merged into one per class (`class[m]` of material m: materials alike in all but
/// their colour share one), every vertex carrying its material's colour (`color[m]`, linear RGB): fewer meshes, the same
/// triangles (a rectangle never spans two materials). Classes ascending, each part's triangles in material order.
pub fn merge_by_class(parts: Vec<MeshPart>, class: &[u8], color: &[[f32; 3]]) -> Vec<MeshPart> {
    let mut out: BTreeMap<u8, MeshPart> = BTreeMap::new();
    for p in parts {
        let c = class.get(p.material as usize).copied().unwrap_or(p.material);
        let rgb = color.get(p.material as usize).copied().unwrap_or([1.0; 3]);
        let q = out.entry(c).or_insert_with(|| MeshPart { material: c, ..Default::default() });
        let base = (q.positions.len() / 3) as u32;
        q.indices.extend(p.indices.iter().map(|i| i + base));
        q.colors.extend(std::iter::repeat_n(rgb, p.positions.len() / 3).flatten());
        q.positions.extend(p.positions);
        q.normals.extend(p.normals);
    }
    out.into_values().collect()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoxelMesh {
    /// By material, ascending.
    pub parts: Vec<MeshPart>,
    /// Exposed voxel faces, and the rectangles they merged into (triangles = 2 x quads).
    pub faces: usize,
    pub quads: usize,
}

/// The grid's exposed faces, greedy-meshed, voxel (0, 0, 0)'s corner at `origin` with voxels of `size`.
/// `solid_outside(x, y, z)`: whether an empty voxel of this grid is filled by something else that hides the face (a
/// neighbouring terrain tile, the ground under the bedrock).
pub fn greedy_mesh(g: &VoxelGrid, origin: [f32; 3], size: f32, solid_outside: &mut dyn FnMut(i32, i32, i32) -> bool) -> VoxelMesh {
    let Some((lo, hi)) = g.bounds() else { return VoxelMesh::default() };
    let dims = [0, 1, 2].map(|a| (hi[a] - lo[a]) as usize);
    let at = |x: usize, y: usize, z: usize| x + dims[0] * (y + dims[1] * z);
    // Materials (dense, for the faces' material) and, per axis, each column's voxels as bits along that axis, filled in
    // one pass over the bricks; a full brick of one material (most of a terrain tile) by rows of 8.
    let words = dims.map(|d| d.div_ceil(64));
    let col_at = |a: usize, c: [usize; 3]| (c[(a + 2) % 3] * dims[(a + 1) % 3] + c[(a + 1) % 3]) * words[a] + (c[a] >> 6);
    let mut cols: [Vec<u64>; 3] = [0, 1, 2].map(|a| vec![0u64; dims[(a + 1) % 3] * dims[(a + 2) % 3] * words[a]]);
    let mut dense = vec![0u8; dims[0] * dims[1] * dims[2]];
    for (k, b) in &g.bricks {
        let base = [k.0 * 8 - lo[0], k.1 * 8 - lo[1], k.2 * 8 - lo[2]].map(|v| v as usize);
        if b.iter().all(|&v| v == b[0]) && b[0] != 0 {
            for z in 0..8 {
                for y in 0..8 {
                    let i = at(base[0], base[1] + y, base[2] + z);
                    dense[i..i + 8].fill(b[0]);
                }
            }
            for (a, col) in cols.iter_mut().enumerate() {
                let (ua, wa) = ((a + 1) % 3, (a + 2) % 3);
                let bits = 0xffu64 << (base[a] & 63);
                for w in 0..8 {
                    for u in 0..8 {
                        let mut c = base;
                        c[ua] += u;
                        c[wa] += w;
                        col[col_at(a, c)] |= bits;
                    }
                }
            }
            continue;
        }
        for (v, &m) in b.iter().enumerate() {
            if m == 0 {
                continue;
            }
            let c = [base[0] + (v & 7), base[1] + ((v >> 3) & 7), base[2] + (v >> 6)];
            dense[at(c[0], c[1], c[2])] = m;
            for (a, col) in cols.iter_mut().enumerate() {
                col[col_at(a, c)] |= 1 << (c[a] & 63);
            }
        }
    }
    let (mut faces, mut quads) = (0usize, 0usize);
    let mut parts: BTreeMap<u8, MeshPart> = BTreeMap::new();
    let s = size as f64;
    for a in 0..3 {
        let (ua, wa) = ((a + 1) % 3, (a + 2) % 3);
        let (da, du, dw) = (dims[a], dims[ua], dims[wa]);
        let words = words[a];
        let cols = &cols[a];
        let mut mask = vec![0u8; du * dw];
        for sign in 0..2 {
            // Faces as (layer, u, w), gathered in (w, u) order, then grouped by layer keeping it (counting sort): each
            // layer's faces in the order the TS mesher sorts them in.
            let mut found: Vec<(u32, u32, u32)> = vec![];
            for w in 0..dw {
                for u in 0..du {
                    let col = &cols[(w * du + u) * words..][..words];
                    for wi in 0..words {
                        let s0 = col[wi];
                        let neighbour = if sign == 0 { (s0 >> 1) | col.get(wi + 1).map_or(0, |n| n << 63) } else { (s0 << 1) | if wi > 0 { col[wi - 1] >> 63 } else { 0 } };
                        let mut f = s0 & !neighbour;
                        while f != 0 {
                            let d = wi * 64 + f.trailing_zeros() as usize;
                            f &= f - 1;
                            let mut n = [0i32; 3];
                            n[a] = lo[a] + d as i32 + if sign == 0 { 1 } else { -1 };
                            n[ua] = lo[ua] + u as i32;
                            n[wa] = lo[wa] + w as i32;
                            if solid_outside(n[0], n[1], n[2]) {
                                continue;
                            }
                            found.push((d as u32, u as u32, w as u32));
                            faces += 1;
                        }
                    }
                }
            }
            let mut start = vec![0usize; da + 1];
            for &(d, _, _) in &found {
                start[d as usize + 1] += 1;
            }
            for d in 0..da {
                start[d + 1] += start[d];
            }
            let mut fill = start.clone();
            let mut sorted = vec![(0u32, 0u32); found.len()];
            for &(d, u, w) in &found {
                sorted[fill[d as usize]] = (u, w);
                fill[d as usize] += 1;
            }
            for d in 0..da {
                let list = &sorted[start[d]..start[d + 1]];
                let mut c = [0usize; 3];
                c[a] = d;
                for &(u, w) in list {
                    c[ua] = u as usize;
                    c[wa] = w as usize;
                    mask[w as usize * du + u as usize] = dense[at(c[0], c[1], c[2])];
                }
                let plane = lo[a] + d as i32 + if sign == 0 { 1 } else { 0 };
                for &(u, w) in list {
                    let (u, w) = (u as usize, w as usize);
                    let i = w * du + u;
                    let m = mask[i];
                    if m == 0 {
                        continue; // taken by an earlier rectangle
                    }
                    let mut nu = 1;
                    while u + nu < du && mask[i + nu] == m {
                        nu += 1;
                    }
                    let mut nw = 1;
                    'grow: while w + nw < dw {
                        for k in 0..nu {
                            if mask[i + nw * du + k] != m {
                                break 'grow;
                            }
                        }
                        nw += 1;
                    }
                    for r in 0..nw {
                        mask[i + r * du..i + r * du + nu].fill(0);
                    }
                    quads += 1;
                    let p = parts.entry(m).or_insert_with(|| MeshPart { material: m, ..Default::default() });
                    let base = (p.positions.len() / 3) as u32;
                    let (u0, w0) = (lo[ua] + u as i32, lo[wa] + w as i32);
                    for (cu, cw) in [(u0, w0), (u0 + nu as i32, w0), (u0 + nu as i32, w0 + nw as i32), (u0, w0 + nw as i32)] {
                        let mut v = [0i32; 3];
                        v[a] = plane;
                        v[ua] = cu;
                        v[wa] = cw;
                        for k in 0..3 {
                            p.positions.push((origin[k] as f64 + v[k] as f64 * s) as f32);
                        }
                        let mut nrm = [0.0f32; 3];
                        nrm[a] = if sign == 0 { 1.0 } else { -1.0 };
                        p.normals.extend_from_slice(&nrm);
                    }
                    // (u, w) follow the axis cyclically, so u x w is +axis: counter-clockwise seen from the + side.
                    if sign == 0 {
                        p.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
                    } else {
                        p.indices.extend_from_slice(&[base, base + 2, base + 1, base, base + 3, base + 2]);
                    }
                }
            }
        }
    }
    VoxelMesh { parts: parts.into_values().collect(), faces, quads }
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    /// Two materials of one class and one of another: two parts, the same triangles, each vertex its material's colour.
    #[test]
    fn parts_merge_by_class_with_vertex_colours() {
        let mut g = VoxelGrid::default();
        g.set(0, 0, 0, 1);
        g.set(1, 0, 0, 2);
        g.set(3, 0, 0, 3);
        let m = greedy_mesh(&g, [0.0; 3], 0.1, &mut |_, _, _| false);
        let tris: usize = m.parts.iter().map(|p| p.indices.len() / 3).sum();
        let colors = [[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let merged = merge_by_class(m.parts.clone(), &[0, 0, 0, 1], &colors);
        assert_eq!(merged.iter().map(|p| p.material).collect::<Vec<_>>(), vec![0, 1]);
        assert_eq!(merged.iter().map(|p| p.indices.len() / 3).sum::<usize>(), tris);
        for p in &merged {
            assert_eq!(p.colors.len(), p.positions.len());
            assert!(p.indices.iter().all(|&i| (i as usize) < p.positions.len() / 3));
        }
        // Class 1 holds material 3 alone: blue everywhere; class 0 red and green vertices, as many as their parts had.
        assert!(merged[1].colors.chunks(3).all(|c| c == [0.0, 0.0, 1.0]));
        let red = merged[0].colors.chunks(3).filter(|c| *c == [1.0, 0.0, 0.0]).count();
        assert_eq!(red, m.parts.iter().find(|p| p.material == 1).unwrap().positions.len() / 3);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const DIRS: [[i32; 3]; 6] = [[1, 0, 0], [-1, 0, 0], [0, 1, 0], [0, -1, 0], [0, 0, 1], [0, 0, -1]];

    /// Every exposed face by brute force: (x, y, z, direction) -> material.
    fn exposed(g: &VoxelGrid, solid: &dyn Fn(i32, i32, i32) -> bool) -> BTreeMap<(i32, i32, i32, usize), u8> {
        let mut f = BTreeMap::new();
        for (k, b) in &g.bricks {
            for v in 0..512usize {
                let m = b[v];
                if m == 0 {
                    continue;
                }
                let (x, y, z) = (k.0 * 8 + (v & 7) as i32, k.1 * 8 + ((v >> 3) & 7) as i32, k.2 * 8 + (v >> 6) as i32);
                for (d, [dx, dy, dz]) in DIRS.iter().enumerate() {
                    if g.get(x + dx, y + dy, z + dz) == 0 && !solid(x + dx, y + dy, z + dz) {
                        f.insert((x, y, z, d), m);
                    }
                }
            }
        }
        f
    }

    /// The mesh's rectangles cut back into unit faces (keyed by the voxel behind each), checking the winding.
    fn rasterize(mesh: &VoxelMesh, origin: [f32; 3], s: f32) -> BTreeMap<(i32, i32, i32, usize), u8> {
        let mut f = BTreeMap::new();
        for p in &mesh.parts {
            for q in p.indices.chunks(6) {
                let v = |k: u32| [0, 1, 2].map(|a| ((p.positions[3 * k as usize + a] - origin[a]) / s) as f64);
                let n = [0, 1, 2].map(|a| p.normals[3 * q[0] as usize + a]);
                let (a0, b0, c0) = (v(q[0]), v(q[1]), v(q[2]));
                let (e1, e2) = ([0, 1, 2].map(|i| b0[i] - a0[i]), [0, 1, 2].map(|i| c0[i] - a0[i]));
                let cross = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
                assert!(cross[0] * n[0] as f64 + cross[1] * n[1] as f64 + cross[2] * n[2] as f64 > 0.0, "counter-clockwise along the normal");
                let corners: Vec<[i32; 3]> = q.iter().map(|&k| v(k).map(|x| x.round() as i32)).collect();
                let lo = [0, 1, 2].map(|i| corners.iter().map(|c| c[i]).min().unwrap());
                let hi = [0, 1, 2].map(|i| corners.iter().map(|c| c[i]).max().unwrap());
                let axis = (0..3).find(|&i| n[i] != 0.0).unwrap();
                let dir = axis * 2 + if n[axis] > 0.0 { 0 } else { 1 };
                for z in lo[2]..hi[2].max(lo[2] + 1) {
                    for y in lo[1]..hi[1].max(lo[1] + 1) {
                        for x in lo[0]..hi[0].max(lo[0] + 1) {
                            let mut c = [x, y, z];
                            if n[axis] > 0.0 {
                                c[axis] -= 1;
                            }
                            assert!(f.insert((c[0], c[1], c[2], dir), p.material).is_none(), "face covered twice");
                        }
                    }
                }
            }
        }
        f
    }

    #[test]
    fn rectangles_cover_exactly_the_exposed_faces() {
        // The TS test's grid (packages/voxel/test/mesh.test.ts): a slab across bricks, holes, other materials, loose
        // voxels, whole buried bricks, negative coordinates.
        let mut g = VoxelGrid::default();
        let mut r: u32 = 12345;
        let mut rnd = || {
            r = r.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;
            r as f64 / 0x7fff_ffff as f64
        };
        for z in -6..11 {
            for y in -3..5 {
                for x in -9..14 {
                    g.set(x, y, z, 2);
                }
            }
        }
        for _ in 0..400 {
            let (x, y, z) = (-12 + (rnd() * 30.0) as i32, -6 + (rnd() * 16.0) as i32, -9 + (rnd() * 24.0) as i32);
            let m = if rnd() < 0.3 { 0 } else { 1 + (rnd() * 3.0) as u8 };
            g.set(x, y, z, m);
        }
        for z in -16..0 {
            for y in -16..0 {
                for x in -16..0 {
                    g.set(x, y, z, 5);
                }
            }
        }
        let mesh = greedy_mesh(&g, [0.0; 3], 0.1, &mut |_, _, _| false);
        let want = exposed(&g, &|_, _, _| false);
        assert_eq!(rasterize(&mesh, [0.0; 3], 0.1), want);
        assert_eq!(mesh.faces, want.len());
        assert!(mesh.quads * 2 < mesh.faces, "{} rectangles for {} faces", mesh.quads, mesh.faces);
    }

    #[test]
    fn a_cuboid_is_six_rectangles_and_hidden_faces_are_left_out() {
        let mut g = VoxelGrid::default();
        for z in 0..13 {
            for y in 0..9 {
                for x in 0..20 {
                    g.set(x, y, z, 4);
                }
            }
        }
        let one = greedy_mesh(&g, [1.0, 2.0, 3.0], 0.25, &mut |_, _, _| false);
        assert_eq!((one.quads, one.faces), (6, 2 * (20 * 9 + 20 * 13 + 9 * 13)));
        let xs: Vec<f32> = one.parts[0].positions.iter().step_by(3).copied().collect();
        assert_eq!((xs.iter().cloned().fold(f32::MAX, f32::min), xs.iter().cloned().fold(f32::MIN, f32::max)), (1.0, 6.0));
        for z in 0..13 {
            for y in 0..9 {
                for x in 10..20 {
                    g.set(x, y, z, 6);
                }
            }
        }
        assert_eq!(greedy_mesh(&g, [0.0; 3], 0.25, &mut |_, _, _| false).quads, 10);
        let ground = greedy_mesh(&g, [0.0; 3], 0.25, &mut |_, y, _| y < 0);
        assert_eq!(ground.quads, 8);
        assert_eq!(rasterize(&ground, [0.0; 3], 0.25), exposed(&g, &|_, y, _| y < 0));
    }

    #[test]
    fn deep_columns_cross_word_boundaries() {
        // Taller than 64 voxels along every axis, with holes at 63 / 64 / 127 / 128: the column bit shifts carry.
        let mut g = VoxelGrid::default();
        for z in 0..70 {
            for y in 0..140 {
                for x in 0..5 {
                    if !(y == 63 || y == 64 || y == 128) || x == 2 {
                        g.set(x, y, z, 1 + (y / 50) as u8);
                    }
                }
            }
        }
        let mesh = greedy_mesh(&g, [0.0; 3], 0.1, &mut |_, _, _| false);
        assert_eq!(rasterize(&mesh, [0.0; 3], 0.1), exposed(&g, &|_, _, _| false));
    }
}
