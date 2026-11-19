// A hit carries one bearing, clockwise from ahead. Its time drives both the lens and the camera.
import { BoxGeometry, DirectionalLight, Mesh, MeshStandardMaterial, PerspectiveCamera, Scene, SkyColor } from "@threert/scene";
import { defineGame, hud } from "@threert/gameplay";

const FADE = 0.7, SHAKE = 0.2;
const effect = {
  params: [0, -1, 0],
  wgsl: `
fn post(uv: vec2f, color: vec3f) -> vec3f {
    let hit = screen.params[0];
    let p = uv * 2.0 - 1.0;
    let edge = smoothstep(0.44, 1.0, max(abs(p.x), abs(p.y)));
    let facing = dot(p / max(length(p), 0.001), hit.xy);
    let weight = hit.z * edge * smoothstep(0.3, 0.94, facing);
    return mix(color, vec3f(0.92, 0.045, 0.055), weight);
}`,
};
let bearing = 0, hitAt = -Infinity, health = 100;

function damage(time: number, angle: number) {
  if (!Number.isFinite(angle) || health <= 0) return;
  hitAt = time;
  bearing = angle;
  health = Math.max(0, health - 12);
}

export default defineGame({
  state: {},
  keys: { A: "left", D: "right", W: "front", S: "back", R: "reset" },
  setup() {
    hitAt = -Infinity;
    health = 100;
    const scene = new Scene();
    scene.postPass = effect;
    scene.environment = new SkyColor(0xd5e7ff, 800);
    const sun = new DirectionalLight(0xfff3e0, 8000);
    sun.setSunAngles(35, 125);
    scene.add(sun);
    const block = (x: number, y: number, z: number, w: number, h: number, d: number, color: number) => {
      const m = new Mesh(new BoxGeometry(w, h, d), new MeshStandardMaterial({ color }));
      m.position.set(x, y, z);
      scene.add(m);
    };
    block(0, -0.2, -6, 18, 0.4, 26, 0x89939c);
    block(0, 2, -14, 18, 4, 0.6, 0x849ba7);
    for (const x of [-5, 5]) {
      block(x, 1.5, -7, 1.2, 3, 1.2, 0x44515d);
      block(x, 2.3, -6.35, 1.4, 0.55, 0.15, 0xe4a14e);
    }
    block(0, 1, -10, 1.8, 2, 1, 0x5d737c);
    const camera = new PerspectiveCamera(60);
    camera.setManualExposure(11);
    camera.position.set(0, 1.6, 1);
    scene.add(camera);
    return { scene, camera };
  },
  on: {
    damage(w, e) { damage(w.time, e.angle); },
    left(w) { damage(w.time, -Math.PI / 2); },
    right(w) { damage(w.time, Math.PI / 2); },
    front(w) { damage(w.time, 0); },
    back(w) { damage(w.time, Math.PI); },
    reset() { hitAt = -Infinity; health = 100; },
  },
  frame(f) {
    const age = f.time - hitAt;
    const x = Math.sin(bearing), y = -Math.cos(bearing);
    effect.params = [x, y, 0.9 * Math.max(0, 1 - age / FADE) ** 2];
    const kick = 0.033 * Math.max(0, 1 - age / SHAKE) ** 2;
    const wave = kick > 0 ? kick * Math.cos(age * 106) : 0;
    f.camera.rotation.set(wave * (0.35 - y), -wave * x, wave * x * 0.6, "YXZ");
    hud.set({ crosshair: { style: "cross" }, items: [
      { type: "bar", anchor: "bottom-left", label: "Health", value: health / 100, text: String(health), tone: health > 30 ? "good" : "bad" },
      { type: "text", anchor: "top-left", text: "W / A / S / D · take a hit from that side     R · reset", size: "small" },
    ] });
  },
});
