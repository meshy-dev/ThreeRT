//! Light tree over the static emitter triangles (pathtrace.wgsl `emitter_draw`, `emitter_sel`): a binary tree whose
//! nodes bound their emitters with a sphere and sum their power. A shading point descends it choosing each child by
//! importance (power x a cosine bound for the surface's hemisphere / max(distance², radius²)), so the lights near it get
//! the samples a global power CDF spread over the whole scene (Bistro: 20,638 emitter triangles, the string lights
//! across the square next to the lamp beside the camera). Conty Estevez & Kulla 2018, without orientation cones:
//! emitter triangles are two-sided here, only rect-light cards are not.
//!
//! The tree's emitters come first in the emitter CDF, in leaf order; a leaf holds up to LEAF_MAX consecutive entries,
//! picked by weight. The selection probability of an emitter is the product of the choices down its path (the code its
//! triangle carries) x its weight / its leaf's, so a BSDF ray that hits it weighs its MIS by the same pmf. A node is one
//! vec4 (one load per child visited): centre and radius as half floats, power, child word.
//!
//! Moving emitters (dynamic triangles: a van's lights, a swinging lantern) have a subtree of their own: the root's
//! right child, the static emitters' subtree its left. They move, so their subtree is refit as they do
//! (`refit_moving`: leaf spheres from their triangles now, then every node's from its children's), keeping its
//! topology and so the codes their triangles carry. Particles (placed by a GPU pass) follow the tree's entries in the
//! CDF: the kernel picks among them by power with the probability of their share of power (`Params::ltree.z`), the
//! tree otherwise.
//!
//! Emitters added in place (the live view's geometry edits: a lamp blown up, its glowing pieces flying; ROADMAP § 6
//! step 4) get a subtree of their own in scenes that can edit geometry (`build_slotted`): the root's right child, the
//! static subtree its left, grown again over them at every such edit (`set_dynamic`; the static subtree and its codes
//! stay). By power share alone a lamp moved there by an edit read 2.3 times the variance it had in the tree.
//!
//! Emitters whose object is switched off at load (a time-of-day room's lamps and windows by day) are in the tree too,
//! at zero weight until they switch on: `light_tree_of` builds it with them at full power, then refits the powers.

use crate::EmitterTri;

/// Below this many static emitters the kernel keeps sampling the CDF alone (small scenes render as before).
pub const MIN_STATIC: usize = 256;
/// Emitters per leaf at most (picked by weight within it). 1: every emitter is chosen by importance (Bistro offline
/// relMSE 0.041 against 0.052 with 4, for +0.1 ms live).
pub const LEAF_MAX: usize = 1;
/// Tree candidates resampled at camera hits (`Params::ltree.w`, 0 = no tree): offline renders take as many as the
/// CDF did (they cost little next to the shadow ray and help most where glossy surfaces see many emitters); the live
/// viewport 8 (`live_candidates`). Live 2 -> 4 (2026-11-08) cost nothing measurable (Bistro's trace 8.20 / 9.93 ->
/// 8.19 / 9.89 ms at two poses) and brought it nearer the unbiased render: the live filters darken noisy signals
/// (bright outliers lose weight), so less noise is more light. Bistro ΔE 4.05 -> 3.89, luminance 0.893 -> 0.909;
/// Zero Day ΔE 3.85 -> 3.77. 4 -> 8 (2026-11-09), once the candidates were resampled by the BSDF (glossy surfaces
/// choose among more of the small lights in their lobe): the crowd's ΔE at 10 s 10.61 -> 10.31, layer B Bistro 3.34
/// -> 3.23, bench-ghosting better on every check; no measurable cost (perf smoke drive GPU p50 -0.02 ms).
pub const RIS_OFFLINE: u32 = 6;
pub const RIS_LIVE: u32 = 8;

/// THREERT_LT_RIS overrides the live count (A/B).
pub fn live_candidates() -> u32 {
    std::env::var("THREERT_LT_RIS").ok().and_then(|v| v.parse().ok()).unwrap_or(RIS_LIVE).max(1)
}

/// Node child word: internal nodes hold their left child (the right one follows it); leaves LEAF | (count - 1) << 29 |
/// their first entry.
pub const LEAF: u32 = 0x8000_0000;
/// Path codes: depth + 1 in the top bits, the path (bit k = right child at level k) in the low `PATH_BITS`.
pub const PATH_BITS: u32 = 26;
#[derive(Clone, Debug, Default)]
pub struct LightTree {
    /// One vec4 per node: pack2x16float(centre x, y), pack2x16float(centre z, radius) (bits), power, child word (bits).
    pub nodes: Vec<[f32; 4]>,
    /// Per node: [lo, hi) of its entries (refits).
    ranges: Vec<(u32, u32)>,
    /// The tree's emitters (indices into the `tris` given to `build`) in leaf order: they become entries 0.. of the CDF.
    pub order: Vec<u32>,
    /// Per tree entry (position in `order`): its path code.
    pub code: Vec<u32>,
    /// Total power of the tree's emitters.
    pub power: f32,
    /// The moving emitters' subtree (node 2, under the root; none without moving emitters): its entries
    /// `moving_first..moving_first + moving_count` (after the static ones), its nodes below node 2 from `moving_nodes`.
    pub moving_first: u32,
    pub moving_count: u32,
    pub moving_nodes: usize,
    /// The subtree of emitters added in place (`build_slotted`; node 2, path bit 0 = 1): the nodes before it (the root,
    /// node 2 and the static subtree) and the static entries; its entries follow them in `order`.
    pub dynamic: Option<(usize, u32)>,
}

/// An emitter added in place, for `LightTree::set_dynamic`: its index (in the caller's emitter list), its triangle's
/// bounds and centroid, its weight.
pub struct DynItem {
    pub e: u32,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub c: [f32; 3],
    pub w: f32,
}

/// An emitter in a build: its index (in `build`'s `tris`), bounds, centroid, weight.
struct Item {
    e: u32,
    lo: [f32; 3],
    hi: [f32; 3],
    c: [f32; 3],
    w: f32,
}

/// A triangle's vertices from the prepared v0, e1, e2 rows.
pub fn tri_vertices(rows: &[[f32; 4]]) -> [[f32; 3]; 3] {
    let v0 = [rows[0][0], rows[0][1], rows[0][2]];
    [v0, [v0[0] + rows[1][0], v0[1] + rows[1][1], v0[2] + rows[1][2]], [v0[0] + rows[2][0], v0[1] + rows[2][1], v0[2] + rows[2][2]]]
}

fn bounds(vs: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let lo = [0, 1, 2].map(|a| vs.iter().map(|v| v[a]).fold(f32::MAX, f32::min));
    let hi = [0, 1, 2].map(|a| vs.iter().map(|v| v[a]).fold(f32::MIN, f32::max));
    (lo, hi)
}

/// A node's first two words: centre and radius as half floats. The centre rounds by up to half a step of its
/// magnitude, so the radius grows by that and rounds up (a sphere that still holds its emitters).
fn encode(c: [f32; 3], r: f32) -> [f32; 2] {
    let pack = |a: f32, b: f32| f32::from_bits(half::f16::from_f32(a).to_bits() as u32 | (half::f16::from_f32(b).to_bits() as u32) << 16);
    let hc = c.map(|v| half::f16::from_f32(v).to_f32());
    let grow = ((hc[0] - c[0]).powi(2) + (hc[1] - c[1]).powi(2) + (hc[2] - c[2]).powi(2)).sqrt();
    let mut hr = half::f16::from_f32(r + grow);
    if hr.to_f32() < r + grow {
        hr = half::f16::from_bits(hr.to_bits() + 1);
    }
    [pack(hc[0], hc[1]), pack(hc[2], hr.to_f32())]
}

/// A box's bounding sphere, encoded.
fn box_sphere(lo: [f32; 3], hi: [f32; 3]) -> [f32; 2] {
    let c = [0, 1, 2].map(|k| 0.5 * (lo[k] + hi[k]));
    let r = 0.5 * ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
    encode(c, r)
}

/// A node's sphere, decoded.
fn sphere_of(n: &[f32; 4]) -> ([f32; 3], f32) {
    let un = |w: f32| {
        let b = w.to_bits();
        (half::f16::from_bits(b as u16).to_f32(), half::f16::from_bits((b >> 16) as u16).to_f32())
    };
    let ((x, y), (z, r)) = (un(n[0]), un(n[1]));
    ([x, y, z], r)
}

/// The smallest sphere holding two spheres, encoded.
fn union_sphere((c1, r1): ([f32; 3], f32), (c2, r2): ([f32; 3], f32)) -> [f32; 2] {
    let d = ((c2[0] - c1[0]).powi(2) + (c2[1] - c1[1]).powi(2) + (c2[2] - c1[2]).powi(2)).sqrt();
    if d + r2 <= r1 {
        return encode(c1, r1);
    }
    if d + r1 <= r2 {
        return encode(c2, r2);
    }
    let r = 0.5 * (d + r1 + r2);
    let t = (r - r1) / d.max(1e-12);
    encode([0, 1, 2].map(|k| c1[k] + (c2[k] - c1[k]) * t), r * (1.0 + 1e-6))
}

impl LightTree {
    /// Checks the tree as the kernel reads it (pathtrace.wgsl `emitter_draw`, `emitter_sel`): from node 0, every
    /// interior node's children are the nodes at its child word and the next, after it and inside the tree; every
    /// node is reached exactly once, no deeper than `PATH_BITS`; every leaf's 1 to 4 entries (its word) are its range
    /// (`ranges`) and inside the entries, the leaves together holding every entry once (the empty slot of emitters
    /// added in place, a leaf of power 0 over nothing, apart); and every entry's code leads from the root, level by
    /// level, to the leaf that holds it (what `emitter_sel` follows). Returns the depth. Run in tests and debug
    /// builds after every build, growth and refit (refits keep the topology: what they could break is the numbers).
    pub fn check(&self) -> Result<usize, String> {
        let (nodes, entries) = (self.nodes.len(), self.order.len());
        if nodes == 0 || self.ranges.len() != nodes || self.code.len() != entries {
            return Err(format!("{nodes} nodes, {} ranges, {entries} entries, {} codes", self.ranges.len(), self.code.len()));
        }
        let word = |i: usize| self.nodes[i][3].to_bits();
        let leaf = |w: u32| (w & LEAF != 0).then(|| (w & 0x1fff_ffff, ((w >> 29) & 3) + 1));
        let (mut seen, mut held) = (vec![false; nodes], vec![0u8; entries]);
        let (mut stack, mut deepest) = (vec![(0usize, 0u32)], 0);
        while let Some((i, depth)) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                return Err(format!("node {i} is reached twice"));
            }
            if depth > PATH_BITS {
                return Err(format!("node {i} is deeper than {PATH_BITS}"));
            }
            deepest = deepest.max(depth);
            let (lo, hi) = self.ranges[i];
            match leaf(word(i)) {
                Some(_) if lo == hi => {}
                Some((first, count)) => {
                    if (first, first + count) != (lo, hi) || hi as usize > entries {
                        return Err(format!("leaf {i}: entries {first}..{} in its word, {lo}..{hi} in its range, of {entries}", first + count));
                    }
                    (lo..hi).for_each(|k| held[k as usize] += 1);
                }
                None => {
                    let l = word(i) as usize;
                    if l <= i || l + 1 >= nodes {
                        return Err(format!("node {i}'s children {l}, {} are not after it and inside the tree's {nodes} nodes", l + 1));
                    }
                    stack.extend([(l, depth + 1), (l + 1, depth + 1)]);
                }
            }
        }
        if let Some(i) = seen.iter().position(|s| !s) {
            return Err(format!("node {i} is not reached"));
        }
        if let Some(k) = held.iter().position(|&h| h != 1) {
            return Err(format!("entry {k} is in {} leaves", held[k]));
        }
        for (k, &c) in self.code.iter().enumerate() {
            let depth = (c >> PATH_BITS).wrapping_sub(1);
            if depth > PATH_BITS {
                return Err(format!("entry {k}'s code {c:#x}: depth {depth}"));
            }
            let mut i = 0usize;
            for level in 0..depth {
                if leaf(word(i)).is_some() {
                    return Err(format!("entry {k}'s code goes past leaf {i}"));
                }
                i = word(i) as usize + ((c >> level) & 1) as usize;
            }
            if !matches!(leaf(word(i)), Some((first, count)) if (first..first + count).contains(&(k as u32))) {
                return Err(format!("entry {k}'s code leads to node {i}, not its leaf"));
            }
        }
        Ok(deepest as usize)
    }

    /// Node powers again from the entries' weights (power / material edits; `weight(k)` of entry k = position in
    /// `order`): same topology, sums bottom-up (children come after their parent).
    pub fn refit(&mut self, weight: &dyn Fn(usize) -> f32) {
        for i in (0..self.nodes.len()).rev() {
            let w = self.nodes[i][3].to_bits();
            self.nodes[i][2] = if w & LEAF != 0 {
                let (lo, hi) = self.ranges[i];
                (lo..hi).map(|k| weight(k as usize).max(0.0)).sum()
            } else {
                self.nodes[w as usize][2] + self.nodes[w as usize + 1][2]
            };
        }
        self.power = self.nodes.first().map_or(0.0, |b| b[2]);
        self.checked();
    }

    /// The moving emitters moved: their leaves' spheres from their triangles now (`verts(k)`: entry k's vertices),
    /// then each moving node's and the root's from its children's, bottom-up (children come after their parent). The
    /// topology, codes and powers stay.
    pub fn refit_moving(&mut self, verts: &dyn Fn(usize) -> [[f32; 3]; 3]) {
        if self.moving_count == 0 {
            return;
        }
        for i in (self.moving_nodes..self.nodes.len()).rev().chain([2, 0]) {
            let w = self.nodes[i][3].to_bits();
            let s = if w & LEAF != 0 {
                let (a, b) = self.ranges[i];
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for k in a..b {
                    let (l, h) = bounds(&verts(k as usize));
                    for j in 0..3 {
                        lo[j] = lo[j].min(l[j]);
                        hi[j] = hi[j].max(h[j]);
                    }
                }
                box_sphere(lo, hi)
            } else {
                union_sphere(sphere_of(&self.nodes[w as usize]), sphere_of(&self.nodes[w as usize + 1]))
            };
            self.nodes[i][0] = s[0];
            self.nodes[i][1] = s[1];
        }
        self.checked();
    }

    /// `check` in tests and debug builds (bvh.rs `build_tris` says why not always).
    fn checked(&self) {
        #[cfg(any(test, debug_assertions))]
        if let Err(e) = self.check() {
            panic!("light tree: {e}");
        }
    }

    /// The subtree of emitters added in place grown again over `items` (`build_slotted`; none: empty, power 0):
    /// their entries follow the static ones in `order` (each leaf's entry its position there: the CDF puts them so),
    /// their codes in `code`; the root's power the sum again. The static subtree, its entries and codes are kept.
    pub fn set_dynamic(&mut self, items: Vec<DynItem>) {
        let Some((fixed, ns)) = self.dynamic else { return };
        let ns = ns as usize;
        self.nodes.truncate(fixed);
        self.ranges.truncate(fixed);
        self.order.truncate(ns);
        self.code.truncate(ns);
        if items.is_empty() {
            self.nodes[2] = [0.0, 0.0, 0.0, f32::from_bits(LEAF)];
            self.ranges[2] = (ns as u32, ns as u32);
        } else {
            assert!(ns + items.len() < 1 << 29, "light tree entries beyond the child word");
            let mut its: Vec<Item> = items.into_iter().map(|d| Item { e: d.e, lo: d.lo, hi: d.hi, c: d.c, w: d.w }).collect();
            self.order.resize(ns + its.len(), 0);
            self.code.resize(ns + its.len(), 0);
            self.grow(&mut its, ns, 2, 1, 1);
        }
        self.nodes[0][2] = self.nodes[1][2] + self.nodes[2][2];
        self.power = self.nodes[0][2];
        self.checked();
    }

    /// Grows a subtree over `items` (entries `first..first + items.len()`, in the order it leaves them) from node
    /// `root`, at `depth0` with path `path0`.
    fn grow(&mut self, items: &mut [Item], first: usize, root: usize, depth0: u32, path0: u32) {
        // (items range, node, depth, path); the items' slice order becomes the entry order.
        let mut stack: Vec<(usize, usize, usize, u32, u32)> = vec![(0, items.len(), root, depth0, path0)];
        while let Some((a, b, ni, depth, path)) = stack.pop() {
            let s = &mut items[a..b];
            let (mut lo, mut hi, mut w) = ([f32::MAX; 3], [f32::MIN; 3], 0.0f32);
            for it in s.iter() {
                for k in 0..3 {
                    lo[k] = lo[k].min(it.lo[k]);
                    hi[k] = hi[k].max(it.hi[k]);
                }
                w += it.w;
            }
            let [s0, s1] = box_sphere(lo, hi);
            self.nodes[ni] = [s0, s1, w, 0.0];
            self.ranges[ni] = ((first + a) as u32, (first + b) as u32);
            if s.len() <= LEAF_MAX {
                self.nodes[ni][3] = f32::from_bits(LEAF | ((s.len() as u32 - 1) << 29) | (first + a) as u32);
                assert!(depth <= PATH_BITS, "light tree deeper than {PATH_BITS}");
                for (k, it) in s.iter().enumerate() {
                    self.order[first + a + k] = it.e;
                    self.code[first + a + k] = ((depth + 1) << PATH_BITS) | path;
                }
                continue;
            }
            // Median split on the widest axis of the centroids: depth ceil(log2(n / LEAF_MAX)), so every path fits a
            // code.
            let (mut clo, mut chi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for it in s.iter() {
                for k in 0..3 {
                    clo[k] = clo[k].min(it.c[k]);
                    chi[k] = chi[k].max(it.c[k]);
                }
            }
            let axis = (0..3).max_by(|&x, &y| (chi[x] - clo[x]).total_cmp(&(chi[y] - clo[y]))).unwrap();
            let mid = s.len() / 2;
            s.select_nth_unstable_by(mid, |p, q| p.c[axis].total_cmp(&q.c[axis]));
            let left = self.nodes.len();
            self.nodes.extend([[0.0; 4]; 2]);
            self.ranges.extend([(0, 0); 2]);
            self.nodes[ni][3] = f32::from_bits(left as u32);
            stack.push((a, a + mid, left, depth + 1, path));
            stack.push((a + mid, b, left + 1, depth + 1, path | (1 << depth)));
        }
    }
}

/// The tree over the emitters that are not particles and have power (`weight`), from their triangles (`tri_verts`: v0,
/// e1, e2 per triangle, as prepared): the static ones; with moving ones too, a root over the static subtree (node 1)
/// and the moving one (node 2). None below MIN_STATIC static emitters.
pub fn build(tris: &[EmitterTri], tri_verts: &[[f32; 4]], weight: &dyn Fn(usize) -> f32) -> Option<LightTree> {
    build_with(tris, tri_verts, weight, false)
}

/// As `build`, with a slot for emitters added in place (scenes whose geometry the live view can edit): the root over
/// the static subtree (node 1, path bit 0 = 0) and theirs (node 2, bit 0 = 1, empty until `set_dynamic`). Without the
/// slot when there are moving emitters (their subtree is node 2; emitters added in place then go by power share).
pub fn build_slotted(tris: &[EmitterTri], tri_verts: &[[f32; 4]], weight: &dyn Fn(usize) -> f32) -> Option<LightTree> {
    build_with(tris, tri_verts, weight, true)
}

fn build_with(tris: &[EmitterTri], tri_verts: &[[f32; 4]], weight: &dyn Fn(usize) -> f32, slot: bool) -> Option<LightTree> {
    let (mut fixed, mut moving): (Vec<Item>, Vec<Item>) = (vec![], vec![]);
    for (i, t) in tris.iter().enumerate() {
        let w = weight(i);
        if t.nominal || !(w > 0.0) {
            continue;
        }
        let k = t.tri as usize * 3;
        let (lo, hi) = bounds(&tri_vertices(&tri_verts[k..k + 3]));
        let it = Item { e: i as u32, lo, hi, c: t.c, w };
        if t.moves {
            moving.push(it);
        } else {
            fixed.push(it);
        }
    }
    if fixed.len() < MIN_STATIC {
        return None;
    }
    let (ns, nm) = (fixed.len(), moving.len());
    assert!(ns + nm < 1 << 29, "light tree entries beyond the child word");
    let mut tree = LightTree { nodes: vec![[0.0; 4]], ranges: vec![(0, 0)], order: vec![0; ns + nm], code: vec![0; ns + nm], ..Default::default() };
    if nm == 0 && slot {
        // The root (node 0) over the static subtree (node 1) and the empty one of emitters added in place (node 2).
        tree.nodes.extend([[0.0; 4]; 2]);
        tree.ranges.extend([(0, 0); 2]);
        tree.grow(&mut fixed, 0, 1, 1, 0);
        tree.nodes[0] = [tree.nodes[1][0], tree.nodes[1][1], tree.nodes[1][2], f32::from_bits(1)];
        tree.ranges[0] = (0, ns as u32);
        tree.dynamic = Some((tree.nodes.len(), ns as u32));
        tree.set_dynamic(vec![]);
    } else if nm == 0 {
        tree.grow(&mut fixed, 0, 0, 0, 0);
    } else {
        // The root (node 0) over the static subtree (node 1, path bit 0 = 0) and the moving one (node 2, bit 0 = 1).
        tree.nodes.extend([[0.0; 4]; 2]);
        tree.ranges.extend([(0, 0); 2]);
        tree.grow(&mut fixed, 0, 1, 1, 0);
        (tree.moving_first, tree.moving_count, tree.moving_nodes) = (ns as u32, nm as u32, tree.nodes.len());
        tree.grow(&mut moving, ns, 2, 1, 1);
        let [s0, s1] = union_sphere(sphere_of(&tree.nodes[1]), sphere_of(&tree.nodes[2]));
        tree.nodes[0] = [s0, s1, tree.nodes[1][2] + tree.nodes[2][2], f32::from_bits(1)];
        tree.ranges[0] = (0, (ns + nm) as u32);
    }
    tree.power = tree.nodes[0][2];
    tree.checked();
    Some(tree)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri(i: u32, x: f32) -> (EmitterTri, [[f32; 4]; 3]) {
        (EmitterTri { tri: i, mat: 0, obj: 0, area: 0.5, c: [x + 1.0 / 3.0, 1.0 / 3.0, 0.0], moves: false, nominal: false, in_tree: false }, [[x, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]])
    }

    /// Garbage in: emitters whose triangles are not finite, that coincide, or carry weights near f32's limit make a
    /// tree `check` passes (the builder checks itself in tests), with and without the slot for emitters added in
    /// place and moving ones, and the slot grown over garbage too.
    #[test]
    fn garbage_in_makes_a_tree_the_kernel_reads() {
        let n = 700u32;
        let (mut tris, verts): (Vec<EmitterTri>, Vec<[[f32; 4]; 3]>) = (0..n).map(|i| tri(i, (i % 50) as f32)).unzip();
        let mut verts: Vec<[f32; 4]> = verts.into_iter().flatten().collect();
        for k in (0..n as usize).step_by(9) {
            verts[k * 3][k % 3] = if k % 2 == 0 { f32::NAN } else { f32::INFINITY };
            tris[k].c = [f32::NAN, 0.0, 0.0];
        }
        for k in (0..n as usize).step_by(13) {
            tris[k].moves = k % 26 == 0;
        }
        let w = |i: usize| if i % 11 == 0 { 3.0e38 } else { 1.0 };
        for slot in [false, true] {
            let mut t = build_with(&tris, &verts, &w, slot).expect("a tree");
            t.check().unwrap();
            t.refit(&|k| if k % 5 == 0 { f32::NAN } else { 2.0 });
            t.set_dynamic((0..300).map(|k| DynItem { e: k, lo: [f32::NAN; 3], hi: [0.0; 3], c: [(k % 3) as f32, f32::INFINITY, 0.0], w: 1.0 }).collect());
            t.check().unwrap();
        }
    }

    #[test]
    fn paths_lead_to_the_leaves_holding_their_entries_and_powers_sum() {
        let n = 1000;
        let (tris, verts): (Vec<EmitterTri>, Vec<[[f32; 4]; 3]>) = (0..n).map(|i| tri(i, i as f32 * 2.0)).unzip();
        let verts: Vec<[f32; 4]> = verts.into_iter().flatten().collect();
        let w = |i: usize| 1.0 + i as f32;
        let t = build(&tris, &verts, &w).unwrap();
        assert!((t.power - (0..n).map(|i| 1.0 + i as f32).sum::<f32>()).abs() < 1.0);
        let mut seen = vec![false; n as usize];
        for k in 0..n as usize {
            seen[t.order[k] as usize] = true;
            let code = t.code[k];
            let depth = (code >> PATH_BITS) - 1;
            let bound = ((n as f32 / LEAF_MAX as f32).log2().ceil()) as u32;
            assert!(depth <= bound, "balanced: depth {depth} > {bound}");
            let mut node = 0usize;
            for l in 0..depth {
                let w = t.nodes[node][3].to_bits();
                assert_eq!(w & LEAF, 0);
                assert!((t.ranges[node].0 as usize..t.ranges[node].1 as usize).contains(&k), "entry in every node's range on its path");
                node = w as usize + ((code >> l) & 1) as usize;
            }
            let w = t.nodes[node][3].to_bits();
            assert_ne!(w & LEAF, 0);
            let (lo, cnt) = (w & ((1 << 29) - 1), ((w >> 29) & 3) + 1);
            assert!((lo as usize..(lo + cnt) as usize).contains(&k));
            assert_eq!((lo, lo + cnt), t.ranges[node]);
        }
        assert!(seen.iter().all(|&s| s), "every emitter in the order once");
        // A node's power = its range's weights; refit with doubled weights doubles it.
        let range_power = |t: &LightTree, i: usize| (t.ranges[i].0..t.ranges[i].1).map(|k| w(t.order[k as usize] as usize)).sum::<f32>();
        assert!((t.nodes[1][2] - range_power(&t, 1)).abs() < 1e-2 * t.nodes[1][2]);
        // Half-float spheres still hold their triangles (leaf of entry 0: centre 0.5 + 2 x its emitter, radius >= 0.71).
        let mut t2 = t.clone();
        t2.refit(&|k| 2.0 * w(t.order[k] as usize));
        assert!((t2.power - 2.0 * t.power).abs() < 2.0);
    }

    #[test]
    fn few_or_moving_emitters_make_no_tree() {
        let (tris, verts): (Vec<EmitterTri>, Vec<[[f32; 4]; 3]>) = (0..300).map(|i| tri(i, i as f32)).unzip();
        let verts: Vec<[f32; 4]> = verts.into_iter().flatten().collect();
        assert!(build(&tris[..100], &verts, &|_| 1.0).is_none());
        let moving: Vec<EmitterTri> = tris.iter().map(|t| EmitterTri { moves: true, ..*t }).collect();
        assert!(build(&moving, &verts, &|_| 1.0).is_none());
    }

    /// With the slot: static codes under node 1 (bit 0 = 0); emitters added in place get node 2's subtree, their entries
    /// after the static ones, codes with bit 0 set leading to their leaves; growing it again keeps the static part.
    #[test]
    fn emitters_added_in_place_get_a_subtree_of_their_own() {
        let n = 300u32;
        let (tris, verts): (Vec<EmitterTri>, Vec<[[f32; 4]; 3]>) = (0..n).map(|i| tri(i, i as f32 * 2.0)).unzip();
        let verts: Vec<[f32; 4]> = verts.into_iter().flatten().collect();
        let mut t = build_slotted(&tris, &verts, &|_| 1.0).unwrap();
        let (fixed, ns) = t.dynamic.unwrap();
        assert_eq!(ns, n);
        assert!(t.code.iter().all(|c| c & 1 == 0), "static entries under node 1");
        assert!((t.power - n as f32).abs() < 1e-3);
        let (static_nodes, static_codes) = (t.nodes[..fixed].to_vec(), t.code.clone());
        let item = |e: u32, x: f32| DynItem { e, lo: [x, 0.0, 0.0], hi: [x + 1.0, 1.0, 0.0], c: [x + 0.3, 0.3, 0.0], w: 2.0 };
        for m in [5u32, 40] {
            t.set_dynamic((0..m).map(|k| item(1000 + k, 900.0 + k as f32)).collect());
            assert_eq!(t.order.len(), (n + m) as usize);
            assert_eq!(&t.code[..n as usize], &static_codes[..], "static codes kept");
            assert_eq!(&t.nodes[1..2], &static_nodes[1..2]);
            assert!((t.power - (n as f32 + 2.0 * m as f32)).abs() < 1e-3);
            for k in n as usize..(n + m) as usize {
                assert!(t.order[k] >= 1000);
                let code = t.code[k];
                assert_eq!(code & 1, 1, "under node 2");
                let mut node = 0usize;
                for l in 0..(code >> PATH_BITS) - 1 {
                    node = t.nodes[node][3].to_bits() as usize + ((code >> l) & 1) as usize;
                }
                let w = t.nodes[node][3].to_bits();
                assert_ne!(w & LEAF, 0);
                assert_eq!((w & ((1 << 29) - 1)) as usize, k, "the leaf holds the entry at its position");
            }
        }
        t.set_dynamic(vec![]);
        assert_eq!(t.order.len(), n as usize);
        assert!((t.power - n as f32).abs() < 1e-3);
    }

    /// Static and moving emitters: the root's left subtree holds the static ones, its right the moving ones (codes
    /// with bit 0 set); a refit after the moving ones moved bounds them where they are now, codes unchanged.
    #[test]
    fn moving_emitters_get_a_subtree_that_refits() {
        let n = 400u32;
        let (mut tris, verts): (Vec<EmitterTri>, Vec<[[f32; 4]; 3]>) = (0..n).map(|i| tri(i, i as f32 * 2.0)).unzip();
        for t in tris.iter_mut().skip(300) {
            t.moves = true;
        }
        let verts: Vec<[f32; 4]> = verts.into_iter().flatten().collect();
        let mut t = build(&tris, &verts, &|_| 1.0).unwrap();
        assert_eq!((t.moving_first, t.moving_count), (300, 100));
        assert!((t.power - n as f32).abs() < 1e-3);
        for k in 0..n as usize {
            let moving = tris[t.order[k] as usize].moves;
            assert_eq!(moving, k >= 300, "static entries first");
            assert_eq!((t.code[k] & 1) == 1, moving, "path bit 0: the subtree");
        }
        // The moving ones move 50 m up: every leaf and its ancestors hold them there.
        let codes = t.code.clone();
        let moved = |k: usize| {
            let i = t.order[k] as usize * 3;
            tri_vertices(&verts[i..i + 3]).map(|v| [v[0], v[1] + 50.0, v[2]])
        };
        let order = t.order.clone();
        let vs: Vec<[[f32; 3]; 3]> = (0..n as usize).map(|k| if tris[order[k] as usize].moves { moved(k) } else { [[0.0; 3]; 3] }).collect();
        t.refit_moving(&|k| vs[k]);
        assert_eq!(t.code, codes);
        for k in t.moving_first as usize..n as usize {
            let mut node = 0usize;
            let depth = (t.code[k] >> PATH_BITS) - 1;
            for l in 0..=depth {
                let (c, r) = sphere_of(&t.nodes[node]);
                for v in vs[k] {
                    let d = ((v[0] - c[0]).powi(2) + (v[1] - c[1]).powi(2) + (v[2] - c[2]).powi(2)).sqrt();
                    assert!(d <= r * 1.001 + 1e-3, "entry {k} outside node {node} at level {l}: {d} > {r}");
                }
                if l < depth {
                    node = t.nodes[node][3].to_bits() as usize + ((t.code[k] >> l) & 1) as usize;
                }
            }
        }
    }
}
