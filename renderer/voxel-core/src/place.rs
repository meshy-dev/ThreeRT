//! Where a voxel object stands in the world lattice (ROADMAP § 6 step 6): every object's voxels are cells of one world
//! grid of the voxel size, the object turned about y by quarter turns and moved by whole voxels. Integers only, so
//! connectivity across objects and edits in world space are exact.

/// The object's cell g is world cell `turn(g) + t`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Placement {
    /// Quarter turns about y (0..3), counter-clockwise seen from above (+x toward -z).
    pub turn: u8,
    pub t: [i32; 3],
}

/// Cell `c` turned about the y axis through the lattice's origin: a cell is the unit cube from its corner, so a turn
/// moves it to the cell whose corner the turned cube has.
fn turn_cell(turn: u8, c: [i32; 3]) -> [i32; 3] {
    match turn & 3 {
        0 => c,
        1 => [c[2], c[1], -c[0] - 1],
        2 => [-c[0] - 1, c[1], -c[2] - 1],
        _ => [-c[2] - 1, c[1], c[0]],
    }
}

impl Placement {
    /// An object whose grid's cell (0, 0, 0) is cell `off` of its own space, turned by `turn` quarter turns and then
    /// moved by `pos` (voxels): as a transform of rotation and translation places it.
    pub fn new(pos: [i32; 3], turn: i32, off: [i32; 3]) -> Placement {
        let turn = turn.rem_euclid(4) as u8;
        let o = turn_cell(turn, off);
        Placement { turn, t: [o[0] + pos[0], o[1] + pos[1], o[2] + pos[2]] }
    }

    fn lin(&self, g: [i32; 3]) -> [i32; 3] {
        match self.turn & 3 {
            0 => g,
            1 => [g[2], g[1], -g[0]],
            2 => [-g[0], g[1], -g[2]],
            _ => [-g[2], g[1], g[0]],
        }
    }

    /// The world cell of the object's cell `g`.
    pub fn to_world(&self, g: [i32; 3]) -> [i32; 3] {
        let l = self.lin(g);
        [l[0] + self.t[0], l[1] + self.t[1], l[2] + self.t[2]]
    }

    /// The object's cell at world cell `w`.
    pub fn to_grid(&self, w: [i32; 3]) -> [i32; 3] {
        let d = [w[0] - self.t[0], w[1] - self.t[1], w[2] - self.t[2]];
        match self.turn & 3 {
            0 => d,
            1 => [-d[2], d[1], d[0]],
            2 => [-d[0], d[1], -d[2]],
            _ => [d[2], d[1], -d[0]],
        }
    }

    /// The cells between two object cells (inclusive), as world cells (inclusive, in order).
    pub fn box_to_world(&self, lo: [i32; 3], hi: [i32; 3]) -> ([i32; 3], [i32; 3]) {
        let (a, b) = (self.to_world(lo), self.to_world(hi));
        ([0, 1, 2].map(|k| a[k].min(b[k])), [0, 1, 2].map(|k| a[k].max(b[k])))
    }

    /// The cells between two world cells (inclusive), as the object's cells (inclusive, in order).
    pub fn box_to_grid(&self, lo: [i32; 3], hi: [i32; 3]) -> ([i32; 3], [i32; 3]) {
        let (a, b) = (self.to_grid(lo), self.to_grid(hi));
        ([0, 1, 2].map(|k| a[k].min(b[k])), [0, 1, 2].map(|k| a[k].max(b[k])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A placed cell is where the float transform puts the cell's cube: the cell of the cube's centre, for every turn;
    /// `to_grid` inverts `to_world`.
    #[test]
    fn cells_follow_the_transform() {
        for turn in 0..4 {
            let (pos, off) = ([7, -3, 12], [-10, 2, -6]);
            let p = Placement::new(pos, turn, off);
            let a = turn as f64 * std::f64::consts::FRAC_PI_2;
            for g in [[0, 0, 0], [1, 0, 0], [0, 5, 3], [-4, 2, 9], [13, -7, -2]] {
                // The centre of the object-space cube, rotated about y (x' = x cos + z sin, z' = -x sin + z cos), moved.
                let c = [g[0] as f64 + off[0] as f64 + 0.5, g[1] as f64 + off[1] as f64 + 0.5, g[2] as f64 + off[2] as f64 + 0.5];
                let w = [c[0] * a.cos() + c[2] * a.sin() + pos[0] as f64, c[1] + pos[1] as f64, -c[0] * a.sin() + c[2] * a.cos() + pos[2] as f64];
                let cell = w.map(|v| v.floor() as i32);
                assert_eq!(p.to_world(g), cell, "turn {turn} cell {g:?}");
                assert_eq!(p.to_grid(cell), g);
            }
            let (lo, hi) = p.box_to_world([0, 0, 0], [3, 1, 5]);
            assert!((0..3).all(|k| lo[k] <= hi[k]));
            assert_eq!(p.box_to_grid(lo, hi), ([0, 0, 0], [3, 1, 5]));
        }
        assert_eq!(Placement::new([0; 3], -1, [0; 3]).turn, 3);
    }
}
