// Display-referred RGB, with pixel (0, 0) at the top left of the final output.
struct Screen {
    size: vec2f,
    time: f32,
    pad: f32,
    params: array<vec4f, 8>,
    textureUV: array<vec4f, 4>,
}
@group(0) @binding(0) var<uniform> screen: Screen;
@group(0) @binding(1) var<storage, read> scenePixels: array<u32>;
@group(0) @binding(2) var<storage, read_write> outputPixels: array<u32>;

fn scenePixel(p: vec2u) -> vec3f {
    return unpack4x8unorm(scenePixels[p.y * u32(screen.size.x) + p.x]).bgr;
}

fn sceneColor(uv: vec2f) -> vec3f {
    let last = screen.size - 1.0;
    let p = clamp(uv * screen.size - 0.5, vec2f(0.0), last);
    let a = vec2u(p);
    let b = min(a + 1u, vec2u(last));
    let f = fract(p);
    return mix(mix(scenePixel(a), scenePixel(vec2u(b.x, a.y)), f.x),
               mix(scenePixel(vec2u(a.x, b.y)), scenePixel(b), f.x), f.y);
}

@compute @workgroup_size(8, 8)
fn screenMain(@builtin(global_invocation_id) id: vec3u) {
    if any(id.xy >= vec2u(screen.size)) { return; }
    let uv = (vec2f(id.xy) + 0.5) / screen.size;
    let rgb = post(uv, scenePixel(id.xy));
    outputPixels[id.y * u32(screen.size.x) + id.x] = pack4x8unorm(vec4f(rgb.bgr, 1.0));
}
