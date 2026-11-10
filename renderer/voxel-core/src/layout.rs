//! A voxel object as procedural geometry data, the words `VOXEL_BRICKS` reads (packages/voxel/src/grid.ts, the same
//! layout): from the data start,
//!   0..3   brick grid size bx, by, bz; voxel size (m, f32 bits)
//!   4..6   object-space position of the grid's voxel (0, 0, 0) corner (f32 bits)
//!   7      first word of the brick grid: bx * by * bz words, 0 = an empty brick, k = brick record k - 1
//!   8      first word of the brick records: per brick 16 words of occupancy (bit x + 8 y + 64 z), then its material word
//!          (bit 31 set: every voxel is material bits 0..7; else the word offset of 128 words, a byte per voxel)
//!   9      brick count; 10, 11 reserved
//! and boxes (6 floats each) per non-empty brick, per non-empty 32³ region or one for the object.

use crate::grid::{VoxelGrid, BRICK};

pub const HEADER_WORDS: usize = 12;
const BRICK_WORDS: usize = 17;

/// Boxes the hardware traverses: one per non-empty 8³ brick, per non-empty 32³ region, or one for the object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Granularity {
    Brick,
    Region,
    Object,
}

/// A grid's procedural geometry: boxes (min xyz, max xyz) and data words.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    pub boxes: Vec<f32>,
    pub data: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutError(pub String);

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "voxel layout: {}", self.0)
    }
}

impl std::error::Error for LayoutError {}

/// The grid of a layout's data words (as an asset file holds them; every index is checked: files come from games),
/// with the object-space corner of its voxel (0, 0, 0) and the voxel size.
pub fn decode(data: &[u32]) -> Result<(VoxelGrid, [f32; 3], f32), LayoutError> {
    let err = |m: &str| LayoutError(m.to_string());
    if data.len() < HEADER_WORDS {
        return Err(err("shorter than its header"));
    }
    let (bx, by, bz) = (data[0] as usize, data[1] as usize, data[2] as usize);
    let size = f32::from_bits(data[3]);
    let corner = [f32::from_bits(data[4]), f32::from_bits(data[5]), f32::from_bits(data[6])];
    let (grid_at, bricks_at, count) = (data[7] as usize, data[8] as usize, data[9] as usize);
    if !(size.is_finite() && size > 0.0) || corner.iter().any(|c| !c.is_finite()) {
        return Err(err("voxel size or corner not finite"));
    }
    let cells = bx.checked_mul(by).and_then(|v| v.checked_mul(bz)).ok_or_else(|| err("brick grid too large"))?;
    if bx > 4096 || by > 4096 || bz > 4096 || grid_at.checked_add(cells).is_none_or(|e| e > data.len()) {
        return Err(err("brick grid out of the data"));
    }
    if bricks_at.checked_add(count.checked_mul(BRICK_WORDS).ok_or_else(|| err("brick count too large"))?).is_none_or(|e| e > data.len()) {
        return Err(err("brick records out of the data"));
    }
    let mut g = VoxelGrid::default();
    for cell in 0..cells {
        let k = data[grid_at + cell] as usize;
        if k == 0 {
            continue;
        }
        if k > count {
            return Err(err("brick index past the brick count"));
        }
        let w = bricks_at + (k - 1) * BRICK_WORDS;
        let mat = data[w + 16];
        let bytes = if mat & 0x8000_0000 == 0 {
            let at = mat as usize;
            if at.checked_add(128).is_none_or(|e| e > data.len()) {
                return Err(err("brick materials out of the data"));
            }
            Some(&data[at..at + 128])
        } else {
            None
        };
        let (x, y, z) = ((cell % bx) as i32, ((cell / bx) % by) as i32, (cell / (bx * by)) as i32);
        let mut b = Box::new([0u8; 512]);
        for (v, out) in b.iter_mut().enumerate() {
            if (data[w + (v >> 5)] >> (v & 31)) & 1 == 0 {
                continue;
            }
            *out = match bytes {
                None => (mat & 0xff) as u8,
                Some(m) => ((m[v >> 2] >> (8 * (v & 3))) & 0xff) as u8,
            };
        }
        if b.iter().any(|&v| v != 0) {
            g.bricks.insert((x, y, z), b);
        }
    }
    Ok((g, corner, size))
}

/// The grid as procedural geometry data (the header above), voxel (0, 0, 0)'s corner at `origin` (object space) and
/// voxels of `size` m; the brick grid starts at the lowest brick, rounded down to a 32³ region. Bricks in key order.
pub fn layout(g: &VoxelGrid, granularity: Granularity, origin: [f32; 3], size: f32) -> Layout {
    let list: Vec<(&(i32, i32, i32), &Box<[u8; 512]>)> = g.bricks.iter().filter(|(_, b)| b.iter().any(|&v| v != 0)).collect();
    if list.is_empty() {
        return Layout { boxes: vec![], data: vec![0; HEADER_WORDS] };
    }
    let (mut lo, mut hi) = ([i32::MAX; 3], [i32::MIN; 3]);
    for (k, _) in &list {
        for (a, v) in [k.0, k.1, k.2].into_iter().enumerate() {
            lo[a] = lo[a].min(v);
            hi[a] = hi[a].max(v);
        }
    }
    let lo = lo.map(|v| v.div_euclid(4) * 4);
    let b = [0, 1, 2].map(|a| (hi[a] - lo[a] + 1) as usize);
    let s = size as f64;
    let corner = [0, 1, 2].map(|a| (origin[a] as f64 + (lo[a] * BRICK) as f64 * s) as f32);
    let grid_at = HEADER_WORDS;
    let bricks_at = grid_at + b[0] * b[1] * b[2];
    let mixed = list.iter().filter(|(_, br)| !uniform(br)).count();
    let mut data = vec![0u32; bricks_at + list.len() * BRICK_WORDS + mixed * 128];
    data[..HEADER_WORDS].copy_from_slice(&[b[0] as u32, b[1] as u32, b[2] as u32, size.to_bits(), corner[0].to_bits(), corner[1].to_bits(), corner[2].to_bits(), grid_at as u32, bricks_at as u32, list.len() as u32, 0, 0]);
    let mut mat_at = bricks_at + list.len() * BRICK_WORDS;
    for (k, (key, br)) in list.iter().enumerate() {
        let cell = (key.0 - lo[0]) as usize + b[0] * ((key.1 - lo[1]) as usize + b[1] * (key.2 - lo[2]) as usize);
        data[grid_at + cell] = k as u32 + 1;
        let w = bricks_at + k * BRICK_WORDS;
        for (v, &m) in br.iter().enumerate() {
            if m != 0 {
                data[w + (v >> 5)] |= 1 << (v & 31);
            }
        }
        if uniform(br) {
            data[w + 16] = 0x8000_0000 | br[0] as u32;
        } else {
            data[w + 16] = mat_at as u32;
            for (v, &m) in br.iter().enumerate() {
                data[mat_at + (v >> 2)] |= (m as u32) << (8 * (v & 3));
            }
            mat_at += 128;
        }
    }
    let edge = (BRICK as f64) * s;
    let bx = |x: i32, y: i32, z: i32, n: i32| -> [f32; 6] {
        let c = |a: usize, v: i32| (corner[a] as f64 + v as f64 * edge) as f32;
        [c(0, x), c(1, y), c(2, z), c(0, x + n), c(1, y + n), c(2, z + n)]
    };
    let boxes = match granularity {
        Granularity::Object => {
            let far = [0, 1, 2].map(|a| (corner[a] as f64 + b[a] as f64 * edge) as f32);
            vec![corner[0], corner[1], corner[2], far[0], far[1], far[2]]
        }
        Granularity::Brick => list.iter().flat_map(|(k, _)| bx(k.0 - lo[0], k.1 - lo[1], k.2 - lo[2], 1)).collect(),
        Granularity::Region => {
            let mut seen = std::collections::BTreeSet::new();
            let mut out = vec![];
            for (k, _) in &list {
                let r = [k.0 - lo[0], k.1 - lo[1], k.2 - lo[2]].map(|v| v.div_euclid(4) * 4);
                if seen.insert(r) {
                    out.extend(bx(r[0], r[1], r[2], 4));
                }
            }
            out
        }
    };
    Layout { boxes, data }
}

/// Every voxel set, one material: the brick record holds the material, no bytes.
fn uniform(b: &[u8; 512]) -> bool {
    b[0] != 0 && b.iter().all(|&v| v == b[0])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> VoxelGrid {
        let mut g = VoxelGrid::default();
        for z in 0..8 {
            for y in 0..8 {
                for x in 0..16 {
                    g.set(x, y, z, 3); // two uniform bricks
                }
            }
        }
        for z in 1..4 {
            for y in 2..9 {
                for x in -5..-1 {
                    g.set(x, y, z, 7); // negative coordinates, partial bricks
                }
            }
        }
        g.set(20, 3, -9, 200);
        g.set(21, 3, -9, 201);
        g
    }

    #[test]
    fn layout_round_trips_every_voxel() {
        let g = sample();
        for gran in [Granularity::Brick, Granularity::Region, Granularity::Object] {
            let l = layout(&g, gran, [1.0, 2.0, 3.0], 0.1);
            let (back, corner, size) = decode(&l.data).unwrap();
            assert_eq!(size, 0.1);
            assert_eq!(back.count(), g.count());
            // The decoded grid starts at the layout's corner: shift back by its offset in voxels.
            let off = [0, 1, 2].map(|a| ((corner[a] - [1.0, 2.0, 3.0][a]) / 0.1).round() as i32);
            for (k, b) in &g.bricks {
                for v in 0..512 {
                    let (x, y, z) = (k.0 * 8 + (v as i32 & 7), k.1 * 8 + ((v as i32 >> 3) & 7), k.2 * 8 + (v as i32 >> 6));
                    assert_eq!(back.get(x - off[0], y - off[1], z - off[2]), b[v], "{gran:?} voxel {x} {y} {z}");
                }
            }
        }
        let l = layout(&g, Granularity::Object, [0.0; 3], 0.1);
        assert_eq!(l.boxes.len(), 6);
        assert_eq!(layout(&g, Granularity::Brick, [0.0; 3], 0.1).boxes.len() / 6, g.bricks.len());
    }

    #[test]
    fn malformed_data_is_refused() {
        let good = layout(&sample(), Granularity::Brick, [0.0; 3], 0.1).data;
        assert!(decode(&good[..5]).is_err(), "short");
        let mut d = good.clone();
        d[0] = 100_000;
        assert!(decode(&d).is_err(), "grid past the data");
        let mut d = good.clone();
        d[9] = 1 << 30;
        assert!(decode(&d).is_err(), "records past the data");
        let mut d = good.clone();
        let cell = d[7] as usize;
        let first = (cell..cell + (good[0] * good[1] * good[2]) as usize).find(|&i| d[i] != 0).unwrap();
        d[first] = good[9] + 1;
        assert!(decode(&d).is_err(), "brick index past the count");
        let mut d = good.clone();
        let rec = d[8] as usize;
        let mixed = (0..good[9] as usize).map(|k| rec + k * 17 + 16).find(|&w| d[w] & 0x8000_0000 == 0).unwrap();
        d[mixed] = u32::MAX >> 1;
        assert!(decode(&d).is_err(), "materials past the data");
        let mut d = good;
        d[3] = f32::NAN.to_bits();
        assert!(decode(&d).is_err(), "voxel size");
    }
}
