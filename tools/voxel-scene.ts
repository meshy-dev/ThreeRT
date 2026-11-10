// The voxel engine's check scene (ROADMAP § 6 step 2b; tools/unbiased-check.ts `voxel`): voxel objects of
// packages/voxel in sun and sky on a voxel terrain. A hollow house with windows (light through openings, interior
// bounces), a sphere, a glowing voxel lamp (lights the terrain by BSDF rays only), boxes at all three granularities.
//   node tools/bundle-game.ts tools/voxel-scene.ts -o out/bundles/voxel-scene.js
import { DirectionalLight, MeshStandardMaterial, PerspectiveCamera, Scene, SkyAtmosphere } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { VoxelGrid, voxelMesh } from "@threert/voxel";

export default defineGame({
  state: {},
  setup: () => {
    const scene = new Scene();
    const sun = new DirectionalLight(0xffffff, 90000);
    sun.setSunAngles(35, 300);
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun });
    const camera = new PerspectiveCamera(50, 16 / 9, 0.05, 300);
    camera.position.set(-5, 4, 9);
    camera.lookAt(1, 1, 0);
    camera.setManualExposure(13);
    scene.add(camera);

    const mats = [
      new MeshStandardMaterial({ name: "unused" }),
      new MeshStandardMaterial({ name: "grass", color: 0x4f7a32, roughness: 0.9 }),
      new MeshStandardMaterial({ name: "dirt", color: 0x7a5a3a, roughness: 0.95 }),
      new MeshStandardMaterial({ name: "wall", color: 0xc8bca8, roughness: 0.8 }),
      new MeshStandardMaterial({ name: "roof", color: 0x8a3020, roughness: 0.6 }),
      new MeshStandardMaterial({ name: "metal", color: 0xb0b4bc, roughness: 0.3, metalness: 1 }),
      new MeshStandardMaterial({ name: "lamp", color: 0x000000, emissive: 0xffd080, emissiveIntensity: 60000 }),
    ];
    // Terrain: 160 x 160 voxels of rolling ground.
    const terrain = new VoxelGrid(0.1);
    for (let z = 0; z < 160; z++) for (let x = 0; x < 160; x++) {
      const h = Math.floor(6 + 3 * Math.sin(x * 0.06) * Math.cos(z * 0.05) + 2 * Math.sin((x + z) * 0.11));
      for (let y = 0; y < h; y++) terrain.set(x, y, z, y === h - 1 ? 1 : 2);
    }
    const ground = voxelMesh(terrain, mats, "region");
    ground.position.set(-8, -1, -8);
    scene.add(ground);
    // A hollow house: walls with windows, a floor, a pitched roof.
    const house = new VoxelGrid(0.1);
    for (let y = 0; y < 30; y++) for (let z = 0; z < 40; z++) for (let x = 0; x < 50; x++) {
      const wall = x < 2 || x >= 48 || z < 2 || z >= 38;
      const window = wall && y >= 10 && y < 20 && ((x % 12) >= 4 && (x % 12) < 9 || (z % 12) >= 4 && (z % 12) < 9) && !(x < 2 && z < 2);
      if ((wall && !window) || y < 2) house.set(x, y, z, 3);
    }
    for (let k = 0; k < 22; k++) for (let z = -2; z < 42; z++) for (let x = k - 2; x < 52 - k; x++) house.set(x, 30 + k, z, 4);
    const home = voxelMesh(house, mats, "brick");
    home.position.set(0, 0, -2);
    scene.add(home);
    // A metal sphere, its boxes one for the object.
    const ball = new VoxelGrid(0.1);
    for (let z = -8; z < 8; z++) for (let y = -8; y < 8; y++) for (let x = -8; x < 8; x++) if ((x + 0.5) ** 2 + (y + 0.5) ** 2 + (z + 0.5) ** 2 <= 64) ball.set(x, y, z, 5);
    const orb = voxelMesh(ball, mats, "object");
    orb.position.set(-2.5, 0.8, 2);
    scene.add(orb);
    // A glowing lamp post.
    const post = new VoxelGrid(0.1);
    post.fill(0, 0, 0, 2, 14, 2, 5);
    post.fill(-1, 14, -1, 3, 17, 3, 6);
    const lamp = voxelMesh(post, mats);
    lamp.position.set(2.5, 0, 3);
    lamp.castShadow = false;
    scene.add(lamp);
    return { scene, camera, options: { name: "voxel-scene", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  handlers: {},
});
