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
        return Packed { side: ALIGN, layers: vec![mip_chain(img)], info: vec![[0.0; 4], [0.0; 4]], streams: vec![] };
    }
    // Power-of-two textures >= 512 give up 2 x GUTTER texels so the padded rect stays a power of two and four
    // 2048s (or sixteen 1024s) fill a page exactly; anything else keeps its size and is padded.
    let src: Vec<std::borrow::Cow<image::RgbaImage>> = textures
        .par_iter()
        .map(|t| {
            let (w, h) = (t.width.min(MAX_DIM), t.height.min(MAX_DIM));
            let k = (w as f32 / t.width as f32).min(h as f32 / t.height as f32);
            let (mut w, mut h) = (((t.width as f32 * k) as u32).max(1), ((t.height as f32 * k) as u32).max(1));
            if w >= 512 && w.is_power_of_two() {
                w -= 2 * GUTTER;
            }
            if h >= 512 && h.is_power_of_two() {
                h -= 2 * GUTTER;
            }
            if (w, h) == (t.width, t.height) {
                std::borrow::Cow::Borrowed(&*t.rgba)
            } else {
                std::borrow::Cow::Owned(resize_triangle(&t.rgba, w, h))
            }
        })
        .collect();
    let padded: Vec<(u32, u32)> = src.iter().map(|i| (round_up(i.width() + 2 * GUTTER), round_up(i.height() + 2 * GUTTER))).collect();
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
        }
        place[i] = (layer, x, y);
        x += w;
        shelf = shelf.max(h);
    }
    let mut info = vec![[0.0f32; 4]; src.len() * 2];
    for (i, img) in src.iter().enumerate() {
        let (l, px, py) = place[i];
        let (w, h) = (img.width(), img.height());
        let s = side as f32;
        info[i * 2] = [(px + GUTTER) as f32 / s, (py + GUTTER) as f32 / s, w as f32 / s, h as f32 / s];
        info[i * 2 + 1] = [l as f32, textures[i].wrap as u32 as f32, w as f32, h as f32];
    }
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
    let streams = textures
        .iter()
        .enumerate()
        .filter_map(|(i, t)| {
            let (layer, x, y) = place[i];
            Some((stream_name(&t.key)?.to_string(), Place { layer, x, y, padded: padded[i], size: src[i].dimensions(), wrap: t.wrap }))
        })
        .collect();
    Packed { side, layers: pages.into_par_iter().map(mip_chain).collect(), info, streams }
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
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for t in textures {
        t.key.hash(&mut h);
        (t.width, t.height, t.wrap).hash(&mut h);
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
