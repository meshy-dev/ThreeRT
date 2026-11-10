// The benchmark level by day as the renderer's native voxel objects on the mesh route (renderer/src/voxel.rs), its
// palette as one mesh per entry ("material") or one per class of entries alike in all but their colour, each vertex its
// voxel's colour ("vertex", F_VCOLOR): the comparison of docs/VOXEL.md, "Measured: vertex colours"
// (tools/bench-voxel.ts --colors). Sun and sky, the camera at the camera path's start (a benchmark drives it).
//   node tools/voxel-level/generate.ts --granularity object && node tools/bundle-game.ts games/voxel-level/level-native.game.ts
import { DirectionalLight, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, Scene, SkyAtmosphere } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { voxelLevel } from "@threert/voxel";
import { nativeObjects, nativePalette } from "./native.ts";
import { EDGE, PALETTE, SIZE, VOXEL, cameraPath } from "./plan.ts";

/** `occupancy`: the cell size of the voxel occupancy grid for GPU particles (`NativeLevel.occupancy`; default none). */
export function nativeLevelGame(colors: "material" | "vertex", seed = 1, occupancy = 0) {
  const key = cameraPath(seed)[0];
  return defineGame<Record<string, never>>({
    state: {},
    setup: () => {
      const scene = new Scene();
      const sun = new DirectionalLight(0xffffff, 90000);
      sun.setSunAngles(38, 300);
      scene.add(sun);
      scene.environment = new SkyAtmosphere({ sun });
      const camera = new PerspectiveCamera(55, 16 / 9, 0.05, 2000);
      camera.name = "camera";
      camera.position.set(...key.pos);
      camera.lookAt(...key.target);
      camera.setManualExposure(13);
      scene.add(camera);
      const ground = new MeshStandardMaterial({ name: "ground", color: PALETTE[1][1], roughness: 0.9 });
      const half = (SIZE / 2) * VOXEL, far = 100000;
      for (const [w, d, x, z] of [[2 * far, far - half, 0, -(far + half) / 2], [2 * far, far - half, 0, (far + half) / 2], [far - half, 2 * half, -(far + half) / 2, 0], [far - half, 2 * half, (far + half) / 2, 0]]) {
        const strip = new Mesh(new PlaneGeometry(w, d), ground);
        strip.rotation.x = -Math.PI / 2;
        strip.position.set(x, EDGE * VOXEL, z);
        scene.add(strip);
      }
      voxelLevel({ route: "mesh", palette: nativePalette(false), objects: nativeObjects(seed), colors, ...(occupancy > 0 ? { occupancy } : {}) });
      return { scene, camera, options: { name: `voxel-level-native-${colors}`, width: 1280, height: 720, preset: "cinematic" as const } };
    },
    handlers: {},
  });
}

export default nativeLevelGame("material");
