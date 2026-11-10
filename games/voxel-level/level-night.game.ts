// The benchmark level at night (ROADMAP § 6 step 4; tools/bench-voxel-light.ts): the level's voxels loaded by the
// renderer's native voxel objects as meshes (renderer/src/voxel.rs), the sun below the horizon, the night sky's glow
// and the lamps' emissive voxels lighting the streets as emitters. What an explosion cuts loose falls (ROADMAP § 6 step
// 6); a glowing part lights what it passes.
//
// `nightGame` options:
//   shot   the camera: one of three poses on the level's camera path (0: down the first road, past a lamp; 1: the
//          corner of the fourth road; 2: along the fourth road)
//   lamps  the lamps glow (default) or not (the radiance a lamp adds, against `lamps: false`)
//   blast  "lamp": a 1 m explosion at the head of the lamp ahead of shot 0's camera (`blastLamp`); "pole": a 0.15 m one
//          at its pole, 1 m up, which changes the lamp's object (so its emitters move out of the light tree when it
//          is done in place) without touching its head
//   when   "setup": the explosion is done in setup (`advance` seconds of the level's ticks with it: what it cut loose
//          has fallen that long; the same world for the live view and the offline render); a number: the frame it is
//          done at, live, in place
//   node tools/voxel-level/generate.ts --granularity object && node tools/bundle-game.ts games/voxel-level/level-night.game.ts
import { DirectionalLight, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, Scene, SkyAtmosphere } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { voxelExplode, voxelFrame, voxelLevel } from "@threert/voxel";
import { nativeObjects, nativePalette } from "./native.ts";
import { EDGE, PALETTE, SIZE, VOXEL, cameraPath, levelPlan, toScene } from "./plan.ts";

export interface Night {
  shot?: number;
  lamps?: boolean;
  blast?: "lamp" | "pole";
  when?: "setup" | number;
  advance?: number;
  seed?: number;
}

/** The camera path's keys the three shots take. */
export const SHOTS = [0, 2, 3];
/** Manual exposure (EV100) of the night shots. */
export const NIGHT_EV = 4;

/** The lamp ahead of shot 0's camera, on its right (the nearest lamp turned along +x, 8–30 m down the road): its
 *  ground point and the centre of its head (plan.ts, generate.ts `propGrid`: a 0.5 x 0.3 x 0.4 m head 4.2 m up, 0.5 m
 *  out along +x). */
export function blastLamp(seed = 1): { ground: [number, number, number]; head: [number, number, number] } {
  const [cx, , cz] = cameraPath(seed)[0].pos;
  const lamps = levelPlan(seed).props.filter((p) => p.kind === "lamp" && p.turn === 0).map((p) => ({ p, at: toScene(p.x, p.y, p.z) }));
  const ahead = lamps.filter(({ at }) => at[2] - cz > 8 && at[2] - cz < 30 && Math.abs(at[0] - cx) < 8).sort((a, b) => a.at[2] - b.at[2])[0];
  if (!ahead) throw new Error("blastLamp: no lamp ahead of the camera");
  const [x, y, z] = ahead.at;
  return { ground: [x, y, z], head: [x + 0.75, y + 4.35, z + 0.05] };
}

export function nightGame(o: Night = {}) {
  const seed = o.seed ?? 1;
  const keys = cameraPath(seed);
  const key = keys[SHOTS[o.shot ?? 0]];
  const lamp = blastLamp(seed);
  const blast = (): void => {
    if (!o.blast) return;
    const [at, r]: [[number, number, number], number] = o.blast === "lamp" ? [lamp.head, 1] : [[lamp.ground[0] - 0.05, lamp.ground[1] + 1, lamp.ground[2] - 0.05], 0.15];
    const e = voxelExplode(at[0], at[1], at[2], r);
    console.log(`explosion ${JSON.stringify({ frame: frames, blast: o.blast, r, ...e })}`);
  };
  let frames = 0;
  return defineGame<Record<string, never>>({
    state: {},
    setup: () => {
      frames = 0;
      const scene = new Scene();
      // Below the horizon: the atmosphere goes dark; the glow is the night sky's over a lit town.
      const sun = new DirectionalLight(0xffffff, 90000);
      sun.setSunAngles(-18, 300);
      scene.add(sun);
      scene.environment = new SkyAtmosphere({ sun, glow: 0x6b7fb3, glowIntensity: 0.2 });
      const camera = new PerspectiveCamera(55, 16 / 9, 0.05, 2000);
      camera.name = "camera";
      camera.position.set(...key.pos);
      camera.lookAt(...key.target);
      camera.setManualExposure(NIGHT_EV);
      scene.add(camera);
      const ground = new MeshStandardMaterial({ name: "ground", color: PALETTE[1][1], roughness: 0.9 });
      const half = (SIZE / 2) * VOXEL, far = 100000;
      for (const [w, d, x, z] of [[2 * far, far - half, 0, -(far + half) / 2], [2 * far, far - half, 0, (far + half) / 2], [far - half, 2 * half, -(far + half) / 2, 0], [far - half, 2 * half, (far + half) / 2, 0]]) {
        const strip = new Mesh(new PlaneGeometry(w, d), ground);
        strip.rotation.x = -Math.PI / 2;
        strip.position.set(x, EDGE * VOXEL, z);
        scene.add(strip);
      }
      voxelLevel({ route: "mesh", palette: nativePalette(o.lamps ?? true), objects: nativeObjects(seed) });
      if (o.when === "setup") {
        blast();
        for (let k = 0; k < Math.round((o.advance ?? 0) * 60); k++) voxelFrame(1 / 60);
      }
      const name = `voxel-level-night${o.lamps === false ? "-dark" : ""}${o.blast ? `-${o.blast}-${o.when === "setup" ? "setup" : "live"}` : ""}-${o.shot ?? 0}`;
      return { scene, camera, options: { name, width: 1280, height: 720, preset: "cinematic" as const } };
    },
    frame(f) {
      frames++;
      if (typeof o.when !== "number") return;
      if (frames === o.when) blast();
      const b = voxelFrame(f.dt);
      if ("batches" in b && (b.batches > 0 || (b.settled ?? 0) > 0)) console.log(`batch ${JSON.stringify({ frame: frames, ...b })}`);
    },
    handlers: {},
  });
}

export default nightGame();
