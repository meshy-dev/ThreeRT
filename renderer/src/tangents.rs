//! Tangents for geometry whose file has none: one a corner of each triangle, by the algorithm normal maps are baked
//! against (MikkTSpace), from the corners' positions, normals and UVs. Corners of one vertex that come out with
//! different tangents (a seam of a mirrored texture) are vertices apart afterwards (indexed.rs).

/// The tangents of `n` triangles, a corner each: the direction of u on the surface and the frame's hand (the
/// bitangent is cross(normal, tangent) x hand and points up the image). `corner(k, c)`: corner c of triangle k: its
/// position, its normal and its UV (v growing down the image). None: a triangle the algorithm gives no frame (it has
/// no area, on the surface or in the texture, and no neighbour to take one from), or one that is not finite.
pub fn generate(n: usize, corner: impl Fn(usize, usize) -> ([f32; 3], [f32; 3], [f32; 2])) -> Vec<Option<[[f32; 4]; 3]>> {
    struct Faces<F> {
        corner: F,
        out: Vec<Option<[[f32; 4]; 3]>>,
    }
    impl<F: Fn(usize, usize) -> ([f32; 3], [f32; 3], [f32; 2])> bevy_mikktspace::Geometry for Faces<F> {
        fn num_faces(&self) -> usize {
            self.out.len()
        }
        fn num_vertices_of_face(&self, _: usize) -> usize {
            3
        }
        fn position(&self, face: usize, vert: usize) -> [f32; 3] {
            (self.corner)(face, vert).0
        }
        fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
            (self.corner)(face, vert).1
        }
        fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
            (self.corner)(face, vert).2
        }
        fn set_tangent(&mut self, space: Option<bevy_mikktspace::TangentSpace>, face: usize, vert: usize) {
            // (The algorithm's hand is for a v that grows up the image: ours is the other.)
            let t = space.map(|s| s.tangent_encoded()).map(|t| [t[0], t[1], t[2], -t[3]]);
            // (A corner without a frame, or with one that is not finite, keeps its hand of 0: see below.)
            let whole = t.filter(|t| t.iter().all(|x| x.is_finite()) && t[..3].iter().any(|x| *x != 0.0));
            if let (Some(t), Some(tri)) = (whole, &mut self.out[face]) {
                tri[vert] = t;
            }
        }
    }
    let mut faces = Faces { corner, out: vec![Some([[0.0; 4]; 3]); n] };
    // (Its error type has no value: it cannot fail.)
    let _ = bevy_mikktspace::generate_tangents(&mut faces);
    // (A triangle with a corner the algorithm gave nothing has none.)
    for t in &mut faces.out {
        if t.is_some_and(|t| t.iter().any(|c| c[3] == 0.0)) {
            *t = None;
        }
    }
    faces.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec2, Vec3};

    /// A quad in the xy plane facing +z, its texture upright (u along x, the image's top at the larger y): the
    /// tangent is +x and the hand 1, the bitangent cross(n, t) x hand pointing up the image. With the texture
    /// mirrored (u against x) the tangent is -x and the hand -1: the bitangent still points up the image.
    #[test]
    fn a_quads_tangent_is_along_u_and_its_bitangent_up_the_image() {
        let p = [Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y];
        let tris = [[0, 1, 2], [0, 2, 3]];
        for (what, mirrored) in [("upright", false), ("mirrored", true)] {
            let uv = |v: Vec3| Vec2::new(if mirrored { 1.0 - v.x } else { v.x }, 1.0 - v.y);
            let got = generate(2, |k, c| (p[tris[k][c]].to_array(), [0.0, 0.0, 1.0], uv(p[tris[k][c]]).to_array()));
            let along = if mirrored { -1.0 } else { 1.0 };
            for t in got.iter().flat_map(|t| t.expect("a frame")) {
                assert!((Vec3::new(t[0], t[1], t[2]) - Vec3::X * along).length() < 1e-5 && t[3] == along, "{what}: {t:?}");
                let up = Vec3::Z.cross(Vec3::new(t[0], t[1], t[2])) * t[3];
                assert!((up - Vec3::Y).length() < 1e-5, "{what}: the bitangent points up the image: {up}");
            }
        }
    }

    /// A triangle without area in the texture and with no neighbour has no frame; one beside it keeps its own.
    #[test]
    fn a_triangle_without_uvs_has_no_frame() {
        let p = [Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::new(5.0, 0.0, 0.0), Vec3::new(6.0, 0.0, 0.0), Vec3::new(5.0, 1.0, 0.0)];
        let got = generate(2, |k, c| (p[k * 3 + c].to_array(), [0.0, 0.0, 1.0], if k == 0 { [p[c].x, 1.0 - p[c].y] } else { [0.5, 0.5] }));
        assert!(got[0].is_some() && got[1].is_none(), "{got:?}");
    }

    /// (A measurement, not a test: `cargo test --release -- --ignored --nocapture tangents::tests::how_long`, with
    /// THREERT_TANGENT_FILES=<a.glb,b.glb>.) The time to make tangents for every triangle of the files, and how many
    /// vertices more they have afterwards.
    #[test]
    #[ignore]
    fn how_long() {
        let files = std::env::var("THREERT_TANGENT_FILES").unwrap_or_default();
        for f in files.split(',').filter(|f| !f.is_empty()) {
            let (doc, bufs, _) = gltf::import(f).unwrap();
            let (mut tris, mut verts, mut split, mut none, mut secs) = (0usize, 0usize, 0usize, 0usize, 0.0);
            let mut against = [0usize; 4];
            for mesh in doc.meshes() {
                for prim in mesh.primitives() {
                    let r = prim.reader(|b| Some(&bufs[b.index()]));
                    let p: Vec<[f32; 3]> = r.read_positions().unwrap().collect();
                    let Some(n) = r.read_normals().map(|n| n.collect::<Vec<[f32; 3]>>()) else { continue };
                    let uv: Vec<[f32; 2]> = r.read_tex_coords(0).map(|t| t.into_f32().collect()).unwrap_or_else(|| vec![[0.0; 2]; p.len()]);
                    let ix: Vec<u32> = r.read_indices().map(|i| i.into_u32().collect()).unwrap_or_else(|| (0..p.len() as u32).collect());
                    let t0 = std::time::Instant::now();
                    let made = generate(ix.len() / 3, |k, c| {
                        let v = ix[k * 3 + c] as usize;
                        (p[v], n[v], uv[v])
                    });
                    secs += t0.elapsed().as_secs_f64();
                    let mut seen = std::collections::HashSet::new();
                    for (k, t) in made.iter().enumerate() {
                        none += t.is_none() as usize;
                        for c in 0..3 {
                            seen.insert((ix[k * 3 + c], t.map_or([0u32; 4], |t| t[c].map(f32::to_bits))));
                        }
                    }
                    (tris, verts, split) = (tris + ix.len() / 3, verts + p.len(), split + seen.len());
                    // (Against the file's own, where it has them: the angle between the two and the hands.)
                    if let Some(own) = r.read_tangents().map(|t| t.collect::<Vec<[f32; 4]>>()) {
                        for (k, t) in made.iter().enumerate() {
                            let Some(t) = t else { continue };
                            for c in 0..3 {
                                let o = own[ix[k * 3 + c] as usize];
                                let cos = (t[c][0] * o[0] + t[c][1] * o[1] + t[c][2] * o[2]).clamp(-1.0, 1.0);
                                against[0] += 1;
                                against[1] += (cos.acos().to_degrees() > 1.0) as usize;
                                against[2] += (cos.acos().to_degrees() > 10.0) as usize;
                                against[3] += (t[c][3] != o[3]) as usize;
                            }
                        }
                    }
                }
            }
            eprintln!("{f}: {tris} triangles in {secs:.2} s ({:.2} M a second); {verts} vertices -> {split} ({:+.1} %); {none} triangles without a frame", tris as f64 / secs / 1e6, (split as f64 / verts as f64 - 1.0) * 100.0);
            if against[0] > 0 {
                eprintln!("    against the file's tangents: {} corners, {} more than 1 degree off, {} more than 10, {} of the other hand", against[0], against[1], against[2], against[3]);
            }
        }
    }
}
