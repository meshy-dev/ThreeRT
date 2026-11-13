//! The live passes' GPU side: the shader module and its pipelines (`Passes`), the screen-size buffers and the bind
//! groups (`Targets`), from the layout live.wgsl declares and `layout`'s table; and the tonemap LUT and bloom pyramid
//! they are sized for.

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

/// The live passes' source: live.wgsl, natively and in a browser alike (each pass binds at most a browser's storage
/// buffers: `layout`).
pub(crate) fn live_source() -> &'static str {
    include_str!("../../shaders/live.wgsl")
}

pub(super) struct Passes {
    /// Each pass's bind group layout: the bindings it uses.
    pub(super) layouts: Vec<wgpu::BindGroupLayout>,
    /// The passes in `layout::PASSES`' order, with the grid each runs over.
    pub(super) pipes: Vec<(&'static str, layout::Grid, wgpu::ComputePipeline)>,
}

/// A binding's layout entry (live.wgsl's kind of it).
fn layout_entry(b: &layout::Binding) -> wgpu::BindGroupLayoutEntry {
    let ty = match b.kind {
        layout::Kind::Uniform => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        layout::Kind::Storage { read_only, .. } => wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
        layout::Kind::Texture(view_dimension) => wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension, multisampled: false },
        layout::Kind::Sampler => wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
    };
    wgpu::BindGroupLayoutEntry { binding: b.binding, visibility: wgpu::ShaderStages::COMPUTE, ty, count: None }
}

impl Passes {
    pub(super) fn new(g: &gpu::Gpu) -> Passes {
        layout::check();
        if g.merged {
            // (A browser's stage binds at most eight storage buffers; the device here is held to them.)
            let over: Vec<String> = layout::PASSES.iter().map(|p| (p.0, layout::storage_count(p.0))).filter(|p| p.1 as u32 > crate::merged::STORAGE_BUFFERS).map(|p| format!("{} binds {}", p.0, p.1)).collect();
            assert!(over.is_empty(), "live passes over a browser's storage buffers: {}", over.join(", "));
        }
        let d = &g.device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("live"), source: wgpu::ShaderSource::Wgsl(live_source().into()) });
        let pipe = |name: &str, layout: &wgpu::BindGroupLayout| {
            let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(layout)], immediate_size: 0 });
            d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: Some(name), layout: Some(&pl), module: &module, entry_point: Some(name), compilation_options: Default::default(), cache: g.cache() })
        };
        // A layout a pass, of the bindings it uses.
        let mut layouts = vec![];
        let mut pipes = vec![];
        for &(name, grid) in layout::PASSES {
            let entries: Vec<wgpu::BindGroupLayoutEntry> = layout::uses(name).iter().map(|&b| layout_entry(layout::BINDINGS.iter().find(|x| x.binding == b).expect("a binding of live.wgsl"))).collect();
            let l = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some(name), entries: &entries });
            pipes.push((name, grid, pipe(name, &l)));
            layouts.push(l);
        }
        g.save_pipeline_cache();
        Passes { layouts, pipes }
    }

    /// The layout pass `k` binds.
    pub(super) fn layout(&self, k: usize) -> &wgpu::BindGroupLayout {
        &self.layouts[k]
    }
}

/// Screen-size targets shared across scene reloads.
pub struct Targets {
    pub(super) params: wgpu::Buffer,
    /// The passes' own buffers (`layout::RESOURCES`' `Own`), by binding: what each holds and when is live.wgsl's
    /// table ("The live passes' buffers").
    pub(super) bufs: Vec<(u32, Buf)>,
    /// The output (`out_px`: sRGB BGRA8 a pixel), what the hosts draw.
    pub out: wgpu::Buffer,
    /// The kernel's group 1 (`gpu::KernelBufs`): the radiance cache, and what the passes here read of
    /// a trace: the specular-lobe rays (`refl`), motion vectors per traced pixel (pathtrace.wgsl `motion_out`) and the
    /// deterministic direct light per traced pixel (`direct_out`).
    pub kernel: gpu::KernelBufs,
    pub(super) lut_tex: wgpu::Texture,
    pub(super) lut_view: wgpu::TextureView,
    pub(super) sampler: wgpu::Sampler,
}

impl Targets {
    /// For a view of `w` x `h` and a radiance cache of `cache` entries.
    pub(super) fn new(g: &gpu::Gpu, w: u32, h: u32, lut: &[u16], cache: u64) -> Targets {
        let d = &g.device;
        let n = (w * h) as u64;
        let sb = |label: &str, bytes: u64| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: bytes, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false });
        let quarter = (w.div_ceil(4) * h.div_ceil(4)) as u64;
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
        // The passes' own buffers, in binding order.
        let bufs: Vec<(u32, Buf)> = layout::BINDINGS
            .iter()
            .filter_map(|b| {
                let layout::Res::Own(count) = layout::resource(b) else { return None };
                let layout::Kind::Storage { stride, len, .. } = b.kind else { unreachable!("layout::check") };
                let elements = match count {
                    layout::Count::Pixels(k) => k * n,
                    layout::Count::Quarter => quarter,
                    layout::Count::Bloom => bloom_texels,
                    layout::Count::Declared => len,
                };
                Some((b.binding, Buf::whole(sb(b.name, elements * stride))))
            })
            .collect();
        let out = bufs.iter().find(|b| b.0 == layout::binding("out_px").binding).expect("out_px").1.buffer.clone();
        Targets {
            params: d.create_buffer(&wgpu::BufferDescriptor { label: Some("live params"), size: std::mem::size_of::<LiveParams>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            bufs,
            out,
            kernel: gpu::KernelBufs::new(d, g.merged, g.guard_room(), cache, n * 32, n * 16, n * 16),
            lut_view: lut_tex.create_view(&Default::default()),
            lut_tex,
            sampler: d.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() }),
        }
    }

    /// The buffer live.wgsl binds as `name` (the passes' own, or the kernel's).
    pub(super) fn buf(&self, name: &str) -> &Buf {
        self.buf_at(layout::binding(name))
    }

    fn buf_at(&self, b: &layout::Binding) -> &Buf {
        match layout::resource(b) {
            layout::Res::KernelRefl => &self.kernel.refl,
            layout::Res::KernelMotion => &self.kernel.motion,
            layout::Res::KernelDirect => &self.kernel.direct,
            layout::Res::Own(_) => &self.bufs.iter().find(|x| x.0 == b.binding).expect("an own buffer").1,
            r => panic!("`{}` is bound to {r:?}, not a buffer of the passes'", b.name),
        }
    }

    /// The bind group of each pass, in `Passes::pipes`' order (`sample`: the trace's samples).
    pub(super) fn bind_groups(&self, d: &wgpu::Device, passes: &Passes, sample: &wgpu::Buffer) -> Vec<wgpu::BindGroup> {
        let entry = |b: &'_ layout::Binding| -> wgpu::BindGroupEntry<'_> {
            let resource = match layout::resource(b) {
                layout::Res::Params => self.params.as_entire_binding(),
                layout::Res::Samples => sample.as_entire_binding(),
                layout::Res::Lut => wgpu::BindingResource::TextureView(&self.lut_view),
                layout::Res::LutSampler => wgpu::BindingResource::Sampler(&self.sampler),
                _ => self.buf_at(b).binding(),
            };
            wgpu::BindGroupEntry { binding: b.binding, resource }
        };
        passes
            .pipes
            .iter()
            .enumerate()
            .map(|(k, (name, ..))| {
                let entries: Vec<wgpu::BindGroupEntry> = layout::uses(name).iter().map(|&b| entry(layout::BINDINGS.iter().find(|x| x.binding == b).expect("a binding of live.wgsl"))).collect();
                d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some(name), layout: passes.layout(k), entries: &entries })
            })
            .collect()
    }
}
