// Layer B of the render suite (docs/ROADMAP.md § 3): a Khronos glTF Sample Asset in a neutral daylight studio that
// three.js, UE and ThreeRT all render the same way (sun + sky atmosphere, grey ground, manual exposure; the light of
// scenes/mannequin.ts). The model stands on the ground, faced from +Z (glTF front). Assets: node tools/fetch-khronos.ts.
import { DirectionalLight, GLTFModel, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, Scene, SkyAtmosphere } from "@threert/scene";

/** A model, its world bounds (tools: GLB node hierarchy x accessor bounds), the camera distance from its front. */
export interface Exhibit {
  name: string;
  file: string;
  /** What it tests (the suite's feature x pass-rate table). */
  features: string[];
  min: [number, number, number];
  max: [number, number, number];
  distance: number;
}

export const SUN = { elevation: 38, azimuth: -35, lux: 80000 };
export const EV100 = 14.6;
export const FOV = 35;

/** Where the camera stands and looks for an exhibit lifted onto the ground. */
export function framing(e: Exhibit) {
  const lift = -e.min[1];
  const c = [(e.min[0] + e.max[0]) / 2, (e.min[1] + e.max[1]) / 2 + lift, (e.min[2] + e.max[2]) / 2];
  return { lift, target: c as [number, number, number], eye: [c[0], c[1] + 0.12 * e.distance, e.max[2] + e.distance] as [number, number, number] };
}

export function studio(e: Exhibit) {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, SUN.lux);
  sun.setSunAngles(SUN.elevation, SUN.azimuth);
  scene.add(sun);
  scene.environment = new SkyAtmosphere({ sun });
  // 2 km like the plaza's: the atmosphere below the horizon would show past a smaller one.
  const ground = new Mesh(new PlaneGeometry(2000, 2000), new MeshStandardMaterial({ name: "ground", color: [0.35, 0.35, 0.35], roughness: 0.6 }));
  ground.name = "ground";
  ground.rotation.x = -Math.PI / 2;
  scene.add(ground);

  const f = framing(e);
  const model = new GLTFModel(`assets/khronos/${e.file}`);
  model.name = e.name;
  model.position.set(0, f.lift, 0);
  scene.add(model);

  const camera = new PerspectiveCamera(FOV, 16 / 9, 0.05, 500);
  camera.name = "camera";
  camera.position.set(...f.eye);
  camera.lookAt(...f.target);
  camera.setManualExposure(EV100);
  scene.add(camera);
  return { scene, camera, options: { name: e.name, width: 1280, height: 720 } };
}
