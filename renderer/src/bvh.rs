//! Binned-SAH BVH over triangles (and over boxes). Leaves reference contiguous ranges of the reordered list. What a
//! tree this module makes holds to, and why the kernel's walk ends on it: `build_tris`.

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
    /// order[i] = original item (triangle or box) stored at position i. Items whose box is not finite are in no leaf:
    /// they come after every leaf's range, so they keep a position (the kernel's arrays hold them; a triangle's index
    /// is its position) and no walk reaches them.
    pub order: Vec<u32>,
    /// Levels below the root (0: the root is a leaf). At most the `max_depth` it was built for.
    pub depth: usize,
}

const BINS: usize = 16;
const MAX_LEAF: usize = 4;
/// The most items a leaf the surface-area heuristic prefers to a split may hold.
const SAH_LEAF: usize = 16;
/// The kernel's stack for this module's trees (pathtrace.wgsl `trace`, `trace_procs`: `array<u32, 48>`). A walk holds
/// at most one entry a level of its tree, so a tree no deeper than this is walked whole; a push past it would be
/// dropped (geometry missed, never a hang).
pub const STACK: usize = 48;

/// The tree over no triangle, as the kernel can walk it. A node with `count: 0` is an interior node to the kernel
/// (pathtrace.wgsl `trace`): the one-node tree this module used to make for nothing had itself for a left child and,
/// for a right child, whatever lay after the node buffer's 32 bytes. Where that was zeros the walk ended; where it was
/// what an earlier scene left there, it might not (a dispatch that does not end: docs/RUNTIME.md "The engine in a
/// browser"). The encoding has no empty leaf, so this is a leaf of one triangle, triangle 0: with no triangles the
/// vertex buffer is zeros (gpu.rs `upload`), a triangle without area that no ray meets (`intersect_tri`: its
/// determinant is 0). One node read, no child, no box.
pub const EMPTY: GpuNode = GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 1 };

/// The tree over triangles the kernel's software traversal walks (the still scene's), no deeper than `STACK`.
pub fn build(tris: &[[V; 3]]) -> Bvh {
    build_tris(tris, STACK)
}

/// The tree over triangles, no deeper than `max_depth` (docs/RUNTIME.md "What the kernel trusts": what every tree
/// this module makes is):
///  - children after their parent: an interior node's children are the nodes at `left` and `left + 1`, both after it
///    and inside the tree, so a walk that goes only down and pops what it pushed reaches each node at most once and
///    ends, whatever the boxes say;
///  - every node reached exactly once from the root; leaves of 1 to `SAH_LEAF` items, inside the items;
///  - every box finite: an item whose box is not (a vertex NaN or infinite) is in no leaf (`Bvh::order`);
///  - no deeper than `max_depth`: binned SAH is not balanced, so where the levels left could not finish a split it
///    makes (both sides no deeper than `max_depth` by halving), the node is split at the median instead.
///
/// In tests and debug builds `check_tree` holds every tree it returns to this.
pub fn build_tris(tris: &[[V; 3]], max_depth: usize) -> Bvh {
    if tris.is_empty() {
        return Bvh { nodes: vec![EMPTY], order: vec![], depth: 0 };
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
    let b = build_from(boxes, centroids, max_depth);
    // (Every triangle's box not finite: the tree over nothing.)
    if b.nodes.len() == 1 && b.nodes[0].count == 0 {
        return Bvh { nodes: vec![EMPTY], order: b.order, depth: 0 };
    }
    b
}

/// The same BVH over boxes (min xyz, max xyz each; procedural geometry, software traversal), split at their centres,
/// no deeper than `STACK`. Over no box it is one node the kernel cannot walk (`EMPTY`'s comment; a leaf would name a
/// box that is not there): software traversal leaves objects without boxes out of what the kernel is given (lib.rs
/// `prepare`), and boxes that are not finite are refused where they are given (world.rs `set_proc_geometry`).
pub fn build_boxes(b: &[f32]) -> Bvh {
    let boxes: Vec<Aabb> = b.chunks_exact(6).map(|c| Aabb { min: V::new(c[0], c[1], c[2]), max: V::new(c[3], c[4], c[5]) }).collect();
    let centroids = boxes.iter().map(|a| (a.min + a.max) * 0.5).collect();
    build_from(boxes, centroids, STACK)
}

/// Levels of halving `count` items take to reach leaves of `MAX_LEAF`.
fn levels(count: usize) -> usize {
    count.div_ceil(MAX_LEAF).next_power_of_two().trailing_zeros() as usize
}

impl Aabb {
    fn finite(&self) -> bool {
        self.min.is_finite() && self.max.is_finite() && self.min.cmple(self.max).all()
    }
}

fn build_from(boxes: Vec<Aabb>, centroids: Vec<V>, max_depth: usize) -> Bvh {
    // Items with a box that is not finite go last, in no leaf.
    let mut order: Vec<u32> = (0..boxes.len() as u32).filter(|&i| boxes[i as usize].finite()).collect();
    let n = order.len();
    order.extend((0..boxes.len() as u32).filter(|&i| !boxes[i as usize].finite()));
    let mut nodes = vec![GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 }];
    if n == 0 {
        return Bvh { nodes, order, depth: 0 };
    }
    assert!(levels(n) <= max_depth, "{n} items do not fit a tree {max_depth} deep");
    let mut deepest = 0;
    // (node index, start, end, depth)
    let mut stack = vec![(0usize, 0usize, n, 0usize)];
    while let Some((ni, start, end, depth)) = stack.pop() {
        deepest = deepest.max(depth);
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
                // (A split the levels left could not finish is not taken: the median split below is.)
                let fits = depth + 1 + levels(left_cnt[i - 1].max(c)) <= max_depth;
                if left_cnt[i - 1] > 0 && c > 0 && cost < best.0 && fits {
                    best = (cost, axis, i);
                }
            }
        }
        let leaf_cost = bounds.area() * count as f32;
        let splittable = best.0.is_finite();
        if count <= SAH_LEAF && (!splittable || best.0 >= leaf_cost) {
            make_leaf(&mut nodes);
            continue;
        }
        let mid = if splittable {
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
        } else if (0..3).all(|k| ext[k] <= 1e-9) {
            // Degenerate centroids: split in the middle.
            start + count / 2
        } else {
            // No split the levels left could finish: the median along the centroids' widest axis, which halves.
            let axis = (0..3).max_by(|&x, &y| ext[x].total_cmp(&ext[y])).unwrap_or(0);
            let half = count / 2;
            order[start..end].select_nth_unstable_by(half, |p, q| centroids[*p as usize][axis].total_cmp(&centroids[*q as usize][axis]));
            start + half
        };
        let left = nodes.len();
        nodes.push(GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 });
        nodes.push(GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 });
        nodes[ni] = GpuNode { bmin: bounds.min.into(), left_or_first: left as u32, bmax: bounds.max.into(), count: 0 };
        stack.push((left + 1, mid, end, depth + 1));
        stack.push((left, start, mid, depth + 1));
    }
    #[cfg(any(test, debug_assertions))]
    {
        let checked = check_tree(&nodes, max_depth, |first, count| if count as usize <= SAH_LEAF && first as usize + count as usize <= n { Ok(()) } else { Err(format!("items {first}..{} of {n}", first as u64 + count as u64)) });
        assert_eq!(checked, Ok(deepest), "bvh: a tree the kernel cannot walk as this module says");
    }
    Bvh { nodes, order, depth: deepest }
}

/// Checks that `nodes` is a tree the kernel's walk ends on, from node 0 (`build_tris`' list): every interior node's
/// children are the two nodes at `left` and `left + 1`, `left` greater than the node's own index and inside the
/// tree; every node is reached exactly once; every box is finite; every leaf passes `leaf(first, count)`; nothing is
/// deeper than `max_depth`. Returns the depth (levels below the root). (The units' trees are held to it at every
/// build, units.rs; this module's in tests and debug builds.)
pub fn check_tree(nodes: &[GpuNode], max_depth: usize, leaf: impl Fn(u32, u32) -> Result<(), String>) -> Result<usize, String> {
    if nodes.is_empty() {
        return Err("a tree without nodes".into());
    }
    let finite = |n: &GpuNode| n.bmin.iter().chain(&n.bmax).all(|v| v.is_finite()) && (0..3).all(|k| n.bmin[k] <= n.bmax[k]);
    let mut seen = vec![false; nodes.len()];
    let mut stack = vec![(0usize, 0usize)];
    let (mut deepest, mut reached) = (0, 0);
    while let Some((i, depth)) = stack.pop() {
        if std::mem::replace(&mut seen[i], true) {
            return Err(format!("node {i} is reached twice"));
        }
        reached += 1;
        deepest = deepest.max(depth);
        let n = &nodes[i];
        if n.count > 0 {
            leaf(n.left_or_first, n.count).map_err(|e| format!("leaf {i}: {e}"))?;
            continue;
        }
        if !finite(n) {
            return Err(format!("node {i}'s box is not finite"));
        }
        let l = n.left_or_first as usize;
        if l <= i || l + 1 >= nodes.len() {
            return Err(format!("node {i}'s children {l}, {} are not after it and inside the tree's {} nodes", l + 1, nodes.len()));
        }
        if depth + 1 > max_depth {
            return Err(format!("deeper than {max_depth}"));
        }
        stack.push((l, depth + 1));
        stack.push((l + 1, depth + 1));
    }
    if reached != nodes.len() {
        return Err(format!("{} of {} nodes are not reached", nodes.len() - reached, nodes.len()));
    }
    Ok(deepest)
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
    /// Garbage in (as Embree's builders are tested): every tree `build_tris` makes of it is one `check_tree` passes
    /// (the builder checks itself in tests), that the kernel's walk ends on within its buffers, with the items whose
    /// box is not finite after every leaf's range. NaN and infinite vertices among good ones, all of them bad, values
    /// near f32's limit, a thousand copies of one triangle (centroids that coincide), a needle (centroids on a line).
    #[test]
    fn garbage_in_makes_a_tree_the_kernel_walks() {
        let mut rnd = stream(11);
        let mut p = |s: f32| V::new(rnd() - 0.5, rnd() - 0.5, rnd() - 0.5) * s;
        let good: Vec<[V; 3]> = (0..600).map(|_| { let c = p(20.0); [c + p(1.0), c + p(1.0), c + p(1.0)] }).collect();
        let (nan, inf) = (V::new(f32::NAN, 0.0, 0.0), V::new(0.0, f32::INFINITY, 0.0));
        let mut mixed = good.clone();
        for k in (0..mixed.len()).step_by(7) {
            mixed[k][k % 3] = if k % 2 == 0 { nan } else { inf };
        }
        let cases: Vec<(&str, Vec<[V; 3]>)> = vec![
            ("mixed", mixed),
            ("all bad", vec![[nan, V::ZERO, V::ONE], [V::ZERO, inf, V::ONE], [V::splat(f32::NEG_INFINITY), V::ZERO, V::ONE]]),
            ("near the limit", (0..200).map(|k| { let c = V::splat(3.0e38 * if k % 2 == 0 { 1.0 } else { -1.0 }); [c, c * 0.5, V::ZERO] }).collect()),
            ("one triangle a thousand times", vec![good[0]; 1000]),
            ("a needle", (0..500).map(|k| { let c = V::new(k as f32 * 1e-3, 0.0, 0.0); [c, c + V::Y * 1e-4, c + V::Z * 1e-4] }).collect()),
        ];
        for (what, tris) in cases {
            let b = build_tris(&tris, STACK);
            let bad: Vec<u32> = (0..tris.len() as u32).filter(|&i| !tris[i as usize].iter().all(|v| v.is_finite())).collect();
            let firsts = b.nodes.iter().filter(|n| n.count > 0).map(|n| n.left_or_first + n.count).max().unwrap_or(0) as usize;
            if bad.len() < tris.len() {
                assert!(firsts <= tris.len() - bad.len(), "{what}: a leaf reaches the items left out");
            }
            let mut tail = b.order[tris.len() - bad.len()..].to_vec();
            tail.sort();
            assert_eq!(tail, bad, "{what}: the items left out come last");
            for k in 0..200 {
                let o = V::new(k as f32 - 100.0, 7.0, 3.0);
                walk(&b.nodes, tris.len().max(3), o, (V::new(0.3, -1.0, 0.2) - o * 0.001).normalize()).unwrap_or_else(|e| panic!("{what}: {e}"));
            }
        }
    }

    /// Binary SAH is not balanced: boxes spaced in powers of two make it split one off at a time. Held to a depth, the
    /// builder splits at the median where the levels left could not finish the split SAH wants: the tree is no deeper
    /// than asked, and unchanged where it fits (the same scene with room enough is the same tree).
    #[test]
    fn a_tree_is_no_deeper_than_its_stack() {
        let tris: Vec<[V; 3]> = (0..120).map(|k| { let c = V::new(2f32.powi(k), 0.0, 0.0); [c, c + V::Y, c + V::Z] }).collect();
        let free = build_tris(&tris, 1000);
        assert!(free.depth > 20, "the spacing makes SAH deep ({})", free.depth);
        for cap in [8, 12, 20] {
            let held = build_tris(&tris, cap);
            assert!(held.depth <= cap);
        }
        let roomy = build_tris(&tris, free.depth);
        assert_eq!((roomy.depth, roomy.order.clone()), (free.depth, free.order.clone()), "where the tree fits, the cap changes nothing");
        assert_eq!(bytemuck::cast_slice::<GpuNode, u8>(&roomy.nodes), bytemuck::cast_slice::<GpuNode, u8>(&free.nodes));
    }

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
