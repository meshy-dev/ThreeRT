// The C slice of docs/ROADMAP.md § 3, part 1 (ours; the UE side comes later): Kabukichō Ichiban-gai at night seen across
// a canal, one fixed 60 s shot with everything item 6 of § 5 brought. The Kabukichō game (games/kabukicho) builds the
// street, its signs, sky, fog and rain and runs its frame script; this game cuts a canal into Yasukuni-dori in front
// of the gate and adds the waterfront, potted plants in the wind, pedestrians and an explosion, and a timeline:
//   0 s   wet street, no rain falling; people walk, plants sway, the gate shines in the canal
//   10 s  a taxi turns into the street (Kabukichō's traffic)
//   15 s  the rain starts (Kabukichō's rain)
//   40 s  an explosion in the street: a fireball, sparks and debris; smoke rises after it
//   60 s  end of the shot (the timeline repeats)
// The timeline is a function of the frame script's clock: it hands the Kabukichō frame script the state its handlers
// would have set. Run live (lockstep for a capture), the shot plays the same every time.
// Assets: Kabukichō's (see its game), node tools/fetch-quaternius.ts, node tools/fetch-polyhaven.ts --res 1k --models
// potted_plant_02.
import { BoxGeometry, Debris, Fireball, GLTFModel, Group, Mesh, MeshStandardMaterial, type Object3D, SmokeColumn, SparkBurst, WaterBody, windSway } from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";
import kabukicho from "../kabukicho/kabukicho.game.ts";

type KbState = typeof kabukicho.state;

/** The canal across Yasukuni-dori (m): x span, water surface height, depth; the road stays on either side. */
const CANAL = { x0: -30, x1: -20, water: -1.2, depth: 1.5, length: 140 };
/** The shot: behind the far railing, looking across the canal at the gate. */
const CAMERA = { position: [-31.5, 2.0, -7] as const, target: [0, 4, 1] as const, fov: 60 };
const SHOT = 60;
/** The explosion: on Yasukuni-dori between the canal and the gate. */
const EXPLOSION = { at: 40, x: -9, z: -5 };
const MANNEQUIN = "assets/quaternius/ual/UAL1_Standard.glb";
const PLANT = "assets/polyhaven/models/potted_plant_02_1k.glb";
const WIND = { strength: 0.08, frequency: 0.6, direction: 0.4 };

/** Pedestrians: a straight walk between two points (m) and back, at a speed (m/s), the clip's time offset. */
const WALKERS = [
  { id: "ped_gate", a: [-2.2, -8], b: [-2.2, 8], speed: 1.3, t0: 0.2 },
  { id: "ped_street", a: [4, 1.8], b: [40, 1.8], speed: 1.4, t0: 0.7 },
  { id: "ped_street2", a: [36, -1.2], b: [6, -1.2], speed: 1.2, t0: 1.1 },
  { id: "ped_bank", a: [-18.5, 12], b: [-18.5, -12], speed: 1.35, t0: 0.4 },
] as const;

/** Kabukichō's state at scene time t of the shot. */
function timeline(base: KbState, t: number): KbState {
  const s = t % SHOT;
  return { ...base, rain: s >= 15 ? 1 : 0, wet: 1, traffic: s >= 10 };
}

/** Removes the road under the canal from the street (Kabukichō's ground nodes) and lays the road strips either side. */
function cutCanal(root: Object3D) {
  const inCanal = (x: number) => x > CANAL.x0 - 0.5 && x < CANAL.x1 + 0.5;
  let asphalt: Mesh | null = null;
  const remove: Object3D[] = [];
  root.traverse((o: Object3D) => {
    if (!(o instanceof Mesh)) return;
    const name = (o.material as MeshStandardMaterial).name ?? "";
    // Yasukuni-dori's asphalt (one quad, x -34 .. -4) and its paint (lane lines, the crosswalk) inside the canal.
    if (name === "MI_Ground_Asphalt" && Math.abs(o.position.x + 19) < 0.5) {
      asphalt = o;
      remove.push(o);
    } else if (name.startsWith("MI_RoadPaint") && inCanal(o.position.x) && o.position.x < -4) remove.push(o);
  });
  for (const o of remove) o.parent?.remove(o);
  if (!asphalt) throw new Error("slice: Yasukuni-dori's asphalt not found");
  const a = asphalt as Mesh;
  // The quad's scale is its size in m (x across the avenue first): two strips, -34 .. x0 and x1 .. -4.
  for (const [x0, x1] of [[-34, CANAL.x0], [CANAL.x1, -4]]) {
    const strip = new Mesh(a.geometry, a.material);
    strip.position.set((x0 + x1) / 2, a.position.y, a.position.z);
    strip.quaternion.copy(a.quaternion);
    strip.scale.set(x1 - x0, a.scale.y, a.scale.z);
    a.parent ? a.parent.add(strip) : root.add(strip);
  }
}

function waterfront(): Group {
  const g = new Group();
  g.id = "waterfront";
  const concrete = new MeshStandardMaterial({ name: "embankment", color: 0x6f6b66, roughness: 0.85 });
  const bed = new MeshStandardMaterial({ name: "canal_bed", color: 0x3b3a33, roughness: 0.95 });
  const rail = new MeshStandardMaterial({ name: "rail", color: 0x2a2c30, roughness: 0.35, metalness: 1 });
  const box = (m: MeshStandardMaterial, size: [number, number, number], at: [number, number, number]) => {
    const b = new Mesh(new BoxGeometry(...size), m);
    b.position.set(...at);
    g.add(b);
  };
  const bottom = CANAL.water - CANAL.depth;
  const water = new WaterBody({ width: CANAL.x1 - CANAL.x0, depth: CANAL.depth, length: CANAL.length, extinction: 1.2, albedo: [0.1, 0.16, 0.15] });
  water.position.set((CANAL.x0 + CANAL.x1) / 2, CANAL.water, 0);
  g.add(water);
  box(bed, [CANAL.x1 - CANAL.x0, 0.2, CANAL.length], [(CANAL.x0 + CANAL.x1) / 2, bottom - 0.1, 0]);
  for (const x of [CANAL.x0 - 0.2, CANAL.x1 + 0.2]) {
    box(concrete, [0.4, -bottom, CANAL.length], [x, bottom / 2, 0]);
    // Railing: posts every 2 m, a top rail.
    box(rail, [0.05, 0.05, CANAL.length], [x, 1.05, 0]);
    for (let z = -CANAL.length / 2; z <= CANAL.length / 2; z += 2) box(rail, [0.04, 1.05, 0.04], [x, 0.525, z]);
  }
  return g;
}

/** A potted plant: the pot fixed, the plant swaying about the soil (0.305 m up in the file). */
function plant(x: number, z: number, scale: number, stiffness: number): Group {
  const g = new Group();
  g.position.set(x, 0, z);
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

let fx: { fireball: Fireball; sparks: SparkBurst; debris: Debris; smoke: SmokeColumn } | null = null;

export default defineGame<KbState>({
  seed: kabukicho.seed,
  state: kabukicho.state,
  keys: {},
  setup(ctx) {
    const r = kabukicho.setup(ctx);
    const scene = r.scene;
    cutCanal(scene);
    scene.add(waterfront());
    for (const [x, z, s, k] of [[-2.1, -3.6, 1.7, 0], [-2.1, 3.6, 1.7, 0.6], [-19.0, -2.0, 1.5, 0.3], [-19.0, 6.0, 1.5, 0.9]] as const) scene.add(plant(x, z, s, k));
    for (const w of WALKERS) {
      const m = new GLTFModel(MANNEQUIN, { animation: { clip: "Walk_Loop", time: w.t0 } });
      m.id = w.id;
      m.movable = true;
      scene.add(m);
    }
    // Fireball and sparks fire at customData[0] and again every SHOT seconds (the timeline repeats).
    const fireball = new Fireball({ intensity: 150, period: SHOT, life: 2.2 });
    fireball.position.set(EXPLOSION.x, 2, EXPLOSION.z);
    fireball.scale.setScalar(5);
    fireball.customData = [EXPLOSION.at, 0, 0, 0];
    const sparks = new SparkBurst({ count: 900, speed: 13, intensity: 1500, life: 1.8, period: SHOT });
    sparks.position.set(EXPLOSION.x, 0.8, EXPLOSION.z);
    sparks.customData = [EXPLOSION.at, 0, 0, 0];
    const debris = new Debris({ count: 18, seed: 7, speed: 9 });
    debris.position.set(EXPLOSION.x, 0, EXPLOSION.z);
    debris.fire(EXPLOSION.at);
    // The smoke rises from the explosion: a movable medium grown by the frame script (hiding it would rebuild).
    const smoke = new SmokeColumn({ density: 3 });
    smoke.movable = true;
    smoke.position.set(EXPLOSION.x, 0, EXPLOSION.z);
    smoke.scale.set(0.01, 0.01, 0.01);
    scene.add(fireball, sparks, debris, smoke);
    fx = { fireball, sparks, debris, smoke };
    const cam = r.camera;
    cam.position.set(...CAMERA.position);
    cam.lookAt(...CAMERA.target);
    cam.fov = CAMERA.fov;
    return { ...r, options: { ...r.options, name: "slice" } };
  },
  frame(f: FrameContext<KbState>) {
    const t = f.time % SHOT;
    kabukicho.frame!({ ...f, state: timeline(f.state, f.time) });
    // The shot's camera, whatever the street's markers say.
    f.camera.position.set(...CAMERA.position);
    f.camera.lookAt(...CAMERA.target);
    for (const w of WALKERS) {
      const m = f.object(w.id) as GLTFModel;
      const len = Math.hypot(w.b[0] - w.a[0], w.b[1] - w.a[1]);
      const d = (t * w.speed) % (2 * len);
      const back = d > len;
      const u = back ? 2 - d / len : d / len;
      const [dx, dz] = [(w.b[0] - w.a[0]) / len, (w.b[1] - w.a[1]) / len];
      m.position.set(w.a[0] + (w.b[0] - w.a[0]) * u, 0, w.a[1] + (w.b[1] - w.a[1]) * u);
      // The clip walks along the model's +z: yaw toward the way it goes.
      m.rotation.y = Math.atan2(back ? -dx : dx, back ? -dz : dz);
      m.animation = { clip: "Walk_Loop", time: w.t0 + t };
    }
    if (fx) {
      fx.debris.update(t);
      // Smoke from 0.5 s after the blast, full height (7 m) 6 s later.
      const k = Math.min(Math.max((t - EXPLOSION.at - 0.5) / 6, 0), 1);
      const h = 7 * (k * k * (3 - 2 * k)) + 0.01;
      fx.smoke.scale.set(Math.max(3 * Math.min(k * 3, 1), 0.01), h, Math.max(3 * Math.min(k * 3, 1), 0.01));
      fx.smoke.position.set(EXPLOSION.x, h / 2, EXPLOSION.z);
    }
  },
  handlers: {},
});
