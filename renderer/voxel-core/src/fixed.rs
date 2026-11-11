//! Fixed point for edits (ROADMAP § 6 step 6): positions and radii in 1 / 256 of a voxel, as integers, so the same
//! command removes the same voxels on every machine and in the browser (wasm32). Nothing here uses floats but `to_fp`,
//! which turns a command's numbers into fixed point once, where the command enters.

/// Fixed-point units per voxel.
pub const FP: i32 = 256;
/// The largest radius an edit takes (voxels), and the most cells its bounds may hold: an edit's cost is bounded
/// whatever the game asks for.
pub const MAX_RADIUS: i32 = 128;
pub const MAX_CELLS: i64 = 1 << 27;

/// `voxels` (a position or a length in voxels) in fixed point, rounded to the nearest unit and kept in range.
pub fn to_fp(voxels: f64) -> i32 {
    if voxels.is_nan() {
        return 0;
    }
    (voxels * FP as f64).round().clamp(-(1i64 << 30) as f64, (1i64 << 30) as f64) as i32
}

/// What an edit covers, in world fixed point. A voxel is inside when its centre is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Sphere { c: [i32; 3], r: i32 },
    /// The box between two corners (inclusive).
    Box { lo: [i32; 3], hi: [i32; 3] },
    /// Everything within `r` of the segment a-b: a line cut.
    Capsule { a: [i32; 3], b: [i32; 3], r: i32 },
}

fn floor_cell(v: i32) -> i32 {
    v.div_euclid(FP)
}

impl Shape {
    /// The shape with its radius within `MAX_RADIUS` and its corners in order; None if its bounds hold more than
    /// `MAX_CELLS` cells.
    pub fn checked(self) -> Option<Shape> {
        let cap = MAX_RADIUS * FP;
        let s = match self {
            Shape::Sphere { c, r } => Shape::Sphere { c, r: r.clamp(0, cap) },
            Shape::Box { lo, hi } => Shape::Box { lo: [0, 1, 2].map(|k| lo[k].min(hi[k])), hi: [0, 1, 2].map(|k| lo[k].max(hi[k])) },
            Shape::Capsule { a, b, r } => Shape::Capsule { a, b, r: r.clamp(0, cap) },
        };
        let (lo, hi) = s.cells();
        // (In i128: three sides of up to 2^23 cells are 2^69, which an i64 wraps to 0.)
        let n: i128 = (0..3).map(|k| (hi[k] as i128 - lo[k] as i128 + 1).max(0)).product();
        (n <= MAX_CELLS as i128).then_some(s)
    }

    /// The world cells that can hold a centre inside (inclusive on both ends), at the full radius.
    pub fn cells(&self) -> ([i32; 3], [i32; 3]) {
        let (lo, hi) = match *self {
            Shape::Sphere { c, r } => (c.map(|v| v.saturating_sub(r)), c.map(|v| v.saturating_add(r))),
            Shape::Box { lo, hi } => (lo, hi),
            Shape::Capsule { a, b, r } => ([0, 1, 2].map(|k| a[k].min(b[k]).saturating_sub(r)), [0, 1, 2].map(|k| a[k].max(b[k]).saturating_add(r))),
        };
        (lo.map(floor_cell), hi.map(floor_cell))
    }

    /// Whether the centre of world cell `cell` is inside, the radius scaled by `scale` / 256 (a box is not scaled;
    /// scale 0 holds nothing).
    pub fn contains(&self, cell: [i32; 3], scale: i32) -> bool {
        if scale <= 0 {
            return false;
        }
        let p = cell.map(|v| v as i64 * FP as i64 + FP as i64 / 2);
        match *self {
            Shape::Sphere { c, r } => {
                let r = r as i64 * scale as i64 / 256;
                let d = [0, 1, 2].map(|k| p[k] - c[k] as i64);
                d.iter().all(|v| v.abs() <= r) && d.iter().map(|v| v * v).sum::<i64>() <= r * r
            }
            Shape::Box { lo, hi } => (0..3).all(|k| p[k] >= lo[k] as i64 && p[k] <= hi[k] as i64),
            Shape::Capsule { a, b, r } => {
                let r = r as i128 * scale as i128 / 256;
                let ab = [0, 1, 2].map(|k| b[k] as i128 - a[k] as i128);
                let ap = [0, 1, 2].map(|k| p[k] as i128 - a[k] as i128);
                let len2: i128 = ab.iter().map(|v| v * v).sum();
                let ap2: i128 = ap.iter().map(|v| v * v).sum();
                if len2 == 0 {
                    return ap2 <= r * r;
                }
                // The nearest point of the segment is a + ab t / len2, t in 0..len2; the squared distance times len2.
                let t = (0..3).map(|k| ap[k] * ab[k]).sum::<i128>().clamp(0, len2);
                let dot: i128 = (0..3).map(|k| ap[k] * ab[k]).sum();
                ap2 * len2 - 2 * t * dot + t * t <= r * r * len2
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sphere holds the voxels whose centres are within its radius (against the same test in floats, at a radius where
    /// no centre lies on the surface), a scale shrinks it, and scale 0 holds nothing.
    #[test]
    fn sphere_holds_centres_within_its_radius() {
        let s = Shape::Sphere { c: [to_fp(8.0), to_fp(8.0), to_fp(8.0)], r: to_fp(3.1) };
        let inside = |x: i32, y: i32, z: i32, r: f64| ((x as f64 + 0.5 - 8.0).powi(2) + (y as f64 + 0.5 - 8.0).powi(2) + (z as f64 + 0.5 - 8.0).powi(2)) <= r * r;
        let (lo, hi) = s.cells();
        assert_eq!((lo, hi), ([4, 4, 4], [11, 11, 11]));
        let mut n = 0;
        for z in 0..16 {
            for y in 0..16 {
                for x in 0..16 {
                    assert_eq!(s.contains([x, y, z], 256), inside(x, y, z, 3.1), "{x} {y} {z}");
                    assert_eq!(s.contains([x, y, z], 128), inside(x, y, z, 1.55), "{x} {y} {z} at half");
                    assert!(!s.contains([x, y, z], 0));
                    n += s.contains([x, y, z], 256) as usize;
                    if s.contains([x, y, z], 256) {
                        assert!((0..3).all(|k| [x, y, z][k] >= lo[k] && [x, y, z][k] <= hi[k]));
                    }
                }
            }
        }
        assert!(n > 100);
    }

    #[test]
    fn box_and_capsule() {
        let b = Shape::Box { lo: [to_fp(2.0), to_fp(-1.0), to_fp(0.0)], hi: [to_fp(1.0), to_fp(3.0), to_fp(0.6)] }.checked().unwrap();
        // Cells whose centres (0.5 past the corner) lie in x 1..2, y -1..3, z 0..0.6.
        assert!(b.contains([1, -1, 0], 256) && b.contains([1, 2, 0], 256));
        assert!(!b.contains([0, 0, 0], 256) && !b.contains([2, 0, 0], 256) && !b.contains([1, 3, 0], 256) && !b.contains([1, 0, 1], 256));
        // A line along x from 0 to 10 at y = z = 0.5, radius 1.2: centres within 1.2 of it, round at its ends.
        let c = Shape::Capsule { a: [0, 128, 128], b: [to_fp(10.0), 128, 128], r: to_fp(1.2) };
        assert!(c.contains([5, 0, 0], 256) && c.contains([5, 1, 0], 256) && c.contains([5, 0, -1], 256));
        assert!(!c.contains([5, 2, 0], 256) && !c.contains([5, 1, 1], 256), "1.41 away");
        assert!(c.contains([-1, 0, 0], 256) && !c.contains([-2, 0, 0], 256) && c.contains([10, 0, 0], 256) && !c.contains([11, 0, 0], 256));
        let (lo, hi) = c.cells();
        assert_eq!((lo, hi), ([-2, -1, -1], [11, 1, 1]));
        // A segment of no length is a sphere.
        let p = Shape::Capsule { a: [128; 3], b: [128; 3], r: to_fp(1.0) };
        assert!(p.contains([0, 0, 0], 256) && p.contains([1, 0, 0], 256) && !p.contains([1, 1, 0], 256));
    }

    #[test]
    fn edits_are_bounded() {
        assert_eq!(Shape::Sphere { c: [0; 3], r: i32::MAX }.checked(), Some(Shape::Sphere { c: [0; 3], r: MAX_RADIUS * FP }));
        // The largest box there is (every corner clamped by `to_fp`): 2^69 cells, refused, not wrapped to 0.
        let (a, b) = (to_fp(-1e30), to_fp(1e30));
        assert_eq!(Shape::Box { lo: [a; 3], hi: [b; 3] }.checked(), None);
        assert_eq!(Shape::Box { lo: [0; 3], hi: [to_fp(600.0); 3] }.checked(), None);
        assert_eq!(to_fp(f64::NAN), 0);
        assert_eq!(to_fp(1e30), 1 << 30);
        assert_eq!((to_fp(0.5), to_fp(-0.5), to_fp(1.0 / 512.0 + 1e-9)), (128, -128, 1));
    }
}
