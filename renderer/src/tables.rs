//! In-place edits of a built scene: the CPU copies of the GPU tables (materials, lights, objects, emitters) and the
//! engine handle -> table index maps. `Tables::apply` rewrites the entries a parameter change touches (power, light
//! colour / intensity, material parameters, wetness / rain / fog, camera); `LiveCore::commit` uploads them.

use crate::desc;
use crate::scene::{self, GpuLight, GpuMaterial, Retained};
use crate::world::{Changes, Kind, World};
use crate::EmitterTri;
use anyhow::{anyhow, Result};
use std::collections::{HashMap, HashSet};

/// Which uploads an `apply` needs.
#[derive(Default, Debug, Clone, Copy)]
pub struct Dirty {
    pub materials: bool,
    pub lights: bool,
    /// A light came, went or changed class: the table was listed again in a build's order (`list_lights`).
    pub lights_listed: bool,
    /// A local light moved, or went dark or lit: the light groups are sorted again (`Tables::regroup_lights`).
    pub lights_moved: bool,
    pub objects: bool,
    /// Emitter weights changed (powers): they are made again (live/edits.rs `commit`).
    pub emitters: bool,
    /// A material's class changed (glass or not, a cutout or not): the instances with a row of one have their rows
    /// remade (live/edits.rs `remake_classes`).
    pub reclassed: bool,
}

impl Dirty {
    pub fn any(&self) -> bool {
        self.materials || self.lights || self.objects || self.emitters
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
    /// A material that starts emitting gets emitter entries in place (the live view sets this up): it is queued in
    /// `started_emitting` for live/edits.rs `add_material_emitters`, which gives them to the instances with a row of
    /// it. Off: such a material rebuilds.
    pub emit_in_place: bool,
    /// The particle systems' materials: they get no emitters in place. One that starts emitting, or whose class
    /// changes, rebuilds.
    pub moving_mats: HashSet<u32>,
    pub started_emitting: Vec<u32>,
    /// Materials whose class changed in the last `apply` (`Dirty::reclassed`).
    pub reclassed: Vec<u32>,
    /// Object -> its entries in `lights`.
    pub light_index: HashMap<u32, Vec<usize>>,
    /// vec4s an object in `objects` (`scene::object_stride`).
    pub stride: usize,
    /// Runs of the light list retired since the last commit's end (`retire`), for their records' `EMITTER_GONE`.
    pub retired: Vec<std::ops::Range<usize>>,
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
            emit_in_place: false,
            moving_mats: HashSet::new(),
            started_emitting: vec![],
            reclassed: vec![],
            light_index,
            stride: sc.object_stride(),
            retired: vec![],
        }
    }

    /// Writes power edits into `objects` (the power, and the time of the edit, the second row's w).
    /// A run of the light list retired: its emitters weigh nothing from now on (their entries and the tree keep
    /// their places), on the CPU by their area, on the GPU by their records' `EMITTER_GONE`, written at the commit's
    /// end.
    pub fn retire(&mut self, run: std::ops::Range<usize>) {
        for e in &mut self.emitter_tris[run.clone()] {
            e.area = 0.0;
        }
        if !run.is_empty() {
            self.retired.push(run);
        }
    }

    pub fn put_powers(&mut self, powers: &[(usize, f32, f32)]) {
        for &(row, power, now) in powers {
            self.objects[row][3] = power;
            self.objects[row + 1][3] = now;
        }
    }

    /// Applies parameter changes; Err / Ok(None) means the scene must be rebuilt instead.
    pub fn apply(&mut self, w: &World, c: &Changes, now: f32) -> Result<Option<Dirty>> {
        // (A texture that was replaced is the live view's to place again, before this: live/edits.rs `place_textures`.)
        if c.structural || !c.poses.is_empty() || !c.moved.is_empty() || !c.textures.is_empty() {
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
            if let Some(oi) = self.retained.object(*id) {
                let row = oi as usize * self.stride;
                // (The row of a mesh or a model that is hidden, or placed without size, has no power: its instance's emitters at
                // no weight until it comes back.)
                let unseen = matches!(o.kind, Kind::Mesh { .. } | Kind::Model { .. }) && (!w.shown(*id) || scene::placed_world(w.world_matrix(*id), glam::Vec3::ZERO).1);
                let power = if unseen { 0.0 } else { o.power.max(0.0) };
                self.put_powers(&[(row, power, now)]);
                d.objects = true;
                d.emitters = true;
            }
            self.update_light(w, *id, &mut d);
        }
        for id in &c.customs {
            let o = w.objects.get(id).ok_or_else(|| anyhow!("unknown object {id}"))?;
            if let Some(oi) = self.retained.object(*id) {
                self.objects[oi as usize * self.stride + 2] = o.custom;
                d.objects = true;
            }
        }
        for id in &c.lights {
            // (A light that is gone: its entries with it.)
            let Some(o) = w.objects.get(id) else {
                self.drop_light(*id, &mut d);
                continue;
            };
            if let (Some(mat), Kind::Light(desc::Light::Rect { color, intensity, visible: true, .. })) = (self.retained.proxies.get(id).and_then(|p| p.card), &o.kind) {
                let m = scene::card_material(*color, *intensity);
                if !self.can_emit(mat, &m) {
                    return Ok(None);
                }
                self.materials[mat as usize] = m;
                d.materials = true;
            }
            self.update_light(w, *id, &mut d);
        }
        if d.lights_listed {
            self.list_lights(w);
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
            // A material whose class changes (it turns glass or stops being glass, becomes a cutout or stops being
            // one, gets a function or loses it): the instances with a row of it say so again, in place
            // (`Dirty::reclassed`). A particle system's cannot.
            let old = &self.materials[mi as usize];
            let cutout = (gm.uv2[3] > 0.0) != (old.uv2[3] > 0.0);
            let other = (gm.tex1[3] != 0) != (old.tex1[3] != 0) || (gm.layer[0] > 0.0) != (old.layer[0] > 0.0);
            if (cutout || other) && self.moving_mats.contains(&mi) {
                return Ok(None);
            }
            if cutout || other {
                self.reclassed.push(mi);
                d.reclassed = true;
            }
            self.materials[mi as usize] = gm;
            d.materials = true;
        }
        Ok(Some(d))
    }

    /// The atmosphere's sun: its light's entries as the world has it now (its direction, its size), with the colour the
    /// atmosphere leaves it (`colour`: illuminance through the atmosphere). Whether any entry was written.
    pub fn set_sun_light(&mut self, w: &World, id: u32, colour: glam::Vec3) -> bool {
        let (Some(idx), Some(o)) = (self.light_index.get(&id), w.objects.get(&id)) else { return false };
        let Kind::Light(l) = &o.kind else { return false };
        // (The sun: hidden with a rebuild, `World::set_visible`.)
        let Some(mut gl) = scene::gpu_light(l, w.world_matrix(id), o.power) else { return false };
        gl.color = [colour.x, colour.y, colour.z, 0.0];
        for &i in idx {
            self.lights[i] = gl;
        }
        !idx.is_empty()
    }

    /// A light's entries as its description and place say (not the atmosphere's sun: `retained.sun`).
    pub fn update_light(&mut self, w: &World, id: u32, d: &mut Dirty) {
        if self.retained.sun == Some(id) {
            return; // colour comes from the sky bake
        }
        let Some(o) = w.objects.get(&id) else { return };
        let Kind::Light(l) = &o.kind else { return };
        let Some(gl) = scene::gpu_light_shown(l, w.world_matrix(id), o.power, w.shown(id)) else { return };
        match self.light_index.get(&id) {
            Some(idx) => {
                // A local light somewhere else, or lit where it was dark: the groups are sorted again
                // (`regroup_lights`); so is the table for a light of another class (directional, local, shadowed).
                let moved = idx.iter().any(|&i| self.lights[i].pos != gl.pos || (self.lights[i].color[..3] == [0.0; 3]) != (gl.color[..3] == [0.0; 3]));
                let reclassed = idx.iter().any(|&i| crate::light_class(&self.lights[i]) != crate::light_class(&gl));
                for &i in idx {
                    self.lights[i] = gl;
                }
                d.lights_moved |= (moved && gl.pos[3] != 0.0) || reclassed;
                d.lights_listed |= reclassed;
            }
            // A light that came while the scene runs: an entry after the others, sorted in among them.
            None => {
                self.light_index.insert(id, vec![self.lights.len()]);
                self.lights.push(gl);
                self.retained.light_nodes.push(id);
                (d.lights_moved, d.lights_listed) = (true, true);
            }
        }
        d.lights = true;
    }

    /// The entries of a light that is gone, out of the table (`light_index` and the retained nodes follow).
    fn drop_light(&mut self, id: u32, d: &mut Dirty) {
        let Some(idx) = self.light_index.remove(&id) else { return };
        let keep: Vec<usize> = (0..self.lights.len()).filter(|i| !idx.contains(i)).collect();
        if self.retained.light_nodes.len() == self.lights.len() {
            self.retained.light_nodes = keep.iter().map(|&i| self.retained.light_nodes[i]).collect();
        }
        self.lights = keep.iter().map(|&i| self.lights[i]).collect();
        self.light_index.clear();
        for (i, n) in self.retained.light_nodes.iter().enumerate() {
            self.light_index.entry(*n).or_default().push(i);
        }
        (d.lights, d.lights_moved, d.lights_listed) = (true, true, true);
    }

    /// The table's lights in the order a build lists them (the world's objects' order), before they are sorted into
    /// the kernel's (`regroup_lights`, which keeps that order among lights it has no reason to part): a view where
    /// lights came, went or changed class then has the table a build of its world has.
    fn list_lights(&mut self, w: &World) {
        if self.retained.light_nodes.len() != self.lights.len() {
            return;
        }
        let at: HashMap<u32, usize> = w.ordered().into_iter().enumerate().map(|(k, id)| (id, k)).collect();
        let mut order: Vec<usize> = (0..self.lights.len()).collect();
        order.sort_by_key(|&i| at.get(&self.retained.light_nodes[i]).copied().unwrap_or(usize::MAX));
        self.lights = order.iter().map(|&i| self.lights[i]).collect();
        self.retained.light_nodes = order.iter().map(|&i| self.retained.light_nodes[i]).collect();
        self.light_index.clear();
        for (i, n) in self.retained.light_nodes.iter().enumerate() {
            self.light_index.entry(*n).or_default().push(i);
        }
    }

    /// The lights in the kernel's order again (`crate::light_order`: each class in spatially compact groups), after
    /// local lights moved: a group's sphere holds every point its lights reach, and lights that have drifted apart
    /// make it large. `light_index` and the retained nodes follow.
    pub fn regroup_lights(&mut self) {
        let order = crate::light_order(&self.lights);
        if order.iter().enumerate().all(|(k, &i)| k == i) {
            return;
        }
        self.lights = order.iter().map(|&i| self.lights[i]).collect();
        if self.retained.light_nodes.len() == order.len() {
            self.retained.light_nodes = order.iter().map(|&i| self.retained.light_nodes[i]).collect();
        }
        let mut at = vec![0usize; order.len()];
        for (k, &i) in order.iter().enumerate() {
            at[i] = k;
        }
        for idx in self.light_index.values_mut() {
            for i in idx.iter_mut() {
                *i = at[*i];
            }
        }
    }

    /// Whether a material may emit without a rebuild: it already has emitter entries, or (`emit_in_place`) its
    /// triangles can get them in place, queued in `started_emitting` (ROADMAP § 6 step 3).
    /// Materials created since the build (`Changes::new_materials`: no function), as entries after the scene's,
    /// whatever their class: what an edit draws with one is an instance, whose rows say it. Their textures are the
    /// pages' by now (the live view places them first: live/edits.rs `place_textures`). False, and nothing added, if
    /// one of them cannot be made.
    pub fn add_materials(&mut self, w: &World, ids: &std::collections::BTreeSet<u32>) -> bool {
        let mut fresh = vec![];
        for id in ids {
            if self.retained.materials.contains_key(id) {
                continue;
            }
            let Some(m) = w.materials.get(id) else { return false };
            let slots = &self.retained.textures;
            let mut tex = |tid: u32, srgb: bool| slots.get(&(tid, srgb)).copied().ok_or_else(|| anyhow!("texture {tid} is not in the pages"));
            let Ok(gm) = scene::build_material(*id, m, &w.textures, &self.retained.shaders, &mut tex) else { return false };
            fresh.push((*id, gm));
        }
        for (id, gm) in fresh {
            let mi = self.materials.len() as u32;
            self.retained.materials.insert(id, mi);
            self.materials.push(gm);
        }
        true
    }

    fn can_emit(&mut self, mi: u32, m: &GpuMaterial) -> bool {
        let lum = 0.2126 * m.emissive[0] + 0.7152 * m.emissive[1] + 0.0722 * m.emissive[2];
        if lum <= 0.0 || self.emissive.contains(&mi) {
            return true;
        }
        if !self.emit_in_place || self.moving_mats.contains(&mi) {
            return false;
        }
        self.emissive.insert(mi);
        self.started_emitting.push(mi);
        true
    }
}
