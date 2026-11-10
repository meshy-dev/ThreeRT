// Dependency-free binary encoders that run unchanged in Node, Workers and the render node's engine: GLB packing
// (asset tools), PNG (stored deflate, generated textures) and a 64-bit content hash.

export function packGLB(json: unknown, bin: Uint8Array): Uint8Array {
  const jsonBytes = new TextEncoder().encode(JSON.stringify(json));
  const jsonLen = align4(jsonBytes.length);
  const binLen = align4(bin.length);
  const total = 12 + 8 + jsonLen + 8 + binLen;
  const out = new Uint8Array(total);
  const dv = new DataView(out.buffer);
  dv.setUint32(0, 0x46546c67, true);
  dv.setUint32(4, 2, true);
  dv.setUint32(8, total, true);
  dv.setUint32(12, jsonLen, true);
  dv.setUint32(16, 0x4e4f534a, true);
  out.set(jsonBytes, 20);
  out.fill(0x20, 20 + jsonBytes.length, 20 + jsonLen);
  dv.setUint32(20 + jsonLen, binLen, true);
  dv.setUint32(24 + jsonLen, 0x004e4942, true);
  out.set(bin, 28 + jsonLen);
  return out;
}

const align4 = (n: number) => (n + 3) & ~3;

/** RGBA8 -> PNG using stored (uncompressed) deflate blocks. */
export function encodePNG(width: number, height: number, rgba: Uint8Array): Uint8Array {
  const stride = width * 4;
  const raw = new Uint8Array((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0;
    raw.set(rgba.subarray(y * stride, (y + 1) * stride), y * (stride + 1) + 1);
  }
  const blocks = Math.max(1, Math.ceil(raw.length / 65535));
  const z = new Uint8Array(2 + raw.length + blocks * 5 + 4);
  z[0] = 0x78;
  z[1] = 0x01;
  let o = 2;
  for (let b = 0; b < blocks; b++) {
    const chunk = raw.subarray(b * 65535, Math.min(raw.length, (b + 1) * 65535));
    z[o++] = b === blocks - 1 ? 1 : 0;
    z[o++] = chunk.length & 255;
    z[o++] = chunk.length >> 8;
    z[o++] = ~chunk.length & 255;
    z[o++] = (~chunk.length >> 8) & 255;
    z.set(chunk, o);
    o += chunk.length;
  }
  let a = 1, bsum = 0;
  for (let i = 0; i < raw.length; i++) {
    a = (a + raw[i]) % 65521;
    bsum = (bsum + a) % 65521;
  }
  new DataView(z.buffer).setUint32(o, ((bsum << 16) | a) >>> 0);

  const ihdr = new Uint8Array(13);
  const dv = new DataView(ihdr.buffer);
  dv.setUint32(0, width);
  dv.setUint32(4, height);
  ihdr.set([8, 6, 0, 0, 0], 8);
  const chunks = [pngChunk("IHDR", ihdr), pngChunk("IDAT", z), pngChunk("IEND", new Uint8Array())];
  const sig = [137, 80, 78, 71, 13, 10, 26, 10];
  const out = new Uint8Array(8 + chunks.reduce((s, c) => s + c.length, 0));
  out.set(sig, 0);
  let p = 8;
  for (const c of chunks) {
    out.set(c, p);
    p += c.length;
  }
  return out;
}

function pngChunk(type: string, data: Uint8Array): Uint8Array {
  const out = new Uint8Array(12 + data.length);
  const dv = new DataView(out.buffer);
  dv.setUint32(0, data.length);
  for (let i = 0; i < 4; i++) out[4 + i] = type.charCodeAt(i);
  out.set(data, 8);
  dv.setUint32(8 + data.length, crc32(out.subarray(4, 8 + data.length)));
  return out;
}

let crcTable: Uint32Array | undefined;
export function crc32(bytes: Uint8Array): number {
  if (!crcTable) {
    crcTable = new Uint32Array(256);
    for (let n = 0; n < 256; n++) {
      let c = n;
      for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
      crcTable[n] = c >>> 0;
    }
  }
  let c = 0xffffffff;
  for (let i = 0; i < bytes.length; i++) c = crcTable[(c ^ bytes[i]) & 255] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** 64-bit (2x murmur3_32) content hash as 16 hex chars. Not cryptographic; used for content addressing. */
export function contentHash(bytes: Uint8Array): string {
  return murmur3(bytes, 0x9747b28c).toString(16).padStart(8, "0") + murmur3(bytes, 0x5bd1e995).toString(16).padStart(8, "0");
}

function murmur3(key: Uint8Array, seed: number): number {
  let h = seed >>> 0;
  const n = key.length & ~3;
  const dv = new DataView(key.buffer, key.byteOffset, key.byteLength);
  for (let i = 0; i < n; i += 4) {
    let k = dv.getUint32(i, true);
    k = Math.imul(k, 0xcc9e2d51);
    k = (k << 15) | (k >>> 17);
    k = Math.imul(k, 0x1b873593);
    h ^= k;
    h = (h << 13) | (h >>> 19);
    h = (Math.imul(h, 5) + 0xe6546b64) | 0;
  }
  let k = 0;
  switch (key.length & 3) {
    case 3: k ^= key[n + 2] << 16; // falls through
    case 2: k ^= key[n + 1] << 8; // falls through
    case 1:
      k ^= key[n];
      k = Math.imul(k, 0xcc9e2d51);
      k = (k << 15) | (k >>> 17);
      k = Math.imul(k, 0x1b873593);
      h ^= k;
  }
  h ^= key.length;
  h ^= h >>> 16;
  h = Math.imul(h, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  h ^= h >>> 16;
  return h >>> 0;
}
