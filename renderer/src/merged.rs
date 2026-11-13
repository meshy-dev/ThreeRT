//! The merged binding layout: the kernel with at most eight storage buffers a stage, WebGPU's default limit
//! (`maxStorageBuffersPerShaderStage`). The kernel binds 19 as it is written: more than a browser gives a page. (The
//! live passes need none of this: each binds at most eight, live/layout.rs, and a page binds them as they are.)
//!
//! The kernel's source has both layouts: pathtrace.wgsl declares its tables and its group-1 buffers in an
//! `@if(merged) { .. } @else { .. }` block each (WESL's conditional translation, `gpu::KernelVariant`), and reads and
//! writes them only through functions declared in each (`material_at`, `cache_acc_add`, `refl_store`, ...). This
//! module is the Rust side's half: which buffers are merged, the lengths of the merged structs' arrays (WESL constants
//! the source imports, `gpu::KernelVariant::constants`) and where the arrays are, for the buffers the renderer makes
//! and copies. `THREERT_MERGED=1` runs the same layout natively (software traversal), so the tests that check the
//! renderer check it too (docs/RUNTIME.md "The engine in a browser").
//!
//! What is merged:
//!  - The kernel's seven tables (`TABLES`: materials, lights, emitters, texture pages, objects, instances, media)
//!    become the arrays of one struct, `MT: MergedTables`, in one buffer. Their lengths are each a power of two no
//!    less than `TABLE_MIN`, so a scene that grows compiles a kernel again only when a table doubles. The renderer
//!    keeps writing its own buffers; each frame copies them into the struct's (`Gpu::sync_tables`; they are small).
//!  - The kernel's six group-1 buffers (`KERNEL_LIVE`: the radiance cache, what the live passes read of a trace) and
//!    the loop guard's count become the arrays of `ML: MergedLive`, one buffer that they are ranges of
//!    (`gpu::KernelBufs`). What the live passes read of it starts at a multiple of 256 bytes (`@align(256)`): the
//!    passes bind those ranges as buffers of their own, and a binding's offset is a multiple of that (WebGPU's
//!    `minStorageBufferOffsetAlignment`).
//!
//! The tests below read every variant with naga: valid WGSL, no stage with more than `STORAGE_BUFFERS`, and the
//! structs' arrays where `offsets` and `live_layout` put them.

/// WebGPU's default `maxStorageBuffersPerShaderStage`: what the merged layout fits.
pub const STORAGE_BUFFERS: u32 = 8;

/// One of the kernel's buffers that becomes an array of a merged struct.
pub struct Member {
    pub name: &'static str,
    /// Its element's size in a storage buffer; where its array may start in the struct: its element's alignment, or
    /// more (`@align` in pathtrace.wgsl's struct).
    pub stride: u64,
    pub align: u64,
}

/// The kernel's tables, in `MergedTables`' order (group 0, binding `TABLES_BINDING`).
pub const TABLES: [Member; 7] = [
    Member { name: "materials", stride: 288, align: 16 },
    Member { name: "lights", stride: 80, align: 16 },
    Member { name: "emitters", stride: 16, align: 16 },
    Member { name: "tex_info", stride: 16, align: 16 },
    Member { name: "objects", stride: 16, align: 16 },
    Member { name: "instances", stride: 160, align: 16 },
    Member { name: "media", stride: 224, align: 16 },
];
pub const TABLES_BINDING: u32 = 4;
/// The least length of each table's array: scenes below it share one kernel source.
pub const TABLE_MIN: [u32; 7] = [64, 64, 1024, 256, 1024, 64, 16];

/// The kernel's group-1 buffers, in `MergedLive`'s order (group 1, binding `KERNEL_LIVE_BINDING`).
pub const KERNEL_LIVE: [Member; 6] = [
    Member { name: "cache_keys", stride: 4, align: 4 },
    Member { name: "cache_acc", stride: 4, align: 4 },
    Member { name: "cache_val", stride: 16, align: 16 },
    Member { name: "refl", stride: 16, align: 256 },
    Member { name: "motion_out", stride: 16, align: 256 },
    Member { name: "direct_out", stride: 16, align: 256 },
];
pub const KERNEL_LIVE_BINDING: u32 = 1;
/// `MergedLive`'s last array, after `KERNEL_LIVE`'s: the loop guard's count (`gpu::guard_loops`; its first word: the invocations
/// a loop of which was stopped). Always there in the merged layout, written only by a kernel with the guard.
pub const GUARD_TRIPS: Member = Member { name: "loop_guard_trips", stride: 4, align: 4 };
/// The same count in the layout as written, when the kernel has the guard (group 1, binding `GUARD_BINDING`).
pub const GUARD_BINDING: u32 = 8;
pub const GUARD_WORDS: u32 = 4;

/// Where `MergedLive`'s arrays are for these lengths (bytes): `KERNEL_LIVE`'s six, then the loop guard's count; and
/// the struct's size.
pub fn live_layout(caps: &[u32; 6]) -> (Vec<u64>, u64) {
    let members: Vec<&Member> = KERNEL_LIVE.iter().chain([&GUARD_TRIPS]).collect();
    let caps: Vec<u32> = caps.iter().copied().chain([GUARD_WORDS]).collect();
    let (mut at, mut out) = (0u64, vec![]);
    for (m, &c) in members.iter().zip(&caps) {
        at = at.next_multiple_of(m.align);
        out.push(at);
        at += m.stride * c as u64;
    }
    // (A struct's size is a multiple of its largest member's alignment: 256 where the live passes' ranges are.)
    (out, at.next_multiple_of(members.iter().map(|m| m.align).max().unwrap_or(16)))
}

/// The lengths of the merged structs' arrays: what a kernel's source is written for (`gpu::Kernel::caps`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Caps {
    pub tables: [u32; 7],
    pub live: [u32; 6],
}

/// A table's array length for a buffer of `bytes`.
pub fn table_cap(k: usize, bytes: u64) -> u32 {
    (bytes.div_ceil(TABLES[k].stride).max(1) as u32).next_power_of_two().max(TABLE_MIN[k])
}

/// A group-1 buffer's array length: its own (the kernel reads some of them, `arrayLength`).
pub fn live_cap(k: usize, bytes: u64) -> u32 {
    assert_eq!(bytes % KERNEL_LIVE[k].stride, 0, "merged layout: {} of {bytes} bytes", KERNEL_LIVE[k].name);
    (bytes / KERNEL_LIVE[k].stride) as u32
}

/// Where a merged struct's arrays are (bytes), and the struct's size: WGSL's layout rules.
pub fn offsets(members: &[Member], caps: &[u32]) -> (Vec<u64>, u64) {
    let (mut at, mut out) = (0u64, vec![]);
    for (m, &c) in members.iter().zip(caps) {
        at = at.next_multiple_of(m.align);
        out.push(at);
        at += m.stride * c as u64;
    }
    (out, at.next_multiple_of(members.iter().map(|m| m.align).max().unwrap_or(1)))
}

/// The uniform tables (`gpu::GpuOptions::uniform_tables`; docs/RUNTIME.md "Android"): which of `TABLES` the kernel
/// reads from uniform buffers instead of storage buffers, in the layout as written. Qualcomm's Adreno 740 driver
/// (Vulkan, Android 13) gave the kernel other buffers' contents for these four: a model's textures came out as
/// another table's numbers, instance records sent hits to the wrong triangles, an emissive object's power read as
/// something else's and lit nothing. As uniform buffers they read right. They are the four of the kernel's read-only
/// tables that are bounded (the others: below).
///
/// In the kernel (pathtrace.wgsl, `@if(uniform_tables)`) the four are sized uniform arrays at their own bindings, and
/// every index into them is held to the array (`tex_info_at` and the others: `min(i, last)`). A scene with more
/// entries than an array holds reads the last one for the rest (the renderer says so when it uploads one,
/// `Gpu::upload`); nothing is read outside the buffer, which the kernel's unchecked indexing would otherwise do.
///
/// The kernel's other storage buffers stay storage buffers, each for its reason:
///  - `materials` (288 B each), `lights` (80 B): the device drew right with them as they are, and 64 KiB would hold
///    227 materials;
///  - `emitters` (the emitter CDF and the light tree), `env_cdf` (the environment's CDF, then the procedural
///    geometry's data), `tri_verts`, `attrs`, `nodes`, `prev_verts` (and the vertex colours after them): megabytes,
///    which no uniform binding holds;
///  - `accum` and group 1 (the radiance cache, the trace's outputs): written by the kernel.
pub const UNIFORM_TABLES: [usize; 4] = [3, 4, 5, 6];
/// Their arrays' lengths (a uniform buffer's array is sized): what 64 KiB hold, the size every device gives a uniform
/// binding (WebGPU's `maxUniformBufferBindingSize`, Vulkan's least `maxUniformBufferRange` on the phones this is
/// for), and for the media what a scene may have (`scene::MAX_MEDIA`). In a scene's terms: 2,048 textures, 1,365
/// objects (1,024 with instance colours), 409 instance records, every medium.
pub const UNIFORM_LEN: [u32; 4] = [4096, 4096, 409, 32];

#[cfg(test)]
mod tests {
    use super::*;

    use crate::gpu::{kernel_template, kernel_wgsl, KernelVariant};

    /// A scene's kernel (software traversal, no switch) as compiled, in the merged layout when given its lengths.
    fn variant(features: u32, merged: Option<&Caps>) -> String {
        kernel_wgsl(&kernel_template(), &KernelVariant { merged: merged.copied(), ..KernelVariant::new(false, Default::default(), features) })
    }

    fn caps() -> Caps {
        // A cache of one entry, a view of 64 x 36: lengths that are not round.
        let n = 64 * 36;
        Caps { tables: TABLE_MIN, live: [1, 8, 2, 2 * n, n, n] }
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
    /// declared beyond them (a pipeline layout counts what is declared), and the merged structs' arrays where `offsets`
    /// and `live_layout` put them. The layout as written has 19. Because the merged block declares none of the
    /// buffers by their own names, a read or write of one past its accessor does not compile here (and naga says
    /// where), whatever the other layouts take.
    #[test]
    fn the_merged_kernel_fits_eight_storage_buffers() {
        let (m, info) = parse(&variant(0, None));
        let (used, declared) = storage_buffers(&m, &info);
        assert_eq!(declared, 19, "the kernel as written");
        assert_eq!(used.iter().map(|u| u.1).max(), Some(19), "{used:?}");
        let caps = caps();
        for features in [0, crate::gpu::F_VCOLOR, crate::gpu::F_GRID, crate::gpu::F_TINT | crate::gpu::F_UNLIT | crate::gpu::F_BACKGROUND, crate::gpu::F_SOURCE] {
            let src = variant(features, Some(&caps));
            assert!(src.contains("const MT_MEDIA = 16u;") && src.contains(&format!("const ML_REFL = {}u;", caps.live[3])), "the lengths, as constants");
            let (m, info) = parse(&src);
            let (used, declared) = storage_buffers(&m, &info);
            assert!(declared as u32 <= STORAGE_BUFFERS, "features {features:#x}: {declared} declared");
            assert!(used.iter().all(|u| u.1 as u32 <= STORAGE_BUFFERS), "features {features:#x}: {used:?}");
            assert_eq!(used.iter().find(|u| u.0 == "main").map(|u| u.1), Some(8), "the trace reads all eight");
            assert_eq!(layout(&m, "MergedTables"), offsets(&TABLES, &caps.tables));
            assert_eq!(layout(&m, "MergedLive"), live_layout(&caps.live));
        }
    }

    /// Every kernel variant with the uniform tables: valid WGSL; the four tables are uniform buffers at their own
    /// bindings, sized as the Rust side makes their buffers; the other fifteen storage buffers are as written; and
    /// each of the four is indexed once, in its accessor, held to its array, so nothing is read outside a uniform
    /// buffer.
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
        let (written, _) = parse(&variant(0, None));
        for features in [0, crate::gpu::F_VCOLOR, crate::gpu::F_GRID, crate::gpu::F_TINT | crate::gpu::F_UNLIT | crate::gpu::F_BACKGROUND, crate::gpu::F_SOURCE] {
            let src = kernel_wgsl(&kernel_template(), &KernelVariant { uniform_tables: true, ..KernelVariant::new(false, Default::default(), features) });
            let (m, info) = parse(&src);
            assert_eq!(storage_buffers(&m, &info).1, 15, "features {features:#x}");
            let tables: Vec<(String, u32, u32)> = globals(&m, true).into_iter().filter(|g| g.0 != "P" && g.0 != "energy_lut").collect();
            let want: Vec<(String, u32, u32)> = UNIFORM_TABLES.iter().zip(UNIFORM_LEN).zip([14, 15, 17, 18]).map(|((&k, len), binding)| (TABLES[k].name.to_string(), binding, (TABLES[k].stride * len as u64) as u32)).collect();
            assert_eq!(tables, want, "features {features:#x}");
            // The storage buffers that stay: the same names at the same bindings as the kernel as written.
            let kept = |m: &naga::Module| globals(m, false).into_iter().map(|g| (g.0, g.1)).filter(|g| !want.iter().any(|w| w.0 == g.0)).collect::<Vec<_>>();
            assert_eq!(kept(&m), kept(&written), "features {features:#x}");
            for &k in &UNIFORM_TABLES {
                let name = TABLES[k].name;
                let upper = name.to_uppercase();
                assert_eq!(src.matches(&format!("{name}[")).count(), 1, "{name}: indexed in its accessor only (features {features:#x})");
                assert!(src.contains(&format!("{name}[min(i, UT_{upper} - 1u)]")), "{name}: held to its array");
            }
        }
    }

    /// The Rust side's records are the size the merged structs give their elements.
    #[test]
    fn the_tables_strides_are_the_records() {
        use crate::scene::{GpuInstance, GpuLight, GpuMaterial, MediumGpu};
        let sizes = [std::mem::size_of::<GpuMaterial>(), std::mem::size_of::<GpuLight>(), 16, 16, 16, std::mem::size_of::<GpuInstance>(), std::mem::size_of::<MediumGpu>()];
        assert_eq!(TABLES.iter().map(|m| m.stride as usize).collect::<Vec<_>>(), sizes);
        assert_eq!(table_cap(0, 0), 64);
        assert_eq!(table_cap(0, 288 * 65), 128);
        assert_eq!(table_cap(2, 512), 1024);
    }

    /// The loop guard opens every loop of the kernel, the scene's functions' too, in the layout as written and in the
    /// merged one (its count a buffer of its own, or in `MergedLive`), and what comes out is valid WGSL.
    #[test]
    fn the_guard_bounds_every_loop() {
        let loops = |s: &str| s.matches("loop {").count() + s.matches("for (").count() + s.matches("while (").count();
        for merged in [None, Some(caps())] {
            let v = KernelVariant { merged, ..KernelVariant::new(false, Default::default(), crate::gpu::F_SOURCE) };
            let plain = kernel_wgsl(&kernel_template(), &v);
            let g = kernel_wgsl(&kernel_template(), &KernelVariant { guard: Some(1 << 16), ..v });
            assert!(loops(&plain) > 40, "the kernel's loops");
            assert_eq!(g.matches("if (loop_guard()) {").count(), loops(&plain));
            let (m, info) = parse(&g);
            let (used, declared) = storage_buffers(&m, &info);
            // (As written: one buffer more, where the device has room. Merged: still eight.)
            assert_eq!(declared as u32, if merged.is_some() { STORAGE_BUFFERS } else { 20 }, "{used:?}");
        }
        // The pass on its own: nested loops, a `continuing` block, comments and braces in them, a loop in an `if`.
        let mut tu = crate::gpu::wesl_wgsl("fn f() {\n    for (var i = 0u; i < 2u; i++) { // {\n        loop { if (a) { break; } continuing { while (b) { } } }\n    }\n    if (c) { while (d) { } } else { x(); }\n}\n", &[], &[]);
        crate::gpu::guard_loops(&mut tu);
        let g = tu.to_string();
        assert_eq!(g.matches("if (loop_guard()) {").count(), 4, "{g}");
        let first = |w: &str| g[g.find(w).unwrap()..].split('\n').nth(1).unwrap().trim().to_string();
        assert_eq!([first("for ("), first("loop {"), first("while (b)"), first("while (d)")], ["if (loop_guard()) {"; 4], "{g}");
    }

    /// The constants a variant imports are the ones it is given, by name: a merged kernel's lengths, a guard's limit.
    #[test]
    fn the_variant_gives_what_it_imports() {
        let v = KernelVariant { merged: Some(caps()), guard: Some(9), ..KernelVariant::new(false, Default::default(), 0) };
        let c = v.constants();
        assert_eq!(c.len(), 7 + 6 + 2);
        assert!(c.contains(&("MT_TEX_INFO".into(), TABLE_MIN[3])) && c.contains(&("ML_DIRECT_OUT".into(), 64 * 36)) && c.contains(&("LOOP_GUARD_LIMIT".into(), 9)) && c.contains(&("GUARD_WORDS".into(), GUARD_WORDS)));
        let u = KernelVariant { uniform_tables: true, ..KernelVariant::new(false, Default::default(), 0) }.constants();
        assert_eq!(u, vec![("UT_TEX_INFO".into(), 4096), ("UT_OBJECTS".into(), 4096), ("UT_INSTANCES".into(), 409), ("UT_MEDIA".into(), 32)]);
        assert!(KernelVariant::default().constants().is_empty());
    }

    #[test]
    #[should_panic(expected = "MT_MEDIA")]
    fn a_constant_the_source_imports_must_be_given() {
        let mut c = KernelVariant { merged: Some(caps()), ..KernelVariant::new(false, Default::default(), 0) }.constants();
        c.retain(|(n, _)| n != "MT_MEDIA");
        crate::gpu::wesl_wgsl(&kernel_template(), &KernelVariant { merged: Some(caps()), ..Default::default() }.flags(), &c);
    }
}
