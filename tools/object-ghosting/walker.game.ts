// A round character running over a sunlit plank floor, seen from above and behind as a third-person game sees it
// (Mochi Party, made on threert/native-app-template): its hard sun shadow sweeps over the floor, its body over the
// planks. The driver (bench-object-ghosting.ts) moves it with `{t: "pose", x, y, z}` host messages, one per frame.
import { BoxGeometry, DataTexture, DirectionalLight, Group, Mesh, MeshPhysicalMaterial, MeshStandardMaterial, PerspectiveCamera, Scene, SkyAtmosphere, SphereGeometry } from "@threert/scene";
import { defineGame, host } from "@threert/gameplay";

let walker: Group;

host.on("pose", (m) => {
  walker.position.set(Number(m.x ?? 0), Number(m.y ?? 0), Number(m.z ?? 0));
});

function planks(): DataTexture {
  const w = 256, h = 256, px = new Uint8Array(w * h * 4);
  for (let y = 0; y < h; y++) {
    const row = Math.floor(y / 32), seam = y % 32 < 2, shift = (row * 97) % w;
    for (let x = 0; x < w; x++) {
      const xx = (x + shift) % w, joint = xx % 128 < 2;
      const k = seam || joint ? 0.72 : 0.92 + 0.08 * Math.sin(row * 3.1 + xx * 0.05);
      px.set([214 * k, 160 * k, 108 * k, 255], (y * w + x) * 4);
    }
  }
  const t = new DataTexture(w, h, px);
  t.repeat = [4, 4];
  return t;
}

export default defineGame({
  state: {},
  handlers: {},
  setup: () => {
    const scene = new Scene();
    const sun = new DirectionalLight(0xfff1dc, 80000);
    sun.setSunAngles(38, 205);
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun });
    const floor = new Mesh(new BoxGeometry(10, 0.1, 8), new MeshStandardMaterial({ map: planks(), roughness: 0.55 }));
    floor.position.y = -0.05;
    scene.add(floor);
    const wall = new Mesh(new BoxGeometry(10, 2.6, 0.2), new MeshStandardMaterial({ color: 0xf2dcc4, roughness: 0.9 }));
    wall.position.set(0, 1.3, -4);
    scene.add(wall);
    const rug = new Mesh(new BoxGeometry(2.6, 0.012, 2.6), new MeshStandardMaterial({ color: 0x9be8c4, roughness: 0.95 }));
    rug.position.set(0.6, 0.006, 0.4);
    scene.add(rug);
    walker = new Group();
    const skin = new MeshPhysicalMaterial({ color: 0xff8fb1, roughness: 0.35, clearcoat: 0.8, clearcoatRoughness: 0.15 });
    const body = new Mesh(new SphereGeometry(0.35, 32, 20), skin);
    body.position.y = 0.34;
    walker.add(body);
    for (const s of [-1, 1]) {
      const foot = new Mesh(new SphereGeometry(0.095, 16, 10), skin);
      foot.position.set(s * 0.15, 0.04, 0.05);
      walker.add(foot);
    }
    walker.position.set(-2, 0, 0.6);
    scene.add(walker);
    const camera = new PerspectiveCamera(48, 16 / 9, 0.05, 100);
    camera.position.set(0, 4.0, 5.9);
    camera.lookAt(0, 0.3, 0.3);
    camera.setManualExposure(13.4);
    scene.add(camera);
    return { scene, camera, options: { name: "object-ghosting-walker", width: 1280, height: 720 } };
  },
});
