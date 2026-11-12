//! A wheel's probe: a ball swept down its suspension (ROADMAP § 6 step 10 c2, `out/web-probe/step10/c2-design.md`
//! 2.1). Voxel colliders are swept cell by cell, each cell a box, in closed form (a ray against the box grown by the
//! ball's radius: its faces, its edges as cylinders, its corners as spheres); any other collider through parry's
//! shape cast. Of every hit, the nearest whose ground is no steeper than the slope of cosine `cos_max` (its normal
//! against `up`) is kept, so a chunk's or a step's side face is passed over and the ground beyond it found: parry's
//! cast reports the first hit only. Of a voxel collider only the cells with a free face turned up are swept. A ball that starts in the ground is a hit at 0 (`Hit::inside`). Only `+ - * /`
//! and square roots: the same bits natively and in wasm32.

use rapier3d::math::{Pose, Real, Vector};
use rapier3d::parry::bounding_volume::{Aabb, BoundingVolume};
use rapier3d::parry::query::{cast_shapes, ShapeCastOptions};
use rapier3d::parry::shape::{AxisMask, Ball, Shape};
use rapier3d::prelude::{ColliderHandle, QueryPipeline};

/// What a probe met.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub collider: ColliderHandle,
    /// How far the ball's centre went along the direction (m).
    pub distance: Real,
    /// The ground's outward normal and the point the ball touches (world).
    pub normal: Vector,
    pub point: Vector,
    /// The ball started in the ground (distance 0, no normal of its own).
    pub inside: bool,
}

/// A ball of radius `r` at `o` moving along `d` (unit) against the box of half extents `h` about the origin, within
/// `max`: the distance it goes before touching and the box's outward normal there; `(0, ZERO)` if it starts touching
/// or overlapping it.
pub fn ball_box(o: Vector, d: Vector, h: Vector, r: Real, max: Real) -> Option<(Real, Vector)> {
    ball_box_inv(o, d, inverse(d), h, r, max)
}

/// `1 / d` by component (0 where `d` is 0: `ball_box_inv` does not use it there).
fn inverse(d: Vector) -> Vector {
    let i = |x: Real| if x == 0.0 { 0.0 } else { 1.0 / x };
    Vector::new(i(d.x), i(d.y), i(d.z))
}

/// `ball_box` with `1 / d` given (`inverse`): one sweep against many cells divides once.
fn ball_box_inv(o: Vector, d: Vector, inv: Vector, h: Vector, r: Real, max: Real) -> Option<(Real, Vector)> {
    let near = o.clamp(-h, h);
    if (o - near).length_squared() <= r * r {
        return Some((0.0, Vector::ZERO));
    }
    // The box grown by the radius: where the ray enters it.
    let (mut t0, mut t1) = (0.0 as Real, max);
    let (oa, da, ha, ia) = (o.to_array(), d.to_array(), h.to_array(), inv.to_array());
    for i in 0..3 {
        let e = ha[i] + r;
        if da[i] == 0.0 {
            if oa[i].abs() > e {
                return None;
            }
            continue;
        }
        let (a, b) = ((-e - oa[i]) * ia[i], (e - oa[i]) * ia[i]);
        let (a, b) = if a < b { (a, b) } else { (b, a) };
        t0 = t0.max(a);
        t1 = t1.min(b);
        if t0 > t1 {
            return None;
        }
    }
    let p = (o + d * t0).to_array();
    let outside = [p[0].abs() > ha[0], p[1].abs() > ha[1], p[2].abs() > ha[2]];
    let normal_at = |t: Real| {
        let x = o + d * t;
        let n = x - x.clamp(-h, h);
        let l = n.length();
        if l > 0.0 { n / l } else { Vector::ZERO }
    };
    if outside.iter().filter(|&&b| b).count() <= 1 {
        // On a face: there the grown box is the rounded one.
        return Some((t0, normal_at(t0)));
    }
    // Near an edge or a corner: the edges of the octant the entry point is in, and its corner (a ray inside the
    // grown box meets no face of the rounded one).
    let s = [p[0].signum(), p[1].signum(), p[2].signum()];
    let corner = [s[0] * ha[0], s[1] * ha[1], s[2] * ha[2]];
    let mut best: Option<Real> = None;
    let mut take = |t: Real| {
        if t >= t0 - 1e-6 && t <= t1 + 1e-6 && best.is_none_or(|b| t < b) {
            best = Some(t.max(0.0));
        }
    };
    // The corner's sphere.
    {
        let oc = [oa[0] - corner[0], oa[1] - corner[1], oa[2] - corner[2]];
        let b = oc[0] * da[0] + oc[1] * da[1] + oc[2] * da[2];
        let c = oc[0] * oc[0] + oc[1] * oc[1] + oc[2] * oc[2] - r * r;
        let disc = b * b - c;
        if disc >= 0.0 {
            take(-b - disc.sqrt());
        }
    }
    // The three edges' cylinders, along axis `a`, through the corner's other two coordinates.
    for a in 0..3 {
        let (u, v) = ((a + 1) % 3, (a + 2) % 3);
        let (ou, ov) = (oa[u] - corner[u], oa[v] - corner[v]);
        let qa = da[u] * da[u] + da[v] * da[v];
        if qa <= 0.0 {
            continue;
        }
        let qb = ou * da[u] + ov * da[v];
        let qc = ou * ou + ov * ov - r * r;
        let disc = qb * qb - qa * qc;
        if disc >= 0.0 {
            let t = (-qb - disc.sqrt()) / qa;
            if (oa[a] + da[a] * t).abs() <= ha[a] {
                take(t);
            }
        }
    }
    best.map(|t| (t, normal_at(t)))
}

/// The probe: a ball of radius `r` from each of `starts` along `dir` (unit) for at most `max`, against every collider
/// the pipeline's filter lets through and the `fresh` ones (those the broad phase does not have right until the next
/// step: added, edited or moved since the last one; the filter leaves them out, and each is given where it is now);
/// the nearest acceptable hit of them all (above). The sweeps from the starts share one walk over the cells (a wheel's
/// two, now and a tick ahead, cover nearly the same cells); every cell is tested against each.
pub fn sweep_ball(queries: &QueryPipeline, fresh: &[(ColliderHandle, Pose)], starts: &[Vector], dir: Vector, r: Real, max: Real, up: Vector, cos_max: Real) -> Option<Hit> {
    let mut swept: Option<Aabb> = None;
    for &start in starts {
        let end = start + dir * max;
        let a = Aabb::new(start - Vector::splat(r), start + Vector::splat(r)).merged(&Aabb::new(end - Vector::splat(r), end + Vector::splat(r)));
        swept = Some(swept.map_or(a, |s| s.merged(&a)));
    }
    let swept = swept?;
    let mut best: Option<Hit> = None;
    for (h, co) in queries.intersect_aabb_conservative(swept) {
        sweep_collider(h, co.shape(), co.position(), swept, starts, dir, r, max, up, cos_max, &mut best);
    }
    for &(h, pose) in fresh {
        if let Some(co) = queries.colliders.get(h) {
            if co.shape().compute_aabb(&pose).intersects(&swept) {
                sweep_collider(h, co.shape(), &pose, swept, starts, dir, r, max, up, cos_max, &mut best);
            }
        }
    }
    best
}

/// One collider's part of `sweep_ball`, the collider at `pose`.
#[allow(clippy::too_many_arguments)]
fn sweep_collider(h: ColliderHandle, shape: &dyn Shape, pose: &Pose, swept: Aabb, starts: &[Vector], dir: Vector, r: Real, max: Real, up: Vector, cos_max: Real, best: &mut Option<Hit>) {
    let keep = |hit: Hit, best: &mut Option<Hit>| {
        let ok = hit.inside || hit.normal.dot(up) > cos_max;
        if ok && best.is_none_or(|b| hit.distance < b.distance) {
            *best = Some(hit);
        }
    };
    match shape.as_voxels() {
        Some(v) => {
            let half = v.voxel_size() / 2.0;
            let d = pose.rotation.inverse() * dir;
            let inv = inverse(d);
            let local_starts: Vec<Vector> = starts.iter().map(|&s| pose.inverse_transform_point(s)).collect();
            let local = swept.transform_by(&pose.inverse());
            // Only cells with a free face turned up (its normal against `up` above 0) can hold an acceptable hit: a
            // point of the cells' surface whose normal is within the slope of `up` lies on such a face or on its edge,
            // and that face's cell meets the ball there first. A cell covered on every upward side would only give
            // hits on faces inside the ground.
            let u = pose.rotation.inverse() * up;
            let mut upward = AxisMask::empty();
            for (c, pos, neg) in [(u.x, AxisMask::X_POS, AxisMask::X_NEG), (u.y, AxisMask::Y_POS, AxisMask::Y_NEG), (u.z, AxisMask::Z_POS, AxisMask::Z_NEG)] {
                if c > 0.0 {
                    upward |= pos;
                } else if c < 0.0 {
                    upward |= neg;
                }
            }
            for cell in v.voxels_intersecting_local_aabb(&local) {
                if cell.state.is_empty() || !cell.state.free_faces().intersects(upward) {
                    continue;
                }
                let c = v.voxel_center(cell.grid_coords);
                for (k, &o) in local_starts.iter().enumerate() {
                    let limit = best.map_or(max, |b| b.distance);
                    if let Some((t, n)) = ball_box_inv(o - c, d, inv, half, r, limit) {
                        let normal = pose.rotation * n;
                        let centre = starts[k] + dir * t;
                        keep(Hit { collider: h, distance: t, normal, point: centre - normal * r, inside: t == 0.0 }, best);
                    }
                }
            }
        }
        None => {
            for &start in starts {
                let limit = best.map_or(max, |b| b.distance);
                let options = ShapeCastOptions { max_time_of_impact: limit, target_distance: 0.0, stop_at_penetration: true, compute_impact_geometry_on_penetration: true };
                if let Ok(Some(hit)) = cast_shapes(pose, Vector::ZERO, shape, &Pose::from_translation(start), dir, &Ball::new(r), options) {
                    let normal = pose.rotation * hit.normal1;
                    let inside = hit.time_of_impact == 0.0;
                    keep(Hit { collider: h, distance: hit.time_of_impact, normal, point: pose * hit.witness1, inside }, best);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rapier3d::parry::shape::Cuboid;

    /// The exact sweep in f64, for reference: the distance from a point on the ray to the box is convex along it, so
    /// its smallest is found by ternary search and the first touch by bisection before it.
    fn reference(o: Vector, d: Vector, h: Vector, r: f32, max: f32) -> Option<(f64, [f64; 3])> {
        let (o, d, h, r) = (o.to_array().map(|x| x as f64), d.to_array().map(|x| x as f64), h.to_array().map(|x| x as f64), r as f64);
        let off = |t: f64| -> [f64; 3] { std::array::from_fn(|i| { let x = o[i] + d[i] * t; x - x.clamp(-h[i], h[i]) }) };
        let gap = |t: f64| { let n = off(t); (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() - r };
        let (mut a, mut b) = (0.0f64, max as f64);
        for _ in 0..200 {
            let (m1, m2) = (a + (b - a) / 3.0, b - (b - a) / 3.0);
            if gap(m1) <= gap(m2) { b = m2 } else { a = m1 }
        }
        if gap(0.0) <= 0.0 {
            return Some((0.0, [0.0; 3]));
        }
        if gap(a) > 0.0 {
            return None;
        }
        let (mut lo, mut hi) = (0.0f64, a);
        for _ in 0..200 {
            let m = (lo + hi) / 2.0;
            if gap(m) > 0.0 { lo = m } else { hi = m }
        }
        let n = off(hi);
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        Some((hi, n.map(|x| x / l)))
    }

    /// The closed form against the exact sweep in f64 (distances within 2e-5 m, normals within 1e-3) and against
    /// parry's general sweep of a ball and a box (GJK, off by up to 5 mm here: within 1 cm): random balls, directions and
    /// boxes; touching at the start, a hit at 0.
    #[test]
    fn a_ball_against_a_box_in_closed_form() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 40) as f32 / 16_777_216.0
        };
        let (mut hits, mut misses) = (0, 0);
        for k in 0..20_000 {
            let h = Vector::new(0.05 + rnd() * 0.4, 0.05 + rnd() * 0.4, 0.05 + rnd() * 0.4);
            let r = 0.05 + rnd() * 0.5;
            let o = Vector::new(rnd() * 4.0 - 2.0, rnd() * 4.0 - 2.0, rnd() * 4.0 - 2.0);
            // (Towards the box, or anywhere; a fifth straight down.)
            let mut d = if k % 3 == 0 { Vector::new(rnd() - 0.5, rnd() - 0.5, rnd() - 0.5) } else { Vector::new(rnd() * 0.4 - 0.2, rnd() * 0.4 - 0.2, rnd() * 0.4 - 0.2) - o };
            if k % 5 == 0 {
                d = Vector::new(0.0, -1.0, 0.0);
            }
            let d = d.normalize();
            let max = 0.5 + rnd() * 4.0;
            let ours = ball_box(o, d, h, r, max);
            let exact = reference(o, d, h, r, max);
            let options = ShapeCastOptions { max_time_of_impact: max, target_distance: 0.0, stop_at_penetration: true, compute_impact_geometry_on_penetration: true };
            let parry = cast_shapes(&Pose::IDENTITY, Vector::ZERO, &Cuboid::new(h), &Pose::from_translation(o), d, &Ball::new(r), options).unwrap();
            match (ours, exact) {
                (Some((t, n)), Some((te, ne))) => {
                    assert!((t as f64 - te).abs() < 2e-5, "{t} against {te}: o {o:?} d {d:?} h {h:?} r {r}");
                    if t > 0.0 {
                        let e = ((n.x as f64 - ne[0]).powi(2) + (n.y as f64 - ne[1]).powi(2) + (n.z as f64 - ne[2]).powi(2)).sqrt();
                        assert!(e < 1e-3, "normal {n:?} against {ne:?}");
                    }
                    if let Some(p) = parry {
                        assert!((t - p.time_of_impact).abs() < 1e-2, "{t} against parry's {}", p.time_of_impact);
                    }
                    hits += 1;
                }
                (None, None) => misses += 1,
                (a, b) => {
                    // (A graze within 2e-5 of the surface, or of the reach, may go either way.)
                    let t = a.map(|x| x.0 as f64).or(b.map(|x| x.0)).unwrap();
                    let x = o + d * t as f32;
                    let gap = (x - x.clamp(-h, h)).length() - r;
                    assert!(gap.abs() < 2e-5 || t > max as f64 - 2e-5, "ours {a:?}, exact {b:?}: o {o:?} d {d:?} h {h:?} r {r}");
                }
            }
        }
        assert!(hits > 5_000 && misses > 3_000, "{hits} hits, {misses} misses");
        assert_eq!(ball_box(Vector::new(0.0, 0.2, 0.0), Vector::new(0.0, -1.0, 0.0), Vector::splat(0.1), 0.15, 1.0), Some((0.0, Vector::ZERO)));
    }
}
