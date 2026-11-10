//! A level of voxel objects and its edits (ROADMAP § 6 step 6): every object stands in one world lattice
//! (`Placement`), edits are shapes in world fixed point (`Shape`), and what an edit leaves without a chain of
//! six-connected voxels to an anchor (the terrain, objects marked as anchors) becomes a new object that falls until it
//! rests on something. Integers throughout: the same commands give the same voxels on every machine and in the browser
//! (`hash`).

use crate::fixed::Shape;
use crate::grid::VoxelGrid;
use crate::lattice::{locate, Bits, Key, Lattice, Node};
use crate::place::Placement;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

/// A loose part of fewer voxels is no object: its voxels are debris.
pub const MIN_PIECE: usize = 8;
/// Hardness classes of palette entries (`Level::hardness`).
pub const SOFT: u8 = 0;
pub const MEDIUM: u8 = 1;
pub const HARD: u8 = 2;
pub const INDESTRUCTIBLE: u8 = 3;
/// An explosion's radius for each class, in 1 / 256 of its own: soft all of it, medium 0.7, hard 0.4, indestructible
/// none.
pub const RADII: [i32; 4] = [256, 179, 102, 0];
/// Fixed-point units per cell of a falling object's drop and speed.
pub const FALL_FP: i64 = 1 << 16;
/// A falling object's speed is capped at this many cells a tick.
const MAX_FALL: i64 = 8 * FALL_FP;

/// A falling object: how far it has dropped (cells x `FALL_FP`), its speed (per tick), the whole cells of the drop
/// found free so far.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fall {
    pub drop: i64,
    pub vel: i64,
    pub cells: i32,
    /// The object's cells with nothing of it below them (grid coordinates): what can come to rest on something.
    bottoms: Vec<[i32; 3]>,
}

#[derive(Clone, Debug)]
pub struct Object {
    pub grid: Rc<VoxelGrid>,
    pub place: Placement,
    /// It holds what touches it and never falls (terrain, objects marked so).
    pub anchor: bool,
    /// World cells it may have voxels in (inclusive; not shrunk as it loses voxels).
    pub lo: [i32; 3],
    pub hi: [i32; 3],
    pub fall: Option<Fall>,
}

/// What an edit did: the objects whose voxels changed (and in which of their bricks), the objects it made (loose
/// parts, falling; added voxels) and the ones left with no voxel, the voxels removed that are in no new object (world
/// cell and material: debris), and counts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Edit {
    pub changed: BTreeMap<u32, BTreeSet<Key>>,
    pub created: Vec<u32>,
    pub removed: Vec<u32>,
    pub debris: Vec<([i32; 3], u8)>,
    /// Per changed object, the world cells (inclusive bounds) its removed voxels were in.
    pub bounds: BTreeMap<u32, ([i32; 3], [i32; 3])>,
    /// Voxels the shape removed, painted or added.
    pub voxels: usize,
    /// Loose parts that became objects, and their voxels.
    pub pieces: usize,
    pub piece_voxels: usize,
}

/// What a tick did: the objects still falling, the ones that came to rest (static again, where they are now), the
/// ones that fell out of the world.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tick {
    pub falling: Vec<u32>,
    pub landed: Vec<u32>,
    pub removed: Vec<u32>,
}

pub struct Level {
    pub objects: Vec<Option<Object>>,
    /// The hardness class of each palette entry (default soft).
    pub hardness: [u8; 256],
    pub min_piece: usize,
    /// Gravity, `FALL_FP` units of a cell per tick² (`gravity_for`).
    pub gravity: i64,
    lattice: Lattice,
    /// Lattice bricks whose components are stale.
    stale: Vec<Key>,
    /// Anchors by the 64 x 64-cell column (x >> 6, z >> 6) their bounds touch.
    anchors: BTreeMap<(i32, i32), Vec<u32>>,
    /// The anchors' cells by world brick, as asked for (None: none there).
    anchor_bits: HashMap<Key, Option<Box<Bits>>>,
    falling: BTreeSet<u32>,
    /// Falling objects below this height are gone.
    floor: i32,
}

/// Gravity for voxels of `size` metres and `rate` ticks a second, 9.81 m/s².
pub fn gravity_for(size: f64, rate: f64) -> i64 {
    (9.81 / size / (rate * rate) * FALL_FP as f64).round() as i64
}

fn intersects(alo: [i32; 3], ahi: [i32; 3], blo: [i32; 3], bhi: [i32; 3]) -> bool {
    (0..3).all(|k| alo[k] <= bhi[k] && blo[k] <= ahi[k])
}

fn grow(bounds: &mut BTreeMap<u32, ([i32; 3], [i32; 3])>, id: u32, lo: [i32; 3], hi: [i32; 3]) {
    let b = bounds.entry(id).or_insert((lo, hi));
    *b = ([0, 1, 2].map(|k| b.0[k].min(lo[k])), [0, 1, 2].map(|k| b.1[k].max(hi[k])));
}

fn brick_of(g: [i32; 3]) -> Key {
    (g[0] >> 3, g[1] >> 3, g[2] >> 3)
}

/// The anchors' cells in world brick `key` (cached).
fn anchor_bits(objects: &[Option<Object>], anchors: &BTreeMap<(i32, i32), Vec<u32>>, cache: &mut HashMap<Key, Option<Box<Bits>>>, key: Key) -> Option<Bits> {
    if let Some(c) = cache.get(&key) {
        return c.as_deref().copied();
    }
    let (lo, hi) = ([key.0 * 8, key.1 * 8, key.2 * 8], [key.0 * 8 + 7, key.1 * 8 + 7, key.2 * 8 + 7]);
    let mut bits = [0u64; 8];
    let mut any = false;
    for &id in anchors.get(&(key.0 >> 3, key.2 >> 3)).map_or(&[][..], |v| v.as_slice()) {
        let Some(o) = objects[id as usize].as_ref().filter(|o| o.anchor && intersects(lo, hi, o.lo, o.hi)) else { continue };
        let mut last: Option<(Key, Option<&[u8; 512]>)> = None;
        for i in 0..512i32 {
            let g = o.place.to_grid([lo[0] + (i & 7), lo[1] + ((i >> 3) & 7), lo[2] + (i >> 6)]);
            let k = brick_of(g);
            let brick = match last {
                Some((lk, b)) if lk == k => b,
                _ => {
                    let b = o.grid.bricks.get(&k).map(|b| &**b);
                    last = Some((k, b));
                    b
                }
            };
            if brick.is_some_and(|b| b[((g[0] & 7) + 8 * (g[1] & 7) + 64 * (g[2] & 7)) as usize] != 0) {
                bits[(i >> 6) as usize] |= 1 << (i & 63);
                any = true;
            }
        }
    }
    let out = any.then_some(bits);
    cache.insert(key, out.map(Box::new));
    out
}

impl Level {
    pub fn new(gravity: i64) -> Level {
        Level { objects: vec![], hardness: [SOFT; 256], min_piece: MIN_PIECE, gravity, lattice: Lattice::default(), stale: vec![], anchors: BTreeMap::new(), anchor_bits: HashMap::new(), falling: BTreeSet::new(), floor: i32::MAX }
    }

    fn bounds(grid: &VoxelGrid, place: &Placement) -> ([i32; 3], [i32; 3]) {
        match grid.bounds() {
            Some((lo, hi)) => place.box_to_world(lo, hi.map(|v| v - 1)),
            None => ([0; 3], [-1; 3]),
        }
    }

    /// Adds a static object; its id. Call `settle` once the objects are in (and after adding more later).
    pub fn add_object(&mut self, grid: Rc<VoxelGrid>, place: Placement, anchor: bool) -> u32 {
        let id = self.objects.len() as u32;
        let (lo, hi) = Self::bounds(&grid, &place);
        if anchor {
            if lo[0] <= hi[0] {
                for x in lo[0] >> 6..=hi[0] >> 6 {
                    for z in lo[2] >> 6..=hi[2] >> 6 {
                        self.anchors.entry((x, z)).or_default().push(id);
                    }
                }
                self.floor = self.floor.min(lo[1] - 64);
                // (What was asked about these bricks before the anchor came is stale.)
                self.anchor_bits.clear();
            }
        } else {
            self.lattice.set_cells(grid.voxels().map(|(g, _)| place.to_world(g)), &mut self.stale);
        }
        self.objects.push(Some(Object { grid, place, anchor, lo, hi, fall: None }));
        id
    }

    /// Brings the lattice's components up to date with the objects added since.
    pub fn settle(&mut self) {
        let mut keys = std::mem::take(&mut self.stale);
        keys.sort_unstable();
        keys.dedup();
        for k in keys {
            self.lattice.recompute(k, &mut |q| anchor_bits(&self.objects, &self.anchors, &mut self.anchor_bits, q));
        }
    }

    /// Bytes of the connectivity data: the lattice and the cache of the anchors' cells (the objects' voxels are not
    /// counted).
    pub fn connectivity_bytes(&self) -> usize {
        self.lattice.bytes() + self.anchor_bits.capacity() * (std::mem::size_of::<Key>() + 8 + 1) + self.anchor_bits.values().filter(|b| b.is_some()).count() * 64
    }

    /// The lattice's bricks (destructible static voxels' world bricks).
    pub fn lattice_bricks(&self) -> usize {
        self.lattice.len()
    }

    /// Whether world cell `c` holds a static voxel (an anchor's or a destructible object's; not a falling one's).
    pub fn solid(&mut self, c: [i32; 3]) -> bool {
        if self.lattice.get(c) {
            return true;
        }
        let (key, z, bit) = locate(c);
        anchor_bits(&self.objects, &self.anchors, &mut self.anchor_bits, key).is_some_and(|b| b[z] & bit != 0)
    }

    /// Static objects whose bounds meet the world cells lo..=hi, ascending.
    fn static_in(&self, lo: [i32; 3], hi: [i32; 3]) -> Vec<u32> {
        self.objects.iter().enumerate().filter(|(_, o)| o.as_ref().is_some_and(|o| o.fall.is_none() && intersects(lo, hi, o.lo, o.hi))).map(|(k, _)| k as u32).collect()
    }

    /// The parts of the static destructible voxels that hang on no anchor, as the level stands (at load: objects placed
    /// in the air, or touching the ground only by an edge): each group's world cells. Nothing is changed: a part falls
    /// only once an edit touches it.
    pub fn floating_groups(&mut self) -> Vec<Vec<[i32; 3]>> {
        self.settle();
        let mut keys = self.lattice.keys();
        keys.sort_unstable();
        let groups = self.lattice.detached(&keys);
        groups.iter().map(|g| g.iter().flat_map(|&n| self.lattice.cells(n)).collect()).collect()
    }

    /// `floating_groups` counted: how many groups, and their voxels.
    pub fn floating(&mut self) -> (usize, usize) {
        let g = self.floating_groups();
        (g.len(), g.iter().map(|c| c.len()).sum())
    }

    /// The first static object with a voxel in world cell `c`.
    pub fn object_at(&self, c: [i32; 3]) -> Option<u32> {
        self.objects.iter().enumerate().find(|(_, o)| o.as_ref().is_some_and(|o| o.fall.is_none() && intersects(c, c, o.lo, o.hi) && { let g = o.place.to_grid(c); o.grid.get(g[0], g[1], g[2]) != 0 })).map(|(k, _)| k as u32)
    }

    /// The object's voxels inside the shape's cells that `keep` accepts (grid cell, world cell, material), bricks in key
    /// order.
    fn scan(o: &Object, lo: [i32; 3], hi: [i32; 3], keep: &mut dyn FnMut([i32; 3], [i32; 3], u8) -> bool) -> Vec<([i32; 3], [i32; 3], u8)> {
        let (lo, hi) = ([0, 1, 2].map(|k| lo[k].max(o.lo[k])), [0, 1, 2].map(|k| hi[k].min(o.hi[k])));
        let mut out = vec![];
        if (0..3).any(|k| lo[k] > hi[k]) {
            return out;
        }
        let (glo, ghi) = o.place.box_to_grid(lo, hi);
        for bx in glo[0] >> 3..=ghi[0] >> 3 {
            for by in glo[1] >> 3..=ghi[1] >> 3 {
                for bz in glo[2] >> 3..=ghi[2] >> 3 {
                    let Some(b) = o.grid.bricks.get(&(bx, by, bz)) else { continue };
                    for (v, &m) in b.iter().enumerate() {
                        if m == 0 {
                            continue;
                        }
                        let g = [bx * 8 + (v & 7) as i32, by * 8 + ((v >> 3) & 7) as i32, bz * 8 + (v >> 6) as i32];
                        if (0..3).any(|k| g[k] < glo[k] || g[k] > ghi[k]) {
                            continue;
                        }
                        let w = o.place.to_world(g);
                        if keep(g, w, m) {
                            out.push((g, w, m));
                        }
                    }
                }
            }
        }
        out
    }

    /// Removes the voxels inside `shape`: each palette entry within the shape's radius scaled by `radii` of its
    /// hardness class (`RADII`: an explosion; `[256; 4]`: a cut through everything). The voxels are debris; what is
    /// left hanging on no anchor becomes falling objects (fewer than `min_piece` voxels: debris too).
    pub fn carve(&mut self, shape: Shape, radii: [i32; 4]) -> Edit {
        let mut edit = Edit::default();
        let Some(shape) = shape.checked() else { return edit };
        self.settle();
        let (lo, hi) = shape.cells();
        let hit = self.static_in(lo, hi);
        let mut anchors_changed = false;
        // The smallest reach of a class that is reached at all: a brick wholly inside the shape at it loses every voxel
        // of those classes (the shapes are convex: its eight corner cells inside means all of it).
        let least = radii.iter().copied().filter(|&r| r > 0).min().unwrap_or(0);
        for &id in &hit {
            let o = self.objects[id as usize].as_ref().unwrap();
            let (clo, chi) = ([0, 1, 2].map(|k| lo[k].max(o.lo[k])), [0, 1, 2].map(|k| hi[k].min(o.hi[k])));
            if (0..3).any(|k| clo[k] > chi[k]) {
                continue;
            }
            let (place, anchor) = (o.place, o.anchor);
            let (glo, ghi) = place.box_to_grid(clo, chi);
            // The voxels that go, brick by brick (bit v: the brick's voxel v).
            let mut gone: Vec<(Key, Bits)> = vec![];
            for bx in glo[0] >> 3..=ghi[0] >> 3 {
                for by in glo[1] >> 3..=ghi[1] >> 3 {
                    for bz in glo[2] >> 3..=ghi[2] >> 3 {
                        let Some(b) = o.grid.bricks.get(&(bx, by, bz)) else { continue };
                        let whole = least > 0 && (0..8).all(|c| shape.contains(place.to_world([bx * 8 + 7 * (c & 1), by * 8 + 7 * ((c >> 1) & 1), bz * 8 + 7 * (c >> 2)]), least));
                        let mut mask = [0u64; 8];
                        for (v, &m) in b.iter().enumerate() {
                            if m == 0 {
                                continue;
                            }
                            let r = radii[(self.hardness[m as usize] & 3) as usize];
                            if r <= 0 {
                                continue;
                            }
                            if !whole {
                                let g = [bx * 8 + (v & 7) as i32, by * 8 + ((v >> 3) & 7) as i32, bz * 8 + (v >> 6) as i32];
                                if (0..3).any(|k| g[k] < glo[k] || g[k] > ghi[k]) || !shape.contains(place.to_world(g), r) {
                                    continue;
                                }
                            }
                            mask[v >> 6] |= 1 << (v & 63);
                        }
                        if mask != [0; 8] {
                            gone.push(((bx, by, bz), mask));
                        }
                    }
                }
            }
            if gone.is_empty() {
                continue;
            }
            anchors_changed |= anchor;
            let mut cleared: Vec<[i32; 3]> = vec![];
            let o = self.objects[id as usize].as_mut().unwrap();
            let grid = Rc::make_mut(&mut o.grid);
            let keys = edit.changed.entry(id).or_default();
            let (mut blo, mut bhi) = ([i32::MAX; 3], [i32::MIN; 3]);
            for (key, mask) in gone {
                let b = grid.bricks.get_mut(&key).unwrap();
                for (z, &plane) in mask.iter().enumerate() {
                    let mut p = plane;
                    while p != 0 {
                        let i = p.trailing_zeros() as usize;
                        p &= p - 1;
                        let m = std::mem::take(&mut b[z * 64 + i]);
                        let w = place.to_world([key.0 * 8 + (i & 7) as i32, key.1 * 8 + (i >> 3) as i32, key.2 * 8 + z as i32]);
                        for k in 0..3 {
                            blo[k] = blo[k].min(w[k]);
                            bhi[k] = bhi[k].max(w[k]);
                        }
                        edit.debris.push((w, m));
                        edit.voxels += 1;
                        if !anchor {
                            cleared.push(w);
                        }
                    }
                }
                if b.iter().all(|&v| v == 0) {
                    grid.bricks.remove(&key);
                }
                keys.insert(key);
            }
            grow(&mut edit.bounds, id, blo, bhi);
            // The lattice's cells (another destructible object's voxel in the same cell keeps the cell).
            let others: Vec<u32> = hit.iter().copied().filter(|&q| q != id && self.objects[q as usize].as_ref().is_some_and(|p| !p.anchor)).collect();
            for w in cleared {
                let kept = others.iter().any(|&q| {
                    let p = self.objects[q as usize].as_ref().unwrap();
                    intersects(w, w, p.lo, p.hi) && {
                        let g = p.place.to_grid(w);
                        p.grid.get(g[0], g[1], g[2]) != 0
                    }
                });
                if !kept {
                    self.lattice.clear(w);
                }
            }
        }
        if edit.voxels == 0 {
            return edit;
        }
        if anchors_changed {
            // The anchors' cells around the edit are asked for again.
            for x in (lo[0] >> 3) - 1..=(hi[0] >> 3) + 1 {
                for y in (lo[1] >> 3) - 1..=(hi[1] >> 3) + 1 {
                    for z in (lo[2] >> 3) - 1..=(hi[2] >> 3) + 1 {
                        self.anchor_bits.remove(&(x, y, z));
                    }
                }
            }
        }
        self.release(lo, hi, &mut edit);
        self.sweep(&mut edit);
        edit
    }

    /// The exposed voxels inside `shape` (a voxel with an empty neighbour in its own object) take palette entry
    /// `material`: paint, soot. No voxel is added or removed.
    pub fn paint(&mut self, shape: Shape, material: u8) -> Edit {
        let mut edit = Edit::default();
        let Some(shape) = shape.checked() else { return edit };
        if material == 0 {
            return edit;
        }
        let (lo, hi) = shape.cells();
        for id in self.static_in(lo, hi) {
            let o = self.objects[id as usize].as_ref().unwrap();
            let grid = &o.grid;
            let hit = Self::scan(o, lo, hi, &mut |g, w, m| {
                m != material && shape.contains(w, 256) && crate::lattice::DIRS.iter().any(|d| grid.get(g[0] + d[0], g[1] + d[1], g[2] + d[2]) == 0)
            });
            if hit.is_empty() {
                continue;
            }
            let o = self.objects[id as usize].as_mut().unwrap();
            let grid = Rc::make_mut(&mut o.grid);
            let keys = edit.changed.entry(id).or_default();
            for &(g, _, _) in &hit {
                grid.set(g[0], g[1], g[2], material);
                keys.insert(brick_of(g));
            }
            edit.voxels += hit.len();
        }
        edit
    }

    /// Voxels of palette entry `material` in every empty cell inside `shape`, as a new object: static if it touches
    /// something that hangs on an anchor, falling otherwise.
    pub fn add(&mut self, shape: Shape, material: u8) -> Edit {
        let mut edit = Edit::default();
        let Some(shape) = shape.checked() else { return edit };
        if material == 0 {
            return edit;
        }
        self.settle();
        let (lo, hi) = shape.cells();
        let mut grid = VoxelGrid::default();
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    if shape.contains([x, y, z], 256) && !self.solid([x, y, z]) {
                        grid.set(x - lo[0], y - lo[1], z - lo[2], material);
                        edit.voxels += 1;
                    }
                }
            }
        }
        if edit.voxels == 0 {
            return edit;
        }
        let id = self.add_object(Rc::new(grid), Placement { turn: 0, t: lo }, false);
        edit.created.push(id);
        self.settle();
        self.release(lo, hi, &mut edit);
        // (Loose: its voxels went into a falling object and it is gone before anyone saw it.)
        if edit.removed.contains(&id) {
            edit.removed.retain(|&r| r != id);
            edit.created.retain(|&c| c != id);
        }
        edit.changed.remove(&id);
        edit
    }

    /// After voxels in world cells lo..=hi changed: the lattice's bricks around them again, and what hangs on no anchor
    /// any more taken out of its objects.
    fn release(&mut self, lo: [i32; 3], hi: [i32; 3], edit: &mut Edit) {
        // A voxel next to a changed cell may be in the next brick: one cell around.
        let mut starts = vec![];
        for x in (lo[0] - 1) >> 3..=(hi[0] + 1) >> 3 {
            for y in (lo[1] - 1) >> 3..=(hi[1] + 1) >> 3 {
                for z in (lo[2] - 1) >> 3..=(hi[2] + 1) >> 3 {
                    if self.lattice.has((x, y, z)) {
                        self.lattice.recompute((x, y, z), &mut |q| anchor_bits(&self.objects, &self.anchors, &mut self.anchor_bits, q));
                        if self.lattice.has((x, y, z)) {
                            starts.push((x, y, z));
                        }
                    }
                }
            }
        }
        let groups = self.lattice.detached(&starts);
        if groups.is_empty() {
            return;
        }
        // Every group's cells first (taking one out changes the components of the bricks it shares with the next).
        let cells: Vec<Vec<[i32; 3]>> = groups
            .iter()
            .map(|g| {
                let mut nodes: Vec<Node> = g.clone();
                nodes.sort_unstable();
                nodes.iter().flat_map(|&n| self.lattice.cells(n)).collect()
            })
            .collect();
        let mut touched: Vec<Key> = vec![];
        for cells in cells {
            let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
            for c in &cells {
                for k in 0..3 {
                    lo[k] = lo[k].min(c[k]);
                    hi[k] = hi[k].max(c[k]);
                }
            }
            let owners: Vec<u32> = self.static_in(lo, hi).into_iter().filter(|&id| !self.objects[id as usize].as_ref().unwrap().anchor).collect();
            let mut taken: BTreeMap<u32, Vec<[i32; 3]>> = BTreeMap::new();
            let mut voxels: Vec<([i32; 3], u8)> = Vec::with_capacity(cells.len());
            for &c in &cells {
                let mut mat = 0;
                for &id in &owners {
                    let o = self.objects[id as usize].as_ref().unwrap();
                    if !intersects(c, c, o.lo, o.hi) {
                        continue;
                    }
                    let g = o.place.to_grid(c);
                    let m = o.grid.get(g[0], g[1], g[2]);
                    if m != 0 {
                        if mat == 0 {
                            mat = m;
                        }
                        taken.entry(id).or_default().push(g);
                    }
                }
                self.lattice.clear(c);
                touched.push(locate(c).0);
                if mat != 0 {
                    voxels.push((c, mat));
                }
            }
            for (id, list) in taken {
                grow(&mut edit.bounds, id, lo, hi);
                let o = self.objects[id as usize].as_mut().unwrap();
                let grid = Rc::make_mut(&mut o.grid);
                let keys = edit.changed.entry(id).or_default();
                for g in list {
                    grid.take(g[0], g[1], g[2]);
                    keys.insert(brick_of(g));
                }
                for k in keys.iter() {
                    grid.prune(*k);
                }
            }
            if voxels.len() < self.min_piece {
                edit.debris.extend(voxels);
                continue;
            }
            let mut grid = VoxelGrid::default();
            for &(c, m) in &voxels {
                grid.set(c[0] - lo[0], c[1] - lo[1], c[2] - lo[2], m);
            }
            let bottoms = grid.voxels().map(|(g, _)| g).filter(|g| grid.get(g[0], g[1] - 1, g[2]) == 0).collect();
            let id = self.objects.len() as u32;
            self.objects.push(Some(Object { grid: Rc::new(grid), place: Placement { turn: 0, t: lo }, anchor: false, lo, hi, fall: Some(Fall { drop: 0, vel: 0, cells: 0, bottoms }) }));
            self.falling.insert(id);
            edit.created.push(id);
            edit.pieces += 1;
            edit.piece_voxels += voxels.len();
        }
        touched.sort_unstable();
        touched.dedup();
        for k in touched {
            self.lattice.recompute(k, &mut |q| anchor_bits(&self.objects, &self.anchors, &mut self.anchor_bits, q));
        }
        self.sweep(edit);
    }

    /// Objects an edit left with no voxel are gone.
    fn sweep(&mut self, edit: &mut Edit) {
        let empty: Vec<u32> = edit.changed.keys().copied().filter(|&id| self.objects[id as usize].as_ref().is_some_and(|o| o.grid.bricks.is_empty())).collect();
        for id in empty {
            self.objects[id as usize] = None;
            edit.changed.remove(&id);
            edit.bounds.remove(&id);
            edit.removed.push(id);
        }
    }

    /// One step of the falling objects (in id order): gravity, then down cell by cell until something static is in
    /// the way; there the object is static again.
    pub fn tick(&mut self) -> Tick {
        let mut out = Tick::default();
        if self.falling.is_empty() {
            return out;
        }
        self.settle();
        let ids: Vec<u32> = self.falling.iter().copied().collect();
        for id in ids {
            let (t, mut f) = {
                let o = self.objects[id as usize].as_mut().unwrap();
                (o.place.t, o.fall.take().unwrap())
            };
            f.vel = (f.vel + self.gravity).min(MAX_FALL);
            f.drop += f.vel;
            let target = (f.drop / FALL_FP) as i32;
            let mut landed = false;
            while f.cells < target {
                let n = f.cells + 1;
                if f.bottoms.iter().any(|b| self.solid([b[0] + t[0], b[1] + t[1] - n, b[2] + t[2]])) {
                    landed = true;
                    break;
                }
                f.cells = n;
            }
            if landed {
                let o = self.objects[id as usize].as_mut().unwrap();
                o.place.t[1] -= f.cells;
                o.lo[1] -= f.cells;
                o.hi[1] -= f.cells;
                let place = o.place;
                let grid = o.grid.clone();
                let mut touched = vec![];
                self.lattice.set_cells(grid.voxels().map(|(g, _)| place.to_world(g)), &mut touched);
                touched.sort_unstable();
                touched.dedup();
                for k in touched {
                    self.lattice.recompute(k, &mut |q| anchor_bits(&self.objects, &self.anchors, &mut self.anchor_bits, q));
                }
                self.falling.remove(&id);
                out.landed.push(id);
            } else if t[1] - f.cells < if self.floor == i32::MAX { -(1 << 20) } else { self.floor } {
                self.objects[id as usize] = None;
                self.falling.remove(&id);
                out.removed.push(id);
            } else {
                self.objects[id as usize].as_mut().unwrap().fall = Some(f);
                out.falling.push(id);
            }
        }
        out
    }

    /// Objects falling now.
    pub fn falling(&self) -> usize {
        self.falling.len()
    }

    /// A hash of every object's placement, state and voxels (FNV-1a, objects in id order, bricks in key order): equal
    /// levels hash alike on every machine.
    pub fn hash(&self) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut feed = |bytes: &[u8]| {
            for &b in bytes {
                h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        for (id, o) in self.objects.iter().enumerate() {
            let Some(o) = o else { continue };
            feed(&(id as u32).to_le_bytes());
            feed(&[o.place.turn, o.anchor as u8, o.fall.is_some() as u8]);
            for v in o.place.t {
                feed(&v.to_le_bytes());
            }
            if let Some(f) = &o.fall {
                feed(&f.drop.to_le_bytes());
                feed(&f.vel.to_le_bytes());
                feed(&f.cells.to_le_bytes());
            }
            for (k, b) in &o.grid.bricks {
                for v in [k.0, k.1, k.2] {
                    feed(&v.to_le_bytes());
                }
                feed(&b[..]);
            }
        }
        h
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixed::{to_fp, FP};

    fn block(lo: [i32; 3], hi: [i32; 3], m: u8) -> Rc<VoxelGrid> {
        let mut g = VoxelGrid::default();
        for x in lo[0]..hi[0] {
            for y in lo[1]..hi[1] {
                for z in lo[2]..hi[2] {
                    g.set(x, y, z, m);
                }
            }
        }
        Rc::new(g)
    }

    fn sphere(c: [f64; 3], r: f64) -> Shape {
        Shape::Sphere { c: c.map(to_fp), r: to_fp(r) }
    }

    /// A level with a ground slab (the anchor: world cells x, z 0..32, y 0..4).
    fn ground() -> Level {
        let mut l = Level::new(gravity_for(0.1, 60.0));
        l.add_object(block([0, 0, 0], [32, 4, 32], 1), Placement::default(), true);
        l
    }

    fn run(l: &mut Level) -> (usize, Vec<u32>) {
        let mut landed = vec![];
        for n in 0..10_000 {
            let t = l.tick();
            landed.extend(t.landed);
            assert!(t.removed.is_empty());
            if l.falling() == 0 {
                return (n, landed);
            }
        }
        panic!("still falling");
    }

    fn count(l: &Level, id: u32) -> usize {
        l.objects[id as usize].as_ref().map_or(0, |o| o.grid.count())
    }

    /// A tower on the ground, cut through: the part above the cut is a new object that falls on the stump and is
    /// static there; cutting the stump at its foot then drops the stump and what rests on it as one object.
    #[test]
    fn a_cut_tower_falls() {
        let mut l = ground();
        // 4 x 36 x 4 voxels, placed so that it stands on the slab at x, z 10..14, y 4..40.
        let tower = l.add_object(block([0, 0, 0], [4, 36, 4], 2), Placement::new([10, 4, 10], 0, [0, 0, 0]), false);
        l.settle();
        assert_eq!(l.floating(), (0, 0));
        assert_eq!(l.gravity, 1786);
        let e = l.carve(sphere([12.0, 20.0, 12.0], 3.2), [256; 4]);
        // The cut takes whole layers y 18..22 (and parts of the next ones: a 4 x 4 section is within 3.2 of the axis
        // only where |dy| <= 2.4).
        assert!(e.voxels >= 64, "{}", e.voxels);
        assert_eq!((e.pieces, e.created.len(), e.removed.len()), (1, 1, 0));
        assert_eq!(e.debris.len(), e.voxels, "the carved voxels are debris; the loose part is an object");
        let piece = e.created[0];
        assert_eq!(count(&l, tower) + count(&l, piece) + e.voxels, 4 * 36 * 4);
        assert_eq!(count(&l, piece), e.piece_voxels);
        let top = l.objects[piece as usize].clone().unwrap();
        assert!(top.fall.is_some() && top.lo[1] >= 22 && top.hi[1] == 39);
        assert!(e.changed.contains_key(&tower) && !e.changed.contains_key(&piece));
        // Falling voxels are not solid; the stump is.
        assert!(!l.solid([12, 39, 12]) && l.solid([12, 5, 12]) && l.solid([3, 2, 3]) && !l.solid([3, 4, 3]));
        // The cut is a sphere's: the stump is highest and the part above lowest at the corners, where they meet again.
        let gap = top.lo[1] - (0..40).rev().find(|&y| l.objects[tower as usize].as_ref().unwrap().grid.get(0, y - 4, 0) != 0).unwrap() - 1;
        assert!(gap >= 4);
        let (ticks, landed) = run(&mut l);
        assert_eq!(landed, vec![piece]);
        // Free fall over `gap` cells of 0.1 m: sqrt(2 x 0.1 gap / 9.81) s, in ticks of 1 / 60 s (one more to find the
        // stump in the way).
        let expect = ((2.0 * 0.1 * gap as f64 / 9.81).sqrt() * 60.0).ceil() as usize;
        assert!((ticks as i64 - expect as i64).abs() <= 2, "{ticks} ticks for {gap} cells, about {expect}");
        let top = l.objects[piece as usize].clone().unwrap();
        assert!(top.fall.is_none());
        assert_eq!(top.hi[1], 39 - gap);
        assert!(l.solid([10, top.lo[1], 10]) && l.solid([10, top.lo[1] - 1, 10]), "it rests on the stump (at the corners)");
        assert_eq!(l.tick(), Tick::default());
        // Cut the stump's foot: the stump and the part on it are one loose group (two objects' voxels), one new object.
        let before = count(&l, tower) + count(&l, piece);
        let e = l.carve(Shape::Box { lo: [0, 4 * FP, 0], hi: [32 * FP, 6 * FP, 32 * FP] }, [256; 4]);
        assert_eq!(e.voxels, 4 * 2 * 4);
        assert_eq!((e.pieces, e.piece_voxels), (1, before - 32));
        assert_eq!(e.removed, vec![tower, piece], "both gave all their voxels to the new object");
        assert!(l.objects[tower as usize].is_none() && l.objects[piece as usize].is_none());
        let (_, landed) = run(&mut l);
        assert_eq!(landed, e.created);
        let o = l.objects[e.created[0] as usize].as_ref().unwrap();
        assert_eq!((o.lo[1], o.grid.count()), (4, before - 32), "on the ground");
    }

    /// A roof over two pillars, three objects: with one pillar cut the roof holds (through the other); with both cut,
    /// the roof and the pillars' upper parts fall together.
    #[test]
    fn connections_cross_objects() {
        let mut l = ground();
        let a = l.add_object(block([0, 0, 0], [2, 10, 2], 2), Placement::new([4, 4, 4], 0, [0; 3]), false);
        // The second pillar is turned a quarter: its grid's x runs along the world's -z.
        let b = l.add_object(block([0, 0, 0], [2, 10, 2], 2), Placement::new([20, 4, 6], 1, [0; 3]), false);
        let roof = l.add_object(block([0, 0, 0], [20, 2, 4], 3), Placement::new([3, 14, 3], 0, [0; 3]), false);
        l.settle();
        // (Turned: it stands at x 20..22, z 4..6.)
        assert!(l.solid([20, 4, 4]) && l.solid([21, 13, 5]) && !l.solid([22, 4, 4]) && !l.solid([20, 4, 6]) && !l.solid([20, 4, 3]));
        assert_eq!(l.floating(), (0, 0));
        let cut = |x: i32| Shape::Box { lo: [(x - 2) * FP, 8 * FP, 0], hi: [(x + 4) * FP, 10 * FP, 12 * FP] };
        let e = l.carve(cut(4), [256; 4]);
        assert_eq!((e.voxels, e.pieces), (2 * 2 * 2, 0), "the roof holds the pillar's top");
        assert_eq!(e.changed.keys().copied().collect::<Vec<_>>(), vec![a]);
        let e = l.carve(cut(20), [256; 4]);
        assert_eq!(e.voxels, 8);
        assert_eq!((e.pieces, e.piece_voxels), (1, 20 * 2 * 4 + 2 * (2 * 4 * 2)));
        assert_eq!(e.removed, vec![roof]);
        assert_eq!(e.changed.keys().copied().collect::<Vec<_>>(), vec![a, b]);
        let (_, landed) = run(&mut l);
        let o = l.objects[landed[0] as usize].as_ref().unwrap();
        assert_eq!(o.lo[1], 8, "the pillars' upper parts stand on their stumps again");
        assert_eq!(l.floating(), (0, 0));
    }

    /// An explosion reaches each material by its hardness; an indestructible one stays; a loose part of fewer than
    /// `MIN_PIECE` voxels is debris.
    #[test]
    fn hardness_and_small_parts() {
        let mut l = ground();
        l.hardness[2] = MEDIUM;
        l.hardness[3] = HARD;
        l.hardness[4] = INDESTRUCTIBLE;
        let mut g = VoxelGrid::default();
        for x in 0..24 {
            for y in 0..2 {
                for z in 0..4 {
                    g.set(x, y, z, 1 + (z as u8));
                }
            }
        }
        let wall = l.add_object(Rc::new(g), Placement::new([4, 4, 4], 0, [0; 3]), false);
        l.settle();
        let e = l.carve(sphere([16.0, 5.0, 6.0], 8.0), RADII);
        let left = &l.objects[wall as usize].as_ref().unwrap().grid;
        let row = |z: i32| (0..24).filter(|&x| left.get(x, 0, z) == 0).count();
        // Rows z = 0..3 are soft, medium, hard, indestructible: they lose their cells within 8, 5.59, 3.19 and 0 voxels of
        // the centre (1.5, 0.5, 0.5, 1.5 away in z, 0.5 in y: 16, 12, 6 and 0 cells along x).
        assert_eq!((row(0), row(1), row(2), row(3)), (16, 12, 6, 0));
        assert_eq!(24 * 2 * 4 - left.count(), 2 * (16 + 12 + 6));
        assert!(e.voxels > 68, "the ground under it lost voxels too");
        assert_eq!(e.pieces, 0);
        // A stalk of 5 voxels on the wall: cut at its foot, its 4 loose voxels are debris, no object.
        let stalk = l.add_object(block([0, 0, 0], [1, 5, 1], 1), Placement::new([5, 6, 7], 0, [0; 3]), false);
        l.settle();
        let e = l.carve(Shape::Box { lo: [5 * FP, 6 * FP, 7 * FP], hi: [6 * FP, 7 * FP, 8 * FP] }, [256; 4]);
        assert_eq!((e.voxels, e.pieces, e.debris.len()), (1, 0, 5));
        assert_eq!(e.removed, vec![stalk]);
        assert_eq!(l.falling(), 0);
    }

    /// Paint changes exposed voxels' material only; added voxels are an object, static where they touch something
    /// held, falling where they do not.
    #[test]
    fn paint_and_add() {
        let mut l = ground();
        let cube = l.add_object(block([0, 0, 0], [6, 6, 6], 2), Placement::new([8, 4, 8], 0, [0; 3]), false);
        l.settle();
        let h = l.hash();
        let e = l.paint(sphere([11.0, 7.0, 11.0], 20.0), 5);
        // The ground's top and sides are exposed too; of the cube, its shell: 6³ - 4³ (its bottom's neighbours below are
        // the ground's, another object: exposed for paint).
        assert_eq!(e.changed[&cube].len(), 1);
        let g = &l.objects[cube as usize].as_ref().unwrap().grid;
        assert_eq!((g.get(0, 0, 0), g.get(1, 1, 1), g.get(5, 3, 2), g.get(3, 3, 3)), (5, 2, 5, 2));
        assert_eq!(g.voxels().filter(|v| v.1 == 5).count(), 216 - 64);
        assert_eq!(g.count(), 216);
        assert_ne!(l.hash(), h);
        assert_eq!(l.paint(sphere([11.0, 7.0, 11.0], 20.0), 5).voxels, 0, "already that colour");
        // A block on the cube: static. One in the air: it falls on the ground.
        let e = l.add(Shape::Box { lo: [9 * FP, 10 * FP, 9 * FP], hi: [11 * FP, 12 * FP, 11 * FP] }, 3);
        assert_eq!((e.voxels, e.created.len(), e.pieces), (8, 1, 0));
        assert!(l.objects[e.created[0] as usize].as_ref().unwrap().fall.is_none());
        let e = l.add(Shape::Box { lo: [20 * FP, 20 * FP, 20 * FP], hi: [22 * FP, 22 * FP, 22 * FP] }, 3);
        assert_eq!((e.voxels, e.created.len(), e.pieces, e.removed.len()), (8, 1, 1, 0));
        let id = e.created[0];
        assert!(l.objects[id as usize].as_ref().unwrap().fall.is_some());
        let (_, landed) = run(&mut l);
        assert_eq!(landed, vec![id]);
        assert_eq!(l.objects[id as usize].as_ref().unwrap().lo[1], 4);
        // Adding where voxels are adds only the empty cells.
        let e = l.add(Shape::Box { lo: [8 * FP, 4 * FP, 8 * FP], hi: [14 * FP, 11 * FP, 14 * FP] }, 3);
        assert_eq!(e.voxels, 6 * 7 * 6 - 216 - 4, "the cube and half of the block on it are there");
    }

    /// The same commands twice give the same level; a different command a different one.
    #[test]
    fn edits_are_deterministic() {
        assert_eq!(crate::check::script(), crate::check::script());
        assert_eq!(crate::check::script(), crate::check::EXPECTED, "the script's level changed: if meant, record {:#x}", crate::check::script());
    }
}
