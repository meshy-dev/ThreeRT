// The foliage scene of ROADMAP § 5 item 6.5: a shop front in afternoon sun with potted plants and ferns swaying in a
// light wind (windSway, rigid). Poly Haven CC0 models (node tools/fetch-polyhaven.ts --res 1k --models
// potted_plant_02,potted_plant_04,fern_02): each pot stays put and its plant leans about the soil, a group at the soil
// height holding the plant's node; the ferns lean about their base. Wind toward +x; the camera at eye height looks
// along the shop front.
//   node tools/bundle-game.ts tools/foliage-scene.ts -o out/bundles/foliage-scene.js
import { BoxGeometry, DirectionalLight, GLTFModel, Group, Mesh, MeshStandardMaterial, PerspectiveCamera, Scene, SkyAtmosphere, type WindSwayOptions, windSway } from "@threert/scene";
import { defineGame } from "@threert/gameplay";

const M = "assets/polyhaven/models";
/** Wind: toward +x, gentle. */
const WIND: WindSwayOptions = { direction: 0, strength: 0.09, frequency: 0.7 };

/** A potted plant: the pot and soil fixed, the plant swaying about the soil (its node's origin in the file). */
function pottedPlant(uri: string, pot: string[], plant: string, soil: number, at: [number, number], scale: number, wind: WindSwayOptions) {
  const g = new Group();
  g.position.set(at[0], 0, at[1]);
  g.scale.setScalar(scale);
  g.add(new GLTFModel(uri, { nodes: pot }));
  const pivot = new Group();
  pivot.position.y = soil;
  const leaves = new GLTFModel(uri, { nodes: [plant] });
  leaves.position.y = -soil;
  pivot.add(leaves);
  g.add(windSway(pivot, wind));
  return g;
}

/** A plant on the ground (its node's origin at the base), swaying as a whole. */
function groundPlant(uri: string, nodes: string[], at: readonly [number, number], yaw: number, scale: number, wind: WindSwayOptions) {
  const m = new GLTFModel(uri, { nodes });
  m.position.set(at[0], 0, at[1]);
  m.rotation.y = yaw;
  m.scale.setScalar(scale);
  return windSway(m, wind);
}

export default defineGame({
  state: {},
  setup: () => {
    const scene = new Scene();
    const sun = new DirectionalLight(0xffffff, 90000);
    sun.setSunAngles(35, 320);
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun });
    const camera = new PerspectiveCamera(50, 16 / 9, 0.05, 300);
    camera.position.set(-3.2, 1.6, 4.2);
    camera.lookAt(1.5, 0.5, -0.4);
    camera.setManualExposure(13);
    scene.add(camera);

    const paving = new MeshStandardMaterial({ name: "paving", color: 0x9a958c, roughness: 0.8 });
    const plaster = new MeshStandardMaterial({ name: "plaster", color: 0xc9bfae, roughness: 0.9 });
    const frame = new MeshStandardMaterial({ name: "frame", color: 0x2a2c30, roughness: 0.5, metalness: 1 });
    const ground = new Mesh(new BoxGeometry(2000, 0.1, 2000), paving);
    ground.position.y = -0.05;
    scene.add(ground);
    const wall = new Mesh(new BoxGeometry(14, 4, 0.3), plaster);
    wall.position.set(0, 2, -1.6);
    scene.add(wall);
    for (const x of [-3, 0, 3]) {
      const pane = new Mesh(new BoxGeometry(2, 2.2, 0.05), frame);
      pane.position.set(x, 1.6, -1.43);
      scene.add(pane);
    }

    const pp2 = `${M}/potted_plant_02_1k.glb`, pp4 = `${M}/potted_plant_04_1k.glb`;
    for (const [i, x] of [-2.4, 0.9, 3.4].entries()) {
      scene.add(pottedPlant(pp2, ["potted_plant_02_pot", "potted_plant_02_dirt"], "potted_plant_02_leaves", 0.305, [x, -0.9], 1.6, { ...WIND, stiffness: 0.3 * i }));
    }
    for (const x of [-1.2, -0.7, 2.1]) {
      scene.add(pottedPlant(pp4, ["potted_plant_04_pot", "potted_plant_04_dirt", "potted_plant_04_ground"], "potted_plant_04_plant", 0.2, [x, -1.0], 2.5, { ...WIND, stiffness: 1 }));
    }
    for (const [x, z, yaw] of [[-1.6, 0.1, 0.4], [-0.2, 0.9, 1.9], [1.7, 0.3, 3.0], [2.8, 1.1, 0.8]] as const) {
      scene.add(groundPlant(`${M}/fern_02_1k.glb`, ["fern_02_b"], [x, z], yaw, 1.3, { ...WIND, strength: 0.15, frequency: 0.9 }));
    }
    return { scene, camera, options: { name: "foliage", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  handlers: {},
});
