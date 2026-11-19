// Mac live passes. Appended to live.wgsl so camera conventions and post-process targets stay shared.
// Render attachments allow native R8/R16/RG16 textures without optional WebGPU storage formats.
@vertex fn fx_vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}
struct FxInputs {
    @location(0) color: vec4<f32>,
    @location(1) depth: f32,
    @location(2) motion: vec2<f32>,
    @location(3) reactive: f32,
};
// A fixed radiometric unit between exposure changes. The post chain receives scene radiance again.
fn fx_radiance_unit() -> f32 {
    // The denoiser expects lighting near display white. Its overlay preserves much brighter
    // emitters. The plain temporal scaler receives the whole lit frame in a wider linear range.
    return select(32.0, 1.0, L.mode.w == 1u) / clamp(L.misc.x, 0.00001, 64.0);
}
fn pack_fx_inputs(p: vec4<f32>) -> FxInputs {
    let q = u32(p.y) * L.isz.x + u32(p.x);
    let v = sample_vel[q];
    let color = clamp(fb[q].rgb / fx_radiance_unit(), vec3<f32>(0.0), vec3<f32>(65504.0));
    // Infinite reverse-Z perspective, near 1 cm; zero is the sky. fx_encode uses the same projection.
    let depth = select(0.0, 0.01 / max(v.z, 0.01), v.z > 0.0);
    let motion = -v.xy * vec2<f32>(L.isz.xy) / vec2<f32>(L.size.xy);
    return FxInputs(vec4<f32>(color, 1.0), depth, motion, v.w);
}
@fragment fn fx_inputs(@builtin(position) p: vec4<f32>) -> FxInputs {
    return pack_fx_inputs(p);
}
struct FxDenoisedInputs {
    @location(0) color: vec4<f32>,
    @location(1) depth: f32,
    @location(2) motion: vec2<f32>,
    @location(3) reactive: f32,
    @location(4) overlay: vec4<f32>,
    @location(5) bypass_denoise: f32,
};
@fragment fn fx_denoised_inputs(@builtin(position) p: vec4<f32>) -> FxDenoisedInputs {
    let base = pack_fx_inputs(p);
    let q = u32(p.y) * L.isz.x + u32(p.x);
    let px = sample_px[q];
    // Directly visible sky and emitters are deterministic. The native overlay upscales them with
    // the same jitter and visibility as the denoised frame. A mixed or moving medium stays in color.
    let direct = sample_emission(q);
    let overlay = medium_share(q) == 0.0 && (px.color.w == 0.0 || lum(direct) > max(0.001, 0.98 * lum(fb[q].rgb)));
    let layer = select(vec3<f32>(0.0), base.color.rgb, overlay);
    let lighting = base.color.rgb - layer;
    // The SDK's denoise-strength texture takes 1 where denoising should be bypassed.
    return FxDenoisedInputs(vec4<f32>(lighting, 1.0), base.depth, base.motion, base.reactive, vec4<f32>(layer, f32(overlay)), f32(overlay));
}
struct FxGuides {
    @location(0) diffuse: vec4<f32>,
    @location(1) specular: vec4<f32>,
    @location(2) normal: vec4<f32>,
    @location(3) roughness: f32,
    @location(4) hit: f32,
};
@fragment fn fx_guides(@builtin(position) p: vec4<f32>) -> FxGuides {
    let q = u32(p.y) * L.isz.x + u32(p.x);
    let px = sample_px[q];
    let diffuse = refl_in[2u*q];
    let specular = refl_in[2u*q+1u];
    return FxGuides(vec4<f32>(diffuse.rgb, 1.0), vec4<f32>(specular.rgb, 1.0), vec4<f32>(oct_dec(bitcast<u32>(diffuse.w)), 1.0),
        clamp(specular.w, 0.0, 1.0), clamp(px.spec.w, 0.0, 65504.0));
}
@group(1) @binding(0) var fx_output: texture_2d<f32>;
@compute @workgroup_size(8,8) fn fx_output_to_post(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= L.size.xy)) { return; }
    let i = id.y * L.size.x + id.x;
    let c = finite_or_zero(max(textureLoad(fx_output, vec2<i32>(id.xy), 0).rgb, vec3<f32>(0.0))) * fx_radiance_unit();
    let nearest = clamp(vec2<i32>(round((vec2<f32>(id.xy)+0.5)*vec2<f32>(L.isz.xy)/vec2<f32>(L.size.xy)-L.jit.xy)),
                        vec2<i32>(0), vec2<i32>(L.isz.xy)-1);
    let img = vec4<f32>(c, dof_coc(sample_vel[u32(nearest.y)*L.isz.x+u32(nearest.x)].z));
    if (L.misc2.z > 0.0 || L.dof.x > 0.0) { sb[i] = img; } else { fa[i] = img; }
}
