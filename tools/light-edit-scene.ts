// The lighting-edit scene of bench-temporal's `--shot edit`: a closed room, 10 x 4 x 8 m of grey plaster, the camera
// inside it. Three sources:
//   - a ceiling lamp on the left (a point light, always on): the room's base light;
//   - `lamp`: a spot on the right aimed at the back wall, where it makes a pool; the floor before the wall is outside
//     its cone and takes its light only bounced off the wall (the radiance cache's);
//   - `sign`: an emissive panel on the left wall (light-sampled as an emitter), lighting the floor beside it.
// Events `lamp` and `sign` switch one off or on at once (the object's power: a parameter change, in place), so the
// bench can count the frames the live view takes to follow: direct light of a local light, its bounce, an emitter's.
//   node tools/bench-temporal.ts --shot edit      (it bundles this file to out/bundles/light-edit-scene.js)
import { BoxGeometry, Color, Mesh, MeshStandardMaterial, PerspectiveCamera, PointLight, Scene, SkyColor, SpotLight } from "@threert/scene";
import { defineGame } from "@threert/gameplay";

const box = (m: MeshStandardMaterial, size: [number, number, number], at: [number, number, number], name?: string) => {
  const b = new Mesh(new BoxGeometry(...size), m);
  b.position.set(...at);
  if (name) b.name = name;
  return b;
};

export default defineGame({
  state: { lamp: true, sign: true },
  setup: () => {
    const scene = new Scene();
    scene.environment = new SkyColor(0x000000, 0);
    const camera = new PerspectiveCamera(60, 16 / 9, 0.05, 100);
    camera.position.set(0, 1.6, 3.4);
    camera.lookAt(0, 1.3, -4);
    camera.setManualExposure(5);
    scene.add(camera);

    const plaster = new MeshStandardMaterial({ name: "plaster", color: 0xb0aca4, roughness: 0.9 });
    const floor = new MeshStandardMaterial({ name: "floor", color: 0x8c8880, roughness: 0.7 });
    const wood = new MeshStandardMaterial({ name: "wood", color: 0x6b4a30, roughness: 0.6 });
    scene.add(box(floor, [10, 0.2, 8], [0, -0.1, 0]));
    scene.add(box(plaster, [10, 0.2, 8], [0, 4.1, 0]));
    scene.add(box(plaster, [10, 4, 0.2], [0, 2, -4.1]));
    scene.add(box(plaster, [10, 4, 0.2], [0, 2, 4.1]));
    for (const s of [-1, 1]) scene.add(box(plaster, [0.2, 4, 8], [s * 5.1, 2, 0]));
    // Things to cast and take shadows: a table and a cabinet.
    scene.add(box(wood, [1.6, 0.08, 0.9], [1.2, 0.75, -1.2]));
    for (const [x, z] of [[0.5, -1.55], [1.9, -1.55], [0.5, -0.85], [1.9, -0.85]]) scene.add(box(wood, [0.07, 0.75, 0.07], [x, 0.375, z]));
    scene.add(box(wood, [0.9, 1.8, 0.5], [-2.2, 0.9, -3.7]));

    const base = new PointLight(0xffe2c0, 250, 0);
    base.position.set(-3, 3.6, 0.5);
    base.radius = 0.08;
    scene.add(base);

    const lamp = new SpotLight(0xfff4e0, 3000, 0, Math.PI / 7, 0.3);
    lamp.name = "lamp";
    lamp.position.set(3, 3.7, 0.5);
    lamp.target!.position.set(3, 1.6, -4);
    lamp.radius = 0.05;
    scene.add(lamp, lamp.target!);

    const neon = new MeshStandardMaterial({ name: "neon", color: 0x000000, emissive: new Color().setRGB(1, 0.2, 0.55), emissiveIntensity: 400 });
    scene.add(box(neon, [0.05, 1.2, 2], [-4.95, 1.6, -1], "sign"));
    return { scene, camera, options: { name: "light-edit", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  frame(f) {
    f.object("lamp")!.power = f.state.lamp ? 1 : 0;
    f.object("sign")!.power = f.state.sign ? 1 : 0;
  },
  handlers: {
    lamp(tx) {
      tx.state.lamp = !tx.state.lamp;
    },
    sign(tx) {
      tx.state.sign = !tx.state.sign;
    },
  },
});
