// Instanced cutout props for tools/check-instancing.ts, which writes the model first (out/check-instancing/fence.glb:
// a 1 m quad of thin bars cut by an alpha mask, 2 texels of every 8, so a coarser mip of the mask averages below the
// cutoff and the bars vanish). Two fences at scale 1, 2.5 m from the camera (left), and two at scale 5, five times as
// far (right): the same size on screen, so the alpha test should pick the same mip for all four. A uniform sky behind.
import { GLTFModel, PerspectiveCamera, Scene, SkyColor } from "@threert/scene";
import { lambert, plane } from "../render/lib.ts";

export default function () {
  const scene = new Scene();
  scene.environment = new SkyColor(0xffffff, 1);
  const fence = (x: number, z: number, s: number) => {
    const m = new GLTFModel("out/check-instancing/fence.glb");
    m.name = `fence_${x}_${z}`;
    m.position.set(x, 1.5 - 0.5 * s, z);
    m.scale.set(s, s, s);
    return m;
  };
  const floor = plane(400, lambert(0.2), "floor");
  floor.position.y = -1.5;
  scene.add(floor, fence(-1.45, -2.5, 1), fence(-0.4, -2.5, 1), fence(2.0, -12.5, 5), fence(7.25, -12.5, 5));
  const camera = new PerspectiveCamera(50, 16 / 9, 0.05, 500);
  camera.position.set(0, 1.5, 0);
  camera.lookAt(0, 1.5, -3);
  camera.setManualExposure(0);
  scene.add(camera);
  return { scene, camera, options: { name: "instanced-cutouts", width: 960, height: 540, preset: "cinematic" as const } };
}
