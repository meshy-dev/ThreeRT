use crate::math::{IVectorExt, Int, Real, Vector, VectorExt};
use crate::partitioning::BvhNode;
use crate::query::{Ray, RayCast, RayIntersection};
use crate::shape::{FeatureId, Voxels, VoxelsChunkRef};

impl RayCast for Voxels {
    #[inline]
    fn cast_local_ray_and_get_normal(
        &self,
        ray: &Ray,
        max_time_of_impact: Real,
        solid: bool,
    ) -> Option<RayIntersection> {
        self.chunk_bvh()
            .find_best(
                max_time_of_impact,
                |node: &BvhNode, best_so_far| node.cast_ray(ray, best_so_far),
                |primitive, best_so_far| {
                    let chunk = self.chunk_ref(primitive);
                    chunk.cast_local_ray_and_get_normal(ray, best_so_far, solid)
                },
            )
            .map(|(_chunk_id, hit)| hit)
    }
}

impl<'a> RayCast for VoxelsChunkRef<'a> {
    /// The cells of the chunk walked one by one along the ray (Amanatides and Woo), each one's bounds the lattice
    /// planes `key * voxel_size`: the first solid cell is hit where the ray entered it, on the face it crossed.
    ///
    /// ThreeRT's patch (see `PATCH.md`): the walk used to ask a solid cell's box for the hit again, and a ray lying in
    /// one of the box's face planes (a ray straight down at x = 1.5 over cells of 0.1 m) grazes that box, which
    /// answered none: the walk went on and the ray missed every cell. Here a ray along the planes of an axis belongs
    /// to the cells its coordinate floors into (on a plane between two cells or chunks, the upper one), and a cell
    /// that is solid is a hit. A ray that starts in a solid cell is, `solid`, a hit at 0 with no normal (as every shape
    /// answers from inside); only the hollow cast still asks that cell's box where the ray leaves it.
    #[inline]
    fn cast_local_ray_and_get_normal(
        &self,
        ray: &Ray,
        max_time_of_impact: Real,
        solid: bool,
    ) -> Option<RayIntersection> {
        #[cfg(feature = "dim2")]
        let ii = [0, 1];
        #[cfg(feature = "dim3")]
        let ii = [0, 1, 2];

        let size = self.parent.voxel_size();
        let [domain_mins, domain_maxs] = self.domain();
        let origin_key = self.voxel_at_point_unchecked(ray.origin);

        // The stretch of the ray inside the chunk's box, and the axis it enters by.
        let mut t0: Real = 0.0;
        let mut t1 = max_time_of_impact;
        let mut entered: Option<usize> = None;
        for i in ii {
            let d = ray.dir.vget(i);
            if d == 0.0 {
                // (By the cell its coordinate floors into, as the walk takes cells: the same answer in both chunks
                // on either side of a plane.)
                let k = origin_key.ivget(i);
                if k < domain_mins.ivget(i) || k >= domain_maxs.ivget(i) {
                    return None;
                }
                continue;
            }
            let o = ray.origin.vget(i);
            let lo = (domain_mins.ivget(i) as Real * size.vget(i) - o) / d;
            let hi = (domain_maxs.ivget(i) as Real * size.vget(i) - o) / d;
            let (near, far) = if lo <= hi { (lo, hi) } else { (hi, lo) };
            if near > t0 {
                t0 = near;
                entered = Some(i);
            }
            if far < t1 {
                t1 = far;
            }
        }
        if !(t0 <= t1) {
            return None;
        }

        // The cell the ray is in there, held inside the chunk (it enters on a face, to rounding).
        let mut key = if entered.is_some() {
            self.voxel_at_point_unchecked(ray.point_at(t0))
        } else {
            origin_key
        };
        for i in ii {
            key.ivset(
                i,
                key.ivget(i)
                    .clamp(domain_mins.ivget(i), domain_maxs.ivget(i) - 1),
            );
        }
        let mut t = t0;
        let mut axis = entered;

        loop {
            if let Some(voxel) = self.voxel_state(key) {
                if !voxel.is_empty() {
                    // We hit a voxel!
                    let mut hit = match axis {
                        // Entered across a face: that face, when it was crossed.
                        Some(i) => {
                            let mut normal = Vector::ZERO;
                            let up = ray.dir.vget(i) > 0.0;
                            normal.vset(i, if up { -1.0 } else { 1.0 });
                            let face = if up { i as u32 } else { i as u32 + 3 };
                            RayIntersection::new(t, normal, FeatureId::Face(face))
                        }
                        // The ray starts in this cell (the cell its origin floors into): inside a solid shape, a hit at
                        // once with no normal, as every shape answers; the cell's box is not asked (centre +- half a
                        // cell, it may round off a plane the origin lies on, and graze).
                        None if solid => RayIntersection::new(0.0, Vector::ZERO, FeatureId::Unknown),
                        // Hollow: where the ray leaves the cell's box, as before.
                        // TODO: the ray should go on until it reaches empty space again.
                        None => self
                            .voxel_aabb_unchecked(key)
                            .cast_local_ray_and_get_normal(ray, t1, solid)?,
                    };
                    // `subshape` says which voxel it belongs to.
                    hit.subshape = self.flat_id(key).unwrap_or_else(|| unreachable!());
                    return Some(hit);
                }
            }

            /*
             * The next cell: across the nearest lattice plane ahead (the lower axis first on a tie), never behind
             * where the ray is.
             */
            let mut next: Option<(Real, usize)> = None;
            for i in ii {
                let d = ray.dir.vget(i);
                if d != 0.0 {
                    let plane = (key.ivget(i) + (d > 0.0) as Int) as Real * size.vget(i);
                    let ti = ((plane - ray.origin.vget(i)) / d).max(t);
                    if next.map_or(true, |(tn, _)| ti < tn) {
                        next = Some((ti, i));
                    }
                }
            }
            // (A ray with no direction stays in its cell.)
            let (tn, i) = next?;
            if tn > t1 {
                return None;
            }
            t = tn;
            axis = Some(i);
            let k = key.ivget(i) + if ray.dir.vget(i) > 0.0 { 1 } else { -1 };
            if k < domain_mins.ivget(i) || k >= domain_maxs.ivget(i) {
                // Leaving the chunk's bounds.
                return None;
            }
            key.ivset(i, k);
        }
    }
}
