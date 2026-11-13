// The host channel in one small game (docs/RUNTIME.md "The host channel"): a game has no sockets of its own, so it
// asks the app that runs it. Opened with a socket's address (`launch`), it has the app open the socket, says hello,
// and answers each of the first messages that come back; every message raises the cube a step. Opened with an
// address to fetch, it has the app make the request.
//
// - `host.post` asks; `host.on` hears the answer, on the node that owns the scene (a listener gets the same `world` a
//   world handler gets).
// - What a message means to the game is a state transaction (`world.emit("received")`), so the scene follows the
//   state in `frame` like anything else that happens in the game.
// - An app without a network never answers. The game then stays as it started: the cube on the ground, the HUD
//   saying "offline".
// - The request is written the way a library would wrap it, as a promise (`fetched`): the app's answer, a later
//   message, resolves it, and the code after the `await` runs in that message's entry (docs/RUNTIME.md "Promises").
//
//   node tools/bundle-game.ts games/host/echo.game.ts -o out/bundles/host-echo.js
//   renderer/target/release/threert-render play out/bundles/host-echo.js --launch '{"url":"wss://echo.example/socket"}'
//   node tools/host-echo.ts          (the channel checked: this script as the app, then the window's real sockets)
import { BoxGeometry, DirectionalLight, Mesh, MeshStandardMaterial, PerspectiveCamera, Scene, SkyAtmosphere } from "@threert/scene";
import { defineGame, host, hud, type HostEvents } from "@threert/gameplay";

/** How many messages the game answers before it closes the socket. */
export const ROUNDS = 3;
const SOCKET = "echo";
let status = "offline";

/** `net.fetch` as a promise of its answer. (No timers: it settles when the app answers, in a message.) */
const answers = new Map<string, (m: HostEvents["net.fetched"]) => void>();
host.on("net.fetched", (m) => answers.get(m.id)?.(m));
const fetched = (id: string, url: string) =>
  new Promise<HostEvents["net.fetched"]>((resolve) => {
    answers.set(id, resolve);
    host.post({ t: "net.fetch", id, url });
  });

async function page(url: string): Promise<void> {
  const m = await fetched("page", url);
  console.log(`echo fetched ${m.status} ${JSON.stringify(m.body.slice(0, 40))}${m.error ? ` ${m.error}` : ""}`);
}

host.on("launch", (m) => {
  if (typeof m.params.url === "string") {
    status = "connecting";
    host.post({ t: "net.open", id: SOCKET, url: m.params.url });
  }
  // (A listener returns no promise itself: what it starts goes on after it.)
  if (typeof m.params.fetch === "string") void page(m.params.fetch);
});
host.on("net.opened", (m) => {
  status = "open";
  host.post({ t: "net.send", id: m.id, data: "hello" });
});
host.on("net.messages", (m, world) => {
  // One batch a frame: every message that arrived since the last one, in order.
  for (const text of m.list) {
    console.log(`echo got ${JSON.stringify(text)}`);
    world.emit("received", { text });
  }
  const seen = (world.state as { received: number }).received + m.list.length;
  if (seen < ROUNDS) host.post({ t: "net.send", id: m.id, data: `again ${seen}` });
  else host.post({ t: "net.close", id: m.id });
});
host.on("net.closed", (m) => {
  status = `closed (${m.code})`;
  console.log(`echo closed ${m.code} ${m.reason}`);
});

export default defineGame({
  state: { received: 0 },
  setup: () => {
    const scene = new Scene();
    const sun = new DirectionalLight(0xffffff, 90000);
    sun.setSunAngles(50, 215);
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun });
    const camera = new PerspectiveCamera(45, 16 / 9, 0.1, 100);
    camera.position.set(3, 2.5, 6);
    camera.lookAt(0, 1, 0);
    camera.setManualExposure(14);
    const ground = new Mesh(new BoxGeometry(12, 0.2, 12), new MeshStandardMaterial({ name: "ground", color: 0x6f7468, roughness: 0.95 }));
    ground.position.y = -0.1;
    const cube = new Mesh(new BoxGeometry(1, 1, 1), new MeshStandardMaterial({ name: "cube", color: 0x3d7ec8, roughness: 0.6 }));
    cube.id = "cube";
    scene.add(camera, ground, cube);
    return { scene, camera, options: { name: "host-echo", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  handlers: {
    received(tx) {
      tx.state.received += 1;
    },
  },
  frame(f) {
    f.object("cube")!.position.y = 0.5 + 0.5 * f.state.received;
    hud.set({
      items: [
        { type: "text", anchor: "top-left", label: "Socket", text: status, tone: status === "open" ? "good" : "dim" },
        { type: "text", anchor: "top-left", label: "Messages", text: String(f.state.received) },
      ],
    });
  },
});
