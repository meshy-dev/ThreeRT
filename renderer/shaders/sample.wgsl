// The kernel's random numbers: shuffled, Owen-scrambled Sobol (Burley 2020; Cycles' Sobol-Burley). Four dimensions,
// more by padding: each decision has a dimension id that seeds the scramble. renderer/src/sample.rs: the same on the CPU.

// Sobol dimensions 1-3 (Joe & Kuo, bit-reversed) of a 16-bit bit-reversed index, unrolled. Dimension 0: reverseBits.
fn sobol_d1(x: u32) -> u32 {
    var r = 0u;
    r ^= select(0u, 0x00000001u, (x & 0x80000000u) != 0u);
    r ^= select(0u, 0x00000003u, (x & 0x40000000u) != 0u);
    r ^= select(0u, 0x00000005u, (x & 0x20000000u) != 0u);
    r ^= select(0u, 0x0000000fu, (x & 0x10000000u) != 0u);
    r ^= select(0u, 0x00000011u, (x & 0x08000000u) != 0u);
    r ^= select(0u, 0x00000033u, (x & 0x04000000u) != 0u);
    r ^= select(0u, 0x00000055u, (x & 0x02000000u) != 0u);
    r ^= select(0u, 0x000000ffu, (x & 0x01000000u) != 0u);
    r ^= select(0u, 0x00000101u, (x & 0x00800000u) != 0u);
    r ^= select(0u, 0x00000303u, (x & 0x00400000u) != 0u);
    r ^= select(0u, 0x00000505u, (x & 0x00200000u) != 0u);
    r ^= select(0u, 0x00000f0fu, (x & 0x00100000u) != 0u);
    r ^= select(0u, 0x00001111u, (x & 0x00080000u) != 0u);
    r ^= select(0u, 0x00003333u, (x & 0x00040000u) != 0u);
    r ^= select(0u, 0x00005555u, (x & 0x00020000u) != 0u);
    r ^= select(0u, 0x0000ffffu, (x & 0x00010000u) != 0u);
    return r;
}
fn sobol_d2(x: u32) -> u32 {
    var r = 0u;
    r ^= select(0u, 0x00000001u, (x & 0x80000000u) != 0u);
    r ^= select(0u, 0x00000003u, (x & 0x40000000u) != 0u);
    r ^= select(0u, 0x00000006u, (x & 0x20000000u) != 0u);
    r ^= select(0u, 0x00000009u, (x & 0x10000000u) != 0u);
    r ^= select(0u, 0x00000017u, (x & 0x08000000u) != 0u);
    r ^= select(0u, 0x0000003au, (x & 0x04000000u) != 0u);
    r ^= select(0u, 0x00000071u, (x & 0x02000000u) != 0u);
    r ^= select(0u, 0x000000a3u, (x & 0x01000000u) != 0u);
    r ^= select(0u, 0x00000116u, (x & 0x00800000u) != 0u);
    r ^= select(0u, 0x00000339u, (x & 0x00400000u) != 0u);
    r ^= select(0u, 0x00000677u, (x & 0x00200000u) != 0u);
    r ^= select(0u, 0x000009aau, (x & 0x00100000u) != 0u);
    r ^= select(0u, 0x00001601u, (x & 0x00080000u) != 0u);
    r ^= select(0u, 0x00003903u, (x & 0x00040000u) != 0u);
    r ^= select(0u, 0x00007706u, (x & 0x00020000u) != 0u);
    r ^= select(0u, 0x0000aa09u, (x & 0x00010000u) != 0u);
    return r;
}
fn sobol_d3(x: u32) -> u32 {
    var r = 0u;
    r ^= select(0u, 0x00000001u, (x & 0x80000000u) != 0u);
    r ^= select(0u, 0x00000003u, (x & 0x40000000u) != 0u);
    r ^= select(0u, 0x00000004u, (x & 0x20000000u) != 0u);
    r ^= select(0u, 0x0000000au, (x & 0x10000000u) != 0u);
    r ^= select(0u, 0x0000001fu, (x & 0x08000000u) != 0u);
    r ^= select(0u, 0x0000002eu, (x & 0x04000000u) != 0u);
    r ^= select(0u, 0x00000045u, (x & 0x02000000u) != 0u);
    r ^= select(0u, 0x000000c9u, (x & 0x01000000u) != 0u);
    r ^= select(0u, 0x0000011bu, (x & 0x00800000u) != 0u);
    r ^= select(0u, 0x000002a4u, (x & 0x00400000u) != 0u);
    r ^= select(0u, 0x0000079au, (x & 0x00200000u) != 0u);
    r ^= select(0u, 0x00000b67u, (x & 0x00100000u) != 0u);
    r ^= select(0u, 0x0000101eu, (x & 0x00080000u) != 0u);
    r ^= select(0u, 0x0000302du, (x & 0x00040000u) != 0u);
    r ^= select(0u, 0x00004041u, (x & 0x00020000u) != 0u);
    r ^= select(0u, 0x0000a0c3u, (x & 0x00010000u) != 0u);
    return r;
}

// Owen scrambling of a bit-reversed integer (Vegdahl 2021's hash, a better Laine-Karras permutation).
fn owen_rev(n_in: u32, seed: u32) -> u32 {
    var n = n_in;
    n ^= n * 0x3d20adeau;
    n += seed;
    n *= (seed >> 16u) | 1u;
    n ^= n * 0x05526c56u;
    n ^= n * 0x53a22864u;
    return n;
}

fn hash_hp(x: u32) -> u32 {
    var i = x;
    i ^= i >> 16u;
    i *= 0x21f0aaadu;
    i ^= i >> 15u;
    i *= 0xd35a2d97u;
    i ^= i >> 15u;
    return i ^ 0xe6fe3bebu;
}

// One dimension (0-3) of the Sobol point at a bit-reversed index, Owen-scrambled.
fn sobol_owen(rev_index: u32, dim: u32, seed: u32) -> f32 {
    var r = 0u;
    if (dim == 0u) {
        r = reverseBits(rev_index);
    } else if (dim == 1u) {
        r = sobol_d1(rev_index);
    } else if (dim == 2u) {
        r = sobol_d2(rev_index);
    } else {
        r = sobol_d3(rev_index);
    }
    r = reverseBits(owen_rev(r, seed));
    return f32(r) * (1.0 / 4294967808.0);
}

// A path's sample: its index in the pixel's sequence, the pixel's seed, which path of the invocation it is.
var<private> RNG_INDEX: u32;
var<private> RNG_SEED: u32;
var<private> RNG_PATH: u32;

// The id of decision `slot` at the path's current vertex (`cur_depth`); `rng_sub` the k-th part of decision `d` (a
// loop's iteration, a light).
fn rng_dim(slot: u32) -> u32 { return (RNG_PATH << 28u) ^ (cur_depth << 20u) ^ slot; }
fn rng_sub(d: u32, k: u32) -> u32 { return hash_hp(d ^ hash_hp(k + 0x9e3779b9u)) | 0x80000000u; }

// A path vertex's decisions (`rng_dim`).
const D_CAMERA: u32 = 0u;    // pixel jitter, lens (4D)
const D_BSDF: u32 = 1u;      // lobe, specular or not, direction (4D)
const D_RR: u32 = 2u;        // Russian roulette
const D_PICK: u32 = 3u;      // light_pick's kind of light
const D_SUN: u32 = 4u;       // a directional light's cone (2D a light)
const D_SUN_VIS: u32 = 5u;   // its shadow ray
const D_LOCAL: u32 = 6u;     // local lights (`local_lights`)
const D_EMIT: u32 = 7u;      // emitters (`sample_emitter_ris`, `sample_emitter`)
const D_EMIT_VIS: u32 = 8u;
const D_ENV: u32 = 9u;       // the environment map (4D)
const D_ENV_VIS: u32 = 10u;
const D_FOG: u32 = 11u;      // height fog's in-scattering (`fog_scatter`)
const D_MEDIA: u32 = 12u;    // bounded media (`media_track`, `media_march`)
const D_PHASE: u32 = 13u;    // a medium's scattered direction (`sample_hg`)
const D_VOL_LIGHT: u32 = 14u; // lights at a point in a medium (`phase_direct`, `point_direct`)
const D_VOXEL: u32 = 15u;    // the radiance cache's volume points (`point_sphere`)
const D_CACHE: u32 = 16u;    // the cache's fixed-point dither

// Live: Sobol only for the camera path's first vertex (not a loop's parts: `rng_sub` sets the top bit), a hash
// elsewhere (as NVIDIA's RTXPT; all-Sobol cost 2 ms of Bistro's trace). Offline: Sobol everywhere (as Cycles).
fn rng_white(d: u32) -> bool { return LIVE && (cur_depth > 0u || RNG_PATH != 0u || d >= 0x80000000u); }
fn white(d: u32, c: u32) -> f32 { return f32(hash_hp(rng_seed(d) ^ (RNG_INDEX * 0x9e3779b9u + c * 0x85ebca6bu)) >> 8u) * (1.0 / 16777216.0); }

// Dimension id `d`'s scramble seed; the shuffled index, 16 bits (65,536 samples, then it repeats; Cycles' index mask).
fn rng_seed(d: u32) -> u32 { return RNG_SEED ^ hash_hp(d); }
fn rng_index(salt: u32) -> u32 { return owen_rev(reverseBits(RNG_INDEX), salt) & 0xffff0000u; }

fn rng1(d: u32) -> f32 {
    if (rng_white(d)) { return white(d, 0u); }
    let s = rng_seed(d);
    return sobol_owen(rng_index(s ^ 0xbff95bfeu), 0u, s ^ 0x635c77bdu);
}
fn rng2(d: u32) -> vec2<f32> {
    if (rng_white(d)) { return vec2<f32>(white(d, 0u), white(d, 1u)); }
    let s = rng_seed(d);
    let i = rng_index(s ^ 0xf8ade99au);
    return vec2<f32>(sobol_owen(i, 0u, s ^ 0xe0aaaf76u), sobol_owen(i, 1u, s ^ 0x94964d4eu));
}
fn rng3(d: u32) -> vec3<f32> {
    if (rng_white(d)) { return vec3<f32>(white(d, 0u), white(d, 1u), white(d, 2u)); }
    let s = rng_seed(d);
    let i = rng_index(s ^ 0xcaa726acu);
    return vec3<f32>(sobol_owen(i, 0u, s ^ 0x9e78e391u), sobol_owen(i, 1u, s ^ 0x67c33241u), sobol_owen(i, 2u, s ^ 0x78c395c5u));
}
fn rng4(d: u32) -> vec4<f32> {
    if (rng_white(d)) { return vec4<f32>(white(d, 0u), white(d, 1u), white(d, 2u), white(d, 3u)); }
    let s = rng_seed(d);
    let i = rng_index(s ^ 0xc2c1a055u);
    return vec4<f32>(sobol_owen(i, 0u, s ^ 0x39468210u), sobol_owen(i, 1u, s ^ 0xe9d8a845u), sobol_owen(i, 2u, s ^ 0x5f32b482u), sobol_owen(i, 3u, s ^ 0x1524cc56u));
}

// Starts sample `index` of a pixel (the sample offline, the frame live) on path `path`.
fn rng_start(pix: vec2<u32>, index: u32, stream: u32, path: u32) {
    RNG_SEED = hash_hp(pix.x ^ hash_hp(pix.y ^ hash_hp(stream)));
    RNG_INDEX = index;
    RNG_PATH = path;
}
