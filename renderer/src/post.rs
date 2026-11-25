//! The post chain of the offline renderer and the reference for the live passes (shaders/live.wgsl): exposure
//! (manual or automatic), local exposure, bloom, vignette, chromatic aberration, colour grading and the tone curve,
//! film grain, display encoding. Input: scene-linear radiance in nits (Rec.709 primaries, D65); output: 8-bit sRGB.
//!
//! What is here, and where each piece comes from (docs/RUNTIME.md "The post chain"; licences in
//! THIRD_PARTY_NOTICES.md):
//! - `exposure_scale`: histogram metering of log2 luminance (own design after the settings' public description).
//! - `Tonemapper`: the Academy's reference rendering as a structure (ACES 1.x RRT + ODT, AMPAS licence: working space
//!   AP1, the "glow" and "red modifier", the 0.96 / 0.93 desaturations, the blue-light-artifact-fix matrix) around a
//!   tone curve and a gamut expansion of our own whose free parameters are fitted to measurements of the look the
//!   presets ask for.
//! - bloom: a sum of Gaussian blurs of the picture, one a stage (`bloom_stages`, `gaussian`). The live passes
//!   approximate the same Gaussians on a 1/4 .. 1/128 image pyramid (Burt & Adelson 1983): `BloomPlan` is what they
//!   do for a preset and an output size.
//! - local exposure: base / detail decomposition of log luminance (Durand & Dorsey 2002).
//! - vignette (cos^4 law), chromatic aberration, film grain: own designs after the settings' public descriptions.
//!
//! The live passes (shaders/live.wgsl `lum_down` .. `post`) mirror `finish` step by step. They differ in two places:
//! bloom, as said, and colour grading with the tone curve, which they read from a 3D table built here (`lut_coord`,
//! live/targets.rs `tonemap_lut`). tools/post-live.ts measures the two against each other.

use crate::desc::{Exposure, PostPreset};
use glam::{DMat3, DVec3, Mat3, Vec3};
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
    /// Bilinear, x / y in pixels (pixel centres at integers).
    fn bilinear(&self, x: f32, y: f32) -> Vec3 {
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let (x0, y0) = (x0 as isize, y0 as isize);
        let a = self.at(x0, y0).lerp(self.at(x0 + 1, y0), fx);
        let b = self.at(x0, y0 + 1).lerp(self.at(x0 + 1, y0 + 1), fx);
        a.lerp(b, fy)
    }
}

/// Relative luminance of linear Rec.709 / sRGB (ITU-R BT.709-6, the Y row of its RGB -> XYZ matrix).
pub const LUMA: Vec3 = Vec3::new(0.2126, 0.7152, 0.0722);

/// Middle grey: the 18 % reflectance a metered scene average is exposed to.
pub const MIDDLE_GREY: f32 = 0.18;

// ------------------------------------------------------------------------------------------------ exposure

/// Bins of the metering histogram (the same number as the documented histogram of Unreal's auto exposure).
pub const AE_BINS: usize = 64;

/// The metered average luminance (nits) from a histogram of log2 luminance: `count[b]` samples in bin b of
/// `AE_BINS` equal bins over `log_min .. log_max`, `mean[b]` their mean position in 0 .. 1 of that range. The average
/// is taken over the samples between the `low` and `high` fractions of the sorted population (bins cut by a bound
/// count in part), each bin at the luminance of its mean. None: no sample in range.
///
/// Own design, after the public description of histogram metering ("the average between the low and the high
/// percentile is exposed as middle grey"): the percentile window drops what a few very bright or very dark pixels
/// would do to a plain mean. The live `exposure` pass does the same sum on the GPU.
pub fn metered_average(count: &[f32; AE_BINS], mean: &[f32; AE_BINS], low: f32, high: f32, log_min: f32, log_max: f32) -> Option<f32> {
    let total: f32 = count.iter().sum();
    if total <= 0.0 {
        return None;
    }
    let (lo, hi) = (low.clamp(0.0, 1.0) * total, high.clamp(0.0, 1.0).max(low.clamp(0.0, 1.0)) * total);
    let (mut seen, mut sum, mut n) = (0.0f32, 0.0f32, 0.0f32);
    for b in 0..AE_BINS {
        let part = (seen + count[b]).min(hi) - seen.max(lo);
        if part > 0.0 {
            sum += part * 2f32.powf(log_min + mean[b] * (log_max - log_min));
            n += part;
        }
        seen += count[b];
    }
    // (A window narrower than one sample: the sample at the low bound.)
    if n <= 0.0 {
        let mut seen = 0.0;
        for b in 0..AE_BINS {
            seen += count[b];
            if seen >= lo && count[b] > 0.0 {
                return Some(2f32.powf(log_min + mean[b] * (log_max - log_min)));
            }
        }
        return None;
    }
    Some(sum / n)
}

/// The exposure auto exposure aims at for a metered average luminance: middle grey x 2^compensation over the average,
/// the average held within the luminances that EV100 `min_ev100` .. `max_ev100` expose as middle grey.
pub fn auto_target(average: f32, compensation: f32, min_ev100: f32, max_ev100: f32) -> f32 {
    let (lo, hi) = (MIDDLE_GREY * 2f32.powf(min_ev100), MIDDLE_GREY * 2f32.powf(max_ev100.max(min_ev100)));
    MIDDLE_GREY * 2f32.powf(compensation) / average.clamp(lo, hi)
}

/// The log2 luminance of each 4 x 4 block of the image (its mean colour's; the last column / row of a size that is
/// no multiple of 4 holds fewer pixels): what auto exposure meters and local exposure decomposes. `-inf` for black.
fn quarter_log_luminance(img: &Image) -> (usize, usize, Vec<f32>) {
    let (qw, qh) = (img.w.div_ceil(4), img.h.div_ceil(4));
    let l = (0..qw * qh)
        .into_par_iter()
        .map(|i| {
            let (bx, by) = (i % qw * 4, i / qw * 4);
            let (mut s, mut n) = (0.0f32, 0.0f32);
            for y in by..(by + 4).min(img.h) {
                for x in bx..(bx + 4).min(img.w) {
                    let c = img.px[y * img.w + x];
                    if c.is_finite() {
                        s += c.max(Vec3::ZERO).dot(LUMA);
                    }
                    n += 1.0;
                }
            }
            (s / n.max(1.0)).log2()
        })
        .collect();
    (qw, qh, l)
}

/// Exposure scale applied to scene radiance (nits), and the EV100 it corresponds to (-log2 of the scale: at EV100 0
/// one nit is 1.0, the scene DSL's convention). Manual: 2^-EV100. Auto: `metered_average` of the image's
/// quarter-resolution luminance within the preset's histogram range (samples outside it are not counted, as the
/// setting's documentation says), `auto_target` of that. An image with nothing in range is shown at the exposure a
/// view starts with.
pub fn exposure_scale(img: &Image, exp: &Exposure, p: &PostPreset) -> (f32, f32) {
    let scale = match exp {
        Exposure::Manual { ev100 } => 2f32.powf(-ev100),
        Exposure::Auto { compensation, min_ev100, max_ev100 } => {
            let ae = &p.auto_exposure;
            let (lo, hi) = (ae.histogram_log_min, ae.histogram_log_max.max(ae.histogram_log_min + 1.0));
            let (mut count, mut sum) = ([0f32; AE_BINS], [0f64; AE_BINS]);
            for l in quarter_log_luminance(img).2 {
                let t = (l - lo) / (hi - lo);
                if (0.0..=1.0).contains(&t) {
                    let b = ((t * AE_BINS as f32) as usize).min(AE_BINS - 1);
                    count[b] += 1.0;
                    sum[b] += t as f64;
                }
            }
            let mut mean = [0f32; AE_BINS];
            for b in 0..AE_BINS {
                mean[b] = if count[b] > 0.0 { (sum[b] / count[b] as f64) as f32 } else { 0.0 };
            }
            match metered_average(&count, &mean, ae.low_percent / 100.0, ae.high_percent / 100.0, lo, hi) {
                Some(avg) => auto_target(avg, *compensation, *min_ev100, *max_ev100),
                None => exp.start(),
            }
        }
    };
    (scale, -scale.log2())
}

// ------------------------------------------------------------------------------------------------ colour science

/// CIE 1931 xy chromaticities. Rec.709 / sRGB primaries and D65: ITU-R BT.709-6, IEC 61966-2-1. AP0, AP1 and the ACES
/// white: SMPTE ST 2065-1 and Academy S-2014-004 (ACEScg), as tabulated in the Academy's reference transforms
/// (aces-dev 1.3, `ACESlib.Utilities_Color.ctl`).
const REC709: [[f64; 2]; 3] = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]];
const D65: [f64; 2] = [0.3127, 0.3290];
const AP0: [[f64; 2]; 3] = [[0.7347, 0.2653], [0.0, 1.0], [0.0001, -0.077]];
const AP1: [[f64; 2]; 3] = [[0.713, 0.293], [0.165, 0.830], [0.128, 0.044]];
const ACES_WHITE: [f64; 2] = [0.32168, 0.33767];

/// The wide gamut an sRGB colour's coordinates are re-read in by the gamut expansion (`Tonemapper`): primaries
/// fitted to the measured look (white D65), between DCI-P3 and Rec.2020. Own design.
const EXPAND_GAMUT: [[f64; 2]; 3] = [[0.7035, 0.3032], [0.2396, 0.7227], [0.1325, 0.0504]];

/// The Bradford cone response matrix (Lam 1985; the chromatic adaptation the ACES transforms and ICC profiles use).
const BRADFORD: [[f64; 3]; 3] = [[0.8951, 0.2664, -0.1614], [-0.7502, 1.7135, 0.0367], [0.0389, -0.0685, 1.0296]];

/// The Academy's "blue light artifact fix" look (aces-dev 1.3, `LMT.Academy.BlueLightArtifactFix.ctl`, AMPAS
/// licence: THIRD_PARTY_NOTICES.md), an AP0 -> AP0 matrix that desaturates blue hues so that bright blue light does
/// not clip to violet. Written here for column vectors (the CTL file multiplies row vectors).
const BLUE_LIGHT_FIX_AP0: [[f64; 3]; 3] = [
    [0.9404372683, -0.0183068787, 0.0778696104],
    [0.0083786969, 0.8286599939, 0.1629613092],
    [0.0005471261, -0.0008833746, 1.0003362486],
];

fn rows(m: [[f64; 3]; 3]) -> DMat3 {
    DMat3::from_cols_array_2d(&m).transpose()
}

fn xyz_of(xy: [f64; 2]) -> DVec3 {
    DVec3::new(xy[0] / xy[1], 1.0, (1.0 - xy[0] - xy[1]) / xy[1])
}

/// RGB -> CIE XYZ of a colour space given by its primaries and white (the standard derivation: SMPTE RP 177).
fn rgb_to_xyz(primaries: [[f64; 2]; 3], white: [f64; 2]) -> DMat3 {
    let m = DMat3::from_cols(xyz_of(primaries[0]), xyz_of(primaries[1]), xyz_of(primaries[2]));
    let s = m.inverse() * xyz_of(white);
    DMat3::from_cols(m.x_axis * s.x, m.y_axis * s.y, m.z_axis * s.z)
}

/// The von Kries adaptation in Bradford's cone space that takes the white `from` to the white `to` (XYZ -> XYZ).
fn adapt(from: [f64; 2], to: [f64; 2]) -> DMat3 {
    let b = rows(BRADFORD);
    b.inverse() * DMat3::from_diagonal(b * xyz_of(to) / (b * xyz_of(from))) * b
}

/// The matrix that keeps `sat` of a colour and takes the rest from its luminance (weights `y`), as the ACES
/// transforms build their desaturations (`calc_sat_adjust_matrix`).
fn saturation_matrix(sat: f64, y: DVec3) -> DMat3 {
    let grey = DMat3::from_cols(DVec3::splat(y.x), DVec3::splat(y.y), DVec3::splat(y.z));
    grey * (1.0 - sat) + DMat3::IDENTITY * sat
}

fn to_f32(m: DMat3) -> Mat3 {
    m.as_mat3()
}

/// CIE daylight locus: the chromaticity of daylight of correlated colour temperature `t` K (CIE 15 "Colorimetry";
/// Wyszecki & Stiles 1982, section 3.3.4; given for 4000 .. 25000 K), in CIE 1960 uv.
fn daylight_uv(t: f64) -> [f64; 2] {
    let x = if t <= 7000.0 { 0.244063 + 0.09911e3 / t + 2.9678e6 / (t * t) - 4.6070e9 / (t * t * t) } else { 0.237040 + 0.24748e3 / t + 1.9018e6 / (t * t) - 2.0064e9 / (t * t * t) };
    let y = -3.0 * x * x + 2.87 * x - 0.275;
    [4.0 * x / (-2.0 * x + 12.0 * y + 3.0), 6.0 * y / (-2.0 * x + 12.0 * y + 3.0)]
}

/// The Planckian locus in CIE 1960 uv (Krystek 1985, "An algorithm to calculate correlated colour temperature",
/// Color Research & Application 10: rational approximation, 1000 .. 15000 K).
fn planckian_uv(t: f64) -> [f64; 2] {
    [
        (0.860117757 + 1.54118254e-4 * t + 1.28641212e-7 * t * t) / (1.0 + 8.42420235e-4 * t + 7.08145163e-7 * t * t),
        (0.317398726 + 4.22806245e-5 * t + 4.20481691e-8 * t * t) / (1.0 - 2.89741816e-5 * t + 1.61456053e-7 * t * t),
    ]
}

/// CIE 1960 u shift of the white a unit of `whiteTint`: fitted to the measured tint tables (own design: positive tint
/// takes the scene illuminant toward green, so the picture toward magenta).
const TINT_U: f64 = -0.0195;

/// The scene illuminant `whiteTemp` / `whiteTint` name, xy: on the daylight locus from 4000 K up, on the Planckian
/// locus below 3000 K (a lamp), between the two in between; the tint shifts it across the locus.
fn white_point(temp: f64, tint: f64) -> [f64; 2] {
    let t = temp.clamp(1500.0, 25000.0);
    let d = daylight_uv(t.max(4000.0));
    let [u, v] = if t < 4000.0 {
        // Below the daylight locus' range: the Planckian locus, keeping at 4000 K the daylight's offset from it and
        // losing it by 3000 K (so the white is continuous in the temperature).
        let (p, p4) = (planckian_uv(t), planckian_uv(4000.0));
        let f = ((t - 3000.0) / 1000.0).clamp(0.0, 1.0);
        [p[0] + f * (d[0] - p4[0]), p[1] + f * (d[1] - p4[1])]
    } else {
        d
    };
    let u = u + TINT_U * tint;
    [3.0 * u / (2.0 * u - 8.0 * v + 4.0), 2.0 * v / (2.0 * u - 8.0 * v + 4.0)]
}

/// White balance as a linear sRGB -> linear sRGB matrix: the Bradford adaptation from the illuminant the settings
/// name to the one 6500 K names, so that light of `temp` K renders white (same meaning as Unreal's "Temp" / "Tint"
/// white balance settings, per its documentation). 6500 / 0: the identity.
fn white_balance(temp: f32, tint: f32) -> DMat3 {
    if temp == 6500.0 && tint == 0.0 {
        return DMat3::IDENTITY;
    }
    let srgb = rgb_to_xyz(REC709, D65);
    srgb.inverse() * adapt(white_point(temp as f64, tint as f64), white_point(6500.0, 0.0)) * srgb
}

// ------------------------------------------------------------------------------------------------ tone curve

/// The tone curve: display-linear output for scene-linear (exposed) input, per channel of the rendering space.
///
/// Own design: a film characteristic curve drawn, as sensitometry draws it, against log10 exposure, made of two
/// logistic branches that meet with equal value and slope at `(mid_in, mid_out)`. The lower branch (the toe) rises
/// from 0 toward `toe`, the upper (the shoulder) from `white - shoulder` toward `white`; a logistic in log exposure
/// is a power law at its far end, so shadows fall to black as x^(toe exponent) and highlights close on `white`
/// without ever clipping. The six numbers are fitted to the measured neutral ramp of the look the presets ask for
/// (257 greys from 1e-4 to 1e3: largest difference 0.0004 in sRGB-encoded value).
#[derive(Clone, Copy, Debug)]
pub struct ToneCurve {
    /// Exposure at which the branches meet, and the output there.
    pub mid_in: f32,
    pub mid_out: f32,
    /// Slope at the meeting point, output per decade of exposure (the film's "gamma" in these axes).
    pub slope: f32,
    /// Height the toe branch would reach, and depth the shoulder branch rises from below `white`.
    pub toe: f32,
    pub shoulder: f32,
    /// The curve's top: display values above 1 are clipped by the display, which sets how soon highlights clip.
    pub white: f32,
}

impl Default for ToneCurve {
    fn default() -> Self {
        ToneCurve { mid_in: 0.345_476, mid_out: 0.386_434, slope: 0.855_193, toe: 0.889_282, shoulder: 1.553_913, white: 1.039_985 }
    }
}

/// `ToneCurve` in the form `eval` computes: the two logistics' rates and centres in log10 exposure.
#[derive(Clone, Copy, Debug)]
struct CurveEval {
    mid: f32,
    toe: [f32; 3],
    shoulder: [f32; 3],
    white: f32,
}

impl ToneCurve {
    fn eval(&self) -> CurveEval {
        let mid = self.mid_in.log10();
        // A logistic A / (1 + exp(-k (X - c))) through (mid, y) with slope s there: u = y / A, k = s / (A u (1 - u)),
        // c = mid - ln(u / (1 - u)) / k.
        let branch = |a: f32, y: f32| {
            let u = (y / a).clamp(1e-4, 1.0 - 1e-4);
            let k = self.slope / (a * u * (1.0 - u));
            [a, k, mid - (u / (1.0 - u)).ln() / k]
        };
        let toe = branch(self.toe, self.mid_out);
        // The shoulder is the same logistic turned over: white - A / (1 + exp(k (X - c))).
        let s = branch(self.shoulder, self.white - self.mid_out);
        CurveEval { mid, toe, shoulder: [s[0], s[1], 2.0 * mid - s[2]], white: self.white }
    }
}

impl CurveEval {
    fn at(&self, x: f32) -> f32 {
        let lx = x.max(1e-10).log10();
        if lx < self.mid {
            let [a, k, c] = self.toe;
            a / (1.0 + (-k * (lx - c)).exp())
        } else {
            let [a, k, c] = self.shoulder;
            self.white - a / (1.0 + (k * (lx - c)).exp())
        }
    }
}

/// Constants of the Academy's reference rendering transform that the rendering here keeps (aces-dev 1.3,
/// `ACESlib.RRT_Common.ctl`, `ACESlib.ODT_Common.ctl`; AMPAS licence: THIRD_PARTY_NOTICES.md).
const RRT_GLOW_GAIN: f32 = 0.05;
const RRT_GLOW_MID: f32 = 0.08;
const RRT_RED_SCALE: f32 = 0.82;
const RRT_RED_PIVOT: f32 = 0.03;
const RRT_RED_WIDTH: f32 = 135.0;
const RRT_SAT_FACTOR: f64 = 0.96;
const ODT_SAT_FACTOR: f64 = 0.93;

/// The gamut expansion's weight: the luminance (rendering space, exposed) at which it is 1 - 1/e of the way in, and
/// the same for the chroma (the colour's distance from its grey, over its luminance). Fitted with `EXPAND_GAMUT`.
const EXPAND_LUMINANCE: f32 = 0.5867;
const EXPAND_CHROMA: f32 = 0.6134;

/// Colour grading's tonal ranges, as luminance (rendering space, exposed): shadows fade out by `SHADOWS_MAX`,
/// highlights fade in from `HIGHLIGHTS_MIN` to `HIGHLIGHTS_MAX`. Fitted to the measured gain tables (0.0905 / 0.492 /
/// 1.011) and rounded.
const SHADOWS_MAX: f32 = 0.09;
const HIGHLIGHTS_MIN: f32 = 0.5;
const HIGHLIGHTS_MAX: f32 = 1.0;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Colour grading and the tone curve of a preset: exposed scene-linear sRGB in, display-linear sRGB out.
///
/// The rendering is the Academy's reference rendering (ACES 1.x RRT then ODT for an sRGB display; the Filmic
/// tonemapper the presets are compatible with "matches the ACES standard", its documentation says) with our own
/// tone curve in place of the Academy's two tone scales and two looks in front of it:
///
/// 1. white balance (`white_balance`), in linear sRGB;
/// 2. to the rendering space AP1 (ACEScg), Bradford-adapted from D65 to the ACES white;
/// 3. colour grading on scene-linear AP1: saturation about the luminance, contrast as a power about middle grey,
///    gains weighted to shadows / highlights by luminance (the settings' documented meanings);
/// 4. the RRT's "glow" (dark saturated colours a little brighter) and "red modifier" (saturated reds toward a pivot),
///    in AP0, with the Academy's constants;
/// 5. gamut expansion (`colorGrading.expandGamut`; own design after the setting's documented purpose, "pushes bright,
///    saturated colours beyond the sRGB gamut"): the colour's sRGB coordinates are re-read as coordinates of the
///    wider `EXPAND_GAMUT`, by a weight that grows with luminance and with chroma;
/// 6. the Academy's blue-light-artifact-fix matrix by `colorGrading.blueCorrection`, undone after the curve, so that
///    only what the curve compresses (bright blue) changes: it desaturates instead of turning violet;
/// 7. negative components clipped, the RRT's 0.96 desaturation, the tone curve per channel, the ODT's 0.93
///    desaturation, back to linear sRGB (adapted to D65).
///
/// Against the measured table of the previous look (6 657 colours): mean CIE dE2000 0.19, 99th percentile 1.1.
pub struct Tonemapper {
    wb_to_ap1: Mat3,
    ap1_to_ap0: Mat3,
    ap0_to_ap1: Mat3,
    /// The gamut expansion in AP1, less the identity.
    expand: Mat3,
    expand_amount: f32,
    /// Blue fix and the RRT desaturation; the ODT desaturation, the blue fix undone and AP1 -> sRGB.
    pre: Mat3,
    blue: Mat3,
    post: Mat3,
    luma: Vec3,
    curve: CurveEval,
    saturation: Vec3,
    contrast: Vec3,
    shadows_gain: Vec3,
    highlights_gain: Vec3,
    graded: bool,
}

impl Tonemapper {
    pub fn new(p: &PostPreset) -> Self {
        let g = &p.color_grading;
        let srgb = rgb_to_xyz(REC709, D65);
        let (ap0, ap1) = (rgb_to_xyz(AP0, ACES_WHITE), rgb_to_xyz(AP1, ACES_WHITE));
        let srgb_to_ap1 = ap1.inverse() * adapt(D65, ACES_WHITE) * srgb;
        let ap1_to_srgb = srgb.inverse() * adapt(ACES_WHITE, D65) * ap1;
        let (ap1_to_ap0, ap0_to_ap1) = (ap0.inverse() * ap1, ap1.inverse() * ap0);
        // AP1's luminance weights: the Y row of AP1 -> XYZ.
        let y = ap1.row(1);
        // sRGB coordinates read as the wide gamut's, as an sRGB -> sRGB matrix, then in AP1.
        let widen = srgb.inverse() * rgb_to_xyz(EXPAND_GAMUT, D65);
        let expand = srgb_to_ap1 * widen * ap1_to_srgb - DMat3::IDENTITY;
        let t = g.blue_correction.clamp(0.0, 1.0) as f64;
        let blue = ap0_to_ap1 * (DMat3::IDENTITY * (1.0 - t) + rows(BLUE_LIGHT_FIX_AP0) * t) * ap1_to_ap0;
        let v = |c: crate::desc::Rgb| Vec3::from(c.0);
        let one = Vec3::ONE;
        Tonemapper {
            wb_to_ap1: to_f32(srgb_to_ap1 * white_balance(g.white_temp, g.white_tint)),
            ap1_to_ap0: to_f32(ap1_to_ap0),
            ap0_to_ap1: to_f32(ap0_to_ap1),
            expand: to_f32(expand),
            expand_amount: g.expand_gamut.max(0.0),
            blue: to_f32(blue),
            pre: to_f32(saturation_matrix(RRT_SAT_FACTOR, y)),
            post: to_f32(ap1_to_srgb * blue.inverse() * saturation_matrix(ODT_SAT_FACTOR, y)),
            luma: y.as_vec3(),
            curve: ToneCurve::default().eval(),
            saturation: v(g.saturation),
            contrast: v(g.contrast),
            shadows_gain: v(g.shadows_gain),
            highlights_gain: v(g.highlights_gain),
            graded: v(g.saturation) != one || v(g.contrast) != one || v(g.shadows_gain) != one || v(g.highlights_gain) != one,
        }
    }

    /// Step 3: saturation, contrast, shadow / highlight gains (1 = none each; scalar or per channel).
    fn grade(&self, a: Vec3) -> Vec3 {
        let l = a.dot(self.luma);
        let a = (Vec3::splat(l) + (a - l) * self.saturation).max(Vec3::ZERO);
        let a = (a / MIDDLE_GREY).powf_each(self.contrast) * MIDDLE_GREY;
        let (shadows, highlights) = (1.0 - smoothstep(0.0, SHADOWS_MAX, l), smoothstep(HIGHLIGHTS_MIN, HIGHLIGHTS_MAX, l));
        a * (Vec3::ONE + (self.shadows_gain - 1.0) * shadows + (self.highlights_gain - 1.0) * highlights)
    }

    /// Step 4: the RRT's glow and red modifier (aces-dev 1.3, `RRT.ctl` and `ACESlib.RRT_Common.ctl`: `glow_fwd`,
    /// `sigmoid_shaper`, `rgb_2_saturation`, `rgb_2_yc`, `rgb_2_hue`, `cubic_basis_shaper`), AP1 in and out.
    fn sweeten(&self, a: Vec3) -> Vec3 {
        let mut aces = self.ap1_to_ap0 * a;
        let (hi, lo) = (aces.max_element(), aces.min_element());
        let saturation = (hi.max(1e-10) - lo.max(1e-10)) / hi.max(1e-2);
        // Glow: a gain on dark colours (below `RRT_GLOW_MID` of the luminance proxy YC), by their saturation.
        let chroma = (aces.z * (aces.z - aces.y) + aces.y * (aces.y - aces.x) + aces.x * (aces.x - aces.z)).max(0.0).sqrt();
        let yc = (aces.x + aces.y + aces.z + 1.75 * chroma) / 3.0;
        let x = (saturation - 0.4) / 0.2;
        let t = (1.0 - (x / 2.0).abs()).max(0.0);
        let shaped = (1.0 + x.signum() * (1.0 - t * t)) / 2.0;
        let gain = RRT_GLOW_GAIN * shaped;
        let glow = if yc <= 2.0 / 3.0 * RRT_GLOW_MID {
            gain
        } else if yc >= 2.0 * RRT_GLOW_MID {
            0.0
        } else {
            gain * (RRT_GLOW_MID / yc - 0.5)
        };
        aces *= 1.0 + glow;
        // Red modifier: red hues (a cubic B-spline window `RRT_RED_WIDTH` degrees wide about hue 0) move toward
        // `RRT_RED_PIVOT`, by their saturation.
        if aces.x != aces.y || aces.y != aces.z {
            let hue = (3f32.sqrt() * (aces.y - aces.z)).atan2(2.0 * aces.x - aces.y - aces.z).to_degrees();
            let k = (hue + RRT_RED_WIDTH / 2.0) * 4.0 / RRT_RED_WIDTH;
            if k > 0.0 && k < 4.0 {
                let (j, t) = (k.floor(), k - k.floor());
                let window = match j as i32 {
                    0 => t * t * t / 6.0,
                    1 => (-3.0 * t * t * t + 3.0 * t * t + 3.0 * t + 1.0) / 6.0,
                    2 => (3.0 * t * t * t - 6.0 * t * t + 4.0) / 6.0,
                    _ => (1.0 - t) * (1.0 - t) * (1.0 - t) / 6.0,
                } * 1.5;
                aces.x += window * saturation * (RRT_RED_PIVOT - aces.x) * (1.0 - RRT_RED_SCALE);
            }
        }
        (self.ap0_to_ap1 * aces.max(Vec3::ZERO)).max(Vec3::ZERO)
    }

    /// Exposed linear sRGB scene colour -> display-referred linear sRGB (before the sRGB encoding; not clipped at 1:
    /// the curve's top is `ToneCurve::white`).
    pub fn apply(&self, linear: Vec3) -> Vec3 {
        let c = if linear.is_finite() { linear.max(Vec3::ZERO) } else { Vec3::ZERO };
        let mut a = self.wb_to_ap1 * c;
        if self.graded {
            a = self.grade(a.max(Vec3::ZERO));
        }
        a = self.sweeten(a);
        // Gamut expansion: by (1 - exp(-(L / L0)^2)) (1 - exp(-(d / d0)^2)), L the luminance and d the colour's
        // distance from its grey over L.
        let l = a.dot(self.luma).max(1e-9);
        let d = (a - l).length() / l;
        let w = self.expand_amount * (1.0 - (-(l / EXPAND_LUMINANCE).powi(2)).exp()) * (1.0 - (-(d / EXPAND_CHROMA).powi(2)).exp());
        a += self.expand * a * w;
        // (Negatives are clipped before the desaturation, as the RRT does: a colour outside the rendering gamut keeps
        // the grey the desaturation gives it.)
        let v = (self.pre * (self.blue * a).max(Vec3::ZERO)).max(Vec3::ZERO);
        let t = Vec3::new(self.curve.at(v.x), self.curve.at(v.y), self.curve.at(v.z));
        (self.post * t).max(Vec3::ZERO)
    }
}

trait PowfEach {
    fn powf_each(self, e: Vec3) -> Vec3;
}

impl PowfEach for Vec3 {
    fn powf_each(self, e: Vec3) -> Vec3 {
        Vec3::new(self.x.powf(e.x), self.y.powf(e.y), self.z.powf(e.z))
    }
}

/// The sRGB encoding (IEC 61966-2-1).
pub fn linear_to_srgb(c: f32) -> f32 {
    if c < 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// The live path's colour table (live/targets.rs `tonemap_lut`): `LUT_SIZE` texels a side, indexed by exposed linear
/// sRGB on a log2 axis, `LUT_LOG_MIN` .. `LUT_LOG_MAX` stops (0.0007 .. 90) over its texels' centres. The tone curve
/// is a function of log exposure, so equal steps of the table are equal steps of the curve (0.36 stops a texel: greys
/// come out within a third of an 8-bit step of `Tonemapper::apply`); below the range the curve is under 2e-5 (black
/// in 8 bits), above it over 1 (white).
pub const LUT_SIZE: u32 = 48;
pub const LUT_LOG_MIN: f32 = -10.5;
pub const LUT_LOG_MAX: f32 = 6.5;

/// The exposed linear value at position `t` (0 .. 1) of the table's axis.
pub fn lut_value(t: f32) -> f32 {
    2f32.powf(LUT_LOG_MIN + t * (LUT_LOG_MAX - LUT_LOG_MIN))
}

/// Position (0 .. 1) of an exposed linear value on the table's axis (the same in live.wgsl `post`).
pub fn lut_coord(x: f32) -> f32 {
    ((x.max(1e-9).log2() - LUT_LOG_MIN) / (LUT_LOG_MAX - LUT_LOG_MIN)).clamp(0.0, 1.0)
}

// ------------------------------------------------------------------------------------------------ bloom

/// Bloom's pyramid: levels of 1/4, 1/8 .. 1/128 of the output (each way).
pub const BLOOM_LEVELS: usize = 6;

/// Size of level `j` (0 = the quarter-resolution level) of an output of `w` x `h`.
pub fn bloom_dim(w: u32, h: u32, j: usize) -> (u32, u32) {
    let (mut lw, mut lh) = (w.div_ceil(4), h.div_ceil(4));
    for _ in 0..j {
        (lw, lh) = (lw.div_ceil(2), lh.div_ceil(2));
    }
    (lw.max(1), lh.max(1))
}

/// Texels of the pyramid's buffer: the levels one after another, then the coarsest eight times more (the wide
/// Gaussians' sum, and their seven row blurs: live.wgsl `bloom_wide`, `bloom_wide_rows`).
pub fn bloom_texels(w: u32, h: u32) -> u64 {
    let n = |j| bloom_dim(w, h, j).0 as u64 * bloom_dim(w, h, j).1 as u64;
    (0..BLOOM_LEVELS).map(n).sum::<u64>() + 8 * n(BLOOM_LEVELS - 1)
}

/// A bloom stage's Gaussian: its standard deviation as a fraction of the stage's size (`sizes[k]` x `sizeScale`,
/// percent of the view width), and the share of `intensity` x `tints[k]` it adds. Both fitted to the reference
/// pictures of the look the presets ask for (seven scenes, with the vignette's reach).
const BLOOM_SIGMA: f32 = 0.1;
const BLOOM_GAIN: f32 = 0.175;

/// Variance (output pixels^2, each way) of the live final pass's kernel of the output's own pixels: the pixel and
/// its four neighbours, a fifth each.
const FINE_VARIANCE: f64 = 0.4;
/// The widest kernel of the direct pass, texels of the coarsest level each way.
const WIDE_RADIUS: u32 = 24;
/// The narrowest Gaussian the direct pass sums on the coarsest level's grid (texels): narrower ones are a blend of
/// it and the level itself.
const WIDE_MIN_SIGMA: f64 = 0.6;

/// What the bloom passes do for a preset and an output size: how much of each pyramid level and of each wide
/// Gaussian the sum takes (`Pyramid`), for the colour (bloom) and for the fourth channel (the wide blur of log
/// luminance local exposure blends its base with).
///
/// A stage is a Gaussian of standard deviation s (output pixels). The pyramid's level j, brought back up to the
/// output, is the image under a kernel of known variance V_j (`level_variance`): a stage between two levels is their
/// blend with the stage's variance (the blend has the right energy and second moment); a stage wider than the
/// coarsest level is a Gaussian summed directly on that level's grid, of the variance that is missing.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BloomPlan {
    /// Whether the pyramid is needed at all (bloom or local exposure on).
    pub on: bool,
    /// Colour: weight of the five-pixel kernel of the output's own pixels (stages narrower than the first level; 0
    /// at the sizes where no stage is); weight of each level; per wide Gaussian its weight and 1 / (2 sigma^2) in
    /// texels of the coarsest level (weight 0: unused; the passes divide each kernel by its sum over `radius`).
    pub fine: f32,
    pub level: [f32; BLOOM_LEVELS],
    pub wide: [[f32; 2]; 6],
    /// The same for the fourth channel: one Gaussian, weights summing to 1.
    pub base_level: [f32; BLOOM_LEVELS],
    pub base_wide: [f32; 2],
    /// The direct pass's reach, texels of the coarsest level each way.
    pub radius: u32,
    /// Bloom threshold in exposed luminance (< 0: everything blooms).
    pub threshold: f32,
}

/// Variance (output pixels^2) of pyramid level `j` as the final picture samples it. Down: a 4 x 4 box from the output
/// (variance (16 - 1) / 12), then per level the binomial [1 3 3 1] / 8 at the finer level's spacing s (0.75 s^2).
/// Up: each bilinear interpolation is a tent of the coarser spacing (s^2 / 6), down to the output's pixels.
fn level_variance(j: usize) -> f64 {
    let mut v = 1.25;
    for m in 0..j {
        v += 0.75 * 16f64 * 4f64.powi(m as i32);
    }
    for m in 0..=j {
        v += 16f64 * 4f64.powi(m as i32) / 6.0;
    }
    v
}

/// The preset's bloom as Gaussians of an output `w` pixels wide: per stage that adds anything (up to six) its
/// weight and its standard deviation, output pixels.
pub fn bloom_stages(p: &PostPreset, w: u32) -> Vec<(f32, f32)> {
    let b = &p.bloom;
    if b.intensity <= 0.0 {
        return vec![];
    }
    (0..b.sizes.len().min(b.tints.len()).min(6))
        .map(|k| (b.intensity * BLOOM_GAIN * b.tints[k], (BLOOM_SIGMA * b.sizes[k] * b.size_scale / 100.0 * w as f32).max(0.0)))
        .filter(|s| s.0 > 0.0)
        .collect()
}

/// The standard deviation (output pixels) of local exposure's wide blur, if local exposure does anything.
pub fn local_exposure_sigma(p: &PostPreset, w: u32) -> Option<f32> {
    let lx = &p.local_exposure;
    (lx.highlight_contrast != 1.0 || lx.shadow_contrast != 1.0 || lx.detail_strength != 1.0).then(|| (BLOOM_SIGMA * lx.blurred_luminance_kernel / 100.0 * w as f32).max(0.0))
}

/// A Gaussian blur of standard deviation `sigma` pixels of an image (edge pixels repeat outward): the offline
/// chain's bloom stages and wide luminance blur, to the kernel's own shape (the live passes approximate the same
/// Gaussians on a pyramid: `BloomPlan`).
///
/// A wide Gaussian is taken on a smaller image: box-reduced by a power of two until sigma is at most 6 texels,
/// blurred there by the separable kernel (out to 3.5 sigma), and brought back bilinearly, the Gaussian narrowed by
/// the variance the box ((f^2 - 1) / 12) and the bilinear tent (f^2 / 6) add.
pub fn gaussian(src: &[Vec3], w: usize, h: usize, sigma: f32) -> Vec<Vec3> {
    if sigma <= 0.05 {
        return src.to_vec();
    }
    let mut f = 1usize;
    while sigma / f as f32 > 6.0 && w.min(h) / (f * 2) >= 4 {
        f *= 2;
    }
    let (lw, lh) = (w.div_ceil(f), h.div_ceil(f));
    let small: Vec<Vec3> = if f == 1 {
        src.to_vec()
    } else {
        (0..lw * lh)
            .into_par_iter()
            .map(|i| {
                let (bx, by) = (i % lw * f, i / lw * f);
                let (mut s, mut n) = (Vec3::ZERO, 0f32);
                for y in by..(by + f).min(h) {
                    for x in bx..(bx + f).min(w) {
                        s += src[y * w + x];
                        n += 1.0;
                    }
                }
                s / n
            })
            .collect()
    };
    let ff = (f * f) as f32;
    let var = if f == 1 { sigma * sigma } else { ((sigma * sigma - (ff - 1.0) / 12.0 - ff / 6.0) / ff).max(0.25) };
    let r = (3.5 * var.sqrt()).ceil() as isize;
    let k: Vec<f32> = (-r..=r).map(|d| (-(d * d) as f32 / (2.0 * var)).exp()).collect();
    let norm: f32 = k.iter().sum();
    let pass = |img: &[Vec3], horizontal: bool| -> Vec<Vec3> {
        (0..lw * lh)
            .into_par_iter()
            .map(|i| {
                let (x, y) = ((i % lw) as isize, (i / lw) as isize);
                let mut s = Vec3::ZERO;
                for (j, kj) in k.iter().enumerate() {
                    let d = j as isize - r;
                    let (sx, sy) = if horizontal { ((x + d).clamp(0, lw as isize - 1), y) } else { (x, (y + d).clamp(0, lh as isize - 1)) };
                    s += img[sy as usize * lw + sx as usize] * *kj;
                }
                s / norm
            })
            .collect()
    };
    let blurred = pass(&pass(&small, true), false);
    if f == 1 {
        return blurred;
    }
    let low = Image { w: lw, h: lh, px: blurred };
    (0..w * h).into_par_iter().map(|i| low.bilinear(((i % w) as f32 + 0.5) / f as f32 - 0.5, ((i / w) as f32 + 0.5) / f as f32 - 0.5)).collect()
}

impl BloomPlan {
    pub fn new(p: &PostPreset, w: u32, _h: u32) -> BloomPlan {
        let b = &p.bloom;
        let mut plan = BloomPlan { threshold: b.threshold, ..Default::default() };
        let coarsest = 4f64 * 2f64.powi(BLOOM_LEVELS as i32 - 1);
        let mut sigma_max = 0f64;
        // One Gaussian of variance `var` (output px^2) and weight `a`: into the levels, or the direct pass; narrower
        // than the first level, a blend of it and the five-pixel kernel of the output's own pixels (variance
        // FINE_VARIANCE), where there is one (`fine`: the colour's).
        let mut place = |a: f32, var: f64, fine: Option<&mut f32>, level: &mut [f32; BLOOM_LEVELS], wide: &mut [f32; 2]| {
            let top = level_variance(BLOOM_LEVELS - 1);
            if var <= level_variance(0) {
                let t = match &fine {
                    Some(_) => ((var - FINE_VARIANCE) / (level_variance(0) - FINE_VARIANCE)).clamp(0.0, 1.0) as f32,
                    None => 1.0,
                };
                level[0] += a * t;
                if let Some(f) = fine {
                    *f += a * (1.0 - t);
                }
            } else if var <= top {
                let j = (0..BLOOM_LEVELS - 1).find(|&j| var <= level_variance(j + 1)).unwrap();
                let t = ((var - level_variance(j)) / (level_variance(j + 1) - level_variance(j))) as f32;
                level[j] += a * (1.0 - t);
                level[j + 1] += a * t;
            } else {
                // Texels^2 of the coarsest level still missing.
                let need = (var - top) / (coarsest * coarsest);
                let min = WIDE_MIN_SIGMA * WIDE_MIN_SIGMA;
                let t = (need / min).min(1.0) as f32;
                let sigma = need.max(min).sqrt();
                level[BLOOM_LEVELS - 1] += a * (1.0 - t);
                *wide = [a * t, (0.5 / (sigma * sigma)) as f32];
                sigma_max = sigma_max.max(sigma);
            }
        };
        let stages = bloom_stages(p, w);
        for (k, (a, sigma)) in stages.iter().enumerate() {
            let sigma = *sigma as f64;
            place(*a, sigma * sigma, Some(&mut plan.fine), &mut plan.level, &mut plan.wide[k]);
        }
        let local = local_exposure_sigma(p, w);
        if let Some(sigma) = local {
            place(1.0, (sigma * sigma) as f64, None, &mut plan.base_level, &mut plan.base_wide);
        }
        plan.on = !stages.is_empty() || local.is_some();
        plan.radius = ((3.0 * sigma_max).ceil() as u32).clamp(1, WIDE_RADIUS);
        plan
    }

    pub fn has_wide(&self) -> bool {
        self.wide.iter().any(|g| g[0] > 0.0) || self.base_wide[0] > 0.0
    }
}

/// The pyramid on the CPU, as the live passes build it (live.wgsl `lum_down`, `bloom_down`, `bloom_wide`,
/// `bloom_up`): rgb the bloom, w the log2 luminance. The tests' model of the passes' arithmetic (`BloomPlan`'s
/// weights against the Gaussians they stand for).
#[cfg(test)]
struct Pyramid {
    dims: [(usize, usize); BLOOM_LEVELS],
    /// Level 0 after the sum: what the final pass samples.
    sum: Vec<[f32; 4]>,
}

#[cfg(test)]
fn texel(l: &[[f32; 4]], (w, h): (usize, usize), x: isize, y: isize) -> [f32; 4] {
    l[y.clamp(0, h as isize - 1) as usize * w + x.clamp(0, w as isize - 1) as usize]
}

#[cfg(test)]
fn bilinear4(l: &[[f32; 4]], dim: (usize, usize), x: f32, y: f32) -> [f32; 4] {
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let (x0, y0) = (x0 as isize, y0 as isize);
    let (a, b, c, d) = (texel(l, dim, x0, y0), texel(l, dim, x0 + 1, y0), texel(l, dim, x0, y0 + 1), texel(l, dim, x0 + 1, y0 + 1));
    std::array::from_fn(|k| (a[k] * (1.0 - fx) + b[k] * fx) * (1.0 - fy) + (c[k] * (1.0 - fx) + d[k] * fx) * fy)
}

#[cfg(test)]
impl Pyramid {
    /// From the quarter-resolution level (`lum_down`'s).
    fn new(level0: Vec<[f32; 4]>, w: usize, h: usize, plan: &BloomPlan) -> Pyramid {
        let dims: [(usize, usize); BLOOM_LEVELS] = std::array::from_fn(|j| {
            let (lw, lh) = bloom_dim(w as u32, h as u32, j);
            (lw as usize, lh as usize)
        });
        // Down: each level the one above under the binomial [1 3 3 1] / 8 each way, centred between its texels.
        let mut levels = vec![level0];
        for j in 1..BLOOM_LEVELS {
            let (src, sd, (lw, lh)) = (&levels[j - 1], dims[j - 1], dims[j]);
            const K: [f32; 4] = [0.125, 0.375, 0.375, 0.125];
            let next: Vec<[f32; 4]> = (0..lw * lh)
                .into_par_iter()
                .map(|i| {
                    let (x, y) = ((i % lw) as isize * 2, (i / lw) as isize * 2);
                    let mut s = [0f32; 4];
                    for (dy, ky) in K.iter().enumerate() {
                        for (dx, kx) in K.iter().enumerate() {
                            let t = texel(src, sd, x + dx as isize - 1, y + dy as isize - 1);
                            for k in 0..4 {
                                s[k] += t[k] * kx * ky;
                            }
                        }
                    }
                    s
                })
                .collect();
            levels.push(next);
        }
        // The coarsest level: its own weight, and the wide Gaussians summed over it (edge texels repeat outward).
        let top = BLOOM_LEVELS - 1;
        let mut up = Self::wide(&levels[top], dims[top], plan);
        // Up: each level by its weight, plus the sum so far, bilinear.
        for j in (0..top).rev() {
            let (lw, lh) = dims[j];
            let (coarse, cd, own) = (&up, dims[j + 1], &levels[j]);
            let (wc, wb) = (plan.level[j], plan.base_level[j]);
            up = (0..lw * lh)
                .into_par_iter()
                .map(|i| {
                    let c = bilinear4(coarse, cd, ((i % lw) as f32 + 0.5) / 2.0 - 0.5, ((i / lw) as f32 + 0.5) / 2.0 - 0.5);
                    let o = own[i];
                    [o[0] * wc + c[0], o[1] * wc + c[1], o[2] * wc + c[2], o[3] * wb + c[3]]
                })
                .collect();
        }
        Pyramid { dims, sum: up }
    }

    fn wide(l: &[[f32; 4]], dim: (usize, usize), plan: &BloomPlan) -> Vec<[f32; 4]> {
        let (lw, lh) = dim;
        let (wc, wb) = (plan.level[BLOOM_LEVELS - 1], plan.base_level[BLOOM_LEVELS - 1]);
        let r = if plan.has_wide() { plan.radius as isize } else { 0 };
        // (Each kernel over its sum, as the passes divide it: `bloom_wide_rows` the rows', `bloom_wide` the columns'.)
        let norm = |inv: f32| (-r..=r).map(|d| (-(d * d) as f32 * inv).exp()).sum::<f32>().powi(2);
        let pair = |g: [f32; 2]| if g[0] > 0.0 { [g[0] / norm(g[1]), g[1]] } else { g };
        let (wide, base_wide) = (plan.wide.map(pair), pair(plan.base_wide));
        (0..lw * lh)
            .into_par_iter()
            .map(|i| {
                let (x, y) = ((i % lw) as isize, (i / lw) as isize);
                let o = l[i];
                let mut s = [o[0] * wc, o[1] * wc, o[2] * wc, o[3] * wb];
                if r > 0 {
                    for dy in -r..=r {
                        for dx in -r..=r {
                            let d2 = (dx * dx + dy * dy) as f32;
                            let (mut g, gb) = (0f32, base_wide[0] * (-d2 * base_wide[1]).exp());
                            for k in wide {
                                if k[0] > 0.0 {
                                    g += k[0] * (-d2 * k[1]).exp();
                                }
                            }
                            let t = texel(l, dim, x + dx, y + dy);
                            s = [s[0] + t[0] * g, s[1] + t[1] * g, s[2] + t[2] * g, s[3] + t[3] * gb];
                        }
                    }
                }
                s
            })
            .collect()
    }

    /// The sum at output pixel (x, y): rgb the bloom (unexposed), w the wide blur of log2 luminance.
    fn at(&self, x: usize, y: usize) -> [f32; 4] {
        bilinear4(&self.sum, self.dims[0], (x as f32 + 0.5) / 4.0 - 0.5, (y as f32 + 0.5) / 4.0 - 0.5)
    }
}

/// How much of a pixel blooms: all of it under a negative threshold; else nothing up to the threshold (exposed
/// luminance) and all of it one unit above (the setting's documented band).
fn bloom_share(exposed_luminance: f32, threshold: f32) -> f32 {
    if threshold < 0.0 { 1.0 } else { (exposed_luminance - threshold).clamp(0.0, 1.0) }
}

/// The darkest log2 luminance local exposure reasons about: 12 stops under middle grey at the frame's exposure
/// (black, log2 0, would otherwise drag every base around it down without end).
fn log_floor(exposure: f32) -> f32 {
    (MIDDLE_GREY / 4096.0 / exposure.max(1e-20)).log2()
}

// ------------------------------------------------------------------------------------------------ local exposure

/// Local exposure's factor for a pixel of exposed log2 luminance `l`, given the base layer's two estimates there
/// (exposed log2 luminance: `edge`, the edge-aware blur; `wide`, the wide Gaussian blur) and the pivot (log2).
///
/// A base / detail decomposition of log luminance (Durand & Dorsey 2002, "Fast Bilateral Filtering for the Display
/// of High-Dynamic-Range Images", section 6): the base is what an edge-preserving blur leaves, the detail the rest;
/// the base's distance from the pivot is scaled (`highlightContrast` above it, `shadowContrast` below), the detail
/// by `detailStrength`. The base is the edge-aware blur blended with a wide plain blur by `blurredLuminanceBlend`
/// (same meanings as Unreal's Local Exposure settings, per its documentation: the blend keeps highlights' shape
/// and avoids the edge-aware filter's ringing). All 1: factor 1.
pub fn local_exposure_factor(lx: &crate::desc::LocalExposure, l: f32, edge: f32, wide: f32, pivot: f32) -> f32 {
    let base = edge + (wide - edge) * lx.blurred_luminance_blend.clamp(0.0, 1.0);
    let contrast = if base > pivot { lx.highlight_contrast } else { lx.shadow_contrast };
    let out = pivot + (base - pivot) * contrast + (l - base) * lx.detail_strength;
    2f32.powf((out - l).clamp(-8.0, 8.0))
}

/// The edge-aware blur of the quarter-resolution log2 luminance: live.wgsl `lum_blur`, on the CPU (9 x 9 taps two
/// texels apart, a Gaussian in space times a Gaussian in the difference of log2 luminance).
fn lum_blur(l: &[f32], w: usize, h: usize) -> Vec<f32> {
    (0..w * h)
        .into_par_iter()
        .map(|p| {
            let (px, py) = ((p % w) as isize, (p / w) as isize);
            let c = l[p];
            let (mut acc, mut ws) = (0f32, 0f32);
            for j in -4..=4isize {
                for i in -4..=4isize {
                    let x = (px + i * 2).clamp(0, w as isize - 1) as usize;
                    let y = (py + j * 2).clamp(0, h as isize - 1) as usize;
                    let v = l[y * w + x];
                    let d = v - c;
                    let wgt = (-((i * i + j * j) as f32) / 18.0 - d * d * 0.5).exp();
                    acc += v * wgt;
                    ws += wgt;
                }
            }
            acc / ws
        })
        .collect()
}

// ------------------------------------------------------------------------------------------------ lens, grain

/// Vignette: the picture's falloff at `r` (distance from the centre over the half diagonal) for the setting `v`.
///
/// Natural vignetting: the illuminance of an image point falls as cos^4 of its field angle (the "cosine fourth"
/// law; e.g. Kingslake, "Optics in Photography", 1992, ch. 6). The setting is the lens' reach: the corner's field
/// angle has the tangent `VIGNETTE_FIELD` x v. Own parametrisation; `VIGNETTE_FIELD` is fitted to the reference
/// pictures at v = 0.4 (their only value: 16:9 frames).
pub fn vignette(v: f32, r: f32) -> f32 {
    let t = VIGNETTE_FIELD * v * r;
    1.0 / ((1.0 + t * t) * (1.0 + t * t))
}
pub const VIGNETTE_FIELD: f32 = 1.425;

/// Chromatic aberration: how far red is magnified and blue reduced about the centre, a unit of the setting
/// (lateral colour: a lens' magnification varies with wavelength, so the fringe grows with the distance from the
/// centre; own design: a unit moves red and blue 0.2 % of that distance apart each way).
pub const FRINGE: f32 = 0.002;

/// Film grain's amplitude in the encoded picture at setting 1 (the standard deviation it adds to a mid grey, about
/// 9 of 255), own design.
pub const GRAIN: f32 = 0.085;

/// A 32-bit integer hash (Chris Wellons' "lowbias32", public domain: https://nullprogram.com/blog/2018/07/31/).
fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846ca68b);
    x ^= x >> 16;
    x
}

/// Film grain for pixel (x, y) of frame `frame` on an encoded value of luma `v`: zero-mean noise of triangular
/// distribution (the sum of two uniform numbers), the same on the three channels (luminance noise), weighted
/// 4 v (1 - v): none in black or white, most in the mid-tones. The live `post` computes the same number.
pub fn grain(x: u32, y: u32, frame: u32, v: f32, amount: f32) -> f32 {
    let h = hash(x ^ hash(y ^ hash(frame)));
    let (a, b) = ((h & 0xffff) as f32 / 65536.0, (h >> 16) as f32 / 65536.0);
    amount * GRAIN * (a + b - 1.0) * 4.0 * v * (1.0 - v)
}

// ------------------------------------------------------------------------------------------------ the chain

/// Full chain -> RGBA8. `exposure`: the scale `exposure_scale` gave. `compensation`: what local exposure's pivot is
/// over middle grey (auto exposure: 2^compensation, so the pivot is the metered average; manual: 1).
///
/// In the order a pixel goes through it: chromatic aberration (red and blue read from where the lens puts them),
/// exposure, local exposure, bloom added, vignette, colour grading and the tone curve, the sRGB encoding, film grain.
/// (The fringe is taken of the scene's image alone, not of the glow over it, and grain is added to the encoded value:
/// each is the same picture to the eye as the order a camera has them in, and the live final pass reads its inputs
/// once this way.)
pub fn finish(img: &Image, exposure: f32, compensation: f32, p: &PostPreset) -> Vec<u8> {
    finish_frame(img, exposure, compensation, p, 0)
}

/// `finish` for frame `frame` of a sequence (film grain is new each frame).
pub fn finish_frame(img: &Image, exposure: f32, compensation: f32, p: &PostPreset, frame: u32) -> Vec<u8> {
    let (w, h) = (img.w, img.h);
    let tm = Tonemapper::new(p);
    let lx = &p.local_exposure;
    let clean = |c: Vec3| if c.is_finite() { c.max(Vec3::ZERO) } else { Vec3::ZERO };
    // Bloom: the sum of the stages' Gaussians of what blooms (unexposed).
    let stages = bloom_stages(p, w as u32);
    let glow: Option<Vec<Vec3>> = (!stages.is_empty()).then(|| {
        let src: Vec<Vec3> = img
            .px
            .par_iter()
            .map(|c| {
                let c = clean(*c);
                c * bloom_share(c.dot(LUMA) * exposure, p.bloom.threshold)
            })
            .collect();
        let mut sum = vec![Vec3::ZERO; w * h];
        for (a, sigma) in stages {
            let g = gaussian(&src, w, h, sigma);
            sum.par_iter_mut().zip(g).for_each(|(s, g)| *s += g * a);
        }
        sum
    });
    // Local exposure's two base layers, on the quarter-resolution log2 luminance: the edge-aware blur and the wide
    // Gaussian.
    let floor = log_floor(exposure);
    let base: Option<(Image, Image)> = local_exposure_sigma(p, w as u32).map(|sigma| {
        let (qw, qh, l) = quarter_log_luminance(img);
        let l: Vec<f32> = l.iter().map(|v| v.max(floor)).collect();
        let wide = gaussian(&l.iter().map(|v| Vec3::splat(*v)).collect::<Vec<_>>(), qw, qh, sigma / 4.0);
        let edge = lum_blur(&l, qw, qh).iter().map(|v| Vec3::splat(*v)).collect();
        (Image { w: qw, h: qh, px: edge }, Image { w: qw, h: qh, px: wide })
    });
    let centre = Vec3::new(w as f32 * 0.5, h as f32 * 0.5, 0.0);
    let half_diagonal = centre.length();
    let (fringe, pivot, log_e) = (p.chromatic_aberration.max(0.0) * FRINGE, (MIDDLE_GREY * compensation).log2(), exposure.log2());
    img.px
        .par_iter()
        .enumerate()
        .flat_map_iter(|(i, c)| {
            let (x, y) = (i % w, i / w);
            let at = Vec3::new(x as f32 + 0.5, y as f32 + 0.5, 0.0) - centre;
            let mut c = *c;
            if fringe > 0.0 {
                let (r, b) = (centre + at * (1.0 - fringe) - 0.5, centre + at * (1.0 + fringe) - 0.5);
                c = Vec3::new(img.bilinear(r.x, r.y).x, c.y, img.bilinear(b.x, b.y).z);
            }
            let mut c = clean(c) * exposure;
            if let Some((edge, wide)) = &base {
                let (qx, qy) = ((x as f32 + 0.5) / 4.0 - 0.5, (y as f32 + 0.5) / 4.0 - 0.5);
                let l = c.dot(LUMA).log2().max(floor + log_e);
                c *= local_exposure_factor(lx, l, edge.bilinear(qx, qy).x + log_e, wide.bilinear(qx, qy).x + log_e, pivot);
            }
            if let Some(glow) = &glow {
                c += glow[i] * exposure;
            }
            c *= vignette(p.vignette, at.length() / half_diagonal);
            let d = tm.apply(c);
            let mut e = Vec3::new(linear_to_srgb(d.x.clamp(0.0, 1.0)), linear_to_srgb(d.y.clamp(0.0, 1.0)), linear_to_srgb(d.z.clamp(0.0, 1.0)));
            if p.film_grain > 0.0 {
                e += grain(x as u32, y as u32, frame, e.dot(LUMA), p.film_grain.min(1.0));
            }
            let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            [q(e.x), q(e.y), q(e.z), 255]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset() -> PostPreset {
        let p: crate::desc::Preset = serde_json::from_str(include_str!("../../presets/cinematic.json")).unwrap();
        p.post
    }

    /// Greys stay grey, black is black, the curve is monotonic and ends at its top.
    #[test]
    fn the_tone_curve_is_a_curve() {
        let tm = Tonemapper::new(&preset());
        assert!(tm.apply(Vec3::ZERO).max_element() < 1e-6);
        let mut last = 0.0;
        for k in 0..400 {
            let x = 10f32.powf(-4.0 + k as f32 * 7.0 / 399.0);
            let d = tm.apply(Vec3::splat(x));
            assert!((d.x - d.y).abs() < 2e-4 && (d.y - d.z).abs() < 2e-4, "grey {x} -> {d}");
            assert!(d.y >= last, "not monotonic at {x}");
            last = d.y;
        }
        assert!((last - ToneCurve::default().white).abs() < 1e-3);
        // Middle grey lands near middle grey, and the branches meet where they are said to.
        let mid = ToneCurve::default();
        assert!((tm.apply(Vec3::splat(mid.mid_in)).y - mid.mid_out).abs() < 1e-3);
        assert!((tm.apply(Vec3::splat(0.18)).y - 0.18).abs() < 0.02);
    }

    /// The live table against the function it tabulates, sampled trilinearly as the final pass samples it.
    #[test]
    fn the_table_follows_the_curve() {
        let tm = Tonemapper::new(&preset());
        const N: usize = LUT_SIZE as usize;
        let enc = |d: Vec3| Vec3::new(linear_to_srgb(d.x.clamp(0.0, 1.0)), linear_to_srgb(d.y.clamp(0.0, 1.0)), linear_to_srgb(d.z.clamp(0.0, 1.0)));
        let lut: Vec<Vec3> = (0..N * N * N).map(|i| enc(tm.apply(Vec3::new(lut_value((i % N) as f32 / (N - 1) as f32), lut_value((i / N % N) as f32 / (N - 1) as f32), lut_value((i / N / N) as f32 / (N - 1) as f32))))).collect();
        let sample = |c: Vec3| {
            let t = Vec3::new(lut_coord(c.x), lut_coord(c.y), lut_coord(c.z)) * (N - 1) as f32;
            let i = t.floor().min(Vec3::splat(N as f32 - 2.0));
            let f = t - i;
            let at = |dx: usize, dy: usize, dz: usize| lut[(i.x as usize + dx) + (i.y as usize + dy) * N + (i.z as usize + dz) * N * N];
            let x = |dy, dz| at(0, dy, dz).lerp(at(1, dy, dz), f.x);
            x(0, 0).lerp(x(1, 0), f.y).lerp(x(0, 1).lerp(x(1, 1), f.y), f.z)
        };
        // Greys from 1e-3 to 60: within half an 8-bit step.
        let mut worst = 0f32;
        for k in 0..2000 {
            let x = 10f32.powf(-3.0 + k as f32 * (60f32.log10() + 3.0) / 1999.0);
            worst = worst.max((sample(Vec3::splat(x)) - enc(tm.apply(Vec3::splat(x)))).abs().max_element());
        }
        assert!(worst < 0.5 / 255.0, "greys off by {worst}");
        // Colours: within two 8-bit steps on average over a random set (the table bends where the looks do).
        let (mut sum, mut n) = (0f32, 0);
        for k in 0..20000u32 {
            let r = |s: u32| 10f32.powf(-3.0 + (hash(k * 3 + s) as f32 / u32::MAX as f32) * 4.5);
            let c = Vec3::new(r(0), r(1), r(2));
            sum += (sample(c) - enc(tm.apply(c))).abs().max_element();
            n += 1;
        }
        assert!(sum / (n as f32) < 2.0 / 255.0, "colours off by {} on average", sum / n as f32);
    }

    #[test]
    fn white_balance_at_6500_is_none_and_turns_the_right_way() {
        assert_eq!(white_balance(6500.0, 0.0), DMat3::IDENTITY);
        // A scene lit by a 3000 K lamp rendered white: the picture turns blue; by 12000 K sky: orange.
        let warm = white_balance(3000.0, 0.0) * DVec3::ONE;
        let cold = white_balance(12000.0, 0.0) * DVec3::ONE;
        assert!(warm.z > 2.0 * warm.x && cold.x > 1.5 * cold.z, "{warm} {cold}");
        // Near 6500 K it is nearly none, and continuous across the loci's joint at 4000 K.
        assert!((white_balance(6510.0, 0.0) * DVec3::ONE - DVec3::ONE).abs().max_element() < 0.01);
        assert!((white_balance(3999.0, 0.0) * DVec3::ONE - white_balance(4001.0, 0.0) * DVec3::ONE).abs().max_element() < 0.01);
    }

    #[test]
    fn metering_takes_the_window_of_the_histogram() {
        // A uniform image: its luminance, whatever the window.
        let (mut count, mut mean) = ([0f32; AE_BINS], [0f32; AE_BINS]);
        let t = (3000f32.log2() + 10.0) / 30.0;
        count[(t * 64.0) as usize] = 1000.0;
        mean[(t * 64.0) as usize] = t;
        let avg = metered_average(&count, &mean, 0.1, 0.9, -10.0, 20.0).unwrap();
        assert!((avg / 3000.0 - 1.0).abs() < 1e-3, "{avg}");
        assert!((-auto_target(avg, 0.0, -10.0, 20.0).log2() - (3000f32 / 0.18).log2()).abs() < 1e-3);
        // A tenth of the frame a hundred times brighter is outside a 10 .. 80 % window.
        let b = ((t + 6.6 / 30.0) * 64.0) as usize;
        count[b] = 100.0;
        mean[b] = t + 6.6 / 30.0;
        let avg = metered_average(&count, &mean, 0.1, 0.8, -10.0, 20.0).unwrap();
        assert!((avg / 3000.0 - 1.0).abs() < 1e-3, "{avg}");
        // The limits hold the average.
        assert!((auto_target(1e-6, 0.0, 1.0, 5.0) - 0.5).abs() < 1e-6 && (auto_target(1e9, 1.0, 1.0, 5.0) - 2.0 / 32.0).abs() < 1e-6);
        assert!(metered_average(&[0.0; AE_BINS], &mean, 0.1, 0.9, -10.0, 20.0).is_none());
    }

    /// The pyramid's sum is the sum of Gaussians it stands for: every stage's share is in the plan, and a point's
    /// glow has the narrow stages' energy near it and the second moment of their Gaussians.
    #[test]
    fn bloom_of_a_point_is_the_stages_gaussians() {
        let mut p = preset();
        let (w, h) = (960usize, 540usize);
        let plan = BloomPlan::new(&p, w as u32, h as u32);
        let stages: f32 = p.bloom.tints.iter().sum::<f32>() * p.bloom.intensity * BLOOM_GAIN;
        let wide: f32 = plan.wide.iter().map(|g| g[0]).sum();
        let levels: f32 = plan.fine + plan.level.iter().sum::<f32>();
        assert!((levels + wide - stages).abs() < 1e-4 * stages, "{levels} + {wide} != {stages}");
        // One stage alone, a Gaussian of 3 % of the width (sigma 29.3 px): a point in the middle of a black frame
        // (a unit of energy: one output pixel of 16 in a 4 x 4 block) glows with the stage's energy and variance.
        p.bloom.size_scale = 1.0;
        p.bloom.sizes = vec![30.0];
        p.bloom.tints = vec![1.0];
        let plan = BloomPlan::new(&p, w as u32, h as u32);
        let sigma = (BLOOM_SIGMA * 0.3 * w as f32) as f64;
        let mut level0 = vec![[0f32; 4]; w.div_ceil(4) * h.div_ceil(4)];
        level0[(h / 8) * w.div_ceil(4) + w / 8] = [1.0 / 16.0, 0.0, 0.0, 0.0];
        let py = Pyramid::new(level0, w, h, &plan);
        let (cx, cy) = (w as f64 / 2.0 + 2.0, h as f64 / 2.0 + 2.0);
        let (mut e, mut m2) = (0f64, 0f64);
        for y in 0..h {
            for x in 0..w {
                let v = py.at(x, y)[0] as f64;
                e += v;
                m2 += v * (x as f64 + 0.5 - cx).powi(2);
            }
        }
        let want = (p.bloom.intensity * BLOOM_GAIN) as f64;
        assert!((e / want - 1.0).abs() < 0.02, "energy {e} for {want}");
        assert!(((m2 / e).sqrt() / sigma - 1.0).abs() < 0.05, "sigma {} for {sigma}", (m2 / e).sqrt());
    }

    #[test]
    fn local_exposure_is_none_at_one_and_compresses_below() {
        let mut lx = crate::desc::LocalExposure::default();
        assert_eq!(local_exposure_factor(&lx, 1.0, 2.0, 3.0, 0.0), 1.0);
        lx.highlight_contrast = 0.5;
        lx.shadow_contrast = 0.5;
        // A region two stops over the pivot comes down a stop, one two stops under comes up a stop; detail stays.
        let f = local_exposure_factor(&lx, 2.3, 2.0, 2.0, 0.0);
        assert!((f - 0.5).abs() < 1e-5, "{f}");
        let f = local_exposure_factor(&lx, -2.0, -2.0, -2.0, 0.0);
        assert!((f - 2.0).abs() < 1e-5, "{f}");
    }

    #[test]
    fn the_chain_runs_and_grain_has_no_mean() {
        let mut p = preset();
        let (w, h) = (64usize, 36usize);
        let img = Image { w, h, px: vec![Vec3::splat(0.18); w * h] };
        let plain = finish(&img, 1.0, 1.0, &p);
        p.film_grain = 1.0;
        p.chromatic_aberration = 2.0;
        p.local_exposure.highlight_contrast = 0.8;
        p.local_exposure.shadow_contrast = 0.8;
        let grainy = finish(&img, 1.0, 1.0, &p);
        let mean = |v: &[u8]| v.chunks(4).map(|c| c[1] as f32).sum::<f32>() / (w * h) as f32;
        assert!((mean(&plain) - mean(&grainy)).abs() < 2.0, "{} {}", mean(&plain), mean(&grainy));
        assert!(plain != grainy);
    }
}
