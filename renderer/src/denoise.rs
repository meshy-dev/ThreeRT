//! Edge-avoiding a-trous wavelet denoiser (Dammertz et al. 2010) guided by first-hit albedo / normal and
//! per-pixel luminance variance of the mean. Operates on demodulated irradiance so texture detail survives.
//! The luminance edge-stopping width comes from the variance filtered over 3 x 3 pixels (SVGF, Schied et al. 2017):
//! with each pixel's own, a pixel among fireflies that drew none (low mean, low variance) rejected all its neighbours
//! and stayed a dark hole (Bistro's glossy shop windows, Zero Day's machine tops).

use glam::Vec3;
use rayon::prelude::*;

pub fn denoise(w: usize, h: usize, color: &[Vec3], albedo: &[Vec3], normal: &[Vec3], variance: &[f32], iterations: u32) -> Vec<Vec3> {
    let n = w * h;
    let eps = Vec3::splat(1e-3);
    let demod: Vec<Vec3> = (0..n).map(|i| color[i] / albedo[i].max(eps)).collect();
    let mut cur = demod;
    let mut var: Vec<f32> = (0..n).map(|i| variance[i] / albedo[i].max(eps).dot(Vec3::new(0.2126, 0.7152, 0.0722)).powi(2)).collect();
    const K: [f32; 3] = [3.0 / 8.0, 1.0 / 4.0, 1.0 / 16.0];
    for it in 0..iterations {
        let step = 1isize << it;
        // 3 x 3 Gaussian of the variance (the edge-stopping width; the propagated variance stays per pixel).
        let var_f: Vec<f32> = (0..n)
            .into_par_iter()
            .map(|i| {
                let (x, y) = ((i % w) as isize, (i / w) as isize);
                let (mut s, mut ws) = (0.0f32, 0.0f32);
                for dy in -1isize..=1 {
                    for dx in -1isize..=1 {
                        let (qx, qy) = (x + dx, y + dy);
                        if qx < 0 || qy < 0 || qx >= w as isize || qy >= h as isize {
                            continue;
                        }
                        let k = [0.5f32, 0.25][dx.unsigned_abs().min(1)] * [0.5f32, 0.25][dy.unsigned_abs().min(1)];
                        s += k * var[qy as usize * w + qx as usize];
                        ws += k;
                    }
                }
                s / ws
            })
            .collect();
        let (next, next_var): (Vec<Vec3>, Vec<f32>) = (0..n)
            .into_par_iter()
            .map(|i| {
                let (x, y) = ((i % w) as isize, (i / w) as isize);
                let cp = cur[i];
                let lp = luminance(cp);
                let np = normal[i];
                let ap = albedo[i];
                let sigma_l = 4.0 * var_f[i].max(0.0).sqrt() + 1e-4;
                let (mut sum, mut wsum, mut vsum) = (Vec3::ZERO, 0.0f32, 0.0f32);
                for dy in -2isize..=2 {
                    for dx in -2isize..=2 {
                        let (qx, qy) = (x + dx * step, y + dy * step);
                        if qx < 0 || qy < 0 || qx >= w as isize || qy >= h as isize {
                            continue;
                        }
                        let q = qy as usize * w + qx as usize;
                        let k = K[dx.unsigned_abs()] * K[dy.unsigned_abs()];
                        let cq = cur[q];
                        let wl = (-(lp - luminance(cq)).abs() / sigma_l).exp();
                        let wn = np.dot(normal[q]).max(0.0).powi(64);
                        let wa = (-(ap - albedo[q]).length_squared() / 0.01).exp();
                        let wt = k * wl * wn * wa;
                        sum += cq * wt;
                        vsum += wt * wt * var[q];
                        wsum += wt;
                    }
                }
                if wsum > 0.0 { (sum / wsum, vsum / (wsum * wsum)) } else { (cp, var[i]) }
            })
            .unzip();
        cur = next;
        var = next_var;
    }
    (0..n).map(|i| cur[i] * albedo[i].max(eps)).collect()
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// Intel Open Image Denoise (RT filter, albedo + normal guided). `None` if unavailable at runtime.
#[cfg(feature = "oidn")]
pub fn oidn(w: usize, h: usize, color: &[Vec3], albedo: &[Vec3], normal: &[Vec3], exposure: f32) -> Option<Vec<Vec3>> {
    thread_local! {
        static DEVICE: Option<oidn::Device> = oidn::Device::new().ok();
    }
    DEVICE.with(|dev| {
        let dev = dev.as_ref()?;
        let flat = |v: &[Vec3]| -> Vec<f32> { v.iter().flat_map(|c| [c.x, c.y, c.z]).collect() };
        let (c, a, n) = (flat(color), flat(albedo), flat(normal));
        let mut out = vec![0f32; c.len()];
        let mut f = oidn::RayTracing::new(dev);
        f.srgb(false).hdr(true).hdr_scale(exposure).image_dimensions(w, h).albedo_normal(&a, &n).filter_quality(oidn::Quality::High);
        if let Err(e) = f.filter(&c, &mut out) {
            eprintln!("oidn failed: {e:?}");
            return None;
        }
        if let Err(e) = dev.get_error() {
            eprintln!("oidn error: {e:?}");
            return None;
        }
        Some(out.chunks(3).map(|p| Vec3::new(p[0], p[1], p[2])).collect())
    })
}

#[cfg(not(feature = "oidn"))]
pub fn oidn(_w: usize, _h: usize, _c: &[Vec3], _a: &[Vec3], _n: &[Vec3], _e: f32) -> Option<Vec<Vec3>> {
    None
}
