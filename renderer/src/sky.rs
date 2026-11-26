//! The sky: a physical atmosphere (the sun's light scattered by air, haze and ozone above a ground), baked into an
//! equirectangular environment map (Y-up; the lower hemisphere black, with a fade at the horizon); analytic domes for
//! night and overcast skies; a cloud layer from a mask.

use glam::Vec3;
use rayon::prelude::*;
use std::f32::consts::PI;

// ------------------------------------------------------------------------------------------------ the atmosphere
//
// The medium is the Earth atmosphere of Bruneton & Neyret 2008 ("Precomputed Atmospheric Scattering", EGSR) in the
// RGB form of Bruneton's 2017 reimplementation and of Hillaire 2020 ("A Scalable and Production Ready Sky and
// Atmosphere Rendering Technique", EGSR), section 4 and Table 1: a spherical planet, air that scatters (Rayleigh),
// haze that scatters and absorbs (Mie), ozone that absorbs, a Lambertian ground. Lengths are km here.
//
// What is computed, and how (the numerical methods are this file's own):
//  - `columns_exact`: the amount of each component along a ray to the top of the atmosphere, by Gauss-Legendre
//    quadrature; `slant` extends it to rays that point down.
//  - `Atmosphere::sun_transmittance`: the transmittance from the ground toward the sun, from those columns.
//  - `Luts::build`: the same optical depth as a table for the sky's inner loop, and the light scattered more than
//    once as tables over altitude and the sun's zenith angle (the isotropic approximation of Hillaire 2020 section
//    5.5, computed order by order).
//  - `Luts::sky`: equation 11 of that paper integrated along the view ray, per texel of the bake, by Simpson's rule.
// The tests at the end of the file hold the model to the same medium path traced.

/// Radii of the ground and of the atmosphere's top, km (Hillaire 2020 section 4: 6360 km and 6460 km).
const R_GROUND: f64 = 6360.0;
const R_TOP: f64 = 6460.0;
/// Scale heights of the air's and the haze's density, km: d(h) = exp(-h / H) (Bruneton & Neyret 2008 section 2.1,
/// H_R = 8 km, H_M = 1.2 km; Hillaire 2020 section 4).
const H_RAYLEIGH: f64 = 8.0;
const H_MIE: f64 = 1.2;
/// Rayleigh scattering at sea level, per km, at 680 / 550 / 440 nm (Hillaire 2020 Table 1; Bruneton 2017 demo.cc:
/// 1.24062e-6 lambda^-4 per m, lambda in micrometres).
const RAYLEIGH_SCATTERING: [f64; 3] = [5.802e-3, 13.558e-3, 33.1e-3];
/// Mie scattering and absorption at sea level, per km (Bruneton 2017 demo.cc: extinction 5.328e-3 / 1200 m = 4.44e-6
/// per m, single-scattering albedo 0.9; the scattering is Hillaire 2020 Table 1's 3.996e-6 per m). The kernel's aerial
/// perspective (pathtrace.wgsl `aerial_tr`) has the same two numbers: they are one air.
const MIE_SCATTERING: f64 = 3.996e-3;
const MIE_ABSORPTION: f64 = 0.444e-3;
/// Ozone absorption at the layer's densest, per km (Hillaire 2020 Table 1; Bruneton 2017: 300 Dobson units), and the
/// layer: a tent of width 30 km centred at 25 km, d(h) = max(0, 1 - |h - 25| / 15) (Hillaire 2020 section 4).
const OZONE_ABSORPTION: [f64; 3] = [0.650e-3, 1.881e-3, 0.085e-3];
const OZONE_CENTRE: f64 = 25.0;
const OZONE_HALF_WIDTH: f64 = 15.0;

/// How far below the ground's surface a ray toward the sun may pass and still carry light, km (own design, see
/// `slant`): the ground is not a hard edge for the sun's light, so that a setting sun fades instead of switching off.
const SOFT_GROUND: f64 = 5.0;
const R_BASE: f64 = R_GROUND - SOFT_GROUND;
/// The soft ground's own optical depth for a ray that passes `depth` km under the surface: (depth / 2 km)^4 (own
/// design: under 6 % at the 1 km a ray 1 degree below the horizon reaches, opaque by 4 km, 2 degrees). Earth's own
/// air does most of the fading (its density continued downward, `densities`); this term keeps a thin or absent
/// atmosphere (`rayleighScale`, `mieScale` near 0) from lighting the ground from below the horizon.
fn ground_depth(p: f64) -> f64 {
    let d = ((R_GROUND - p) / 2.0).max(0.0);
    d * d * d * d
}

/// The sun's light on the ground, calibrated to the previous release (its measured table of the sun's colour by
/// elevation, a black box). That release's sun was dimmer and redder than this medium's transmittance, by an optical
/// depth that fits a thin veil of the sea-level air and haze high in the atmosphere: `SUN_VEIL_COLUMN` km of them
/// (straight up) in a layer at `SUN_VEIL_ALTITUDE` km, which the sun's ray crosses at a slant. With it the sun's
/// transmittance is within 1.1 % of that table from 5 degrees of elevation up, 1.5 % at 2 and 3 degrees, 5 % at 1.
/// Scenes were lit with that sun, so `Atmosphere::sun_transmittance` keeps it: the medium's transmittance times the
/// veil's. It is not part of the published medium, and the sky's own light (`Luts`) has none of it (that release's
/// sky fits the medium without it). A column of 0 gives the medium's own transmittance: +1.6 / +2.8 / +5.9 % (R / G
/// / B) with the sun at the zenith, +7.8 / +14 / +32 % at 10 degrees.
const SUN_VEIL_COLUMN: f64 = 1.65;
const SUN_VEIL_ALTITUDE: f64 = 55.0;

pub struct Atmosphere {
    /// Scattering and extinction at sea level, per km, with the scene's scales in.
    rayleigh: [f64; 3],
    mie_scattering: f64,
    mie_extinction: f64,
    ozone: [f64; 3],
    mie_g: f64,
    ground_albedo: [f64; 3],
}

/// The densities of air, haze and ozone at altitude `h` km, relative to the constants above. Below the ground (h < 0;
/// only `slant`'s soft ground asks) the exponentials go on.
fn densities(h: f64) -> [f64; 3] {
    [(-h / H_RAYLEIGH).exp(), (-h / H_MIE).exp(), (1.0 - (h - OZONE_CENTRE).abs() / OZONE_HALF_WIDTH).max(0.0)]
}

/// e^x of four numbers at once in single precision (the result's relative error is that of the argument's last bit:
/// under 3e-7 for |x| < 2, 4e-6 at -87), for the inner loops: x = k ln 2 + r with k whole and |r| <= ln 2 / 2, e^r by
/// its Taylor polynomial of degree 7, 2^k by its bits (own code; no branches, so the compiler makes it one vector
/// operation). Arguments are clamped to [-87, 88]: below that the result is 1.6e-38, not 0.
#[inline(always)]
fn exp4(x: [f64; 4]) -> [f64; 4] {
    let mut out = [0.0f64; 4];
    for i in 0..4 {
        let v = (x[i] as f32).clamp(-87.0, 88.0);
        let k = (v * std::f32::consts::LOG2_E + 0.5).floor();
        // (ln 2 in two parts, its leading bits first: k times them has little or nothing to round.)
        let r = (v - k * 0.693_145_75) - k * 1.428_606_8e-6;
        let poly = 1.0 + r * (1.0 + r * (0.5 + r * (1.0 / 6.0 + r * (1.0 / 24.0 + r * (1.0 / 120.0 + r * (1.0 / 720.0 + r * (1.0 / 5040.0)))))));
        out[i] = (poly * f32::from_bits(((k as i32 + 127) as u32) << 23)) as f64;
    }
    out
}

/// The distance from radius `r` along a ray of zenith cosine `mu` to the sphere of radius `to` (the far crossing).
fn distance_out(r: f64, mu: f64, to: f64) -> f64 {
    -r * mu + (r * r * (mu * mu - 1.0) + to * to).max(0.0).sqrt()
}

/// The distance to the ground along a ray that points down, if it reaches it.
fn distance_to_ground(r: f64, mu: f64) -> Option<f64> {
    let disc = r * r * (mu * mu - 1.0) + R_GROUND * R_GROUND;
    (mu < 0.0 && disc >= 0.0).then(|| (-r * mu - disc.sqrt()).max(0.0))
}

/// Gauss-Legendre nodes and weights, 8 points on [-1, 1] (Abramowitz & Stegun 1964, Table 25.4; the positive half).
const GL8: [(f64, f64); 4] = [
    (0.183_434_642_495_649_8, 0.362_683_783_378_362),
    (0.525_532_409_916_329, 0.313_706_645_877_887_3),
    (0.796_666_477_413_626_7, 0.222_381_034_453_374_5),
    (0.960_289_856_497_536_2, 0.101_228_536_290_376_3),
];

/// The columns of air, haze and ozone (km at their reference densities) from radius `r` to the top of the atmosphere
/// along a ray that points up or level (`mu` >= 0): the integral of `densities`, by 8-point Gauss-Legendre quadrature
/// on 12 panels of u, the distance being d u^2 (the nodes crowd the start, where the ray is in the dense air).
fn columns_exact(r: f64, mu: f64) -> [f64; 3] {
    const PANELS: usize = 12;
    let d = distance_out(r, mu, R_TOP);
    let mut n = [0.0f64; 3];
    for p in 0..PANELS {
        let (centre, half) = ((p as f64 + 0.5) / PANELS as f64, 0.5 / PANELS as f64);
        for (x, w) in GL8 {
            for s in [-1.0, 1.0] {
                let u = centre + s * half * x;
                let t = d * u * u;
                let dens = densities((r * r + t * t + 2.0 * r * mu * t).sqrt() - R_GROUND);
                let k = w * half * 2.0 * d * u;
                for c in 0..3 {
                    n[c] += k * dens[c];
                }
            }
        }
    }
    n
}

/// The columns from radius `r` to the top along any ray, and the soft ground's optical depth (`ground_depth`). A ray
/// that points down passes its lowest point, at radius p = r sqrt(1 - mu^2), and from there on it is the level ray
/// from p; before that point it is the mirror image of the ray that points up from `r`:
/// columns(r, mu) = 2 columns(p, 0) - columns(r, -mu).
/// Own design for the sun below a point's horizon: the ray is not cut by the ground's sphere. It goes through the
/// ground's first kilometres, where the air's density is continued downward and `ground_depth` is added, so the
/// sun's light falls smoothly to nothing within about two degrees below the horizon instead of at one angle (on the
/// real Earth refraction and the sun's half degree do the like: the sun's upper limb is in view until its centre is
/// 0.83 degrees below the geometric horizon). `None`: the ray is deeper than SOFT_GROUND, no light.
fn slant(r: f64, mu: f64) -> Option<([f64; 3], f64)> {
    if mu >= 0.0 {
        return Some((columns_exact(r, mu), 0.0));
    }
    let p = r * (1.0 - mu * mu).max(0.0).sqrt();
    if p <= R_BASE {
        return None;
    }
    let (level, mirror) = (columns_exact(p, 0.0), columns_exact(r, -mu));
    Some(([0, 1, 2].map(|c| (2.0 * level[c] - mirror[c]).max(0.0)), ground_depth(p)))
}

impl Atmosphere {
    /// Earth's atmosphere with its Rayleigh and Mie scattering scaled (1 = Earth's), the haze's anisotropy `mie_g`
    /// and the ground's albedo.
    pub fn earth(rayleigh_scale: f32, mie_scale: f32, mie_g: f32, ground_albedo: Vec3) -> Self {
        let (rs, ms) = (rayleigh_scale.max(0.0) as f64, mie_scale.max(0.0) as f64);
        Atmosphere {
            rayleigh: RAYLEIGH_SCATTERING.map(|b| b * rs),
            mie_scattering: MIE_SCATTERING * ms,
            // (The absorption is not scaled: as the kernel's aerial perspective has it.)
            mie_extinction: MIE_SCATTERING * ms + MIE_ABSORPTION,
            ozone: OZONE_ABSORPTION,
            mie_g: (mie_g as f64).clamp(-0.99, 0.99),
            ground_albedo: [ground_albedo.x, ground_albedo.y, ground_albedo.z].map(|v| (v as f64).clamp(0.0, 1.0)),
        }
    }

    /// Extinction (RGB, per km) at densities of air, haze and ozone.
    #[inline(always)]
    fn extinction(&self, dens: [f64; 3]) -> [f64; 3] {
        [0, 1, 2].map(|c| self.rayleigh[c] * dens[0] + self.mie_extinction * dens[1] + self.ozone[c] * dens[2])
    }

    /// Optical depth (RGB) from radius `r` to the top of the atmosphere along a ray of zenith cosine `mu` (`slant`);
    /// `OPAQUE` where the ground lets nothing through.
    fn optical_depth(&self, r: f64, mu: f64) -> [f64; 3] {
        match slant(r, mu) {
            Some((n, ground)) => self.extinction(n).map(|t| (t + ground).min(OPAQUE)),
            None => [OPAQUE; 3],
        }
    }

    /// The medium's transmittance (RGB) from the ground toward a sun of zenith cosine `mu`, without the sun's veil.
    fn ground_transmittance(&self, mu: f64) -> [f64; 3] {
        self.optical_depth(R_GROUND, mu).map(|t| (-t).exp())
    }

    /// The share of the sun's light (RGB) that reaches the ground from the direction `sun_dir_y_up` (world, Y up):
    /// what a directional sun light of the scene is tinted by. The transmittance of the sky's own medium from sea
    /// level to the top of the atmosphere, smooth through the horizon (`slant`), times the calibration's veil
    /// (`SUN_VEIL_COLUMN`).
    pub fn sun_transmittance(&self, sun_dir_y_up: Vec3) -> Vec3 {
        let mu = (sun_dir_y_up.normalize_or_zero().y as f64).clamp(-1.0, 1.0);
        let medium = self.ground_transmittance(mu);
        // The veil's column along the ray: a thin layer at a radius R, crossed at the angle whose sine is
        // R_GROUND cos(elevation) / R (the same above and below the horizon).
        let level = R_GROUND / (R_GROUND + SUN_VEIL_ALTITUDE);
        let veil = SUN_VEIL_COLUMN / (1.0 - level * level * (1.0 - mu * mu)).sqrt();
        Vec3::from([0, 1, 2].map(|c| (medium[c] * (-veil * (self.rayleigh[c] + self.mie_extinction)).exp()) as f32))
    }

    /// The air's phase function (Rayleigh; Hillaire 2020 section 4), per steradian, for the cosine of the angle
    /// between the light's and the view's directions.
    fn phase_rayleigh(nu: f64) -> f64 {
        3.0 / (16.0 * std::f64::consts::PI) * (1.0 + nu * nu)
    }

    /// The haze's phase function: Henyey & Greenstein 1941 ("Diffuse radiation in the galaxy", Astrophysical Journal
    /// 93) of anisotropy g, per steradian (Hillaire 2020 section 4 names it as the simpler alternative to its
    /// Cornette-Shanks form).
    fn phase_mie(&self, nu: f64) -> f64 {
        let g = self.mie_g;
        (1.0 - g * g) / (4.0 * std::f64::consts::PI * (1.0 + g * g - 2.0 * g * nu).max(1e-9).powf(1.5))
    }
}

/// The optical depth tables hold where no light gets through.
const OPAQUE: f64 = 80.0;

/// Where a point is in the tables over the sun's zenith cosine and altitude (`Table`), all in 0..1. Own choice of
/// coordinates (Hillaire 2020 section 5.5.2 has both linear):
///  - the altitude as its square root, denser near the ground, where the haze is;
///  - for the scattered light, the sun's zenith cosine as sign(mu) sqrt(|mu|), which puts half the entries within 14
///    degrees of the horizon, where the light turns from day to night;
///  - for the optical depth toward the sun, the same above the level direction; below it, by how far under the point
///    the sun's ray passes at its lowest (radius p), as sqrt((r - p) / (r - R_BASE)): it is that depth which decides
///    how much of the sun's light is left, at any altitude, down to the soft ground's end (Bruneton & Neyret 2008
///    section 4 make the like point for their tables: a direction is best taken by its ray's length to the boundary).
#[derive(Clone, Copy)]
struct At {
    sun: f64,
    scatter: f64,
    altitude: f64,
}

impl At {
    #[inline(always)]
    fn new(mu_sun: f64, r: f64) -> At {
        let root = mu_sun.abs().sqrt();
        let altitude = ((r - R_GROUND) * (1.0 / (R_TOP - R_GROUND))).clamp(0.0, 1.0).sqrt();
        if mu_sun >= 0.0 {
            return At { sun: 0.5 + 0.5 * root, scatter: 0.5 + 0.5 * root, altitude };
        }
        // r - p, p = r sqrt(1 - mu^2), without the difference of two near numbers.
        let under = r * mu_sun * mu_sun / (1.0 + (1.0 - mu_sun * mu_sun).max(0.0).sqrt());
        At { sun: 0.5 - 0.5 * (under / (r - R_BASE)).min(1.0).sqrt(), scatter: 0.5 - 0.5 * root, altitude }
    }
    /// On the ground.
    fn ground(mu_sun: f64) -> At {
        At::new(mu_sun, R_GROUND)
    }
    /// The sun's zenith cosine at a coordinate of the scattered light's tables, and of the sun's table at radius `r`.
    fn mu_of_scatter(x: f64, _r: f64) -> f64 {
        let s = 2.0 * x - 1.0;
        s * s.abs()
    }
    fn mu_of_sun(x: f64, r: f64) -> f64 {
        if x >= 0.5 {
            return (2.0 * x - 1.0).powi(2);
        }
        let p = r - (r - R_BASE) * (1.0 - 2.0 * x).powi(2);
        -(1.0 - (p / r).powi(2)).max(0.0).sqrt()
    }
}

/// An RGB table over the sun's zenith cosine (columns: one of `At`'s two coordinates) and altitude (rows; one row:
/// the ground), linear between entries.
struct Table {
    columns: usize,
    rows: usize,
    data: Vec<[f32; 3]>,
}

impl Table {
    /// `f` of the sun's zenith cosine (`mu_of` the column's coordinate) and the radius of each entry.
    fn fill(columns: usize, rows: usize, mu_of: fn(f64, f64) -> f64, f: impl Fn(f64, f64) -> [f64; 3] + Sync) -> Table {
        let data = (0..columns * rows)
            .into_par_iter()
            .map(|i| {
                let q = if rows > 1 { (i / columns) as f64 / (rows - 1) as f64 } else { 0.0 };
                let r = R_GROUND + q * q * (R_TOP - R_GROUND);
                f(mu_of((i % columns) as f64 / (columns - 1) as f64, r), r).map(|v| v as f32)
            })
            .collect();
        Table { columns, rows, data }
    }

    #[inline(always)]
    fn at(&self, column: f64, altitude: f64) -> [f64; 3] {
        let fu = column * (self.columns - 1) as f64;
        let iu = (fu as usize).min(self.columns - 2);
        let wu = (fu - iu as f64) as f32;
        let row = |v: usize| {
            let (p, q) = (self.data[v * self.columns + iu], self.data[v * self.columns + iu + 1]);
            [0, 1, 2].map(|c| p[c] + (q[c] - p[c]) * wu)
        };
        if self.rows == 1 {
            return row(0).map(|v| v as f64);
        }
        let fv = altitude * (self.rows - 1) as f64;
        let iv = (fv as usize).min(self.rows - 2);
        let (lo, hi, wv) = (row(iv), row(iv + 1), (fv - iv as f64) as f32);
        [0, 1, 2].map(|c| (lo[c] + (hi[c] - lo[c]) * wv) as f64)
    }
}

/// A ray in the atmosphere: its start's radius, the zenith cosines of its direction and of the sun's there, the cosine
/// between the two directions, its length.
#[derive(Clone, Copy)]
struct Ray {
    r: f64,
    mu: f64,
    mu_sun: f64,
    nu: f64,
    length: f64,
}

impl Ray {
    /// From radius `r` to the ground or to the top of the atmosphere; whether it ends on the ground.
    fn new(r: f64, mu: f64, mu_sun: f64, nu: f64) -> (Ray, bool) {
        let ground = distance_to_ground(r, mu);
        (Ray { r, mu, mu_sun, nu, length: ground.unwrap_or_else(|| distance_out(r, mu, R_TOP)) }, ground.is_some())
    }
    /// The sun's zenith cosine where the ray ends on the ground.
    fn mu_sun_at_ground(&self) -> f64 {
        ((self.r * self.mu_sun + self.length * self.nu) / R_GROUND).clamp(-1.0, 1.0)
    }
}

/// How finely the tables of scattered light are computed (`Luts::build`).
#[derive(Clone, Copy)]
struct Quality {
    /// Scattering orders computed one by one, from the second; the last stands for all after it.
    orders: usize,
    /// Azimuths of the directions gathered from, over the half turn (x 16 zenith angles).
    azimuths: usize,
    /// Intervals of Simpson's rule along a gathering ray, and along a view ray of the sky (even numbers).
    steps: usize,
    view_steps: usize,
}

/// Against several times each (the test `quality_is_enough`): the sky within 1.5 % by day (the most at the horizon) and
/// within 8 % in the last of twilight, the sun 6 degrees down.
const QUALITY: Quality = Quality { orders: 3, azimuths: 2, steps: 12, view_steps: 16 };

/// The sizes of the tables: of the optical depth toward the sun (it turns from clear to opaque within a few degrees
/// at the horizon; an odd number of columns, so that one is the level direction, where its two coordinates meet), and
/// of the scattered light (broad).
const SUN_TABLE: (usize, usize) = (257, 128);
const SCATTER_TABLE: (usize, usize) = (64, 32);
/// The sun's zenith cosine under which the scattered light's tables are left at zero: 17 degrees below the horizon
/// (the sun's own light stops 10 degrees below it for a point at the top of the atmosphere; the scattered light of
/// the lit air beyond reaches a little further).
const NIGHT: f64 = -0.3;

struct Luts {
    /// The optical depth from a point to the top of the atmosphere toward the sun (`Atmosphere::optical_depth`).
    sun: Table,
    /// The light scattered twice or more, as a source: the radiance each point of the air scatters on per unit of
    /// its scattering coefficient, per unit of the sun's illuminance (sr^-1). The Psi_ms of Hillaire 2020 equation
    /// 10 (its use: equation 11); how it is computed here: `Luts::build`.
    scattered: Table,
    /// The radiance of the ground from the light that reaches it scattered (per unit of the sun's illuminance).
    ground: Table,
    view_steps: usize,
}

impl Luts {
    fn build(a: &Atmosphere) -> Luts {
        Luts::build_at(a, QUALITY)
    }

    /// The light of scattering orders two and up (`scattered`, `ground`).
    ///
    /// Hillaire 2020 section 5.5 takes the scattering of orders two and up as isotropic, which makes its source a
    /// function of altitude and the sun's zenith angle alone, and so do the tables here. That paper computes order
    /// two (equations 5 and 6: the light scattered once or sent back by the ground toward a point, averaged over
    /// all directions) and closes the series with one factor per point (equations 7-10), which takes the light around
    /// a point as uniform. Here the next orders are computed like the second, each from the table of the one before
    /// (the order-by-order scheme of Bruneton & Neyret 2008 section 4, Algorithm 4.1, on these small tables), with the ground lit by
    /// each order as well; only what is left after `Quality::orders` is closed as a geometric series, by the ratio of
    /// the last two orders. Against the path-traced medium (the test `against_path_tracing`) the sky holds within 3 %
    /// down to a sun below the horizon, where the one-factor series reads the further orders a third low: there the
    /// light comes down from the air above, which the sun still reaches.
    fn build_at(a: &Atmosphere, quality: Quality) -> Luts {
        let sun = Table::fill(SUN_TABLE.0, SUN_TABLE.1, At::mu_of_sun, |mu_sun, r| a.optical_depth(r, mu_sun));
        let empty = |rows: usize| Table { columns: SCATTER_TABLE.0, rows, data: vec![[0.0; 3]; rows * SCATTER_TABLE.0] };
        let mut luts = Luts { sun, scattered: empty(SCATTER_TABLE.1), ground: empty(1), view_steps: quality.view_steps };
        // Order one as a source: the sun's light at a point, scattered isotropically; the ground lit by the sun.
        let isotropic = 1.0 / (4.0 * std::f64::consts::PI);
        let first = |at: At| luts.sun_transmittance(at).map(|t| t * isotropic);
        let first_ground = |mu_sun: f64| luts.sunlit_ground(a, mu_sun);
        let (mut order, mut order_ground) = (luts.gather(a, quality, first, first_ground), luts.gather_ground(a, quality, first));
        let (mut sum, mut sum_ground) = (order.data.clone(), order_ground.data.clone());
        for n in 3..=quality.orders.max(2) + 1 {
            let source = |at: At| order.at(at.scatter, at.altitude);
            let (next, next_ground) = (luts.gather(a, quality, source, |mu_sun| order_ground.at(At::ground(mu_sun).scatter, 0.0)), luts.gather_ground(a, quality, source));
            // The last one computed stands for all the orders after it: its share of the one before, as a series.
            let last = n == quality.orders.max(2) + 1;
            let add = |sum: &mut Vec<[f32; 3]>, before: &[[f32; 3]], next: &[[f32; 3]]| {
                for (i, s) in sum.iter_mut().enumerate() {
                    for c in 0..3 {
                        let ratio = if before[i][c] > 0.0 { (next[i][c] / before[i][c]).clamp(0.0, 0.9) } else { 0.0 };
                        s[c] += if last { next[i][c] / (1.0 - ratio) } else { next[i][c] };
                    }
                }
            };
            add(&mut sum, &order.data, &next.data);
            add(&mut sum_ground, &order_ground.data, &next_ground.data);
            (order, order_ground) = (next, next_ground);
        }
        luts.scattered.data = sum;
        luts.ground.data = sum_ground;
        luts
    }

    /// The transmittance from a point to the top of the atmosphere toward the sun.
    #[inline(always)]
    fn sun_transmittance(&self, at: At) -> [f64; 3] {
        let t = self.sun.at(at.sun, at.altitude);
        let e = exp4([-t[0], -t[1], -t[2], 0.0]);
        // (`exp4` has no zero: the opaque is.)
        [0, 1, 2].map(|c| if t[c] < OPAQUE - 1.0 { e[c] } else { 0.0 })
    }

    /// The radiance of the ground lit by the sun through the atmosphere, per unit of the sun's illuminance: a
    /// Lambertian surface (the L_o of Hillaire 2020 equation 6).
    fn sunlit_ground(&self, a: &Atmosphere, mu_sun: f64) -> [f64; 3] {
        let t = self.sun_transmittance(At::ground(mu_sun));
        [0, 1, 2].map(|c| a.ground_albedo[c] / std::f64::consts::PI * mu_sun.max(0.0) * t[c])
    }

    /// Nodes on the sphere of directions about the zenith at radius `r`, by the sun's side (the other half mirrors
    /// it), with weights that sum to 1 over the sphere: 8-point Gauss-Legendre in the zenith cosine on each side of
    /// the horizon (below it a ray ends on the ground: the radiance jumps there) x midpoints in azimuth. `up`: the
    /// part above the level direction only (for the ground: weights that sum to 1 over the hemisphere).
    fn directions(r: f64, up: bool, azimuths: usize) -> impl Iterator<Item = (f64, f64, f64)> {
        let horizon = -(1.0 - (R_GROUND / r).powi(2)).max(0.0).sqrt();
        let panels = if up { [(0.0, 1.0), (0.0, 0.0)] } else { [(-1.0, horizon), (horizon, 1.0)] };
        let whole = if up { 1.0 } else { 2.0 };
        panels.into_iter().filter(|(lo, hi)| hi > lo).flat_map(move |(lo, hi)| {
            let (centre, half) = (0.5 * (lo + hi), 0.5 * (hi - lo));
            GL8.into_iter().flat_map(move |(x, w)| [-1.0, 1.0].into_iter().flat_map(move |sign| (0..azimuths).map(move |k| (centre + sign * half * x, (k as f64 + 0.5) / azimuths as f64 * std::f64::consts::PI, w * half / whole / azimuths as f64))))
        })
    }

    /// The next order's source from this one's (`source`: the radiance a point scatters on per unit of its scattering
    /// coefficient; `ground`: the ground's radiance, by the sun's zenith cosine): at every entry of the table, the
    /// radiance that arrives from all directions, averaged (the isotropic phase function).
    fn gather(&self, a: &Atmosphere, quality: Quality, source: impl Fn(At) -> [f64; 3] + Sync, ground: impl Fn(f64) -> [f64; 3] + Sync) -> Table {
        Table::fill(SCATTER_TABLE.0, SCATTER_TABLE.1, At::mu_of_scatter, |mu_sun, r| {
            if mu_sun < NIGHT {
                return [0.0; 3];
            }
            let r = r.clamp(R_GROUND + 1e-3, R_TOP - 1e-3);
            let sin_sun = (1.0 - mu_sun * mu_sun).max(0.0).sqrt();
            let mut sum = [0.0f64; 3];
            for (mu, phi, weight) in Self::directions(r, false, quality.azimuths) {
                let nu = (1.0 - mu * mu).max(0.0).sqrt() * phi.cos() * sin_sun + mu * mu_sun;
                let (ray, lands) = Ray::new(r, mu, mu_sun, nu);
                let (l, through) = Self::march(a, ray, quality.steps, |at, air, haze| {
                    let s = source(at);
                    [0, 1, 2].map(|c| (a.rayleigh[c] * air + a.mie_scattering * haze) * s[c])
                });
                let below = if lands { ground(ray.mu_sun_at_ground()) } else { [0.0; 3] };
                for c in 0..3 {
                    sum[c] += weight * (l[c] + through[c] * below[c]);
                }
            }
            sum
        })
    }

    /// The ground's radiance from the light of `source`: its albedo / pi times the irradiance from the hemisphere.
    fn gather_ground(&self, a: &Atmosphere, quality: Quality, source: impl Fn(At) -> [f64; 3] + Sync) -> Table {
        Table::fill(SCATTER_TABLE.0, 1, At::mu_of_scatter, |mu_sun, _| {
            if mu_sun < NIGHT {
                return [0.0; 3];
            }
            let sin_sun = (1.0 - mu_sun * mu_sun).max(0.0).sqrt();
            let mut sum = [0.0f64; 3];
            for (mu, phi, weight) in Self::directions(R_GROUND, true, quality.azimuths) {
                let nu = (1.0 - mu * mu).max(0.0).sqrt() * phi.cos() * sin_sun + mu * mu_sun;
                let (ray, _) = Ray::new(R_GROUND + 1e-3, mu, mu_sun, nu);
                let (l, _) = Self::march(a, ray, quality.steps, |at, air, haze| {
                    let s = source(at);
                    [0, 1, 2].map(|c| (a.rayleigh[c] * air + a.mie_scattering * haze) * s[c])
                });
                // (The weights are of a mean over the hemisphere, 2 pi sr: the irradiance is 2 pi x mean(L mu).)
                for c in 0..3 {
                    sum[c] += weight * 2.0 * mu * l[c] * a.ground_albedo[c];
                }
            }
            sum
        })
    }

    /// The radiance that arrives at the start of `ray` from the air along it, and the ray's transmittance: the
    /// integral of the view's transmittance times `emitted` (the radiance the air sends along the ray per km, given
    /// where a point is in the tables and the air's and the haze's densities there; Hillaire 2020 equations 1-3 and
    /// 11), by Simpson's rule on `steps` intervals of u, the distance being length u^2: the nodes crowd the ray's
    /// start, where a view ray from the ground is in the dense air and what it gathers is least dimmed. The view's
    /// optical depth at the nodes is the running Simpson integral of the extinction (at the odd nodes, the
    /// three-point rule for half a panel: h (5 f0 + 8 f1 - f2) / 12).
    #[inline(always)]
    fn march(a: &Atmosphere, ray: Ray, steps: usize, emitted: impl Fn(At, f64, f64) -> [f64; 3]) -> ([f64; 3], [f64; 3]) {
        let steps = steps.max(2) & !1;
        let h = 1.0 / steps as f64;
        // A node: where it is in the tables, the air's and the haze's densities, the extinction's integrand (per unit
        // u), the distance's derivative (dt / du).
        let node = |i: usize| {
            let u = i as f64 * h;
            let t = ray.length * u * u;
            let r = (ray.r * ray.r + t * t + 2.0 * ray.r * ray.mu * t).sqrt();
            let altitude = (r - R_GROUND).max(0.0);
            let e = exp4([-altitude * (1.0 / H_RAYLEIGH), -altitude * (1.0 / H_MIE), 0.0, 0.0]);
            let ozone = (1.0 - (altitude - OZONE_CENTRE).abs() * (1.0 / OZONE_HALF_WIDTH)).max(0.0);
            let jacobian = 2.0 * ray.length * u;
            (At::new(((ray.r * ray.mu_sun + t * ray.nu) / r).clamp(-1.0, 1.0), r), e[0], e[1], a.extinction([e[0], e[1], ozone]).map(|x| x * jacobian), jacobian)
        };
        let (mut out, mut depth) = ([0.0f64; 3], [0.0f64; 3]);
        let mut add = |n: &(At, f64, f64, [f64; 3], f64), tau: [f64; 3], simpson: f64| {
            let (e, view) = (emitted(n.0, n.1, n.2), exp4([-tau[0], -tau[1], -tau[2], 0.0]));
            for c in 0..3 {
                out[c] += simpson * h / 3.0 * n.4 * view[c] * e[c];
            }
        };
        // (The first node has no length: dt / du = 0 there.)
        let mut n0 = node(0);
        for k in (0..steps).step_by(2) {
            let (n1, n2) = (node(k + 1), node(k + 2));
            let middle = [0, 1, 2].map(|c| depth[c] + h / 12.0 * (5.0 * n0.3[c] + 8.0 * n1.3[c] - n2.3[c]));
            depth = [0, 1, 2].map(|c| depth[c] + h / 3.0 * (n0.3[c] + 4.0 * n1.3[c] + n2.3[c]));
            add(&n1, middle, 4.0);
            add(&n2, depth, if k + 2 == steps { 1.0 } else { 2.0 });
            n0 = n2;
        }
        let through = exp4([-depth[0], -depth[1], -depth[2], 0.0]);
        (out, [through[0], through[1], through[2]])
    }

    /// The sky's radiance per unit of the sun's illuminance above the atmosphere (sr^-1, RGB), seen from
    /// `altitude_km` above the ground in direction `dir`, the sun in direction `sun` (both unit, planet frame: Z up);
    /// without the sun's disk, with multiple scattering and the ground's bounce.
    /// A viewer above the atmosphere's top sees the air from where the ray enters it, and black space where it does
    /// not.
    fn sky(&self, a: &Atmosphere, altitude_km: f32, dir: Vec3, sun: Vec3) -> Vec3 {
        let r0 = R_GROUND + altitude_km.max(0.0) as f64;
        let (mu0, mu_sun0) = ((dir.z as f64).clamp(-1.0, 1.0), (sun.z as f64).clamp(-1.0, 1.0));
        let nu = (dir.dot(sun) as f64).clamp(-1.0, 1.0);
        let top = R_TOP - 1e-3;
        let (r, mu, mu_sun) = if r0 <= top {
            (r0, mu0, mu_sun0)
        } else {
            // The near crossing of the top's sphere; the ray's zenith cosines (its own, the sun's) there.
            let disc = r0 * r0 * (mu0 * mu0 - 1.0) + top * top;
            if mu0 >= 0.0 || disc <= 0.0 {
                return Vec3::ZERO;
            }
            let t = -r0 * mu0 - disc.sqrt();
            let r = (r0 * r0 + t * t + 2.0 * r0 * mu0 * t).sqrt();
            (r, ((r0 * mu0 + t) / r).clamp(-1.0, 1.0), ((r0 * mu_sun0 + t * nu) / r).clamp(-1.0, 1.0))
        };
        let (ray, lands) = Ray::new(r, mu, mu_sun, nu);
        let (by_air, by_haze) = (Atmosphere::phase_rayleigh(nu), a.phase_mie(nu) * a.mie_scattering);
        // Equation 11 of Hillaire 2020: at each point the sun's light scattered once toward the viewer, by the air's
        // and the haze's phase functions, and the light of the further orders.
        let (l, through) = Self::march(a, ray, self.view_steps, |at, air, haze| {
            let (sun, further) = (self.sun_transmittance(at), self.scattered.at(at.scatter, at.altitude));
            [0, 1, 2].map(|c| {
                let air = a.rayleigh[c] * air;
                (air * by_air + haze * by_haze) * sun[c] + (air + haze * a.mie_scattering) * further[c]
            })
        });
        // A ray that ends on the ground sees it lit by the sun and by the sky.
        let below = if lands {
            let mu_sun = ray.mu_sun_at_ground();
            let (direct, scattered) = (self.sunlit_ground(a, mu_sun), self.ground.at(At::ground(mu_sun).scatter, 0.0));
            [0, 1, 2].map(|c| direct[c] + scattered[c])
        } else {
            [0.0; 3]
        };
        Vec3::from([0, 1, 2].map(|c| (l[c] + through[c] * below[c]) as f32))
    }
}

pub struct SkyBake {
    pub width: u32,
    pub height: u32,
    /// Linear radiance (nits) per texel, RGBA f32, equirect: u = atan2(x, -z)/2pi + 0.5, v = acos(y)/pi.
    pub texels: Vec<[f32; 4]>,
    /// Sun illuminance reaching the ground (lux, RGB), i.e. E_sun * T(ground -> sun).
    pub sun_illuminance: Vec3,
    /// A planet's bake (`Horizon::Planet`): the texture's v at the centre of the row the dipped horizon crosses, the
    /// limb's row, where the aerial perspective reads the air's light (pathtrace.wgsl `aerial_light`); 0 otherwise.
    pub horizon_v: f32,
}

/// Where the bake's sky ends below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Horizon {
    /// A flat world's (a scene's ground plane): the level direction, faded as UE's sky light cube fades it.
    Level,
    /// A planet's (`SkyAtmosphere.altitude`: the scene's ground is the globe): the horizon dips with the viewer's
    /// altitude (19.8 degrees at 400 km), black below it, the texel row it crosses covered in proportion (its alpha:
    /// the share, so that the limb's own radiance can be read back, `SkyBake::horizon_v`).
    Planet,
}

/// The sky's size when its sun moves (`desc::Sky::Atmosphere::dynamic`): a bake takes ~1.5 ms of all cores here (~8 ms
/// of the live view's three) against ~20 ms at the static 1024 x 512, so the live view can re-bake it on a worker
/// thread many times a second.
/// The sun's disk is not in the bake (the kernel draws directional lights' disks), so the low size only softens the
/// sky's own gradients, which are broad.
pub const DYNAMIC_SKY: (u32, u32) = (256, 128);

/// UE's SkyLight CubemapResolution default (SkyLightComponent.cpp): the sky light holds the atmosphere at 128 texels per
/// cube face.
const SKY_LIGHT_CUBE: f32 = 128.0;

/// Bake sky radiance for a viewer `altitude_m` above ground. `sun_dir` points toward the sun (Y-up).
/// As UE's sky light gives it: a cubemap of the atmosphere with the lower hemisphere black, sampled bilinearly, so the
/// sky fades to black across one texel row of the side faces at the horizon (0.9 deg at a face's centre, 0.63 at its
/// edges). UE's path tracer shows that fade below every horizon (Khronos layer B: the band beyond a 2 km ground plane).
pub fn bake_atmosphere(a: &Atmosphere, sun_dir: Vec3, sun_lux: Vec3, altitude_m: f32, width: u32, height: u32) -> SkyBake {
    bake_atmosphere_with(a, &Luts::build(a), sun_dir, sun_lux, altitude_m, width, height, Glow::default(), 0.0, Horizon::Level)
}

/// The atmosphere's transmittance and multiple-scattering tables: they depend on the atmosphere alone, so a sky whose
/// sun moves (`bake_atmosphere_with`) builds them once.
pub struct SkyTables(Luts);

impl SkyTables {
    pub fn new(a: &Atmosphere) -> SkyTables {
        SkyTables(Luts::build(a))
    }
}

/// The night sky's glow (light pollution), which the atmosphere alone does not give once the sun has set: radiance
/// (nits) added above the horizon, `uniform` everywhere and `horizon` at the horizon, falling off with elevation (1/e at
/// `height` radians): a city's light dome.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Glow {
    pub uniform: Vec3,
    pub horizon: Vec3,
    pub height: f32,
}

/// The glow of an atmosphere sky's description (`desc::Sky::Atmosphere` `glow`, `horizon_glow`).
pub fn glow_of(uniform: &Option<[f32; 3]>, horizon: &Option<[f32; 4]>) -> Glow {
    Glow { uniform: uniform.map(Vec3::from).unwrap_or(Vec3::ZERO), horizon: horizon.map_or(Vec3::ZERO, |h| Vec3::new(h[0], h[1], h[2])), height: horizon.map_or(0.0, |h| h[3]) }
}

impl Glow {
    /// The glow at `elevation` radians (the horizon's below it).
    pub fn at(&self, elevation: f32) -> Vec3 {
        if self.horizon == Vec3::ZERO {
            return self.uniform;
        }
        self.uniform + self.horizon * (-elevation.max(0.0) / self.height.max(1e-3)).exp()
    }
    pub fn is_zero(&self) -> bool {
        self.uniform == Vec3::ZERO && self.horizon == Vec3::ZERO
    }
}

/// `bake_atmosphere` with tables built once (`SkyTables`), plus the night sky's `glow` above the horizon, a cloud
/// `cover` (0..1, `overcast`) and where the sky ends below (`Horizon`).
#[allow(clippy::too_many_arguments)]
pub fn bake_atmosphere_tables(a: &Atmosphere, t: &SkyTables, sun_dir: Vec3, sun_lux: Vec3, altitude_m: f32, width: u32, height: u32, glow: Glow, cover: f32, horizon: Horizon) -> SkyBake {
    bake_atmosphere_with(a, &t.0, sun_dir, sun_lux, altitude_m, width, height, glow, cover, horizon)
}

/// The sun's illuminance at the ground (lux, RGB) for a sun of `sun_lux` above the atmosphere under a cloud `cover`,
/// as the bake gives it: what a directional sun light carries when it moves without a new bake.
pub fn sun_illuminance(a: &Atmosphere, sun_dir: Vec3, sun_lux: Vec3, cover: f32) -> Vec3 {
    sun_lux * a.sun_transmittance(sun_dir) * cloud_transmittance(cover)
}

/// The sun's direct light through a cloud deck of `cover` (0..1): the share of time it is not behind a cloud, as a mean
/// (a thick deck passes no direct light).
pub fn cloud_transmittance(cover: f32) -> f32 {
    1.0 - cover.clamp(0.0, 1.0)
}

/// An overcast deck's diffuse transmittance: the ground's illuminance under full cover against what the clear sky and
/// sun give it (stratus / nimbostratus pass 0.1-0.4 of it; a mid value).
pub const OVERCAST_TRANSMITTANCE: f32 = 0.3;

/// A city's light dome off a cloud base: clouds scatter the upward light back, overcast city skies read several times
/// as bright as clear ones at night (Kyba et al. 2011, Berlin: up to ~10x at the zenith; a low value).
pub const OVERCAST_GLOW: f32 = 3.0;

/// The CIE standard overcast sky (ISO 15469 type 1): radiance at elevation `e` (radians) for a horizontal illuminance
/// `e_h`: L(e) = L_z (1 + 2 sin e) / 3, with E_h = 7 pi L_z / 9.
pub fn cie_overcast(e_h: Vec3, e: f32) -> Vec3 {
    e_h * (9.0 / (7.0 * PI)) * ((1.0 + 2.0 * e.max(0.0).sin()) / 3.0)
}

/// With a cloud `cover`: the atmosphere's sky (and glow) blended with an overcast dome (`cie_overcast`) whose
/// illuminance is OVERCAST_TRANSMITTANCE of what the clear sky and sun give the ground (their luminance, white), the
/// glow at OVERCAST_GLOW x.
#[allow(clippy::too_many_arguments)]
fn bake_atmosphere_with(a: &Atmosphere, luts: &Luts, sun_dir: Vec3, sun_lux: Vec3, altitude_m: f32, width: u32, height: u32, glow: Glow, cover: f32, horizon: Horizon) -> SkyBake {
    // Planet frame: Z up. Map Y-up world -> planet frame (x, z, y).
    let to_planet = |d: Vec3| Vec3::new(d.x, d.z, d.y);
    let sun_p = to_planet(sun_dir).normalize();
    let altitude_km = (altitude_m / 1000.0).max(0.005);
    let row = PI / height as f32;
    let dip = horizon_elevation(altitude_km);
    let texels: Vec<[f32; 4]> = (0..(width * height) as usize)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i as u32 % width) as f32 + 0.5, (i as u32 / width) as f32 + 0.5);
            let theta = y / height as f32 * PI;
            let phi = (x / width as f32 - 0.5) * 2.0 * PI;
            let d = Vec3::new(theta.sin() * phi.sin(), theta.cos(), -theta.sin() * phi.cos());
            if horizon == Horizon::Planet {
                // The share of the texel's row above the dipped horizon; the sky at the row's centre, or just above
                // the horizon where that is below it (the limb: the air seen edge on, its brightest).
                let k = ((PI / 2.0 - theta + 0.5 * row - dip) / row).clamp(0.0, 1.0);
                if k <= 0.0 {
                    return [0.0, 0.0, 0.0, 0.0];
                }
                let e = (PI / 2.0 - theta).max(dip + 1e-4);
                let flat = Vec3::new(phi.sin(), 0.0, -phi.cos());
                let dir = flat * e.cos() + Vec3::Y * e.sin();
                let l = luts.sky(a, altitude_km, to_planet(dir), sun_p);
                let l = (l * sun_lux + glow.at(e)) * k;
                return [l.x, l.y, l.z, k];
            }
            // Texel rows above the horizon on the side face this direction falls on; within half a row of it the
            // value blends the first row's centre with the black row below.
            let side = d.x.abs().max(d.z.abs()).max(1e-6);
            let r = d.y / side * (0.5 * SKY_LIGHT_CUBE);
            if r <= -0.5 {
                return [0.0, 0.0, 0.0, 0.0];
            }
            let (dir, k) = if r < 0.5 { (Vec3::new(d.x, side / SKY_LIGHT_CUBE, d.z).normalize(), r + 0.5) } else { (d, 1.0) };
            let l = luts.sky(a, altitude_km, to_planet(dir), sun_p);
            let l = (l * sun_lux + glow.at(dir.y.clamp(-1.0, 1.0).asin())) * k;
            // (w: the horizon fade's factor until the overcast blend below; 1 in the bake.)
            [l.x, l.y, l.z, k]
        })
        .collect();
    let mut texels = texels;
    let cover = cover.clamp(0.0, 1.0);
    if cover > 0.0 {
        // What the clear sky and the sun give a horizontal surface (texel solid angles of the equirectangular map).
        let (dt, dp) = (PI / height as f32, 2.0 * PI / width as f32);
        let e_sky: Vec3 = texels.iter().enumerate().map(|(i, t)| {
            let theta = ((i as u32 / width) as f32 + 0.5) * dt;
            Vec3::new(t[0], t[1], t[2]) * (theta.cos().max(0.0) * theta.sin() * dt * dp)
        }).sum();
        let e_sun = sun_illuminance(a, sun_dir, sun_lux, 0.0) * sun_dir.normalize().y.max(0.0);
        let lum = |v: Vec3| 0.2126 * v.x + 0.7152 * v.y + 0.0722 * v.z;
        // (The glow's share is the night's, scaled apart.)
        let e_glow: Vec3 = if glow.is_zero() { Vec3::ZERO } else {
            (0..height).map(|y| {
                let theta = (y as f32 + 0.5) * dt;
                glow.at((PI / 2.0 - theta).max(-PI / 2.0)) * (theta.cos().max(0.0) * theta.sin() * dt * dp * width as f32)
            }).sum()
        };
        let e_oc = Vec3::splat(lum(e_sky - e_glow + e_sun).max(0.0) * OVERCAST_TRANSMITTANCE);
        for (i, t) in texels.iter_mut().enumerate() {
            let e = PI / 2.0 - ((i as u32 / width) as f32 + 0.5) * dt;
            let oc = (cie_overcast(e_oc, e) + glow.at(e.max(-PI / 2.0)) * OVERCAST_GLOW) * t[3];
            let l = Vec3::new(t[0], t[1], t[2]).lerp(oc, cover);
            *t = [l.x, l.y, l.z, t[3]];
        }
    }
    // (A planet's limb row keeps its share in alpha, `Horizon::Planet`.)
    let planet = horizon == Horizon::Planet;
    for t in &mut texels {
        t[3] = if planet && t[3] > 0.0 { t[3] } else { 1.0 };
    }
    // (The row the horizon's zenith angle falls in; on a row's edge, the row above it, wholly sky.)
    let horizon_v = if planet { ((((PI / 2.0 - dip) / row).ceil() - 1.0).clamp(0.0, height as f32 - 1.0) + 0.5) / height as f32 } else { 0.0 };
    SkyBake { width, height, texels, sun_illuminance: sun_illuminance(a, sun_dir, sun_lux, cover), horizon_v }
}

// ------------------------------------------------------------------------------------------------ aerial perspective
//
// The kernel's aerial perspective on a planet (pathtrace.wgsl `aerial_planet`, F_AERIAL with an altitude): the air's
// columns along a segment over a sphere of radius R_GROUND, the viewer `altitude` above it and its centre straight
// below the camera. This is its reference, line by line in single precision, which the tests hold to a quadrature.
//
// The column of a density exp(-h / H) from a point at altitude h, radius r = R + h, along a ray of zenith cosine
// mu >= 0 to infinity is H exp(-h / H) Ch(x, mu), x = r / H, with Ch Chapman's grazing-incidence function (Chapman
// 1931, "The absorption and ionising effect of monochromatic radiation in an atmosphere on a rotating earth"). With
// the altitude along the ray to second order, h(s) = h + mu s + s^2 / (2 r), the integral is Gaussian:
// Ch(x, mu) = sqrt(pi x / 2) erfcx(mu sqrt(x / 2)), erfcx(y) = exp(y^2) erfc(y) (1 at mu = 0, sqrt(pi x / 2) there,
// and 1 / mu for a steep ray). erfcx is the bound of Abramowitz & Stegun 1964, 7.1.13, 2 / sqrt(pi) / (y +
// sqrt(y^2 + 4 / pi)), with a term a y under the root that makes its slope at 0 exact (a = 4 (4 / pi - 1) / sqrt(pi):
// a level ray's column then changes along it as the density, so that short level segments at the ground are right)
// and a factor 1 + b y^2 / (1 + c y^3) (own fit, b = 0.0295, c = 0.312): within 0.42 % for y >= 0, where the bound
// itself is 5.8 % off.
// A segment from a to b is a difference of such columns: both ends' columns onward when the ray climbs all along it,
// both ends' columns backward when it descends all along it, and when it passes its lowest point p between them,
// twice p's level column less the two ends' outward ones.

/// erfcx(y) = exp(y^2) erfc(y), y >= 0 (above).
fn erfcx_approx(y: f32) -> f32 {
    1.128_379_2 / (y + (y * y + 0.616_635_6 * y + 1.273_239_5).sqrt()) * (1.0 + 0.0295 * y * y / (1.0 + 0.312 * y * y * y))
}

/// The column (m at sea-level density) from altitude `h` m along zenith cosine `mu` (>= 0) to infinity, scale height
/// `scale` m.
fn aerial_column(h: f32, mu: f32, scale: f32) -> f32 {
    let x = (R_GROUND as f32 * 1000.0 + h) / scale;
    scale * (-h.max(-500.0) / scale).exp() * (0.5 * PI * x).sqrt() * erfcx_approx(mu.max(0.0) * (0.5 * x).sqrt())
}

/// A point's altitude and the zenith cosine of direction `d` there: `q` the point less the camera, `h0` the camera's
/// altitude (the planet's centre at (0, -(R + h0), 0) from the camera). Small terms first, so that the altitude keeps
/// its precision a planet's radius from the centre.
fn aerial_at(q: Vec3, d: Vec3, h0: f32) -> (f32, f32) {
    let r0 = R_GROUND as f32 * 1000.0;
    let up = q.y + h0;
    let s2 = q.x * q.x + q.z * q.z;
    let y = up + r0;
    let r = (y * y + s2).sqrt();
    (up + s2 / (r + y), (q.x * d.x + y * d.y + q.z * d.z) / r)
}

/// The air's and the haze's columns (m at sea-level density; scale heights 8 km and 1.2 km) from `q` (the point less
/// the camera) along `d` for `t` m, the camera `h0` m above the planet.
pub fn aerial_columns(q: Vec3, d: Vec3, t: f32, h0: f32) -> [f32; 2] {
    let (ha, ma) = aerial_at(q, d, h0);
    let (hb, mb) = aerial_at(q + d * t, d, h0);
    // How far along the ray its lowest point is.
    let r0 = R_GROUND as f32 * 1000.0;
    let lowest = -(q.x * d.x + (q.y + h0 + r0) * d.y + q.z * d.z);
    let (hp, _) = aerial_at(q + d * lowest.clamp(0.0, t), d, h0);
    [8000.0, 1200.0].map(|scale| {
        let od = if lowest <= 0.0 {
            aerial_column(ha, ma, scale) - aerial_column(hb, mb, scale)
        } else if lowest >= t {
            aerial_column(hb, -mb, scale) - aerial_column(ha, -ma, scale)
        } else {
            2.0 * aerial_column(hp, 0.0, scale) - aerial_column(ha, -ma, scale) - aerial_column(hb, mb, scale)
        };
        od.max(0.0)
    })
}

/// The elevation of a planet's horizon (radians, negative: it dips) for a viewer `altitude_km` above the ground's
/// sphere: -acos(R / (R + h)), as an arcsine that keeps its precision near the ground.
pub fn horizon_elevation(altitude_km: f32) -> f32 {
    let (r, h) = (R_GROUND, altitude_km.max(0.0) as f64);
    (-(h * (2.0 * r + h)).sqrt() / (r + h)).asin() as f32
}

/// Analytic dome for night / overcast scenes (UE session: gradient + light-pollution glow + clouds as a sky light).
/// All radiances are absolute (nits).
#[derive(Clone, Debug, PartialEq)]
pub struct Gradient {
    pub zenith: Vec3,
    pub horizon: Vec3,
    pub ground: Vec3,
    /// UE M_NightSky blend: lerp(zenith, horizon, (1 - max(sin e, 0))^exponent).
    pub horizon_exponent: Option<f32>,
    /// Horizon glow radiance, elevation falloff (radians to 1/e), or UE band (width, power).
    pub glow: Option<(Vec3, f32, Option<(f32, f32)>)>,
    /// Coverage 0..1, cloud-base radiance, feature scale, seed.
    pub clouds: Option<(f32, Vec3, f32, f32)>,
    pub cloud_map: Option<CloudMap>,
}

/// UE-style cloud layer from a tileable mask: uv = d.xz / (max(d.y, 0.02) + bias) * projection,
/// cover = smoothstep(threshold), sky = lerp(sky, sky * gain + offset, cover * saturate(sin e * fade)).
#[derive(Clone, Debug)]
pub struct CloudMap {
    pub key: String,
    pub image: std::sync::Arc<image::RgbaImage>,
    pub threshold: [f32; 2],
    pub gain: f32,
    pub offset: Vec3,
    pub projection: f32,
    pub bias: f32,
    pub fade: f32,
}

impl PartialEq for CloudMap {
    fn eq(&self, o: &Self) -> bool {
        self.key == o.key && self.threshold == o.threshold && self.gain == o.gain && self.offset == o.offset && self.projection == o.projection && self.bias == o.bias && self.fade == o.fade
    }
}

fn sample_r(img: &image::RgbaImage, u: f32, v: f32) -> f32 {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let (x, y) = (u * w - 0.5, v * h - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let at = |xx: f32, yy: f32| img.get_pixel(xx.rem_euclid(w) as u32, yy.rem_euclid(h) as u32)[0] as f32 / 255.0;
    let a = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
    let b = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
    a * (1.0 - fy) + b * fy
}

fn hash3(x: i32, y: i32, z: i32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6b343) ^ (y as u32).wrapping_mul(0xd8163841) ^ (z as u32).wrapping_mul(0xcb1ab31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1e995);
    h ^= h >> 15;
    (h & 0xffffff) as f32 / 16777216.0
}

fn value_noise(p: Vec3) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec3::splat(3.0) - 2.0 * f);
    let (x, y, z) = (i.x as i32, i.y as i32, i.z as i32);
    let mut acc = 0.0;
    for k in 0..8 {
        let (dx, dy, dz) = (k & 1, (k >> 1) & 1, (k >> 2) & 1);
        let w = (if dx == 1 { u.x } else { 1.0 - u.x }) * (if dy == 1 { u.y } else { 1.0 - u.y }) * (if dz == 1 { u.z } else { 1.0 - u.z });
        acc += w * hash3(x + dx, y + dy, z + dz);
    }
    acc
}

pub fn bake_gradient(g: &Gradient, width: u32, height: u32) -> SkyBake {
    let texels = (0..(width * height) as usize)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i as u32 % width) as f32 + 0.5, (i as u32 / width) as f32 + 0.5);
            let theta = y / height as f32 * PI;
            let phi = (x / width as f32 - 0.5) * 2.0 * PI;
            let d = Vec3::new(theta.sin() * phi.sin(), theta.cos(), -theta.sin() * phi.cos());
            if let Some(exp) = g.horizon_exponent {
                // UE M_NightSky, evaluated on the whole sphere.
                let h = d.y;
                let mut c = g.zenith.lerp(g.horizon, (1.0 - h.max(0.0)).clamp(0.0, 1.0).powf(exp));
                if let Some((gc, _, Some((width, power)))) = g.glow {
                    c += gc * (1.0 - h.abs() * width).clamp(0.0, 1.0).powf(power);
                }
                if let Some(cm) = &g.cloud_map {
                    let k = cm.projection / (h.max(0.02) + cm.bias);
                    let cl = sample_r(&cm.image, d.x * k, d.z * k);
                    let t = ((cl - cm.threshold[0]) / (cm.threshold[1] - cm.threshold[0])).clamp(0.0, 1.0);
                    let cover = t * t * (3.0 - 2.0 * t);
                    c = c.lerp(c * cm.gain + cm.offset, cover * (h * cm.fade).clamp(0.0, 1.0));
                }
                return [c.x, c.y, c.z, 1.0];
            }
            if d.y < 0.0 {
                // Below the horizon: ground radiance blending up into the horizon over a few degrees.
                let k = (-d.y / 0.08).min(1.0);
                let l = g.horizon.lerp(g.ground, k);
                return [l.x, l.y, l.z, 1.0];
            }
            let e = d.y.asin();
            let mut l = g.horizon.lerp(g.zenith, (e / (0.5 * PI)).powf(0.45));
            if let Some((c, h, _)) = g.glow {
                l += c * (-e / h.max(1e-3)).exp();
            }
            if let Some((cover, base, scale, seed)) = g.clouds {
                // Clouds on a flat layer: project onto y = 1 so features shrink toward the horizon.
                let q = Vec3::new(d.x / d.y.max(0.04), seed * 7.31, d.z / d.y.max(0.04)) * (1.6 / scale);
                let mut n = 0.0;
                let mut amp = 0.5;
                let mut f = 1.0;
                for o in 0..5 {
                    n += amp * value_noise(q * f + Vec3::splat(o as f32 * 17.0));
                    amp *= 0.5;
                    f *= 2.03;
                }
                let dens = ((n - (1.0 - cover)) / 0.18).clamp(0.0, 1.0);
                // Thin toward the horizon (haze takes over), brighter near it (more city glow underneath).
                let fade = (d.y / 0.06).min(1.0);
                let under = 1.0 + 0.8 * (-e / 0.35).exp();
                l = l.lerp(base * under, dens * fade * 0.9);
            }
            [l.x, l.y, l.z, 1.0]
        })
        .collect();
    SkyBake { width, height, texels, sun_illuminance: Vec3::ZERO, horizon_v: 0.0 }
}

#[cfg(test)]
mod overcast_tests {
    use super::*;

    #[test]
    fn full_cover_is_the_cie_overcast_sky_lit_by_the_clear_day() {
        let a = Atmosphere::earth(1.0, 10.0, 0.8, Vec3::splat(0.3));
        let t = SkyTables::new(&a);
        let (sun, lux) = (Vec3::new(0.3, 0.5, -0.8).normalize(), Vec3::splat(100_000.0));
        let (w, h) = (256u32, 128u32);
        let clear = bake_atmosphere_tables(&a, &t, sun, lux, 10.0, w, h, Glow::default(), 0.0, Horizon::Level);
        let oc = bake_atmosphere_tables(&a, &t, sun, lux, 10.0, w, h, Glow::default(), 1.0, Horizon::Level);
        let lum = |v: [f32; 4]| 0.2126 * v[0] + 0.7152 * v[1] + 0.0722 * v[2];
        // Horizontal illuminance of a bake (+ its sun).
        let e_h = |b: &SkyBake| -> f32 {
            let (dt, dp) = (PI / h as f32, 2.0 * PI / w as f32);
            let sky: f32 = b.texels.iter().enumerate().map(|(i, t)| {
                let th = ((i as u32 / w) as f32 + 0.5) * dt;
                lum(*t) * th.cos().max(0.0) * th.sin() * dt * dp
            }).sum();
            let s = b.sun_illuminance * sun.y;
            sky + 0.2126 * s.x + 0.7152 * s.y + 0.0722 * s.z
        };
        // No direct sun; the ground gets OVERCAST_TRANSMITTANCE of the clear day's light.
        assert_eq!(oc.sun_illuminance, Vec3::ZERO);
        let ratio = e_h(&oc) / e_h(&clear);
        assert!((ratio - OVERCAST_TRANSMITTANCE).abs() < 0.01, "{ratio}");
        // The dome's shape: (1 + 2 sin e) / 3, zenith over 10 degrees up, the same in every azimuth; white.
        let row = |e_deg: f32| (((90.0 - e_deg) / 180.0 * h as f32) as u32).min(h - 1);
        let at = |r: u32, x: u32| oc.texels[(r * w + x) as usize];
        let e_of = |r: u32| PI / 2.0 - (r as f32 + 0.5) * PI / h as f32;
        let (z, ten) = (row(89.0), row(10.0));
        let expect = (1.0 + 2.0 * e_of(z).sin()) / (1.0 + 2.0 * e_of(ten).sin());
        for x in [0, w / 3, 2 * w / 3] {
            let got = lum(at(z, x)) / lum(at(ten, x));
            assert!((got / expect - 1.0).abs() < 1e-3, "{got} {expect}");
            let c = at(ten, x);
            assert!((c[0] - c[2]).abs() < 1e-3 * c[1], "{c:?}");
        }
        // Half cover: halfway in every texel, the sun at half.
        let half = bake_atmosphere_tables(&a, &t, sun, lux, 10.0, w, h, Glow::default(), 0.5, Horizon::Level);
        assert!((half.sun_illuminance - clear.sun_illuminance * 0.5).length() < 1e-3 * clear.sun_illuminance.length());
        let k = (ten * w + 7) as usize;
        assert!((lum(half.texels[k]) - 0.5 * (lum(clear.texels[k]) + lum(oc.texels[k]))).abs() < 1e-3 * lum(clear.texels[k]));
    }
}

#[cfg(test)]
mod atmosphere_tests {
    use super::*;

    fn earth() -> Atmosphere {
        Atmosphere::earth(1.0, 1.0, 0.8, Vec3::splat(0.401978))
    }
    /// Toward a sun at an elevation (degrees) in the world's frame (Y up), and in the planet's (Z up).
    fn sun_at(elevation_deg: f32) -> Vec3 {
        let e = elevation_deg.to_radians();
        Vec3::new(e.cos(), e.sin(), 0.0)
    }
    fn planet(elevation_deg: f32, azimuth_deg: f32) -> Vec3 {
        let (e, a) = (elevation_deg.to_radians(), azimuth_deg.to_radians());
        Vec3::new(e.cos() * a.cos(), e.cos() * a.sin(), e.sin())
    }
    fn luminance(v: Vec3) -> f32 {
        0.2126 * v.x + 0.7152 * v.y + 0.0722 * v.z
    }

    /// From above the atmosphere (`SkyAtmosphere.altitude` up to low orbit): space is black, up and below the limb; the
    /// limb, the air seen edge on just above the dipped horizon, is lit and blue by day; and the sky does not jump as the
    /// viewer crosses the atmosphere's top.
    #[test]
    fn sky_from_space() {
        let (a, luts) = (earth(), Luts::build(&earth()));
        let sun = planet(40.0, 0.0);
        let dip = horizon_elevation(400.0).to_degrees();
        assert!((dip + 19.83).abs() < 0.05, "{dip}");
        // The top's tangent from 400 km: acos(6460 / 6760) = 17.1 degrees down.
        let top = -(R_TOP / (R_GROUND + 400.0)).acos().to_degrees() as f32;
        for az in [0.0, 90.0, 180.0] {
            assert_eq!(luts.sky(&a, 400.0, planet(90.0, az), sun), Vec3::ZERO);
            assert_eq!(luts.sky(&a, 400.0, planet(top + 0.2, az), sun), Vec3::ZERO);
            // Below the horizon the ray ends on the globe (the bake blacks it out: `Horizon::Planet`).
            let limb = luts.sky(&a, 400.0, planet(dip + 0.1, az), sun);
            assert!(luminance(limb) > 0.01 && limb.z > limb.x, "{az}: {limb:?}");
            // Thinner higher up the limb.
            let high = luts.sky(&a, 400.0, planet(dip + 1.5, az), sun);
            assert!(luminance(high) < 0.5 * luminance(limb), "{az}: {high:?} {limb:?}");
        }
        // Across the top: 99.9 and 100.1 km see the same sky (within the change 200 m of height makes).
        for (e, az) in [(90.0, 0.0), (10.0, 0.0), (-5.0, 180.0), (-10.2, 90.0)] {
            let (below, above) = (luts.sky(&a, 99.9, planet(e, az), sun), luts.sky(&a, 100.1, planet(e, az), sun));
            let tol = 0.03 * luminance(below).max(luminance(above)) + 1e-6;
            assert!((luminance(below) - luminance(above)).abs() <= tol, "{e} {az}: {below:?} {above:?}");
        }
    }

    /// A planet's bake: black below the dipped horizon and in space above the limb, the limb's row covered in
    /// proportion with its share in alpha (`SkyBake::horizon_v`), so that the air's light reads back whole.
    #[test]
    fn planet_bake_dips_its_horizon() {
        let a = earth();
        let t = SkyTables::new(&a);
        let (w, h) = DYNAMIC_SKY;
        for altitude_km in [0.002f32, 10.0, 400.0, 2000.0] {
            let b = bake_atmosphere_tables(&a, &t, sun_at(30.0), Vec3::splat(120_000.0), altitude_km * 1000.0, w, h, Glow::default(), 0.0, Horizon::Planet);
            let dip = horizon_elevation(altitude_km.max(0.005));
            let row_of = |v: f32| (v * h as f32).floor() as u32;
            let limb = row_of(b.horizon_v);
            let elevation = |y: u32| PI / 2.0 - (y as f32 + 0.5) / h as f32 * PI;
            assert!(elevation(limb) - 0.5 * PI / h as f32 <= dip + 1e-5 && dip < elevation(limb) + 0.5 * PI / h as f32 + 1e-5, "{altitude_km}: row {limb}");
            for y in 0..h {
                let row = &b.texels[(y * w) as usize..((y + 1) * w) as usize];
                let lum: f32 = row.iter().map(|t| 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]).sum::<f32>() / w as f32;
                if y > limb {
                    assert_eq!(lum, 0.0, "{altitude_km}: row {y} below the horizon");
                } else if y == limb {
                    assert!(lum > 0.0 && row.iter().all(|t| t[3] > 0.0 && t[3] <= 1.0), "{altitude_km}: the limb's row");
                } else {
                    assert!(row.iter().all(|t| t[3] == 1.0));
                }
                // Above the top's tangent from orbit: space.
                if altitude_km > 100.0 && elevation(y) > -(R_TOP / (R_GROUND + altitude_km as f64)).acos() as f32 + 0.03 {
                    assert_eq!(lum, 0.0, "{altitude_km}: row {y} in space");
                }
            }
            // Read back whole: the limb row's light over its share is the sky just above the horizon.
            let i = (limb * w + w / 2) as usize;
            let back = Vec3::new(b.texels[i][0], b.texels[i][1], b.texels[i][2]) / b.texels[i][3];
            assert!(back.z > 0.0, "{altitude_km}: {back:?}");
        }
        // The flat world's bake is as it was: alpha 1, black from just under the level horizon.
        let flat = bake_atmosphere_tables(&a, &t, sun_at(30.0), Vec3::ONE, 400_000.0, w, h, Glow::default(), 0.0, Horizon::Level);
        assert!(flat.horizon_v == 0.0 && flat.texels.iter().all(|t| t[3] == 1.0));
    }

    /// The kernel's haze on a planet (pathtrace.wgsl `aerial_light`): (1 - T) of the limb's radiance in the ray's
    /// azimuth (the bake's limb row), against the medium's own light scattered toward a viewer 10-400 km up along rays
    /// that end on the ground (the sky's march over a black ground): within 0.5-1.8 x (in luminance): over where the limb is
    /// toward the sun (the haze's forward peak) and the ray steep, under for rays that graze the globe near its horizon from orbit.
    #[test]
    fn planet_haze_is_the_airs_light() {
        let a = Atmosphere::earth(1.0, 1.0, 0.8, Vec3::ZERO);
        let luts = Luts::build(&a);
        let sun = planet(30.0, 0.0);
        for h in [10.0f32, 100.0, 400.0] {
            let dip = horizon_elevation(h).to_degrees();
            for az in [0.0f32, 90.0, 180.0] {
                let limb = luts.sky(&a, h, planet(dip + 0.05, az), sun);
                for e in [-90.0f32, -60.0, -30.0, dip - 2.0] {
                    let d = planet(e, az);
                    let air = luts.sky(&a, h, d, sun);
                    let r = (R_GROUND + h as f64).min(R_TOP - 1e-3);
                    let (_, t) = Luts::march(&a, Ray::new(r, d.z as f64, sun.z as f64, d.dot(sun) as f64).0, 64, |_, _, _| [0.0; 3]);
                    let haze = Vec3::new(1.0 - t[0] as f32, 1.0 - t[1] as f32, 1.0 - t[2] as f32) * limb;
                    let ratio = luminance(haze) / luminance(air);
                    assert!((0.5..1.8).contains(&ratio), "{h} km, azimuth {az}, elevation {e}: {haze:?} against {air:?}");
                }
            }
        }
    }

    /// The air's columns along a path over the planet (`aerial_columns`, the kernel's `aerial_planet`) against a
    /// quadrature of the densities over the exact sphere: from the street to 2,000 km up, segments of 10 m to the
    /// globe's far side, climbing, descending and passing their lowest point.
    #[test]
    fn aerial_columns_over_the_planet() {
        let r = R_GROUND * 1000.0;
        let exact = |h0: f64, q: glam::DVec3, d: glam::DVec3, t: f64| -> [f64; 2] {
            let c = glam::DVec3::new(0.0, -(r + h0), 0.0);
            let n = 200_000;
            [8000.0, 1200.0].map(|scale| {
                let f = |s: f64| (-(((q + d * s) - c).length() - r).max(-500.0) / scale).exp();
                let hstep = t / n as f64;
                let mut sum = f(0.0) + f(t);
                for i in 1..n {
                    sum += f(i as f64 * hstep) * if i % 2 == 1 { 4.0 } else { 2.0 };
                }
                sum * hstep / 3.0
            })
        };
        // Blue Rayleigh and Mie extinction at sea level (per m).
        let tr = |c: [f64; 2]| (-(33.1e-6 * c[0] + 4.44e-6 * c[1])).exp();
        let mut worst = 0.0f64;
        for h0 in [2.0f64, 300.0, 3000.0, 12_000.0, 100_000.0, 400_000.0, 2_000_000.0] {
            for elevation in [-90.0f64, -60.0, -30.0, -10.0, -3.0, -1.0, 0.0, 1.0, 5.0, 30.0] {
                let e = elevation.to_radians();
                let d = glam::DVec3::new(e.cos() * 0.6, e.sin(), -e.cos() * 0.8);
                for q in [glam::DVec3::ZERO, glam::DVec3::new(3000.0, -h0.min(2000.0) * 0.5, 1000.0)] {
                    // To the ground, or this far.
                    let c = glam::DVec3::new(0.0, -(r + h0), 0.0);
                    let oc = q - c;
                    let b = oc.dot(d);
                    let disc = b * b - (oc.length_squared() - r * r);
                    let ground = if disc > 0.0 && -b - disc.sqrt() > 0.0 { Some(-b - disc.sqrt()) } else { None };
                    for t in [10.0f64, 1000.0, 10_000.0, 100_000.0, 1_000_000.0, 3_000_000.0] {
                        let t = ground.map_or(t, |g| t.min(g));
                        let (want, got) = (exact(h0, q, d, t), aerial_columns(q.as_vec3(), d.as_vec3(), t as f32, h0 as f32));
                        let got = [got[0] as f64, got[1] as f64];
                        let err = (tr(want) - tr(got)).abs();
                        worst = worst.max(err);
                        assert!(err < 0.005, "h0 {h0} e {elevation} q {q} t {t}: exact {want:?} model {got:?} (transmittance {} / {})", tr(want), tr(got));
                    }
                }
            }
        }
        println!("aerial columns: transmittance within {worst:.4}");
        // From 400 km the air thins out: straight down, three quarters of the blue light comes through (the flat
        // model with the camera at y = 0 has the whole 400 km at sea-level density: none).
        let down = aerial_columns(Vec3::ZERO, Vec3::NEG_Y, 400_000.0, 400_000.0);
        assert!((0.7..0.8).contains(&tr([down[0] as f64, down[1] as f64])), "{down:?}");
    }

    #[test]
    fn fast_exponential() {
        let mut worst = (0.0f64, 0.0f64);
        for i in -8690..=200 {
            let x = i as f64 * 0.01;
            let e = exp4([x, x - 0.003, 0.0, -1e-9]);
            let error = (e[0] / x.exp() - 1.0).abs().max((e[1] / (x - 0.003).exp() - 1.0).abs());
            worst = (worst.0.max(error), if x.abs() < 2.0 { worst.1.max(error) } else { worst.1 });
            assert!(e[2] == 1.0 && (e[3] - 1.0).abs() < 1e-7);
        }
        // (Single precision in the argument: its last bit is 4e-6 of the result at -87.)
        assert!(worst.0 < 5e-6 && worst.1 < 3e-7, "{worst:?}");
        assert!(exp4([-1000.0, 0.0, 0.0, 0.0])[0] < 2e-38);
    }

    /// The quadrature of the columns against what they are in closed form (straight up: the air's and the haze's
    /// H (1 - exp(-100 km / H)), the ozone tent's area) and against a fine midpoint sum on slanted rays.
    #[test]
    fn columns_are_integrated_exactly() {
        let up = columns_exact(R_GROUND, 1.0);
        assert!((up[0] - 8.0 * (1.0 - (-100.0f64 / 8.0).exp())).abs() < 1e-6, "{up:?}");
        // (The ozone tent's corners are not at panel ends: 0.2 %.)
        assert!((up[1] - 1.2).abs() < 1e-6 && (up[2] - 15.0).abs() < 0.03, "{up:?}");
        for (h, mu) in [(0.0, 0.0), (0.0, 0.02), (0.0, 0.3), (3.0, 0.0), (20.0, 0.05), (60.0, 0.0), (-3.0, 0.0)] {
            let r = R_GROUND + h;
            let (d, n) = (distance_out(r, mu, R_TOP), 400_000);
            let mut sum = [0.0f64; 3];
            for i in 0..n {
                let t = (i as f64 + 0.5) / n as f64 * d;
                let dens = densities((r * r + t * t + 2.0 * r * mu * t).sqrt() - R_GROUND);
                for c in 0..3 {
                    sum[c] += dens[c] * d / n as f64;
                }
            }
            let fast = columns_exact(r, mu);
            for c in 0..3 {
                assert!((fast[c] - sum[c]).abs() < if c == 2 { 2e-3 } else { 2e-4 } * sum[c].max(1.0), "{h} {mu}: {fast:?} {sum:?}");
            }
        }
    }

    /// The table of the optical depth toward the sun against the quadrature it tabulates, as transmittance: every
    /// altitude and direction, the sun below the horizon included.
    #[test]
    fn sun_table_matches_the_quadrature() {
        let a = earth();
        let luts = Luts::build(&a);
        let mut worst = (0.0f64, 0.0, 0.0, 0);
        for i in 0..150 {
            let r = R_GROUND + 100.0 * (i as f64 / 150.0).powi(3);
            for k in 0..=500 {
                let mu = -0.25 + 1.25 * (k as f64 / 500.0).powi(2);
                let (exact, t) = (a.optical_depth(r, mu).map(|t| (-t).exp()), luts.sun_transmittance(At::new(mu, r)));
                for c in 0..3 {
                    if (exact[c] - t[c]).abs() > worst.0 {
                        worst = ((exact[c] - t[c]).abs(), r - R_GROUND, mu, c);
                    }
                }
            }
        }
        // (The worst is a ray that grazes the top of the ozone layer.)
        assert!(worst.0 < 6e-3, "{worst:?}");
    }

    /// The sun's light on the ground: redder and dimmer toward the horizon, never rising as the sun goes down,
    /// without a step anywhere (the horizon included), nothing from three degrees below it; the medium's own
    /// transmittance times the calibration's veil.
    #[test]
    fn sun_transmittance_is_smooth_and_reddens() {
        let a = earth();
        let zenith = a.sun_transmittance(sun_at(90.0));
        // Straight up the medium is 8 km of air, 1.2 km of haze and 15 km of ozone at their reference densities; the
        // veil is its column.
        for c in 0..3 {
            let medium = RAYLEIGH_SCATTERING[c] * 8.0 * (1.0 - (-12.5f64).exp()) + (MIE_SCATTERING + MIE_ABSORPTION) * 1.2 + OZONE_ABSORPTION[c] * 15.0;
            let veil = SUN_VEIL_COLUMN * (RAYLEIGH_SCATTERING[c] + MIE_SCATTERING + MIE_ABSORPTION);
            assert!((zenith[c] as f64 - (-(medium + veil)).exp()).abs() < 1e-4, "{zenith:?}");
            assert!((a.ground_transmittance(1.0)[c] - (-medium).exp()).abs() < 1e-4);
        }
        let mut last = zenith;
        for i in 0..=1900 {
            let elevation = 90.0 - i as f32 * 0.05;
            let t = a.sun_transmittance(sun_at(elevation));
            assert!(t.x >= t.y && t.y >= t.z && t.z >= 0.0, "{elevation}: {t:?}");
            assert!(t.cmple(last + 1e-6).all(), "{elevation}: {t:?} {last:?}");
            assert!((last - t).max_element() < 0.012, "a step at {elevation}: {t:?} {last:?}");
            if elevation < -3.0 {
                assert!(t.max_element() < 1e-6, "{elevation}: {t:?}");
            }
            last = t;
        }
        // The sun on the horizon: about a tenth of its red light, a hundredth of its green, none of its blue.
        let level = a.sun_transmittance(sun_at(0.0));
        assert!(level.x > 0.05 && level.x < 0.15 && level.y < 0.02 && level.z < 1e-3, "{level:?}");
        // The scene's scales: no air and no haze leaves the ozone and the haze's absorption, nearly clear.
        let bare = Atmosphere::earth(0.0, 0.0, 0.8, Vec3::ZERO).sun_transmittance(sun_at(45.0));
        assert!(bare.min_element() > 0.95 && bare.y < bare.z, "{bare:?}");
        let hazy = Atmosphere::earth(1.0, 10.0, 0.8, Vec3::ZERO).sun_transmittance(sun_at(45.0));
        assert!(hazy.x < a.sun_transmittance(sun_at(45.0)).x * 0.95, "{hazy:?}");
    }

    /// The three.js side has the same function (tools/three/sky.ts `sunTransmittance`) and the same table in its test
    /// (tools/three/test/sky.test.ts): sun elevation (degrees), rayleigh scale, mie scale, then R, G, B.
    #[test]
    fn sun_transmittance_is_the_same_on_both_sides() {
        const TABLE: [[f64; 6]; 12] = [
            [90.0, 1.0, 1.0, 0.924612, 0.84226, 0.716526],
            [45.0, 1.0, 1.0, 0.89537, 0.784979, 0.624876],
            [20.0, 1.0, 1.0, 0.799125, 0.611592, 0.384148],
            [10.0, 1.0, 1.0, 0.659276, 0.400327, 0.166401],
            [5.0, 1.0, 1.0, 0.488058, 0.206397, 0.042848],
            [2.0, 1.0, 1.0, 0.291951, 0.0684695, 0.00389411],
            [0.0, 1.0, 1.0, 0.0935232, 0.00763352, 3.24136e-05],
            [-1.0, 1.0, 1.0, 0.00821672, 0.000199967, 3.94459e-08],
            [-3.0, 1.0, 1.0, 0.0, 0.0, 0.0],
            [30.0, 2.0, 5.0, 0.700244, 0.50156, 0.250179],
            [3.0, 0.5, 20.0, 0.0428066, 0.0215879, 0.00795393],
            [15.0, 0.0, 0.0, 0.960424, 0.897494, 0.990766],
        ];
        for row in TABLE {
            let e = (row[0] as f32).to_radians();
            // (Any length of the direction, any azimuth.)
            let t = Atmosphere::earth(row[1] as f32, row[2] as f32, 0.8, Vec3::ZERO).sun_transmittance(Vec3::new(e.cos() * 0.6, e.sin(), e.cos() * 0.8) * 3.0);
            for c in 0..3 {
                assert!((t[c] as f64 - row[3 + c]).abs() <= 1e-3 * row[3 + c] + 1e-9, "{row:?}: {t:?}");
            }
        }
    }

    /// The sky of a clear day, by physics: no negative or runaway radiance; what the ground gets from the sky and
    /// the sun is less than what the sun brings to the top of the atmosphere, and most of it (a black ground, so that
    /// nothing is counted twice); bluer at the zenith than at the horizon; the same on both sides of the sun's
    /// meridian and for any azimuth of the sun; brighter around the sun (the haze's lobe).
    #[test]
    fn day_sky_is_physical() {
        let black = Atmosphere::earth(1.0, 1.0, 0.8, Vec3::ZERO);
        let luts = Luts::build(&black);
        for elevation in [75.0f32, 40.0, 15.0] {
            let sun = planet(elevation, 0.0);
            // Horizontal irradiance of the sky: a midpoint grid over the hemisphere.
            let (n, m) = (48, 96);
            let mut sky = Vec3::ZERO;
            for i in 0..n {
                let z = (i as f32 + 0.5) / n as f32;
                for k in 0..m {
                    let phi = (k as f32 + 0.5) / m as f32 * 2.0 * PI;
                    let h = (1.0 - z * z).sqrt();
                    let l = luts.sky(&black, 0.0, Vec3::new(h * phi.cos(), h * phi.sin(), z), sun);
                    assert!(l.min_element() >= 0.0 && l.max_element() < 5.0 && l.is_finite(), "{l:?}");
                    sky += l * z * (2.0 * PI / (n * m) as f32);
                }
            }
            let direct = Vec3::from(black.ground_transmittance(sun.z as f64).map(|t| t as f32)) * sun.z;
            let (total, top) = (sky + direct, sun.z);
            assert!(total.max_element() < top, "{elevation}: {total:?} of {top}");
            assert!(total.min_element() > 0.6 * top, "{elevation}: {total:?} of {top}");
            assert!(sky.z > sky.y && sky.y > sky.x, "{sky:?}");
        }
        let (a, sun) = (earth(), planet(60.0, 0.0));
        let luts = Luts::build(&a);
        let (zenith, horizon) = (luts.sky(&a, 0.0, planet(90.0, 0.0), sun), luts.sky(&a, 0.0, planet(3.0, 180.0), sun));
        assert!(zenith.z / zenith.x > 1.5 * horizon.z / horizon.x, "{zenith:?} {horizon:?}");
        assert!(luminance(horizon) > 2.0 * luminance(zenith), "{zenith:?} {horizon:?}");
        for (e, az) in [(10.0, 40.0), (45.0, 90.0), (80.0, 170.0), (2.0, 5.0)] {
            let (left, right) = (luts.sky(&a, 0.0, planet(e, az), sun), luts.sky(&a, 0.0, planet(e, -az), sun));
            assert!((left - right).abs().max_element() < 1e-5 * left.max_element(), "{left:?} {right:?}");
            let turned = luts.sky(&a, 0.0, planet(e, az + 77.0), planet(60.0, 77.0));
            assert!((left - turned).abs().max_element() < 1e-4 * left.max_element(), "{left:?} {turned:?}");
        }
        let (near, far) = (luts.sky(&a, 0.0, planet(60.0, 8.0), sun), luts.sky(&a, 0.0, planet(60.0, 120.0), sun));
        assert!(near.x > 3.0 * far.x, "{near:?} {far:?}");
        // From 10 km up there is less air overhead: a darker zenith.
        assert!(luminance(luts.sky(&a, 10.0, planet(90.0, 0.0), sun)) < 0.5 * luminance(zenith));
        // A view that ends on the ground sees it lit: near albedo / pi x the sun's and the sky's light on it.
        let down = luts.sky(&a, 1.0, planet(-60.0, 90.0), sun);
        assert!(down.y > 0.09 && down.y < 0.16, "{down:?}");
    }

    /// A low sun: the sky around it is red, the sky opposite and overhead stays blue, and the whole of it dims as the
    /// sun goes under, without going out at the horizon.
    #[test]
    fn low_sun_reddens_and_twilight_fades() {
        let a = earth();
        let luts = Luts::build(&a);
        let sunset = planet(2.0, 0.0);
        let (glow, overhead) = (luts.sky(&a, 0.0, planet(3.0, 0.0), sunset), luts.sky(&a, 0.0, planet(90.0, 0.0), sunset));
        assert!(glow.x > 2.0 * glow.y && glow.y > 2.0 * glow.z, "{glow:?}");
        assert!(overhead.z > 1.5 * overhead.x, "{overhead:?}");
        let mut last = f32::MAX;
        for elevation in [30.0f32, 10.0, 2.0, 0.0, -2.0, -4.0, -6.0, -9.0, -12.0] {
            let l = luminance(luts.sky(&a, 0.0, planet(60.0, 90.0), planet(elevation, 0.0)));
            assert!(l < last && l >= 0.0, "{elevation}: {l} {last}");
            // (Civil twilight: the sky still has a hundredth of its light of ten minutes before sunset.)
            assert!(elevation < -5.0 || l > 5e-5, "{elevation}: {l}");
            last = l;
        }
        assert!(last < 2e-6, "{last}");
        assert_eq!(luts.sky(&a, 0.0, planet(60.0, 90.0), planet(-30.0, 0.0)), Vec3::ZERO);
    }

    /// The sky along a meridian of the bake, as its second difference between rows relative to the value: the
    /// largest size, and the largest change from one row to the next (a band is a jump of it). Rows from the zenith
    /// down to 11 degrees above the horizon (below that the sky's own profile turns within a row or two).
    fn roughness(texels: &[[f32; 4]], w: u32, h: u32, column: u32) -> (f32, f32) {
        let at = |row: u32| luminance(Vec3::from_slice(&texels[(row * w + column) as usize][..3]));
        let second = |row: u32| (at(row - 1) - 2.0 * at(row) + at(row + 1)) / at(row);
        let last = h / 2 - h / 16;
        ((1..=last).map(|r| second(r).abs()).fold(0.0, f32::max), (1..last).map(|r| (second(r + 1) - second(r)).abs()).fold(0.0, f32::max))
    }

    /// No bands: along meridians of the bake away from the sun's own lobe, the second difference between rows is
    /// small and changes slowly, by day and in twilight.
    #[test]
    fn bake_has_no_bands() {
        let a = earth();
        let t = SkyTables::new(&a);
        let (w, h) = DYNAMIC_SKY;
        for elevation in [60.0f32, 10.0, 2.0, -3.0] {
            let bake = bake_atmosphere_tables(&a, &t, sun_at(elevation), Vec3::ONE, 0.0, w, h, Glow::default(), 0.0, Horizon::Level);
            assert!(bake.texels.iter().all(|t| t.iter().all(|v| v.is_finite() && *v >= 0.0)));
            // (The sun is at u = 0.75: the meridians a quarter turn from it and opposite it.)
            for column in [0, w / 4, w / 2] {
                let (size, change) = roughness(&bake.texels, w, h, column);
                let (most, jump) = if elevation > 0.0 { (0.02, 0.004) } else { (0.05, 0.015) };
                assert!(size < most && change < jump, "sun {elevation}, column {column}: {size} {change}");
            }
        }
    }

    /// The numerical settings: with twice the orders, four times the directions and steps of the tables and four
    /// times the view's steps the sky changes by under 1.5 % by day and 8 % in twilight.
    #[test]
    fn quality_is_enough() {
        let a = earth();
        let (fast, fine) = (Luts::build(&a), Luts::build_at(&a, Quality { orders: 6, azimuths: 8, steps: 48, view_steps: 64 }));
        for (sun, within) in [(75.0f32, 0.015f32), (30.0, 0.015), (10.0, 0.015), (2.0, 0.015), (-2.0, 0.08), (-6.0, 0.08)] {
            for e in [90.0f32, 30.0, 8.0, 2.0, 0.45] {
                for az in [0.0f32, 3.0, 20.0, 90.0, 180.0] {
                    let (x, y) = (fast.sky(&a, 0.005, planet(e, az), planet(sun, 0.0)), fine.sky(&a, 0.005, planet(e, az), planet(sun, 0.0)));
                    assert!((x - y).abs().cmple(y * within + 1e-9).all(), "sun {sun}, view {e} {az}: {x:?} {y:?}");
                }
            }
        }
    }

    /// The scene's settings do what they say.
    #[test]
    fn settings() {
        let (sun, view) = (planet(40.0, 0.0), planet(50.0, 150.0));
        let sky = |a: &Atmosphere, dir: Vec3| Luts::build(a).sky(a, 0.0, dir, sun);
        let base = sky(&earth(), view);
        // No air and no haze: nothing scatters.
        assert_eq!(sky(&Atmosphere::earth(0.0, 0.0, 0.8, Vec3::splat(0.4)), view), Vec3::ZERO);
        // Twice the air: a brighter blue. More haze: a brighter, whiter sky around the sun.
        assert!(sky(&Atmosphere::earth(2.0, 1.0, 0.8, Vec3::splat(0.401978)), view).z > 1.3 * base.z);
        let around = planet(40.0, 10.0);
        let (clear, hazy) = (sky(&earth(), around), sky(&Atmosphere::earth(1.0, 5.0, 0.8, Vec3::splat(0.401978)), around));
        assert!(hazy.x > 3.0 * clear.x && hazy.z / hazy.x < clear.z / clear.x, "{clear:?} {hazy:?}");
        // The anisotropy: a tighter lobe is brighter close to the sun and dimmer away from it.
        let tight = Atmosphere::earth(1.0, 5.0, 0.9, Vec3::splat(0.401978));
        assert!(sky(&tight, planet(40.0, 4.0)).x > sky(&Atmosphere::earth(1.0, 5.0, 0.8, Vec3::splat(0.401978)), planet(40.0, 4.0)).x);
        assert!(sky(&tight, planet(40.0, 60.0)).x < sky(&Atmosphere::earth(1.0, 5.0, 0.8, Vec3::splat(0.401978)), planet(40.0, 60.0)).x);
        // The ground's albedo: its bounce lights the sky, a coloured ground tints it.
        let (dark, bright) = (sky(&Atmosphere::earth(1.0, 1.0, 0.8, Vec3::ZERO), view), sky(&Atmosphere::earth(1.0, 1.0, 0.8, Vec3::ONE), view));
        assert!(dark.y < base.y && base.y < bright.y && bright.y > 1.15 * dark.y, "{dark:?} {base:?} {bright:?}");
        let red = sky(&Atmosphere::earth(1.0, 1.0, 0.8, Vec3::new(1.0, 0.0, 0.0)), view);
        assert!(red.x > 1.1 * dark.x && (red.z - dark.z).abs() < 1e-3 * dark.z, "{dark:?} {red:?}");
    }

    /// A random number in [0, 1): xorshift64* (Marsaglia 2003, "Xorshift RNGs"; Vigna 2016 for the multiplier).
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// A unit vector at polar cosine `c` and azimuth `phi` about the axis `n`.
    fn around(n: glam::DVec3, c: f64, phi: f64) -> glam::DVec3 {
        let t = if n.x.abs() < 0.9 { glam::DVec3::X } else { glam::DVec3::Y }.cross(n).normalize();
        let b = n.cross(t);
        let s = (1.0 - c * c).max(0.0).sqrt();
        (t * phi.cos() + b * phi.sin()) * s + n * c
    }

    /// The reference for the model: the same medium path traced (delta tracking with next-event estimation toward
    /// the sun at every collision and ground hit; Novak et al. 2018, "Monte Carlo Methods for Volumetric Light
    /// Transport Simulation", sections 3 and 4), one colour channel at a time, up to `orders` scattering events
    /// with the true phase functions, a Lambertian ground. Returns the mean radiance per unit of the sun's
    /// illuminance over the directions of an `elevation` range (degrees) and an `azimuth` range from the sun's
    /// (degrees, both sides), seen from 5 m, and its standard error.
    /// `soft`: the sun's light below a point's horizon as the model has it (`slant`), or cut by the ground's sphere.
    #[allow(clippy::too_many_arguments)]
    fn path_traced(a: &Atmosphere, luts: &Luts, sun_elevation: f64, elevation: (f64, f64), azimuth: (f64, f64), paths: usize, soft: bool, orders: usize) -> ([f64; 3], [f64; 3]) {
        use glam::DVec3;
        let e = sun_elevation.to_radians();
        let sun = DVec3::new(e.cos(), 0.0, e.sin());
        let origin = DVec3::new(0.0, 0.0, R_GROUND + 0.005);
        let to_sun = |x: DVec3, c: usize| -> f64 {
            let r = x.length().max(R_GROUND);
            let mu = x.dot(sun) / x.length();
            if !soft && distance_to_ground(r, mu).is_some() {
                return 0.0;
            }
            luts.sun_transmittance(At::new(mu, r))[c]
        };
        let chunks = 64usize;
        let sums: Vec<[f64; 3]> = (0..chunks)
            .into_par_iter()
            .map(|chunk| {
                let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ ((chunk as u64 + 1) * 0xD1B5_4A32_D192_ED03));
                let mut sum = [0.0f64; 3];
                for (c, sum) in sum.iter_mut().enumerate() {
                    // The largest extinction anywhere above the ground: every component at its densest.
                    let majorant = a.rayleigh[c] + a.mie_extinction + a.ozone[c];
                    for _ in 0..paths / chunks {
                        let (s0, s1) = (elevation.0.to_radians().sin(), elevation.1.to_radians().sin());
                        let z = s0 + (s1 - s0) * rng.next();
                        let phi = (azimuth.0 + (azimuth.1 - azimuth.0) * rng.next()).to_radians() * if rng.next() < 0.5 { -1.0 } else { 1.0 };
                        let h = (1.0 - z * z).sqrt();
                        let (mut x, mut d) = (origin, DVec3::new(h * phi.cos(), h * phi.sin(), z));
                        let mut weight = 1.0f64;
                        for _ in 0..orders {
                            let (r, mu) = (x.length(), x.dot(d) / x.length());
                            let ground = distance_to_ground(r, mu);
                            let end = ground.unwrap_or_else(|| distance_out(r, mu, R_TOP));
                            // Delta tracking to the next real collision.
                            let mut t = 0.0;
                            let hit = loop {
                                t -= (1.0 - rng.next()).ln() / majorant;
                                if t >= end {
                                    break None;
                                }
                                let dens = densities(((x + d * t).length() - R_GROUND).max(0.0));
                                let extinction = a.extinction(dens)[c];
                                if rng.next() * majorant < extinction {
                                    break Some((dens, extinction));
                                }
                            };
                            match hit {
                                Some((dens, extinction)) => {
                                    x += d * t;
                                    let (air, haze) = (a.rayleigh[c] * dens[0], a.mie_scattering * dens[1]);
                                    weight *= (air + haze) / extinction;
                                    let nu = d.dot(sun);
                                    *sum += weight * (air * Atmosphere::phase_rayleigh(nu) + haze * a.phase_mie(nu)) / (air + haze) * to_sun(x, c);
                                    // The next direction: the haze's lobe sampled (Henyey & Greenstein's inversion),
                                    // the air's uniformly with its phase function as the weight.
                                    if rng.next() * (air + haze) < haze {
                                        let g = a.mie_g;
                                        let cos = if g.abs() < 1e-3 { 1.0 - 2.0 * rng.next() } else {
                                            let q = (1.0 - g * g) / (1.0 - g + 2.0 * g * rng.next());
                                            (1.0 + g * g - q * q) / (2.0 * g)
                                        };
                                        d = around(d, cos.clamp(-1.0, 1.0), 2.0 * std::f64::consts::PI * rng.next());
                                    } else {
                                        let n = around(DVec3::Z, 1.0 - 2.0 * rng.next(), 2.0 * std::f64::consts::PI * rng.next());
                                        weight *= Atmosphere::phase_rayleigh(d.dot(n)) * 4.0 * std::f64::consts::PI;
                                        d = n;
                                    }
                                }
                                None if ground.is_some() => {
                                    x = (x + d * end).normalize() * (R_GROUND + 1e-6);
                                    let n = x.normalize();
                                    weight *= a.ground_albedo[c];
                                    *sum += weight / std::f64::consts::PI * n.dot(sun).max(0.0) * to_sun(x, c);
                                    d = around(n, rng.next().sqrt(), 2.0 * std::f64::consts::PI * rng.next());
                                }
                                None => break,
                            }
                            if weight < 1e-4 {
                                break;
                            }
                        }
                    }
                }
                sum
            })
            .collect();
        let per = (paths / chunks) as f64;
        let mean = [0, 1, 2].map(|c| sums.iter().map(|s| s[c]).sum::<f64>() / (per * chunks as f64));
        (mean, [0, 1, 2].map(|c| (sums.iter().map(|s| (s[c] / per - mean[c]).powi(2)).sum::<f64>() / (chunks * (chunks - 1)) as f64).sqrt()))
    }

    /// The model's mean over the same directions as `path_traced` (a midpoint grid).
    fn model_mean(a: &Atmosphere, luts: &Luts, sun_elevation: f64, elevation: (f64, f64), azimuth: (f64, f64)) -> [f64; 3] {
        let sun = planet(sun_elevation as f32, 0.0);
        let (n, m) = (64, 64);
        let mut sum = [0.0f64; 3];
        for i in 0..n {
            let (s0, s1) = (elevation.0.to_radians().sin(), elevation.1.to_radians().sin());
            let z = s0 + (s1 - s0) * (i as f64 + 0.5) / n as f64;
            for k in 0..m {
                let phi = (azimuth.0 + (azimuth.1 - azimuth.0) * (k as f64 + 0.5) / m as f64).to_radians();
                let h = (1.0 - z * z).sqrt();
                let l = luts.sky(a, 0.005, Vec3::new((h * phi.cos()) as f32, (h * phi.sin()) as f32, z as f32), sun);
                for c in 0..3 {
                    sum[c] += l[c] as f64 / (n * m) as f64;
                }
            }
        }
        sum
    }

    /// The model against the same medium path traced, quickly (the long version: `against_path_tracing`): the mean
    /// of the upper sky with the sun high and low, within 4 % (the path tracing's own noise is 0.5 % here).
    #[test]
    fn sky_matches_path_tracing() {
        let a = earth();
        let luts = Luts::build(&a);
        for sun in [40.0, 5.0] {
            let (model, (traced, noise)) = (model_mean(&a, &luts, sun, (10.0, 90.0), (0.0, 180.0)), path_traced(&a, &luts, sun, (10.0, 90.0), (0.0, 180.0), 192_000, false, 64));
            for c in 0..3 {
                assert!(noise[c] < 0.01 * traced[c], "{noise:?} {traced:?}");
                assert!((model[c] / traced[c] - 1.0).abs() < 0.04, "sun {sun}: {model:?} {traced:?}");
            }
        }
    }

    /// The previous release's bake of the sky (`cleanroom-data/sky`, not part of the repository), if it is here.
    fn previous_release(sun_elevation: i32) -> Option<Vec<[f32; 4]>> {
        let dir = std::env::var("CLEANROOM_SKY_DATA").unwrap_or_else(|_| "../cleanroom-data/sky".into());
        let bytes = std::fs::read(format!("{dir}/sky-{sun_elevation}.f32")).ok()?;
        Some(bytes.as_chunks::<16>().0.iter().map(|c| [0, 1, 2, 3].map(|k| f32::from_le_bytes([c[4 * k], c[4 * k + 1], c[4 * k + 2], c[4 * k + 3]]))).collect())
    }

    /// The mean of a 256 x 128 bake over its first `rows` rows (from the zenith), by solid angle.
    fn bake_mean(texels: &[[f32; 4]], rows: usize) -> [f64; 3] {
        let (mut s, mut n) = ([0.0f64; 3], 0.0f64);
        for (i, t) in texels.iter().enumerate().take(rows * 256) {
            let k = (((i / 256) as f64 + 0.5) / 128.0 * std::f64::consts::PI).sin();
            for c in 0..3 {
                s[c] += k * t[c] as f64;
            }
            n += k;
        }
        s.map(|v| v / n)
    }

    /// `cargo test --release against_path_tracing -- --ignored --nocapture`: the model against the path-traced
    /// medium (a minute), over the sky above the bake's horizon rows and over parts of it; single scattering alone,
    /// and the further orders alone; the previous release's bake against the same reference where its data is here.
    #[test]
    #[ignore]
    fn against_path_tracing() {
        let a = earth();
        let luts = Luts::build(&a);
        let single = Luts { sun: Table::fill(SUN_TABLE.0, SUN_TABLE.1, At::mu_of_sun, |mu_sun, r| a.optical_depth(r, mu_sun)), scattered: Table { columns: 2, rows: 2, data: vec![[0.0; 3]; 4] }, ground: Table { columns: 2, rows: 1, data: vec![[0.0; 3]; 2] }, view_steps: QUALITY.view_steps };
        let paths = std::env::var("SKY_PATHS").ok().and_then(|v| v.parse().ok()).unwrap_or(4_000_000usize);
        println!("sun elevation, part of the sky: model / traced (R G B); single scattering; further orders (their share of the traced light); the path tracing's standard error %");
        for sun in [60.0, 30.0, 10.0, 2.0, -3.0] {
            for (name, elevation, azimuth) in [("above 2.8 deg", (2.8125, 90.0), (0.0, 180.0)), ("above 45 deg", (45.0, 90.0), (0.0, 180.0)), ("3-15 deg, sun side", (2.8125, 15.0), (0.0, 60.0)), ("3-15 deg, opposite", (2.8125, 15.0), (120.0, 180.0))] {
                let (model, once_model) = (model_mean(&a, &luts, sun, elevation, azimuth), model_mean(&a, &single, sun, elevation, azimuth));
                let ((traced, noise), (once, _)) = (path_traced(&a, &luts, sun, elevation, azimuth, paths, true, 64), path_traced(&a, &luts, sun, elevation, azimuth, paths, true, 1));
                let ratio = |x: [f64; 3], y: [f64; 3]| format!("{:.3} {:.3} {:.3}", x[0] / y[0], x[1] / y[1], x[2] / y[2]);
                let (further_model, further) = ([0, 1, 2].map(|c| model[c] - once_model[c]), [0, 1, 2].map(|c| traced[c] - once[c]));
                println!("{sun:5} {name:18}: {} | {} | {} ({}) | {:.2} {:.2} {:.2}", ratio(model, traced), ratio(once_model, once), ratio(further_model, further), ratio(further, traced), 100.0 * noise[0] / traced[0], 100.0 * noise[1] / traced[1], 100.0 * noise[2] / traced[2]);
                if name.starts_with("above 2") {
                    if sun < 5.0 {
                        let (hard, _) = path_traced(&a, &luts, sun, elevation, azimuth, paths, false, 64);
                        println!("{sun:5} with the sun cut at the ground's sphere (no soft ground) the traced light is {} of the above", ratio(hard, traced));
                    }
                    if let Some(previous) = previous_release(sun as i32) {
                        println!("{sun:5} the previous release's bake / traced: {}", ratio(bake_mean(&previous, 62), traced));
                    }
                }
            }
        }
    }

    /// `cargo test --release reference_data -- --ignored --nocapture`: the acceptance numbers against the previous
    /// release's measurements (`cleanroom-data/sky`).
    #[test]
    #[ignore]
    fn reference_data() {
        let dir = std::env::var("CLEANROOM_SKY_DATA").unwrap_or_else(|_| "../cleanroom-data/sky".into());
        let a = earth();
        let csv = std::fs::read_to_string(format!("{dir}/sun-transmittance.csv")).expect("reference data");
        println!("sun transmittance: elevation | ours | previous release | difference % | the medium's own (no veil) against the previous release %");
        for line in csv.lines().skip(1) {
            let v: Vec<f32> = line.split(',').map(|x| x.parse().unwrap()).collect();
            let (t, medium) = (a.sun_transmittance(sun_at(v[0])), a.ground_transmittance((v[0] as f64).to_radians().sin()));
            let rel = |o: f32, r: f32| if r > 1e-6 { format!("{:+.2}", (o / r - 1.0) * 100.0) } else { "-".into() };
            println!("{:5} | {:.6} {:.6} {:.6} | {:.6} {:.6} {:.6} | {} {} {} | {} {} {}", v[0], t.x, t.y, t.z, v[1], v[2], v[3], rel(t.x, v[1]), rel(t.y, v[2]), rel(t.z, v[3]), rel(medium[0] as f32, v[1]), rel(medium[1] as f32, v[2]), rel(medium[2] as f32, v[3]));
        }
        let (w, h) = (256u32, 128u32);
        println!("sky bakes: sun elevation | upper hemisphere's mean, ours | previous release | difference % | roughness along the meridians at u = 0, 0.25, 0.5 (largest second difference / value, largest change of it), ours | previous release");
        for e in [60, 30, 10, 2, -3] {
            let reference = previous_release(e).expect("reference data");
            let ours = bake_atmosphere(&a, sun_at(e as f32), Vec3::ONE, 0.0, w, h);
            let (mo, mr) = (bake_mean(&ours.texels, 64), bake_mean(&reference, 64));
            let rough = |t: &[[f32; 4]]| [0, w / 4, w / 2].map(|c| roughness(t, w, h, c)).iter().map(|(a, b)| format!("{a:.4}/{b:.4}")).collect::<Vec<_>>().join(" ");
            println!("{e:3} | {:.5} {:.5} {:.5} | {:.5} {:.5} {:.5} | {:+.2} {:+.2} {:+.2} | {} | {}", mo[0], mo[1], mo[2], mr[0], mr[1], mr[2], (mo[0] / mr[0] - 1.0) * 100.0, (mo[1] / mr[1] - 1.0) * 100.0, (mo[2] / mr[2] - 1.0) * 100.0, rough(&ours.texels), rough(&reference));
            // What the sky gives a horizontal surface (per unit of the sun's illuminance), ours against the previous.
            let irradiance = |t: &[[f32; 4]]| {
                let mut e = Vec3::ZERO;
                for (i, t) in t.iter().enumerate().take((w * h / 2) as usize) {
                    let theta = ((i as u32 / w) as f32 + 0.5) / h as f32 * PI;
                    e += Vec3::new(t[0], t[1], t[2]) * (theta.cos() * theta.sin() * (PI / h as f32) * (2.0 * PI / w as f32));
                }
                e
            };
            let (eo, er) = (irradiance(&ours.texels), irradiance(&reference));
            println!("      horizontal irradiance from the sky: ours {:.5} {:.5} {:.5}, previous {:.5} {:.5} {:.5}, luminance ours / previous {:.4}", eo.x, eo.y, eo.z, er.x, er.y, er.z, luminance(eo) / luminance(er));
            // Ours / the previous release along the sun's meridian (u = 0.75), the opposite one and one across.
            for (name, col) in [("sun's side", 192u32), ("opposite", 64), ("across", 0)] {
                let mut s = format!("      {name:10} ours / previous, rows from the zenith:");
                for row in [0u32, 16, 32, 44, 52, 58, 61, 62, 63] {
                    let (o, r) = (ours.texels[(row * w + col) as usize], reference[(row * w + col) as usize]);
                    s += &format!(" {row}: {:.2}/{:.2}/{:.2}", o[0] / r[0], o[1] / r[1], o[2] / r[2]);
                }
                println!("{s}");
            }
        }
    }

    /// `cargo test --release timings -- --ignored --nocapture`: the tables' and the bake's times (the best of 15), on
    /// all cores and on the live view's three; and, for a busy machine, a bake small enough to run between two
    /// interruptions on one thread, the best of many.
    #[test]
    #[ignore]
    fn timings() {
        let a = earth();
        let best = |n: usize, f: &dyn Fn()| (0..n).map(|_| { let t = std::time::Instant::now(); f(); t.elapsed().as_secs_f64() * 1e3 }).fold(f64::MAX, f64::min);
        let run = |name: &str| {
            println!("{name}: tables (SkyTables::new) {:.1} ms", best(15, &|| { SkyTables::new(&a); }));
            let t = SkyTables::new(&a);
            for e in [60.0, 10.0, -3.0] {
                println!("{name}: bake_atmosphere_tables, sun at {e:3}: 256 x 128 {:.1} ms, 1024 x 512 {:.1} ms", best(15, &|| { bake_atmosphere_tables(&a, &t, sun_at(e), Vec3::ONE, 0.0, 256, 128, Glow::default(), 0.0, Horizon::Level); }), best(5, &|| { bake_atmosphere_tables(&a, &t, sun_at(e), Vec3::ONE, 0.0, 1024, 512, Glow::default(), 0.0, Horizon::Level); }));
            }
        };
        run("all cores");
        rayon::ThreadPoolBuilder::new().num_threads(3).build().unwrap().install(|| run("3 threads"));
        rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap().install(|| {
            let t = SkyTables::new(&a);
            let small = best(300, &|| { bake_atmosphere_tables(&a, &t, sun_at(10.0), Vec3::ONE, 0.0, 64, 32, Glow::default(), 0.0, Horizon::Level); });
            println!("one thread: 64 x 32 {small:.3} ms, so 256 x 128 is {:.1} ms of one core and 1024 x 512 {:.0} ms; tables {:.0} ms of one core", small * 16.0, small * 256.0, best(5, &|| { SkyTables::new(&a); }));
        });
    }
}
