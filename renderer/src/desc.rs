//! Descriptors scene code passes to the engine (materials, lights, camera, environment; see world.rs and the
//! binding in packages/scene/src/engine.ts) and the render presets. Texture / object references are engine handles.

use serde::Deserialize;

pub type Vec3 = [f32; 3];

#[derive(Deserialize, Debug, Clone)]
pub struct Shader {
    /// `fn mf_<id>(i: MatIn, m: Material, base: MatOut) -> MatOut { ... }`
    pub code: String,
    #[serde(default)]
    pub helpers: String,
}

/// A material's binding to a material function: 32 parameter floats (8 vec4) and up to 8 textures.
#[derive(Deserialize, Debug, Clone)]
pub struct ShaderRef {
    /// Material function name.
    pub id: String,
    pub params: Vec<f32>,
    pub textures: Vec<Option<u32>>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Light {
    #[serde(rename_all = "camelCase")]
    Directional { color: Vec3, cast_shadow: bool, intensity: f32, angular_diameter: f32, #[serde(default = "one")] volumetric_scattering: f32, #[serde(default = "one")] specular_scale: f32 },
    #[serde(rename_all = "camelCase")]
    Point { color: Vec3, cast_shadow: bool, intensity: f32, radius: f32, range: f32, #[serde(default = "one")] volumetric_scattering: f32, #[serde(default = "one")] specular_scale: f32 },
    #[serde(rename_all = "camelCase")]
    Spot { color: Vec3, cast_shadow: bool, intensity: f32, radius: f32, range: f32, angle: f32, penumbra: f32, #[serde(default = "one")] volumetric_scattering: f32, #[serde(default = "one")] specular_scale: f32 },
    /// Luminance (nits), one-sided Lambertian rectangle; analytic (lighting only) unless `card` rendering is wanted.
    #[serde(rename_all = "camelCase")]
    Rect {
        color: Vec3,
        cast_shadow: bool,
        intensity: f32,
        width: f32,
        height: f32,
        #[serde(default)]
        range: f32,
        #[serde(default = "ninety")]
        barn_door_angle: f32,
        #[serde(default = "one")]
        volumetric_scattering: f32,
        #[serde(default = "one")]
        specular_scale: f32,
        /// Visible emissive card (default, three.js-scene semantics) or a pure light (UE RectLight).
        #[serde(default = "yes")]
        visible: bool,
    },
}

fn yes() -> bool {
    true
}

fn ninety() -> f32 {
    90.0
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum Exposure {
    Manual { ev100: f32 },
    #[serde(rename_all = "camelCase")]
    Auto { compensation: f32, min_ev100: f32, max_ev100: f32 },
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Dof {
    pub focus_distance: f32,
    pub f_stop: f32,
    pub sensor_width: f32,
}

/// Pose of an animated model: clip (by name) sampled at `time` s.
#[derive(Deserialize, Debug, Clone, PartialEq)]
pub struct Animation {
    pub clip: String,
    pub time: f32,
    #[serde(default = "yes")]
    pub r#loop: bool,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Camera {
    pub fov: f32,
    #[allow(dead_code)]
    pub near: f32,
    #[allow(dead_code)]
    pub far: f32,
    pub exposure: Exposure,
    pub dof: Option<Dof>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Environment {
    pub sky: Sky,
    pub fog: Option<Fog>,
    /// three.js `scene.background`: what camera rays that miss see (radiance, linear rgb), lighting nothing.
    #[serde(default)]
    pub background: Option<Vec3>,
    /// Global surface wetness 0..1 (material functions' `i.wetness`).
    #[serde(default)]
    pub wetness: f32,
    /// Rain intensity 0..1 (material functions' `i.rain`: ripples on wet surfaces).
    #[serde(default)]
    pub rain: f32,
}

/// Exponential height fog (UE ExponentialHeightFog): extinction `density * exp(-heightFalloff * (y - height))`.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Fog {
    pub density: f32,
    pub height_falloff: f32,
    #[serde(default)]
    pub height: f32,
    /// Ambient inscattering radiance (linear rgb x intensity, nits).
    pub color: Vec3,
    pub intensity: Option<f32>,
    #[serde(default)]
    pub start_distance: f32,
    pub max_opacity: Option<f32>,
    pub volumetric: Option<Volumetric>,
}

/// Single scattering of lights and emissive surfaces in the fog along camera rays (halos, light shafts).
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Volumetric {
    /// Scattering albedo (0..1).
    pub scattering: f32,
    /// Henyey-Greenstein phase anisotropy (-1..1).
    #[serde(default)]
    pub anisotropy: f32,
    /// Density multiplier inside the volumetric range (UE VolumetricFogExtinctionScale relative to the height fog).
    #[serde(default = "one")]
    pub extinction_scale: f32,
    /// Scattering tint.
    pub albedo: Option<Vec3>,
    /// Range of the lit volumetric fog from the camera, m (beyond: height fog only). 0 = unlimited.
    #[serde(default)]
    pub distance: f32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Sky {
    /// `dynamic`: the sun moves while the scene runs (time of day). The live view then bakes the sky at
    /// `sky::DYNAMIC_SKY` on a worker thread as the sun moves, instead of rebuilding the scene; offline renders bake it
    /// at that size too, so both see the same sky. `glow`: night-sky radiance (nits) added above the horizon.
    #[serde(rename_all = "camelCase")]
    Atmosphere {
        sun: Option<u32>,
        ground_albedo: Vec3,
        rayleigh_scale: f32,
        mie_scale: f32,
        mie_anisotropy: f32,
        sky_light_intensity: f32,
        #[serde(default)]
        dynamic: bool,
        #[serde(default)]
        glow: Option<Vec3>,
        /// Aerial perspective: the atmosphere's haze on the surfaces it lies in front of, at this multiple of its own
        /// density (0 = off; pathtrace.wgsl `aerial_tr`).
        #[serde(default)]
        aerial: f32,
    },
    #[serde(rename_all = "camelCase")]
    Color { color: Vec3, intensity: f32, sky_light_intensity: f32 },
    #[serde(rename_all = "camelCase")]
    Hdri { texture: u32, intensity: f32, rotation: f32, sky_light_intensity: f32 },
    /// Analytic night / overcast dome: zenith-horizon-ground gradient, horizon (light-pollution) glow, clouds.
    #[serde(rename_all = "camelCase")]
    Gradient { zenith: Vec3, horizon: Vec3, ground: Vec3, intensity: f32, horizon_exponent: Option<f32>, glow: Option<SkyGlow>, clouds: Option<SkyClouds>, sky_light_intensity: f32 },
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkyGlow {
    pub color: Vec3,
    /// Relative to `intensity`.
    pub intensity: f32,
    /// Elevation falloff (radians to 1/e).
    #[serde(default)]
    pub height: f32,
    /// UE band: (1 - |sin e| * width)^power when set.
    pub width: Option<f32>,
    pub power: Option<f32>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkyClouds {
    /// 0..1 sky fraction.
    pub coverage: f32,
    /// Radiance of the cloud base, relative to `intensity` (night clouds are lit from below by the city).
    pub color: Vec3,
    /// Angular size of cloud features (larger = bigger clouds).
    #[serde(default = "one")]
    pub scale: f32,
    #[serde(default)]
    pub seed: f32,
    /// Tileable cloud mask (R): UE-style projection, smoothstep(threshold), sky * gain + color.
    pub map: Option<u32>,
    pub threshold: Option<[f32; 2]>,
    pub gain: Option<f32>,
    pub projection: Option<f32>,
    pub bias: Option<f32>,
    pub fade: Option<f32>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    #[allow(dead_code)]
    pub name: String,
    pub color: Vec3,
    pub opacity: f32,
    pub metalness: f32,
    pub roughness: f32,
    pub emissive: Vec3,
    pub emissive_intensity: f32,
    pub map: Option<u32>,
    pub normal_map: Option<u32>,
    pub normal_scale: Option<[f32; 2]>,
    pub roughness_map: Option<u32>,
    pub metalness_map: Option<u32>,
    pub ao_map: Option<u32>,
    pub emissive_map: Option<u32>,
    pub specular_intensity: Option<f32>,
    pub clearcoat: Option<f32>,
    pub clearcoat_roughness: Option<f32>,
    pub transmission: Option<f32>,
    /// Red channel scales `transmission` (glTF transmissionTexture).
    #[serde(default)]
    pub transmission_map: Option<u32>,
    pub ior: Option<f32>,
    pub double_sided: bool,
    /// Alpha cutout threshold on base-colour alpha x opacity (None = opaque).
    pub alpha_test: Option<f32>,
    /// Blended by opacity: particles with it go through the translucency pass (gpu.rs `Translucent`).
    #[serde(default)]
    pub transparent: bool,
    pub shader: Option<ShaderRef>,
    /// three.js MeshBasicMaterial: shows color x map to camera rays, lit by nothing, lighting nothing (F_UNLIT).
    #[serde(default)]
    pub unlit: bool,
    /// three.js `vertexColors`: the color is multiplied by the geometry's vertex colours (F_VCOLOR).
    #[serde(default)]
    pub vertex_colors: bool,
}

// ------------------------------------------------------------------------------------------------
// Presets (presets/*.json)

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub name: String,
    pub post: PostPreset,
    pub native: NativePreset,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PostPreset {
    pub tonemapper: Tonemapper,
    pub color_grading: ColorGrading,
    pub bloom: Bloom,
    pub vignette: f32,
    pub auto_exposure: AutoExposure,
    /// UE FilmGrainIntensity (0..1).
    #[serde(default)]
    pub film_grain: f32,
    /// UE SceneFringeIntensity (chromatic aberration, 0..5).
    #[serde(default)]
    pub chromatic_aberration: f32,
    /// UE local exposure (bilateral): contrast of the low-frequency luminance around middle grey.
    #[serde(default)]
    pub local_exposure: LocalExposure,
    /// UE motion blur (amount, max, target FPS; UE's defaults when absent). The presets switch it off, like their UE
    /// references (r.MotionBlurQuality 0): stills and temporal benchmarks compare unblurred frames.
    #[serde(default)]
    pub motion_blur: MotionBlur,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct MotionBlur {
    /// UE MotionBlurAmount: shutter as a fraction of the target frame time (0 = off).
    pub amount: f32,
    /// UE MotionBlurMax: longest blur, percent of the screen width.
    pub max: f32,
    /// UE MotionBlurTargetFPS: blur length as if the game ran at this rate.
    pub target_fps: f32,
}

impl Default for MotionBlur {
    fn default() -> Self {
        MotionBlur { amount: 0.5, max: 5.0, target_fps: 30.0 }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct LocalExposure {
    pub highlight_contrast: f32,
    pub shadow_contrast: f32,
    pub detail_strength: f32,
    /// Base luminance = lerp(bilateral grid, wide blur, blend) (UE BlurredLuminanceBlend 0.6).
    pub blurred_luminance_blend: f32,
    /// Wide blur kernel, percent of the view width (UE 50).
    pub blurred_luminance_kernel: f32,
}

impl Default for LocalExposure {
    fn default() -> Self {
        LocalExposure { highlight_contrast: 1.0, shadow_contrast: 1.0, detail_strength: 1.0, blurred_luminance_blend: 0.6, blurred_luminance_kernel: 50.0 }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Tonemapper {
    pub slope: f32,
    pub toe: f32,
    pub shoulder: f32,
    pub black_clip: f32,
    pub white_clip: f32,
    pub tone_curve_amount: f32,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ColorGrading {
    pub expand_gamut: f32,
    pub blue_correction: f32,
    /// UE white balance: the scene illuminant (K) that renders as white, and green-magenta tint.
    #[serde(default = "d65")]
    pub white_temp: f32,
    #[serde(default)]
    pub white_tint: f32,
    /// UE ColorSaturation / ColorContrast (global), ColorGainShadows / ColorGainHighlights: scalar or RGB.
    #[serde(default = "one3")]
    pub saturation: Rgb,
    #[serde(default = "one3")]
    pub contrast: Rgb,
    #[serde(default = "one3")]
    pub shadows_gain: Rgb,
    #[serde(default = "one3")]
    pub highlights_gain: Rgb,
}

/// A scalar or RGB grading value.
#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(from = "RgbIn")]
pub struct Rgb(pub [f32; 3]);

#[derive(Deserialize)]
#[serde(untagged)]
enum RgbIn {
    S(f32),
    V([f32; 3]),
}

impl From<RgbIn> for Rgb {
    fn from(v: RgbIn) -> Self {
        match v {
            RgbIn::S(s) => Rgb([s; 3]),
            RgbIn::V(v) => Rgb(v),
        }
    }
}

fn one3() -> Rgb {
    Rgb([1.0; 3])
}

fn d65() -> f32 {
    6500.0
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Bloom {
    pub intensity: f32,
    pub threshold: f32,
    pub size_scale: f32,
    pub sizes: Vec<f32>,
    pub tints: Vec<f32>,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AutoExposure {
    pub low_percent: f32,
    pub high_percent: f32,
    /// Histogram range, log2 luminance. The presets use UE 5's extended range (EV100 -10 .. 20, which with the default
    /// lens attenuation 0.78 is the same log2 luminance: Scene.cpp, r.DefaultFeature.AutoExposure.
    /// ExtendDefaultLuminanceRange, on in every UE 5 project template). UE's legacy range, -8 .. 4, saturates in
    /// daylight (a 3000-nit sky would pin the exposure at EV 6.5); scenes ported from a project without the extended
    /// range set it explicitly (Kabukichō).
    pub histogram_log_min: f32,
    pub histogram_log_max: f32,
    /// Eye adaptation speed toward brighter / darker targets, EV per second (UE defaults 3 / 1; real-time only).
    #[serde(default = "three")]
    pub speed_up: f32,
    #[serde(default = "one")]
    pub speed_down: f32,
}

fn three() -> f32 {
    3.0
}

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NativePreset {
    pub spp: u32,
    pub max_bounces: u32,
    pub denoise: bool,
    pub clamp_indirect: f32,
    pub russian_roulette_depth: u32,
    #[serde(default = "one")]
    pub energy_compensation: f32,
}

fn one() -> f32 {
    1.0
}

pub fn merge_json(a: &mut serde_json::Value, b: &serde_json::Value) {
    match (a, b) {
        (serde_json::Value::Object(a), serde_json::Value::Object(b)) => {
            for (k, v) in b {
                merge_json(a.entry(k.clone()).or_insert(serde_json::Value::Null), v);
            }
        }
        (a, b) => *a = b.clone(),
    }
}
