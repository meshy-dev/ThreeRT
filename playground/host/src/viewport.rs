//! Viewport compositor. Fills the viewport node's texture (exact physical size of its painted rect) with the live
//! image from LiveCore's BGRA8 storage buffer, letterboxed to the render aspect; optionally A/B against a UE5
//! reference image (full, or side by side with a split line); and draws the UE-style selection outline from a
//! silhouette mask rasterized from the selected node's world-space triangles.

use threert_render::live::Camera;
use glam::{Vec3, Vec3A};

pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// What the viewport shows: the live render, or the live render split against a UE5 reference at `split`
/// (0..1 of the image width; live on the left).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Compare {
    Live,
    SplitLumen,
    SplitPathTracer,
    /// three.js r187 rendering the same scene code (tools/bench-three.ts).
    SplitThree,
}

impl Compare {
    pub fn parse(s: &str) -> Option<Compare> {
        Some(match s {
            "live" => Compare::Live,
            "split-lumen" => Compare::SplitLumen,
            "split-pathtracer" => Compare::SplitPathTracer,
            "split-three" => Compare::SplitThree,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Compare::Live => "live",
            Compare::SplitLumen => "split-lumen",
            Compare::SplitPathTracer => "split-pathtracer",
            Compare::SplitThree => "split-three",
        }
    }
    /// Reference slot (0 UE5 Lumen, 1 UE5 Path Tracer, 2 three.js) shown by this mode.
    fn reference(self) -> Option<usize> {
        match self {
            Compare::Live => None,
            Compare::SplitLumen => Some(0),
            Compare::SplitPathTracer => Some(1),
            Compare::SplitThree => Some(2),
        }
    }
    fn code(self) -> f32 {
        if self == Compare::Live { 0.0 } else { 2.0 }
    }
}

pub struct Target {
    _texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub size: (u32, u32),
    mask: wgpu::TextureView,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ComposeU {
    /// viewport w, h; image rect x0, y0 (physical px)
    dst: [f32; 4],
    /// image rect w, h; source (live) w, h
    img: [f32; 4],
    /// mode (0 live, 2 split), split (0..1), outline on, outline width px
    mode: [f32; 4],
    outline: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaskU {
    rows: [[f32; 4]; 4],
}

pub struct Viewport {
    pub target: Option<Target>,
    compose: wgpu::RenderPipeline,
    compose_layout: wgpu::BindGroupLayout,
    mask_pipe: wgpu::RenderPipeline,
    mask_bg: wgpu::BindGroup,
    compose_u: wgpu::Buffer,
    mask_u: wgpu::Buffer,
    refs: [Option<(wgpu::TextureView, (u32, u32))>; 3],
    dummy: wgpu::TextureView,
    sampler: wgpu::Sampler,
    selection: Option<(wgpu::Buffer, u32)>,
    pub compare: Compare,
    pub split: f32,
    pub outline: bool,
    /// Selection outline opacity (fades in on a new selection).
    pub outline_alpha: f32,
}

fn entry(binding: u32, vis: wgpu::ShaderStages, ty: wgpu::BindingType) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry { binding, visibility: vis, ty, count: None }
}

fn texture_2d(d: &wgpu::Device, label: &str, size: (u32, u32), format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
    d.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: size.0.max(1), height: size.1.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    })
}

/// Uploads an RGBA8 image as a sampled texture (sRGB-encoded bytes, sampled raw like the live image).
pub fn upload_rgba(d: &wgpu::Device, q: &wgpu::Queue, label: &str, img: &image::RgbaImage) -> wgpu::TextureView {
    let size = (img.width(), img.height());
    let t = texture_2d(d, label, size, FORMAT, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
    q.write_texture(
        wgpu::TexelCopyTextureInfo { texture: &t, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
        img.as_raw(),
        wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(size.0 * 4), rows_per_image: Some(size.1) },
        wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
    );
    t.create_view(&Default::default())
}

/// Largest rect of aspect `aspect` centered in (w, h): (x0, y0, w, h).
pub fn letterbox(w: f32, h: f32, aspect: f32) -> [f32; 4] {
    let (iw, ih) = if w / h > aspect { (h * aspect, h) } else { (w, w / aspect) };
    [((w - iw) * 0.5).floor(), ((h - ih) * 0.5).floor(), iw.round(), ih.round()]
}

impl Viewport {
    pub fn new(d: &wgpu::Device, q: &wgpu::Queue) -> Viewport {
        let storage = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None };
        let uniform = wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None };
        let tex = |filterable| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
        let fs = wgpu::ShaderStages::FRAGMENT;
        let compose_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewport compose"),
            entries: &[
                entry(0, fs, storage),
                entry(1, fs, uniform),
                entry(2, fs, tex(true)),
                entry(3, fs, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
                entry(4, fs, tex(false)),
            ],
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("viewport"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
        let pl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&compose_layout)], immediate_size: 0 });
        let compose = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewport compose"),
            layout: Some(&pl),
            vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_full"), buffers: &[], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_compose"), targets: &[Some(FORMAT.into())], compilation_options: Default::default() }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("selection mask"), source: wgpu::ShaderSource::Wgsl(MASK_SHADER.into()) });
        let mask_layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("selection mask"), entries: &[entry(0, wgpu::ShaderStages::VERTEX, uniform)] });
        let mpl = d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(&mask_layout)], immediate_size: 0 });
        let mask_pipe = d.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("selection mask"),
            layout: Some(&mpl),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_mask"),
                buffers: &[Some(wgpu::VertexBufferLayout { array_stride: 16, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x4] })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs_mask"), targets: &[Some(MASK_FORMAT.into())], compilation_options: Default::default() }),
            primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let buf = |label, size| d.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let compose_u = buf("viewport u", std::mem::size_of::<ComposeU>() as u64);
        let mask_u = buf("mask u", std::mem::size_of::<MaskU>() as u64);
        let mask_bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &mask_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: mask_u.as_entire_binding() }] });
        let dummy = upload_rgba(d, q, "no reference", &image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255])));
        let sampler = d.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
        Viewport { target: None, compose, compose_layout, mask_pipe, mask_bg, compose_u, mask_u, refs: [None, None, None], dummy, sampler, selection: None, compare: Compare::Live, split: 0.5, outline: true, outline_alpha: 1.0 }
    }

    /// Reference images for the open scene (UE5 Lumen, UE5 Path Tracer, three.js); missing files leave the slot empty.
    pub fn set_references(&mut self, d: &wgpu::Device, q: &wgpu::Queue, paths: [Option<&str>; 3]) -> Vec<String> {
        let mut log = vec![];
        for (k, p) in paths.iter().enumerate() {
            self.refs[k] = p.and_then(|p| match image::open(p) {
                Ok(img) => {
                    let img = img.to_rgba8();
                    let size = img.dimensions();
                    Some((upload_rgba(d, q, "ue reference", &img), size))
                }
                Err(e) => {
                    log.push(format!("reference {p}: {e}"));
                    None
                }
            });
        }
        log
    }

    pub fn has_reference(&self, c: Compare) -> bool {
        c.reference().is_none_or(|k| self.refs[k].is_some())
    }

    /// World-space triangles of the selection (None clears the outline).
    pub fn set_selection(&mut self, d: &wgpu::Device, tris: Option<&[[Vec3A; 3]]>) {
        self.selection = tris.filter(|t| !t.is_empty()).map(|t| {
            let verts: Vec<[f32; 4]> = t.iter().flat_map(|tri| tri.iter().map(|v| [v.x, v.y, v.z, 1.0])).collect();
            let b = d.create_buffer(&wgpu::BufferDescriptor { label: Some("selection"), size: (verts.len() * 16) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: true });
            b.slice(..).get_mapped_range_mut().expect("mapped at creation").copy_from_slice(bytemuck::cast_slice(&verts));
            b.unmap();
            (b, verts.len() as u32)
        });
    }

    /// (Re)creates the viewport texture at exactly `size` physical pixels.
    pub fn ensure_target(&mut self, d: &wgpu::Device, size: (u32, u32)) {
        if self.target.as_ref().map(|t| t.size) == Some(size) {
            return;
        }
        let texture = texture_2d(d, "viewport", size, FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC);
        let mask = texture_2d(d, "selection mask", size, MASK_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        self.target = Some(Target { view: texture.create_view(&Default::default()), _texture: texture, size, mask: mask.create_view(&Default::default()) });
    }

    /// Image rect (physical px within the viewport texture) for a source of aspect `aspect`.
    pub fn image_rect(&self, aspect: f32) -> [f32; 4] {
        let (w, h) = self.target.as_ref().map_or((1, 1), |t| t.size);
        letterbox(w as f32, h as f32, aspect)
    }

    /// Writes this frame's uniforms. `live` = (source width, height); `cam` = camera the live frame is rendered with.
    pub fn prepare(&self, q: &wgpu::Queue, live: (u32, u32), cam: &Camera, aspect: f32) {
        let Some(t) = &self.target else { return };
        let r = self.image_rect(aspect);
        let outline = self.outline && self.selection.is_some();
        let u = ComposeU {
            dst: [t.size.0 as f32, t.size.1 as f32, r[0], r[1]],
            img: [r[2], r[3], live.0 as f32, live.1 as f32],
            mode: [self.compare.code(), self.split, outline as u32 as f32, 2.0],
            // UE5 selection orange
            outline: [0.96, 0.62, 0.04, self.outline_alpha],
        };
        q.write_buffer(&self.compose_u, 0, bytemuck::bytes_of(&u));
        // Clip transform matching the path tracer's pinhole: x = right / (tan * aspect), y = up / tan, w = forward.
        let (n, f) = (0.01f32, 10000.0f32);
        let (a, b) = (f / (f - n), -f * n / (f - n));
        let row = |v: Vec3, s: f32| [v.x * s, v.y * s, v.z * s, -v.dot(cam.pos) * s];
        let mut z = row(cam.fwd, a);
        z[3] += b;
        let m = MaskU { rows: [row(cam.right, 1.0 / (cam.tan_half_fov * aspect)), row(cam.up, 1.0 / cam.tan_half_fov), z, row(cam.fwd, 1.0)] };
        q.write_buffer(&self.mask_u, 0, bytemuck::bytes_of(&m));
    }

    /// Records the mask + compose passes into `e` (after LiveCore's post passes, so `out` holds this frame).
    pub fn encode(&self, d: &wgpu::Device, e: &mut wgpu::CommandEncoder, out: &wgpu::Buffer, aspect: f32) {
        let Some(t) = &self.target else { return };
        let r = self.image_rect(aspect);
        {
            let mut rp = e.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("selection mask"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.mask, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some((vb, n)) = &self.selection {
                if r[2] >= 1.0 && r[3] >= 1.0 {
                    rp.set_viewport(r[0], r[1], r[2], r[3], 0.0, 1.0);
                    rp.set_pipeline(&self.mask_pipe);
                    rp.set_bind_group(0, &self.mask_bg, &[]);
                    rp.set_vertex_buffer(0, vb.slice(..));
                    rp.draw(0..*n, 0..1);
                }
            }
        }
        let reference = self.compare.reference().and_then(|k| self.refs[k].as_ref()).map_or(&self.dummy, |(v, _)| v);
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.compose_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: out.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: self.compose_u.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(reference) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(&t.mask) },
            ],
        });
        let mut rp = e.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewport compose"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        rp.set_pipeline(&self.compose);
        rp.set_bind_group(0, &bg, &[]);
        rp.draw(0..3, 0..1);
    }

    /// Clears the viewport (no scene loaded yet).
    pub fn clear(&self, e: &mut wgpu::CommandEncoder) {
        let Some(t) = &self.target else { return };
        let _ = e.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("viewport clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.02, g: 0.02, b: 0.02, a: 1.0 }), store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
}

const SHADER: &str = r#"
struct ComposeU { dst: vec4<f32>, img: vec4<f32>, mode: vec4<f32>, outline: vec4<f32> };
@group(0) @binding(0) var<storage, read> px: array<u32>;
@group(0) @binding(1) var<uniform> u: ComposeU;
@group(0) @binding(2) var refimg: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;
@group(0) @binding(4) var mask: texture_2d<f32>;

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
}

fn texel(x: i32, y: i32) -> vec4<f32> {
    let w = i32(u.img.z);
    let h = i32(u.img.w);
    let c = px[u32(clamp(y, 0, h - 1) * w + clamp(x, 0, w - 1))];
    return vec4<f32>(f32((c >> 16u) & 255u), f32((c >> 8u) & 255u), f32(c & 255u), 255.0) / 255.0;
}

fn live(uv: vec2<f32>) -> vec4<f32> {
    let q = uv * u.img.zw - 0.5;
    let f = fract(q);
    let i = vec2<i32>(floor(q));
    let a = mix(texel(i.x, i.y), texel(i.x + 1, i.y), f.x);
    let b = mix(texel(i.x, i.y + 1), texel(i.x + 1, i.y + 1), f.x);
    return mix(a, b, f.y);
}

fn m(p: vec2<i32>) -> f32 {
    // Clamped to the image rect: objects cut by the image border get no outline along it.
    let lo = vec2<i32>(u.dst.zw);
    let hi = lo + vec2<i32>(u.img.xy) - 1;
    return textureLoad(mask, clamp(p, lo, hi), 0).r;
}

@fragment fn fs_compose(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let q = p.xy - u.dst.zw;
    if (q.x < 0.0 || q.y < 0.0 || q.x >= u.img.x || q.y >= u.img.y) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let uv = q / u.img.xy;
    let mode = u32(u.mode.x);
    let show_ref = mode == 2u && uv.x > u.mode.y;
    var c: vec4<f32>;
    if (show_ref) {
        c = textureSampleLevel(refimg, samp, uv, 0.0);
    } else {
        c = live(uv);
    }
    if (mode == 2u && abs(q.x - u.mode.y * u.img.x) < 1.5) {
        c = vec4<f32>(0.85, 0.85, 0.85, 1.0);
    }
    if (u.mode.z > 0.5 && !show_ref) {
        // Silhouette edge: the mask differs within the outline radius.
        let ip = vec2<i32>(p.xy);
        let r = i32(u.mode.w);
        let c0 = m(ip);
        var edge = false;
        for (var dy = -r; dy <= r; dy++) {
            for (var dx = -r; dx <= r; dx++) {
                if (dx * dx + dy * dy <= r * r + 1 && m(ip + vec2<i32>(dx, dy)) != c0) {
                    edge = true;
                }
            }
        }
        if (edge) {
            c = vec4<f32>(mix(c.rgb, u.outline.rgb, u.outline.w), 1.0);
        } else if (c0 > 0.5) {
            c = vec4<f32>(mix(c.rgb, u.outline.rgb, 0.06 * u.outline.w), 1.0);
        }
    }
    return vec4<f32>(c.rgb, 1.0);
}

"#;

const MASK_SHADER: &str = r#"
struct MaskU { r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, r3: vec4<f32> };
@group(0) @binding(0) var<uniform> mu: MaskU;
@vertex fn vs_mask(@location(0) pos: vec4<f32>) -> @builtin(position) vec4<f32> {
    return vec4<f32>(dot(mu.r0, pos), dot(mu.r1, pos), dot(mu.r2, pos), dot(mu.r3, pos));
}
@fragment fn fs_mask() -> @location(0) vec4<f32> {
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}
"#;
