//! The kernel's sampler on the CPU (shaders/sample.wgsl, the same numbers): shuffled, Owen-scrambled Sobol (Burley
//! 2020). For tests: the sequence's properties here, and the shader held to this.

/// Direction numbers of Sobol dimensions 1-3 (Joe & Kuo), bit-reversed: `SOBOL_REV` in sample.wgsl.
const SOBOL_REV: [[u32; 32]; 3] = {
    const fn dirs(s: usize, a: u32, m0: [u32; 3]) -> [u32; 32] {
        let mut m = [0u32; 32];
        let mut i = 0;
        while i < 32 {
            m[i] = if i < s {
                m0[i]
            } else {
                let mut v = m[i - s] ^ (m[i - s] << s);
                let mut k = 1;
                while k < s {
                    v ^= ((a >> (s - 1 - k)) & 1) * (m[i - k] << k);
                    k += 1;
                }
                v
            };
            i += 1;
        }
        let mut out = [0u32; 32];
        let mut i = 0;
        while i < 32 {
            out[i] = (m[i] << (31 - i)).reverse_bits();
            i += 1;
        }
        out
    }
    [dirs(1, 0, [1, 0, 0]), dirs(2, 1, [1, 3, 0]), dirs(3, 1, [1, 3, 1])]
};

pub fn owen_rev(mut n: u32, seed: u32) -> u32 {
    n ^= n.wrapping_mul(0x3d20adea);
    n = n.wrapping_add(seed);
    n = n.wrapping_mul((seed >> 16) | 1);
    n ^= n.wrapping_mul(0x05526c56);
    n ^= n.wrapping_mul(0x53a22864);
    n
}

pub fn hash_hp(mut i: u32) -> u32 {
    i ^= i >> 16;
    i = i.wrapping_mul(0x21f0aaad);
    i ^= i >> 15;
    i = i.wrapping_mul(0xd35a2d97);
    i ^= i >> 15;
    i ^ 0xe6fe3beb
}

pub fn sobol_owen(rev_index: u32, dim: usize, seed: u32) -> f32 {
    let mut r = 0;
    if dim == 0 {
        r = rev_index.reverse_bits();
    } else {
        let (mut x, mut i) = (rev_index, 0usize);
        for _ in 0..16 {
            if x == 0 {
                break;
            }
            let j = x.leading_zeros() as usize;
            r ^= SOBOL_REV[dim - 1][i + j];
            i += j + 1;
            x = (x << j) << 1;
        }
    }
    owen_rev(r, seed).reverse_bits() as f32 * (1.0 / 4294967808.0)
}

/// `rng2` of sample.wgsl for a pixel seed (`RNG_SEED`), a sample index and a dimension id.
pub fn rng2(pixel_seed: u32, index: u32, d: u32) -> [f32; 2] {
    let s = pixel_seed ^ hash_hp(d);
    let i = owen_rev(index.reverse_bits(), s ^ 0xf8ade99a) & 0xffff0000;
    [sobol_owen(i, 0, s ^ 0xe0aaaf76), sobol_owen(i, 1, s ^ 0x94964d4e)]
}

/// `rng4` of sample.wgsl.
pub fn rng4(pixel_seed: u32, index: u32, d: u32) -> [f32; 4] {
    let s = pixel_seed ^ hash_hp(d);
    let i = owen_rev(index.reverse_bits(), s ^ 0xc2c1a055) & 0xffff0000;
    [sobol_owen(i, 0, s ^ 0x39468210), sobol_owen(i, 1, s ^ 0xe9d8a845), sobol_owen(i, 2, s ^ 0x5f32b482), sobol_owen(i, 3, s ^ 0x1524cc56)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is sample.wgsl's, and Cycles' (`sobol_burley_table`, its last entries).
    #[test]
    fn the_directions_are_the_shaders() {
        let wgsl = include_str!("../shaders/sample.wgsl");
        for row in SOBOL_REV {
            for v in &row[..16] {
                assert!(wgsl.contains(&format!("0x{v:08x}u")), "{v:#010x}");
            }
        }
        assert_eq!([SOBOL_REV[0][31], SOBOL_REV[1][31], SOBOL_REV[2][31]], [0xffffffff, 0xaaaa00a3, 0xc900a00a]);
    }

    /// Each 2D pair of a 4D point set of 2^m samples is a (0, m, 2)-net: every elementary interval of area 2^-m holds
    /// exactly one point (dimensions 0-1 of Sobol are; the shuffle and Owen scrambling keep it). And 1D strata.
    #[test]
    fn the_first_2_to_the_m_points_are_a_net() {
        for (seed, d) in [(0u32, 0u32), (0x1234_5678, 7), (hash_hp(99), 0xdead)] {
            for m in [4u32, 6, 8] {
                let n = 1u32 << m;
                let pts: Vec<[f32; 4]> = (0..n).map(|i| rng4(seed, i, d)).collect();
                let cell = |v: f32, k: u32| ((v * (1u64 << k) as f32) as u32).min((1 << k) - 1);
                for a in 0..=m {
                    let mut seen = vec![false; n as usize];
                    for p in &pts {
                        let c = (cell(p[0], a) << (m - a)) | cell(p[1], m - a);
                        assert!(!std::mem::replace(&mut seen[c as usize], true), "seed {seed:#x} d {d} m {m}: interval {a}x{} twice", m - a);
                    }
                }
                for k in 0..4 {
                    let mut strata: Vec<u32> = pts.iter().map(|p| cell(p[k], m)).collect();
                    strata.sort();
                    assert_eq!(strata, (0..n).collect::<Vec<_>>(), "dimension {k}");
                }
                assert!(pts.iter().flatten().all(|v| (0.0..1.0).contains(v)));
            }
        }
    }

    /// sample.wgsl on the GPU gives these numbers, offline (`LIVE` false; to the last place: a u32's conversion to f32
    /// may round either way). (Skipped without a GPU.)
    #[test]
    fn the_shader_is_this() {
        use wgpu::util::DeviceExt;
        let g = match crate::gpu::Gpu::new_with(crate::gpu::GpuOptions::default()) {
            Ok(g) => g,
            Err(e) => return eprintln!("skipped: {e}"),
        };
        let d = &g.device;
        let seeds = [0u32, 0x1234_5678, hash_hp(99)];
        let src = format!("{}\noverride LIVE: bool = false;\nvar<private> cur_depth: u32;\n@group(0) @binding(0) var<storage, read_write> out: array<u32>;\nconst SEEDS = array<u32, 3>({}u, {}u, {}u);\n@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) id: vec3<u32>) {{\n    let i = id.x;\n    RNG_SEED = SEEDS[i % 3u];\n    RNG_INDEX = i;\n    let r = rng4(7u + i % 5u);\n    for (var k = 0u; k < 4u; k++) {{ out[i * 4u + k] = bitcast<u32>(r[k]); }}\n}}\n", include_str!("../shaders/sample.wgsl"), seeds[0], seeds[1], seeds[2]);
        let module = d.create_shader_module(wgpu::ShaderModuleDescriptor { label: None, source: wgpu::ShaderSource::Wgsl(src.into()) });
        let pipeline = d.create_compute_pipeline(&wgpu::ComputePipelineDescriptor { label: None, layout: None, module: &module, entry_point: Some("main"), compilation_options: Default::default(), cache: None });
        let n = 256u32;
        let buf = d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: None, contents: &vec![0u8; n as usize * 16], usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC });
        let read = d.create_buffer(&wgpu::BufferDescriptor { label: None, size: n as u64 * 16, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
        let bg = d.create_bind_group(&wgpu::BindGroupDescriptor { label: None, layout: &pipeline.get_bind_group_layout(0), entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }] });
        let mut enc = d.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_compute_pass(&Default::default());
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &bg, &[]);
            pass.dispatch_workgroups(n / 64, 1, 1);
        }
        enc.copy_buffer_to_buffer(&buf, 0, &read, 0, n as u64 * 16);
        g.queue.submit([enc.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        d.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let words: Vec<u32> = bytemuck::cast_slice(&read.slice(..).get_mapped_range().unwrap()).to_vec();
        for i in 0..n {
            let want = rng4(seeds[(i % 3) as usize], i, 7 + i % 5);
            for k in 0..4 {
                let got = f32::from_bits(words[(i * 4 + k) as usize]);
                assert!((got - want[k as usize]).abs() <= 1.2e-7, "index {i} component {k}: {got} on the GPU, {} here", want[k as usize]);
            }
        }
    }

    /// Different dimension ids and seeds give different (uncorrelated) sequences.
    #[test]
    fn ids_and_seeds_decorrelate() {
        let a: Vec<[f32; 2]> = (0..64).map(|i| rng2(5, i, 1)).collect();
        let b: Vec<[f32; 2]> = (0..64).map(|i| rng2(5, i, 2)).collect();
        let c: Vec<[f32; 2]> = (0..64).map(|i| rng2(6, i, 1)).collect();
        let corr = |x: &[[f32; 2]], y: &[[f32; 2]]| {
            let (mx, my) = (x.iter().map(|p| p[0]).sum::<f32>() / 64.0, y.iter().map(|p| p[0]).sum::<f32>() / 64.0);
            let cov: f32 = x.iter().zip(y).map(|(p, q)| (p[0] - mx) * (q[0] - my)).sum();
            let (vx, vy): (f32, f32) = (x.iter().map(|p| (p[0] - mx).powi(2)).sum(), y.iter().map(|q| (q[0] - my).powi(2)).sum());
            cov / (vx * vy).sqrt()
        };
        assert!(corr(&a, &b).abs() < 0.35 && corr(&a, &c).abs() < 0.35, "{} {}", corr(&a, &b), corr(&a, &c));
    }
}
