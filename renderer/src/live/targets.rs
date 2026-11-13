//! The live passes' GPU side: the shader module and its pipelines (`Passes`), the screen-size buffers and the bind
//! group (`Targets`), and the tonemap LUT and bloom pyramid they are sized for.

use super::*;

/// Bloom pyramid level j (0..4: 1/4 .. 1/64 resolution) size; stage k (0..5) runs on level max(k - 1, 0).
pub(super) fn bloom_dim(w: u32, h: u32, j: u32) -> (u32, u32) {
    (w.div_ceil(4).div_ceil(1 << j), h.div_ceil(4).div_ceil(1 << j))
}

pub(super) fn bloom_stage_level(k: u32) -> u32 {
    k.max(1) - 1
}

/// UE Standard bloom per stage: Gaussian radius in texels of its level (BloomNSize % of the stage width * size scale
/// / 2, as `post::bloom`) and tint (BloomNTint * intensity / 6).
pub(super) fn bloom_stages(pp: &crate::desc::PostPreset, w: u32, h: u32) -> ([f32; 6], [f32; 6]) {
    let (mut r, mut t) = ([0f32; 6], [0f32; 6]);
    for k in 0..6 {
        let (lw, _) = bloom_dim(w, h, bloom_stage_level(k as u32));
        r[k] = lw as f32 * pp.bloom.sizes.get(k).copied().unwrap_or(0.0) * pp.bloom.size_scale * 0.01 * 0.5;
        t[k] = pp.bloom.tints.get(k).copied().unwrap_or(0.0) * pp.bloom.intensity / 6.0;
    }
    (r, t)
}

pub(super) const LOG_OFFSET: f32 = 0.0026883; // must match live.wgsl lin_to_log input offset
pub(super) const LUT: u32 = 32;

/// UE-style 32^3 LUT: log-encoded linear input -> display sRGB, built from the exact CPU port of the filmic tonemapper.
pub(super) fn tonemap_lut(p: &crate::desc::PostPreset) -> Vec<u16> {
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
    let src = include_str!("../../shaders/live.wgsl");
    if merged {
        crate::merged::live(src).into()
    } else {
        src.into()
    }
}

pub(super) struct Passes {
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) pipes: Vec<(&'static str, wgpu::ComputePipeline)>,
}

impl Passes {
    pub(super) fn new(g: &gpu::Gpu) -> Passes {
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
        entries.extend([buf(1, true), buf(2, false), buf(3, false), buf(4, false), buf(5, false), buf(6, false), buf(7, false), buf(8, false), buf(9, false), buf(10, false)]);
        entries.push(wgpu::BindGroupLayoutEntry {
            binding: 11,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
            count: None,
        });
        entries.push(wgpu::BindGroupLayoutEntry { binding: 12, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None });
        entries.extend([buf(13, false), buf(14, false), buf(15, false), buf(16, false), buf(17, false), buf(18, false)]);
        entries.extend([buf(19, false), buf(20, false), buf(21, false), buf(22, false), buf(23, true), buf(24, false), buf(25, true), buf(26, true)]);
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
                (name, d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(name), layout: Some(&pl), module: &module, entry_point: Some(name), compilation_options: Default::default(), cache: g.cache() }))
            })
            .collect();
        g.save_pipeline_cache();
        Passes { layout, pipes }
    }
}

/// Screen-size targets shared across scene reloads.
pub struct Targets {
    pub(super) params: wgpu::Buffer,
    /// What lasts from one frame to the next (live.wgsl, "The live passes' buffers"), two halves each: this frame's
    /// and the previous frame's (`LiveParams::state`). Per traced sample: the diffuse irradiance's history, its
    /// luminance moments and its fast mean in colour, the specular history, the g-buffer, the upscaler's state; per
    /// output pixel: the colour history.
    pub(super) diffuse_hist: Buf,
    pub(super) diffuse_mom: Buf,
    pub(super) diffuse_fast: Buf,
    pub(super) spec_hist: Buf,
    pub(super) gbuffer: Buf,
    pub(super) up_state: Buf,
    pub(super) color_hist: Buf,
    /// What lives within a frame (live.wgsl says who uses each when): per traced sample, screen velocity, view depth
    /// and animated shading; the specular filter's two slots; the scratch the diffuse filter and the upscaled image
    /// share; the irradiance filter's ping-pong, the composed colour and the image; bloom's pyramid and scratch.
    pub(super) sample_vel: Buf,
    pub(super) spec_pp: Buf,
    pub(super) sb: Buf,
    pub(super) fa: Buf,
    pub(super) fb: Buf,
    pub(super) bloom: [Buf; 2],
    /// Reflection resolve: the resolved specular reflection per traced pixel (live.wgsl `refl`), from the trace
    /// kernel's specular-lobe rays (2 vec4 per traced pixel, pathtrace.wgsl `refl`: `kernel.refl`).
    pub(super) refl_res: Buf,
    pub out: wgpu::Buffer,
    /// Auto exposure: 64 histogram buckets (fixed point), state (exposure, target, mean log2 luminance, valid).
    pub(super) ae_bins: wgpu::Buffer,
    pub(super) ae_state: Buf,
    /// Quarter-resolution log2 luminance and its edge-aware blur (local exposure base).
    pub(super) lumq: Buf,
    pub(super) lumb: Buf,
    /// The kernel's group 1 (`gpu::KernelBufs`): the radiance cache, and what the passes here read of
    /// a trace: the specular-lobe rays (`refl`), motion vectors per traced pixel (pathtrace.wgsl `motion_out`) and the
    /// deterministic direct light per traced pixel (`direct_out`).
    pub kernel: gpu::KernelBufs,
    /// The merged layout (merged.rs): the slabs the buffers above are ranges of (`SLABS`' first four).
    pub(super) slabs: Option<[wgpu::Buffer; 4]>,
    pub(super) lut_tex: wgpu::Texture,
    pub(super) lut_view: wgpu::TextureView,
    pub(super) sampler: wgpu::Sampler,
}

impl Targets {
    /// For a view of `w` x `h` and a radiance cache of `cache` entries.
    pub(super) fn new(g: &gpu::Gpu, w: u32, h: u32, lut: &[u16], cache: u64) -> Targets {
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
        // (The lasting ones are two halves of `n` elements: this frame's and the previous frame's.)
        let specs: [(&str, u64, usize); 18] = [
            ("diffuse_hist", 2 * n * 16, SLAB_V4),
            ("diffuse_mom", 2 * n * 16, SLAB_V4),
            ("diffuse_fast", 2 * n * 8, SLAB_U2),
            ("spec_hist", 2 * n * 16, SLAB_V4),
            ("gbuffer", 2 * n * 8, SLAB_U2),
            ("up_state", 2 * n * 16, SLAB_U4),
            ("color_hist", 2 * n * 16, SLAB_V4),
            ("sample_vel", n * 16, SLAB_V4),
            ("spec_pp", n * 16, SLAB_U2), // two slots of `n` (vec2<u32>)
            ("sb", n * 16, SLAB_V4),
            ("fa", n * 16, SLAB_V4),
            ("fb", n * 16, SLAB_V4),
            ("bloom_a", bloom_texels * 16, SLAB_V4),
            ("bloom_b", bn, SLAB_V4),
            ("refl_res", n * 16, SLAB_V4),
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
            diffuse_hist: b(),
            diffuse_mom: b(),
            diffuse_fast: b(),
            spec_hist: b(),
            gbuffer: b(),
            up_state: b(),
            color_hist: b(),
            sample_vel: b(),
            spec_pp: b(),
            sb: b(),
            fa: b(),
            fb: b(),
            bloom: [b(), b()],
            refl_res: b(),
            out: sb("out", n * 4),
            ae_bins: sb("ae_bins", 64 * 4),
            ae_state: b(),
            lumq: b(),
            lumb: b(),
            kernel: gpu::KernelBufs::new(d, g.merged, g.guard_room(), cache, n * 32, n * 16, n * 16),
            slabs,
            lut_view: lut_tex.create_view(&Default::default()),
            lut_tex,
            sampler: d.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() }),
        }
    }

    /// What live.wgsl's buffer bindings hold (besides the uniforms at 0, the trace's samples at 1, `out` at 10 and
    /// the histogram at 13).
    pub(super) fn buffers(&self) -> [(u32, &Buf); 21] {
        [
            (2, &self.diffuse_hist),
            (3, &self.color_hist),
            (4, &self.gbuffer),
            (5, &self.spec_hist),
            (6, &self.diffuse_mom),
            (7, &self.diffuse_fast),
            (8, &self.up_state),
            (9, &self.sample_vel),
            (14, &self.ae_state),
            (15, &self.lumq),
            (16, &self.lumb),
            (17, &self.fa),
            (18, &self.fb),
            (19, &self.bloom[0]),
            (20, &self.bloom[1]),
            (21, &self.spec_pp),
            (22, &self.sb),
            (23, &self.kernel.refl),
            (24, &self.refl_res),
            (25, &self.kernel.motion),
            (26, &self.kernel.direct),
        ]
    }

    /// The live passes' bind group. (The lasting buffers' halves alternate by `LiveParams::state`, not by binding.) In
    /// the merged layout (merged.rs) it binds the slabs, and the offsets the passes read (`LO`).
    pub(super) fn bind_group(&self, d: &wgpu::Device, layout: &wgpu::BindGroupLayout, sample: &wgpu::Buffer) -> wgpu::BindGroup {
        let mk = || {
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
            let buffers = self.buffers();
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
        mk()
    }
}
