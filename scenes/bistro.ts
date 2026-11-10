// Amazon Lumberyard Bistro at night (ORCA, CC-BY 4.0; bun scripts/orca.ts): the exterior lit only by its emitters.
//
// Bistro becomes a night scene "when rendered without analytical lighting or environment map" (ORCA's README): the
// FBX's sun is dropped in the conversion and the sky is an urban night sky glow, so the street lamps, the string lights
// and the shop signs light the street. The emitters carry Falcor's texture values (x 1); `power` scales them to nits:
// at 1000 the street lamp globes are ~420 nits, ~370 lm each (a 40 W-class lamp).
// The camera is the FBX camera's first pose (the start of its path; Falcor's default view down the street).
// Credits: Amazon Lumberyard (2017), Open Research Content Archive (ORCA),
// http://developer.nvidia.com/orca/amazon-lumberyard-bistro
import { Color, GLTFModel, PerspectiveCamera, Scene, SkyColor } from "@threert/scene";

export const BISTRO = "assets/orca/bistro/BistroExterior.gltf";
/** Emitter scale: Falcor's emissive texture values (x 1) -> nits. */
export const EMISSIVE_POWER = 1000;
/** The FBX camera at the start of its path (glTF: Y up, metres); vertical field of view (degrees). */
export const STREET = { position: [24.823, 3.161, -61.648] as const, target: [21.639, 2.578, -52.187] as const, fov: 33.97 };
/** Manual exposure (EV100) of the night views. */
export const EV100 = 2;

export default function build() {
  const scene = new Scene();
  // Urban night sky glow: ~0.02 nits, blue.
  scene.environment = new SkyColor(new Color().setRGB(0.35, 0.45, 0.8), 0.04);
  const bistro = new GLTFModel(BISTRO);
  bistro.name = "bistro";
  bistro.power = EMISSIVE_POWER;
  scene.add(bistro);

  const camera = new PerspectiveCamera(STREET.fov, 16 / 9, 0.05, 1000);
  camera.name = "camera";
  camera.position.set(...STREET.position);
  camera.lookAt(...STREET.target);
  camera.setManualExposure(EV100);
  scene.add(camera);
  return { scene, camera, options: { name: "bistro", width: 1280, height: 720 } };
}
