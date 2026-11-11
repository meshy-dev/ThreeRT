// Hello world: a living room lit by the afternoon sun through a window, the sky, and a floor lamp.
import { Scene, PerspectiveCamera, Mesh, BoxGeometry, SphereGeometry, CylinderGeometry, MeshStandardMaterial, DirectionalLight, SkyAtmosphere, Color } from "@threert/scene";

export default function build() {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 90000).setSunAngles(28, 100);
  scene.add(sun);
  scene.environment = new SkyAtmosphere({ sun });

  const plaster = new MeshStandardMaterial({ color: 0xd9d4cc, roughness: 0.92 });
  const oak = new MeshStandardMaterial({ color: 0x8c6239, roughness: 0.55 });
  const fabric = new MeshStandardMaterial({ color: 0x2e4a7a, roughness: 0.95 });
  const ceramic = new MeshStandardMaterial({ color: 0xf2efe8, roughness: 0.15 });
  const brass = new MeshStandardMaterial({ color: new Color().setRGB(0.91, 0.78, 0.42), metalness: 1, roughness: 0.3 });
  const lampShade = new MeshStandardMaterial({ color: 0xfff1dc, emissive: new Color().setRGB(1.0, 0.78, 0.5), emissiveIntensity: 2500, roughness: 0.9 });
  lampShade.side = "double";

  const add = (mesh: Mesh, x: number, y: number, z: number) => (mesh.position.set(x, y, z), scene.add(mesh));
  const box = (w: number, h: number, d: number, x: number, y: number, z: number, mat: MeshStandardMaterial) => add(new Mesh(new BoxGeometry(w, h, d), mat), x, y, z);
  // 6 x 3 x 6 m room, 0.3 m walls; a 2.2 x 1.6 m window in the +X wall (sill 0.8 m)
  box(6.6, 0.3, 6.6, 0, -0.15, 0, oak);
  box(6.6, 0.3, 6.6, 0, 3.15, 0, plaster);
  box(6.6, 3, 0.3, 0, 1.5, -3.15, plaster);
  box(6.6, 3, 0.3, 0, 1.5, 3.15, plaster);
  box(0.3, 3, 6, -3.15, 1.5, 0, plaster);
  box(0.3, 3, 1.3, 3.15, 1.5, -2.35, plaster);
  box(0.3, 3, 2.5, 3.15, 1.5, 1.75, plaster);
  box(0.3, 0.8, 2.2, 3.15, 0.4, -0.6, plaster);
  box(0.3, 0.6, 2.2, 3.15, 2.7, -0.6, plaster);
  // sofa, table, vase, floor lamp
  box(2.2, 0.45, 0.9, -1.4, 0.225, -2.2, fabric);
  box(2.2, 0.5, 0.25, -1.4, 0.7, -2.55, fabric);
  box(1.2, 0.06, 0.7, 0.2, 0.45, -0.9, oak);
  for (const [x, z] of [[-0.3, -1.15], [0.7, -1.15], [-0.3, -0.65], [0.7, -0.65]]) box(0.05, 0.42, 0.05, x, 0.21, z, oak);
  add(new Mesh(new SphereGeometry(0.16, 64, 48), ceramic), 0, 0.64, -0.9);
  add(new Mesh(new CylinderGeometry(0.02, 0.02, 1.5, 24), brass), -2.6, 0.75, -2.5);
  add(new Mesh(new CylinderGeometry(0.16, 0.22, 0.3, 48, 1, true), lampShade), -2.6, 1.6, -2.5);

  const camera = new PerspectiveCamera(58, 16 / 9, 0.05, 200);
  camera.position.set(-2.3, 1.45, 2.4);
  camera.lookAt(1.0, 0.9, -1.2);
  scene.add(camera);
  return { scene, camera };
}
