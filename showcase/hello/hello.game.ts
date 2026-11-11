import { BoxGeometry, Color, Mesh, MeshStandardMaterial, PerspectiveCamera, RectAreaLight, Scene, SkyColor, SphereGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
export default defineGame<{ on: boolean }>({
  state: { on: true },
  setup() {
    const scene = new Scene();
    scene.environment = new SkyColor(0x101018, 0.2);
    const floor = new Mesh(new BoxGeometry(6, 0.2, 6), new MeshStandardMaterial({ color: 0x808080, roughness: 0.4 }));
    floor.id = "floor"; floor.position.y = -0.1; scene.add(floor);
    const ball = new Mesh(new SphereGeometry(0.5, 48, 24), new MeshStandardMaterial({ color: new Color().setRGB(1, 0.77, 0.34), metalness: 1, roughness: 0.2 }));
    ball.id = "ball"; ball.position.y = 0.5; scene.add(ball);
    const light = new RectAreaLight(0xffffff, 8000, 1, 1);
    light.id = "light"; light.position.set(0, 3, 0); light.lookAt(0, 0, 0); scene.add(light);
    const camera = new PerspectiveCamera(40, 16 / 9, 0.05, 100);
    camera.position.set(0, 1.4, 4); camera.lookAt(0, 0.5, 0); camera.setManualExposure(8); scene.add(camera);
    return { scene, camera, options: { name: "hello", preset: "preview" as const, width: 1280, height: 720 } };
  },
  frame(f) { (f.object("light") as RectAreaLight).intensity = f.state.on ? 8000 : 500; },
  handlers: { click(tx) { tx.state.on = !tx.state.on; } },
});
