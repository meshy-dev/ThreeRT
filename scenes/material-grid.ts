// PBR coverage: dielectric / colored / metal rows x roughness sweep, sun + physical sky, soft sun shadows.
import { Scene, PerspectiveCamera, Mesh, SphereGeometry, PlaneGeometry, MeshStandardMaterial, DirectionalLight, SkyAtmosphere, Color } from "@threert/scene";

export default function build() {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 80000);
  sun.name = "sun";
  sun.setSunAngles(32, 35);
  scene.add(sun);
  scene.environment = new SkyAtmosphere({ sun });

  const ground = new Mesh(new PlaneGeometry(60, 60), new MeshStandardMaterial({ name: "ground", color: 0x7a7a7a, roughness: 0.85 }));
  ground.name = "ground";
  ground.rotation.x = -Math.PI / 2;
  scene.add(ground);

  const rows = [
    { name: "white", color: new Color(0xe8e8e8), metalness: 0 },
    { name: "red", color: new Color(0xc81e1e), metalness: 0 },
    { name: "gold", color: new Color().setRGB(1.0, 0.766, 0.336), metalness: 1 },
  ];
  const roughness = [0.05, 0.2, 0.35, 0.5, 0.7, 0.9];
  const sphere = new SphereGeometry(0.42, 96, 64);
  rows.forEach((row, r) => {
    roughness.forEach((rough, c) => {
      const m = new Mesh(sphere, new MeshStandardMaterial({ name: `${row.name}_r${Math.round(rough * 100)}`, color: row.color, metalness: row.metalness, roughness: rough }));
      m.name = `ball_${row.name}_${c}`;
      m.position.set((c - 2.5) * 1.05, 0.42, (r - 1) * 1.15);
      scene.add(m);
    });
  });

  const camera = new PerspectiveCamera(36, 16 / 9, 0.1, 500);
  camera.name = "camera";
  camera.position.set(0, 3.4, 7.6);
  camera.lookAt(0, 0.25, 0.1);
  camera.setManualExposure(14.6);
  scene.add(camera);
  return { scene, camera, options: { name: "material-grid" } };
}
