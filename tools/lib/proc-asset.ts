// Procedural geometry files (renderer/src/procasset.rs; docs/MATERIAL_FUNCTIONS.md, "Procedural geometry files"): boxes
// and data words written for `ProceduralGeometry.fromAsset`, little-endian, with the SHA-256 of the payload. Offline
// tools (Node) build them; the renderer checks every field when it reads one.
import { createHash } from "node:crypto";

export const MAGIC = "TRTPROC\0";
export const VERSION = 1;

/** The file of `boxes` (min xyz, max xyz each) and `data` (u32 words). */
export function encodeProcAsset(boxes: Float32Array, data: Uint32Array): Buffer {
  if (boxes.length % 6 !== 0) throw new Error("boxes are six numbers each");
  const payload = Buffer.alloc(boxes.length * 4 + data.length * 4);
  for (let i = 0; i < boxes.length; i++) payload.writeFloatLE(boxes[i], i * 4);
  for (let i = 0; i < data.length; i++) payload.writeUInt32LE(data[i], boxes.length * 4 + i * 4);
  const head = Buffer.alloc(64);
  head.write(MAGIC, 0, "latin1");
  head.writeUInt32LE(VERSION, 8);
  head.writeUInt32LE(0x01020304, 12);
  head.writeUInt32LE(boxes.length / 6, 16);
  head.writeUInt32LE(data.length, 20);
  createHash("sha256").update(payload).digest().copy(head, 32);
  return Buffer.concat([head, payload]);
}
