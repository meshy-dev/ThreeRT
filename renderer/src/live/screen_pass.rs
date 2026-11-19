//! The game's optional full-screen pass. It owns its input so a refracted sample always sees the completed scene.
use super::LiveCore;
use crate::{gpu::Gpu, scene, world::World};
use anyhow::{ensure, Context, Result};
use std::{path::Path, sync::Arc};
use wgpu::{BindingResource, BindingType, BufferBindingType};

#[derive(Clone)]
pub struct ScreenShader {
    source: Arc<str>,
    pub(crate) textures: Vec<u32>,
}

impl ScreenShader {
    pub(crate) fn new(code: &str, textures: Vec<u32>) -> Result<Self> {
        use std::fmt::Write;
        let mut source = include_str!("../../shaders/screen_pass.wgsl").to_owned();
        for i in 0..textures.len() {
            writeln!(source, "@group(0) @binding({}) var tex{i}: texture_2d<f32>;", 3 + 2 * i)?;
            writeln!(source, "@group(0) @binding({}) var sampler{i}: sampler;", 4 + 2 * i)?;
            writeln!(source, "fn sample{i}(uv: vec2f) -> vec4f {{ let t = screen.textureUV[{i}]; return textureSampleLevel(tex{i}, sampler{i}, uv * t.xy + t.zw, 0.0); }}")?;
        }
        source.push_str(code);
        let module = naga::front::wgsl::parse_str(&source).map_err(|e| anyhow::anyhow!("postPass: {}", e.emit_to_string(&source)))?;
        ensure!(
            module.entry_points.len() == 1
                && module.overrides.is_empty()
                && module.global_variables.iter().all(|(_, v)| v.space != naga::AddressSpace::WorkGroup)
                && module.global_variables.iter().filter(|(_, v)| v.binding.is_some()).count() == 3 + textures.len() * 2,
            "postPass: supply a post function and helpers, with the engine's bindings and entry point"
        );
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
            .validate(&module)
            .map_err(|e| anyhow::anyhow!("postPass: {}", e.emit_to_string(&source)))?;
        Ok(Self { source: source.into(), textures })
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    size: [f32; 2],
    time: f32,
    pad: f32,
    params: [f32; 32],
    texture_uv: [[f32; 4]; 4],
}

pub(super) struct ScreenPass {
    shader: Arc<ScreenShader>,
    pipe: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    values: Uniforms,
    textures: Vec<(wgpu::TextureView, wgpu::Sampler)>,
    pub(super) input: wgpu::Buffer,
    group: wgpu::BindGroup,
}

impl LiveCore {
    pub(super) fn sync_screen_pass(&mut self, world: &World) -> Result<()> {
        let Some(shader) = &world.post_pass else {
            if self.screen_pass.take().is_some() {
                self.targets.bind_output(&self.g.device, &self.passes, self.targets.out.clone());
            }
            return Ok(());
        };
        if let Some(pass) = self.screen_pass.as_mut().filter(|p| Arc::ptr_eq(shader, &p.shader)) {
            pass.values.params = world.post_params;
            return Ok(());
        }
        let pass = ScreenPass::new(&self.g, world, shader.clone(), &self.assets, &self.targets.out)?;
        self.targets.bind_output(&self.g.device, &self.passes, pass.input.clone());
        self.screen_pass = Some(pass);
        Ok(())
    }
}

impl ScreenPass {
    fn new(g: &Gpu, world: &World, shader: Arc<ScreenShader>, assets: &Path, out: &wgpu::Buffer) -> Result<Self> {
        let d = &g.device;
        let mut values = Uniforms { params: world.post_params, ..Default::default() };
        let mut textures = Vec::new();
        for (i, id) in shader.textures.iter().enumerate() {
            let t = world.textures.get(id).context("postPass: missing texture")?;
            let (_, img) = scene::texture_image(t, assets)?;
            let (width, height) = img.dimensions();
            let max = d.limits().max_texture_dimension_2d;
            ensure!(
                (1..=max).contains(&width) && (1..=max).contains(&height),
                "postPass: texture {i} size {width} x {height} exceeds device limits"
            );
            let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
            let tex = d.create_texture(&wgpu::TextureDescriptor {
                label: Some("game post texture"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: if t.srgb { wgpu::TextureFormat::Rgba8UnormSrgb } else { wgpu::TextureFormat::Rgba8Unorm },
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            g.queue.write_texture(
                tex.as_image_copy(),
                img.as_raw(),
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
                size,
            );
            let mode = if t.wrap { wgpu::AddressMode::Repeat } else { wgpu::AddressMode::ClampToEdge };
            let sampler = d.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: mode,
                address_mode_v: mode,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });
            textures.push((tex.create_view(&Default::default()), sampler));
            values.texture_uv[i] = [t.repeat[0], t.repeat[1], t.offset[0], t.offset[1]];
        }

        let mut entries = Vec::new();
        let mut entry = |ty| {
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: entries.len() as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty,
                count: None,
            })
        };
        for ty in
            [BufferBindingType::Uniform, BufferBindingType::Storage { read_only: true }, BufferBindingType::Storage { read_only: false }]
        {
            entry(BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None });
        }
        for _ in &textures {
            entry(BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            });
            entry(BindingType::Sampler(wgpu::SamplerBindingType::Filtering));
        }
        let label = Some("game post");
        let layout = d.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label, entries: &entries });
        let pl =
            d.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label, bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label,
            source: wgpu::ShaderSource::Wgsl(shader.source.as_ref().into()),
        });
        let pipe = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label,
            layout: Some(&pl),
            module: &module,
            entry_point: Some("screenMain"),
            compilation_options: Default::default(),
            cache: g.cache(),
        });
        let uniform = d.create_buffer(&wgpu::BufferDescriptor {
            label,
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let (input, group) = Self::bind(d, out, &layout, &uniform, &textures);
        Ok(Self { shader, pipe, layout, uniform, values, textures, input, group })
    }

    fn bind(
        d: &wgpu::Device,
        out: &wgpu::Buffer,
        layout: &wgpu::BindGroupLayout,
        uniform: &wgpu::Buffer,
        textures: &[(wgpu::TextureView, wgpu::Sampler)],
    ) -> (wgpu::Buffer, wgpu::BindGroup) {
        let input = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("game post scene color"),
            size: out.size(),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let mut entries = vec![
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: input.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: out.as_entire_binding() },
        ];
        for (i, (view, sampler)) in textures.iter().enumerate() {
            entries.push(wgpu::BindGroupEntry { binding: 3 + 2 * i as u32, resource: BindingResource::TextureView(view) });
            entries.push(wgpu::BindGroupEntry { binding: 4 + 2 * i as u32, resource: BindingResource::Sampler(sampler) });
        }
        let group = d.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("game post"), layout, entries: &entries });
        (input, group)
    }

    pub(super) fn resize(&mut self, d: &wgpu::Device, out: &wgpu::Buffer) {
        (self.input, self.group) = Self::bind(d, out, &self.layout, &self.uniform, &self.textures);
    }

    pub(super) fn write_uniforms(&self, queue: &wgpu::Queue, w: u32, h: u32, time: f32) {
        let values = Uniforms { size: [w as f32, h as f32], time, ..self.values };
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&values));
    }

    pub(super) fn dispatch(&self, pass: &mut wgpu::ComputePass, w: u32, h: u32) {
        pass.set_pipeline(&self.pipe);
        pass.set_bind_group(0, &self.group, &[]);
        pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
    }
}
