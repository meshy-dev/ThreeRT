//! In-place edits of a built scene: the CPU copies of the GPU tables (materials, lights, objects, emitters) and the
//! engine handle -> table index maps. `Tables::apply` rewrites the entries a parameter change touches (power, light
//! colour / intensity, material parameters, wetness / rain / fog, camera); `LiveCore::commit` uploads them.

use crate::desc;
use crate::scene::{self, CameraData, GpuLight, GpuMaterial, Retained};
use crate::world::{Changes, Kind, World};
use crate::EmitterTri;
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};

/// Which uploads an `apply` needs.
#[derive(Default, Debug, Clone, Copy)]
pub struct Dirty {
    pub materials: bool,
    pub lights: bool,
    pub objects: bool,
    /// Emitter weights changed (powers): the emitter CDF and the light tree's powers are rebuilt.
    pub emitters: bool,
    /// Power edits held back (`Tables::defer_powers`): the live view's emitter worker rebuilds the CDF and the tree's
    /// powers for them, and they land together.
    pub deferred: bool,
    /// Wetness / rain / fog kernel parameters.
    pub environment: bool,
    pub camera: bool,
}

impl Dirty {
    pub fn any(&self) -> bool {
        self.materials || self.lights || self.objects || self.emitters || self.deferred || self.environment || self.camera
    }
}

pub struct Tables {
    pub retained: Retained,
    pub materials: Vec<GpuMaterial>,
    pub lights: Vec<GpuLight>,
    pub objects: Vec<[f32; 4]>,
    pub emitter_tris: Vec<EmitterTri>,
    /// The light tree over the static emitters (`ltree`): power / material edits refit its node powers.
    pub light_tree: Option<crate::ltree::LightTree>,
    /// Materials that have emitter triangles (others cannot start emitting without a rebuild).
    pub(crate) emissive: HashSet<u32>,
    pub wetness: f32,
    pub rain: f32,
    pub fog: Option<desc::Fog>,
    /// three.js `scene.background` (`Params::background`).
    pub background: Option<[f32; 3]>,
    /// A material that starts emitting gets emitter entries in place (the live view sets these up, `prepare`'s
    /// `mat_runs` / `fixed_mats`): it is queued in `started_emitting` for live.rs `add_material_emitters`, unless its
    /// triangles include pool or dynamic ones (fixed: a rebuild flattens them). Off: such a material rebuilds.
    pub emit_in_place: bool,
    pub fixed_mats: HashSet<u32>,
    pub started_emitting: Vec<u32>,
    /// `scene::aerial_params` of the sky.
    pub aerial: [f32; 4],
    pub camera: Option<CameraData>,
    /// Object -> its entries in `lights`.
    pub light_index: HashMap<u32, Vec<usize>>,
    /// vec4s an object in `objects` (`scene::object_stride`).
    pub stride: usize,
    /// Power edits are held back in `deferred` instead of written to `objects` (the live view's emitter worker: a
    /// scene with many emitters, live.rs `EmitterWorker`).
    pub defer_powers: bool,
    /// Held-back power edits: (the object's first row in `objects`, its power, the time of the edit).
    pub deferred: Vec<(usize, f32, f32)>,
}

impl Tables {
    pub fn new(sc: &scene::Scene, emitter_tris: Vec<EmitterTri>, light_tree: Option<crate::ltree::LightTree>) -> Tables {
        let mut light_index: HashMap<u32, Vec<usize>> = HashMap::new();
        for (i, n) in sc.retained.light_nodes.iter().enumerate() {
            light_index.entry(*n).or_default().push(i);
        }
        Tables {
            retained: sc.retained.clone(),
            materials: sc.materials.clone(),
            lights: sc.lights.clone(),
            objects: sc.objects.clone(),
            emissive: emitter_tris.iter().map(|e| e.mat).collect(),
            emitter_tris,
            light_tree,
            wetness: sc.wetness,
            rain: sc.rain,
            fog: sc.fog.clone(),
            background: sc.background.map(|b| b.to_array()),
            emit_in_place: false,
            fixed_mats: HashSet::new(),
            started_emitting: vec![],
            aerial: sc.aerial,
            camera: None,
            light_index,
            stride: sc.object_stride(),
            defer_powers: false,
            deferred: vec![],
        }
    }

    /// Writes power edits into `objects` (the power, and the time of the edit, the second row's w).
    pub fn put_powers(&mut self, powers: &[(usize, f32, f32)]) {
        for &(row, power, now) in powers {
            self.objects[row][3] = power;
            self.objects[row + 1][3] = now;
        }
    }

    /// Applies parameter changes; Err / Ok(None) means the scene must be rebuilt instead.
    pub fn apply(&mut self, w: &World, c: &Changes, now: f32) -> Result<Option<Dirty>> {
        if c.structural || !c.poses.is_empty() || !c.moved.is_empty() {
            return Ok(None);
        }
        let mut d = Dirty::default();
        // (The live view has added them before its geometry edits; here for the callers without one.)
        if !c.new_materials.is_empty() {
            if !self.add_materials(w, &c.new_materials) {
                return Ok(None);
            }
            d.materials = true;
        }
        for id in &c.powers {
            let o = w.objects.get(id).ok_or_else(|| anyhow!("unknown object {id}"))?;
            if let Some(&oi) = self.retained.objects.get(id) {
                let row = oi as usize * self.stride;
                if self.defer_powers {
                    self.deferred.retain(|p| p.0 != row);
                    self.deferred.push((row, o.power.max(0.0), now));
                    d.deferred = true;
                } else {
                    self.put_powers(&[(row, o.power.max(0.0), now)]);
                    d.objects = true;
                    d.emitters = true;
                }
            }
            self.update_light(w, *id, &mut d);
        }
        for id in &c.customs {
            let o = w.objects.get(id).ok_or_else(|| anyhow!("unknown object {id}"))?;
            if let Some(&oi) = self.retained.objects.get(id) {
                self.objects[oi as usize * self.stride + 2] = o.custom;
                d.objects = true;
            }
        }
        for id in &c.lights {
            let o = w.objects.get(id).ok_or_else(|| anyhow!("unknown object {id}"))?;
            if let (Some(&mat), Kind::Light(desc::Light::Rect { color, intensity, visible: true, .. })) = (self.retained.cards.get(id), &o.kind) {
                let m = scene::card_material(*color, *intensity);
                if !self.can_emit(mat, &m) {
                    return Ok(None);
                }
                self.materials[mat as usize] = m;
                d.materials = true;
            }
            self.update_light(w, *id, &mut d);
        }
        for id in &c.materials {
            let Some(&mi) = self.retained.materials.get(id) else { return Ok(None) };
            let m = w.materials.get(id).ok_or_else(|| anyhow!("unknown material {id}"))?;
            let slots = &self.retained.textures;
            let mut tex = |tid: u32, srgb: bool| slots.get(&(tid, srgb)).copied().ok_or_else(|| anyhow!("texture {tid} not built"));
            let Ok(gm) = scene::build_material(*id, m, &w.textures, &self.retained.shaders, &mut tex) else { return Ok(None) };
            if !self.can_emit(mi, &gm) {
                return Ok(None);
            }
            self.materials[mi as usize] = gm;
            d.materials = true;
        }
        if c.environment {
            self.wetness = w.environment.wetness.clamp(0.0, 1.0);
            self.rain = w.environment.rain.clamp(0.0, 1.0);
            self.fog = w.environment.fog.clone();
            self.background = w.environment.background;
            self.aerial = scene::aerial_params(&w.environment.sky);
            d.environment = true;
        }
        if c.camera {
            let Some(cid) = w.settings.camera else { return Ok(None) };
            let Some(Kind::Camera(cam)) = w.objects.get(&cid).map(|o| &o.kind) else { return Ok(None) };
            self.camera = Some(scene::camera_data(w.world_matrix(cid), cam));
            d.camera = true;
        }
        Ok(Some(d))
    }

    fn update_light(&mut self, w: &World, id: u32, d: &mut Dirty) {
        if self.retained.sun == Some(id) {
            return; // colour comes from the sky bake
        }
        let (Some(idx), Some(o)) = (self.light_index.get(&id), w.objects.get(&id)) else { return };
        let Kind::Light(l) = &o.kind else { return };
        if let Some(gl) = scene::gpu_light(l, w.world_matrix(id), o.power) {
            for &i in idx {
                self.lights[i] = gl;
            }
            d.lights = true;
        }
    }

    /// Whether a material may emit without a rebuild: it already has emitter entries, or (`emit_in_place`) its
    /// triangles can get them in place, queued in `started_emitting` (ROADMAP § 6 step 3).
    /// Materials created since the build (`Changes::new_materials`: no texture, no function), as entries after the
    /// scene's. False, and nothing added, if one of them cannot be: it emits, is glass or layered, or a cutout (the
    /// build sorts triangles by those; the caller rebuilds).
    pub fn add_materials(&mut self, w: &World, ids: &std::collections::BTreeSet<u32>) -> bool {
        let mut fresh = vec![];
        for id in ids {
            if self.retained.materials.contains_key(id) {
                continue;
            }
            let Some(m) = w.materials.get(id) else { return false };
            let Ok(gm) = scene::build_material(*id, m, &w.textures, &self.retained.shaders, &mut |_, _| Err(anyhow!("a texture"))) else { return false };
            if scene::material_class(&gm) != 0 {
                return false;
            }
            fresh.push((*id, gm));
        }
        for (id, gm) in fresh {
            let mi = self.materials.len() as u32;
            self.retained.materials.insert(id, mi);
            self.materials.push(gm);
            // (What is drawn with it are instances' triangles: it cannot start emitting in place, `can_emit`.)
            self.fixed_mats.insert(mi);
        }
        true
    }

    fn can_emit(&mut self, mi: u32, m: &GpuMaterial) -> bool {
        let lum = 0.2126 * m.emissive[0] + 0.7152 * m.emissive[1] + 0.0722 * m.emissive[2];
        if lum <= 0.0 || self.emissive.contains(&mi) {
            return true;
        }
        if !self.emit_in_place || self.fixed_mats.contains(&mi) {
            return false;
        }
        self.emissive.insert(mi);
        self.started_emitting.push(mi);
        true
    }
}
