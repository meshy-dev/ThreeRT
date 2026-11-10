// External GLB asset (textured PBR: base color, ORM, normal, emissive) on a pedestal under the sky.
import { Scene, PerspectiveCamera, Mesh, CylinderGeometry, PlaneGeometry, MeshStandardMaterial, DirectionalLight, SkyAtmosphere, GLTFModel, checkerTexture } from "@threert/scene";

export default function build() {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 80000);
  sun.name = "sun";
  sun.setSunAngles(40, -40);
  scene.add(sun);
  scene.environment = new SkyAtmosphere({ sun });

  const checker = checkerTexture(0xbdbdbd, 0x5a5a5a, 16);
  checker.repeat = [8, 8];
  const ground = new Mesh(new PlaneGeometry(30, 30), new MeshStandardMaterial({ name: "checker_floor", map: checker, roughness: 0.7 }));
  ground.name = "ground";
  ground.rotation.x = -Math.PI / 2;
  scene.add(ground);
  const pedestal = new Mesh(new CylinderGeometry(0.75, 0.85, 0.8, 96), new MeshStandardMaterial({ name: "marble", color: 0xe9e6e1, roughness: 0.25 }));
  pedestal.name = "pedestal";
  pedestal.position.set(0, 0.4, 0);
  scene.add(pedestal);

  const helmet = new GLTFModel("assets/DamagedHelmet.glb");
  helmet.name = "helmet";
  helmet.position.set(0, 1.55, 0);
  helmet.rotation.y = 0.5;
  helmet.scale.set(0.7, 0.7, 0.7);
  scene.add(helmet);

  const camera = new PerspectiveCamera(35, 16 / 9, 0.05, 200);
  camera.name = "camera";
  camera.position.set(1.6, 2.0, 3.6);
  camera.lookAt(0, 1.35, 0);
  camera.setManualExposure(14.6);
  scene.add(camera);
  return { scene, camera, options: { name: "glb-asset" } };
}
