// The C slice of docs/ROADMAP.md § 3 on Bistro night (§ 0 step 9; the Kabukichō version, games/slice, stays as
// history): one fixed 60 s shot down Bistro's street with what item 6 of § 5 brought.
//   0 s   wet street, no rain falling; people walk, potted plants sway, Bistro's lanterns and string lights swing (its
//         FBX take, played through the rigid-node path)
//   15 s  the rain starts
//   25 s  a van drives down the street toward the camera and past it, headlights on (34 s gone)
//   40 s  an explosion in the street: a fireball, sparks and debris; smoke rises after it
//   60 s  end of the shot (the timeline repeats)
// The timeline is a function of the frame script's clock: run live (lockstep for a capture), the shot plays the same
// every time.
// Assets: bun scripts/orca.ts (Bistro), node tools/fetch-quaternius.ts, node tools/fetch-polyhaven.ts --res 1k --models
// potted_plant_02.
import { BoxGeometry, CylinderGeometry, Debris, Fireball, GLTFModel, Group, Mesh, MeshStandardMaterial, RainCurtain, SmokeColumn, SparkBurst, windSway } from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";
import build, { STREET } from "../../scenes/bistro.ts";

const SHOT = 60;
/** Bistro's FBX take as one glTF clip (scripts/orca.ts): the camera path and 15 swinging lanterns / string lights. */
const TAKE = { clip: "BistroExterior", length: 154.458 };
/** The street's surface (m): 0.25-0.43 along the view, ray-cast on the converted glTF. */
const GROUND = 0.33;
/** The walkers: Quaternius' mannequin at 3.5 k triangles of its 13.8 k (the LOD node tools/fetch-quaternius.ts makes
 *  with Blender): a skinned character is re-skinned and rebuilt into the dynamic BLAS every frame. */
const MANNEQUIN = "assets/quaternius/ual/UAL1_Standard_lod1.glb";
const PLANT = "assets/polyhaven/models/potted_plant_02_1k.glb";
const WIND = { strength: 0.08, frequency: 0.6, direction: 0.4 };

// Placement along the camera's view: d metres ahead, s metres to the left (both on the ground plane).
const [fx, fz] = (() => {
  const x = STREET.target[0] - STREET.position[0], z = STREET.target[2] - STREET.position[2], n = Math.hypot(x, z);
  return [x / n, z / n];
})();
const at = (d: number, s: number): [number, number] => [STREET.position[0] + fx * d + fz * s, STREET.position[2] + fz * d - fx * s];

/** Pedestrians: a straight walk between two points (d, s) and back, at a speed (m/s), the clip's time offset. */
const WALKERS = [
  { id: "ped_street", a: at(10, 0.5), b: at(34, 1.5), speed: 1.3, t0: 0.3 },
  { id: "ped_left", a: at(5, 3.6), b: at(24, 3.6), speed: 1.4, t0: 0.9 },
  { id: "ped_cross", a: at(17, -3.2), b: at(19, 3.4), speed: 1.1, t0: 0.1 },
  { id: "ped_far", a: at(38, -4.0), b: at(28, -4.6), speed: 1.2, t0: 1.4 },
] as const;
const EXPLOSION = { at: 40, p: at(22, -1) };
/** The van: from `from` metres down the view to `to` (past the camera) on a lane `lane` metres right of the axis, at
 *  `speed` m/s, starting at `at` s; parked under the street otherwise. Its 1.8 m body spans 0.6–2.4 m right of the axis:
 *  it passes the crossing pedestrian (17–19 m ahead) when that one is on the far side of the street (30 s), and the far
 *  pedestrian walks 4–4.6 m right, clear of it (the street walker keeps to the left). */
const VAN = { at: 25.5, from: 45, to: -6, lane: -1.5, speed: 6, wheel: 0.34, parked: [0, -60, 0] as const };
const RAIN_AT = 15;

/** A potted plant: the pot fixed, the plant swaying about the soil (0.305 m up in the file). */
function plant(p: [number, number], scale: number, stiffness: number): Group {
  const g = new Group();
  g.position.set(p[0], GROUND, p[1]);
  g.scale.setScalar(scale);
  g.add(new GLTFModel(PLANT, { nodes: ["potted_plant_02_pot", "potted_plant_02_dirt"] }));
  const soil = new Group();
  soil.position.y = 0.305;
  const leaves = new GLTFModel(PLANT, { nodes: ["potted_plant_02_leaves"] });
  leaves.position.y = -0.305;
  soil.add(leaves);
  g.add(windSway(soil, { ...WIND, stiffness }));
  return g;
}

/** A plain delivery van (body, cabin, wheels, head and tail lights) along +x, y up; `movable`: driven every frame without
 *  rebuilding the scene. */
function van(): Group {
  const g = new Group();
  g.id = "van";
  g.movable = true;
  g.position.set(...VAN.parked);
  const mat = (name: string, p: ConstructorParameters<typeof MeshStandardMaterial>[0]) => new MeshStandardMaterial({ name, ...p });
  const part = (id: string, geometry: BoxGeometry | CylinderGeometry, material: MeshStandardMaterial, x: number, y: number, z: number) => {
    const m = new Mesh(geometry, material);
    m.id = id;
    m.position.set(x, y, z);
    g.add(m);
    return m;
  };
  part("van_body", new BoxGeometry(4.6, 1.5, 1.8), mat("van_paint", { color: [0.32, 0.03, 0.03], roughness: 0.3, metalness: 0.3 }), -0.2, 1.15, 0);
  part("van_cabin", new BoxGeometry(1.1, 0.75, 1.7), mat("van_glass", { color: [0.02, 0.02, 0.025], roughness: 0.05 }), 1.95, 1.45, 0);
  const light = (name: string, color: [number, number, number], nits: number) => mat(name, { color: [0, 0, 0], emissive: color, emissiveIntensity: nits });
  for (const [k, z] of [[0, 0.7], [1, -0.7]] as const) {
    part(`van_head${k}`, new BoxGeometry(0.04, 0.14, 0.26), light("van_headlight", [1, 0.92, 0.78], 2500), 2.12, 0.78, z);
    part(`van_tail${k}`, new BoxGeometry(0.04, 0.2, 0.14), light("van_taillight", [1, 0.05, 0.03], 300), -2.52, 0.95, z);
  }
  const tyre = mat("van_tyre", { color: [0.03, 0.03, 0.03], roughness: 0.8 });
  for (const [k, x, z] of [[0, 1.5, 0.82], [1, 1.5, -0.82], [2, -1.6, 0.82], [3, -1.6, -0.82]] as const) {
    part(`van_wheel${k}`, new CylinderGeometry(VAN.wheel, VAN.wheel, 0.24, 20), tyre, x, VAN.wheel, z).rotation.x = Math.PI / 2;
  }
  return g;
}

let fx_: { debris: Debris; smoke: SmokeColumn } | null = null;
let rain = 0;

export default defineGame<Record<string, never>>({
  state: {},
  keys: {},
  setup() {
    const r = build();
    const scene = r.scene;
    // Bistro's own motion: its take on the frame clock.
    const bistro = scene.getObjectByName("bistro") as GLTFModel;
    bistro.id = "bistro";
    bistro.animation = { clip: TAKE.clip, time: 0 };
    scene.wetness = 1;
    scene.rain = 0;
    scene.add(new RainCurtain());
    for (const [d, s, k, stiff] of [[7, 4.6, 1.6, 0], [12.5, 4.6, 1.5, 0.6], [15, -4.4, 1.6, 0.3]] as const) scene.add(plant(at(d, s), k, stiff));
    for (const w of WALKERS) {
      const m = new GLTFModel(MANNEQUIN, { animation: { clip: "Walk_Loop", time: w.t0 } });
      m.id = w.id;
      m.movable = true;
      scene.add(m);
    }
    // Fireball and sparks fire at customData[0] and again every SHOT seconds (the timeline repeats).
    const [ex, ez] = EXPLOSION.p;
    const fireball = new Fireball({ intensity: 150, period: SHOT, life: 2.2 });
    fireball.position.set(ex, GROUND + 1.7, ez);
    fireball.scale.setScalar(5);
    fireball.customData = [EXPLOSION.at, 0, 0, 0];
    const sparks = new SparkBurst({ count: 900, speed: 13, intensity: 1500, life: 1.8, period: SHOT });
    sparks.position.set(ex, GROUND + 0.5, ez);
    sparks.customData = [EXPLOSION.at, 0, 0, 0];
    const debris = new Debris({ count: 18, seed: 7, speed: 9 });
    debris.position.set(ex, GROUND, ez);
    debris.fire(EXPLOSION.at);
    // The smoke rises from the explosion: a movable medium grown by the frame script (hiding it would rebuild).
    const smoke = new SmokeColumn({ density: 3 });
    smoke.movable = true;
    smoke.position.set(ex, GROUND, ez);
    smoke.scale.set(0.01, 0.01, 0.01);
    scene.add(fireball, sparks, debris, smoke, van());
    fx_ = { debris, smoke };
    rain = 0;
    return { ...r, options: { ...r.options, name: "bistro-slice" } };
  },
  frame(f: FrameContext<Record<string, never>>) {
    const t = f.time % SHOT;
    (f.object("bistro") as GLTFModel).animation = { clip: TAKE.clip, time: f.time % TAKE.length };
    // The rain comes in over about two seconds.
    const target = t >= RAIN_AT ? 1 : 0;
    rain = f.snap ? target : rain + (target - rain) * Math.min(1, f.dt / 0.6);
    f.scene.rain = rain;
    for (const w of WALKERS) {
      const m = f.object(w.id) as GLTFModel;
      const len = Math.hypot(w.b[0] - w.a[0], w.b[1] - w.a[1]);
      const d = (t * w.speed) % (2 * len);
      const back = d > len;
      const u = back ? 2 - d / len : d / len;
      const [dx, dz] = [(w.b[0] - w.a[0]) / len, (w.b[1] - w.a[1]) / len];
      m.position.set(w.a[0] + (w.b[0] - w.a[0]) * u, GROUND, w.a[1] + (w.b[1] - w.a[1]) * u);
      // The clip walks along the model's +z: yaw toward the way it goes.
      m.rotation.y = Math.atan2(back ? -dx : dx, back ? -dz : dz);
      m.animation = { clip: "Walk_Loop", time: w.t0 + t };
    }
    // The van: down the street toward the camera (its +x along the way it goes), wheels turning.
    const v = f.object("van");
    if (v) {
      const d = VAN.from - (t - VAN.at) * VAN.speed;
      if (t < VAN.at || d < VAN.to) v.position.set(...VAN.parked);
      else {
        const [x, z] = at(d, VAN.lane);
        v.position.set(x, GROUND, z);
        v.rotation.y = Math.atan2(fz, -fx);
        const spin = -((t - VAN.at) * VAN.speed) / VAN.wheel;
        for (let k = 0; k < 4; k++) f.object(`van_wheel${k}`)!.rotation.set(Math.PI / 2, spin, 0);
      }
    }
    if (fx_) {
      fx_.debris.update(t);
      // Smoke from 0.5 s after the blast, full height (7 m) 6 s later.
      const k = Math.min(Math.max((t - EXPLOSION.at - 0.5) / 6, 0), 1);
      const h = 7 * (k * k * (3 - 2 * k)) + 0.01;
      const wdt = Math.max(3 * Math.min(k * 3, 1), 0.01);
      fx_.smoke.scale.set(wdt, h, wdt);
      fx_.smoke.position.set(EXPLOSION.p[0], GROUND + h / 2, EXPLOSION.p[1]);
    }
  },
  handlers: {},
});

