//! Units: what software traversal can add, move and remove without rebuilding the scene (docs/RUNTIME.md "Software
//! traversal with units"; ROADMAP § 6 step 7b). A unit is a model instance, as under hardware traversal: its mesh's
//! triangles once, in model space; a tree over them (a mesh tree, the software BLAS); its kernel record with the
//! tree's root in it; and a leaf of the top tree (the software TLAS), which is over the units' world boxes. The
//! kernel's walk is shaders/trace_units.wgsl. This module builds the trees, says where they go in the node buffer,
//! and checks every tree before it is written: the kernel follows the indices it reads, and ends only if they are a
//! tree's.
//!
//! The node buffer: the still scene's tree (as before), then the mesh trees' range (trees side by side, room after
//! them), then the top tree's range. A mesh tree is relocatable: child indices count from its root, a leaf's
//! triangles from its mesh's first triangle. The top tree's child indices count from its root, and each of its leaves
//! is one unit: `left_or_first` the unit's record, `count` 1. It is rebuilt whole whenever a unit changes.
//!
//! Depths: both levels share the kernel's stack of `STACK` entries (a push past it is dropped: geometry missed, never
//! a hang). A walk's stack is never deeper than its tree, so the top tree is held to `TOP_DEPTH` and a mesh tree to
//! `MESH_DEPTH` (a tree deeper than that is built again by median splits, which halve), and the two fit.

use crate::bvh::{self, GpuNode};
use crate::scene::GpuInstance;
use glam::{Mat4, Vec3A};

/// The kernel's traversal stack (pathtrace.wgsl `trace`, trace_units.wgsl): entries shared by both levels.
pub const STACK: usize = 48;
/// The deepest top tree (levels below the root): 2^18 units by median splits.
pub const TOP_DEPTH: usize = 18;
/// The deepest mesh tree: 2^30 leaves by median splits; SAH trees deeper than this are rebuilt that way.
pub const MESH_DEPTH: usize = STACK - TOP_DEPTH;
/// The most units a top tree holds.
pub const MAX_UNITS: usize = 1 << TOP_DEPTH;
/// A leaf's most triangles (bvh.rs builds leaves of up to 16).
pub const MAX_LEAF: u32 = 16;

/// What unused room in the node buffer holds: a leaf of one item, which ends a walk that lands on it (zeros would be
/// an interior node whose children are node 0 and node 1).
pub const FILL: GpuNode = bvh::EMPTY;

/// A mesh's tree: its nodes (relocatable), the order it puts the mesh's triangles in (`order[i]`: the triangle now at
/// position i), its depth.
pub struct MeshTree {
    pub nodes: Vec<GpuNode>,
    pub order: Vec<u32>,
    pub depth: usize,
}

fn finite(n: &GpuNode) -> bool {
    n.bmin.iter().chain(&n.bmax).all(|v| v.is_finite()) && (0..3).all(|k| n.bmin[k] <= n.bmax[k])
}

/// Checks that `nodes` is a tree the kernel's walk ends on, from node 0: every interior node's children are the two
/// nodes at `left` and `left + 1`, `left` greater than the node's own index (so nothing is entered twice) and inside
/// the tree; every node is reached exactly once; every box is finite; every leaf passes `leaf(first, count)`; nothing
/// is deeper than `max_depth`. Returns the depth (levels below the root).
fn check_tree(nodes: &[GpuNode], max_depth: usize, leaf: impl Fn(u32, u32) -> Result<(), String>) -> Result<usize, String> {
    if nodes.is_empty() {
        return Err("a tree without nodes".into());
    }
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

/// A mesh tree over `tris` triangles, as the kernel may walk it: `check_tree`, leaves of 1 to `MAX_LEAF` triangles
/// inside the mesh, no deeper than `MESH_DEPTH`. Its depth.
pub fn check_mesh(nodes: &[GpuNode], tris: usize) -> Result<usize, String> {
    check_tree(nodes, MESH_DEPTH, |first, count| {
        if count > MAX_LEAF || first as usize + count as usize > tris {
            return Err(format!("triangles {first}..{} of {tris}", first as u64 + count as u64));
        }
        Ok(())
    })
}

/// A top tree whose leaves are among the records `is_unit` holds true for, each of them at most once, no deeper than
/// `TOP_DEPTH`.
pub fn check_top(nodes: &[GpuNode], is_unit: impl Fn(u32) -> bool) -> Result<usize, String> {
    let seen = std::cell::RefCell::new(std::collections::HashSet::new());
    check_tree(nodes, TOP_DEPTH, |rec, count| {
        if count != 1 || !is_unit(rec) {
            return Err(format!("record {rec} (count {count}) is not one unit"));
        }
        if !seen.borrow_mut().insert(rec) {
            return Err(format!("record {rec} twice"));
        }
        Ok(())
    })
}

/// A tree by median splits over boxes (min, max): leaves of up to `max_leaf` boxes, each split halving along the
/// widest axis of the boxes' centres, so its depth is ceil(log2(boxes / max_leaf)). Returns the nodes (a leaf's
/// `left_or_first`: its first position in the order) and the order.
fn build_median(boxes: &[(Vec3A, Vec3A)], max_leaf: usize) -> (Vec<GpuNode>, Vec<u32>) {
    let mut order: Vec<u32> = (0..boxes.len() as u32).collect();
    let mut nodes = vec![FILL];
    if boxes.is_empty() {
        return (nodes, order);
    }
    let centre = |k: u32| (boxes[k as usize].0 + boxes[k as usize].1) * 0.5;
    let mut stack = vec![(0usize, 0usize, boxes.len())];
    while let Some((ni, start, end)) = stack.pop() {
        let (mut lo, mut hi) = (Vec3A::splat(f32::INFINITY), Vec3A::splat(f32::NEG_INFINITY));
        let (mut clo, mut chi) = (lo, hi);
        for &k in &order[start..end] {
            lo = lo.min(boxes[k as usize].0);
            hi = hi.max(boxes[k as usize].1);
            clo = clo.min(centre(k));
            chi = chi.max(centre(k));
        }
        let count = end - start;
        if count <= max_leaf {
            nodes[ni] = GpuNode { bmin: lo.into(), left_or_first: start as u32, bmax: hi.into(), count: count as u32 };
            continue;
        }
        let ext = chi - clo;
        let axis = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
        let mid = count / 2;
        order[start..end].select_nth_unstable_by(mid, |&a, &b| centre(a)[axis].total_cmp(&centre(b)[axis]).then(a.cmp(&b)));
        let left = nodes.len();
        nodes.push(FILL);
        nodes.push(FILL);
        nodes[ni] = GpuNode { bmin: lo.into(), left_or_first: left as u32, bmax: hi.into(), count: 0 };
        stack.push((left + 1, start + mid, end));
        stack.push((left, start, start + mid));
    }
    (nodes, order)
}

fn tri_boxes(tris: &[[Vec3A; 3]]) -> Vec<(Vec3A, Vec3A)> {
    tris.iter().map(|t| (t[0].min(t[1]).min(t[2]), t[0].max(t[1]).max(t[2]))).collect()
}

/// The tree of a mesh's model-space triangles: binned SAH as the still scene's (bvh.rs), or by median splits when
/// that came out deeper than `MESH_DEPTH`. Checked. A mesh without triangles has no tree, and is no unit.
pub fn mesh_tree(tris: &[[Vec3A; 3]]) -> Result<MeshTree, String> {
    if tris.is_empty() {
        return Err("a mesh without triangles has no tree".into());
    }
    if tris.iter().flatten().any(|v| !v.is_finite()) {
        return Err("a mesh's vertex is not finite".into());
    }
    let b = bvh::build(tris);
    let (nodes, order) = match check_mesh(&b.nodes, tris.len()) {
        Ok(_) => (b.nodes, b.order),
        Err(_) => build_median(&tri_boxes(tris), 4),
    };
    let depth = check_mesh(&nodes, tris.len())?;
    Ok(MeshTree { nodes, order, depth })
}

/// A unit for the top tree: its record, and its mesh's model-space bounds under its world transform.
pub struct UnitBox {
    pub record: u32,
    pub lo: Vec3A,
    pub hi: Vec3A,
}

impl UnitBox {
    /// The world box of model bounds `lo..hi` under `world`: the box of its eight corners, a little larger than
    /// computed (the kernel tests the triangles in the unit's own space, to its own rounding: a hit there must not
    /// be lost to this box's).
    pub fn new(record: u32, world: Mat4, lo: Vec3A, hi: Vec3A) -> UnitBox {
        let (mut a, mut b) = (Vec3A::splat(f32::INFINITY), Vec3A::splat(f32::NEG_INFINITY));
        for c in 0..8 {
            let p = world.transform_point3a(Vec3A::new(if c & 1 == 0 { lo.x } else { hi.x }, if c & 2 == 0 { lo.y } else { hi.y }, if c & 4 == 0 { lo.z } else { hi.z }));
            a = a.min(p);
            b = b.max(p);
        }
        let pad = Vec3A::splat((b - a).max_element() * 1e-4 + a.abs().max(b.abs()).max_element() * 1e-5 + 1e-6);
        UnitBox { record, lo: a - pad, hi: b + pad }
    }
}

/// The top tree over `units` (at least one): median splits down to one unit a leaf, each leaf's `left_or_first` its
/// unit's record. Checked against the units it was given.
pub fn top_tree(units: &[UnitBox]) -> Result<Vec<GpuNode>, String> {
    if units.is_empty() {
        return Err("a top tree needs a unit".into());
    }
    if units.len() > MAX_UNITS {
        return Err(format!("{} units: more than a top tree holds ({MAX_UNITS})", units.len()));
    }
    if units.iter().any(|u| !(u.lo.is_finite() && u.hi.is_finite())) {
        return Err("a unit's box is not finite".into());
    }
    let boxes: Vec<(Vec3A, Vec3A)> = units.iter().map(|u| (u.lo, u.hi)).collect();
    let (mut nodes, order) = build_median(&boxes, 1);
    for n in nodes.iter_mut().filter(|n| n.count > 0) {
        n.left_or_first = units[order[n.left_or_first as usize] as usize].record;
    }
    let records: std::collections::HashSet<u32> = units.iter().map(|u| u.record).collect();
    check_top(&nodes, |r| records.contains(&r))?;
    Ok(nodes)
}

/// A unit's record: the instance's (`scene::Instance::gpu`), with its mesh tree's root and whether it casts shadows in
/// the two words the normal rows leave unused (trace_units.wgsl reads them as bits).
pub fn unit_record(mut r: GpuInstance, root: u32, cast_shadow: bool) -> GpuInstance {
    r.n[0][3] = f32::from_bits(root);
    r.n[1][3] = f32::from_bits(cast_shadow as u32);
    r
}

/// Where the units' trees are in the node buffer, and which of the mesh trees' range is free. Nodes are counted, not
/// bytes.
#[derive(Clone, Debug)]
pub struct Layout {
    /// The mesh trees' range: its first node and its length.
    pub mesh_first: u32,
    pub mesh_cap: u32,
    /// The top tree's range (its root is its first node) and the most units it has room for.
    pub top_first: u32,
    pub top_cap: u32,
    /// Free runs of the mesh range (first node, length), in order, never adjacent.
    free: Vec<(u32, u32)>,
}

impl Layout {
    /// After a still tree of `still` nodes: room for `mesh_nodes` nodes of mesh trees and a top tree of `units` units.
    pub fn new(still: usize, mesh_nodes: usize, units: usize) -> Layout {
        let (mesh_first, mesh_cap) = (still as u32, mesh_nodes as u32);
        let top_cap = (2 * units.clamp(1, MAX_UNITS) - 1) as u32;
        Layout { mesh_first, mesh_cap, top_first: mesh_first + mesh_cap, top_cap, free: if mesh_cap > 0 { vec![(mesh_first, mesh_cap)] } else { vec![] } }
    }

    /// Every node of the buffer after the still tree's: all room (`FILL`).
    pub fn nodes(&self) -> usize {
        (self.mesh_cap + self.top_cap) as usize
    }

    /// The most units the top tree's range holds.
    pub fn max_units(&self) -> usize {
        (self.top_cap as usize + 1) / 2
    }

    /// A place for a mesh tree of `len` nodes: its root (first fit), or None when no free run is long enough.
    pub fn place(&mut self, len: usize) -> Option<u32> {
        let len = len as u32;
        let k = self.free.iter().position(|f| f.1 >= len)?;
        let at = self.free[k].0;
        if self.free[k].1 == len {
            self.free.remove(k);
        } else {
            self.free[k] = (at + len, self.free[k].1 - len);
        }
        Some(at)
    }

    /// Gives a mesh tree's nodes back (its root and length, as `place` gave them).
    pub fn release(&mut self, root: u32, len: usize) {
        let len = len as u32;
        debug_assert!(root >= self.mesh_first && root + len <= self.mesh_first + self.mesh_cap);
        let k = self.free.partition_point(|f| f.0 < root);
        debug_assert!(k == 0 || self.free[k - 1].0 + self.free[k - 1].1 <= root, "released twice");
        debug_assert!(k == self.free.len() || root + len <= self.free[k].0, "released twice");
        self.free.insert(k, (root, len));
        if k + 1 < self.free.len() && self.free[k].0 + self.free[k].1 == self.free[k + 1].0 {
            self.free[k].1 += self.free[k + 1].1;
            self.free.remove(k + 1);
        }
        if k > 0 && self.free[k - 1].0 + self.free[k - 1].1 == self.free[k].0 {
            self.free[k - 1].1 += self.free[k].1;
            self.free.remove(k);
        }
    }

    /// The free nodes of the mesh range.
    pub fn free_nodes(&self) -> usize {
        self.free.iter().map(|f| f.1 as usize).sum()
    }
}

/// A unit as the top tree sees it: where its mesh's model bounds are in the world. Hidden: it keeps its place (an
/// instanced object's instance that is not drawn now) and is in no top tree.
#[derive(Clone, Copy, Debug)]
pub struct Unit {
    pub world: Mat4,
    pub lo: Vec3A,
    pub hi: Vec3A,
    pub hidden: bool,
}

/// Software traversal's units of a scene (what the TLAS and the mesh BLASes are to hardware traversal): where the
/// trees are, each instanced mesh's tree, and the unit at each kernel record.
#[derive(Clone, Debug)]
pub struct Units {
    pub layout: Layout,
    /// By instanced mesh (the scene's `meshes` index): its tree's root and node count. None: no tree (a mesh that
    /// deforms or has no triangle, or one an edit freed).
    pub trees: Vec<Option<(u32, u32)>>,
    /// By kernel record: the unit there, if the record is one's.
    pub slots: Vec<Option<Unit>>,
    /// The units in the top tree as last built (`top`): the kernel's `P.motion.z`.
    pub count: u32,
}

impl Units {
    /// Mesh `k`'s tree's root.
    pub fn root(&self, k: u32) -> Option<u32> {
        self.trees.get(k as usize).copied().flatten().map(|t| t.0)
    }

    /// The unit at `record` (None: none there now). Records past the slots there are make them.
    pub fn set(&mut self, record: usize, unit: Option<Unit>) {
        if record >= self.slots.len() {
            if unit.is_none() {
                return;
            }
            self.slots.resize(record + 1, None);
        }
        self.slots[record] = unit;
    }

    /// A place for mesh `k`'s tree (the tree it had, if any, given back first): its root, or None when the mesh
    /// range has no run that long. The caller writes the tree's nodes there.
    pub fn add_tree(&mut self, k: u32, tree: &MeshTree) -> Option<u32> {
        self.drop_tree(k);
        let root = self.layout.place(tree.nodes.len())?;
        if self.trees.len() <= k as usize {
            self.trees.resize(k as usize + 1, None);
        }
        self.trees[k as usize] = Some((root, tree.nodes.len() as u32));
        Some(root)
    }

    /// Mesh `k`'s tree given back: its root and length (the caller fills them with `FILL`), if it had one.
    pub fn drop_tree(&mut self, k: u32) -> Option<(u32, u32)> {
        let t = self.trees.get_mut(k as usize)?.take()?;
        self.layout.release(t.0, t.1 as usize);
        Some(t)
    }

    /// The top tree over the units that are shown, for the top range's start, and their count kept (`count`). No
    /// unit: no nodes (the kernel does not walk a top tree then). Err: more units than the range holds, or a unit
    /// whose box is not finite; nothing is changed then.
    pub fn top(&mut self) -> Result<Vec<GpuNode>, String> {
        let boxes: Vec<UnitBox> = self.slots.iter().enumerate().filter_map(|(r, u)| u.as_ref().filter(|u| !u.hidden).map(|u| UnitBox::new(r as u32, u.world, u.lo, u.hi))).collect();
        if boxes.len() > self.layout.max_units() {
            return Err(format!("{} units: more than the top tree's range holds ({})", boxes.len(), self.layout.max_units()));
        }
        let nodes = if boxes.is_empty() { vec![] } else { top_tree(&boxes)? };
        self.count = boxes.len() as u32;
        Ok(nodes)
    }
}

/// The kernel's traversal on the CPU, for tests (here and lib.rs): what the kernel binds, read through accessors that
/// refuse an index outside their buffer, every loop iteration counted.
#[cfg(test)]
pub(crate) mod cpu {
    use super::*;

    pub(crate) const INF: f32 = 3.0e38;
    pub(crate) const NO_INST: u32 = 0xffff_ffff;
    pub(crate) const TRACE_CLOSEST: u32 = 0;
    pub(crate) const TRACE_SHADOW: u32 = 1;
    pub(crate) const TRACE_ANY: u32 = 2;

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub(crate) struct Hit {
        pub(crate) t: f32,
        pub(crate) tri: u32,
        pub(crate) inst: u32,
    }

    /// What the kernel binds, on the CPU: every read through `node`, `vert`, `record`, which refuse an index outside
    /// their buffer; every loop iteration counted against `budget` (the loop guard's role: the walk must end).
    pub(crate) struct Kernel<'a> {
        pub(crate) nodes: &'a [GpuNode],
        pub(crate) tri_verts: &'a [[f32; 4]],
        pub(crate) instances: &'a [GpuInstance],
        /// `P.motion.y`, `P.motion.z`.
        pub(crate) top: u32,
        pub(crate) units: u32,
        pub(crate) steps: std::cell::Cell<u64>,
        pub(crate) budget: u64,
        /// The deepest the shared stack got.
        pub(crate) deepest: std::cell::Cell<usize>,
    }

    pub(crate) type Walk<T> = Result<T, String>;

    impl Kernel<'_> {
        pub(crate) fn node(&self, i: u32) -> Walk<GpuNode> {
            self.nodes.get(i as usize).copied().ok_or(format!("reads node {i} of {}", self.nodes.len()))
        }
        pub(crate) fn vert(&self, i: u32) -> Walk<Vec3A> {
            self.tri_verts.get(i as usize).map(|v| Vec3A::new(v[0], v[1], v[2])).ok_or(format!("reads vertex {i} of {}", self.tri_verts.len()))
        }
        pub(crate) fn record(&self, i: u32) -> Walk<GpuInstance> {
            self.instances.get(i as usize).copied().ok_or(format!("reads record {i} of {}", self.instances.len()))
        }
        pub(crate) fn step(&self) -> Walk<()> {
            self.steps.set(self.steps.get() + 1);
            if self.steps.get() > self.budget {
                return Err("does not stop".into());
            }
            Ok(())
        }

        /// pathtrace.wgsl `ray_aabb`.
        pub(crate) fn ray_aabb(o: Vec3A, inv_d: Vec3A, bmin: [f32; 3], bmax: [f32; 3], tmax: f32) -> f32 {
            let t0 = (Vec3A::from(bmin) - o) * inv_d;
            let t1 = (Vec3A::from(bmax) - o) * inv_d;
            let (tmin_v, tmax_v) = (t0.min(t1), t0.max(t1));
            let tn = tmin_v.x.max(tmin_v.y).max(tmin_v.z.max(0.0));
            let tf = tmax_v.x.min(tmax_v.y).min(tmax_v.z.min(tmax));
            if tn <= tf {
                tn
            } else {
                INF
            }
        }

        /// pathtrace.wgsl `intersect_tri`: (t, u, v), t = INF for a miss.
        pub(crate) fn intersect_tri(&self, o: Vec3A, d: Vec3A, i: u32, tmax: f32) -> Walk<f32> {
            let (v0, e1, e2) = (self.vert(i * 3)?, self.vert(i * 3 + 1)?, self.vert(i * 3 + 2)?);
            let pv = d.cross(e2);
            let det = e1.dot(pv);
            if det.abs() < 1e-12 {
                return Ok(INF);
            }
            let inv = 1.0 / det;
            let tv = o - v0;
            let u = tv.dot(pv) * inv;
            if !(0.0..=1.0).contains(&u) {
                return Ok(INF);
            }
            let qv = tv.cross(e1);
            let v = d.dot(qv) * inv;
            if v < 0.0 || u + v > 1.0 {
                return Ok(INF);
            }
            let t = e2.dot(qv) * inv;
            if t <= 0.0 || t >= tmax {
                return Ok(INF);
            }
            Ok(t)
        }

        pub(crate) fn inv(d: Vec3A) -> Vec3A {
            Vec3A::ONE / Vec3A::select(d.abs().cmplt(Vec3A::splat(1e-20)), Vec3A::splat(1e-20), d)
        }

        /// pathtrace.wgsl `proc_dir`, `proc_point`.
        pub(crate) fn proc_dir(r: &GpuInstance, d: Vec3A) -> Vec3A {
            let n = |k: usize| Vec3A::new(r.n[k][0], r.n[k][1], r.n[k][2]);
            n(0) * d.x + n(1) * d.y + n(2) * d.z
        }
        pub(crate) fn proc_point(r: &GpuInstance, p: Vec3A) -> Vec3A {
            Self::proc_dir(r, p - Vec3A::new(r.m[0][3], r.m[1][3], r.m[2][3]))
        }

        /// pathtrace.wgsl `trace`, the still scene's tree (no cutouts: the scenes here have none), then the units as
        /// `kernel_source` splices them in.
        pub(crate) fn trace(&self, o: Vec3A, d: Vec3A, tmax_in: f32, mode: u32) -> Walk<Hit> {
            let mut hit = Hit { t: tmax_in, tri: 0xffff_ffff, inst: NO_INST };
            let inv_d = Self::inv(d);
            let (mut stack, mut sp, mut node) = ([0u32; STACK], 0usize, 0u32);
            loop {
                self.step()?;
                let n = self.node(node)?;
                if n.count > 0 {
                    for k in 0..n.count {
                        self.step()?;
                        let ti = n.left_or_first + k;
                        let flags = self.tri_verts.get(ti as usize * 3).ok_or(format!("reads triangle {ti}"))?[3].to_bits();
                        if mode == TRACE_SHADOW && flags & crate::scene::TRI_NO_SHADOW != 0 {
                            continue;
                        }
                        let t = self.intersect_tri(o, d, ti, hit.t)?;
                        if t < hit.t {
                            hit = Hit { t, tri: ti, inst: NO_INST };
                            if mode != TRACE_CLOSEST {
                                return Ok(hit);
                            }
                        }
                    }
                    if sp == 0 {
                        break;
                    }
                    sp -= 1;
                    node = stack[sp];
                    continue;
                }
                let l = n.left_or_first;
                let r = l + 1;
                let (nl, nr) = (self.node(l)?, self.node(r)?);
                let mut tl = Self::ray_aabb(o, inv_d, nl.bmin, nl.bmax, hit.t);
                let mut tr = Self::ray_aabb(o, inv_d, nr.bmin, nr.bmax, hit.t);
                if tl == INF && tr == INF {
                    if sp == 0 {
                        break;
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
                if tr != INF && sp < STACK {
                    stack[sp] = far;
                    sp += 1;
                }
            }
            self.trace_units(o, d, hit, mode)
        }

        /// shaders/trace_units.wgsl `trace_units`, statement for statement.
        pub(crate) fn trace_units(&self, o: Vec3A, d: Vec3A, hit_in: Hit, mode: u32) -> Walk<Hit> {
            let mut hit = hit_in;
            if self.units == 0 {
                return Ok(hit);
            }
            let top = self.top;
            let inv_d = Self::inv(d);
            let (mut stack, mut sp, mut node) = ([0u32; STACK], 0usize, top);
            loop {
                self.step()?;
                let n = self.node(node)?;
                if n.count > 0 {
                    let rec = n.left_or_first;
                    let r = self.record(rec)?;
                    if mode != TRACE_SHADOW || r.n[1][3].to_bits() != 0 {
                        let lo = Self::proc_point(&r, o);
                        let ld = Self::proc_dir(&r, d);
                        let linv = Self::inv(ld);
                        let root = r.n[0][3].to_bits();
                        let first = r.info[0];
                        let base = sp;
                        let mut m = root;
                        loop {
                            self.step()?;
                            let q = self.node(m)?;
                            if q.count > 0 {
                                for k in 0..q.count {
                                    self.step()?;
                                    let ti = first + q.left_or_first + k;
                                    let x = self.intersect_tri(lo, ld, ti, hit.t)?;
                                    if x < hit.t {
                                        hit = Hit { t: x, tri: ti, inst: rec };
                                        if mode != TRACE_CLOSEST {
                                            return Ok(hit);
                                        }
                                    }
                                }
                                if sp == base {
                                    break;
                                }
                                sp -= 1;
                                m = stack[sp];
                                continue;
                            }
                            let ml = root + q.left_or_first;
                            let mr = ml + 1;
                            let (ql, qr) = (self.node(ml)?, self.node(mr)?);
                            let mut ul = Self::ray_aabb(lo, linv, ql.bmin, ql.bmax, hit.t);
                            let mut ur = Self::ray_aabb(lo, linv, qr.bmin, qr.bmax, hit.t);
                            if ul == INF && ur == INF {
                                if sp == base {
                                    break;
                                }
                                sp -= 1;
                                m = stack[sp];
                                continue;
                            }
                            let (mut mnear, mut mfar) = (ml, mr);
                            if ur < ul {
                                (mnear, mfar) = (mr, ml);
                                std::mem::swap(&mut ul, &mut ur);
                            }
                            m = mnear;
                            if ur != INF && sp < STACK {
                                stack[sp] = mfar;
                                sp += 1;
                                self.deepest.set(self.deepest.get().max(sp));
                            }
                        }
                    }
                    if sp == 0 {
                        break;
                    }
                    sp -= 1;
                    node = stack[sp];
                    continue;
                }
                let l = top + n.left_or_first;
                let r = l + 1;
                let (nl, nr) = (self.node(l)?, self.node(r)?);
                let mut tl = Self::ray_aabb(o, inv_d, nl.bmin, nl.bmax, hit.t);
                let mut tr = Self::ray_aabb(o, inv_d, nr.bmin, nr.bmax, hit.t);
                if tl == INF && tr == INF {
                    if sp == 0 {
                        break;
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
                if tr != INF && sp < STACK {
                    stack[sp] = far;
                    sp += 1;
                    self.deepest.set(self.deepest.get().max(sp));
                }
            }
            Ok(hit)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::cpu::*;
    use super::*;
    use glam::{Quat, Vec3};

    /// The transcriptions above are the shaders': the statements they follow are still there, in this order. A change
    /// to either walk fails here, and the transcription is to follow it.
    #[test]
    fn the_cpu_kernel_is_the_shaders() {
        let ordered = |body: &str, what: &str, lines: &[&str]| {
            let mut at = 0;
            for l in lines {
                at += body[at..].find(l).unwrap_or_else(|| panic!("{what} no longer has `{l}` where the CPU kernel expects it")) + l.len();
            }
        };
        let units = include_str!("../shaders/trace_units.wgsl");
        ordered(
            units,
            "trace_units.wgsl",
            &[
                "fn trace_units(o: vec3<f32>, d: vec3<f32>, hit_in: Hit, mode: u32) -> Hit {",
                "if (bitcast<u32>(P.motion.z) == 0u) { return hit; }",
                "let top = bitcast<u32>(P.motion.y);",
                "var stack: array<u32, 48>;",
                "var node = top;",
                "let n = nodes[node];",
                "if (n.count > 0u) {",
                "let rec = n.left_or_first;",
                "let r = instances[rec];",
                "if (mode != TRACE_SHADOW || bitcast<u32>(r.n1.w) != 0u) {",
                "let lo = proc_point(r, o);",
                "let ld = proc_dir(r, d);",
                "let linv = 1.0 / select(ld, vec3<f32>(1e-20), abs(ld) < vec3<f32>(1e-20));",
                "let root = bitcast<u32>(r.n0.w);",
                "let first = r.info.x;",
                "let base = sp;",
                "var m = root;",
                "let q = nodes[m];",
                "if (q.count > 0u) {",
                "for (var k = 0u; k < q.count; k++) {",
                "let ti = first + q.left_or_first + k;",
                "let x = intersect_tri(lo, ld, ti, hit.t);",
                "if (x.x < hit.t) {",
                "hit = Hit(x.x, ti, x.y, x.z, rec);",
                "if (mode != TRACE_CLOSEST) { return hit; }",
                "if (sp == base) { break; }",
                "m = stack[sp];",
                "let ml = root + q.left_or_first;",
                "let mr = ml + 1u;",
                "var ul = ray_aabb(lo, linv, ql.bmin, ql.bmax, hit.t);",
                "var ur = ray_aabb(lo, linv, qr.bmin, qr.bmax, hit.t);",
                "if (ul == INF && ur == INF) {",
                "if (sp == base) { break; }",
                "if (ur < ul) {",
                "m = mnear;",
                "if (ur != INF && sp < 48u) {",
                "stack[sp] = mfar;",
                "if (sp == 0u) { break; }",
                "node = stack[sp];",
                "let l = top + n.left_or_first;",
                "let r = l + 1u;",
                "var tl = ray_aabb(o, inv_d, nl.bmin, nl.bmax, hit.t);",
                "var tr = ray_aabb(o, inv_d, nr.bmin, nr.bmax, hit.t);",
                "if (tl == INF && tr == INF) {",
                "if (sp == 0u) { break; }",
                "if (tr < tl) {",
                "node = near;",
                "if (tr != INF && sp < 48u) {",
                "stack[sp] = far;",
                "return hit;",
            ],
        );
        assert_eq!(units.matches("loop {").count(), 2, "two walks");
        let kernel = include_str!("../shaders/pathtrace.wgsl");
        ordered(
            kernel,
            "pathtrace.wgsl",
            &[
                "fn proc_dir(r: Instance, d: vec3<f32>) -> vec3<f32> { return r.n0.xyz * d.x + r.n1.xyz * d.y + r.n2.xyz * d.z; }",
                "fn proc_point(r: Instance, p: vec3<f32>) -> vec3<f32> { return proc_dir(r, p - vec3<f32>(r.m0.w, r.m1.w, r.m2.w)); }",
                "fn intersect_tri(o: vec3<f32>, d: vec3<f32>, i: u32, tmax: f32) -> vec3<f32> {",
                "let v0 = tri_verts[i * 3u].xyz;",
                "let e1 = tri_verts[i * 3u + 1u].xyz;",
                "let e2 = tri_verts[i * 3u + 2u].xyz;",
                "let pv = cross(d, e2);",
                "let det = dot(e1, pv);",
                "if (abs(det) < 1e-12) { return vec3<f32>(INF, 0.0, 0.0); }",
                "let u = dot(tv, pv) * inv;",
                "if (u < 0.0 || u > 1.0) { return vec3<f32>(INF, 0.0, 0.0); }",
                "let qv = cross(tv, e1);",
                "let v = dot(d, qv) * inv;",
                "if (v < 0.0 || u + v > 1.0) { return vec3<f32>(INF, 0.0, 0.0); }",
                "let t = dot(e2, qv) * inv;",
                "if (t <= 0.0 || t >= tmax) { return vec3<f32>(INF, 0.0, 0.0); }",
                "fn trace(o: vec3<f32>, d: vec3<f32>, tmax_in: f32, mode: u32) -> Hit {",
                "if (mode == TRACE_SHADOW && (flags & TRI_NO_SHADOW) != 0u) { continue; }",
                "let r = intersect_tri(o, d, ti, hit.t);",
                "hit = Hit(r.x, ti, r.y, r.z, NO_INST);",
                "if (mode != TRACE_CLOSEST) { return hit; }",
                "if (has(F_PROC)) { return trace_procs(o, d, hit, mode); }",
            ],
        );
        // What the stack and the record's words are here is what they are there.
        assert_eq!(STACK, 48);
        assert!(TOP_DEPTH + MESH_DEPTH <= STACK);
    }

    /// A deterministic stream of numbers in [0, 1).
    struct Rng(u64);
    impl Rng {
        fn f(&mut self) -> f32 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((self.0 >> 40) as f32) / (1u64 << 24) as f32
        }
        fn below(&mut self, n: usize) -> usize {
            ((self.f() * n as f32) as usize).min(n.saturating_sub(1))
        }
        fn v(&mut self, s: f32) -> Vec3A {
            Vec3A::new(self.f() - 0.5, self.f() - 0.5, self.f() - 0.5) * s
        }
        fn tris(&mut self, n: usize, spread: f32, size: f32) -> Vec<[Vec3A; 3]> {
            (0..n)
                .map(|_| {
                    let c = self.v(spread);
                    [c + self.v(size), c + self.v(size), c + self.v(size)]
                })
                .collect()
        }
    }

    /// A scene as the renderer keeps one under software traversal with units, small: the buffers the kernel reads and
    /// the bookkeeping that writes them. Its operations are what an edit does (live.rs): a mesh's triangles and tree
    /// into room, a record, the top tree again.
    struct Scene {
        nodes: Vec<GpuNode>,
        tri_verts: Vec<[f32; 4]>,
        instances: Vec<GpuInstance>,
        layout: Layout,
        still_tris: usize,
        /// Meshes: first triangle, triangle count, root, node count, model bounds. None: removed.
        meshes: Vec<Option<(u32, u32, u32, usize, Vec3A, Vec3A)>>,
        /// Units by record: mesh, world, casts shadows. None: a free record.
        units: Vec<Option<(usize, Mat4, bool)>>,
        top: u32,
        count: u32,
    }

    fn put_tris(out: &mut Vec<[f32; 4]>, tris: &[[Vec3A; 3]]) {
        for [a, b, c] in tris {
            let (e1, e2) = (*b - *a, *c - *a);
            out.extend([[a.x, a.y, a.z, 0.0], [e1.x, e1.y, e1.z, 0.0], [e2.x, e2.y, e2.z, 0.0]]);
        }
    }

    impl Scene {
        fn new(still: &[[Vec3A; 3]], mesh_room: usize, units: usize) -> Scene {
            let b = bvh::build(still);
            let mut tri_verts = vec![];
            put_tris(&mut tri_verts, &b.order.iter().map(|&i| still[i as usize]).collect::<Vec<_>>());
            // (An empty still scene: the tree over nothing is a leaf of triangle 0, here three zero triangles; the meshes'
            // triangles follow whole triangles.)
            if still.is_empty() {
                tri_verts.resize(9, [0.0; 4]);
            }
            let layout = Layout::new(b.nodes.len(), mesh_room, units);
            let mut nodes = b.nodes;
            nodes.resize(nodes.len() + layout.nodes(), FILL);
            Scene { nodes, tri_verts, instances: vec![GpuInstance::default()], top: layout.top_first, layout, still_tris: still.len(), meshes: vec![], units: vec![], count: 0 }
        }

        /// A new mesh: its tree placed and written, its triangles appended in the tree's order.
        fn add_mesh(&mut self, tris: &[[Vec3A; 3]]) -> Result<usize, String> {
            let t = mesh_tree(tris)?;
            let root = self.layout.place(t.nodes.len()).ok_or("no room for the tree")?;
            self.nodes[root as usize..root as usize + t.nodes.len()].copy_from_slice(&t.nodes);
            let first = (self.tri_verts.len() / 3) as u32;
            put_tris(&mut self.tri_verts, &t.order.iter().map(|&i| tris[i as usize]).collect::<Vec<_>>());
            let (lo, hi) = tri_boxes(tris).into_iter().fold((Vec3A::splat(f32::INFINITY), Vec3A::splat(f32::NEG_INFINITY)), |a, b| (a.0.min(b.0), a.1.max(b.1)));
            self.meshes.push(Some((first, tris.len() as u32, root, t.nodes.len(), lo, hi)));
            Ok(self.meshes.len() - 1)
        }

        /// The mesh gone: its tree's nodes room again (`FILL`: a stale root must end a walk).
        fn remove_mesh(&mut self, k: usize) {
            let (_, _, root, len, ..) = self.meshes[k].take().unwrap();
            self.nodes[root as usize..root as usize + len].fill(FILL);
            self.layout.release(root, len);
        }

        fn record(&self, mesh: usize, world: Mat4, cast: bool) -> GpuInstance {
            let (first, _, root, ..) = self.meshes[mesh].unwrap();
            let inst = crate::scene::Instance { mesh: mesh as u32, world, object: 0, cast_shadow: cast, name: String::new(), sub: None, hidden: false };
            unit_record(inst.gpu(first, 0), root, cast)
        }

        fn add_unit(&mut self, mesh: usize, world: Mat4, cast: bool) -> usize {
            let k = self.units.iter().position(|u| u.is_none()).unwrap_or_else(|| {
                self.units.push(None);
                self.units.len() - 1
            });
            self.units[k] = Some((mesh, world, cast));
            if self.instances.len() <= k {
                self.instances.resize(k + 1, GpuInstance::default());
            }
            self.instances[k] = self.record(mesh, world, cast);
            k
        }

        /// The top tree again, over the units there are now, into its range.
        fn retop(&mut self) -> Result<(), String> {
            let boxes: Vec<UnitBox> = self.units.iter().enumerate().filter_map(|(k, u)| u.map(|(m, w, _)| (k, m, w))).map(|(k, m, w)| { let (.., lo, hi) = self.meshes[m].unwrap(); UnitBox::new(k as u32, w, lo, hi) }).collect();
            self.count = boxes.len() as u32;
            if boxes.is_empty() {
                return Ok(());
            }
            if boxes.len() > self.layout.max_units() {
                return Err("no room for the top tree".into());
            }
            let t = top_tree(&boxes)?;
            let at = self.layout.top_first as usize;
            self.nodes[at..at + t.len()].copy_from_slice(&t);
            Ok(())
        }

        fn kernel(&self) -> Kernel<'_> {
            let budget = 64 * (self.nodes.len() as u64 + self.tri_verts.len() as u64) + 1024;
            Kernel { nodes: &self.nodes, tri_verts: &self.tri_verts, instances: &self.instances, top: self.top, units: self.count, steps: Default::default(), budget, deepest: Default::default() }
        }

        /// Every triangle a ray could meet, tested one by one with the kernel's own triangle test (the still scene's in
        /// the world, a unit's in the unit's space, as the kernel does): the closest hit.
        fn brute(&self, o: Vec3A, d: Vec3A, mode: u32) -> Hit {
            let k = self.kernel();
            let mut best = Hit { t: INF, tri: 0xffff_ffff, inst: NO_INST };
            for ti in 0..self.still_tris as u32 {
                let t = k.intersect_tri(o, d, ti, best.t).unwrap();
                if t < best.t {
                    best = Hit { t, tri: ti, inst: NO_INST };
                }
            }
            for (rec, u) in self.units.iter().enumerate() {
                let Some((mesh, _, cast)) = u else { continue };
                if mode == TRACE_SHADOW && !cast {
                    continue;
                }
                let r = &self.instances[rec];
                let (lo, ld) = (Kernel::proc_point(r, o), Kernel::proc_dir(r, d));
                let (first, count, ..) = self.meshes[*mesh].unwrap();
                for ti in first..first + count {
                    let t = k.intersect_tri(lo, ld, ti, best.t).unwrap();
                    if t < best.t {
                        best = Hit { t, tri: ti, inst: rec as u32 };
                    }
                }
            }
            best
        }

        /// Rays against the scene: the kernel's walk reads nothing outside its buffers, stops, and finds brute force's
        /// closest hit (the same distance to the bit; the same triangle unless two are at that distance); an any-hit
        /// walk finds something exactly when there is something. Returns how many rays hit a unit.
        fn check(&self, rng: &mut Rng, rays: usize, what: &str) -> usize {
            let mut on_units = 0;
            let targets: Vec<Vec3A> = self.units.iter().flatten().map(|(m, w, _)| { let (.., lo, hi) = self.meshes[*m].unwrap(); w.transform_point3a((lo + hi) * 0.5) }).collect();
            for k in 0..rays {
                let o = rng.v(60.0);
                // Half of the rays at a unit (random ones mostly miss small things), half anywhere.
                let d = if k % 2 == 0 && !targets.is_empty() { (targets[rng.below(targets.len())] + rng.v(1.0) - o).normalize_or(Vec3A::X) } else { rng.v(2.0).normalize_or(Vec3A::X) };
                for mode in [TRACE_CLOSEST, TRACE_SHADOW, TRACE_ANY] {
                    let kernel = self.kernel();
                    let got = kernel.trace(o, d, INF, mode).unwrap_or_else(|e| panic!("{what}: ray {o} {d}, mode {mode}: the kernel {e}"));
                    let want = self.brute(o, d, mode);
                    if mode == TRACE_CLOSEST {
                        assert_eq!(got.t.to_bits(), want.t.to_bits(), "{what}: ray {o} {d}: the walk's hit {got:?}, brute force's {want:?}");
                        if got.tri != want.tri || got.inst != want.inst {
                            // Two triangles at one distance: both are closest.
                            let again = Kernel { budget: u64::MAX, ..self.kernel() };
                            let t = if got.inst == NO_INST { again.intersect_tri(o, d, got.tri, INF).unwrap() } else { let r = &self.instances[got.inst as usize]; again.intersect_tri(Kernel::proc_point(r, o), Kernel::proc_dir(r, d), got.tri, INF).unwrap() };
                            assert_eq!(t.to_bits(), want.t.to_bits(), "{what}: ray {o} {d}: {got:?} is not at brute force's distance {want:?}");
                        }
                        on_units += (got.inst != NO_INST) as usize;
                    } else {
                        assert_eq!(got.tri != 0xffff_ffff, want.tri != 0xffff_ffff, "{what}: ray {o} {d}, mode {mode}: {got:?} against {want:?}");
                    }
                    assert!(kernel.deepest.get() <= STACK);
                }
            }
            on_units
        }
    }

    fn pose(rng: &mut Rng) -> Mat4 {
        let s = Vec3::new(0.3 + 2.0 * rng.f(), 0.3 + 2.0 * rng.f(), 0.3 + 2.0 * rng.f());
        let q = Quat::from_euler(glam::EulerRot::XYZ, rng.f() * 6.0, rng.f() * 6.0, rng.f() * 6.0);
        Mat4::from_scale_rotation_translation(s, q, Vec3::from(rng.v(30.0)))
    }

    /// Random scenes under random edits: meshes added and removed, units added, moved and removed, the top tree built
    /// again after each, as an edit does. After every edit the kernel's two walks, on the CPU with every read checked,
    /// agree with brute force. Scenes with and without a still part; units that overlap; a scene emptied of units.
    #[test]
    fn the_walks_agree_with_brute_force_under_random_edits() {
        let (mut edits, mut unit_hits, mut rays) = (0, 0, 0);
        for seed in 0..12u64 {
            let mut rng = Rng(seed * 7919 + 1);
            let n = 50 + rng.below(400);
            let still = if seed % 3 == 0 { vec![] } else { rng.tris(n, 40.0, 3.0) };
            let mut sc = Scene::new(&still, 6000, 64);
            sc.check(&mut rng, 40, "no unit yet");
            for step in 0..60 {
                let live_meshes: Vec<usize> = (0..sc.meshes.len()).filter(|&k| sc.meshes[k].is_some()).collect();
                let live_units: Vec<usize> = (0..sc.units.len()).filter(|&k| sc.units[k].is_some()).collect();
                let what;
                match rng.below(10) {
                    0 | 1 => {
                        // A mesh: a few triangles, or a few hundred; some flat (a quad's two triangles in one plane).
                        let n = [1, 2, 12, 80, 300][rng.below(5)];
                        let tris = if rng.f() < 0.2 { vec![[Vec3A::ZERO, Vec3A::X, Vec3A::Y], [Vec3A::X, Vec3A::new(1.0, 1.0, 0.0), Vec3A::Y]] } else { rng.tris(n, 4.0, 1.5) };
                        match sc.add_mesh(&tris) {
                            Ok(m) => {
                                sc.add_unit(m, pose(&mut rng), rng.f() < 0.8);
                                what = "a mesh and a unit of it";
                            }
                            Err(e) => {
                                assert_eq!(e, "no room for the tree");
                                continue;
                            }
                        }
                    }
                    2 | 3 | 4 if !live_meshes.is_empty() && live_units.len() < sc.layout.max_units() => {
                        // Another unit of a mesh there is; sometimes right on top of a unit there is (overlap).
                        let world = if !live_units.is_empty() && rng.f() < 0.3 { sc.units[live_units[rng.below(live_units.len())]].unwrap().1 * Mat4::from_translation(Vec3::from(rng.v(0.5))) } else { pose(&mut rng) };
                        sc.add_unit(live_meshes[rng.below(live_meshes.len())], world, rng.f() < 0.8);
                        what = "a unit";
                    }
                    5 | 6 | 7 if !live_units.is_empty() => {
                        let k = live_units[rng.below(live_units.len())];
                        let (m, _, cast) = sc.units[k].unwrap();
                        let world = pose(&mut rng);
                        sc.units[k] = Some((m, world, cast));
                        sc.instances[k] = sc.record(m, world, cast);
                        what = "a unit moved";
                    }
                    8 if !live_units.is_empty() => {
                        let k = live_units[rng.below(live_units.len())];
                        let (m, ..) = sc.units[k].take().unwrap();
                        // (The record stays as it was: nothing names it once the top tree is built again.)
                        if !sc.units.iter().flatten().any(|u| u.0 == m) {
                            sc.remove_mesh(m);
                        }
                        what = "a unit removed";
                    }
                    9 if seed % 4 == 1 && step > 30 => {
                        // Every unit and every mesh gone: a scene that has units' room and none.
                        for k in live_units {
                            sc.units[k] = None;
                        }
                        for m in live_meshes {
                            sc.remove_mesh(m);
                        }
                        what = "every unit removed";
                    }
                    _ => continue,
                }
                sc.retop().unwrap();
                edits += 1;
                unit_hits += sc.check(&mut rng, 30, what);
                rays += 30;
            }
            // The mesh range's free runs add up to what the removed trees gave back.
            let used: usize = sc.meshes.iter().flatten().map(|m| m.3).sum();
            assert_eq!(sc.layout.free_nodes() + used, sc.layout.mesh_cap as usize);
        }
        assert!(edits > 300 && unit_hits * 8 > rays, "{edits} edits, {unit_hits} of {rays} closest hits on units");
    }

    /// The trees the builders make are the trees the checks accept, at their depth bounds: a top tree of a thousand
    /// units, a mesh of triangles all in one place (no split by position: median splits), a long strip.
    #[test]
    fn the_builders_keep_their_depths() {
        let mut rng = Rng(5);
        let units: Vec<UnitBox> = (0..1000).map(|k| UnitBox::new(k, Mat4::from_translation(Vec3::from(rng.v(100.0))), Vec3A::splat(-0.25), Vec3A::splat(0.25))).collect();
        let top = top_tree(&units).unwrap();
        assert_eq!(top.len(), 1999);
        assert_eq!(check_top(&top, |r| r < 1000).unwrap(), 10, "1000 units: ceil(log2)");
        assert!(top_tree(&[]).is_err());
        let one = top_tree(&units[..1]).unwrap();
        assert_eq!((one.len(), one[0].count, one[0].left_or_first), (1, 1, 0));
        // 5,000 copies of one triangle: no position splits them.
        let same = vec![[Vec3A::ZERO, Vec3A::X, Vec3A::Y]; 5000];
        let t = mesh_tree(&same).unwrap();
        assert!(t.depth <= MESH_DEPTH && check_mesh(&t.nodes, 5000).is_ok(), "depth {}", t.depth);
        let mut seen = t.order.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..5000).collect::<Vec<u32>>());
        // A strip along x, each triangle a little longer than the last.
        let strip: Vec<[Vec3A; 3]> = (0..4000).map(|k| { let x = (k as f32).powf(1.5); [Vec3A::new(x, 0.0, 0.0), Vec3A::new(x + 1.0, 0.0, 0.0), Vec3A::new(x, 1.0, 0.0)] }).collect();
        assert!(mesh_tree(&strip).unwrap().depth <= MESH_DEPTH);
        assert!(mesh_tree(&[]).is_err());
        assert!(mesh_tree(&[[Vec3A::ZERO, Vec3A::X, Vec3A::splat(f32::NAN)]]).is_err());
    }

    /// The checks refuse what the kernel's walk would not end on, or would read outside its buffers on: a child that
    /// points back or at itself, a child outside the tree, a node with two parents, a leaf past its mesh, a leaf of
    /// too many triangles, a box that is not a number, a top leaf that names no unit or one twice, a tree too deep.
    #[test]
    fn the_checks_refuse_broken_trees() {
        let mut rng = Rng(9);
        let tris = rng.tris(200, 5.0, 1.0);
        let good = mesh_tree(&tris).unwrap().nodes;
        assert!(check_mesh(&good, 200).is_ok());
        let interior = good.iter().position(|n| n.count == 0).unwrap();
        let leaf = good.iter().position(|n| n.count > 0).unwrap();
        let broken = |f: &dyn Fn(&mut Vec<GpuNode>)| {
            let mut n = good.clone();
            f(&mut n);
            check_mesh(&n, 200).err().unwrap_or_else(|| "(accepted)".into())
        };
        assert!(broken(&|n| n[interior].left_or_first = interior as u32).contains("not after it"), "a child that is itself");
        let deeper = (1..good.len()).find(|&k| good[k].count == 0).unwrap();
        assert!(broken(&|n| n[deeper].left_or_first = 0).contains("not after it"), "a child that points back");
        assert!(broken(&|n| n[interior].left_or_first = n.len() as u32 - 1).contains("inside the tree"), "a right child outside");
        assert!(broken(&|n| { let l = n[0].left_or_first; let k = (l as usize..n.len()).find(|&k| n[k].count == 0 && n[k].left_or_first > l + 1).unwrap(); n[k].left_or_first = n[0].left_or_first + 2; }).len() > 0, "a node with two parents, or nodes not reached");
        assert!(broken(&|n| n[leaf].left_or_first = 200).contains("triangles"), "a leaf past its mesh");
        assert!(broken(&|n| n[leaf].count = 17).contains("triangles"), "a leaf of 17");
        assert!(broken(&|n| n[leaf].count = 0x3f80_0000).contains("triangles"), "a count that is a float's bits");
        assert!(broken(&|n| n[interior].bmin[1] = f32::NAN).contains("not finite"));
        assert!(broken(&|n| n.truncate(n.len() - 1)).len() > 0, "a tree cut short");
        assert!(check_mesh(&[], 1).is_err());
        // Zeros (room that was never filled): an interior node whose children are nodes 0 and 1.
        assert!(check_mesh(&[GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 }; 3], 1).is_err());
        // A top tree.
        let units: Vec<UnitBox> = (0..9).map(|k| UnitBox::new(k * 2, Mat4::from_translation(Vec3::new(k as f32, 0.0, 0.0)), Vec3A::ZERO, Vec3A::ONE)).collect();
        let top = top_tree(&units).unwrap();
        assert!(check_top(&top, |r| r % 2 == 0 && r < 18).is_ok());
        assert!(check_top(&top, |r| r < 8).unwrap_err().contains("is not one unit"));
        let mut twice = top.clone();
        let leaves: Vec<usize> = (0..twice.len()).filter(|&k| twice[k].count > 0).collect();
        twice[leaves[0]].left_or_first = twice[leaves[1]].left_or_first;
        assert!(check_top(&twice, |r| r % 2 == 0 && r < 18).unwrap_err().contains("twice"));
        // A chain deeper than a top tree may be.
        let mut chain = vec![];
        for k in 0..(TOP_DEPTH as u32 + 2) {
            chain.push(GpuNode { bmin: [0.0; 3], left_or_first: 2 * k + 1, bmax: [1.0; 3], count: 0 });
            chain.push(GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [1.0; 3], count: 1 });
        }
        // (Node 2k is interior with children 2k + 1 (a leaf) and 2k + 2 (the next interior); the last pair are leaves.)
        chain.pop();
        let last = chain.len() - 1;
        chain[last] = GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [1.0; 3], count: 1 };
        assert!(check_tree(&chain, 64, |_, _| Ok(())).is_ok());
        assert!(check_tree(&chain, TOP_DEPTH, |_, _| Ok(())).unwrap_err().contains("deeper"));
    }

    /// Room and the place of trees: first fit, runs joined again when trees go, nothing lost.
    #[test]
    fn the_layout_places_and_takes_back() {
        let mut l = Layout::new(100, 50, 8);
        assert_eq!((l.mesh_first, l.top_first, l.top_cap, l.nodes(), l.max_units()), (100, 150, 15, 65, 8));
        let (a, b, c) = (l.place(10).unwrap(), l.place(20).unwrap(), l.place(20).unwrap());
        assert_eq!((a, b, c), (100, 110, 130));
        assert!(l.place(1).is_none());
        l.release(b, 20);
        assert_eq!(l.place(5), Some(110));
        l.release(a, 10);
        l.release(110, 5);
        assert_eq!(l.free_nodes(), 30);
        assert_eq!(l.place(30), Some(100), "the runs are one again");
        l.release(c, 20);
        l.release(100, 30);
        assert_eq!((l.free_nodes(), l.place(50)), (50, Some(100)));
        // No unit at all: a top range of one node, no mesh range.
        let none = Layout::new(7, 0, 0);
        assert_eq!((none.nodes(), none.max_units()), (1, 1));
    }
}
