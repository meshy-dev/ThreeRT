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
    /// The instances' rows (`scene::Row`) edits can place, as the records are, and the run each record an edit placed
    /// has (its first row, how many): given back with the record, kept where an object is placed again with as many.
    pub(super) rows: Holes,
    pub(super) row_runs: std::collections::HashMap<usize, (usize, usize)>,
    /// The first pool triangle (instanced meshes' triangles are counted from it).
    pub(super) pool_first: usize,
    /// Procedural blobs in the region by hash: their first word (the build's and those edits wrote).
    pub(super) blobs: std::collections::HashMap<[u8; 32], usize>,
    /// The previous frame's positions (a vertex each) of the deforming meshes edits place: the build's before them,
    /// then room that grows. And the `scene::Skinned` slots (the skinning pass's jobs) of those an edit took out
    /// again: the next one placed takes one.
    pub(super) prev: Holes,
    /// The vertices edits can place (a mesh's run: `InstMesh::vfirst`), as the triangles are.
    pub(super) verts: Holes,
    pub(super) free_skinned: Vec<u32>,
}

impl EditState {
    /// Whether `record` is one of the room's: an edit placed what is there (the build's are below).
    pub(super) fn in_room(&self, record: usize) -> bool {
        record >= self.base.1
    }

    /// `n` rows for what an edit places at `record`: the run it placed there before, if as long; the rows the build
    /// gave the instance there, if it has as many; else a hole or the room's end. None: no room (the caller grows it).
    pub(super) fn take_rows(&mut self, instances: &[scene::Instance], meshes: &[scene::InstMesh], record: usize, n: usize) -> Option<usize> {
        match self.row_runs.get(&record) {
            Some(&(at, len)) if len == n => return Some(at),
            Some(&(at, len)) => {
                self.rows.give(at, len);
                self.row_runs.remove(&record);
            }
            None => {
                let built = instances.get(record).filter(|_| !self.in_room(record));
                if let Some(i) = built.filter(|i| meshes.get(i.mesh as usize).is_some_and(|m| m.sect_count as usize >= n)) {
                    return Some(i.rows as usize);
                }
            }
        }
        let at = self.rows.take(n)?;
        self.row_runs.insert(record, (at, n));
        Some(at)
    }

    /// The rows an edit placed at `record` go back to the room.
    pub(super) fn give_rows(&mut self, record: usize) {
        if let Some((at, len)) = self.row_runs.remove(&record) {
            self.rows.give(at, len);
        }
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

/// What an edit places for an object: a procedural object with its BLAS (hardware traversal) and its record, an
/// instance of a mesh, or a posed rigid model's parts.
pub(super) enum Placed {
    Proc(wgpu::Blas, scene::ProcObject, scene::GpuInstance),
    /// An instance and its rows (its mesh's sections' materials, classes and emitters).
    Mesh(scene::Instance, Vec<scene::Row>),
    /// A part: its node (`scene::STILL_NODES`: those its clip leaves alone), its instance and its rows. Their records
    /// follow each other.
    Parts(Vec<(usize, scene::Instance, Vec<scene::Row>)>),
    /// A posed skinned model: the instance of its own mesh, its rows, and its slot among `Retained::skinned`.
    Skinned(scene::Instance, Vec<scene::Row>, u32),
}

/// What `make_mesh` writes to.
pub(super) struct Meshing<'a, 'b> {
    g: &'a gpu::Gpu,
    edit: &'a mut EditState,
    grow: &'a Grow<'a>,
    vertices: &'a mut wgpu::Buffer,
    words: &'a mut Heap<Stream>,
    /// Software traversal's nodes (a unit's tree is written there).
    nodes: &'a wgpu::Buffer,
    top: &'a mut gpu::Top<'b>,
    meshes: &'a mut Vec<scene::InstMesh>,
    sections: &'a mut Vec<scene::Section>,
    pool_tris: &'a mut Vec<[glam::Vec3A; 3]>,
    pool_place: &'a mut Vec<[u32; 2]>,
    split: &'a mut EditSplit,
    w_vert: &'a mut Gather,
    w_index: &'a mut Gather,
    w_leaf: &'a mut Gather,
    w_colors: &'a mut Gather,
    vcolors: bool,
}

/// A mesh an edit makes, as `prepare` lays one of the pool out: its triangles by section (`data`: a scene's geometry's,
/// `scene::section_mesh`, or a model's), its vertices each once, its acceleration structure or, under software
/// traversal, its tree. In the room: a hole that fits, else its end, grown where it lacks. `same`: the mesh it
/// takes the place of where that is (as many triangles a section; `colored`: an object of it reads its colours).
/// Returns the mesh.
pub(super) fn make_mesh(cx: &mut Meshing, data: scene::MeshData, colored: bool, same: Option<u32>) -> std::result::Result<u32, String> {
    let t0 = Instant::now();
    let scene::MeshData { tris: mut t, attrs: mut a, colors: mut c, sections: sects, .. } = data;
    // Software traversal: the mesh's tree, and its triangles (their attributes, further down their
    // colours) in the tree's order, as `prepare` lays a unit's mesh out.
    let tree = match &*cx.top {
        gpu::Top::Hw(_) => None,
        gpu::Top::Sw(_) => {
            let tree = crate::units::mesh_tree(&t)?;
            t = tree.order.iter().map(|&i| t[i as usize]).collect();
            a = tree.order.iter().map(|&i| a[i as usize]).collect();
            c = c.map(|c| tree.order.iter().map(|&i| c[i as usize]).collect());
            scene::rank_in_sections(&mut a);
            Some(tree)
        }
    };
    // As `prepare` lays the pool out: the mesh's vertices, each once (indexed.rs), its triangles'
    // indices from its first vertex, and for a tree its leaves: each triangle with its section
    // and its index in it.
    let scope = crate::indexed::shared(&t, &a);
    let nv = scope.vertices.len();
    let leaves: Vec<crate::indexed::Leaf> = if tree.is_some() { t.iter().zip(&a).map(|(t, x)| crate::indexed::leaf(*t, [x.uv[3].to_bits(), x.x[3].to_bits()])).collect() } else { vec![] };
    cx.split.tris += t0.elapsed().as_secs_f64() * 1e3;
    // The same sections as the mesh it replaces (as many triangles in each: a BLAS's geometries keep
    // their sizes): rewritten where it is, its BLAS rebuilt (no room used; a geometry that moves
    // every frame, three.js's dynamic draw usage). Otherwise new triangles in the room and a new
    // BLAS.
    if sects.len() > scene::MESH_SECTIONS {
        return Err("a mesh with more than 65,536 sections".into());
    }
    let (k, first, im) = match same {
        Some(k) => {
            // (Its colours stay its own while another object of it reads them.)
            cx.meshes[k as usize].colored |= colored;
            // Its vertices where they were, if the run holds them; else another run, and its
            // BLAS made again for as many (the objects that draw it are all placed again by
            // this edit: their records and TLAS instances take what it has now).
            let old = cx.meshes[k as usize];
            if nv > old.vcount as usize {
                let vfirst = take_verts(cx.edit, cx.grow, cx.vertices, nv, cx.top.sw())?;
                cx.edit.verts.give(old.vfirst as usize, old.vcount as usize);
                (cx.meshes[k as usize].vfirst, cx.meshes[k as usize].vcount) = (vfirst as u32, nv as u32);
                if let gpu::Top::Hw(accel) = &mut *cx.top {
                    cx.g.set_mesh(accel, k, cx.edit.pool_first + old.first as usize, &cx.meshes[k as usize], cx.sections);
                }
            }
            let im = cx.meshes[k as usize];
            cx.split.deformed += 1;
            // (Its tree again: the records of the objects that draw it, all placed again by
            // this edit, take the root it has now.)
            if let (gpu::Top::Sw(units), Some(tree)) = (&mut *cx.top, &tree) {
                if let Some(old) = units.drop_tree(k) {
                    cx.g.fill_tree(cx.nodes, old);
                }
                let root = units.add_tree(k, tree).ok_or("edit room: tree nodes")?;
                cx.g.write_tree(cx.nodes, root, tree);
            }
            (k, cx.edit.pool_first + im.first as usize, im)
        }
        None => {
            // A hole that fits, else the room's end (grown if it lacks room).
            let first = match cx.edit.tris.take(t.len()) {
                Some(f) => f,
                // (Software traversal's room does not grow: the rebuild makes a larger one.)
                None if !cx.top.sw() && cx.grow.tris(&mut cx.edit.tris, t.len(), cx.words, cx.vcolors) => {
                    cx.edit.grows += 1;
                    cx.edit.rebind = true;
                    cx.edit.tris.take(t.len()).ok_or("edit room: triangles")?
                }
                None => return Err(if matches!(*cx.top, gpu::Top::Sw(_)) { "edit room: triangles (software traversal's does not grow)" } else { "edit room: triangles (particles, or the device's limits)" }.into()),
            };
            let vfirst = take_verts(cx.edit, cx.grow, cx.vertices, nv, cx.top.sw())?;
            let t1 = Instant::now();
            // A freed mesh's slot (and its sections', where they are as many at least), else new ones.
            let dead = cx.edit.dead_meshes.pop();
            let n = sects.len();
            let sect_first = match dead.map(|k| cx.meshes[k as usize]).filter(|m| m.sect_count as usize >= n) {
                Some(m) => m.sect_first,
                None => {
                    cx.sections.resize(cx.sections.len() + n, scene::Section { first: 0, count: 0, slot: 0 });
                    (cx.sections.len() - n) as u32
                }
            };
            cx.sections[sect_first as usize..][..n].copy_from_slice(&sects);
            let im = scene::InstMesh { sect_first, sect_count: n as u32, colored, vfirst: vfirst as u32, vcount: nv as u32, ..scene::own_inst_mesh(&t, (first - cx.edit.pool_first) as u32) };
            let k = match &mut *cx.top {
                gpu::Top::Hw(accel) => match dead {
                    Some(k) => {
                        cx.g.set_mesh(accel, k, first, &im, cx.sections);
                        k
                    }
                    None => cx.g.add_mesh(accel, first, &im, cx.sections),
                },
                // Its tree into the mesh trees' room (a freed mesh's slot gives its tree back first).
                gpu::Top::Sw(units) => {
                    let k = dead.unwrap_or(cx.meshes.len() as u32);
                    let tree = tree.as_ref().expect("software traversal made the tree");
                    if let Some(old) = units.drop_tree(k) {
                        cx.g.fill_tree(cx.nodes, old);
                    }
                    let Some(root) = units.add_tree(k, tree) else {
                        cx.edit.dead_meshes.extend(dead);
                        cx.edit.tris.give(first, t.len());
                        return Err("edit room: tree nodes".into());
                    };
                    cx.g.write_tree(cx.nodes, root, tree);
                    k
                }
            };
            if k as usize == cx.meshes.len() {
                cx.meshes.push(im);
            } else {
                cx.meshes[k as usize] = im;
            }
            cx.edit.managed.insert(k);
            cx.split.blas += t1.elapsed().as_secs_f64() * 1e3;
            cx.split.added += 1;
            (k, first, im)
        }
    };
    let t2 = Instant::now();
    cx.w_vert.put((im.vfirst as usize * crate::indexed::VERTEX_BYTES) as u64, bytemuck::cast_slice(&scope.vertices));
    cx.w_index.put((first * 12) as u64, bytemuck::cast_slice(&scope.indices));
    // (A unit's tree's leaves; a BLAS is built from the vertices.)
    if cx.top.sw() {
        cx.w_leaf.put((first * crate::indexed::LEAF_BYTES) as u64, bytemuck::cast_slice(&leaves));
    }
    if cx.vcolors {
        // White where it has none: the room's slots may hold an earlier mesh's.
        let c = c.filter(|_| im.colored).unwrap_or_else(|| vec![scene::WHITE; t.len()]);
        cx.w_colors.put((first * 12) as u64, bytemuck::cast_slice(&c));
    }
    cx.split.upload += t2.elapsed().as_secs_f64() * 1e3;
    let rel = first - cx.edit.pool_first;
    if cx.pool_tris.len() < rel + t.len() {
        cx.pool_tris.resize(rel + t.len(), [glam::Vec3A::ZERO; 3]);
    }
    cx.pool_tris[rel..rel + t.len()].copy_from_slice(&t);
    // (Software traversal: where its tree left each triangle of its sections, `mesh_places`.)
    if cx.top.sw() {
        if cx.pool_place.len() < rel + t.len() {
            cx.pool_place.resize(rel + t.len(), [0; 2]);
        }
        for (j, x) in a.iter().enumerate() {
            cx.pool_place[rel + j] = [x.uv[3].to_bits(), x.x[3].to_bits()];
        }
    }
    let _ = im;
    Ok(k)
}

/// What `place_instance` writes to.
pub(super) struct Placing<'a, 'b> {
    edit: &'a mut EditState,
    grow: &'a Grow<'a>,
    words: &'a mut Heap<Stream>,
    tables: &'a mut tables::Tables,
    instances: &'a mut Vec<scene::Instance>,
    gpu_instances: &'a mut Vec<scene::GpuInstance>,
    meshes: &'a [scene::InstMesh],
    sections: &'a [scene::Section],
    rows: &'a mut Vec<scene::Row>,
    pool_tris: &'a [[glam::Vec3A; 3]],
    pool_place: &'a [[u32; 2]],
    w_rows: &'a mut Gather,
    w_records: &'a mut Gather,
    new_emitters: &'a mut Vec<(u32, crate::GpuEmitterTri)>,
    emitters_changed: &'a mut bool,
    top: &'a mut gpu::Top<'b>,
    inst_base: usize,
}

/// An instance placed at `record` by an edit: its rows in the room, each with where its section's triangles start;
/// its emitters, if a section's material emits (`moves`: as moving ones); its kernel record; its place above the
/// triangles; its value where the build's are (`instances[record]`).
pub(super) fn place_instance(cx: &mut Placing, record: usize, inst: &mut scene::Instance, mine: &mut [scene::Row], moves: bool) -> std::result::Result<(), String> {
    let im = cx.meshes[inst.mesh as usize];
    let sects = im.sections(cx.sections);
    for (s, row) in sects.iter().zip(mine.iter_mut()) {
        row[1] = cx.edit.pool_first as u32 + im.first + s.first;
    }
    let row_at = rows_for(cx.edit, cx.grow, cx.words, cx.instances, cx.meshes, record, mine.len())?;
    inst.rows = row_at as u32;
    // Its sections whose material emits: its emitters, a run of the light list, a section after another. The run its
    // record had, if that has as many and none is the light tree's (those keep their place there, at no weight); else
    // a new one at the list's end. A section's start goes in its row.
    let n = scene::rows_emitters(sects, mine, &cx.tables.materials) as usize;
    if n > 0 {
        let starts = scene::section_emitters(sects, mine, &cx.tables.materials);
        let old = emitter_run(&cx.tables.emitter_tris, cx.instances, record as u32);
        let first = if old.len() == n && cx.tables.emitter_tris[old.clone()].iter().all(|e| !e.in_tree) { old.start } else { cx.tables.emitter_tris.len() };
        inst.first_emitter = first as u32;
        for (row, start) in mine.iter_mut().zip(&starts) {
            row[2] = start.map_or(scene::NO_EMITTER, |x| first as u32 + x);
        }
        let (place, of) = (mesh_places(&im, cx.sections, cx.pool_place), mine.iter().map(scene::row_material).collect::<Vec<u32>>());
        let numbered = |j: usize| {
            let (s, i) = place[j];
            starts.get(s as usize).copied().flatten().map(|x| ((x + i) as usize, of[s as usize], s))
        };
        let (a, b) = (im.first as usize, (im.first + im.count) as usize);
        let made = crate::instance_emitters(inst, record as u32, &cx.pool_tris[a..b], (cx.edit.pool_first + a) as u32, &numbered, n, &cx.tables.materials, moves);
        for (j, (e, w)) in made.into_iter().enumerate() {
            cx.new_emitters.push(((first + j) as u32, crate::GpuEmitterTri::of(&e, w, 0)));
            match cx.tables.emitter_tris.get_mut(first + j) {
                Some(old) => *old = e,
                None => cx.tables.emitter_tris.push(e),
            }
        }
        for (&mat, start) in of.iter().zip(&starts) {
            if start.is_some() {
                cx.tables.emissive.insert(mat);
            }
        }
        *cx.emitters_changed = true;
    }
    cx.w_rows.put((row_at * 16) as u64, bytemuck::cast_slice(mine));
    if cx.rows.len() < row_at + mine.len() {
        cx.rows.resize(row_at + mine.len(), [0; 4]);
    }
    cx.rows[row_at..row_at + mine.len()].copy_from_slice(mine);
    inst.verts = im.vfirst;
    let gpu = cx.top.record(inst.gpu(cx.edit.pool_first as u32 + im.first), inst.mesh, inst.cast_shadow);
    cx.w_records.put((record * std::mem::size_of::<scene::GpuInstance>()) as u64, bytemuck::bytes_of(&gpu));
    if record >= cx.gpu_instances.len() {
        cx.gpu_instances.resize(record + 1, scene::GpuInstance::default());
    }
    cx.gpu_instances[record] = gpu;
    cx.top.place(cx.inst_base + record, inst.mesh, inst.world, gpu::INSTANCE_BIT | record as u32, inst.cast_shadow, inst.hidden);
    if cx.instances.len() <= record {
        cx.instances.resize(record + 1, scene::Instance { name: String::new(), hidden: true, ..inst.clone() });
    }
    cx.instances[record] = inst.clone();
    Ok(())
}

/// Each of a mesh's pool triangles' section and its index in it: as they lie (a section after another), or, where a
/// unit's tree ordered them (software traversal), as kept (`LoadedScene::pool_place`).
pub(super) fn mesh_places(im: &scene::InstMesh, sections: &[scene::Section], pool_place: &[[u32; 2]]) -> Vec<(u32, u32)> {
    let (a, b) = (im.first as usize, (im.first + im.count) as usize);
    if let Some(kept) = pool_place.get(a..b).filter(|_| !pool_place.is_empty()) {
        return kept.iter().map(|p| (p[0], p[1])).collect();
    }
    let mut out = vec![(0, 0); im.count as usize];
    for (s, x) in im.sections(sections).iter().enumerate() {
        for i in 0..x.count {
            out[(x.first + i) as usize] = (s as u32, i);
        }
    }
    out
}

/// Spans (first triangle, first of the previous vertices, how many) in order, the same one once, neighbours in both
/// as one: the copies `keep_prev_spans` makes.
pub(super) fn merge_spans(mut spans: Vec<(usize, usize, usize)>) -> Vec<(usize, usize, usize)> {
    spans.sort_unstable();
    spans.dedup();
    let mut out: Vec<(usize, usize, usize)> = vec![];
    for s in spans {
        match out.last_mut() {
            Some(l) if l.0 + l.2 == s.0 && l.1 + l.2 == s.1 => l.2 += s.2,
            _ => out.push(s),
        }
    }
    out
}

/// One object fewer draws instanced mesh `k`: a mesh edits manage that none draws any more is freed (its triangles a
/// hole the next new mesh can take, its slot reused, its BLAS dropped with it).
pub(super) fn drop_mesh_user(k: u32, edit: &mut EditState, tables: &mut tables::Tables, meshes: &[scene::InstMesh]) {
    let n = edit.users.entry(k).or_default();
    *n = n.saturating_sub(1);
    if *n == 0 && edit.managed.remove(&k) {
        let im = meshes[k as usize];
        edit.tris.give(edit.pool_first + im.first as usize, im.count as usize);
        edit.verts.give(im.vfirst as usize, im.vcount as usize);
        tables.retained.own_mesh.retain(|_, v| *v != k);
        tables.retained.model_mesh.retain(|_, v| *v != k);
        tables.retained.mesh_rows.remove(&k);
        edit.dead_meshes.push(k);
    }
}

/// `n` vertices of the room: a hole that fits, else its end, grown if it lacks room (`sw`: software traversal's does
/// not grow).
fn take_verts(edit: &mut EditState, grow: &Grow, vertices: &mut wgpu::Buffer, n: usize, sw: bool) -> std::result::Result<usize, String> {
    if let Some(first) = edit.verts.take(n) {
        return Ok(first);
    }
    if !sw && grow.verts(&mut edit.verts, n, vertices) {
        edit.grows += 1;
        edit.rebind = true;
        return edit.verts.take(n).ok_or_else(|| "edit room: vertices".to_string());
    }
    Err(if sw { "edit room: vertices (software traversal's does not grow)" } else { "edit room: vertices (particles, or the device's limits)" }.into())
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

    /// Triangles: their indices, and in a scene with them (`colors`) vertex colours: sections of the words' heap,
    /// grown together. (A mesh's triangle has no row under hardware traversal, whose rooms these are.)
    pub(super) fn tris(&self, h: &mut Holes, extra: usize, words: &mut Heap<Stream>, colors: bool) -> bool {
        if !self.ok {
            return false;
        }
        let cap = Self::cap(h, extra);
        let mut sections = vec![(Stream::Indices, (cap * 12) as u64)];
        if colors {
            sections.push((Stream::VertexColors, (cap * 12) as u64));
        }
        if !self.fits(&[words.size_with_all(&sections)]) {
            return false;
        }
        words.grow_all(&self.g.device, &self.g.queue, &sections);
        h.cap = cap;
        true
    }

    /// Vertices (`vertices`), or the previous frame's of deforming meshes (`prev`: whole records too): a record each.
    fn vertex_room(&self, h: &mut Holes, extra: usize, buffer: &mut wgpu::Buffer, label: &str) -> bool {
        if !self.ok {
            return false;
        }
        let bytes = (Self::cap(h, extra) * crate::indexed::VERTEX_BYTES) as u64;
        if !self.fits(&[bytes]) {
            return false;
        }
        *buffer = self.g.grown(buffer, bytes, label);
        h.cap = Self::cap(h, extra);
        true
    }
    pub(super) fn verts(&self, h: &mut Holes, extra: usize, vertices: &mut wgpu::Buffer) -> bool {
        self.vertex_room(h, extra, vertices, "vertices")
    }
    pub(super) fn prev(&self, h: &mut Holes, extra: usize, prev: &mut wgpu::Buffer) -> bool {
        self.vertex_room(h, extra, prev, "previous vertices")
    }

    /// A table's section to `bytes` (heap.rs `Heap::grow`), if the device binds the buffer that makes.
    fn table(&self, heap: &mut Heap<Table>, t: Table, bytes: u64) -> bool {
        if !self.fits(&[heap.size_with(t, bytes)]) {
            return false;
        }
        heap.grow(&self.g.device, &self.g.queue, t, bytes);
        true
    }

    /// The instances' rows.
    pub(super) fn rows(&self, h: &mut Holes, extra: usize, words: &mut Heap<Stream>) -> bool {
        if !self.ok {
            return false;
        }
        let cap = Self::cap(h, extra);
        if !self.fits(&[words.size_with(Stream::Rows, (cap * 16) as u64)]) {
            return false;
        }
        words.grow(&self.g.device, &self.g.queue, Stream::Rows, (cap * 16) as u64);
        h.cap = cap;
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

/// `n` rows for what an edit places at `record` (`EditState::take_rows`), their section grown where it has no room.
#[allow(clippy::too_many_arguments)]
pub(super) fn rows_for(edit: &mut EditState, grow: &Grow, words: &mut Heap<Stream>, instances: &[scene::Instance], meshes: &[scene::InstMesh], record: usize, n: usize) -> Result<usize, String> {
    if let Some(at) = edit.take_rows(instances, meshes, record, n) {
        return Ok(at);
    }
    if !grow.rows(&mut edit.rows, n, words) {
        return Err("edit room: rows (particles, or the device's limits)".into());
    }
    edit.grows += 1;
    edit.rebind = true;
    edit.take_rows(instances, meshes, record, n).ok_or_else(|| "edit room: rows".to_string())
}

/// Whether an object's instance is all of it, so that an edit can hide, move or take out the object by it: a mesh's,
/// or a still model's whose mesh has nothing flattened at each placement beside.
pub(super) fn whole(i: &scene::InstanceRef, instances: &[scene::Instance], meshes: &[scene::InstMesh]) -> bool {
    i.mesh.own() || (matches!(i.mesh, scene::MeshKind::Model) && instances.get(i.instance as usize).is_some_and(|x| meshes.get(x.mesh as usize).is_some_and(|m| m.rest_count == 0)))
}

/// An object removed or hidden in place is no longer drawn: hidden where its record's value is (`LiveCore::pick`
/// skips it). The mesh it was an instance of, if it was drawn as one: a user fewer (`drop_mesh_user`).
pub(super) fn undraw(draw: Option<&scene::Draw>, instances: &mut [scene::Instance], procs: &mut [scene::ProcObject]) -> Option<u32> {
    match draw? {
        scene::Draw::Instance(i) => instances.get_mut(i.instance as usize).and_then(|i| {
            // (One hidden by its object's visibility still had its mesh.)
            let held = !i.hidden || i.masked;
            (i.hidden, i.masked) = (true, false);
            held.then_some(i.mesh)
        }),
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
                    Some(crate::world::Kind::Mesh { .. } | crate::world::Kind::Model { .. } | crate::world::Kind::Group) => self.show_meshes(world, id, &mut changes, &mut top),
                    _ => false,
                };
                if !done {
                    if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") && !changes.structural { eprintln!("edit: structural at edits.rs:568 (shown/hidden not in place: {:?})", world.objects.get(&id).map(|o| (o.name.clone(), std::mem::discriminant(&o.kind), self.sc.tables.retained.instance(id).is_some()))); }
                    changes.structural = true;
                }
            }
        }
        // Material functions that came or changed: the scene's kernel with them, before the materials of them.
        if changes.functions && !changes.structural {
            self.set_functions(world);
        }
        // The textures the new and the changed materials name: those the pages have not got are placed in them now;
        // a texture that was replaced has its materials made again.
        if !changes.structural && (!changes.new_materials.is_empty() || !changes.materials.is_empty() || !changes.textures.is_empty()) {
            if let Err(why) = self.place_textures(world, &mut changes) {
                self.fallback = Some(format!("a texture that cannot be placed in the pages ({why:#})"));
                self.fallbacks += 1;
                changes.structural = true;
            }
        }
        // The materials of the models the commit adds that the scene has not got, their textures with them.
        if !changes.structural && !changes.added.is_empty() {
            match self.place_models(world, &changes) {
                Ok(true) => {
                    self.upload_materials();
                    self.sc.up.kernel.features = self.kernel_features();
                }
                Ok(false) => {}
                Err(why) => {
                    self.fallback = Some(format!("a model whose materials cannot be made in place ({why:#})"));
                    self.fallbacks += 1;
                    changes.structural = true;
                }
            }
        }
        // New materials: entries after the scene's, before the edits that place objects drawn with them (and the GPU's
        // table with them, grown if it is full).
        if !changes.new_materials.is_empty() && !changes.structural {
            if self.sc.tables.add_materials(world, &changes.new_materials) {
                self.upload_materials();
                // (A clear coat, an unlit material: the kernel variant with it, compiled once.)
                self.sc.up.kernel.features = self.kernel_features();
            } else {
                self.fallback = Some("a new material that cannot be added in place".into());
                self.fallbacks += 1;
                if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") && !changes.structural { eprintln!("edit: structural at edits.rs:582"); }
                changes.structural = true;
            }
        }
        // An object an edit placed (its record is the room's) moves in place as an instance does (a still one from its
        // first move: `start_moving`), unless an edit took the instance out while the object was hidden; procedural
        // geometry an edit placed rebuilds.
        let stuck = |id: &u32| match self.sc.tables.retained.proxies.get(id).map(|p| &p.draw) {
            Some(scene::Draw::Instance(i)) => self.sc.edit.in_room(i.instance as usize) && self.sc.instances.get(i.instance as usize).is_none_or(|i| i.hidden && !i.masked),
            Some(scene::Draw::Procedural(j)) => self.sc.edit.in_room(self.sc.proc_first + *j as usize),
            _ => false,
        };
        if !changes.structural && changes.moved.iter().any(stuck) {
            self.fallback = Some("an object added in place moved".into());
            self.fallbacks += 1;
            if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") && !changes.structural { eprintln!("edit: structural at edits.rs:596"); }
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
                    if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") && !changes.structural { eprintln!("edit: structural at edits.rs:626"); }
                    changes.structural = true;
                }
            }
        }
        if !changes.promoted.is_empty() {
            eprintln!("live: {} moved, now movable (this once rebuilds; set `movable = true` in the scene code to avoid it)", changes.promoted.join(", "));
        }
        // Moved movable subtrees: their drawables are re-transformed with the posed models, their lights rewritten in
        // the light table.
        let mut dynamic = changes.poses.clone();
        // Still objects that moved for the first time: their meshes' instances move from now on, or the scene rebuilds.
        for id in std::mem::take(&mut changes.started) {
            if !changes.structural && !self.start_moving(world, id) {
                if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") && !changes.structural { eprintln!("edit: structural at edits.rs:639"); }
                changes.structural = true;
            }
        }
        let mut placed = vec![];
        let mut procs_moved = vec![];
        let mut instanced_moved = vec![];
        let children = if changes.moved.is_empty() { Default::default() } else { world.children() };
        for id in &changes.moved {
            for k in world.subtree_with(*id, &children) {
                match &world.objects[&k].kind {
                    crate::world::Kind::Light(crate::desc::Light::Rect { visible: true, .. }) | crate::world::Kind::Mesh { .. } | crate::world::Kind::Model { .. } => {
                        // (A mesh hidden by its visibility keeps its instance, which moves with it.)
                        let inst = self.sc.tables.retained.instance(k).filter(|i| i.moves).copied();
                        let masked = inst.is_some_and(|i| self.sc.instances.get(i.instance as usize).is_some_and(|x| x.masked));
                        if world.shown(k) || masked {
                            // Movable models placed as instances move their transform (a posed skinned model too); the rest
                            // re-transform triangles.
                            match inst.map(|i| i.instance) {
                                Some(i) if !changes.poses.contains(&k) || inst.is_some_and(|i| matches!(i.mesh, scene::MeshKind::Skinned(_))) => placed.push((k, i)),
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
        // Posed skinned models: their meshes skinned again (`repose_skinned`, their BLASes only); their moves place instances.
        let shared: Vec<u32> = dynamic.iter().copied().filter(|k| self.sc.tables.retained.instance(*k).is_some_and(|i| matches!(i.mesh, scene::MeshKind::Skinned(_)))).collect();
        for k in &shared {
            dynamic.remove(k);
        }
        let shared_ok = shared.is_empty() || (!changes.structural && self.repose_skinned(world, &shared, &mut top)?);
        // Procedural geometry moves its records and TLAS transforms.
        if !procs_moved.is_empty() && !changes.structural {
            self.move_procs(world, &procs_moved, &mut top);
        }
        let placed_ok = shared_ok && (placed.is_empty() || (!changes.structural && self.move_instances(world, &placed, &mut top)));
        changes.powers.extend(self.flipped.drain(..));
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
            if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") && !changes.structural { eprintln!("edit: structural at edits.rs:762"); }
            changes.structural = true;
        }
        if (changes.camera || changes.view || changes.settings) && !changes.structural && !self.apply_view(world, &changes)? {
            if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") && !changes.structural { eprintln!("edit: structural at edits.rs:765"); }
            changes.structural = true;
        }
        let now = shader_time(self.time);
        // The tables' edits, then what the edits above left to build, once. Either may say the scene is to be rebuilt,
        // and the rebuild builds all of it.
        let applied_r = self.sc.tables.apply(world, &changes, now);
        let log = std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1");
        if log {
            match &applied_r {
                Err(e) => eprintln!("edit: rebuild (tables: {e:#})"),
                Ok(None) => eprintln!("edit: rebuild (tables: structural {}, {} added, {} removed, {} geometries, {} new materials)", changes.structural, changes.added.len(), changes.removed.len(), changes.geometries.len(), changes.new_materials.len()),
                Ok(Some(_)) => {}
            }
        }
        let mut applied = applied_r.ok().flatten();
        // (Materials whose class changed: their instances' rows, bits and masks, before the build that reads the masks.)
        if applied.is_some_and(|d| d.reclassed) {
            if let Err(why) = self.remake_classes(world, &mut top) {
                self.fallback = Some(why);
                self.fallbacks += 1;
                applied = None;
            }
        }
        self.sc.tables.reclassed.clear();
        if applied.is_some() && !self.build_top(top) {
            if log {
                eprintln!("edit: rebuild (the top-level build refused)");
            }
            applied = None;
        }
        // Lights that came or went past what the kernel sums exactly: the light tiles come or go with their buffers,
        // a rebuild's to make.
        if applied.is_some_and(|d| d.lights) {
            let lists = gpu::light_tiles_on() && self.sc.tables.lights.iter().filter(|l| l.pos[3] != 0.0 && l.ext[3] >= 0.5).count() > gpu::RIS_EXACT;
            if lists != (self.sc.instance_features & gpu::F_LISTS != 0) {
                self.fallback = Some("as many shadow-casting local lights as bring the light tiles, or take them away".into());
                self.fallbacks += 1;
                applied = None;
            }
        }
        // (Features the edits asked for: the scene's from now on. A rebuild finds them itself.)
        let more = self.more_features.take();
        let Some(d) = applied else {
            let t = Instant::now();
            self.pending_emitters.clear();
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
        if more != 0 {
            self.sc.instance_features |= more;
            self.sc.up.kernel.features = self.kernel_features();
        }
        if d.lights {
            if d.lights_moved {
                self.sc.tables.regroup_lights();
            }
            // (More lights than the table has room for: it grows.)
            let l = &self.sc.tables.lights;
            let need = [(Table::Lights, std::mem::size_of_val(&l[..]) as u64), (Table::LightGroups, crate::light_groups(l).len() as u64 * 16)];
            if need.iter().any(|&(t, n)| n > self.sc.up.heap.cap(t)) {
                let cap = &self.sc.up.heap;
                let want = need.map(|(t, n)| (t, if n > cap.cap(t) { n * 2 } else { cap.cap(t) }));
                self.sc.up.heap.grow_all(&self.g.device, &self.g.queue, &want);
                self.g.rebind_scene(&mut self.sc.up);
            }
            // The kernel's counts of them: all, the directional ones, where the shadowed ones start.
            let l = &self.sc.tables.lights;
            self.sc.base.counts[0] = l.len() as u32;
            self.sc.base.counts2[0] = l.iter().filter(|l| l.pos[3] == 0.0).count() as u32;
            self.sc.base.counts2[3] = l.iter().position(|l| crate::light_class(l) == 2).unwrap_or(l.len()) as u32;
        }
        let (q, up, e) = (&self.g.queue, &self.sc.up, &mut self.sc.tables);
        if d.lights && !e.lights.is_empty() {
            up.put_lights(q, &e.lights);
        }
        if d.objects {
            up.heap.write(q, Table::Objects, 0, bytemuck::cast_slice(&e.objects));
        }
        if d.materials || d.emitters || std::mem::take(&mut self.emitters_reweigh) {
            let mut new = self.add_material_emitters();
            new.append(&mut self.pending_emitters);
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

    /// An instance that came back, or went unseen: its emitters' areas and centres as it stands now (an unseen one's
    /// are its mesh's own: `placed_emitter`), for their weights.
    fn weigh_run(&mut self, r: u32) {
        let LoadedScene { tables, instances, pool_tris, edit, .. } = &mut self.sc;
        let Some(inst) = instances.get(r as usize) else { return };
        let run = emitter_run(&tables.emitter_tris, instances, r);
        for j in run.clone() {
            let e = tables.emitter_tris[j];
            if e.area > 0.0 {
                let (area, c, _) = crate::placed_emitter(inst, &pool_tris[e.tri as usize - edit.pool_first], e.mat, &tables.materials);
                (tables.emitter_tris[j].area, tables.emitter_tris[j].c) = (area, c);
            }
        }
        self.emitters_reweigh |= !run.is_empty();
    }

    /// A still object's first move, in place: the meshes under it (itself, if it is one) move by their instances from
    /// now on (`InstanceRef::moves`), and one that emits has its emitters as moving ones: a new run of the light list
    /// (the light tree keeps the old one where it stood, at no weight). False where something under it is drawn
    /// another way: the caller rebuilds.
    fn start_moving(&mut self, world: &World, id: u32) -> bool {
        let children = world.children();
        let sub = world.subtree_with(id, &children);
        // (Nothing is changed unless all of it can be.)
        let held = |k: &u32| match &world.objects[k].kind {
            crate::world::Kind::Group | crate::world::Kind::Camera(_) => true,
            crate::world::Kind::Light(l) => !matches!(l, crate::desc::Light::Rect { visible: true, .. }),
            crate::world::Kind::Mesh { .. } | crate::world::Kind::Model { animation: None, .. } => {
                self.sc.tables.retained.instance(*k).is_some_and(|i| whole(i, &self.sc.instances, &self.sc.meshes) && self.sc.instances.get(i.instance as usize).is_some_and(|x| !x.hidden || x.masked))
            }
            _ => false,
        };
        if !sub.iter().all(held) {
            return false;
        }
        let mut started = vec![];
        for k in sub {
            let Some(scene::Draw::Instance(i)) = self.sc.tables.retained.proxies.get_mut(&k).map(|p| &mut p.draw) else { continue };
            if std::mem::replace(&mut i.moves, true) {
                continue;
            }
            // (Where this commit's move places it: `move_instances`, next.)
            let (to, _) = scene::placed_world(world.world_matrix(k), self.sc.instances[i.instance as usize].world.w_axis.truncate());
            started.push((i.instance, to));
        }
        for (r, to) in started {
            self.emitters_start_moving(r, to);
        }
        true
    }

    /// Instance `r`'s emitters become moving ones, where it has some that are not: a new run of the light list, placed
    /// by `to` (where this commit moves the instance); the light tree keeps the old one where it stood, at no weight.
    fn emitters_start_moving(&mut self, r: u32, to: glam::Mat4) {
        let LoadedScene { tables, instances, pool_tris, meshes, rows, up, edit, .. } = &mut self.sc;
        let old = emitter_run(&tables.emitter_tris, instances, r);
        if old.is_empty() || tables.emitter_tris[old.clone()].iter().all(|e| e.moves) {
            return;
        }
        let (was, first) = (old.start as u32, tables.emitter_tris.len() as u32);
        let inst = &mut instances[r as usize];
        for j in old {
            let e = crate::EmitterTri { moves: true, in_tree: false, ..tables.emitter_tris[j] };
            tables.emitter_tris[j].area = 0.0;
            let w = pool_tris[e.tri as usize - edit.pool_first].map(|v| glam::Vec3A::from(to.transform_point3(v.into())));
            self.pending_emitters.push((tables.emitter_tris.len() as u32, crate::GpuEmitterTri::of(&e, w, 0)));
            tables.emitter_tris.push(e);
        }
        inst.first_emitter = first;
        // (Its rows say where its sections' emitters start: where the run is now.)
        let at = inst.rows as usize..inst.rows as usize + meshes[inst.mesh as usize].sect_count as usize;
        for row in rows[at.clone()].iter_mut().filter(|row| row[2] != scene::NO_EMITTER) {
            row[2] = row[2] - was + first;
        }
        if !at.is_empty() {
            up.words.write(&self.g.queue, Stream::Rows, (at.start * 16) as u64, bytemuck::cast_slice(&rows[at]));
        }
        self.emitters_reweigh = true;
    }

    /// An object shown or hidden by its `visible`, in place: the meshes and models under it (itself, if it is one)
    /// each by its instance's mask above the triangles (a posed rigid model's every part's; a posed skinned model's
    /// own), where no ray sees a
    /// hidden one, and its row's power (`Tables::apply`: a hidden mesh's emitters at no weight). False where
    /// something under it is drawn another way, or a mesh is not such an instance (the build flattened it, or an edit
    /// took it out while it was hidden): the caller rebuilds.
    fn show_meshes(&mut self, world: &World, id: u32, changes: &mut crate::world::Changes, build: &mut gpu::TopBuild) -> bool {
        let children = world.children();
        for k in world.subtree_with(id, &children) {
            match &world.objects[&k].kind {
                crate::world::Kind::Group | crate::world::Kind::Camera(_) => {}
                // (One this commit adds is placed as it is, shown or not: `edit_geometry`.)
                crate::world::Kind::Mesh { .. } | crate::world::Kind::Model { .. } if changes.added.contains(&k) => {}
                // A mesh, a still model, or a posed skinned model (an instance of its own mesh): its instance.
                kind @ (crate::world::Kind::Mesh { .. } | crate::world::Kind::Model { .. })
                    if !matches!(kind, crate::world::Kind::Model { animation: Some(_), .. }) || self.sc.tables.retained.instance(k).is_some_and(|i| matches!(i.mesh, scene::MeshKind::Skinned(_))) =>
                {
                    let skinned = matches!(kind, crate::world::Kind::Model { animation: Some(_), .. });
                    let Some(r) = self.sc.tables.retained.instance(k).filter(|i| skinned || whole(i, &self.sc.instances, &self.sc.meshes)).map(|i| i.instance as usize) else { return false };
                    let Some(inst) = self.sc.instances.get_mut(r) else { return false };
                    if inst.hidden && !inst.masked {
                        return false;
                    }
                    // (Where it stands: it may have moved, or lost its size, while it was hidden.)
                    let (w, gone) = scene::placed_world(world.world_matrix(k), inst.world.w_axis.truncate());
                    let unseen = gone || !world.shown(k);
                    if inst.hidden == unseen {
                        continue;
                    }
                    let Some(mut top) = gpu::Top::of(&mut self.sc.up.accel, &mut self.sc.up.units) else { return false };
                    // (Software traversal: a mesh that deforms and has no tree is in the still tree, no instance to mask.)
                    if !top.has_mesh(inst.mesh) {
                        return false;
                    }
                    (inst.hidden, inst.masked) = (unseen, unseen);
                    top.moved(top.inst_base() + r, w, Some((inst.cast_shadow, unseen)));
                    build.instances = true;
                    changes.powers.insert(k);
                    self.weigh_run(r as u32);
                    // (A skinned model shown is skinned as it is posed now: its mesh kept the pose it was hidden in.)
                    if skinned && !unseen {
                        changes.poses.insert(k);
                    }
                }
                // A posed rigid model: every part's instance. Shown, it is placed where it stands and posed as it is
                // now (`repose_rigid`, this commit: a move while it was hidden placed nothing).
                crate::world::Kind::Model { .. } => {
                    let Some(records) = self.sc.tables.retained.rigid(k).filter(|r| r.rest.mats.is_empty()).map(|r| r.records()) else { return false };
                    let unseen = !world.shown(k);
                    if records.iter().all(|&r| self.sc.instances.get(r as usize).is_some_and(|i| i.hidden == unseen)) {
                        continue;
                    }
                    let Some(mut top) = gpu::Top::of(&mut self.sc.up.accel, &mut self.sc.up.units) else { return false };
                    for &r in &records {
                        let Some(inst) = self.sc.instances.get_mut(r as usize) else { return false };
                        (inst.hidden, inst.masked) = (unseen, unseen);
                        top.moved(top.inst_base() + r as usize, inst.world, Some((inst.cast_shadow, unseen)));
                    }
                    build.instances = true;
                    changes.powers.insert(k);
                    for r in records {
                        self.weigh_run(r);
                    }
                    if !unseen {
                        changes.poses.insert(k);
                    }
                }
                _ => return false,
            }
        }
        true
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
            let inst = &mut self.sc.instances[i as usize];
            // (A placement without size, as scene code hides by scaling to 0: at a point, where no ray sees it.)
            let (w, gone) = scene::placed_world(world.world_matrix(node), inst.world.w_axis.truncate());
            // (Software traversal: an instance of a mesh that deforms is no unit; its triangles are among the still
            // scene's, in the world, and a rebuild moves them.)
            if !top.has_mesh(inst.mesh) {
                return false;
            }
            let old = self.sc.gpu_instances[i as usize];
            inst.world = w;
            let mut r = top.record(inst.gpu(old.info[0]), inst.mesh, inst.cast_shadow);
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
            // (Unseen while it has no size, or its object is hidden: `Instance::masked`. Its row's power follows.)
            let unseen = gone || !world.shown(node);
            let flipped = inst.hidden != unseen;
            (inst.hidden, inst.masked) = (unseen, unseen);
            top.moved(slot, w, Some((inst.cast_shadow, unseen)));
            self.inst_moving.push(i);
            if flipped {
                self.flipped.push(node);
            }
        }
        if merged {
            self.sc.up.heap.write(q, Table::Instances, lo as u64 * size, bytemuck::cast_slice(&self.sc.gpu_instances[lo..=hi]));
        }
        self.sc.up.heap.write(q, Table::Objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
        for k in 0..self.flipped.len() {
            if let Some(r) = self.sc.tables.retained.instance(self.flipped[k]).map(|i| i.instance) {
                self.weigh_run(r);
            }
        }
        self.fill_moved(placed.iter().map(|&(_, i)| i));
        self.inst_moved = true;
        build.instances = true;
        true
    }

    /// The material table to the GPU: written in place; with more materials than its section holds (materials added
    /// while the game runs), the section is made twice the size first and the scene's bind groups made again.
    /// The scene's kernel with the world's material functions as they are now (`World::set_shader`): each under the
    /// number it has, one the scene had not under the next, so the kernel the view runs meanwhile draws every
    /// material it knows as before, and a material of a function it has not as one without (its dispatch's default).
    /// The kernel with them is another module: made as any variant is (`choose_kernel`).
    fn set_functions(&mut self, world: &World) {
        let sc = &mut self.sc;
        for (name, (code, helpers)) in &world.shaders {
            let f = (name.clone(), code.clone(), helpers.clone());
            match sc.tables.retained.shaders.get(name) {
                Some(&k) => sc.shaders[k as usize] = f,
                None => {
                    sc.tables.retained.shaders.insert(name.clone(), sc.shaders.len() as i32);
                    sc.shaders.push(f);
                }
            }
        }
        let used: std::collections::BTreeSet<i32> = sc.translucent_mats.iter().filter_map(|&m| sc.tables.materials.get(m as usize)).map(|m| m.tex1[3] - 1).filter(|&f| f >= 0).collect();
        sc.up.kernel = sc.up.kernel.with_material_functions(&sc.shaders, &used);
        sc.fixed_features.1 = !sc.shaders.is_empty();
        self.sc.up.kernel.features = self.kernel_features();
    }

    /// Places in the texture pages what the commit's materials need of them (`Gpu::place_texture`), before the
    /// materials are made:
    /// - A texture a new or a changed material names and the pages have not got: decoded and placed, its rows in
    ///   the texture table, its slot remembered (`Retained::textures`).
    /// - A texture that was replaced: its new pixels over the old where they fit their place and the slot is no
    ///   other texture's, else its slots are forgotten and it is placed as another; the materials that name it are
    ///   made again either way (its repeat and offset are theirs).
    fn place_textures(&mut self, world: &World, changes: &mut crate::world::Changes) -> Result<()> {
        for tid in std::mem::take(&mut changes.textures) {
            let Some(t) = world.textures.get(&tid) else { continue };
            let slots: Vec<(bool, i32)> = self.sc.tables.retained.textures.iter().filter(|(k, _)| k.0 == tid).map(|(k, s)| (k.1, *s)).collect();
            for (srgb_default, slot) in slots {
                let shared = self.sc.tables.retained.textures.iter().any(|(k, s)| *s == slot && k.0 != tid);
                if shared || !self.g.rewrite_texture(slot, &scene::Texture::of_world(t, srgb_default, &self.assets)?) {
                    self.sc.tables.retained.textures.remove(&(tid, srgb_default));
                }
            }
            let known = &self.sc.tables.retained.materials;
            changes.materials.extend(world.materials.iter().filter(|(id, m)| known.contains_key(id) && crate::world::material_texture_ids(m).any(|x| x == tid)).map(|(id, _)| *id));
        }
        let (mut info, mut view) = (None, None);
        let ids: Vec<u32> = changes.new_materials.iter().chain(&changes.materials).copied().collect();
        for id in ids {
            let Some(m) = world.materials.get(&id) else { continue };
            for (tid, srgb_default) in scene::material_textures(id, m, &world.textures, &self.sc.tables.retained.shaders)? {
                if self.sc.tables.retained.textures.contains_key(&(tid, srgb_default)) {
                    continue;
                }
                let t = world.textures.get(&tid).ok_or_else(|| anyhow::anyhow!("material {id}: unknown texture {tid}"))?;
                let e = self.g.place_texture(&scene::Texture::of_world(t, srgb_default, &self.assets)?)?;
                self.sc.tables.retained.textures.insert((tid, srgb_default), e.slot);
                view = e.view.or(view);
                info = e.info.or(info);
            }
        }
        self.apply_pages(info, view);
        Ok(())
    }

    /// The pages after textures were placed in them (`Gpu::place_texture`): every texture's rows (`info`, if one was
    /// placed) in the texture table, the kernel's variant that samples textures, and the bind groups again where the
    /// pages (`view`) or the table are another resource now.
    fn apply_pages(&mut self, info: Option<std::sync::Arc<Vec<[f32; 4]>>>, view: Option<wgpu::TextureView>) {
        let mut rebind = false;
        if let Some(view) = view {
            self.sc.up.bg_parts.tex_view = view;
            rebind = true;
        }
        if let Some(info) = info {
            let bytes: &[u8] = bytemuck::cast_slice(&info[..]);
            if bytes.len() as u64 > self.sc.up.heap.cap(Table::TexInfo) {
                self.sc.up.heap.grow(&self.g.device, &self.g.queue, Table::TexInfo, bytes.len() as u64 * 2);
                rebind = true;
            }
            self.sc.up.heap.write(&self.g.queue, Table::TexInfo, 0, bytes);
            // (The scene's first texture: the kernel variant that samples them.)
            self.sc.fixed_features.0 = true;
            self.sc.up.kernel.features = self.kernel_features();
        }
        if rebind {
            self.g.rebind_scene(&mut self.sc.up);
        }
    }

    /// The materials of the models the commit adds that the scene has not got (a file's own, by file and index:
    /// `Retained::asset_materials`): made as the build makes them, entries after the scene's, their textures placed
    /// in the pages. A model with material functions is the rebuild's. Whether any was made.
    fn place_models(&mut self, world: &World, changes: &crate::world::Changes) -> Result<bool> {
        let (mut info, mut view, mut made) = (None, None, false);
        for id in &changes.added {
            let Some(crate::world::Kind::Model { uri, nodes, functions, .. }) = world.objects.get(id).map(|o| &o.kind) else { continue };
            if !functions.is_empty() {
                continue;
            }
            let glb = scene::model_gltf(uri, &self.assets)?;
            let (doc, _, images) = &glb.gltf;
            for idx in glb.used_materials(nodes.as_deref()) {
                let key = (uri.clone(), idx.unwrap_or(usize::MAX));
                if self.sc.tables.retained.asset_materials.contains_key(&key) {
                    continue;
                }
                let g = &self.g;
                let mut texture = |(k, srgb): (String, bool), wrap: bool, img: &mut dyn FnMut() -> Result<std::sync::Arc<image::RgbaImage>>| -> Result<i32> {
                    if let Some(slot) = g.texture_slot(&scene::Texture::key_of(&k, srgb, wrap)) {
                        return Ok(slot);
                    }
                    let e = g.place_texture(&scene::Texture::of(&k, srgb, wrap, img()?))?;
                    view = e.view.or(view.take());
                    info = e.info.or(info.take());
                    Ok(e.slot)
                };
                let gm = scene::gltf_material(idx.and_then(|i| doc.materials().nth(i)), images, uri, &mut texture, &Default::default())?;
                self.sc.tables.retained.asset_materials.insert(key, self.sc.tables.materials.len() as u32);
                self.sc.tables.materials.push(gm);
                made = true;
            }
        }
        self.apply_pages(info, view);
        Ok(made)
    }

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
                if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") && !changes.structural { eprintln!("edit: structural at edits.rs:1228"); }
                changes.structural = true;
                false
            }
        }
    }

    /// Materials whose class changed (`Tables::reclassed`: glass or not, a cutout or not, with a function or not):
    /// each instance with a row of one has its rows' classes again, its record's bits, and whether it casts a shadow
    /// (its mask above the triangles; the commit builds what is above them after). Err: the kernel lacks what the
    /// class needs (the scene's first cutout, its first caster with glass among its sections, its first function
    /// on an instance); the caller rebuilds.
    fn remake_classes(&mut self, world: &World, build: &mut gpu::TopBuild) -> std::result::Result<(), String> {
        let more = &self.more_features;
        let mats: std::collections::HashSet<u32> = self.sc.tables.reclassed.iter().copied().collect();
        let (features, q) = (self.sc.up.kernel.features, &self.g.queue);
        let LoadedScene { up, tables, meshes, instances, gpu_instances, rows, .. } = &mut self.sc;
        // Each drawn record with whether its object casts a shadow.
        let mut records: Vec<(usize, bool)> = vec![];
        for (id, p) in &tables.retained.proxies {
            let Some(o) = world.objects.get(id) else { continue };
            let of: Vec<u32> = match &p.draw {
                scene::Draw::Instance(i) => vec![i.instance],
                scene::Draw::Rigid(r) => r.records(),
                scene::Draw::Instanced(c) if c.mesh.is_some() => (c.first..c.first + c.capacity).collect(),
                _ => vec![],
            };
            records.extend(of.into_iter().map(|r| (r as usize, o.cast_shadow)));
        }
        let mut top = gpu::Top::of(&mut up.accel, &mut up.units);
        let size = std::mem::size_of::<scene::GpuInstance>() as u64;
        for (r, casts) in records {
            let Some(inst) = instances.get_mut(r) else { continue };
            let (at, n) = (inst.rows as usize, meshes[inst.mesh as usize].sect_count as usize);
            let Some(mine) = rows.get_mut(at..at + n).filter(|x| x.iter().any(|row| mats.contains(&scene::row_material(row)))) else { continue };
            for row in mine.iter_mut() {
                let mat = scene::row_material(row);
                row[0] = scene::row_of(mat, &tables.materials)[0];
                if tables.materials.get(mat as usize).is_some_and(|m| m.tex1[3] != 0) && features & gpu::F_IFUNC == 0 {
                    more.set(more.get() | gpu::F_IFUNC);
                }
            }
            let (bits, cast) = (scene::rows_bits(mine), scene::rows_cast(mine, casts));
            if bits & scene::INST_CUTOUT != 0 && features & gpu::F_MASKED == 0 {
                more.set(more.get() | gpu::F_MASKED);
            }
            if bits & scene::INST_GLASS != 0 && cast && features & gpu::F_IGLASS == 0 {
                more.set(more.get() | gpu::F_IGLASS);
            }
            let Some(top) = top.as_mut() else { return Err("software traversal".into()) };
            // (Its rows' bits; whether its mesh deforms is its own.)
            (inst.bits, inst.cast_shadow) = (bits | inst.bits & scene::INST_DEFORMS, cast);
            up.words.write(q, Stream::Rows, (at * 16) as u64, bytemuck::cast_slice(mine));
            let old = gpu_instances[r];
            let mut rec = top.record(inst.gpu(old.info[0]), inst.mesh, cast);
            rec.p = old.p;
            gpu_instances[r] = rec;
            up.heap.write(q, Table::Instances, r as u64 * size, bytemuck::bytes_of(&rec));
            top.moved(top.inst_base() + r, inst.world, Some((cast, inst.hidden)));
            build.instances = true;
        }
        Ok(())
    }

    /// Emitter entries for the materials that started emitting (`Tables::started_emitting`): their flattened static
    /// triangles (`mat_runs`), positions from `tris`, in the power-share part of the CDF (path code 0, as the light
    /// tree's moving emitters; ROADMAP § 6 step 3). Returns them (each one's index and triangle) for `write_emitters`.
    pub(super) fn add_material_emitters(&mut self) -> Vec<(u32, crate::GpuEmitterTri)> {
        let q = &self.g.queue;
        let sc = &mut self.sc;
        let mut new = vec![];
        let started = std::mem::take(&mut sc.tables.started_emitting);
        for &mat in &started {
            for &[first, end, obj] in sc.mat_runs.get(&mat).map_or(&[][..], |v| v.as_slice()) {
                for k in first..end {
                    let Some(&[a, b, c]) = sc.tri_order.get(k as usize).and_then(|&i| sc.tris.get(i as usize)) else { continue };
                    let area = 0.5 * (b - a).cross(c - a).length();
                    if area > 0.0 {
                        let centre = (a + b + c) / 3.0;
                        let e = crate::EmitterTri { tri: k, mat, obj, area, c: centre.to_array(), moves: false, nominal: false, in_tree: false, inst: crate::NO_INST, part: 0 };
                        new.push((sc.tables.emitter_tris.len() as u32, crate::GpuEmitterTri::of(&e, [a, b, c], 0)));
                        sc.tables.emitter_tris.push(e);
                    }
                }
            }
        }
        // The instances with a row of one of them (a mesh's, a model's, an instanced object's copies): each gets its
        // emitters as a new run at the list's end, the sections that emit now one after another, where each starts
        // in its row and the run's first in the instance's record; the run it had, of fewer, stays at no weight.
        if !started.is_empty() && !sc.meshes.is_empty() {
            let LoadedScene { tables, instances, meshes, pool_tris, pool_place, sections, rows, edit, up, .. } = sc;
            let moving = tables.retained.moving_records();
            let mut records: Vec<usize> = tables
                .retained
                .proxies
                .values()
                .flat_map(|p| match &p.draw {
                    scene::Draw::Instance(i) => vec![i.instance],
                    scene::Draw::Rigid(r) => r.records(),
                    scene::Draw::Instanced(c) if c.mesh.is_some() => (c.first..c.first + c.capacity).collect(),
                    _ => vec![],
                })
                .map(|r| r as usize)
                .collect();
            records.sort_unstable();
            // Each mesh's triangles as the pool has them: their section and their index in it (`mesh_places`).
            let mut places: std::collections::HashMap<u32, Vec<(u32, u32)>> = Default::default();
            let (st, mut dark) = (tables.stride, false);
            for r in records {
                // (A record no object draws any more has none; one hidden by its object's visibility has, and a copy
                // that is not shown.)
                let Some(inst) = instances.get(r).filter(|i| !i.hidden || i.masked || i.sub.is_some()).cloned() else { continue };
                let (k, im) = (inst.mesh, meshes[inst.mesh as usize]);
                let sects = im.sections(sections);
                let at = inst.rows as usize;
                if sects.is_empty() || !rows.get(at..at + sects.len()).is_some_and(|x| x.iter().any(|row| started.contains(&scene::row_material(row)))) {
                    continue;
                }
                places.entry(k).or_insert_with(|| mesh_places(&im, sections, pool_place));
                // (A copy that is not shown: its emitters at no weight by its object's entry, `scene::copy_row`.)
                if inst.sub.is_some() && inst.hidden {
                    tables.objects[inst.object as usize * st][3] = 0.0;
                    dark = true;
                }
                let place = &places[&k];
                let at = inst.rows as usize;
                let mine = rows[at..at + sects.len()].to_vec();
                let starts = scene::section_emitters(sects, &mine, &tables.materials);
                let n = scene::rows_emitters(sects, &mine, &tables.materials);
                let old = emitter_run(&tables.emitter_tris, instances, r as u32);
                for e in &mut tables.emitter_tris[old] {
                    e.area = 0.0;
                }
                let first = tables.emitter_tris.len() as u32;
                instances[r].first_emitter = first;
                for (s, start) in starts.iter().enumerate() {
                    rows[at + s][2] = start.map_or(scene::NO_EMITTER, |x| first + x);
                }
                up.words.write(q, Stream::Rows, (at * 16) as u64, bytemuck::cast_slice(&rows[at..at + sects.len()]));
                let numbered = |j: usize| {
                    let (s, i) = place[j];
                    starts.get(s as usize).copied().flatten().map(|x| ((x + i) as usize, scene::row_material(&mine[s as usize]), s))
                };
                let first_tri = edit.pool_first + im.first as usize;
                let (b0, b1) = (im.first as usize, (im.first + im.count) as usize);
                for (e, w) in crate::instance_emitters(&instances[r], r as u32, &pool_tris[b0..b1], first_tri as u32, &numbered, n as usize, &tables.materials, moving.contains(&(r as u32))) {
                    new.push((tables.emitter_tris.len() as u32, crate::GpuEmitterTri::of(&e, w, 0)));
                    tables.emitter_tris.push(e);
                }
            }
            if dark {
                up.heap.write(q, Table::Objects, 0, bytemuck::cast_slice(&tables.objects));
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
        let LoadedScene { tables: e, base: b, up, pool_tris, tris: flat, tri_order, edit, instances, codes_sent: sent, .. } = &mut self.sc;
        let size = std::mem::size_of::<crate::GpuEmitterTri>();
        let (mut w_tris, mut w_rows) = (Gather::default(), Gather::default());
        for (i, t) in new {
            w_tris.put((*i as usize * size) as u64, bytemuck::bytes_of(t));
            // (A triangle in world space names its emitter; a mesh's is numbered among its mesh's.)
            if t.inst == crate::NO_INST {
                w_rows.put((t.tri as usize * 16 + 12) as u64, bytemuck::bytes_of(&(i + 1)));
            }
        }
        w_rows.flush_section(&g.queue, &up.words, Stream::Triangles);
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
        // Only the codes that changed since the last write (a power edit leaves them all as they were; each is a
        // write of its own, the list's records being 48 bytes apart): a new emitter's record was made with code 0.
        sent.resize(e.emitter_tris.len(), u32::MAX);
        for &(i, _) in new {
            sent[i as usize] = u32::MAX;
        }
        for &i in &order[fixed.min(order.len())..] {
            let c = codes[i as usize];
            if sent[i as usize] != c {
                w_tris.put((i as usize * size + std::mem::offset_of!(crate::GpuEmitterTri, code)) as u64, bytemuck::bytes_of(&c));
                sent[i as usize] = c;
            }
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
    /// An object given another material (`World::set_object`): every copy's row names it, with its class, and the
    /// copies' emitters go or are made with it.
    /// Returns how many objects it updated; Err: why it cannot (software traversal, an object the build
    /// flattened or did not place this way, a kernel feature the material needs), and the caller rebuilds.
    /// `THREERT_EDIT_REBUILD=1` always rebuilds (A/B).
    pub(super) fn update_instanced(&mut self, world: &World, ids: &[u32], build: &mut gpu::TopBuild) -> std::result::Result<usize, String> {
        use crate::world::Kind;
        if std::env::var("THREERT_EDIT_REBUILD").is_ok_and(|v| v == "1") {
            return Err("THREERT_EDIT_REBUILD=1".into());
        }
        let more = &self.more_features;
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
            let Kind::Instanced { capacity, procedural, materials, .. } = &n.kind else { continue };
            let (capacity, procedural) = (*capacity, *procedural);
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
            // Another material: every copy's row (a copy that is not shown too: it is shown without its rows made
            // again), its record's bits with it below; the emitters the copies had at no weight, those of a material
            // that emits a run each at the list's end.
            let mut cap = cap;
            let mat = if procedural { None } else { materials.first().and_then(|k| sc.tables.retained.materials.get(k)).copied() };
            if let Some(mat) = mat.filter(|&mat| sc.instances.get(first).is_some_and(|i| scene::row_material(&sc.rows[i.rows as usize]) != mat)) {
                let (gm, features) = (sc.tables.materials[mat as usize], sc.up.kernel.features);
                if gm.tex1[3] != 0 && features & gpu::F_IFUNC == 0 {
                    more.set(more.get() | gpu::F_IFUNC);
                }
                if scene::mesh_class(&gm) == 1 && features & gpu::F_MASKED == 0 {
                    more.set(more.get() | gpu::F_MASKED);
                }
                let emits = scene::emissive(&gm);
                let mut w_rows = Gather::default();
                for r in first..first + capacity as usize {
                    let run = emitter_run(&sc.tables.emitter_tris, &sc.instances, r as u32);
                    reweigh |= !run.is_empty();
                    sc.tables.emitter_tris[run].iter_mut().for_each(|e| e.area = 0.0);
                    let inst = &mut sc.instances[r];
                    let mesh = sc.meshes[inst.mesh as usize];
                    let mut row = scene::row_of(mat, &sc.tables.materials);
                    row[1] = sc.rows[inst.rows as usize][1];
                    (inst.bits, inst.first_emitter) = (scene::rows_bits(&[row]), 0);
                    // (Where this update places the copy: its emitters are made there, as the records below have it.)
                    (inst.world, inst.hidden) = scene::instance_world(m, t, r - first);
                    if emits {
                        let at = sc.tables.emitter_tris.len();
                        (inst.first_emitter, row[2]) = (at as u32, at as u32);
                        let (a, b) = (mesh.first as usize, (mesh.first + mesh.count) as usize);
                        let made = crate::instance_emitters(inst, r as u32, &sc.pool_tris[a..b], (sc.edit.pool_first + a) as u32, &|k| Some((k, mat, 0)), mesh.count as usize, &sc.tables.materials, true);
                        for (j, (e, w)) in made.into_iter().enumerate() {
                            self.pending_emitters.push(((at + j) as u32, crate::GpuEmitterTri::of(&e, w, 0)));
                            sc.tables.emitter_tris.push(e);
                        }
                        reweigh = true;
                    }
                    sc.rows[inst.rows as usize] = row;
                    w_rows.put(inst.rows as u64 * 16, bytemuck::bytes_of(&row));
                }
                if emits {
                    sc.tables.emissive.insert(mat);
                }
                w_rows.flush_section(&self.g.queue, &sc.up.words, Stream::Rows);
                cap = capacity as usize;
            }
            // (A record's slot is `inst_base` + it, a procedural copy's record the first procedural one + its index.)
            let rec0 = if procedural { sc.proc_first + first } else { first };
            let slot0 = top.inst_base() + rec0;
            for i in 0..cap {
                let (wm, hidden) = scene::instance_world(m, t, i);
                let old = sc.gpu_instances[rec0 + i];
                let (mut emits, mut casts) = (false, n.cast_shadow);
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
                    let mine = &sc.rows[inst.rows as usize..][..im.sect_count as usize];
                    emits = scene::rows_emitters(im.sections(&sc.sections), mine, &sc.tables.materials) > 0;
                    casts = scene::rows_cast(mine, n.cast_shadow);
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
                    (top.record(inst.gpu(old.info[0]), inst.mesh, casts), scene::copy_row(row, emits && hidden))
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
                top.moved(slot0 + i, wm, Some((casts, hidden)));
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
    /// (previous rows kept for motion vectors), the instance of the nodes left alone the object's where the object
    /// moved (its emitters moving ones from its first move, `emitters_start_moving`), and their flattened triangles
    /// are re-transformed in place; the commit builds the dynamic BLASes (if they have such triangles) and what is
    /// above the instances (the TLAS, or software traversal's top tree over its units). A hidden model's parts are
    /// placed as a shown one's, unseen. False when it cannot (software traversal without units, a part that is no
    /// unit, or flattened triangles there; a pose that names other nodes; a model changed): the caller rebuilds.
    pub(super) fn repose_rigid(&mut self, world: &World, ids: &[u32], build: &mut gpu::TopBuild) -> Result<bool> {
        // (Nothing is changed unless all of it can be.)
        {
            let Some(top) = gpu::Top::of(&mut self.sc.up.accel, &mut self.sc.up.units) else { return Ok(false) };
            let (retained, instances) = (&self.sc.tables.retained, &self.sc.instances);
            let placed = |id: &u32| retained.rigid(*id).is_some_and(|r| r.rest.finals.len() == r.rest.mats.len() && r.records().iter().all(|&i| instances.get(i as usize).is_some_and(|x| top.has_mesh(x.mesh))));
            // (A pose that names other nodes is other parts.)
            let same_parts = |id: &u32| match world.objects.get(id).map(|o| &o.kind) {
                Some(crate::world::Kind::Model { animation, .. }) => retained.rigid(*id).is_some_and(|r| animation.as_ref().map_or(String::new(), scene::rigid_pose_key) == r.pose),
                _ => false,
            };
            if !ids.iter().all(|id| placed(id) && same_parts(id)) {
                return Ok(false);
            }
        }
        let mut placements: Vec<(u32, glam::Mat4)> = vec![];
        // (Object, where it stands now, and the instance of its nodes left alone if this is its first move.)
        let mut worlds: Vec<(u32, glam::Mat4, Option<u32>)> = vec![];
        let (mut lo, mut hi) = (usize::MAX, 0usize);
        for id in ids {
            let Some(o) = world.objects.get(id) else { return Ok(false) };
            let crate::world::Kind::Model { uri, animation, .. } = &o.kind else { return Ok(false) };
            let globals = scene::model_gltf(uri, &self.assets)?.node_globals(animation.as_ref())?;
            let m = world.world_matrix(*id);
            let r = self.sc.tables.retained.rigid(*id).expect("a rigid model (`commit`)");
            // (The nodes its clip leaves alone move with the object alone.)
            let moved = m != r.world;
            placements.extend(r.instances.iter().map(|&(node, i)| (i, m * globals[node])));
            if moved {
                placements.extend(r.still.map(|i| (i, m)));
                worlds.push((*id, m, r.still.filter(|_| !r.still_moves)));
            }
            let d = &r.rest;
            let (first, verts, attrs, blas) = &mut self.sc.dynamic;
            for (j, (node, p, n, uv, tg)) in r.rest_local.iter().enumerate() {
                let (pw, nw, uvw, tw) = scene::placed_tri(scene::RigidModel::part_world(m, *node, &globals), p, n, uv, *tg);
                let a = scene::tri_attr(nw, uvw, tw, d.mats[j]);
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
        for (id, m, first_move) in worlds {
            if let Some(scene::Draw::Rigid(r)) = self.sc.tables.retained.proxies.get_mut(&id).map(|p| &mut p.draw) {
                (r.world, r.still_moves) = (m, true);
            }
            if let Some(still) = first_move {
                let (to, _) = scene::placed_world(m, self.sc.instances[still as usize].world.w_axis.truncate());
                self.emitters_start_moving(still, to);
            }
        }
        // The instances: rows, previous rows (motion vectors), and their places above the triangles (TLAS transforms,
        // or software traversal's units).
        let size = std::mem::size_of::<scene::GpuInstance>() as u64;
        let (mut ilo, mut ihi) = (usize::MAX, 0usize);
        {
            let mut top = gpu::Top::of(&mut self.sc.up.accel, &mut self.sc.up.units).expect("checked above");
            for &(i, w) in &placements {
                let inst = &mut self.sc.instances[i as usize];
                let old = self.sc.gpu_instances[i as usize];
                // (A node its clip scales to nothing: at a point, where no ray sees it.)
                let (w, gone) = scene::placed_world(w, inst.world.w_axis.truncate());
                inst.world = w;
                let mut r = top.record(inst.gpu(old.info[0]), inst.mesh, inst.cast_shadow);
                r.p = old.m;
                self.sc.gpu_instances[i as usize] = r;
                (ilo, ihi) = (ilo.min(i as usize), ihi.max(i as usize + 1));
                top.moved(top.inst_base() + i as usize, w, Some((inst.cast_shadow, gone || inst.hidden)));
                // (Rigid models' instances are theirs alone: no other update this commit placed them.)
                self.inst_moving.push(i);
            }
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
            self.upload_dynamic(lo, hi);
        }
        build.instances = true;
        build.dynamic |= lo < hi;
        Ok(true)
    }

    /// Posed skinned models (`scene::Skinned`): each whose pose changed is skinned in its own space by the skinning
    /// pass (gpu.rs `skin`: the CPU sends its pose's matrices). Hardware traversal: the commit builds their BLASes
    /// and the TLAS over their new bounds. Software traversal: each mesh's tree keeps its shape and takes its boxes
    /// again on the CPU, from the model's corners placed by the pose (`units::refit`), and the commit makes the top
    /// tree over the units' new bounds. One that is hidden keeps the pose its mesh has: it is skinned when it is
    /// shown (`show_meshes`). False when one is another model now, its new pose mirrors other primitives than the one
    /// its mesh was made for, or (software traversal) its mesh is no unit: the caller rebuilds. (The CPU keeps no
    /// triangles of such a pose: `LiveCore::mesh_tris` skins it when asked.)
    pub(super) fn repose_skinned(&mut self, world: &World, ids: &[u32], build: &mut gpu::TopBuild) -> Result<bool> {
        use rayon::prelude::*;
        if self.sc.up.skin.is_none() || (self.sc.up.accel.is_none() && self.sc.up.units.is_none()) {
            return Ok(false);
        }
        let r = &self.sc.tables.retained;
        let mut jobs: Vec<(u32, crate::desc::Animation)> = vec![];
        for id in ids {
            let Some(scene::MeshKind::Skinned(g)) = r.instance(*id).map(|i| i.mesh) else { unreachable!("a posed skinned model (`commit`)") };
            let sk = &r.skinned[g as usize];
            let Some(o) = world.objects.get(id) else { return Ok(false) };
            let crate::world::Kind::Model { uri, nodes, animation: Some(a), .. } = &o.kind else { return Ok(false) };
            if *uri != sk.uri || *nodes != sk.nodes {
                return Ok(false);
            }
            if world.shown(*id) && *a != sk.animation {
                jobs.push((g, a.clone()));
            }
        }
        if jobs.is_empty() {
            return Ok(true);
        }
        // The poses' matrices (the models in parallel).
        let t_skin = Instant::now();
        let assets = &self.assets;
        let palettes: Vec<Result<Option<Vec<crate::skin::PaletteRows>>>> = jobs
            .par_iter()
            .map(|(g, a)| {
                let sk = &r.skinned[*g as usize];
                let model = scene::model_gltf(&sk.uri, assets)?;
                if model.mirrored(Some(a))? != sk.mirrors {
                    return Ok(None);
                }
                Ok(Some(model.palette(Some(a))?))
            })
            .collect();
        let mut work = Vec::with_capacity(jobs.len());
        for ((g, _), rows) in jobs.iter().zip(palettes) {
            let Some(rows) = rows? else { return Ok(false) };
            work.push((*g, rows));
        }
        // Software traversal: the meshes' trees, their boxes taken again from the corners placed by the poses (the
        // models in parallel). Nothing is written unless every one is a unit with such a tree.
        let mut trees: Vec<(u32, u32, Vec<crate::bvh::GpuNode>, (glam::Vec3A, glam::Vec3A))> = vec![];
        if let Some(units) = &self.sc.up.units {
            let (skins, shapes) = (&self.sc.skins, &self.sc.skin_trees);
            let made: Vec<Option<(u32, u32, Vec<crate::bvh::GpuNode>, (glam::Vec3A, glam::Vec3A))>> = work
                .par_iter()
                .map(|(g, rows)| {
                    let sk = &r.skinned[*g as usize];
                    let ((vertices, index), shape) = (skins.get(sk.source as usize)?, shapes.get(sk.source as usize)?);
                    let tree = units.trees.get(sk.mesh as usize).copied().flatten().filter(|t| t.len as usize == shape.len() && !shape.is_empty() && index.len() == sk.order.len())?;
                    let mut nodes = shape.clone();
                    // (The model's vertices placed once each; a triangle's corners through its indices.)
                    let placed: Vec<glam::Vec3A> = vertices.iter().map(|c| glam::Vec3A::from(crate::skin::skin_corner(c, rows).0)).collect();
                    let bounds = crate::units::refit(&mut nodes, |i| index[i].map(|v| placed[v as usize]));
                    Some((sk.mesh, tree.root, nodes, bounds))
                })
                .collect();
            for m in made {
                let Some(m) = m else { return Ok(false) };
                trees.push(m);
            }
        }
        self.cpu_skin_ms += t_skin.elapsed().as_secs_f32() * 1000.0;
        let t_up = Instant::now();
        // Motion vectors: the GPU still holds the previous pose of these meshes (and of the last re-skin's, if others):
        // keep it in `prev_verts`, each mesh's where its instance says, before the pass writes the new one.
        let posed: Vec<(usize, usize, usize)> = jobs
            .iter()
            .map(|(g, _)| {
                let sk = &self.sc.tables.retained.skinned[*g as usize];
                let i = &self.sc.instances[sk.instance as usize];
                (i.verts as usize, i.prev as usize, self.sc.meshes[sk.mesh as usize].vcount as usize)
            })
            .collect();
        self.keep_prev_spans(&posed.iter().chain(&self.pose_spans).copied().collect::<Vec<_>>());
        if !self.g.skin(&self.sc.up, &work) {
            return Ok(false);
        }
        self.pose_spans = posed;
        self.pose_reposed = true;
        for (g, a) in jobs {
            let sk = &mut self.sc.tables.retained.skinned[g as usize];
            sk.animation = a;
            build.meshes.push(sk.mesh);
        }
        // (Software traversal: the trees where they lie, and the units' bounds for the top tree.)
        for (mesh, root, nodes, (lo, hi)) in trees {
            self.g.write_nodes(&self.sc.up.bg_parts.nodes, root, &nodes);
            if let Some(units) = self.sc.up.units.as_mut() {
                units.set_bounds(mesh, lo, hi);
            }
        }
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
        self.upload_dynamic(lo, hi);
        self.sc.up.heap.write(&self.g.queue, Table::Objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
        build.dynamic = true;
        self.cpu_upload_ms += t_up.elapsed().as_secs_f32() * 1000.0;
        Ok(true)
    }

    /// The emitters of instances that moved (those of a mesh that emits): their runs of the light list taken again on
    /// the GPU, through the records just queued; the light tree's moving subtree after (`move_emitters`).
    pub(super) fn fill_moved(&mut self, records: impl Iterator<Item = u32>) {
        // An instance whose emitters this commit has still to write (`pending_emitters`: a still mesh's first move
        // makes it new ones, at the list's end) needs no fill: they are made where it stands now, and the list on the
        // GPU does not hold them yet (a write past its end lands on its last row).
        let pending: std::collections::HashSet<u32> = self.pending_emitters.iter().map(|p| p.0).collect();
        // (Runs that follow each other as one: an instanced object's copies'.)
        let mut runs: Vec<[u32; 2]> = vec![];
        for r in records.map(|i| emitter_run(&self.sc.tables.emitter_tris, &self.sc.instances, i)).filter(|r| !r.is_empty() && !pending.contains(&(r.start as u32))) {
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

    /// The dynamic tail's triangles `lo..hi` as the CPU has them now (`sc.dynamic`), to the GPU: their vertices, three
    /// a triangle from the first vertex that moves (their indices and rows are the build's).
    fn upload_dynamic(&self, lo: usize, hi: usize) {
        let (_, _, attrs, corners) = &self.sc.dynamic;
        let vertices = crate::indexed::apart_vertices(&corners[lo * 3..hi * 3], &attrs[lo..hi]);
        let at = (self.sc.dynamic_verts.0 as usize + lo * 3) * crate::indexed::VERTEX_BYTES;
        self.g.queue.write_buffer(&self.sc.up.vertices, at as u64, bytemuck::cast_slice(&vertices));
    }

    /// Copies the GPU's current vertices of a span of the dynamic tail's triangles (three each, the first of the
    /// vertices that move) into the previous frame's (submitted before any write queued after it, so it reads what the
    /// last frame traced).
    pub(super) fn keep_prev(&self, (lo, hi): (usize, usize)) {
        let up = &self.sc.up;
        let first = self.sc.dynamic_verts.0 as usize;
        let mut e = self.g.device.create_command_encoder(&Default::default());
        let size = crate::indexed::VERTEX_BYTES;
        e.copy_buffer_to_buffer(&up.vertices, ((first + lo * 3) * size) as u64, &up.prev_vertices, (lo * 3 * size) as u64, ((hi - lo) * 3 * size) as u64);
        self.g.queue.submit([e.finish()]);
    }

    /// Likewise for deforming meshes, each where it lies: (its first vertex in the scene's, its first among the
    /// previous frame's, how many vertices).
    pub(super) fn keep_prev_spans(&self, spans: &[(usize, usize, usize)]) {
        if spans.is_empty() {
            return;
        }
        let up = &self.sc.up;
        let mut e = self.g.device.create_command_encoder(&Default::default());
        for (verts, prev, n) in merge_spans(spans.to_vec()) {
            let size = crate::indexed::VERTEX_BYTES;
            e.copy_buffer_to_buffer(&up.vertices, (verts * size) as u64, &up.prev_vertices, (prev * size) as u64, (n * size) as u64);
        }
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
    /// cannot (software traversal without units, the room is full, a kernel feature the scene lacks, ...); the caller
    /// rebuilds the scene then. `THREERT_EDIT_REBUILD=1` always rebuilds (A/B).
    pub(super) fn edit_geometry(&mut self, world: &World, ch: &crate::world::Changes, build: &mut gpu::TopBuild) -> std::result::Result<(), String> {
        use crate::world::Kind;
        if std::env::var("THREERT_EDIT_REBUILD").is_ok_and(|v| v == "1") {
            return Err("THREERT_EDIT_REBUILD=1".into());
        }
        let more = &self.more_features;
        let LoadedScene { up, tables, edit, meshes, pool_tris, pool_place, instances, gpu_instances, sections, rows, procs, proc_first, moving_instances, skins, .. } = &mut self.sc;
        let (g, assets) = (&self.g, &self.assets);
        let features = up.kernel.features;
        let grow = Grow { g: &self.g, ok: up.particles.is_none() && up.gpu_particles.is_none() };
        // (Hardware traversal's TLAS and BLASes, or software traversal's units: `gpu::Top`.)
        let Some(mut top) = gpu::Top::of(&mut up.accel, &mut up.units) else {
            // Software traversal without units. With units on, that is a scene without an instance yet: the first
            // one brings the unit walk into the kernel (F_UNITS), as hardware traversal's first brings F_INST.
            let first = crate::units_on(true) && ch.added.iter().any(|id| matches!(world.objects.get(id).map(|o| &o.kind), Some(Kind::Mesh { .. } | Kind::Instanced { procedural: false, .. })));
            return Err(if first { "the scene's first instanced mesh (kernel feature)" } else { "software traversal" }.into());
        };
        // The objects to place again: those added, and those whose geometry was replaced.
        let mut todo: std::collections::BTreeSet<u32> = ch.added.clone();
        if !ch.geometries.is_empty() {
            for (id, o) in &world.objects {
                match &o.kind {
                    Kind::Procedural { geometry, .. } | Kind::Mesh { geometry, .. } if ch.geometries.contains(geometry) => {
                        todo.insert(*id);
                    }
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
                    edit.give_rows(r0);
                    edit.objects.give(place.object as usize, cap);
                    drop_mesh_user(mesh, edit, tables, meshes);
                }
                tables.retained.proxies.remove(id);
                continue;
            }
            // A posed rigid model: its parts' instances out, as an instance is; what an edit placed goes back to the
            // room. (One with triangles flattened beside them is the build's to take out.)
            if let Some(r) = tables.retained.rigid(*id) {
                if !r.rest.mats.is_empty() {
                    return Err("an object the build placed otherwise (a model in parts, something it flattened) removed".into());
                }
                for rec in r.records() {
                    let rec = rec as usize;
                    top.clear(inst_base + rec);
                    let run = emitter_run(&tables.emitter_tris, instances, rec as u32);
                    emitters_changed |= !run.is_empty();
                    tables.emitter_tris[run].iter_mut().for_each(|e| e.area = 0.0);
                    if let Some(i) = instances.get_mut(rec) {
                        (i.hidden, i.masked) = (true, false);
                        if edit.in_room(rec) {
                            drop_mesh_user(i.mesh, edit, tables, meshes);
                        }
                    }
                    if edit.in_room(rec) {
                        edit.records.give(rec, 1);
                        edit.give_rows(rec);
                    }
                }
                if let Some(k) = tables.retained.proxies.remove(id).and_then(|p| p.object).filter(|&k| k as usize >= edit.base.2) {
                    edit.objects.give(k as usize, 1);
                }
                continue;
            }
            emitters_changed |= release(*id, tables, instances);
            // (A still model the build placed, whose instance is all of it, is taken out by it too; and a posed skinned
            // model, an instance of its own mesh, where its mesh is one the rays reach by its instance. Its mesh's
            // triangles and its skinning job stay until the next rebuild, as the build's do.)
            let its_own = |i: &scene::InstanceRef| whole(i, instances, meshes) || (matches!(i.mesh, scene::MeshKind::Skinned(_)) && instances.get(i.instance as usize).is_some_and(|x| top.has_mesh(x.mesh)));
            let at = slot_of(*id, edit, tables).or_else(|| tables.retained.instance(*id).filter(|i| its_own(i)).map(|i| (inst_base + i.instance as usize, i.instance as usize)));
            // Drawn, and not at a record an edit can clear (a model with triangles flattened beside its instance,
            // anything the build flattened): a rebuild takes it out.
            if at.is_none() && tables.retained.proxies.contains_key(id) {
                return Err("an object the build placed otherwise (a model in parts, something it flattened) removed".into());
            }
            // (A skinned model an edit placed: with its mesh's triangles go the room of its previous vertices and its
            // slot among the skinned, which the next one placed takes. The build's stay until the next rebuild.)
            if let Some((g, rec)) = tables.retained.instance(*id).and_then(|i| match i.mesh {
                scene::MeshKind::Skinned(g) if edit.in_room(i.instance as usize) => Some((g as usize, i.instance as usize)),
                _ => None,
            }) {
                let mesh = tables.retained.skinned[g].mesh;
                if let (Some(inst), Some(im)) = (instances.get(rec), meshes.get(mesh as usize)) {
                    edit.prev.give(inst.prev as usize, im.vcount as usize);
                    let first = im.vfirst as usize;
                    self.pose_spans.retain(|s| s.0 != first);
                }
                tables.retained.skinned[g].mesh = u32::MAX;
                edit.free_skinned.push(g as u32);
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
                    edit.give_rows(record);
                }
            }
            if let Some(k) = proxy.and_then(|p| p.object).filter(|&k| k as usize >= edit.base.2) {
                edit.objects.give(k as usize, 1);
            }
        }
        let mut new_meshes = vec![];
        let mut pending_procs = vec![];
        let mut split = EditSplit::default();
        // (Vertices; triangles' indices; under software traversal a unit's tree's leaves.)
        let (mut w_cdf, mut w_vert, mut w_index, mut w_leaf, mut w_records, mut w_objects) = (Gather::default(), Gather::default(), Gather::default(), Gather::default(), Gather::default(), Gather::default());
        // Vertex colours (F_VCOLOR).
        let mut w_colors = Gather::default();
        let vcolors = up.vcolors;
        // (geometry, material) -> the mesh built in this batch (objects sharing a replaced geometry share it again).
        let mut fresh: std::collections::HashMap<(u32, bool), u32> = Default::default();
        let mut w_rows = Gather::default();
        let mask = |shadow: bool| if shadow { 1 } else { 2 };
        for id in todo {
            let Some(o) = world.objects.get(&id) else { continue };
            let at = slot_of(id, edit, tables);
            emitters_changed |= release(id, tables, instances);
            // (A mesh that is hidden is placed all the same, as an instance no ray sees: below.)
            let is_mesh = matches!(o.kind, Kind::Mesh { .. });
            if !world.shown(id) && !is_mesh {
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
            let was = at.and_then(|(_, r)| instances.get(r)).filter(|i| !i.hidden || i.masked).map(|i| i.mesh);
            // Drawn but not by a TLAS instance of its own: the build flattened it (movable, or a material instances skip).
            if at.is_none() && tables.retained.proxies.contains_key(&id) {
                return Err("an object the build flattened".into());
            }
            let m = world.world_matrix(id);
            // A mesh that is hidden, or placed without size, is an instance no ray sees, at a point if it has no size
            // (`Instance::masked`, as the build places it): it comes back in place.
            let (m, gone) = if is_mesh { scene::placed_world(m, glam::Vec3::ZERO) } else { (m, false) };
            let unseen = is_mesh && (gone || !world.shown(id));
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
            let (record, blas, entry) = match &o.kind {
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
                    (at.map(|(_, r)| r), Placed::Proc(blas, p, gpu), entry)
                }
                Kind::Model { uri, nodes, animation, functions } => {
                    // A model already in the scene: another instance of its mesh, of each of its parts' meshes if it is
                    // a posed rigid one, or of a mesh of its own if it is a posed skinned one. A still one the scene
                    // has not got: its mesh is made here, its materials and textures were before (`place_models`); a
                    // posed one the scene has not got is the rebuild's.
                    if !functions.is_empty() {
                        return Err("a model with material functions (its materials load with a rebuild)".into());
                    }
                    if features & gpu::F_INST == 0 || m.determinant() == 0.0 || std::env::var("THREERT_INSTANCING").is_ok_and(|v| v == "0") {
                        return Err("a model the build would flatten (no size, instancing off, or the scene's first instance)".into());
                    }
                    // (Shadow rays pass the glass among a caster's other sections under a kernel feature: a scene where
                    // no instance of the model cast a shadow was built without it.)
                    let glass_caster = |its: &[scene::Row]| scene::rows_bits(its) & scene::INST_GLASS != 0 && scene::rows_cast(its, o.cast_shadow) && features & gpu::F_IGLASS == 0;
                    let glb = scene::model_gltf(uri, assets).map_err(|e| e.to_string())?;
                    if let Some(a) = animation.as_ref().filter(|_| glb.animated() && !(nodes.is_none() && glb.rigid())) {
                        // A posed skinned model the scene has: a mesh of its own in the room, as the build makes one. Its
                        // triangles are skinned here once, for the pose it comes in; the pass skins it from its next
                        // pose on, from the corners the scene's others of the model are skinned from.
                        if top.sw() {
                            // (Its triangles and its tree need room, which software traversal's scenes have only
                            // with geometry of their own to edit, and which does not grow there.)
                            return Err("a skinned model added (software traversal)".into());
                        }
                        if at.is_some() {
                            return Err("a skinned model placed again".into());
                        }
                        let t = *tables.retained.skinned_model.get(&(uri.clone(), nodes.clone())).ok_or("a skinned model not in the scene (its materials and textures load with a rebuild)")?;
                        let (order, source, palette_len, its_mesh, mirrors) = {
                            let s = &tables.retained.skinned[t as usize];
                            (s.order.clone(), s.source, s.palette_len, s.mesh, s.mirrors.clone())
                        };
                        if glb.mirrored(Some(a)).map_err(|e| e.to_string())? != mirrors {
                            return Err("a skinned model whose pose mirrors other primitives than the scene's (its triangles are a build's to order)".into());
                        }
                        let mine = tables.retained.mesh_rows.get(&its_mesh).ok_or("a model's mesh without its rows")?.clone();
                        if glass_caster(&mine) {
                            more.set(more.get() | gpu::F_IGLASS);
                        }
                        let posed = glb.triangles(glam::Mat4::IDENTITY, nodes.as_deref(), Some(a)).map_err(|e| e.to_string())?;
                        let n = order.len();
                        if order.iter().any(|&(k, _, _)| k as usize >= posed.len()) {
                            return Err("a skinned model whose triangles are not the scene's".into());
                        }
                        // Its vertices as it is posed, each from a corner that is it (the model's vertices and its
                        // triangles' indices are the source's, as every mesh of the model has them).
                        let (model, index) = skins.get(source as usize).ok_or("a skinned model without its vertices")?;
                        let nv = model.len();
                        if index.len() != n {
                            return Err("a skinned model whose triangles are not the scene's".into());
                        }
                        let mut vertices: Vec<crate::indexed::Vertex> = vec![[[0.0f32; 4]; 3]; nv];
                        let mut t = Vec::with_capacity(n);
                        for (j, &(k, _, _)) in order.iter().enumerate() {
                            let q = &posed[k as usize];
                            for c in 0..3 {
                                let v = index[j][c] as usize;
                                // (A position that is not finite is the mesh's origin, as the pass writes it; a
                                // model without normals has none: the triangle's own where it is hit.)
                                let p = if q.p[c].is_finite() { q.p[c] } else { glam::Vec3::ZERO };
                                let normal = if model[v].t[3].to_bits() & crate::skin::CORNER_NORMAL != 0 { q.n[c] } else { glam::Vec3::ZERO };
                                vertices[v][0] = [p.x, p.y, p.z, q.uv[c].x];
                                vertices[v][1] = [normal.x, normal.y, normal.z, q.uv[c].y];
                            }
                            if let Some(tangent) = q.tangent {
                                for c in 0..3 {
                                    let v = index[j][c] as usize;
                                    if model[v].t[3].to_bits() & crate::skin::CORNER_TANGENT != 0 {
                                        vertices[v][2] = [tangent[c].0.x, tangent[c].0.y, tangent[c].0.z, tangent[c].1];
                                    }
                                }
                            }
                            t.push(q.p.map(glam::Vec3A::from));
                        }
                        // Its triangles, its vertices and their previous frame's positions: holes that fit, else the
                        // rooms' ends.
                        let first = match edit.tris.take(n) {
                            Some(f) => f,
                            None if !top.sw() && grow.tris(&mut edit.tris, n, &mut up.words, vcolors) => {
                                edit.grows += 1;
                                edit.rebind = true;
                                edit.tris.take(n).ok_or("edit room: triangles")?
                            }
                            None => return Err("edit room: triangles (particles, or the device's limits)".into()),
                        };
                        let vfirst = take_verts(edit, &grow, &mut up.vertices, nv, top.sw())?;
                        let prev = match edit.prev.take(nv) {
                            Some(f) => f,
                            None if grow.prev(&mut edit.prev, nv, &mut up.prev_vertices) => {
                                edit.grows += 1;
                                edit.rebind = true;
                                edit.prev.take(nv).ok_or("edit room: previous vertices")?
                            }
                            None => return Err("edit room: previous vertices (particles, or the device's limits)".into()),
                        };
                        // A freed mesh's slot (and its sections', where they are as many at least), else new ones: the
                        // model's sections, its own copy (a slot's are written over by the next mesh that takes it).
                        let dead = edit.dead_meshes.pop();
                        let sects = meshes[its_mesh as usize].sections(sections).to_vec();
                        let ns = sects.len();
                        let sect_first = match dead.map(|k| meshes[k as usize]).filter(|m| m.sect_count as usize >= ns) {
                            Some(m) => m.sect_first,
                            None => {
                                sections.resize(sections.len() + ns, scene::Section { first: 0, count: 0, slot: 0 });
                                (sections.len() - ns) as u32
                            }
                        };
                        sections[sect_first as usize..][..ns].copy_from_slice(&sects);
                        // (Other poses reach further: its bounds wider, as the build's.)
                        let own = scene::own_inst_mesh(&t, (first - edit.pool_first) as u32);
                        let pad = if a.nodes.is_empty() { (own.hi - own.lo) * 0.25 } else { glam::Vec3A::splat((own.hi - own.lo).length()) };
                        let im = scene::InstMesh { lo: own.lo - pad, hi: own.hi + pad, deforms: true, sect_first, sect_count: ns as u32, vfirst: vfirst as u32, vcount: nv as u32, ..own };
                        let mesh = match &mut top {
                            gpu::Top::Hw(accel) => match dead {
                                Some(k) => {
                                    g.set_mesh(accel, k, first, &im, sections);
                                    k
                                }
                                None => g.add_mesh(accel, first, &im, sections),
                            },
                            gpu::Top::Sw(_) => unreachable!("refused above"),
                        };
                        if mesh as usize == meshes.len() {
                            meshes.push(im);
                        } else {
                            meshes[mesh as usize] = im;
                        }
                        edit.managed.insert(mesh);
                        *edit.users.entry(mesh).or_default() += 1;
                        new_meshes.push(mesh);
                        split.added += 1;
                        w_vert.put((vfirst * crate::indexed::VERTEX_BYTES) as u64, bytemuck::cast_slice(&vertices));
                        w_index.put((first * 12) as u64, bytemuck::cast_slice(&index[..]));
                        if vcolors {
                            w_colors.put((first * 12) as u64, bytemuck::cast_slice(&vec![scene::WHITE; n]));
                        }
                        // (It has not moved yet: its previous vertices are its own.)
                        q.write_buffer(&up.prev_vertices, (prev * crate::indexed::VERTEX_BYTES) as u64, bytemuck::cast_slice(&vertices));
                        let rel = first - edit.pool_first;
                        if pool_tris.len() < rel + n {
                            pool_tris.resize(rel + n, [glam::Vec3A::ZERO; 3]);
                        }
                        pool_tris[rel..rel + n].copy_from_slice(&t);
                        // Its slot among the skinned (one an edit took out again, else a new one), and the pass's job.
                        let slot = edit.free_skinned.pop().unwrap_or(tables.retained.skinned.len() as u32);
                        let sk = scene::Skinned { mesh, uri: uri.clone(), nodes: nodes.clone(), animation: a.clone(), order, object: id, instance: 0, source, palette_len, mirrors };
                        if slot as usize == tables.retained.skinned.len() {
                            tables.retained.skinned.push(sk);
                        } else {
                            tables.retained.skinned[slot as usize] = sk;
                        }
                        let pass = up.skin.as_mut().ok_or("a scene without a skinning pass")?;
                        edit.rebind |= pass.set_job(&g.device, q, slot as usize, vfirst as u32, nv as u32, source, palette_len);
                        let inst = scene::Instance { mesh, world: m, object: object as u32, cast_shadow: scene::rows_cast(&mine, o.cast_shadow), name: o.name.clone(), sub: None, hidden: false, masked: false, first_emitter: 0, rows: 0, bits: scene::rows_bits(&mine) | scene::INST_DEFORMS, prev: prev as u32, verts: 0 };
                        let (lo, hi) = im.placed_bounds(m);
                        let entry = scene::object_row(m.transform_point3(glam::Vec3::ZERO), o.power, hi - lo, o.custom, [1.0; 3]);
                        (None, Placed::Skinned(inst, mine, slot), entry)
                    } else if let Some(a) = animation {
                        let parts = nodes.is_none().then(|| tables.retained.rigid_meshes.get(&(uri.clone(), scene::rigid_pose_key(a)))).flatten().ok_or("an animated model that is no rigid one of the scene's with this pose's nodes (its meshes are a build's to make)")?.clone();
                        let globals = scene::model_gltf(uri, assets).and_then(|g| g.node_globals(Some(a))).map_err(|e| e.to_string())?;
                        let (mut lo, mut hi) = (glam::Vec3A::splat(f32::MAX), glam::Vec3A::splat(f32::MIN));
                        let mut placed = vec![];
                        for &(node, mesh) in &parts {
                            if !top.has_mesh(mesh) {
                                return Err("a model whose mesh is no unit (software traversal flattens it)".into());
                            }
                            let mine = tables.retained.mesh_rows.get(&mesh).ok_or("a model's mesh without its rows")?.clone();
                            if glass_caster(&mine) {
                                more.set(more.get() | gpu::F_IGLASS);
                            }
                            // (A part without size, a node its clip scales to nothing: at a point, as the build has it.)
                            let (w_n, _) = scene::placed_world(scene::RigidModel::part_world(m, node, &globals), m.w_axis.truncate());
                            let (plo, phi) = meshes[mesh as usize].placed_bounds(w_n);
                            (lo, hi) = (lo.min(plo), hi.max(phi));
                            let inst = scene::Instance { mesh, world: w_n, object: object as u32, cast_shadow: scene::rows_cast(&mine, o.cast_shadow), name: o.name.clone(), sub: None, hidden: false, masked: false, first_emitter: 0, rows: 0, bits: scene::rows_bits(&mine), prev: 0, verts: 0 };
                            placed.push((node, inst, mine));
                        }
                        for &(_, mesh) in &parts {
                            *edit.users.entry(mesh).or_default() += 1;
                        }
                        let entry = scene::object_row(m.transform_point3(glam::Vec3::ZERO), o.power, (hi - lo).max(glam::Vec3A::ZERO), o.custom, [1.0; 3]);
                        (None, Placed::Parts(placed), entry)
                    } else {
                        let mesh = match tables.retained.model_mesh.get(&(uri.clone(), nodes.clone())).copied() {
                            Some(k) => k,
                            None => {
                                // A model the scene has not got: its mesh made here as the build makes it, model-space
                                // triangles by section, a material of the file (made before: `place_models`).
                                let t0 = Instant::now();
                                let local = glb.triangles(glam::Mat4::IDENTITY, nodes.as_deref(), None).map_err(|e| e.to_string())?;
                                let mut classed: [Vec<(usize, u32)>; 4] = Default::default();
                                let mut emitting: std::collections::HashSet<u32> = Default::default();
                                for (k, t) in local.iter().enumerate() {
                                    let mat = *tables.retained.asset_materials.get(&(uri.clone(), t.material.unwrap_or(usize::MAX))).ok_or("a model's material that was not made")?;
                                    let gm = &tables.materials[mat as usize];
                                    classed[scene::mesh_class(gm) as usize].push((k, mat));
                                    if scene::emissive(gm) {
                                        emitting.insert(mat);
                                    }
                                }
                                scene::flatten_coincident(&mut classed, &emitting, &|k| local[k].p);
                                if !classed[scene::FLAT].is_empty() {
                                    return Err("a model with triangles the build flattens at each placement".into());
                                }
                                if !classed[1].is_empty() && features & gpu::F_MASKED == 0 {
                                    more.set(more.get() | gpu::F_MASKED);
                                }
                                let (order, sects, mine) = scene::model_sections(&scene::kept(&classed));
                                if order.is_empty() {
                                    return Err("a model without triangles".into());
                                }
                                let tris: Vec<[glam::Vec3A; 3]> = order.iter().map(|&(k, _, _)| local[k].p.map(glam::Vec3A::from)).collect();
                                let attrs = order.iter().map(|&(k, section, rank)| scene::section_attr(local[k].n, local[k].uv, local[k].tangent, section, rank)).collect();
                                let data = scene::MeshData { mesh: scene::own_inst_mesh(&tris, 0), tris, attrs, colors: None, sections: sects };
                                split.tris += t0.elapsed().as_secs_f64() * 1e3;
                                let mut cx = Meshing { g, edit: &mut *edit, grow: &grow, vertices: &mut up.vertices, words: &mut up.words, nodes: &up.bg_parts.nodes, top: &mut top, meshes: &mut *meshes, sections: &mut *sections, pool_tris: &mut *pool_tris, pool_place: &mut *pool_place, split: &mut split, w_vert: &mut w_vert, w_index: &mut w_index, w_leaf: &mut w_leaf, w_colors: &mut w_colors, vcolors };
                                let k = make_mesh(&mut cx, data, false, None)?;
                                new_meshes.push(k);
                                tables.retained.mesh_rows.insert(k, mine);
                                tables.retained.model_mesh.insert((uri.clone(), nodes.clone()), k);
                                k
                            }
                        };
                        let im = meshes[mesh as usize];
                        if !top.has_mesh(mesh) {
                            return Err("a model whose mesh is no unit (it deforms; software traversal flattens it)".into());
                        }
                        // (Its rows, the mesh's own, are placed with its record below.)
                        let mine = tables.retained.mesh_rows.get(&mesh).ok_or("a model's mesh without its rows")?.clone();
                        if glass_caster(&mine) {
                            more.set(more.get() | gpu::F_IGLASS);
                        }
                        *edit.users.entry(mesh).or_default() += 1;
                        if let Some(k) = was {
                            drop_mesh_user(k, edit, tables, meshes);
                        }
                        let inst = scene::Instance { mesh, world: m, object: object as u32, cast_shadow: scene::rows_cast(&mine, o.cast_shadow), name: o.name.clone(), sub: None, hidden: false, masked: false, first_emitter: 0, rows: 0, bits: scene::rows_bits(&mine), prev: 0, verts: 0 };
                        let (lo, hi) = im.placed_bounds(m);
                        let entry = scene::object_row(m.transform_point3(glam::Vec3::ZERO), o.power, hi - lo, o.custom, [1.0; 3]);
                        (at.map(|(_, r)| r), Placed::Mesh(inst, mine), entry)
                    }
                }
                Kind::Mesh { .. } | Kind::Instanced { procedural: false, .. } => {
                    // A mesh, or (added in place) an instanced mesh: their mesh made alike. A mesh with several
                    // materials has a geometry group's triangles with each (`scene::by_groups`); an instanced mesh
                    // its first.
                    let (geometry, ids, capacity) = match &o.kind {
                        Kind::Mesh { geometry, material, materials, .. } => (*geometry, if materials.len() > 1 { materials.clone() } else { vec![*material] }, None),
                        Kind::Instanced { geometry, materials, capacity, .. } => (*geometry, vec![*materials.first().ok_or("an instanced mesh without a material")?], Some(*capacity as usize)),
                        _ => unreachable!(),
                    };
                    let geo = world.geometries.get(&geometry).ok_or("unknown geometry")?;
                    let grouped = scene::by_groups(geo, ids.len());
                    let ids = if grouped { ids } else { ids[..1].to_vec() };
                    let geometry = &geometry;
                    if features & gpu::F_INST == 0 {
                        return Err("the scene's first instanced mesh (kernel feature)".into());
                    }
                    // An instanced object placed without size, or instancing off (the build flattens).
                    if (capacity.is_some() && m.determinant() == 0.0) || std::env::var("THREERT_INSTANCING").is_ok_and(|v| v == "0") {
                        return Err("an instanced object without size, or instancing off".into());
                    }
                    let mats = ids.iter().map(|k| tables.retained.materials.get(k).copied().ok_or("unknown material")).collect::<std::result::Result<Vec<u32>, _>>()?;
                    let mat = mats[0];
                    for &mat in &mats {
                        let gm = tables.materials[mat as usize];
                        if gm.tex1[3] != 0 && features & gpu::F_IFUNC == 0 {
                            more.set(more.get() | gpu::F_IFUNC);
                        }
                        if scene::mesh_class(&gm) == 1 && features & gpu::F_MASKED == 0 {
                            more.set(more.get() | gpu::F_MASKED);
                        }
                    }
                    // An emitting material: a mesh as any other, each instance's emitters in the light list (given
                    // below, where its record is known; scene.rs; ROADMAP § 6 step 3).
                    let emits = scene::emissive(&tables.materials[mat as usize]);
                    let colored = ids.iter().any(|k| world.materials.get(k).is_some_and(|m| m.vertex_colors)) && !geo.colors.is_empty();
                    if colored && !vcolors {
                        return Err("the scene's first vertex colours (kernel feature)".into());
                    }
                    // (The mesh every such object of the geometry shares, by section: the geometry whole, or its groups.
                    // Its materials are this instance's rows.)
                    let key = (*geometry, grouped);
                    let kept = (!ch.geometries.contains(geometry)).then(|| tables.retained.own_mesh.get(&key).copied()).flatten();
                    let mesh = match fresh.get(&key).copied().or(kept) {
                        // (A mesh made without its geometry's colours, for objects that read none.)
                        Some(k) if colored && !meshes[k as usize].colored => return Err("vertex colours on a mesh made without them".into()),
                        Some(k) => k,
                        None => {
                            let t0 = Instant::now();
                            // (With its colours where an object of it reads them: this one, or one that shares the mesh.)
                            let had = tables.retained.own_mesh.get(&key).is_some_and(|&k| meshes[k as usize].colored);
                            let data = scene::section_mesh(geo, grouped, 0, 0, colored || had);
                            if data.tris.is_empty() {
                                if let Some((slot, _)) = at {
                                    top.clear(slot);
                                }
                                // (No longer drawn: its mesh a user fewer now, as when it is hidden.)
                                if let Some(k) = undraw(tables.retained.proxies.get(&id).map(|p| &p.draw), instances, procs) {
                                    drop_mesh_user(k, edit, tables, meshes);
                                }
                                continue;
                            }
                            split.tris += t0.elapsed().as_secs_f64() * 1e3;
                            // The same sections as the mesh it replaces: that mesh, rewritten where it is.
                            let same = tables.retained.own_mesh.get(&key).copied().filter(|&k| meshes[k as usize].sections(sections) == data.sections.as_slice());
                            let mut cx = Meshing { g, edit: &mut *edit, grow: &grow, vertices: &mut up.vertices, words: &mut up.words, nodes: &up.bg_parts.nodes, top: &mut top, meshes: &mut *meshes, sections: &mut *sections, pool_tris: &mut *pool_tris, pool_place: &mut *pool_place, split: &mut split, w_vert: &mut w_vert, w_index: &mut w_index, w_leaf: &mut w_leaf, w_colors: &mut w_colors, vcolors };
                            let k = make_mesh(&mut cx, data, colored, same)?;
                            new_meshes.push(k);
                            fresh.insert(key.clone(), k);
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
                            let filler = scene::Instance { mesh, world: glam::Mat4::IDENTITY, object: 0, cast_shadow: false, name: String::new(), sub: None, hidden: true, masked: false, first_emitter: 0, rows: 0, bits: 0, prev: 0, verts: 0 };
                            instances.resize(r0 + cap, filler);
                        }
                        if gpu_instances.len() < r0 + cap {
                            gpu_instances.resize(r0 + cap, scene::GpuInstance::default());
                        }
                        if tables.objects.len() < (o0 + cap) * st {
                            tables.objects.resize((o0 + cap) * st, [0.0; 4]);
                        }
                        // (A row a copy, the copies' in a run as their records are.)
                        let run = rows_for(edit, &grow, &mut up.words, instances, meshes, r0, cap)?;
                        for (i, &(wm, hidden)) in placed.iter().enumerate() {
                            let row_at = run + i;
                            let mut row = scene::row_of(mat, &tables.materials);
                            row[1] = edit.pool_first as u32 + im.first;
                            let mut inst = scene::Instance { mesh, world: wm, object: (o0 + i) as u32, cast_shadow: scene::rows_cast(&[row], o.cast_shadow), name: o.name.clone(), sub: Some(i as u32), hidden, masked: false, first_emitter: 0, rows: row_at as u32, bits: scene::rows_bits(&[row]), prev: 0, verts: im.vfirst };
                            // A copy whose material emits: its emitters, a run at the light list's end; moving ones
                            // (`update_instanced` changes the copies in place), a hidden copy's at no weight.
                            if emits {
                                let first = tables.emitter_tris.len();
                                (inst.first_emitter, row[2]) = (first as u32, first as u32);
                                let (a, b) = (im.first as usize, (im.first + im.count) as usize);
                                let made = crate::instance_emitters(&inst, (r0 + i) as u32, &pool_tris[a..b], (edit.pool_first + a) as u32, &|k| Some((k, mat, 0)), im.count as usize, &tables.materials, true);
                                for (j, (e, w)) in made.into_iter().enumerate() {
                                    new_emitters.push(((first + j) as u32, crate::GpuEmitterTri::of(&e, w, 0)));
                                    tables.emitter_tris.push(e);
                                }
                                tables.emissive.insert(mat);
                                emitters_changed = true;
                            }
                            w_rows.put((row_at * 16) as u64, bytemuck::bytes_of(&row));
                            if rows.len() <= row_at {
                                rows.resize(row_at + 1, [0; 4]);
                            }
                            rows[row_at] = row;
                            let rec = top.record(inst.gpu((edit.pool_first as u32) + im.first), mesh, inst.cast_shadow);
                            w_records.put(((r0 + i) * size) as u64, bytemuck::bytes_of(&rec));
                            gpu_instances[r0 + i] = rec;
                            top.place(inst_base + r0 + i, mesh, wm, gpu::INSTANCE_BIT | (r0 + i) as u32, inst.cast_shadow, hidden);
                            let (lo, hi) = im.placed_bounds(wm);
                            let entry = scene::copy_row(scene::object_row(wm.transform_point3(glam::Vec3::ZERO), o.power, hi - lo, t.custom(i, o.custom), t.color(i)), emits && hidden);
                            w_objects.put(((o0 + i) * st * 16) as u64, bytemuck::cast_slice(&entry[..st]));
                            tables.objects[(o0 + i) * st..(o0 + i + 1) * st].copy_from_slice(&entry[..st]);
                            instances[r0 + i] = inst;
                        }
                        *tables.retained.proxy(id) = scene::Proxy { object: None, card: None, draw: scene::Draw::Instanced(scene::InstancedPlace { first: r0 as u32, capacity: cap as u32, object: o0 as u32, mesh: Some(mesh), drawn: t.count }) };
                        continue;
                    }
                    // (Its rows, which say its materials, are placed with its record below; its bits and whether it casts
                    // are theirs.)
                    let mine = scene::section_rows(im.sections(sections), &mats, &tables.materials);
                    // (Shadow rays pass the glass among a caster's other sections under a kernel feature.)
                    if scene::rows_bits(&mine) & scene::INST_GLASS != 0 && scene::rows_cast(&mine, o.cast_shadow) && features & gpu::F_IGLASS == 0 {
                        more.set(more.get() | gpu::F_IGLASS);
                    }
                    let inst = scene::Instance { mesh, world: m, object: object as u32, cast_shadow: scene::rows_cast(&mine, o.cast_shadow), name: o.name.clone(), sub: None, hidden: unseen, masked: unseen, first_emitter: 0, rows: 0, bits: scene::rows_bits(&mine), prev: 0, verts: 0 };
                    let (lo, hi) = im.placed_bounds(m);
                    let entry = scene::object_row(m.transform_point3(glam::Vec3::ZERO), if unseen { 0.0 } else { o.power }, hi - lo, o.custom, [1.0; 3]);
                    (at.map(|(_, r)| r), Placed::Mesh(inst, mine), entry)
                }
                _ => return Err("not an object an edit places".into()),
            };
            // Its record (a posed rigid model's: a part each, in a run), TLAS instance and object entry: where it had
            // them, or new ones from the room.
            let n = match &blas {
                Placed::Parts(parts) => parts.len(),
                _ => 1,
            };
            let (slot, record) = match (at, record) {
                (Some((slot, _)), Some(r)) => (slot, r),
                _ => {
                    let r = match edit.records.take(n) {
                        Some(r) => r,
                        None if match &mut top { gpu::Top::Hw(accel) => grow.records(&mut edit.records, n, &mut up.heap, accel), gpu::Top::Sw(_) => false } => {
                            edit.grows += 1;
                            edit.rebind = true;
                            edit.records.take(n).ok_or("edit room: records")?
                        }
                        None => return Err(if matches!(top, gpu::Top::Sw(_)) { "edit room: records (software traversal's does not grow)" } else { "edit room: records (particles, or the device's limits)" }.into()),
                    };
                    (inst_base + r, r)
                }
            };
            // Above the triangles, and the record's value where the build's are: `instances[record]`, or `procs` at the
            // record less the first procedural one (hidden fillers at the records between, which are other objects').
            // (A mesh a body drives moves as an instance does, as the build's: `commit`.)
            let moves = o.driven || world.movable(id);
            let mut place = |record: usize, inst: &mut scene::Instance, mine: &mut [scene::Row], moves: bool| {
                let mut cx = Placing { edit: &mut *edit, grow: &grow, words: &mut up.words, tables: &mut *tables, instances: &mut *instances, gpu_instances: &mut *gpu_instances, meshes: &meshes[..], sections: &sections[..], rows: &mut *rows, pool_tris: &pool_tris[..], pool_place: &pool_place[..], w_rows: &mut w_rows, w_records: &mut w_records, new_emitters: &mut new_emitters, emitters_changed: &mut emitters_changed, top: &mut top, inst_base };
                place_instance(&mut cx, record, inst, mine, moves)
            };
            let draw = match blas {
                Placed::Proc(blas, p, gpu) => {
                    w_records.put((record * std::mem::size_of::<scene::GpuInstance>()) as u64, bytemuck::bytes_of(&gpu));
                    if record >= gpu_instances.len() {
                        gpu_instances.resize(record + 1, scene::GpuInstance::default());
                    }
                    gpu_instances[record] = gpu;
                    top.place_blas(slot, &blas, m, record as u32, mask(o.cast_shadow));
                    let j = record - proc_first;
                    if procs.len() <= j {
                        procs.resize(j + 1, scene::ProcObject { name: String::new(), materials: vec![], hidden: true, ..p.clone() });
                    }
                    procs[j] = p;
                    scene::Draw::Procedural(j as u32)
                }
                Placed::Mesh(mut inst, mut mine) => {
                    place(record, &mut inst, &mut mine, moves)?;
                    let mesh = if matches!(o.kind, Kind::Model { .. }) { scene::MeshKind::Model } else { scene::MeshKind::Own };
                    scene::Draw::Instance(scene::InstanceRef { instance: record as u32, mesh, moves })
                }
                // A posed skinned model: the instance of its own mesh; a move is its transform's.
                Placed::Skinned(mut inst, mut mine, g) => {
                    place(record, &mut inst, &mut mine, true)?;
                    tables.retained.skinned[g as usize].instance = record as u32;
                    scene::Draw::Instance(scene::InstanceRef { instance: record as u32, mesh: scene::MeshKind::Skinned(g), moves: true })
                }
                // A posed rigid model: a part a record, the nodes a pose may move with moving emitters; where it
                // stands as an update finds it (`repose_rigid` tells a move by that).
                Placed::Parts(parts) => {
                    let pose = match &o.kind {
                        Kind::Model { animation: Some(a), .. } => scene::rigid_pose_key(a),
                        _ => String::new(),
                    };
                    let mut rigid = scene::RigidModel { world: world.world_matrix(id), pose, ..Default::default() };
                    // (Its row of the object table: where a new pose or a move writes its pivot.)
                    rigid.rest.object = object as u32;
                    for (j, (node, mut inst, mut mine)) in parts.into_iter().enumerate() {
                        place(record + j, &mut inst, &mut mine, node != scene::STILL_NODES)?;
                        if node == scene::STILL_NODES {
                            rigid.still = Some((record + j) as u32);
                        } else {
                            rigid.instances.push((node, (record + j) as u32));
                        }
                    }
                    scene::Draw::Rigid(rigid)
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
        w_vert.flush(q, &up.vertices);
        w_index.flush_section(q, &up.words, Stream::Indices);
        w_leaf.flush(q, &up.bg_parts.leaves);
        w_colors.flush_section(q, &up.words, Stream::VertexColors);
        w_rows.flush_section(q, &up.words, Stream::Rows);
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
