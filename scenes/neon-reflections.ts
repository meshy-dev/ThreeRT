// Reflections + post coverage: glossy floor, chrome/gold, emissive neon (bloom), spot lights, low key exposure.
import { Scene, PerspectiveCamera, Mesh, BoxGeometry, SphereGeometry, PlaneGeometry, TorusKnotGeometry, CylinderGeometry, MeshStandardMaterial, MeshPhysicalMaterial, SpotLight, SkyColor, Color } from "@threert/scene";

export default function build() {
  const scene = new Scene();
  scene.environment = new SkyColor(new Color().setRGB(0.05, 0.08, 0.2), 0.6);

  const floor = new Mesh(new PlaneGeometry(40, 40), new MeshPhysicalMaterial({ name: "wet_floor", color: 0x0c0c0e, roughness: 0.06, metalness: 0 }));
  floor.name = "floor";
  floor.rotation.x = -Math.PI / 2;
  scene.add(floor);
  const wall = new Mesh(new BoxGeometry(14, 6, 0.4), new MeshStandardMaterial({ name: "concrete", color: 0x3a3a3c, roughness: 0.8 }));
  wall.name = "back_wall";
  wall.position.set(0, 3, -3.2);
  scene.add(wall);

  const chrome = new Mesh(new SphereGeometry(0.75, 128, 96), new MeshStandardMaterial({ name: "chrome", color: 0xf4f4f4, metalness: 1, roughness: 0.03 }));
  chrome.name = "chrome_ball";
  chrome.position.set(-1.6, 0.75, -0.4);
  scene.add(chrome);
  const knot = new Mesh(new TorusKnotGeometry(0.55, 0.18, 384, 48), new MeshStandardMaterial({ name: "gold", color: new Color().setRGB(1.0, 0.766, 0.336), metalness: 1, roughness: 0.22 }));
  knot.name = "gold_knot";
  knot.position.set(0.6, 1.0, -0.8);
  knot.rotation.set(0.3, 0.5, 0);
  scene.add(knot);
  const block = new Mesh(new BoxGeometry(0.8, 0.8, 0.8), new MeshStandardMaterial({ name: "copper", color: new Color().setRGB(0.955, 0.637, 0.538), metalness: 1, roughness: 0.45 }));
  block.name = "copper_block";
  block.position.set(2.3, 0.4, 0.2);
  block.rotation.y = 0.6;
  scene.add(block);

  const neon = (name: string, color: Color, x: number, y: number, len: number, rotZ: number) => {
    const m = new Mesh(new CylinderGeometry(0.035, 0.035, len, 24), new MeshStandardMaterial({ name: `neon_${name}`, color: 0x000000, emissive: color, emissiveIntensity: 1800 }));
    m.name = `neon_${name}`;
    m.position.set(x, y, -2.95);
    m.rotation.z = rotZ;
    m.castShadow = false;
    scene.add(m);
  };
  neon("pink", new Color().setRGB(1.0, 0.08, 0.55), -2.2, 2.4, 2.6, Math.PI / 2);
  neon("cyan", new Color().setRGB(0.05, 0.75, 1.0), 1.8, 2.9, 2.0, Math.PI / 2);
  neon("amber", new Color().setRGB(1.0, 0.45, 0.05), 3.3, 1.6, 1.8, 0);

  const key = new SpotLight(new Color().setRGB(1.0, 0.9, 0.8), 2500, 0, 0.45, 0.35);
  key.name = "key_spot";
  key.position.set(2.5, 4.5, 3.0);
  key.target!.position.set(0, 0.6, -0.5);
  scene.add(key);
  const rim = new SpotLight(new Color().setRGB(0.5, 0.6, 1.0), 1500, 0, 0.6, 0.5);
  rim.name = "rim_spot";
  rim.position.set(-3.5, 3.5, -1.5);
  rim.target!.position.set(-0.5, 0.5, -0.5);
  scene.add(rim);

  const camera = new PerspectiveCamera(40, 16 / 9, 0.05, 200);
  camera.name = "camera";
  camera.position.set(0.3, 1.5, 5.6);
  camera.lookAt(0.2, 0.9, -0.6);
  camera.setManualExposure(6.0);
  scene.add(camera);
  return { scene, camera, options: { name: "neon-reflections" } };
}
