//! The merged binding layout: the engine's shaders with at most eight storage buffers a stage, WebGPU's default limit
//! (`maxStorageBuffersPerShaderStage`). The kernel binds 19 as it is written and the live passes 24: more than a
//! browser gives a page. There is one source: pathtrace.wgsl and live.wgsl, and the bindings gpu.rs and live.rs make for
//! them, are what the native renderer runs, unchanged. This module writes the browser's from them, as text (as
//! `gpu::kernel_source` patches the kernel for a scene's features), and gives the Rust side the numbers that go with
//! it. `THREERT_MERGED=1` runs the same layout natively (software traversal), so the tests that check the renderer
//! check it too (docs/RUNTIME.md "The engine in a browser").
//!
//! What is merged, and how:
//!  - The kernel's seven tables (`TABLES`: materials, lights, emitters, texture pages, objects, instances, media)
//!    become the arrays of one struct, `MT: MergedTables`, in one buffer. Their lengths are written into the source
//!    (a struct's arrays are sized), each a power of two no less than `TABLE_MIN`, so a scene that grows compiles a
//!    kernel again only when a table doubles. The renderer keeps writing its own buffers; each frame copies them into
//!    the struct's (`Gpu::sync_tables`; they are small).
//!  - The kernel's six group-1 buffers (`KERNEL_LIVE`: the radiance cache, what the live passes read
//!    of a trace) become the arrays of `ML: MergedLive`, one buffer that they are ranges of (`gpu::KernelBufs`).
//!  - The live passes' buffers of one element type are ranges of one slab each (`SLABS`), indexed from an offset
//!    the pass reads from a uniform (`LO`): the source does not change with the size of the view. (The lasting
//!    buffers' two halves alternate by a uniform, live.wgsl `this_half`.) `MK` is the kernel's `ML` buffer read as
//!    vec4s.
//!
//! A use of a merged name the generator does not know (anything but `name[...]`, `&name[...]` and
//! `arrayLength(&name)`), a declaration that is not found exactly once, a name the generator adds that the source
//! already has: each stops the renderer with the name and the line. Locals and parameters that shadow a merged name
//! are followed through their scopes and left alone. The test below reads what comes out with naga: it is valid WGSL,
//! no stage has more than `STORAGE_BUFFERS`, and the structs' arrays are where the Rust side puts them.

/// WebGPU's default `maxStorageBuffersPerShaderStage`: what the merged layout fits.
pub const STORAGE_BUFFERS: u32 = 8;

/// One of the kernel's buffers that becomes an array of a merged struct.
pub struct Member {
    pub name: &'static str,
    /// Its element type, the element's size and alignment in a storage buffer.
    pub ty: &'static str,
    pub stride: u64,
    pub align: u64,
    /// Its declaration in pathtrace.wgsl, as written.
    pub decl: &'static str,
}

/// The kernel's tables, in `MergedTables`' order (group 0, binding `TABLES_BINDING`).
pub const TABLES: [Member; 7] = [
    Member { name: "materials", ty: "Material", stride: 288, align: 16, decl: "@group(0) @binding(4) var<storage, read> materials: array<Material>;" },
    Member { name: "lights", ty: "Light", stride: 80, align: 16, decl: "@group(0) @binding(5) var<storage, read> lights: array<Light>;" },
    Member { name: "emitters", ty: "vec4<f32>", stride: 16, align: 16, decl: "@group(0) @binding(6) var<storage, read> emitters: array<vec4<f32>>;" },
    Member { name: "tex_info", ty: "vec4<f32>", stride: 16, align: 16, decl: "@group(0) @binding(14) var<storage, read> tex_info: array<vec4<f32>>;" },
    Member { name: "objects", ty: "vec4<f32>", stride: 16, align: 16, decl: "@group(0) @binding(15) var<storage, read> objects: array<vec4<f32>>;" },
    Member { name: "instances", ty: "Instance", stride: 160, align: 16, decl: "@group(0) @binding(17) var<storage, read> instances: array<Instance>;" },
    Member { name: "media", ty: "Medium", stride: 224, align: 16, decl: "@group(0) @binding(18) var<storage, read> media: array<Medium>;" },
];
pub const TABLES_BINDING: u32 = 4;
/// The least length of each table's array: scenes below it share one kernel source.
pub const TABLE_MIN: [u32; 7] = [64, 64, 1024, 256, 1024, 64, 16];

/// The kernel's group-1 buffers, in `MergedLive`'s order (group 1, binding `KERNEL_LIVE_BINDING`).
pub const KERNEL_LIVE: [Member; 6] = [
    Member { name: "cache_keys", ty: "atomic<u32>", stride: 4, align: 4, decl: "@group(1) @binding(1) var<storage, read_write> cache_keys: array<atomic<u32>>;" },
    Member { name: "cache_acc", ty: "atomic<u32>", stride: 4, align: 4, decl: "@group(1) @binding(2) var<storage, read_write> cache_acc: array<atomic<u32>>;" },
    Member { name: "cache_val", ty: "vec4<f32>", stride: 16, align: 16, decl: "@group(1) @binding(3) var<storage, read_write> cache_val: array<vec4<f32>>;" },
    Member { name: "refl", ty: "vec4<f32>", stride: 16, align: 16, decl: "@group(1) @binding(5) var<storage, read_write> refl: array<vec4<f32>>;" },
    Member { name: "motion_out", ty: "vec4<f32>", stride: 16, align: 16, decl: "@group(1) @binding(6) var<storage, read_write> motion_out: array<vec4<f32>>;" },
    Member { name: "direct_out", ty: "vec4<f32>", stride: 16, align: 16, decl: "@group(1) @binding(7) var<storage, read_write> direct_out: array<vec4<f32>>;" },
];
pub const KERNEL_LIVE_BINDING: u32 = 1;
/// `MergedLive`'s last array, after `KERNEL_LIVE`'s: the loop guard's count (`guard`; its first word: the invocations
/// a loop of which was stopped). Always there in the merged layout, written only by a kernel with the guard.
pub const GUARD_TRIPS: Member = Member { name: "loop_guard_trips", ty: "atomic<u32>", stride: 4, align: 4, decl: "" };
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
    (out, at.next_multiple_of(16))
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

/// A slab of the live passes: buffers of one element type as ranges of one buffer.
pub struct Slab {
    pub name: &'static str,
    pub ty: &'static str,
    pub stride: u64,
    pub binding: u32,
    pub read_only: bool,
}

pub const SLABS: [Slab; 5] = [
    Slab { name: "MV4", ty: "vec4<f32>", stride: 16, binding: 2, read_only: false },
    Slab { name: "MU2", ty: "vec2<u32>", stride: 8, binding: 4, read_only: false },
    Slab { name: "MF1", ty: "f32", stride: 4, binding: 15, read_only: false },
    Slab { name: "MU4", ty: "vec4<u32>", stride: 16, binding: 17, read_only: false },
    // The kernel's `ML` buffer (what the live passes read of a trace), as vec4s: every array of it starts at a
    // multiple of 16 bytes.
    Slab { name: "MK", ty: "vec4<f32>", stride: 16, binding: 25, read_only: true },
];
pub const SLAB_V4: usize = 0;
pub const SLAB_U2: usize = 1;
pub const SLAB_F1: usize = 2;
pub const SLAB_U4: usize = 3;
pub const SLAB_K: usize = 4;

/// One of the live passes' buffers that becomes a range of a slab: its binding in live.wgsl as written (what
/// `live::Targets` binds there), its slab, its declaration up to the `;`.
pub struct LiveBuf {
    pub name: &'static str,
    pub binding: u32,
    pub slab: usize,
    pub decl: &'static str,
}

/// In `MergedOffsets`' order (group 0, binding `OFFSETS_BINDING`: an (offset, length) pair of u32 each, in elements).
pub const LIVE_BUFS: [LiveBuf; 21] = [
    LiveBuf { name: "diffuse_hist", binding: 2, slab: SLAB_V4, decl: "@group(0) @binding(2) var<storage, read_write> diffuse_hist: array<vec4<f32>>;" },
    LiveBuf { name: "color_hist", binding: 3, slab: SLAB_V4, decl: "@group(0) @binding(3) var<storage, read_write> color_hist: array<vec4<f32>>;" },
    LiveBuf { name: "gbuffer", binding: 4, slab: SLAB_U2, decl: "@group(0) @binding(4) var<storage, read_write> gbuffer: array<vec2<u32>>;" },
    LiveBuf { name: "spec_hist", binding: 5, slab: SLAB_V4, decl: "@group(0) @binding(5) var<storage, read_write> spec_hist: array<vec4<f32>>;" },
    LiveBuf { name: "diffuse_mom", binding: 6, slab: SLAB_V4, decl: "@group(0) @binding(6) var<storage, read_write> diffuse_mom: array<vec4<f32>>;" },
    LiveBuf { name: "diffuse_fast", binding: 7, slab: SLAB_U2, decl: "@group(0) @binding(7) var<storage, read_write> diffuse_fast: array<vec2<u32>>;" },
    LiveBuf { name: "up_state", binding: 8, slab: SLAB_U4, decl: "@group(0) @binding(8) var<storage, read_write> up_state: array<vec4<u32>>;" },
    LiveBuf { name: "sample_vel", binding: 9, slab: SLAB_V4, decl: "@group(0) @binding(9) var<storage, read_write> sample_vel: array<vec4<f32>>;" },
    LiveBuf { name: "ae_state", binding: 14, slab: SLAB_V4, decl: "@group(0) @binding(14) var<storage, read_write> ae_state: array<vec4<f32>, 2>;" },
    LiveBuf { name: "lumq", binding: 15, slab: SLAB_F1, decl: "@group(0) @binding(15) var<storage, read_write> lumq: array<f32>;" },
    LiveBuf { name: "lumb", binding: 16, slab: SLAB_F1, decl: "@group(0) @binding(16) var<storage, read_write> lumb: array<f32>;" },
    LiveBuf { name: "fa", binding: 17, slab: SLAB_V4, decl: "@group(0) @binding(17) var<storage, read_write> fa: array<vec4<f32>>;" },
    LiveBuf { name: "fb", binding: 18, slab: SLAB_V4, decl: "@group(0) @binding(18) var<storage, read_write> fb: array<vec4<f32>>;" },
    LiveBuf { name: "bloom_a", binding: 19, slab: SLAB_V4, decl: "@group(0) @binding(19) var<storage, read_write> bloom_a: array<vec4<f32>>;" },
    LiveBuf { name: "bloom_b", binding: 20, slab: SLAB_V4, decl: "@group(0) @binding(20) var<storage, read_write> bloom_b: array<vec4<f32>>;" },
    LiveBuf { name: "spec_pp", binding: 21, slab: SLAB_U2, decl: "@group(0) @binding(21) var<storage, read_write> spec_pp: array<vec2<u32>>;" },
    LiveBuf { name: "sb", binding: 22, slab: SLAB_V4, decl: "@group(0) @binding(22) var<storage, read_write> sb: array<vec4<f32>>;" },
    LiveBuf { name: "refl_in", binding: 23, slab: SLAB_K, decl: "@group(0) @binding(23) var<storage, read> refl_in: array<vec4<f32>>;" },
    LiveBuf { name: "refl_res", binding: 24, slab: SLAB_V4, decl: "@group(0) @binding(24) var<storage, read_write> refl_res: array<vec4<f32>>;" },
    LiveBuf { name: "motion_in", binding: 25, slab: SLAB_K, decl: "@group(0) @binding(25) var<storage, read> motion_in: array<vec4<f32>>;" },
    LiveBuf { name: "direct_in", binding: 26, slab: SLAB_K, decl: "@group(0) @binding(26) var<storage, read> direct_in: array<vec4<f32>>;" },
];
pub const OFFSETS_BINDING: u32 = 31;
/// `MergedOffsets`' size: a pair for each of `LIVE_BUFS`, and one unused that makes it a multiple of 16 bytes.
pub const OFFSETS_BYTES: usize = (LIVE_BUFS.len() + 1) * 8;

/// The live passes' storage bindings that stay as they are: (binding, read only).
pub const LIVE_KEPT: [(u32, bool); 3] = [(1, true), (10, false), (13, false)];

/// How a merged name's uses are rewritten.
struct Rule {
    name: &'static str,
    /// What `name[` becomes, and what goes before the `]` that closes it.
    open: String,
    close: String,
    /// What `arrayLength(&name)` becomes.
    len: String,
}

fn fail(what: &str, src: &str, at: usize) -> ! {
    panic!("merged layout: {what} (line {})", src[..at].matches('\n').count() + 1)
}

/// The identifier at `i` (bytes), if one starts there.
fn ident(b: &[u8], i: usize) -> Option<usize> {
    let start = |c: u8| c.is_ascii_alphabetic() || c == b'_';
    if i >= b.len() || !start(b[i]) {
        return None;
    }
    let mut j = i + 1;
    while j < b.len() && (start(b[j]) || b[j].is_ascii_digit()) {
        j += 1;
    }
    Some(j)
}

fn skip_space(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

/// Rewrites every use of the rules' names in `src` (WGSL): a scan that knows comments, blocks and the locals and
/// parameters that shadow a name (from the end of their declaration to the end of their block).
fn rewrite(src: &str, rules: &[Rule]) -> String {
    let b = src.as_bytes();
    let rule = |name: &[u8]| rules.iter().position(|r| r.name.as_bytes() == name);
    let mut out: Vec<u8> = Vec::with_capacity(b.len() + b.len() / 16);
    // Blocks and brackets open; the brackets a rewritten index closes with more than `]`.
    let (mut depth, mut brackets, mut parens) = (0usize, 0usize, 0usize);
    let mut closes: Vec<(usize, &str)> = vec![];
    // Names shadowed, each with the block depth it lives in; those declared in the statement being read.
    let mut shadows: Vec<(usize, usize)> = vec![];
    let mut pending: Vec<(usize, usize)> = vec![];
    // The last byte of code before here (not space, not comment); after `var` / `let` / `const` in a function; in a
    // function's header (its parameters).
    let (mut last, mut declares, mut header) = (0u8, false, false);
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            let j = b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |k| i + k);
            out.extend_from_slice(&b[i..j]);
            i = j;
            continue;
        }
        if b[i..].starts_with(b"/*") {
            let j = src[i + 2..].find("*/").map_or(b.len(), |k| i + 2 + k + 2);
            out.extend_from_slice(&b[i..j]);
            i = j;
            continue;
        }
        let Some(j) = ident(b, i) else {
            let c = b[i];
            match c {
                b'{' => {
                    depth += 1;
                    header = false;
                }
                b'}' => {
                    depth = depth.checked_sub(1).unwrap_or_else(|| fail("an unmatched }", src, i));
                    shadows.retain(|s| s.1 <= depth);
                }
                b';' => shadows.append(&mut pending),
                b'(' => parens += 1,
                b')' => parens = parens.saturating_sub(1),
                b'[' => brackets += 1,
                b']' => {
                    if closes.last().is_some_and(|c| c.0 == brackets) {
                        out.extend_from_slice(closes.pop().unwrap().1.as_bytes());
                    }
                    brackets = brackets.checked_sub(1).unwrap_or_else(|| fail("an unmatched ]", src, i));
                }
                _ => {}
            }
            if !c.is_ascii_whitespace() {
                last = c;
            }
            out.push(c);
            i += 1;
            continue;
        };
        let word = &b[i..j];
        let after = skip_space(b, j);
        let next = b.get(after).copied().unwrap_or(0);
        let (was, declared) = (last, std::mem::take(&mut declares));
        last = b'a';
        if depth > 0 && matches!(word, b"var" | b"let" | b"const") {
            declares = true;
        } else if depth == 0 && word == b"fn" {
            header = true;
        } else if word == b"arrayLength" && next == b'(' {
            // `arrayLength(&name)` of a merged name that nothing shadows: its length.
            let amp = skip_space(b, after + 1);
            if b.get(amp) == Some(&b'&') {
                let n0 = skip_space(b, amp + 1);
                if let Some(n1) = ident(b, n0) {
                    let end = skip_space(b, n1);
                    if let Some(r) = rule(&b[n0..n1]).filter(|r| !shadows.iter().any(|s| s.0 == *r)) {
                        if b.get(end) != Some(&b')') {
                            fail(&format!("arrayLength of something inside `{}`", rules[r].name), src, i);
                        }
                        out.extend_from_slice(rules[r].len.as_bytes());
                        last = b')';
                        i = end + 1;
                        continue;
                    }
                }
            }
        } else if let Some(r) = rule(word) {
            let name = rules[r].name;
            if declared {
                // A local of that name: it shadows the buffer from the end of this statement to the end of its block
                // (a `for`'s own variable: the loop's block).
                pending.push((r, depth + (parens > 0) as usize));
            } else if was == b'.' {
                // A member of something else.
            } else if next == b':' && b.get(after + 1) != Some(&b':') {
                // A parameter of that name shadows the buffer in the function's body; a struct's member does nothing.
                if header {
                    shadows.push((r, 1));
                }
            } else if shadows.iter().any(|s| s.0 == r) {
                // The local.
            } else if next == b'[' {
                out.extend_from_slice(rules[r].open.as_bytes());
                brackets += 1;
                if !rules[r].close.is_empty() {
                    closes.push((brackets, &rules[r].close));
                }
                last = b'[';
                i = after + 1;
                continue;
            } else {
                fail(&format!("`{name}` is used in a way the generator does not know"), src, i);
            }
        }
        out.extend_from_slice(word);
        i = j;
    }
    if depth != 0 || brackets != 0 || !closes.is_empty() {
        fail("unbalanced braces or brackets", src, b.len());
    }
    String::from_utf8(out).expect("the source's own bytes")
}

/// Takes the declaration `decl` out of `s` (it is there exactly once), leaving `with` in its place.
fn take(s: &mut String, decl: &str, with: &str) {
    assert_eq!(s.matches(decl).count(), 1, "merged layout: the declaration {decl:?}");
    *s = s.replacen(decl, with, 1);
}

/// Stops if the source already has a name the generator adds.
fn fresh(src: &str, names: &[&str]) {
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            i = b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |k| i + k);
        } else if let Some(j) = ident(b, i) {
            if names.iter().any(|n| n.as_bytes() == &b[i..j]) {
                fail(&format!("the source already has a `{}`", &src[i..j]), src, i);
            }
            i = j;
        } else {
            i += 1;
        }
    }
}

fn merged_struct(name: &str, members: &[&Member], caps: &[u32]) -> String {
    let fields: Vec<String> = members.iter().zip(caps).map(|(m, c)| format!("    {}: array<{}, {c}>,", m.name, m.ty)).collect();
    format!("struct {name} {{\n{}\n}};\n", fields.join("\n"))
}

/// The kernel (pathtrace.wgsl as a scene's variant: `gpu::kernel_source`, its functions spliced in) in the merged
/// layout for arrays of `caps`' lengths: group 0 binds tri_verts, attrs, nodes, `MT`, env_cdf, accum and prev_verts,
/// group 1 `ML`.
pub fn kernel(src: &str, caps: &Caps) -> String {
    fresh(src, &["MT", "ML", "MergedTables", "MergedLive", GUARD_TRIPS.name]);
    let mut s = src.to_string();
    for (k, m) in TABLES.iter().enumerate() {
        let with = if k == 0 {
            format!("{}@group(0) @binding({TABLES_BINDING}) var<storage, read> MT: MergedTables;", merged_struct("MergedTables", &TABLES.iter().collect::<Vec<_>>(), &caps.tables))
        } else {
            String::new()
        };
        take(&mut s, m.decl, &with);
    }
    for (k, m) in KERNEL_LIVE.iter().enumerate() {
        let with = if k == 0 {
            format!("{}@group(1) @binding({KERNEL_LIVE_BINDING}) var<storage, read_write> ML: MergedLive;", merged_struct("MergedLive", &KERNEL_LIVE.iter().chain([&GUARD_TRIPS]).collect::<Vec<_>>(), &caps.live.iter().copied().chain([GUARD_WORDS]).collect::<Vec<_>>()))
        } else {
            String::new()
        };
        take(&mut s, m.decl, &with);
    }
    let rule = |var: &str, m: &Member, cap: u32| Rule { name: m.name, open: format!("{var}.{}[", m.name), close: String::new(), len: format!("{cap}u") };
    let rules: Vec<Rule> = TABLES.iter().zip(caps.tables).map(|(m, c)| rule("MT", m, c)).chain(KERNEL_LIVE.iter().zip(caps.live).map(|(m, c)| rule("ML", m, c))).collect();
    rewrite(&s, &rules)
}

/// The uniform tables (`gpu::GpuOptions::uniform_tables`; docs/RUNTIME.md "Android"): which of `TABLES` the kernel
/// reads from uniform buffers instead of storage buffers, in the layout as written. Qualcomm's Adreno 740 driver
/// (Vulkan, Android 13) gave the kernel other buffers' contents for these four: a model's textures came out as
/// another table's numbers, instance records sent hits to the wrong triangles, an emissive object's power read as
/// something else's and lit nothing. As uniform buffers they read right. They are the four of the kernel's read-only
/// tables that are bounded (the others: `uniform_tables`).
pub const UNIFORM_TABLES: [usize; 4] = [3, 4, 5, 6];
/// Their arrays' lengths (a uniform buffer's array is sized): what 64 KiB hold, the size every device gives a uniform
/// binding (WebGPU's `maxUniformBufferBindingSize`, Vulkan's least `maxUniformBufferRange` on the phones this is
/// for), and for the media what a scene may have (`scene::MAX_MEDIA`). In a scene's terms: 2,048 textures, 1,365
/// objects (1,024 with instance colours), 409 instance records, every medium.
pub const UNIFORM_LEN: [u32; 4] = [4096, 4096, 409, 32];

/// The kernel (as `kernel` takes it: a scene's variant, its functions spliced in) with the uniform tables: the four
/// declarations as sized uniform arrays at their own bindings, and every index into them held to the array
/// (`min(u32(i), last)`). A scene with more entries than an array holds reads the last one for the rest (the renderer
/// says so when it uploads one, `Gpu::upload`); nothing is read outside the buffer, which the kernel's unchecked
/// indexing would otherwise do. Everything else is the source as written.
///
/// The kernel's other storage buffers stay storage buffers, each for its reason:
///  - `materials` (288 B each), `lights` (80 B): the device drew right with them as they are, and 64 KiB would hold
///    227 materials;
///  - `emitters` (the emitter CDF and the light tree), `env_cdf` (the environment's CDF, then the procedural
///    geometry's data), `tri_verts`, `attrs`, `nodes`, `prev_verts` (and the vertex colours after them): megabytes,
///    which no uniform binding holds;
///  - `accum` and group 1 (the radiance cache, the trace's outputs): written by the kernel.
pub fn uniform_tables(src: &str) -> String {
    let mut s = src.to_string();
    let mut rules = vec![];
    for (&k, &len) in UNIFORM_TABLES.iter().zip(&UNIFORM_LEN) {
        let m = &TABLES[k];
        let sized = m.decl.replace("var<storage, read>", "var<uniform>").replace(&format!("array<{}>", m.ty), &format!("array<{}, {len}>", m.ty));
        assert!(sized.contains("var<uniform>") && sized.ends_with(&format!(", {len}>;")), "uniform tables: the declaration {:?}", m.decl);
        take(&mut s, m.decl, &sized);
        rules.push(Rule { name: m.name, open: format!("{}[min(u32(", m.name), close: format!("), {}u)", len - 1), len: format!("{len}u") });
    }
    rewrite(&s, &rules)
}

/// The live passes (live.wgsl) in the merged layout: sample_px, out_px and ae_bins as they are, the slabs, and the
/// offsets `LO`.
pub fn live(src: &str) -> String {
    let mut names: Vec<&str> = SLABS.iter().map(|s| s.name).collect();
    names.extend(["LO", "MergedOffsets", "merged_pad"]);
    fresh(src, &names);
    let mut s = src.to_string();
    for b in LIVE_BUFS.iter() {
        take(&mut s, b.decl, "");
    }
    // (A last pair unused: the struct is 208 bytes, a multiple of 16. WebGPU does not ask for that, but wgpu's
    // downlevel targets do of a uniform binding's size, and it costs nothing; `OFFSETS_BYTES`.)
    let fields: Vec<String> = LIVE_BUFS.iter().map(|b| format!("    {}: vec2<u32>,", b.name)).chain(["    merged_pad: vec2<u32>,".to_string()]).collect();
    let mut head = format!("// The merged layout (renderer/src/merged.rs): each buffer's (first element, length) in its slab.\nstruct MergedOffsets {{\n{}\n}};\n@group(0) @binding({OFFSETS_BINDING}) var<uniform> LO: MergedOffsets;\n", fields.join("\n"));
    for sl in &SLABS {
        head += &format!("@group(0) @binding({}) var<storage, {}> {}: array<{}>;\n", sl.binding, if sl.read_only { "read" } else { "read_write" }, sl.name, sl.ty);
    }
    // After the uniforms' declaration (the file's first binding).
    let anchor = "@group(0) @binding(0) var<uniform> L: LiveParams;\n";
    take(&mut s, anchor, &format!("{anchor}{head}"));
    let rules: Vec<Rule> = LIVE_BUFS.iter().map(|b| Rule { name: b.name, open: format!("{}[LO.{}.x + (", SLABS[b.slab].name, b.name), close: ")".into(), len: format!("LO.{}.y", b.name) }).collect();
    rewrite(&s, &rules)
}

/// A bound on every loop of a shader: each `loop`, `for` and `while` body begins with a count of the invocation's
/// loop iterations so far, and leaves the loop past `limit`. The kernel's traversals (the BVH, the light tree, the
/// procedural boxes) follow indices they read from buffers and end only if the data is sound: on data that is not, a
/// dispatch never ends, and a GPU that never gets a dispatch back takes the whole machine's display with it (it did:
/// docs/RUNTIME.md "The engine in a browser"). With the bound a dispatch always ends: wrong pixels instead of no
/// frame. No formula changes, but the image is not bit-identical to the kernel's without the guard: the compiler
/// arranges the arithmetic of the functions it is in differently (hello world, one sample a pixel: 354 of 43,200
/// numbers differ, by at most 2 x 10^-7 of their value). Between the two layouts, guard or not, the bytes are equal.
/// `THREERT_LOOP_GUARD=<limit>` natively (`gpu::loop_guard_of`: a kernel with units has it with nothing set); a
/// browser's kernel always has it.
///
/// A stopped loop is counted, not hidden: the invocation that reaches the limit adds one to the first word of `trips`
/// (an `array<atomic<u32>, 4>` the source can name: `ML.loop_guard_trips` in the merged layout, the buffer `guard_decl`
/// declares in the layout as written), which the renderer reads back (`live::LiveCore::guard_trips`, an offline
/// render's error). A frame with a count above zero is a frame with wrong pixels, and says so.
pub fn guard(src: &str, limit: u32, trips: &str) -> String {
    fresh(src, &["loop_guard", "LOOP_GUARD_N"]);
    let b = src.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len() + 4096);
    let mut i = 0;
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            let j = b[i..].iter().position(|&c| c == b'\n').map_or(b.len(), |k| i + k);
            out.extend_from_slice(&b[i..j]);
            i = j;
            continue;
        }
        if b[i..].starts_with(b"/*") {
            let j = src[i + 2..].find("*/").map_or(b.len(), |k| i + 2 + k + 2);
            out.extend_from_slice(&b[i..j]);
            i = j;
            continue;
        }
        let Some(j) = ident(b, i) else {
            out.push(b[i]);
            i += 1;
            continue;
        };
        let word = &b[i..j];
        let mut body = None;
        let after = skip_space(b, j);
        if word == b"loop" && b.get(after) == Some(&b'{') {
            body = Some(after);
        } else if matches!(word, b"for" | b"while") && b.get(after) == Some(&b'(') {
            // The header's closing parenthesis, then the body.
            let (mut depth, mut k) = (0usize, after);
            while k < b.len() {
                match b[k] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
            let open = skip_space(b, k + 1);
            if b.get(open) != Some(&b'{') {
                fail("a loop without a body the guard can open", src, i);
            }
            body = Some(open);
        }
        match body {
            Some(open) => {
                out.extend_from_slice(&b[i..=open]);
                out.extend_from_slice(b" if (loop_guard()) { break; }");
                i = open + 1;
            }
            None => {
                out.extend_from_slice(word);
                i = j;
            }
        }
    }
    let mut s = String::from_utf8(out).expect("the source's own bytes");
    s += &format!("\n// The loop guard (renderer/src/merged.rs `guard`): an invocation's loop iterations, all its loops together; the\n// invocations stopped at the limit are counted.\nvar<private> LOOP_GUARD_N: u32 = 0u;\nfn loop_guard() -> bool {{\n    LOOP_GUARD_N += 1u;\n    if (LOOP_GUARD_N == {limit}u + 1u) {{ atomicAdd(&{trips}[0], 1u); }}\n    return LOOP_GUARD_N > {limit}u;\n}}\n");
    s
}

/// The loop guard's count as a buffer of its own, for a kernel in the layout as written (the merged layout has it in
/// `MergedLive`): the declaration `guard`'s `trips` names.
pub fn guard_decl() -> String {
    format!("@group(1) @binding({GUARD_BINDING}) var<storage, read_write> {}: array<atomic<u32>, {GUARD_WORDS}>;\n", GUARD_TRIPS.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KERNEL: &str = include_str!("../shaders/pathtrace.wgsl");
    const LIVE: &str = include_str!("../shaders/live.wgsl");

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

    /// Every kernel variant the source patches make, in the merged layout: valid WGSL, no entry point with more than
    /// eight storage buffers, none declared beyond them (a pipeline layout counts what is declared), and the merged
    /// structs' arrays where `offsets` puts them. The source as written has 19, so the count is the generator's.
    #[test]
    fn the_merged_kernel_fits_eight_storage_buffers() {
        let (m, info) = parse(KERNEL);
        let (used, declared) = storage_buffers(&m, &info);
        assert_eq!(declared, 19, "the kernel as written");
        assert_eq!(used.iter().map(|u| u.1).max(), Some(19), "{used:?}");
        let caps = caps();
        for features in [0, crate::gpu::F_VCOLOR, crate::gpu::F_GRID, crate::gpu::F_TINT | crate::gpu::F_UNLIT | crate::gpu::F_BACKGROUND, crate::gpu::F_SOURCE] {
            let src = kernel(&crate::gpu::kernel_source(KERNEL, features), &caps);
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
    /// every index into the four is held to its array, so nothing is read outside a uniform buffer.
    #[test]
    fn the_uniform_tables_are_four_sized_uniform_arrays() {
        // A name's index sites in code (not comments): `name[` that is not part of a longer name or a member.
        let sites = |src: &str, name: &str| {
            let code: String = src.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
            let open = format!("{name}[");
            code.match_indices(&open).filter(|(k, _)| !code[..*k].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.')).count()
        };
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
        let (written, _) = parse(KERNEL);
        for features in [0, crate::gpu::F_VCOLOR, crate::gpu::F_GRID, crate::gpu::F_TINT | crate::gpu::F_UNLIT | crate::gpu::F_BACKGROUND, crate::gpu::F_SOURCE] {
            let base = crate::gpu::kernel_source(KERNEL, features);
            let src = uniform_tables(&base);
            let (m, info) = parse(&src);
            assert_eq!(storage_buffers(&m, &info).1, 15, "features {features:#x}");
            let tables: Vec<(String, u32, u32)> = globals(&m, true).into_iter().filter(|g| g.0 != "P" && g.0 != "energy_lut").collect();
            let want: Vec<(String, u32, u32)> = UNIFORM_TABLES.iter().zip(UNIFORM_LEN).zip([14, 15, 17, 18]).map(|((&k, len), binding)| (TABLES[k].name.to_string(), binding, (TABLES[k].stride * len as u64) as u32)).collect();
            assert_eq!(tables, want, "features {features:#x}");
            // The storage buffers that stay: the same names at the same bindings as the kernel as written.
            let kept = |m: &naga::Module| globals(m, false).into_iter().map(|g| (g.0, g.1)).filter(|g| !want.iter().any(|w| w.0 == g.0)).collect::<Vec<_>>();
            assert_eq!(kept(&m), kept(&written), "features {features:#x}");
            for (&k, len) in UNIFORM_TABLES.iter().zip(UNIFORM_LEN) {
                let name = TABLES[k].name;
                let held = format!("{name}[min(u32(");
                assert!(sites(&base, name) >= 3, "{name}: the kernel indexes it");
                assert_eq!(src.matches(&held).count(), sites(&base, name), "{name}: every index is held (features {features:#x})");
                assert_eq!(sites(&src, name), src.matches(&held).count(), "{name}: no index is left as written");
                assert!(src.contains(&format!("), {}u)]", len - 1)));
            }
        }
        // One index, as it is rewritten: an expression with brackets of its own, and a table inside another's index.
        let src = "@group(0) @binding(14) var<storage, read> tex_info: array<vec4<f32>>;\n@group(0) @binding(15) var<storage, read> objects: array<vec4<f32>>;\n@group(0) @binding(17) var<storage, read> instances: array<Instance>;\n@group(0) @binding(18) var<storage, read> media: array<Medium>;\nfn f(a: array<u32, 4>, k: i32) -> f32 { return objects[a[1] * 3u + 1u].w + tex_info[u32(objects[k].x)].y + media[0].b.x; }\n";
        let out = uniform_tables(src);
        assert!(out.starts_with("@group(0) @binding(14) var<uniform> tex_info: array<vec4<f32>, 4096>;\n@group(0) @binding(15) var<uniform> objects: array<vec4<f32>, 4096>;\n@group(0) @binding(17) var<uniform> instances: array<Instance, 409>;\n@group(0) @binding(18) var<uniform> media: array<Medium, 32>;\n"), "{out}");
        assert!(out.ends_with("{ return objects[min(u32(a[1] * 3u + 1u), 4095u)].w + tex_info[min(u32(u32(objects[min(u32(k), 4095u)].x)), 4095u)].y + media[min(u32(0), 31u)].b.x; }\n"), "{out}");
    }

    /// The same for the live passes (28 as written).
    #[test]
    fn the_merged_live_passes_fit_eight_storage_buffers() {
        let (m, info) = parse(LIVE);
        assert_eq!(storage_buffers(&m, &info).1, 24, "the live passes as written");
        let passes = m.entry_points.len();
        let (m, info) = parse(&live(LIVE));
        let (used, declared) = storage_buffers(&m, &info);
        assert_eq!(declared as u32, STORAGE_BUFFERS);
        assert!(used.iter().all(|u| u.1 as u32 <= STORAGE_BUFFERS), "{used:?}");
        assert_eq!(used.len(), passes, "every pass is still there");
        // The offsets' struct: a pair of u32 each, in order (what `live::Targets` writes).
        assert_eq!(layout(&m, "MergedOffsets"), ((0..22).map(|k| k * 8).collect(), OFFSETS_BYTES as u64));
        assert_eq!(OFFSETS_BYTES % 16, 0);
        assert_eq!(LIVE_BUFS.len(), 21);
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

    /// The scan follows scopes: a local or a parameter of a merged name is left alone, from the end of its
    /// declaration to the end of its block; comments are not read.
    #[test]
    fn locals_that_shadow_a_buffer_are_left_alone() {
        let rules = [Rule { name: "refl", open: "ML.refl[".into(), close: String::new(), len: "7u".into() }, Rule { name: "fa", open: "MV4[LO.fa.x + (".into(), close: ")".into(), len: "LO.fa.y".into() }];
        let src = "struct S { refl: f32 };\nfn f(i: u32) -> f32 {\n    let a = refl[i].x; // refl[0] }\n    {\n        let refl = refl[i + 1u].y;\n        if (refl > 0.0) { return refl; }\n    }\n    var s: S;\n    return s.refl + refl[arrayLength(&refl) - 1u].x + fa[fa[i].x].y;\n}\nfn g(refl: array<f32, 4>) -> f32 { return refl[1]; }\nfn h() -> f32 { return refl[2].x; }\n";
        let want = "struct S { refl: f32 };\nfn f(i: u32) -> f32 {\n    let a = ML.refl[i].x; // refl[0] }\n    {\n        let refl = ML.refl[i + 1u].y;\n        if (refl > 0.0) { return refl; }\n    }\n    var s: S;\n    return s.refl + ML.refl[7u - 1u].x + MV4[LO.fa.x + (MV4[LO.fa.x + (i)].x)].y;\n}\nfn g(refl: array<f32, 4>) -> f32 { return refl[1]; }\nfn h() -> f32 { return ML.refl[2].x; }\n";
        assert_eq!(rewrite(src, &rules), want);
    }

    /// The loop guard opens every loop of the kernel and of the live passes, in the layout as written and in the
    /// merged one, and what comes out is valid WGSL.
    #[test]
    fn the_guard_bounds_every_loop() {
        let loops = |s: &str| {
            let code: String = s.lines().map(|l| l.split("//").next().unwrap_or("")).collect::<Vec<_>>().join("\n");
            let words = |w: &str| code.match_indices(w).filter(|(k, _)| !code[..*k].ends_with(|c: char| c.is_alphanumeric() || c == '_')).count();
            words("loop {") + words("for (") + words("while (")
        };
        // The kernel as written (its count a buffer of its own) and merged (its count in `MergedLive`).
        let merged = kernel(&crate::gpu::kernel_source(KERNEL, crate::gpu::F_SOURCE), &caps());
        for (src, trips) in [(format!("{KERNEL}{}", guard_decl()), "loop_guard_trips"), (merged, "ML.loop_guard_trips")] {
            let g = guard(&src, 1 << 16, trips);
            assert!(loops(&src) > 40, "the kernel's loops");
            assert_eq!(g.matches("if (loop_guard()) { break; }").count(), loops(&src));
            let (m, info) = parse(&g);
            let (used, declared) = storage_buffers(&m, &info);
            // (As written: one buffer more, where the device has room. Merged: still eight.)
            assert!(declared == 20 || declared as u32 == STORAGE_BUFFERS, "{declared}: {used:?}");
        }
        let g = guard("fn f() {\n    for (var i = 0u; i < n(2u); i++) { x(); }\n    loop { // {\n        break;\n    }\n    while (a) { }\n}\n", 9, "T");
        assert!(g.starts_with("fn f() {\n    for (var i = 0u; i < n(2u); i++) { if (loop_guard()) { break; } x(); }\n    loop { if (loop_guard()) { break; } // {\n        break;\n    }\n    while (a) { if (loop_guard()) { break; } }\n}\n"), "{g}");
        assert!(g.ends_with("fn loop_guard() -> bool {\n    LOOP_GUARD_N += 1u;\n    if (LOOP_GUARD_N == 9u + 1u) { atomicAdd(&T[0], 1u); }\n    return LOOP_GUARD_N > 9u;\n}\n"), "{g}");
    }

    #[test]
    #[should_panic(expected = "`refl` is used in a way the generator does not know (line 2)")]
    fn a_use_the_generator_does_not_know_stops_it() {
        rewrite("fn f() {\n    let p = &refl;\n}\n", &[Rule { name: "refl", open: "ML.refl[".into(), close: String::new(), len: "7u".into() }]);
    }

    #[test]
    #[should_panic(expected = "the declaration")]
    fn a_declaration_that_moved_stops_it() {
        kernel(&KERNEL.replace("var<storage, read> lights: array<Light>;", "var<storage, read>  lights: array<Light>;"), &caps());
    }
}
