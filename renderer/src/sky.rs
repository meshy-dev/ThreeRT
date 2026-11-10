//! Physical sky matching UE5's SkyAtmosphere (Hillaire 2020): Earth defaults from
//! SkyAtmosphereComponent.cpp, transmittance + multiple-scattering LUTs, radiance baked into an
//! equirectangular environment map (Y-up, upper hemisphere; lower hemisphere black like UE's SkyLight, with its
//! cubemap's fade at the horizon).

use glam::Vec3;
use rayon::prelude::*;
use std::f32::consts::PI;

pub struct Atmosphere {
    bottom: f32,
    top: f32,
    rayleigh_scattering: Vec3,
    rayleigh_exp: f32,
    mie_scattering: Vec3,
    mie_extinction: Vec3,
    mie_exp: f32,
    mie_g: f32,
    ozone_absorption: Vec3,
    ground_albedo: Vec3,
}

impl Atmosphere {
    pub fn earth(rayleigh_scale: f32, mie_scale: f32, mie_g: f32, ground_albedo: Vec3) -> Self {
        let mie_s = Vec3::splat(0.003996 * mie_scale);
        Atmosphere {
            bottom: 6360.0,
            top: 6420.0,
            rayleigh_scattering: Vec3::new(0.005802, 0.013558, 0.033100) * rayleigh_scale,
            rayleigh_exp: -1.0 / 8.0,
            mie_scattering: mie_s,
            mie_extinction: mie_s + Vec3::splat(0.000444),
            mie_exp: -1.0 / 1.2,
            mie_g,
            ozone_absorption: Vec3::new(0.000650, 0.001881, 0.000085),
            ground_albedo,
        }
    }

    fn medium(&self, h: f32) -> (Vec3, Vec3, Vec3) {
        let dr = (self.rayleigh_exp * h).exp();
        let dm = (self.mie_exp * h).exp();
        let doz = (1.0 - (h - 25.0).abs() / 15.0).clamp(0.0, 1.0);
        let sr = self.rayleigh_scattering * dr;
        let sm = self.mie_scattering * dm;
        let ext = sr + self.mie_extinction * dm + self.ozone_absorption * doz;
        (sr, sm, ext)
    }

    /// UE FAtmosphereSetup::GetTransmittanceAtGroundLevel: 15-step optical depth from 0.5 km altitude.
    pub fn sun_transmittance_ue(&self, sun_dir_y_up: Vec3) -> Vec3 {
        let elev = sun_dir_y_up.y.clamp(-1.0, 1.0).asin();
        let pos = Vec3::new(0.0, 0.0, self.bottom + 0.5);
        let dir = Vec3::new(elev.cos(), 0.0, elev.sin());
        let tmax = ray_sphere_nearest(pos, dir, self.top);
        let mut od = Vec3::ZERO;
        if tmax > 0.0 {
            let n = 15;
            let step = tmax / n as f32;
            for i in 0..n {
                let p = pos + dir * (step * i as f32);
                let h = p.length() - self.bottom;
                let (sr, _, _) = self.medium(h);
                let dm = (self.mie_exp * h).exp().max(0.0);
                let doz = (1.0 - (h - 25.0).abs() / 15.0).clamp(0.0, 1.0);
                od += step * (dm * self.mie_extinction + sr + doz * self.ozone_absorption);
            }
        }
        Vec3::new((-od.x).exp(), (-od.y).exp(), (-od.z).exp())
    }

    /// Transmittance from point (r = |p|) along mu=cos(zenith) to the top of the atmosphere (0 if it hits ground).
    fn transmittance_raw(&self, r: f32, mu: f32, steps: usize) -> Vec3 {
        let pos = Vec3::new(0.0, 0.0, r);
        let dir = Vec3::new((1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu);
        if ray_sphere_nearest(pos, dir, self.bottom) > 0.0 {
            return Vec3::ZERO;
        }
        let tmax = ray_sphere_nearest(pos, dir, self.top);
        if tmax <= 0.0 {
            return Vec3::ONE;
        }
        let dt = tmax / steps as f32;
        let mut od = Vec3::ZERO;
        for i in 0..steps {
            let p = pos + dir * (dt * (i as f32 + 0.5));
            od += self.medium(p.length() - self.bottom).2 * dt;
        }
        Vec3::new((-od.x).exp(), (-od.y).exp(), (-od.z).exp())
    }
}

fn ray_sphere_nearest(o: Vec3, d: Vec3, r: f32) -> f32 {
    let b = o.dot(d);
    let c = o.dot(o) - r * r;
    let disc = b * b - c;
    if disc < 0.0 {
        return -1.0;
    }
    let s = disc.sqrt();
    let (t0, t1) = (-b - s, -b + s);
    if t0 < 0.0 && t1 < 0.0 {
        -1.0
    } else if t0 < 0.0 {
        t1.max(0.0)
    } else {
        t0.max(0.0)
    }
}

fn rayleigh_phase(c: f32) -> f32 {
    3.0 / (16.0 * PI) * (1.0 + c * c)
}

fn hg_phase(g: f32, c: f32) -> f32 {
    let n = 1.0 - g * g;
    let d = 1.0 + g * g - 2.0 * g * c;
    n / (4.0 * PI * d * d.sqrt())
}

struct Luts {
    trans: Vec<Vec3>, // [mu][r]
    ms: Vec<Vec3>,    // [r][mu_s]
}

const T_MU: usize = 128;
const T_R: usize = 32;
const MS_N: usize = 32;

impl Luts {
    fn build(a: &Atmosphere) -> Luts {
        let trans: Vec<Vec3> = (0..T_MU * T_R)
            .into_par_iter()
            .map(|i| {
                let (mi, ri) = (i / T_R, i % T_R);
                let r = a.bottom + (a.top - a.bottom) * (ri as f32 / (T_R - 1) as f32);
                let mu = -1.0 + 2.0 * (mi as f32 / (T_MU - 1) as f32);
                a.transmittance_raw(r, mu, 64)
            })
            .collect();
        let mut l = Luts { trans, ms: vec![Vec3::ZERO; MS_N * MS_N] };
        // Multiple scattering transfer (Hillaire 2020, Sec 5.5): psi = L2 / (1 - fms), isotropic phase.
        let dirs = sphere_dirs(64);
        let ms: Vec<Vec3> = (0..MS_N * MS_N)
            .into_par_iter()
            .map(|i| {
                let (ri, si) = (i / MS_N, i % MS_N);
                let r = a.bottom + (a.top - a.bottom) * ((ri as f32 + 0.5) / MS_N as f32);
                let mu_s = -1.0 + 2.0 * ((si as f32 + 0.5) / MS_N as f32);
                let sun = Vec3::new((1.0 - mu_s * mu_s).max(0.0).sqrt(), 0.0, mu_s);
                let pos = Vec3::new(0.0, 0.0, r);
                let (mut l2, mut fms) = (Vec3::ZERO, Vec3::ZERO);
                for d in &dirs {
                    let (lum, f) = l.integrate(a, pos, *d, sun, 20, true, None, true);
                    l2 += lum;
                    fms += f;
                }
                let n = dirs.len() as f32;
                l2 /= n;
                fms /= n;
                l2 / (Vec3::ONE - fms)
            })
            .collect();
        l.ms = ms;
        l
    }

    fn trans(&self, a: &Atmosphere, r: f32, mu: f32) -> Vec3 {
        let x = ((mu + 1.0) * 0.5 * (T_MU - 1) as f32).clamp(0.0, (T_MU - 1) as f32);
        let y = ((r - a.bottom) / (a.top - a.bottom) * (T_R - 1) as f32).clamp(0.0, (T_R - 1) as f32);
        bilerp(&self.trans, T_MU, T_R, x, y, |mi, ri| mi * T_R + ri)
    }

    fn ms(&self, a: &Atmosphere, r: f32, mu_s: f32) -> Vec3 {
        let x = ((mu_s + 1.0) * 0.5 * MS_N as f32 - 0.5).clamp(0.0, (MS_N - 1) as f32);
        let y = ((r - a.bottom) / (a.top - a.bottom) * MS_N as f32 - 0.5).clamp(0.0, (MS_N - 1) as f32);
        bilerp(&self.ms, MS_N, MS_N, x, y, |si, ri| ri * MS_N + si)
    }

    /// Raymarch in-scattering. Returns (luminance for unit sun illuminance, multi-scatter "as 1" term).
    #[allow(clippy::too_many_arguments)]
    fn integrate(&self, a: &Atmosphere, pos: Vec3, dir: Vec3, sun: Vec3, steps: usize, ground: bool, ms_lut: Option<()>, isotropic: bool) -> (Vec3, Vec3) {
        let t_ground = ray_sphere_nearest(pos, dir, a.bottom);
        let t_top = ray_sphere_nearest(pos, dir, a.top);
        let tmax = if t_ground > 0.0 { t_ground } else { t_top };
        if tmax <= 0.0 {
            return (Vec3::ZERO, Vec3::ZERO);
        }
        let cos = dir.dot(sun);
        let (pr, pm) = if isotropic { (1.0 / (4.0 * PI), 1.0 / (4.0 * PI)) } else { (rayleigh_phase(cos), hg_phase(a.mie_g, cos)) };
        let mut lum = Vec3::ZERO;
        let mut fms = Vec3::ZERO;
        let mut thr = Vec3::ONE;
        let mut t = 0.0f32;
        for i in 0..steps {
            // Slightly non-linear distribution concentrates samples near the viewer like UE.
            let t1 = tmax * ((i as f32 + 1.0) / steps as f32).powf(1.5);
            let dt = t1 - t;
            let p = pos + dir * (t + 0.3 * dt);
            t = t1;
            let r = p.length();
            let up = p / r;
            let mu_s = sun.dot(up);
            let (sr, sm, ext) = a.medium(r - a.bottom);
            let tsun = self.trans(a, r, mu_s);
            let earth_shadow = if ray_sphere_nearest(p, sun, a.bottom) > 0.0 { 0.0 } else { 1.0 };
            let ms = if ms_lut.is_some() { self.ms(a, r, mu_s) } else { Vec3::ZERO };
            let s = earth_shadow * tsun * (sr * pr + sm * pm) + ms * (sr + sm);
            let st = Vec3::new((-ext.x * dt).exp(), (-ext.y * dt).exp(), (-ext.z * dt).exp());
            let e = ext.max(Vec3::splat(1e-9));
            lum += thr * (s - s * st) / e;
            fms += thr * ((sr + sm) - (sr + sm) * st) / e;
            thr *= st;
        }
        if ground && t_ground > 0.0 {
            let p = pos + dir * t_ground;
            let up = p.normalize();
            let mu_s = sun.dot(up);
            let tsun = self.trans(a, p.length(), mu_s);
            lum += thr * tsun * mu_s.max(0.0) * a.ground_albedo / PI;
        }
        (lum, fms)
    }
}

fn bilerp(v: &[Vec3], nx: usize, ny: usize, x: f32, y: f32, idx: impl Fn(usize, usize) -> usize) -> Vec3 {
    let (x0, y0) = ((x.floor() as usize).min(nx - 1), (y.floor() as usize).min(ny - 1));
    let (x1, y1) = ((x0 + 1).min(nx - 1), (y0 + 1).min(ny - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let a = v[idx(x0, y0)].lerp(v[idx(x1, y0)], fx);
    let b = v[idx(x0, y1)].lerp(v[idx(x1, y1)], fx);
    a.lerp(b, fy)
}

fn sphere_dirs(n: usize) -> Vec<Vec3> {
    // Fibonacci sphere.
    let ga = PI * (3.0 - 5f32.sqrt());
    (0..n)
        .map(|i| {
            let z = 1.0 - 2.0 * (i as f32 + 0.5) / n as f32;
            let r = (1.0 - z * z).sqrt();
            let phi = ga * i as f32;
            Vec3::new(r * phi.cos(), r * phi.sin(), z)
        })
        .collect()
}

pub struct SkyBake {
    pub width: u32,
    pub height: u32,
    /// Linear radiance (nits) per texel, RGBA f32, equirect: u = atan2(x, -z)/2pi + 0.5, v = acos(y)/pi.
    pub texels: Vec<[f32; 4]>,
    /// Sun illuminance reaching the ground (lux, RGB), i.e. E_sun * T(ground -> sun).
    pub sun_illuminance: Vec3,
}

/// The sky's size when its sun moves (`desc::Sky::Atmosphere::dynamic`): a bake takes ~10 ms of all cores here
/// against ~70 ms at the static 1024 x 512, so the live view can re-bake it on a worker thread several times a second.
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
    bake_atmosphere_with(a, &Luts::build(a), sun_dir, sun_lux, altitude_m, width, height, Glow::default(), 0.0)
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

/// `bake_atmosphere` with tables built once (`SkyTables`), plus the night sky's `glow` above the horizon and a cloud
/// `cover` (0..1, `overcast`).
#[allow(clippy::too_many_arguments)]
pub fn bake_atmosphere_tables(a: &Atmosphere, t: &SkyTables, sun_dir: Vec3, sun_lux: Vec3, altitude_m: f32, width: u32, height: u32, glow: Glow, cover: f32) -> SkyBake {
    bake_atmosphere_with(a, &t.0, sun_dir, sun_lux, altitude_m, width, height, glow, cover)
}

/// The sun's illuminance at the ground (lux, RGB) for a sun of `sun_lux` above the atmosphere under a cloud `cover`,
/// as the bake gives it: what a directional sun light carries when it moves without a new bake.
pub fn sun_illuminance(a: &Atmosphere, sun_dir: Vec3, sun_lux: Vec3, cover: f32) -> Vec3 {
    sun_lux * a.sun_transmittance_ue(sun_dir) * cloud_transmittance(cover)
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
fn bake_atmosphere_with(a: &Atmosphere, luts: &Luts, sun_dir: Vec3, sun_lux: Vec3, altitude_m: f32, width: u32, height: u32, glow: Glow, cover: f32) -> SkyBake {
    // Planet frame: Z up. Map Y-up world -> planet frame (x, z, y).
    let to_planet = |d: Vec3| Vec3::new(d.x, d.z, d.y);
    let sun_p = to_planet(sun_dir).normalize();
    let pos = Vec3::new(0.0, 0.0, a.bottom + (altitude_m / 1000.0).max(0.005));
    let texels: Vec<[f32; 4]> = (0..(width * height) as usize)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i as u32 % width) as f32 + 0.5, (i as u32 / width) as f32 + 0.5);
            let theta = y / height as f32 * PI;
            let phi = (x / width as f32 - 0.5) * 2.0 * PI;
            let d = Vec3::new(theta.sin() * phi.sin(), theta.cos(), -theta.sin() * phi.cos());
            // Texel rows above the horizon on the side face this direction falls on; within half a row of it the
            // value blends the first row's centre with the black row below.
            let side = d.x.abs().max(d.z.abs()).max(1e-6);
            let r = d.y / side * (0.5 * SKY_LIGHT_CUBE);
            if r <= -0.5 {
                return [0.0, 0.0, 0.0, 0.0];
            }
            let (dir, k) = if r < 0.5 { (Vec3::new(d.x, side / SKY_LIGHT_CUBE, d.z).normalize(), r + 0.5) } else { (d, 1.0) };
            let (l, _) = luts.integrate(a, pos, to_planet(dir), sun_p, 40, false, Some(()), false);
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
    for t in &mut texels {
        t[3] = 1.0;
    }
    SkyBake { width, height, texels, sun_illuminance: sun_illuminance(a, sun_dir, sun_lux, cover) }
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
    SkyBake { width, height, texels, sun_illuminance: Vec3::ZERO }
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
        let clear = bake_atmosphere_tables(&a, &t, sun, lux, 10.0, w, h, Glow::default(), 0.0);
        let oc = bake_atmosphere_tables(&a, &t, sun, lux, 10.0, w, h, Glow::default(), 1.0);
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
        let half = bake_atmosphere_tables(&a, &t, sun, lux, 10.0, w, h, Glow::default(), 0.5);
        assert!((half.sun_illuminance - clear.sun_illuminance * 0.5).length() < 1e-3 * clear.sun_illuminance.length());
        let k = (ten * w + 7) as usize;
        assert!((lum(half.texels[k]) - 0.5 * (lum(clear.texels[k]) + lum(oc.texels[k]))).abs() < 1e-3 * lum(clear.texels[k]));
    }
}
