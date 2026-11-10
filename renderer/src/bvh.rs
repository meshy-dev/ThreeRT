//! Binned-SAH BVH over triangles. Leaves reference contiguous ranges of the reordered triangle list.

use glam::Vec3A as V;

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuNode {
    pub bmin: [f32; 3],
    /// Interior: index of the left child (right = left + 1). Leaf: first triangle.
    pub left_or_first: u32,
    pub bmax: [f32; 3],
    /// 0 for interior nodes.
    pub count: u32,
}

#[derive(Clone, Copy)]
struct Aabb {
    min: V,
    max: V,
}

impl Aabb {
    const EMPTY: Aabb = Aabb { min: V::splat(f32::INFINITY), max: V::splat(f32::NEG_INFINITY) };
    fn grow(&mut self, p: V) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    fn merge(&mut self, o: &Aabb) {
        self.min = self.min.min(o.min);
        self.max = self.max.max(o.max);
    }
    fn area(&self) -> f32 {
        let e = (self.max - self.min).max(V::ZERO);
        2.0 * (e.x * e.y + e.y * e.z + e.z * e.x)
    }
}

pub struct Bvh {
    pub nodes: Vec<GpuNode>,
    /// order[i] = original triangle index stored at position i.
    pub order: Vec<u32>,
}

const BINS: usize = 16;
const MAX_LEAF: usize = 4;

pub fn build(tris: &[[V; 3]]) -> Bvh {
    let n = tris.len();
    let mut boxes = Vec::with_capacity(n);
    let mut centroids = Vec::with_capacity(n);
    for t in tris {
        let mut b = Aabb::EMPTY;
        for p in t {
            b.grow(*p);
        }
        boxes.push(b);
        centroids.push((t[0] + t[1] + t[2]) / 3.0);
    }
    build_from(boxes, centroids)
}

/// The same BVH over boxes (min xyz, max xyz each; procedural geometry, software traversal), split at their centres.
pub fn build_boxes(b: &[f32]) -> Bvh {
    let boxes: Vec<Aabb> = b.chunks_exact(6).map(|c| Aabb { min: V::new(c[0], c[1], c[2]), max: V::new(c[3], c[4], c[5]) }).collect();
    let centroids = boxes.iter().map(|a| (a.min + a.max) * 0.5).collect();
    build_from(boxes, centroids)
}

fn build_from(boxes: Vec<Aabb>, centroids: Vec<V>) -> Bvh {
    let n = boxes.len();
    let mut order: Vec<u32> = (0..n as u32).collect();
    let mut nodes = vec![GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 }];
    if n == 0 {
        return Bvh { nodes, order };
    }
    // (node index, start, end)
    let mut stack = vec![(0usize, 0usize, n)];
    while let Some((ni, start, end)) = stack.pop() {
        let mut bounds = Aabb::EMPTY;
        let mut cb = Aabb::EMPTY;
        for &t in &order[start..end] {
            bounds.merge(&boxes[t as usize]);
            cb.grow(centroids[t as usize]);
        }
        let count = end - start;
        let make_leaf = |nodes: &mut Vec<GpuNode>| {
            nodes[ni] = GpuNode { bmin: bounds.min.into(), left_or_first: start as u32, bmax: bounds.max.into(), count: count as u32 };
        };
        if count <= MAX_LEAF {
            make_leaf(&mut nodes);
            continue;
        }
        // Binned SAH along each axis.
        let ext = cb.max - cb.min;
        let mut best = (f32::INFINITY, 0usize, 0usize);
        for axis in 0..3 {
            if ext[axis] <= 1e-9 {
                continue;
            }
            let mut bin_box = [Aabb::EMPTY; BINS];
            let mut bin_cnt = [0usize; BINS];
            let scale = BINS as f32 / ext[axis];
            for &t in &order[start..end] {
                let b = (((centroids[t as usize][axis] - cb.min[axis]) * scale) as usize).min(BINS - 1);
                bin_cnt[b] += 1;
                bin_box[b].merge(&boxes[t as usize]);
            }
            let mut left_area = [0.0f32; BINS];
            let mut left_cnt = [0usize; BINS];
            let mut acc = Aabb::EMPTY;
            let mut c = 0;
            for i in 0..BINS {
                acc.merge(&bin_box[i]);
                c += bin_cnt[i];
                left_area[i] = acc.area();
                left_cnt[i] = c;
            }
            let mut acc = Aabb::EMPTY;
            let mut c = 0;
            for i in (1..BINS).rev() {
                acc.merge(&bin_box[i]);
                c += bin_cnt[i];
                let cost = left_area[i - 1] * left_cnt[i - 1] as f32 + acc.area() * c as f32;
                if left_cnt[i - 1] > 0 && c > 0 && cost < best.0 {
                    best = (cost, axis, i);
                }
            }
        }
        let leaf_cost = bounds.area() * count as f32;
        let splittable = best.0.is_finite();
        if (splittable && best.0 >= leaf_cost && count <= 16) || (!splittable && count <= 16) {
            make_leaf(&mut nodes);
            continue;
        }
        let mid = if best.0.is_finite() {
            let (_, axis, split) = best;
            let scale = BINS as f32 / ext[axis];
            let slice = &mut order[start..end];
            let mut i = 0;
            for j in 0..slice.len() {
                let b = (((centroids[slice[j] as usize][axis] - cb.min[axis]) * scale) as usize).min(BINS - 1);
                if b < split {
                    slice.swap(i, j);
                    i += 1;
                }
            }
            start + i
        } else {
            // Degenerate centroids: split in the middle.
            start + count / 2
        };
        let left = nodes.len();
        nodes.push(GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 });
        nodes.push(GpuNode { bmin: [0.0; 3], left_or_first: 0, bmax: [0.0; 3], count: 0 });
        nodes[ni] = GpuNode { bmin: bounds.min.into(), left_or_first: left as u32, bmax: bounds.max.into(), count: 0 };
        stack.push((left + 1, mid, end));
        stack.push((left, start, mid));
    }
    Bvh { nodes, order }
}
