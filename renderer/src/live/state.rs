//! What a live view holds beside the scene's data, each derived from the world by one function, at load and on an
//! edit, and replaced whole:
//! - `ViewState`: how a frame is taken and shown (the scene's camera and its lens, depth of field and exposure; the
//!   render preset). The view's, not the scene's: a rebuild of the scene's data leaves it.
//! - `Environment`: the environment's settings (`scene::EnvSettings`) and the state of its sky.
//!
//! The scene's data then knows no camera and no post state, and the environment is an object of its own, on the
//! scene's side as the scene DSL has it (`scene.environment`, `background`, fog).

use super::*;

/// Auto / local exposure settings (UE PostProcessVolume semantics).
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct ExposureSettings {
    pub(super) ae: [f32; 4],
    pub(super) ae2: [f32; 4],
    pub(super) speed: [f32; 2],
    pub(super) lx: [f32; 4],
}

impl ExposureSettings {
    fn new(exp: &crate::desc::Exposure, pp: &crate::desc::PostPreset) -> Self {
        let a = &pp.auto_exposure;
        let le = &pp.local_exposure;
        let ae2 = match exp {
            crate::desc::Exposure::Manual { ev100 } => [2f32.powf(-ev100), 0.0, 0.0, 0.0],
            crate::desc::Exposure::Auto { compensation, min_ev100, max_ev100 } => [2f32.powf(*compensation), 0.18 * 2f32.powf(*min_ev100), 0.18 * 2f32.powf(*max_ev100), 1.0],
        };
        ExposureSettings {
            ae: [a.low_percent.clamp(1.0, 99.0) / 100.0, a.high_percent.clamp(1.0, 99.0) / 100.0, a.histogram_log_min, a.histogram_log_max.max(a.histogram_log_min + 1.0)],
            ae2,
            speed: [a.speed_up.max(0.01), a.speed_down.max(0.01)],
            lx: [le.highlight_contrast, le.shadow_contrast, le.blurred_luminance_blend, le.detail_strength],
        }
    }
}

/// How a frame is taken and shown.
pub(super) struct ViewState {
    /// The scene's camera: where a view starts, and its lens (a view moved by its host keeps the lens).
    pub(super) camera: Camera,
    /// The camera's depth of field (UE: off unless it sets a focus distance): a post pass.
    pub(super) dof: Option<crate::desc::Dof>,
    /// The exposure a view starts at (auto exposure takes over from the first frame), and its settings.
    pub(super) exposure: f32,
    pub(super) exposure_settings: ExposureSettings,
    exposure_desc: crate::desc::Exposure,
    /// The render preset: the kernel's bounce limits and clamp, the post passes' settings.
    pub(super) preset: Arc<crate::desc::Preset>,
}

impl ViewState {
    /// From the scene's camera (a build's, or `scene::camera_data` of the world's) and the preset.
    pub(super) fn new(c: &scene::CameraData, preset: Arc<crate::desc::Preset>) -> ViewState {
        ViewState {
            camera: Camera { pos: c.position, fwd: c.forward, up: c.up, right: c.right, tan_half_fov: c.tan_half_fov },
            dof: c.dof.clone(),
            exposure: c.exposure.start(),
            exposure_settings: ExposureSettings::new(&c.exposure, &preset.post),
            exposure_desc: c.exposure.clone(),
            preset,
        }
    }

    /// The world's, as it is now (`preset`: the one to keep; None: the world's settings' own, read again). None: the
    /// world names no camera.
    pub(super) fn of(world: &World, presets: &Path, preset: Option<Arc<crate::desc::Preset>>) -> Result<Option<ViewState>> {
        let (Some(id), Some(c)) = (world.settings.camera, world.scene_camera()) else { return Ok(None) };
        let preset = match preset {
            Some(p) => p,
            None => Arc::new(scene::preset_of(world, presets, None)?),
        };
        Ok(Some(ViewState::new(&scene::camera_data(world.world_matrix(id), c), preset)))
    }

    /// The kernel's parameters that are the view's (`gpu::Params::set_preset`, as a build writes them).
    pub(super) fn write(&self, p: &mut gpu::Params) {
        p.set_preset(&self.preset.native, &self.exposure_desc);
    }
}

/// The environment: its settings, and its sky's state.
pub(super) struct Environment {
    pub(super) settings: scene::EnvSettings,
    pub(super) sky: SkyState,
}

/// What the view keeps of the sky that is on the GPU (`Gpu::put_env`).
pub(super) struct SkyState {
    /// The environment map's size.
    pub(super) map: (u32, u32),
    /// The camera's altitude it was baked for, m.
    pub(super) altitude: f32,
    /// The brightest texel from which a sky is light-sampled (`Prepared::env_nee_floor`).
    pub(super) floor: f32,
    /// The cloud cover it was baked with.
    pub(super) cover: f32,
    /// A dynamic sky's bakes on a worker thread (started by the sun's first move).
    pub(super) worker: Option<DynamicSky>,
}
