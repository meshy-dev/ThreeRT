//! UE5 post-processing chain ported from the engine shaders (UE 5.8):
//!   PostProcessBloomSetup.cpp / PostProcessWeightedSampleSum.cpp (Gaussian "SOG" bloom),
//!   PostProcessEyeAdaptation.cpp (manual / histogram exposure),
//!   PostProcessTonemap.usf + PostProcessCombineLUTs.usf + TonemapCommon.ush (Filmic tonemapper),
//!   PostProcessCommon.ush (cosine-fourth vignette).

use crate::desc::{Exposure, PostPreset};
use glam::{Mat3, Vec3};
use rayon::prelude::*;

pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<Vec3>,
}

impl Image {
    fn at(&self, x: isize, y: isize) -> Vec3 {
        let x = x.clamp(0, self.w as isize - 1) as usize;
        let y = y.clamp(0, self.h as isize - 1) as usize;
        self.px[y * self.w + x]
    }
    fn sample(&self, u: f32, v: f32) -> Vec3 {
        // Bilinear, u/v in [0,1] texture space.
        let x = u * self.w as f32 - 0.5;
        let y = v * self.h as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let a = self.at(x0, y0).lerp(self.at(x0 + 1, y0), fx);
        let b = self.at(x0, y0 + 1).lerp(self.at(x0 + 1, y0 + 1), fx);
        a.lerp(b, fy)
    }
    fn downsample(&self) -> Image {
        let (w, h) = ((self.w / 2).max(1), (self.h / 2).max(1));
        let px = (0..w * h)
            .into_par_iter()
            .map(|i| {
                let (x, y) = ((i % w) as isize * 2, (i / w) as isize * 2);
                (self.at(x, y) + self.at(x + 1, y) + self.at(x, y + 1) + self.at(x + 1, y + 1)) * 0.25
            })
            .collect();
        Image { w, h, px }
    }
}

/// UE Compute1DGaussianFilterKernel: weights exp(-16.7 (x/R)^2), radius clamped to 31 texels.
fn gaussian_kernel(radius: f32) -> Vec<(isize, f32)> {
    let r = radius.clamp(1e-4, 31.0);
    let ir = r.ceil().min(31.0) as isize;
    let mut k: Vec<(isize, f32)> = (-ir..=ir).map(|x| (x, (-16.7 * (x as f32 / r).powi(2)).exp())).collect();
    let s: f32 = k.iter().map(|w| w.1).sum();
    for w in &mut k {
        w.1 /= s;
    }
    k
}

fn blur(img: &Image, radius: f32) -> Image {
    let k = gaussian_kernel(radius);
    let (w, h) = (img.w, img.h);
    let tmp: Vec<Vec3> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            k.iter().map(|(o, wt)| img.at(x + o, y) * *wt).sum()
        })
        .collect();
    let tmp = Image { w, h, px: tmp };
    let px = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            k.iter().map(|(o, wt)| tmp.at(x, y + o) * *wt).sum()
        })
        .collect();
    Image { w, h, px }
}

/// Gaussian bloom at half resolution (BloomQuality 5: six stages on a 1/2 .. 1/64 downsample chain).
pub fn bloom(scene: &Image, p: &PostPreset) -> Option<Image> {
    if p.bloom.intensity <= 0.0 {
        return None;
    }
    let thresholded = if p.bloom.threshold > 0.0 {
        // UE: Color * saturate((Luminance(Color) - Threshold) * 0.5), Luminance = dot(c, (0.3, 0.59, 0.11)).
        let t = p.bloom.threshold;
        Image { w: scene.w, h: scene.h, px: scene.px.iter().map(|c| *c * ((c.dot(Vec3::new(0.3, 0.59, 0.11)) - t) * 0.5).clamp(0.0, 1.0)).collect() }
    } else {
        Image { w: scene.w, h: scene.h, px: scene.px.clone() }
    };
    let mut chain = vec![thresholded.downsample()];
    for _ in 1..6 {
        let next = chain.last().unwrap().downsample();
        chain.push(next);
    }
    let tint_scale = p.bloom.intensity / 6.0;
    let mut out: Option<Image> = None;
    // Largest kernel (lowest resolution) first, accumulating upward like UE's additive passes.
    for stage in (0..6).rev() {
        let src = &chain[stage];
        let size = p.bloom.sizes[stage] * p.bloom.size_scale;
        let radius = src.w as f32 * size * 0.01 * 0.5;
        let blurred = blur(src, radius);
        let tint = p.bloom.tints[stage] * tint_scale;
        let px: Vec<Vec3> = (0..src.w * src.h)
            .into_par_iter()
            .map(|i| {
                let mut c = blurred.px[i] * tint;
                if let Some(prev) = &out {
                    let (x, y) = ((i % src.w) as f32 + 0.5, (i / src.w) as f32 + 0.5);
                    c += prev.sample(x / src.w as f32, y / src.h as f32);
                }
                c
            })
            .collect();
        out = Some(Image { w: src.w, h: src.h, px });
    }
    out
}

/// Exposure scale applied to scene radiance (nits). Manual: 2^-EV100 (UE: LuminanceMax = 1 with
/// r.EyeAdaptation.LensAttenuation 0.78 and extended luminance range). Auto: UE histogram method.
pub fn exposure_scale(img: &Image, exp: &Exposure, p: &PostPreset) -> (f32, f32) {
    match exp {
        Exposure::Manual { ev100 } => (2f32.powf(-ev100), *ev100),
        Exposure::Auto { compensation, min_ev100, max_ev100 } => {
            // UE 5 histogram eye adaptation (PostProcessHistogram.usf / PostProcessHistogramCommon.ush): 64 buckets
            // over [HistogramLogMin, HistogramLogMax] (EV100 = log2 luminance with the extended range), uniform RGB
            // weights (r.AutoExposure.LuminanceMethod 0), no screen weighting (default meter mask), each sample split
            // between its two buckets; the average log luminance between the low / high percentiles.
            let (lo, hi) = (p.auto_exposure.histogram_log_min, p.auto_exposure.histogram_log_max.max(p.auto_exposure.histogram_log_min + 1.0));
            let bins = 64usize;
            let mut hist = vec![0f64; bins];
            for c in &img.px {
                let l = ((c.x + c.y + c.z) / 3.0).max(1e-10);
                let fb = ((l.log2() - lo) / (hi - lo)).clamp(0.0, 1.0) * (bins - 1) as f32;
                let b0 = (fb.floor() as usize).min(bins - 1);
                let w1 = (fb - b0 as f32) as f64;
                hist[b0] += 1.0 - w1;
                hist[(b0 + 1).min(bins - 1)] += w1;
            }
            let total: f64 = hist.iter().sum();
            let (mut min_f, mut max_f) = (p.auto_exposure.low_percent.clamp(1.0, 99.0) as f64 / 100.0 * total, p.auto_exposure.high_percent.clamp(1.0, 99.0) as f64 / 100.0 * total);
            min_f = min_f.min(max_f);
            let (mut sum, mut cnt) = (0f64, 0f64);
            for (b, v) in hist.iter().enumerate() {
                let mut v = *v;
                let sub = v.min(min_f);
                v -= sub;
                min_f -= sub;
                max_f -= sub;
                v = v.min(max_f);
                max_f -= v;
                let log_l = lo + b as f32 / (bins - 1) as f32 * (hi - lo);
                sum += v * log_l as f64;
                cnt += v;
            }
            let avg = if cnt > 0.0 { 2f32.powf((sum / cnt) as f32) } else { 0.18 * 2f32.powf(*min_ev100) / 0.18 };
            let avg = avg.clamp(0.18 * 2f32.powf(*min_ev100), 0.18 * 2f32.powf(*max_ev100));
            let scale = 0.18 * 2f32.powf(*compensation) / avg;
            (scale, -scale.log2())
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Filmic tonemapper (TonemapCommon.ush FilmToneMap + PostProcessCombineLUTs.usf)

fn m3(r: [[f32; 3]; 3]) -> Mat3 {
    // HLSL initializers are row-major; glam is column-major.
    Mat3::from_cols_array_2d(&r).transpose()
}

struct Matrices {
    srgb_to_ap1: Mat3,
    ap1_to_srgb: Mat3,
    ap0_to_ap1: Mat3,
    ap1_to_ap0: Mat3,
    expand: Mat3,
    blue: Mat3,
    blue_inv: Mat3,
}

fn matrices() -> Matrices {
    let ap0_to_xyz = m3([[0.9525523959, 0.0, 0.0000936786], [0.3439664498, 0.7281660966, -0.0721325464], [0.0, 0.0, 1.0088251844]]);
    let xyz_to_ap0 = m3([[1.0498110175, 0.0, -0.0000974845], [-0.4959030231, 1.3733130458, 0.0982400361], [0.0, 0.0, 0.9912520182]]);
    let ap1_to_xyz = m3([[0.6624541811, 0.1340042065, 0.1561876870], [0.2722287168, 0.6740817658, 0.0536895174], [-0.0055746495, 0.0040607335, 1.0103391003]]);
    let xyz_to_ap1 = m3([[1.6410233797, -0.3248032942, -0.2364246952], [-0.6636628587, 1.6153315917, 0.0167563477], [0.0117218943, -0.0082844420, 0.9883948585]]);
    let xyz_to_srgb = m3([[3.2409699419, -1.5373831776, -0.4986107603], [-0.9692436363, 1.8759675015, 0.0415550574], [0.0556300797, -0.2039769589, 1.0569715142]]);
    let srgb_to_xyz = m3([[0.4123907993, 0.3575843394, 0.1804807884], [0.2126390059, 0.7151686788, 0.0721923154], [0.0193308187, 0.1191947798, 0.9505321522]]);
    let d65_to_d60 = m3([[1.0130349146, 0.0061052578, -0.0149709436], [0.0076982301, 0.9981633521, -0.0050320385], [-0.0028413174, 0.0046851567, 0.9245061375]]);
    let d60_to_d65 = m3([[0.9872240087, -0.0061132286, 0.0159532883], [-0.0075983718, 1.0018614847, 0.0053300358], [0.0030725771, -0.0050959615, 1.0816806031]]);
    let srgb_to_ap1 = xyz_to_ap1 * d65_to_d60 * srgb_to_xyz;
    let ap1_to_srgb = xyz_to_srgb * d60_to_d65 * ap1_to_xyz;
    let wide_to_xyz = m3([[0.5441691, 0.2395926, 0.1666943], [0.2394656, 0.7021530, 0.0583814], [-0.0023439, 0.0361834, 1.0552183]]);
    let ap0_to_ap1 = xyz_to_ap1 * ap0_to_xyz;
    let ap1_to_ap0 = xyz_to_ap0 * ap1_to_xyz;
    let blue_ap0 = m3([[0.9404372683, -0.0183068787, 0.0778696104], [0.0083786969, 0.8286599939, 0.1629613092], [0.0005471261, -0.0008833746, 1.0003362486]]);
    let blue_inv_ap0 = m3([[1.06318, 0.0233956, -0.0865726], [-0.0106337, 1.20632, -0.19569], [-0.000590887, 0.00105248, 0.999538]]);
    Matrices {
        srgb_to_ap1,
        ap1_to_srgb,
        ap0_to_ap1,
        ap1_to_ap0,
        expand: (xyz_to_ap1 * wide_to_xyz) * ap1_to_srgb,
        blue: ap0_to_ap1 * blue_ap0 * ap1_to_ap0,
        blue_inv: ap0_to_ap1 * blue_inv_ap0 * ap1_to_ap0,
    }
}

const AP1_RGB2Y: Vec3 = Vec3::new(0.2722287168, 0.6740817658, 0.0536895174);

fn rgb_2_saturation(c: Vec3) -> f32 {
    let mn = c.min_element();
    let mx = c.max_element();
    (mx.max(1e-10) - mn.max(1e-10)) / mx.max(1e-2)
}

fn rgb_2_yc(c: Vec3) -> f32 {
    let (r, g, b) = (c.x, c.y, c.z);
    let chroma = (b * (b - g) + g * (g - r) + r * (r - b)).max(0.0).sqrt();
    (b + g + r + 1.75 * chroma) / 3.0
}

fn sigmoid_shaper(x: f32) -> f32 {
    let t = (1.0 - (0.5 * x).abs()).max(0.0);
    let y = 1.0 + x.signum() * (1.0 - t * t);
    0.5 * y
}

fn glow_fwd(yc: f32, gain: f32, mid: f32) -> f32 {
    if yc <= 2.0 / 3.0 * mid {
        gain
    } else if yc >= 2.0 * mid {
        0.0
    } else {
        gain * (mid / yc - 0.5)
    }
}

fn rgb_2_hue(c: Vec3) -> f32 {
    let mut hue = if c.x == c.y && c.y == c.z { 0.0 } else { (180.0 / std::f32::consts::PI) * (3f32.sqrt() * (c.y - c.z)).atan2(2.0 * c.x - c.y - c.z) };
    if hue < 0.0 {
        hue += 360.0;
    }
    hue.clamp(0.0, 360.0)
}

fn center_hue(hue: f32, center: f32) -> f32 {
    let mut h = hue - center;
    if h < -180.0 {
        h += 360.0;
    } else if h > 180.0 {
        h -= 360.0;
    }
    h
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub struct Tonemapper {
    m: Matrices,
    /// White balance (Bradford, scene white -> D65) in linear sRGB; identity at 6500 K / tint 0.
    white_balance: Mat3,
    saturation: Vec3,
    contrast: Vec3,
    shadows_gain: Vec3,
    highlights_gain: Vec3,
    slope: f32,
    toe: f32,
    shoulder: f32,
    black_clip: f32,
    white_clip: f32,
    tone_curve_amount: f32,
    expand_gamut: f32,
    blue_correction: f32,
}

impl Tonemapper {
    pub fn new(p: &PostPreset) -> Self {
        let t = &p.tonemapper;
        let g = &p.color_grading;
        Tonemapper {
            m: matrices(),
            white_balance: white_balance(g.white_temp, g.white_tint),
            saturation: Vec3::from(g.saturation.0),
            contrast: Vec3::from(g.contrast.0),
            shadows_gain: Vec3::from(g.shadows_gain.0),
            highlights_gain: Vec3::from(g.highlights_gain.0),
            slope: t.slope,
            toe: t.toe,
            shoulder: t.shoulder,
            black_clip: t.black_clip,
            white_clip: t.white_clip,
            tone_curve_amount: t.tone_curve_amount,
            expand_gamut: p.color_grading.expand_gamut,
            blue_correction: p.color_grading.blue_correction,
        }
    }

    fn film_tonemap(&self, color_ap1: Vec3) -> Vec3 {
        let m = &self.m;
        let mut ap0 = m.ap1_to_ap0 * color_ap1;
        // ACES RRT glow module
        let sat = rgb_2_saturation(ap0);
        let yc = rgb_2_yc(ap0);
        let s = sigmoid_shaper((sat - 0.4) / 0.2);
        let glow = 1.0 + glow_fwd(yc, 0.05 * s, 0.08);
        ap0 *= glow;
        // Red modifier
        let hue = rgb_2_hue(ap0);
        let ch = center_hue(hue, 0.0);
        let hw = smoothstep(0.0, 1.0, 1.0 - (2.0 * ch / 135.0).abs()).powi(2);
        ap0.x += hw * sat * (0.03 - ap0.x) * (1.0 - 0.82);
        let mut wc = (m.ap0_to_ap1 * ap0).max(Vec3::ZERO);
        wc = Vec3::splat(wc.dot(AP1_RGB2Y)).lerp(wc, 0.96);
        let toe_scale = 1.0 + self.black_clip - self.toe;
        let shoulder_scale = 1.0 + self.white_clip - self.shoulder;
        let (in_match, out_match) = (0.18f32, 0.18f32);
        let toe_match = if self.toe > 0.8 {
            (1.0 - self.toe - out_match) / self.slope + in_match.log10()
        } else {
            let bt = (out_match + self.black_clip) / toe_scale - 1.0;
            in_match.log10() - 0.5 * ((1.0 + bt) / (1.0 - bt)).ln() * (toe_scale / self.slope)
        };
        let straight_match = (1.0 - self.toe) / self.slope - toe_match;
        let shoulder_match = self.shoulder / self.slope - straight_match;
        let f = |c: f32| -> f32 {
            let log_c = c.max(1e-10).log10();
            let straight = self.slope * (log_c + straight_match);
            let mut toe = -self.black_clip + (2.0 * toe_scale) / (1.0 + ((-2.0 * self.slope / toe_scale) * (log_c - toe_match)).exp());
            let mut sh = (1.0 + self.white_clip) - (2.0 * shoulder_scale) / (1.0 + ((2.0 * self.slope / shoulder_scale) * (log_c - shoulder_match)).exp());
            if log_c >= toe_match {
                toe = straight;
            }
            if log_c <= shoulder_match {
                sh = straight;
            }
            let mut t = ((log_c - toe_match) / (shoulder_match - toe_match)).clamp(0.0, 1.0);
            if shoulder_match < toe_match {
                t = 1.0 - t;
            }
            t = (3.0 - 2.0 * t) * t * t;
            toe + (sh - toe) * t
        };
        let tone = Vec3::new(f(wc.x), f(wc.y), f(wc.z));
        Vec3::splat(tone.dot(AP1_RGB2Y)).lerp(tone, 0.93).max(Vec3::ZERO)
    }

    /// Exposed linear sRGB scene color -> display-referred linear sRGB (before the sRGB OETF).
    pub fn apply(&self, linear: Vec3) -> Vec3 {
        let m = &self.m;
        let mut ap1 = m.srgb_to_ap1 * (self.white_balance * linear);
        ap1 = self.grade(ap1);
        let luma = ap1.dot(AP1_RGB2Y);
        let chroma = ap1 / luma.max(1e-10);
        let cd = (chroma - Vec3::ONE).length_squared();
        let expand_amount = (1.0 - (-4.0 * cd).exp2()) * (1.0 - (-4.0 * self.expand_gamut * luma * luma).exp2());
        if luma > 0.0 {
            ap1 = ap1.lerp(m.expand * ap1, expand_amount);
        }
        ap1 = ap1.lerp(m.blue * ap1, self.blue_correction);
        let toned = self.film_tonemap(ap1);
        ap1 = ap1.lerp(toned, self.tone_curve_amount);
        ap1 = ap1.lerp(m.blue_inv * ap1, self.blue_correction);
        (m.ap1_to_srgb * ap1).max(Vec3::ZERO)
    }
}

impl Tonemapper {
    /// UE ColorCorrect (PostProcessCombineLUTs): saturation and contrast around 0.18, then shadow / highlight gains
    /// weighted by luma (ColorCorrectionShadowsMax 0.09, HighlightsMin 0.5 defaults).
    fn grade(&self, ap1: Vec3) -> Vec3 {
        let luma = ap1.dot(AP1_RGB2Y);
        let mut c = (Vec3::splat(luma) + (ap1 - Vec3::splat(luma)) * self.saturation).max(Vec3::ZERO);
        if self.contrast != Vec3::ONE {
            let k = c / 0.18;
            c = Vec3::new(k.x.powf(self.contrast.x), k.y.powf(self.contrast.y), k.z.powf(self.contrast.z)) * 0.18;
        }
        let ws = 1.0 - smoothstep(0.0, 0.09, luma);
        let wh = smoothstep(0.5, 1.0, luma);
        c * (ws * self.shadows_gain + wh * self.highlights_gain + Vec3::splat(1.0 - ws - wh))
    }
}

/// UE WhiteBalance: chromaticity of the D-series illuminant (Planckian locus below 4000 K) at `temp`, shifted by
/// `tint` along the isotherm, adapted to D65 with the Bradford transform; returned as a linear-sRGB matrix.
fn white_balance(temp: f32, tint: f32) -> Mat3 {
    if (temp - 6500.0).abs() < 1.0 && tint == 0.0 {
        return Mat3::IDENTITY;
    }
    let t = temp.clamp(1900.0, 25000.0) as f64;
    let (mut x, mut y) = if t >= 4000.0 {
        let x = if t <= 7000.0 { 0.244063 + 0.09911e3 / t + 2.9678e6 / (t * t) - 4.6070e9 / (t * t * t) } else { 0.237040 + 0.24748e3 / t + 1.9018e6 / (t * t) - 2.0064e9 / (t * t * t) };
        (x, -3.0 * x * x + 2.87 * x - 0.275)
    } else {
        // Krystek 1985 Planckian locus in uv, then to xy.
        let u = (0.860117757 + 1.54118254e-4 * t + 1.28641212e-7 * t * t) / (1.0 + 8.42420235e-4 * t + 7.08145163e-7 * t * t);
        let v = (0.317398726 + 4.22806245e-5 * t + 4.20481691e-8 * t * t) / (1.0 - 2.89741816e-5 * t + 1.61456053e-7 * t * t);
        (3.0 * u / (2.0 * u - 8.0 * v + 4.0), 2.0 * v / (2.0 * u - 8.0 * v + 4.0))
    };
    if tint != 0.0 {
        // Offset perpendicular to the locus in uv (UE: Tint * 0.05 / sqrt(du^2 + dv^2)), approximated in xy.
        let k = tint as f64 * 0.05 * 0.5;
        let (dx, dy) = (-(y - 0.3290), x - 0.3127);
        let l = (dx * dx + dy * dy).sqrt().max(1e-6);
        x += k * dx / l;
        y += k * dy / l;
    }
    let xyz = |x: f64, y: f64| Vec3::new((x / y) as f32, 1.0, ((1.0 - x - y) / y) as f32);
    let bradford = m3([[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]]);
    let src = bradford * xyz(x, y);
    let dst = bradford * xyz(0.3127, 0.3290);
    let cat = bradford.inverse() * Mat3::from_diagonal(dst / src) * bradford;
    let srgb_to_xyz = m3([[0.4123907993, 0.3575843394, 0.1804807884], [0.2126390059, 0.7151686788, 0.0721923154], [0.0193308187, 0.1191947798, 0.9505321522]]);
    srgb_to_xyz.inverse() * cat * srgb_to_xyz
}

/// Per-pixel film grain (UE FilmGrainIntensity): mid-tone weighted multiplicative noise.
pub fn grain(d: Vec3, intensity: f32, hash: u32) -> Vec3 {
    if intensity <= 0.0 {
        return d;
    }
    let mut h = hash.wrapping_mul(747796405).wrapping_add(2891336453);
    h = ((h >> ((h >> 28) + 4)) ^ h).wrapping_mul(277803737);
    h = (h >> 22) ^ h;
    let n = (h >> 8) as f32 / 16777216.0 - 0.5;
    let l = d.dot(Vec3::new(0.2126, 0.7152, 0.0722)).clamp(0.0, 1.0);
    d * (1.0 + intensity * n * 0.6 * (1.0 - (2.0 * l - 1.0).powi(2)).max(0.15))
}

pub fn linear_to_srgb(c: f32) -> f32 {
    if c < 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// UE local exposure (PostProcessTonemap / CalculateLogLocalExposure with zero thresholds): per pixel
/// exp2((base - mid) * (contrast - 1) + (L - base) * (detail - 1)), L = log2 of the exposed luminance (uniform RGB
/// weights), base = lerp(bilateral grid of L, wide Gaussian blur of L, blend), mid = log2(0.18 * compensation).
pub fn local_exposure(exposed: &Image, p: &PostPreset, compensation: f32) -> Option<Vec<f32>> {
    let le = &p.local_exposure;
    if le.highlight_contrast == 1.0 && le.shadow_contrast == 1.0 && le.detail_strength == 1.0 {
        return None;
    }
    let (w, h) = (exposed.w, exposed.h);
    let lum: Vec<f32> = exposed.px.par_iter().map(|c| ((c.x + c.y + c.z) / 3.0).max(1e-6).log2()).collect();
    // Bilateral grid: 32 px cells x 16 log-luminance slices, splat, 3x3x3 blur, trilinear slice.
    const CELL: usize = 32;
    const Z: usize = 16;
    let (gw, gh) = (w.div_ceil(CELL) + 1, h.div_ceil(CELL) + 1);
    let (lmin, lmax) = (-16.0f32, 8.0f32);
    let zpos = |l: f32| ((l - lmin) / (lmax - lmin)).clamp(0.0, 1.0) * (Z - 1) as f32;
    let mut grid = vec![[0f32; 2]; gw * gh * Z];
    for y in 0..h {
        for x in 0..w {
            let l = lum[y * w + x];
            let (gx, gy, gz) = ((x as f32 / CELL as f32).round() as usize, (y as f32 / CELL as f32).round() as usize, zpos(l).round() as usize);
            let c = &mut grid[(gz * gh + gy) * gw + gx];
            c[0] += l;
            c[1] += 1.0;
        }
    }
    let mut blurred_grid = grid.clone();
    for axis in 0..3 {
        let src = blurred_grid.clone();
        for z in 0..Z {
            for y in 0..gh {
                for x in 0..gw {
                    let mut acc = [0f32; 2];
                    for (d, wgt) in [(-1i64, 0.25f32), (0, 0.5), (1, 0.25)] {
                        let (mut xx, mut yy, mut zz) = (x as i64, y as i64, z as i64);
                        match axis {
                            0 => xx += d,
                            1 => yy += d,
                            _ => zz += d,
                        }
                        if xx < 0 || yy < 0 || zz < 0 || xx >= gw as i64 || yy >= gh as i64 || zz >= Z as i64 {
                            continue;
                        }
                        let s = src[(zz as usize * gh + yy as usize) * gw + xx as usize];
                        acc[0] += s[0] * wgt;
                        acc[1] += s[1] * wgt;
                    }
                    blurred_grid[(z * gh + y) * gw + x] = acc;
                }
            }
        }
    }
    // Wide blur of L (separable Gaussian at 1/8 resolution, sigma = kernel% of the width / 4).
    let (bw, bh) = (w.div_ceil(8), h.div_ceil(8));
    let mut small = vec![0f32; bw * bh];
    for y in 0..bh {
        for x in 0..bw {
            let (mut s, mut n) = (0f32, 0f32);
            for yy in y * 8..((y + 1) * 8).min(h) {
                for xx in x * 8..((x + 1) * 8).min(w) {
                    s += lum[yy * w + xx];
                    n += 1.0;
                }
            }
            small[y * bw + x] = s / n.max(1.0);
        }
    }
    let sigma = (le.blurred_luminance_kernel / 100.0 * bw as f32 / 4.0).max(0.5);
    let r = (sigma * 2.5).ceil() as i64;
    let k: Vec<f32> = (-r..=r).map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp()).collect();
    let blur1 = |src: &[f32], horizontal: bool| -> Vec<f32> {
        (0..bw * bh)
            .into_par_iter()
            .map(|i| {
                let (x, y) = ((i % bw) as i64, (i / bw) as i64);
                let (mut s, mut ws) = (0f32, 0f32);
                for (j, wgt) in k.iter().enumerate() {
                    let o = j as i64 - r;
                    let (xx, yy) = if horizontal { ((x + o).clamp(0, bw as i64 - 1), y) } else { (x, (y + o).clamp(0, bh as i64 - 1)) };
                    s += src[yy as usize * bw + xx as usize] * wgt;
                    ws += wgt;
                }
                s / ws
            })
            .collect()
    };
    let wide = blur1(&blur1(&small, true), false);
    let mid = (0.18 * compensation).log2();
    Some(
        (0..w * h)
            .into_par_iter()
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let l = lum[i];
                // trilinear slice of the bilateral grid at (x, y, l)
                let (fx, fy, fz) = (x as f32 / CELL as f32, y as f32 / CELL as f32, zpos(l));
                let (x0, y0, z0) = (fx.floor() as usize, fy.floor() as usize, fz.floor() as usize);
                let (tx, ty, tz) = (fx - x0 as f32, fy - y0 as f32, fz - z0 as f32);
                let mut acc = [0f32; 2];
                for c in 0..8 {
                    let (dx, dy, dz) = (c & 1, (c >> 1) & 1, (c >> 2) & 1);
                    let (xx, yy, zz) = ((x0 + dx).min(gw - 1), (y0 + dy).min(gh - 1), (z0 + dz).min(Z - 1));
                    let wgt = (if dx == 1 { tx } else { 1.0 - tx }) * (if dy == 1 { ty } else { 1.0 - ty }) * (if dz == 1 { tz } else { 1.0 - tz });
                    let g = blurred_grid[(zz * gh + yy) * gw + xx];
                    acc[0] += g[0] * wgt;
                    acc[1] += g[1] * wgt;
                }
                let bilateral = if acc[1] > 1e-4 { acc[0] / acc[1] } else { l };
                let blurred = wide[(y / 8).min(bh - 1) * bw + (x / 8).min(bw - 1)];
                let base = bilateral + (blurred - bilateral) * le.blurred_luminance_blend;
                let contrast = if base > mid { le.highlight_contrast } else { le.shadow_contrast };
                ((base - mid) * (contrast - 1.0) + (l - base) * (le.detail_strength - 1.0)).exp2()
            })
            .collect(),
    )
}

/// Full chain: exposure, bloom, vignette, tonemap, sRGB encode -> RGBA8. `compensation` = the exposure
/// compensation (2^bias) that sets local exposure's middle grey.
pub fn finish(img: &Image, exposure: f32, compensation: f32, p: &PostPreset) -> Vec<u8> {
    let exposed = Image { w: img.w, h: img.h, px: img.px.iter().map(|c| *c * exposure).collect() };
    let bloom = bloom(&exposed, p);
    let local = local_exposure(&exposed, p, compensation);
    let tm = Tonemapper::new(p);
    let (w, h) = (img.w, img.h);
    let aspect = h as f32 / w as f32;
    let vscale = 2f32.sqrt() / (1.0 + aspect * aspect).sqrt();
    let out: Vec<[u8; 4]> = (0..w * h)
        .into_par_iter()
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let (u, v) = ((x as f32 + 0.5) / w as f32, (y as f32 + 0.5) / h as f32);
            let mut c = exposed.px[i];
            if let Some(le) = &local {
                c *= le[i];
            }
            if p.chromatic_aberration > 0.0 {
                // UE scene fringe: red / blue sampled radially out / in.
                let k = p.chromatic_aberration * 0.0025;
                let (du, dv) = (u - 0.5, v - 0.5);
                c.x = exposed.sample(0.5 + du * (1.0 + k), 0.5 + dv * (1.0 + k)).x;
                c.z = exposed.sample(0.5 + du * (1.0 - k), 0.5 + dv * (1.0 - k)).z;
            }
            if let Some(b) = &bloom {
                c += b.sample(u, v);
            }
            if p.vignette > 0.0 {
                let pos = glam::Vec2::new(u * 2.0 - 1.0, 1.0 - v * 2.0) * glam::Vec2::new(1.0, aspect) * vscale * p.vignette;
                let t2 = pos.length_squared();
                c *= (1.0 / (t2 + 1.0)).powi(2);
            }
            let d = grain(tm.apply(c), p.film_grain, i as u32 ^ 0x9e37_79b9);
            // UE adds +-0.5 LSB grain quantization noise; use ordered-free rounding instead.
            let q = |v: f32| (linear_to_srgb(v.clamp(0.0, 1.0)) * 255.0 + 0.5) as u8;
            [q(d.x), q(d.y), q(d.z), 255]
        })
        .collect();
    out.into_iter().flatten().collect()
}
