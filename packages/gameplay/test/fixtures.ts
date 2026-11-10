// Shared test game: three coins, a lamp and a camera. Handlers change the state only; `frame` derives the scene.
import { BoxGeometry, Mesh, MeshStandardMaterial, PerspectiveCamera, PointLight, Scene, SphereGeometry } from "@threert/scene";
import { createStore, defineGame, initialWorld, type ExecResult, type GameDefinition, type GameEvent, type Store } from "../src/index.ts";

export const COINS = ["coin_a", "coin_b", "coin_c"];

type S = {
  score: number;
  coins: Record<string, boolean>;
  log: string[];
  lamp: number;
  lampEdits: number;
  paint: Record<string, number>;
  bonusAttempt: boolean | null;
  spawned: string[];
};

export const coinGame = defineGame<S>({
  seed: 42,
  state: { score: 0, coins: {}, log: [], lamp: 100, lampEdits: 0, paint: {}, bonusAttempt: null, spawned: [] },
  setup() {
    const scene = new Scene();
    COINS.forEach((id, i) => {
      const coin = new Mesh(new SphereGeometry(0.2, 16, 8), new MeshStandardMaterial({ name: id, color: 0xffd700, metalness: 1, roughness: 0.3 }));
      coin.id = id;
      coin.position.set(i - 1, 0.5, 0);
      scene.add(coin);
    });
    const lamp = new PointLight(0xffffff, 100);
    lamp.id = "lamp";
    lamp.position.set(0, 3, 0);
    scene.add(lamp);
    const camera = new PerspectiveCamera(50, 16 / 9);
    camera.id = "camera";
    camera.position.set(0, 1, 5);
    scene.add(camera);
    return { scene, camera, options: { name: "coins" } };
  },
  frame(f) {
    for (const id of COINS) {
      const coin = f.object(id) as Mesh;
      coin.visible = !f.state.coins[id];
      if (f.state.paint[id] !== undefined) coin.material.color.set(f.state.paint[id]);
    }
    (f.object("lamp") as PointLight).intensity = f.state.lamp;
    // Spawned boxes: one per name in the state.
    for (const name of f.state.spawned) {
      if (f.object(name)) continue;
      const box = new Mesh(new BoxGeometry(0.5, 0.5, 0.5), new MeshStandardMaterial({ color: 0xff0000 }));
      box.id = name;
      f.scene.add(box);
    }
    for (const o of [...f.scene.children]) if (o.stableId?.startsWith("box_") && !f.state.spawned.includes(o.stableId)) f.scene.remove(o);
  },
  handlers: {
    click(tx, ev: { target: string; player?: string }) {
      tx.decide(COINS.includes(ev.target), `"${ev.target}" is not a coin`);
      tx.decide(!tx.state.coins[ev.target], "already collected");
      tx.state.coins[ev.target] = true;
      tx.state.score += 1;
      tx.state.log.push(`${ev.player ?? "anon"}:${ev.target}`);
      tx.emit("scored", { by: ev.player ?? "anon" });
      tx.requestRender({ preset: "preview" });
    },
    reset(tx, ev: { target: string }) {
      delete tx.state.coins[ev.target];
    },
    lamp(tx, ev: { intensity: number }) {
      tx.state.lamp = ev.intensity;
      tx.state.lampEdits += 1;
    },
    paint(tx, ev: { target: string; color: number }) {
      tx.state.paint[ev.target] = ev.color;
    },
    bonus(tx) {
      tx.state.score += 10;
      tx.state.bonusAttempt = tx.attempt(() => {
        tx.state.score += 100;
        tx.emit("jackpot");
        tx.decide(false, "no jackpot");
      });
    },
    fail(tx) {
      tx.state.score = 999;
      tx.decide(false, "nope");
    },
    boom() {
      throw new Error("boom");
    },
    spawn(tx, ev: { name: string }) {
      tx.state.spawned.push(`box_${ev.name}`);
    },
    remove(tx, ev: { name: string }) {
      tx.state.spawned = tx.state.spawned.filter((n) => n !== `box_${ev.name}`);
    },
  },
}) as GameDefinition<any>;

export function freshStore(): Store {
  return createStore(initialWorld(coinGame), 42);
}

let nextId = 1;
export function event(type: string, payload: object = {}, ts = 1_700_000_000_000): GameEvent {
  return { id: nextId++, type, payload: payload as GameEvent["payload"], ts };
}

export function writePaths(r: ExecResult): string[] {
  return r.writes.map((w) => w.path);
}
