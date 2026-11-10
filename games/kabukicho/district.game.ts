// Kabukichō district: the LOD / streaming acceptance scene (docs/ROADMAP.md § 5, item 5). Nine Kabukichō streets on a
// 3 x 3 grid, each built from its own seed (games/kabukicho/street.ts: different buildings, shops, signs, props and
// lights; no two blocks alike), under the game's night sky, fog and rain. The middle row runs end to end as one 520 m
// street for the drive (tools/bench-district.ts). Each block keeps its street core; the single street's backdrop
// skyline (500 m out) is dropped, the other blocks are the skyline now.
import { Color, ExponentialHeightFog, Group, PerspectiveCamera, Scene, SkyGradient, Texture, type Object3D } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { buildStreet } from "./street.ts";

/** Block spacing (m): along the street (a street core is ~165 m) and between rows (buildings reach ±35 m). */
export const PITCH = { along: 175, across: 90 };
/** The street core of one block, in its frame: x along the street, z across. */
const CORE = { x0: -25, x1: 150, z: 40 };

function block(seed: number, i: number, j: number) {
  const s = buildStreet(seed);
  for (const o of [...s.root.children]) {
    const p = o.position;
    if (p.x < CORE.x0 || p.x > CORE.x1 || Math.abs(p.z) > CORE.z) s.root.remove(o);
  }
  // Object ids are unique per street only: prefix them per block.
  const prefix = `b${i}${j}_`;
  s.root.traverse((o: Object3D) => {
    if (o.stableId) o.id = prefix + o.stableId;
  });
  const g = new Group();
  g.id = `block_${i}${j}`;
  g.position.set(i * PITCH.along, 0, (j - 1) * PITCH.across);
  g.add(s.root);
  return { group: g, sun: s.sun, objects: s.stats.actors };
}

/** The district: `rows` rows (centred on the middle one) of `cols` blocks; seeds seed, seed + 1, ... */
export function build(seed: number, rows = 3, cols = 3) {
  const scene = new Scene();
  let first: ReturnType<typeof block> | null = null;
  for (let j = 1 - Math.floor(rows / 2); j < 1 - Math.floor(rows / 2) + rows; j++) {
    for (let i = 0; i < cols; i++) {
      const b = block(seed + 3 * j + i, i, j);
      // One moon for the district: the other blocks' directional lights go.
      if (first) b.sun.parent?.remove(b.sun);
      first ??= b;
      scene.add(b.group);
    }
  }
  // Night sky, fog and rain as games/kabukicho/kabukicho.game.ts (UE M_NightSky, NightFog, "Rainy night").
  const sky = new SkyGradient({ zenith: new Color().setRGB(0.0025, 0.0035, 0.009), horizon: new Color().setRGB(0.075, 0.032, 0.045), intensity: 2.5 });
  sky.horizonExponent = 5;
  sky.ground = new Color().setRGB(0.075, 0.032, 0.045);
  sky.setGlow(new Color().setRGB(0.05, 0.022, 0.012), 1, 0, { width: 3, power: 6 });
  sky.clouds = { coverage: 0, color: new Color().setRGB(0.012, 0.006, 0.01), scale: 1, seed: 0, map: new Texture("assets/kabukicho/masks/T_Cloud.png", "linear"), threshold: [0.35, 0.85], gain: 1.8, projection: 0.35, bias: 0.25, fade: 4 };
  scene.environment = sky;
  scene.fog = ExponentialHeightFog.fromUnreal({
    fogDensity: 0.035, fogHeightFalloff: 0.06, height: -100, inscatteringLuminance: [0.012, 0.008, 0.02],
    volumetric: { scatteringDistribution: 0.55, albedo: [240, 230, 235], extinctionScale: 1.6, distance: 9000 },
  });
  scene.wetness = 1;
  scene.rain = 1;
  // At the start of the middle street, eye height, looking down it (+x).
  const camera = new PerspectiveCamera(55, 16 / 9, 0.1, 2000);
  camera.id = "camera";
  camera.position.set(-10, 1.7, 0);
  camera.lookAt(30, 1.7, 0);
  camera.setAutoExposure(0.6, -10, Math.log2(3));
  scene.add(camera);
  return { scene, camera };
}

/** The game setup for `rows` x `cols` blocks (fixed seeds from 4, whatever the room's: the acceptance numbers are for
 * this district; the middle row is seeds 7, 8, 9 in every variant). */
export function setupDistrict(rows = 3, cols = 3, name = "kabukicho-district") {
  const { scene, camera } = build(4, rows, cols);
  return {
    scene, camera,
    options: {
      name, preset: "cinematic" as const, width: 1280, height: 720,
      // Kabukichō's post (games/kabukicho/kabukicho.game.ts POST) without motion blur, for the measurements.
      overrides: { post: { bloom: { intensity: 0.85, threshold: 1.2, sizeScale: 5 }, vignette: 0.5, autoExposure: { lowPercent: 70, highPercent: 97, histogramLogMin: -8, histogramLogMax: 4, speedUp: 2, speedDown: 1.2 }, localExposure: { highlightContrast: 0.75, shadowContrast: 0.9 } } },
    },
  };
}

export default defineGame<Record<string, never>>({
  state: {},
  setup: () => setupDistrict(),
  handlers: {},
});
