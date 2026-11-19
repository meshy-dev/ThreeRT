// A generated texture holds a drop's normal (RG) and footprint (A). Sample the neon through that moving lens.
import { BoxGeometry, DataTexture, Mesh, MeshStandardMaterial, PerspectiveCamera, Scene, SkyColor } from "@threert/scene";
import { defineGame, hud } from "@threert/gameplay";

function droplet(): DataTexture {
  const size = 64, rgba = new Uint8Array(size * size * 4);
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) {
    const v = (y + 0.5) / size * 2 - 1;
    const u = ((x + 0.5) / size * 2 - 1) / (0.72 + 0.2 * v);
    const r = Math.hypot(u, v);
    const i = 4 * (y * size + x);
    rgba[i] = Math.round(127.5 * (1 + Math.max(-1, Math.min(1, u))));
    rgba[i + 1] = Math.round(127.5 * (1 + v));
    rgba[i + 3] = Math.round(255 * Math.max(0, Math.min(1, (1 - r) * 12)));
  }
  const t = new DataTexture(size, size, rgba, "linear");
  t.wrap = "clamp";
  return t;
}

const lens = {
  textures: [droplet()],
  params: [0.036, 0.85], // refraction in screen heights, fall speed in cells per second
  wgsl: `
fn hash(n: f32) -> f32 { return fract(sin(n * 127.1 + 17.0) * 4375.3); }
fn post(uv: vec2f, color: vec3f) -> vec3f {
    let aspect = vec2f(screen.size.x / screen.size.y, 1.0);
    let grid = uv * aspect * 6.0;
    let col = floor(grid.x);
    let seed = hash(col);
    let flow = grid.y + 8.0 * seed - screen.time * screen.params[0].y * (0.55 + seed);
    let cell = col * 23.0 + floor(flow);
    let center = vec2f(0.3 + 0.4 * hash(cell), 0.5);
    let radius = 0.13 + 0.13 * hash(cell + 1.0);
    let pos = (fract(vec2f(grid.x, flow)) - center) / (radius * vec2f(1.6, 2.8)) + 0.5;
    let drop = sample0(clamp(pos, vec2f(0), vec2f(1)));
    if any(pos < vec2f(0)) || any(pos > vec2f(1)) || drop.a == 0.0 { return color; }
    let normal = drop.rg * 2.0 - 1.0;
    let refracted = sceneColor(uv - normal * screen.params[0].x / aspect);
    let glint = 0.16 * pow(max(dot(normal, vec2f(-0.6, -0.8)), 0.0), 6.0);
    return mix(color, refracted + glint, drop.a);
}`,
};

export default defineGame({
  state: {}, keys: { R: "rain" },
  setup() {
    const scene = new Scene();
    scene.rain = 1;
    scene.postPass = lens;
    scene.environment = new SkyColor(0x577194, 1.5);
    const camera = new PerspectiveCamera(48);
    camera.position.set(0, 1.75, 4.4);
    camera.setManualExposure(2);
    scene.add(camera);
    const box = (x: number, y: number, z: number, w: number, h: number, d: number, color: number, nits = 0) => {
      const m = new Mesh(new BoxGeometry(w, h, d), new MeshStandardMaterial({ color, emissive: color, emissiveIntensity: nits, roughness: 0.32 }));
      m.position.set(x, y, z);
      scene.add(m);
    };
    box(0, -0.15, 0, 24, 0.3, 22, 0x343b46);
    box(0, 2.4, -2, 16, 4.8, 0.4, 0x344353);
    box(0, 2.05, -1.65, 5.4, 2.16, 0.24, 0x192035);
    // Seven-segment neon tubes spelling OPEN against the signboard.
    const segments = [[0, 0.60, 0.8, 0.09], [0.4, 0.31, 0.09, 0.5], [0.4, -0.31, 0.09, 0.5],
      [0, -0.60, 0.8, 0.09], [-0.4, -0.31, 0.09, 0.5], [-0.4, 0.31, 0.09, 0.5], [0, 0, 0.8, 0.09]];
    ["012345", "01564", "05643", "246"].forEach((glyph, i) => {
      for (const k of glyph) {
        const [x, y, width, height] = segments[Number(k)];
        box((i - 1.5) * 1.2 + x, 2.05 + y, -1.46, width, height, 0.08, i % 2 ? 0xff2081 : 0x13eaff, 28);
      }
    });
    for (const y of [1.17, 2.94]) box(0, y, -1.48, 5.1, 0.035, 0.08, 0xff4385, 13);
    return { scene, camera };
  },
  on: {
    rain(w) {
      w.scene.rain = 1 - w.scene.rain;
      w.scene.postPass = w.scene.rain ? lens : null;
    },
  },
  frame(f) {
    hud.set({ crosshair: { style: "cross" }, items: [
      { type: "bar", anchor: "bottom-left", label: "Health", text: "84", value: 0.84, tone: "good" },
      { type: "text", anchor: "top-left", text: f.scene.rain ? "R · stop rain" : "R · start rain", size: "small" },
    ] });
  },
});
