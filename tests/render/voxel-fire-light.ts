// The fire light stands in for the fireball's own light (ROADMAP § 6 step 10 b; `FIRE_LIGHT` in @threert/voxel): two
// fires of 1 m (fireballs 2.2 m across) either side of a wall, 0.48 s after they start (0.3 of their life), over a grey
// floor, no sky. Left: the fireball alone, its light path traced from its emission (no fire light). Right: the fire
// light alone (the fireball there made transparent: a density of 1e-6, so it emits nothing). The floor before each
// should read the same: right / left = 1 (the table was measured this way, in tools/bench-voxel-fire.ts --only
// calibrate, over a floor and a wall; within about 10 % of each other there).
import { BoxGeometry, Mesh, type PerspectiveCamera } from "@threert/scene";
import { Explosions } from "@threert/voxel";
import { camera, lambert, plane, result, sceneWithSky, type Region, type TestMeta } from "./lib.ts";

const EYE: [number, number, number] = [0, 9, 14], AT: [number, number, number] = [0, 0, 1], FOV = 50;

/** A floor point's place in the image (u, v from the top left). */
function project(p: [number, number, number]): [number, number] {
  const f = AT.map((t, i) => t - EYE[i]), fl = Math.hypot(...f), fw = f.map((v) => v / fl);
  const r = [-fw[2], 0, fw[0]], rl = Math.hypot(...r), rt = r.map((v) => v / rl);
  const up = [rt[1] * fw[2] - rt[2] * fw[1], rt[2] * fw[0] - rt[0] * fw[2], rt[0] * fw[1] - rt[1] * fw[0]];
  const d = p.map((v, i) => v - EYE[i]);
  const z = d[0] * fw[0] + d[1] * fw[1] + d[2] * fw[2], x = d[0] * rt[0] + d[1] * rt[1] + d[2] * rt[2], y = d[0] * up[0] + d[1] * up[1] + d[2] * up[2];
  const t = Math.tan((FOV * Math.PI) / 360);
  return [0.5 + x / (z * t * (16 / 9)) / 2, 0.5 - y / (z * t) / 2];
}
/** The floor from 2 to 5 m before a fire at x, 2 m either side of it. */
function patch(x: number): Region {
  const c = [project([x - 2, 0, 2]), project([x + 2, 0, 2]), project([x - 2, 0, 5]), project([x + 2, 0, 5])];
  return [Math.min(...c.map((p) => p[0])), Math.min(...c.map((p) => p[1])), Math.max(...c.map((p) => p[0])), Math.max(...c.map((p) => p[1]))];
}

export const test: TestMeta = {
  id: "voxel/fire-light", features: ["volume", "lights"], reference: "converged", temporal: false, spp: 4096, time: 0.48,
  metric: { kind: "ratio", a: patch(5), b: patch(-5), expected: 1, tolerance: 0.15 },
  note: "the floor before a fire light alone over the floor before a fireball alone (its own emission), 0.3 of its life",
};

export default function () {
  const scene = sceneWithSky(0);
  const grey = lambert(0.5, "grey");
  const floor = plane(60, grey, "floor");
  // From z -10 to 8: between the halves (a straight line from one fire to the other side's floor crosses it), and in
  // front of the camera, not around it.
  const wall = new Mesh(new BoxGeometry(0.2, 12, 18), grey);
  wall.name = "wall";
  wall.position.set(0, 6, -1);
  // Centres 1.32 m up (0.6 of the size, as in the calibration room): the explosion's point 0.3 r below.
  const fireball = new Explosions({ fireballs: 1, lights: 0 });
  fireball.fire([-5, 1.02, 0], 1, 0);
  const light = new Explosions({ fireballs: 1, lights: 1, density: 1e-6 });
  light.fire([5, 1.02, 0], 1, 0);
  // (No frame script: the light is set for the render's time here; the fireball follows the scene time itself.)
  light.step(test.time!);
  const cam: PerspectiveCamera = camera(scene, EYE, AT, FOV);
  return result(scene, cam, [floor, wall, fireball, light]);
}
