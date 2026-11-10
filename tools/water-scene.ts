// The water scene of ROADMAP § 5 item 6.4: a night canal. A WaterBody 6 m wide, 40 m long and 1.5 m deep (its
// surface at y = 0) between concrete embankments whose walkways stand 0.6 m above it; a wooden pier from the left
// walkway over the water on four posts down to the bed; on the right walkway two warm street lamps and a magenta neon
// sign on a wall, both reflected in the water; a dim blue night sky. The camera stands on the left walkway at
// (-2.4, 2.1, 12) looking down the canal at (1.2, 0.2, -6), 45-degree vertical field of view, manual exposure EV100 1.
//   node tools/bundle-game.ts tools/water-scene.ts -o out/bundles/water-scene.js
import { BoxGeometry, Color, Mesh, MeshStandardMaterial, PerspectiveCamera, PointLight, Scene, SkyColor, WaterBody } from "@threert/scene";
import { defineGame } from "@threert/gameplay";

const box = (mat: MeshStandardMaterial, size: [number, number, number], at: [number, number, number]) => {
  const m = new Mesh(new BoxGeometry(...size), mat);
  m.position.set(...at);
  return m;
};

export default defineGame({
  state: {},
  setup: () => {
    const scene = new Scene();
    scene.environment = new SkyColor(0x1a2a50, 0.02);
    const camera = new PerspectiveCamera(45, 16 / 9, 0.05, 200);
    camera.position.set(-2.4, 2.1, 12);
    camera.lookAt(1.2, 0.2, -6);
    camera.setManualExposure(1);
    scene.add(camera);

    const concrete = new MeshStandardMaterial({ name: "concrete", color: 0x8a8580, roughness: 0.85 });
    const bed = new MeshStandardMaterial({ name: "canal-bed", color: 0x4a4a40, roughness: 0.95 });
    const wood = new MeshStandardMaterial({ name: "wood", color: 0x6b4a30, roughness: 0.7 });
    const wall = new MeshStandardMaterial({ name: "wall", color: 0x5a5560, roughness: 0.8 });
    const pole = new MeshStandardMaterial({ name: "pole", color: 0x303030, roughness: 0.4, metalness: 1 });

    // The canal: water over a bed, embankment walls from the bed to the walkways (y = 0.6) on either side.
    const water = new WaterBody({ width: 6, depth: 1.5, length: 40 });
    scene.add(water);
    scene.add(box(bed, [6, 0.2, 40], [0, -1.6, 0]));
    for (const s of [-1, 1]) {
      scene.add(box(concrete, [0.4, 2.1, 40], [s * 3.2, -0.45, 0]));
      scene.add(box(concrete, [5, 0.2, 40], [s * 5.9, 0.5, 0]));
    }

    // The pier: a deck 0.35 m above the water from the left wall, on four posts standing on the bed.
    scene.add(box(wood, [2.4, 0.12, 3], [-1.8, 0.35, 2]));
    for (const [x, z] of [[-0.8, 0.7], [-0.8, 3.3], [-2.4, 0.7], [-2.4, 3.3]]) scene.add(box(wood, [0.16, 2.0, 0.16], [x, -0.6, z]));

    // Right walkway: two street lamps and a wall with a neon sign.
    for (const z of [2, -10]) {
      scene.add(box(pole, [0.12, 4, 0.12], [4, 2.6, z]));
      const lamp = new PointLight(0xffc080, 400);
      lamp.position.set(3.7, 4.5, z);
      lamp.radius = 0.1;
      scene.add(lamp);
    }
    scene.add(box(wall, [0.4, 6, 14], [6.5, 3.6, -4]));
    const neon = new MeshStandardMaterial({ name: "neon", color: 0x000000, emissive: new Color().setRGB(1, 0.15, 0.6), emissiveIntensity: 30 });
    scene.add(box(neon, [0.08, 0.5, 4], [6.25, 2.6, -4]));
    scene.add(box(neon, [0.08, 0.12, 3], [6.25, 3.2, -4]));
    return { scene, camera, options: { name: "water", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  handlers: {},
});
