//! A voxel object: sparse 8³ bricks, a byte per voxel (0 empty, 1..255 a palette entry).

use std::collections::BTreeMap;

pub const BRICK: i32 = 8;

/// Brick coordinates -> its 512 bytes (voxel x + 8 y + 64 z). A BTreeMap: iteration (layout, meshing) is in a fixed
/// order, so the same voxels always give the same data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct VoxelGrid {
    pub bricks: BTreeMap<(i32, i32, i32), Box<[u8; 512]>>,
}

const fn local(x: i32, y: i32, z: i32) -> usize {
    ((x & 7) + 8 * (y & 7) + 64 * (z & 7)) as usize
}

impl VoxelGrid {
    pub fn get(&self, x: i32, y: i32, z: i32) -> u8 {
        self.bricks.get(&(x >> 3, y >> 3, z >> 3)).map_or(0, |b| b[local(x, y, z)])
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, m: u8) {
        let key = (x >> 3, y >> 3, z >> 3);
        if m == 0 {
            if let Some(b) = self.bricks.get_mut(&key) {
                b[local(x, y, z)] = 0;
            }
            return;
        }
        self.bricks.entry(key).or_insert_with(|| Box::new([0; 512]))[local(x, y, z)] = m;
    }

    /// Voxels set.
    pub fn count(&self) -> usize {
        self.bricks.values().map(|b| b.iter().filter(|&&v| v != 0).count()).sum()
    }

    /// Voxel bounds [lo, hi) of the non-empty bricks (None: empty).
    pub fn bounds(&self) -> Option<([i32; 3], [i32; 3])> {
        let mut it = self.bricks.iter().filter(|(_, b)| b.iter().any(|&v| v != 0)).map(|(k, _)| *k);
        let first = it.next()?;
        let (mut lo, mut hi) = ([first.0, first.1, first.2], [first.0, first.1, first.2]);
        for (x, y, z) in it {
            for (a, v) in [x, y, z].into_iter().enumerate() {
                lo[a] = lo[a].min(v);
                hi[a] = hi[a].max(v);
            }
        }
        Some((lo.map(|v| v * BRICK), hi.map(|v| (v + 1) * BRICK)))
    }

    /// Empties voxel (x, y, z); returns what it held. A brick left empty stays until `prune`.
    pub fn take(&mut self, x: i32, y: i32, z: i32) -> u8 {
        match self.bricks.get_mut(&(x >> 3, y >> 3, z >> 3)) {
            Some(b) => std::mem::take(&mut b[local(x, y, z)]),
            None => 0,
        }
    }

    /// Drops brick `key` if it holds no voxel.
    pub fn prune(&mut self, key: (i32, i32, i32)) {
        if self.bricks.get(&key).is_some_and(|b| b.iter().all(|&v| v == 0)) {
            self.bricks.remove(&key);
        }
    }

    /// Every voxel with its coordinates, bricks in key order and voxels in index order (x fastest).
    pub fn voxels(&self) -> impl Iterator<Item = ([i32; 3], u8)> + '_ {
        self.bricks.iter().flat_map(|(k, b)| b.iter().enumerate().filter(|(_, m)| **m != 0).map(move |(v, &m)| ([k.0 * 8 + (v & 7) as i32, k.1 * 8 + ((v >> 3) & 7) as i32, k.2 * 8 + (v >> 6) as i32], m)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_empties_voxels_and_prune_drops_empty_bricks() {
        let mut g = VoxelGrid::default();
        g.set(1, 2, 3, 5);
        g.set(9, 2, 3, 6);
        assert_eq!(g.voxels().collect::<Vec<_>>(), vec![([1, 2, 3], 5), ([9, 2, 3], 6)]);
        assert_eq!(g.take(1, 2, 3), 5);
        assert_eq!(g.take(1, 2, 3), 0);
        assert_eq!(g.bricks.len(), 2);
        g.prune((0, 0, 0));
        g.prune((1, 0, 0));
        assert_eq!(g.bricks.len(), 1);
        assert_eq!(g.count(), 1);
    }
}
