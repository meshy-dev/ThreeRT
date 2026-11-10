// Skinned characters: four UE-style mannequins (Quaternius Universal Animation Library, CC0; UE5 mannequin bone names)
// posed from their clips, on a UE default-level setup (grey ground, a few template blocks, sky atmosphere + sun).
// Assets: node tools/fetch-quaternius.ts.
import { BoxGeometry, DirectionalLight, GLTFModel, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, Scene, SkyAtmosphere } from "@threert/scene";

export const MANNEQUIN = "assets/quaternius/ual/UAL1_Standard.glb";

/** The poses of the still (clip, time in s, position x / z, facing in degrees about +Y). */
export const POSES = [
  { id: "idle", clip: "Idle_Loop", time: 0.8, x: -1.65, z: 0.2, yaw: 20 },
  { id: "walk", clip: "Walk_Loop", time: 0.45, x: -0.55, z: -0.1, yaw: 35 },
  { id: "jog", clip: "Jog_Fwd_Loop", time: 0.3, x: 0.55, z: 0.15, yaw: -30 },
  { id: "dance", clip: "Dance_Loop", time: 0.5, x: 1.65, z: -0.15, yaw: -15 },
] as const;

export default function build() {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 80000);
  sun.setSunAngles(38, -35);
  scene.add(sun);
  scene.environment = new SkyAtmosphere({ sun });

  const ground = new Mesh(new PlaneGeometry(2000, 2000), new MeshStandardMaterial({ name: "ground", color: [0.35, 0.35, 0.35], roughness: 0.6 }));
  ground.name = "ground";
  ground.rotation.x = -Math.PI / 2;
  scene.add(ground);
  const block = new MeshStandardMaterial({ name: "block", color: [0.6, 0.6, 0.6], roughness: 0.5 });
  for (const [name, w, h, d, x, z, yaw] of [["block_a", 1, 1, 1, -3.4, -2.2, 0.3], ["block_b", 2, 0.5, 1, 3.1, -2.6, -0.2], ["block_c", 0.6, 2.2, 0.6, 2.4, -1.4, 0.0]] as const) {
    const m = new Mesh(new BoxGeometry(w, h, d), block);
    m.name = name;
    m.position.set(x, h / 2, z);
    m.rotation.y = yaw;
    scene.add(m);
  }

  for (const p of POSES) {
    const m = new GLTFModel(MANNEQUIN, { animation: { clip: p.clip, time: p.time } });
    m.id = p.id;
    m.position.set(p.x, 0, p.z);
    m.rotation.y = (p.yaw * Math.PI) / 180;
    scene.add(m);
  }

  const camera = new PerspectiveCamera(38, 16 / 9, 0.05, 500);
  camera.position.set(0.3, 1.45, 5.6);
  camera.lookAt(0, 0.85, 0);
  camera.setManualExposure(14.6);
  scene.add(camera);
  return { scene, camera, options: { name: "mannequin", width: 1280, height: 720 } };
}
