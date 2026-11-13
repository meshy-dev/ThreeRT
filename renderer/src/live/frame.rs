//! One frame of the live view (`LiveCore::render`): the trace, the live passes, the radiance cache's and the media's
//! training, the raster visibility pass, and the GPU timing and profile.

use super::*;

/// GPU time per frame from timestamp queries (start of the first trace pass .. end of the post pass), read back a few
/// frames later without waiting, so measuring never drains the pipeline.
pub(super) struct FrameTimer {
    pub(super) queries: wgpu::QuerySet,
    pub(super) resolve: wgpu::Buffer,
    /// Per slot: readback buffer and its state (0 free, 1 mapping, 2 mapped).
    pub(super) reads: Vec<(wgpu::Buffer, Arc<AtomicU32>)>,
    /// Per slot: the frame it timed, and that frame's samples per pixel (its traces; only the first is timestamped).
    pub(super) frames: [u32; TIMER_SLOTS],
    pub(super) spps: [u32; TIMER_SLOTS],
    pub(super) slot: usize,
}

pub(super) const TIMER_SLOTS: usize = 4;

/// Gap (ms) past which a slot's post pass is not its trace's frame's (`frame_gpu_ms`).
pub(super) const TIMER_GAP_MS: f32 = 2.0;

/// A frame's GPU time (ms) from its slot's timestamps (trace, post, raster, particle pass pairs; zero: not run): from
/// its first pass (particles, raster or trace) to the end of its post pass. The post pass follows the trace's command
/// buffer within ~0.2 ms, after the frame's further samples (`spp` - 1 more traces, untimed, each about as long as
/// the first). In 2–3 % of the crowd scene's frames (Metal) a slot read the post pair of the frame that used the slot
/// next, 4 frames on: the span then read 70–80 ms while frames came every 16.7 ms. A post pass that begins more than
/// TIMER_GAP_MS past where those traces would end (twice their length) is not this frame's: the frame is counted as
/// its passes, the further traces at the first one's length.
pub(super) fn frame_gpu_ms(t: &[u64; 8], spp: u32, period_ns: f32) -> Option<f32> {
    let start = [t[6], t[4], t[0]].into_iter().filter(|&v| v > 0).min()?;
    if t[3] <= start {
        return None;
    }
    let ms = |ticks: u64| ticks as f32 * period_ns * 1e-6;
    let (trace, more) = (ms(t[1].saturating_sub(t[0])), spp.max(1) - 1);
    if t[2] >= t[1] && ms(t[2] - t[1]) > TIMER_GAP_MS + 2.0 * more as f32 * trace {
        return Some(ms(t[1] - start) + more as f32 * trace + ms(t[3].saturating_sub(t[2])));
    }
    Some(ms(t[3] - start))
}

impl FrameTimer {
    pub(super) fn new(d: &wgpu::Device) -> FrameTimer {
        let n = TIMER_SLOTS as u64;
        FrameTimer {
            // Per slot: trace pass begin / end, post pass begin / end, raster pass begin / end, particle pass begin / end
            // (Metal writes a pass's timestamps in pairs).
            queries: d.create_query_set(&wgpu::QuerySetDescriptor { label: Some("frame timer"), ty: wgpu::QueryType::Timestamp, count: (n * 8) as u32 }),
            resolve: d.create_buffer(&wgpu::BufferDescriptor { label: None, size: n * 4 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            reads: (0..n).map(|_| (d.create_buffer(&wgpu::BufferDescriptor { label: None, size: 64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }), Arc::default())).collect(),
            frames: [0; TIMER_SLOTS],
            spps: [1; TIMER_SLOTS],
            slot: 0,
        }
    }

    /// Latest finished measurement (frame ms, the frame it timed, its trace pass ms), and a free slot for this frame
    /// (None: all readbacks still pending, skip).
    pub(super) fn harvest(&mut self, period_ns: f32) -> (Option<(f32, u32, f32)>, Option<usize>) {
        let mut latest: Option<(f32, u32, f32)> = None;
        for (k, (buf, state)) in self.reads.iter().enumerate() {
            if state.load(Ordering::Acquire) == 2 {
                let t: [u64; 8] = bytemuck::pod_read_unaligned(&buf.slice(..).get_mapped_range().unwrap()[..64]);
                buf.unmap();
                state.store(0, Ordering::Release);
                let frame = self.frames[k];
                if let Some(ms) = frame_gpu_ms(&t, self.spps[k], period_ns).filter(|_| latest.is_none_or(|(_, f, _)| frame.wrapping_sub(f) as i32 > 0)) {
                    latest = Some((ms, frame, t[1].saturating_sub(t[0]) as f32 * period_ns * 1e-6));
                }
            }
        }
        let slot = (0..TIMER_SLOTS).map(|k| (self.slot + k) % TIMER_SLOTS).find(|&k| self.reads[k].1.load(Ordering::Acquire) == 0);
        if let Some(k) = slot {
            self.slot = (k + 1) % TIMER_SLOTS;
        }
        (latest, slot)
    }
}

/// Primary visibility by rasterization (the decision experiment of docs/ROADMAP.md § 1): every triangle drawn into a
/// triangle-index target (+ depth) at the frame's sub-pixel jitter; the kernel starts each live path at that triangle
/// instead of tracing the camera ray. `live --raster` / THREERT_RASTER=1.
pub(super) struct Raster {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) uniform: wgpu::Buffer,
    pub(super) vis: wgpu::TextureView,
    pub(super) depth: wgpu::TextureView,
}

pub(super) const RASTER_WGSL: &str = r#"
struct R { pos: vec4<f32>, fwd: vec4<f32>, right: vec4<f32>, up: vec4<f32>, jit: vec4<f32> }; // pos.w = tan(fov/2), fwd.w = aspect, jit = ndc offset xy, near
@group(0) @binding(0) var<uniform> r: R;
@group(0) @binding(1) var<storage, read> tri_verts: array<vec4<f32>>; // per triangle: a, b - a, c - a (kernel layout)
struct V { @builtin(position) p: vec4<f32>, @location(0) @interpolate(flat) id: u32 };
@vertex fn vs(@builtin(vertex_index) vi: u32) -> V {
    let ti = vi / 3u;
    let k = vi % 3u;
    var p = tri_verts[ti * 3u].xyz;
    if (k != 0u) { p += tri_verts[ti * 3u + k].xyz; }
    let d = p - r.pos.xyz;
    let z = dot(d, r.fwd.xyz);
    // Reversed infinite depth: near / z.
    return V(vec4<f32>(dot(d, r.right.xyz) / (r.pos.w * r.fwd.w) + r.jit.x * z, dot(d, r.up.xyz) / r.pos.w + r.jit.y * z, r.jit.z, z), ti + 1u);
}
@fragment fn fs(v: V) -> @location(0) u32 { return v.id; }
"#;

impl Raster {
    pub(super) fn new(g: &gpu::Gpu, w: u32, h: u32) -> Raster {
        let d = &g.device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("raster"), source: wgpu::ShaderSource::Wgsl(RASTER_WGSL.into()) });
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raster"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
            ],
        });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipeline = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("raster vis"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), targets: &[Some(wgpu::TextureFormat::R32Uint.into())], compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Greater), stencil: Default::default(), bias: Default::default() }),
            multisample: Default::default(),
            multiview_mask: None,
            cache: g.cache(),
        });
        g.save_pipeline_cache();
        let tex = |format, usage| {
            d.create_texture(&wgpu::TextureDescriptor { label: Some("raster"), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
                .create_view(&Default::default())
        };
        let vis = tex(wgpu::TextureFormat::R32Uint, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let depth = tex(wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT);
        let uniform = d.create_buffer(&wgpu::BufferDescriptor { label: Some("raster"), size: 80, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        Raster { pipeline, layout, uniform, vis, depth }
    }

    /// Records the visibility pass for `tris` triangles at internal size (iw, ih), sampling at jitter (jx, jy).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record(&self, g: &gpu::Gpu, e: &mut wgpu::CommandEncoder, verts: &wgpu::Buffer, tris: u32, cam: &Camera, aspect: f32, (iw, ih): (u32, u32), (jx, jy): (f32, f32), timestamps: Option<wgpu::RenderPassTimestampWrites>) {
        let u: [[f32; 4]; 5] = [
            [cam.pos.x, cam.pos.y, cam.pos.z, cam.tan_half_fov],
            [cam.fwd.x, cam.fwd.y, cam.fwd.z, aspect],
            [cam.right.x, cam.right.y, cam.right.z, 0.0],
            [cam.up.x, cam.up.y, cam.up.z, 0.0],
            // Pixel centres sample at +0.5; shift the image so they sample at the jittered position instead.
            [2.0 * (0.5 - jx) / iw as f32, -2.0 * (0.5 - jy) / ih as f32, 0.01, 0.0],
        ];
        g.queue.write_buffer(&self.uniform, 0, bytemuck::cast_slice(&u));
        let bg = g.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: self.uniform.as_entire_binding() }, wgpu::BindGroupEntry { binding: 1, resource: verts.as_entire_binding() }],
        });
        let mut pass = e.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("raster vis"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &self.vis, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(0.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
            timestamp_writes: timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(0.0, 0.0, iw as f32, ih as f32, 0.0, 1.0);
        pass.set_scissor_rect(0, 0, iw, ih);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bg, &[]);
        pass.draw(0..tris * 3, 0..1);
    }
}

/// The live kernel's world-space radiance cache (docs/ROADMAP.md § 1, step 1): its buffers and per-frame resolve.
pub(super) struct RadianceCache {
    pub(super) bufs: gpu::CacheBuffers,
    pub(super) resolve: wgpu::ComputePipeline,
}

/// Share of the pixels showing translucent particles whose nearest hit trains the volume voxels (up to
/// gpu::VOLUME_TRAIN_CAP points per frame).
pub(super) const VOLUME_TRAIN_SHARE: f32 = 1.0 / 8.0;
/// Media voxel training points per frame (pathtrace.wgsl MEDIA_TRAIN_POINTS).
pub(super) const MEDIA_TRAIN_POINTS: u32 = 4096;
pub(super) struct Profile {
    pub(super) queries: wgpu::QuerySet,
    pub(super) resolve: wgpu::Buffer,
    pub(super) read: wgpu::Buffer,
    pub(super) sums: Vec<f64>,
    pub(super) frames: u32,
    /// Timed apart from the slots above: the grid programs' two passes (`gridsim`, queries 0..4) and the media passes
    /// (`media`, queries 4..6: the camera rays' march and, at one sample a frame, the volume voxels' training, which
    /// are otherwise in the trace's pass). Their queries, the resolve and read buffers, the sums.
    pub(super) grid: (wgpu::QuerySet, wgpu::Buffer, wgpu::Buffer),
    pub(super) grid_sum: f64,
    pub(super) media_sum: f64,
}

impl LiveCore {
    /// Profile slots after the cache resolve that this scene writes: particle pass + BLAS rebuild (2; stateless or GPU
    /// particles), translucency + volume training (2 more).
    pub(super) fn profile_extra(&self) -> u32 {
        if self.sc.up.translucent.is_some() {
            4
        } else if self.sc.up.particles.is_some() || self.sc.up.gpu_particles.is_some() {
            2
        } else {
            0
        }
    }

    /// Light LOD: the emitter CDF weighted by distance to the camera (`emitter_lod`) and the emitter cull with its view
    /// cone (`EmitterCull`), rebuilt when the camera is 15 m from where it was last built or has turned LIGHT_LOD_TURN
    /// from that direction (the kernel's emitter pdfs read the same reference from the parameters).
    pub(super) fn update_light_lod(&mut self) {
        let d0 = LIGHT_LOD_D0;
        let (w, h) = (self.w, self.h);
        let b = &mut self.sc.base;
        // The light tree weighs emitters by their distance to each shading point itself: no light LOD, no cull.
        if d0 <= 0.0 || self.sc.tables.emitter_tris.is_empty() || b.ltree[3] > 0.5 {
            return;
        }
        let r = glam::Vec3::new(b.light_lod[0], b.light_lod[1], b.light_lod[2]);
        let dir = glam::Vec3::new(b.lod_view[0], b.lod_view[1], b.lod_view[2]);
        let turned = b.emit_power[2] > 0.0 && dir.dot(self.cam.fwd) < LIGHT_LOD_TURN.to_radians().cos();
        if b.light_lod[3] > 0.0 && r.distance(self.cam.pos) < 15.0 && !turned {
            return;
        }
        b.light_lod = [self.cam.pos.x, self.cam.pos.y, self.cam.pos.z, d0];
        b.emit_power[1] = EMITTER_CULL;
        b.emit_power[2] = EMITTER_CULL * EMITTER_CULL_VIEW;
        // The frustum's half-diagonal: tan = tan(fov / 2) x sqrt(1 + aspect^2).
        let half = (self.cam.tan_half_fov * (1.0 + (w as f32 / h as f32).powi(2)).sqrt()).atan() + EMITTER_CULL_MARGIN.to_radians();
        b.lod_view = [self.cam.fwd.x, self.cam.fwd.y, self.cam.fwd.z, half.min(std::f32::consts::PI).cos()];
        // Power edits held back for the emitter worker land first (its job in flight was built for the old reference:
        // dropped), so the CDF built here has the powers the objects hold and the reference the kernel reads.
        self.land_powers_now();
        let (e, b) = (&self.sc.tables, &mut self.sc.base);
        let (em, total, power) = crate::emitter_cdf(&e.emitter_tris, &e.materials, &e.objects, e.stride, b.light_lod, crate::EmitterCull::of(b));
        self.g.queue.write_buffer(&self.sc.up.emitters, 0, bytemuck::cast_slice(&em));
        b.misc[1] = total;
        b.emit_power[0] = power;
    }

    pub fn render(&mut self, spp: u32, extra: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::Buffer)) {
        // A dynamic sky's bake that finished since the last frame (time of day).
        self.take_sky();
        self.take_emitters();
        let (w, h) = (self.w, self.h);
        let (iw, ih) = self.internal_size();
        let now = Instant::now();
        // Lockstep (benchmarks) renders at a fixed frame time, whatever the wall clock says.
        self.frame_dt = self.fixed_dt.unwrap_or_else(|| now.duration_since(self.last_frame).as_secs_f32());
        self.last_frame = now;
        let moved = self.cam != self.prev_cam;
        self.update_light_lod();
        // A camera cut (a jump no camera makes in one frame: 2 m or 30 degrees) is not motion: no blur (UE: bCameraCut).
        let cut = (self.cam.pos - self.prev_cam.pos).length() > 2.0 || self.cam.fwd.dot(self.prev_cam.fwd) < 0.866;
        self.cold_frames = if self.reset || cut { 0 } else { self.cold_frames.saturating_add(1) };
        // (Objects moved this frame: re-posed triangles or moved instances, e.g. walkers in a shared pose.)
        let blur = !self.motion_blur_off && self.blur.0 > 0.0 && (moved || self.reposed || self.pose_reposed || self.inst_moved) && !cut;
        // Camera motion is reprojected exactly (disocclusions are rejected per pixel), so history survives motion;
        // the cap only bounds smearing of view-dependent shading (glossy reflections).
        // Diffuse lighting reprojects exactly with the camera; view-dependent specular is handled per pixel (temporal pass).
        // The most frames a pixel's history holds: HISTORY_LIVE while the scene's lighting changes, HISTORY_FRAMES once
        // it has settled. Within it the length is each pixel's own (live.wgsl `dtemporal`: less where it moves, anti-lag
        // where its lighting changes, restarted where its surface is new). No edit cuts it: until 2026-11-12 a
        // lighting edit left every pixel 16 frames and a geometry edit 4, the whole frame for a change in one corner.
        let cap = if self.reset { 1.0 } else if self.live_lighting() { HISTORY_LIVE } else { HISTORY_FRAMES };
        self.history_len = if self.reset { 1.0 } else { (self.history_len + 1.0).min(cap) };
        let cam = self.cam;
        let g = &self.g;
        // At most two frames in flight: beyond that the CPU waits, so latency stays bounded when GPU-bound.
        // Wait for the oldest frame only (waiting for all work would leave the GPU idle while this frame records).
        let t_wait = Instant::now();
        while self.inflight.len() >= 2 {
            let oldest = self.inflight.pop_front().unwrap();
            let _ = g.device.poll(wgpu::PollType::Wait { submission_index: Some(oldest), timeout: None });
        }
        let waited = t_wait.elapsed().as_secs_f32() * 1000.0;
        // The loop guard's count, read back since an earlier frame.
        let mut guard_free = false;
        if let Some((read, state)) = &self.guard_read {
            if state.load(Ordering::Acquire) == 2 {
                let n = u32::from_le_bytes(read.slice(..).get_mapped_range().unwrap()[..4].try_into().expect("four bytes"));
                read.unmap();
                state.store(0, Ordering::Release);
                if self.guard_trips != Some(n) {
                    eprintln!("live: the loop guard has stopped {n} of the kernel's invocations: frames with wrong pixels (a traversal that does not end on this scene's data)");
                }
                self.guard_trips = Some(n);
            }
            guard_free = state.load(Ordering::Acquire) == 0;
        }
        // GPU time drives adaptive resolution / samples per frame: timestamps when available (no stall), else twice a
        // second a frame timed in isolation (idle GPU -> submit -> wait).
        let period = g.queue.get_timestamp_period();
        let timed = match &mut self.timer {
            Some(t) => {
                let (latest, slot) = t.harvest(period);
                if let Some((ms, frame, trace)) = latest {
                    (self.gpu_ms, self.gpu_frame, self.gpu_trace_ms) = (ms, frame, trace);
                }
                if let Some(k) = slot {
                    t.frames[k] = self.frame;
                    t.spps[k] = spp.max(1);
                }
                slot
            }
            None => None,
        };
        let measure = self.timer.is_none() && self.frame % 30 == 0;
        if measure {
            let _ = g.device.poll(wgpu::PollType::wait_indefinitely());
        }
        // Nothing moved since the last in-place update: its span's previous-frame vertices become the current ones.
        if let (false, Some(span)) = (self.reposed, self.prev_span) {
            self.keep_prev(span);
            self.prev_span = None;
        }
        self.reposed = false;
        if let (false, Some(span)) = (self.pose_reposed, self.pose_span) {
            self.keep_prev(span);
            self.pose_span = None;
        }
        self.pose_reposed = false;
        // Likewise the instances the last update moved: their previous-frame rows become the current ones.
        if !self.inst_moved && !self.inst_moving.is_empty() {
            let size = std::mem::size_of::<scene::GpuInstance>() as u64;
            for i in std::mem::take(&mut self.inst_moving) {
                let r = &mut self.sc.gpu_instances[i as usize];
                r.p = r.m;
                self.g.queue.write_buffer(&self.sc.up.instances, i as u64 * size, bytemuck::bytes_of(r));
            }
        }
        self.inst_moved = false;
        let g = &self.g;
        let t_submit = Instant::now();
        let pipeline = g.pipeline(&self.sc.up.kernel, true);
        // (No media pass while every medium is hidden: a pool of idle fireballs costs the frame nothing.)
        let media = self.sc.up.kernel.features & gpu::F_MEDIA != 0 && self.sc.up.media_hidden.borrow().iter().any(|h| !h);
        let media_train = (media && self.cache.is_some()).then(|| g.media_pipeline(&self.sc.up.kernel, "media_train"));
        let media_composite = media.then(|| g.media_pipeline(&self.sc.up.kernel, "media_composite"));
        // This frame's sub-pixel jitter (Halton 2, 3), every camera ray's and the raster visibility pass's: 8 phases at
        // full resolution, as many more as an output pixel has fewer traced ones (8 / scale^2: the upscaler needs each
        // output pixel visited), in the 12 bits a side the samples carry it in.
        let phases = ((8.0 * (w as f32 / iw as f32) * (h as f32 / ih as f32)).round() as u32).clamp(8, 512);
        let quant = |v: f32| (v * 4095.0).round() / 4095.0;
        let jitter = (quant(halton(self.frame % phases + 1, 2)), quant(halton(self.frame % phases + 1, 3)));
        for k in 0..spp.max(1) {
            let mut p = self.sc.base;
            // Software traversal's units as edits left them: the top tree's root and how many it holds (trace_units.wgsl).
            if let Some(u) = &self.sc.up.units {
                (p.motion[2], p.motion[3]) = (f32::from_bits(u.layout.top_first), f32::from_bits(u.count));
            }
            p.raster = [(self.raster.is_some() && !self.sc.instanced) as u32 as f32, jitter.0, jitter.1, 0.0];
            p.raster[3] = if self.rays { 0.0 } else { 1.0 };
            if !self.sc.dynamic.1.is_empty() || !self.sc.tables.retained.instance_of.is_empty() || !self.sc.tables.retained.rigid.is_empty() {
                // The kernel finds the first dynamic triangle in Params::group_first (group 4), a u32.
                p.motion[0] = 1.0;
            }
            if self.cache.is_some() {
                // Light-sampled particles (sparks: Particles emitterArea) move their light every frame: the cache and
                // the temporal pass keep the short history of a lighting edit while the scene has them.
                let lit = self.live_lighting();
                let cold = if self.cold_frames >= COLD_FALLBACK_FRAMES { 1 } else { 0 };
                p.cache = [1.0, CACHE_SAMPLES, if lit { CACHE_FRAMES_LIT } else { CACHE_FRAMES }, (cache_flags() | cold) as f32];
            }
            p.time[0] = shader_time(self.time);
            p.cam_pos = [cam.pos.x, cam.pos.y, cam.pos.z, cam.tan_half_fov];
            p.cam_fwd = [cam.fwd.x, cam.fwd.y, cam.fwd.z, self.aspect];
            p.cam_right = [cam.right.x, cam.right.y, cam.right.z, 0.0];
            p.cam_up = [cam.up.x, cam.up.y, cam.up.z, 1.0];
            p.size = [iw, ih, self.frame.wrapping_mul(16).wrapping_add(k), crate::rng_seed()];
            p.counts2[2] = if ih < h { h } else { 0 };
            p.misc[3] = k as f32;
            p.counts[2] = p.counts[2].min(self.bounces);
            p.counts[3] = p.counts[3].min(2);
            g.queue.write_buffer(&self.sc.up.params, 0, bytemuck::bytes_of(&p));
            let mut e = g.device.create_command_encoder(&Default::default());
            // Particles: placed for this frame's time and camera, then the dynamic BLASes rebuilt (before the raster and
            // the trace).
            // The grid programs (density grids changed on the GPU), before anything reads the grids.
            if k == 0 && gpu::Gpu::has_grid_sim(&self.sc.up) {
                let sources = std::mem::take(&mut self.grid_sources);
                g.run_grids(&self.sc.up, shader_time(self.time), self.frame_dt, &sources, &mut e, self.profile.as_ref().map(|p| (&p.grid.0, 0)));
                if let Some(p) = &self.profile {
                    e.resolve_query_set(&p.grid.0, 0..4, &p.grid.1, 0);
                    e.copy_buffer_to_buffer(&p.grid.1, 0, &p.grid.2, 0, 32);
                }
            }
            // Stateful GPU particles first (their emissions, the game's spawns): with stateless ones the pass below
            // rebuilds the dynamic BLASes for both, else this one does.
            if let (Some(_), 0) = (&self.sc.up.gpu_particles, k) {
                let n = self.passes.pipes.len() as u32 + 2;
                let stateless = self.sc.up.particles.is_some();
                let (pass_ts, blas_ts) = match (&self.profile, &self.timer, timed, stateless) {
                    (Some(p), _, _, false) => (Some((&p.queries, n * 2)), Some((&p.queries, n * 2 + 2))),
                    (None, Some(t), Some(slot), false) => (Some((&t.queries, slot as u32 * 8 + 6)), None),
                    _ => (None, None),
                };
                let spawns = std::mem::take(&mut self.particle_spawns);
                g.run_gpu_particles(&self.sc.up, cam.pos.to_array(), cam.fwd.to_array(), shader_time(self.time), self.frame_dt, &spawns, !stateless, &mut e, pass_ts, blas_ts);
            }
            if let (Some(_), 0) = (&self.sc.up.particles, k) {
                let b = &self.sc.base;
                let n = self.passes.pipes.len() as u32 + 2;
                let pass_ts = match (&self.profile, &self.timer, timed) {
                    (Some(p), _, _) => Some((&p.queries, n * 2)),
                    (None, Some(t), Some(slot)) => Some((&t.queries, slot as u32 * 8 + 6)),
                    _ => None,
                };
                let blas_ts = self.profile.as_ref().map(|p| (&p.queries, n * 2 + 2));
                g.run_particles(&self.sc.up, cam.pos.to_array(), cam.fwd.to_array(), shader_time(self.time), self.frame_dt, b.time[2], b.time[1], &mut e, pass_ts, blas_ts);
            }
            // (The merged layout and the uniform tables: the tables as this frame's edits and the particle passes above
            // left them, where the kernel reads them; merged.rs.)
            if k == 0 {
                g.sync_tables(&self.sc.up, &mut e);
            }
            if let (Some(r), 0, false) = (&self.raster, k, self.sc.instanced) {
                let tw = self.timer.as_ref().zip(timed).map(|(t, slot)| wgpu::RenderPassTimestampWrites { query_set: &t.queries, beginning_of_pass_write_index: Some(slot as u32 * 8 + 4), end_of_pass_write_index: Some(slot as u32 * 8 + 5) });
                r.record(g, &mut e, &self.sc.up.verts, self.sc.tris.len() as u32, &cam, self.aspect, (iw, ih), jitter, tw);
            }
            {
                let tw = match (&self.profile, &self.timer, timed) {
                    (Some(p), _, _) if k == 0 => Some(wgpu::ComputePassTimestampWrites { query_set: &p.queries, beginning_of_pass_write_index: Some(0), end_of_pass_write_index: Some(1) }),
                    (None, Some(t), Some(slot)) if k == 0 => Some(wgpu::ComputePassTimestampWrites { query_set: &t.queries, beginning_of_pass_write_index: Some(slot as u32 * 8), end_of_pass_write_index: Some(slot as u32 * 8 + 1) }),
                    _ => None,
                };
                let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("live trace"), timestamp_writes: tw });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &self.sc.up.bg, &[]);
                pass.set_bind_group(1, &self.kernel_bg, &[]);
                pass.dispatch_workgroups(iw.div_ceil(8), ih.div_ceil(8), 1);
                // Participating media: their volume voxels trained after the trace (pathtrace.wgsl media_train), and
                // camera rays through media once the frame's samples are traced (media_composite). In the trace's pass
                // and timing; profiled, the frame's last sample's media passes are a pass of their own (`media`).
                let last = k + 1 == spp.max(1);
                if let (Some(p), true, true) = (&self.profile, last, media_composite.is_some()) {
                    drop(pass);
                    pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("media"), timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: &p.grid.0, beginning_of_pass_write_index: Some(4), end_of_pass_write_index: Some(5) }) });
                    pass.set_bind_group(0, &self.sc.up.bg, &[]);
                    pass.set_bind_group(1, &self.kernel_bg, &[]);
                }
                if let (Some(mt), 0) = (&media_train, k) {
                    pass.set_pipeline(mt);
                    pass.dispatch_workgroups(MEDIA_TRAIN_POINTS.div_ceil(64), 1, 1);
                }
                if let (Some(mc), true) = (&media_composite, last) {
                    pass.set_pipeline(mc);
                    pass.dispatch_workgroups(iw.div_ceil(8), ih.div_ceil(8), 1);
                }
            }
            if let (Some(p), true, true) = (&self.profile, k + 1 == spp.max(1), media_composite.is_some()) {
                e.resolve_query_set(&p.grid.0, 4..6, &p.grid.1, wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT);
                e.copy_buffer_to_buffer(&p.grid.1, wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT, &p.grid.2, 32, 16);
            }
            // Each pair is resolved in the command buffer that wrote it (Metal reads zeros otherwise).
            if let (None, Some(t), Some(slot), 0) = (&self.profile, &self.timer, timed, k) {
                let q = slot as u32 * 8;
                let at = slot as u64 * 4 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
                e.resolve_query_set(&t.queries, q..q + 2, &t.resolve, at);
                e.copy_buffer_to_buffer(&t.resolve, at, &t.reads[slot].0, 0, 16);
                // Raster and particle pairs (pairs 2 and 3); zeros when the pass did not run.
                for (pair, ran) in [(2u32, self.raster.is_some() && !self.sc.instanced), (3, self.sc.up.particles.is_some() || self.sc.up.gpu_particles.is_some())] {
                    let at = at + pair as u64 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
                    if ran {
                        e.resolve_query_set(&t.queries, q + pair * 2..q + pair * 2 + 2, &t.resolve, at);
                    } else {
                        e.clear_buffer(&t.resolve, at, Some(16));
                    }
                    e.copy_buffer_to_buffer(&t.resolve, at, &t.reads[slot].0, pair as u64 * 16, 16);
                }
            }
            g.queue.submit([e.finish()]);
        }
        let (r, t) = self.bloom;
        let lp = LiveParams {
            cur: cam.uniform(self.aspect),
            prev: self.prev_cam.uniform(self.aspect),
            size: [w, h, self.frame, self.reset as u32],
            misc: [self.sc.exposure, cap, self.vignette, (self.bloom.1.iter().sum::<f32>() > 0.0) as u32 as f32],
            // misc2.z: motion blur, this frame's motion (pixels) -> blur length. UE (PostProcessMotionBlur.cpp,
            // UpdateMotionBlurTimeScale) blurs over +- amount / 2 x the motion x (1 / target FPS) / frame time, the
            // actual frame time without a target. 0 while nothing moves (the pass then costs nothing).
            misc2: [1.0, SAMPLE_CEILING, if !blur { 0.0 } else if self.blur.1 > 0.0 { self.blur.0 / (self.blur.1 * self.frame_dt.clamp(1.0 / 240.0, 0.25)) } else { self.blur.0 }, spp.max(1) as f32],
            mode: [self.view_mode, 0, self.live_lighting() as u32, 0],
            post: self.post,
            ae: self.sc.exposure_settings.ae,
            ae2: self.sc.exposure_settings.ae2,
            ae3: [self.sc.exposure_settings.speed[0], self.sc.exposure_settings.speed[1], self.frame_dt.clamp(1.0 / 240.0, 0.25), (self.reset || self.frame == 0 || exposure_snap()) as u32 as f32],
            lx: self.sc.exposure_settings.lx,
            isz: [iw, ih, self.prev_isz.0, self.prev_isz.1],
            br: [r[0], r[1], r[2], r[3]],
            br2: [r[4], r[5], 0.0, 0.0],
            bt: [t[0], t[1], t[2], t[3]],
            bt2: [t[4], t[5], 0.0, 0.0],
            dof: dof_params(self.sc.dof.as_ref(), cam.tan_half_fov, w, h),
            jit: [jitter.0, jitter.1, 0.0, 0.0],
            state: [1 - self.frame % 2, 0, 0, 0],
        };
        g.queue.write_buffer(&self.targets.params, 0, bytemuck::bytes_of(&lp));
        let bg = &self.sc.bg;
        // Translucent particles: drawn over the upscaled image right after `upscale` (in sb when depth of field / motion
        // blur follow, else fa: they have no motion vectors, so they are no part of the colour history; UE draws its
        // translucency after the upscaler the same way), then their volume voxels trained.
        // (Nothing to draw while every translucent particle system is hidden in place.)
        let tl = self.sc.up.translucent.as_ref().filter(|_| g.translucent_particles_live(&self.sc.up)).map(|t| {
            let pipes = g.translucent_pipelines(&self.sc.up.kernel, true);
            let target = if lp.misc2[2] > 0.0 || lp.dof[0] > 0.0 { &self.targets.sb.buffer } else { &self.targets.fa.buffer };
            // Below full resolution the particles are drawn per traced pixel, then laid over the output (`translucent_up`).
            let low = (iw, ih) != (w, h);
            let tp = gpu::TranslucentParams { size: [w, h, t.first, 1], view: [iw, ih, gpu::VOLUME_TRAIN_CAP, low as u32], misc: [VOLUME_TRAIN_SHARE, translucent_hidden(self.frame) as u32 as f32, 0.0, 0.0] };
            g.queue.write_buffer(&t.params, 0, bytemuck::bytes_of(&tp));
            (pipes, g.translucent_bind_group(t, target), &t.count)
        });
        // The pass (train = false) or the volume training (train = true).
        let tl_dispatch = |pass: &mut wgpu::ComputePass, train: bool| {
            if let Some((pipes, tbg, _)) = &tl {
                pass.set_bind_group(0, &self.sc.up.bg, &[]);
                pass.set_bind_group(1, &self.kernel_bg, &[]);
                pass.set_bind_group(2, tbg, &[]);
                if train {
                    pass.set_pipeline(&pipes.train);
                    pass.dispatch_workgroups(gpu::VOLUME_TRAIN_CAP / 64, 1, 1);
                } else if (iw, ih) != (w, h) {
                    pass.set_pipeline(&pipes.draw);
                    pass.dispatch_workgroups(iw.div_ceil(8), ih.div_ceil(8), 1);
                    pass.set_pipeline(&pipes.up);
                    pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
                } else {
                    pass.set_pipeline(&pipes.draw);
                    pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
                }
            }
        };
        let mut e = g.device.create_command_encoder(&Default::default());
        if let Some((_, _, count)) = &tl {
            e.clear_buffer(count, 0, None);
        }
        if let Some(c) = &self.cache {
            let n = self.passes.pipes.len() as u32 + 1;
            let tw = self.profile.as_ref().map(|p| wgpu::ComputePassTimestampWrites { query_set: &p.queries, beginning_of_pass_write_index: Some(n * 2), end_of_pass_write_index: Some(n * 2 + 1) });
            let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("cache resolve"), timestamp_writes: tw });
            pass.set_pipeline(&c.resolve);
            pass.set_bind_group(0, &self.sc.up.bg, &[]);
            pass.set_bind_group(1, &self.kernel_bg, &[]);
            pass.dispatch_workgroups((gpu::CACHE_SIZE / 256) as u32, 1, 1);
        }
        if let Some(p) = &self.profile {
            // One compute pass per stage, each timed.
            for (k, (name, pipe)) in self.passes.pipes.iter().enumerate() {
                let q = (k as u32 + 1) * 2;
                let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(name), timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: &p.queries, beginning_of_pass_write_index: Some(q), end_of_pass_write_index: Some(q + 1) }) });
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, bg, &[]);
                let (gw, gh) = pass_grid(name, (w, h), (iw, ih), self.prev_isz);
                pass.dispatch_workgroups(gw.div_ceil(8), gh.div_ceil(8), 1);
                drop(pass);
                if *name == "upscale" && tl.is_some() {
                    for (j, train) in [(0u32, false), (1, true)] {
                        let q = (self.passes.pipes.len() as u32 + 4 + j) * 2;
                        let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some(if train { "vtrain" } else { "translucent" }), timestamp_writes: Some(wgpu::ComputePassTimestampWrites { query_set: &p.queries, beginning_of_pass_write_index: Some(q), end_of_pass_write_index: Some(q + 1) }) });
                        tl_dispatch(&mut pass, train);
                    }
                }
            }
            let n = self.passes.pipes.len() as u32 + 2 + self.profile_extra();
            e.resolve_query_set(&p.queries, 0..n * 2, &p.resolve, 0);
            e.copy_buffer_to_buffer(&p.resolve, 0, &p.read, 0, p.read.size());
        } else {
            let tw = self.timer.as_ref().zip(timed).map(|(t, slot)| wgpu::ComputePassTimestampWrites { query_set: &t.queries, beginning_of_pass_write_index: Some(slot as u32 * 8 + 2), end_of_pass_write_index: Some(slot as u32 * 8 + 3) });
            let mut pass = e.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("live post"), timestamp_writes: tw });
            for (name, pipe) in &self.passes.pipes {
                pass.set_pipeline(pipe);
                pass.set_bind_group(0, bg, &[]);
                let (gw, gh) = pass_grid(name, (w, h), (iw, ih), self.prev_isz);
                pass.dispatch_workgroups(gw.div_ceil(8), gh.div_ceil(8), 1);
                if *name == "upscale" {
                    tl_dispatch(&mut pass, false);
                    tl_dispatch(&mut pass, true);
                }
            }
            drop(pass);
            if let (Some(t), Some(slot)) = (&self.timer, timed) {
                let q = slot as u32 * 8 + 2;
                let at = (slot as u64 * 4 + 1) * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
                e.resolve_query_set(&t.queries, q..q + 2, &t.resolve, at);
                e.copy_buffer_to_buffer(&t.resolve, at, &t.reads[slot].0, 16, 16);
            }
        }
        if let (Some((read, _)), Some(count), true) = (&self.guard_read, &self.targets.kernel.guard, guard_free) {
            e.copy_buffer_to_buffer(&count.buffer, count.offset, read, 0, 4);
        }
        extra(&mut e, &self.targets.out);
        let done = g.queue.submit([e.finish()]);
        self.inflight.push_back(done);
        self.cpu_wait_ms = waited;
        if let (Some((read, state)), true) = (&self.guard_read, guard_free) {
            let state = state.clone();
            state.store(1, Ordering::Release);
            read.slice(..).map_async(wgpu::MapMode::Read, move |r| state.store(if r.is_ok() { 2 } else { 0 }, Ordering::Release));
        }
        if let (Some(t), Some(slot)) = (&self.timer, timed) {
            let state = t.reads[slot].1.clone();
            state.store(1, Ordering::Release);
            t.reads[slot].0.slice(..).map_async(wgpu::MapMode::Read, move |r| state.store(if r.is_ok() { 2 } else { 0 }, Ordering::Release));
        }
        // Without timestamps: GPU time ~ submit -> completion (the queue is idle at submit when the frame fits the budget).
        if measure {
            let _ = g.device.poll(wgpu::PollType::wait_indefinitely());
            (self.gpu_ms, self.gpu_frame) = (t_submit.elapsed().as_secs_f32() * 1000.0, self.frame);
        }
        let extra = self.profile_extra() as usize;
        if let Some(p) = &mut self.profile {
            let _ = g.device.poll(wgpu::PollType::wait_indefinitely());
            p.read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            let _ = g.device.poll(wgpu::PollType::wait_indefinitely());
            let t: Vec<u64> = bytemuck::cast_slice::<u8, u64>(&p.read.slice(..).get_mapped_range().unwrap()).to_vec();
            p.read.unmap();
            let period = g.queue.get_timestamp_period() as f64 * 1e-6;
            // The particle, BLAS, translucency and volume training slots (the last four) are written only when the scene
            // has particles / translucent ones.
            let n = self.passes.pipes.len() + 2 + extra;
            for (k, s) in p.sums.iter_mut().enumerate().take(n) {
                *s += t[k * 2 + 1].saturating_sub(t[k * 2]) as f64 * period;
            }
            let (sim, media) = (gpu::Gpu::has_grid_sim(&self.sc.up), media_composite.is_some());
            if sim || media {
                p.grid.2.slice(..).map_async(wgpu::MapMode::Read, |_| {});
                let _ = g.device.poll(wgpu::PollType::wait_indefinitely());
                let t: Vec<u64> = bytemuck::cast_slice::<u8, u64>(&p.grid.2.slice(..).get_mapped_range().unwrap()).to_vec();
                p.grid.2.unmap();
                if sim {
                    p.grid_sum += (t[1].saturating_sub(t[0]) + t[3].saturating_sub(t[2])) as f64 * period;
                }
                if media {
                    p.media_sum += t[5].saturating_sub(t[4]) as f64 * period;
                }
            }
            p.frames += 1;
            if p.frames == 120 {
                let names: Vec<&str> = std::iter::once("trace").chain(self.passes.pipes.iter().map(|(n, _)| *n)).chain(["cache", "particles", "blas", "translucent", "vtrain"]).collect();
                let mut rows: Vec<String> = names.iter().zip(&p.sums).map(|(n, s)| format!("{n} {:.2}", s / p.frames as f64)).collect();
                rows.push(format!("gridsim {:.2}", p.grid_sum / p.frames as f64));
                rows.push(format!("media {:.2}", p.media_sum / p.frames as f64));
                rows.retain(|r| !r.ends_with(" 0.00"));
                eprintln!("profile ms/frame: total {:.2} | {}", (p.sums.iter().sum::<f64>() + p.grid_sum + p.media_sum) / p.frames as f64, rows.join(", "));
                (p.grid_sum, p.media_sum) = (0.0, 0.0);
                p.sums.iter_mut().for_each(|s| *s = 0.0);
                p.frames = 0;
            }
        }
        self.prev_cam = cam;
        self.shown_spp = spp.max(1);
        self.prev_isz = (iw, ih);
        self.reset = false;
        self.frame = self.frame.wrapping_add(1);
    }

}
