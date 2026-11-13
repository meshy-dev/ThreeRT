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
//! is one unit: `left_or_first` the unit's record, `count` 1. It is built by the surface-area heuristic (`build_sah`)
//! and written whole whenever a unit changes, but seldom built over all the units: it is kept with its boxes brought
//! up to date (`refit_top`) while units only move a little, and units that come, or travel, get a tree of their own
//! beside the others' (`Kept`: the settled and the loose), which is what is built when they move.
//!
//! Depths: one stack of `STACK` entries, the kernel module's (trace_units.wgsl says why not a walk's own): the still
//! walk uses it, leaves it empty, and both levels of the unit walk share it (a push past it is dropped: geometry
//! missed, never a hang). A walk's stack is never deeper than its tree, so the
//! top tree is held to `TOP_DEPTH` (its builder takes no split the levels left could not finish, and stops a level
//! short of it: the top tree may be a root over two built trees) and a mesh tree to
//! `MESH_DEPTH` (a tree deeper than that is built again by median splits, which halve), and the two fit. Binned SAH
//! is not balanced: its tree over 5.86 M triangles (the catalog's largest model) is 42 deep, twice a balanced one's
//! 21, and over 30 k to 500 k triangles 17 to 22. The still scene's walk uses 48 entries for its one tree, as before.

use crate::bvh::{self, GpuNode};
use crate::scene::GpuInstance;
use glam::{Mat4, Vec3A};

/// The stack of a kernel with units (trace_units.wgsl `stack`, the module's): entries shared by the unit walk's two
/// levels, and used by the still walk before them.
pub const STACK: usize = 64;
/// The still walk's stack in a kernel without units, and how much of the module's the still walk uses in one with.
pub const STILL_STACK: usize = 48;
/// The deepest top tree (levels below the root): a built tree is no deeper than 18, whatever its units' boxes
/// (`build_sah`), and the top tree is one, or a root over two (`Kept`).
pub const TOP_DEPTH: usize = 19;
/// The deepest mesh tree (45): what the stack has left. SAH trees deeper than this are rebuilt by median splits.
pub const MESH_DEPTH: usize = STACK - TOP_DEPTH;
/// The most units a top tree holds: 2^18, what a built tree's depth has room for.
pub const MAX_UNITS: usize = 1 << (TOP_DEPTH - 1);
/// A leaf's most triangles (bvh.rs builds leaves of up to 16).
pub const MAX_LEAF: u32 = 16;

/// What unused room in the node buffer holds: a leaf of one item, which ends a walk that lands on it (zeros would be
/// an interior node whose children are node 0 and node 1).
pub const FILL: GpuNode = bvh::EMPTY;

/// A mesh's tree: its nodes (relocatable), the order it puts the mesh's triangles in (`order[i]`: the triangle now at
/// position i), its depth, the bounds of the triangles it was built over (model space).
pub struct MeshTree {
    pub nodes: Vec<GpuNode>,
    pub order: Vec<u32>,
    pub depth: usize,
    pub lo: Vec3A,
    pub hi: Vec3A,
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

/// The bins of a top tree's split along an axis.
const TOP_BINS: usize = 32;

fn area(lo: Vec3A, hi: Vec3A) -> f32 {
    let e = (hi - lo).max(Vec3A::ZERO);
    2.0 * (e.x * e.y + e.y * e.z + e.z * e.x)
}

/// A tree over boxes (min, max) by the surface-area heuristic, one box a leaf, no deeper than `max_depth` whatever
/// the boxes are (there are at most 2^`max_depth` of them). Each split is the cheapest of the binned ones along the
/// three axes of the boxes' centres (cost: each side's area times its count, the chance a ray through the node meets
/// the side times what is in it), among those that leave both sides no more boxes than the levels below can still
/// halve down to one; where there is none (every centre in one place; a split the depth forbids), the median along
/// the widest axis, which halves. So a node at depth d holds at most 2^(`max_depth` - d) boxes, and a leaf is no
/// deeper than `max_depth`. Returns the nodes (a leaf's `left_or_first`: its position in the order) and the order.
///
/// Why not the median everywhere (what the top tree was at first): units are not of one size. A median split by
/// centres puts a wall and the lamp beside it in whichever halves their centres fall in, and both halves' boxes then
/// span the wall: a ray reads far more of the tree than it enters units (the voxel level: 82 top nodes read a ray
/// for 2.2 units entered; `units_cost.rs`).
fn build_sah(boxes: &[(Vec3A, Vec3A)], max_depth: usize) -> (Vec<GpuNode>, Vec<u32>) {
    let mut order: Vec<u32> = (0..boxes.len() as u32).collect();
    let mut nodes = vec![FILL];
    if boxes.is_empty() {
        return (nodes, order);
    }
    debug_assert!(max_depth >= usize::BITS as usize || boxes.len() <= 1 << max_depth);
    let centres: Vec<Vec3A> = boxes.iter().map(|b| (b.0 + b.1) * 0.5).collect();
    let mut stack = vec![(0usize, 0usize, boxes.len(), 0usize)];
    while let Some((ni, start, end, depth)) = stack.pop() {
        let (mut lo, mut hi) = (Vec3A::splat(f32::INFINITY), Vec3A::splat(f32::NEG_INFINITY));
        let (mut clo, mut chi) = (lo, hi);
        for &k in &order[start..end] {
            lo = lo.min(boxes[k as usize].0);
            hi = hi.max(boxes[k as usize].1);
            clo = clo.min(centres[k as usize]);
            chi = chi.max(centres[k as usize]);
        }
        let count = end - start;
        if count == 1 {
            nodes[ni] = GpuNode { bmin: lo.into(), left_or_first: start as u32, bmax: hi.into(), count: 1 };
            continue;
        }
        // The most boxes a side may hold: what the levels under a child still halve down to one.
        let room = 1usize.checked_shl((max_depth - depth - 1) as u32).unwrap_or(usize::MAX);
        let ext = chi - clo;
        let bin = |k: u32, axis: usize| (((centres[k as usize][axis] - clo[axis]) * (TOP_BINS as f32 / ext[axis])) as usize).min(TOP_BINS - 1);
        let mut best = (f32::INFINITY, 0usize, 0usize);
        for axis in 0..3 {
            if !(ext[axis] > 0.0) {
                continue;
            }
            let mut bins = [(Vec3A::splat(f32::INFINITY), Vec3A::splat(f32::NEG_INFINITY), 0usize); TOP_BINS];
            for &k in &order[start..end] {
                let b = &mut bins[bin(k, axis)];
                *b = (b.0.min(boxes[k as usize].0), b.1.max(boxes[k as usize].1), b.2 + 1);
            }
            // The right sides' areas from the last bin back, then the left sides' forward.
            let mut right = [0.0f32; TOP_BINS];
            let mut acc = (Vec3A::splat(f32::INFINITY), Vec3A::splat(f32::NEG_INFINITY));
            for i in (1..TOP_BINS).rev() {
                acc = (acc.0.min(bins[i].0), acc.1.max(bins[i].1));
                right[i] = area(acc.0, acc.1);
            }
            let mut acc = (Vec3A::splat(f32::INFINITY), Vec3A::splat(f32::NEG_INFINITY));
            let mut nl = 0;
            for i in 1..TOP_BINS {
                acc = (acc.0.min(bins[i - 1].0), acc.1.max(bins[i - 1].1));
                nl += bins[i - 1].2;
                let nr = count - nl;
                if nl == 0 || nr == 0 || nl > room || nr > room {
                    continue;
                }
                let cost = area(acc.0, acc.1) * nl as f32 + right[i] * nr as f32;
                if cost < best.0 {
                    best = (cost, axis, i);
                }
            }
        }
        let mid = if best.0.is_finite() {
            let (_, axis, split) = best;
            let slice = &mut order[start..end];
            let mut i = 0;
            for j in 0..slice.len() {
                if bin(slice[j], axis) < split {
                    slice.swap(i, j);
                    i += 1;
                }
            }
            i
        } else {
            let axis = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
            order[start..end].select_nth_unstable_by(count / 2, |&a, &b| centres[a as usize][axis].total_cmp(&centres[b as usize][axis]).then(a.cmp(&b)));
            count / 2
        };
        let left = nodes.len();
        nodes.push(FILL);
        nodes.push(FILL);
        nodes[ni] = GpuNode { bmin: lo.into(), left_or_first: left as u32, bmax: hi.into(), count: 0 };
        stack.push((left + 1, start + mid, end, depth + 1));
        stack.push((left, start, start + mid, depth + 1));
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
    let (lo, hi) = tris.iter().flatten().fold((Vec3A::splat(f32::INFINITY), Vec3A::splat(f32::NEG_INFINITY)), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    Ok(MeshTree { nodes, order, depth, lo, hi })
}

/// A unit for the top tree: its record, and its mesh's model-space bounds under its world transform.
#[derive(Clone, Copy, Debug)]
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

/// The top tree over `units` (at least one): by the surface-area heuristic (`build_sah`) down to one unit a leaf, a
/// level less deep than `TOP_DEPTH` (so that two can be joined); each leaf's `left_or_first` its unit's record.
/// Checked against the units it was given.
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
    let (mut nodes, order) = build_sah(&boxes, TOP_DEPTH - 1);
    for n in nodes.iter_mut().filter(|n| n.count > 0) {
        n.left_or_first = units[order[n.left_or_first as usize] as usize].record;
    }
    let records: std::collections::HashSet<u32> = units.iter().map(|u| u.record).collect();
    check_top(&nodes, |r| records.contains(&r))?;
    Ok(nodes)
}

/// A unit's record: the instance's (`scene::Instance::gpu`), with, in the three words the normal rows leave unused
/// (trace_units.wgsl reads them as bits): its mesh tree's root; whether it casts shadows; whether it is placed by a
/// translation alone (or not moved at all). The last is read off the record's own normal rows, which are then the
/// identity's: the walk moves the ray's origin and leaves the rest of the ray as it is, which is what multiplying by
/// these rows comes to, to the bit (the sign of a zero aside, which no comparison in the walk sees).
pub fn unit_record(mut r: GpuInstance, root: u32, cast_shadow: bool) -> GpuInstance {
    r.n[0][3] = f32::from_bits(root);
    r.n[1][3] = f32::from_bits(cast_shadow as u32);
    r.n[2][3] = f32::from_bits(translation_alone(&r) as u32);
    r
}

/// Whether a record's normal rows are the identity's.
pub fn translation_alone(r: &GpuInstance) -> bool {
    (0..3).all(|k| (0..3).all(|c| r.n[k][c] == if k == c { 1.0 } else { 0.0 }))
}

/// How much more than at its build the settled units' tree may cost (`top_cost`) and still be kept with its boxes
/// brought up to date (`refit_top`); past it a tree is built over all the units. (The top tree is about half of what
/// a ray reads on a level of 10,000 units, so 30 % of it is about 15 % of a ray.)
pub const REFIT_SLACK: f64 = 1.3;

/// How far a unit may go from where it was placed in the top tree before it is no longer kept there: this many times
/// the longest side of the box it was placed with (`Kept::astray`).
pub const ASTRAY: f32 = 1.0;

/// A scene of no more units than this has its top tree built whenever a unit comes, goes or goes astray: a build is
/// then microseconds.
pub const SMALL: usize = 64;

/// The loose units that have come to rest are put back with the others (one tree built over all) once they are this
/// share of the units; at rest: not gone astray through `QUIET` builds of the loose tree in a row.
pub const FOLD: f64 = 1.0 / 8.0;
pub const QUIET: u8 = 8;

/// What a top tree costs a ray, by surface area: the sum of its interior nodes' box areas (a line through space meets
/// a box in proportion to its area; at each interior node it meets, the walk reads two nodes). Not divided by the
/// root's: a unit that flies off makes the root large, and the tree no better.
pub fn top_cost(nodes: &[GpuNode]) -> f64 {
    nodes.iter().filter(|n| n.count == 0).map(|n| area(Vec3A::from(n.bmin), Vec3A::from(n.bmax)) as f64).sum()
}

/// A top tree as it stands (`nodes`: one `top_tree` built, or this made of it) with its boxes again from `units`: a
/// leaf's is its unit's, an interior node's what holds its two children's. The tree's shape is not touched: units
/// that moved a little are where the tree has them, and a tree that is not rebuilt costs a pass over its nodes
/// instead of a build (0.2 ms against 3.7 ms for 10,000 units). None: the units are not the tree's leaves, one for
/// one (a unit added, removed, hidden or shown: build it again), or `nodes` is no tree. The caller checks the result
/// as it checks a built tree, and compares its cost.
pub fn refit_top(nodes: &[GpuNode], units: &[UnitBox]) -> Option<Vec<GpuNode>> {
    if units.is_empty() || nodes.len() != 2 * units.len() - 1 {
        return None;
    }
    let most = units.iter().map(|u| u.record).max()? as usize;
    let mut at = vec![u32::MAX; most + 1];
    for (k, u) in units.iter().enumerate() {
        if std::mem::replace(&mut at[u.record as usize], k as u32) != u32::MAX {
            return None;
        }
    }
    let mut out = nodes.to_vec();
    // Children are after their parents (`check_tree`): from the last node back, both are done before their parent.
    for i in (0..out.len()).rev() {
        let n = out[i];
        if n.count > 0 {
            let k = std::mem::replace(at.get_mut(n.left_or_first as usize)?, u32::MAX);
            let u = units.get(k as usize)?;
            out[i] = GpuNode { bmin: u.lo.into(), bmax: u.hi.into(), ..n };
        } else {
            let l = n.left_or_first as usize;
            if l <= i || l + 1 >= out.len() {
                return None;
            }
            let (a, b) = (out[l], out[l + 1]);
            out[i] = GpuNode { bmin: Vec3A::from(a.bmin).min(b.bmin.into()).into(), bmax: Vec3A::from(a.bmax).max(b.bmax.into()).into(), ..n };
        }
    }
    Some(out)
}

/// A top tree as it stands (`nodes`, as for `refit_top`) without the leaves whose units are not among `units` (a
/// leaf's sibling takes its parent's place), every box brought up to date. With it, what the tree cost (`top_cost`)
/// with its boxes up to date before any leaf was taken out, and after. Taking leaves out leaves a tree no deeper and
/// no worse for the units that stay. None: `nodes` is no tree, a record is there twice, a unit has no leaf, or no
/// leaf is left. The result is laid out as a build's (children side by side, after their parent); the caller checks
/// it as it checks a built tree.
pub fn thin_top(nodes: &[GpuNode], units: &[UnitBox]) -> Option<(Vec<GpuNode>, [f64; 2])> {
    const NONE: u32 = u32::MAX;
    let n = nodes.len();
    if units.is_empty() || n == 0 {
        return None;
    }
    let most = units.iter().map(|u| u.record).max()? as usize;
    let mut at = vec![NONE; most + 1];
    for (k, u) in units.iter().enumerate() {
        if std::mem::replace(&mut at[u.record as usize], k as u32) != NONE {
            return None;
        }
    }
    // The tree as nodes that name their parent and children. Every node but the root is some node's child once, and
    // children are after their parents: a tree from node 0, whose every walk below ends.
    let mut lo: Vec<Vec3A> = nodes.iter().map(|q| q.bmin.into()).collect();
    let mut hi: Vec<Vec3A> = nodes.iter().map(|q| q.bmax.into()).collect();
    let mut par = vec![NONE; n];
    let mut kid = vec![[NONE; 2]; n];
    let mut alive = vec![true; n];
    for (i, q) in nodes.iter().enumerate() {
        if q.count > 0 {
            continue;
        }
        let l = q.left_or_first as usize;
        if l <= i || l + 1 >= n || par[l] != NONE || par[l + 1] != NONE {
            return None;
        }
        (par[l], par[l + 1], kid[i]) = (i as u32, i as u32, [l as u32, l as u32 + 1]);
    }
    if par[1..].contains(&NONE) {
        return None;
    }
    // Interior nodes' boxes from their children's, the last node first (a node taken out leaves its sibling under
    // its grandparent: still after it), and the sum of their areas.
    let sweep = |lo: &mut [Vec3A], hi: &mut [Vec3A], kid: &[[u32; 2]], alive: &[bool]| {
        let mut cost = 0.0;
        for i in (0..n).rev() {
            if alive[i] && kid[i][0] != NONE {
                let (a, b) = (kid[i][0] as usize, kid[i][1] as usize);
                (lo[i], hi[i]) = (lo[a].min(lo[b]), hi[a].max(hi[b]));
                cost += area(lo[i], hi[i]) as f64;
            }
        }
        cost
    };
    // The leaves that stay get their units' boxes; the others go after the cost with them is taken.
    let (mut stay, mut gone) = (0, vec![]);
    for i in 0..n {
        if nodes[i].count == 0 {
            continue;
        }
        match at.get(nodes[i].left_or_first as usize).copied().filter(|&k| k != NONE) {
            Some(k) => {
                (lo[i], hi[i]) = (units[k as usize].lo, units[k as usize].hi);
                at[nodes[i].left_or_first as usize] = NONE;
                stay += 1;
            }
            None => gone.push(i),
        }
    }
    if stay != units.len() {
        return None;
    }
    let before = sweep(&mut lo, &mut hi, &kid, &alive);
    let mut root = 0usize;
    for &x in &gone {
        // (A leaf that stays is left, so `x` is never the root: it has a parent, and a sibling to take its place.)
        let p = par[x] as usize;
        (alive[x], alive[p]) = (false, false);
        let s = if kid[p][0] as usize == x { kid[p][1] } else { kid[p][0] };
        let g = par[p];
        par[s as usize] = g;
        if g == NONE {
            root = s as usize;
        } else {
            let slot = if kid[g as usize][0] as usize == p { 0 } else { 1 };
            kid[g as usize][slot] = s;
        }
    }
    let after = if gone.is_empty() { before } else { sweep(&mut lo, &mut hi, &kid, &alive) };
    let mut out = vec![FILL];
    let mut stack = vec![(root, 0usize)];
    while let Some((i, to)) = stack.pop() {
        if kid[i][0] == NONE {
            out[to] = GpuNode { bmin: lo[i].into(), left_or_first: nodes[i].left_or_first, bmax: hi[i].into(), count: 1 };
            continue;
        }
        let left = out.len();
        out.push(FILL);
        out.push(FILL);
        out[to] = GpuNode { bmin: lo[i].into(), left_or_first: left as u32, bmax: hi[i].into(), count: 0 };
        stack.push((kid[i][1] as usize, left + 1));
        stack.push((kid[i][0] as usize, left));
    }
    (out.len() == 2 * units.len() - 1).then_some((out, [before, after]))
}

/// One top tree of two: a root over `a`'s and `b`'s, then `a`'s other nodes, then `b`'s. A level deeper than the
/// deeper of the two.
pub fn join_tops(a: &[GpuNode], b: &[GpuNode]) -> Vec<GpuNode> {
    let hold = |x: &GpuNode, y: &GpuNode| (Vec3A::from(x.bmin).min(y.bmin.into()), Vec3A::from(x.bmax).max(y.bmax.into()));
    let (lo, hi) = hold(&a[0], &b[0]);
    let mut out = Vec::with_capacity(a.len() + b.len() + 1);
    out.push(GpuNode { bmin: lo.into(), left_or_first: 1, bmax: hi.into(), count: 0 });
    // Where a tree's nodes after its root go: `a`'s from 3, `b`'s after them. A child index counts from the top
    // tree's root, and moves by as much as the node it names.
    let moved = |q: &GpuNode, by: u32| if q.count > 0 { *q } else { GpuNode { left_or_first: q.left_or_first + by, ..*q } };
    let (by_a, by_b) = (2, a.len() as u32 + 1);
    out.push(moved(&a[0], by_a));
    out.push(moved(&b[0], by_b));
    out.extend(a[1..].iter().map(|q| moved(q, by_a)));
    out.extend(b[1..].iter().map(|q| moved(q, by_b)));
    out
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

/// A unit as the top tree sees it: where it places its mesh, and the tree its record names (its mesh, and the root
/// the record was last written with: `Units::top` refuses a unit whose record names a tree that is no longer there).
/// Its box in the top tree is its tree's bounds under `world`, taken when the top tree is built: a mesh written
/// again where it was (a geometry that moves every frame) has a new tree, and its units the new bounds with it.
/// Hidden: it keeps its place (an instanced object's instance that is not drawn now) and is in no top tree.
#[derive(Clone, Copy, Debug)]
pub struct Unit {
    pub world: Mat4,
    pub hidden: bool,
    pub mesh: u32,
    pub root: u32,
}

/// A mesh's tree in the node buffer: its root, its node count, the bounds of its triangles (model space).
#[derive(Clone, Copy, Debug)]
pub struct Tree {
    pub root: u32,
    pub len: u32,
    pub lo: Vec3A,
    pub hi: Vec3A,
}

/// Software traversal's units of a scene (what the TLAS and the mesh BLASes are to hardware traversal): where the
/// trees are, each instanced mesh's tree, and the unit at each kernel record.
#[derive(Clone, Debug)]
pub struct Units {
    pub layout: Layout,
    /// By instanced mesh (the scene's `meshes` index): its tree. None: no tree (a mesh that deforms or has no
    /// triangle, or one an edit freed).
    pub trees: Vec<Option<Tree>>,
    /// By kernel record: the unit there, if the record is one's.
    pub slots: Vec<Option<Unit>>,
    /// The units in the top tree as last built (`top`): the kernel's `P.motion.w`.
    pub count: u32,
    /// The records the kernel's instance table holds (the scene's and its room's): no unit is past them.
    pub records: usize,
    /// The top tree as `top` last made it, and how it came to be.
    pub kept: Kept,
}

/// The top tree `Units::top` last returned, and how it came to be. The units are in two sets. The settled, which are
/// where they were when a tree was last built over all the units (or have moved less than `ASTRAY` from there):
/// their tree is that build's, kept, its boxes brought up to date, leaves taken out as units go. And the loose: units
/// that have come since, or gone astray (pieces an explosion throws, bodies that travel): a tree of their own, built
/// again whenever one of them comes, goes or goes astray, which costs by their number and not by the scene's. With
/// loose units the top tree is a root over the two (`join_tops`); without, the settled tree alone.
///
/// Why not one tree with the new units put into it a leaf at a time: tried (greedy descent by least added area, the
/// depth held, equals balanced), and after a few thousand pieces had flown, the tree cost rays through the piles
/// several times a built tree's (`units_cost.rs` `units_pile`: 16,000 pieces, 204 k bytes a pixel against 22 k),
/// while by the area of its nodes it looked 10 % worse. Two trees that are each built are each good, and a ray that
/// passes both walks both: what it costs is measured there too.
#[derive(Clone, Debug, Default)]
pub struct Kept {
    /// The top tree as written.
    pub nodes: Vec<GpuNode>,
    /// With loose units: the settled units' tree by itself (`nodes` holds it from node 1). Without: empty, and
    /// `nodes` is it.
    settled: Vec<GpuNode>,
    /// By record: 0 not in the tree, 1 settled, 2 loose.
    part: Vec<u8>,
    loose: usize,
    /// By record: the box the unit had when it was placed (the build over all, or the loose tree's last build).
    placed: Vec<(Vec3A, Vec3A)>,
    /// By record, for a loose unit: how many builds of the loose tree in a row it has not gone astray since.
    calm: Vec<u8>,
    /// What the settled tree cost when it was built (`top_cost`), less the share of the leaves taken out since.
    pub built_cost: f64,
    /// How many times a tree was built over all the units; how many times the loose units' tree was built (the
    /// settled one kept); how many times the tree was kept as it was, its boxes brought up to date; the ms all of
    /// them took (the frame log's `top`).
    pub builds: u64,
    pub changes: u64,
    pub refits: u64,
    pub ms: f64,
}

impl Units {
    /// Mesh `k`'s tree's root.
    pub fn root(&self, k: u32) -> Option<u32> {
        self.trees.get(k as usize).copied().flatten().map(|t| t.root)
    }

    /// A unit of mesh `k` placed by `world`, whose record names the tree mesh `k` has now. None: the mesh has no
    /// tree (it is no unit's).
    pub fn unit(&self, k: u32, world: Mat4, hidden: bool) -> Option<Unit> {
        self.root(k).map(|root| Unit { world, hidden, mesh: k, root })
    }

    /// The unit at `record` moved to `world` (with `hidden`: shown or not), its record written again: it names its
    /// mesh's tree as it is now.
    pub fn moved(&mut self, record: usize, world: Mat4, hidden: Option<bool>) {
        let Some(mut u) = self.slots.get(record).copied().flatten() else { return };
        u.world = world;
        u.hidden = hidden.unwrap_or(u.hidden);
        u.root = self.root(u.mesh).unwrap_or(u.root);
        self.slots[record] = Some(u);
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
        self.trees[k as usize] = Some(Tree { root, len: tree.nodes.len() as u32, lo: tree.lo, hi: tree.hi });
        Some(root)
    }

    /// Mesh `k`'s tree given back: its root and length (the caller fills them with `FILL`), if it had one.
    pub fn drop_tree(&mut self, k: u32) -> Option<(u32, u32)> {
        let t = self.trees.get_mut(k as usize)?.take()?;
        self.layout.release(t.root, t.len as usize);
        Some((t.root, t.len))
    }

    /// The top tree over the units that are shown, for the top range's start, and their count kept (`count`). No
    /// unit: no nodes (the kernel does not walk a top tree then). The tree it made last is kept as far as it is
    /// still good (`Kept::top`): whole, its boxes brought up to date, while units only move a little; its settled
    /// part, with a tree built over the units that have come or travelled; else it is built again. Err: more units than the
    /// range holds, a unit whose box is not finite, one past the kernel's records, or one whose record names a tree
    /// that is not its mesh's now (freed, or placed elsewhere since the record was written: the kernel would walk
    /// whatever is there); nothing is changed then.
    pub fn top(&mut self) -> Result<Vec<GpuNode>, String> {
        let t = web_time::Instant::now();
        let mut boxes: Vec<UnitBox> = vec![];
        for (r, u) in self.slots.iter().enumerate() {
            let Some(u) = u.as_ref().filter(|u| !u.hidden) else { continue };
            if r >= self.records {
                return Err(format!("the unit at record {r} is past the kernel's {} records", self.records));
            }
            match self.trees.get(u.mesh as usize).copied().flatten() {
                Some(t) if t.root == u.root => boxes.push(UnitBox::new(r as u32, u.world, t.lo, t.hi)),
                _ => return Err(format!("the unit at record {r} names a tree at node {} that is not mesh {}'s now", u.root, u.mesh)),
            }
        }
        if boxes.len() > self.layout.max_units() {
            return Err(format!("{} units: more than the top tree's range holds ({})", boxes.len(), self.layout.max_units()));
        }
        let slots = self.slots.len();
        let nodes = if boxes.is_empty() { vec![] } else { self.kept.top(&boxes, slots)? };
        self.count = boxes.len() as u32;
        self.kept.nodes = nodes.clone();
        self.kept.ms += t.elapsed().as_secs_f64() * 1e3;
        Ok(nodes)
    }
}

impl Kept {
    /// The top tree over `boxes` (at least one; their records below `records`), by the least work that leaves it
    /// good: kept, its boxes brought up to date (no unit has come, gone or gone astray); the settled tree kept and
    /// the loose units' built; or one tree built over all (the first; a small scene's; a settled tree that costs
    /// more than `REFIT_SLACK` times what it may; loose units at rest that are many: `FOLD`). Whichever it is was
    /// checked (`check_top`) before anything here was changed for it.
    pub fn top(&mut self, boxes: &[UnitBox], records: usize) -> Result<Vec<GpuNode>, String> {
        if let Some(nodes) = self.keep(boxes, records) {
            return Ok(nodes);
        }
        let built = top_tree(boxes)?;
        self.built_cost = top_cost(&built);
        (self.settled, self.loose) = (vec![], 0);
        self.part.clear();
        self.place(boxes, 1);
        self.builds += 1;
        Ok(built)
    }

    fn place(&mut self, units: &[UnitBox], part: u8) {
        for u in units {
            let r = u.record as usize;
            if self.part.len() <= r {
                self.part.resize(r + 1, 0);
                self.placed.resize(r + 1, (Vec3A::ZERO, Vec3A::ZERO));
                self.calm.resize(r + 1, 0);
            }
            (self.part[r], self.placed[r]) = (part, (u.lo, u.hi));
        }
    }

    /// Whether a unit has gone from where the tree has it: its box is no longer within the box it was placed with,
    /// widened by that box's longest side (`ASTRAY`). A body that turns where it is, or has moved less than its own
    /// size, has not; one that flies has, every few frames. Left where it is in the tree, its leaf is among its old
    /// neighbours, and its box, brought up to date, stretches every node above it to where it is now: a tree that
    /// costs little more by the area of its nodes (what `REFIT_SLACK` sees) and many times more to the rays that
    /// pass the place (a thousand pieces thrown from one point, kept as they were built: 3.6 times the bytes a pixel).
    fn astray(&self, u: &UnitBox) -> bool {
        let (lo, hi) = self.placed[u.record as usize];
        let by = Vec3A::splat((hi - lo).max_element() * ASTRAY);
        u.lo.cmplt(lo - by).any() || u.hi.cmpgt(hi + by).any()
    }

    /// The kept tree made `boxes`', or None: build one over all (nothing is changed then).
    fn keep(&mut self, boxes: &[UnitBox], records: usize) -> Option<Vec<GpuNode>> {
        if self.nodes.is_empty() || !boxes.iter().all(|u| u.lo.is_finite() && u.hi.is_finite() && (u.record as usize) < records) {
            return None;
        }
        let mut is = vec![false; records];
        for u in boxes {
            is[u.record as usize] = true;
        }
        let checked = |t: Vec<GpuNode>| check_top(&t, |r| is.get(r as usize).copied().unwrap_or(false)).is_ok().then_some(t);
        // Each unit's part now: settled if it was and has not gone astray, else loose. (`stirred`: the loose tree is
        // no longer its units': one came, or went astray. `resting`: loose units that have long kept still.)
        let (mut settled, mut loose): (Vec<UnitBox>, Vec<UnitBox>) = (vec![], vec![]);
        let (mut stirred, mut resting) = (false, 0);
        let mut strayed = vec![];
        for u in boxes {
            match self.part.get(u.record as usize).copied().unwrap_or(0) {
                1 if !self.astray(u) => settled.push(*u),
                2 => {
                    let astray = self.astray(u);
                    stirred |= astray;
                    resting += (!astray && self.calm[u.record as usize] >= QUIET) as usize;
                    strayed.push(astray);
                    loose.push(*u);
                }
                _ => {
                    stirred = true;
                    strayed.push(true);
                    loose.push(*u);
                }
            }
        }
        let was_settled = if self.loose > 0 { self.settled.len() } else { self.nodes.len() }.div_ceil(2);
        if !stirred && settled.len() == was_settled && loose.len() == self.loose {
            // The tree as it is. (With loose units the settled tree's cost is its own nodes': node 1, and those from
            // 3 to where the loose tree's begin. The settled tree kept by itself gets its boxes when it is next used.)
            let nodes = checked(refit_top(&self.nodes, boxes)?)?;
            let cost = if self.loose > 0 { top_cost(&nodes[1..2]) + top_cost(&nodes[3..self.settled.len() + 2]) } else { top_cost(&nodes) };
            if cost > self.built_cost * REFIT_SLACK {
                return None;
            }
            self.refits += 1;
            return Some(nodes);
        }
        if boxes.len() <= SMALL || settled.is_empty() || resting as f64 > boxes.len() as f64 * FOLD {
            return None;
        }
        // The settled tree without the leaves that went, as good as it was; the loose units' tree, built.
        let (kept, [before, after]) = thin_top(if self.loose > 0 { &self.settled } else { &self.nodes }, &settled)?;
        if before > self.built_cost * REFIT_SLACK {
            return None;
        }
        let nodes = checked(if loose.is_empty() { kept.clone() } else { join_tops(&kept, &top_tree(&loose).ok()?) })?;
        for part in self.part.iter_mut() {
            *part = 0;
        }
        for u in &settled {
            self.part[u.record as usize] = 1;
        }
        self.place(&loose, 2);
        for (u, astray) in loose.iter().zip(strayed) {
            let calm = &mut self.calm[u.record as usize];
            *calm = if astray { 0 } else { calm.saturating_add(1) };
        }
        self.built_cost *= if before > 0.0 { after / before } else { 1.0 };
        (self.settled, self.loose) = (if loose.is_empty() { vec![] } else { kept }, loose.len());
        self.changes += 1;
        Some(nodes)
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
        /// `P.motion.z`, `P.motion.w`.
        pub(crate) top: u32,
        pub(crate) units: u32,
        pub(crate) steps: std::cell::Cell<u64>,
        pub(crate) budget: u64,
        /// The deepest the shared stack got.
        pub(crate) deepest: std::cell::Cell<usize>,
        /// What the walks read, counted (units_cost.rs).
        pub(crate) reads: std::cell::Cell<Reads>,
    }

    /// A walk's reads by kind: nodes of the still tree, of the top tree and of mesh trees, vertices (three a triangle
    /// test) and unit records; `still_steps`: the loop iterations of the still scene's walk (the rest of `steps` are
    /// the unit walk's). `in_units`: the still walk is over.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub(crate) struct Reads {
        pub(crate) still_nodes: u64,
        pub(crate) top_nodes: u64,
        pub(crate) mesh_nodes: u64,
        pub(crate) still_verts: u64,
        pub(crate) mesh_verts: u64,
        pub(crate) records: u64,
        pub(crate) still_steps: u64,
        pub(crate) in_units: bool,
    }

    pub(crate) type Walk<T> = Result<T, String>;

    impl Kernel<'_> {
        fn count(&self, f: impl Fn(&mut Reads)) {
            let mut r = self.reads.get();
            f(&mut r);
            self.reads.set(r);
        }
        /// The still walk is over. (The top tree's nodes are the buffer's last: a node at `top` or after is the top
        /// tree's, one before it a mesh tree's.)
        fn still_done(&self) {
            self.count(|r| r.in_units = true);
        }
        pub(crate) fn node(&self, i: u32) -> Walk<GpuNode> {
            self.count(|r| *(if !r.in_units { &mut r.still_nodes } else if i >= self.top { &mut r.top_nodes } else { &mut r.mesh_nodes }) += 1);
            self.nodes.get(i as usize).copied().ok_or(format!("reads node {i} of {}", self.nodes.len()))
        }
        pub(crate) fn vert(&self, i: u32) -> Walk<Vec3A> {
            self.count(|r| *(if r.in_units { &mut r.mesh_verts } else { &mut r.still_verts }) += 1);
            self.tri_verts.get(i as usize).map(|v| Vec3A::new(v[0], v[1], v[2])).ok_or(format!("reads vertex {i} of {}", self.tri_verts.len()))
        }
        pub(crate) fn record(&self, i: u32) -> Walk<GpuInstance> {
            self.count(|r| r.records += 1);
            self.instances.get(i as usize).copied().ok_or(format!("reads record {i} of {}", self.instances.len()))
        }
        pub(crate) fn step(&self) -> Walk<()> {
            self.steps.set(self.steps.get() + 1);
            self.count(|r| r.still_steps += !r.in_units as u64);
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
            // (The stack is the module's in a kernel with units, 64 long, the still walk's own bound kept; in one
            // without, the walk's own 48.)
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
                if tr != INF && sp < STILL_STACK {
                    stack[sp] = far;
                    sp += 1;
                }
            }
            self.still_done();
            self.trace_units(o, d, inv_d, hit, mode, &mut stack)
        }

        /// shaders/trace_units.wgsl `trace_units`, statement for statement.
        pub(crate) fn trace_units(&self, o: Vec3A, d: Vec3A, inv_d: Vec3A, hit_in: Hit, mode: u32, stack: &mut [u32; STACK]) -> Walk<Hit> {
            let mut hit = hit_in;
            if self.units == 0 {
                return Ok(hit);
            }
            let top = self.top;
            let whole = self.node(top)?;
            if Self::ray_aabb(o, inv_d, whole.bmin, whole.bmax, hit.t) == INF {
                return Ok(hit);
            }
            let (mut sp, mut node) = (0usize, top);
            let mut rec = NO_INST;
            let (mut ro, mut rd, mut rinv) = (o, d, inv_d);
            let (mut off, mut first, mut base) = (top, 0u32, 0usize);
            loop {
                self.step()?;
                let n = self.node(node)?;
                if n.count == 0 {
                    let l = off + n.left_or_first;
                    let r = l + 1;
                    let (nl, nr) = (self.node(l)?, self.node(r)?);
                    let mut tl = Self::ray_aabb(ro, rinv, nl.bmin, nl.bmax, hit.t);
                    let mut tr = Self::ray_aabb(ro, rinv, nr.bmin, nr.bmax, hit.t);
                    if tl != INF || tr != INF {
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
                        continue;
                    }
                } else if rec == NO_INST {
                    let u = self.record(n.left_or_first)?;
                    if mode != TRACE_SHADOW || u.n[1][3].to_bits() != 0 {
                        rec = n.left_or_first;
                        ro = o - Vec3A::new(u.m[0][3], u.m[1][3], u.m[2][3]);
                        if u.n[2][3].to_bits() == 0 {
                            ro = Self::proc_dir(&u, ro);
                            rd = Self::proc_dir(&u, d);
                            rinv = Self::inv(rd);
                        }
                        off = u.n[0][3].to_bits();
                        first = u.info[0];
                        base = sp;
                        node = off;
                        continue;
                    }
                } else {
                    for k in 0..n.count {
                        self.step()?;
                        let ti = first + n.left_or_first + k;
                        let x = self.intersect_tri(ro, rd, ti, hit.t)?;
                        if x < hit.t {
                            hit = Hit { t: x, tri: ti, inst: rec };
                            if mode != TRACE_CLOSEST {
                                return Ok(hit);
                            }
                        }
                    }
                }
                if rec != NO_INST && sp == base {
                    rec = NO_INST;
                    ro = o;
                    rd = d;
                    rinv = inv_d;
                    off = top;
                }
                if sp == 0 {
                    break;
                }
                sp -= 1;
                node = stack[sp];
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
                "var<private> stack: array<u32, 64>;",
                "fn trace_units(o: vec3<f32>, d: vec3<f32>, inv_d: vec3<f32>, hit_in: Hit, mode: u32) -> Hit {",
                "if (bitcast<u32>(P.motion.w) == 0u) { return hit; }",
                "let top = bitcast<u32>(P.motion.z);",
                "let whole = nodes[top];",
                "if (ray_aabb(o, inv_d, whole.bmin, whole.bmax, hit.t) == INF) { return hit; }",
                "var sp = 0u;",
                "var node = top;",
                "var rec = NO_INST;",
                "var ro = o;",
                "var rd = d;",
                "var rinv = inv_d;",
                "var off = top;",
                "var first = 0u;",
                "var base = 0u;",
                "let n = nodes[node];",
                "if (n.count == 0u) {",
                "let l = off + n.left_or_first;",
                "let r = l + 1u;",
                "var tl = ray_aabb(ro, rinv, nl.bmin, nl.bmax, hit.t);",
                "var tr = ray_aabb(ro, rinv, nr.bmin, nr.bmax, hit.t);",
                "if (tl != INF || tr != INF) {",
                "if (tr < tl) {",
                "node = near;",
                "if (tr != INF && sp < 64u) {",
                "stack[sp] = far;",
                "continue;",
                "} else if (rec == NO_INST) {",
                "let u = instances[n.left_or_first];",
                "if (mode != TRACE_SHADOW || bitcast<u32>(u.n1.w) != 0u) {",
                "rec = n.left_or_first;",
                "ro = o - vec3<f32>(u.m0.w, u.m1.w, u.m2.w);",
                "if (bitcast<u32>(u.n2.w) == 0u) {",
                "ro = proc_dir(u, ro);",
                "rd = proc_dir(u, d);",
                "rinv = 1.0 / select(rd, vec3<f32>(1e-20), abs(rd) < vec3<f32>(1e-20));",
                "off = bitcast<u32>(u.n0.w);",
                "first = u.info.x;",
                "base = sp;",
                "node = off;",
                "continue;",
                "} else {",
                "for (var k = 0u; k < n.count; k++) {",
                "let ti = first + n.left_or_first + k;",
                "let x = intersect_tri(ro, rd, ti, hit.t);",
                "if (x.x < hit.t) {",
                "hit = Hit(x.x, ti, x.y, x.z, rec);",
                "if (mode != TRACE_CLOSEST) { return hit; }",
                "if (rec != NO_INST && sp == base) {",
                "rec = NO_INST;",
                "ro = o;",
                "rd = d;",
                "rinv = inv_d;",
                "off = top;",
                "if (sp == 0u) { break; }",
                "node = stack[sp];",
                "return hit;",
            ],
        );
        assert_eq!(units.matches("loop {").count(), 1, "one loop for both levels");
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
                "var stack: array<u32, 48>;",
                "if (mode == TRACE_SHADOW && (flags & TRI_NO_SHADOW) != 0u) { continue; }",
                "let r = intersect_tri(o, d, ti, hit.t);",
                "hit = Hit(r.x, ti, r.y, r.z, NO_INST);",
                "if (mode != TRACE_CLOSEST) { return hit; }",
                "if (tr != INF && sp < 48u) {",
                "if (has(F_PROC)) { return trace_procs(o, d, hit, mode); }",
            ],
        );
        // What the stack and the record's words are here is what they are there.
        assert_eq!((STACK, STILL_STACK), (64, 48));
        assert!(TOP_DEPTH + MESH_DEPTH <= STACK && STILL_STACK <= STACK, "one stack holds each walk");
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
    /// the bookkeeping that writes them. Its operations are what an edit does (live/edits.rs): a mesh's triangles and tree
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
        /// The top tree as last written, and how it came to be (`Units::top`'s rule: `Kept::top`).
        kept: Kept,
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
            Scene { nodes, tri_verts, instances: vec![GpuInstance::default()], top: layout.top_first, layout, still_tris: still.len(), meshes: vec![], units: vec![], count: 0, kept: Kept::default() }
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

        /// The top tree again, over the units there are now, into its range: kept, changed or built, by
        /// `Units::top`'s rule.
        fn retop(&mut self) -> Result<(), String> {
            let boxes: Vec<UnitBox> = self.units.iter().enumerate().filter_map(|(k, u)| u.map(|(m, w, _)| (k, m, w))).map(|(k, m, w)| { let (.., lo, hi) = self.meshes[m].unwrap(); UnitBox::new(k as u32, w, lo, hi) }).collect();
            self.count = boxes.len() as u32;
            if boxes.is_empty() {
                return Ok(());
            }
            if boxes.len() > self.layout.max_units() {
                return Err("no room for the top tree".into());
            }
            let t = self.kept.top(&boxes, self.units.len())?;
            check_top(&t, |r| self.units.get(r as usize).is_some_and(|u| u.is_some()))?;
            let at = self.layout.top_first as usize;
            self.nodes[at..at + t.len()].copy_from_slice(&t);
            self.kept.nodes = t;
            Ok(())
        }

        fn kernel(&self) -> Kernel<'_> {
            let budget = 64 * (self.nodes.len() as u64 + self.tri_verts.len() as u64) + 1024;
            Kernel { nodes: &self.nodes, tri_verts: &self.tri_verts, instances: &self.instances, top: self.top, units: self.count, steps: Default::default(), budget, deepest: Default::default(), reads: Default::default() }
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

    /// A placement: a third of them a translation alone (the walk's short way), a few none at all.
    fn pose(rng: &mut Rng) -> Mat4 {
        let kind = rng.f();
        if kind < 0.05 {
            return Mat4::IDENTITY;
        }
        if kind < 0.35 {
            return Mat4::from_translation(Vec3::from(rng.v(30.0)));
        }
        let s = Vec3::new(0.3 + 2.0 * rng.f(), 0.3 + 2.0 * rng.f(), 0.3 + 2.0 * rng.f());
        let q = Quat::from_euler(glam::EulerRot::XYZ, rng.f() * 6.0, rng.f() * 6.0, rng.f() * 6.0);
        Mat4::from_scale_rotation_translation(s, q, Vec3::from(rng.v(30.0)))
    }

    /// Random scenes under random edits: meshes added and removed, units added, moved (far, or a little: the top tree
    /// is then kept and refitted) and removed, the top tree made again after each, as an edit does. After every edit the kernel's two walks, on the CPU with every read checked,
    /// agree with brute force. Scenes with and without a still part; units that overlap; a scene emptied of units.
    #[test]
    fn the_walks_agree_with_brute_force_under_random_edits() {
        let (mut edits, mut unit_hits, mut rays, mut refits, mut changes, mut builds, mut short) = (0, 0, 0, 0, 0, 0, 0);
        for seed in 0..12u64 {
            let mut rng = Rng(seed * 7919 + 1);
            let n = 50 + rng.below(400);
            let still = if seed % 3 == 0 { vec![] } else { rng.tris(n, 40.0, 3.0) };
            // (Every other scene starts with more units than a small scene's: there the top tree is kept in parts.)
            let mut sc = Scene::new(&still, 6000, if seed % 2 == 0 { 64 } else { 400 });
            sc.check(&mut rng, 40, "no unit yet");
            if seed % 2 == 1 {
                let m = sc.add_mesh(&rng.tris(12, 3.0, 1.0)).unwrap();
                for _ in 0..150 {
                    sc.add_unit(m, pose(&mut rng), true);
                }
                sc.retop().unwrap();
                sc.check(&mut rng, 40, "a scene of many units");
            }
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
                        let (m, was, cast) = sc.units[k].unwrap();
                        // Somewhere else, or (half of them) a step from where it was, turned a little or not.
                        let world = match rng.below(4) {
                            0 => Mat4::from_translation(Vec3::from(rng.v(0.5))) * was,
                            1 => was * Mat4::from_rotation_y(rng.f() * 0.2),
                            _ => pose(&mut rng),
                        };
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
            refits += sc.kept.refits;
            changes += sc.kept.changes;
            builds += sc.kept.builds;
            short += sc.units.iter().enumerate().filter(|(k, u)| u.is_some() && sc.instances[*k].n[2][3].to_bits() == 1).count();
        }
        assert!(edits > 300 && unit_hits * 8 > rays, "{edits} edits, {unit_hits} of {rays} closest hits on units");
        assert!(refits > 40 && changes > 100 && builds > 100 && short > 20, "{refits} top trees kept, {changes} with the loose units' built, {builds} built over all; {short} units placed by a translation alone at the end");
    }

    /// The trees the builders make are the trees the checks accept, at their depth bounds: a top tree of a thousand
    /// units, a mesh of triangles all in one place (no split by position: median splits), a long strip.
    #[test]
    fn the_builders_keep_their_depths() {
        let mut rng = Rng(5);
        let units: Vec<UnitBox> = (0..1000).map(|k| UnitBox::new(k, Mat4::from_translation(Vec3::from(rng.v(100.0))), Vec3A::splat(-0.25), Vec3A::splat(0.25))).collect();
        let top = top_tree(&units).unwrap();
        assert_eq!(top.len(), 1999);
        let depth = check_top(&top, |r| r < 1000).unwrap();
        assert!((10..=TOP_DEPTH).contains(&depth), "1000 units: {depth}");
        // By surface area, not by count: cheaper for a ray than the tree of median splits over the same units, of one
        // size or not (every tenth here a wall among lamps). (13 % and 15 % here; the voxel level's 10,459 units: 24 %,
        // units_cost.rs.)
        let boxes: Vec<(Vec3A, Vec3A)> = units.iter().map(|u| (u.lo, u.hi)).collect();
        let (sah, median) = (top_cost(&top), top_cost(&build_median(&boxes, 1).0));
        assert!(sah < 0.95 * median, "{sah} against {median}");
        let mixed: Vec<(Vec3A, Vec3A)> = boxes.iter().enumerate().map(|(k, b)| if k % 10 == 0 { (b.0 - Vec3A::new(12.0, 6.0, 0.0), b.1 + Vec3A::new(12.0, 6.0, 0.0)) } else { *b }).collect();
        let (sah, median) = (top_cost(&build_sah(&mixed, TOP_DEPTH).0), top_cost(&build_median(&mixed, 1).0));
        assert!(sah < 0.9 * median, "{sah} against {median}");
        // The depth holds whatever the boxes are: as many as the depth allows (every split must halve), one more level
        // than that, plenty; boxes all in one place, in a row each further than all before it together (the cheapest
        // split by area peels one off, a chain), in two clumps of unequal size, anywhere.
        for round in 0..400 {
            let n = 1 + rng.below(300);
            let least = (n as f32).log2().ceil() as usize;
            let max_depth = least + [0, 0, 1, 2, 40][round % 5];
            let boxes: Vec<(Vec3A, Vec3A)> = (0..n)
                .map(|k| {
                    let c = match round % 4 {
                        0 => Vec3A::ZERO,
                        1 => Vec3A::new(1.5f32.powi(k as i32 % 60) * (1 + k / 60) as f32, 0.0, 0.0),
                        2 => rng.v(1.0) + if k % 7 == 0 { Vec3A::splat(500.0) } else { Vec3A::ZERO },
                        _ => rng.v(50.0),
                    };
                    let h = Vec3A::splat(0.1 + rng.f() * if k % 9 == 0 { 20.0 } else { 1.0 });
                    (c - h, c + h)
                })
                .collect();
            let (nodes, order) = build_sah(&boxes, max_depth);
            assert_eq!(nodes.len(), 2 * n - 1);
            let depth = check_tree(&nodes, max_depth, |_, count| if count == 1 { Ok(()) } else { Err("a leaf of several".into()) }).unwrap_or_else(|e| panic!("round {round}, {n} boxes, depth {max_depth}: {e}"));
            assert!(depth >= least);
            let mut seen = order.clone();
            seen.sort_unstable();
            assert_eq!(seen, (0..n as u32).collect::<Vec<_>>());
            // Every leaf's box is its box, every interior node's holds its children's.
            for (i, node) in nodes.iter().enumerate() {
                if node.count > 0 {
                    let b = boxes[order[node.left_or_first as usize] as usize];
                    assert_eq!((Vec3A::from(node.bmin), Vec3A::from(node.bmax)), b);
                } else {
                    let (a, b) = (nodes[node.left_or_first as usize], nodes[node.left_or_first as usize + 1]);
                    assert_eq!((Vec3A::from(node.bmin), Vec3A::from(node.bmax)), (Vec3A::from(a.bmin).min(b.bmin.into()), Vec3A::from(a.bmax).max(b.bmax.into())), "node {i}");
                }
            }
        }
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

    /// `Units` (what live/edits.rs's edits keep): a unit names the tree its mesh has when its record is written; a top tree
    /// is refused, and nothing changed, while a shown unit names a tree that was freed or placed again elsewhere, or
    /// sits past the kernel's records; writing the record again (`moved`), hiding it or clearing it mends that.
    #[test]
    fn the_units_refuse_a_record_that_names_a_tree_that_is_gone() {
        let mut rng = Rng(11);
        let (small, large) = (mesh_tree(&rng.tris(40, 2.0, 0.3)).unwrap(), mesh_tree(&rng.tris(400, 2.0, 0.3)).unwrap());
        let (ns, nl) = (small.nodes.len(), large.nodes.len());
        assert!(nl > ns);
        let mut u = Units { layout: Layout::new(10, ns + nl, 4), trees: vec![], slots: vec![], count: 0, records: 4, kept: Default::default() };
        // A mesh without a tree is no unit's.
        assert!(u.unit(0, Mat4::IDENTITY, false).is_none());
        assert_eq!(u.add_tree(0, &small), Some(10));
        assert_eq!(u.add_tree(1, &large), Some(10 + ns as u32));
        assert_eq!(u.layout.free_nodes(), 0);
        let (a, b) = (u.unit(0, Mat4::IDENTITY, false).unwrap(), u.unit(1, Mat4::from_translation(glam::Vec3::X * 5.0), false).unwrap());
        assert_eq!((a.root, b.root), (10, 10 + ns as u32));
        u.set(0, Some(a));
        u.set(2, Some(b));
        // (Clearing a record past the slots makes none.)
        u.set(9, None);
        assert_eq!(u.slots.len(), 3);
        let top = u.top().unwrap();
        assert_eq!((top.len(), u.count), (3, 2));
        check_top(&top, |r| u.slots.get(r as usize).is_some_and(|s| s.is_some())).unwrap();

        // Mesh 0's tree is freed: its unit's record still names node 10.
        assert_eq!(u.drop_tree(0), Some((10, ns as u32)));
        assert_eq!(u.drop_tree(0), None);
        let e = u.top().unwrap_err();
        assert!(e.contains("record 0") && e.contains("mesh 0"), "{e}");
        assert_eq!(u.count, 2, "a refused top tree changes nothing");
        // Hidden, it is in no top tree and nothing reads its record's root.
        u.moved(0, Mat4::IDENTITY, Some(true));
        assert_eq!((u.top().unwrap().len(), u.count), (1, 1));
        u.moved(0, Mat4::IDENTITY, Some(false));
        assert!(u.top().is_err());

        // Mesh 1's tree is made again: its own nodes are given back first, one run with mesh 0's, and the new tree
        // starts where that run does. Its unit's record names the old root until it is written again.
        u.set(0, None);
        assert_eq!(u.add_tree(1, &large), Some(10));
        assert_eq!(u.layout.free_nodes(), ns);
        assert!(u.top().is_err());
        assert_eq!(u.add_tree(1, &small), Some(10));
        let e = u.top().unwrap_err();
        assert!(e.contains("record 2") && e.contains("mesh 1"), "{e}");
        u.moved(2, Mat4::from_translation(glam::Vec3::Y), None);
        assert_eq!(u.slots[2].unwrap().root, 10);
        assert_eq!((u.top().unwrap().len(), u.count), (1, 1));
        assert_eq!(u.layout.free_nodes(), nl);

        // A unit past the kernel's records is refused; hidden or cleared, it is not there.
        u.set(4, u.unit(1, Mat4::IDENTITY, false));
        let e = u.top().unwrap_err();
        assert!(e.contains("record 4") && e.contains("4 records"), "{e}");
        u.set(4, None);
        // More units than the top tree's range holds.
        u.records = 16;
        for r in 3..8 {
            u.set(r, u.unit(1, Mat4::from_translation(glam::Vec3::Z * r as f32), false));
        }
        assert!(u.top().unwrap_err().contains("more than the top tree's range holds"));
        assert_eq!(u.count, 1);
        // No unit shown: no nodes, a count of 0 (the kernel walks no top tree).
        for r in 0..8 {
            u.set(r, None);
        }
        assert_eq!((u.top().unwrap().len(), u.count), (0, 0));
    }

    /// A mesh written again where it was (a geometry that moves every frame: live/edits.rs writes a mesh of the same
    /// triangle count over the one it replaces) has a new tree, and its unit's box in the top tree is the new tree's:
    /// the walk must reach triangles that left the old bounds. (Seen on the GPU, voxel/cut-falls-live: a part that
    /// fell 0.2 m was cut off at the box it had before.)
    #[test]
    fn a_unit_of_a_mesh_written_again_has_its_new_bounds() {
        let tri = |y: f32| [[Vec3A::new(0.0, y, 0.0), Vec3A::new(1.0, y, 0.0), Vec3A::new(0.0, y + 1.0, 0.0)]];
        let (first, fallen) = (mesh_tree(&tri(2.0)).unwrap(), mesh_tree(&tri(-3.0)).unwrap());
        assert_eq!((first.lo.y, first.hi.y, fallen.lo.y, fallen.hi.y), (2.0, 3.0, -3.0, -2.0));
        let mut u = Units { layout: Layout::new(1, 8, 2), trees: vec![], slots: vec![], count: 0, records: 2, kept: Default::default() };
        u.add_tree(0, &first).unwrap();
        u.set(0, u.unit(0, Mat4::IDENTITY, false));
        let top = u.top().unwrap();
        assert!(top[0].bmin[1] <= 2.0 && top[0].bmax[1] >= 3.0, "{:?}", top[0]);
        // The same mesh, other triangles; its unit placed again, as the edit places it, and then only moved.
        u.add_tree(0, &fallen).unwrap();
        u.set(0, u.unit(0, Mat4::IDENTITY, false));
        let top = u.top().unwrap();
        assert!(top[0].bmin[1] <= -3.0 && top[0].bmax[1] >= -2.0 && top[0].bmax[1] < 0.0, "{:?}", top[0]);
        u.add_tree(0, &first).unwrap();
        u.moved(0, Mat4::from_translation(glam::Vec3::Y * 10.0), None);
        let top = u.top().unwrap();
        assert!(top[0].bmin[1] <= 12.0 && top[0].bmin[1] > 11.0 && top[0].bmax[1] >= 13.0, "{:?}", top[0]);
    }

    /// Units that only move keep their top tree, its boxes theirs again (each leaf's its unit's box, each node's what
    /// holds its children's), while it costs a ray no more than `REFIT_SLACK` of what it did when built; a unit that
    /// goes far, comes, goes, is hidden or shown has it built again. A kept tree is one a build could have made: the
    /// same checks pass, and a build over the same units costs no more.
    #[test]
    fn a_top_tree_is_kept_while_units_only_move() {
        let mut rng = Rng(21);
        let mesh = mesh_tree(&rng.tris(30, 1.0, 0.3)).unwrap();
        let n = 200;
        let mut u = Units { layout: Layout::new(1, mesh.nodes.len(), n + 200), trees: vec![], slots: vec![], count: 0, records: n + 200, kept: Default::default() };
        u.add_tree(0, &mesh).unwrap();
        let mut at: Vec<Vec3> = (0..n).map(|_| Vec3::from(rng.v(40.0))).collect();
        for (k, p) in at.iter().enumerate() {
            u.set(k, u.unit(0, Mat4::from_translation(*p), false));
        }
        let holds = |u: &Units, top: &[GpuNode]| {
            check_top(top, |r| u.slots.get(r as usize).is_some_and(|s| s.is_some_and(|s| !s.hidden))).unwrap();
            for node in top {
                if node.count > 0 {
                    let unit = u.slots[node.left_or_first as usize].unwrap();
                    let b = UnitBox::new(node.left_or_first, unit.world, mesh.lo, mesh.hi);
                    assert_eq!((Vec3A::from(node.bmin), Vec3A::from(node.bmax)), (b.lo, b.hi));
                } else {
                    let (a, b) = (top[node.left_or_first as usize], top[node.left_or_first as usize + 1]);
                    assert_eq!((Vec3A::from(node.bmin), Vec3A::from(node.bmax)), (Vec3A::from(a.bmin).min(b.bmin.into()), Vec3A::from(a.bmax).max(b.bmax.into())));
                }
            }
        };
        let first = u.top().unwrap();
        holds(&u, &first);
        assert_eq!((u.kept.builds, u.kept.refits), (1, 0));
        // Nothing moved: the same tree, to the byte.
        assert_eq!(bytemuck::cast_slice::<_, u8>(&u.top().unwrap()), bytemuck::cast_slice::<_, u8>(&first));
        assert_eq!((u.kept.builds, u.kept.refits), (1, 1));
        // Every unit a little a frame (bodies that settle): the tree is kept.
        for _ in 0..60 {
            for (k, p) in at.iter_mut().enumerate() {
                *p += Vec3::from(rng.v(0.02));
                u.moved(k, Mat4::from_translation(*p), None);
            }
            let top = u.top().unwrap();
            holds(&u, &top);
        }
        assert_eq!((u.kept.builds, u.kept.changes, u.kept.refits), (1, 0, 61));
        // Units close together, each grown to nearly twice its size where it is: none has gone astray, and the tree
        // costs too much: built.
        let mut d = Units { layout: Layout::new(1, mesh.nodes.len(), 100), trees: vec![], slots: vec![], count: 0, records: 100, kept: Default::default() };
        d.add_tree(0, &mesh).unwrap();
        let close: Vec<Vec3> = (0..100).map(|_| Vec3::from(rng.v(2.0))).collect();
        for (k, p) in close.iter().enumerate() {
            d.set(k, d.unit(0, Mat4::from_translation(*p), false));
        }
        d.top().unwrap();
        for (k, p) in close.iter().enumerate() {
            d.moved(k, Mat4::from_translation(*p) * Mat4::from_scale(Vec3::splat(1.9)), None);
        }
        d.top().unwrap();
        assert_eq!((d.kept.builds, d.kept.changes, d.kept.refits), (2, 0, 0));
        for (k, p) in at.iter().enumerate() {
            u.moved(k, Mat4::from_translation(*p) * Mat4::from_scale(Vec3::splat(1.5)), None);
        }
        let top = u.top().unwrap();
        holds(&u, &top);
        assert_eq!((u.kept.builds, u.kept.changes, u.kept.refits), (1, 0, 62));
        // Ten of them fly, a step of their own size a frame: they are the loose ones, their tree is built each
        // frame, the others' is kept; and the whole is near what a tree built over all would cost.
        let boxes_of = |u: &Units| u.slots.iter().enumerate().filter_map(|(r, s)| s.filter(|s| !s.hidden).map(|s| UnitBox::new(r as u32, s.world, mesh.lo, mesh.hi))).collect::<Vec<UnitBox>>();
        let mut worst: f64 = 0.0;
        for _ in 0..40 {
            for (k, p) in at.iter_mut().enumerate().take(10) {
                *p += Vec3::from(rng.v(6.0));
                u.moved(k, Mat4::from_translation(*p) * Mat4::from_scale(Vec3::splat(1.5)), None);
            }
            let top = u.top().unwrap();
            holds(&u, &top);
            worst = worst.max(top_cost(&top) / top_cost(&top_tree(&boxes_of(&u)).unwrap()));
        }
        assert_eq!((u.kept.builds, u.kept.loose, u.kept.settled.len()), (1, 10, 2 * (n - 10) - 1));
        assert!(u.kept.changes >= 30 && worst < 1.25, "{} times the loose tree; at worst {worst:.2} of a built tree's cost", u.kept.changes);
        // One of the settled far off: loose too. A unit hidden, shown (loose now), gone, another come.
        u.moved(77, Mat4::from_translation(Vec3::splat(4000.0)), None);
        let top = u.top().unwrap();
        holds(&u, &top);
        assert_eq!((u.kept.builds, u.kept.loose), (1, 11));
        u.moved(33, Mat4::IDENTITY, Some(true));
        let top = u.top().unwrap();
        holds(&u, &top);
        assert_eq!((top.len(), u.kept.builds, u.kept.loose), (2 * (n - 1) - 1, 1, 11));
        u.moved(33, Mat4::IDENTITY, Some(false));
        let top = u.top().unwrap();
        holds(&u, &top);
        assert_eq!((top.len(), u.kept.builds, u.kept.loose), (2 * n - 1, 1, 12));
        u.set(55, None);
        u.set(n, u.unit(0, Mat4::IDENTITY, false));
        let top = u.top().unwrap();
        holds(&u, &top);
        assert_eq!((top.len(), u.kept.builds, u.kept.loose), (2 * n - 1, 1, 13), "as many units, not the same ones");
        // The loose ones gone: the settled tree alone again.
        for k in (0..10).chain([33, 77, n]) {
            u.set(k, None);
        }
        let top = u.top().unwrap();
        holds(&u, &top);
        assert_eq!((top.len(), u.kept.builds, u.kept.loose, u.kept.settled.len()), (2 * (n - 13) - 1, 1, 0, 0));
        // Pieces thrown, thirty at a time, that fly a while and come to rest; once those at rest are an eighth of the
        // units, one tree is built over all, and they are settled.
        let (builds, mut next, mut folded) = (u.kept.builds, n + 1, 0);
        let mut flying: Vec<(usize, Vec3, u32)> = vec![];
        for frame in 0..200 {
            if frame % 20 == 0 && frame < 120 {
                let from = Vec3::from(rng.v(30.0));
                for _ in 0..30 {
                    flying.push((next, from, 0));
                    u.set(next, u.unit(0, Mat4::from_translation(from), false));
                    next += 1;
                }
            }
            for f in flying.iter_mut().filter(|f| f.2 < 12) {
                f.1 += Vec3::from(rng.v(3.0));
                f.2 += 1;
                u.moved(f.0, Mat4::from_translation(f.1), None);
            }
            let was = u.kept.builds;
            let top = u.top().unwrap();
            holds(&u, &top);
            assert_eq!(top.len(), 2 * boxes_of(&u).len() - 1);
            folded += (u.kept.builds > was) as usize;
            assert!(top_cost(&top) <= 1.25 * top_cost(&top_tree(&boxes_of(&u)).unwrap()), "frame {frame}");
        }
        assert!((1..=4).contains(&folded) && u.kept.builds == builds + folded as u64, "{folded} built over all in 200 frames");
        assert!(u.kept.loose <= 60, "{} loose at the end", u.kept.loose);
        // `thin_top` by itself: leaves taken out down to one; what it refuses. `join_tops`: a tree.
        let boxes = |records: &[u32]| records.iter().map(|&k| UnitBox::new(k, Mat4::from_translation(Vec3::X * k as f32 * 3.0), Vec3A::ZERO, Vec3A::ONE)).collect::<Vec<UnitBox>>();
        let eight = top_tree(&boxes(&[0, 1, 2, 3, 4, 5, 6, 7])).unwrap();
        let (five, [before, after]) = thin_top(&eight, &boxes(&[0, 2, 3, 6, 7])).unwrap();
        assert_eq!((five.len(), check_top(&five, |r| [0, 2, 3, 6, 7].contains(&r)).is_ok()), (9, true));
        assert!(before == top_cost(&eight) && after == top_cost(&five) && after < before);
        let (one, [_, after]) = thin_top(&five, &boxes(&[6])).unwrap();
        assert_eq!((one.len(), one[0].left_or_first, one[0].count, after), (1, 6, 1, 0.0));
        assert!(thin_top(&eight, &boxes(&[0, 9])).is_none(), "a unit without a leaf");
        assert!(thin_top(&eight, &boxes(&[1, 1])).is_none(), "a record twice");
        assert!(thin_top(&[], &boxes(&[1])).is_none() && thin_top(&eight, &[]).is_none());
        let mut broken = eight.clone();
        broken[0].left_or_first = 0;
        assert!(thin_top(&broken, &boxes(&[0, 1])).is_none());
        let mut shared = eight.clone();
        let inner: Vec<usize> = (1..shared.len()).filter(|&i| shared[i].count == 0).collect();
        shared[inner[1]].left_or_first = shared[inner[0]].left_or_first.max(inner[1] as u32 + 1);
        assert!(thin_top(&shared, &boxes(&[0, 1])).is_none(), "two nodes with the same children");
        let joined = join_tops(&five, &top_tree(&boxes(&[10, 11, 12])).unwrap());
        assert_eq!((joined.len(), check_top(&joined, |r| [0, 2, 3, 6, 7, 10, 11, 12].contains(&r)).unwrap()), (15, 1 + check_top(&five, |_| true).unwrap()));
        let joined = join_tops(&one, &one);
        assert_eq!((joined.len(), joined[0].left_or_first, joined[1].count, joined[2].count), (3, 1, 1, 1));
        // A built tree is a level short of the bound: two of them joined are at it.
        assert!(check_tree(&build_sah(&(0..1 << 12).map(|k| (Vec3A::splat(k as f32), Vec3A::splat(k as f32 + 1.0))).collect::<Vec<_>>(), 12).0, 64, |_, _| Ok(())).unwrap() <= 12);
        // The record's word for a placement by a translation alone.
        let rec = |world: Mat4| unit_record(crate::scene::Instance { mesh: 0, world, object: 0, cast_shadow: true, name: String::new(), sub: None, hidden: false }.gpu(0, 0), 9, true);
        assert_eq!(rec(Mat4::IDENTITY).n[2][3].to_bits(), 1);
        assert_eq!(rec(Mat4::from_translation(Vec3::new(3.0, -2.0, 0.5))).n[2][3].to_bits(), 1);
        assert_eq!(rec(Mat4::from_rotation_y(0.3)).n[2][3].to_bits(), 0);
        assert_eq!(rec(Mat4::from_scale(Vec3::splat(2.0))).n[2][3].to_bits(), 0);
        assert_eq!(rec(Mat4::from_scale(Vec3::new(1.0, -1.0, 1.0))).n[2][3].to_bits(), 0, "a mirror");
        assert_eq!((rec(Mat4::IDENTITY).n[0][3].to_bits(), rec(Mat4::IDENTITY).n[1][3].to_bits()), (9, 1));
    }
}
