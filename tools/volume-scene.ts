// The volume presets of ROADMAP § 5 item 6.3 side by side, the scene ue5/volumes.ts rebuilds in UE (keep in sync): a
// 30 x 30 m floor and a back wall (albedo 0.35), no sky. Left, a FogVolume (UE's Local Fog Volume: radius 1.2 m, cut
// flat at half its radius above the centre) lit by a warm point light inside it;
// middle, a SmokeColumn lit by a cool moon; right, a Fireball 0.7 s into its life at
// time 0 (offline stills and frozen-time live frames show it there), repeating every 3 s. The camera is at
// (0, 1.6, 7) looking at (0, 1.4, 0), 50-degree vertical field of view, manual exposure EV100 3.
//   node tools/bundle-game.ts tools/volume-scene.ts -o out/bundles/volume-scene.js
import { DirectionalLight, Fireball, FogVolume, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, PointLight, Scene, SkyColor, SmokeColumn } from "@threert/scene";
import { defineGame } from "@threert/gameplay";

let fireball: Fireball;

export default defineGame({
  state: {},
  setup: () => {
    const scene = new Scene();
    scene.environment = new SkyColor(0x000000, 0);
    const camera = new PerspectiveCamera(50, 16 / 9, 0.05, 200);
    camera.position.set(0, 1.6, 7);
    camera.lookAt(0, 1.4, 0);
    camera.setManualExposure(3);
    scene.add(camera);
    const grey = new MeshStandardMaterial({ name: "grey", color: 0xa0a0a0, roughness: 0.8 });
    const floor = new Mesh(new PlaneGeometry(30, 30), grey);
    floor.rotation.x = -Math.PI / 2;
    scene.add(floor);
    const wall = new Mesh(new PlaneGeometry(14, 7), grey);
    wall.position.set(0, 3.5, -3);
    scene.add(wall);

    const fog = new FogVolume({ radialExtinction: 2, heightExtinction: 1, heightFalloff: 1000, heightOffset: 0.5, albedo: [0.9, 0.9, 0.9], phaseG: 0.2 });
    fog.position.set(-3.2, 1.1, 0);
    fog.scale.set(2.4, 2.4, 2.4);
    scene.add(fog);
    const lamp = new PointLight(0xffb070, 60);
    lamp.position.set(-3.2, 1.4, 0);
    lamp.radius = 0.05;
    scene.add(lamp);

    const smoke = new SmokeColumn({ density: 4 });
    smoke.position.set(0, 2.2, -0.5);
    smoke.scale.set(1.6, 4.4, 1.6);
    scene.add(smoke);
    const moon = new DirectionalLight(0xb0c4ff, 25);
    moon.position.set(4, 8, 6);
    scene.add(moon);

    fireball = new Fireball({ period: 3, intensity: 40 });
    fireball.position.set(3.2, 1.2, 0);
    fireball.scale.set(2, 2, 2);
    fireball.customData = [-0.7, 0, 0, 0];
    scene.add(fireball);
    return { scene, camera, options: { name: "volumes", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  handlers: {},
});
