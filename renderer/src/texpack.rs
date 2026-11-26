//! Texture pages: every scene texture is shelf-packed into square array layers with gutters filled by wrapped (or
//! clamped) texels, so the shader can tile any sub-rect seamlessly and sample a short mip chain without bleeding.
//! Any number of textures of any size share one binding; a per-texture table gives each one's rect, layer and size.

use crate::scene::Texture;
use rayon::prelude::*;

/// Gutter (texels) around each texture. Rects are aligned to `ALIGN` so the 2x2 box mip chain never mixes two
/// textures, and the last level still has a gutter wider than the bilinear footprint.
pub const GUTTER: u32 = 16;
pub const MIPS: u32 = 5;
const ALIGN: u32 = 1 << (MIPS - 1);
const MAX_PAGE: u32 = 4096;
const MAX_DIM: u32 = 2048;

pub struct Packed {
    pub side: u32,
    /// Per layer, levels 0..MIPS, RGBA8.
    pub layers: Vec<Vec<image::RgbaImage>>,
    /// Per texture two vec4: (u0, v0, du, dv) of the content rect in page UV; (layer, wrap, width, height).
    pub info: Vec<[f32; 4]>,
    /// Where each stream texture (`stream_key`) sits, by name: its pixels are replaced in place (`patch`).
    pub streams: Vec<(String, Place)>,
    /// Where every texture sits and what the pages have free: a texture is placed among them later (`Atlas::place`).
    pub atlas: Atlas,
}

/// The pages as what they hold and what they have free: every texture's place, and each layer's shelves. A texture
/// added while the scene runs takes a place here (`place`) and has its pixels written there (`patch`); nothing is
/// packed again.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Atlas {
    pub side: u32,
    /// By texture.
    pub places: Vec<Place>,
    /// By layer, top to bottom: a shelf's y, its height, and the x its next rect starts at.
    shelves: Vec<Vec<(u32, u32, u32)>>,
    /// Slots whose texture was freed (`free`): the next texture placed takes the lowest.
    free_slots: Vec<usize>,
    /// Rects freed textures left, by layer: (layer, x, y, width, height), aligned to ALIGN. A texture placed takes the
    /// smallest that holds it, the rest of it split in two (to its right, below it: a guillotine cut).
    free_rects: Vec<(u32, u32, u32, u32, u32)>,
}

/// The size a texture of `width` x `height` has in the pages: at most `MAX_DIM` a side, and a power of two >= 512
/// gives up 2 x GUTTER texels, so that its padded rect stays a power of two and four 2048s (or sixteen 1024s) fill a
/// page exactly; anything else keeps its size and is padded.
pub fn content_size(width: u32, height: u32) -> (u32, u32) {
    let (w, h) = (width.min(MAX_DIM), height.min(MAX_DIM));
    let k = (w as f32 / width as f32).min(h as f32 / height as f32);
    let (mut w, mut h) = (((width as f32 * k) as u32).max(1), ((height as f32 * k) as u32).max(1));
    if w >= 512 && w.is_power_of_two() {
        w -= 2 * GUTTER;
    }
    if h >= 512 && h.is_power_of_two() {
        h -= 2 * GUTTER;
    }
    (w, h)
}

fn padded_size(content: (u32, u32)) -> (u32, u32) {
    (round_up(content.0 + 2 * GUTTER), round_up(content.1 + 2 * GUTTER))
}

impl Atlas {
    pub fn layers(&self) -> u32 {
        self.shelves.len() as u32
    }

    /// Two rows a texture, at the pages' side as it is now: (u0, v0, du, dv) of the content rect in page UV; (layer,
    /// wrap, width, height). (No texture: two rows of zeros, so the table is never empty.)
    pub fn info(&self) -> Vec<[f32; 4]> {
        let s = self.side as f32;
        let mut info = vec![[0.0f32; 4]; self.places.len().max(1) * 2];
        for (i, p) in self.places.iter().enumerate() {
            // (A freed slot: zeros, as no texture.)
            if self.free_slots.contains(&i) {
                continue;
            }
            let (w, h) = p.size;
            info[i * 2] = [(p.x + GUTTER) as f32 / s, (p.y + GUTTER) as f32 / s, w as f32 / s, h as f32 / s];
            info[i * 2 + 1] = [p.layer as f32, p.wrap as u32 as f32, w as f32, h as f32];
        }
        info
    }

    /// A place for one more texture of `width` x `height` as its source has it: on a shelf it fits (the lowest such),
    /// else on a new shelf of a layer with room below its last, else on a new layer. The pages get a longer side
    /// where the texture is wider than they are, or where they have `max_layers` already; what they hold stays where
    /// it is, and a longer side changes every texture's rows (`info`). None: no room at the longest side and
    /// `max_layers`.
    pub fn place(&mut self, width: u32, height: u32, wrap: bool, max_layers: u32) -> Option<(usize, Place)> {
        let size = content_size(width, height);
        let (w, h) = padded_size(size);
        // A rect a freed texture left, the smallest that holds it.
        if let Some(i) = (0..self.free_rects.len()).filter(|&i| self.free_rects[i].3 >= w && self.free_rects[i].4 >= h).min_by_key(|&i| self.free_rects[i].3 as u64 * self.free_rects[i].4 as u64) {
            let (layer, x, y, fw, fh) = self.free_rects.swap_remove(i);
            for r in [(layer, x + w, y, fw - w, h), (layer, x, y + h, fw, fh - h)] {
                if r.3 > 0 && r.4 > 0 {
                    self.free_rects.push(r);
                }
            }
            return Some(self.take_slot(Place { layer, x, y, padded: (w, h), size, wrap }));
        }
        if w.max(h) > self.side {
            self.side = w.max(h).next_power_of_two().max(256);
        }
        if self.shelves.is_empty() {
            self.shelves.push(vec![]);
        }
        loop {
            let side = self.side;
            // (The lowest shelf that takes it; the first of them.)
            let on = self
                .shelves
                .iter()
                .enumerate()
                .flat_map(|(l, shelves)| shelves.iter().enumerate().map(move |(k, s)| (l, k, *s)))
                .filter(|(_, _, s)| s.1 >= h && s.2 + w <= side)
                .min_by_key(|(_, _, s)| s.1);
            let (layer, x, y) = if let Some((l, k, s)) = on {
                self.shelves[l][k].2 += w;
                (l, s.2, s.0)
            } else if let Some((l, y)) = self.shelves.iter().enumerate().map(|(l, shelves)| (l, shelves.last().map_or(0, |s| s.0 + s.1))).find(|(_, y)| y + h <= side) {
                self.shelves[l].push((y, h, w));
                (l, 0, y)
            } else if self.layers() < max_layers {
                self.shelves.push(vec![(0, h, w)]);
                (self.shelves.len() - 1, 0, 0)
            } else if side < MAX_PAGE {
                self.side *= 2;
                continue;
            } else {
                return None;
            };
            return Some(self.take_slot(Place { layer: layer as u32, x, y, padded: (w, h), size, wrap }));
        }
    }

    /// A slot for `place`: a freed one (the lowest), or a new one.
    fn take_slot(&mut self, place: Place) -> (usize, Place) {
        let slot = match self.free_slots.iter().enumerate().min_by_key(|(_, s)| **s).map(|(i, _)| i) {
            Some(i) => {
                let s = self.free_slots.swap_remove(i);
                self.places[s] = place;
                s
            }
            None => {
                self.places.push(place);
                self.places.len() - 1
            }
        };
        (slot, place)
    }

    /// Frees the texture in `slot`: its slot and its rect are the next textures' (`place`). Its texels stay in the
    /// pages until another's are written there. False for a slot that holds none.
    pub fn free(&mut self, slot: usize) -> bool {
        if slot >= self.places.len() || self.free_slots.contains(&slot) {
            return false;
        }
        let p = self.places[slot];
        self.free_slots.push(slot);
        self.rebuild_free(p.layer);
        true
    }

    /// A layer's free room made again from what it holds, so that it does not depend on the order textures came and
    /// went: on a grid of ALIGN texels, the shelves' room less the textures held there. Free columns at the end of a
    /// shelf go back to it (its next x moves back) and a layer's last empty shelves go, room a texture of any size can
    /// take again; what is left is cut into rects row by row, each as wide and then as tall as it can be.
    fn rebuild_free(&mut self, layer: u32) {
        let n = (self.side / ALIGN) as usize;
        let cell = |v: u32| (v / ALIGN) as usize;
        // 1: free, 0: held or outside the shelves.
        let mut free = vec![0u8; n * n];
        let Some(shelves) = self.shelves.get_mut(layer as usize) else { return };
        for s in shelves.iter() {
            for y in cell(s.0)..cell(s.0 + s.1).min(n) {
                free[y * n..y * n + cell(s.2).min(n)].fill(1);
            }
        }
        for (i, p) in self.places.iter().enumerate() {
            if p.layer != layer || self.free_slots.contains(&i) {
                continue;
            }
            for y in cell(p.y)..cell(p.y + p.padded.1).min(n) {
                free[y * n + cell(p.x)..y * n + cell(p.x + p.padded.0).min(n)].fill(0);
            }
        }
        for s in shelves.iter_mut() {
            let rows = cell(s.0)..cell(s.0 + s.1).min(n);
            while s.2 > 0 && rows.clone().all(|y| free[y * n + cell(s.2) - 1] == 1) {
                s.2 -= ALIGN;
                for y in rows.clone() {
                    free[y * n + cell(s.2)] = 0;
                }
            }
        }
        while shelves.last().is_some_and(|s| s.2 == 0) {
            shelves.pop();
        }
        self.free_rects.retain(|r| r.0 != layer);
        for y in 0..n {
            let mut x = 0;
            while x < n {
                if free[y * n + x] == 0 {
                    x += 1;
                    continue;
                }
                let mut w = 0;
                while x + w < n && free[y * n + x + w] == 1 {
                    w += 1;
                }
                let mut h = 1;
                while y + h < n && free[(y + h) * n + x..(y + h) * n + x + w].iter().all(|&c| c == 1) {
                    h += 1;
                }
                for yy in y..y + h {
                    free[yy * n + x..yy * n + x + w].fill(0);
                }
                self.free_rects.push((layer, x as u32 * ALIGN, y as u32 * ALIGN, w as u32 * ALIGN, h as u32 * ALIGN));
                x += w;
            }
        }
    }

    /// Whether `slot` holds a texture.
    pub fn holds(&self, slot: usize) -> bool {
        slot < self.places.len() && !self.free_slots.contains(&slot)
    }
}

/// A texture's padded rect in the pages (aligned to `ALIGN`, so its mips are its own) and its content's size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub layer: u32,
    pub x: u32,
    pub y: u32,
    pub padded: (u32, u32),
    /// The content's size in the page: the texture's, or smaller (`pack` shrinks some).
    pub size: (u32, u32),
    pub wrap: bool,
}

/// The key of a stream texture (scene.rs `texture_image`): its name and size, not its pixels.
pub fn stream_key(name: &str, width: u32, height: u32) -> String {
    format!("stream:{name}|{width}x{height}")
}

/// The stream a texture key names, if it is one.
pub fn stream_name(key: &str) -> Option<&str> {
    key.strip_prefix("stream:")?.split('|').next()
}

/// One level of a patch: the rect to write in a page's level, and its RGBA8 texels.
pub struct PatchLevel {
    pub level: u32,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// New pixels for a placed texture: its padded rect (the content, the gutters filled as `pack` fills them) at every
/// level. `img` is resized to the place's content size when it has another.
pub fn patch(place: &Place, img: &image::RgbaImage) -> Vec<PatchLevel> {
    let resized;
    let img = if img.dimensions() == place.size {
        img
    } else {
        resized = resize_triangle(img, place.size.0, place.size.1);
        &resized
    };
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (pw, ph) = place.padded;
    let fold = |v: i64, n: i64| if place.wrap { v.rem_euclid(n) } else { v.clamp(0, n - 1) } as usize;
    let src: &[u32] = bytemuck::cast_slice(img.as_raw());
    let mut level0 = vec![0u32; (pw * ph) as usize];
    let g = GUTTER as i64;
    let cols: Vec<usize> = (0..pw as i64).map(|x| fold(x - g, w)).collect();
    level0.par_chunks_mut(pw as usize).enumerate().for_each(|(row, dst)| {
        let sy = fold(row as i64 - g, h);
        let src_row = &src[sy * w as usize..(sy + 1) * w as usize];
        for (o, &sx) in dst.iter_mut().zip(&cols) {
            *o = src_row[sx];
        }
    });
    let chain = mip_chain(image::RgbaImage::from_raw(pw, ph, bytemuck::cast_slice(&level0).to_vec()).unwrap());
    chain.into_iter().enumerate().map(|(l, i)| PatchLevel { level: l as u32, x: place.x >> l, y: place.y >> l, width: i.width(), height: i.height(), rgba: i.into_raw() }).collect()
}

fn round_up(v: u32) -> u32 {
    v.div_ceil(ALIGN) * ALIGN
}

pub fn pack(textures: &[Texture]) -> Packed {
    if textures.is_empty() {
        let img = image::RgbaImage::from_pixel(ALIGN, ALIGN, image::Rgba([255, 255, 255, 255]));
        let atlas = Atlas { side: ALIGN, places: vec![], shelves: vec![vec![]], ..Default::default() };
        return Packed { side: ALIGN, layers: vec![mip_chain(img)], info: atlas.info(), streams: vec![], atlas };
    }
    let src: Vec<std::borrow::Cow<image::RgbaImage>> = textures
        .par_iter()
        .map(|t| {
            let (w, h) = content_size(t.width, t.height);
            if (w, h) == (t.width, t.height) {
                std::borrow::Cow::Borrowed(&*t.rgba)
            } else {
                std::borrow::Cow::Owned(resize_triangle(&t.rgba, w, h))
            }
        })
        .collect();
    let padded: Vec<(u32, u32)> = src.iter().map(|i| padded_size(i.dimensions())).collect();
    let mut order: Vec<usize> = (0..src.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse((padded[i].1, padded[i].0)));
    let area: u64 = padded.iter().map(|&(w, h)| w as u64 * h as u64).sum();
    let widest = padded.iter().map(|p| p.0.max(p.1)).max().unwrap_or(1);
    let mut side = widest.next_power_of_two().max(256);
    while side < MAX_PAGE && (side as u64 * side as u64) < area {
        side *= 2;
    }
    // Shelf packing; a size that does not fit restarts on a new layer.
    let mut place = vec![(0u32, 0u32, 0u32); src.len()];
    let (mut layer, mut x, mut y, mut shelf) = (0u32, 0u32, 0u32, 0u32);
    let mut shelves: Vec<Vec<(u32, u32, u32)>> = vec![vec![]];
    for &i in &order {
        let (w, h) = padded[i];
        if x + w > side {
            x = 0;
            y += shelf;
            shelf = 0;
        }
        if y + h > side {
            layer += 1;
            x = 0;
            y = 0;
            shelf = 0;
            shelves.push(vec![]);
        }
        place[i] = (layer, x, y);
        x += w;
        shelf = shelf.max(h);
        // (Its shelf: the layer's last where that is at this y, else a new one.)
        match shelves[layer as usize].last_mut() {
            Some(s) if s.0 == y => (s.1, s.2) = (s.1.max(h), x),
            _ => shelves[layer as usize].push((y, h, x)),
        }
    }
    let places = (0..src.len()).map(|i| Place { layer: place[i].0, x: place[i].1, y: place[i].2, padded: padded[i], size: src[i].dimensions(), wrap: textures[i].wrap }).collect();
    let atlas = Atlas { side, places, shelves, ..Default::default() };
    let info = atlas.info();
    // Fill pages row-parallel: every page row gathers from the textures whose padded rect spans it.
    let pages: Vec<image::RgbaImage> = (0..=layer)
        .into_par_iter()
        .map(|l| {
            let mine: Vec<usize> = (0..src.len()).filter(|&i| place[i].0 == l).collect();
            let mut page = vec![0u32; (side * side) as usize];
            page.par_chunks_mut(side as usize).enumerate().for_each(|(row, dst)| {
                let row = row as u32;
                for &i in &mine {
                    let (_, px, py) = place[i];
                    let (pw, ph) = padded[i];
                    if row < py || row >= py + ph {
                        continue;
                    }
                    let img = &src[i];
                    let (w, h) = (img.width() as i64, img.height() as i64);
                    let wrap = textures[i].wrap;
                    let fold = |v: i64, n: i64| if wrap { v.rem_euclid(n) } else { v.clamp(0, n - 1) } as usize;
                    let sy = fold((row - py) as i64 - GUTTER as i64, h);
                    let src_row: &[u32] = bytemuck::cast_slice(&img.as_raw()[sy * w as usize * 4..(sy + 1) * w as usize * 4]);
                    let out = &mut dst[px as usize..(px + pw) as usize];
                    let g = GUTTER as i64;
                    for (xx, o) in out.iter_mut().enumerate() {
                        let sx = xx as i64 - g;
                        *o = src_row[if sx >= 0 && sx < w { sx as usize } else { fold(sx, w) }];
                    }
                }
            });
            image::RgbaImage::from_raw(side, side, bytemuck::cast_slice(&page).to_vec()).unwrap()
        })
        .collect();
    let streams = textures.iter().enumerate().filter_map(|(i, t)| Some((stream_name(&t.key)?.to_string(), atlas.places[i]))).collect();
    Packed { side, layers: pages.into_par_iter().map(mip_chain).collect(), info, streams, atlas }
}

/// 2x2 box-filtered chain (sRGB-encoded texels are averaged as stored, like most engines' default mip gen).
fn mip_chain(level0: image::RgbaImage) -> Vec<image::RgbaImage> {
    let mut chain = vec![level0];
    for _ in 1..MIPS {
        let prev = chain.last().unwrap();
        let (w, h) = ((prev.width() / 2).max(1), (prev.height() / 2).max(1));
        let (src, sw) = (prev.as_raw(), prev.width() as usize);
        let mut out = vec![0u8; (w * h * 4) as usize];
        out.par_chunks_mut(w as usize * 4).enumerate().for_each(|(y, row)| {
            let (r0, r1) = (&src[2 * y * sw * 4..], &src[(2 * y + 1) * sw * 4..]);
            for x in 0..w as usize {
                for c in 0..4 {
                    let v = r0[8 * x + c] as u32 + r0[8 * x + 4 + c] as u32 + r1[8 * x + c] as u32 + r1[8 * x + 4 + c] as u32;
                    row[x * 4 + c] = ((v + 2) / 4) as u8;
                }
            }
        });
        chain.push(image::RgbaImage::from_raw(w, h, out).unwrap());
    }
    chain
}

/// `image::imageops::resize(img, w, h, FilterType::Triangle)` for RGBA8, bit for bit: the same weights, the same f32
/// sums in the same order, the same rounding. Rows run in parallel and each output row is finished from a one-row f32
/// buffer (the image crate's horizontal pass walks a whole-image f32 intermediate column by column: 0.9 s for a 3 x 3
/// district's textures).
pub fn resize_triangle(img: &image::RgbaImage, nw: u32, nh: u32) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    if (nw, nh) == (w, h) || w == 0 || h == 0 {
        return if w == 0 || h == 0 { image::RgbaImage::new(nw, nh) } else { img.clone() };
    }
    // Per output pixel along one axis: the first source pixel and the normalized weights (sample.rs
    // vertical_sample / horizontal_sample).
    let weights = |n: u32, nn: u32| -> Vec<(usize, Vec<f32>)> {
        let ratio = n as f32 / nn as f32;
        let sratio = if ratio < 1.0 { 1.0 } else { ratio };
        let support = 1.0 * sratio;
        (0..nn)
            .map(|o| {
                let input = (o as f32 + 0.5) * ratio;
                let left = ((input - support).floor() as i64).clamp(0, n as i64 - 1);
                let right = ((input + support).ceil() as i64).clamp(left + 1, n as i64);
                let input = input - 0.5;
                let mut ws: Vec<f32> = (left..right)
                    .map(|i| {
                        let x = (i as f32 - input) / sratio;
                        if x.abs() < 1.0 { 1.0 - x.abs() } else { 0.0 }
                    })
                    .collect();
                let mut sum = 0.0f32;
                for v in &ws {
                    sum += *v;
                }
                for v in ws.iter_mut() {
                    *v /= sum;
                }
                (left as usize, ws)
            })
            .collect()
    };
    let (wy, wx) = (weights(h, nh), weights(w, nw));
    let (row, src) = (w as usize * 4, img.as_raw());
    // Per output row: the vertical pass into an f32 row (per texel, the source rows summed in order), then the
    // horizontal pass to RGBA8 (round half away from zero after clamping to [0, 255]).
    let mut out = vec![0u8; nw as usize * 4 * nh as usize];
    out.par_chunks_mut(nw as usize * 4).zip(wy.par_iter()).for_each_init(
        || vec![0.0f32; row],
        |t, (dst, (left, ws))| {
            t.fill(0.0);
            for (k, wgt) in ws.iter().enumerate() {
                let s = &src[(left + k) * row..(left + k + 1) * row];
                for (d, v) in t.iter_mut().zip(s) {
                    *d += *v as f32 * *wgt;
                }
            }
            for (o, (left, ws)) in wx.iter().enumerate() {
                let mut acc = [0.0f32; 4];
                for (k, wgt) in ws.iter().enumerate() {
                    let p = &t[(left + k) * 4..(left + k) * 4 + 4];
                    for c in 0..4 {
                        acc[c] += p[c] * *wgt;
                    }
                }
                for c in 0..4 {
                    dst[o * 4 + c] = acc[c].clamp(0.0, 255.0).round() as u8;
                }
            }
        },
    );
    image::RgbaImage::from_raw(nw, nh, out).unwrap()
}

/// Identity of a texture set (pages are rebuilt only when it changes).
pub fn set_key(textures: &[Texture]) -> u64 {
    set_key_of(textures.iter().map(|t| (t.key.as_str(), t.width, t.height, t.wrap)))
}

/// `set_key` of textures given as (key, width, height, wrap).
pub fn set_key_of<'a>(textures: impl Iterator<Item = (&'a str, u32, u32, bool)>) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (key, width, height, wrap) in textures {
        key.hash(&mut h);
        (width, height, wrap).hash(&mut h);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn tex(w: u32, h: u32, wrap: bool, f: impl Fn(u32, u32) -> [u8; 4]) -> Texture {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba(f(x, y)));
        Texture { width: w, height: h, rgba: Arc::new(img), srgb: true, key: format!("{w}x{h}{wrap}"), wrap }
    }

    #[test]
    fn resize_matches_the_image_crate_bit_for_bit() {
        let mut seed = 5u32;
        let mut rnd = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 24) as u8
        };
        for (w, h, nw, nh) in [(256, 256, 224, 224), (2048, 64, 2016, 40), (300, 170, 97, 170), (100, 150, 160, 75), (5, 3, 2, 7), (4096, 8, 2048, 8)] {
            let img = image::RgbaImage::from_fn(w, h, |_, _| image::Rgba([rnd(), rnd(), rnd(), rnd()]));
            let want = image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle);
            assert!(resize_triangle(&img, nw, nh) == want, "{w}x{h} -> {nw}x{nh}");
        }
    }

    /// A stream's patch is what `pack` would have put there for the same pixels: every level of its padded rect.
    #[test]
    fn a_patch_is_the_pages_texels() {
        let other = tex(300, 200, true, |x, y| [(x % 251) as u8, (y % 241) as u8, 9, 255]);
        let stream = |f: fn(u32, u32) -> [u8; 4]| {
            let mut t = tex(640, 360, false, f);
            t.key = format!("{}|true|false", stream_key("screen", 640, 360));
            t
        };
        let first = pack(&[other, stream(|_, _| [0, 0, 0, 255])]);
        let picture: fn(u32, u32) -> [u8; 4] = |x, y| [(x % 256) as u8, (y % 256) as u8, ((x * 7 + y * 3) % 256) as u8, 255];
        let want = pack(&[tex(300, 200, true, |x, y| [(x % 251) as u8, (y % 241) as u8, 9, 255]), stream(picture)]);
        assert_eq!(first.streams.len(), 1);
        let (name, place) = &first.streams[0];
        assert_eq!(name, "screen");
        assert_eq!(*place, want.streams[0].1);
        assert_eq!((place.x % ALIGN, place.y % ALIGN, place.padded.0 % ALIGN, place.padded.1 % ALIGN), (0, 0, 0, 0));
        let levels = patch(place, &stream(picture).rgba);
        assert_eq!(levels.len() as u32, MIPS);
        for l in &levels {
            let page = &want.layers[place.layer as usize][l.level as usize];
            for y in 0..l.height {
                for x in 0..l.width {
                    let o = ((y * l.width + x) * 4) as usize;
                    assert_eq!(&l.rgba[o..o + 4], &page.get_pixel(l.x + x, l.y + y).0, "level {} at {x},{y}", l.level);
                }
            }
        }
    }

    /// No two of the places overlap, each lies in its page on the 16-texel grid, and each texture's rows say its
    /// place at the pages' side.
    fn sound(what: &str, a: &Atlas) {
        let info = a.info();
        for (i, p) in a.places.iter().enumerate() {
            if !a.holds(i) {
                assert_eq!((info[i * 2], info[i * 2 + 1]), ([0.0; 4], [0.0; 4]), "{what}: freed slot {i} has no rows");
                continue;
            }
            assert!(p.x + p.padded.0 <= a.side && p.y + p.padded.1 <= a.side && p.layer < a.layers(), "{what}: texture {i} in its page: {p:?} of {}", a.side);
            assert_eq!((p.x % ALIGN, p.y % ALIGN, p.padded.0 % ALIGN, p.padded.1 % ALIGN), (0, 0, 0, 0), "{what}: texture {i} on the grid");
            assert!(p.size.0 + 2 * GUTTER <= p.padded.0 && p.size.1 + 2 * GUTTER <= p.padded.1, "{what}: texture {i} has its gutters");
            let s = a.side as f32;
            assert_eq!(info[i * 2], [(p.x + GUTTER) as f32 / s, (p.y + GUTTER) as f32 / s, p.size.0 as f32 / s, p.size.1 as f32 / s], "{what}: texture {i}'s rect");
            assert_eq!(info[i * 2 + 1], [p.layer as f32, p.wrap as u32 as f32, p.size.0 as f32, p.size.1 as f32], "{what}: texture {i}'s layer and size");
            for (j, q) in a.places.iter().enumerate().skip(i + 1).filter(|(j, _)| a.holds(*j)) {
                let apart = p.layer != q.layer || p.x + p.padded.0 <= q.x || q.x + q.padded.0 <= p.x || p.y + p.padded.1 <= q.y || q.y + q.padded.1 <= p.y;
                assert!(apart, "{what}: textures {i} and {j} overlap: {p:?}, {q:?}");
            }
        }
    }

    /// Textures placed after the pack take the room it left: beside the last on a shelf, on a new shelf, on a new
    /// layer; those there stay where they are. A texture wider than the pages gives them a longer side, and with as
    /// many layers as there may be a full page does too; every texture's rows are then of the new side.
    #[test]
    fn a_texture_is_placed_among_the_packed_ones() {
        let flat = |w: u32, h: u32| tex(w, h, true, |_, _| [9, 9, 9, 255]);
        let packed = pack(&[flat(300, 200), flat(100, 60), flat(200, 200)]);
        let mut a = packed.atlas.clone();
        assert_eq!((a.side, a.layers(), a.places.len()), (512, 1, 3));
        sound("packed", &a);
        assert_eq!(a.info(), packed.info, "the pack's rows are the atlas's");
        let before = a.places.clone();
        // Beside the last on a shelf (both are as high: the first), then new shelves, then more than the page has.
        let small = a.place(64, 40, false, 8).unwrap().1;
        assert_eq!((small.layer, small.x, small.y, small.size, small.wrap), (0, before[0].x + before[0].padded.0, before[0].y, (64, 40), false));
        let mut layers = 1;
        for k in 0..12 {
            let p = a.place(200, 120, true, 8).unwrap().1;
            layers = layers.max(p.layer + 1);
            sound(&format!("after {k} more"), &a);
        }
        assert!(layers > 1 && a.side == 512, "the page filled and a layer was added: {layers} layers of {}", a.side);
        assert_eq!(a.places[..3], before[..], "those packed stay where they were");
        // Wider than the pages: a longer side, the places as they were, the rows of the new side.
        let big = a.place(2048, 1024, true, 8).unwrap().1;
        assert_eq!((a.side, big.size, big.padded), (2048, (2016, 992), (2048, 1024)));
        assert_eq!(a.places[..3], before[..]);
        sound("a longer side", &a);
        assert_eq!(a.info()[0][0], (before[0].x + GUTTER) as f32 / 2048.0);
        // As many layers as there may be: a full page gets a longer side, not another layer.
        let mut b = pack(&[flat(224, 224)]).atlas;
        assert_eq!((b.side, b.layers()), (256, 1));
        let p = b.place(224, 224, true, 1).unwrap().1;
        assert_eq!((b.side, b.layers(), p.layer), (512, 1, 0));
        sound("one layer", &b);
        // The longest side and no layer more: no room.
        let mut c = Atlas { side: MAX_PAGE, places: vec![], shelves: vec![vec![]], ..Default::default() };
        assert!((0..4).all(|_| c.place(2048, 2048, true, 1).is_some()) && c.place(16, 16, true, 1).is_none());
        assert_eq!((c.side, c.layers(), c.places.len()), (MAX_PAGE, 1, 4), "what found no room left nothing");
        // A shelf with room beside its last, but lower than the texture, is not its place.
        let mut d = pack(&[flat(100, 60)]).atlas;
        let tall = d.place(60, 150, true, 8).unwrap().1;
        assert!((tall.layer, tall.y) != (0, 0), "not on the shelf that is too low: {tall:?}");
        d.place(200, 60, true, 8).unwrap().1;
        sound("a tall one beside a low shelf", &d);
        // Pages without a texture take their first.
        let mut e = pack(&[]).atlas;
        let first = e.place(100, 60, true, 8).unwrap().1;
        assert_eq!((e.side, first.layer, first.x, first.y), (256, 0, 0, 0));
        sound("the first", &e);
    }

    /// Freed textures give their slots and rects to the next: the lowest slot first, the smallest rect that holds
    /// the texture, the rest of it to the next; the pages grow only when the freed room is not enough. Textures come
    /// and go a thousand times without the pages growing.
    #[test]
    fn freed_textures_give_their_room_to_the_next() {
        let flat = |w: u32, h: u32| tex(w, h, true, |_, _| [9, 9, 9, 255]);
        let mut a = pack(&[flat(300, 200), flat(100, 60), flat(200, 200)]).atlas;
        let before = a.places.clone();
        assert!(a.free(1) && !a.free(1) && !a.free(7) && !a.holds(1));
        sound("one freed", &a);
        // A smaller one: slot 1 again, in the room it left (the pages as they were).
        let (side, layers) = (a.side, a.layers());
        let (slot, p) = a.place(40, 20, false, 8).unwrap();
        assert_eq!((slot, p.layer, a.side, a.layers()), (1, 0, side, layers));
        sound("a smaller one in it", &a);
        let (slot, _) = a.place(16, 16, false, 8).unwrap();
        assert_eq!((slot, a.side, a.layers()), (3, side, layers), "a new slot: none is free");
        sound("another", &a);
        // Larger than the room freed: a place of its own.
        a.free(0);
        let (slot, big) = a.place(600, 600, true, 8).unwrap();
        assert_eq!(slot, 0);
        assert!(big.layer > 0 || a.side > side, "{big:?}");
        sound("a larger one", &a);
        // GLB places that come and go (textures of 256-2048 a side, as models have them, ten at a time): the pages
        // stay as large as the most there were at once needs.
        let mut g = pack(&[]).atlas;
        let sizes = [(1024, 1024), (2048, 2048), (512, 512), (1024, 1024), (256, 256), (2048, 1024), (512, 512), (1024, 512)];
        let mut live: std::collections::VecDeque<usize> = Default::default();
        let mut most = (0, 0);
        for k in 0..2000usize {
            let (w, h) = sizes[(k * 7 + k / 3) % sizes.len()];
            live.push_back(g.place(w, h, true, 64).unwrap().0);
            if live.len() > 10 {
                g.free(live.pop_front().unwrap());
            }
            if k == 200 {
                most = (g.side, g.layers());
            }
            if k % 199 == 0 {
                sound(&format!("place {k}"), &g);
            }
        }
        assert!(g.side == most.0 && g.layers() <= most.1 + 1, "the pages grew: {} layers of {} (after 200 places: {} of {})", g.layers(), g.side, most.1, most.0);
        assert!(g.places.len() <= 11, "slots are taken again: {}", g.places.len());
        // Any sizes, freed as soon as placed: the freed room is taken again, so the pages stop growing (what the
        // pieces of a cut cannot hold takes new room for a while; ten thousand rounds after the first thousand add
        // nothing).
        let mut after = (0, 0);
        for k in 0..11000u32 {
            let (w, h) = (64 + (k * 37) % 400, 64 + (k * 53) % 300);
            let (s1, _) = a.place(w, h, true, 64).unwrap();
            let (s2, _) = a.place(h, w, true, 64).unwrap();
            if k % 997 == 0 {
                sound(&format!("round {k}"), &a);
            }
            a.free(s1);
            a.free(s2);
            if k == 999 {
                after = (a.side, a.layers());
            }
        }
        sound("after them", &a);
        assert!(a.places.len() <= 6, "slots are taken again: {}", a.places.len());
        assert_eq!((a.side, a.layers()), after, "the pages grew after the first thousand rounds");
    }

    #[test]
    fn rects_gutters_and_mips() {
        // 1024 (pow2 >= 512: shrinks by 2 x GUTTER), a 100x60 clamped texture, a 300x200 wrapped one.
        let ts = [tex(1024, 1024, true, |x, _| [(x % 256) as u8, 0, 0, 255]), tex(100, 60, false, |x, y| [x as u8, y as u8, 7, 255]), tex(300, 200, true, |x, y| [(x % 251) as u8, (y % 241) as u8, 9, 255])];
        let p = pack(&ts);
        assert_eq!(p.layers[0].len() as u32, MIPS);
        assert_eq!(p.info.len(), 6);
        let s = p.side as f32;
        for (i, t) in ts.iter().enumerate() {
            let [u0, v0, du, dv] = p.info[i * 2];
            let [layer, wrap, w, h] = p.info[i * 2 + 1];
            assert_eq!(wrap > 0.5, t.wrap);
            let expect = if t.width >= 512 { t.width - 2 * GUTTER } else { t.width } as f32;
            assert_eq!(w, expect);
            assert!((du * s - w).abs() < 1e-3 && (dv * s - h).abs() < 1e-3);
            // Rect origins sit GUTTER inside a 16-aligned padded rect (box mips never mix textures).
            assert_eq!(((u0 * s) as u32 - GUTTER) % 16, 0);
            assert_eq!(((v0 * s) as u32 - GUTTER) % 16, 0);
            let page = &p.layers[layer as usize][0];
            let (x0, y0) = ((u0 * s) as u32, (v0 * s) as u32);
            // Gutter texel left of the content: wrapped -> last column, clamped -> first column.
            let left = page.get_pixel(x0 - 1, y0 + 3).0;
            let src = &t.rgba;
            if i == 1 {
                assert_eq!(left, src.get_pixel(0, 3).0);
            }
            if i == 2 {
                assert_eq!(left, src.get_pixel(t.width - 1, 3).0);
            }
        }
    }
}
