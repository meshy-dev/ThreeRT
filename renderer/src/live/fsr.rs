//! FidelityFX Super Resolution 1: EASU upsampling, then RCAS sharpening into the live HDR post target.
use super::{gpu, layout, targets, Targets};

fn source() -> String {
    [targets::live_source(), include_str!("../../shaders/fsr.wgsl")].concat()
}

struct Pass {
    pipeline: wgpu::ComputePipeline,
    live: wgpu::BindGroup,
    scratch: wgpu::BindGroup,
}

impl Pass {
    fn new(
        g: &gpu::Gpu,
        module: &wgpu::ShaderModule,
        name: &str,
        entries: &[wgpu::BindGroupEntry<'_>],
        scratch: &wgpu::TextureView,
        scratch_binding: u32,
    ) -> Self {
        let d = &g.device;
        let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(name),
            layout: None,
            module,
            entry_point: Some(name),
            compilation_options: Default::default(),
            cache: g.cache(),
        });
        let live = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(name),
            layout: &pipeline.get_bind_group_layout(0),
            entries,
        });
        let scratch = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(name),
            layout: &pipeline.get_bind_group_layout(1),
            entries: &[wgpu::BindGroupEntry {
                binding: scratch_binding,
                resource: wgpu::BindingResource::TextureView(scratch),
            }],
        });
        Self { pipeline, live, scratch }
    }

    fn encode(&self, encoder: &mut wgpu::CommandEncoder, label: &str, [x, y]: [u32; 2]) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.live, &[]);
        pass.set_bind_group(1, &self.scratch, &[]);
        pass.dispatch_workgroups(x, y, 1);
    }
}

pub(super) struct Fsr {
    easu: Pass,
    rcas: Pass,
    workgroups: [u32; 2],
}

impl Fsr {
    pub(super) fn new(g: &gpu::Gpu, targets: &Targets, output: [u32; 2]) -> Self {
        let d = &g.device;
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FidelityFX FSR 1"),
            source: wgpu::ShaderSource::Wgsl(source().into()),
        });
        // Keep FP32 headroom for reversing the HDR proxy. A write-only storage texture and
        // unfiltered textureLoad of this format are both available on a baseline WebGPU device.
        let scratch = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("FSR 1 EASU scratch"),
            size: wgpu::Extent3d { width: output[0], height: output[1], depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }).create_view(&Default::default());
        let entry = |name| wgpu::BindGroupEntry {
            binding: layout::binding(name).binding,
            resource: if name == "L" { targets.params.as_entire_binding() } else { targets.buf(name).binding() },
        };
        Self {
            easu: Pass::new(g, &module, "fsr_easu", &["L", "fb"].map(entry), &scratch, 0),
            rcas: Pass::new(g, &module, "fsr_rcas_to_post", &["L", "sample_vel", "fa", "sb"].map(entry), &scratch, 1),
            workgroups: output.map(|dim| dim.div_ceil(8)),
        }
    }

    pub(super) fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        self.easu.encode(encoder, "FSR 1 EASU", self.workgroups);
        self.rcas.encode(encoder, "FSR 1 RCAS", self.workgroups);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn fsr_shader_only_binds_composed_hdr_depth_and_post_targets() {
        let src = super::source();
        let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&src)));
        for (name, expected) in [("fsr_easu", &["fb"][..]), ("fsr_rcas_to_post", &["fa", "sample_vel", "sb"][..])] {
            let k = module.entry_points.iter().position(|e| e.name == name).unwrap();
            let entry = info.get_entry_point(k);
            let mut buffers: Vec<_> = module.global_variables.iter()
                .filter(|(h, v)| matches!(v.space, naga::AddressSpace::Storage { .. }) && !entry[*h].is_empty())
                .map(|(_, v)| v.name.as_deref().unwrap())
                .collect();
            buffers.sort_unstable();
            assert_eq!(buffers, expected, "{name}");
        }
    }
}
