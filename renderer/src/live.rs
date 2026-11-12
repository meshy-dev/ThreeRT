//! Real-time renderer (LiveCore), modelled on UE5's real-time path with hardware RT: every frame traces 1+ spp
//! (primary visibility, GI, shadows, reflections), reprojects and accumulates history (converging to the offline
//! reference while the camera holds still), denoises and applies the UE post chain. Drawn by the native window viewer
//! (`view`) and the playground client; `live` runs it headless at a fixed frame rate for benchmarks:
//!
//!   threert-render live <game bundle.js> [--assets DIR] [--room id] [--width 1280 --height 720 --fps 60 --bounces 4
//!                       --scale 1|0.6|auto --display-scale 1|2|3] [--frame-log frames.jsonl]
//!
//! `--scale` is the render scale (UE screen percentage): the frame is traced, denoised and composed at that fraction
//! of the output resolution and the temporal upscaler takes it to the output; `auto` = dynamic resolution.
//! `--display-scale`: the output is a display's physical pixels, that many a point (a Retina window: 2), as the
//! window host's is; dynamic resolution then starts from the points' resolution and goes down to half of it.
//!
//! stdin (JSON lines): {"cmd":"camera","pos":[x,y,z],"target":[x,y,z]} | {"cmd":"event","type":t,"payload":{..}}
//!                     | {"cmd":"click","u":u,"v":v} | {"cmd":"reset"} | {"cmd":"time","t":s} (set the scene time)
//!                     | {"cmd":"snapshot","path":p,"after":k (history length ≥ k),"frames":n (the n-th frame from now, default 1)}
//!                       (a path ending in .raw: the linear radiance, RGB f32 at output size; ending in .signals:
//!                       its diffuse / specular / emission apart, <stem>-{diffuse,specular,emission}.raw, as
//!                       `render --signals` writes them)
//!                     | {"cmd":"sync","id":s,"frames":n} (stderr "live: sync s" after the n-th frame from now)
//!                     | {"cmd":"lockstep","dt":s | "on":false} (render only the frames steps ask for, at frame time s)
//!                     | {"cmd":"camera",...,"frames":n,"sync":id | "snapshot":p} | {"cmd":"frames","n":n,"sync":id | "snapshot":p}
//!                       (a step: camera, frames, and the sync answer / snapshot at its last frame, in one command)
//!                     | {"cmd":"scale","scale":s (0 = auto)} | {"cmd":"quit"}

use crate::gpu::Buf;
use crate::world::World;
use crate::{gpu, post, scene, tables};
// (The headless runner below, native only, is what uses all of it.)
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use anyhow::{anyhow, Result};
use glam::Vec3;
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
#[cfg_attr(target_arch = "wasm32", allow(unused_imports))]
use std::time::Duration;
use web_time::Instant;

pub struct LiveOpts {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bounces: u32,
    /// Render scale (traced / output resolution); None = dynamic resolution.
    pub scale: Option<f32>,
    /// Game bundle (game code + runtime) that builds and drives the scene in-process.
    pub bundle: PathBuf,
    /// Asset root its model / texture paths resolve against.
    pub assets: PathBuf,
    pub room: String,
    /// JSON line per frame (wall interval, CPU work, GPU time, scale, history): hitch statistics (tools/bench-hitches.ts).
    /// `interval` runs from the end of one frame's work to the next's, so it carries the variation of the work itself;
    /// `start` runs from one frame's start to the next's, which the pacer sets: the frame rate a player sees.
    pub frame_log: Option<PathBuf>,
    /// The output's pixels a point of its display (1: the output is in points).
    pub display_scale: f32,
}

/// Lowest dynamic-resolution scale (UE r.DynamicRes.MinScreenPercentage default) of a view whose pixels are the
/// display's points; a host whose view is in the display's physical pixels divides it by the display's scale factor
/// (`LiveCore::min_scale`), so the fewest rays a window traces do not depend on the display's density.
pub const MIN_SCALE: f32 = 0.5;
/// The lowest render scale any view takes (a view of 3 pixels a point at `MIN_SCALE`, rounded down).
pub const SCALE_FLOOR: f32 = 0.125;

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct CamU {
    pos: [f32; 4],
    fwd: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct LiveParams {
    cur: CamU,
    prev: CamU,
    size: [u32; 4],
    misc: [f32; 4],
    misc2: [f32; 4],
    mode: [u32; 4],
    post: [f32; 4],
    /// Auto exposure (UE histogram): low / high fraction, histogram log2 min / max.
    ae: [f32; 4],
    /// Compensation (2^bias), min / max average luminance, mode (0 manual, 1 auto).
    ae2: [f32; 4],
    /// Speed up / down (EV/s), frame time (s), snap to target (1).
    ae3: [f32; 4],
    /// Local exposure: highlight contrast, shadow contrast, blurred-luminance blend, detail strength.
    lx: [f32; 4],
    /// Traced (internal) width, height.
    isz: [u32; 4],
    /// Bloom stage radii (texels of each stage's level) and tints, stages 0..3 then 4, 5.
    br: [f32; 4],
    br2: [f32; 4],
    bt: [f32; 4],
    bt2: [f32; 4],
    /// Depth of field: CoC radius of infinity (output px; 0 = off), focus distance (m), max radius (px), -.
    dof: [f32; 4],
    /// This frame's sub-pixel jitter (every camera ray's: x, y in 0 .. 1 of a traced pixel), -, -.
    jit: [f32; 4],
}

/// Bloom pyramid level j (0..4: 1/4 .. 1/64 resolution) size; stage k (0..5) runs on level max(k - 1, 0).
fn bloom_dim(w: u32, h: u32, j: u32) -> (u32, u32) {
    (w.div_ceil(4).div_ceil(1 << j), h.div_ceil(4).div_ceil(1 << j))
}

fn bloom_stage_level(k: u32) -> u32 {
    k.max(1) - 1
}

/// UE Standard bloom per stage: Gaussian radius in texels of its level (BloomNSize % of the stage width * size scale
/// / 2, as `post::bloom`) and tint (BloomNTint * intensity / 6).
fn bloom_stages(pp: &crate::desc::PostPreset, w: u32, h: u32) -> ([f32; 6], [f32; 6]) {
    let (mut r, mut t) = ([0f32; 6], [0f32; 6]);
    for k in 0..6 {
        let (lw, _) = bloom_dim(w, h, bloom_stage_level(k as u32));
        r[k] = lw as f32 * pp.bloom.sizes.get(k).copied().unwrap_or(0.0) * pp.bloom.size_scale * 0.01 * 0.5;
        t[k] = pp.bloom.tints.get(k).copied().unwrap_or(0.0) * pp.bloom.intensity / 6.0;
    }
    (r, t)
}

/// Auto / local exposure settings of the loaded scene (UE PostProcessVolume semantics).
#[derive(Clone, Copy, Default)]
struct ExposureSettings {
    ae: [f32; 4],
    ae2: [f32; 4],
    speed: [f32; 2],
    lx: [f32; 4],
}

impl ExposureSettings {
    fn new(exp: &crate::desc::Exposure, pp: &crate::desc::PostPreset) -> Self {
        let a = &pp.auto_exposure;
        let le = &pp.local_exposure;
        let (ae2, comp) = match exp {
            crate::desc::Exposure::Manual { ev100 } => ([2f32.powf(-ev100), 0.0, 0.0, 0.0], 2f32.powf(-ev100)),
            crate::desc::Exposure::Auto { compensation, min_ev100, max_ev100 } => {
                let c = 2f32.powf(*compensation);
                ([c, 0.18 * 2f32.powf(*min_ev100), 0.18 * 2f32.powf(*max_ev100), 1.0], c)
            }
        };
        let _ = comp;
        ExposureSettings {
            ae: [a.low_percent.clamp(1.0, 99.0) / 100.0, a.high_percent.clamp(1.0, 99.0) / 100.0, a.histogram_log_min, a.histogram_log_max.max(a.histogram_log_min + 1.0)],
            ae2,
            speed: [a.speed_up.max(0.01), a.speed_down.max(0.01)],
            lx: [le.highlight_contrast, le.shadow_contrast, le.blurred_luminance_blend, le.detail_strength],
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub struct Camera {
    pub pos: Vec3,
    pub fwd: Vec3,
    pub up: Vec3,
    pub right: Vec3,
    pub tan_half_fov: f32,
}

impl Camera {
    pub fn look_at(pos: Vec3, target: Vec3, tan_half_fov: f32) -> Camera {
        let fwd = (target - pos).normalize();
        let right = fwd.cross(Vec3::Y).normalize();
        Camera { pos, fwd, up: right.cross(fwd), right, tan_half_fov }
    }
    fn uniform(&self, aspect: f32) -> CamU {
        CamU {
            pos: [self.pos.x, self.pos.y, self.pos.z, self.tan_half_fov],
            fwd: [self.fwd.x, self.fwd.y, self.fwd.z, aspect],
            right: [self.right.x, self.right.y, self.right.z, 0.0],
            up: [self.up.x, self.up.y, self.up.z, 0.0],
        }
    }
}

const LOG_OFFSET: f32 = 0.0026883; // must match live.wgsl lin_to_log input offset
const LUT: u32 = 32;

/// UE-style 32^3 LUT: log-encoded linear input -> display sRGB, built from the exact CPU port of the filmic tonemapper.
fn tonemap_lut(p: &crate::desc::PostPreset) -> Vec<u16> {
    let tm = post::Tonemapper::new(p);
    let mut out = Vec::with_capacity((LUT * LUT * LUT * 4) as usize);
    let log_to_lin = |v: f32| (2f32.powf((v - 444.0 / 1023.0) * 14.0) * 0.18 - LOG_OFFSET).max(0.0);
    for b in 0..LUT {
        for g in 0..LUT {
            for r in 0..LUT {
                let enc = |k: u32| log_to_lin(k as f32 / (LUT - 1) as f32);
                let d = tm.apply(Vec3::new(enc(r), enc(g), enc(b)));
                for c in [d.x, d.y, d.z, 1.0] {
                    out.push(half::f16::from_f32(post::linear_to_srgb(c.clamp(0.0, 1.0))).to_bits());
                }
            }
        }
    }
    out
}

/// The live passes' source: live.wgsl as it is, or in the merged layout its buffers merged into slabs (merged.rs).
pub(crate) fn live_source(merged: bool) -> std::borrow::Cow<'static, str> {
    let src = include_str!("../shaders/live.wgsl");
    if merged {
        crate::merged::live(src).into()
    } else {
        src.into()
    }
}

struct Passes {
    layout: wgpu::BindGroupLayout,
    pipes: Vec<(&'static str, wgpu::ComputePipeline)>,
}

impl Passes {
    fn new(g: &gpu::Gpu) -> Passes {
        let d = &g.device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("live"), source: wgpu::ShaderSource::Wgsl(live_source(g.merged).into()) });
        let buf = |binding: u32, read_only: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let mut entries = vec![wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }];
        entries.extend([buf(1, true), buf(2, true), buf(3, false), buf(4, true), buf(5, false), buf(6, false), buf(7, false), buf(8, false), buf(9, false), buf(10, false)]);
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 11,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry { binding: 12, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None });
        entries.extend([buf(13, false), buf(14, false), buf(15, false), buf(16, false), buf(17, true), buf(18, false)]);
        entries.extend([buf(19, true), buf(20, false), buf(21, true), buf(22, false), buf(23, false), buf(24, false), buf(25, true), buf(26, false), buf(27, true), buf(28, true), buf(29, true), buf(30, false)]);
        // The merged layout: the buffers that stay, the slabs, and the offsets' uniform (merged.rs).
        if g.merged {
            use crate::merged::{LIVE_KEPT, OFFSETS_BINDING, SLABS};
            entries.retain(|e| !matches!(e.ty, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { .. }, .. }) || LIVE_KEPT.iter().any(|k| k.0 == e.binding));
            entries.extend(SLABS.iter().map(|s| buf(s.binding, s.read_only)));
            entries.push(wgpu::BindGroupLayoutEntry { binding: OFFSETS_BINDING, ..entries[0] });
        }
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("live"), entries: &entries });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let pipes = ["refl", "dtemporal", "atrous0", "atrous1", "atrous2", "atrous3", "atrous4", "stemporal", "sfilt0", "sfilt1", "sfilt2", "compose", "judge", "reject", "upscale", "dof_tiles", "dof", "mblur", "lum_down", "exposure", "lum_blur", "bloom_down", "bloom_dn1", "bloom_dn2", "bloom_dn3", "bloom_dn4",
                     "bloom_h5", "bloom_v5", "bloom_h4", "bloom_v4", "bloom_h3", "bloom_v3", "bloom_h2", "bloom_v2", "bloom_h1", "bloom_v1", "bloom_h0", "bloom_v0", "post"]
            .into_iter()
            .map(|name| {
                (name, d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(name), layout: Some(&pl), module: &module, entry_point: Some(name), compilation_options: Default::default(), cache: None }))
            })
            .collect();
        Passes { layout, pipes }
    }
}

/// Screen-size targets shared across scene reloads.
pub struct Targets {
    params: wgpu::Buffer,
    /// The diffuse irradiance history of the traced grid, then the output grid's colour history (ping-pong).
    hist: [Buf; 2],
    /// The traced grid's g-buffer (ping-pong: this frame's, the previous frame's).
    gb: [Buf; 2],
    /// What `judge` finds of the colour history per traced sample (live.wgsl `Judged`); the second is not read.
    alb: [Buf; 2],
    /// Specular history, diffuse luminance moments (ping-pong, traced grid), the specular filter's two slots, and the
    /// scratch `dtemporal` and the upscaled image share.
    shist: [Buf; 2],
    mom: [Buf; 2],
    sa: Buf,
    sb: Buf,
    /// Reflection resolve: the resolved specular reflection per traced pixel (live.wgsl `refl`), from the trace
    /// kernel's specular-lobe rays (2 vec4 per traced pixel, pathtrace.wgsl `refl`: `kernel.refl`).
    refl_res: Buf,
    /// Per traced sample: screen velocity, view depth, animated shading (live.wgsl `compose`); the frame's parity
    /// picks which of the two is written, the other is not read.
    xhist: [Buf; 2],
    fa: Buf,
    fb: Buf,
    bloom: [Buf; 2],
    pub out: wgpu::Buffer,
    /// Auto exposure: 64 histogram buckets (fixed point), state (exposure, target, mean log2 luminance, valid).
    ae_bins: wgpu::Buffer,
    ae_state: Buf,
    /// Quarter-resolution log2 luminance and its edge-aware blur (local exposure base).
    lumq: Buf,
    lumb: Buf,
    /// The kernel's group 1 (`gpu::KernelBufs`): the radiance cache, the reservoirs, and what the passes here read of
    /// a trace: the specular-lobe rays (`refl`), motion vectors per traced pixel (pathtrace.wgsl `motion_out`) and the
    /// deterministic direct light per traced pixel (`direct_out`).
    pub kernel: gpu::KernelBufs,
    /// The merged layout (merged.rs): the slabs the buffers above are ranges of (`SLABS`' first four).
    slabs: Option<[wgpu::Buffer; 4]>,
    lut_tex: wgpu::Texture,
    lut_view: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

impl Targets {
    /// For a view of `w` x `h`, a radiance cache of `cache` entries and reservoirs for `resv` pixels.
    fn new(g: &gpu::Gpu, w: u32, h: u32, lut: &[u16], cache: u64, resv: u64) -> Targets {
        use crate::merged::{SLABS, SLAB_F1, SLAB_U2, SLAB_U4, SLAB_V4};
        let d = &g.device;
        let n = (w * h) as u64;
        let sb = |label: &str, bytes: u64| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let bn = (w.div_ceil(4) * h.div_ceil(4)) as u64 * 16;
        let texels = |j: u32| bloom_dim(w, h, j).0 as u64 * bloom_dim(w, h, j).1 as u64;
        let bloom_texels = (0..5).map(texels).sum::<u64>() + (0..6).map(|k| texels(bloom_stage_level(k))).sum::<u64>();
        let lut_tex = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("tonemap lut"),
            size: wgpu::Extent3d { width: LUT, height: LUT, depth_or_array_layers: LUT },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        g.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &lut_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(lut),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(LUT * 8), rows_per_image: Some(LUT) },
            wgpu::Extent3d { width: LUT, height: LUT, depth_or_array_layers: LUT },
        );
        // The buffers, in the order of the fields below: a buffer each, or in the merged layout (merged.rs) a range of
        // the slab of its element type (each range starts at a multiple of 16 bytes: of every slab's element).
        let specs: [(&str, u64, usize); 22] = [
            ("hist0", n * 32, SLAB_V4), // traced grid, then output grid (live.wgsl `out_hist`)
            ("hist1", n * 32, SLAB_V4),
            ("gb0", n * 16, SLAB_U2),
            ("gb1", n * 16, SLAB_U2),
            ("alb0", n * 16, SLAB_U4),
            ("alb1", n * 16, SLAB_U4),
            ("shist0", n * 16, SLAB_V4),
            ("shist1", n * 16, SLAB_V4),
            ("mom0", n * 16, SLAB_V4),
            ("mom1", n * 16, SLAB_V4),
            ("spec_a", n * 16, SLAB_U2),
            ("spec_b", n * 16, SLAB_V4),
            ("refl_res", n * 16, SLAB_V4),
            ("direct_hist_a", n * 16, SLAB_V4),
            ("direct_hist_b", n * 16, SLAB_V4),
            ("fa", n * 16, SLAB_V4),
            ("fb", n * 16, SLAB_V4),
            ("bloom_a", bloom_texels * 16, SLAB_V4),
            ("bloom_b", bn, SLAB_V4),
            ("ae_state", 32, SLAB_V4),
            ("lumq", bn / 4, SLAB_F1),
            ("lumb", bn / 4, SLAB_F1),
        ];
        let slabs: Option<[wgpu::Buffer; 4]> = g.merged.then(|| std::array::from_fn(|k| sb(SLABS[k].name, specs.iter().filter(|s| s.2 == k).map(|s| s.1.next_multiple_of(16)).sum())));
        let mut at = [0u64; 4];
        let bufs: Vec<Buf> = specs
            .iter()
            .map(|&(label, bytes, slab)| match &slabs {
                None => Buf::whole(sb(label, bytes)),
                Some(s) => {
                    let b = Buf { buffer: s[slab].clone(), offset: at[slab], size: bytes };
                    at[slab] += bytes.next_multiple_of(16);
                    b
                }
            })
            .collect();
        let mut bufs = bufs.into_iter();
        let mut b = || bufs.next().expect("a buffer of `specs`");
        Targets {
            params: d.create_buffer(&wgpu::BufferDescriptor { label: Some("live params"), size: std::mem::size_of::<LiveParams>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            hist: [b(), b()],
            gb: [b(), b()],
            alb: [b(), b()],
            shist: [b(), b()],
            mom: [b(), b()],
            sa: b(),
            sb: b(),
            refl_res: b(),
            xhist: [b(), b()],
            fa: b(),
            fb: b(),
            bloom: [b(), b()],
            out: sb("out", n * 4),
            ae_bins: sb("ae_bins", 64 * 4),
            ae_state: b(),
            lumq: b(),
            lumb: b(),
            kernel: gpu::KernelBufs::new(d, g.merged, g.guard_room(), cache, resv, n * 32, n * 16, n * 16),
            slabs,
            lut_view: lut_tex.create_view(&Default::default()),
            lut_tex,
            sampler: d.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() }),
        }
    }

    /// What live.wgsl's buffer bindings hold in the bind group of parity `a` (besides the uniforms at 0, the trace's
    /// samples at 1, `out` at 10 and the histogram at 13): the history and the g-buffer swap roles with the parity.
    fn buffers(&self, a: usize) -> [(u32, &Buf); 25] {
        let b = 1 - a;
        [
            (2, &self.hist[a]),
            (3, &self.hist[b]),
            (4, &self.gb[a]),
            (5, &self.gb[b]),
            (6, &self.fa),
            (7, &self.fb),
            (8, &self.bloom[0]),
            (9, &self.bloom[1]),
            (14, &self.ae_state),
            (15, &self.lumq),
            (16, &self.lumb),
            (17, &self.alb[a]),
            (18, &self.alb[b]),
            (19, &self.shist[a]),
            (20, &self.shist[b]),
            (21, &self.mom[a]),
            (22, &self.mom[b]),
            (23, &self.sa),
            (24, &self.sb),
            (25, &self.kernel.refl),
            (26, &self.refl_res),
            (27, &self.kernel.motion),
            (28, &self.kernel.direct),
            (29, &self.xhist[a]),
            (30, &self.xhist[b]),
        ]
    }

    /// Two bind groups that swap history / g-buffer ping-pong roles on alternate frames. In the merged layout
    /// (merged.rs) they bind the same slabs, and differ in the offsets the passes read (`LO`).
    fn bind_groups(&self, d: &wgpu::Device, layout: &wgpu::BindGroupLayout, sample: &wgpu::Buffer) -> [wgpu::BindGroup; 2] {
        let mk = |a: usize| {
            fn e(binding: u32, buf: &wgpu::Buffer) -> wgpu::BindGroupEntry<'_> {
                wgpu::BindGroupEntry { binding, resource: buf.as_entire_binding() }
            }
            let mut entries = vec![
                e(0, &self.params),
                e(1, sample),
                e(10, &self.out),
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(&self.lut_view) },
                wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                e(13, &self.ae_bins),
            ];
            let buffers = self.buffers(a);
            let offsets;
            match (&self.slabs, &self.kernel.merged) {
                (Some(slabs), Some((live, _))) => {
                    use crate::merged::{LIVE_BUFS, OFFSETS_BINDING, SLABS, SLAB_K};
                    // Each merged buffer's (first element, length) in its slab, in `MergedOffsets`' order.
                    let lo: Vec<[u32; 2]> = LIVE_BUFS
                        .iter()
                        .map(|m| {
                            let buf = buffers.iter().find(|b| b.0 == m.binding).expect("a merged buffer's binding").1;
                            let stride = SLABS[m.slab].stride;
                            debug_assert!(buf.offset % stride == 0 && buf.size % stride == 0);
                            [(buf.offset / stride) as u32, (buf.size / stride) as u32]
                        })
                        .collect();
                    // (And the struct's unused last pair: `merged::OFFSETS_BYTES`.)
                    let mut lo = lo;
                    lo.push([0, 0]);
                    debug_assert_eq!(lo.len() * 8, crate::merged::OFFSETS_BYTES);
                    offsets = wgpu::util::DeviceExt::create_buffer_init(d, &wgpu::util::BufferInitDescriptor { label: Some("live offsets"), contents: bytemuck::cast_slice(&lo), usage: wgpu::BufferUsages::UNIFORM });
                    entries.extend(slabs.iter().zip(&SLABS).map(|(s, m)| e(m.binding, s)));
                    entries.push(e(SLABS[SLAB_K].binding, live));
                    entries.push(e(OFFSETS_BINDING, &offsets));
                }
                _ => entries.extend(buffers.iter().map(|(binding, buf)| wgpu::BindGroupEntry { binding: *binding, resource: buf.binding() })),
            }
            d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("live"), layout, entries: &entries })
        };
        [mk(0), mk(1)]
    }
}

struct LoadedScene {
    /// Load phases (parse, bvh, sky, accel build, upload) in ms.
    timings: Vec<(String, f64)>,
    /// World-space triangles + per-node ranges, kept on the CPU for picking and selection outlines; instanced meshes
    /// (model space) and their placements likewise.
    tris: Vec<[glam::Vec3A; 3]>,
    tri_nodes: Vec<(String, u32, u32)>,
    /// Flattened instances of instanced objects: (first, end, instance index), for picking.
    tri_instances: Vec<(u32, u32, u32)>,
    pool_tris: Vec<[glam::Vec3A; 3]>,
    meshes: Vec<scene::InstMesh>,
    instances: Vec<scene::Instance>,
    /// The instances' kernel records as uploaded (movable ones are rewritten when they move), then the procedural
    /// objects' (from `proc_first`).
    gpu_instances: Vec<scene::GpuInstance>,
    /// Procedural geometry (moves, picking), by node in `proc_of`; its records start at `proc_first`.
    procs: Vec<scene::ProcObject>,
    proc_of: std::collections::HashMap<u32, usize>,
    proc_first: usize,
    /// The scene has instanced meshes or procedural geometry (the raster experiment draws only flattened triangles:
    /// primary visibility is traced).
    instanced: bool,
    /// Kernel features of the instanced meshes (gpu::instance_features), which edits cannot change.
    instance_features: u32,
    name: String,
    triangles: usize,
    up: gpu::Uploaded,
    base: gpu::Params,
    bgs: [wgpu::BindGroup; 2],
    exposure: f32,
    exposure_settings: ExposureSettings,
    tan_half_fov: f32,
    scene_camera: Camera,
    /// The camera's depth of field (UE: off unless it sets a focus distance).
    dof: Option<crate::desc::Dof>,
    /// GPU table copies that parameter edits rewrite in place (`commit`).
    tables: tables::Tables,
    /// CPU copy of the dynamic triangles (posed models: the tail of the GPU triangle order from `dynamic_first`):
    /// vertices (kernel layout), shading attributes, BLAS vertices.
    dynamic: (usize, Vec<[f32; 4]>, Vec<scene::GpuTriAttr>, Vec<[f32; 3]>),
    /// Kernel feature inputs edits cannot change: textures, material functions, traversal groups.
    fixed_features: (bool, bool, [usize; crate::GROUPS]),
    /// Where in-place geometry edits write next (`edit_geometry`); a rebuild starts it afresh.
    edit: EditState,
    /// The atmosphere's sky once its sun has moved in place (`move_sun` starts it), and what it bakes with: the
    /// environment map's size, the viewer's altitude and the environment's light-sampling floor, as the load had them.
    sky: Option<DynamicSky>,
    /// Power edits' emitter rebuilds on a worker (`EmitterWorker`), for scenes with many emitters.
    emit: Option<EmitterWorker>,
    sky_inputs: ((u32, u32), f32, f32),
    /// For materials that start emitting (`add_material_emitters`): their flattened static triangles' runs (`prepare`)
    /// and the flattened triangles' build indices in traversal order (positions in `tris`).
    mat_runs: std::collections::HashMap<u32, Vec<[u32; 3]>>,
    tri_order: Vec<u32>,
}

/// Cores the sky's worker thread bakes with: the rest stay with the frame (scripts, uploads, the GPU's driver).
const SKY_THREADS: usize = 3;

/// An atmosphere whose sun moves while the scene runs (time of day). The sun's light is rewritten in place at every
/// move, its direction and its colour through the atmosphere (`sky::sun_illuminance`, no bake needed), which carries
/// the direct light and the shadows. The sky's radiance (the environment map and its sampling CDF) is baked again on a
/// worker thread with `SKY_THREADS` cores, the newest sun first, and lands with a later frame (`take_sky`): the sky
/// trails the sun by one bake (~10–40 ms at `sky::DYNAMIC_SKY`). Lockstep runs wait for it, so they repeat.
struct DynamicSky {
    atm: Arc<crate::sky::Atmosphere>,
    /// (sun direction, sun illuminance above the atmosphere, cloud cover)
    ask: mpsc::Sender<(glam::Vec3, glam::Vec3, f32)>,
    done: mpsc::Receiver<(gpu::EnvMap, Vec<f32>, f32)>,
    /// The brightest sky texel from which the environment is light-sampled (`Prepared::env_nee_floor`).
    env_nee_floor: f32,
}

impl DynamicSky {
    fn new(world: &World, (w, h): (u32, u32), altitude: f32, env_nee_floor: f32) -> Option<DynamicSky> {
        let crate::desc::Sky::Atmosphere { rayleigh_scale, mie_scale, mie_anisotropy, ground_albedo, glow, horizon_glow, sun: Some(_), .. } = &world.environment.sky else {
            return None;
        };
        let atm = Arc::new(crate::sky::Atmosphere::earth(*rayleigh_scale, *mie_scale, *mie_anisotropy, glam::Vec3::from(*ground_albedo)));
        let glow = crate::sky::glow_of(glow, horizon_glow);
        let (ask, asked) = mpsc::channel::<(glam::Vec3, glam::Vec3, f32)>();
        let (tx, done) = mpsc::channel();
        let a = atm.clone();
        std::thread::Builder::new()
            .name("sky bake".into())
            .spawn(move || {
                let pool = rayon::ThreadPoolBuilder::new().num_threads(SKY_THREADS).build().ok();
                let tables = match &pool {
                    Some(p) => p.install(|| crate::sky::SkyTables::new(&a)),
                    None => crate::sky::SkyTables::new(&a),
                };
                let bake = |sun: (glam::Vec3, glam::Vec3, f32)| {
                    let b = crate::sky::bake_atmosphere_tables(&a, &tables, sun.0, sun.1, altitude, w, h, glow, sun.2);
                    let env = gpu::EnvMap { width: b.width, height: b.height, texels: b.texels };
                    let cdf = crate::build_env_cdf(&env);
                    let max = env.texels.iter().map(|t| 0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]).fold(0.0f32, f32::max);
                    (env, cdf, max)
                };
                // The newest sun only: bakes asked for while one ran are stale.
                while let Ok(mut sun) = asked.recv() {
                    while let Ok(s) = asked.try_recv() {
                        sun = s;
                    }
                    let out = match &pool {
                        Some(p) => p.install(|| bake(sun)),
                        None => bake(sun),
                    };
                    if tx.send(out).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(DynamicSky { atm, ask, done, env_nee_floor })
    }
}

/// Scenes with at least this many emitters rebuild their emitter CDF for power edits on a worker (`EmitterWorker`).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
const ASYNC_EMITTERS: usize = 100_000;

/// Power edits in a scene with many emitters (a city's lamps and windows switching with the clock): the emitter CDF and
/// the light tree's node powers are rebuilt on a worker thread (~13 ms over 1.4 M emitters, which held up the frame in
/// `write_emitters`), and the edits land with them, in a later frame (`take_emitters`): the objects' powers, the CDF
/// and the tree's powers together, so sampling and shading switch at once. The worker writes the CDF and the tree's
/// nodes into a mapped staging buffer; the frame only copies it on the GPU (a 22 MB `write_buffer` cost it ~5 ms).
/// Lockstep frames wait for it, so they repeat. Scenes whose emitters change in place (added in place: the tree's slot;
/// moving emitters, whose nodes the frame refits) keep `write_emitters`; so does any other emitter edit (materials),
/// which first lands the edits held back. THREERT_EMIT_SYNC=1: never.
struct EmitterWorker {
    ask: mpsc::Sender<EmitterJob>,
    done: mpsc::Receiver<EmitterBuilt>,
    /// The emitters it was made for: a list that grew (added in place) makes another.
    count: usize,
    /// The edits of the job it is on (they land with its result).
    flight: Option<Vec<(usize, f32, f32)>>,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct EmitterJob {
    materials: Vec<scene::GpuMaterial>,
    objects: Vec<[f32; 4]>,
    lod: [f32; 4],
    cull: crate::EmitterCull,
}

struct EmitterBuilt {
    /// The CDF then the tree's nodes, as the emitters buffer holds them (`put_emitters`), and their bytes.
    staging: wgpu::Buffer,
    bytes: u64,
    /// The CDF's entries (its guide table included).
    entries: usize,
    total: f32,
    power: f32,
    /// The light tree's node powers (`nodes[i][2]`), for the frame's copy of the tree.
    node_powers: Vec<f32>,
}

impl EmitterWorker {
    /// wasm32: none, and power edits rebuild in the frame as before (the browser's WebGPU device and buffers stay on
    /// the thread that made them).
    #[cfg(target_arch = "wasm32")]
    fn new(_e: &tables::Tables, _device: &wgpu::Device) -> Option<EmitterWorker> {
        None
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn new(e: &tables::Tables, device: &wgpu::Device) -> Option<EmitterWorker> {
        let changing = e.light_tree.as_ref().is_some_and(|t| t.dynamic.is_some() || t.moving_count > 0);
        if e.emitter_tris.len() < ASYNC_EMITTERS || changing || std::env::var("THREERT_EMIT_SYNC").is_ok_and(|v| v == "1") {
            return None;
        }
        let device = device.clone();
        let (tris, mut tree, stride) = (e.emitter_tris.clone(), e.light_tree.clone(), e.stride);
        let (ask, asked) = mpsc::channel::<EmitterJob>();
        let (tx, done) = mpsc::channel();
        std::thread::Builder::new()
            .name("emitters".into())
            .spawn(move || {
                while let Ok(j) = asked.recv() {
                    let (em, total, power) = crate::emitter_cdf(&tris, &j.materials, &j.objects, stride, j.lod, j.cull);
                    let node_powers = tree.as_mut().map_or(vec![], |t| {
                        let w: Vec<f32> = t.order.iter().map(|&i| crate::emitter_power(&tris[i as usize], &j.materials, &j.objects, stride)).collect();
                        t.refit(&|k| w[k]);
                        t.nodes.iter().map(|n| n[2]).collect()
                    });
                    let nodes: &[[f32; 4]] = tree.as_ref().map_or(&[], |t| &t.nodes);
                    let bytes = ((em.len() + nodes.len()) * 16) as u64;
                    let staging = device.create_buffer(&wgpu::BufferDescriptor { label: Some("emitters staging"), size: bytes.max(16), usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: true });
                    let at = em.len() as u64 * 16;
                    for (range, data) in [(0..at, bytemuck::cast_slice::<_, u8>(&em)), (at..bytes, bytemuck::cast_slice(nodes))] {
                        if !data.is_empty() {
                            let Ok(mut m) = staging.slice(range).get_mapped_range_mut() else { break };
                            m.copy_from_slice(data);
                        }
                    }
                    staging.unmap();
                    if tx.send(EmitterBuilt { staging, bytes, entries: em.len(), total, power, node_powers }).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(EmitterWorker { ask, done, count: e.emitter_tris.len(), flight: None })
    }
}

/// Writes to one buffer gathered during an edit and sent merged: adjacent ranges as one `write_buffer` (an edit writes
/// hundreds of small records and meshes, contiguous in the room). Overlapping writes go in the order they came.
#[derive(Default)]
struct Gather {
    parts: Vec<(u64, Vec<u8>)>,
}

impl Gather {
    fn put(&mut self, at: u64, bytes: &[u8]) {
        self.parts.push((at, bytes.to_vec()));
    }

    fn flush(self, q: &wgpu::Queue, b: &wgpu::Buffer) {
        let mut parts = self.parts;
        let mut sorted = parts.clone();
        sorted.sort_by_key(|p| p.0);
        if sorted.windows(2).any(|w| w[0].0 + w[0].1.len() as u64 > w[1].0) {
            for (at, bytes) in parts.drain(..) {
                q.write_buffer(b, at, &bytes);
            }
            return;
        }
        let mut run: Option<(u64, Vec<u8>)> = None;
        for (at, bytes) in sorted {
            match &mut run {
                Some((start, data)) if *start + data.len() as u64 == at => data.extend_from_slice(&bytes),
                _ => {
                    if let Some((start, data)) = run.take() {
                        q.write_buffer(b, start, &data);
                    }
                    run = Some((at, bytes));
                }
            }
        }
        if let Some((start, data)) = run {
            q.write_buffer(b, start, &data);
        }
    }
}

/// Where an in-place edit's CPU time goes (ms) and what it did: building the triangles' GPU form, uploading them,
/// creating and recording BLASes and the TLAS; meshes added and rewritten in place, objects added or removed. Frame log
/// `edit_split`.
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
    fn add(&mut self, o: &EditSplit) {
        (self.tris, self.upload, self.blas) = (self.tris + o.tris, self.upload + o.upload, self.blas + o.blas);
        (self.added, self.deformed, self.objects) = (self.added + o.added, self.deformed + o.deformed, self.objects + o.objects);
    }
}

/// The edit room in use (gpu.rs `EditRoom`): the next free triangle (GPU order), kernel record, object-table entry,
/// procedural word (counted from the region's start) and TLAS instance, and where they started.
#[derive(Default)]
struct EditState {
    /// Triangles (GPU order), kernel records (a record's TLAS instance is `inst_base` + it) and object-table entries
    /// edits can place: holes left by what they no longer draw, then the room, which grows past its end (ROADMAP § 6
    /// step 5).
    tris: Holes,
    records: Holes,
    objects: Holes,
    /// The next procedural word, and where the region (grown as needed) ends.
    word: usize,
    word_cap: usize,
    /// The instanced mesh each object edits manage draws (`own_blas`: the build's and theirs), each mesh's objects, the
    /// meshes edits may free (own-BLAS meshes: shared only by objects of one geometry and material), and the slots of
    /// freed ones, reused by the next new mesh.
    mesh_of: std::collections::HashMap<u32, u32>,
    users: std::collections::HashMap<u32, u32>,
    managed: std::collections::HashSet<u32>,
    dead_meshes: Vec<u32>,
    /// Times the room grew (frame log `grows`), and whether the scene bind group must be made again.
    grows: u32,
    rebind: bool,
    /// (triangles, records, objects, words, TLAS instances) the scene itself has: the room starts there.
    base: (usize, usize, usize, usize, usize),
    /// The first pool triangle (instanced meshes' triangles are counted from it).
    pool_first: usize,
    /// Procedural blobs in the region by hash: their first word (the build's and those edits wrote).
    blobs: std::collections::HashMap<[u8; 32], usize>,
    /// Objects edits added: their (TLAS instance, record); instanced meshes edits added: their first record, capacity
    /// and first object entry (contiguous, as `update_instanced` rewrites them).
    placed: std::collections::HashMap<u32, (usize, usize)>,
    instanced: std::collections::HashMap<u32, (usize, usize, usize)>,
    /// The build's model instances (records below are theirs, in `LoadedScene::instances` order).
    inst_at_load: usize,
    /// What clicks pick among the objects edits added (`LiveCore::pick`; the build's are `instances` and `procs`).
    added: std::collections::BTreeMap<u32, Picked>,
}

/// Ranges of one kind of the edit room: free holes (start -> length; first fit, neighbours merged) below `end`, the
/// part in use, which may reach `cap`, what the buffers hold (more by growing them).
#[derive(Default, Debug)]
struct Holes {
    free: std::collections::BTreeMap<usize, usize>,
    end: usize,
    cap: usize,
    /// Where the room starts (the scene's own data before it).
    base: usize,
}

impl Holes {
    fn new(end: usize, cap: usize) -> Holes {
        Holes { free: Default::default(), end, cap, base: end }
    }

    /// `n` contiguous: the first hole that fits, else at the end if within `cap`.
    fn take(&mut self, n: usize) -> Option<usize> {
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
    fn give(&mut self, at: usize, n: usize) {
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
    fn used(&self) -> usize {
        self.end - self.free.values().sum::<usize>()
    }
}

/// An object an edit placed, as `LiveCore::pick` tests it.
enum Picked {
    Mesh(scene::Instance),
    Proc(scene::ProcObject),
}

/// What an edit places at an object's slot: a procedural object's BLAS (hardware traversal), or instanced mesh `k`.
enum Placed {
    Proc(wgpu::Blas),
    Mesh(u32),
}

/// One object fewer draws instanced mesh `k`: a mesh edits manage that none draws any more is freed (its triangles a
/// hole the next new mesh can take, its slot reused, its BLAS dropped with it).
fn drop_mesh_user(k: u32, edit: &mut EditState, tables: &mut tables::Tables, meshes: &[scene::InstMesh]) {
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
struct Grow<'a> {
    g: &'a gpu::Gpu,
    ok: bool,
}

impl Grow<'_> {
    /// The room twice as large (the scene's own data before it stays as it is), at least `extra` more at the end.
    fn cap(h: &Holes, extra: usize) -> usize {
        h.base + ((h.cap - h.base) * 2).max(h.end + extra - h.base).max(1024)
    }

    /// Whether buffers of these sizes (bytes) can be bound (the device's storage binding and buffer limits): past them
    /// the edit rebuilds instead (Bugbot on #131).
    fn fits(&self, sizes: &[u64]) -> bool {
        let l = self.g.device.limits();
        let max = (l.max_storage_buffer_binding_size as u64).min(l.max_buffer_size);
        sizes.iter().all(|&s| s <= max)
    }

    /// Triangles: vertices, attributes, the BLASes' vertices, vertex colours (after the previous vertices).
    fn tris(&self, h: &mut Holes, extra: usize, verts: &mut wgpu::Buffer, attrs: &mut wgpu::Buffer, prev: &mut wgpu::Buffer, vcolor_base: Option<u64>, accel: &mut gpu::Accel) -> bool {
        if !self.ok {
            return false;
        }
        let cap = Self::cap(h, extra);
        let prev_bytes = vcolor_base.map_or(0, |base| (base + cap as u64 * 12).next_multiple_of(16));
        if !self.fits(&[(cap * 48) as u64, (cap * std::mem::size_of::<scene::GpuTriAttr>()) as u64, (cap * 36) as u64, prev_bytes]) {
            return false;
        }
        *verts = self.g.grown(verts, (cap * 48) as u64, "verts");
        *attrs = self.g.grown(attrs, (cap * std::mem::size_of::<scene::GpuTriAttr>()) as u64, "attrs");
        self.g.grow_blas_verts(accel, (cap * 36) as u64);
        if vcolor_base.is_some() {
            *prev = self.g.grown(prev, prev_bytes, "prev verts + vertex colours");
        }
        h.cap = cap;
        true
    }

    /// Kernel records and the TLAS (a record's instance is `inst_base` + it).
    fn records(&self, h: &mut Holes, extra: usize, instances: &mut wgpu::Buffer, accel: &mut gpu::Accel) -> bool {
        if !self.ok {
            return false;
        }
        let cap = Self::cap(h, extra);
        if !self.fits(&[(cap * std::mem::size_of::<scene::GpuInstance>()) as u64]) {
            return false;
        }
        *instances = self.g.grown(instances, (cap * std::mem::size_of::<scene::GpuInstance>()) as u64, "instances");
        self.g.grow_tlas(accel, accel.inst_base + cap);
        h.cap = cap;
        true
    }

    /// Object-table entries (`stride` vec4s each).
    fn objects(&self, h: &mut Holes, extra: usize, objects: &mut wgpu::Buffer, stride: usize) -> bool {
        if !self.ok {
            return false;
        }
        let cap = Self::cap(h, extra);
        if !self.fits(&[(cap * stride * 16) as u64]) {
            return false;
        }
        *objects = self.g.grown(objects, (cap * stride * 16) as u64, "objects");
        h.cap = cap;
        true
    }

    /// Procedural words, after the environment's CDF (`cdf_words` of them).
    fn words(&self, cap: &mut usize, need: usize, cdf: &mut wgpu::Buffer, cdf_words: usize) -> bool {
        if !self.ok {
            return false;
        }
        let new = (*cap * 2).max(need);
        if !self.fits(&[((cdf_words + new) * 4) as u64]) {
            return false;
        }
        *cap = new;
        *cdf = self.g.grown(cdf, ((cdf_words + *cap) * 4) as u64, "env_cdf + procedural");
        true
    }
}

/// An object removed or hidden in place: no longer picked until the next rebuild (`LiveCore::pick`).
fn unpick(id: u32, edit: &mut EditState, tables: &tables::Tables, instances: &mut [scene::Instance], procs: &mut [scene::ProcObject], proc_of: &std::collections::HashMap<u32, usize>) {
    edit.added.remove(&id);
    if let Some(i) = tables.retained.own_instance.get(&id).and_then(|&k| instances.get_mut(k as usize)) {
        i.hidden = true;
    }
    if let Some(p) = proc_of.get(&id).and_then(|&j| procs.get_mut(j)) {
        p.hidden = true;
    }
}

fn load_scene(g: &gpu::Gpu, world: &World, assets: &Path, presets: &Path, w: u32, h: u32, targets: &Targets, passes: &Passes) -> Result<(LoadedScene, crate::desc::PostPreset)> {
    let t0 = Instant::now();
    let mut sc = scene::build(world, assets, presets, None)?;
    sc.width = w;
    sc.height = h;
    // Emitters its edits add get a light-tree subtree of their own (ROADMAP § 6 step 4; THREERT_LT_DYNAMIC=0: by power
    // share, the A/B).
    // (Software traversal with units edits in place as hardware traversal does: `units_wanted`.)
    sc.emitter_slot = gpu::EditRoom::editable(&sc, g.hwrt || crate::units_wanted(g.hwrt, &sc)) && std::env::var("THREERT_LT_DYNAMIC").map_or(true, |v| v != "0");
    let mut timings = vec![("build_ms".to_string(), t0.elapsed().as_secs_f64() * 1000.0)];
    let mut prep = crate::prepare_live(g, &mut sc, &mut timings);
    let mut tables = tables::Tables::new(&sc, std::mem::take(&mut prep.emitter_tris), prep.light_tree.take());
    // (THREERT_EDIT_REBUILD=1: every edit rebuilds, a material that starts emitting too: the A/B.)
    tables.emit_in_place = std::env::var("THREERT_EDIT_REBUILD").map_or(true, |v| v != "1");
    tables.fixed_mats = std::mem::take(&mut prep.fixed_mats);
    let emit = EmitterWorker::new(&tables, &g.device);
    tables.defer_powers = emit.is_some();
    let (mat_runs, tri_order) = (std::mem::take(&mut prep.mat_runs), std::mem::take(&mut prep.tri_order));
    let fixed_features = (!sc.textures.is_empty(), !sc.shaders.is_empty(), prep.groups);
    let mut up = g.upload(&sc, w, h, &prep.tri_verts, prep.groups, bytemuck::cast_slice(&prep.nodes), &prep.emitters, &prep.env, &prep.env_cdf, &prep.energy, &mut timings)?;
    // The merged layout: the kernel written for this view's group-1 buffers (merged.rs).
    if let (Some(caps), Some((_, live))) = (&mut up.kernel.caps, &targets.kernel.merged) {
        caps.live = *live;
    }
    let _ = g.pipeline(&up.kernel, true); // compile the scene's kernel variant now, not in the first frame
    let sky_inputs = ((prep.env.width, prep.env.height), sc.camera.position.y.max(0.0), prep.env_nee_floor);
    let mut base = prep.params;
    // The live viewport resamples fewer light-tree candidates at camera hits than offline renders (ltree.rs).
    if base.ltree[3] > 0.5 {
        base.ltree[3] = crate::ltree::live_candidates() as f32;
    }
    base.cam_fwd[3] = w as f32 / h as f32;
    let bgs = targets.bind_groups(&g.device, &passes.layout, &up.accum);
    let exposure_settings = ExposureSettings::new(&sc.camera.exposure, &sc.preset.post);
    // Manual: 2^-EV100. Auto: a first guess for the firefly clamp (the GPU histogram takes over from frame 1).
    let exposure = match &sc.camera.exposure {
        crate::desc::Exposure::Manual { ev100 } => 2f32.powf(-ev100),
        crate::desc::Exposure::Auto { compensation, max_ev100, .. } => 2f32.powf(*compensation) / 2f32.powf(*max_ev100),
    };
    let c = &sc.camera;
    let scene_camera = Camera { pos: c.position, fwd: c.forward, up: c.up, right: c.right, tan_half_fov: c.tan_half_fov };
    let (tris, tri_nodes, tri_instances) = (std::mem::take(&mut sc.tris), std::mem::take(&mut sc.tri_nodes), std::mem::take(&mut sc.tri_instances));
    let instanced = !sc.gpu_instances.is_empty() || !sc.procs.is_empty();
    let instance_features = gpu::instance_features(&sc);
    let (pool_tris, meshes, instances) = (std::mem::take(&mut sc.pool_tris), std::mem::take(&mut sc.meshes), std::mem::take(&mut sc.instances));
    let mut gpu_instances = std::mem::take(&mut sc.gpu_instances);
    let proc_first = gpu_instances.len();
    gpu_instances.extend_from_slice(&sc.proc_records);
    let procs = std::mem::take(&mut sc.procs);
    let proc_of = procs.iter().enumerate().filter(|(_, p)| p.sub.is_none()).map(|(j, p)| (p.node, j)).collect();
    let triangles = sc.stats.get("triangles").map_or(tris.len(), |t| *t as usize);
    let first = prep.groups[..4].iter().sum::<usize>();
    // The kernel's first dynamic triangle (motion vectors, prev_verts) is group 4's first.
    debug_assert_eq!(prep.params.group_first[1][0] as usize, first);
    // The dynamic tail: groups 4..8 (the instanced pool follows them).
    let end = prep.groups.iter().sum::<usize>();
    let blas = prep.tri_verts[first * 3..end * 3].chunks(3).flat_map(|t| [0, 1, 2].map(|k| if k == 0 { [t[0][0], t[0][1], t[0][2]] } else { [t[0][0] + t[k][0], t[0][1] + t[k][1], t[0][2] + t[k][2]] })).collect();
    let dynamic = (first, prep.tri_verts[first * 3..end * 3].to_vec(), sc.attrs[first..end].to_vec(), blas);
    let blobs = sc.proc_layout.as_ref().map_or_else(Default::default, |l| l.blobs.iter().map(|(b, off)| (b.hash, l.head.len() + off)).collect());
    let slot = up.accel.as_ref().map_or(0, |a| a.proc_base + procs.len());
    let room_base = (prep.tri_verts.len() / 3, gpu_instances.len(), tables.objects.len() / tables.stride, up.region_words, slot);
    // A record's TLAS instance is `inst_base` + it (model instances, then procedural objects, then the room's).
    debug_assert!(up.accel.as_ref().is_none_or(|a| room_base.4 == a.inst_base + room_base.1));
    let room = up.room;
    let mut users: std::collections::HashMap<u32, u32> = Default::default();
    for i in &instances {
        *users.entry(i.mesh).or_default() += 1;
    }
    let mesh_of = tables.retained.own_instance.iter().filter_map(|(&id, &k)| instances.get(k as usize).map(|i| (id, i.mesh))).collect();
    let managed = tables.retained.own_mesh.values().chain(tables.retained.emitter_mesh.values()).copied().collect();
    let edit = EditState {
        tris: Holes::new(room_base.0, room_base.0 + room.tris),
        records: Holes::new(room_base.1, room_base.1 + room.records),
        objects: Holes::new(room_base.2, room_base.2 + room.objects),
        word: room_base.3,
        word_cap: room_base.3 + room.proc_words,
        mesh_of,
        users,
        managed,
        dead_meshes: vec![],
        grows: 0,
        rebind: false,
        base: room_base,
        pool_first: prep.groups.iter().sum(),
        blobs,
        placed: Default::default(),
        instanced: Default::default(),
        inst_at_load: instances.len(),
        added: Default::default(),
    };
    Ok((LoadedScene { timings, triangles, tris, tri_nodes, tri_instances, pool_tris, meshes, instances, gpu_instances, procs, proc_of, proc_first, instanced, instance_features, name: sc.name.clone(), up, base, bgs, exposure, exposure_settings, tan_half_fov: c.tan_half_fov, scene_camera, dof: c.dof.clone(), tables, fixed_features, dynamic, edit, sky: None, emit, sky_inputs, mat_runs, tri_order }, sc.preset.post))
}

pub enum Cmd {
    Camera(Camera),
    /// An event for the in-process game (type, payload).
    Event(String, serde_json::Value),
    /// A click at (u, v) for the in-process game (picks the node under it).
    Click(f32, f32),
    /// Snapshot (path) of the first frame whose history length reaches `after` (None = any) and that is at least the
    /// `frames`-th frame from now (None = the next).
    Snapshot(PathBuf, Option<u32>, Option<u32>),
    /// Answer `live: sync <id>` on stderr once the n-th frame from now is rendered (benchmarks step the camera frame by
    /// frame, so what they measure does not depend on how many frames a busy machine renders per second).
    Sync(String, u32),
    /// Lockstep (benchmarks): with Some(frame time, s) the runner renders only the frames `Step`s ask for, at that frame
    /// time (motion blur, exposure, frame script); None = real time again.
    Lockstep(Option<f32>),
    /// A step: an optional camera, `frames` frames to render (lockstep), and a sync answer / snapshot at the last of them,
    /// registered with the camera in one command so neither can land a frame late.
    Step { cam: Option<Camera>, frames: u32, sync: Option<String>, snapshot: Option<PathBuf> },
    /// Drop all accumulated history (benchmarks measure convergence from here).
    Reset,
    /// Set the scene time, s (benchmarks: with lockstep, frame k from here renders at t + k x its frame time; frame
    /// time 0 holds it there).
    Time(f32),
    /// View mode (see LiveCore::view_mode).
    Mode(u32),
    Bounces(u32),
    /// Max samples per frame (adaptive below it).
    SppCap(u32),
    /// Viewer status text (e.g. the game score), shown in the window title.
    Status(String),
    /// Render scale (0 = dynamic resolution).
    Scale(f32),
    Quit,
}

/// The real-time renderer: scene + screen targets + camera/history state. Records one frame at a time; the output
/// (sRGB BGRA8 per pixel) lands in `targets.out`. Used by the headless runner, the native window viewer and the playground.
pub struct LiveCore {
    /// The game's particle spawns since the last frame (`World::particle_spawns`), for the GPU particle pass.
    particle_spawns: std::collections::BTreeMap<u32, Vec<f32>>,
    /// The sources the game gave its density grids since the last frame (`World::grid_sources`), for the grid pass.
    grid_sources: std::collections::BTreeMap<u32, Vec<f32>>,
    pub g: gpu::Gpu,
    passes: Passes,
    pub targets: Targets,
    sc: LoadedScene,
    presets: PathBuf,
    /// Asset root (models, file textures).
    assets: PathBuf,
    pub w: u32,
    pub h: u32,
    /// Committed state sequence number the image shows (set by the game session).
    pub seq: i64,
    aspect: f32,
    pub cam: Camera,
    prev_cam: Camera,
    pub frame: u32,
    pub history_len: f32,
    reset: bool,
    /// Bloom stage radii / tints (`bloom_stages`).
    bloom: ([f32; 6], [f32; 6]),
    vignette: f32,
    /// Chromatic aberration, film grain, bloom threshold, longest motion blur (output pixels).
    post: [f32; 4],
    /// Motion blur (UE MotionBlurAmount, MotionBlurTargetFPS; amount 0 = off).
    blur: (f32, f32),
    /// Frames since the view last moved sideways or along its axis (0 = this frame; rotation alone keeps counting).
    still: u32,
    /// Last submission of each frame still on the GPU (at most two frames in flight).
    inflight: std::collections::VecDeque<wgpu::SubmissionIndex>,
    pub gpu_ms: f32,
    /// The frame `gpu_ms` timed (it lags by the frames in flight).
    gpu_frame: u32,
    /// That frame's trace pass alone (the path-tracing kernel: intersection and shading), ms; 0 without timestamps.
    /// Frame log `trace`: the per-frame cost a geometry representation changes, without THREERT_PROFILE's stalls.
    pub gpu_trace_ms: f32,
    /// This frame's geometry edits applied in place (`edit_geometry`: CPU ms, its BLAS / TLAS builds submitted), and
    /// scene rebuilds (ms) and, when edits fell back to one, why: summed over the frame's commits (a game event's and the
    /// frame script's), reset when the frame is logged. Fallbacks since the viewport opened.
    /// Frame log `edit`, `rebuild`, `fallback`, `fallbacks`.
    pub edit_ms: f32,
    /// CPU time of this frame's in-place instance updates (`update_instanced`), ms.
    pub instanced_ms: f32,
    pub edit_split: EditSplit,
    pub rebuild_ms: f32,
    pub fallback: Option<String>,
    pub fallbacks: u32,
    /// Path-tracing bounces for real-time frames (the offline preset uses more).
    pub bounces: u32,
    /// Ray tracing (true). Off (`set_rays`), the kernel traces the camera ray of each pixel and no other: no shadows,
    /// no bounce light, reflections of the sky only; analytic lights and emissive surfaces light unoccluded. The
    /// fallback for GPUs that trace too slowly (no hardware ray queries), and the "what tracing adds" comparison on
    /// those that do: the same scene, materials, exposure and post either way.
    rays: bool,
    /// 0 lit, 1 base color, 2 world normal, 3 lit without denoiser, 4 raw sample, 5 history length, 6 diffuse,
    /// 7 specular, 8 deterministic direct light (THREERT_VIEW_MODE at start: e.g. bench-temporal per signal).
    pub view_mode: u32,
    /// Lockstep frame time (s; benchmarks): motion blur and exposure adaptation use it instead of the wall clock.
    pub fixed_dt: Option<f32>,
    /// A/B switches (playground keys N / H): motion blur off whatever the scene's post settings say; the
    /// translation-aware specular history cap of PR #24 off (temporal flag 16384).
    pub motion_blur_off: bool,
    pub ghost_fix_off: bool,
    pub triangles: usize,
    pub scene_name: String,
    /// Scene time, s: what `advance_time` added up, the dt each frame script gets, so it is the game's clock (`f.time`)
    /// and a burst fired at a game time starts then; lockstep runs repeat exactly. Animated shading, the particle pass
    /// and power edits' time stamps use it (`shader_time`).
    time: f32,
    /// Time between rendered frames (auto exposure adaptation), s.
    frame_dt: f32,
    last_frame: Instant,
    /// The post preset the LUT / bloom / grain settings were built from (scenes may override it).
    post_key: String,
    /// Traced / output resolution (UE screen percentage); the temporal pass upsamples below 1.
    pub render_scale: f32,
    /// Adjust `render_scale` to the frame budget (`govern_resolution`).
    pub dynamic_res: bool,
    /// Dynamic resolution's lowest scale: `MIN_SCALE` over the display's scale factor.
    pub min_scale: f32,
    /// The view's pixels a point of its display (`set_display_scale`): dynamic resolution's steps are a point's.
    display_scale: f32,
    /// Dynamic resolution: consecutive measured frames well over the frame time, and the first frame at the current
    /// scale.
    over_budget: (u32, u32),
    /// THREERT_PROFILE=1: GPU time per pass (timestamp queries), printed every 2 s.
    profile: Option<Profile>,
    /// GPU time per frame without stalls (timestamp queries; None: measured by draining the queue twice a second).
    timer: Option<FrameTimer>,
    /// Rasterized primary visibility (None: the kernel traces camera rays).
    raster: Option<Raster>,
    /// World-space radiance cache (pathtrace.wgsl; None: THREERT_CACHE=0, paths trace every bounce).
    cache: Option<RadianceCache>,
    /// Emitter reservoirs at camera hits (pathtrace.wgsl; experiment, THREERT_RESTIR=1), and the previous frame's traced
    /// size while its reservoirs are valid (not after a reset or a rebuild: triangle indices change).
    restir: bool,
    restir_prev: Option<(u32, u32)>,
    /// Kernel group 1: primary visibility + radiance cache + emitter reservoirs.
    kernel_bg: wgpu::BindGroup,
    /// Span (relative to the dynamic tail) re-transformed by the last in-place update whose previous-frame vertices
    /// (`Uploaded::prev_verts`) still hold the frame before it: synced to the current ones next frame.
    prev_span: Option<(usize, usize)>,
    /// An in-place update happened this frame (it keeps `prev_verts` itself).
    reposed: bool,
    /// Dynamic triangles moved in this commit: the light tree's moving subtree is refit once at its end.
    emitters_moved: bool,
    /// Frames since the last reset or camera cut (`COLD_FALLBACK_FRAMES`).
    cold_frames: u32,
    /// Likewise for the deforming meshes of shared poses (`repose_groups`): the span they took (relative to the dynamic
    /// tail, past its end) and whether this frame re-skinned any.
    pose_span: Option<(usize, usize)>,
    pose_reposed: bool,
    /// Movable instances moved by the last update (their records' previous-frame rows differ), and whether it was this
    /// frame's.
    inst_moving: Vec<u32>,
    inst_moved: bool,
    /// The previous frame's traced size (its traced-grid diffuse history; 0 before the first frame).
    prev_isz: (u32, u32),
    /// Frame of the last lighting edit (power, light / material parameters, environment): the temporal pass looks
    /// for changed lighting (anti-lag) only shortly after one.
    lit_changed: u32,
    /// The scene has light-sampled particles (emitter table entries with a nominal area).
    spark_lights: bool,
    /// Samples per pixel of the last frame rendered (the kernel's sample buffer holds their sums).
    shown_spp: u32,
    /// CPU time of the last commit (ms): skinning and re-transforming moving objects (`reskin`, rigid instance poses), and
    /// uploading them with the dynamic BLAS / TLAS rebuild submitted (frame log `skin`, `upload`).
    pub cpu_skin_ms: f32,
    pub cpu_upload_ms: f32,
    /// THREERT_PROFILE_BLAS=1: GPU time of the last dynamic BLAS / TLAS rebuild (ms; frame log `blas`), measured by draining
    /// the queue before it and waiting for it after (it stalls the frame: measurement runs only). The pass profile has
    /// no timestamps for it (Metal cannot write them between acceleration-structure builds).
    pub gpu_blas_ms: f32,
    /// CPU time the last `render` spent waiting for the oldest frame in flight (ms; frame log `wait`): GPU-bound frames.
    pub cpu_wait_ms: f32,
    /// The cloud cover the sky and the sun light were made with (`move_sun`).
    sky_cover: f32,
    /// The loop guard (merged.rs `guard`), if a kernel of this session may have it: the invocations it has stopped at
    /// its limit since the view was made, as last read back (a frame or two behind, without waiting). Above zero:
    /// frames with wrong pixels, a traversal that did not end on the scene's data. None without the guard.
    pub guard_trips: Option<u32>,
    /// Its readback: the buffer and its state (0 free, 1 mapping, 2 mapped), as the frame timer's.
    guard_read: Option<(wgpu::Buffer, Arc<AtomicU32>)>,
}

/// GPU time per frame from timestamp queries (start of the first trace pass .. end of the post pass), read back a few
/// frames later without waiting, so measuring never drains the pipeline.
struct FrameTimer {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    /// Per slot: readback buffer and its state (0 free, 1 mapping, 2 mapped).
    reads: Vec<(wgpu::Buffer, Arc<AtomicU32>)>,
    /// Per slot: the frame it timed, and that frame's samples per pixel (its traces; only the first is timestamped).
    frames: [u32; TIMER_SLOTS],
    spps: [u32; TIMER_SLOTS],
    slot: usize,
}

const TIMER_SLOTS: usize = 4;

/// Gap (ms) past which a slot's post pass is not its trace's frame's (`frame_gpu_ms`).
const TIMER_GAP_MS: f32 = 2.0;

/// A frame's GPU time (ms) from its slot's timestamps (trace, post, raster, particle pass pairs; zero: not run): from
/// its first pass (particles, raster or trace) to the end of its post pass. The post pass follows the trace's command
/// buffer within ~0.2 ms, after the frame's further samples (`spp` - 1 more traces, untimed, each about as long as
/// the first). In 2–3 % of the crowd scene's frames (Metal) a slot read the post pair of the frame that used the slot
/// next, 4 frames on: the span then read 70–80 ms while frames came every 16.7 ms. A post pass that begins more than
/// TIMER_GAP_MS past where those traces would end (twice their length) is not this frame's: the frame is counted as
/// its passes, the further traces at the first one's length.
fn frame_gpu_ms(t: &[u64; 8], spp: u32, period_ns: f32) -> Option<f32> {
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
    fn new(d: &wgpu::Device) -> FrameTimer {
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
    fn harvest(&mut self, period_ns: f32) -> (Option<(f32, u32, f32)>, Option<usize>) {
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
struct Raster {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    vis: wgpu::TextureView,
    depth: wgpu::TextureView,
}

const RASTER_WGSL: &str = r#"
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
    fn new(g: &gpu::Gpu, w: u32, h: u32) -> Raster {
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
            cache: None,
        });
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
    fn record(&self, g: &gpu::Gpu, e: &mut wgpu::CommandEncoder, verts: &wgpu::Buffer, tris: u32, cam: &Camera, aspect: f32, (iw, ih): (u32, u32), (jx, jy): (f32, f32), timestamps: Option<wgpu::RenderPassTimestampWrites>) {
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

/// THREERT_CACHE_FLAGS: experiment switches of the radiance cache (pathtrace.wgsl `cfl`), 0 = defaults: 1 = no cold
/// fallback (cold voxels read as they are), 2 = no hit lighting (reflection hits use the cache's total), 4 = no
/// training tiles, 8 = the exact light loop at reflection hits. THREERT_CACHE=0 turns the cache off.
/// Frames after a reset or a camera cut during which pixels whose radiance-cache voxel is cold trace a fresh estimate
/// (pathtrace.wgsl, cache flag 1 off). Later the cold voxels left are mostly where bounce rays land off screen,
/// scattered, sparsely trained: their traces cost each SIMD group nearly a whole extra trace (Bistro's path 0.7-1.2
/// ms, the crowd's square 2.3 ms) for ~1 % of the image (bench-crowd ΔE 13.1 -> 13.3).
const COLD_FALLBACK_FRAMES: u32 = 30;

/// Ceiling for one live lighting sample, in display units at the current exposure (live.wgsl `sample_clamp`): fireflies
/// far above white. 64 until 2026-11-09, when it cut the light of small strong lights that the denoisers now keep: the
/// Bistro crowd in motion (lockstep at 10 s) diffuse 0.87 -> 0.90 of our offline render, specular 0.83 -> 0.89, the
/// pavement under the headlights 0.71 -> 0.77. 1024 adds nothing more.
const SAMPLE_CEILING: f32 = 256.0;

fn cache_flags() -> u32 {
    static F: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("THREERT_CACHE_FLAGS").ok().and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// THREERT_FREEZE_TIME=1: animation holds still (shader time 0, the frame script gets dt 0: signs, screens, flicker groups),
/// so frames taken at different times compare (tools/bench-ghosting.ts).
fn freeze_time() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("THREERT_FREEZE_TIME").is_ok_and(|v| v == "1"))
}

/// Seconds of animated shading (sign flicker, screens, ripples) and particles: the scene time; 0 under THREERT_FREEZE_TIME.
fn shader_time(time: f32) -> f32 {
    if freeze_time() { 0.0 } else { time }
}

/// THREERT_EXPOSURE_SNAP=1: auto exposure jumps to its target every frame (no adaptation lag), so a frame taken while the
/// view moves has the exposure a settled view of that pose would have (tools/bench-ghosting.ts).
fn exposure_snap() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("THREERT_EXPOSURE_SNAP").is_ok_and(|v| v == "1"))
}

/// UE's physical CoC model (DiaphragmDOFUtils.cpp, FPhysicalCocModel::Compile): focal length from the sensor height
/// (sensor width / aspect) and the vertical FOV, the CoC of infinity on the sensor f^2 / (N (focus - f)), as a radius in
/// output pixels; max radius r.DOF.Kernel.Max{Fore,Back}groundRadius = 2.5 % of the width. Off without a focus distance.
fn dof_params(dof: Option<&crate::desc::Dof>, tan_half_fov: f32, w: u32, h: u32) -> [f32; 4] {
    let Some(d) = dof.filter(|d| d.focus_distance > 0.0 && d.f_stop > 0.0 && d.sensor_width > 0.0) else { return [0.0; 4] };
    let aspect = w as f32 / h as f32;
    let sensor_h = d.sensor_width / aspect;
    let f = 0.5 * sensor_h / tan_half_fov;
    let diameter = f * f / (d.f_stop * (d.focus_distance * 1000.0 - f).max(1e-3));
    [diameter * 0.5 / sensor_h / aspect * w as f32, d.focus_distance, 0.025 * w as f32, 0.0]
}

/// Live passes that run on the traced grid (the rest on the output grid).
fn traced_pass(name: &str) -> bool {
    name == "refl" || name == "dtemporal" || name.starts_with("atrous") || name == "stemporal" || name.starts_with("sfilt") || name == "compose" || name == "judge" || name == "reject"
}

/// Per-object motion vectors (THREERT_NO_MV=1: off, moving objects keep a short history instead).
fn motion_vectors() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("THREERT_NO_MV").is_ok_and(|v| v == "1"))
}

/// Light LOD (`emitter_lod`): emitters within this distance of the camera keep their sampling weight, farther ones
/// lose it as (d0 / d)^2. THREERT_LIGHT_LOD = d0 in metres (0 = off).
const LIGHT_LOD_D0: f32 = 10.0;

/// Emitters farther than this from the light LOD's reference are not proposed (`emitter_lod`); BSDF rays bring their
/// light. THREERT_EMITTER_CULL = metres (0 = off).
const EMITTER_CULL: f32 = 60.0;

/// The cull's reach inside the view cone (`EmitterCull`), in cull radii: emitters ahead of the camera are kept out to
/// this. THREERT_EMITTER_CULL_VIEW = the multiple (0 = no cone: the camera-centred cull alone, as before 2026-11-08).
const EMITTER_CULL_VIEW: f32 = 2.0;
/// The view cone: the frustum's half-diagonal plus this margin (degrees); the CDF is rebuilt when the camera turns by
/// LIGHT_LOD_TURN, so the frustum stays inside the cone between rebuilds.
const EMITTER_CULL_MARGIN: f32 = 20.0;
const LIGHT_LOD_TURN: f32 = 15.0;

fn emitter_cull_view() -> f32 {
    static F: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("THREERT_EMITTER_CULL_VIEW").ok().and_then(|v| v.parse().ok()).unwrap_or(EMITTER_CULL_VIEW))
}

fn emitter_cull() -> f32 {
    static F: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("THREERT_EMITTER_CULL").ok().and_then(|v| v.parse().ok()).unwrap_or(EMITTER_CULL))
}

fn light_lod_d0() -> f32 {
    static F: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("THREERT_LIGHT_LOD").ok().and_then(|v| v.parse().ok()).unwrap_or(LIGHT_LOD_D0))
}

/// THREERT_TEMPORAL_FLAGS: experiment switches of the temporal pass (live.wgsl `xflag`), 0 = defaults.
fn temporal_flags() -> u32 {
    static F: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("THREERT_TEMPORAL_FLAGS").ok().and_then(|v| v.parse().ok()).unwrap_or(0))
}

/// Halton(base) of index i (sub-pixel jitter sequence).
fn halton(mut i: u32, base: u32) -> f32 {
    let (mut f, mut r) = (1.0f32, 0.0f32);
    while i > 0 {
        f /= base as f32;
        r += f * (i % base) as f32;
        i /= base;
    }
    r
}

/// The live kernel's world-space radiance cache (docs/ROADMAP.md § 1, step 1): its buffers and per-frame resolve.
struct RadianceCache {
    bufs: gpu::CacheBuffers,
    resolve: wgpu::ComputePipeline,
}

/// History caps of the cache: samples (noise), frames (lag; shorter for a while after a lighting edit).
const CACHE_SAMPLES: f32 = 256.0;
const CACHE_FRAMES: f32 = 32.0;
const CACHE_FRAMES_LIT: f32 = 4.0;
/// Share of the pixels showing translucent particles whose nearest hit trains the volume voxels (up to
/// gpu::VOLUME_TRAIN_CAP points per frame).
const VOLUME_TRAIN_SHARE: f32 = 1.0 / 8.0;
/// Media voxel training points per frame (pathtrace.wgsl MEDIA_TRAIN_POINTS).
const MEDIA_TRAIN_POINTS: u32 = 4096;
/// The most frames a pixel's history holds while the scene's lighting changes: short enough to follow the change by
/// itself (a real-time denoiser's bound: NRD's default is 30), so no edit has to cut it. Against main's rules (24
/// frames for good in a scene with spark lights, every pixel cut to 16 at a lighting edit, an on / off anti-lag), dE
/// against the offline render or the converged frame: the Bistro slice half a second after its explosion 6.04 for
/// 6.58, at rest 2.82 for 2.79; Kabukichō 2 s and 4 s after its signs close 6.70 and 2.92 for 6.56 and 2.82. At 64
/// frames the explosion read 5.72 and the signs 7.82 and 3.37.
const HISTORY_LIVE: f32 = 32.0;
/// The most once the scene has settled: holding still, the frame converges on the offline render.
const HISTORY_FRAMES: f32 = 4096.0;

/// THREERT_TL_HIDE (diagnostics): the translucency pass trains the volume voxels but draws nothing, so frames show what is
/// behind translucent particles; `odd` hides them on odd frames only (rain on / off pairs from one run).
fn translucent_hidden(frame: u32) -> bool {
    static F: std::sync::OnceLock<Option<bool>> = std::sync::OnceLock::new();
    match F.get_or_init(|| std::env::var("THREERT_TL_HIDE").ok().map(|v| v == "odd")) {
        None => false,
        Some(false) => true,
        Some(true) => frame % 2 == 1,
    }
}

struct Profile {
    queries: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    read: wgpu::Buffer,
    sums: Vec<f64>,
    frames: u32,
    /// Timed apart from the slots above: the grid programs' two passes (`gridsim`, queries 0..4) and the media passes
    /// (`media`, queries 4..6: the camera rays' march and, at one sample a frame, the volume voxels' training, which
    /// are otherwise in the trace's pass). Their queries, the resolve and read buffers, the sums.
    grid: (wgpu::QuerySet, wgpu::Buffer, wgpu::Buffer),
    grid_sum: f64,
    media_sum: f64,
}

impl LiveCore {
    /// Advances the scene time by a frame's dt (the frame script's: at most 0.1 s).
    pub fn advance_time(&mut self, dt: f32) {
        self.time += dt.clamp(0.0, 0.1);
    }

    /// Sets the scene time: a game session's frame script clock (`GameSession::frame`).
    pub fn set_time(&mut self, t: f32) {
        self.time = t;
    }

    /// The scene time animation sees (0 under THREERT_FREEZE_TIME).
    pub fn scene_time(&self) -> f32 {
        shader_time(self.time)
    }

    /// A live view of the engine's world; asset files resolve against `assets`.
    pub fn new(g: gpu::Gpu, world: &World, assets: &Path, presets: &Path, w: u32, h: u32) -> Result<LiveCore> {
        let passes = Passes::new(&g);
        // The scene's post preset replaces this LUT below (apply_post).
        let cache_on = !std::env::var("THREERT_CACHE").is_ok_and(|v| v == "0");
        let restir = std::env::var("THREERT_RESTIR").is_ok_and(|v| v == "1");
        // (Without the cache the kernel binds one of one entry; without ReSTIR, one reservoir.)
        let targets = Targets::new(&g, w, h, &vec![0u16; (LUT * LUT * LUT * 4) as usize], if cache_on { gpu::CACHE_SIZE } else { 1 }, if restir { w as u64 * h as u64 } else { 1 });
        let (sc, pp) = load_scene(&g, world, assets, presets, w, h, &targets, &passes)?;
        let cam = sc.scene_camera;
        let (triangles, scene_name) = (sc.triangles, sc.name.clone());
        let profile = (g.timestamps && std::env::var("THREERT_PROFILE").is_ok()).then(|| {
            // trace, the post passes, cache resolve, then the particle pass, the BLAS rebuild, the translucency pass and
            // the volume training.
            let n = (passes.pipes.len() as u64 + 6) * 2;
            Profile {
                queries: g.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("profile"), ty: wgpu::QueryType::Timestamp, count: n as u32 }),
                resolve: g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: n * 8, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
                read: g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: n * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
                sums: vec![0.0; passes.pipes.len() + 6],
                frames: 0,
                grid: (
                    g.device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("profile grid"), ty: wgpu::QueryType::Timestamp, count: 6 }),
                    g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 2 * wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
                    g.device.create_buffer(&wgpu::BufferDescriptor { label: None, size: 48, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
                ),
                grid_sum: 0.0,
                media_sum: 0.0,
            }
        });
        let timer = (g.timestamps && profile.is_none()).then(|| FrameTimer::new(&g.device));
        let raster = std::env::var("THREERT_RASTER").is_ok_and(|v| v == "1").then(|| Raster::new(&g, w, h));
        let cache = cache_on.then(|| RadianceCache { bufs: targets.kernel.cache.clone(), resolve: g.cache_resolve_pipeline(&targets.kernel) });
        let kernel_bg = gpu::vis_bind_group(&g.device, &g.vis_layout, raster.as_ref().map_or(&g.no_vis_view, |r| &r.vis), &targets.kernel);
        // (Read whenever a kernel of this session may have the guard: under software traversal a scene's first unit
        // brings it, `gpu::loop_guard_of`.)
        let guard_read = (gpu::loop_guard_of(!g.hwrt).is_some() && targets.kernel.guard.is_some())
            .then(|| (g.device.create_buffer(&wgpu::BufferDescriptor { label: Some("loop guard read"), size: 4, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }), Arc::new(AtomicU32::new(0))));
        let mut core = LiveCore { particle_spawns: Default::default(), grid_sources: Default::default(), g, passes, targets, sc, presets: presets.to_path_buf(), assets: assets.to_path_buf(), w, h, seq: 0, aspect: w as f32 / h as f32, cam, prev_cam: cam, frame: 0, history_len: 0.0, reset: true, bloom: ([0.0; 6], [0.0; 6]), vignette: 0.0, post: [0.0; 4], blur: (0.0, 0.0), still: 0, inflight: Default::default(), gpu_ms: 0.0, gpu_frame: 0, gpu_trace_ms: 0.0, edit_ms: 0.0, instanced_ms: 0.0, edit_split: EditSplit::default(), rebuild_ms: 0.0, fallback: None, fallbacks: 0, bounces: 4, rays: true, view_mode: std::env::var("THREERT_VIEW_MODE").ok().and_then(|v| v.parse().ok()).unwrap_or(0), fixed_dt: None, motion_blur_off: false, ghost_fix_off: false, triangles, scene_name, time: 0.0, frame_dt: 1.0 / 60.0, last_frame: Instant::now(), post_key: String::new(), render_scale: 1.0, dynamic_res: false, min_scale: MIN_SCALE, display_scale: 1.0, over_budget: (0, 0), profile, timer, raster, cache, restir, restir_prev: None, kernel_bg, prev_span: None, reposed: false, emitters_moved: false, cold_frames: 0, pose_span: None, pose_reposed: false, inst_moving: vec![], inst_moved: false, prev_isz: (0, 0), lit_changed: 0, spark_lights: false, shown_spp: 1, cpu_skin_ms: 0.0, cpu_upload_ms: 0.0, gpu_blas_ms: 0.0, cpu_wait_ms: 0.0, sky_cover: 0.0, guard_trips: guard_read.as_ref().map(|_| 0), guard_read };
        core.spark_lights = core.sc.tables.emitter_tris.iter().any(|e| e.nominal);
        core.sky_cover = core.sc.tables.cloud_cover;
        core.apply_post(&pp);
        Ok(core)
    }

    pub fn scene_camera(&self) -> Camera {
        self.sc.scene_camera
    }

    /// Applies what scene code changed in the world since the last commit. Parameter changes (power, light / material
    /// parameters, wetness / rain / fog, the camera) rewrite GPU table entries in place; anything structural rebuilds
    /// the GPU scene from the world (history survives, validated per pixel by depth / normal). Returns true on a
    /// rebuild.
    pub fn commit(&mut self, world: &mut World) -> Result<bool> {
        self.cpu_skin_ms = 0.0;
        self.cpu_upload_ms = 0.0;
        self.gpu_blas_ms = 0.0;
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
        // Particle systems shown or hidden by their own `visible`: in place when the build has them (a rain curtain
        // the weather turns off costs nothing while off), else with the rebuild.
        if !changes.structural {
            for id in std::mem::take(&mut changes.particles_shown) {
                if !self.g.set_particles_hidden(&self.sc.up, id, !world.shown(id)) {
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
                let (e, (tex, func, groups)) = (&self.sc.tables, self.sc.fixed_features);
                self.sc.up.kernel.features = gpu::features_of(&e.materials, e.fog.as_ref(), tex, func, groups, self.sc.instance_features) | if e.aerial[0] > 0.0 { gpu::F_AERIAL } else { 0 };
            } else {
                self.fallback = Some("a new material that cannot be added in place (it emits, is glass or a cutout)".into());
                self.fallbacks += 1;
                changes.structural = true;
            }
        }
        // Objects added in place are not in the move tables (their records follow the scene's): a move rebuilds. Not a
        // mesh a body drives: it is an instance, and its record moves as the build's do (`placed_movable`).
        if !changes.structural && changes.moved.iter().any(|id| self.sc.edit.placed.contains_key(id) && self.placed_movable(world, *id).is_none()) {
            self.fallback = Some("an object added in place moved".into());
            self.fallbacks += 1;
            changes.structural = true;
            if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") {
                eprintln!("edit: rebuild (an object added in place moved)");
            }
        }
        // Instanced objects rewrite their instances in place (`update_instanced`); one that cannot rebuilds the scene. Before
        // the geometry edits, whose TLAS build (always, or the rebuild they fall back to) then carries the instances too
        // (a voxel scene's debris and batches: one build of a 20 k instance TLAS a frame, not two); else they build it.
        // (The instances the last update moved, taken first: what this one moves, from here on, keeps its motion.)
        let moved_before = std::mem::take(&mut self.inst_moving);
        let mut instanced_early = false;
        // (Not those added this commit: the geometry edits place them with their instances as they are.)
        let early: Vec<u32> = std::mem::take(&mut changes.instanced).into_iter().filter(|id| !changes.added.contains(id)).collect();
        if !early.is_empty() && !changes.structural {
            instanced_early = self.instanced_in_place(world, &early, !changes.edits(), &mut changes);
        }
        // Geometry edits in place when nothing else is structural; what they cannot apply rebuilds the scene.
        if changes.edits() && !changes.structural {
            let t = Instant::now();
            let r = self.edit_geometry(world, &changes);
            if std::env::var("THREERT_EDIT_LOG").is_ok_and(|v| v == "1") {
                eprintln!("edit: {} geometries, {} added, {} removed: {}", changes.geometries.len(), changes.added.len(), changes.removed.len(), r.as_ref().map_or_else(|e| format!("rebuild ({e})"), |_| "in place".into()));
            }
            match r {
                Ok(()) => {
                    self.edit_ms += t.elapsed().as_secs_f32() * 1e3;
                    // As after a rebuild: history validated per pixel, the reservoirs' triangle indices dropped.
                    self.restir_prev = None;
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
                            let r = &self.sc.tables.retained;
                            match r.instance_of.get(&k).copied().or_else(|| self.placed_movable(world, k)) {
                                Some(i) if !changes.poses.contains(&k) || r.pose_member.contains_key(&k) => placed.push((k, i)),
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
        // The TLAS is built by whatever moves after them, or there.
        let mut instanced_done = instanced_early;
        instanced_moved.retain(|k| !early.contains(k));
        if !instanced_moved.is_empty() && !changes.structural {
            let others = !placed.is_empty() || !dynamic.is_empty() || !procs_moved.is_empty();
            instanced_done |= self.instanced_in_place(world, &instanced_moved, !others, &mut changes);
        }
        // Posed rigid models move their nodes' instances (`repose_rigid`), not triangles.
        let rigid: Vec<u32> = dynamic.iter().copied().filter(|k| self.sc.tables.retained.rigid.contains_key(k)).collect();
        for k in &rigid {
            dynamic.remove(k);
        }
        // Models in a shared pose re-skin their group's mesh (`repose_groups`, its BLAS only); their moves place instances.
        let shared: Vec<u32> = dynamic.iter().copied().filter(|k| self.sc.tables.retained.pose_member.contains_key(k)).collect();
        for k in &shared {
            dynamic.remove(k);
        }
        let shared_ok = shared.is_empty() || (!changes.structural && self.repose_groups(world, &shared)?);
        // Procedural geometry moves its records and TLAS transforms; the TLAS is built by whatever moves after it, or here.
        if !procs_moved.is_empty() && !changes.structural {
            self.move_procs(world, &procs_moved, placed.is_empty() && dynamic.is_empty() && rigid.is_empty() && shared.is_empty());
        }
        let placed_ok = shared_ok && (placed.is_empty() || (!changes.structural && self.move_instances(world, &placed, dynamic.is_empty() && rigid.is_empty())));
        let t_rigid = Instant::now();
        let rigid_ok = placed_ok && (rigid.is_empty() || (!changes.structural && self.repose_rigid(world, &rigid, dynamic.is_empty())?));
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
            catch_up.flush(&self.g.queue, &self.sc.up.instances);
        }
        if rigid_ok && (dynamic.is_empty() || (!changes.structural && self.repose(world, &dynamic)?)) {
            // A new shared pose and nothing else that rebuilds the TLAS (no move): rebuild it for the meshes' new bounds.
            if !shared.is_empty() && placed.is_empty() && rigid.is_empty() && dynamic.is_empty() {
                if let Some(accel) = self.sc.up.accel.as_ref() {
                    let mut e = self.g.device.create_command_encoder(&Default::default());
                    self.g.rebuild_tlas(accel, &mut e);
                    self.g.queue.submit([e.finish()]);
                }
            }
            // Moved emitters: the light tree's moving subtree around them, once (after every in-place update).
            if std::mem::take(&mut self.emitters_moved) {
                self.refit_moving_tree();
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
        let now = shader_time(self.time);
        let applied = self.sc.tables.apply(world, &changes, now).ok().flatten();
        let Some(d) = applied else {
            let t = Instant::now();
            self.rebuild(world)?;
            self.rebuild_ms += t.elapsed().as_secs_f32() * 1e3;
            if changes.camera {
                self.cam = self.sc.scene_camera;
            }
            return Ok(true);
        };
        if d.materials {
            self.upload_materials();
        }
        let (q, up, e) = (&self.g.queue, &self.sc.up, &mut self.sc.tables);
        if d.lights && !e.lights.is_empty() {
            q.write_buffer(&up.lights, 0, bytemuck::cast_slice(&crate::lights_with_groups(&e.lights)));
        }
        if d.objects {
            q.write_buffer(&up.objects, 0, bytemuck::cast_slice(&e.objects));
        }
        if d.materials || d.emitters {
            self.add_material_emitters();
            self.write_emitters();
        } else if d.deferred {
            self.send_powers();
        }
        let (e, b) = (&mut self.sc.tables, &mut self.sc.base);
        if d.environment {
            b.time[1] = e.wetness;
            b.time[2] = e.rain;
            b.background = e.background.map_or([0.0; 4], |c| [c[0], c[1], c[2], 1.0]);
            b.aerial = e.aerial;
            [b.fog, b.fog_color, b.vol, b.vol2] = crate::fog_params(e.fog.as_ref());
        }
        if d.materials || d.environment {
            // e.g. fog appearing needs the fog kernel variant (compiled once, then cached)
            let (tex, func, groups) = self.sc.fixed_features;
            self.sc.up.kernel.features = gpu::features_of(&e.materials, e.fog.as_ref(), tex, func, groups, self.sc.instance_features) | if e.aerial[0] > 0.0 { gpu::F_AERIAL } else { 0 };
        }
        if let Some(c) = e.camera.take() {
            self.sc.scene_camera = Camera { pos: c.position, fwd: c.forward, up: c.up, right: c.right, tan_half_fov: c.tan_half_fov };
            self.sc.tan_half_fov = c.tan_half_fov;
            self.sc.dof = c.dof.clone();
            self.cam = self.sc.scene_camera;
        }
        // Lighting changed: for 45 frames the histories hold HISTORY_LIVE frames at most (`live_lighting`), and where the
        // change is strong the temporal pass cuts the history locally (anti-lag).
        if d.materials || d.lights || d.objects || d.environment {
            self.lit_changed = self.frame.max(1);
        }
        // (A new cloud cover re-bakes the sky and dims the sun as a sun move does.)
        if changes.sun || self.sc.tables.cloud_cover != self.sky_cover {
            self.sky_cover = self.sc.tables.cloud_cover;
            self.move_sun(world);
        }
        if changes.exposure {
            self.apply_exposure(world);
        }
        Ok(false)
    }

    /// The atmosphere's sun moved (time of day): its light entry is rewritten now (direction, and its colour through
    /// the atmosphere), and the sky's re-bake is asked of the worker thread (`DynamicSky`, started here on the first
    /// move). Lockstep frames wait for the bake, so a lockstep run sees the sky that goes with each sun.
    fn move_sun(&mut self, world: &World) {
        let Some((id, dir, lux)) = scene::atmosphere_sun(world) else { return };
        if self.sc.sky.is_none() {
            let (size, alt, floor) = self.sc.sky_inputs;
            self.sc.sky = DynamicSky::new(world, size, alt, floor);
        }
        let Some(sky) = self.sc.sky.as_ref() else { return };
        let cover = self.sc.tables.cloud_cover;
        let colour = crate::sky::sun_illuminance(&sky.atm, dir, lux, cover);
        let e = &mut self.sc.tables;
        if let (Some(idx), Some(o)) = (e.light_index.get(&id), world.objects.get(&id)) {
            if let crate::world::Kind::Light(l) = &o.kind {
                if let Some(mut gl) = scene::gpu_light(l, world.world_matrix(id), o.power) {
                    gl.color = [colour.x, colour.y, colour.z, 0.0];
                    for &i in idx {
                        e.lights[i] = gl;
                    }
                    self.g.queue.write_buffer(&self.sc.up.lights, 0, bytemuck::cast_slice(&crate::lights_with_groups(&e.lights)));
                }
            }
        }
        let _ = sky.ask.send((dir, lux, cover));
        if self.fixed_dt.is_some() {
            if let Ok(b) = sky.done.recv() {
                self.put_sky(b);
            }
        }
        self.lit_changed = self.frame.max(1);
    }

    /// A sky the worker thread has baked since the last frame (the newest), into the environment map and its CDF.
    fn take_sky(&mut self) {
        let Some(sky) = self.sc.sky.as_ref() else { return };
        let mut last = None;
        while let Ok(b) = sky.done.try_recv() {
            last = Some(b);
        }
        if let Some(b) = last {
            self.put_sky(b);
        }
    }

    fn put_sky(&mut self, (env, cdf, max): (gpu::EnvMap, Vec<f32>, f32)) {
        let Some(sky) = self.sc.sky.as_ref() else { return };
        self.g.write_env(&self.sc.up, &env, &cdf);
        // As the load decides it: the sky is light-sampled unless it is negligible next to the emitters (night).
        self.sc.base.counts2[1] = (max >= sky.env_nee_floor) as u32;
        self.lit_changed = self.frame.max(1);
    }

    /// The camera's manual exposure changed value (an exposure that follows the time of day), in place: the post pass's
    /// exposure and the kernel's firefly clamp, which is relative to it.
    fn apply_exposure(&mut self, world: &World) {
        let Some(cid) = world.settings.camera else { return };
        let Some(crate::world::Kind::Camera(c)) = world.objects.get(&cid).map(|o| &o.kind) else { return };
        let crate::desc::Exposure::Manual { ev100 } = c.exposure else { return };
        let e = 2f32.powf(-ev100);
        if self.sc.exposure > 0.0 {
            self.sc.base.misc[0] *= self.sc.exposure / e;
        }
        self.sc.exposure = e;
        self.sc.exposure_settings.ae2 = [e, 0.0, 0.0, 0.0];
    }

    /// Moves movable instances: their kernel records (the old transform kept as the previous frame's, for motion
    /// vectors), their object entries' pivots and their TLAS transforms (hardware traversal; with `build_tlas` it
    /// rebuilds the TLAS, otherwise the dynamic rebuild that follows does) or their units and the top tree over them
    /// (software traversal, units.rs). No triangles change. False when it cannot (software traversal without units,
    /// or an instance that is no unit; a top tree that cannot be made): the caller rebuilds.
    fn move_instances(&mut self, world: &World, placed: &[(u32, u32)], build_tlas: bool) -> bool {
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
            // (A mesh an edit added in place keeps its instance with the edit's, `placed_movable`; its record and TLAS
            // instance are where the build's are.)
            let inst = match self.sc.edit.added.get_mut(&node) {
                Some(Picked::Mesh(inst)) => inst,
                _ => &mut self.sc.instances[i as usize],
            };
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
                q.write_buffer(&self.sc.up.instances, i as u64 * size, bytemuck::bytes_of(&r));
            }
            let pivot = w.transform_point3(glam::Vec3::ZERO);
            let st = self.sc.tables.stride;
            let o = &mut self.sc.tables.objects[inst.object as usize * st];
            (o[0], o[1], o[2]) = (pivot.x, pivot.y, pivot.z);
            // Without motion vectors (THREERT_NO_MV=1) its surfaces keep a short history, as moved triangles do.
            if !motion_vectors() {
                self.sc.tables.objects[inst.object as usize * st + 1][3] = shader_time(self.time);
            }
            let slot = top.inst_base() + i as usize;
            top.moved(slot, w, None);
            self.inst_moving.push(i);
        }
        if merged {
            q.write_buffer(&self.sc.up.instances, lo as u64 * size, bytemuck::cast_slice(&self.sc.gpu_instances[lo..=hi]));
        }
        q.write_buffer(&self.sc.up.objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
        self.inst_moved = true;
        match top {
            gpu::Top::Hw(accel) => {
                if build_tlas {
                    let mut e = self.g.device.create_command_encoder(&Default::default());
                    self.g.rebuild_tlas(accel, &mut e);
                    q.submit([e.finish()]);
                }
                true
            }
            // (The top tree is built here whoever else changes units this commit: it is CPU work and one write, and a
            // tree that cannot be made must send the caller to a rebuild now.)
            gpu::Top::Sw(units) => self.g.retop(units, &self.sc.up.bg_parts.nodes).map_err(|e| eprintln!("units: {e}: rebuilding the scene")).is_ok(),
        }
    }

    /// The material table to the GPU: written in place, or, with more materials than its buffer holds (materials added
    /// while the game runs), into a buffer twice the size, the scene's bind group made over it.
    fn upload_materials(&mut self) {
        let bytes: &[u8] = bytemuck::cast_slice(&self.sc.tables.materials);
        if bytes.len() as u64 > self.sc.up.materials.size() {
            let usage = self.sc.up.materials.usage();
            self.sc.up.materials = self.g.device.create_buffer(&wgpu::BufferDescriptor { label: Some("materials"), size: (bytes.len() as u64 * 2).next_multiple_of(16), usage, mapped_at_creation: false });
            self.g.rebind_scene(&mut self.sc.up);
        }
        self.g.queue.write_buffer(&self.sc.up.materials, 0, bytes);
    }

    /// The record of a mesh an edit added in place that moves as an instance does (`move_instances`): one a rigid body
    /// drives, placed by its node's transform (an emitter mesh's triangles are in world space: it cannot).
    fn placed_movable(&self, world: &World, id: u32) -> Option<u32> {
        let &(_, record) = self.sc.edit.placed.get(&id)?;
        let mesh = matches!(self.sc.edit.added.get(&id), Some(Picked::Mesh(_)));
        (mesh && world.objects.get(&id).is_some_and(|o| o.driven) && !self.sc.tables.retained.emitter_mesh.contains_key(&id)).then_some(record as u32)
    }

    /// `update_instanced` with its fallback (the reason in the frame log, the scene rebuilt) and its CPU time; whether it
    /// changed anything.
    fn instanced_in_place(&mut self, world: &World, ids: &[u32], build_tlas: bool, changes: &mut crate::world::Changes) -> bool {
        let t = Instant::now();
        let r = self.update_instanced(world, ids, build_tlas);
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
    /// tree's moving emitters; ROADMAP § 6 step 3). `write_emitters` uploads them.
    fn add_material_emitters(&mut self) {
        let sc = &mut self.sc;
        for mat in std::mem::take(&mut sc.tables.started_emitting) {
            for &[first, end, obj] in sc.mat_runs.get(&mat).map_or(&[][..], |v| v.as_slice()) {
                for k in first..end {
                    let Some(&[a, b, c]) = sc.tri_order.get(k as usize).and_then(|&i| sc.tris.get(i as usize)) else { continue };
                    let area = 0.5 * (b - a).cross(c - a).length();
                    if area > 0.0 {
                        let centre = (a + b + c) / 3.0;
                        sc.tables.emitter_tris.push(crate::EmitterTri { tri: k, mat, obj, area, c: centre.to_array(), moves: false, nominal: false, in_tree: false });
                    }
                }
            }
        }
    }

    /// The emitter CDF from `Tables::emitter_tris` (weights from materials and powers; entries added in place at its
    /// end, removed ones at area 0), the light tree refit to the new weights and its nodes after the CDF, the CDF's
    /// share of power, its count. With the tree's slot for emitters added in place (scenes that can edit geometry), their
    /// subtree is grown again first and their entries go right after the static ones (ROADMAP § 6 step 4); without it
    /// they stay at the end, by power share. The buffer grows (and the scene bind group is made again) when entries
    /// were added past it: a scene that adds none keeps its layout.
    fn write_emitters(&mut self) {
        self.land_powers_now();
        let g = &self.g;
        let LoadedScene { tables: e, base: b, up, pool_tris, tris: flat, tri_order, edit, .. } = &mut self.sc;
        // Emitters added in place, with the tree's slot for them (`ltree::build_slotted`): its subtree grown again over
        // those alive, their entries right after the static ones in the CDF (in its leaf order: each leaf's entry is its
        // position) and their path codes in their triangles' attrs (written over the code 0 an edit gave them).
        let mut perm: Option<Vec<usize>> = None;
        if let Some(t) = e.light_tree.as_mut() {
            if let Some((_, ns)) = t.dynamic {
                let ns = ns as usize;
                let (mats, objs, st, pf) = (&e.materials, &e.objects, e.stride, edit.pool_first);
                let verts = |k: usize| if k >= pf { pool_tris.get(k - pf).copied() } else { tri_order.get(k).and_then(|&i| flat.get(i as usize)).copied() };
                let mut items = vec![];
                for (i, x) in e.emitter_tris.iter().enumerate().skip(ns) {
                    let w = crate::emitter_power(x, mats, objs, st);
                    let Some([va, vb, vc]) = verts(x.tri as usize).filter(|_| !x.in_tree && !x.nominal && !x.moves && w > 0.0) else { continue };
                    items.push(crate::ltree::DynItem { e: i as u32, lo: va.min(vb).min(vc).to_array(), hi: va.max(vb).max(vc).to_array(), c: x.c, w });
                }
                t.set_dynamic(items);
                let at = std::mem::offset_of!(scene::GpuTriAttr, x) + 12;
                let mut codes = Gather::default();
                let mut placed = vec![false; e.emitter_tris.len()];
                let mut p: Vec<usize> = (0..ns).collect();
                for k in ns..t.order.len() {
                    let i = t.order[k] as usize;
                    codes.put((e.emitter_tris[i].tri as usize * std::mem::size_of::<scene::GpuTriAttr>() + at) as u64, bytemuck::bytes_of(&t.code[k]));
                    placed[i] = true;
                    p.push(i);
                }
                codes.flush(&g.queue, &up.attrs);
                p.extend((ns..e.emitter_tris.len()).filter(|&i| !placed[i]));
                perm = Some(p);
            }
        }
        let permuted: Vec<crate::EmitterTri>;
        let list = match &perm {
            Some(p) => {
                permuted = p.iter().map(|&i| e.emitter_tris[i]).collect();
                &permuted[..]
            }
            None => &e.emitter_tris[..],
        };
        let (em, total, power) = crate::emitter_cdf(list, &e.materials, &e.objects, e.stride, b.light_lod, crate::EmitterCull::of(b));
        if let Some(t) = e.light_tree.as_mut() {
            let (tris, mats, objs, st) = (&e.emitter_tris, &e.materials, &e.objects, e.stride);
            let w: Vec<f32> = t.order.iter().map(|&i| crate::emitter_power(&tris[i as usize], mats, objs, st)).collect();
            t.refit(&|k| w[k]);
        }
        self.put_emitters(&em, total, power);
        // Emitters added in place since the worker was made: another, over them.
        if self.sc.emit.as_ref().is_some_and(|w| w.count != self.sc.tables.emitter_tris.len()) {
            self.sc.emit = EmitterWorker::new(&self.sc.tables, &self.g.device);
            self.sc.tables.defer_powers = self.sc.emit.is_some();
        }
    }

    /// The emitter CDF `em` (its total weight and the emitters' power), then the light tree's nodes, into the emitters
    /// buffer (grown, and the scene bind group made again, when they no longer fit), and the kernel's parameters.
    fn put_emitters(&mut self, em: &[[f32; 4]], total: f32, power: f32) {
        let g = &self.g;
        let LoadedScene { tables: e, base: b, up, .. } = &mut self.sc;
        let nodes: &[[f32; 4]] = e.light_tree.as_ref().map_or(&[], |t| &t.nodes);
        let need = ((em.len() + nodes.len()) * 16) as u64;
        if need > up.emitters.size() {
            // (The usage it was made with: the merged layout copies the tables each frame, `Gpu::sync_tables`.)
            let usage = up.emitters.usage();
            up.emitters = g.device.create_buffer(&wgpu::BufferDescriptor { label: Some("emitters"), size: need + need / 2, usage, mapped_at_creation: false });
            g.rebind_scene(up);
        }
        if !em.is_empty() {
            g.queue.write_buffer(&up.emitters, 0, bytemuck::cast_slice(em));
        }
        if let Some(t) = e.light_tree.as_ref() {
            g.queue.write_buffer(&up.emitters, em.len() as u64 * 16, bytemuck::cast_slice(&t.nodes));
        }
        let _ = b;
        self.emitter_params(em.len(), total, power);
    }

    /// The kernel's emitter parameters for a CDF of `entries` (its total weight; the emitters' power) and the tree.
    fn emitter_params(&mut self, entries: usize, total: f32, power: f32) {
        let LoadedScene { tables: e, base: b, .. } = &mut self.sc;
        b.emit_power[0] = power;
        b.misc[1] = total;
        if let Some(t) = e.light_tree.as_ref() {
            b.ltree[0] = f32::from_bits(entries as u32);
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
    fn send_powers(&mut self) {
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
            self.g.queue.write_buffer(&self.sc.up.objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
            self.write_emitters();
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
    fn take_emitters(&mut self) {
        let Some(r) = self.sc.emit.as_ref().and_then(|w| w.done.try_recv().ok()) else { return };
        self.land_emitters(r);
        self.send_powers();
    }

    /// A job's edits and its emitter CDF and tree powers, together: the objects' powers, the CDF, the tree.
    fn land_emitters(&mut self, r: EmitterBuilt) {
        let Some(powers) = self.sc.emit.as_mut().and_then(|w| w.flight.take()) else { return };
        let e = &mut self.sc.tables;
        e.put_powers(&powers);
        self.g.queue.write_buffer(&self.sc.up.objects, 0, bytemuck::cast_slice(&e.objects));
        if let Some(t) = e.light_tree.as_mut() {
            for (n, &p) in t.nodes.iter_mut().zip(&r.node_powers) {
                n[2] = p;
            }
            t.power = t.nodes.first().map_or(0.0, |n| n[2]);
        }
        // (Same entries and nodes as the CDF the scene was loaded with: the buffer holds them.)
        let mut enc = self.g.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("emitters") });
        enc.copy_buffer_to_buffer(&r.staging, 0, &self.sc.up.emitters, 0, r.bytes.min(self.sc.up.emitters.size()));
        self.g.queue.submit([enc.finish()]);
        self.emitter_params(r.entries, r.total, r.power);
        self.lit_changed = self.frame.max(1);
    }

    /// Before another emitter edit (`write_emitters`, a light-LOD rebuild): the job in flight waited for and its result
    /// dropped, its edits and those held back written into the objects (and uploaded), so the rebuild sees them all.
    fn land_powers_now(&mut self) {
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
            self.g.queue.write_buffer(&self.sc.up.objects, 0, bytemuck::cast_slice(&e.objects));
            // The lighting changed (as when a result lands): `live_lighting` for the next 45 frames.
            self.lit_changed = self.frame.max(1);
        }
    }

    /// Instanced objects whose instances changed (`World::instanced`, the node's transform, power or customData): their
    /// kernel records (the old transforms kept as the previous frame's, for motion vectors), object entries (pivot,
    /// bounds, power, data, colour), TLAS transforms and masks (`scene::instance_world`: a hidden instance 0), each
    /// object's records and entries written as one range; with `build_tlas` the TLAS is rebuilt here. No triangles or
    /// BLASes change. Returns how many objects it updated; Err: why it cannot (software traversal, an object the build
    /// flattened or did not place this way, an instance now mirrored), and the caller rebuilds.
    /// `THREERT_EDIT_REBUILD=1` always rebuilds (A/B).
    fn update_instanced(&mut self, world: &World, ids: &[u32], build_tlas: bool) -> std::result::Result<usize, String> {
        use crate::world::Kind;
        if std::env::var("THREERT_EDIT_REBUILD").is_ok_and(|v| v == "1") {
            return Err("THREERT_EDIT_REBUILD=1".into());
        }
        let size = std::mem::size_of::<scene::GpuInstance>();
        let (now, mv) = (shader_time(self.time), motion_vectors());
        let empty = crate::world::InstanceTable::default();
        let sc = &mut self.sc;
        let Some(mut top) = gpu::Top::of(&mut sc.up.accel, &mut sc.up.units) else { return Err("software traversal".into()) };
        let mut done = 0;
        for &id in ids {
            // (Removed or hidden: nothing is drawn; a structural change rebuilds.)
            let Some(n) = world.objects.get(&id).filter(|_| world.shown(id)) else { continue };
            let Kind::Instanced { capacity, procedural, .. } = n.kind else { continue };
            let Some(&place) = sc.tables.retained.instanced.get(&id) else { return Err("an instanced mesh the build flattened".into()) };
            if place.capacity != capacity || place.procedural != procedural {
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
            let (rec0, slot0) = if procedural { (sc.proc_first + first, top.proc_base(sc.proc_first) + first) } else { (first, top.inst_base() + first) };
            for i in 0..cap {
                let (wm, hidden) = scene::instance_world(m, t, i);
                if !procedural && !hidden && wm.determinant() < 0.0 {
                    return Err("an instance is mirrored".into());
                }
                let old = sc.gpu_instances[rec0 + i];
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
                    let (lo, hi) = sc.meshes[inst.mesh as usize].placed_bounds(wm);
                    (top.record(inst.gpu(old.info[0], old.info[3]), inst.mesh, n.cast_shadow), scene::object_row(wm.transform_point3(glam::Vec3::ZERO), n.power, hi - lo, t.custom(i, n.custom), t.color(i)))
                };
                rec.p = old.m;
                sc.gpu_instances[rec0 + i] = rec;
                // The time of the last power edit: now for a new power, or (without motion vectors, THREERT_NO_MV=1)
                // for every move, as `move_instances` does.
                let st = sc.tables.stride;
                let at = (place.object as usize + i) * st;
                let e = &mut sc.tables.objects[at..at + st];
                let edited = if !mv || e[0][3] != entry[0][3] { now } else { e[1][3] };
                e.copy_from_slice(&entry[..st]);
                e[1][3] = edited;
                top.moved(slot0 + i, wm, Some((n.cast_shadow, hidden)));
            }
            let q = &self.g.queue;
            q.write_buffer(&sc.up.instances, (rec0 * size) as u64, bytemuck::cast_slice(&sc.gpu_instances[rec0..rec0 + cap]));
            let st = sc.tables.stride;
            let at = place.object as usize * st;
            q.write_buffer(&sc.up.objects, (at * 16) as u64, bytemuck::cast_slice(&sc.tables.objects[at..at + cap * st]));
            self.inst_moving.extend(rec0 as u32..(rec0 + cap) as u32);
            sc.tables.retained.instanced.get_mut(&id).unwrap().drawn = t.count;
            done += 1;
        }
        if done > 0 {
            self.inst_moved = true;
            match top {
                gpu::Top::Hw(accel) => {
                    if build_tlas {
                        let mut e = self.g.device.create_command_encoder(&Default::default());
                        self.g.rebuild_tlas(accel, &mut e);
                        self.g.queue.submit([e.finish()]);
                    }
                }
                gpu::Top::Sw(units) => self.g.retop(units, &sc.up.bg_parts.nodes)?,
            }
        }
        Ok(done)
    }

    /// Moves procedural objects: their kernel records (the old transform kept as the previous frame's, for motion
    /// vectors), object pivots and, with hardware RT, TLAS transforms (`build_tlas`: the TLAS rebuilt here). Software
    /// traversal reads the records alone, so neither path rebuilds the scene.
    fn move_procs(&mut self, world: &World, ids: &[u32], build_tlas: bool) {
        let size = std::mem::size_of::<scene::GpuInstance>() as u64;
        for id in ids {
            let Some(&j) = self.sc.proc_of.get(id) else { continue };
            let w = world.world_matrix(*id);
            let p = &mut self.sc.procs[j];
            p.world = w;
            let i = self.sc.proc_first + j;
            let old = self.sc.gpu_instances[i];
            let mut r = p.gpu(old.info[0]);
            r.p = old.m;
            self.sc.gpu_instances[i] = r;
            self.g.queue.write_buffer(&self.sc.up.instances, i as u64 * size, bytemuck::bytes_of(&r));
            let pivot = w.transform_point3(glam::Vec3::ZERO);
            let st = self.sc.tables.stride;
            let o = &mut self.sc.tables.objects[p.object as usize * st];
            (o[0], o[1], o[2]) = (pivot.x, pivot.y, pivot.z);
            if !motion_vectors() {
                self.sc.tables.objects[p.object as usize * st + 1][3] = shader_time(self.time);
            }
            if let Some(accel) = self.sc.up.accel.as_mut() {
                if let Some(t) = accel.tlas[accel.proc_base + j].as_mut() {
                    t.transform = gpu::tlas_rows(w);
                }
            }
            self.inst_moving.push(i as u32);
        }
        self.g.queue.write_buffer(&self.sc.up.objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
        if let (true, Some(accel)) = (build_tlas, self.sc.up.accel.as_ref()) {
            let mut e = self.g.device.create_command_encoder(&Default::default());
            self.g.rebuild_tlas(accel, &mut e);
            self.g.queue.submit([e.finish()]);
        }
        self.inst_moved = true;
    }

    /// Posed rigid models (`scene::RigidModel`): each moving node's instance takes object x node transform from the clip
    /// (previous rows kept for motion vectors) and their flattened triangles are re-transformed in place; then the dynamic
    /// BLASes (if they have such triangles) and, when `build_tlas`, the TLAS are rebuilt (`repose` rebuilds both after
    /// it otherwise). False when it cannot (no hardware RT, a model hidden or changed): the caller rebuilds.
    fn repose_rigid(&mut self, world: &World, ids: &[u32], build_tlas: bool) -> Result<bool> {
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
            let r = &self.sc.tables.retained.rigid[id];
            placements.extend(r.instances.iter().map(|&(node, i)| (i, m * globals[node])));
            let d = &r.rest;
            if d.finals.len() != d.mats.len() {
                return Ok(false);
            }
            let (first, verts, attrs, blas) = &mut self.sc.dynamic;
            for (j, (node, p, n, uv, tg)) in r.rest_local.iter().enumerate() {
                let w = m * globals[*node];
                let nm = glam::Mat3::from_mat4(w).inverse().transpose();
                let pw = p.map(|v| w.transform_point3(v));
                let nw = n.map(|v| (nm * v).normalize_or_zero());
                let tw = tg.map(|(v, s)| (w.transform_vector3(v).normalize_or_zero(), s));
                let a = scene::tri_attr(pw, nw, *uv, tw, d.mats[j]);
                let k = d.finals[j] as usize - *first;
                let (e1, e2) = (pw[1] - pw[0], pw[2] - pw[0]);
                verts[k * 3..k * 3 + 3].copy_from_slice(&[[pw[0].x, pw[0].y, pw[0].z, f32::from_bits(d.flags[j])], [e1.x, e1.y, e1.z, 0.0], [e2.x, e2.y, e2.z, 0.0]]);
                attrs[k] = scene::GpuTriAttr { x: [a.x[0], f32::from_bits(d.object), a.x[2], attrs[k].x[3]], ..a };
                blas[k * 3..k * 3 + 3].copy_from_slice(&pw.map(|v| v.to_array()));
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
            self.g.queue.write_buffer(&self.sc.up.instances, ilo as u64 * size, bytemuck::cast_slice(&self.sc.gpu_instances[ilo..ihi]));
        }
        self.inst_moved = true;
        self.g.queue.write_buffer(&self.sc.up.objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
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
        if build_tlas {
            let accel = self.sc.up.accel.as_ref().unwrap();
            let mut e = self.g.device.create_command_encoder(&Default::default());
            if lo < hi {
                self.g.rebuild_dynamic(accel, &mut e);
            } else {
                self.g.rebuild_tlas(accel, &mut e);
            }
            self.g.queue.submit([e.finish()]);
        }
        Ok(true)
    }

    /// Models in a shared pose (`scene::PoseGroup`): each group whose pose changed is skinned once, in model space (the
    /// groups in parallel), into its mesh's pool triangles, and their BLASes are rebuilt; the TLAS is left to the moves
    /// that follow (or `commit`). False when a group's members no longer share one pose, or one is hidden or changed:
    /// the caller rebuilds, which groups them again. (Deforming meshes get no motion vectors of their own yet: an
    /// instance's are its transform's.)
    fn repose_groups(&mut self, world: &World, members: &[u32]) -> Result<bool> {
        use rayon::prelude::*;
        if self.sc.up.accel.is_none() {
            return Ok(false);
        }
        let r = &self.sc.tables.retained;
        let mut touched: Vec<u32> = members.iter().map(|k| r.pose_member[k]).collect();
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
                    b.extend(q.p.map(|p| p.to_array()));
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
        let mut e = self.g.device.create_command_encoder(&Default::default());
        self.g.rebuild_meshes(accel, &meshes, &mut e);
        let profile_blas = std::env::var("THREERT_PROFILE_BLAS").is_ok_and(|v| v == "1");
        if profile_blas {
            let _ = self.g.device.poll(wgpu::PollType::wait_indefinitely());
        }
        let t_blas = Instant::now();
        q.submit([e.finish()]);
        if profile_blas {
            let _ = self.g.device.poll(wgpu::PollType::wait_indefinitely());
            self.gpu_blas_ms += t_blas.elapsed().as_secs_f32() * 1000.0;
        }
        self.cpu_upload_ms += t_up.elapsed().as_secs_f32() * 1000.0;
        Ok(true)
    }

    /// Re-skins posed models and re-transforms movable objects in place (hardware RT): their triangles, shading
    /// attributes and dynamic BLASes, then the TLAS. Their directly seen surfaces keep a short history for a moment,
    /// like a power edit (no per-object motion vectors). False when it cannot (not dynamic at build, no hardware RT, a
    /// different triangle count): the caller rebuilds.
    fn repose(&mut self, world: &World, ids: &std::collections::HashSet<u32>) -> Result<bool> {
        if self.sc.up.accel.is_none() || ids.iter().any(|id| !self.sc.tables.retained.dynamic.contains_key(id)) {
            return Ok(false);
        }
        let now = shader_time(self.time);
        let models = std::mem::take(&mut self.sc.tables.retained.dynamic);
        let t_skin = Instant::now();
        let res = self.reskin(world, ids, &models, now);
        self.cpu_skin_ms += t_skin.elapsed().as_secs_f32() * 1000.0;
        self.sc.tables.retained.dynamic = models;
        let Some((lo, hi)) = res? else { return Ok(false) };
        self.emitters_moved = true;
        let t_up = Instant::now();
        // Motion vectors: the GPU still holds the previous frame's vertices for this span (and for last update's span,
        // if different): keep them as `prev_verts` before uploading the new ones.
        let both = self.prev_span.map_or((lo, hi), |(a, b)| (a.min(lo), b.max(hi)));
        self.keep_prev(both);
        self.prev_span = Some((lo, hi));
        self.reposed = true;
        // Upload the touched span of the dynamic tail, then rebuild the dynamic BLASes and the TLAS.
        let (q, up, (first, verts, attrs, blas)) = (&self.g.queue, &self.sc.up, &self.sc.dynamic);
        let accel = up.accel.as_ref().unwrap();
        let k = (first + lo) as u64;
        q.write_buffer(&up.verts, k * 48, bytemuck::cast_slice(&verts[lo * 3..hi * 3]));
        q.write_buffer(&up.attrs, k * 96, bytemuck::cast_slice(&attrs[lo..hi]));
        q.write_buffer(&accel.verts, k * 36, bytemuck::cast_slice(&blas[lo * 3..hi * 3]));
        q.write_buffer(&up.objects, 0, bytemuck::cast_slice(&self.sc.tables.objects));
        let mut e = self.g.device.create_command_encoder(&Default::default());
        self.g.rebuild_dynamic(accel, &mut e);
        let profile_blas = std::env::var("THREERT_PROFILE_BLAS").is_ok_and(|v| v == "1");
        if profile_blas {
            let _ = self.g.device.poll(wgpu::PollType::wait_indefinitely());
        }
        let t_blas = Instant::now();
        q.submit([e.finish()]);
        if profile_blas {
            let _ = self.g.device.poll(wgpu::PollType::wait_indefinitely());
            self.gpu_blas_ms += t_blas.elapsed().as_secs_f32() * 1000.0;
        }
        self.cpu_upload_ms += t_up.elapsed().as_secs_f32() * 1000.0;
        Ok(true)
    }

    /// `guard_trips` for the frame log: None while the scene's kernel has no guard. Natively with nothing set that is
    /// a scene without units (`gpu::loop_guard_of`): under software traversal the first unit brings the guard.
    pub fn guard_count(&self) -> Option<u32> {
        self.guard_trips.filter(|_| gpu::loop_guard_of(self.sc.up.kernel.features & gpu::F_UNITS != 0).is_some())
    }

    /// The edit room as the frame log reports it (`room`): triangles, records and object entries in use and held (GPU
    /// order counts, holes not in use), its growths, and the GPU bytes of the buffers edits write and of the triangle
    /// BLASes the live meshes need (estimated from the build's bytes per triangle), MB.
    pub fn room_stats(&self) -> serde_json::Value {
        let (e, up) = (&self.sc.edit, &self.sc.up);
        // Software traversal's (it does not grow): the same counts, and of the node buffer the mesh trees' range (in
        // use, held) and the units the top tree has against the most its range holds.
        if let Some(u) = up.units.as_ref() {
            return serde_json::json!({
                "tris": [e.tris.used(), e.tris.cap], "records": [e.records.used(), e.records.cap], "objects": [e.objects.used(), e.objects.cap],
                "holes": e.tris.free.len(), "grows": e.grows, "nodes": [u.layout.mesh_cap as usize - u.layout.free_nodes(), u.layout.mesh_cap], "units": [u.count, u.layout.max_units()],
            });
        }
        let Some(a) = up.accel.as_ref() else { return serde_json::Value::Null };
        let buffers = [&up.verts, &up.attrs, &up.prev_verts, &up.instances, &up.objects, &up.cdf, &up.emitters, &a.verts].iter().map(|b| b.size()).sum::<u64>();
        let live: u64 = self.sc.meshes.iter().enumerate().filter(|(k, _)| e.users.get(&(*k as u32)).copied().unwrap_or(0) > 0).map(|(_, m)| m.count as u64).sum::<u64>() + e.pool_first as u64;
        let mb = |b: f64| (b / (1 << 20) as f64 * 10.0).round() / 10.0;
        serde_json::json!({
            "tris": [e.tris.used(), e.tris.cap], "records": [e.records.used(), e.records.cap], "objects": [e.objects.used(), e.objects.cap],
            "holes": e.tris.free.len(), "grows": e.grows, "buffers_mb": mb(buffers as f64), "blas_mb": mb(live as f64 * a.blas_per_tri),
        })
    }

    /// Software traversal's top tree so far (the frame log's `top`): how many times it was built, kept with leaves
    /// taken out and put in, and kept with its boxes brought up to date, and the ms all of that took. Null without
    /// units.
    pub fn top_stats(&self) -> serde_json::Value {
        self.sc.up.units.as_ref().map_or(serde_json::Value::Null, |u| serde_json::json!([u.kept.builds, u.kept.changes, u.kept.refits, (u.kept.ms * 1e3).round() / 1e3]))
    }

    /// now (the CPU copy of the dynamic tail), its nodes and the root uploaded (`ltree::LightTree::refit_moving`).
    fn refit_moving_tree(&mut self) {
        let first = self.sc.dynamic.0;
        let (verts, tables) = (&self.sc.dynamic.1, &mut self.sc.tables);
        let tris = &tables.emitter_tris;
        let Some(t) = tables.light_tree.as_mut().filter(|t| t.moving_count > 0) else { return };
        t.refit_moving(&|k| {
            let r = (tris[k].tri as usize - first) * 3;
            crate::ltree::tri_vertices(&verts[r..r + 3])
        });
        let (q, up, off) = (&self.g.queue, &self.sc.up, self.sc.base.ltree[0].to_bits() as u64);
        q.write_buffer(&up.emitters, off * 16, bytemuck::cast_slice(&t.nodes[..1]));
        q.write_buffer(&up.emitters, (off + 2) * 16, bytemuck::cast_slice(&t.nodes[2..3]));
        q.write_buffer(&up.emitters, (off + t.moving_nodes as u64) * 16, bytemuck::cast_slice(&t.nodes[t.moving_nodes..]));
    }

    /// Profile slots after the cache resolve that this scene writes: particle pass + BLAS rebuild (2; stateless or GPU
    /// particles), translucency + volume training (2 more).
    fn profile_extra(&self) -> u32 {
        if self.sc.up.translucent.is_some() {
            4
        } else if self.sc.up.particles.is_some() || self.sc.up.gpu_particles.is_some() {
            2
        } else {
            0
        }
    }

    /// Copies the GPU's current vertices of a dynamic-tail span into `prev_verts` (submitted before any write queued
    /// after it, so it reads what the last frame traced).
    fn keep_prev(&self, (lo, hi): (usize, usize)) {
        let up = &self.sc.up;
        let first = self.sc.dynamic.0;
        let mut e = self.g.device.create_command_encoder(&Default::default());
        e.copy_buffer_to_buffer(&up.verts, ((first + lo) * 48) as u64, &up.prev_verts, (lo * 48) as u64, ((hi - lo) * 48) as u64);
        self.g.queue.submit([e.finish()]);
    }

    /// Skins / transforms the objects into the CPU copy of the dynamic tail; the touched span (relative to the tail), or
    /// None when one cannot be updated in place.
    fn reskin(&mut self, world: &World, ids: &std::collections::HashSet<u32>, models: &std::collections::HashMap<u32, scene::DynamicModel>, now: f32) -> Result<Option<(usize, usize)>> {
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
                    b[j * 3..j * 3 + 3].copy_from_slice(&p.map(|q| q.to_array()));
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
                blas[k * 3..k * 3 + 3].copy_from_slice(&p.map(|v| v.to_array()));
                sc.tris[(d.first + j as u32) as usize] = p.map(glam::Vec3A::from);
                (lo, hi) = (lo.min(k), hi.max(k + 1));
            }
        }
        for id in ids {
            let d = &models[id];
            // Pivot (material functions' object position). Motion vectors carry its history; without them
            // (THREERT_NO_MV=1) its surfaces keep a short history for a moment, like a power edit.
            let pivot = world.world_matrix(*id).transform_point3(glam::Vec3::ZERO);
            let st = sc.tables.stride;
            let o = &mut sc.tables.objects[d.object as usize * st];
            (o[0], o[1], o[2]) = (pivot.x, pivot.y, pivot.z);
            if !motion_vectors() {
                sc.tables.objects[d.object as usize * st + 1][3] = now;
            }
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
    fn edit_geometry(&mut self, world: &World, ch: &crate::world::Changes) -> std::result::Result<(), String> {
        use crate::world::Kind;
        if std::env::var("THREERT_EDIT_REBUILD").is_ok_and(|v| v == "1") {
            return Err("THREERT_EDIT_REBUILD=1".into());
        }
        let LoadedScene { up, tables, edit, meshes, pool_tris, instances, gpu_instances, procs, proc_of, proc_first, .. } = &mut self.sc;
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
        let (inst_base, proc_base, proc_first) = (top.inst_base(), top.proc_base(*proc_first), *proc_first);
        let slot_of = |id: u32, edit: &EditState, tables: &tables::Tables, proc_of: &std::collections::HashMap<u32, usize>| -> Option<(usize, usize)> {
            if let Some(&v) = edit.placed.get(&id) {
                Some(v)
            } else if let Some(&k) = tables.retained.own_instance.get(&id) {
                Some((inst_base + k as usize, k as usize))
            } else {
                proc_of.get(&id).map(|&j| (proc_base + j, proc_first + j))
            }
        };
        let q = &g.queue;
        let mut e = g.device.create_command_encoder(&Default::default());
        // An emitter mesh's entries go (area 0: their weight; the light tree keeps its shape) when it is removed, hidden
        // or placed again.
        let mut emitters_changed = false;
        let release = |id: u32, tables: &mut tables::Tables, meshes: &[scene::InstMesh], pool_first: usize| -> bool {
            let Some(k) = tables.retained.emitter_mesh.remove(&id) else { return false };
            let im = meshes[k as usize];
            let (lo, hi) = ((pool_first + im.first as usize) as u32, (pool_first + (im.first + im.count) as usize) as u32);
            for e in tables.emitter_tris.iter_mut().filter(|e| (lo..hi).contains(&e.tri)) {
                e.area = 0.0;
            }
            true
        };
        for id in &ch.removed {
            // An instanced mesh an edit added: its instances, records and entries.
            if let Some((r0, cap, o0)) = edit.instanced.remove(id) {
                for r in r0..r0 + cap {
                    top.clear(inst_base + r);
                    if let Some(i) = instances.get_mut(r) {
                        i.hidden = true;
                    }
                }
                edit.records.give(r0, cap);
                edit.objects.give(o0, cap);
                tables.retained.instanced.remove(id);
                tables.retained.objects.remove(id);
                if let Some(k) = edit.mesh_of.remove(id) {
                    drop_mesh_user(k, edit, tables, meshes);
                }
                continue;
            }
            // An instanced mesh the build placed: its instances out (records and entries stay until the next rebuild).
            if let Some(place) = tables.retained.instanced.get(id).filter(|p| !p.procedural) {
                for r in place.first as usize..(place.first + place.capacity) as usize {
                    top.clear(inst_base + r);
                    if let Some(i) = instances.get_mut(r) {
                        i.hidden = true;
                    }
                }
                tables.retained.instanced.remove(id);
                tables.retained.objects.remove(id);
                continue;
            }
            emitters_changed |= release(*id, tables, meshes, edit.pool_first);
            let at = slot_of(*id, edit, tables, proc_of);
            // Placed by the build otherwise (a model, a procedural instanced mesh): a rebuild takes it out.
            if at.is_none() && tables.retained.objects.contains_key(id) {
                return Err("an object the build placed otherwise (a model, a procedural instanced mesh) removed".into());
            }
            if let Some((slot, _)) = at {
                top.clear(slot);
            }
            unpick(*id, edit, tables, instances, procs, proc_of);
            // What it held goes back to the room (the build's records and entries stay until the next rebuild).
            if let Some((_, r)) = edit.placed.remove(id) {
                edit.records.give(r, 1);
            }
            if let Some(k) = edit.mesh_of.remove(id) {
                drop_mesh_user(k, edit, tables, meshes);
            }
            tables.retained.own_instance.remove(id);
            tables.retained.instance_of.remove(id);
            if let Some(k) = tables.retained.objects.remove(id).filter(|&k| k as usize >= edit.base.2) {
                edit.objects.give(k as usize, 1);
            }
            proc_of.remove(id);
        }
        let mut new_meshes = vec![];
        let mut fresh_blases = vec![];
        let mut pending_procs = vec![];
        let mut split = EditSplit::default();
        let (mut w_cdf, mut w_verts, mut w_attrs, mut w_blas, mut w_records, mut w_objects) = (Gather::default(), Gather::default(), Gather::default(), Gather::default(), Gather::default(), Gather::default());
        // Vertex colours (F_VCOLOR), in `prev_verts` after the previous vertices.
        let mut w_colors = Gather::default();
        let vcolor_base = up.vcolor_base;
        // (geometry, material) -> the mesh built in this batch (objects sharing a replaced geometry share it again).
        let mut fresh: std::collections::HashMap<(u32, u32), u32> = Default::default();
        let mask = |shadow: bool| if shadow { 1 } else { 2 };
        for id in todo {
            let Some(o) = world.objects.get(&id) else { continue };
            let at = slot_of(id, edit, tables, proc_of);
            emitters_changed |= release(id, tables, meshes, edit.pool_first);
            if !world.shown(id) {
                if let Some((slot, _)) = at {
                    top.clear(slot);
                }
                if edit.instanced.contains_key(&id) {
                    return Err("an instanced mesh added in place hidden".into());
                }
                unpick(id, edit, tables, instances, procs, proc_of);
                if let Some(k) = edit.mesh_of.remove(&id) {
                    drop_mesh_user(k, edit, tables, meshes);
                }
                continue;
            }
            // Drawn but not by a TLAS instance of its own: the build flattened it (movable, or a material instances skip).
            if at.is_none() && tables.retained.objects.contains_key(&id) {
                return Err("an object the build flattened".into());
            }
            let m = world.world_matrix(id);
            let object = match tables.retained.objects.get(&id) {
                Some(&k) => k as usize,
                None => match edit.objects.take(1) {
                    Some(k) => k,
                    None if grow.objects(&mut edit.objects, 1, &mut up.objects, tables.stride) => {
                        edit.grows += 1;
                        edit.rebind = true;
                        edit.objects.take(1).ok_or("edit room: objects")?
                    }
                    None => return Err("edit room: objects (particles, or the device's limits)".into()),
                },
            };
            // Where its TLAS instance places it: the node, or identity for an emitter mesh (world-space triangles).
            let mut placement = m;
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
                        unpick(id, edit, tables, instances, procs, proc_of);
                        continue;
                    };
                    placement = m;
                    // Room for `used` words: the region grown if it lacks it.
                    let mut words_left = |used: usize, edit: &mut EditState| {
                        if used <= edit.word_cap {
                            return true;
                        }
                        let ok = grow.words(&mut edit.word_cap, used, &mut up.cdf, up.cdf_words);
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
                            let at = |word: usize| ((up.cdf_words + word) * 4) as u64;
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
                    w_cdf.put(((up.cdf_words + header) * 4) as u64, bytemuck::cast_slice(&words));
                    edit.word += words.len();
                    let blas = g.proc_blas(&p.blob, &mut pending_procs, &mut fresh_blases);
                    let entry = p.object_entry(o);
                    let gpu = p.gpu(header as u32);
                    let record = at.map(|(_, r)| r);
                    match proc_of.get(&id) {
                        Some(&j) => procs[j] = p,
                        None => {
                            edit.added.insert(id, Picked::Proc(p));
                        }
                    }
                    (record, gpu, Placed::Proc(blas), entry)
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
                    if features & gpu::F_INST == 0 || m.determinant() <= 0.0 || std::env::var("THREERT_INSTANCING").is_ok_and(|v| v == "0") {
                        return Err("a model the build would flatten (mirrored, instancing off, or the scene's first instance)".into());
                    }
                    let mesh = *tables.retained.model_mesh.get(&(uri.clone(), nodes.clone())).ok_or("a model not in the scene (its materials and textures load with a rebuild)")?;
                    let im = meshes[mesh as usize];
                    if !top.has_mesh(mesh) {
                        return Err("a model whose mesh is no unit (it deforms; software traversal flattens it)".into());
                    }
                    *edit.users.entry(mesh).or_default() += 1;
                    if let Some(k) = edit.mesh_of.insert(id, mesh) {
                        drop_mesh_user(k, edit, tables, meshes);
                    }
                    let inst = scene::Instance { mesh, world: m, object: object as u32, cast_shadow: o.cast_shadow, name: o.name.clone(), sub: None, hidden: false };
                    let gpu = top.record(inst.gpu((edit.pool_first as u32) + im.first, im.opaque), mesh, o.cast_shadow);
                    let (lo, hi) = im.placed_bounds(m);
                    let entry = scene::object_row(m.transform_point3(glam::Vec3::ZERO), o.power, hi - lo, o.custom, [1.0; 3]);
                    edit.added.insert(id, Picked::Mesh(inst));
                    (at.map(|(_, r)| r), gpu, Placed::Mesh(mesh), entry)
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
                    // What the build flattens instead (scene.rs: mirrored, or instancing off).
                    if m.determinant() <= 0.0 || std::env::var("THREERT_INSTANCING").is_ok_and(|v| v == "0") {
                        return Err("a mirrored mesh, or instancing off (the build flattens it)".into());
                    }
                    let mat = *tables.retained.materials.get(material).ok_or("unknown material")?;
                    let gm = tables.materials[mat as usize];
                    // An emitting material (no glass, no function): an emitter mesh, its triangles in world space and its
                    // own (never shared), its emitter entries added (scene.rs; ROADMAP § 6 step 3).
                    let emits = scene::emits_only(&gm);
                    let class = if emits { u32::from(gm.uv2[3] > 0.0) } else { scene::material_class(&gm) };
                    if class == 2 || (emits && capacity.is_some()) {
                        return Err("a glass or function material (an instanced mesh: emitting too)".into());
                    }
                    // (An emitter mesh's triangles are in world space: one a body moves is flattened by the build.)
                    if emits && o.driven {
                        return Err("an emitting mesh a rigid body drives (the build flattens it)".into());
                    }
                    if class == 1 && features & gpu::F_MASKED == 0 {
                        return Err("the scene's first cutout (kernel feature)".into());
                    }
                    let colored = world.materials.get(material).is_some_and(|m| m.vertex_colors) && world.geometries.get(geometry).is_some_and(|g| !g.colors.is_empty());
                    if colored && vcolor_base.is_none() {
                        return Err("the scene's first vertex colours (kernel feature)".into());
                    }
                    let key = if emits { (u32::MAX - id, mat) } else { (*geometry, mat) };
                    let kept = (!emits && !ch.geometries.contains(geometry)).then(|| tables.retained.own_mesh.get(&key).copied()).flatten();
                    let mesh = match fresh.get(&key).copied().or(kept) {
                        Some(k) => k,
                        None => {
                            let geo = world.geometries.get(geometry).ok_or("unknown geometry")?;
                            let t0 = Instant::now();
                            let (mut t, mut a) = if emits { scene::emitter_mesh_tris(geo, m, mat, object as u32) } else { scene::own_mesh_tris(geo, mat) };
                            if t.is_empty() {
                                if let Some((slot, _)) = at {
                                    top.clear(slot);
                                }
                                unpick(id, edit, tables, instances, procs, proc_of);
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
                            // (A unit's cutout triangles carry their flag; hardware traversal has them in a geometry apart.)
                            let flags = f32::from_bits(if tree.is_some() && class == 1 { scene::TRI_MASKED } else { 0 });
                            // As `prepare` lays the pool out: (a, flags), e1, e2; attributes; the BLAS's vertices.
                            let verts: Vec<[f32; 4]> = t.iter().flat_map(|&[a, b, c]| [[a.x, a.y, a.z, flags], [b.x - a.x, b.y - a.y, b.z - a.z, 0.0], [c.x - a.x, c.y - a.y, c.z - a.z, 0.0]]).collect();
                            let blas_verts: Vec<[f32; 3]> = t.iter().flat_map(|&[a, b, c]| [[a.x, a.y, a.z], [b.x, b.y, b.z], [c.x, c.y, c.z]]).collect();
                            split.tris += t0.elapsed().as_secs_f64() * 1e3;
                            // The same triangle count as the mesh it replaces: rewritten where it is, its BLAS rebuilt (no
                            // room used; a geometry that moves every frame, three.js's dynamic draw usage). Otherwise new
                            // triangles in the room and a new BLAS.
                            let same = tables.retained.own_mesh.get(&key).copied().filter(|&k| meshes[k as usize].count as usize == t.len() && meshes[k as usize].emits == emits);
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
                                        None if match &mut top { gpu::Top::Hw(accel) => grow.tris(&mut edit.tris, t.len(), &mut up.verts, &mut up.attrs, &mut up.prev_verts, vcolor_base, accel), gpu::Top::Sw(_) => false } => {
                                            edit.grows += 1;
                                            edit.rebind = true;
                                            edit.tris.take(t.len()).ok_or("edit room: triangles")?
                                        }
                                        None => return Err(if matches!(top, gpu::Top::Sw(_)) { "edit room: triangles (software traversal's does not grow)" } else { "edit room: triangles (particles, or the device's limits)" }.into()),
                                    };
                                    let mut im = scene::own_inst_mesh(&t, (first - edit.pool_first) as u32, class);
                                    im.emits = emits;
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
                            if let Some(base) = vcolor_base {
                                // White where it has none: the room's slots may hold an earlier mesh's.
                                let mut c = if colored { scene::tri_colors(geo, false, None) } else { vec![scene::WHITE; t.len()] };
                                if let (Some(tree), true) = (&tree, colored) {
                                    c = tree.order.iter().map(|&i| c[i as usize]).collect();
                                }
                                w_colors.put(base + (first * 12) as u64, bytemuck::cast_slice(&c));
                            }
                            split.upload += t2.elapsed().as_secs_f64() * 1e3;
                            let rel = first - edit.pool_first;
                            if pool_tris.len() < rel + t.len() {
                                pool_tris.resize(rel + t.len(), [glam::Vec3A::ZERO; 3]);
                            }
                            pool_tris[rel..rel + t.len()].copy_from_slice(&t);
                            if emits {
                                // Its emitter entries, in the power-share part of the CDF (path code 0). Rewritten where
                                // it was (a mesh that moves every frame: flying pieces), it takes back the entries it had
                                // there (`release` left them at area 0), so the CDF does not grow frame by frame; those
                                // in the light tree stay out (their triangles now carry code 0).
                                let range = first as u32..(first + t.len()) as u32;
                                let mut free: Vec<usize> = if same.is_some() { tables.emitter_tris.iter().enumerate().filter(|(_, e)| !e.in_tree && range.contains(&e.tri)).map(|(i, _)| i).rev().collect() } else { vec![] };
                                for (j, tri) in t.iter().enumerate() {
                                    let [ta, tb, tc] = *tri;
                                    let area = 0.5 * (tb - ta).cross(tc - ta).length();
                                    if area > 0.0 {
                                        let e = crate::EmitterTri { tri: (first + j) as u32, mat, obj: object as u32, area, c: ((ta + tb + tc) / 3.0).to_array(), moves: false, nominal: false, in_tree: false };
                                        match free.pop() {
                                            Some(i) => tables.emitter_tris[i] = e,
                                            None => tables.emitter_tris.push(e),
                                        }
                                    }
                                }
                                tables.retained.emitter_mesh.insert(id, k);
                                tables.emissive.insert(mat);
                                emitters_changed = true;
                            }
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
                    if let Some(k) = edit.mesh_of.insert(id, mesh) {
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
                        if placed.iter().any(|(wm, hidden)| !hidden && wm.determinant() < 0.0) {
                            return Err("a mirrored instance (the build flattens them)".into());
                        }
                        if !tables.retained.objects.contains_key(&id) {
                            edit.objects.give(object, 1);
                        }
                        let r0 = match edit.records.take(cap) {
                            Some(r) => r,
                            None if match &mut top { gpu::Top::Hw(accel) => grow.records(&mut edit.records, cap, &mut up.instances, accel), gpu::Top::Sw(_) => false } => {
                                edit.grows += 1;
                                edit.rebind = true;
                                edit.records.take(cap).ok_or("edit room: records")?
                            }
                            None => return Err(if matches!(top, gpu::Top::Sw(_)) { "edit room: records (software traversal's does not grow)" } else { "edit room: records (particles, or the device's limits)" }.into()),
                        };
                        let o0 = match edit.objects.take(cap) {
                            Some(k) => k,
                            None if grow.objects(&mut edit.objects, cap, &mut up.objects, tables.stride) => {
                                edit.grows += 1;
                                edit.rebind = true;
                                edit.objects.take(cap).ok_or("edit room: objects")?
                            }
                            None => return Err("edit room: objects (particles, or the device's limits)".into()),
                        };
                        let (st, size) = (tables.stride, std::mem::size_of::<scene::GpuInstance>());
                        if instances.len() < r0 + cap {
                            let filler = scene::Instance { mesh, world: glam::Mat4::IDENTITY, object: 0, cast_shadow: false, name: String::new(), sub: None, hidden: true };
                            instances.resize(r0 + cap, filler);
                        }
                        if gpu_instances.len() < r0 + cap {
                            gpu_instances.resize(r0 + cap, scene::GpuInstance::default());
                        }
                        if tables.objects.len() < (o0 + cap) * st {
                            tables.objects.resize((o0 + cap) * st, [0.0; 4]);
                        }
                        for (i, &(wm, hidden)) in placed.iter().enumerate() {
                            let inst = scene::Instance { mesh, world: wm, object: (o0 + i) as u32, cast_shadow: o.cast_shadow, name: o.name.clone(), sub: Some(i as u32), hidden };
                            let rec = top.record(inst.gpu((edit.pool_first as u32) + im.first, im.opaque), mesh, o.cast_shadow);
                            w_records.put(((r0 + i) * size) as u64, bytemuck::bytes_of(&rec));
                            gpu_instances[r0 + i] = rec;
                            top.place(inst_base + r0 + i, mesh, wm, gpu::INSTANCE_BIT | (r0 + i) as u32, o.cast_shadow, hidden);
                            let (lo, hi) = im.placed_bounds(wm);
                            let entry = scene::object_row(wm.transform_point3(glam::Vec3::ZERO), o.power, hi - lo, t.custom(i, o.custom), t.color(i));
                            w_objects.put(((o0 + i) * st * 16) as u64, bytemuck::cast_slice(&entry[..st]));
                            tables.objects[(o0 + i) * st..(o0 + i + 1) * st].copy_from_slice(&entry[..st]);
                            instances[r0 + i] = inst;
                        }
                        tables.retained.instanced.insert(id, scene::InstancedPlace { first: r0 as u32, capacity: cap as u32, object: o0 as u32, procedural: false, drawn: t.count });
                        tables.retained.objects.insert(id, o0 as u32);
                        edit.instanced.insert(id, (r0, cap, o0));
                        continue;
                    }
                    if emits {
                        placement = glam::Mat4::IDENTITY;
                    }
                    let inst = scene::Instance { mesh, world: placement, object: object as u32, cast_shadow: o.cast_shadow, name: o.name.clone(), sub: None, hidden: false };
                    let gpu = top.record(inst.gpu((edit.pool_first as u32) + im.first, im.opaque), mesh, o.cast_shadow);
                    let (lo, hi) = im.placed_bounds(placement);
                    let entry = scene::object_row(m.transform_point3(glam::Vec3::ZERO), o.power, hi - lo, o.custom, [1.0; 3]);
                    let record = at.map(|(_, r)| r);
                    match record.filter(|&r| r < edit.inst_at_load) {
                        Some(r) => instances[r] = inst,
                        None => {
                            edit.added.insert(id, Picked::Mesh(inst));
                        }
                    }
                    (record, gpu, Placed::Mesh(mesh), entry)
                }
                _ => return Err("not an object with its own BLAS".into()),
            };
            // Its record, TLAS instance and object entry: where it had them, or new ones from the room.
            let (slot, record) = match (at, record) {
                (Some((slot, _)), Some(r)) => (slot, r),
                _ => {
                    let r = match edit.records.take(1) {
                        Some(r) => r,
                        None if match &mut top { gpu::Top::Hw(accel) => grow.records(&mut edit.records, 1, &mut up.instances, accel), gpu::Top::Sw(_) => false } => {
                            edit.grows += 1;
                            edit.rebind = true;
                            edit.records.take(1).ok_or("edit room: records")?
                        }
                        None => return Err(if matches!(top, gpu::Top::Sw(_)) { "edit room: records (software traversal's does not grow)" } else { "edit room: records (particles, or the device's limits)" }.into()),
                    };
                    let v = (inst_base + r, r);
                    edit.placed.insert(id, v);
                    v
                }
            };
            w_records.put((record * std::mem::size_of::<scene::GpuInstance>()) as u64, bytemuck::bytes_of(&gpu));
            if record >= gpu_instances.len() {
                gpu_instances.resize(record + 1, scene::GpuInstance::default());
            }
            gpu_instances[record] = gpu;
            let custom = if matches!(o.kind, Kind::Procedural { .. }) { record as u32 } else { gpu::INSTANCE_BIT | record as u32 };
            match blas {
                Placed::Proc(blas) => top.place_blas(slot, &blas, placement, custom, mask(o.cast_shadow)),
                Placed::Mesh(k) => top.place(slot, k, placement, custom, o.cast_shadow, false),
            }
            let st = tables.stride;
            w_objects.put((object * st * 16) as u64, bytemuck::cast_slice(&entry[..st]));
            let at = object * st;
            if at >= tables.objects.len() {
                tables.objects.resize(at + st, [0.0; 4]);
            }
            tables.objects[at..at + st].copy_from_slice(&entry[..st]);
            tables.retained.objects.insert(id, object as u32);
        }
        // The gathered writes, merged, before the builds that read them.
        let tw = Instant::now();
        w_cdf.flush(q, &up.cdf);
        w_verts.flush(q, &up.verts);
        w_attrs.flush(q, &up.attrs);
        if let gpu::Top::Hw(accel) = &top {
            w_blas.flush(q, &accel.verts);
        }
        w_colors.flush(q, &up.prev_verts);
        w_records.flush(q, &up.instances);
        w_objects.flush(q, &up.objects);
        split.upload += tw.elapsed().as_secs_f64() * 1e3;
        let t3 = Instant::now();
        new_meshes.sort_unstable();
        new_meshes.dedup();
        // The new meshes' and procedural BLASes and the TLAS in one build (`Gpu::build_meshes`).
        match &mut top {
            gpu::Top::Hw(accel) => g.build_edit(accel, &new_meshes, &pending_procs, &mut e, true),
            // The trees are written; the meshes this edit freed give theirs back; the top tree over the units as they
            // are now (one that cannot be made: the scene is rebuilt).
            gpu::Top::Sw(units) => {
                for &k in &edit.dead_meshes {
                    if let Some(old) = units.drop_tree(k) {
                        g.fill_tree(&up.bg_parts.nodes, old);
                    }
                }
                g.retop(units, &up.bg_parts.nodes)?;
            }
        }
        split.blas += t3.elapsed().as_secs_f64() * 1e3;
        // THREERT_PROFILE_BLAS=1: the BLAS / TLAS builds' GPU time (the queue drained before and after; frame log
        // `blas`), as for the dynamic rebuilds.
        let profile_blas = std::env::var("THREERT_PROFILE_BLAS").is_ok_and(|v| v == "1");
        if profile_blas {
            let _ = g.device.poll(wgpu::PollType::wait_indefinitely());
        }
        let t = Instant::now();
        q.submit([e.finish()]);
        g.keep_proc_blases(fresh_blases);
        // Buffers that grew: the scene bind group over them.
        if std::mem::take(&mut edit.rebind) {
            g.rebind_scene(up);
        }
        if profile_blas {
            let _ = g.device.poll(wgpu::PollType::wait_indefinitely());
            self.gpu_blas_ms += t.elapsed().as_secs_f32() * 1000.0;
        }
        split.objects = ch.added.len() + ch.removed.len();
        self.edit_split.add(&split);
        if emitters_changed {
            self.write_emitters();
        }
        Ok(())
    }

    pub fn rebuild(&mut self, world: &World) -> Result<()> {
        let (s, pp) = load_scene(&self.g, world, &self.assets, &self.presets, self.w, self.h, &self.targets, &self.passes)?;
        self.apply_post(&pp);
        self.triangles = s.triangles;
        self.scene_name = s.name.clone();
        // The scene's camera changed (a new lens rebuilds, and a move in the same commit comes with it): the view takes it.
        // Otherwise the view stays where it is.
        if s.scene_camera != self.sc.scene_camera {
            self.cam = s.scene_camera;
        }
        self.sc = s;
        self.spark_lights = self.sc.tables.emitter_tris.iter().any(|e| e.nominal);
        self.restir_prev = None;
        self.inst_moving.clear();
        self.prev_span = None;
        self.pose_span = None;
        self.emitters_moved = false;
        self.lit_changed = self.frame.max(1);
        Ok(())
    }

    /// Rebuilds the tonemap LUT and bloom / vignette / fringe / grain settings when the scene's post preset changed.
    fn apply_post(&mut self, pp: &crate::desc::PostPreset) {
        let key = format!("{pp:?}");
        if key == self.post_key {
            return;
        }
        let lut = tonemap_lut(pp);
        self.g.queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &self.targets.lut_tex, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            bytemuck::cast_slice(&lut),
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(LUT * 8), rows_per_image: Some(LUT) },
            wgpu::Extent3d { width: LUT, height: LUT, depth_or_array_layers: LUT },
        );
        self.bloom = bloom_stages(pp, self.w, self.h);
        self.vignette = pp.vignette;
        // THREERT_NO_GRAIN=1: no film grain (a fresh random pattern every frame; comparisons between frames see it as error).
        let grain = if std::env::var("THREERT_NO_GRAIN").is_ok_and(|v| v == "1") { 0.0 } else { pp.film_grain };
        // THREERT_MOTION_BLUR=1: UE's default motion blur whatever the preset says; =amount,max,targetFps: those.
        let mb = match std::env::var("THREERT_MOTION_BLUR").ok().as_deref() {
            Some("1") => crate::desc::MotionBlur::default(),
            Some(v) => match v.split(',').map(|x| x.trim().parse::<f32>()).collect::<Result<Vec<_>, _>>().as_deref() {
                Ok([amount, max, target_fps]) => crate::desc::MotionBlur { amount: *amount, max: *max, target_fps: *target_fps },
                _ => pp.motion_blur.clone(),
            },
            None => pp.motion_blur.clone(),
        };
        self.post = [pp.chromatic_aberration, grain, pp.bloom.threshold, mb.max / 100.0 * self.w as f32];
        self.blur = (if mb.max > 0.0 { mb.amount.max(0.0) } else { 0.0 }, mb.target_fps);
        self.post_key = key;
    }

    /// Switches to a different world: camera from it, history cleared.
    pub fn open(&mut self, world: &World, assets: &Path) -> Result<()> {
        self.assets = assets.to_path_buf();
        self.rebuild(world)?;
        if let Some(c) = &self.cache {
            let mut e = self.g.device.create_command_encoder(&Default::default());
            c.bufs.clear(&mut e);
            self.g.queue.submit([e.finish()]);
        }
        self.cam = self.sc.scene_camera;
        self.prev_cam = self.cam;
        self.reset = true;
        Ok(())
    }

    /// The view is a display's physical pixels, `k` a point: dynamic resolution, from the points' resolution (the rays
    /// a view in points starts with) down to `MIN_SCALE` of it. A fixed scale is set after it (`set_scale`).
    pub fn set_display_scale(&mut self, k: f32) {
        let k = k.max(1.0);
        self.display_scale = k;
        self.min_scale = (MIN_SCALE / k).max(SCALE_FLOOR);
        self.render_scale = 1.0 / k;
        self.dynamic_res = true;
    }

    /// The scene's lighting is changing: something moved or was edited in the last 45 frames, or the scene has spark
    /// lights (light-sampled particles: they move every frame, and no edit says so).
    fn live_lighting(&self) -> bool {
        self.spark_lights || (self.lit_changed > 0 && self.frame.wrapping_sub(self.lit_changed) < 45)
    }

    /// Fixed render scale (`s` in SCALE_FLOOR ..= 1), or dynamic resolution (`s` = 0).
    pub fn set_scale(&mut self, s: f32) {
        self.dynamic_res = s <= 0.0;
        if s > 0.0 {
            self.render_scale = s.clamp(SCALE_FLOOR, 1.0);
        }
    }

    pub fn reset_history(&mut self) {
        self.reset = true;
    }

    /// Ray tracing on or off (see `rays`). The history starts over: the two modes light a pixel differently.
    pub fn set_rays(&mut self, on: bool) {
        if self.rays != on {
            self.rays = on;
            self.reset = true;
        }
    }

    pub fn rays(&self) -> bool {
        self.rays
    }

    pub fn set_camera(&mut self, mut c: Camera) {
        if c.tan_half_fov <= 0.0 {
            c.tan_half_fov = self.sc.tan_half_fov;
        }
        self.cam = c;
    }

    /// Renders one frame with `spp` samples per pixel; returns the command buffers to submit after the samples.
    /// Path-trace dispatches are submitted inside (each needs its own uniform contents).
    /// Traced resolution at the current render scale.
    pub fn internal_size(&self) -> (u32, u32) {
        let s = self.render_scale.clamp(SCALE_FLOOR, 1.0);
        (((self.w as f32 * s).round() as u32).clamp(8, self.w), ((self.h as f32 * s).round() as u32).clamp(8, self.h))
    }

    /// Dynamic resolution (UE r.DynamicRes): twice a second, move the render scale toward the frame budget (traced
    /// pixels ~ GPU time), within [`min_scale`, 1], in damped steps with hysteresis so it does not oscillate.
    /// Two frames in a row well over the frame time (x 1.25) act at once (UE
    /// r.DynamicRes.MaxConsecutiveOverbudgetGPUFrameCount = 2): turning from a light view at a raised scale toward a
    /// heavy one otherwise cost up to 30 frames at 35-48 ms (the 3 x 3 district's drive, its far end). Just over the
    /// frame time waits for the regular check, as before. After a change, only frames rendered at the new scale count:
    /// GPU time lags the scale by the frames in flight, and acting on the old scale's time overshot to the floor.
    pub fn govern_resolution(&mut self, fps: u32) {
        if !self.dynamic_res || self.gpu_ms <= 0.0 {
            return;
        }
        if (self.gpu_frame.wrapping_sub(self.over_budget.1) as i32) < 0 {
            return;
        }
        // Not in the first 2 s: those frames run while caches fill (Kabukichō's first frames at 0.7 take ~47 ms), and
        // the regular check settles the start as before.
        self.over_budget.0 = if self.frame >= 120 && self.gpu_ms > 1250.0 / fps as f32 && self.render_scale > self.min_scale { self.over_budget.0 + 1 } else { 0 };
        let urgent = self.over_budget.0 >= 2;
        if self.frame % 30 != 1 && !urgent {
            return;
        }
        self.over_budget.0 = 0;
        let before = self.render_scale;
        let budget = 1000.0 / fps as f32 * 0.9;
        let target = (self.render_scale * (budget / self.gpu_ms).sqrt()).clamp(self.min_scale, 1.0);
        // The hysteresis (0.03) does not hold the scale just short of either end: 0.525 with the floor as the target
        // kept the district's return drive at 21-22 ms.
        let to_end = (target == 1.0 && self.render_scale < 1.0 && self.gpu_ms < budget * 0.8) || (target == self.min_scale && self.render_scale > self.min_scale);
        // (The hysteresis, the steps and the grid of scales are a view in points'; on a display of k pixels a point the
        // same share of the rays is 1 / k of the scale. Left absolute, a Retina view held 0.325 at 17.3 ms: its target,
        // 0.30, was inside the hysteresis.)
        let k = self.display_scale;
        if (target - self.render_scale).abs() > 0.03 / k || to_end {
            // Down fast when far over budget (a heavy scene just opened), up slowly; straight to the target when urgent.
            let step = if urgent { target - self.render_scale } else { (target - self.render_scale).clamp(if self.gpu_ms > budget * 1.5 { -0.3 } else { -0.15 } / k, 0.1 / k) };
            self.render_scale = ((self.render_scale + step) * 40.0 * k).round() / (40.0 * k);
        }
        if self.render_scale != before {
            // `frame` is already the next frame's index (render advanced it): the first one at the new scale.
            self.over_budget.1 = self.frame;
        }
    }

    /// Light LOD: the emitter CDF weighted by distance to the camera (`emitter_lod`) and the emitter cull with its view
    /// cone (`EmitterCull`), rebuilt when the camera is 15 m from where it was last built or has turned LIGHT_LOD_TURN
    /// from that direction (the kernel's emitter pdfs read the same reference from the parameters).
    fn update_light_lod(&mut self) {
        let d0 = light_lod_d0();
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
        b.emit_power[1] = emitter_cull();
        b.emit_power[2] = emitter_cull() * emitter_cull_view();
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
        self.still = if self.cam.pos != self.prev_cam.pos { 0 } else { self.still.saturating_add(1) };
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
        let media = self.sc.up.kernel.features & gpu::F_MEDIA != 0;
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
            if self.restir {
                let pc = self.prev_cam;
                p.prev_pos = [pc.pos.x, pc.pos.y, pc.pos.z, pc.tan_half_fov];
                p.prev_fwd = [pc.fwd.x, pc.fwd.y, pc.fwd.z, self.aspect];
                p.prev_right = [pc.right.x, pc.right.y, pc.right.z, 0.0];
                p.prev_up = [pc.up.x, pc.up.y, pc.up.z, 0.0];
                let (mode, (pw, ph)) = match self.restir_prev {
                    Some(prev) if !self.reset => (2.0, prev),
                    _ => (1.0, (iw, ih)),
                };
                p.restir = [mode, pw as f32, ph as f32, (self.frame % 2) as f32];
            }
            if (!self.sc.dynamic.1.is_empty() || !self.sc.tables.retained.instance_of.is_empty() || !self.sc.tables.retained.rigid.is_empty()) && motion_vectors() {
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
            // (The merged layout: the tables as this frame's edits left them, where the kernel reads them; merged.rs.)
            if k == 0 {
                g.sync_tables(&self.sc.up, &mut e);
            }
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
            mode: [self.view_mode, self.still, self.live_lighting() as u32, temporal_flags() | if self.ghost_fix_off { 16384 } else { 0 }],
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
        };
        g.queue.write_buffer(&self.targets.params, 0, bytemuck::bytes_of(&lp));
        let bg = &self.sc.bgs[(self.frame % 2) as usize];
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
                let (gw, gh) = if *name == "exposure" { (8, 8) } else if *name == "dof_tiles" { (w.div_ceil(16), h.div_ceil(16)) } else if traced_pass(name) { (iw, ih) } else if name.starts_with("bloom") || name.starts_with("lum_") { (w.div_ceil(4), h.div_ceil(4)) } else { (w, h) };
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
                if *name == "exposure" {
                    pass.dispatch_workgroups(1, 1, 1);
                    continue;
                }
                let (gw, gh) = if *name == "dof_tiles" { (w.div_ceil(16), h.div_ceil(16)) } else if traced_pass(name) { (iw, ih) } else if name.starts_with("bloom") || name.starts_with("lum_") { (w.div_ceil(4), h.div_ceil(4)) } else { (w, h) };
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
        self.restir_prev = Some((iw, ih));
        self.prev_isz = (iw, ih);
        self.reset = false;
        self.frame = self.frame.wrapping_add(1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// The last frame's linear HDR radiance (nits, before exposure and post), RGB per output pixel. Waits for the GPU.
    pub fn read_hdr(&self) -> Result<Vec<f32>> {
        let d = &self.g.device;
        let size = self.w as u64 * self.h as u64 * 16;
        let read = d.create_buffer(&wgpu::BufferDescriptor { label: Some("hdr read"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut e = d.create_command_encoder(&Default::default());
        e.copy_buffer_to_buffer(&self.targets.fa.buffer, self.targets.fa.offset, &read, 0, size);
        self.g.queue.submit([e.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        d.poll(wgpu::PollType::wait_indefinitely())?;
        let px = bytemuck::cast_slice::<u8, [f32; 4]>(&read.slice(..).get_mapped_range().unwrap()).iter().flat_map(|p| [p[0], p[1], p[2]]).collect();
        Ok(px)
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// The last frame's signals apart, linear RGB f32 per output pixel: diffuse (the composed colour less the rest, so
    /// with the sun's direct light, which view mode 6 leaves out), specular (the specular denoiser's output: mode 7) and
    /// emission (what the camera ray saw directly). They are the render resolution's (the frame as `compose` hands it to
    /// the upscaler), each output pixel taking the traced sample under it; not with depth of field and motion blur both
    /// on (the composed colour's buffer is theirs then). Waits for the GPU.
    pub fn read_signals(&self) -> Result<[Vec<f32>; 3]> {
        let d = &self.g.device;
        let (w, h) = (self.w as usize, self.h as usize);
        let (iw, ih) = (self.prev_isz.0 as usize, self.prev_isz.1 as usize);
        let (n, tn) = ((w * h) as u64, (iw * ih) as u64);
        let t = &self.targets;
        let read = |buffer: &wgpu::Buffer, offset: u64, size: u64| -> Result<Vec<u8>> {
            let r = d.create_buffer(&wgpu::BufferDescriptor { label: Some("signals read"), size, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
            let mut e = d.create_command_encoder(&Default::default());
            e.copy_buffer_to_buffer(buffer, offset, &r, 0, size);
            self.g.queue.submit([e.finish()]);
            r.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            d.poll(wgpu::PollType::wait_indefinitely())?;
            let v = r.slice(..).get_mapped_range()?.to_vec();
            Ok(v)
        };
        let half = |b: u32, hi: bool| half::f16::from_bits(if hi { (b >> 16) as u16 } else { b as u16 }).to_f32();
        let composed = read(&t.fb.buffer, t.fb.offset, tn * 16)?;
        let spec = read(&t.sa.buffer, t.sa.offset + n * 8, tn * 8)?; // spec_pp slot 1: (rgb, history) as two pairs of f16
        let samples = read(&self.sc.up.accum, 0, tn * 80)?; // the kernel's samples (live.wgsl `Pixel`): emit at 48
        let (composed, spec, samples): (&[[f32; 4]], &[[u32; 2]], &[[f32; 20]]) = (bytemuck::cast_slice(&composed), bytemuck::cast_slice(&spec), bytemuck::cast_slice(&samples));
        let (mut dif, mut spe, mut emi) = (Vec::with_capacity(w * h * 3), Vec::with_capacity(w * h * 3), Vec::with_capacity(w * h * 3));
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        for y in 0..h {
            for x in 0..w {
                // live.wgsl `sample_under`.
                let q = (((y as f32 + 0.5) * ih as f32 / h as f32) as usize).min(ih - 1) * iw + (((x as f32 + 0.5) * iw as f32 / w as f32) as usize).min(iw - 1);
                let s = [half(spec[q][0], false), half(spec[q][0], true), half(spec[q][1], false)];
                for c in 0..3 {
                    let e = finite(samples[q][12 + c] / self.shown_spp as f32);
                    spe.push(s[c]);
                    emi.push(e);
                    dif.push((composed[q][c] - s[c] - e).max(0.0));
                }
            }
        }
        Ok([dif, spe, emi])
    }

    #[cfg(not(target_arch = "wasm32"))]
    /// The exposure the last frame was shown at, EV100 (auto exposure: its adapted value).
    pub fn read_ev100(&self) -> Result<f32> {
        if self.sc.exposure_settings.ae2[3] <= 0.5 {
            return Ok(-self.sc.exposure.log2());
        }
        let d = &self.g.device;
        let read = d.create_buffer(&wgpu::BufferDescriptor { label: Some("ae read"), size: 16, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let mut e = d.create_command_encoder(&Default::default());
        e.copy_buffer_to_buffer(&self.targets.ae_state.buffer, self.targets.ae_state.offset, &read, 0, 16);
        self.g.queue.submit([e.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        d.poll(wgpu::PollType::wait_indefinitely())?;
        let s = bytemuck::cast_slice::<u8, f32>(&read.slice(..).get_mapped_range().unwrap())[0];
        Ok(-s.max(1e-30).log2())
    }

    /// Phases of the last scene (re)load, ms.
    pub fn load_timings(&self) -> &[(String, f64)] {
        &self.sc.timings
    }

    pub fn aspect(&self) -> f32 {
        self.aspect
    }

    /// Primary ray through (u, v) of the image (0..1, v down), as the path tracer generates it.
    pub fn ray(&self, u: f32, v: f32) -> (Vec3, Vec3) {
        let c = &self.cam;
        let d = c.fwd + c.right * ((u * 2.0 - 1.0) * c.tan_half_fov * self.aspect) + c.up * ((1.0 - v * 2.0) * c.tan_half_fov);
        (c.pos, d.normalize())
    }

    /// Closest surface under (u, v): (node id, world-space hit point). CPU ray cast over the scene triangles, and over
    /// each instance's model-space mesh with the ray in its space (same ray parameter t).
    /// The object under view point (u, v): its name, the hit point, and for an instanced object the instance's index.
    pub fn pick(&self, u: f32, v: f32) -> Option<(String, Vec3, Option<u32>)> {
        let (o, d) = self.ray(u, v);
        let (o, d) = (glam::Vec3A::from(o), glam::Vec3A::from(d));
        // Moller-Trumbore, two-sided: the ray parameter of the closest hit in `tris` beyond `near`.
        let closest = |tris: &[[glam::Vec3A; 3]], o: glam::Vec3A, d: glam::Vec3A| {
            let mut best = (f32::INFINITY, usize::MAX);
            for (i, [a, b, c]) in tris.iter().enumerate() {
                let (e1, e2) = (*b - *a, *c - *a);
                let p = d.cross(e2);
                let det = e1.dot(p);
                if det.abs() < 1e-12 {
                    continue;
                }
                let inv = 1.0 / det;
                let s = o - *a;
                let bu = s.dot(p) * inv;
                if !(0.0..=1.0).contains(&bu) {
                    continue;
                }
                let q = s.cross(e1);
                let bv = d.dot(q) * inv;
                if bv < 0.0 || bu + bv > 1.0 {
                    continue;
                }
                let t = e2.dot(q) * inv;
                if t > 1e-4 && t < best.0 {
                    best = (t, i);
                }
            }
            best
        };
        let (mut t, i) = closest(&self.sc.tris, o, d);
        let mut node = self.sc.tri_nodes.iter().find(|(_, a, b)| (*a as usize..*b as usize).contains(&i)).map(|n| n.0.as_str());
        let mut sub = self.sc.tri_instances.iter().find(|(a, b, _)| (*a as usize..*b as usize).contains(&i)).map(|r| r.2);
        let added = |want_mesh: bool| self.sc.edit.added.values().filter_map(move |p| match p {
            Picked::Mesh(i) if want_mesh => Some(Ok(i)),
            Picked::Proc(q) if !want_mesh => Some(Err(q)),
            _ => None,
        });
        for inst in self.sc.instances.iter().filter(|i| !i.hidden).chain(added(true).filter_map(|x| x.ok())) {
            let m = self.sc.meshes[inst.mesh as usize];
            let inv = inst.world.inverse();
            let (lo, ld) = (inv.transform_point3a(o), inv.transform_vector3a(d));
            // Slab test against the mesh bounds before its triangles.
            let (t0, t1) = ((m.lo - lo) / ld, (m.hi - lo) / ld);
            let (near, far) = (t0.min(t1).max_element(), t0.max(t1).min_element());
            if far < near.max(0.0) || near > t {
                continue;
            }
            let (ti, _) = closest(&self.sc.pool_tris[m.first as usize..(m.first + m.count) as usize], lo, ld);
            if ti < t {
                t = ti;
                node = Some(inst.name.as_str());
                sub = inst.sub;
            }
        }
        // Procedural geometry: its boxes (the geometry function runs on the GPU only).
        for p in self.sc.procs.iter().filter(|p| !p.hidden).chain(added(false).filter_map(|x| x.err())) {
            let inv = p.world.inverse();
            let (lo, ld) = (inv.transform_point3a(o), inv.transform_vector3a(d));
            let slab = |a: glam::Vec3A, b: glam::Vec3A| {
                let (t0, t1) = ((a - lo) / ld, (b - lo) / ld);
                let (near, far) = (t0.min(t1).max_element().max(1e-4), t0.max(t1).min_element());
                (near <= far).then_some(near)
            };
            if slab(p.lo.into(), p.hi.into()).is_none_or(|near| near > t) {
                continue;
            }
            for b in p.blob.boxes.chunks(6) {
                if let Some(near) = slab(glam::Vec3A::new(b[0], b[1], b[2]), glam::Vec3A::new(b[3], b[4], b[5])).filter(|&near| near < t) {
                    t = near;
                    node = Some(p.name.as_str());
                    sub = p.sub;
                }
            }
        }
        Some((node?.to_string(), Vec3::from(o + d * t), sub))
    }

    /// World-space triangles of a node (empty for lights without geometry, cameras, hidden nodes).
    pub fn node_tris(&self, id: &str) -> Vec<[glam::Vec3A; 3]> {
        // World-space triangles (an instance's glass / emissive part among them), then its instanced mesh.
        let mut out: Vec<[glam::Vec3A; 3]> = scene::node_ranges(&self.sc.tri_nodes, id).flat_map(|r| self.sc.tris[r].iter().copied()).collect();
        // A model's first instance; every drawn instance of an instanced object.
        let mut of_node = self.sc.instances.iter().filter(|i| i.name == id && !i.hidden);
        let first = of_node.next();
        for inst in first.into_iter().chain(of_node.filter(|i| i.sub.is_some())) {
            let m = self.sc.meshes[inst.mesh as usize];
            out.extend(self.sc.pool_tris[m.first as usize..(m.first + m.count) as usize].iter().map(|t| inst.tri(t)));
        }
        // Objects added in place (Bugbot on #131: `pick` saw them, this did not).
        for p in self.sc.edit.added.values() {
            if let Picked::Mesh(inst) = p {
                if inst.name != id {
                    continue;
                }
                let m = self.sc.meshes[inst.mesh as usize];
                out.extend(self.sc.pool_tris[m.first as usize..(m.first + m.count) as usize].iter().map(|t| inst.tri(t)));
            }
        }
        out
    }

    /// Frame metadata (camera pose for picking, convergence, timings).
    pub fn meta(&self) -> serde_json::Value {
        let c = &self.cam;
        serde_json::json!({
            "seq": self.seq, "frame": self.frame, "history": self.history_len, "gpu_ms": self.gpu_ms,
            "scene": self.scene_name, "triangles": self.triangles, "bounces": self.bounces, "mode": self.view_mode, "width": self.w, "height": self.h, "scale": self.render_scale,
            "cam": { "pos": c.pos.to_array(), "fwd": c.fwd.to_array(), "up": c.up.to_array(), "right": c.right.to_array(), "tan": c.tan_half_fov, "aspect": self.aspect },
        })
    }
}

/// Adaptive samples per frame: fill ~75% of the frame budget with path tracing.
pub fn next_spp(spp: u32, gpu_ms: f32, fps: u32) -> u32 {
    if gpu_ms <= 0.0 {
        return spp;
    }
    let budget = 750.0 / fps as f32;
    let per_sample = gpu_ms / spp as f32;
    let target = (budget / per_sample).floor() as u32;
    target.clamp(1, 16).min(spp + 1).max(spp.saturating_sub(2).max(1))
}

#[cfg(not(target_arch = "wasm32"))]
/// Headless runner: renders at a fixed frame rate and saves snapshots on request (tools/bench-live.ts).
pub fn run(presets: &Path, opts: LiveOpts) -> Result<()> {
    let (w, h) = (opts.width, opts.height);
    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    spawn_stdin_reader(cmd_tx);
    let (mut game, mut core) = crate::script::GameSession::start(&opts.bundle, &opts.room, &opts.assets, gpu::Gpu::new()?, presets, w, h)?;
    // Frame-script cost (script + commit), reported every 2 s.
    let (mut fs_sum, mut fs_max, mut fs_n) = (0.0f32, 0.0f32, 0u32);
    core.bounces = opts.bounces;
    core.set_display_scale(opts.display_scale);
    if let Some(s) = opts.scale {
        core.set_scale(s);
    }
    eprintln!("live: load {}", serde_json::Value::Object(core.load_timings().iter().map(|(k, v)| (k.clone(), (*v).into())).collect()));
    eprintln!("live: {}x{} @{} fps on {} ({})", w, h, opts.fps, core.g.adapter_info.name, if core.g.hwrt { "HW RT" } else { "BVH" });
    let staging = core.g.device.create_buffer(&wgpu::BufferDescriptor { label: Some("snapshot"), size: (w * h * 4) as u64, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
    let frame_dt = Duration::from_secs_f64(1.0 / opts.fps as f64);
    let mut next = Instant::now();
    let mut snapshots: Vec<(PathBuf, u32, u32)> = vec![]; // path, min history, first eligible frame
    let mut syncs: Vec<(String, u32)> = vec![]; // id, frame after which to answer
    let mut spp = 1u32;
    let mut last = Instant::now();
    let start = Instant::now();
    let mut log = opts.frame_log.as_ref().map(|p| std::fs::File::create(p).map(std::io::BufWriter::new)).transpose()?;
    let mut last_frame = Instant::now();
    let mut prev_start = Instant::now();
    let mut lockstep: Option<f32> = None;
    let mut budget = 0u32; // lockstep: frames the commands so far asked for and not yet rendered
    loop {
        let t_frame = Instant::now();
        let mut cmds: Vec<Cmd> = vec![];
        if lockstep.is_some() && budget == 0 {
            match cmd_rx.recv() {
                Ok(c) => cmds.push(c),
                Err(_) => return Ok(()),
            }
        }
        cmds.extend(std::iter::from_fn(|| cmd_rx.try_recv().ok()));
        for c in cmds {
            match c {
                Cmd::Camera(c) => core.set_camera(c),
                Cmd::Lockstep(dt) => {
                    lockstep = dt;
                    core.fixed_dt = dt;
                    budget = 0;
                }
                Cmd::Step { cam, frames, sync, snapshot } => {
                    if let Some(c) = cam {
                        core.set_camera(c);
                    }
                    let n = frames.max(1);
                    if lockstep.is_some() {
                        budget += n;
                    }
                    let last = core.frame + n - 1;
                    if let Some(p) = snapshot {
                        snapshots.push((p, 0, last));
                    }
                    if let Some(id) = sync {
                        syncs.push((id, last));
                    }
                }
                Cmd::Snapshot(path, after, frames) => snapshots.push((path, after.unwrap_or(0), core.frame + frames.unwrap_or(1).max(1) - 1)),
                Cmd::Sync(id, frames) => syncs.push((id, core.frame + frames.max(1) - 1)),
                Cmd::Reset => core.reset_history(),
                Cmd::Time(t) => core.set_time(t),
                Cmd::Mode(m) => core.view_mode = m,
                Cmd::Bounces(b) => core.bounces = b,
                Cmd::Scale(s) => core.set_scale(s),
                Cmd::Event(kind, payload) => {
                    if let Err(e) = game.event(&mut core, &kind, payload) {
                        eprintln!("live: event {kind} failed: {e:#}");
                    }
                }
                Cmd::Click(u, v) => {
                    if let Err(e) = game.click(&mut core, u, v) {
                        eprintln!("live: click failed: {e:#}");
                    }
                }
                Cmd::SppCap(_) | Cmd::Status(_) => {}
                Cmd::Quit => return Ok(()),
            }
        }
        // The frame script runs on wall-clock time (frames slower than the target rate still animate at real speed).
        if lockstep.is_some() && budget == 0 {
            continue; // commands without frames (events, switches): wait for a step
        }
        let dt = if freeze_time() { 0.0 } else { lockstep.unwrap_or_else(|| last.elapsed().as_secs_f32().min(0.1)) };
        last = Instant::now();
        core.advance_time(dt);
        if let Err(e) = game.frame(&mut core, dt) {
            eprintln!("live: frame script: {e:#}");
        }
        if game.host.has_frame() {
            (fs_sum, fs_max, fs_n) = (fs_sum + game.frame_ms, fs_max.max(game.frame_ms), fs_n + 1);
            if fs_n == opts.fps * 2 {
                eprintln!("live: frame script {:.3} ms avg, {:.3} ms max (script + commit)", fs_sum / fs_n as f32, fs_max);
                (fs_sum, fs_max, fs_n) = (0.0, 0.0, 0);
            }
        }
        // Snapshot metadata is this frame's (history / spp at render time), so copy the output whenever one is pending.
        let pending = !snapshots.is_empty();
        core.render(spp, |e, out| {
            if pending {
                e.copy_buffer_to_buffer(out, 0, &staging, 0, (w * h * 4) as u64);
            }
        });
        game.click_shown();
        let (hist, fno) = (core.history_len as u32, core.frame);
        syncs.retain(|(id, from)| {
            if fno > *from {
                eprintln!("live: sync {id}");
            }
            fno <= *from
        });
        if snapshots.iter().any(|(_, after, from)| fno > *from && hist >= *after) {
            staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            core.g.device.poll(wgpu::PollType::wait_indefinitely())?;
            let px = staging.slice(..).get_mapped_range()?.to_vec();
            staging.unmap();
            let mut hdr: Option<Vec<f32>> = None;
            snapshots.retain(|(path, after, from)| {
                if fno > *from && hist >= *after {
                    if path.extension().is_some_and(|e| e == "signals") {
                        // The signals apart (`read_signals`), as `render --signals` writes them.
                        if let Ok([d, s, e]) = core.read_signals() {
                            let stem = path.with_extension("");
                            for (name, v) in [("diffuse", d), ("specular", s), ("emission", e)] {
                                let _ = std::fs::write(format!("{}-{name}.raw", stem.display()), bytemuck::cast_slice(&v));
                            }
                        }
                    } else if path.extension().is_some_and(|e| e == "raw") {
                        // Linear radiance (RGB f32, output size), as `serve` live jobs write it.
                        if hdr.is_none() {
                            hdr = core.read_hdr().ok();
                        }
                        let _ = std::fs::write(path, bytemuck::cast_slice(hdr.as_deref().unwrap_or(&[])));
                    } else {
                        save_bgra(path, &px, w, h);
                    }
                    eprintln!("live: snapshot {} at history {hist} ({spp} spp/frame, GPU {:.2} ms, scale {:.2})", path.display(), core.gpu_ms, core.render_scale);
                    false
                } else {
                    true
                }
            });
        }
        if let Some(l) = &mut log {
            use std::io::Write;
            let line = serde_json::json!({
                "t": start.elapsed().as_secs_f64() * 1e3, "interval": last_frame.elapsed().as_secs_f64() * 1e3, "start": t_frame.duration_since(prev_start).as_secs_f64() * 1e3,
                "cpu": t_frame.elapsed().as_secs_f64() * 1e3, "script": game.frame_ms, "js": game.js_ms, "commit": game.commit_ms, "skin": core.cpu_skin_ms, "upload": core.cpu_upload_ms, "blas": core.gpu_blas_ms, "wait": core.cpu_wait_ms, "gpu": core.gpu_ms, "trace": core.gpu_trace_ms, "edit": core.edit_ms, "instanced": core.instanced_ms, "edit_split": [core.edit_split.tris, core.edit_split.upload, core.edit_split.blas, core.edit_split.added, core.edit_split.deformed, core.edit_split.objects], "rebuild": core.rebuild_ms, "fallback": core.fallback, "fallbacks": core.fallbacks, "room": core.room_stats(), "top": core.top_stats(), "voxel": std::mem::take(&mut game.host.voxels.borrow_mut().ticks), "physics": game.host.physics.borrow_mut().as_mut().map(|p| { let s = p.take_stats(); serde_json::json!({ "ticks": s.ticks, "step": s.step_ms, "sync": s.sync_ms, "bodies": s.bodies, "moved": s.moved, "kicks": s.kicks }) }), "scale": core.render_scale, "history": core.history_len, "guard": core.guard_count(),
            });
            writeln!(l, "{line}")?;
        }
        // (Summed over the frame's commits, a game event's and the frame script's: reset once logged.)
        (core.edit_ms, core.instanced_ms, core.edit_split, core.rebuild_ms, core.fallback) = (0.0, 0.0, EditSplit::default(), 0.0, None);
        prev_start = t_frame;
        last_frame = Instant::now();
        core.govern_resolution(opts.fps);
        spp = if core.render_scale < 1.0 || lockstep.is_some() { 1 } else { next_spp(spp, core.gpu_ms, opts.fps) };
        let _ = core.g.device.poll(wgpu::PollType::Poll);
        if lockstep.is_some() {
            budget -= 1;
            continue; // no pacing: frames as fast as the GPU renders them
        }
        // Pace to the frame rate; when the GPU is the bottleneck the two-frames-in-flight limit paces instead (sleeping
        // after a frame that did not wait would alternate long and short frames).
        next += frame_dt;
        let now = Instant::now();
        if next > now && core.gpu_ms < frame_dt.as_secs_f32() * 1000.0 * 0.95 {
            sleep_until(next);
        } else {
            next = now;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Sleeps until `t` without trusting the timer's precision: macOS gives a process that is not a foreground app timer
/// leeway of half the interval (a 16 ms sleep returned after 24, measured from a terminal-launched viewport and from
/// plain Python alike). Pacing sleeps of the headless viewport overslept by ~5 ms once the GPU fit the frame: Bistro's
/// camera path read frame interval p95 21.9 ms at GPU p95 16.0. Sleep 60 % of what is left, again, and yield through
/// the last 1.5 ms.
fn sleep_until(t: Instant) {
    loop {
        let now = Instant::now();
        if now >= t {
            return;
        }
        let rem = t - now;
        if rem > Duration::from_micros(1500) {
            std::thread::sleep(rem.mul_f64(0.6));
        } else {
            std::thread::yield_now();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn save_bgra(path: &Path, px: &[u8], w: u32, h: u32) {
    let rgba: Vec<u8> = px.chunks(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
    if let Err(e) = image::save_buffer(path, &rgba, w, h, image::ColorType::Rgba8) {
        eprintln!("live: snapshot failed: {e}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn_stdin_reader(cmd_tx: mpsc::Sender<Cmd>) {
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            let Some(cmd) = parse_cmd(&line) else { continue };
            if cmd_tx.send(cmd).is_err() {
                break;
            }
        }
        let _ = cmd_tx.send(Cmd::Quit);
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub fn parse_cmd(line: &str) -> Option<Cmd> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let vec = |k: &str| v[k].as_array().map(|a| Vec3::new(a[0].as_f64().unwrap_or(0.0) as f32, a[1].as_f64().unwrap_or(0.0) as f32, a[2].as_f64().unwrap_or(0.0) as f32));
    Some(match v["cmd"].as_str()? {
        "camera" if v.get("sync").is_some() || v.get("snapshot").is_some() || v.get("frames").is_some() => Cmd::Step {
            cam: Some(Camera::look_at(vec("pos")?, vec("target")?, v["tan"].as_f64().unwrap_or(0.0) as f32)),
            frames: v["frames"].as_u64().unwrap_or(1) as u32,
            sync: v.get("sync").map(|s| s.as_str().map(str::to_string).unwrap_or_else(|| s.to_string())),
            snapshot: v["snapshot"].as_str().map(PathBuf::from),
        },
        "camera" => Cmd::Camera(Camera::look_at(vec("pos")?, vec("target")?, v["tan"].as_f64().unwrap_or(0.0) as f32)),
        "frames" => Cmd::Step {
            cam: None,
            frames: v["n"].as_u64().unwrap_or(1) as u32,
            sync: v.get("sync").map(|s| s.as_str().map(str::to_string).unwrap_or_else(|| s.to_string())),
            snapshot: v["snapshot"].as_str().map(PathBuf::from),
        },
        "lockstep" => Cmd::Lockstep(if v["on"].as_bool().unwrap_or(true) { Some(v["dt"].as_f64().unwrap_or(1.0 / 60.0) as f32) } else { None }),
        "event" => Cmd::Event(v["type"].as_str()?.to_string(), if v["payload"].is_null() { serde_json::json!({}) } else { v["payload"].clone() }),
        "click" => Cmd::Click(v["u"].as_f64()? as f32, v["v"].as_f64()? as f32),
        "snapshot" => Cmd::Snapshot(PathBuf::from(v["path"].as_str().unwrap_or("snapshot.png")), v["after"].as_u64().map(|a| a as u32), v["frames"].as_u64().map(|a| a as u32)),
        "reset" => Cmd::Reset,
        "time" => Cmd::Time(v["t"].as_f64()? as f32),
        "mode" => Cmd::Mode(v["mode"].as_u64()? as u32),
        "bounces" => Cmd::Bounces(v["bounces"].as_u64()?.clamp(1, 16) as u32),
        "spp" => Cmd::SppCap(v["max"].as_u64()?.clamp(1, 16) as u32),
        "scale" => Cmd::Scale(v["scale"].as_f64()? as f32),
        "sync" => Cmd::Sync(v["id"].as_str().map(str::to_string).unwrap_or_else(|| v["id"].to_string()), v["frames"].as_u64().unwrap_or(1) as u32),
        "quit" => Cmd::Quit,
        _ => return None,
    })
}

#[cfg(not(target_arch = "wasm32"))]
pub fn parse_opts(args: &[String]) -> Result<LiveOpts> {
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let num = |name: &str, d: u32| -> Result<u32> { flag(name).map(|v| v.parse().map_err(|_| anyhow!("{name} must be a number"))).transpose().map(|v| v.unwrap_or(d)) };
    Ok(LiveOpts {
        width: num("--width", 1280)?,
        height: num("--height", 720)?,
        fps: num("--fps", 60)?,
        bounces: num("--bounces", 4)?,
        // (Without `--scale`: full resolution, or dynamic resolution on a display's physical pixels.)
        scale: parse_scale(flag("--scale").as_deref(), if flag("--display-scale").is_some() { None } else { Some(1.0) })?,
        bundle: PathBuf::from(args.get(2).filter(|a| !a.starts_with("--")).ok_or_else(|| anyhow!("game bundle required"))?),
        assets: PathBuf::from(flag("--assets").unwrap_or_else(|| ".".into())),
        room: flag("--room").unwrap_or_else(|| "local".into()),
        frame_log: flag("--frame-log").map(PathBuf::from),
        display_scale: flag("--display-scale").map(|v| v.parse::<f32>()).transpose()?.unwrap_or(1.0),
    })
}

#[cfg(not(target_arch = "wasm32"))]
/// `--scale`: a fraction (0.25 .. 1) or `auto` (dynamic resolution).
pub fn parse_scale(v: Option<&str>, default: Option<f32>) -> Result<Option<f32>> {
    match v {
        None => Ok(default),
        Some("auto") => Ok(None),
        Some(v) => v.parse::<f32>().map(|s| Some(s.clamp(SCALE_FLOOR, 1.0))).map_err(|_| anyhow!("--scale must be a number or auto")),
    }
}

#[cfg(test)]
mod timer_tests {
    use super::*;

    #[test]
    fn a_view_in_physical_pixels_starts_with_dynamic_resolution() {
        let opts = |extra: &[&str]| parse_opts(&["threert-render", "live", "game.js"].iter().chain(extra).map(|a| a.to_string()).collect::<Vec<_>>()).unwrap();
        assert_eq!(opts(&[]).scale, Some(1.0));
        assert_eq!(opts(&["--display-scale", "2"]).scale, None);
        assert_eq!(opts(&["--display-scale", "2", "--scale", "0.2"]).scale, Some(0.2));
        assert_eq!(opts(&["--scale", "auto"]).scale, None);
    }

    #[test]
    fn a_post_pass_from_another_frame_counts_by_its_own_length() {
        // Two frames of the crowd scene (ns, period 1): frame 453 as read, and frame 454, whose slot read frame 458's
        // post pair (74.5 ms after its trace began).
        let t453 = [35929577531333, 35929585495833, 35929585700583, 35929591610875, 0, 0, 0, 0];
        let t454 = [35929593765500, 35929601614666, 35929668312875, 35929673973041, 0, 0, 0, 0];
        let ms = |t: [u64; 8]| frame_gpu_ms(&t, 1, 1.0).unwrap();
        assert_eq!(ms(t453), (t453[3] - t453[0]) as f32 * 1e-6);
        assert!((ms(t454) - (7.849166 + 5.660166)).abs() < 1e-3, "{}", ms(t454));
        // The first pass is the frame's start; nothing timed, no measurement.
        assert_eq!(frame_gpu_ms(&[0, 0, 0, 0, 0, 0, 0, 0], 1, 1.0), None);
        let particles_first = [2_000_000, 9_000_000, 9_200_000, 14_000_000, 0, 0, 1_000_000, 1_900_000];
        assert_eq!(frame_gpu_ms(&particles_first, 1, 1.0), Some(13.0));
        // 3 samples: two more traces (untimed) between the first and the post pass, a 16.2 ms gap: the span. The post
        // pair of frame 458 under 3 samples: still out of place (trace 7.85 ms, gap 66.7 ms), counted as the passes.
        let three = [1_000_000, 9_000_000, 25_200_000, 31_000_000, 0, 0, 0, 0];
        assert_eq!(frame_gpu_ms(&three, 3, 1.0), Some(30.0));
        assert!((frame_gpu_ms(&t454, 3, 1.0).unwrap() - (3.0 * 7.849166 + 5.660166)).abs() < 1e-3);
    }
}

#[cfg(test)]
mod instanced_tests {
    use super::*;
    use crate::world::{Geometry, Kind};
    use glam::Mat4;

    /// Clicks pick what is drawn (ROADMAP § 6 step 5): an object added in place is picked, one removed in place no
    /// longer is, and a rebuild agrees. (Skipped without a GPU with hardware RT.)
    #[test]
    fn picks_follow_in_place_edits() {
        let g = match gpu::Gpu::new() {
            Ok(g) if g.hwrt => g,
            other => return eprintln!("skipped: {}", other.map_or_else(|e| e.to_string(), |_| "no hardware RT".into())),
        };
        let mut w = World::default();
        w.set_material(1, serde_json::from_value(serde_json::json!({ "name": "m", "color": [0.5, 0.5, 0.5], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true })).unwrap());
        let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: vec![0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
        w.set_geometry(2, quad(1.0)).unwrap();
        w.set_geometry(3, quad(0.5)).unwrap();
        let mesh = |geometry| Kind::Mesh { geometry, material: 1, own_blas: true, materials: vec![] };
        w.set_object(10, "a".into(), None, mesh(2), true, false).unwrap();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(4, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_transform(4, Vec3::new(0.0, 0.0, 5.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_settings(crate::world::Settings { camera: Some(4), ..Default::default() });
        let mut core = LiveCore::new(g, &w, Path::new("."), &crate::default_presets_dir(), 64, 36).unwrap();
        w.take_changes();
        let picked = |core: &LiveCore| core.pick(0.5, 0.5).map(|p| p.0);
        assert_eq!(picked(&core).as_deref(), Some("a"));
        // Added in place, in front of it: picked.
        w.set_object(11, "b".into(), None, mesh(3), true, false).unwrap();
        w.set_transform(11, Vec3::new(0.0, 0.0, 1.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        assert!(!core.commit(&mut w).unwrap(), "added in place");
        assert_eq!(picked(&core).as_deref(), Some("b"));
        assert_eq!(core.node_tris("b").len(), 2, "its triangles under its name");
        // Removed in place: the one behind it is picked again; both removed: nothing.
        w.remove(11);
        assert!(!core.commit(&mut w).unwrap(), "removed in place");
        assert_eq!(picked(&core).as_deref(), Some("a"));
        assert!(core.node_tris("b").is_empty(), "removed: none");
        w.remove(10);
        assert!(!core.commit(&mut w).unwrap(), "removed in place");
        assert_eq!(picked(&core), None);
        core.rebuild(&w).unwrap();
        assert_eq!(picked(&core), None, "a rebuild agrees");
    }

    /// A mesh a rigid body drives, added while the game runs with a material the scene did not have (ROADMAP § 6 step
    /// 7): placed in place, and its body's moves are its instance's transform, with no rebuild; a rebuild places it the
    /// same and it goes on moving in place. (Skipped without a GPU with hardware RT.)
    #[test]
    fn driven_meshes_added_in_place_move_as_instances() {
        let g = match gpu::Gpu::new() {
            Ok(g) if g.hwrt => g,
            other => return eprintln!("skipped: {}", other.map_or_else(|e| e.to_string(), |_| "no hardware RT".into())),
        };
        let mut w = World::default();
        let mat = |r: f32| serde_json::from_value(serde_json::json!({ "name": "m", "color": [r, r, r], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": true })).unwrap();
        w.set_material(1, mat(0.5));
        let quad = |s: f32| Geometry { positions: vec![-s, -s, 0.0, s, -s, 0.0, s, s, 0.0, -s, s, 0.0], normals: vec![0.0, 0.0, 1.0].repeat(4), uvs: vec![0.0; 8], indices: vec![0, 1, 2, 0, 2, 3], groups: vec![], colors: vec![] };
        w.set_geometry(2, quad(1.0)).unwrap();
        w.set_geometry(3, quad(0.5)).unwrap();
        let mesh = |geometry, material| Kind::Mesh { geometry, material, own_blas: true, materials: vec![] };
        w.set_object(10, "a".into(), None, mesh(2, 1), true, false).unwrap();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(4, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_transform(4, Vec3::new(0.0, 0.0, 5.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_settings(crate::world::Settings { camera: Some(4), ..Default::default() });
        let mut core = LiveCore::new(g, &w, Path::new("."), &crate::default_presets_dir(), 64, 36).unwrap();
        w.take_changes();
        let picked = |core: &LiveCore, u: f32| core.pick(u, 0.5).map(|p| p.0);
        // The box and its material arrive together, as scene code adds them; its body is told of it right after.
        let materials = core.sc.tables.materials.len();
        w.set_material(5, mat(0.9));
        w.set_object(11, "b".into(), None, mesh(3, 5), true, false).unwrap();
        w.set_driven(11).unwrap();
        w.set_transform(11, Vec3::new(0.0, 0.0, 1.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        assert!(!core.commit(&mut w).unwrap(), "added in place, its material too");
        assert_eq!(core.sc.tables.materials.len(), materials + 1);
        assert_eq!(picked(&core, 0.5).as_deref(), Some("b"));
        // Its body moves it 3 m to the side (u 0.95 of the image, 4 m from the camera): no rebuild, its record moved
        // with the rows it had the frame before.
        let record = core.sc.edit.placed[&11].1;
        assert!(w.place_driven(11, Vec3::new(3.0, 0.0, 1.0), glam::Quat::IDENTITY));
        assert!(!core.commit(&mut w).unwrap(), "moved as an instance");
        assert_eq!(core.fallbacks, 0);
        assert_eq!((picked(&core, 0.5).as_deref(), picked(&core, 0.95).as_deref()), (Some("a"), Some("b")));
        let r = core.sc.gpu_instances[record];
        assert_eq!((r.m[0][3], r.p[0][3]), (3.0, 0.0), "moved: last frame's rows kept");
        // A rebuild places it as an instance too, and it moves on in place.
        core.rebuild(&w).unwrap();
        assert!(core.sc.tables.retained.instance_of.contains_key(&11) && core.sc.edit.placed.is_empty());
        assert_eq!((picked(&core, 0.5).as_deref(), picked(&core, 0.95).as_deref()), (Some("a"), Some("b")));
        assert!(w.place_driven(11, Vec3::new(0.0, 0.0, 1.0), glam::Quat::IDENTITY));
        assert!(!core.commit(&mut w).unwrap(), "moved as an instance");
        assert_eq!((picked(&core, 0.5).as_deref(), picked(&core, 0.95)), (Some("b"), None));
        // Removed in place.
        w.remove(11);
        assert!(!core.commit(&mut w).unwrap(), "removed in place");
        assert_eq!(picked(&core, 0.5).as_deref(), Some("a"));
    }

    /// An instanced mesh's moved instances keep their previous-frame rows (motion vectors) through the commit that moves
    /// them, and catch up at the next commit that does not move them. (Skipped without a GPU with hardware RT.)
    #[test]
    fn moved_instances_keep_their_motion() {
        let g = match gpu::Gpu::new() {
            Ok(g) if g.hwrt => g,
            other => return eprintln!("skipped: {}", other.map_or_else(|e| e.to_string(), |_| "no hardware RT".into())),
        };
        let mut w = World::default();
        let mat = |r: f32| serde_json::from_value(serde_json::json!({ "name": "m", "color": [r, r, r], "opacity": 1, "metalness": 0, "roughness": 1, "emissive": [0, 0, 0], "emissiveIntensity": 0, "doubleSided": false })).unwrap();
        w.set_material(1, mat(0.5));
        w.set_geometry(2, Geometry { positions: vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0], normals: vec![0.0, 0.0, 1.0].repeat(3), uvs: vec![0.0; 6], indices: vec![0, 1, 2], groups: vec![], colors: vec![] }).unwrap();
        w.set_object(3, "copies".into(), None, Kind::Instanced { geometry: 2, procedural: false, materials: vec![1], capacity: 2 }, true, false).unwrap();
        let at = |x: f32| [Mat4::IDENTITY.to_cols_array(), Mat4::from_translation(Vec3::new(x, 0.0, 0.0)).to_cols_array()].concat();
        w.set_instances(3, 2, Some(at(1.0)), None, None).unwrap();
        let cam = serde_json::from_value(serde_json::json!({ "fov": 50, "near": 0.1, "far": 100, "exposure": { "mode": "manual", "ev100": 0 } })).unwrap();
        w.set_object(4, "camera".into(), None, Kind::Camera(cam), true, false).unwrap();
        w.set_transform(4, Vec3::new(0.0, 0.0, 5.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_settings(crate::world::Settings { camera: Some(4), ..Default::default() });
        let mut core = LiveCore::new(g, &w, Path::new("."), &crate::default_presets_dir(), 64, 36).unwrap();
        w.take_changes();
        let first = core.sc.tables.retained.instanced[&3].first as usize;
        w.set_instances(3, 2, Some(at(2.0)), None, None).unwrap();
        assert!(!core.commit(&mut w).unwrap(), "in place");
        let r = core.sc.gpu_instances[first + 1];
        assert_eq!((r.m[0][3], r.p[0][3]), (2.0, 1.0), "moved: last frame's rows kept");
        w.set_material(1, mat(0.6));
        core.commit(&mut w).unwrap();
        let r = core.sc.gpu_instances[first + 1];
        assert_eq!(r.p, r.m, "not moved since: caught up");
        // Movable, moved and given new instances in one commit: updated once (twice would lose the motion).
        w.set_object(3, "copies".into(), None, Kind::Instanced { geometry: 2, procedural: false, materials: vec![1], capacity: 2 }, true, true).unwrap();
        assert!(core.commit(&mut w).unwrap(), "made movable: rebuilt");
        let first = core.sc.tables.retained.instanced[&3].first as usize;
        w.set_transform(3, Vec3::new(0.0, 1.0, 0.0), glam::Quat::IDENTITY, Vec3::ONE).unwrap();
        w.set_instances(3, 2, Some(at(3.0)), None, None).unwrap();
        let c = w.changes.clone();
        assert!(c.moved.contains(&3) && c.instanced.contains(&3), "{c:?}");
        assert!(!core.commit(&mut w).unwrap(), "in place");
        let r = core.sc.gpu_instances[first + 1];
        assert_eq!((r.m[0][3], r.m[1][3], r.p[0][3], r.p[1][3]), (3.0, 1.0, 2.0, 0.0), "moved: last frame's rows kept");
    }
}
