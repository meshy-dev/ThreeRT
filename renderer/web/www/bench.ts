// The boundary's cost: engine calls and the pose table timed on the module (the page runs it in a browser, `?bench=1`;
// tools/web-engine.ts --bench in Node, the same JavaScript engine without a page).
import type { WebEngine } from "./engine.ts";

/** The boundary's cost: each kind of engine call timed over many (ns a call), and the pose table of `n` bodies read
 *  (ms). The engine's world gets `n` objects with a body each (weightless spheres 2 m apart: nothing moves), and one
 *  tick (`tick`: the module's). */
export function bench(e: WebEngine, n: number, tick: () => void, progress?: (what: string, ms: number) => void) {
  const tri = [new Float32Array([0, 0, 0, 1, 0, 0, 0, 1, 0]), new Float32Array([0, 0, 1, 0, 0, 1, 0, 0, 1]), new Float32Array([0, 0, 1, 0, 0, 1]), new Uint32Array([0, 1, 2])] as const;
  e.geometry(1, ...tri);
  const mat = { name: "m", color: [1, 1, 1], opacity: 1, metalness: 0, roughness: 0.5, emissive: [0, 0, 0], emissiveIntensity: 0, doubleSided: false };
  e.material(2, mat as never);
  const desc = (k: number) => ({ name: `o${k}`, parent: null, castShadow: true, kind: "mesh", geometry: 1, material: 2 });
  const trs = new Float32Array([0, 0, 0, 0, 0, 0, 1, 1, 1, 1]);
  const t_build = performance.now();
  for (let k = 0; k < n; k++) {
    e.object(10 + k, desc(k) as never);
    trs[0] = (k % 100) * 2;
    trs[1] = 1;
    trs[2] = Math.floor(k / 100) * 2;
    e.transform(10 + k, trs);
    e.physics("body", { id: k + 1, object: 10 + k, slot: k, gravityScale: 0 });
    e.physics("collider", { id: k + 1, body: k + 1, shape: { type: "sphere", radius: 0.5 } });
  }
  const build_ms = performance.now() - t_build;
  // One tick: the world as a game's queries find it. Until a tick every collider is one "added since the last tick",
  // which each query tests one by one (physics-core `fresh`): 42 µs a ray over 1,000 of them, measured here in Node,
  // against the settled world's broad phase below.
  tick();
  progress?.(`${n} objects with a body each`, build_ms);
  /** ns a call: the best of five runs of `count` calls. */
  const time = (count: number, f: (k: number) => void) => {
    const t0 = performance.now();
    let best = Infinity;
    for (let r = 0; r < 5; r++) {
      const t = performance.now();
      for (let k = 0; k < count; k++) f(k);
      best = Math.min(best, performance.now() - t);
    }
    progress?.(`${count} calls x 5`, performance.now() - t0);
    return Math.round((best / count) * 1e6 * 10) / 10;
  };
  let sink = 0;
  const calls = {
    "visible(id, bool)": time(1e6, (k) => e.visible(10 + (k % n), true)),
    "power(id, number)": time(1e6, (k) => e.power(10 + (k % n), 1)),
    "transform(id, Float32Array(10))": time(2e5, (k) => e.transform(10 + (k % n), trs)),
    "customData(id, [4])": time(2e5, (k) => e.customData(10 + (k % n), [1, 2, 3, 4])),
    "material(id, desc)": time(5e4, () => e.material(2, mat as never)),
    "object(id, desc)": time(5e4, (k) => e.object(10 + (k % n), desc(k % n) as never)),
    'physics("state", { body })': time(5e4, (k) => (sink += e.physics("state", { body: 1 + (k % n) }).length)),
    'physics("impulse", { body, impulse })': time(5e4, (k) => (sink += e.physics("impulse", { body: 1 + (k % n), impulse: [0, 0, 0] }).length)),
    'physics("ray", { origin, direction })': time(2e4, () => (sink += e.physics("ray", { origin: [0.2, 9, 0.1], direction: [0, -1, 0] }).length)),
    "physicsPoses(), not read": time(2e3, () => (sink += e.physicsPoses().length)),
  };
  /** ms to fetch the table and read every body's seven numbers: the best of 20. */
  const read = (m: number) => {
    let best = Infinity;
    for (let r = 0; r < 20; r++) {
      const t = performance.now();
      const p = e.physicsPoses();
      for (let k = 0; k < m * 7; k++) sink += p[k];
      best = Math.min(best, performance.now() - t);
    }
    return Math.round(best * 1e4) / 1e4;
  };
  const poses: Record<string, number> = {};
  for (const m of [1000, 10000]) if (m <= n) poses[`${m} bodies' poses read from the table, ms`] = read(m);
  return { bodies: n, build_ms, calls_ns: calls, poses, isolated: (globalThis as { crossOriginIsolated?: boolean }).crossOriginIsolated ?? null, sink };
}
