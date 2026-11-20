// A dark hall hung with 400 small lanterns of warm and coloured light over a polished floor, a
// white figure and a few columns among them. Every lantern lights and shadows (the light tree picks among them per
// pixel); ray tracing off, each lights every surface it faces and nothing casts a shadow.
import { BoxGeometry, Color, CylinderGeometry, Mesh, MeshStandardMaterial, PerspectiveCamera, Scene, SkyColor, SphereGeometry, TorusKnotGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";

const EV = 5.0, N = 400;
function rng(seed: number) { return () => { seed |= 0; seed = (seed + 0x6d2b79f5) | 0; let t = Math.imul(seed ^ (seed >>> 15), 1 | seed); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; }; }
export default defineGame({
  state: {},
  handlers: {},
  setup() {
    const scene = new Scene();
    scene.environment = new SkyColor(0x000000, 0);
    const mat = (name: string, c: number | Color, roughness: number, metalness = 0, extra = {}) => new MeshStandardMaterial({ name, color: c, roughness, metalness, ...extra });
    const box = (id: string, s: [number, number, number], p: [number, number, number], m: MeshStandardMaterial) => { const b = new Mesh(new BoxGeometry(...s), m); b.name = id; b.position.set(...p); scene.add(b); return b; };
    const W = 14, H = 5, D = 14, T = 0.3;
    const wall = mat("wall", 0x4a4540, 0.9);
    box("floor", [W + 2 * T, T, D + 2 * T], [0, -T / 2, 0], mat("floor", 0x1a1a1c, 0.1));
    box("ceiling", [W + 2 * T, T, D + 2 * T], [0, H + T / 2, 0], wall);
    for (const [id, s, p] of [["back", [W, H, T], [0, H / 2, -D / 2]], ["front", [W, H, T], [0, H / 2, D / 2]], ["left", [T, H, D], [-W / 2, H / 2, 0]], ["right", [T, H, D], [W / 2, H / 2, 0]]] as const) box(id, s as never, p as never, wall);
    const r = rng(3);
    const warm = [new Color().setRGB(1, 0.62, 0.28), new Color().setRGB(1, 0.75, 0.45), new Color().setRGB(1, 0.5, 0.2)];
    const tint = [new Color().setRGB(0.3, 0.6, 1), new Color().setRGB(1, 0.25, 0.5), new Color().setRGB(0.4, 1, 0.6)];
    const mats = [...warm, ...tint].map((c, i) => mat(`lantern${i}`, 0x000000, 1, 0, { emissive: c, emissiveIntensity: 2600 }));
    const wire = mat("wire", 0x0d0d0d, 0.6);
    const geo = new SphereGeometry(0.06, 16, 8);
    for (let i = 0; i < N; i++) {
      let x = (r() * 2 - 1) * 6.2, z = (r() * 2 - 1) * 6.2 - 0.5;
      if (Math.hypot(x, z + 0.5) < 1.1) x += 1.6;
      const y = 1.4 + r() * 2.6;
      const m = new Mesh(geo, mats[r() < 0.82 ? Math.floor(r() * 3) : 3 + Math.floor(r() * 3)]);
      m.name = `l${i}`; m.position.set(x, y, z); scene.add(m);
      const w = new Mesh(new CylinderGeometry(0.004, 0.004, H - y, 4), wire); w.name = `w${i}`; w.position.set(x, (H + y) / 2, z); scene.add(w);
    }
    const stone = mat("stone", 0xe8e3da, 0.5);
    const fig = new Mesh(new TorusKnotGeometry(0.45, 0.14, 256, 32, 3, 4), stone); fig.name = "figure"; fig.position.set(0, 1.4, -0.5); scene.add(fig);
    box("base", [0.9, 0.7, 0.9], [0, 0.35, -0.5], stone);
    for (const [i, x, z] of [[0, -2.5, -2.5], [1, 2.6, -2.0], [2, -0.6, 3.6], [3, 2.4, 2.6]] as const) { const c = new Mesh(new CylinderGeometry(0.25, 0.25, H, 48), mat("column", 0xcfc8bd, 0.6)); c.name = `col${i}`; c.position.set(x, H / 2, z); scene.add(c); }
    const camera = new PerspectiveCamera(50, 16 / 9, 0.05, 100);
    camera.name = "camera"; camera.position.set(-4.5, 1.3, 5.5); camera.lookAt(0, 1.4, -0.5); camera.setManualExposure(EV); scene.add(camera);
    return { scene, camera, options: { name: "lanterns", preset: "preview" as const, width: 1920, height: 1080 } };
  },
  frame(f) { f.object("figure")!.rotation.set(0.2, f.time * 0.3, 0); },
});
