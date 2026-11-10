// A glTF camera node's animated path (translation / rotation channels, linear), sampled as live `camera` commands:
// the Bistro district drive follows Bistro's own FBX camera path (scripts/orca.ts keeps it in the take).
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";

type M = number[]; // 4 x 4, column-major
const mul = (a: M, b: M): M => {
  const o = new Array(16).fill(0);
  for (let c = 0; c < 4; c++) for (let r = 0; r < 4; r++) for (let k = 0; k < 4; k++) o[c * 4 + r] += a[k * 4 + r] * b[c * 4 + k];
  return o;
};
const trs = (t: number[], q: number[], s: number[]): M => {
  const [x, y, z, w] = q;
  return [
    (1 - 2 * (y * y + z * z)) * s[0], 2 * (x * y + z * w) * s[0], 2 * (x * z - y * w) * s[0], 0,
    2 * (x * y - z * w) * s[1], (1 - 2 * (x * x + z * z)) * s[1], 2 * (y * z + x * w) * s[1], 0,
    2 * (x * z + y * w) * s[2], 2 * (y * z - x * w) * s[2], (1 - 2 * (x * x + y * y)) * s[2], 0,
    t[0], t[1], t[2], 1,
  ];
};
const slerp = (a: number[], b: number[], u: number) => {
  let d = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
  const bb = d < 0 ? b.map((v) => -v) : b;
  d = Math.abs(d);
  if (d > 0.9995) {
    const r = a.map((v, i) => v + (bb[i] - v) * u), n = Math.hypot(...r);
    return r.map((v) => v / n);
  }
  const th = Math.acos(d), s = Math.sin(th);
  return a.map((v, i) => (Math.sin((1 - u) * th) * v + Math.sin(u * th) * bb[i]) / s);
};

/** The path of the node named `name` (with a camera) in `file` (.gltf + its .bin): pose at time t (s) as {pos, target,
 *  tan} in glTF space shifted by `offset`; `duration` = the clip's length. Parents are taken at rest. */
export function gltfCameraPath(file: string, name = "Camera", offset: number[] = [0, 0, 0]) {
  const g = JSON.parse(readFileSync(file, "utf8"));
  const bin = readFileSync(join(dirname(file), g.buffers[0].uri));
  const acc = (i: number) => {
    const a = g.accessors[i], v = g.bufferViews[a.bufferView], n = ({ SCALAR: 1, VEC3: 3, VEC4: 4 } as Record<string, number>)[a.type];
    const f = new Float32Array(bin.buffer, bin.byteOffset + (v.byteOffset ?? 0) + (a.byteOffset ?? 0), a.count * n);
    return Array.from({ length: a.count }, (_, k) => Array.from(f.subarray(k * n, k * n + n)));
  };
  const ni = g.nodes.findIndex((n: { name?: string; camera?: number }) => n.name === name && n.camera !== undefined);
  if (ni < 0) throw new Error(`${file}: no camera node ${name}`);
  const parent = new Map<number, number>();
  g.nodes.forEach((n: { children?: number[] }, i: number) => n.children?.forEach((c) => parent.set(c, i)));
  const local = (n: { translation?: number[]; rotation?: number[]; scale?: number[] }) => trs(n.translation ?? [0, 0, 0], n.rotation ?? [0, 0, 0, 1], n.scale ?? [1, 1, 1]);
  let pm: M = trs([0, 0, 0], [0, 0, 0, 1], [1, 1, 1]);
  for (let p = parent.get(ni); p !== undefined; p = parent.get(p)) pm = mul(local(g.nodes[p]), pm);
  const node = g.nodes[ni];
  const tracks: Record<string, { t: number[]; v: number[][] }> = {};
  for (const a of g.animations ?? []) {
    for (const c of a.channels) {
      if (c.target.node !== ni) continue;
      const s = a.samplers[c.sampler];
      tracks[c.target.path] = { t: acc(s.input).map((x) => x[0]), v: acc(s.output) };
    }
  }
  const sample = (path: string, t: number, rest: number[]) => {
    const k = tracks[path];
    if (!k) return rest;
    if (t <= k.t[0]) return k.v[0];
    if (t >= k.t[k.t.length - 1]) return k.v[k.v.length - 1];
    let lo = 0, hi = k.t.length - 1;
    while (hi - lo > 1) {
      const mid = (lo + hi) >> 1;
      if (k.t[mid] <= t) lo = mid;
      else hi = mid;
    }
    const u = (t - k.t[lo]) / (k.t[hi] - k.t[lo]);
    return path === "rotation" ? slerp(k.v[lo], k.v[hi], u) : k.v[lo].map((v, i) => v + (k.v[hi][i] - v) * u);
  };
  const yfov = g.cameras[node.camera].perspective.yfov as number;
  const duration = Math.max(...Object.values(tracks).map((k) => k.t[k.t.length - 1]), 0);
  return {
    duration,
    at(t: number) {
      const w = mul(pm, trs(sample("translation", t, node.translation ?? [0, 0, 0]), sample("rotation", t, node.rotation ?? [0, 0, 0, 1]), [1, 1, 1]));
      const pos = [w[12] + offset[0], w[13] + offset[1], w[14] + offset[2]];
      const f = [-w[8], -w[9], -w[10]], n = Math.hypot(...f);
      return { pos, target: pos.map((v, i) => v + (f[i] / n) * 10), tan: Math.tan(yfov / 2) };
    },
  };
}
