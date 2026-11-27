// The glTF camera path (tools/lib/gltf-camera.ts) on small hand-made files: rest pose under its parents, linear and
// slerped keys, clamping outside the clip, and accessors laid out as the glTF spec allows (interleaved bufferViews,
// normalized integer rotations, a second buffer).
import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { gltfCameraPath } from "../gltf-camera.ts";

const FLOAT = 5126, BYTE = 5120, SHORT = 5122;
const size = { SCALAR: 1, VEC3: 3, VEC4: 4 } as Record<string, number>;

/** A glTF (+ .bin files) in a temporary directory, built from accessors given as plain numbers. */
function gltf(nodes: object[], o: { channels?: { node: number; path: string; times: number[]; values: number[][]; type?: number; interleave?: boolean; buffer?: number }[]; yfov?: number } = {}) {
  const bins: number[][] = [[], []];
  const accessors: object[] = [], bufferViews: object[] = [], samplers: object[] = [], channels: object[] = [];
  const push = (buffer: number, bytes: Buffer) => {
    const b = bins[buffer];
    while (b.length % 4) b.push(0);
    const off = b.length;
    b.push(...bytes);
    return off;
  };
  const encode = (vals: number[], type: number) => {
    const per = type === FLOAT ? 4 : type === SHORT ? 2 : 1;
    const out = Buffer.alloc(vals.length * per);
    vals.forEach((v, i) => (type === FLOAT ? out.writeFloatLE(v, i * 4) : type === SHORT ? out.writeInt16LE(Math.round(v * 32767), i * 2) : out.writeInt8(Math.round(v * 127), i)));
    return out;
  };
  for (const c of o.channels ?? []) {
    const type = c.path === "rotation" ? "VEC4" : "VEC3", n = size[type], ct = c.type ?? FLOAT, buffer = c.buffer ?? 0;
    const input = accessors.length;
    bufferViews.push({ buffer, byteOffset: push(buffer, encode(c.times, FLOAT)), byteLength: c.times.length * 4 });
    accessors.push({ bufferView: bufferViews.length - 1, componentType: FLOAT, count: c.times.length, type: "SCALAR", min: [c.times[0]], max: [c.times.at(-1)] });
    if (c.interleave) {
      // Each value followed by 8 bytes of something else (another attribute in the same view).
      const stride = n * 4 + 8, bytes = Buffer.alloc(stride * c.values.length, 0x7f);
      c.values.forEach((v, k) => encode(v, FLOAT).copy(bytes, k * stride));
      bufferViews.push({ buffer, byteOffset: push(buffer, bytes), byteLength: bytes.length, byteStride: stride });
    } else {
      const bytes = encode(c.values.flat(), ct);
      bufferViews.push({ buffer, byteOffset: push(buffer, bytes), byteLength: bytes.length });
    }
    accessors.push({ bufferView: bufferViews.length - 1, componentType: ct, normalized: ct !== FLOAT || undefined, count: c.values.length, type });
    samplers.push({ input, output: accessors.length - 1, interpolation: "LINEAR" });
    channels.push({ sampler: samplers.length - 1, target: { node: c.node, path: c.path } });
  }
  const dir = mkdtempSync(join(tmpdir(), "gltf-camera-"));
  const buffers = bins.map((b, i) => {
    writeFileSync(join(dir, `b${i}.bin`), Buffer.from(b));
    return { uri: `b${i}.bin`, byteLength: b.length };
  });
  const doc = { asset: { version: "2.0" }, buffers, bufferViews, accessors, nodes, cameras: [{ type: "perspective", perspective: { yfov: o.yfov ?? Math.PI / 3, znear: 0.1 } }], animations: channels.length ? [{ samplers, channels }] : [] };
  const file = join(dir, "scene.gltf");
  writeFileSync(file, JSON.stringify(doc));
  return file;
}

const close = (a: number[], b: number[], what: string, eps = 1e-5) => assert.ok(a.length === b.length && a.every((v, i) => Math.abs(v - b[i]) < eps), `${what}: ${a} vs ${b}`);
/** Unit view direction of a pose. */
const fwd = (p: { pos: number[]; target: number[] }) => {
  const d = p.target.map((v, i) => v - p.pos[i]), n = Math.hypot(...d);
  return d.map((v) => v / n);
};
const yaw = (deg: number) => [0, Math.sin((deg * Math.PI) / 360), 0, Math.cos((deg * Math.PI) / 360)];

test("a still camera: its rest pose under its parents, looking down its -Z, shifted by the offset", () => {
  // Parent: 90 degrees about +Y, then 10 m up; the camera 2 m along the parent's +X (world -Z after the turn).
  const file = gltf([{ name: "Rig", rotation: yaw(90), translation: [0, 10, 0], children: [1] }, { name: "Camera", camera: 0, translation: [2, 0, 0] }], { yfov: Math.PI / 2 });
  const path = gltfCameraPath(file, "Camera", [100, 0, 0]);
  assert.equal(path.duration, 0);
  const p = path.at(3);
  close(p.pos, [100, 10, -2], "position");
  close(fwd(p), [-1, 0, 0], "view direction"); // -Z turned 90 degrees about +Y
  close([Math.hypot(...p.target.map((v, i) => v - p.pos[i]))], [10], "target distance");
  close([p.tan], [1], "tan(yfov / 2)");
});

test("keys: translation lerped, rotation slerped, held before the first and after the last", () => {
  const file = gltf([{ name: "Camera", camera: 0 }], {
    channels: [
      { node: 0, path: "translation", times: [1, 3], values: [[0, 0, 0], [10, 0, -20]] },
      { node: 0, path: "rotation", times: [1, 3], values: [yaw(0), yaw(90)] },
    ],
  });
  const path = gltfCameraPath(file);
  assert.equal(path.duration, 3);
  close(path.at(2).pos, [5, 0, -10], "halfway");
  const h = Math.SQRT1_2;
  close(fwd(path.at(2)), [-h, 0, -h], "halfway view: 45 degrees"); // slerp, not a normalized lerp of the ends
  close(path.at(0).pos, [0, 0, 0], "before the clip");
  close(path.at(9).pos, [10, 0, -20], "after the clip");
  close(fwd(path.at(9)), [-1, 0, 0], "after the clip: view");
});

test("another node named like the camera, without a camera, is not taken", () => {
  const file = gltf([{ name: "Camera", translation: [5, 5, 5] }, { name: "Camera", camera: 0, translation: [1, 2, 3] }]);
  close(gltfCameraPath(file).at(0).pos, [1, 2, 3], "position");
  assert.throws(() => gltfCameraPath(file, "Nope"), /no camera node Nope/);
});

test("an interleaved bufferView (byteStride) is read at its stride", () => {
  const file = gltf([{ name: "Camera", camera: 0 }], { channels: [{ node: 0, path: "translation", times: [0, 1, 2], values: [[0, 0, 0], [1, 2, 3], [4, 5, 6]], interleave: true }] });
  const path = gltfCameraPath(file);
  close(path.at(1).pos, [1, 2, 3], "second key");
  close(path.at(2).pos, [4, 5, 6], "third key");
});

test("rotations as normalized shorts and bytes (glTF 2.0 animation outputs) are decoded", () => {
  for (const type of [SHORT, BYTE]) {
    const file = gltf([{ name: "Camera", camera: 0 }], { channels: [{ node: 0, path: "rotation", times: [0, 1], values: [yaw(0), yaw(90)], type }] });
    close(fwd(gltfCameraPath(file).at(1)), [-1, 0, 0], `componentType ${type}`, type === BYTE ? 2e-2 : 1e-4);
  }
});

test("a bufferView in another buffer is read from that buffer", () => {
  const file = gltf([{ name: "Camera", camera: 0 }], { channels: [{ node: 0, path: "translation", times: [0, 1], values: [[0, 0, 0], [7, 8, 9]], buffer: 1 }] });
  close(gltfCameraPath(file).at(1).pos, [7, 8, 9], "position");
});
