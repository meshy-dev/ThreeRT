// A hand-held device dragged over a workbench (Pocket Bench's Tab5, made of boxes): a white case with an emissive
// screen in front of a wall, over a green cutting mat with a white grid. The driver (bench-object-ghosting.ts) moves
// it with `{t: "pose", x, y, z, ry}` host messages, one per frame, as an app's drag would.
import {
  BoxGeometry, DataTexture, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, RectAreaLight, Scene, SkyGradient,
} from "@threert/scene";
import { defineGame, host } from "@threert/gameplay";

export const CAMERA = { position: [0, 0.34, 0.78] as const, target: [0, 0.2, 0] as const, fov: 40 };

function matTexture(): DataTexture {
  const n = 512, px = new Uint8Array(n * n * 4);
  for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
    const line = x % 64 < 2 || y % 64 < 2, fine = x % 16 === 0 || y % 16 === 0;
    const c = line ? [210, 230, 215] : fine ? [40, 120, 80] : [24, 104, 66];
    px.set([...c, 255], (y * n + x) * 4);
  }
  const t = new DataTexture(n, n, px);
  t.repeat = [2, 2];
  return t;
}

function screenTexture(): DataTexture {
  const w = 320, h = 180, px = new Uint8Array(w * h * 4);
  for (let y = 0; y < h; y++) for (let x = 0; x < w; x++) {
    const frame = x > 120 && x < 200 && y > 50 && y < 110 && !(x > 126 && x < 194 && y > 56 && y < 104);
    const text = y > 128 && y < 140 && x > 90 && x < 230 && (x % 9 < 6);
    const c = frame ? [60, 220, 140] : text ? [230, 235, 240] : [16, 26, 38];
    px.set([...c, 255], (y * w + x) * 4);
  }
  return new DataTexture(w, h, px);
}

let device: Mesh;

host.on("pose", (m) => {
  device.position.set(Number(m.x ?? 0), Number(m.y ?? 0.2), Number(m.z ?? 0));
  device.rotation.y = Number(m.ry ?? 0);
});

export default defineGame({
  state: {},
  handlers: {},
  setup: () => {
    const scene = new Scene();
    scene.environment = new SkyGradient({ zenith: 0x9aa6b0, horizon: 0x6c7076, ground: 0x303030, intensity: 0.4 });
    const panel = new RectAreaLight(0xfff4e6, 9000, 1.2, 0.6);
    panel.position.set(0, 1.4, 0.3);
    panel.lookAt(0, 0, 0);
    scene.add(panel);

    const mat = new Mesh(new PlaneGeometry(2.4, 2.4), new MeshStandardMaterial({ map: matTexture(), roughness: 0.55 }));
    mat.rotation.x = -Math.PI / 2;
    scene.add(mat);
    const wall = new Mesh(new BoxGeometry(2.4, 1.6, 0.05), new MeshStandardMaterial({ color: 0x7a7f72, roughness: 0.92 }));
    wall.position.set(0, 0.8, -0.5);
    scene.add(wall);

    device = new Mesh(new BoxGeometry(0.25, 0.155, 0.022), new MeshStandardMaterial({ color: 0xf2f2ee, roughness: 0.45 }));
    device.position.set(0, 0.2, 0);
    const screen = new Mesh(new PlaneGeometry(0.21, 0.118), new MeshStandardMaterial({
      color: 0x000000, roughness: 0.08, emissive: 0xffffff, emissiveMap: screenTexture(), emissiveIntensity: 80,
    }));
    screen.position.z = 0.0115;
    device.add(screen);
    scene.add(device);

    const camera = new PerspectiveCamera(CAMERA.fov, 16 / 9, 0.02, 20);
    camera.position.set(...CAMERA.position);
    camera.lookAt(...CAMERA.target);
    camera.setManualExposure(8);
    scene.add(camera);
    return { scene, camera, options: { name: "object-ghosting-device", width: 1280, height: 720 } };
  },
});
