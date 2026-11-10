// Beeple's Zero-Day, Measure One (ORCA, CC-BY 4.0; bun scripts/orca.ts), static: the opening room lit only by its
// ~10 k emissive triangles (light strips, accents, the tower).
//
// The emitters carry Falcor's texture values (x 1); `power` scales them to nits, as in scenes/bistro.ts. No sky: the
// room is closed. The camera is the FBX camera (DynamicCamera2) at the start of its move; the scene holds the first
// frame of the animation (ROADMAP § 0 step 8 plays it).
// Credits: Mike Winkelmann (Beeple) and Kate Anderson (2019), Open Research Content Archive (ORCA),
// https://developer.nvidia.com/orca/beeple-zero-day
import { GLTFModel, PerspectiveCamera, Scene, SkyColor } from "@threert/scene";

export const ZERO_DAY = "assets/orca/zeroday/MEASURE_ONE.gltf";
/** Emitter scale: Falcor's emissive texture values (x 1) -> nits. */
export const EMISSIVE_POWER = 1000;
/** The FBX camera at the start of its move (glTF: Y up, metres); vertical field of view (degrees). */
export const SHOT = { position: [13.351, -0.356, -0.013] as const, target: [3.355, -0.205, -0.257] as const, fov: 33.97 };
/** Manual exposure (EV100). */
export const EV100 = 3;

export default function build() {
  const scene = new Scene();
  scene.environment = new SkyColor(0x000000, 0);
  const room = new GLTFModel(ZERO_DAY);
  room.name = "zero_day";
  room.power = EMISSIVE_POWER;
  scene.add(room);

  const camera = new PerspectiveCamera(SHOT.fov, 16 / 9, 0.05, 1000);
  camera.name = "camera";
  camera.position.set(...SHOT.position);
  camera.lookAt(...SHOT.target);
  camera.setManualExposure(EV100);
  scene.add(camera);
  return { scene, camera, options: { name: "zero-day", width: 1280, height: 720 } };
}
