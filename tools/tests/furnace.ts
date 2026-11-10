// White furnace: F0=1 rough metal spheres under a uniform 1-nit sky must reflect ~1 everywhere (energy conservation).
//   node tools/bundle-game.ts tools/tests/furnace.ts && renderer/target/release/threert-render render out/bundles/furnace.js -o out/tests/furnace.png
import { Scene, PerspectiveCamera, Mesh, SphereGeometry, MeshStandardMaterial, SkyColor } from "@threert/scene";

export default function build() {
  const scene = new Scene();
  scene.environment = new SkyColor(0xffffff, 1);
  [0.1, 0.5, 0.9].forEach((r, i) => {
    const m = new Mesh(new SphereGeometry(0.45, 128, 96), new MeshStandardMaterial({ color: [1, 1, 1], metalness: 1, roughness: r }));
    m.position.set((i - 1) * 1.1, 0, 0);
    scene.add(m);
  });
  const camera = new PerspectiveCamera(30, 2, 0.1, 100);
  camera.position.set(0, 0, 4.5);
  camera.setManualExposure(0);
  scene.add(camera);
  return { scene, camera, options: { name: "furnace", width: 600, height: 300, overrides: { post: { bloom: { intensity: 0 }, vignette: 0 } } } };
}
