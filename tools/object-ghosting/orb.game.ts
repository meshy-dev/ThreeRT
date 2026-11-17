// A glowing orb thrown across a dark room with a polished floor (the Atrium of threert/native-app-template, at
// night): a moving light, seen directly and in the floor's reflection, lighting the floor and wall as it goes. The
// driver (bench-object-ghosting.ts) moves it with `{t: "pose", x, y, z}` host messages, one per frame.
import { BoxGeometry, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, Scene, SkyGradient, SphereGeometry } from "@threert/scene";
import { defineGame, host } from "@threert/gameplay";

let orb: Mesh;

host.on("pose", (m) => {
  orb.position.set(Number(m.x ?? 0), Number(m.y ?? 0.2), Number(m.z ?? 0));
});

export default defineGame({
  state: {},
  handlers: {},
  setup: () => {
    const scene = new Scene();
    scene.environment = new SkyGradient({ zenith: 0x0a0d14, horizon: 0x0c0d10, ground: 0x050505, intensity: 0.05 });
    const floor = new Mesh(new PlaneGeometry(4, 4), new MeshStandardMaterial({ color: 0x1c1c1e, roughness: 0.08 }));
    floor.rotation.x = -Math.PI / 2;
    scene.add(floor);
    const wall = new Mesh(new BoxGeometry(4, 2, 0.05), new MeshStandardMaterial({ color: 0x8a857c, roughness: 0.9 }));
    wall.position.set(0, 1, -1.2);
    scene.add(wall);
    // A still object for the orb to pass behind and light.
    const block = new Mesh(new BoxGeometry(0.18, 0.3, 0.18), new MeshStandardMaterial({ color: 0x9c6a3c, roughness: 0.5 }));
    block.position.set(0.05, 0.15, -0.45);
    scene.add(block);
    orb = new Mesh(new SphereGeometry(0.035, 24, 16), new MeshStandardMaterial({ color: 0x000000, emissive: 0xffc27a, emissiveIntensity: 40000 }));
    orb.position.set(0, 0.2, 0);
    scene.add(orb);
    const camera = new PerspectiveCamera(45, 16 / 9, 0.02, 30);
    camera.position.set(0, 0.45, 1.25);
    camera.lookAt(0, 0.18, -0.2);
    camera.setManualExposure(3);
    scene.add(camera);
    return { scene, camera, options: { name: "object-ghosting-orb", width: 1280, height: 720 } };
  },
});
