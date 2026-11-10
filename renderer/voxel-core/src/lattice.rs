//! Connectivity in the world lattice (ROADMAP § 6 step 6): the occupancy of every destructible static object, in
//! world bricks of 8³ cells, whatever object a cell belongs to. Each brick keeps its connected components (six-connected
//! cells), each component the cells it has on the brick's six faces and whether it touches an anchor. Two components of
//! neighbouring bricks are connected where their facing cells meet. An edit recomputes the bricks it changed; whether a
//! component still hangs on an anchor is then a search over components, not over voxels.

use std::collections::HashMap;

/// A world brick: cell coordinates >> 3.
pub type Key = (i32, i32, i32);
/// A brick's cells as bits: plane z in word z, cell (x, y) at bit x + 8 y.
pub type Bits = [u64; 8];

/// The six neighbours, in face order (-x, +x, -y, +y, -z, +z); face d's opposite is d ^ 1.
pub const DIRS: [[i32; 3]; 6] = [[-1, 0, 0], [1, 0, 0], [0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1]];

const COL0: u64 = 0x0101_0101_0101_0101;
const COL7: u64 = 0x8080_8080_8080_8080;

/// The brick of world cell `c`, and the cell's plane and bit in it.
pub fn locate(c: [i32; 3]) -> (Key, usize, u64) {
    ((c[0] >> 3, c[1] >> 3, c[2] >> 3), (c[2] & 7) as usize, 1u64 << ((c[0] & 7) + 8 * (c[1] & 7)))
}

/// The cells of `m` and their six neighbours inside the brick.
fn dilate(m: &Bits) -> Bits {
    let mut out = [0u64; 8];
    for z in 0..8 {
        let p = m[z];
        let mut n = p | ((p << 1) & !COL0) | ((p >> 1) & !COL7) | (p << 8) | (p >> 8);
        if z > 0 {
            n |= m[z - 1];
        }
        if z < 7 {
            n |= m[z + 1];
        }
        out[z] = n;
    }
    out
}

/// The six-connected components of `occ`, in the order of their lowest cell.
pub fn components(occ: &Bits) -> Vec<Bits> {
    let mut left = *occ;
    let mut out = vec![];
    while let Some(z) = left.iter().position(|&p| p != 0) {
        let mut cur = [0u64; 8];
        cur[z] = left[z] & left[z].wrapping_neg();
        loop {
            let mut next = dilate(&cur);
            for k in 0..8 {
                next[k] &= left[k];
            }
            if next == cur {
                break;
            }
            cur = next;
        }
        for k in 0..8 {
            left[k] &= !cur[k];
        }
        out.push(cur);
    }
    out
}

/// The cells `m` has on each face of the brick: 64 bits a face, the two in-face coordinates in axis order (face ±x:
/// bit y + 8 z; ±y: x + 8 z; ±z: x + 8 y), so facing cells of neighbouring bricks have the same bit.
pub fn faces(m: &Bits) -> [u64; 6] {
    let mut f = [0u64; 6];
    for z in 0..8 {
        let p = m[z];
        for y in 0..8 {
            f[0] |= ((p >> (8 * y)) & 1) << (y + 8 * z);
            f[1] |= ((p >> (8 * y + 7)) & 1) << (y + 8 * z);
        }
        f[2] |= (p & 0xff) << (8 * z);
        f[3] |= (p >> 56) << (8 * z);
    }
    f[4] = m[0];
    f[5] = m[7];
    f
}

fn count(m: &Bits) -> u32 {
    m.iter().map(|p| p.count_ones()).sum()
}

/// A component of a brick: its cells on the six faces, whether it touches an anchor, and the search's marks.
#[derive(Clone, Debug)]
struct Comp {
    faces: [u64; 6],
    anchored: bool,
    mark: u32,
    held: bool,
}

#[derive(Clone, Debug)]
struct Brick {
    key: Key,
    occ: Bits,
    comps: Vec<Comp>,
}

/// A component: its brick's key and its place among `components` of the brick's cells.
pub type Node = (Key, u16);

#[derive(Default)]
pub struct Lattice {
    bricks: Vec<Brick>,
    index: HashMap<Key, u32>,
    free: Vec<u32>,
    stamp: u32,
}

impl Lattice {
    pub fn get(&self, c: [i32; 3]) -> bool {
        let (key, z, bit) = locate(c);
        self.index.get(&key).is_some_and(|&i| self.bricks[i as usize].occ[z] & bit != 0)
    }

    /// Bricks that hold cells.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    pub fn has(&self, key: Key) -> bool {
        self.index.contains_key(&key)
    }

    fn slot(&mut self, key: Key) -> u32 {
        if let Some(&i) = self.index.get(&key) {
            return i;
        }
        let b = Brick { key, occ: [0; 8], comps: vec![] };
        let i = match self.free.pop() {
            Some(i) => {
                self.bricks[i as usize] = b;
                i
            }
            None => {
                self.bricks.push(b);
                self.bricks.len() as u32 - 1
            }
        };
        self.index.insert(key, i);
        i
    }

    /// Sets world cells (consecutive cells of one brick find it once); adds the bricks they are in to `touched` (a
    /// brick may be listed more than once). Their components are stale until `recompute`.
    pub fn set_cells(&mut self, cells: impl Iterator<Item = [i32; 3]>, touched: &mut Vec<Key>) {
        let mut last: Option<(Key, u32)> = None;
        for c in cells {
            let (key, z, bit) = locate(c);
            let i = match last {
                Some((k, i)) if k == key => i,
                _ => {
                    let i = self.slot(key);
                    last = Some((key, i));
                    touched.push(key);
                    i
                }
            };
            self.bricks[i as usize].occ[z] |= bit;
        }
    }

    /// Bytes the lattice holds (its bricks, their components, the index).
    pub fn bytes(&self) -> usize {
        self.bricks.capacity() * std::mem::size_of::<Brick>() + self.bricks.iter().map(|b| b.comps.capacity() * std::mem::size_of::<Comp>()).sum::<usize>() + self.index.capacity() * (std::mem::size_of::<Key>() + 4 + 1)
    }

    /// Every brick's key (in no order).
    pub fn keys(&self) -> Vec<Key> {
        self.index.keys().copied().collect()
    }

    /// Clears world cell `c` (stale until `recompute`, which also drops a brick left empty).
    pub fn clear(&mut self, c: [i32; 3]) {
        let (key, z, bit) = locate(c);
        if let Some(&i) = self.index.get(&key) {
            self.bricks[i as usize].occ[z] &= !bit;
        }
    }

    /// Brick `key`'s components again from its cells; `anchor(k)`: the anchors' cells in world brick k (None: none).
    /// A component is anchored when a cell of it is an anchor's cell or next to one. An empty brick is dropped.
    pub fn recompute(&mut self, key: Key, anchor: &mut dyn FnMut(Key) -> Option<Bits>) {
        let Some(&i) = self.index.get(&key) else { return };
        let occ = self.bricks[i as usize].occ;
        if occ == [0; 8] {
            self.index.remove(&key);
            self.bricks[i as usize].comps.clear();
            self.free.push(i);
            return;
        }
        let own = anchor(key);
        let around: [Option<u64>; 6] = std::array::from_fn(|d| anchor((key.0 + DIRS[d][0], key.1 + DIRS[d][1], key.2 + DIRS[d][2])).map(|b| faces(&b)[d ^ 1]));
        let comps = components(&occ)
            .iter()
            .map(|m| {
                let f = faces(m);
                let near = dilate(m);
                let anchored = own.is_some_and(|a| (0..8).any(|z| near[z] & a[z] != 0)) || (0..6).any(|d| around[d].is_some_and(|a| f[d] & a != 0));
                Comp { faces: f, anchored, mark: 0, held: false }
            })
            .collect();
        self.bricks[i as usize].comps = comps;
    }

    /// The components among those of bricks `starts` that no chain of touching components joins to an anchored one: each
    /// group of connected components once, with every component of it (in the lattice, not only in `starts`). Groups
    /// in the order of `starts`, their components in the order the search met them.
    pub fn detached(&mut self, starts: &[Key]) -> Vec<Vec<Node>> {
        self.stamp += 1;
        let stamp = self.stamp;
        let mut out = vec![];
        let mut stack: Vec<(u32, u16)> = vec![];
        let mut seen: Vec<(u32, u16)> = vec![];
        for key in starts {
            let Some(&bi) = self.index.get(key) else { continue };
            for ci in 0..self.bricks[bi as usize].comps.len() as u16 {
                if self.bricks[bi as usize].comps[ci as usize].mark == stamp {
                    continue;
                }
                stack.clear();
                seen.clear();
                self.bricks[bi as usize].comps[ci as usize].mark = stamp;
                self.bricks[bi as usize].comps[ci as usize].held = false;
                stack.push((bi, ci));
                let mut held = false;
                while let Some((b, c)) = stack.pop() {
                    seen.push((b, c));
                    let (key, f, anchored) = {
                        let br = &self.bricks[b as usize];
                        (br.key, br.comps[c as usize].faces, br.comps[c as usize].anchored)
                    };
                    if anchored {
                        held = true;
                        break;
                    }
                    // Down last, so it is taken first: the ground is usually below.
                    for d in [3usize, 0, 1, 4, 5, 2] {
                        if f[d] == 0 {
                            continue;
                        }
                        let Some(&ni) = self.index.get(&(key.0 + DIRS[d][0], key.1 + DIRS[d][1], key.2 + DIRS[d][2])) else { continue };
                        let nb = &mut self.bricks[ni as usize];
                        for (nc, comp) in nb.comps.iter_mut().enumerate() {
                            if comp.faces[d ^ 1] & f[d] == 0 {
                                continue;
                            }
                            if comp.mark == stamp {
                                // Met in this search (no news), or in an earlier one: then it was held (a group found
                                // loose is closed: nothing outside it touches it).
                                held |= comp.held;
                                continue;
                            }
                            comp.mark = stamp;
                            comp.held = false;
                            stack.push((ni, nc as u16));
                        }
                        if held {
                            break;
                        }
                    }
                    if held {
                        break;
                    }
                }
                for &(b, c) in seen.iter().chain(stack.iter()) {
                    self.bricks[b as usize].comps[c as usize].held = held;
                }
                if !held {
                    out.push(seen.iter().map(|&(b, c)| (self.bricks[b as usize].key, c)).collect());
                }
            }
        }
        out
    }

    /// The world cells of a component, in cell order (x fastest).
    pub fn cells(&self, node: Node) -> Vec<[i32; 3]> {
        let Some(&i) = self.index.get(&node.0) else { return vec![] };
        let Some(m) = components(&self.bricks[i as usize].occ).into_iter().nth(node.1 as usize) else { return vec![] };
        let (bx, by, bz) = (node.0 .0 * 8, node.0 .1 * 8, node.0 .2 * 8);
        let mut out = Vec::with_capacity(count(&m) as usize);
        for (z, &p) in m.iter().enumerate() {
            let mut p = p;
            while p != 0 {
                let b = p.trailing_zeros() as i32;
                p &= p - 1;
                out.push([bx + (b & 7), by + (b >> 3), bz + z as i32]);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(cells: &[[i32; 3]]) -> Bits {
        let mut m = [0u64; 8];
        for &c in cells {
            let (_, z, b) = locate(c);
            m[z] |= b;
        }
        m
    }

    /// Components against a flood fill cell by cell, on a pattern with several pieces, cells touching only across the
    /// wrap of a row (x = 7 and x = 0 of the next row are not neighbours) and diagonal cells (not connected).
    #[test]
    fn components_are_six_connected() {
        let mut cells = vec![];
        for i in 0..512usize {
            let (x, y, z) = (i & 7, (i >> 3) & 7, i >> 6);
            if (x * 7 + y * 13 + z * 29 + (x * y * z) % 5) % 3 != 0 {
                cells.push([x as i32, y as i32, z as i32]);
            }
        }
        cells.extend([[7, 0, 0], [0, 1, 0]]);
        let occ = bits(&cells);
        let comps = components(&occ);
        // Brute force.
        let mut label = [0usize; 512];
        let at = |c: [i32; 3]| (c[0] + 8 * c[1] + 64 * c[2]) as usize;
        let set = |c: [i32; 3]| (0..3).all(|k| (0..8).contains(&c[k])) && occ[c[2] as usize] >> (c[0] + 8 * c[1]) & 1 != 0;
        let mut n = 0;
        for i in 0..512 {
            let c = [(i & 7) as i32, ((i >> 3) & 7) as i32, (i >> 6) as i32];
            if !set(c) || label[i] != 0 {
                continue;
            }
            n += 1;
            let mut stack = vec![c];
            label[i] = n;
            while let Some(c) = stack.pop() {
                for d in DIRS {
                    let q = [c[0] + d[0], c[1] + d[1], c[2] + d[2]];
                    if set(q) && label[at(q)] == 0 {
                        label[at(q)] = n;
                        stack.push(q);
                    }
                }
            }
        }
        assert!(n >= 3, "{n} components");
        assert_eq!(comps.len(), n);
        for (k, m) in comps.iter().enumerate() {
            for i in 0..512 {
                assert_eq!(m[i >> 6] >> (i & 63) & 1 != 0, label[i] == k + 1, "cell {i} of component {k}");
            }
        }
        // Two diagonal cells are two components; a row's end and the next row's start too.
        assert_eq!(components(&bits(&[[0, 0, 0], [1, 1, 0]])).len(), 2);
        assert_eq!(components(&bits(&[[7, 0, 0], [0, 1, 0]])).len(), 2);
        assert_eq!(components(&bits(&[[3, 3, 3], [3, 3, 4], [3, 4, 4]])).len(), 1);
    }

    #[test]
    fn faces_match_across_bricks() {
        // Cell (7, 2, 5) of one brick faces cell (0, 2, 5) of the next along x: the same face bit; likewise y and z.
        for (a, b, d) in [([7, 2, 5], [0, 2, 5], 1usize), ([3, 7, 6], [3, 0, 6], 3), ([4, 1, 7], [4, 1, 0], 5)] {
            let (fa, fb) = (faces(&bits(&[a])), faces(&bits(&[b])));
            assert_ne!(fa[d], 0);
            assert_eq!(fa[d], fb[d ^ 1]);
            assert_eq!(fa.iter().filter(|f| **f != 0).count(), 1);
        }
        let full = faces(&[u64::MAX; 8]);
        assert!(full.iter().all(|f| *f == u64::MAX));
    }

    /// A column over three bricks standing on an anchor below; cut in the middle, the upper part is loose and the lower
    /// held. A second column beside it, never touched by the edit's bricks, is not reported.
    #[test]
    fn loose_components_are_found() {
        let mut l = Lattice::default();
        l.set_cells((0..24).flat_map(|y| [[2, y, 2], [20, y, 2]]), &mut vec![]);
        // The anchor: the layer y = -1 (the top plane of the bricks below).
        let mut ground = |k: Key| (k.1 == -1).then(|| {
            let mut b = [0u64; 8];
            for z in 0..8 {
                b[z] = 0xff << 56;
            }
            b
        });
        let keys: Vec<Key> = (0..3).flat_map(|y| [(0, y, 0), (2, y, 0)]).collect();
        for &k in &keys {
            l.recompute(k, &mut ground);
        }
        assert_eq!(l.len(), 6);
        assert!(l.detached(&keys).is_empty(), "both stand on the ground");
        // Cut the first column at y = 10..12.
        for y in 10..12 {
            l.clear([2, y, 2]);
        }
        l.recompute((0, 1, 0), &mut ground);
        let loose = l.detached(&[(0, 1, 0)]);
        assert_eq!(loose.len(), 1);
        let mut cells: Vec<[i32; 3]> = loose[0].iter().flat_map(|&n| l.cells(n)).collect();
        cells.sort();
        assert_eq!(cells, (12..24).map(|y| [2, y, 2]).collect::<Vec<_>>());
        // The same search again gives the same answer; from the top brick alone too.
        assert_eq!(l.detached(&[(0, 1, 0)]), loose);
        assert_eq!(l.detached(&[(0, 2, 0)]).len(), 1);
        // The lower part and the other column are held.
        assert!(l.detached(&[(0, 0, 0), (2, 0, 0), (2, 1, 0), (2, 2, 0)]).is_empty());
        // Clearing a whole brick's cells drops it.
        for y in 16..24 {
            l.clear([2, y, 2]);
        }
        l.recompute((0, 2, 0), &mut ground);
        assert!(!l.has((0, 2, 0)));
        assert_eq!(l.len(), 5);
    }
}
