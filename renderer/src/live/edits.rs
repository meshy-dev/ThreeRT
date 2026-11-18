//! The world's edits applied in place (`LiveCore::commit` and what it calls): moved instances and bodies, posed
//! skins, emitters, the sun and the sky, and geometry added, removed or changed (`edit_geometry`, `EditState`).

use super::*;

/// The run of the light list that is instance `record`'s: from its first emitter, those that name it.
fn emitter_run(tris: &[crate::EmitterTri], instances: &[scene::Instance], record: u32) -> std::ops::Range<usize> {
    let first = instances.get(record as usize).map_or(0, |i| i.first_emitter as usize).min(tris.len());
    first..first + tris[first..].iter().take_while(|e| e.inst == record).count()
}

/// Writes to one buffer gathered during an edit and sent merged: adjacent ranges as one `write_buffer` (an edit writes
/// hundreds of small records and meshes, contiguous in the room). Overlapping writes go in the order they came.
#[derive(Default)]
pub(super) struct Gather {
    pub(super) parts: Vec<(u64, Vec<u8>)>,
}

impl Gather {
    pub(super) fn put(&mut self, at: u64, bytes: &[u8]) {
        self.parts.push((at, bytes.to_vec()));
    }

    /// Into a heap's section (offsets from its start).
    pub(super) fn flush_section<S: Section + std::fmt::Debug>(self, q: &wgpu::Queue, heap: &Heap<S>, s: S) {
        let end = self.parts.iter().map(|p| p.0 + p.1.len() as u64).max().unwrap_or(0);
        assert!(end <= heap.cap(s), "{s:?}: {end} bytes written, the section holds {}", heap.cap(s));
        self.flush_at(q, &heap.buffer, heap.at(s));
    }

    pub(super) fn flush(self, q: &wgpu::Queue, b: &wgpu::Buffer) {
        self.flush_at(q, b, 0);
    }

    fn flush_at(self, q: &wgpu::Queue, b: &wgpu::Buffer, base: u64) {
        let mut parts = self.parts;
        let mut sorted = parts.clone();
        sorted.sort_by_key(|p| p.0);
        if sorted.windows(2).any(|w| w[0].0 + w[0].1.len() as u64 > w[1].0) {
            for (at, bytes) in parts.drain(..) {
                q.write_buffer(b, base + at, &bytes);
            }
            return;
        }
        let mut run: Option<(u64, Vec<u8>)> = None;
        for (at, bytes) in sorted {
            match &mut run {
                Some((start, data)) if *start + data.len() as u64 == at => data.extend_from_slice(&bytes),
                _ => {
                    if let Some((start, data)) = run.take() {
                        q.write_buffer(b, base + start, &data);
                    }
                    run = Some((at, bytes));
                }
            }
        }
        if let Some((start, data)) = run {
            q.write_buffer(b, base + start, &data);
        }
    }
}

/// Where an in-place edit's CPU time goes (ms) and what it did: building the triangles' GPU form, uploading them,
/// making its BLASes (software traversal: its trees; their build is the commit's, `LiveCore::build_ms`); meshes added
/// and rewritten in place, objects added or removed. Frame log `edit_split`.
#[derive(Default, Clone, Copy, Debug)]
pub struct EditSplit {
    pub tris: f64,
    pub upload: f64,
    pub blas: f64,
    pub added: usize,
    pub deformed: usize,
    pub objects: usize,
}

impl EditSplit {
    pub(super) fn add(&mut self, o: &EditSplit) {
        (self.tris, self.upload, self.blas) = (self.tris + o.tris, self.upload + o.upload, self.blas + o.blas);
        (self.added, self.deformed, self.objects) = (self.added + o.added, self.deformed + o.deformed, self.objects + o.objects);
    }
}

/// The edit room in use (gpu.rs `EditRoom`): the next free triangle (GPU order), kernel record, object-table entry,
/// procedural word (counted from the region's start) and TLAS instance, and where they started.
#[derive(Default)]
pub(super) struct EditState {
    /// Triangles (GPU order), kernel records (a record's TLAS instance is `inst_base` + it) and object-table entries
    /// edits can place: holes left by what they no longer draw, then the room, which grows past its end (ROADMAP § 6
    /// step 5).
    pub(super) tris: Holes,
    pub(super) records: Holes,
    pub(super) objects: Holes,
    /// The next procedural word, and where the region (grown as needed) ends.
    pub(super) word: usize,
    pub(super) word_cap: usize,
    /// How many objects draw each instanced mesh (an instanced object counts once), the meshes edits may free
    /// (own-BLAS meshes: shared only by objects of one geometry and material), and the slots of freed ones, reused by
    /// the next new mesh.
    pub(super) users: std::collections::HashMap<u32, u32>,
    pub(super) managed: std::collections::HashSet<u32>,
    pub(super) dead_meshes: Vec<u32>,
    /// Times the room grew (frame log `grows`), and whether the scene bind group must be made again.
    pub(super) grows: u32,
    pub(super) rebind: bool,
    /// The uniform tables (`gpu::Gpu::uniform_tables`): an edit placed entries past what their arrays hold, and it
    /// was said (once a scene, as its upload says it of the scene's own).
    pub(super) uniform_said: bool,
    /// (triangles, records, objects, words, TLAS instances) the scene itself has: the room starts there.
    pub(super) base: (usize, usize, usize, usize, usize),
    /// The first pool triangle (instanced meshes' triangles are counted from it).
    pub(super) pool_first: usize,
    /// Procedural blobs in the region by hash: their first word (the build's and those edits wrote).
    pub(super) blobs: std::collections::HashMap<[u8; 32], usize>,
}

impl EditState {
    /// Whether `record` is one of the room's: an edit placed what is there (the build's are below).
    pub(super) fn in_room(&self, record: usize) -> bool {
        record >= self.base.1
    }
}

/// Ranges of one kind of the edit room: free holes (start -> length; first fit, neighbours merged) below `end`, the
/// part in use, which may reach `cap`, what the buffers hold (more by growing them).
#[derive(Default, Debug)]
pub(super) struct Holes {
    pub(super) free: std::collections::BTreeMap<usize, usize>,
    pub(super) end: usize,
    pub(super) cap: usize,
    /// Where the room starts (the scene's own data before it).
    pub(super) base: usize,
}

impl Holes {
    pub(super) fn new(end: usize, cap: usize) -> Holes {
        Holes { free: Default::default(), end, cap, base: end }
    }

    /// `n` contiguous: the first hole that fits, else at the end if within `cap`.
    pub(super) fn take(&mut self, n: usize) -> Option<usize> {
        if let Some((&at, &len)) = self.free.iter().find(|(_, &len)| len >= n) {
            self.free.remove(&at);
            if len > n {
                self.free.insert(at + n, len - n);
            }
            return Some(at);
        }
        (self.end + n <= self.cap).then(|| {
            self.end += n;
            self.end - n
        })
    }

    /// `n` from `at` free again: merged with the holes beside it; at the end, the end moves back.
    pub(super) fn give(&mut self, at: usize, n: usize) {
        if n == 0 {
            return;
        }
        let (mut a, mut len) = (at, n);
        if let Some((&pa, &pl)) = self.free.range(..at).next_back().filter(|(&pa, &pl)| pa + pl == at) {
            self.free.remove(&pa);
            (a, len) = (pa, len + pl);
        }
        if let Some(nl) = self.free.remove(&(at + n)) {
            len += nl;
        }
        if a + len == self.end {
            self.end = a;
        } else {
            self.free.insert(a, len);
        }
    }

    /// In use (below `end`, less the holes).
    pub(super) fn used(&self) -> usize {
        self.end - self.free.values().sum::<usize>()
    }
}

/// What an edit places at an object's record: a procedural object with its BLAS (hardware traversal), or an instance
/// of a mesh.
pub(super) enum Placed {
    Proc(wgpu::Blas, scene::ProcObject),
    Mesh(scene::Instance),
}

/// One object fewer draws instanced mesh `k`: a mesh edits manage that none draws any more is freed (its triangles a
/// hole the next new mesh can take, its slot reused, its BLAS dropped with it).
pub(super) fn drop_mesh_user(k: u32, edit: &mut EditState, tables: &mut tables::Tables, meshes: &[scene::InstMesh]) {
    let n = edit.users.entry(k).or_default();
    *n = n.saturating_sub(1);
    if *n == 0 && edit.managed.remove(&k) {
        let im = meshes[k as usize];
        edit.tris.give(edit.pool_first + im.first as usize, im.count as usize);
        tables.retained.own_mesh.retain(|_, v| *v != k);
        edit.dead_meshes.push(k);
    }
}

/// The room's buffers past their end, doubled at least (ROADMAP § 6 step 5): new buffers, the old contents copied on
/// the GPU; the scene bind group is made again after the edit (`EditState::rebind`). Not with particles, whose pass
/// keeps a bind group of its own over these buffers.
pub(super) struct Grow<'a> {
    pub(super) g: &'a gpu::Gpu,
    pub(super) ok: bool,
}

impl Grow<'_> {
    /// The room twice as large (the scene's own data before it stays as it is), at least `extra` more at the end.
    pub(super) fn cap(h: &Holes, extra: usize) -> usize {
        h.base + ((h.cap - h.base) * 2).max(h.end + extra - h.base).max(1024)
    }

    /// Whether buffers of these sizes (bytes) can be bound (the device's storage binding and buffer limits): past them
    /// the edit rebuilds instead (Bugbot on #131).
    pub(super) fn fits(&self, sizes: &[u64]) -> bool {
        sizes.iter().all(|&s| self.g.buffer_fits(s))
    }

    /// Triangles: vertices, attributes, the BLASes' vertices, and in a scene with them (`colors`) vertex colours.
    pub(super) fn tris(&self, h: &mut Holes, extra: usize, verts: &mut wgpu::Buffer, attrs: &mut wgpu::Buffer, words: &mut Heap<Stream>, colors: bool, accel: &mut gpu::Accel) -> bool {
        if !self.ok {
            return false;
        }
        let cap = Self::cap(h, extra);
        let color_bytes = (cap * 12) as u64;
        if !self.fits(&[(cap * 48) as u64, (cap * std::mem::size_of::<scene::GpuTriAttr>()) as u64, (cap * 36) as u64, if colors { words.size_with(Stream::VertexColors, color_bytes) } else { 0 }]) {
            return false;
        }
        *verts = self.g.grown(verts, (cap * 48) as u64, "verts");
        *attrs = self.g.grown(attrs, (cap * std::mem::size_of::<scene::GpuTriAttr>()) as u64, "attrs");
        self.g.grow_blas_verts(accel, (cap * 36) as u64);
        if colors {
            words.grow(&self.g.device, &self.g.queue, Stream::VertexColors, color_bytes);
        }
        h.cap = cap;
        true
    }

    /// A table's section to `bytes` (heap.rs `Heap::grow`), if the device binds the buffer that makes.
    fn table(&self, heap: &mut Heap<Table>, t: Table, bytes: u64) -> bool {
        if !self.fits(&[heap.size_with(t, bytes)]) {
            return false;
        }
        heap.grow(&self.g.device, &self.g.queue, t, bytes);
        true
    }

    /// Kernel records and the TLAS (a record's instance is `inst_base` + it).
    pub(super) fn records(&self, h: &mut Holes, extra: usize, heap: &mut Heap<Table>, accel: &mut gpu::Accel) -> bool {
        if !self.ok {
            return false;
        }
        let cap = Self::cap(h, extra);
        if !self.table(heap, Table::Instances, (cap * std::mem::size_of::<scene::GpuInstance>()) as u64) {
            return false;
        }
        self.g.grow_tlas(accel, accel.inst_base + cap);
        h.cap = cap;
        true
    }

    /// Object-table entries (`stride` vec4s each).
    pub(super) fn objects(&self, h: &mut Holes, extra: usize, heap: &mut Heap<Table>, stride: usize) -> bool {
        if !self.ok {
            return false;
        }
        let cap = Self::cap(h, extra);
        if !self.table(heap, Table::Objects, (cap * stride * 16) as u64) {
            return false;
        }
        h.cap = cap;
        true
    }

    /// The procedural region's words.
    pub(super) fn words(&self, cap: &mut usize, need: usize, words: &mut Heap<Stream>) -> bool {
        if !self.ok {
            return false;
        }
        let new = (*cap * 2).max(need);
        if !self.fits(&[words.size_with(Stream::Procs, (new * 4) as u64)]) {
            return false;
        }
        *cap = new;
        words.grow(&self.g.device, &self.g.queue, Stream::Procs, (new * 4) as u64);
        true
    }
}

/// An object removed or hidden in place is no longer drawn: hidden where its record's value is (`LiveCore::pick`
/// skips it). The mesh it was an instance of, if it was drawn as one: a user fewer (`drop_mesh_user`).
pub(super) fn undraw(draw: Option<&scene::Draw>, instances: &mut [scene::Instance], procs: &mut [scene::ProcObject]) -> Option<u32> {
    match draw? {
        scene::Draw::Instance(i) => instances.get_mut(i.instance as usize).and_then(|i| (!std::mem::replace(&mut i.hidden, true)).then_some(i.mesh)),
        scene::Draw::Procedural(j) => {
            if let Some(p) = procs.get_mut(*j as usize) {
                p.hidden = true;
            }
            None
        }
        _ => None,
    }
}

impl LiveCore {
    /// Applies what scene code changed in the world since the last commit. Parameter changes (power, light / material
    /// parameters, wetness / rain / fog, the camera) rewrite GPU table entries in place; anything structural rebuilds
    /// the GPU scene from the world (history survives, validated per pixel by depth / normal). Returns true on a
    /// rebuild.
    pub fn commit(&mut self, world: &mut World) -> Result<bool> {
        self.cpu_skin_ms = 0.0;
        self.cpu_upload_ms = 0.0;
        self.gpu_blas_ms = 0.0;
        // What the edits below leave to build above the triangles: built once, at the end (`build_top`).
        let mut top = gpu::TopBuild::default();
        // The game's particle spawns, for the next frame's pass; the voxel occupancy grid's changes, uploaded.
        for (id, v) in std::mem::take(&mut world.particle_spawns) {
            self.particle_spawns.entry(id).or_default().extend(v);
        }
        if let Some(o) = &world.voxel_occupancy {
            let mut o = o.write().unwrap();
            if !o.dirty.is_empty() {
                self.g.upload_occupancy(&self.sc.up, &mut o);
            }
        }
        // The density grids: the game's sources for the next frame's grid pass, and new data uploaded in place.
        for (id, v) in std::mem::take(&mut world.grid_sources) {
            self.grid_sources.entry(id).or_default().extend(v);
        }
        for id in std::mem::take(&mut world.grid_dirty) {
            if let Some(d) = world.grids.get(&id).and_then(|g| g.data.as_ref()) {
                self.g.upload_grid(&self.sc.up, id, d);
            }
        }
        let mut changes = world.take_changes();
        if !changes.any() {
            return Ok(false);
        }
        // Objects shown or hidden by their own `visible`: in place when the build has them (a rain curtain the weather
        // turns off, a pool's idle fireball and its light cost nothing while off), else with the rebuild.
        if !changes.structural {
            for id in std::mem::take(&mut changes.shown) {
                let hide = !world.shown(id);
                let done = match world.objects.get(&id).map(|o| &o.kind) {
                    Some(crate::world::Kind::Particles { .. }) => self.g.set_particles_hidden(&self.sc.up, id, hide),
                    Some(crate::world::Kind::Volume { shape, .. }) => self.g.set_medium_hidden(&self.sc.up, id, *shape, hide),
                    // (Its table entry, black while hidden: `Tables::update_light`.)
                    Some(crate::world::Kind::Light(_)) if self.sc.tables.light_index.contains_key(&id) => {
                        changes.lights.insert(id);
                        true
                    }
                    _ => false,
                };
                if !done {
                    changes.structural = true;
                }
            }
        }
        // New materials without textures: entries after the scene's, before the edits that place objects drawn with
        // them (and the GPU's table with them, grown if it is full).
        if !changes.new_materials.is_empty() && !changes.structural {
            if self.sc.tables.add_materials(world, &changes.new_materials) {
                self.upload_materials();
                // (A clear coat, an unlit material: the kernel variant with it, compiled once.)
                self.sc.up.kernel.features = self.kernel_features();
            } else {
                self.fallback = Some("a new material that cannot be added in place (it emits, is glass or a cutout)".into());
                self.fallbacks += 1;
                changes.structural = true;
            }
        }
        // An object an edit placed (its record is the room's) moves in place only as an instance does: a mesh a body
        // drives, while it is drawn. Any other rebuilds.
        let stuck = |id: &u32| match self.sc.tables.retained.proxies.get(id).map(|p| &p.draw) {
            Some(scene::Draw::Instance(i)) => self.sc.edit.in_room(i.instance as usize) && !(i.moves && self.sc.instances.get(i.instance as usize).is_some_and(|i| !i.hidden)),
            Some(scene::Draw::Procedural(j)) => self.sc.edit.in_room(self.sc.proc_first + *j as usize),
            _ => false,
        };
        if !changes.structural && changes.moved.iter().any(stuck) {
            self.fallback = Some("an object added in place moved".into());
            self.fallbacks += 1;
            changes.structural = true;
            if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") {
                eprintln!("edit: rebuild (an object added in place moved)");
            }
        }
        // Instanced objects rewrite their instances in place (`update_instanced`); one that cannot rebuilds the scene.
        // (The instances the last update moved, taken first: what this one moves, from here on, keeps its motion.)
        let moved_before = std::mem::take(&mut self.inst_moving);
        let mut instanced_early = false;
        // (Not those added this commit: the geometry edits place them with their instances as they are.)
        let early: Vec<u32> = std::mem::take(&mut changes.instanced).into_iter().filter(|id| !changes.added.contains(id)).collect();
        if !early.is_empty() && !changes.structural {
            instanced_early = self.instanced_in_place(world, &early, &mut changes, &mut top);
        }
        // Geometry edits in place when nothing else is structural; what they cannot apply rebuilds the scene.
        if changes.edits() && !changes.structural {
            let t = Instant::now();
            let r = self.edit_geometry(world, &changes, &mut top);
            if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") {
                eprintln!("edit: {} geometries, {} added, {} removed: {}", changes.geometries.len(), changes.added.len(), changes.removed.len(), r.as_ref().map_or_else(|e| format!("rebuild ({e})"), |_| "in place".into()));
            }
            match r {
                Ok(()) => {
                    self.edit_ms += t.elapsed().as_secs_f32() * 1e3;
                    // As after a rebuild: history validated per pixel.
                    self.lit_changed = self.frame.max(1);
                }
                Err(why) => {
                    self.fallback = Some(why);
                    self.fallbacks += 1;
                    changes.structural = true;
                }
            }
        }
        if !changes.promoted_geometry.is_empty() {
            eprintln!("live: {}: geometry changed, now with its own BLAS (this once rebuilds; set `dynamicGeometry` or the attribute's DynamicDrawUsage to avoid it)", changes.promoted_geometry.join(", "));
            self.fallback = Some("promote".into());
            self.fallbacks += 1;
        }
        if !changes.promoted.is_empty() {
            eprintln!("live: {} moved, now movable (this once rebuilds; set `movable = true` in the scene code to avoid it)", changes.promoted.join(", "));
        }
        // Moved movable subtrees: their drawables are re-transformed with the posed models, their lights rewritten in
        // the light table.
        let mut dynamic = changes.poses.clone();
        let mut placed = vec![];
        let mut procs_moved = vec![];
        let mut instanced_moved = vec![];
        let children = if changes.moved.is_empty() { Default::default() } else { world.children() };
        for id in &changes.moved {
            for k in world.subtree_with(*id, &children) {
                match &world.objects[&k].kind {
                    crate::world::Kind::Light(crate::desc::Light::Rect { visible: true, .. }) | crate::world::Kind::Mesh { .. } | crate::world::Kind::Model { .. } => {
                        if world.shown(k) {
                            // Movable models placed as instances move their transform (with a shared pose too); the rest
                            // re-transform triangles.
                            let inst = self.sc.tables.retained.instance(k).filter(|i| i.moves).copied();
                            match inst.map(|i| i.instance) {
                                Some(i) if !changes.poses.contains(&k) || inst.is_some_and(|i| matches!(i.mesh, scene::MeshKind::Pose(_))) => placed.push((k, i)),
                                _ => {
                                    dynamic.insert(k);
                                }
                            }
                        }
                    }
                    crate::world::Kind::Light(_) => {
                        changes.lights.insert(k);
                    }
                    // Particle functions place their particles around the node (`i.pivot`).
                    crate::world::Kind::Particles { .. } => {
                        let p = world.world_matrix(k).transform_point3(glam::Vec3::ZERO);
                        self.g.set_particle_pivot(&self.sc.up, k, p.to_array());
                    }
                    crate::world::Kind::GpuParticles { .. } => {
                        let p = world.world_matrix(k).transform_point3(glam::Vec3::ZERO);
                        self.g.set_gpu_particle_pivot(&self.sc.up, k, p.to_array());
                    }
                    // Media: the box / sphere follows the node.
                    crate::world::Kind::Volume { .. } => {
                        self.g.set_medium(&self.sc.up, k, Some(scene::medium_rows(world.world_matrix(k))), None);
                    }
                    crate::world::Kind::Procedural { .. } if world.shown(k) => procs_moved.push(k),
                    crate::world::Kind::Instanced { .. } => instanced_moved.push(k),
                    _ => {}
                }
            }
        }
        // Instanced objects under a moved ancestor (or movable themselves) rewrite their instances too, unless the update
        // above did (with the same world matrices; twice would take the new rows for the previous frame's: no motion).
        let mut instanced_done = instanced_early;
        instanced_moved.retain(|k| !early.contains(k));
        if !instanced_moved.is_empty() && !changes.structural {
            instanced_done |= self.instanced_in_place(world, &instanced_moved, &mut changes, &mut top);
        }
        // Posed rigid models move their nodes' instances (`repose_rigid`), not triangles.
        let rigid: Vec<u32> = dynamic.iter().copied().filter(|k| self.sc.tables.retained.rigid(*k).is_some()).collect();
        for k in &rigid {
            dynamic.remove(k);
        }
        // Models in a shared pose re-skin their group's mesh (`repose_groups`, its BLAS only); their moves place instances.
        let shared: Vec<u32> = dynamic.iter().copied().filter(|k| self.sc.tables.retained.instance(*k).is_some_and(|i| matches!(i.mesh, scene::MeshKind::Pose(_)))).collect();
        for k in &shared {
            dynamic.remove(k);
        }
        let shared_ok = shared.is_empty() || (!changes.structural && self.repose_groups(world, &shared, &mut top)?);
        // Procedural geometry moves its records and TLAS transforms.
        if !procs_moved.is_empty() && !changes.structural {
            self.move_procs(world, &procs_moved, &mut top);
        }
        let placed_ok = shared_ok && (placed.is_empty() || (!changes.structural && self.move_instances(world, &placed, &mut top)));
        let t_rigid = Instant::now();
        let rigid_ok = placed_ok && (rigid.is_empty() || (!changes.structural && self.repose_rigid(world, &rigid, &mut top)?));
        self.cpu_skin_ms += t_rigid.elapsed().as_secs_f32() * 1000.0;
        // Instances the last update moved and this one does not: their motion ends, the previous-frame rows catch up
        // (written merged: an instanced object's thousands of records are contiguous).
        if moved_before != self.inst_moving {
            let size = std::mem::size_of::<scene::GpuInstance>() as u64;
            let now_moving: std::collections::HashSet<u32> = self.inst_moving.iter().copied().collect();
            let mut catch_up = Gather::default();
            for i in moved_before.into_iter().filter(|i| !now_moving.contains(i)) {
                if let Some(r) = self.sc.gpu_instances.get_mut(i as usize) {
                    r.p = r.m;
                    catch_up.put(i as u64 * size, bytemuck::bytes_of(r));
                }
            }
            catch_up.flush_section(&self.g.queue, &self.sc.up.heap, Table::Instances);
        }
        if rigid_ok && (dynamic.is_empty() || (!changes.structural && self.repose(world, &dynamic, &mut top)?)) {
            // Moved emitters: their triangles in the list and the light tree's moving subtree around them, once (after
            // every in-place update).
            if std::mem::take(&mut self.emitters_moved) {
                self.move_emitters();
            }
            changes.poses.clear();
            changes.moved.clear();
            // Moving objects move their shadows and reflections over static surfaces: let anti-lag look for it.
            if !dynamic.is_empty() || !placed.is_empty() || !rigid.is_empty() || !shared.is_empty() || !procs_moved.is_empty() || instanced_done {
                self.lit_changed = self.frame.max(1);
            }
        }
        // Particle nodes' customData: their particle function's `i.custom` (the table has no object for the node).
        for id in &changes.customs {
            if let Some(o) = world.objects.get(id).filter(|o| matches!(o.kind, crate::world::Kind::Particles { .. })) {
                self.g.set_particle_custom(&self.sc.up, *id, o.custom);
            }
        }
        // Volume nodes' customData: their volume function's `i.custom` (the media table, not the objects table).
        for id in &changes.customs {
            if let Some(o) = world.objects.get(id).filter(|o| matches!(o.kind, crate::world::Kind::Volume { .. })) {
                self.g.set_medium(&self.sc.up, *id, None, Some(o.custom));
            }
        }
        // What is not the scene's data (state.rs), each derived again whole from the world, before the tables' edits. The
        // environment's settings, then its sky (one whose map cannot grow in place rebuilds), then the view (a world
        // that names no camera rebuilds, and the build says so).
        if changes.environment && !changes.structural {
            self.env.settings = scene::EnvSettings::of(world);
            self.env.settings.write(&mut self.sc.base);
        }
        if changes.sky && !changes.structural && !self.apply_sky(world)? {
            self.fallback = Some("a sky whose map does not fit the scene's buffers in place".into());
            self.fallbacks += 1;
            changes.structural = true;
        }
        if (changes.camera || changes.view || changes.settings) && !changes.structural && !self.apply_view(world, &changes)? {
            changes.structural = true;
        }
        let now = shader_time(self.time);
        // The tables' edits, then what the edits above left to build, once. Either may say the scene is to be rebuilt,
        // and the rebuild builds all of it.
        let mut applied = self.sc.tables.apply(world, &changes, now).ok().flatten();
        if applied.is_some() && !self.build_top(top) {
            applied = None;
        }
        let Some(d) = applied else {
            let t = Instant::now();
            self.rebuild(world)?;
            self.rebuild_ms += t.elapsed().as_secs_f32() * 1e3;
            if changes.camera {
                self.cam = self.view.camera;
            }
            return Ok(true);
        };
        if d.materials {
            self.upload_materials();
        }
        let (q, up, e) = (&self.g.queue, &self.sc.up, &mut self.sc.tables);
        if d.lights && !e.lights.is_empty() {
            if d.lights_moved {
                e.regroup_lights();
            }
            up.put_lights(q, &e.lights);
        }
        if d.objects {
            up.heap.write(q, Table::Objects, 0, bytemuck::cast_slice(&e.objects));
        }
        if d.materials || d.emitters || std::mem::take(&mut self.emitters_reweigh) {
            let new = self.add_material_emitters();
            self.write_emitters(&new);
        } else if d.deferred {
            self.send_powers();
        }
        if d.materials || changes.environment {
            // (Fog or a background appearing, a clear coat: the kernel's variant with it, compiled once, then cached.)
            self.sc.up.kernel.features = self.kernel_features();
        }
        // Lighting changed: for 45 frames the histories hold HISTORY_LIVE frames at most (`live_lighting`), and where the
        // change is strong the temporal pass cuts the history locally (anti-lag).
        if d.materials || d.lights || d.objects || changes.environment {
            self.lit_changed = self.frame.max(1);
        }
        // (A new cloud cover re-bakes the sky and dims the sun as a sun move does.)
        if changes.sun || self.env.settings.cloud_cover != self.env.sky.cover {
            self.env.sky.cover = self.env.settings.cloud_cover;
            self.move_sun(world);
        }
        Ok(false)
    }

    /// What a commit's edits left to build above the triangles, in one build (`Gpu::build_top`). False: software
    /// traversal's top tree cannot be made, and the caller rebuilds the scene. Its CPU time is `build_ms`;
    /// THREERT_PROFILE_BLAS=1: its GPU time (the queue drained before and after; frame log `blas`).
    fn build_top(&mut self, b: gpu::TopBuild) -> bool {
        if !b.any() {
            return true;
        }
        let profile = std::env::var("THREERT_PROFILE_BLAS").is_ok_and(|v| v == "1");
        if profile {
            let _ = self.g.device.poll(wgpu::PollType::wait_indefinitely());
        }
        let t = Instant::now();
        let built = self.g.build_top(&mut self.sc.up, b);
        self.build_ms += t.elapsed().as_secs_f32() * 1e3;
        if profile {
            let _ = self.g.device.poll(wgpu::PollType::wait_indefinitely());
            self.gpu_blas_ms += t.elapsed().as_secs_f32() * 1e3;
        }
        self.top_builds += 1;
        if let Err(why) = &built {
            eprintln!("units: {why}: rebuilding the scene");
            self.fallback = Some(why.clone());
            self.fallbacks += 1;
        }
        built.is_ok()
    }

    /// The kernel features the scene needs as it is now: what its materials and the environment ask for (edits change
    /// them), with the structural ones the build found (textures, material functions, the traversal groups;
    /// `gpu::instance_features`). A new variant is compiled once, then cached.
    pub(super) fn kernel_features(&self) -> u32 {
        let (textures, functions, groups) = self.sc.fixed_features;
        gpu::features_of(&self.sc.tables.materials, self.env.settings.features(), textures, functions, groups, self.sc.instance_features)
    }

    /// The view's state from the world as it is now (`ViewState::of`: the scene's camera with its lens, depth of field
    /// and exposure; the preset, read again when the settings changed), the kernel's parameters it says, and with
    /// another preset the post passes' settings. The view takes the scene's camera when that moved, changed its lens
    /// or is another one (`Changes::camera`); one its host moved stays through the rest. False: the world names no
    /// camera.
    pub(super) fn apply_view(&mut self, world: &World, ch: &crate::world::Changes) -> Result<bool> {
        let keep = (!ch.settings).then(|| self.view.preset.clone());
        let Some(view) = ViewState::of(world, &self.presets, keep)? else { return Ok(false) };
        self.view = view;
        self.view.write(&mut self.sc.base);
        if ch.settings {
            self.apply_post();
            self.scene_name = world.settings.name.clone();
            self.lit_changed = self.frame.max(1);
        }
        if ch.camera {
            self.cam = self.view.camera;
        }
        Ok(true)
    }

    /// The atmosphere's sun moved (time of day): its light entry is rewritten now (direction, and its colour through
    /// the atmosphere), and the sky's re-bake is asked of the worker thread (`DynamicSky`, started here on the first
    /// move). Lockstep frames wait for the bake, so a lockstep run sees the sky that goes with each sun.
    pub(super) fn move_sun(&mut self, world: &World) {
        let Some((id, dir, lux)) = scene::atmosphere_sun(world) else { return };
        if self.env.sky.worker.is_none() {
            self.env.sky.worker = DynamicSky::new(world, self.env.sky.map, self.env.sky.altitude);
        }
        let Some(sky) = self.env.sky.worker.as_ref() else { return };
        let cover = self.env.settings.cloud_cover;
        let colour = crate::sky::sun_illuminance(&sky.atm, dir, lux, cover);
        let e = &mut self.sc.tables;
        if e.set_sun_light(world, id, colour) {
            self.sc.up.put_lights(&self.g.queue, &e.lights);
        }
        let _ = sky.ask.send((dir, lux, cover));
        if self.fixed_dt.is_some() {
            if let Ok(b) = sky.done.recv() {
                self.put_sky(b);
            }
        }
        self.lit_changed = self.frame.max(1);
    }

    /// Another sky (its kind, its parameters, its sun), as a build takes it: baked here, its environment map and CDF
    /// put in place (of another size: a new texture, the scene bound again), the parameters that say how it is read,
    /// and the lights it owns: the atmosphere's sun takes its colour from the bake, a light that is the sun no longer
    /// its own again. False, with nothing changed, when the map's CDF cannot grow in place (`Gpu::put_env`).
    pub(super) fn apply_sky(&mut self, world: &World) -> Result<bool> {
        let (sky, sun) = scene::sky_data(world, &self.assets, &|id| world.world_matrix(id))?;
        let altitude = world.settings.camera.map_or(self.env.sky.altitude, |id| world.world_matrix(id).w_axis.y.max(0.0));
        let cover = self.env.settings.cloud_cover;
        let baked = crate::bake_sky(&sky, cover, altitude);
        if !self.g.put_env(&mut self.sc.up, &baked.env, &crate::build_env_cdf(&baked.env)) {
            return Ok(false);
        }
        let floor = self.env.sky.floor;
        baked.write(&mut self.sc.base, floor);
        // The lights: what was the sun, and what is.
        let e = &mut self.sc.tables;
        let was = std::mem::replace(&mut e.retained.sun, sun);
        let mut d = tables::Dirty::default();
        if let Some(id) = was.filter(|id| Some(*id) != sun) {
            e.update_light(world, id, &mut d);
        }
        // (Whole, not its colour alone: the tables no longer update a light that is the sun, and it may have been
        // turned in this commit.)
        if let (Some(id), Some(c)) = (sun, baked.sun) {
            d.lights |= e.set_sun_light(world, id, c);
        }
        if d.lights && !e.lights.is_empty() {
            self.sc.up.put_lights(&self.g.queue, &e.lights);
        }
        // (The worker that re-bakes a dynamic sky is this sky's no longer: the next move of the sun starts one.)
        self.env.sky = SkyState { map: (baked.env.width, baked.env.height), altitude, floor, cover, worker: None };
        self.lit_changed = self.frame.max(1);
        Ok(true)
    }

    /// A sky the worker thread has baked since the last frame (the newest), into the environment map and its CDF.
    pub(super) fn take_sky(&mut self) {
        let Some(sky) = self.env.sky.worker.as_ref() else { return };
        let mut last = None;
        while let Ok(b) = sky.done.try_recv() {
            last = Some(b);
        }
        if let Some(b) = last {
            self.put_sky(b);
        }
    }

    pub(super) fn put_sky(&mut self, (env, cdf, max): (gpu::EnvMap, Vec<f32>, f32)) {
        // (The worker bakes at the map's size: written in place.)
        if self.env.sky.worker.is_none() || !self.g.put_env(&mut self.sc.up, &env, &cdf) {
            return;
        }
        // As the load decides it: the sky is light-sampled unless it is negligible next to the emitters (night), or a
        // baked sky is not light-sampled at all (`sky_sampled`).
        self.sc.base.counts2[1] = crate::sky_sampled(max, self.env.sky.floor, true) as u32;
        self.lit_changed = self.frame.max(1);
    }

    /// Moves movable instances: their kernel records (the old transform kept as the previous frame's, for motion
    /// vectors), their object entries' pivots and their TLAS transforms (hardware traversal) or their units (software
    /// traversal, units.rs); the commit builds what is above them (`build_top`). No triangles change. False when it
    /// cannot (software traversal without units, or an instance that is no unit): the caller rebuilds.
    pub(super) fn move_instances(&mut self, world: &World, placed: &[(u32, u32)], build: &mut gpu::TopBuild) -> bool {
        // (Hardware traversal's TLAS instances, or software traversal's units; neither: the caller rebuilds.)
        let Some(mut top) = gpu::Top::of(&mut self.sc.up.accel, &mut self.sc.up.units) else { return false };
        let size = std::mem::size_of::<scene::GpuInstance>() as u64;
        let q = &self.g.queue;
        // (Many records, a rigid body world's thousands a frame, are written as the one range that holds them: the
        // records between are as the GPU has them.)
        let merged = placed.len() > 32;
        let (mut lo, mut hi) = (usize::MAX, 0usize);
        // (Instances the last update moved and this one does not catch up in `commit`.)
        for &(node, i) in placed {
            let w = world.world_matrix(node);
            let inst = &mut self.sc.instances[i as usize];
            // (Software traversal: an instance of a mesh that deforms is no unit; its triangles are among the still
            // scene's, in the world, and a rebuild moves them.)
            if !top.has_mesh(inst.mesh) {
                return false;
            }
            let old = self.sc.gpu_instances[i as usize];
            inst.world = w;
            let mut r = top.record(inst.gpu(old.info[0], old.info[3]), inst.mesh, inst.cast_shadow);
            r.p = old.m;
            self.sc.gpu_instances[i as usize] = r;
            if merged {
                (lo, hi) = (lo.min(i as usize), hi.max(i as usize));
            } else {
                self.sc.up.heap.write(q, Table::Instances, i as u64 * size, bytemuck::bytes_of(&r));
            }
            let pivot = w.transform_point3(glam::Vec3::ZERO);
            let st = self.sc.tables.stride;
            let o = &mut self.sc.tables.objects[inst.object as usize * st];
            (o[0], o[1], o[2]) = (pivot.x, pivot.y, pivot.z);
            let slot = top.inst_base() + i as usize;
            top.moved(slot, w, None);
            self.inst_moving.push(i);
        }
        if merged {
            self.sc.up.heap.write(q, Table::Instances, lo as u64 * size, bytemuck::cast_slice(&self.sc.gpu_instances[lo..=hi]));
        }
        self.sc.up.heap.write(q, Table::Objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
        self.fill_moved(placed.iter().map(|&(_, i)| i));
        self.inst_moved = true;
        build.instances = true;
        true
    }

    /// The material table to the GPU: written in place; with more materials than its section holds (materials added
    /// while the game runs), the section is made twice the size first and the scene's bind groups made again.
    pub(super) fn upload_materials(&mut self) {
        let bytes: &[u8] = bytemuck::cast_slice(&self.sc.tables.materials);
        if bytes.len() as u64 > self.sc.up.heap.cap(Table::Materials) {
            self.sc.up.heap.grow(&self.g.device, &self.g.queue, Table::Materials, bytes.len() as u64 * 2);
            self.g.rebind_scene(&mut self.sc.up);
        }
        self.sc.up.heap.write(&self.g.queue, Table::Materials, 0, bytes);
    }

    /// `update_instanced` with its fallback (the reason in the frame log, the scene rebuilt) and its CPU time; whether it
    /// changed anything.
    pub(super) fn instanced_in_place(&mut self, world: &World, ids: &[u32], changes: &mut crate::world::Changes, build: &mut gpu::TopBuild) -> bool {
        let t = Instant::now();
        let r = self.update_instanced(world, ids, build);
        self.instanced_ms += t.elapsed().as_secs_f32() * 1e3;
        let log = std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1");
        match r {
            Ok(n) => {
                if log {
                    eprintln!("edit: {n} instanced objects in place");
                }
                n > 0
            }
            Err(why) => {
                if log {
                    eprintln!("edit: rebuild ({why})");
                }
                self.fallback = Some(why);
                self.fallbacks += 1;
                changes.structural = true;
                false
            }
        }
    }

    /// Emitter entries for the materials that started emitting (`Tables::started_emitting`): their flattened static
    /// triangles (`mat_runs`), positions from `tris`, in the power-share part of the CDF (path code 0, as the light
    /// tree's moving emitters; ROADMAP § 6 step 3). Returns them (each one's index and triangle) for `write_emitters`.
    pub(super) fn add_material_emitters(&mut self) -> Vec<(u32, crate::GpuEmitterTri)> {
        let sc = &mut self.sc;
        let mut new = vec![];
        for mat in std::mem::take(&mut sc.tables.started_emitting) {
            for &[first, end, obj] in sc.mat_runs.get(&mat).map_or(&[][..], |v| v.as_slice()) {
                for k in first..end {
                    let Some(&[a, b, c]) = sc.tri_order.get(k as usize).and_then(|&i| sc.tris.get(i as usize)) else { continue };
                    let area = 0.5 * (b - a).cross(c - a).length();
                    if area > 0.0 {
                        let centre = (a + b + c) / 3.0;
                        let e = crate::EmitterTri { tri: k, mat, obj, area, c: centre.to_array(), moves: false, nominal: false, in_tree: false, inst: crate::NO_INST };
                        new.push((sc.tables.emitter_tris.len() as u32, crate::GpuEmitterTri::of(&e, [a, b, c], 0)));
                        sc.tables.emitter_tris.push(e);
                    }
                }
            }
        }
        new
    }

    /// The emitter CDF from `Tables::emitter_tris` (weights from materials and powers; entries added in place at its
    /// end, removed ones at area 0), the light tree refit to the new weights and its nodes after the CDF, the CDF's
    /// share of power, its count. With the tree's slot for emitters added in place (scenes that can edit geometry), their
    /// subtree is grown again first and their entries go right after the static ones (ROADMAP § 6 step 4); without it
    /// they stay at the end, by power share. `new`: the emitters made or made again since the last call (each one's
    /// index and triangle): their triangles into the list, their entries into their triangles' attributes. The tables
    /// grow (and the scene bind group is made again) when entries were added past them: a scene that adds none keeps
    /// its layout.
    pub(super) fn write_emitters(&mut self, new: &[(u32, crate::GpuEmitterTri)]) {
        self.land_powers_now();
        let g = &self.g;
        let LoadedScene { tables: e, base: b, up, pool_tris, tris: flat, tri_order, edit, instances, .. } = &mut self.sc;
        let size = std::mem::size_of::<crate::GpuEmitterTri>();
        let (mut w_tris, mut w_attrs) = (Gather::default(), Gather::default());
        for (i, t) in new {
            w_tris.put((*i as usize * size) as u64, bytemuck::bytes_of(t));
            // (A triangle in world space names its emitter; a mesh's is numbered among its mesh's.)
            if t.inst == crate::NO_INST {
                w_attrs.put((t.tri as usize * std::mem::size_of::<scene::GpuTriAttr>() + std::mem::offset_of!(scene::GpuTriAttr, x) + 12) as u64, bytemuck::bytes_of(&(i + 1)));
            }
        }
        w_attrs.flush(&g.queue, &up.attrs);
        // Emitters added in place, with the tree's slot for them (`ltree::build_slotted`): its subtree grown again over
        // those alive, their entries right after the static ones in the CDF (the tree's order: `emitter_order`) and
        // their path codes with their triangles in the list (written over the code 0 they were made with).
        if let Some(t) = e.light_tree.as_mut() {
            if t.dynamic.is_some() {
                let (mats, objs, st, pf) = (&e.materials, &e.objects, e.stride, edit.pool_first);
                // (An emitter's triangle in world space: an instance's is its mesh's, placed.)
                let verts = |x: &crate::EmitterTri| {
                    let k = x.tri as usize;
                    let t = if k >= pf { pool_tris.get(k - pf).copied() } else { tri_order.get(k).and_then(|&i| flat.get(i as usize)).copied() };
                    match instances.get(x.inst as usize) {
                        Some(i) if x.inst != crate::NO_INST => t.map(|t| t.map(|v| glam::Vec3A::from(i.world.transform_point3(v.into())))),
                        _ => t,
                    }
                };
                let mut items = vec![];
                for (i, x) in e.emitter_tris.iter().enumerate() {
                    let w = crate::emitter_power(x, mats, objs, st);
                    let Some([va, vb, vc]) = verts(x).filter(|_| !x.in_tree && !x.nominal && !x.moves && w > 0.0) else { continue };
                    items.push(crate::ltree::DynItem { e: i as u32, lo: va.min(vb).min(vc).to_array(), hi: va.max(vb).max(vc).to_array(), c: x.c, w });
                }
                t.set_dynamic(items);
            }
        }
        let order = crate::emitter_order(&e.emitter_tris, e.light_tree.as_ref());
        // Where each emitter's weight is now (`emitter_codes`), for all but the tree's static ones (theirs stay):
        // written over the code the new ones were made with.
        let codes = crate::emitter_codes(&e.emitter_tris, &order, e.light_tree.as_ref());
        let fixed = e.light_tree.as_ref().map_or(0, |t| t.dynamic.map_or(t.order.len(), |(_, ns)| ns as usize));
        for &i in &order[fixed.min(order.len())..] {
            w_tris.put((i as usize * size + std::mem::offset_of!(crate::GpuEmitterTri, code)) as u64, bytemuck::bytes_of(&codes[i as usize]));
        }
        let (em, total, power) = crate::emitter_cdf(&e.emitter_tris, &order, &e.materials, &e.objects, e.stride, b.light_lod, crate::EmitterCull::of(b));
        if let Some(t) = e.light_tree.as_mut() {
            let (tris, mats, objs, st) = (&e.emitter_tris, &e.materials, &e.objects, e.stride);
            let w: Vec<f32> = t.order.iter().map(|&i| crate::emitter_power(&tris[i as usize], mats, objs, st)).collect();
            t.refit(&|k| w[k]);
        }
        self.put_emitters(&em, total, power);
        // (After `put_emitters`: the list's room for them is there.)
        w_tris.flush_section(&self.g.queue, &self.sc.up.heap, Table::EmitterTris);
        // Emitters added in place since the worker was made: another, over them.
        if self.sc.emit.as_ref().is_some_and(|w| w.count != self.sc.tables.emitter_tris.len()) {
            self.sc.emit = EmitterWorker::new(&self.sc.tables, &self.g.device);
            self.sc.tables.defer_powers = self.sc.emit.is_some();
        }
    }

    /// The emitter CDF `em` (its total weight and the emitters' power) and the light tree's nodes into their tables,
    /// and the kernel's parameters. The tables, and the one of the emitters' triangles, are grown (and the scene's bind
    /// groups made again) when they no longer hold the list.
    pub(super) fn put_emitters(&mut self, em: &[[f32; 4]], total: f32, power: f32) {
        let g = &self.g;
        let LoadedScene { tables: e, up, .. } = &mut self.sc;
        let nodes: &[[f32; 4]] = e.light_tree.as_ref().map_or(&[], |t| &t.nodes);
        let tris = e.emitter_tris.len() * std::mem::size_of::<crate::GpuEmitterTri>();
        let mut grew = false;
        for (t, need) in [(Table::Emitters, em.len() * 16), (Table::LightTree, nodes.len() * 16), (Table::EmitterTris, tris)] {
            let need = need as u64;
            if need > up.heap.cap(t) {
                up.heap.grow(&g.device, &g.queue, t, need + need / 2);
                grew = true;
            }
        }
        up.heap.write(&g.queue, Table::Emitters, 0, bytemuck::cast_slice(em));
        up.heap.write(&g.queue, Table::LightTree, 0, bytemuck::cast_slice(nodes));
        if grew {
            g.rebind_scene(up);
        }
        self.emitter_params(total, power);
    }

    /// The kernel's emitter parameters for a CDF of this total weight (and the emitters' power) and the tree.
    pub(super) fn emitter_params(&mut self, total: f32, power: f32) {
        let LoadedScene { tables: e, base: b, .. } = &mut self.sc;
        b.emit_power[0] = power;
        b.misc[1] = total;
        if let Some(t) = e.light_tree.as_ref() {
            // The tree's entries (with the slot: the static ones and those added in place), the rest by power share.
            if t.dynamic.is_some() {
                b.ltree[1] = f32::from_bits(t.order.len() as u32);
            }
            b.ltree[2] = if power > 0.0 { ((power - t.power) / power).max(0.0) } else { 0.0 };
        }
        b.counts[1] = if total > 0.0 { e.emitter_tris.len() as u32 } else { 0 };
    }

    /// Power edits held back (`Tables::deferred`) to the emitter worker, unless it is on a job (they go with the next,
    /// when that one lands). Lockstep frames wait for the result.
    pub(super) fn send_powers(&mut self) {
        let Some(w) = self.sc.emit.as_mut() else { return };
        if w.flight.is_some() || self.sc.tables.deferred.is_empty() {
            return;
        }
        let e = &mut self.sc.tables;
        let powers = std::mem::take(&mut e.deferred);
        let mut objects = e.objects.clone();
        for &(row, power, _) in &powers {
            objects[row][3] = power;
        }
        let b = &self.sc.base;
        let job = EmitterJob { materials: e.materials.clone(), objects, lod: b.light_lod, cull: crate::EmitterCull::of(b) };
        if w.ask.send(job).is_err() {
            // (The worker is gone: write them now.)
            e.put_powers(&powers);
            self.sc.emit = None;
            e.defer_powers = false;
            self.sc.up.heap.write(&self.g.queue, Table::Objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
            self.write_emitters(&[]);
            return;
        }
        w.flight = Some(powers);
        if self.fixed_dt.is_some() {
            if let Ok(r) = self.sc.emit.as_ref().unwrap().done.recv() {
                self.land_emitters(r);
            }
        }
    }

    /// The emitter worker's result, if it has one: its edits land with it, and the edits held back since go out.
    pub(super) fn take_emitters(&mut self) {
        let Some(r) = self.sc.emit.as_ref().and_then(|w| w.done.try_recv().ok()) else { return };
        self.land_emitters(r);
        self.send_powers();
    }

    /// A job's edits and its emitter CDF and tree powers, together: the objects' powers, the CDF, the tree.
    pub(super) fn land_emitters(&mut self, r: EmitterBuilt) {
        let Some(powers) = self.sc.emit.as_mut().and_then(|w| w.flight.take()) else { return };
        let e = &mut self.sc.tables;
        e.put_powers(&powers);
        self.sc.up.heap.write(&self.g.queue, Table::Objects, 0, bytemuck::cast_slice(&e.objects));
        if let Some(t) = e.light_tree.as_mut() {
            for (n, &p) in t.nodes.iter_mut().zip(&r.node_powers) {
                n[2] = p;
            }
            t.power = t.nodes.first().map_or(0.0, |n| n[2]);
        }
        // (Same entries and nodes as the CDF the scene was loaded with: their tables hold them.)
        let mut enc = self.g.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("emitters") });
        let (heap, cdf) = (&self.sc.up.heap, r.entries as u64 * 16);
        for (t, from, bytes) in [(Table::Emitters, 0, cdf), (Table::LightTree, cdf, r.bytes - cdf)] {
            if bytes > 0 {
                enc.copy_buffer_to_buffer(&r.staging, from, &heap.buffer, heap.at(t), bytes.min(heap.cap(t)));
            }
        }
        self.g.queue.submit([enc.finish()]);
        self.emitter_params(r.total, r.power);
        self.lit_changed = self.frame.max(1);
    }

    /// Before another emitter edit (`write_emitters`, a light-LOD rebuild): the job in flight waited for and its result
    /// dropped, its edits and those held back written into the objects (and uploaded), so the rebuild sees them all.
    pub(super) fn land_powers_now(&mut self) {
        let Some(w) = self.sc.emit.as_mut() else { return };
        let mut powers = vec![];
        if let Some(f) = w.flight.take() {
            let _ = w.done.recv();
            powers = f;
        }
        let e = &mut self.sc.tables;
        powers.append(&mut e.deferred);
        if !powers.is_empty() {
            e.put_powers(&powers);
            self.sc.up.heap.write(&self.g.queue, Table::Objects, 0, bytemuck::cast_slice(&e.objects));
            // The lighting changed (as when a result lands): `live_lighting` for the next 45 frames.
            self.lit_changed = self.frame.max(1);
        }
    }

    /// Instanced objects whose instances changed (`World::instanced`, the node's transform, power or customData): their
    /// kernel records (the old transforms kept as the previous frame's, for motion vectors), object entries (pivot,
    /// bounds, power, data, colour), TLAS transforms and masks (`scene::instance_world`: a hidden instance 0), each
    /// object's records and entries written as one range; the commit builds what is above them. No triangles or
    /// BLASes change. Copies that emit: their emitters in the light list follow (`fill_moved`), a hidden copy's object
    /// without power, and the emitters are weighed again when a copy's size or power changed (`emitters_reweigh`).
    /// Returns how many objects it updated; Err: why it cannot (software traversal, an object the build
    /// flattened or did not place this way), and the caller rebuilds.
    /// `THREERT_EDIT_REBUILD=1` always rebuilds (A/B).
    pub(super) fn update_instanced(&mut self, world: &World, ids: &[u32], build: &mut gpu::TopBuild) -> std::result::Result<usize, String> {
        use crate::world::Kind;
        if std::env::var("THREERT_EDIT_REBUILD").is_ok_and(|v| v == "1") {
            return Err("THREERT_EDIT_REBUILD=1".into());
        }
        let size = std::mem::size_of::<scene::GpuInstance>();
        let now = shader_time(self.time);
        let empty = crate::world::InstanceTable::default();
        let sc = &mut self.sc;
        let Some(mut top) = gpu::Top::of(&mut sc.up.accel, &mut sc.up.units) else { return Err("software traversal".into()) };
        let mut done = 0;
        let (mut emitting, mut reweigh): (Vec<u32>, bool) = (vec![], false);
        for &id in ids {
            // (Removed or hidden: nothing is drawn; a structural change rebuilds.)
            let Some(n) = world.objects.get(&id).filter(|_| world.shown(id)) else { continue };
            let Kind::Instanced { capacity, procedural, .. } = n.kind else { continue };
            let Some(&place) = sc.tables.retained.instanced(id) else { return Err("an instanced mesh the build flattened".into()) };
            if place.capacity != capacity || place.mesh.is_none() != procedural {
                return Err("an instanced mesh the build placed otherwise".into());
            }
            let t = world.instanced.get(&id).unwrap_or(&empty);
            let m = world.world_matrix(id);
            // Past both the old and the new count every instance is hidden and stays so: left as it is.
            let (first, cap) = (place.first as usize, place.drawn.max(t.count).min(capacity) as usize);
            // (A procedural one's hidden instances are out of hardware traversal by their TLAS masks; software
            // traversal walks every record there is: the build leaves them out.)
            if procedural && top.sw() {
                return Err("a procedural instanced mesh (software traversal)".into());
            }
            // (A record's slot is `inst_base` + it, a procedural copy's record the first procedural one + its index.)
            let rec0 = if procedural { sc.proc_first + first } else { first };
            let slot0 = top.inst_base() + rec0;
            for i in 0..cap {
                let (wm, hidden) = scene::instance_world(m, t, i);
                let old = sc.gpu_instances[rec0 + i];
                let mut emits = false;
                let (mut rec, entry) = if procedural {
                    let p = &mut sc.procs[first + i];
                    (p.world, p.hidden) = (wm, hidden);
                    (p.gpu(old.info[0]), p.entry(n.power, t.custom(i, n.custom), t.color(i)))
                } else {
                    let inst = &mut sc.instances[first + i];
                    if !top.has_mesh(inst.mesh) {
                        return Err("an instanced mesh that is no unit (software traversal)".into());
                    }
                    (inst.world, inst.hidden) = (wm, hidden);
                    let im = sc.meshes[inst.mesh as usize];
                    emits = im.emitters > 0;
                    let (lo, hi) = im.placed_bounds(wm);
                    let row = scene::object_row(wm.transform_point3(glam::Vec3::ZERO), n.power, hi - lo, t.custom(i, n.custom), t.color(i));
                    if emits {
                        // Its emitters as it is placed now (their triangles on the GPU: `fill_moved` below).
                        let run = emitter_run(&sc.tables.emitter_tris, &sc.instances, (rec0 + i) as u32);
                        let inst = &sc.instances[first + i];
                        for e in &mut sc.tables.emitter_tris[run] {
                            let (area, c, _) = crate::placed_emitter(inst, &sc.pool_tris[e.tri as usize - sc.edit.pool_first], e.mat, &sc.tables.materials);
                            reweigh |= (area - e.area).abs() > 1e-5 * area.max(e.area);
                            (e.area, e.c) = (area, c);
                        }
                        emitting.push((rec0 + i) as u32);
                    }
                    let inst = &sc.instances[first + i];
                    (top.record(inst.gpu(old.info[0], old.info[3]), inst.mesh, n.cast_shadow), scene::copy_row(row, emits && hidden))
                };
                rec.p = old.m;
                sc.gpu_instances[rec0 + i] = rec;
                // The time of the last power edit: now for a new power.
                let st = sc.tables.stride;
                let at = (place.object as usize + i) * st;
                let e = &mut sc.tables.objects[at..at + st];
                reweigh |= emits && e[0][3] != entry[0][3];
                let edited = if e[0][3] != entry[0][3] { now } else { e[1][3] };
                e.copy_from_slice(&entry[..st]);
                e[1][3] = edited;
                top.moved(slot0 + i, wm, Some((n.cast_shadow, hidden)));
            }
            let q = &self.g.queue;
            sc.up.heap.write(q, Table::Instances, (rec0 * size) as u64, bytemuck::cast_slice(&sc.gpu_instances[rec0..rec0 + cap]));
            let st = sc.tables.stride;
            let at = place.object as usize * st;
            sc.up.heap.write(q, Table::Objects, (at * 16) as u64, bytemuck::cast_slice(&sc.tables.objects[at..at + cap * st]));
            self.inst_moving.extend(rec0 as u32..(rec0 + cap) as u32);
            if let Some(scene::Draw::Instanced(place)) = sc.tables.retained.proxies.get_mut(&id).map(|p| &mut p.draw) {
                place.drawn = t.count;
            }
            done += 1;
        }
        if done > 0 {
            self.inst_moved = true;
            build.instances = true;
        }
        self.fill_moved(emitting.into_iter());
        self.emitters_reweigh |= reweigh;
        Ok(done)
    }

    /// Moves procedural objects: their kernel records (the old transform kept as the previous frame's, for motion
    /// vectors), object pivots and, with hardware RT, TLAS transforms (the commit builds the TLAS). Software
    /// traversal reads the records alone, so neither path rebuilds the scene.
    pub(super) fn move_procs(&mut self, world: &World, ids: &[u32], build: &mut gpu::TopBuild) {
        let size = std::mem::size_of::<scene::GpuInstance>() as u64;
        for id in ids {
            let Some(&scene::Draw::Procedural(j)) = self.sc.tables.retained.proxies.get(id).map(|p| &p.draw) else { continue };
            let j = j as usize;
            let w = world.world_matrix(*id);
            let p = &mut self.sc.procs[j];
            p.world = w;
            let i = self.sc.proc_first + j;
            let old = self.sc.gpu_instances[i];
            let mut r = p.gpu(old.info[0]);
            r.p = old.m;
            self.sc.gpu_instances[i] = r;
            self.sc.up.heap.write(&self.g.queue, Table::Instances, i as u64 * size, bytemuck::bytes_of(&r));
            let pivot = w.transform_point3(glam::Vec3::ZERO);
            let st = self.sc.tables.stride;
            let o = &mut self.sc.tables.objects[p.object as usize * st];
            (o[0], o[1], o[2]) = (pivot.x, pivot.y, pivot.z);
            if let Some(accel) = self.sc.up.accel.as_mut() {
                if let Some(t) = accel.tlas[accel.inst_base + i].as_mut() {
                    t.transform = gpu::tlas_rows(w);
                }
            }
            self.inst_moving.push(i as u32);
        }
        self.sc.up.heap.write(&self.g.queue, Table::Objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
        // (Software traversal reads the records alone: no unit moved.)
        build.instances |= self.sc.up.accel.is_some();
        self.inst_moved = true;
    }

    /// Posed rigid models (`scene::RigidModel`): each moving node's instance takes object x node transform from the clip
    /// (previous rows kept for motion vectors) and their flattened triangles are re-transformed in place; the commit
    /// builds the dynamic BLASes (if they have such triangles) and the TLAS. False when it cannot (no hardware RT, a
    /// model hidden or changed): the caller rebuilds.
    pub(super) fn repose_rigid(&mut self, world: &World, ids: &[u32], build: &mut gpu::TopBuild) -> Result<bool> {
        if self.sc.up.accel.is_none() {
            return Ok(false);
        }
        let mut placements: Vec<(u32, glam::Mat4)> = vec![];
        let (mut lo, mut hi) = (usize::MAX, 0usize);
        for id in ids {
            let Some(o) = world.objects.get(id) else { return Ok(false) };
            let crate::world::Kind::Model { uri, animation, .. } = &o.kind else { return Ok(false) };
            if !world.shown(*id) {
                return Ok(false);
            }
            let globals = scene::model_gltf(uri, &self.assets)?.node_globals(animation.as_ref())?;
            let m = world.world_matrix(*id);
            let r = self.sc.tables.retained.rigid(*id).expect("a rigid model (`commit`)");
            placements.extend(r.instances.iter().map(|&(node, i)| (i, m * globals[node])));
            let d = &r.rest;
            if d.finals.len() != d.mats.len() {
                return Ok(false);
            }
            let (first, verts, attrs, blas) = &mut self.sc.dynamic;
            for (j, (node, p, n, uv, tg)) in r.rest_local.iter().enumerate() {
                let (pw, nw, uvw, tw) = scene::placed_tri(m * globals[*node], p, n, uv, *tg);
                let a = scene::tri_attr(pw, nw, uvw, tw, d.mats[j]);
                let k = d.finals[j] as usize - *first;
                let (e1, e2) = (pw[1] - pw[0], pw[2] - pw[0]);
                verts[k * 3..k * 3 + 3].copy_from_slice(&[[pw[0].x, pw[0].y, pw[0].z, f32::from_bits(d.flags[j])], [e1.x, e1.y, e1.z, 0.0], [e2.x, e2.y, e2.z, 0.0]]);
                attrs[k] = scene::GpuTriAttr { x: [a.x[0], f32::from_bits(d.object), a.x[2], attrs[k].x[3]], ..a };
                blas[k * 3..k * 3 + 3].copy_from_slice(&scene::blas_tri(pw.map(|v| v.to_array())));
                self.sc.tris[(d.first + j as u32) as usize] = pw.map(glam::Vec3A::from);
                (lo, hi) = (lo.min(k), hi.max(k + 1));
            }
            let pivot = m.transform_point3(glam::Vec3::ZERO);
            let o = &mut self.sc.tables.objects[d.object as usize * self.sc.tables.stride];
            (o[0], o[1], o[2]) = (pivot.x, pivot.y, pivot.z);
        }
        // The instances: rows, previous rows (motion vectors), TLAS transforms.
        let size = std::mem::size_of::<scene::GpuInstance>() as u64;
        let accel = self.sc.up.accel.as_mut().unwrap();
        let (mut ilo, mut ihi) = (usize::MAX, 0usize);
        for &(i, w) in &placements {
            let inst = &mut self.sc.instances[i as usize];
            let old = self.sc.gpu_instances[i as usize];
            inst.world = w;
            let mut r = inst.gpu(old.info[0], old.info[3]);
            r.p = old.m;
            self.sc.gpu_instances[i as usize] = r;
            (ilo, ihi) = (ilo.min(i as usize), ihi.max(i as usize + 1));
            let row = |k: usize| [w.x_axis[k], w.y_axis[k], w.z_axis[k], w.w_axis[k]];
            if let Some(t) = accel.tlas[accel.inst_base + i as usize].as_mut() {
                t.transform = [row(0), row(1), row(2)].concat().try_into().unwrap();
            }
            // (Rigid models' instances are theirs alone: no other update this commit placed them.)
            self.inst_moving.push(i);
        }
        if lo < hi {
            self.emitters_moved = true;
        }
        // A model's node instances are contiguous: one write.
        if ilo < ihi {
            self.sc.up.heap.write(&self.g.queue, Table::Instances, ilo as u64 * size, bytemuck::cast_slice(&self.sc.gpu_instances[ilo..ihi]));
        }
        self.fill_moved(placements.iter().map(|&(i, _)| i));
        self.inst_moved = true;
        self.sc.up.heap.write(&self.g.queue, Table::Objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
        if lo < hi {
            // Motion vectors of the flattened triangles: the previous frame's vertices, as `repose` keeps them.
            let both = self.prev_span.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi)));
            self.keep_prev(both);
            self.prev_span = Some((lo, hi));
            self.reposed = true;
            let (q, up, (first, verts, attrs, blas)) = (&self.g.queue, &self.sc.up, &self.sc.dynamic);
            let accel = up.accel.as_ref().unwrap();
            let k = (first + lo) as u64;
            q.write_buffer(&up.verts, k * 48, bytemuck::cast_slice(&verts[lo * 3..hi * 3]));
            q.write_buffer(&up.attrs, k * 96, bytemuck::cast_slice(&attrs[lo..hi]));
            q.write_buffer(&accel.verts, k * 36, bytemuck::cast_slice(&blas[lo * 3..hi * 3]));
        }
        build.instances = true;
        build.dynamic |= lo < hi;
        Ok(true)
    }

    /// Models in a shared pose (`scene::PoseGroup`): each group whose pose changed is skinned once, in model space (the
    /// groups in parallel), into its mesh's pool triangles; the commit builds their BLASes and the TLAS over their
    /// new bounds. False when a group's members no longer share one pose, or one is hidden or changed:
    /// the caller rebuilds, which groups them again. (Deforming meshes get no motion vectors of their own yet: an
    /// instance's are its transform's.)
    pub(super) fn repose_groups(&mut self, world: &World, members: &[u32], build: &mut gpu::TopBuild) -> Result<bool> {
        use rayon::prelude::*;
        if self.sc.up.accel.is_none() {
            return Ok(false);
        }
        let r = &self.sc.tables.retained;
        let group = |k: &u32| match r.instance(*k).map(|i| i.mesh) {
            Some(scene::MeshKind::Pose(g)) => g,
            _ => unreachable!("a pose group's member (`commit`)"),
        };
        let mut touched: Vec<u32> = members.iter().map(group).collect();
        touched.sort_unstable();
        touched.dedup();
        let mut jobs: Vec<(u32, crate::desc::Animation)> = vec![];
        for &g in &touched {
            let pg = &r.pose_groups[g as usize];
            let mut pose: Option<&crate::desc::Animation> = None;
            for (id, _) in &pg.members {
                let Some(o) = world.objects.get(id) else { return Ok(false) };
                let crate::world::Kind::Model { uri, nodes, animation: Some(a), .. } = &o.kind else { return Ok(false) };
                if !world.shown(*id) || *uri != pg.uri || *nodes != pg.nodes || pose.is_some_and(|p| p != a) {
                    return Ok(false);
                }
                pose = Some(a);
            }
            if let Some(a) = pose.filter(|a| **a != pg.animation) {
                jobs.push((g, a.clone()));
            }
        }
        if jobs.is_empty() {
            return Ok(true);
        }
        let t_skin = Instant::now();
        let assets = &self.assets;
        type Skinned = (Vec<[f32; 4]>, Vec<scene::GpuTriAttr>, Vec<[f32; 3]>, Vec<[glam::Vec3A; 3]>);
        let skinned: Vec<Result<Option<Skinned>>> = jobs
            .par_iter()
            .map(|(g, a)| {
                let pg = &r.pose_groups[*g as usize];
                let tris = scene::model_gltf(&pg.uri, assets)?.triangles(glam::Mat4::IDENTITY, pg.nodes.as_deref(), Some(a))?;
                if tris.len() != pg.order.len() {
                    return Ok(None);
                }
                let n = pg.order.len();
                let (mut v, mut at, mut b, mut t) = (Vec::with_capacity(n * 3), Vec::with_capacity(n), Vec::with_capacity(n * 3), Vec::with_capacity(n));
                for &(k, mat) in &pg.order {
                    let q = &tris[k as usize];
                    let (e1, e2) = (q.p[1] - q.p[0], q.p[2] - q.p[0]);
                    // The flag word keeps TRI_DYNAMIC: the kernel takes a deforming mesh's motion from `prev_verts`.
                    v.extend_from_slice(&[[q.p[0].x, q.p[0].y, q.p[0].z, f32::from_bits(scene::TRI_DYNAMIC)], [e1.x, e1.y, e1.z, 0.0], [e2.x, e2.y, e2.z, 0.0]]);
                    at.push(scene::tri_attr(q.p, q.n, q.uv, q.tangent, mat));
                    b.extend(scene::blas_tri(q.p.map(|p| p.to_array())));
                    t.push(q.p.map(glam::Vec3A::from));
                }
                Ok(Some((v, at, b, t)))
            })
            .collect();
        self.cpu_skin_ms += t_skin.elapsed().as_secs_f32() * 1000.0;
        let t_up = Instant::now();
        // Motion vectors: the GPU still holds the previous pose of these meshes (and of the last re-skin's, if others):
        // keep it in `prev_verts` before uploading the new one.
        let df = self.sc.dynamic.0;
        let (mut lo, mut hi) = (usize::MAX, 0usize);
        for (g, _) in &jobs {
            let pg = &self.sc.tables.retained.pose_groups[*g as usize];
            let k = self.sc.gpu_instances[pg.members[0].1 as usize].info[0] as usize - df;
            (lo, hi) = (lo.min(k), hi.max(k + pg.order.len()));
        }
        self.keep_prev(self.pose_span.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi))));
        self.pose_span = Some((lo, hi));
        self.pose_reposed = true;
        let (q, up) = (&self.g.queue, &self.sc.up);
        let accel = up.accel.as_ref().unwrap();
        let mut meshes = Vec::with_capacity(jobs.len());
        for ((g, a), res) in jobs.into_iter().zip(skinned) {
            let Some((v, at, b, t)) = res? else { return Ok(false) };
            let pg = &mut self.sc.tables.retained.pose_groups[g as usize];
            // The mesh's first triangle in the GPU order: any member's record has it.
            let k = self.sc.gpu_instances[pg.members[0].1 as usize].info[0] as u64;
            q.write_buffer(&up.verts, k * 48, bytemuck::cast_slice(&v));
            q.write_buffer(&up.attrs, k * 96, bytemuck::cast_slice(&at));
            q.write_buffer(&accel.verts, k * 36, bytemuck::cast_slice(&b));
            let m = self.sc.meshes[pg.mesh as usize];
            self.sc.pool_tris[m.first as usize..(m.first + m.count) as usize].copy_from_slice(&t);
            pg.animation = a;
            meshes.push(pg.mesh);
        }
        build.meshes.extend(meshes);
        self.cpu_upload_ms += t_up.elapsed().as_secs_f32() * 1000.0;
        Ok(true)
    }

    /// Re-skins posed models and re-transforms movable objects in place (hardware RT): their triangles and shading
    /// attributes; the commit builds the dynamic BLASes and the TLAS. Their directly seen surfaces keep a short history for a moment,
    /// like a power edit (no per-object motion vectors). False when it cannot (not dynamic at build, no hardware RT, a
    /// different triangle count): the caller rebuilds.
    pub(super) fn repose(&mut self, world: &World, ids: &std::collections::HashSet<u32>, build: &mut gpu::TopBuild) -> Result<bool> {
        let dynamic = |p: &mut scene::Proxy| match &mut p.draw {
            scene::Draw::Dynamic(d) => Some(std::mem::take(d)),
            _ => None,
        };
        // (Their models out of the proxies while `reskin` has the scene, then back.)
        let proxies = &mut self.sc.tables.retained.proxies;
        let models: std::collections::HashMap<u32, scene::DynamicModel> = ids.iter().filter_map(|id| Some((*id, dynamic(proxies.get_mut(id)?)?))).collect();
        let t_skin = Instant::now();
        let res = if self.sc.up.accel.is_none() || models.len() < ids.len() { Ok(None) } else { self.reskin(world, ids, &models) };
        self.cpu_skin_ms += t_skin.elapsed().as_secs_f32() * 1000.0;
        for (id, d) in models {
            self.sc.tables.retained.proxy(id).draw = scene::Draw::Dynamic(d);
        }
        let Some((lo, hi)) = res? else { return Ok(false) };
        self.emitters_moved = true;
        let t_up = Instant::now();
        // Motion vectors: the GPU still holds the previous frame's vertices for this span (and for last update's span,
        // if different): keep them as `prev_verts` before uploading the new ones.
        let both = self.prev_span.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi)));
        self.keep_prev(both);
        self.prev_span = Some((lo, hi));
        self.reposed = true;
        // Upload the touched span of the dynamic tail; the commit builds the dynamic BLASes and the TLAS.
        let (q, up, (first, verts, attrs, blas)) = (&self.g.queue, &self.sc.up, &self.sc.dynamic);
        let accel = up.accel.as_ref().unwrap();
        let k = (first + lo) as u64;
        q.write_buffer(&up.verts, k * 48, bytemuck::cast_slice(&verts[lo * 3..hi * 3]));
        q.write_buffer(&up.attrs, k * 96, bytemuck::cast_slice(&attrs[lo..hi]));
        q.write_buffer(&accel.verts, k * 36, bytemuck::cast_slice(&blas[lo * 3..hi * 3]));
        up.heap.write(q, Table::Objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
        build.dynamic = true;
        self.cpu_upload_ms += t_up.elapsed().as_secs_f32() * 1000.0;
        Ok(true)
    }

    /// The emitters of instances that moved (those of a mesh that emits): their runs of the light list taken again on
    /// the GPU, through the records just queued; the light tree's moving subtree after (`move_emitters`).
    pub(super) fn fill_moved(&mut self, records: impl Iterator<Item = u32>) {
        // (Runs that follow each other as one: an instanced object's copies'.)
        let mut runs: Vec<[u32; 2]> = vec![];
        for r in records.map(|i| emitter_run(&self.sc.tables.emitter_tris, &self.sc.instances, i)).filter(|r| !r.is_empty()) {
            match runs.last_mut() {
                Some(l) if l[0] + l[1] == r.start as u32 => l[1] += r.len() as u32,
                _ => runs.push([r.start as u32, r.len() as u32]),
            }
        }
        if !runs.is_empty() {
            self.g.fill_emitter_runs(&self.sc.up, &runs);
            self.emitters_moved = true;
        }
    }

    /// The emitters the CPU moves moved: their triangles in the list taken again on the GPU, from the triangles this
    /// commit uploaded (`gpu::EmitterFills::moved`); and the light tree's moving subtree refit around them (the CPU copy
    /// of the dynamic tail), its nodes and the root uploaded (`ltree::LightTree::refit_moving`).
    pub(super) fn move_emitters(&mut self) {
        if let Some(f) = &self.sc.up.fills.moved {
            let mut e = self.g.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("emitter fill") });
            self.g.fill_emitters(f, &mut e);
            self.g.queue.submit([e.finish()]);
        }
        let first = self.sc.dynamic.0;
        let (verts, tables) = (&self.sc.dynamic.1, &mut self.sc.tables);
        let tris = &tables.emitter_tris;
        let (q, heap) = (&self.g.queue, &self.sc.up.heap);
        let Some(t) = tables.light_tree.as_mut().filter(|t| t.moving_count > 0) else { return };
        let (instances, pool_tris, pool_first) = (&self.sc.instances, &self.sc.pool_tris, self.sc.edit.pool_first);
        t.refit_moving(&|e| {
            let x = &tris[e];
            // (An instance's: its mesh's triangle, placed; none while the instance is hidden.)
            if x.inst != crate::NO_INST {
                let i = &instances[x.inst as usize];
                return (!i.hidden).then(|| pool_tris[x.tri as usize - pool_first].map(|v| i.world.transform_point3(v.into()).to_array()));
            }
            let r = (x.tri as usize - first) * 3;
            Some(crate::ltree::tri_vertices(&verts[r..r + 3]))
        });
        heap.write(q, Table::LightTree, 0, bytemuck::cast_slice(&t.nodes[..1]));
        heap.write(q, Table::LightTree, 2 * 16, bytemuck::cast_slice(&t.nodes[2..3]));
        heap.write(q, Table::LightTree, t.moving_nodes as u64 * 16, bytemuck::cast_slice(&t.nodes[t.moving_nodes..]));
    }

    /// Copies the GPU's current vertices of a dynamic-tail span into `prev_verts` (submitted before any write queued
    /// after it, so it reads what the last frame traced).
    pub(super) fn keep_prev(&self, (lo, hi): (usize, usize)) {
        let up = &self.sc.up;
        let first = self.sc.dynamic.0;
        let mut e = self.g.device.create_command_encoder(&Default::default());
        e.copy_buffer_to_buffer(&up.verts, ((first + lo) * 48) as u64, &up.prev_verts, (lo * 48) as u64, ((hi - lo) * 48) as u64);
        self.g.queue.submit([e.finish()]);
    }

    /// Skins / transforms the objects into the CPU copy of the dynamic tail; the touched span (relative to the tail), or
    /// None when one cannot be updated in place.
    pub(super) fn reskin(&mut self, world: &World, ids: &std::collections::HashSet<u32>, models: &std::collections::HashMap<u32, scene::DynamicModel>) -> Result<Option<(usize, usize)>> {
        use rayon::prelude::*;
        if ids.iter().any(|id| !world.shown(*id)) {
            return Ok(None);
        }
        // Objects whose triangles form one run of the dynamic tail (one traversal group) are skinned / re-transformed in
        // parallel, one task per object, straight into their own spans of the tail and of the CPU triangles (one core
        // took 17 ms for the crowd scene's 150 walkers, docs/ROADMAP.md § 0): ordered by span, the objects split the
        // arrays into disjoint slices. The others (e.g. opaque and cutout materials in different groups) one by one.
        let run = |d: &scene::DynamicModel| !d.finals.is_empty() && d.finals.iter().enumerate().all(|(j, &f)| f == d.finals[0] + j as u32);
        let (mut runs, scattered): (Vec<u32>, Vec<u32>) = ids.iter().copied().partition(|id| run(&models[id]));
        runs.sort_by_key(|id| models[id].finals[0]);
        let assets = &self.assets;
        let sc = &mut self.sc;
        let (first, verts, attrs, blas) = &mut sc.dynamic;
        let first = *first;
        let (mut lo, mut hi) = (usize::MAX, 0);
        // The CPU triangles in build order: each object's slice.
        let mut by_build = runs.clone();
        by_build.sort_by_key(|id| models[id].first);
        let mut tris_of: std::collections::HashMap<u32, &mut [[glam::Vec3A; 3]]> = std::collections::HashMap::with_capacity(runs.len());
        let (mut tr, mut toff): (&mut [[glam::Vec3A; 3]], usize) = (&mut sc.tris[..], 0);
        for id in &by_build {
            let d = &models[id];
            let (t0, n) = (d.first as usize, d.finals.len());
            let (_, r) = std::mem::take(&mut tr).split_at_mut(t0 - toff);
            let (t, r) = r.split_at_mut(n);
            tr = r;
            toff = t0 + n;
            tris_of.insert(*id, t);
        }
        let (mut vr, mut ar, mut br): (&mut [[f32; 4]], &mut [scene::GpuTriAttr], &mut [[f32; 3]]) = (&mut verts[..], &mut attrs[..], &mut blas[..]);
        let mut voff = 0usize;
        let mut jobs = Vec::with_capacity(runs.len());
        for id in &runs {
            let d = &models[id];
            let (k0, n) = (d.finals[0] as usize - first, d.finals.len());
            let (_, r) = std::mem::take(&mut vr).split_at_mut(k0 * 3 - voff * 3);
            let (v, r) = r.split_at_mut(n * 3);
            vr = r;
            let (_, r) = std::mem::take(&mut ar).split_at_mut(k0 - voff);
            let (a, r) = r.split_at_mut(n);
            ar = r;
            let (_, r) = std::mem::take(&mut br).split_at_mut(k0 * 3 - voff * 3);
            let (b, r) = r.split_at_mut(n * 3);
            br = r;
            voff = k0 + n;
            (lo, hi) = (lo.min(k0), hi.max(k0 + n));
            jobs.push((*id, v, a, b, tris_of.remove(id).unwrap()));
        }
        let ok: Vec<Result<bool>> = jobs
            .into_par_iter()
            .map(|(id, v, a, b, t)| {
                let d = &models[&id];
                let Some(tris) = scene::dynamic_tris(world, id, d, assets)? else { return Ok(false) };
                if tris.len() != d.finals.len() {
                    return Ok(false);
                }
                for (j, (p, at)) in tris.into_iter().enumerate() {
                    let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
                    v[j * 3..j * 3 + 3].copy_from_slice(&[[p[0].x, p[0].y, p[0].z, f32::from_bits(d.flags[j])], [e1.x, e1.y, e1.z, 0.0], [e2.x, e2.y, e2.z, 0.0]]);
                    // (x.w: an emitter's light-tree code, which stays with its triangle.)
                    a[j] = scene::GpuTriAttr { x: [at.x[0], f32::from_bits(d.object), at.x[2], a[j].x[3]], ..at };
                    b[j * 3..j * 3 + 3].copy_from_slice(&scene::blas_tri(p.map(|q| q.to_array())));
                    t[j] = p.map(glam::Vec3A::from);
                }
                Ok(true)
            })
            .collect();
        for r in ok {
            if !r? {
                return Ok(None);
            }
        }
        for id in &scattered {
            let d = &models[id];
            let Some(tris) = scene::dynamic_tris(world, *id, d, assets)? else { return Ok(None) };
            if tris.len() != d.finals.len() {
                return Ok(None);
            }
            for (j, (p, a)) in tris.into_iter().enumerate() {
                let k = d.finals[j] as usize - first;
                let (e1, e2) = (p[1] - p[0], p[2] - p[0]);
                verts[k * 3..k * 3 + 3].copy_from_slice(&[[p[0].x, p[0].y, p[0].z, f32::from_bits(d.flags[j])], [e1.x, e1.y, e1.z, 0.0], [e2.x, e2.y, e2.z, 0.0]]);
                attrs[k] = scene::GpuTriAttr { x: [a.x[0], f32::from_bits(d.object), a.x[2], attrs[k].x[3]], ..a };
                blas[k * 3..k * 3 + 3].copy_from_slice(&scene::blas_tri(p.map(|v| v.to_array())));
                sc.tris[(d.first + j as u32) as usize] = p.map(glam::Vec3A::from);
                (lo, hi) = (lo.min(k), hi.max(k + 1));
            }
        }
        for id in ids {
            let d = &models[id];
            // Pivot (material functions' object position). Motion vectors carry its history.
            let pivot = world.world_matrix(*id).transform_point3(glam::Vec3::ZERO);
            let st = sc.tables.stride;
            let o = &mut sc.tables.objects[d.object as usize * st];
            (o[0], o[1], o[2]) = (pivot.x, pivot.y, pivot.z);
        }
        Ok((lo < hi).then_some((lo, hi)))
    }

    /// Rebuilds the GPU scene from the world.
    /// Applies geometry edits in place (`Changes::geometries`, `added`, `removed`; objects with their own BLAS:
    /// procedural meshes and `own_blas` meshes): each touched object's data is written into the edit room
    /// (gpu.rs `EditRoom`) as the build would make it (`scene::proc_object`, `scene::own_mesh_tris`), new BLASes are
    /// built for new geometry only, its TLAS instance is set (cleared for a removed or hidden object) and the TLAS
    /// rebuilt, one submit. Nothing is moved: what an edit replaces stays behind until the next rebuild. Err: why it
    /// cannot (software traversal, the room is full, a kernel feature the scene lacks, an emitting / glass / function
    /// material, ...); the caller rebuilds the scene then. `THREERT_EDIT_REBUILD=1` always rebuilds (A/B).
    pub(super) fn edit_geometry(&mut self, world: &World, ch: &crate::world::Changes, build: &mut gpu::TopBuild) -> std::result::Result<(), String> {
        use crate::world::Kind;
        if std::env::var("THREERT_EDIT_REBUILD").is_ok_and(|v| v == "1") {
            return Err("THREERT_EDIT_REBUILD=1".into());
        }
        let LoadedScene { up, tables, edit, meshes, pool_tris, instances, gpu_instances, procs, proc_first, moving_instances, .. } = &mut self.sc;
        let (g, assets) = (&self.g, &self.assets);
        let features = up.kernel.features;
        let grow = Grow { g: &self.g, ok: up.particles.is_none() && up.gpu_particles.is_none() };
        // (Hardware traversal's TLAS and BLASes, or software traversal's units: `gpu::Top`.)
        let Some(mut top) = gpu::Top::of(&mut up.accel, &mut up.units) else {
            // Software traversal without units. With units on, that is a scene without an instance yet: the first
            // one brings the unit walk into the kernel (F_UNITS), as hardware traversal's first brings F_INST.
            let first = crate::units_on(true) && ch.added.iter().any(|id| matches!(world.objects.get(id).map(|o| &o.kind), Some(Kind::Mesh { own_blas: true, .. } | Kind::Instanced { procedural: false, .. })));
            return Err(if first { "the scene's first instanced mesh (kernel feature)" } else { "software traversal" }.into());
        };
        // The objects to place again: those added, and those whose geometry was replaced.
        let mut todo: std::collections::BTreeSet<u32> = ch.added.clone();
        if !ch.geometries.is_empty() {
            for (id, o) in &world.objects {
                match &o.kind {
                    Kind::Procedural { geometry, .. } | Kind::Mesh { geometry, own_blas: true, .. } if ch.geometries.contains(geometry) => {
                        todo.insert(*id);
                    }
                    Kind::Mesh { geometry, .. } if ch.geometries.contains(geometry) => return Err("a flattened mesh's geometry changed".into()),
                    // (One added this commit is placed with its geometry as it is.)
                    Kind::Instanced { geometry, .. } if ch.geometries.contains(geometry) && !ch.added.contains(id) => return Err("an instanced mesh's geometry changed".into()),
                    _ => {}
                }
            }
        }
        let (inst_base, proc_first) = (top.inst_base(), *proc_first);
        // An object's (slot, record), where an edit can place it again or take it out: an instance of a mesh that is
        // its own, or that an edit placed (a model the build placed is the build's to change), or procedural geometry.
        let slot_of = |id: u32, edit: &EditState, tables: &tables::Tables| -> Option<(usize, usize)> {
            let record = match &tables.retained.proxies.get(&id)?.draw {
                scene::Draw::Instance(i) if i.mesh.own() || edit.in_room(i.instance as usize) => i.instance as usize,
                scene::Draw::Procedural(j) => proc_first + *j as usize,
                _ => return None,
            };
            Some((inst_base + record, record))
        };
        let q = &g.queue;
        // An instance's emitters go (area 0: their weight; the light tree keeps its shape) when its object is removed,
        // hidden or placed again; those it is given are in `new_emitters` (`write_emitters`).
        let mut emitters_changed = false;
        let mut new_emitters: Vec<(u32, crate::GpuEmitterTri)> = vec![];
        let release = |id: u32, tables: &mut tables::Tables, instances: &[scene::Instance]| -> bool {
            let Some(record) = tables.retained.instance(id).map(|i| i.instance) else { return false };
            let run = emitter_run(&tables.emitter_tris, instances, record);
            tables.emitter_tris[run.clone()].iter_mut().for_each(|e| e.area = 0.0);
            !run.is_empty()
        };
        for id in &ch.removed {
            // An instanced mesh: its instances out. One an edit placed gives its records and entries back to the room
            // and its mesh a user fewer (the build's stay until the next rebuild).
            if let Some((place, mesh)) = tables.retained.instanced(*id).and_then(|p| Some((*p, p.mesh?))) {
                let (r0, cap) = (place.first as usize, place.capacity as usize);
                for r in r0..r0 + cap {
                    top.clear(inst_base + r);
                    // (A copy's emitters go as an instance's do.)
                    let run = emitter_run(&tables.emitter_tris, instances, r as u32);
                    emitters_changed |= !run.is_empty();
                    tables.emitter_tris[run].iter_mut().for_each(|e| e.area = 0.0);
                    if let Some(i) = instances.get_mut(r) {
                        i.hidden = true;
                    }
                }
                if edit.in_room(r0) {
                    edit.records.give(r0, cap);
                    edit.objects.give(place.object as usize, cap);
                    drop_mesh_user(mesh, edit, tables, meshes);
                }
                tables.retained.proxies.remove(id);
                continue;
            }
            emitters_changed |= release(*id, tables, instances);
            let at = slot_of(*id, edit, tables);
            // Drawn, and not at a record an edit can clear (a model the build placed, anything it flattened): a rebuild
            // takes it out.
            if at.is_none() && tables.retained.proxies.contains_key(id) {
                return Err("an object the build placed otherwise (a model, something it flattened) removed".into());
            }
            let proxy = tables.retained.proxies.remove(id);
            if let Some((slot, record)) = at {
                top.clear(slot);
                if let Some(k) = undraw(proxy.as_ref().map(|p| &p.draw), instances, procs) {
                    drop_mesh_user(k, edit, tables, meshes);
                }
                // What it held goes back to the room (the build's records and entries stay until the next rebuild).
                if edit.in_room(record) {
                    edit.records.give(record, 1);
                }
            }
            if let Some(k) = proxy.and_then(|p| p.object).filter(|&k| k as usize >= edit.base.2) {
                edit.objects.give(k as usize, 1);
            }
        }
        let mut new_meshes = vec![];
        let mut pending_procs = vec![];
        let mut split = EditSplit::default();
        let (mut w_cdf, mut w_verts, mut w_attrs, mut w_blas, mut w_records, mut w_objects) = (Gather::default(), Gather::default(), Gather::default(), Gather::default(), Gather::default(), Gather::default());
        // Vertex colours (F_VCOLOR).
        let mut w_colors = Gather::default();
        let vcolors = up.vcolors;
        // (geometry, material) -> the mesh built in this batch (objects sharing a replaced geometry share it again).
        let mut fresh: std::collections::HashMap<(u32, u32), u32> = Default::default();
        let mask = |shadow: bool| if shadow { 1 } else { 2 };
        for id in todo {
            let Some(o) = world.objects.get(&id) else { continue };
            let at = slot_of(id, edit, tables);
            emitters_changed |= release(id, tables, instances);
            if !world.shown(id) {
                if let Some((slot, _)) = at {
                    top.clear(slot);
                }
                if tables.retained.instanced(id).is_some_and(|p| edit.in_room(p.first as usize)) {
                    return Err("an instanced mesh added in place hidden".into());
                }
                if let Some(k) = undraw(tables.retained.proxies.get(&id).map(|p| &p.draw), instances, procs) {
                    drop_mesh_user(k, edit, tables, meshes);
                }
                continue;
            }
            // The mesh it draws now, as an instance at its record: a user fewer once it is placed again (below).
            let was = at.and_then(|(_, r)| instances.get(r)).filter(|i| !i.hidden).map(|i| i.mesh);
            // Drawn but not by a TLAS instance of its own: the build flattened it (movable, or a material instances skip).
            if at.is_none() && tables.retained.proxies.contains_key(&id) {
                return Err("an object the build flattened".into());
            }
            let m = world.world_matrix(id);
            let object = match tables.retained.object(id) {
                Some(k) => k as usize,
                None => match edit.objects.take(1) {
                    Some(k) => k,
                    None if grow.objects(&mut edit.objects, 1, &mut up.heap, tables.stride) => {
                        edit.grows += 1;
                        edit.rebind = true;
                        edit.objects.take(1).ok_or("edit room: objects")?
                    }
                    None => return Err("edit room: objects (particles, or the device's limits)".into()),
                },
            };
            let (record, gpu, blas, entry) = match &o.kind {
                Kind::Procedural { .. } => {
                    // (Software traversal walks procedural objects by their count and their own trees in the region:
                    // the build lays those out.)
                    if top.sw() {
                        return Err("a procedural object (software traversal)".into());
                    }
                    if features & gpu::F_PROC == 0 {
                        return Err("the scene's first procedural object (kernel feature)".into());
                    }
                    let Some(p) = scene::proc_object(world, id, m, object as u32, &tables.retained.materials, assets).map_err(|e| format!("{e:#}"))? else {
                        if let Some((slot, _)) = at {
                            top.clear(slot);
                        }
                        undraw(tables.retained.proxies.get(&id).map(|p| &p.draw), instances, procs);
                        continue;
                    };
                    // Room for `used` words: the region grown if it lacks it.
                    let mut words_left = |used: usize, edit: &mut EditState| {
                        if used <= edit.word_cap {
                            return true;
                        }
                        let ok = grow.words(&mut edit.word_cap, used, &mut up.words);
                        if ok {
                            edit.grows += 1;
                            edit.rebind = true;
                        }
                        ok
                    };
                    let boxes = match edit.blobs.get(&p.blob.hash) {
                        Some(&w) => w,
                        None => {
                            let w = edit.word;
                            if !words_left(w + p.blob.boxes.len() + p.blob.data.len(), edit) {
                                return Err("edit room: procedural words".into());
                            }
                            let at = |word: usize| (word * 4) as u64;
                            w_cdf.put(at(w), bytemuck::cast_slice(&p.blob.boxes));
                            w_cdf.put(at(w + p.blob.boxes.len()), bytemuck::cast_slice(&p.blob.data));
                            edit.word += p.blob.boxes.len() + p.blob.data.len();
                            edit.blobs.insert(p.blob.hash, w);
                            w
                        }
                    };
                    let header = edit.word;
                    let words = p.header(header, boxes, (0, 0));
                    if !words_left(header + words.len(), edit) {
                        return Err("edit room: procedural words".into());
                    }
                    w_cdf.put((header * 4) as u64, bytemuck::cast_slice(&words));
                    edit.word += words.len();
                    let blas = g.proc_blas(&p.blob, &mut pending_procs);
                    let entry = p.object_entry(o);
                    let gpu = p.gpu(header as u32);
                    (at.map(|(_, r)| r), gpu, Placed::Proc(blas, p), entry)
                }
                Kind::Mesh { materials, .. } if materials.len() > 1 => return Err("a mesh with several materials".into()),
                Kind::Model { uri, nodes, animation, functions } => {
                    // A static model already in the scene: another instance of its mesh (one not in it brings materials
                    // and textures, which load with a rebuild).
                    if animation.is_some() {
                        return Err("an animated model".into());
                    }
                    if !functions.is_empty() {
                        return Err("a model with material functions (its materials load with a rebuild)".into());
                    }
                    if features & gpu::F_INST == 0 || m.determinant() == 0.0 || std::env::var("THREERT_INSTANCING").is_ok_and(|v| v == "0") {
                        return Err("a model the build would flatten (no size, instancing off, or the scene's first instance)".into());
                    }
                    let mesh = *tables.retained.model_mesh.get(&(uri.clone(), nodes.clone())).ok_or("a model not in the scene (its materials and textures load with a rebuild)")?;
                    let im = meshes[mesh as usize];
                    if !top.has_mesh(mesh) {
                        return Err("a model whose mesh is no unit (it deforms; software traversal flattens it)".into());
                    }
                    *edit.users.entry(mesh).or_default() += 1;
                    if let Some(k) = was {
                        drop_mesh_user(k, edit, tables, meshes);
                    }
                    let inst = scene::Instance { mesh, world: m, object: object as u32, cast_shadow: o.cast_shadow, name: o.name.clone(), sub: None, hidden: false, first_emitter: 0 };
                    let gpu = top.record(inst.gpu((edit.pool_first as u32) + im.first, im.opaque), mesh, o.cast_shadow);
                    let (lo, hi) = im.placed_bounds(m);
                    let entry = scene::object_row(m.transform_point3(glam::Vec3::ZERO), o.power, hi - lo, o.custom, [1.0; 3]);
                    (at.map(|(_, r)| r), gpu, Placed::Mesh(inst), entry)
                }
                Kind::Mesh { own_blas: true, .. } | Kind::Instanced { procedural: false, .. } => {
                    // An own-BLAS mesh, or (added in place) an instanced mesh: their mesh made alike.
                    let (geometry, material, capacity) = match &o.kind {
                        Kind::Mesh { geometry, material, .. } => (*geometry, *material, None),
                        Kind::Instanced { geometry, materials, capacity, .. } => (*geometry, *materials.first().ok_or("an instanced mesh without a material")?, Some(*capacity as usize)),
                        _ => unreachable!(),
                    };
                    let (geometry, material) = (&geometry, &material);
                    if features & gpu::F_INST == 0 {
                        return Err("the scene's first instanced mesh (kernel feature)".into());
                    }
                    // What the build flattens instead (scene.rs: a placement without size, or instancing off).
                    if m.determinant() == 0.0 || std::env::var("THREERT_INSTANCING").is_ok_and(|v| v == "0") {
                        return Err("a mesh without size, or instancing off (the build flattens it)".into());
                    }
                    let mat = *tables.retained.materials.get(material).ok_or("unknown material")?;
                    let gm = tables.materials[mat as usize];
                    // (It covers instances' triangles from here on: `Tables::fixed_mats`.)
                    tables.fixed_mats.insert(mat);
                    // An emitting material (no glass, no function): a mesh as any other, each instance's emitters in
                    // the light list (given below, where its record is known; scene.rs; ROADMAP § 6 step 3).
                    let emits = scene::emits_only(&gm);
                    let class = scene::mesh_class(&gm);
                    if class == 2 {
                        return Err("a glass material".into());
                    }
                    if gm.tex1[3] != 0 && features & gpu::F_IFUNC == 0 {
                        return Err("the scene's first material function on an instance (kernel feature)".into());
                    }
                    if class == 1 && features & gpu::F_MASKED == 0 {
                        return Err("the scene's first cutout (kernel feature)".into());
                    }
                    let colored = world.materials.get(material).is_some_and(|m| m.vertex_colors) && world.geometries.get(geometry).is_some_and(|g| !g.colors.is_empty());
                    if colored && !vcolors {
                        return Err("the scene's first vertex colours (kernel feature)".into());
                    }
                    let key = (*geometry, mat);
                    let kept = (!ch.geometries.contains(geometry)).then(|| tables.retained.own_mesh.get(&key).copied()).flatten();
                    let mesh = match fresh.get(&key).copied().or(kept) {
                        Some(k) => k,
                        None => {
                            let geo = world.geometries.get(geometry).ok_or("unknown geometry")?;
                            let t0 = Instant::now();
                            let (mut t, mut a) = scene::own_mesh_tris(geo, mat);
                            if t.is_empty() {
                                if let Some((slot, _)) = at {
                                    top.clear(slot);
                                }
                                // (No longer drawn: its mesh a user fewer now, as when it is hidden.)
                                if let Some(k) = undraw(tables.retained.proxies.get(&id).map(|p| &p.draw), instances, procs) {
                                    drop_mesh_user(k, edit, tables, meshes);
                                }
                                continue;
                            }
                            // Software traversal: the mesh's tree, and its triangles (their attributes, further down their
                            // colours) in the tree's order, as `prepare` lays a unit's mesh out.
                            let tree = match &top {
                                gpu::Top::Hw(_) => None,
                                gpu::Top::Sw(_) => {
                                    let tree = crate::units::mesh_tree(&t)?;
                                    t = tree.order.iter().map(|&i| t[i as usize]).collect();
                                    a = tree.order.iter().map(|&i| a[i as usize]).collect();
                                    Some(tree)
                                }
                            };
                            // (A mesh that emits: its triangles numbered as they lie, for its instances' emitters.)
                            if emits {
                                scene::number_emitters(&mut a, &tables.materials);
                            }
                            // (A unit's cutout triangles carry their flag; hardware traversal has them in a geometry apart.)
                            let flags = f32::from_bits(if tree.is_some() && class == 1 { scene::TRI_MASKED } else { 0 });
                            // As `prepare` lays the pool out: (a, flags), e1, e2; attributes; the BLAS's vertices.
                            let verts: Vec<[f32; 4]> = t.iter().flat_map(|&[a, b, c]| [[a.x, a.y, a.z, flags], [b.x - a.x, b.y - a.y, b.z - a.z, 0.0], [c.x - a.x, c.y - a.y, c.z - a.z, 0.0]]).collect();
                            let blas_verts: Vec<[f32; 3]> = t.iter().flat_map(|&[a, b, c]| [[a.x, a.y, a.z], [b.x, b.y, b.z], [c.x, c.y, c.z]]).collect();
                            split.tris += t0.elapsed().as_secs_f64() * 1e3;
                            // The same triangle count as the mesh it replaces: rewritten where it is, its BLAS rebuilt (no
                            // room used; a geometry that moves every frame, three.js's dynamic draw usage). Otherwise new
                            // triangles in the room and a new BLAS.
                            let same = tables.retained.own_mesh.get(&key).copied().filter(|&k| meshes[k as usize].count as usize == t.len() );
                            let (k, first, im) = match same {
                                Some(k) => {
                                    let im = meshes[k as usize];
                                    split.deformed += 1;
                                    // (Its tree again: the records of the objects that draw it, all placed again by
                                    // this edit, take the root it has now.)
                                    if let (gpu::Top::Sw(units), Some(tree)) = (&mut top, &tree) {
                                        if let Some(old) = units.drop_tree(k) {
                                            g.fill_tree(&up.bg_parts.nodes, old);
                                        }
                                        let root = units.add_tree(k, tree).ok_or("edit room: tree nodes")?;
                                        g.write_tree(&up.bg_parts.nodes, root, tree);
                                    }
                                    (k, edit.pool_first + im.first as usize, im)
                                }
                                None => {
                                    // A hole that fits, else the room's end (grown if it lacks room).
                                    let first = match edit.tris.take(t.len()) {
                                        Some(f) => f,
                                        // (Software traversal's room does not grow: the rebuild makes a larger one.)
                                        None if match &mut top { gpu::Top::Hw(accel) => grow.tris(&mut edit.tris, t.len(), &mut up.verts, &mut up.attrs, &mut up.words, vcolors, accel), gpu::Top::Sw(_) => false } => {
                                            edit.grows += 1;
                                            edit.rebind = true;
                                            edit.tris.take(t.len()).ok_or("edit room: triangles")?
                                        }
                                        None => return Err(if matches!(top, gpu::Top::Sw(_)) { "edit room: triangles (software traversal's does not grow)" } else { "edit room: triangles (particles, or the device's limits)" }.into()),
                                    };
                                    let mut im = scene::own_inst_mesh(&t, (first - edit.pool_first) as u32, class);
                                    im.emitters = if emits { t.len() as u32 } else { 0 };
                                    let t1 = Instant::now();
                                    // A freed mesh's slot, else a new one.
                                    let dead = edit.dead_meshes.pop();
                                    let k = match &mut top {
                                        gpu::Top::Hw(accel) => match dead {
                                            Some(k) => {
                                                g.set_mesh(accel, k, first, im.count, im.opaque);
                                                k
                                            }
                                            None => g.add_mesh(accel, first, im.count, im.opaque),
                                        },
                                        // Its tree into the mesh trees' room (a freed mesh's slot gives its tree back first).
                                        gpu::Top::Sw(units) => {
                                            let k = dead.unwrap_or(meshes.len() as u32);
                                            let tree = tree.as_ref().expect("software traversal made the tree");
                                            if let Some(old) = units.drop_tree(k) {
                                                g.fill_tree(&up.bg_parts.nodes, old);
                                            }
                                            let Some(root) = units.add_tree(k, tree) else {
                                                edit.dead_meshes.extend(dead);
                                                edit.tris.give(first, t.len());
                                                return Err("edit room: tree nodes".into());
                                            };
                                            g.write_tree(&up.bg_parts.nodes, root, tree);
                                            k
                                        }
                                    };
                                    if k as usize == meshes.len() {
                                        meshes.push(im);
                                    } else {
                                        meshes[k as usize] = im;
                                    }
                                    edit.managed.insert(k);
                                    split.blas += t1.elapsed().as_secs_f64() * 1e3;
                                    split.added += 1;
                                    (k, first, im)
                                }
                            };
                            let t2 = Instant::now();
                            w_verts.put((first * 48) as u64, bytemuck::cast_slice(&verts));
                            w_attrs.put((first * std::mem::size_of::<scene::GpuTriAttr>()) as u64, bytemuck::cast_slice(&a));
                            if !top.sw() {
                                w_blas.put((first * 36) as u64, bytemuck::cast_slice(&blas_verts));
                            }
                            if vcolors {
                                // White where it has none: the room's slots may hold an earlier mesh's.
                                let mut c = if colored { scene::tri_colors(geo, false, None) } else { vec![scene::WHITE; t.len()] };
                                if let (Some(tree), true) = (&tree, colored) {
                                    c = tree.order.iter().map(|&i| c[i as usize]).collect();
                                }
                                w_colors.put((first * 12) as u64, bytemuck::cast_slice(&c));
                            }
                            split.upload += t2.elapsed().as_secs_f64() * 1e3;
                            let rel = first - edit.pool_first;
                            if pool_tris.len() < rel + t.len() {
                                pool_tris.resize(rel + t.len(), [glam::Vec3A::ZERO; 3]);
                            }
                            pool_tris[rel..rel + t.len()].copy_from_slice(&t);
                            let _ = im;
                            new_meshes.push(k);
                            fresh.insert(key, k);
                            tables.retained.own_mesh.insert(key, k);
                            k
                        }
                    };
                    let im = meshes[mesh as usize];
                    // It draws `mesh` now (and no longer the one it drew, if another).
                    *edit.users.entry(mesh).or_default() += 1;
                    if let Some(k) = was {
                        drop_mesh_user(k, edit, tables, meshes);
                    }
                    if let Some(cap) = capacity {
                        // An instanced mesh added in place: `cap` records (their TLAS instances `inst_base` + them) and
                        // object entries, contiguous, as the build places one (`update_instanced` then rewrites them).
                        let empty = crate::world::InstanceTable::default();
                        let t = world.instanced.get(&id).unwrap_or(&empty);
                        if t.colors.is_some() && features & gpu::F_TINT == 0 {
                            return Err("the scene's first instance colours (kernel feature)".into());
                        }
                        let placed: Vec<(glam::Mat4, bool)> = (0..cap).map(|i| scene::instance_world(m, t, i)).collect();
                        // (An entry a copy, below: not the one an object has.)
                        edit.objects.give(object, 1);
                        let r0 = match edit.records.take(cap) {
                            Some(r) => r,
                            None if match &mut top { gpu::Top::Hw(accel) => grow.records(&mut edit.records, cap, &mut up.heap, accel), gpu::Top::Sw(_) => false } => {
                                edit.grows += 1;
                                edit.rebind = true;
                                edit.records.take(cap).ok_or("edit room: records")?
                            }
                            None => return Err(if matches!(top, gpu::Top::Sw(_)) { "edit room: records (software traversal's does not grow)" } else { "edit room: records (particles, or the device's limits)" }.into()),
                        };
                        let o0 = match edit.objects.take(cap) {
                            Some(k) => k,
                            None if grow.objects(&mut edit.objects, cap, &mut up.heap, tables.stride) => {
                                edit.grows += 1;
                                edit.rebind = true;
                                edit.objects.take(cap).ok_or("edit room: objects")?
                            }
                            None => return Err("edit room: objects (particles, or the device's limits)".into()),
                        };
                        let (st, size) = (tables.stride, std::mem::size_of::<scene::GpuInstance>());
                        if instances.len() < r0 + cap {
                            let filler = scene::Instance { mesh, world: glam::Mat4::IDENTITY, object: 0, cast_shadow: false, name: String::new(), sub: None, hidden: true, first_emitter: 0 };
                            instances.resize(r0 + cap, filler);
                        }
                        if gpu_instances.len() < r0 + cap {
                            gpu_instances.resize(r0 + cap, scene::GpuInstance::default());
                        }
                        if tables.objects.len() < (o0 + cap) * st {
                            tables.objects.resize((o0 + cap) * st, [0.0; 4]);
                        }
                        for (i, &(wm, hidden)) in placed.iter().enumerate() {
                            let mut inst = scene::Instance { mesh, world: wm, object: (o0 + i) as u32, cast_shadow: o.cast_shadow, name: o.name.clone(), sub: Some(i as u32), hidden, first_emitter: 0 };
                            // A copy of a mesh that emits: its emitters, a run at the light list's end; moving ones
                            // (`update_instanced` changes the copies in place), a hidden copy's at no weight.
                            if im.emitters > 0 {
                                let first = tables.emitter_tris.len();
                                inst.first_emitter = first as u32;
                                let (a, b) = (im.first as usize, (im.first + im.count) as usize);
                                let made = crate::instance_emitters(&inst, (r0 + i) as u32, &pool_tris[a..b], (edit.pool_first + a) as u32, &|k| Some((k, mat)), im.emitters as usize, &tables.materials, true);
                                for (j, (e, w)) in made.into_iter().enumerate() {
                                    new_emitters.push(((first + j) as u32, crate::GpuEmitterTri::of(&e, w, 0)));
                                    tables.emitter_tris.push(e);
                                }
                                tables.emissive.insert(mat);
                                emitters_changed = true;
                            }
                            let rec = top.record(inst.gpu((edit.pool_first as u32) + im.first, im.opaque), mesh, o.cast_shadow);
                            w_records.put(((r0 + i) * size) as u64, bytemuck::bytes_of(&rec));
                            gpu_instances[r0 + i] = rec;
                            top.place(inst_base + r0 + i, mesh, wm, gpu::INSTANCE_BIT | (r0 + i) as u32, o.cast_shadow, hidden);
                            let (lo, hi) = im.placed_bounds(wm);
                            let entry = scene::copy_row(scene::object_row(wm.transform_point3(glam::Vec3::ZERO), o.power, hi - lo, t.custom(i, o.custom), t.color(i)), im.emitters > 0 && hidden);
                            w_objects.put(((o0 + i) * st * 16) as u64, bytemuck::cast_slice(&entry[..st]));
                            tables.objects[(o0 + i) * st..(o0 + i + 1) * st].copy_from_slice(&entry[..st]);
                            instances[r0 + i] = inst;
                        }
                        *tables.retained.proxy(id) = scene::Proxy { object: None, card: None, draw: scene::Draw::Instanced(scene::InstancedPlace { first: r0 as u32, capacity: cap as u32, object: o0 as u32, mesh: Some(mesh), drawn: t.count }) };
                        continue;
                    }
                    let inst = scene::Instance { mesh, world: m, object: object as u32, cast_shadow: o.cast_shadow, name: o.name.clone(), sub: None, hidden: false, first_emitter: 0 };
                    let gpu = top.record(inst.gpu((edit.pool_first as u32) + im.first, im.opaque), mesh, o.cast_shadow);
                    let (lo, hi) = im.placed_bounds(m);
                    let entry = scene::object_row(m.transform_point3(glam::Vec3::ZERO), o.power, hi - lo, o.custom, [1.0; 3]);
                    (at.map(|(_, r)| r), gpu, Placed::Mesh(inst), entry)
                }
                _ => return Err("not an object with its own BLAS".into()),
            };
            // Its record, TLAS instance and object entry: where it had them, or new ones from the room.
            let (slot, record) = match (at, record) {
                (Some((slot, _)), Some(r)) => (slot, r),
                _ => {
                    let r = match edit.records.take(1) {
                        Some(r) => r,
                        None if match &mut top { gpu::Top::Hw(accel) => grow.records(&mut edit.records, 1, &mut up.heap, accel), gpu::Top::Sw(_) => false } => {
                            edit.grows += 1;
                            edit.rebind = true;
                            edit.records.take(1).ok_or("edit room: records")?
                        }
                        None => return Err(if matches!(top, gpu::Top::Sw(_)) { "edit room: records (software traversal's does not grow)" } else { "edit room: records (particles, or the device's limits)" }.into()),
                    };
                    (inst_base + r, r)
                }
            };
            // An instance of a mesh that emits: its emitters, a run of the light list. The run its record had, if that
            // has as many and none is the light tree's (those keep their place there, at no weight); else a new one at
            // the list's end. Its first goes in its record.
            let (mut gpu, mut blas) = (gpu, blas);
            if let (Placed::Mesh(inst), Kind::Mesh { material, .. }) = (&mut blas, &o.kind) {
                let im = meshes[inst.mesh as usize];
                if let (true, Some(&mat)) = (im.emitters > 0, tables.retained.materials.get(material)) {
                    let old = emitter_run(&tables.emitter_tris, instances, record as u32);
                    let first = if old.len() == im.emitters as usize && tables.emitter_tris[old.clone()].iter().all(|e| !e.in_tree) { old.start } else { tables.emitter_tris.len() };
                    inst.first_emitter = first as u32;
                    gpu.n[2][3] = f32::from_bits(first as u32);
                    let (a, b) = (im.first as usize, (im.first + im.count) as usize);
                    let made = crate::instance_emitters(inst, record as u32, &pool_tris[a..b], (edit.pool_first + a) as u32, &|k| Some((k, mat)), im.emitters as usize, &tables.materials, o.driven);
                    for (j, (e, w)) in made.into_iter().enumerate() {
                        new_emitters.push(((first + j) as u32, crate::GpuEmitterTri::of(&e, w, 0)));
                        match tables.emitter_tris.get_mut(first + j) {
                            Some(old) => *old = e,
                            None => tables.emitter_tris.push(e),
                        }
                    }
                    tables.emissive.insert(mat);
                    emitters_changed = true;
                }
            }
            w_records.put((record * std::mem::size_of::<scene::GpuInstance>()) as u64, bytemuck::bytes_of(&gpu));
            if record >= gpu_instances.len() {
                gpu_instances.resize(record + 1, scene::GpuInstance::default());
            }
            gpu_instances[record] = gpu;
            // Above the triangles, and the record's value where the build's are: `instances[record]`, or `procs` at the
            // record less the first procedural one (hidden fillers at the records between, which are other objects').
            let draw = match blas {
                Placed::Proc(blas, p) => {
                    top.place_blas(slot, &blas, m, record as u32, mask(o.cast_shadow));
                    let j = record - proc_first;
                    if procs.len() <= j {
                        procs.resize(j + 1, scene::ProcObject { name: String::new(), materials: vec![], hidden: true, ..p.clone() });
                    }
                    procs[j] = p;
                    scene::Draw::Procedural(j as u32)
                }
                Placed::Mesh(inst) => {
                    top.place(slot, inst.mesh, m, gpu::INSTANCE_BIT | record as u32, o.cast_shadow, false);
                    if instances.len() <= record {
                        instances.resize(record + 1, scene::Instance { name: String::new(), hidden: true, ..inst.clone() });
                    }
                    let mesh = if matches!(o.kind, Kind::Model { .. }) { scene::MeshKind::Model } else { scene::MeshKind::Own };
                    instances[record] = inst;
                    // (A mesh a body drives moves as an instance does, as the build's: `commit`.)
                    scene::Draw::Instance(scene::InstanceRef { instance: record as u32, mesh, moves: o.driven })
                }
            };
            let st = tables.stride;
            w_objects.put((object * st * 16) as u64, bytemuck::cast_slice(&entry[..st]));
            let at = object * st;
            if at >= tables.objects.len() {
                tables.objects.resize(at + st, [0.0; 4]);
            }
            tables.objects[at..at + st].copy_from_slice(&entry[..st]);
            *tables.retained.proxy(id) = scene::Proxy { object: Some(object as u32), card: None, draw };
        }
        // The gathered writes, merged, before the builds that read them.
        let tw = Instant::now();
        w_cdf.flush_section(q, &up.words, Stream::Procs);
        w_verts.flush(q, &up.verts);
        w_attrs.flush(q, &up.attrs);
        if let gpu::Top::Hw(accel) = &top {
            w_blas.flush(q, &accel.verts);
        }
        w_colors.flush_section(q, &up.words, Stream::VertexColors);
        w_records.flush_section(q, &up.heap, Table::Instances);
        w_objects.flush_section(q, &up.heap, Table::Objects);
        split.upload += tw.elapsed().as_secs_f64() * 1e3;
        let t3 = Instant::now();
        // Software traversal: the trees are written; the meshes this edit freed give theirs back.
        if let gpu::Top::Sw(units) = &mut top {
            for &k in &edit.dead_meshes {
                if let Some(old) = units.drop_tree(k) {
                    g.fill_tree(&up.bg_parts.nodes, old);
                }
            }
        }
        split.blas += t3.elapsed().as_secs_f64() * 1e3;
        // The new meshes' and procedural BLASes and the TLAS (or the top tree over the units as they are now): the
        // commit's one build (`build_top`).
        build.instances = true;
        build.meshes.extend(new_meshes);
        build.procs.extend(pending_procs);
        *moving_instances = tables.retained.moves_instances();
        // Buffers that grew: the scene bind group over them.
        if std::mem::take(&mut edit.rebind) {
            g.rebind_scene(up);
        }
        // The uniform tables: entries the edits placed past what a uniform buffer holds read as its last one.
        if g.uniform_tables && !edit.uniform_said {
            let notes = gpu::uniform_overflow([0, (edit.objects.end * tables.stride) as u64, edit.records.end as u64, 0]);
            for note in &notes {
                eprintln!("threert: {note}");
            }
            edit.uniform_said = !notes.is_empty();
        }
        split.objects = ch.added.len() + ch.removed.len();
        self.edit_split.add(&split);
        if emitters_changed {
            self.write_emitters(&new_emitters);
            // (Instances' emitters came or went: the tree's moving subtree around those there, at the commit's end.)
            self.emitters_moved = true;
        }
        Ok(())
    }

}
