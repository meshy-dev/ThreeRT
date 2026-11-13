// One explosion's fire on the benchmark level by day, and a bare room to calibrate the fire light in, for measuring
// them (ROADMAP § 6 step 10 b; tools/bench-voxel-fire.ts). Every frame script here is a function of the scene time, so
// an offline render at a time shows what a live frame at that time does.
//
// fireShot: the level by day from the destruction game's first camera, a 4 m explosion at its `crater` spot (9 m ahead
// on the road; done in setup, so the crater is there from the start in both renders), its fire (`Explosions`) starting
// at scene time `at`: with the fire light or without; with a smoke grid each fire puffs into (the game's own
// `SimulatedSmoke`, as `Explosions` would make it once it takes one again), a second fireball overlapping the first, or
// an object added at `rebuildAt` to make the live scene rebuild (the checks of docs/VOXEL.md's "Temporary ways round on
// the engine's shared paths"). Offline, the fire light is left
// out: the fireball's own emission lights the street there (paths reach the medium), which is what the light stands in
// for live.
//
// fireRoom: a grey floor and a wall, no sky; either one fireball of size `size` m burning from scene time 0, high enough
// that the floor is below it, or a white point light of 1,000 cd at its centre. The floor's and the wall's radiance
// near it, fire against light, is the fireball's equivalent intensity (`FIRE_LIGHT`).
import { BoxGeometry, DirectionalLight, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, PointLight, Scene, SimulatedSmoke, SkyAtmosphere } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { Explosions, voxelExplode, voxelLevel } from "@threert/voxel";
import { spots } from "./level-destruction.game.ts";
import { nativeObjects, nativePalette } from "./native.ts";
import { EDGE, PALETTE, SIZE, VOXEL, cameraPath } from "./plan.ts";

export interface FireShot {
  /** The fire light (live); off for the offline reference. Default on. */
  light?: boolean;
  /** When the fire starts (scene time, s; default 4). */
  at?: number;
  /** The explosion's radius (m; default 4). */
  r?: number;
  /** The smoke grid (the destruction game's: 88 x 56 x 144 cells over the streets ahead). Default off. */
  smoke?: boolean;
  /** The smoke's turbulence (its random push; `SimulatedSmoke`'s default 15). */
  turbulence?: number;
  /** A second fire as the first, its centre 0.4 of its size to the side: two fireballs that overlap. */
  pair?: boolean;
  /** A box added to the scene at this scene time (s): the live scene rebuilds then. */
  rebuildAt?: number;
}

/** The shot's camera (the destruction game's first) and the explosion's spot. */
export function fireShotView(seed = 1) {
  const key = cameraPath(seed)[0];
  return { pos: key.pos, target: key.target, fov: 55, spot: spots(seed)[2 * 62] };
}

export function fireShot(o: FireShot = {}) {
  const seed = 1, at = o.at ?? 4, r = o.r ?? 4;
  const view = fireShotView(seed);
  let boom: Explosions;
  let cloud: SimulatedSmoke | null = null;
  let fired = false, rebuilt = false;
  let world: Scene;
  // The fire, and with `pair` a second one 0.4 of the fireball's size (2.2 r) to the side; with the grid, a puff of hot
  // smoke at each fireball's centre (0.3 r above the explosion's point, 0.8 r + 0.3 m across).
  const fireAll = () => {
    const spots: [number, number, number][] = [view.spot, ...(o.pair ? [[view.spot[0] + 0.88 * r, view.spot[1], view.spot[2]] as [number, number, number]] : [])];
    for (const p of spots) {
      boom.fire(p, r, at);
      cloud?.puff([p[0], p[1] + 0.3 * r, p[2]], 0.8 * r + 0.3, { density: 0.9, temperature: 1, push: 1.2 });
    }
  };
  return defineGame<Record<string, never>>({
    state: {},
    setup: () => {
      fired = false;
      const scene = new Scene();
      const sun = new DirectionalLight(0xffffff, 90000);
      sun.setSunAngles(38, 300);
      scene.add(sun);
      scene.environment = new SkyAtmosphere({ sun });
      const camera = new PerspectiveCamera(view.fov, 16 / 9, 0.05, 2000);
      camera.name = "camera";
      camera.position.set(...view.pos);
      camera.lookAt(...view.target);
      camera.setManualExposure(13);
      scene.add(camera);
      const ground = new MeshStandardMaterial({ name: "ground", color: PALETTE[1][1], roughness: 0.9 });
      const half = (SIZE / 2) * VOXEL, far = 100000;
      for (const [w, d, x, z] of [[2 * far, far - half, 0, -(far + half) / 2], [2 * far, far - half, 0, (far + half) / 2], [far - half, 2 * half, -(far + half) / 2, 0], [far - half, 2 * half, (far + half) / 2, 0]]) {
        const strip = new Mesh(new PlaneGeometry(w, d), ground);
        strip.rotation.x = -Math.PI / 2;
        strip.position.set(x, EDGE * VOXEL, z);
        scene.add(strip);
      }
      voxelLevel({ route: "mesh", palette: nativePalette(false), objects: nativeObjects(seed), physics: false });
      voxelExplode(...view.spot, r);
      const [cx, cy, cz] = view.pos;
      boom = new Explosions({ fireballs: o.pair ? 2 : 1, lights: o.light === false ? 0 : o.pair ? 2 : 1 });
      scene.add(boom);
      // The smoke grid (the destruction game's box and cells; `Explosions`' old defaults for the smoke and the puff).
      cloud = o.smoke ? new SimulatedSmoke({ size: [88, 56, 144], density: 2.5, wind: [0.8, 0, 0.3], buoyancy: 3.5, cooling: 0.9, dissipation: 0.2, turbulence: o.turbulence ?? 15 }) : null;
      if (cloud) {
        cloud.position.set(cx, cy + 5, cz + 21);
        cloud.scale.set(22, 14, 36);
        scene.add(cloud);
      }
      world = scene;
      rebuilt = false;
      // A fire at scene time 0 is started in setup: its puff then reaches the grid at the first step, live and offline.
      if (at <= 0) {
        fireAll();
        fired = true;
      }
      return { scene, camera, options: { name: `voxel-level-fire`, width: 1280, height: 720, preset: "cinematic" as const } };
    },
    frame(f) {
      if (!fired && f.time >= at) {
        fired = true;
        fireAll();
        console.log(`fire ${JSON.stringify({ at, time: f.time })}`);
      }
      if (o.rebuildAt !== undefined && !rebuilt && f.time >= o.rebuildAt) {
        rebuilt = true;
        // Out of the view, behind the camera: the added object changes what is built, not what is seen.
        const crate = new Mesh(new BoxGeometry(0.5, 0.5, 0.5), new MeshStandardMaterial({ name: "crate", color: 0x777777 }));
        crate.position.set(view.pos[0], view.pos[1] - 3, view.pos[2] - 20);
        world.add(crate);
        console.log(`rebuild ${JSON.stringify({ time: f.time })}`);
      }
      boom.step(f.time);
    },
    handlers: {},
  });
}

export interface FireRoom {
  /** The fireball's full size (m). */
  size: number;
  /** A point light of 1,000 cd at the centre instead of the fireball. */
  light?: boolean;
}

/** The room's camera and the fireball's centre, by the fireball's size `d`. */
export function fireRoomView(d: number) {
  return { pos: [0, 0.8 * d, 4 * d] as const, target: [0.8 * d, 0.4 * d, -0.5 * d] as const, fov: 55, center: [0, 0.6 * d, 0] as const, wall: -1.5 * d };
}

export function fireRoom(o: FireRoom) {
  const d = o.size, v = fireRoomView(d);
  return defineGame<Record<string, never>>({
    state: {},
    setup: () => {
      const scene = new Scene();
      const camera = new PerspectiveCamera(v.fov, 16 / 9, 0.05, 2000);
      camera.name = "camera";
      camera.position.set(...v.pos);
      camera.lookAt(...v.target);
      camera.setManualExposure(8);
      scene.add(camera);
      const grey = new MeshStandardMaterial({ name: "grey", color: 0xbcbcbc, roughness: 1 });
      const floor = new Mesh(new PlaneGeometry(20 * d, 20 * d), grey);
      floor.rotation.x = -Math.PI / 2;
      scene.add(floor);
      const wall = new Mesh(new PlaneGeometry(20 * d, 6 * d), grey);
      wall.position.set(0, 3 * d, v.wall);
      scene.add(wall);
      if (o.light) {
        const l = new PointLight(0xffffff, 1000);
        l.position.set(...v.center);
        scene.add(l);
      } else {
        // r such that the fireball's size is d; its centre 0.3 r above the explosion's point.
        const r = d / 2.2;
        const boom = new Explosions({ fireballs: 1, lights: 0 });
        boom.fire([v.center[0], v.center[1] - 0.3 * r, v.center[2]], r, 0);
        scene.add(boom);
      }
      return { scene, camera, options: { name: `fire-room`, width: 640, height: 360, preset: "cinematic" as const } };
    },
    handlers: {},
  });
}

export default fireShot();
