//! Image comparison against the UE5 reference: PSNR, SSIM (luma, 11x11 Gaussian), mean / p95 CIEDE76 in
//! CIELAB, mean luminance ratio, and a side-by-side sheet (reference | test | 4x error heatmap).

use anyhow::Result;
use image::{Rgb, RgbImage};
use std::path::Path;

pub fn compare(reference: &Path, test: &Path, sheet: Option<&Path>) -> Result<serde_json::Value> {
    compare_region(reference, test, sheet, None)
}

/// `compare` over a region [u0, v0, u1, v1] of the images (0..1, v down), e.g. the wet ground of a street view.
pub fn compare_region(reference: &Path, test: &Path, sheet: Option<&Path>, region: Option<[f32; 4]>) -> Result<serde_json::Value> {
    let mut a = image::open(reference)?.to_rgb8();
    let mut b = image::open(test)?.to_rgb8();
    if a.dimensions() != b.dimensions() {
        b = image::imageops::resize(&b, a.width(), a.height(), image::imageops::FilterType::Lanczos3);
    }
    if let Some([u0, v0, u1, v1]) = region {
        let (w, h) = (a.width() as f32, a.height() as f32);
        let (x, y) = ((u0 * w) as u32, (v0 * h) as u32);
        let (cw, ch) = ((((u1 - u0) * w) as u32).max(1), (((v1 - v0) * h) as u32).max(1));
        a = image::imageops::crop_imm(&a, x, y, cw, ch).to_image();
        b = image::imageops::crop_imm(&b, x, y, cw, ch).to_image();
    }
    let (w, h) = a.dimensions();
    let n = (w * h) as usize;
    let lin = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let mut mse = 0.0f64;
    let mut de = Vec::with_capacity(n);
    let (mut la, mut lb) = (0.0f64, 0.0f64);
    let mut ya = vec![0f64; n];
    let mut yb = vec![0f64; n];
    for (i, (pa, pb)) in a.pixels().zip(b.pixels()).enumerate() {
        for k in 0..3 {
            let d = pa[k] as f64 - pb[k] as f64;
            mse += d * d;
        }
        let ca = [lin(pa[0]), lin(pa[1]), lin(pa[2])];
        let cb = [lin(pb[0]), lin(pb[1]), lin(pb[2])];
        let (l1, a1, b1) = lab(ca);
        let (l2, a2, b2) = lab(cb);
        de.push(((l1 - l2).powi(2) + (a1 - a2).powi(2) + (b1 - b2).powi(2)).sqrt());
        let y = |c: [f64; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        la += y(ca);
        lb += y(cb);
        // SSIM on gamma-encoded luma (as commonly reported)
        ya[i] = 0.299 * pa[0] as f64 + 0.587 * pa[1] as f64 + 0.114 * pa[2] as f64;
        yb[i] = 0.299 * pb[0] as f64 + 0.587 * pb[1] as f64 + 0.114 * pb[2] as f64;
    }
    mse /= (n * 3) as f64;
    let psnr = if mse > 0.0 { 10.0 * (255.0f64 * 255.0 / mse).log10() } else { 99.0 };
    let ssim = ssim(&ya, &yb, w as usize, h as usize);
    let mut sorted = de.clone();
    sorted.sort_by(|x, y| x.partial_cmp(y).unwrap());
    let mean_de = de.iter().sum::<f64>() / n as f64;
    let p95 = sorted[(n as f64 * 0.95) as usize];
    let p99 = sorted[((n as f64 * 0.99) as usize).min(n - 1)];
    // Trail length: per row the longest horizontal run of clearly wrong pixels (dE > 10), p95 over rows. Noise makes
    // runs of a pixel or two; history smeared along a horizontal camera move makes runs as long as the smear.
    let mut runs: Vec<u32> = (0..h as usize)
        .map(|y| {
            let (mut best, mut cur) = (0u32, 0u32);
            for x in 0..w as usize {
                cur = if de[y * w as usize + x] > 10.0 { cur + 1 } else { 0 };
                best = best.max(cur);
            }
            best
        })
        .collect();
    runs.sort_unstable();
    let trail = runs[((runs.len() as f64 * 0.95) as usize).min(runs.len() - 1)];
    if let Some(sheet) = sheet {
        let mut out = RgbImage::new(w * 3, h);
        for y in 0..h {
            for x in 0..w {
                out.put_pixel(x, y, *a.get_pixel(x, y));
                out.put_pixel(x + w, y, *b.get_pixel(x, y));
                let e = (de[(y * w + x) as usize] / 25.0).min(1.0);
                out.put_pixel(x + 2 * w, y, heat(e));
            }
        }
        out.save(sheet)?;
    }
    Ok(serde_json::json!({
        "psnr_db": round(psnr, 2),
        "ssim": round(ssim, 4),
        "mean_delta_e": round(mean_de, 2),
        "p95_delta_e": round(p95, 2),
        "p99_delta_e": round(p99, 2),
        "trail_px": trail,
        "luminance_ratio": round(lb / la.max(1e-9), 3),
    }))
}

/// Blur across a vertical edge: per row y0..y1, the 10-90 % rise width (px) of `channel` (linear) from the first quarter
/// of [x0, x1) (the dark side) to the last quarter; median and quartiles over the rows. A box blur of length L widens a
/// sharp edge's 10-90 % rise by 0.8 L (motion blur length, docs/ROADMAP.md § 2c).
pub fn edge_width(path: &Path, x: [u32; 2], y: [u32; 2], channel: usize) -> Result<serde_json::Value> {
    let img = image::open(path)?.to_rgb8();
    let lin = |c: u8| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
    };
    let mut widths = vec![];
    for row in y[0]..y[1].min(img.height()) {
        let p: Vec<f64> = (x[0]..x[1].min(img.width())).map(|col| lin(img.get_pixel(col, row)[channel])).collect();
        let k = (p.len() / 4).max(1);
        let lo = p[..k].iter().sum::<f64>() / k as f64;
        let hi = p[p.len() - k..].iter().sum::<f64>() / k as f64;
        if hi - lo < 0.05 {
            continue;
        }
        let q: Vec<f64> = p.iter().map(|v| (v - lo) / (hi - lo)).collect();
        let cross = |t: f64| (1..q.len()).find(|&i| q[i - 1] < t && q[i] >= t).map(|i| i as f64 - 1.0 + (t - q[i - 1]) / (q[i] - q[i - 1]));
        if let (Some(a), Some(b)) = (cross(0.1), cross(0.9)) {
            if b > a {
                widths.push(b - a);
            }
        }
    }
    widths.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |f: f64| widths.get(((widths.len() as f64 - 1.0) * f).round() as usize).copied().unwrap_or(f64::NAN);
    Ok(serde_json::json!({ "rows": widths.len(), "width": round(at(0.5), 2), "p25": round(at(0.25), 2), "p75": round(at(0.75), 2) }))
}

fn round(v: f64, d: i32) -> f64 {
    let p = 10f64.powi(d);
    (v * p).round() / p
}

fn lab(c: [f64; 3]) -> (f64, f64, f64) {
    let x = (0.4124 * c[0] + 0.3576 * c[1] + 0.1805 * c[2]) / 0.95047;
    let y = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let z = (0.0193 * c[0] + 0.1192 * c[1] + 0.9505 * c[2]) / 1.08883;
    let f = |t: f64| if t > 0.008856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
}

fn heat(t: f64) -> Rgb<u8> {
    let r = (t * 3.0).clamp(0.0, 1.0);
    let g = (t * 3.0 - 1.0).clamp(0.0, 1.0);
    let b = (t * 3.0 - 2.0).clamp(0.0, 1.0);
    Rgb([(r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8])
}

fn ssim(a: &[f64], b: &[f64], w: usize, h: usize) -> f64 {
    let sigma = 1.5f64;
    let r = 5isize;
    let k: Vec<f64> = (-r..=r).map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp()).collect();
    let ks: f64 = k.iter().sum();
    let k: Vec<f64> = k.iter().map(|v| v / ks).collect();
    let blur = |src: &[f64]| -> Vec<f64> {
        let mut tmp = vec![0f64; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut s = 0.0;
                for (j, kv) in k.iter().enumerate() {
                    let xx = (x as isize + j as isize - r).clamp(0, w as isize - 1) as usize;
                    s += src[y * w + xx] * kv;
                }
                tmp[y * w + x] = s;
            }
        }
        let mut out = vec![0f64; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut s = 0.0;
                for (j, kv) in k.iter().enumerate() {
                    let yy = (y as isize + j as isize - r).clamp(0, h as isize - 1) as usize;
                    s += tmp[yy * w + x] * kv;
                }
                out[y * w + x] = s;
            }
        }
        out
    };
    let ma = blur(a);
    let mb = blur(b);
    let aa: Vec<f64> = a.iter().map(|v| v * v).collect();
    let bb: Vec<f64> = b.iter().map(|v| v * v).collect();
    let ab: Vec<f64> = a.iter().zip(b).map(|(x, y)| x * y).collect();
    let (saa, sbb, sab) = (blur(&aa), blur(&bb), blur(&ab));
    let (c1, c2) = ((0.01f64 * 255.0).powi(2), (0.03f64 * 255.0).powi(2));
    let mut total = 0.0;
    for i in 0..w * h {
        let (mu_a, mu_b) = (ma[i], mb[i]);
        let va = saa[i] - mu_a * mu_a;
        let vb = sbb[i] - mu_b * mu_b;
        let cov = sab[i] - mu_a * mu_b;
        total += ((2.0 * mu_a * mu_b + c1) * (2.0 * cov + c2)) / ((mu_a * mu_a + mu_b * mu_b + c1) * (va + vb + c2));
    }
    total / (w * h) as f64
}

/// Statistics of falling rain streaks in a region, from frames with the streaks (`on`) and the same view without them
/// (`off`): rain is random, so these are compared with UE's instead of the pixels. Luminance is the displayed one.
/// - `added`: mean luminance on / off - 1.
/// - Streak pixels: the horizontal high-pass (centre minus the mean of 2 pixels either side) above 4 x the off frames'
///   robust deviation of it. `coverage`: their share on minus off.
/// - `length_px`: mean length of their vertical runs (a run continues to a neighbour within one column in the next row),
///   runs of 3 pixels or more.
/// - `angle_deg`: their mean orientation from vertical (structure tensor of the high-pass), positive leaning right.
pub fn rain_stats(on: &[&Path], off: &[&Path], region: Option<[f32; 4]>) -> Result<serde_json::Value> {
    let load = |p: &Path| -> Result<(u32, u32, Vec<f32>)> {
        let img = image::open(p)?.to_rgb8();
        let (w, h) = img.dimensions();
        let (x0, y0, x1, y1) = match region {
            Some([u0, v0, u1, v1]) => ((u0 * w as f32) as u32, (v0 * h as f32) as u32, (u1 * w as f32) as u32, (v1 * h as f32) as u32),
            None => (0, 0, w, h),
        };
        let mut l = Vec::with_capacity(((x1 - x0) * (y1 - y0)) as usize);
        for y in y0..y1 {
            for x in x0..x1 {
                let p = img.get_pixel(x, y).0;
                l.push((0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32) / 255.0);
            }
        }
        Ok((x1 - x0, y1 - y0, l))
    };
    let hp = |w: u32, h: u32, l: &[f32]| -> Vec<f32> {
        let (w, h) = (w as usize, h as usize);
        let mut o = vec![0.0f32; w * h];
        for y in 0..h {
            for x in 2..w.saturating_sub(2) {
                let i = y * w + x;
                o[i] = l[i] - 0.25 * (l[i - 2] + l[i - 1] + l[i + 1] + l[i + 2]);
            }
        }
        o
    };
    let offs: Vec<_> = off.iter().map(|p| load(p)).collect::<Result<_>>()?;
    let ons: Vec<_> = on.iter().map(|p| load(p)).collect::<Result<_>>()?;
    let mean = |v: &[(u32, u32, Vec<f32>)]| v.iter().map(|(_, _, l)| l.iter().sum::<f32>() / l.len().max(1) as f32).sum::<f32>() / v.len().max(1) as f32;
    // Robust deviation of the off frames' high-pass (1.4826 x median absolute value).
    let mut all: Vec<f32> = offs.iter().flat_map(|(w, h, l)| hp(*w, *h, l)).map(f32::abs).collect();
    all.sort_by(|a, b| a.total_cmp(b));
    let sigma = 1.4826 * all.get(all.len() / 2).copied().unwrap_or(0.0);
    let thr = 4.0 * sigma.max(1e-4);
    let streaks = |frames: &[(u32, u32, Vec<f32>)]| -> (f64, f64, f64, f64) {
        let (mut n, mut tot, mut runs, mut run_px, mut sxx, mut syy, mut sxy) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
        for (w, h, l) in frames {
            let (w, h) = (*w as usize, *h as usize);
            let f = hp(w as u32, h as u32, l);
            let m: Vec<bool> = f.iter().map(|v| *v > thr).collect();
            n += m.iter().filter(|b| **b).count() as f64;
            tot += (w * h) as f64;
            // Orientation: gradients of the high-pass at streak pixels.
            for y in 1..h.saturating_sub(1) {
                for x in 3..w.saturating_sub(3) {
                    let i = y * w + x;
                    if !m[i] {
                        continue;
                    }
                    let gx = (f[i + 1] - f[i - 1]) as f64;
                    let gy = (f[i + w] - f[i - w]) as f64;
                    sxx += gx * gx;
                    syy += gy * gy;
                    sxy += gx * gy;
                }
            }
            // Vertical runs: each streak pixel not continued from the row above starts a run.
            let mut seen = vec![false; w * h];
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    if !m[i] || seen[i] {
                        continue;
                    }
                    let (mut cx, mut len) = (x, 0usize);
                    let mut cy = y;
                    loop {
                        seen[cy * w + cx] = true;
                        len += 1;
                        if cy + 1 >= h {
                            break;
                        }
                        let nx = [cx, cx.saturating_sub(1), (cx + 1).min(w - 1)].into_iter().find(|&nx| m[(cy + 1) * w + nx] && !seen[(cy + 1) * w + nx]);
                        match nx {
                            Some(nx) => {
                                cx = nx;
                                cy += 1;
                            }
                            None => break,
                        }
                    }
                    if len >= 3 {
                        runs += 1.0;
                        run_px += len as f64;
                    }
                }
            }
        }
        // The streak direction is across the dominant gradient (gradients point across a thin line).
        let theta = 0.5 * (2.0 * sxy).atan2(sxx - syy);
        let along = theta + std::f64::consts::FRAC_PI_2;
        let mut deg = along.to_degrees();
        while deg > 90.0 {
            deg -= 180.0;
        }
        while deg < -90.0 {
            deg += 180.0;
        }
        // From vertical: 90 deg away from horizontal (x right, y down in image rows).
        let from_vertical = if deg >= 0.0 { 90.0 - deg } else { -90.0 - deg };
        (n / tot.max(1.0), if runs > 0.0 { run_px / runs } else { 0.0 }, from_vertical, runs / frames.len().max(1) as f64)
    };
    let (cov_on, len_on, ang_on, runs_on) = streaks(&ons);
    let (cov_off, _, _, runs_off) = streaks(&offs);
    let (m_on, m_off) = (mean(&ons), mean(&offs));
    Ok(serde_json::json!({
        "added": m_on / m_off.max(1e-6) - 1.0,
        "coverage": cov_on - cov_off,
        "length_px": len_on,
        "angle_deg": ang_on,
        "runs_per_frame": runs_on - runs_off,
        "threshold": thr,
        "frames": [ons.len(), offs.len()],
    }))
}

/// Radiometric comparison (no exposure / tonemapping): a scene-linear reference image (e.g. an Unreal EXR rendered
/// with the tone curve disabled at exposure 2^-ev100) against our raw HDR (`render --hdr`, f32 RGB, unexposed).
/// Reports luminance ratios overall and on a 4x4 grid; writes a log2(test / reference) heat map when asked.
pub fn compare_hdr(reference_exr: &Path, test_raw: &Path, ev100: f32, heat: Option<&Path>) -> Result<serde_json::Value> {
    let r = image::open(reference_exr)?.to_rgb32f();
    let (w, h) = (r.width() as usize, r.height() as usize);
    let raw = std::fs::read(test_raw)?;
    let t: &[f32] = bytemuck::cast_slice(&raw);
    anyhow::ensure!(t.len() == w * h * 3, "test raw is {} floats, reference is {w}x{h}", t.len());
    let k = 2f32.powf(-ev100);
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let (mut sr, mut st) = (0f64, 0f64);
    let mut grid = vec![[0f64; 2]; 16];
    let mut logs = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let rp = r.get_pixel(x as u32, y as u32).0;
            let i = (y * w + x) * 3;
            let tp = [t[i] * k, t[i + 1] * k, t[i + 2] * k];
            let (lr, lt) = (lum(rp) as f64, lum(tp) as f64);
            sr += lr;
            st += lt;
            let g = &mut grid[(y * 4 / h) * 4 + x * 4 / w];
            g[0] += lr;
            g[1] += lt;
            logs.push(((lt + 1e-5) / (lr + 1e-5)).log2() as f32);
        }
    }
    if let Some(p) = heat {
        // blue = ours darker, red = ours brighter, white = equal (+-3 EV full scale)
        let img = image::RgbImage::from_fn(w as u32, h as u32, |x, y| {
            let v = (logs[y as usize * w + x as usize] / 3.0).clamp(-1.0, 1.0);
            let (a, b) = if v >= 0.0 { (255.0, 255.0 * (1.0 - v)) } else { (255.0 * (1.0 + v), 255.0) };
            if v >= 0.0 { image::Rgb([a as u8, b as u8, b as u8]) } else { image::Rgb([a as u8, a as u8, b as u8]) }
        });
        img.save(p)?;
    }
    let mut sorted = logs.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];
    Ok(serde_json::json!({
        "luminance_ratio": (st / sr.max(1e-12) * 1000.0).round() / 1000.0,
        "median_log2_ratio": (median * 100.0).round() / 100.0,
        "grid_log2_ratio": grid.iter().map(|g| (((g[1] + 1e-9) / (g[0] + 1e-9)).log2() * 100.0).round() / 100.0).collect::<Vec<_>>(),
    }))
}
