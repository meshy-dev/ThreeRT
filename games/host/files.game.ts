// The player's files and the game's store in one small game (docs/RUNTIME.md "Files"): the player picks a folder, the
// game reads a model from it, "converts" it a slice a frame (a copy, standing for a real conversion), keeps the result
// in its store, and shows it from there as a `user:` asset. Nothing leaves the device.
//
// - Only where the app answers both families (`host.caps`, from the launch message): elsewhere the game says so on
//   its HUD and shows the cube alone.
// - A second run finds the model in the store already and shows it without asking.
//
//   node tools/bundle-game.ts games/host/files.game.ts -o out/bundles/host-files.js
//   renderer/target/release/threert-render play out/bundles/host-files.js --assets . \
//     --store /tmp/host-files-store --pick <a folder with a .glb or .gltf in it>
import { BoxGeometry, DirectionalLight, GLTFModel, Mesh, MeshStandardMaterial, PerspectiveCamera, Scene, SkyAtmosphere } from "@threert/scene";
import { defineGame, files, host, HostRequestError, hud, sliced, store, type Sliced } from "@threert/gameplay";

const STORED = "models/imported.glb";
let status = "no files here";
let scene: Scene | null = null;
let job: Sliced<Uint8Array> | null = null;

/** The stand-in for a conversion: the bytes copied 64 KiB a step. */
function* convert(bytes: Uint8Array): Generator<number, Uint8Array> {
  const out = new Uint8Array(bytes.length);
  for (let i = 0; i < bytes.length; i += 1 << 16) {
    out.set(bytes.subarray(i, i + (1 << 16)), i);
    yield i / bytes.length;
  }
  return out;
}

function show() {
  const model = new GLTFModel(store.uri(STORED));
  scene?.add(model);
  status = "shown from the store";
  console.log(`files: shown ${store.uri(STORED)}`);
}

async function begin() {
  const kept = await store.list("models/");
  console.log(`files: the store has ${JSON.stringify(kept)}`);
  if (kept.some((e) => e.path === STORED)) return show();
  status = "picking";
  const picked = await files.pick({ mode: "folder", accept: [".glb", ".gltf"], title: "A folder with a model" });
  if (!picked) {
    status = "canceled";
    return;
  }
  console.log(`files: picked ${picked.name}: ${JSON.stringify(picked.entries)}`);
  const first = picked.entries[0];
  if (!first) {
    status = "no model in it";
    return;
  }
  const bytes = await files.read(picked.pick, first.path);
  console.log(`files: read ${first.path}, ${bytes.length} bytes`);
  status = "converting";
  job = sliced(convert(bytes));
}

async function keep(bytes: Uint8Array) {
  await store.write(STORED, bytes);
  await store.write("models/imported.json", JSON.stringify({ from: "picked", bytes: bytes.length }));
  console.log(`files: stored ${await store.readText("models/imported.json")}`);
  show();
}

host.on("launch", () => {
  console.log(`files: caps ${JSON.stringify(host.caps)}`);
  if (files.available() && store.available()) begin().catch((e: unknown) => (status = e instanceof HostRequestError && e.reason === "unsupported" ? "nothing to pick from" : `failed: ${String(e)}`));
});

export default defineGame({
  state: {},
  setup: () => {
    scene = new Scene();
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
    const cube = new Mesh(new BoxGeometry(0.5, 0.5, 0.5), new MeshStandardMaterial({ name: "cube", color: 0x3d7ec8, roughness: 0.6 }));
    cube.position.set(-1.5, 0.25, 0);
    scene.add(camera, ground, cube);
    return { scene, camera, options: { name: "host-files", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  handlers: {},
  frame() {
    if (job) {
      const r = job.step(4);
      if (r.done) {
        const out = r.value!;
        job = null;
        status = "storing";
        keep(out).catch((e: unknown) => (status = `failed: ${String(e)}`));
      } else status = `converting ${Math.round((r.value as unknown as number) * 100)} %`;
    }
    hud.set({ items: [{ type: "text", anchor: "top-left", label: "Files", text: status }] });
  },
});
