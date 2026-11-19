// A wrecking ball through a round tower of bricks at last light. The tower is 18 courses of
// 14 bricks, a quarter of them glowing; the ball (chrome, 5 t) hangs on a rigid cable from a point high above, held
// up and back by a hidden latch. On `go` the latch goes and the ball swings down through the tower: the glowing bricks
// tumble out across the polished floor, carrying their light and their reflections with them.
//   node tools/site-reel.ts wreck [--probe]        (the landing page's reel)
import { BallJoint, BoxGeometry, CylinderGeometry, DirectionalLight, FixedJoint, Mesh, MeshStandardMaterial, PerspectiveCamera, Scene, SkyAtmosphere, SphereGeometry, Vector3 } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { GLOWS, glow, plain, seeded } from "./kit.ts";

/** A brick (m), bricks a course, courses. */
export const BW = 0.5, BH = 0.3, BD = 0.3, PER = 14, COURSES = 18;
/** The ball's radius, the cable's length (pivot to the ball's centre), the pivot's height, the angle it is held at. */
export const R = 0.8, L = 8.2, PIVOT = 9.65, HELD = (68 * Math.PI) / 180;
/** Look: the sun (lux above the atmosphere, elevation and azimuth in degrees), the glow (nits), the exposure. */
const SUN = 5000, SUN_EL = 7, SUN_AZ = -20, NITS = 900, EV = 8.2;

const bricks: Mesh[] = [];
let ball: Mesh | undefined, latch: Mesh | undefined;

export default defineGame({
  state: {},
  keys: { Space: "go" },
  setup: () => {
    bricks.length = 0;
    const rnd = seeded(23);
    const scene = new Scene();
    const sun = new DirectionalLight(0xffb070, SUN);
    sun.setSunAngles(SUN_EL, SUN_AZ);
    sun.angularDiameter = 2.4;   // a soft edge to the long shadows
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun });
    const floor = new Mesh(new BoxGeometry(600, 1, 600), new MeshStandardMaterial({ name: "floor", color: 0x5c5f66, roughness: 0.28 }));
    floor.name = "floor";
    floor.position.set(0, -0.5, 0);
    floor.physics = { type: "fixed", friction: 0.9, restitution: 0.1 };
    scene.add(floor);

    // the tower: rings of bricks, each course turned half a brick on the one under it
    const [gold, , copper, lacquer, ceramic, rubber] = plain();
    const plains = [ceramic, ceramic, ceramic, ceramic, ceramic, copper, copper, gold, lacquer, rubber];
    const glows = [GLOWS[0], GLOWS[0], GLOWS[1], GLOWS[1], GLOWS[2], GLOWS[3], GLOWS[4]].map((c, k) => glow(`glow${k}`, c, NITS));
    const geometry = new BoxGeometry(BW, BH, BD);
    const ring = (BW / 2 + 0.003) / Math.tan(Math.PI / PER) + BD / 2;
    let k = 0;
    for (let c = 0; c < COURSES; c++) for (let i = 0; i < PER; i++) {
      const a = ((i + (c % 2) / 2) / PER) * Math.PI * 2;
      const m = new Mesh(geometry, rnd() < 0.24 ? glows[Math.floor(rnd() * glows.length)] : plains[Math.floor(rnd() * plains.length)]);
      m.position.set(Math.sin(a) * ring, BH / 2 + c * BH, Math.cos(a) * ring);
      m.rotation.y = a;
      m.physics = { type: "dynamic", density: 500, friction: 0.85, restitution: 0.05, angularDamping: 0.15 };
      m.name = `brick${k++}`;
      scene.add(m);
      bricks.push(m);
    }

    // the ball on its cable: one rigid pendulum (a ball joint at the pivot, the anchor a cable's length up the ball's
    // own +y), the cable a light rod fixed to it so that it too pushes what it meets
    const pivot = new Mesh(new BoxGeometry(0.3, 0.3, 0.3), rubber);
    pivot.position.set(0, PIVOT, 0);
    pivot.physics = "fixed";
    const chrome = new MeshStandardMaterial({ name: "ball", color: 0xe9ebee, metalness: 1, roughness: 0.05 });
    const steel = new MeshStandardMaterial({ name: "cable", color: 0x33363c, metalness: 1, roughness: 0.4 });
    const up = new Vector3(0, Math.cos(HELD), -Math.sin(HELD)), fwd = new Vector3(0, -Math.sin(HELD), -Math.cos(HELD));
    ball = new Mesh(new SphereGeometry(R, 64, 32), chrome);
    ball.name = "ball";
    ball.position.copy(pivot.position).addScaledVector(up, -L);
    ball.rotation.x = -HELD;
    ball.physics = { type: "dynamic", density: 5000, friction: 0.4, restitution: 0.1, canSleep: false };
    const len = L - R - 0.25;
    const cable = new Mesh(new CylinderGeometry(0.04, 0.04, len, 12), steel);
    cable.name = "cable";
    cable.position.copy(ball.position).addScaledVector(up, R + 0.03 + len / 2);
    cable.rotation.x = -HELD;
    cable.physics = { type: "dynamic", density: 1500, friction: 0.3, restitution: 0, canSleep: false };
    const collar = new Mesh(new CylinderGeometry(0.14, 0.2, 0.22, 20), steel);
    collar.position.set(0, R + 0.06, 0);
    ball.add(collar);
    latch = new Mesh(new BoxGeometry(1.2, 1.2, 0.6), rubber);
    latch.name = "latch";
    latch.position.copy(ball.position).addScaledVector(fwd, R + 0.3);
    latch.rotation.x = -HELD;
    latch.visible = false;
    latch.physics = { type: "fixed", friction: 1, restitution: 0 };
    scene.add(pivot, ball, cable, latch);
    scene.add(new BallJoint(pivot, ball, { anchorA: new Vector3(0, 0, 0), anchorB: new Vector3(0, L, 0) }));
    scene.add(new FixedJoint(ball, cable, { anchorA: new Vector3(0, R + 0.03 + len / 2, 0), anchorB: new Vector3(0, 0, 0) }));

    const camera = new PerspectiveCamera(38, 16 / 9, 0.1, 2000);
    camera.position.set(5.5, 1.4, -8);
    camera.lookAt(0, 2.6, 0);
    camera.setManualExposure(EV);
    scene.add(camera);
    return { scene, camera, options: { name: "launch-wreck", width: 1920, height: 1080, preset: "cinematic" as const } };
  },
  on: {
    /** The latch goes: the ball swings. */
    go(world) {
      if (latch) world.scene.remove(latch);
      latch = undefined;
    },
    /** For composing: where the ball is, and whether the tower stands still. */
    report(world) {
      let fastest = 0, awake = 0, top = 0;
      for (const b of bricks) { fastest = Math.max(fastest, b.getLinearVelocity().length()); if (!b.sleeping) awake++; top = Math.max(top, b.position.y); }
      const p = ball!.position;
      console.log(`report wreck t=${world.time.toFixed(2)} ball=${p.x.toFixed(2)},${p.y.toFixed(2)},${p.z.toFixed(2)} v=${ball!.getLinearVelocity().length().toFixed(2)} bricks: fastest=${fastest.toFixed(3)} awake=${awake} top=${top.toFixed(3)}`);
    },
  },
});
