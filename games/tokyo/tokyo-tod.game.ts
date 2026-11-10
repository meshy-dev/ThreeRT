// Tokyo through the time of day (docs/ROADMAP.md, the dynamic sky): an area of the open city with its sun where it
// stands over Tokyo at each hour of 2026-11-09, the sky following it, the exposure following the light, and the city's
// lamps (street lights, Tokyo Tower's bulbs and beacons) switching on, tile by tile, as the sun goes down. The sky is
// `dynamic`: the live view moves the sun's light and re-bakes the sky in place, with no scene rebuild.
//
// Room id: "<area>[-<from>-<to>-<seconds>][-r<radius>]", e.g. "shiba-15.5-17.75-7-r1600": the clock runs from 15:30 to
// 17:45 (JST) over the first 7 s of scene time, and only the tiles within 1600 m of the area's origin are loaded (Shiba's
// origin is Tokyo Tower). Defaults: shiba, 15.5 -> 17.75 over 7 s, every tile.
//
// Data: 3D city model, Project PLATEAU (MLIT Japan); elevation and aerial photos, GSI; roads and places,
// © OpenStreetMap contributors (ODbL). Compiled tiles are a derived database under the ODbL: not to be published.
import { DirectionalLight, GLTFModel, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, Scene } from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";
import { WINDOW_NODES, windowSwitchAt, type AreaIndex } from "./area.ts";
import shiba from "./areas/shiba.ts";
import shinjuku from "./areas/shinjuku.ts";
import { tokyoSky } from "./tokyo.game.ts";

const AREAS: Record<string, AreaIndex> = { shiba, shinjuku };
const RAD = Math.PI / 180;

/**
 * The sun over Tokyo (35.6586 N, 139.7454 E) at `hour` JST on 2026-11-09 (day 313): elevation, and azimuth as
 * `DirectionalLight.setSunAngles` takes it (from the south, +z, towards the east, +x), degrees. NOAA's approximations
 * (Spencer's declination and equation of time), within a few tenths of a degree.
 */
export function sunAt(hour: number, lat = 35.6586, lon = 139.7454, day = 313): { elevation: number; azimuth: number } {
  const g = ((2 * Math.PI) / 365) * (day - 1 + (hour - 9 - 12) / 24);
  const decl = 0.006918 - 0.399912 * Math.cos(g) + 0.070257 * Math.sin(g) - 0.006758 * Math.cos(2 * g) + 0.000907 * Math.sin(2 * g) - 0.002697 * Math.cos(3 * g) + 0.00148 * Math.sin(3 * g);
  const eot = 229.18 * (0.000075 + 0.001868 * Math.cos(g) - 0.032077 * Math.sin(g) - 0.014615 * Math.cos(2 * g) - 0.040849 * Math.sin(2 * g));
  const solar = hour + (4 * lon - 60 * 9 + eot) / 60; // true solar time, hours
  const h = (solar - 12) * 15 * RAD, phi = lat * RAD;
  const el = Math.asin(Math.sin(phi) * Math.sin(decl) + Math.cos(phi) * Math.cos(decl) * Math.cos(h));
  // azimuth from the north, clockwise
  const a = Math.atan2(Math.sin(h), Math.cos(h) * Math.sin(phi) - Math.tan(decl) * Math.cos(phi)) + Math.PI;
  return { elevation: el / RAD, azimuth: (((180 - a / RAD) % 360) + 360) % 360 };
}

/** Manual exposure (EV100) for a sun elevation: a sunny 14, sunset ~11, civil dusk ~8, the lit city at night 5. */
export function evAt(elevation: number): number {
  const keys: [number, number][] = [[-12, 5], [-6, 7], [-2, 9], [2, 11], [8, 12.8], [20, 14]];
  if (elevation <= keys[0][0]) return keys[0][1];
  for (let i = 1; i < keys.length; i++) {
    const [e1, v1] = keys[i], [e0, v0] = keys[i - 1];
    if (elevation <= e1) {
      const u = (elevation - e0) / (e1 - e0);
      return v0 + (v1 - v0) * u * u * (3 - 2 * u);
    }
  }
  return keys[keys.length - 1][1];
}

/** The clock (hours, JST) at scene time t. */
export function hourAt(t: number, from: number, to: number, seconds: number): number {
  return from + (to - from) * Math.min(1, Math.max(0, t / seconds));
}

/** Where a tile's lamps switch on: a sun elevation between +1 and -3 deg, scattered by tile (lamps on timers and
 *  photocells do not all switch at once). */
export function switchAt(x: number, z: number): number {
  const h = Math.sin(x * 12.9898 + z * 78.233) * 43758.5453;
  return 1 - 4 * (h - Math.floor(h));
}

type Room = { area: AreaIndex; from: number; to: number; seconds: number; radius: number };
export function parseRoom(room: string): Room {
  const parts = room.split("-");
  const area = AREAS[parts[0]] ?? shiba;
  const r = parts.find((p) => /^r\d+$/.test(p));
  const nums = parts.slice(1).filter((p) => !/^r\d+$/.test(p)).map(Number);
  const [from = 15.5, to = 17.75, seconds = 7] = nums.length === 3 && nums.every(Number.isFinite) ? nums : [];
  return { area, from, to, seconds, radius: r ? Number(r.slice(1)) : Infinity };
}

let room: Room = parseRoom("shiba");
/** What the clock switches (lamps, window groups): the object, the sun elevation it comes on at, and whether it is on. */
let lamps: { name: string; at: number; on?: boolean }[] = [];

function place(sun: DirectionalLight, camera: PerspectiveCamera, t: number, lampOf: (name: string) => { power: number } | undefined) {
  const s = sunAt(hourAt(t, room.from, room.to, room.seconds));
  sun.setSunAngles(s.elevation, s.azimuth);
  camera.setManualExposure(evAt(s.elevation));
  // only what switches this frame is looked up and touched: a few hundred objects a frame cost the frame script 15-20 ms
  for (const l of lamps) {
    const on = s.elevation < l.at;
    if (on === l.on) continue;
    const m = lampOf(l.name);
    if (m) {
      m.power = on ? 1 : 0;
      l.on = on;
    }
  }
}

export function sceneFor(id: string) {
  room = parseRoom(id);
  const { area, radius } = room;
  const scene = new Scene();
  // Above the atmosphere, the sun's 100 klx (the atmosphere takes its share, by elevation).
  const sun = new DirectionalLight(0xfff4e8, 100000);
  sun.name = "sun";
  scene.add(sun);
  // The night sky's glow over a lit city once the sun has set (the atmosphere alone goes black).
  scene.environment = tokyoSky(sun, { dynamic: true, glow: 0x6b7fb3, glowIntensity: 0.03 });
  // The land beyond the area (not compiled): a plain plane below the lowest terrain, out to 100 km.
  const land = new Mesh(new PlaneGeometry(200000, 200000), new MeshStandardMaterial({ color: 0x3b3a36, roughness: 0.95 }));
  land.rotation.x = -Math.PI / 2;
  land.position.set((area.bounds.minX + area.bounds.maxX) / 2, 1, (area.bounds.minZ + area.bounds.maxZ) / 2);
  land.name = "land";
  scene.add(land);
  // Each tile's GLB in parts: the city; its lamps lit ("night", tools/tokyo/glb.ts); and its lit windows, a switch group
  // at a time (games/tokyo/area.ts). The clock switches the lamps' and each group's power, with no rebuild.
  const T = 256;
  lamps = [];
  for (const t of area.tiles) {
    const cx = (t.x + 0.5) * T, cz = (t.z + 0.5) * T;
    if (Math.hypot(cx, cz) > radius + T) continue;
    const city = new GLTFModel(`assets/city/glb/${area.area}/${t.file}`, { nodes: ["city"] });
    city.name = `tile ${t.x} ${t.z}`;
    scene.add(city);
    const lit = new GLTFModel(`assets/city/glb/${area.area}/${t.file}`, { nodes: ["night"] });
    lit.name = `lamps ${t.x} ${t.z}`;
    scene.add(lit);
    lamps.push({ name: lit.name, at: switchAt(t.x, t.z) });
    for (const [g, node] of WINDOW_NODES.entries()) {
      const win = new GLTFModel(`assets/city/glb/${area.area}/${t.file}`, { nodes: [node] });
      win.name = `windows ${t.x} ${t.z} ${g}`;
      scene.add(win);
      // A group switches on every tile in the same frame: any power change costs the live view's commit ~13 ms (the
      // emitter CDF and the light tree rebuilt over all 1.4 M emitters, renderer/src/live.rs write_emitters), so fewer
      // switching frames is better until that is incremental.
      lamps.push({ name: win.name, at: windowSwitchAt(g) });
    }
  }
  const camera = new PerspectiveCamera(60, 16 / 9, 0.1, 20000);
  camera.name = "camera";
  camera.position.set(900, 260, -900);
  camera.lookAt(0, 180, 0);
  scene.add(camera);
  place(sun, camera, 0, (n) => scene.getObjectByName(n));
  return { scene, camera, options: { name: `tokyo-${area.area}-tod`, width: 1280, height: 720 } };
}

export default defineGame<Record<string, never>>({
  state: {},
  keys: {},
  handlers: {},
  setup(ctx) {
    return sceneFor(ctx.roomId || "shiba");
  },
  frame(f: FrameContext<Record<string, never>>) {
    place(f.object("sun") as DirectionalLight, f.camera, f.time, (n) => f.object(n));
  },
});
