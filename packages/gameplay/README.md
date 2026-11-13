# @threert/gameplay

Game logic for [ThreeRT](https://threert.dev), the hardware ray-traced engine with a three.js-style API: state, handlers that change it as transactions, and a frame script that shows it in the scene.

[![npm](https://img.shields.io/npm/v/@threert/gameplay?color=f97316)](https://www.npmjs.com/package/@threert/gameplay)
[![license](https://img.shields.io/npm/l/@threert/gameplay)](https://github.com/meshy-dev/ThreeRT/blob/main/LICENSE)

## Install

```bash
npm install @threert/scene @threert/gameplay
```

## A game

`defineGame` takes the initial state, a `setup` that builds the scene with [`@threert/scene`](https://www.npmjs.com/package/@threert/scene), handlers that change the state, and a `frame` script that derives the scene from the state every frame.

```ts
import { Scene, PerspectiveCamera, Mesh, SphereGeometry, MeshStandardMaterial, DirectionalLight, SkyAtmosphere } from "@threert/scene";
import { defineGame } from "@threert/gameplay";

export default defineGame({
  state: { lit: {} as Record<string, boolean>, night: false },

  setup() {
    const scene = new Scene();
    const sun = new DirectionalLight(0xffffff, 80000).setSunAngles(32, 35);
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun });
    const orb = new Mesh(new SphereGeometry(0.4), new MeshStandardMaterial({ color: 0xffffff, emissive: 0xffc890 }));
    orb.id = "orb_0"; // what handlers and the frame script find it by
    orb.position.set(0, 1, 0);
    scene.add(orb);
    const camera = new PerspectiveCamera(40, 16 / 9, 0.1, 200);
    camera.position.set(0, 1.4, 5);
    scene.add(camera);
    return { scene, camera };
  },

  // Handlers change the state, and only the state.
  handlers: {
    click(tx, ev) {
      tx.decide(ev.target.startsWith("orb_"), `nothing happens when you click ${ev.target}`);
      tx.state.lit[ev.target] = !tx.state.lit[ev.target];
    },
    night(tx) {
      tx.state.night = !tx.state.night;
    },
  },
  keys: { L: "night" },

  // The frame script derives the scene from the state, every frame.
  frame(f) {
    const orb = f.object("orb_0") as Mesh;
    orb.material.emissiveIntensity = f.state.lit.orb_0 ? 2500 : 0;
    orb.position.y = 1 + 0.08 * Math.sin(f.time * 2);
  },
});
```

- **Handlers are transactions.** A click reaches the game as a `click` event whose `target` is the object under the cursor. `tx.decide` refuses an event, and nothing is half-written.
- **Deterministic and replayable.** Events are logged with what they read and wrote; `replayLog` and `verifyLog` rebuild a world from its log, and seeded randomness (`seededRandom`) gives the same result on every machine.
- **World handlers** (`on`) reach the engine's own world: physics, a ray cast into the scene, objects added and removed as an event arrives.
- **Input, a HUD and the app.** A game with `pointer: "look"` reads the keys and the mouse every frame (`f.input.move`, `f.input.look`, `f.input.pressed("Space")`); `hud.set(...)` describes a crosshair, figures, bars and a scoreboard for the app to draw; `host.post` / `host.on` reach what the app owns (a WebSocket, an HTTP request, how the game was opened).

How scene code runs in the engine is in [RUNTIME.md](https://github.com/meshy-dev/ThreeRT/blob/main/docs/RUNTIME.md).

## Running it

The game runs inside a ThreeRT client, which path-traces the scene on the device's GPU: [ThreeRT for macOS and iPhone](https://threert.dev/download/), with `threert dev` for the edit-and-reload loop ([getting started](https://threert.dev/docs/)), or `threert-render play` built from [the engine's source](https://github.com/meshy-dev/ThreeRT). The same game runs as three.js in a browser.

## Links

[threert.dev](https://threert.dev) · [Docs](https://threert.dev/docs/) · [Showcase](https://threert.dev/showcase/) · [GitHub](https://github.com/meshy-dev/ThreeRT)

three.js is an independent open-source project; ThreeRT is not affiliated with it. MIT licence.
