// Tokyo, the open city (docs/ROADMAP.md § 0): one compiled area resident, its 256 m tiles as GLB models converted from
// the city pipeline (tools/city, from jeantimex/tokyo; tools/tokyo/to-glb.ts). Static: no traffic yet.
//
// Room id: "<area>[-day|-night]" (default shinjuku-night). The area's tiles must be compiled and converted first:
//   node tools/city/pipeline/fetch.mjs --area=shinjuku
//   node --max-old-space-size=8192 tools/city/pipeline/compile.mjs --area=shinjuku
//   node tools/tokyo/to-glb.ts --area=shinjuku
//
// Data: 3D city model, Project PLATEAU (MLIT Japan); elevation and aerial photos, GSI; roads and places,
// © OpenStreetMap contributors (ODbL). Compiled tiles are a derived database under the ODbL: not to be published.
import { Color, DirectionalLight, GLTFModel, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, Scene, SkyAtmosphere, SkyColor } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { WINDOW_NODES, type AreaIndex } from "./area.ts";
import shinjuku from "./areas/shinjuku.ts";

const AREAS: Record<string, AreaIndex> = { shinjuku };

/**
 * The acceptance poses (ROADMAP § 0 step 1) in Shinjuku's world metres (x east, y up = height above sea level, z south;
 * the origin at Shinjuku Station). Street poses stand 1.7 m above the terrain there.
 */
export const POSES: Record<string, { position: [number, number, number]; target: [number, number, number]; fov: number }> = {
  // Kabukichō Ichibangai's gate (139.70253, 35.69385 -> 184, -472), from Yasukuni-dori to its south (the road graph's
  // node there), up the street
  "kabukicho-gate": { position: [162.7, 34.7, -426.8], target: [190, 38, -520], fov: 60 },
  // the west exit's end of Chūō-dōri (-290, -165), west along it to the skyscraper district
  "west-exit": { position: [-300, 40.7, -165], target: [-650, 75, -120], fov: 60 },
  // on Tochō-dōri south of the Metropolitan Government Building (-792, 0), north between the towers, looking up
  "skyscraper-canyon": { position: [-712, 42.7, 150], target: [-775, 100, -160], fov: 60 },
  // over Shinjuku Gyoen towards the station, and from the west over the skyscrapers
  "aerial-east": { position: [900, 380, 700], target: [-200, 40, -200], fov: 50 },
  "aerial-west": { position: [-1100, 340, 900], target: [0, 40, -200], fov: 50 },
};
/** Manual exposure (EV100): night over a lit city, and a sunny day. */
export const EV100 = { night: 5, day: 14 };
/** The day's sun (degrees; azimuth from the south, +z, towards the east, +x): Tokyo at 12:00 JST on 2026-11-09, where
 *  the source app puts it for ?time=12 that day, low in the south, a little west. */
export const SUN = { elevation: 36.8, azimuth: 349.5 };

/**
 * The sky over the city by day: the atmosphere with Tokyo's autumn air, Mie (aerosols) at 10x the clean-air default,
 * a visual range near 70 km (Koschmieder, 3.9 / extinction at 550 nm), and its aerial perspective, so the land beyond
 * the area and the far city fade into the sky at the horizon.
 */
export function tokyoSky(sun: DirectionalLight, o: { dynamic?: boolean; glow?: number; glowIntensity?: number } = {}): SkyAtmosphere {
  const sky = new SkyAtmosphere({ sun, aerialPerspective: 1, ...o });
  sky.mieScale = 10;
  return sky;
}

export function sceneFor(room: string) {
  const [areaId = "shinjuku", time = "night"] = room.split("-");
  const area = AREAS[areaId] ?? AREAS.shinjuku;
  const scene = new Scene();
  if (time === "day") {
    const sun = new DirectionalLight(0xfff4e8, 100000).setSunAngles(SUN.elevation, SUN.azimuth);
    scene.add(sun);
    scene.environment = tokyoSky(sun);
  } else {
    // Urban night sky glow, as Bistro's.
    scene.environment = new SkyColor(new Color().setRGB(0.35, 0.45, 0.8), 0.04);
  }
  // The land beyond the area (not compiled): a plain plane below the lowest terrain, out to 100 km.
  const land = new Mesh(new PlaneGeometry(200000, 200000), new MeshStandardMaterial({ color: 0x3b3a36, roughness: 0.95 }));
  land.rotation.x = -Math.PI / 2;
  land.position.set((area.bounds.minX + area.bounds.maxX) / 2, 20, (area.bounds.minZ + area.bounds.maxZ) / 2);
  land.name = "land";
  scene.add(land);
  // Each tile's GLB: the city, and the street lamps' heads twice, lit ("night") and unlit ("day"): tools/tokyo/glb.ts.
  const nodes = ["city", time === "day" ? "day" : "night", ...WINDOW_NODES];
  for (const t of area.tiles) {
    const m = new GLTFModel(`assets/city/glb/${area.area}/${t.file}`, { nodes });
    m.name = `tile ${t.x} ${t.z}`;
    scene.add(m);
  }
  const p = POSES["aerial-east"];
  const camera = new PerspectiveCamera(p.fov, 16 / 9, 0.1, 20000);
  camera.name = "camera";
  camera.position.set(...p.position);
  camera.lookAt(...p.target);
  camera.setManualExposure(time === "day" ? EV100.day : EV100.night);
  scene.add(camera);
  return { scene, camera, options: { name: `tokyo-${area.area}-${time}`, width: 1280, height: 720 } };
}

export default defineGame<Record<string, never>>({
  state: {},
  keys: {},
  handlers: {},
  setup(ctx) {
    return sceneFor(ctx.roomId || "shinjuku-night");
  },
});
