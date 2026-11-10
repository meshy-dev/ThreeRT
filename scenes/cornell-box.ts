// GI coverage: color bleeding, area-light soft shadows, contact occlusion.
import { Scene, PerspectiveCamera, Mesh, BoxGeometry, MeshStandardMaterial, RectAreaLight, SkyColor, Color } from "@threert/scene";

export default function build() {
  const scene = new Scene();
  scene.environment = new SkyColor(0x000000, 0);
  const white = new MeshStandardMaterial({ name: "white", color: new Color().setRGB(0.73, 0.73, 0.73), roughness: 0.9 });
  const red = new MeshStandardMaterial({ name: "red", color: new Color().setRGB(0.63, 0.065, 0.05), roughness: 0.9 });
  const green = new MeshStandardMaterial({ name: "green", color: new Color().setRGB(0.14, 0.45, 0.091), roughness: 0.9 });
  const t = 0.2; // wall thickness (keeps Lumen leak-free)
  const wall = (name: string, w: number, h: number, d: number, x: number, y: number, z: number, mat: MeshStandardMaterial) => {
    const m = new Mesh(new BoxGeometry(w, h, d), mat);
    m.name = name;
    m.position.set(x, y, z);
    scene.add(m);
    return m;
  };
  wall("floor", 2 + 2 * t, t, 2 + t, 0, -t / 2, -t / 2, white);
  wall("ceiling", 2 + 2 * t, t, 2 + t, 0, 2 + t / 2, -t / 2, white);
  wall("back", 2 + 2 * t, 2, t, 0, 1, -1 - t / 2, white);
  wall("left", t, 2, 2, -1 - t / 2, 1, 0, red);
  wall("right", t, 2, 2, 1 + t / 2, 1, 0, green);
  const tall = wall("tall_box", 0.6, 1.2, 0.6, -0.33, 0.6, -0.28, white);
  tall.rotation.y = 0.28;
  const short = wall("short_box", 0.6, 0.6, 0.6, 0.35, 0.3, 0.32, white);
  short.rotation.y = -0.3;

  const light = new RectAreaLight(new Color().setRGB(1.0, 0.85, 0.65), 18000, 0.5, 0.4);
  light.name = "ceiling_light";
  light.position.set(0, 1.995, 0);
  light.lookAt(0, 0, 0);
  scene.add(light);

  const camera = new PerspectiveCamera(39, 16 / 9, 0.05, 100);
  camera.name = "camera";
  camera.position.set(0, 1, 3.9);
  camera.lookAt(0, 1, 0);
  camera.setManualExposure(8.0);
  scene.add(camera);
  return { scene, camera, options: { name: "cornell-box" } };
}
