// The spark / debris burst of ROADMAP § 5 item 6.2, as ue5/scripts/ue_burst.py builds it in UE (keep in sync): a
// 30 x 30 m floor (albedo 0.35, roughness 0.6) in the dark, the camera at (0, 1.6, 5) looking at (0, 0.8, 0) with a
// 50-degree vertical field of view, manual exposure EV100 6. The "fire" event fires the burst at the next frame's scene
// time: tools/bench-burst.ts sends it so the burst starts at capture frame 10, as the UE driver fires SimpleExplosion
// at sequence frame 10.
import { Debris, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, Scene, SkyColor, SparkBurst } from "@threert/scene";
import { defineGame } from "@threert/gameplay";

let debris: Debris;
let sparks: SparkBurst;
let fireAt: number | null = null;

export default defineGame<{ fired: boolean }>({
  state: { fired: false },
  setup: () => {
    const scene = new Scene();
    scene.environment = new SkyColor(0x000000, 0);
    const camera = new PerspectiveCamera(50, 16 / 9, 0.05, 200);
    camera.position.set(0, 1.6, 5);
    camera.lookAt(0, 0.8, 0);
    camera.setManualExposure(6);
    scene.add(camera);
    const floor = new Mesh(new PlaneGeometry(30, 30), new MeshStandardMaterial({ name: "floor", color: 0xa0a0a0, roughness: 0.6 }));
    floor.rotation.x = -Math.PI / 2;
    scene.add(floor);
    // SimpleExplosion's sparks, ported by eye: a narrow fast fountain of long, dim streaks that live up to 1.8 s.
    sparks = new SparkBurst({ count: 150, speed: 10, spread: 0.3, drag: 0.8, life: 1.8, streak: 0.05, intensity: 120 });
    sparks.position.set(0, 0.05, 0);
    sparks.customData = [1e9, 0, 0, 0]; // not yet
    scene.add(sparks);
    debris = new Debris();
    scene.add(debris);
    return { scene, camera, options: { name: "burst", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  frame(f) {
    if (f.state.fired && fireAt === null) {
      fireAt = f.time;
      sparks.customData = [fireAt, 0, 0, 0];
      debris.fire(fireAt);
    }
    debris.update(f.time);
  },
  handlers: {
    fire(tx) {
      tx.state.fired = true;
    },
  },
});
