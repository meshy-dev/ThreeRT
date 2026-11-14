// Many shadow-casting local lights, most of them behind walls: a floor of rooms (4 x 3 here; lamp-rooms-wide.ts:
// 12 x 8) with doorways between them, eight lamps a room (four ceiling spots, four point lamps: 96 in all, each with
// a range that reaches into the rooms next door), no sky. What a light sampler has to find here is which lamps a point can see.
import { BoxGeometry, Color, Mesh, MeshStandardMaterial, PerspectiveCamera, PointLight, Scene, SkyColor, SpotLight } from "@threert/scene";

const S = 5, H = 3, T = 0.2, DOOR = 1.2, DOOR_H = 2.2;
const TINTS = [0xffd9a8, 0xbcd4ff, 0xffb38a, 0xc8ffd0, 0xffe6c4, 0xd9c2ff];

/** A floor of `NX` x `NZ` rooms (8 lamps each). */
export function lampRooms(NX: number, NZ: number, name: string) {
  const scene = new Scene();
  scene.environment = new SkyColor(0xffffff, 0);
  const plaster = new MeshStandardMaterial({ name: "plaster", color: 0xd6d1c8, roughness: 0.9 });
  const floorMat = new MeshStandardMaterial({ name: "floor", color: 0x9a9da3, roughness: 0.3 });
  const wood = new MeshStandardMaterial({ name: "wood", color: 0x7a5533, roughness: 0.6 });
  const metal = new MeshStandardMaterial({ name: "steel", color: new Color().setRGB(0.6, 0.62, 0.65), metalness: 1, roughness: 0.35 });
  const box = (name: string, w: number, h: number, d: number, x: number, y: number, z: number, mat: MeshStandardMaterial) => {
    const m = new Mesh(new BoxGeometry(w, h, d), mat);
    m.name = name;
    m.position.set(x, y, z);
    scene.add(m);
  };
  const P = S + T, W = NX * P + T, D = NZ * P + T;
  const cx = (i: number) => (i - (NX - 1) / 2) * P, cz = (j: number) => (j - (NZ - 1) / 2) * P;
  box("floor", W, T, D, 0, -T / 2, 0, floorMat);
  box("ceiling", W, T, D, 0, H + T / 2, 0, plaster);
  // Walls on the grid's lines: solid outside, a doorway in each room's side inside. Posts where the lines cross.
  for (let i = 0; i <= NX; i++) {
    for (let j = 0; j < NZ; j++) {
      const x = cx(i) - P / 2, z = cz(j), n = `wall_x${i}_${j}`;
      if (i === 0 || i === NX) {
        box(n, T, H, S, x, H / 2, z, plaster);
      } else {
        const side = (S - DOOR) / 2;
        box(`${n}_a`, T, H, side, x, H / 2, z - (DOOR + side) / 2, plaster);
        box(`${n}_b`, T, H, side, x, H / 2, z + (DOOR + side) / 2, plaster);
        box(`${n}_top`, T, H - DOOR_H, DOOR, x, (H + DOOR_H) / 2, z, plaster);
      }
    }
  }
  for (let j = 0; j <= NZ; j++) {
    for (let i = 0; i < NX; i++) {
      const z = cz(j) - P / 2, x = cx(i), n = `wall_z${i}_${j}`;
      if (j === 0 || j === NZ) {
        box(n, S, H, T, x, H / 2, z, plaster);
      } else {
        const side = (S - DOOR) / 2;
        box(`${n}_a`, side, H, T, x - (DOOR + side) / 2, H / 2, z, plaster);
        box(`${n}_b`, side, H, T, x + (DOOR + side) / 2, H / 2, z, plaster);
        box(`${n}_top`, DOOR, H - DOOR_H, T, x, (H + DOOR_H) / 2, z, plaster);
      }
    }
  }
  for (let i = 0; i <= NX; i++) for (let j = 0; j <= NZ; j++) box(`post_${i}_${j}`, T, H, T, cx(i) - P / 2, H / 2, cz(j) - P / 2, plaster);

  for (let i = 0; i < NX; i++) {
    for (let j = 0; j < NZ; j++) {
      const x = cx(i), z = cz(j), k = i * NZ + j, tint = TINTS[k % TINTS.length], n = `r${i}_${j}`;
      // Furniture: a table, a shelf against a wall, a steel crate; placed by the room's number.
      const sx = k % 2 ? 1 : -1, sz = k % 3 ? 1 : -1;
      box(`${n}_table`, 1.3, 0.06, 0.8, x + 0.9 * sx, 0.74, z - 0.7 * sz, wood);
      for (const [dx, dz] of [[-0.55, -0.3], [0.55, -0.3], [-0.55, 0.3], [0.55, 0.3]]) box(`${n}_leg${dx}${dz}`, 0.06, 0.71, 0.06, x + 0.9 * sx + dx, 0.355, z - 0.7 * sz + dz, wood);
      box(`${n}_shelf`, 0.4, 1.9, 1.4, x - (S / 2 - 0.25) * sx, 0.95, z + 0.9 * sz, wood);
      box(`${n}_crate`, 0.7, 0.7, 0.7, x - 1.1 * sx, 0.35, z - 1.4 * sz, metal);
      const add = (l: PointLight | SpotLight, name: string, px: number, py: number, pz: number) => {
        l.name = `${n}_${name}`;
        l.position.set(px, py, pz);
        l.radius = 0.03;
        scene.add(l);
      };
      for (const [q, [dx, dz]] of [[-1.25, -1.25], [1.25, -1.25], [-1.25, 1.25], [1.25, 1.25]].entries()) {
        const s = new SpotLight(tint, 1800, 7, (52 * Math.PI) / 180, 0.4);
        add(s, `spot${q}`, x + dx, H - 0.12, z + dz);
        s.target!.position.set(x + dx, 0, z + dz);
        scene.add(s.target!);
      }
      add(new PointLight(tint, 320, 6), "table_lamp", x + 0.9 * sx, 1.05, z - 0.7 * sz);
      add(new PointLight(tint, 260, 6), "shelf_lamp", x - (S / 2 - 0.55) * sx, 2.05, z + 0.9 * sz);
      add(new PointLight(0xffffff, 240, 6), "sconce_a", x + (S / 2 - 0.4) * sx, 2.0, z + 1.5 * sz);
      add(new PointLight(0xffffff, 240, 6), "sconce_b", x - 1.5 * sx, 2.0, z - (S / 2 - 0.4) * sz);
    }
  }

  // From a corner of an inner room, across it and through its doorways into the rooms beyond.
  const camera = new PerspectiveCamera(72, 16 / 9, 0.05, 100);
  camera.name = "camera";
  camera.position.set(cx(1) - 1.9, 1.5, cz(1) + 1.9);
  camera.lookAt(cx(2) + 0.6, 1.1, cz(0) + 1.2);
  camera.setManualExposure(8.2);
  scene.add(camera);
  return { scene, camera, options: { name, width: 1280, height: 720 } };
}

export default function build() {
  return lampRooms(4, 3, "lamp-rooms");
}
