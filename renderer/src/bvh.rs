//! Binned-SAH BVH over triangles. Leaves reference contiguous ranges of the reordered triangle list.

use glam::Vec3A as V;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuNode {
    pub bmin: [f32; 3],
    /// Interior: index of the left child (right = left + 1). Leaf: first triangle.
    pub left_or_first: u32,
    pub bmax: [f32; 3],
    /// 0 for interior nodes.
    pub count: u32,
}

#[derive(Clone, Copy)]
struct Aabb {
    min: V,
    max: V,
}

impl Aabb {
    const EMPTY: Aabb = Aabb { min: V::splat(f32::INFINITY), max: V::splat(f32::NEG_INFINITY) };
    fn grow(&mut self, p: V) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    fn merge(&mut self, o: &Aabb) {
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
    }
    fn area(&self) -> f32 {
        let e = (self.max - self.min).max(V::ZERO);
        2.0 * (e.x * e.y + e.y * e.z + e.z * e.x)
    }
}

pub struct Bvh {
    pub nodes: Vec<GpuNode>,
    /// order[i] = original triangle index stored at position i.
    pub order: Vec<u32>,
}

const BINS: usize = 16;
const MAX_LEAF: usize = 4;

/// The tree over no triangle, as the kernel can walk it. A node with `count: 0` is an interior node to the kernel
/// (pathtrace.wgsl `trace`): the one-node tree this module used to make for nothing had itself for a left child and,
/// for a right child, whatever lay after the node buffer's 32 bytes. Where that was zeros the walk ended; where it was
/// what an earlier scene left there, it might not (a dispatch that does not end: docs/RUNTIME.md "The engine in a
/// browser"). The encoding has no empty leaf, so this is a leaf of one triangle, triangle 0: with no triangles the
/// vertex buffer is zeros (gpu.rs `upload`), a triangle without area that no ray meets (`intersect_tri`: its
/// determinant is 0). One node read, no child, no box.
pub const EMPTY: GpuNode = GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 1 };

pub fn build(tris: &[[V; 3]]) -> Bvh {
    if tris.is_empty() {
        return Bvh { nodes: vec![EMPTY], order: vec![] };
    }
    let n = tris.len();
    let mut boxes = Vec::with_capacity(n);
    let mut centroids = Vec::with_capacity(n);
    for t in tris {
        let mut b = Aabb::EMPTY;
        for p in t {
            b.grow(*p);
        }
        boxes.push(b);
        centroids.push((t[0] + t[1] + t[2]) / 3.0);
    }
    build_from(boxes, centroids)
}

/// The same BVH over boxes (min xyz, max xyz each; procedural geometry, software traversal), split at their centres.
/// Over no box it is one node the kernel cannot walk (`EMPTY`'s comment; a leaf would name a box that is not there):
/// software traversal leaves objects without boxes out of what the kernel is given (lib.rs `prepare`).
pub fn build_boxes(b: &[f32]) -> Bvh {
    let boxes: Vec<Aabb> = b.chunks_exact(6).map(|c| Aabb { min: V::new(c[0], c[1], c[2]), max: V::new(c[3], c[4], c[5]) }).collect();
    let centroids = boxes.iter().map(|a| (a.min + a.max) * 0.5).collect();
    build_from(boxes, centroids)
}

fn build_from(boxes: Vec<Aabb>, centroids: Vec<V>) -> Bvh {
    let n = boxes.len();
    let mut order: Vec<u32> = (0..n as u32).collect();
    let mut nodes = vec![GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 }];
    if n == 0 {
        return Bvh { nodes, order };
    }
    // (node index, start, end)
    let mut stack = vec![(0usize, 0usize, n)];
    while let Some((ni, start, end)) = stack.pop() {
        let mut bounds = Aabb::EMPTY;
        let mut cb = Aabb::EMPTY;
        for &t in &order[start..end] {
            bounds.merge(&boxes[t as usize]);
            cb.grow(centroids[t as usize]);
        }
        let count = end - start;
        let make_leaf = |nodes: &mut Vec<GpuNode>| {
            nodes[ni] = GpuNode { bmin: bounds.min.into(), left_or_first: start as u32, bmax: bounds.max.into(), count: count as u32 };
        };
        if count <= MAX_LEAF {
            make_leaf(&mut nodes);
            continue;
        }
        // Binned SAH along each axis.
        let ext = cb.max - cb.min;
        let mut best = (f32::INFINITY, 0usize, 0usize);
        for axis in 0..3 {
            if ext[axis] <= 1e-9 {
                continue;
            }
            let mut bin_box = [Aabb::EMPTY; BINS];
            let mut bin_cnt = [0usize; BINS];
            let scale = BINS as f32 / ext[axis];
            for &t in &order[start..end] {
                let b = (((centroids[t as usize][axis] - cb.min[axis]) * scale) as usize).min(BINS - 1);
                bin_cnt[b] += 1;
                bin_box[b].merge(&boxes[t as usize]);
            }
            let mut left_area = [0.0f32; BINS];
            let mut left_cnt = [0usize; BINS];
            let mut acc = Aabb::EMPTY;
            let mut c = 0;
            for i in 0..BINS {
                acc.merge(&bin_box[i]);
                c += bin_cnt[i];
                left_area[i] = acc.area();
                left_cnt[i] = c;
            }
            let mut acc = Aabb::EMPTY;
            let mut c = 0;
            for i in (1..BINS).rev() {
                acc.merge(&bin_box[i]);
                c += bin_cnt[i];
                let cost = left_area[i - 1] * left_cnt[i - 1] as f32 + acc.area() * c as f32;
                if left_cnt[i - 1] > 0 && c > 0 && cost < best.0 {
                    best = (cost, axis, i);
                }
            }
        }
        let leaf_cost = bounds.area() * count as f32;
        let splittable = best.0.is_finite();
        if (splittable && best.0 >= leaf_cost && count <= 16) || (!splittable && count <= 16) {
            make_leaf(&mut nodes);
            continue;
        }
        let mid = if best.0.is_finite() {
            let (_, axis, split) = best;
            let scale = BINS as f32 / ext[axis];
            let slice = &mut order[start..end];
            let mut i = 0;
            for j in 0..slice.len() {
                let b = (((centroids[slice[j] as usize][axis] - cb.min[axis]) * scale) as usize).min(BINS - 1);
                if b < split {
                    slice.swap(i, j);
                    i += 1;
                }
            }
            start + i
        } else {
            // Degenerate centroids: split in the middle.
            start + count / 2
        };
        let left = nodes.len();
        nodes.push(GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 });
        nodes.push(GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 });
        nodes[ni] = GpuNode { bmin: bounds.min.into(), left_or_first: left as u32, bmax: bounds.max.into(), count: 0 };
        stack.push((left + 1, mid, end));
        stack.push((left, start, mid));
    }
    Bvh { nodes, order }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INF: f32 = 3.0e38;

    /// pathtrace.wgsl `ray_aabb`, as written.
    fn ray_aabb(o: V, inv_d: V, bmin: [f32; 3], bmax: [f32; 3], tmax: f32) -> f32 {
        let t0 = (V::from(bmin) - o) * inv_d;
        let t1 = (V::from(bmax) - o) * inv_d;
        let (tmin_v, tmax_v) = (t0.min(t1), t0.max(t1));
        let tn = tmin_v.x.max(tmin_v.y).max(tmin_v.z.max(0.0));
        let tf = tmax_v.x.min(tmax_v.y).min(tmax_v.z.min(tmax));
        if tn <= tf {
            tn
        } else {
            INF
        }
    }

    /// The kernel's software traversal (pathtrace.wgsl `trace`, its loop statement for statement) on the CPU, with
    /// what the kernel does not have: every read checked, and the steps counted. `tris`: the triangles the vertex
    /// buffer holds. Returns the triangles the ray is tested against; an error where the kernel would read a node or
    /// a triangle that is not there, or would not stop. No triangle is ever hit here (the ray keeps its full length),
    /// so the walk visits at least every node the kernel visits for the same ray.
    fn walk(nodes: &[GpuNode], tris: usize, o: V, d: V) -> Result<Vec<u32>, String> {
        let node_at = |i: u32| nodes.get(i as usize).copied().ok_or(format!("reads node {i} of {}", nodes.len()));
        let inv_d = V::ONE / V::select(d.abs().cmplt(V::splat(1e-20)), V::splat(1e-20), d);
        let (mut stack, mut sp, mut node) = ([0u32; 48], 0usize, 0u32);
        let mut tested = vec![];
        for _ in 0..4 * nodes.len() + 8 {
            let n = node_at(node)?;
            if n.count > 0 {
                for k in 0..n.count {
                    let ti = n.left_or_first + k;
                    if ti as usize >= tris {
                        return Err(format!("reads triangle {ti} of {tris}"));
                    }
                    tested.push(ti);
                }
                if sp == 0 {
                    return Ok(tested);
                }
                sp -= 1;
                node = stack[sp];
                continue;
            }
            let l = n.left_or_first;
            let r = l + 1;
            let (nl, nr) = (node_at(l)?, node_at(r)?);
            let mut tl = ray_aabb(o, inv_d, nl.bmin, nl.bmax, INF);
            let mut tr = ray_aabb(o, inv_d, nr.bmin, nr.bmax, INF);
            if tl == INF && tr == INF {
                if sp == 0 {
                    return Ok(tested);
                }
                sp -= 1;
                node = stack[sp];
                continue;
            }
            let (mut near, mut far) = (l, r);
            if tr < tl {
                (near, far) = (r, l);
                std::mem::swap(&mut tl, &mut tr);
            }
            node = near;
            if tr != INF && sp < 48 {
                stack[sp] = far;
                sp += 1;
            }
        }
        Err("does not stop".into())
    }

    /// `walk` is the kernel's loop: the lines of pathtrace.wgsl it transcribes are still there, in this order. (A
    /// change to the kernel's traversal fails here, and `walk` is to follow it.)
    #[test]
    fn the_walk_is_the_kernels_loop() {
        let src = include_str!("../shaders/pathtrace.wgsl");
        let (_, rest) = src.split_once("fn trace(o: vec3<f32>, d: vec3<f32>, tmax_in: f32, mode: u32) -> Hit {").expect("the kernel's trace");
        let (body, _) = rest.split_once("if (has(F_PROC)) { return trace_procs(o, d, hit, mode); }").expect("the end of the triangles' loop");
        let lines = [
            "let inv_d = 1.0 / select(d, vec3<f32>(1e-20), abs(d) < vec3<f32>(1e-20));",
            "var stack: array<u32, 48>;",
            "var node = 0u;",
            "let n = nodes[node];",
            "if (n.count > 0u) {",
            "for (var k = 0u; k < n.count; k++) {",
            "let ti = n.left_or_first + k;",
            "if (sp == 0u) { break; }",
            "node = stack[sp];",
            "let l = n.left_or_first;",
            "let r = l + 1u;",
            "let nl = nodes[l];",
            "let nr = nodes[r];",
            "var tl = ray_aabb(o, inv_d, nl.bmin, nl.bmax, hit.t);",
            "var tr = ray_aabb(o, inv_d, nr.bmin, nr.bmax, hit.t);",
            "if (tl == INF && tr == INF) {",
            "if (tr < tl) {",
            "node = near;",
            "if (tr != INF && sp < 48u) {",
            "stack[sp] = far;",
        ];
        let mut at = 0;
        for l in lines {
            at += body[at..].find(l).unwrap_or_else(|| panic!("the kernel's trace no longer has `{l}` where the walk expects it"));
        }
        assert!(src.contains("let tn = max(max(tmin_v.x, tmin_v.y), max(tmin_v.z, 0.0));\n    let tf = min(min(tmax_v.x, tmax_v.y), min(tmax_v.z, tmax));\n    return select(INF, tn, tn <= tf);"), "ray_aabb");
        assert!(src.contains("const INF: f32 = 3.0e38;"));
    }

    /// A deterministic stream of numbers in [0, 1).
    fn stream(seed: u32) -> impl FnMut() -> f32 {
        let mut s = seed;
        move || {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            (s >> 8) as f32 / (1 << 24) as f32
        }
    }

    /// The kernel walks every tree `build` makes without reading past its nodes or its triangles, and stops: trees
    /// over no triangle (a scene with nothing in it, or with procedural geometry only), one, a leaf's worth, many.
    /// The empty scene's vertex buffer holds zeros (gpu.rs `upload`: 512 bytes, so triangles 0..2 are there, without
    /// area).
    /// The tree over no box is not one the kernel can walk (it reads past it): why `prepare` leaves such objects out.
    #[test]
    fn a_tree_over_no_box_is_left_out() {
        let b = build_boxes(&[]);
        assert_eq!(walk(&b.nodes, 0, V::new(3.0, 4.0, 5.0), V::Y), Err("reads node 1 of 1".into()));
    }

    #[test]
    fn the_kernel_walks_every_tree_within_its_buffers() {
        let mut rnd = stream(7);
        let mut p = |s: f32| V::new(rnd() - 0.5, rnd() - 0.5, rnd() - 0.5) * s;
        for n in [0usize, 1, 2, 4, 5, 17, 1000] {
            let tris: Vec<[V; 3]> = (0..n).map(|_| { let c = p(20.0); [c + p(1.0), c + p(1.0), c + p(1.0)] }).collect();
            let b = build(&tris);
            // (An empty scene's buffer: 512 zero bytes are 10 vec4s, three triangles' worth and a third.)
            let held = if n == 0 { 3 } else { n };
            let mut rays: Vec<(V, V)> = (0..2000).map(|_| (p(60.0), p(2.0).normalize_or(V::X))).collect();
            // Rays through the origin and along the axes (an empty tree's boxes are at the origin).
            for a in [V::X, V::Y, V::Z, V::ONE.normalize(), V::new(1.0, -1.0, 0.0).normalize()] {
                rays.extend([(V::ZERO, a), (-a * 5.0, a), (a * 5.0, -a), (V::new(0.0, 4.5, 0.001), -a), (V::new(1.0, 1.0, 5.0), (-V::new(1.0, 1.0, 5.0)).normalize())]);
            }
            let mut seen = vec![false; held];
            for (o, d) in rays {
                let tested = walk(&b.nodes, held, o, d).unwrap_or_else(|e| panic!("{n} triangles, ray {o} {d}: the kernel {e}"));
                tested.iter().for_each(|&t| seen[t as usize] = true);
            }
            if n > 0 {
                assert!(seen.iter().filter(|&&s| s).count() * 2 > n, "{n} triangles: the rays reach the leaves");
            }
        }
    }
}
