// The reconstruction contracts' signals (live/reconstruction.rs): the frame's packed into textures for a vendor's
// upscaler or joint denoiser, and its output taken back into the post chain. Appended to live.wgsl so camera
// conventions and post-process targets stay shared. Render attachments allow R8/R16/RG16 textures without optional
// WebGPU storage formats.
@vertex fn rc_vertex(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}
struct RcInputs {
    @location(0) color: vec4<f32>,
    @location(1) depth: f32,
    @location(2) motion: vec2<f32>,
    @location(3) reactive: f32,
};
// A fixed radiometric unit between exposure changes: the display whites a unit is, as the active provider takes it
// (`L.mode.y`: live/reconstruction.rs `white_units`, which `encode` passes as `radiance_unit`), over the exposure.
// The post chain receives scene radiance again.
fn rc_radiance_unit() -> f32 {
    return f32(max(L.mode.y, 1u)) / clamp(L.misc.x, 0.00001, 64.0);
}
fn pack_rc_inputs(p: vec4<f32>) -> RcInputs {
    let q = u32(p.y) * L.isz.x + u32(p.x);
    let v = sample_vel[q];
    let color = clamp(fb[q].rgb / rc_radiance_unit(), vec3<f32>(0.0), vec3<f32>(65504.0));
    // Infinite reverse-Z perspective, near 1 cm; zero is the sky (the frame's `view_to_clip`).
    let depth = select(0.0, 0.01 / max(v.z, 0.01), v.z > 0.0);
    let motion = -v.xy * vec2<f32>(L.isz.xy) / vec2<f32>(L.size.xy);
    return RcInputs(vec4<f32>(color, 1.0), depth, motion, v.w);
}
@fragment fn rc_upscale_inputs(@builtin(position) p: vec4<f32>) -> RcInputs {
    return pack_rc_inputs(p);
}
struct RcJointInputs {
    @location(0) color: vec4<f32>,
    @location(1) depth: f32,
    @location(2) motion: vec2<f32>,
    @location(3) reactive: f32,
    @location(4) overlay: vec4<f32>,
    @location(5) bypass_denoise: f32,
};
@fragment fn rc_joint_inputs(@builtin(position) p: vec4<f32>) -> RcJointInputs {
    let base = pack_rc_inputs(p);
    let q = u32(p.y) * L.isz.x + u32(p.x);
    let px = sample_px[q];
    // Directly visible sky and emitters are deterministic: the overlay, which the provider upscales with the same
    // jitter and visibility as the denoised frame. A mixed or moving medium stays in color.
    let direct = sample_emission(q);
    let overlay = medium_share(q) == 0.0 && (px.color.w == 0.0 || lum(direct) > max(0.001, 0.5 * lum(fb[q].rgb)));
    let layer = select(vec3<f32>(0.0), base.color.rgb, overlay);
    let lighting = base.color.rgb - layer;
    // The bypass mask is 1 where denoising should be skipped (MetalFX's denoise-strength texture).
    return RcJointInputs(vec4<f32>(lighting, 1.0), base.depth, base.motion, base.reactive, vec4<f32>(layer, f32(overlay)), f32(overlay));
}
struct RcGuides {
    @location(0) diffuse: vec4<f32>,
    @location(1) specular: vec4<f32>,
    @location(2) normal: vec4<f32>,
    @location(3) roughness: f32,
    @location(4) hit: f32,
};
@fragment fn rc_guides(@builtin(position) p: vec4<f32>) -> RcGuides {
    let q = u32(p.y) * L.isz.x + u32(p.x);
    let px = sample_px[q];
    let diffuse = refl_in[2u*q];
    let specular = refl_in[2u*q+1u];
    return RcGuides(vec4<f32>(diffuse.rgb, 1.0), vec4<f32>(specular.rgb, 1.0), vec4<f32>(oct_dec(bitcast<u32>(diffuse.w)), 1.0),
        clamp(specular.w, 0.0, 1.0), clamp(px.spec.w, 0.0, 65504.0));
}
@group(1) @binding(0) var rc_output: texture_2d<f32>;
@compute @workgroup_size(8,8) fn rc_output_to_post(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= L.size.xy)) { return; }
    let i = id.y * L.size.x + id.x;
    let c = finite_or_zero(max(textureLoad(rc_output, vec2<i32>(id.xy), 0).rgb, vec3<f32>(0.0))) * rc_radiance_unit();
    let nearest = clamp(vec2<i32>(round((vec2<f32>(id.xy)+0.5)*vec2<f32>(L.isz.xy)/vec2<f32>(L.size.xy)-L.jit.xy)),
                        vec2<i32>(0), vec2<i32>(L.isz.xy)-1);
    let img = vec4<f32>(c, dof_coc(sample_vel[u32(nearest.y)*L.isz.x+u32(nearest.x)].z));
    if (L.misc2.z > 0.0 || L.dof.x > 0.0) { sb[i] = img; } else { fa[i] = img; }
}
