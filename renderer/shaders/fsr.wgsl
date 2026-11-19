// WGSL port of the FP32 EASU, RCAS, and SRTM math in AMD FidelityFX Super Resolution 1:
// https://github.com/GPUOpen-Effects/FidelityFX-FSR/blob/master/ffx-fsr/ffx_fsr1.h
//
// FidelityFX Super Resolution Sample
// Copyright (c) 2021 Advanced Micro Devices, Inc. All rights reserved.
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files(the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and / or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions :
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

@group(1) @binding(0) var fsr_easu_out: texture_storage_2d<rgba32float, write>;
@group(1) @binding(1) var fsr_easu_in: texture_2d<f32>;

// In one axis, 8 output lanes span at most 8 input centers (input <= output), plus EASU's
// border at -1/+2. Reserve one extra slot for rounding. Cache perceptual RGB and luma together.
var<workgroup> fsr_input_tile: array<vec4<f32>, 144>;
const FSR_HEADROOM: f32 = 1.0 / 1048576.0;

fn fsr_max3(c: vec3<f32>) -> f32 { return max(c.r, max(c.g, c.b)); }
fn fsr_luma(c: vec3<f32>) -> f32 { return c.g + 0.5 * (c.r + c.b); }

fn fsr_exposure() -> f32 { return clamp(L.misc.x, 0.00001, 64.0); }

// SRTM divides by (1 + max(rgb)). Apply that curve to the gamma-2 proxy instead of taking
// gamma after the curve: the shoulder keeps much more floating-point headroom for HDR highlights.
// The manual exposure sets the knee; auto exposure and display tonemapping still run after RCAS.
fn fsr_pack_hdr(rgb: vec3<f32>, gamma_knee: f32) -> vec4<f32> {
    let gamma = sqrt(rgb); // compose already guarantees finite nonnegative RGB
    let peak = fsr_max3(gamma);
    let c = gamma / max(gamma_knee + peak, (1.0 + FSR_HEADROOM) * peak);
    return vec4<f32>(c, fsr_luma(c));
}

fn fsr_unpack_hdr(c: vec3<f32>) -> vec3<f32> {
    let gamma = c / max(1.0 - fsr_max3(c), FSR_HEADROOM);
    return (gamma * gamma) / fsr_exposure();
}

fn fsr_input_position(p: vec2<u32>, scale: vec2<f32>) -> vec2<f32> {
    // FsrEasuCon: output center in input texel coordinates; the trace samples at (0.5, 0.5).
    return (vec2<f32>(p) + 0.5) * scale - 0.5;
}

// FsrEasuSetF: direction and gradient reversal for one of the four surrounding centers.
fn fsr_edge(a: f32, b: f32, c: f32, d: f32, e: f32) -> vec3<f32> {
    let dir = vec2<f32>(d - b, e - a);
    let slope = max(abs(vec2<f32>(d, e) - c), abs(c - vec2<f32>(b, a)));
    let span = min(abs(dir) / max(slope, vec2<f32>(1.0e-12)), vec2<f32>(1.0));
    return vec3<f32>(dir, dot(span, span));
}

// FsrEasuTapF: rotated and stretched polynomial Lanczos-2 with an adaptive window.
fn fsr_tap(offset: vec2<f32>, dir: vec2<f32>, axis: vec2<f32>, lobe: f32, clip: f32, delta: vec3<f32>) -> vec4<f32> {
    let v = axis * vec2<f32>(dot(offset, dir), dot(offset, vec2<f32>(-dir.y, dir.x)));
    let d2 = min(dot(v, v), clip);
    let base = 0.4 * d2 - 1.0;
    let window = lobe * d2 - 1.0;
    let weight = (1.5625 * base * base - 0.5625) * (window * window);
    return vec4<f32>(delta, 1.0) * weight;
}

@compute @workgroup_size(8, 8)
fn fsr_easu(@builtin(global_invocation_id) id: vec3<u32>, @builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) lane: u32) {
    let input = L.isz.xy;
    let output = L.size.xy;
    let scale = vec2<f32>(input) / vec2<f32>(output);
    let first = group.xy * 8u;
    let last = min(first + 7u, output - 1u);
    let origin = vec2<i32>(floor(fsr_input_position(first, scale))) - 1;
    let extent = vec2<u32>(vec2<i32>(floor(fsr_input_position(last, scale))) + 3 - origin);
    let gamma_knee = inverseSqrt(fsr_exposure());
    for (var i = lane; i < extent.x * extent.y; i += 64u) {
        let p = origin + vec2<i32>(i32(i % extent.x), i32(i / extent.x));
        let q = vec2<u32>(clamp(p, vec2<i32>(0), vec2<i32>(input) - 1));
        fsr_input_tile[i] = fsr_pack_hdr(fb[q.y * input.x + q.x].rgb, gamma_knee);
    }
    workgroupBarrier();
    if (any(id.xy >= output)) { return; }

    // FsrEasuF's 12 input taps, gathered from the tile rather than a filtered texture:
    //    b c
    //  e f g h
    //  i j k l
    //    n o
    let position = fsr_input_position(id.xy, scale);
    let pixel = floor(position);
    let pp = position - pixel;
    let q = vec2<u32>(vec2<i32>(pixel) - origin);
    let w = extent.x;
    let at = q.y * w + q.x;
    let b = fsr_input_tile[at - w];
    let c = fsr_input_tile[at - w + 1u];
    let e = fsr_input_tile[at - 1u];
    let f = fsr_input_tile[at];
    let g = fsr_input_tile[at + 1u];
    let h = fsr_input_tile[at + 2u];
    let i = fsr_input_tile[at + w - 1u];
    let j = fsr_input_tile[at + w];
    let k = fsr_input_tile[at + w + 1u];
    let l = fsr_input_tile[at + w + 2u];
    let n = fsr_input_tile[at + 2u * w];
    let o = fsr_input_tile[at + 2u * w + 1u];

    let edge = mix(mix(fsr_edge(b.w, e.w, f.w, g.w, j.w), fsr_edge(c.w, f.w, g.w, h.w, k.w), pp.x),
                   mix(fsr_edge(f.w, i.w, j.w, k.w, n.w), fsr_edge(g.w, j.w, k.w, l.w, o.w), pp.x), pp.y);
    let d2 = dot(edge.xy, edge.xy);
    var dir = vec2<f32>(1.0, 0.0);
    if (d2 >= 1.0 / 32768.0) { dir = edge.xy * inverseSqrt(d2); }
    let edge_len = (0.5 * edge.z) * (0.5 * edge.z);
    let stretch = dot(dir, dir) / max(abs(dir.x), abs(dir.y));
    let axis = vec2<f32>(1.0 + (stretch - 1.0) * edge_len, 1.0 - 0.5 * edge_len);
    let lobe = 0.5 - 0.29 * edge_len;
    let clip = 1.0 / lobe;

    // Center the sum at f so flat HDR colors survive without weight-normalization error.
    var acc = fsr_tap(vec2<f32>(0.0, -1.0) - pp, dir, axis, lobe, clip, b.rgb - f.rgb);
    acc += fsr_tap(vec2<f32>( 1.0, -1.0) - pp, dir, axis, lobe, clip, c.rgb - f.rgb);
    acc += fsr_tap(vec2<f32>(-1.0,  1.0) - pp, dir, axis, lobe, clip, i.rgb - f.rgb);
    acc += fsr_tap(vec2<f32>( 0.0,  1.0) - pp, dir, axis, lobe, clip, j.rgb - f.rgb);
    acc += fsr_tap(-pp,                       dir, axis, lobe, clip, vec3<f32>(0.0));
    acc += fsr_tap(vec2<f32>(-1.0,  0.0) - pp, dir, axis, lobe, clip, e.rgb - f.rgb);
    acc += fsr_tap(vec2<f32>( 1.0,  1.0) - pp, dir, axis, lobe, clip, k.rgb - f.rgb);
    acc += fsr_tap(vec2<f32>( 2.0,  1.0) - pp, dir, axis, lobe, clip, l.rgb - f.rgb);
    acc += fsr_tap(vec2<f32>( 2.0,  0.0) - pp, dir, axis, lobe, clip, h.rgb - f.rgb);
    acc += fsr_tap(vec2<f32>( 1.0,  0.0) - pp, dir, axis, lobe, clip, g.rgb - f.rgb);
    acc += fsr_tap(vec2<f32>( 1.0,  2.0) - pp, dir, axis, lobe, clip, o.rgb - f.rgb);
    acc += fsr_tap(vec2<f32>( 0.0,  2.0) - pp, dir, axis, lobe, clip, n.rgb - f.rgb);
    let lo = min(min(f.rgb, g.rgb), min(j.rgb, k.rgb));
    let hi = max(max(f.rgb, g.rgb), max(j.rgb, k.rgb));
    let rgb = clamp(f.rgb + acc.rgb / acc.a, lo, hi);
    textureStore(fsr_easu_out, vec2<i32>(id.xy), vec4<f32>(rgb, 1.0));
}

fn fsr_rcas_load(p: vec2<i32>, high: vec2<i32>) -> vec3<f32> {
    return textureLoad(fsr_easu_in, clamp(p, vec2<i32>(0), high), 0).rgb;
}

@compute @workgroup_size(8, 8)
fn fsr_rcas_to_post(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= L.size.xy)) { return; }
    let p = vec2<i32>(id.xy);
    let high = vec2<i32>(L.size.xy) - 1;
    let b = fsr_rcas_load(p + vec2<i32>( 0, -1), high);
    let d = fsr_rcas_load(p + vec2<i32>(-1,  0), high);
    let e = textureLoad(fsr_easu_in, p, 0).rgb;
    let f = fsr_rcas_load(p + vec2<i32>( 1,  0), high);
    let h = fsr_rcas_load(p + vec2<i32>( 0,  1), high);

    // FsrRcasF's per-channel clipping constraints. An empty ring channel cannot ring below black.
    let mn4 = min(min(b, d), min(f, h));
    let mx4 = max(max(b, d), max(f, h));
    let lo = min(mn4, e);
    let hi = max(mx4, e);
    let hit_min = select(vec3<f32>(0.25), lo / max(4.0 * mx4, vec3<f32>(1.0e-12)), mx4 > vec3<f32>(0.0));
    let hit_max = (hi - 1.0) / (4.0 * max(1.0 - mn4, vec3<f32>(FSR_HEADROOM)));
    let limit = clamp(fsr_max3(max(-hit_min, hit_max)), -(0.25 - 1.0 / 16.0), 0.0);
    // One stop of sharpening and the reference RCAS noise suppression for residual trace noise.
    let lb = fsr_luma(b); let ld = fsr_luma(d); let le = fsr_luma(e); let lf = fsr_luma(f); let lh = fsr_luma(h);
    let lr = max(max(max(lb, ld), max(lf, lh)), le) - min(min(min(lb, ld), min(lf, lh)), le);
    let nz = clamp(abs(0.25 * ((lb - le) + (ld - le) + (lf - le) + (lh - le))) / max(lr, 1.0e-12), 0.0, 1.0);
    let lobe = 0.5 * limit * (1.0 - 0.5 * nz);
    let delta = (b - e) + (d - e) + (f - e) + (h - e);
    var rgb = max(e + delta * (lobe / (1.0 + 4.0 * lobe)), vec3<f32>(0.0));

    // A small overshoot near the SRTM asymptote can create a huge linear highlight. Preserve
    // hue and retain at least 75% of the brightest neighbor's headroom there.
    let ceiling = max(0.975, 1.0 - 0.75 * (1.0 - fsr_max3(hi)));
    let peak = fsr_max3(rgb);
    if (peak > ceiling) { rgb *= ceiling / peak; }
    let color = fsr_unpack_hdr(rgb);
    let i = id.y * L.size.x + id.x;
    let img = vec4<f32>(color, dof_coc(sample_vel[sample_under(id.x, id.y)].z));
    if (L.misc2.z > 0.0 || L.dof.x > 0.0) { sb[i] = img; } else { fa[i] = img; }
}
