// Hello world: PBR balls (dielectric, red, gold x roughness) on the ground, under the sun and the sky.
import { Scene, PerspectiveCamera, Mesh, SphereGeometry, PlaneGeometry, MeshStandardMaterial, DirectionalLight, SkyAtmosphere, Color } from "@threert/scene";

export default function build() {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 80000).setSunAngles(32, 35);
  scene.add(sun);
  scene.environment = new SkyAtmosphere({ sun });

  const ground = new Mesh(new PlaneGeometry(60, 60), new MeshStandardMaterial({ color: 0x7a7a7a, roughness: 0.85 }));
  ground.rotation.x = -Math.PI / 2;
  scene.add(ground);

  const rows = [{ color: new Color(0xe8e8e8), metalness: 0 }, { color: new Color(0xc81e1e), metalness: 0 }, { color: new Color().setRGB(1.0, 0.766, 0.336), metalness: 1 }];
  const sphere = new SphereGeometry(0.42, 96, 64);
  rows.forEach((row, r) => {
    [0.05, 0.2, 0.35, 0.5, 0.7, 0.9].forEach((roughness, c) => {
      const ball = new Mesh(sphere, new MeshStandardMaterial({ color: row.color, metalness: row.metalness, roughness }));
      ball.position.set((c - 2.5) * 1.05, 0.42, (r - 1) * 1.15);
      scene.add(ball);
    });
  });

  const camera = new PerspectiveCamera(36, 16 / 9, 0.1, 500);
  camera.position.set(0, 3.4, 7.6);
  camera.lookAt(0, 0.25, 0.1);
  scene.add(camera);
  return { scene, camera };
}
