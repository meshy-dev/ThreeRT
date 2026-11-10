// Calibration scene for sky.ts SKY_NITS_PER_LUX (`node tools/bench-three.ts --calibrate-sky`): a 1 lux sun at 30
// degrees elevation over a SkyAtmosphere and a white Lambertian square (no specular) facing up, the sun itself blocked
// by a small black card 50 m away (2.3 degrees), the camera looking down at the square. The native renderer's raw
// radiance of the square is albedo / pi x the sky's horizontal irradiance per lux of sun.
import { BoxGeometry, DirectionalLight, Mesh, MeshPhysicalMaterial, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, Scene, SkyAtmosphere } from "@threert/scene";

export const ELEVATION = 30;

export default function build() {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 1);
  sun.name = "sun";
  sun.setSunAngles(ELEVATION, 90);
  scene.add(sun);
  scene.environment = new SkyAtmosphere({ sun });
  const card = new Mesh(new PlaneGeometry(1, 1), new MeshPhysicalMaterial({ name: "white", color: 0xffffff, roughness: 1, specularIntensity: 0 }));
  card.rotation.x = -Math.PI / 2;
  scene.add(card);
  const e = (ELEVATION * Math.PI) / 180;
  const blocker = new Mesh(new BoxGeometry(2, 2, 2), new MeshStandardMaterial({ name: "black", color: 0x000000 }));
  blocker.position.set(50 * Math.cos(e), 50 * Math.sin(e), 0);
  scene.add(blocker);
  const camera = new PerspectiveCamera(5, 1, 0.1, 100);
  camera.name = "camera";
  camera.position.set(0, 1, 0);
  camera.lookAt(0, 0, 0.001);
  camera.setManualExposure(0);
  scene.add(camera);
  return { scene, camera, options: { name: "calib-sky", width: 64, height: 64 } };
}
