//! The merged binding layout: the kernel with at most eight storage buffers a stage, WebGPU's default limit
//! (`maxStorageBuffersPerShaderStage`). The kernel binds 21 as it is written: more than a browser gives a page.
//! (The live passes need none of this: each binds at most eight, live/layout.rs, and a page binds them as they are.)
//!
//! The kernel's source has both layouts: pathtrace.wgsl declares its tables' views and its group-1 buffers in an
//! `@if(merged) { .. } @else { .. }` block each (WESL's conditional translation, `gpu::KernelVariant`), and reads and
//! writes them only through functions declared in each (`material_at`, `cache_acc_add`, `refl_store`, ...).
//! `THREERT_MERGED=1` runs the same layout natively (software traversal), so the tests that check the renderer
//! check it too (docs/RUNTIME.md "The engine in a browser").
//!
//! What is merged, each into a buffer of sections found by offsets in the kernel's parameters (heap.rs), so no
//! length is the source's and a scene or a view of another size is the same kernel:
//!  - The scene's tables are the sections of one buffer in every layout. As written the kernel binds the seven it
//!    reads at every hit or light again, each section as a typed array (`TABLE_VIEWS`); here it binds none of them
//!    and reads all seven from the buffer whole.
//!  - The kernel's group-1 buffers (the radiance cache, what the live passes read of a trace, the light tiles'
//!    lists, the loop guard's count) are buffers of their own as written; here the sections of one (`heap::Live`;
//!    `gpu::KernelBufs`), which the live passes bind as buffers of their own (a section starts at a multiple of
//!    WebGPU's `minStorageBufferOffsetAlignment`).
//!
//! The tests below read every variant with naga: valid WGSL, and no stage with more than `STORAGE_BUFFERS`.

use crate::heap::Table;

/// WebGPU's default `maxStorageBuffersPerShaderStage`: what the merged layout fits.
pub const STORAGE_BUFFERS: u32 = 8;

/// The tables the kernel as written binds again as typed arrays over their sections, and their bindings (group 0;
/// pathtrace.wgsl `materials`, `lights`, `emitters`, `tex_info`, `objects`, `instances`, `media`).
pub const TABLE_VIEWS: [(Table, u32); 7] = [(Table::Materials, 12), (Table::Lights, 13), (Table::Emitters, 14), (Table::TexInfo, 15), (Table::Objects, 16), (Table::Instances, 17), (Table::Media, 18)];

/// The merged layout's one group-1 buffer (group 1, binding `LIVE_BINDING`; pathtrace.wgsl `live`).
pub const LIVE_BINDING: u32 = 1;
/// The light tiles' lists in the layout as written (group 1, binding `LISTS_BINDING`).
pub const LISTS_BINDING: u32 = 4;
/// The loop guard's count (`gpu::guard_loops`; its first word: the invocations a loop of which was stopped): in the
/// layout as written a buffer of its own when the kernel has the guard (group 1, binding `GUARD_BINDING`), in the
/// merged layout a section (`heap::Live::Guard`), written only by a kernel with the guard.
pub const GUARD_BINDING: u32 = 8;
pub const GUARD_WORDS: u32 = 4;
/// The pixels a live sample leaves for the kernel's `finish` pass (`gpu::KernelBufs::deferred`): the layout as
/// written only.
pub const DEFERRED_BINDING: u32 = 9;

/// The uniform tables (`gpu::GpuOptions::uniform_tables`; docs/RUNTIME.md "Android"): the tables the kernel reads
/// from uniform buffers, copies of their sections, instead of from views of the sections. Qualcomm's Adreno 740 driver
/// (Vulkan, Android 13) gave the kernel other buffers' contents for these four when each was a read-only storage
/// buffer: a model's textures came out as another table's numbers, instance records sent hits to the wrong
/// triangles, an emissive object's power read as something else's and lit nothing. As uniform buffers they read
/// right. (Whether they read right as views of the tables' buffer, which no device has run yet, is not known:
/// until one has, Android keeps the copies.)
///
/// In the kernel (pathtrace.wgsl, `@if(uniform_tables)`) the four are sized uniform arrays at their own bindings, and
/// every index into them is held to the array (`tex_info_at` and the others: `min(i, last)`). A scene with more
/// entries than an array holds reads the last one for the rest (the renderer says so when it uploads one,
/// `Gpu::upload`); nothing is read outside the buffer, which the kernel's unchecked indexing would otherwise do.
///
/// An array's length (a uniform buffer's array is sized) is what 64 KiB hold, the size every device gives a uniform
/// binding (WebGPU's `maxUniformBufferBindingSize`, Vulkan's least `maxUniformBufferRange` on the phones this is
/// for), and for the media what a scene may have (`scene::MAX_MEDIA`). In a scene's terms: 2,048 textures, 1,365
/// objects (1,024 with instance colours), 409 instance records, every medium.
pub struct UniformTable {
    pub table: Table,
    /// Its array in the kernel, the array's binding (group 0) and length, and its element's bytes.
    pub name: &'static str,
    pub binding: u32,
    pub len: u32,
    pub stride: u64,
}
pub const UNIFORM_TABLES: [UniformTable; 4] = [
    UniformTable { table: Table::TexInfo, name: "tex_info", binding: 15, len: 4096, stride: 16 },
    UniformTable { table: Table::Objects, name: "objects", binding: 16, len: 4096, stride: 16 },
    UniformTable { table: Table::Instances, name: "instances", binding: 17, len: 409, stride: 160 },
    UniformTable { table: Table::Media, name: "media", binding: 18, len: 32, stride: 224 },
];

#[cfg(test)]
mod tests {
    use super::*;

    use crate::gpu::{kernel_template, kernel_wgsl, KernelVariant};

    /// A scene's kernel (software traversal, no switch) as compiled, as written or in the merged layout.
    fn variant(features: u32, merged: bool) -> String {
        kernel_wgsl(&kernel_template(), &KernelVariant { merged, ..KernelVariant::new(false, Default::default(), features) })
    }

    fn parse(src: &str) -> (naga::Module, naga::valid::ModuleInfo) {
        let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
        (m, info)
    }

    /// The storage buffers each entry point of a module uses, and the module declares.
    fn storage_buffers(m: &naga::Module, info: &naga::valid::ModuleInfo) -> (Vec<(String, usize)>, usize) {
        let storage = |v: &naga::GlobalVariable| matches!(v.space, naga::AddressSpace::Storage { .. });
        let used = m.entry_points.iter().enumerate().map(|(k, e)| (e.name.clone(), m.global_variables.iter().filter(|(h, v)| storage(v) && !info.get_entry_point(k)[*h].is_empty()).count())).collect();
        (used, m.global_variables.iter().filter(|(_, v)| storage(v)).count())
    }

    /// The members' offsets and the size of a struct as the shader compiler lays it out.
    fn layout(m: &naga::Module, name: &str) -> (Vec<u64>, u64) {
        let (_, ty) = m.types.iter().find(|(_, t)| t.name.as_deref() == Some(name)).unwrap_or_else(|| panic!("no struct {name}"));
        match &ty.inner {
            naga::TypeInner::Struct { members, span } => (members.iter().map(|m| m.offset as u64).collect(), *span as u64),
            _ => panic!("{name} is not a struct"),
        }
    }

    /// Kernel variants in the merged layout: valid WGSL, no entry point with more than eight storage buffers, none
    /// declared beyond them (a pipeline layout counts what is declared), no length of a view's buffer in the source,
    /// and `MergedLive`'s members where the renderer puts their sections.
    /// The layout as written has 22 (21 under hardware traversal, which has no trees' buffer). Because the merged
    /// blocks declare none of the views or of group 1's buffers by their own names, a read or write of one past its
    /// accessor does not compile here (and naga says where), whatever the other layouts take.
    #[test]
    fn the_merged_kernel_fits_eight_storage_buffers() {
        let (m, info) = parse(&variant(0, false));
        let (used, declared) = storage_buffers(&m, &info);
        assert_eq!(declared, 22, "the kernel as written");
        // (`main` no longer adds to the cache's sums: `finish` trains it.)
        assert_eq!(used.iter().map(|u| u.1).max(), Some(21), "{used:?}");
        let hw = kernel_wgsl(&kernel_template(), &KernelVariant::new(true, Default::default(), 0));
        assert_eq!(hw.matches("var<storage").count(), 21 + 4, "hardware traversal: no trees' buffer (and the translucency pass's four)");
        for features in [0, crate::gpu::F_VCOLOR, crate::gpu::F_GRID, crate::gpu::F_TINT | crate::gpu::F_UNLIT | crate::gpu::F_BACKGROUND, crate::gpu::F_SOURCE] {
            let src = variant(features, true);
            assert!(!src.contains("arrayLength"), "the lengths are the parameters'");
            let (m, info) = parse(&src);
            let (used, declared) = storage_buffers(&m, &info);
            assert!(declared as u32 <= STORAGE_BUFFERS, "features {features:#x}: {declared} declared");
            assert!(used.iter().all(|u| u.1 as u32 <= STORAGE_BUFFERS), "features {features:#x}: {used:?}");
            assert_eq!(used.iter().find(|u| u.0 == "main").map(|u| u.1), Some(8), "the trace reads all eight");
            // The struct's members are the first sections; its last, the vec4s, begins where the cache's values do.
            let (at, _) = crate::heap::Heap::<crate::heap::Live>::starts_for(&crate::gpu::live_sizes(1, 64, 64, 64, 64));
            assert_eq!(layout(&m, "MergedLive").0, at[..4]);
        }
    }

    /// Every kernel variant with the uniform tables: valid WGSL; the four tables are uniform buffers at their own
    /// bindings, sized as the Rust side makes their buffers; the storage buffers are as written; and each of the
    /// four is indexed once, in its accessor, held to its array, so nothing is read outside a uniform buffer.
    #[test]
    fn the_uniform_tables_are_four_sized_uniform_arrays() {
        let globals = |m: &naga::Module, uniform: bool| -> Vec<(String, u32, u32)> {
            let mut out: Vec<(String, u32, u32)> = m
                .global_variables
                .iter()
                .filter(|(_, v)| if uniform { v.space == naga::AddressSpace::Uniform } else { matches!(v.space, naga::AddressSpace::Storage { .. }) })
                .map(|(_, v)| {
                    let b = v.binding.as_ref().expect("a bound global");
                    let bytes = match &m.types[v.ty].inner {
                        naga::TypeInner::Array { size: naga::ArraySize::Constant(n), stride, .. } => n.get() * stride,
                        _ => 0,
                    };
                    (v.name.clone().unwrap_or_default(), b.group * 100 + b.binding, bytes)
                })
                .collect();
            out.sort_by_key(|g| g.1);
            out
        };
        let (written, _) = parse(&variant(0, false));
        for features in [0, crate::gpu::F_VCOLOR, crate::gpu::F_GRID, crate::gpu::F_TINT | crate::gpu::F_UNLIT | crate::gpu::F_BACKGROUND, crate::gpu::F_SOURCE] {
            let src = kernel_wgsl(&kernel_template(), &KernelVariant { uniform_tables: true, ..KernelVariant::new(false, Default::default(), features) });
            let (m, info) = parse(&src);
            assert_eq!(storage_buffers(&m, &info).1, 18, "features {features:#x}");
            let tables: Vec<(String, u32, u32)> = globals(&m, true).into_iter().filter(|g| g.0 != "P").collect();
            let want: Vec<(String, u32, u32)> = UNIFORM_TABLES.iter().map(|t| (t.name.to_string(), t.binding, (t.stride * t.len as u64) as u32)).collect();
            assert_eq!(tables, want, "features {features:#x}");
            // The storage buffers that stay: the same names at the same bindings as the kernel as written.
            let kept = |m: &naga::Module| globals(m, false).into_iter().map(|g| (g.0, g.1)).filter(|g| !want.iter().any(|w| w.0 == g.0)).collect::<Vec<_>>();
            assert_eq!(kept(&m), kept(&written), "features {features:#x}");
            for t in &UNIFORM_TABLES {
                let name = t.name;
                let upper = name.to_uppercase();
                assert_eq!(src.matches(&format!("{name}[")).count(), 1, "{name}: indexed in its accessor only (features {features:#x})");
                assert!(src.contains(&format!("{name}[min(i, UT_{upper} - 1u)]")), "{name}: held to its array");
            }
        }
    }

    /// The kernel reads a record from as many vec4s of the tables as the Rust side's record has, and makes of them
    /// a struct of that size.
    #[test]
    fn the_tables_records_are_the_kernels() {
        use crate::scene::{GpuInstance, GpuLight, GpuMaterial, MediumGpu};
        let src = variant(0, true);
        let (m, _) = parse(&src);
        // (The views' bindings follow the buffer's, and are the kernel's.)
        for (k, (table, binding)) in TABLE_VIEWS.iter().enumerate() {
            let name = ["materials", "lights", "emitters", "tex_info", "objects", "instances", "media"][k];
            assert!(kernel_template().contains(&format!("@group(0) @binding({binding}) var<storage, read> {name}: array<")), "{table:?}");
        }
        for (name, constant, bytes) in [("Material", "MATERIAL_VEC4", std::mem::size_of::<GpuMaterial>()), ("Light", "LIGHT_VEC4", std::mem::size_of::<GpuLight>()), ("Instance", "INSTANCE_VEC4", std::mem::size_of::<GpuInstance>()), ("Medium", "MEDIUM_VEC4", std::mem::size_of::<MediumGpu>())] {
            assert!(kernel_template().contains(&format!("const {constant}: u32 = {}u;", bytes / 16)), "{constant}");
            assert_eq!(layout(&m, name).1 as usize, bytes, "{name}");
        }
        assert_eq!(<Table as crate::heap::Section>::COUNT, Table::Energy as usize + 1);
        assert!(kernel_template().contains(&format!("const CACHE_SIZE: u32 = {}u;", crate::gpu::CACHE_SIZE)));
        // `finish`'s records and its dispatch rows.
        assert!(kernel_template().contains(&format!("const DEFERRED_VECS: u32 = {}u;", crate::gpu::DEFERRED_BYTES / 16)));
        assert!(kernel_template().contains(&format!("const DEFERRED_ROW: u32 = {}u;", crate::gpu::DEFERRED_ROW)));
        assert!(kernel_template().contains(&format!("@group(1) @binding({DEFERRED_BINDING}) var<storage, read_write> deferred: Deferred;")));
        // What the kernel reads with no offset is its heap's first section.
        assert_eq!((Table::LightTree as usize, crate::heap::Stream::EnvCdf as usize), (0, 0));
        assert!(kernel_template().contains("fn lt_node(i: u32) -> vec4<f32> { return table_f(i); }") && kernel_template().contains("fn env_cdf_at(i: u32) -> f32 { return bitcast<f32>(words[i]); }"));
        assert!(<Table as crate::heap::Section>::COUNT <= 12, "`Params::table_first` names every table");
    }

    /// The loop guard opens every loop of the kernel but counted ones (`for (var i = N; i < M; i++)`, literals), the
    /// scene's functions' too, in the layout as written and in the merged one (its count a buffer of its own, or a
    /// section of `live`), and what comes out is valid WGSL.
    #[test]
    fn the_guard_bounds_every_loop() {
        let counted = |s: &str| {
            s.lines()
                .filter_map(|l| l.trim().strip_prefix("for (var "))
                .filter(|l| {
                    let parts: Vec<&str> = l.split(';').map(str::trim).collect();
                    let name = parts[0].split([' ', ':']).next().unwrap_or("");
                    let lit = |t: &str| t.trim_end_matches(['u', 'i']).bytes().all(|c| c.is_ascii_digit());
                    parts.len() == 3 && parts[0].rsplit('=').next().is_some_and(|v| lit(v.trim())) && parts[1].strip_prefix(name).is_some_and(|r| lit(r.trim().trim_start_matches(['<', '=']).trim())) && parts[2].starts_with(&format!("{name}++"))
                })
                .count()
        };
        let loops = |s: &str| s.matches("loop {").count() + s.matches("for (").count() + s.matches("while (").count();
        for merged in [false, true] {
            let v = KernelVariant { merged, ..KernelVariant::new(false, Default::default(), crate::gpu::F_SOURCE) };
            let plain = kernel_wgsl(&kernel_template(), &v);
            let g = kernel_wgsl(&kernel_template(), &KernelVariant { guard: Some(1 << 16), ..v });
            assert!(loops(&plain) > 40 && counted(&plain) > 3, "the kernel's loops: {} counted of {}", counted(&plain), loops(&plain));
            assert_eq!(g.matches("if (loop_guard()) {").count(), loops(&plain) - counted(&plain));
            let (m, info) = parse(&g);
            let (used, declared) = storage_buffers(&m, &info);
            // (As written: the guard's count and `finish`'s records too. Merged: still eight.)
            assert_eq!(declared as u32, if merged { STORAGE_BUFFERS } else { 23 }, "{used:?}");
        }
        // The pass on its own: nested loops, a `continuing` block, comments and braces in them, a loop in an `if`; a
        // counted loop left alone, one bounded by a variable or assigning its counter guarded.
        let mut tu = crate::gpu::wesl_wgsl("fn f() {\n    for (var i = 0u; i < n; i++) { // {\n        loop { if (a) { break; } continuing { while (b) { } } }\n    }\n    if (c) { while (d) { } } else { x(); }\n    for (var k = 0u; k < 16u; k++) { if (e) { break; } y(); }\n    for (var j = 0u; j <= 3u; j++) { j += 2u; }\n}\n", &[], &[]);
        crate::gpu::guard_loops(&mut tu);
        let g = tu.to_string();
        assert_eq!(g.matches("if (loop_guard()) {").count(), 5, "{g}");
        let first = |w: &str| g[g.find(w).unwrap()..].split('\n').nth(1).unwrap().trim().to_string();
        assert_eq!([first("for (var i"), first("loop {"), first("while (b)"), first("while (d)"), first("for (var j")], ["if (loop_guard()) {"; 5], "{g}");
        assert_eq!(first("for (var k"), "if (e) {", "{g}");
    }

    /// The constants a variant imports are the ones it is given, by name: a guard's limit, the uniform tables'
    /// lengths. The merged layout has none of its own.
    #[test]
    fn the_variant_gives_what_it_imports() {
        let v = KernelVariant { merged: true, guard: Some(9), ..KernelVariant::new(false, Default::default(), 0) };
        assert_eq!(v.constants(), vec![("GUARD_WORDS".into(), GUARD_WORDS), ("LOOP_GUARD_LIMIT".into(), 9)]);
        assert_eq!(KernelVariant { merged: true, ..Default::default() }.constants(), vec![("GUARD_WORDS".into(), GUARD_WORDS)]);
        let u = KernelVariant { uniform_tables: true, ..KernelVariant::new(false, Default::default(), 0) }.constants();
        assert_eq!(u, vec![("UT_TEX_INFO".into(), 4096), ("UT_OBJECTS".into(), 4096), ("UT_INSTANCES".into(), 409), ("UT_MEDIA".into(), 32)]);
        assert!(KernelVariant::default().constants().is_empty());
    }

    #[test]
    #[should_panic(expected = "LOOP_GUARD_LIMIT")]
    fn a_constant_the_source_imports_must_be_given() {
        let v = KernelVariant { guard: Some(9), ..KernelVariant::new(false, Default::default(), 0) };
        let mut c = v.constants();
        c.retain(|(n, _)| n != "LOOP_GUARD_LIMIT");
        crate::gpu::wesl_wgsl(&kernel_template(), &v.flags(), &c);
    }
}
