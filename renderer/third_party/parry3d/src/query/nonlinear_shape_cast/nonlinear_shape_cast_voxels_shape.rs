use crate::bounding_volume::{Aabb, BoundingVolume};
use crate::math::{Pose, Real, Vector};
use crate::query::sweep_toi::{sweep_time_of_impact, Sweep, SweepToiStatus, ToiProxy};
use crate::query::{NonlinearRigidMotion, QueryDispatcher, ShapeCastHit};
use crate::shape::{Cuboid, Shape, Voxels};
use alloc::collections::BinaryHeap;
use alloc::vec::Vec;
use core::cmp::{Ordering, Reverse};

/// Time Of Impact of a voxels shape with any other shape, under a rigid motion (translation + rotation).
pub fn cast_shapes_nonlinear_voxels_shape<D>(
    dispatcher: &D,
    motion1: &NonlinearRigidMotion,
    g1: &Voxels,
    motion2: &NonlinearRigidMotion,
    g2: &dyn Shape,
    start_time: Real,
    end_time: Real,
    stop_at_penetration: bool,
) -> Option<ShapeCastHit>
where
    D: ?Sized + QueryDispatcher,
{
    // HACK: really supporting nonlinear shape-casting on a rotating voxels shape would
    //       be extremely inefficient without any sort of hierarchical traversal on the voxel.
    //       So for now we assume that the voxels shape only has a translational motion.
    //       We can fix that once we introduce a sparse representation (and its accompanying
    //       acceleration structure) for the voxels shape.
    let mut motion2 = *motion2;
    let mut motion1 = *motion1;
    motion2.linvel -= motion1.linvel;
    motion1.freeze(start_time);
    let pos1 = motion1.position_at_time(start_time);

    // (ThreeRT, renderer/third_party/parry3d/PATCH.md "Sweeps") Every cell the shape can touch over the times is
    // found, and only those it can touch first are cast, in the way composite shapes are swept (a bound on each part,
    // then exact work on the parts left, earliest first):
    //
    // 1. The walk: the shape's box at the start (in the voxel shape's frame), grown by how far the turn can take any
    //    of its points (the turn over the times times the farthest point's distance from the motion's centre of
    //    rotation) and a margin, moves with the shape's velocity in the voxel shape's frame. Row by row along the axis
    //    it moves most along, the cells of the row it overlaps while it is in the row are walked: every cell it
    //    overlaps at some time, once.
    // 2. A cell is culled against the shape's own box (its local AABB) oriented as at the start and grown on every
    //    side by how far any of its points can move (the linear speed times the time, plus the turn's reach): the
    //    shape is inside it all the while, so a cell outside it (by a face axis of either box) is never touched.
    // 3. The shape is then taken moving without turning: at its orientation halfway, its centre (its box's) on the
    //    straight line between where the motion has it at the start and at the end. The motion has any point of the
    //    shape at most `r θ²/8 (1 + θ/2)` (`r` the centre's distance from the centre of rotation, `θ` the turn over the
    //    times: how far an arc strays from its chord at the same fraction of it; `a_turning_centre_keeps_to_its_chord`
    //    holds it) plus `d θ/2` (`d` the farthest point's distance from the centre: the turn either side of halfway)
    //    from there. A cell left is bounded by when its box, grown by that and a margin, first meets the shape's box
    //    so moving (the slab test): a cell it never meets is never touched.
    // 4. In the order of those bounds, while a bound is not past the earliest hit so far, a cell is bounded again by
    //    a conservative-advancement time of impact (`sweep_time_of_impact`, as composite shapes are swept) of its box,
    //    so grown, against the shape's proxy so moving. Without a turn the separation along each axis the advance
    //    takes changes linearly, so the advance is exact: the bound is at or before the cell's contact, and a cell it
    //    finds never within the margin is never touched. A cell whose bound is not past the earliest hit is cast
    //    exactly, as every cell was (the same nonlinear cast over the same times; a shape with no point-cloud proxy
    //    is cast without the second bound). Of the hits, the earliest is kept and, of equal ones, the first cell
    //    walked. That is the hit casting every solid cell gives, to the bit, but where the cast reports a cell's hit
    //    before the cell's contact (it can, by millimetres, for a shape turning fast) and before the cell's bound: the
    //    cell is then not cast, and the answer is later, nearer the contact.
    //
    // (A cell overlapping the shape at the start is bounded at the start time, so it is always cast.)
    //
    // Upstream's walk moved a window of the start box's size along the velocity's axes in the world's frame, both
    // ends together, and cast every cell it held: it missed cells where the box straddles one more row than it did at
    // the start, where the voxel shape is turned, and where the shape turns about a centre off it (a car's part).
    let span = (end_time - start_time).max(0.0);
    let start_pos2_1 = pos1.inv_mul(&motion2.position_at_time(start_time));
    let local2 = g2.compute_local_aabb();
    let (local_center2, local_half2) = (local2.center(), local2.half_extents());
    // (The shape's centre's distance from the motion's centre of rotation, and the farthest any of its points is.)
    let arm = (local_center2 - motion2.local_center).length();
    let radius2 = arm + local_half2.length();
    let turn = motion2.angvel.length() * span;
    let half = g1.voxel_size() / 2.0;
    let margin = 0.02 * half.min_element() * 2.0;
    let linvel2_1 = pos1.rotation.inverse() * motion2.linvel;
    let box0 = g2.compute_aabb(&start_pos2_1).loosened(turn * radius2 + margin);

    let cull = {
        let rot = start_pos2_1.rotation;
        let axes = [rot * Vector::X, rot * Vector::Y, rot * Vector::Z];
        let reach = motion2.linvel.length() * span + turn * radius2 + margin;
        (start_pos2_1 * local_center2, local_half2 + Vector::splat(reach), axes)
    };
    let culled = |center: Vector| {
        let (c, h, axes) = &cull;
        let d = center - *c;
        let a = [axes[0].abs(), axes[1].abs(), axes[2].abs()];
        // (The cell's face axes, then the box's.)
        (0..3).any(|i| d[i].abs() > half[i] + a[0][i] * h.x + a[1][i] * h.y + a[2][i] * h.z)
            || (0..3).any(|j| d.dot(axes[j]).abs() > h[j] + a[j].dot(half))
    };
    let proxy2 = if span > 0.0 { ToiProxy::from_shape(g2) } else { None };
    let corners = [
        Vector::new(-half.x, -half.y, -half.z),
        Vector::new(half.x, -half.y, -half.z),
        Vector::new(half.x, half.y, -half.z),
        Vector::new(-half.x, half.y, -half.z),
        Vector::new(-half.x, -half.y, half.z),
        Vector::new(half.x, -half.y, half.z),
        Vector::new(half.x, half.y, half.z),
        Vector::new(-half.x, half.y, half.z),
    ];
    let stray = arm * turn * turn / 8.0 * (1.0 + turn / 2.0) + local_half2.length() * turn / 2.0;
    let cell_proxy = ToiProxy::from_array(corners, margin + stray);
    let halfway = pos1.inv_mul(&motion2.position_at_time(start_time + span / 2.0)).rotation;
    let at = |centre: Vector| Pose::from_parts(centre - halfway * local_center2, halfway);
    let end_pos2_1 = pos1.inv_mul(&motion2.position_at_time(end_time));
    let (centre_start, centre_end) = (start_pos2_1 * local_center2, end_pos2_1 * local_center2);
    let sweep2 = Sweep::from_poses(&at(centre_start), &at(centre_end), local_center2);
    // (The shape's box so moving, grown, and how far it moves.)
    let box2 = g2.compute_aabb(&at(centre_start)).loosened(stray + margin);
    let shift = centre_end - centre_start;
    // (When, as a fraction of the times, that box first meets a cell's; none if never.)
    let meets = |center: Vector| -> Option<Real> {
        let (mut enter, mut leave) = (0.0 as Real, 1.0 as Real);
        for i in 0..3 {
            let (lo, hi) = (center[i] - half[i] - box2.maxs[i], center[i] + half[i] - box2.mins[i]);
            if shift[i] == 0.0 {
                if lo > 0.0 || hi < 0.0 {
                    return None;
                }
            } else {
                let (a, b) = (lo / shift[i], hi / shift[i]);
                enter = enter.max(a.min(b));
                leave = leave.min(a.max(b));
            }
        }
        (enter <= leave).then_some(enter)
    };
    // (Each cell: its first bound, its place in the walk, and its centre.)
    let mut candidates: Vec<(Real, usize, Vector)> = Vec::new();

    // The walk.
    let moved = |t: Real| Aabb::new(box0.mins + linvel2_1 * t, box0.maxs + linvel2_1 * t);
    let axis = linvel2_1.abs().max_position();
    let speed = linvel2_1[axis];
    let size = g1.voxel_size()[axis];
    let [domain_mins, domain_maxs] = g1.domain();
    let [rows_mins, rows_maxs] = g1.voxel_range_intersecting_local_aabb(&moved(0.0).merged(&moved(span)));
    for row in rows_mins[axis].max(domain_mins[axis])..rows_maxs[axis].min(domain_maxs[axis]) {
        // (When the box is in the row, within the times.)
        let (row_mins, row_maxs) = (row as Real * size, (row + 1) as Real * size);
        let (enter, leave) = if speed > 0.0 {
            ((row_mins - box0.maxs[axis]) / speed, (row_maxs - box0.mins[axis]) / speed)
        } else if speed < 0.0 {
            ((row_maxs - box0.mins[axis]) / speed, (row_mins - box0.maxs[axis]) / speed)
        } else {
            (0.0, span)
        };
        let (enter, leave) = (enter.clamp(0.0, span), leave.clamp(0.0, span));
        let [mut mins, mut maxs] = g1.voxel_range_intersecting_local_aabb(&moved(enter).merged(&moved(leave)));
        mins[axis] = row;
        maxs[axis] = row + 1;
        for vox in g1.voxels_in_range(mins, maxs) {
            if vox.state.is_empty() {
                continue;
            }
            let center = g1.voxel_center(vox.grid_coords);
            if culled(center) {
                continue;
            }
            if let Some(enter) = meets(center) {
                candidates.push((start_time + enter * span, candidates.len(), center));
            }
        }
    }

    // The second bounds and the exact casts, best first: a queue of cells by bound (of equal bounds, the first
    // walked), each first by its first bound, then, once its second is taken, by that.
    let mut queue: BinaryHeap<Reverse<Bounded>> = candidates.into_iter().map(|(bound, place, center)| Reverse(Bounded { bound, place, center, second: false })).collect();
    let cuboid = Cuboid::new(half);
    let (mut hit, mut smallest_t, mut hit_place) = (None, end_time, usize::MAX);
    while let Some(Reverse(cell)) = queue.pop() {
        if cell.bound > smallest_t {
            break;
        }
        if let (Some(p2), false) = (&proxy2, cell.second) {
            let cell_sweep = Sweep::constant(&Pose::from_translation(cell.center), Vector::ZERO);
            let out = sweep_time_of_impact(&cell_proxy, &cell_sweep, p2, &sweep2, 1.0, margin / 10.0);
            let bound = match out.status {
                SweepToiStatus::Separated => continue,
                SweepToiStatus::Overlapped => start_time,
                SweepToiStatus::Hit | SweepToiStatus::Failed => start_time + out.fraction * span,
            };
            // (Never before the first: both are at or before the contact.)
            queue.push(Reverse(Bounded { bound: bound.max(cell.bound), second: true, ..cell }));
            continue;
        }
        let vox_motion1 = motion1.prepend_translation(cell.center);
        if let Some(new_hit) = dispatcher
            .cast_shapes_nonlinear(&vox_motion1, &cuboid, &motion2, g2, start_time, end_time, stop_at_penetration)
            .ok()
            .flatten()
        {
            let toi = new_hit.time_of_impact;
            if toi < smallest_t || (toi == smallest_t && hit.is_some() && cell.place < hit_place) {
                smallest_t = toi;
                hit = Some(new_hit);
                hit_place = cell.place;
            }
        }
    }

    hit
}

/// A cell in the queue of `cast_shapes_nonlinear_voxels_shape` (ThreeRT): a bound on its time of impact, its place in
/// the walk (of equal bounds, the first walked comes first), its centre, and whether the bound is its second.
#[derive(Copy, Clone)]
struct Bounded {
    bound: Real,
    place: usize,
    center: Vector,
    second: bool,
}

impl PartialEq for Bounded {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Bounded {}

impl PartialOrd for Bounded {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Bounded {
    fn cmp(&self, other: &Self) -> Ordering {
        self.bound.total_cmp(&other.bound).then(self.place.cmp(&other.place))
    }
}

/// Time Of Impact of any shape with a composite shape, under a rigid motion (translation + rotation).
pub fn cast_shapes_nonlinear_shape_voxels<D>(
    dispatcher: &D,
    motion1: &NonlinearRigidMotion,
    g1: &dyn Shape,
    motion2: &NonlinearRigidMotion,
    g2: &Voxels,
    start_time: Real,
    end_time: Real,
    stop_at_penetration: bool,
) -> Option<ShapeCastHit>
where
    D: ?Sized + QueryDispatcher,
{
    cast_shapes_nonlinear_voxels_shape(
        dispatcher,
        motion2,
        g2,
        motion1,
        g1,
        start_time,
        end_time,
        stop_at_penetration,
    )
    .map(|hit| hit.swapped())
}
