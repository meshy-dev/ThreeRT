// How the showroom's product files were made from the Khronos glTF sample assets (see CREDITS.md): each original
// .glb is read as data, its textures are scaled to at most 1024 px and stored as JPEG (PNG where there is alpha), and
// KHR_materials_variants (which the engine does not read) is turned into one named node per colourway, so that scene
// code can pick a colourway with `new GLTFModel(uri, { nodes: [...] })`. Geometry is untouched.
//
//   node showcase/showroom/repack-assets.mjs <dir with the original .glb files, <Name>/<Name>.glb> <output dir>
//
// Originals: https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/main/Models/<Name>/glTF-Binary/<Name>.glb
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import sharp from "sharp";

const [src, out] = process.argv.slice(2);
if (!src || !out) throw new Error("usage: node repack-assets.mjs <originals dir> <output dir>");

function readGlb(path) {
  const b = readFileSync(path);
  if (b.readUInt32LE(0) !== 0x46546c67) throw new Error(`${path}: not a GLB`);
  let off = 12, json = null, bin = null;
  while (off < b.length) {
    const len = b.readUInt32LE(off), type = b.readUInt32LE(off + 4);
    const data = b.subarray(off + 8, off + 8 + len);
    if (type === 0x4e4f534a) json = JSON.parse(data.toString("utf8"));
    else if (type === 0x004e4942) bin = data;
    off += 8 + len;
  }
  return { json, bin };
}

/** Writes a GLB of `json` whose bufferViews' bytes are `views[i]` (unused views dropped by the caller). */
function writeGlb(path, json, views) {
  const parts = [];
  let len = 0;
  json.bufferViews = views.map((v, i) => {
    const pad = (4 - (len % 4)) % 4;
    if (pad) (parts.push(Buffer.alloc(pad)), (len += pad));
    const bv = { ...json.bufferViews[i], buffer: 0, byteOffset: len, byteLength: v.length };
    parts.push(v);
    len += v.length;
    return bv;
  });
  const padEnd = (4 - (len % 4)) % 4;
  if (padEnd) (parts.push(Buffer.alloc(padEnd)), (len += padEnd));
  json.buffers = [{ byteLength: len }];
  let js = Buffer.from(JSON.stringify(json), "utf8");
  if (js.length % 4) js = Buffer.concat([js, Buffer.alloc(4 - (js.length % 4), 0x20)]);
  const head = Buffer.alloc(12);
  head.writeUInt32LE(0x46546c67, 0);
  head.writeUInt32LE(2, 4);
  head.writeUInt32LE(12 + 8 + js.length + 8 + len, 8);
  const h1 = Buffer.alloc(8), h2 = Buffer.alloc(8);
  h1.writeUInt32LE(js.length, 0);
  h1.writeUInt32LE(0x4e4f534a, 4);
  h2.writeUInt32LE(len, 0);
  h2.writeUInt32LE(0x004e4942, 4);
  const all = Buffer.concat([head, h1, js, h2, ...parts]);
  writeFileSync(path, all);
  return all.length;
}

/**
 * `edit(json)` changes the document (nodes, materials); then the images of materials that nothing in the scene draws
 * any more are emptied and the rest re-encoded. `flat`: image index -> [r, g, b] to replace an image with one colour.
 */
async function repack(name, outName, { edit, max = 1024, quality = 88, flat = {} } = {}) {
  const { json: j, bin } = readGlb(join(src, name, `${name}.glb`));
  const views = j.bufferViews.map((bv) => Buffer.from(bin.subarray(bv.byteOffset ?? 0, (bv.byteOffset ?? 0) + bv.byteLength)));
  edit?.(j);
  j.asset.extras = { ...(j.asset.extras ?? {}), modified: `Repacked from KhronosGroup/glTF-Sample-Assets Models/${name} for the ThreeRT showroom showcase: textures at most ${max} px, see CREDITS.md` };
  // Materials that a mesh still draws, textures those use, images those use.
  const drawn = new Set();
  const visit = (n) => (j.nodes[n].mesh !== undefined && drawn.add(j.nodes[n].mesh), (j.nodes[n].children ?? []).forEach(visit));
  j.scenes[j.scene ?? 0].nodes.forEach(visit);
  const usedMat = new Set([...drawn].flatMap((m) => j.meshes[m].primitives.map((p) => p.material)).filter((m) => m !== undefined));
  const texOf = (m) => {
    const t = [];
    JSON.stringify(m, (k, v) => (v && typeof v === "object" && typeof v.index === "number" && /[tT]exture$/.test(k) ? (t.push(v.index), v) : v));
    return t;
  };
  const usedImg = new Set([...usedMat].flatMap((m) => texOf(j.materials[m])).map((t) => j.textures[t].source));
  for (let i = 0; i < (j.images ?? []).length; i++) {
    const im = j.images[i];
    if (!usedImg.has(i)) {
      // Not drawn any more: a 2 x 2 grey stand-in keeps the file valid.
      views[im.bufferView] = await sharp({ create: { width: 2, height: 2, channels: 3, background: { r: 128, g: 128, b: 128 } } }).jpeg().toBuffer();
      im.mimeType = "image/jpeg";
      continue;
    }
    let img = flat[i] ? sharp({ create: { width: 8, height: 8, channels: 3, background: { r: flat[i][0], g: flat[i][1], b: flat[i][2] } } }) : sharp(views[im.bufferView]).resize(max, max, { fit: "inside", withoutEnlargement: true });
    const meta = flat[i] ? { hasAlpha: false } : await sharp(views[im.bufferView]).metadata();
    const opaque = !meta.hasAlpha || (await sharp(views[im.bufferView]).stats()).channels[3].min === 255;
    if (opaque) {
      views[im.bufferView] = await img.removeAlpha().jpeg({ quality, chromaSubsampling: "4:4:4", mozjpeg: true }).toBuffer();
      im.mimeType = "image/jpeg";
    } else {
      views[im.bufferView] = await img.png({ compressionLevel: 9 }).toBuffer();
      im.mimeType = "image/png";
    }
  }
  const bytes = writeGlb(join(out, outName), j, views);
  console.log(`${outName}: ${(bytes / 1048576).toFixed(2)} MiB (${name})`);
}

/** KHR_materials_variants -> one node per (node, material) the variants name, called `<prefix><label>`. */
function dropVariants(j) {
  delete j.extensions?.KHR_materials_variants;
  if (j.extensions && !Object.keys(j.extensions).length) delete j.extensions;
  j.extensionsUsed = (j.extensionsUsed ?? []).filter((e) => e !== "KHR_materials_variants");
  for (const m of j.meshes) for (const p of m.primitives) if (p.extensions) (delete p.extensions.KHR_materials_variants, Object.keys(p.extensions).length || delete p.extensions);
}
/** A copy of node `from` (same mesh geometry, another material) named `name`, next to it under the same parent. */
function nodeWithMaterial(j, from, name, material) {
  const n = j.nodes[from];
  const mesh = JSON.parse(JSON.stringify(j.meshes[n.mesh]));
  for (const p of mesh.primitives) p.material = material;
  j.meshes.push(mesh);
  j.nodes.push({ ...n, name, mesh: j.meshes.length - 1, children: undefined });
  const id = j.nodes.length - 1;
  const parent = j.nodes.find((x) => x.children?.includes(from));
  if (parent) parent.children.push(id);
  else j.scenes[j.scene ?? 0].nodes.push(id);
}
const nodeIndex = (j, name) => {
  const i = j.nodes.findIndex((n) => n.name === name);
  if (i < 0) throw new Error(`no node ${name}`);
  return i;
};

// The sneaker: three colourways (diffuse maps) as three nodes.
await repack("MaterialsVariantsShoe", "shoe.glb", {
  edit(j) {
    dropVariants(j);
    const base = nodeIndex(j, "Shoe");
    ["midnight", "beach", "street"].forEach((v, i) => {
      j.materials[i].name = `shoe_${v}`;
      if (i === 0) j.nodes[base].name = "shoe_midnight";
      else nodeWithMaterial(j, base, `shoe_${v}`, i);
    });
  },
});
// The velvet chair: mango / peacock fabric with brown / black wood as nodes; the paper label under the seat left out.
await repack("SheenChair", "chair.glb", {
  edit(j) {
    dropVariants(j);
    const fabric = nodeIndex(j, "SheenChair_fabric"), wood = nodeIndex(j, "SheenChair_wood"), label = nodeIndex(j, "SheenChair_label");
    j.nodes[fabric].name = "chair_fabric_mango";
    j.nodes[wood].name = "chair_wood_brown";
    j.nodes[nodeIndex(j, "SheenChair_metal")].name = "chair_metal";
    nodeWithMaterial(j, fabric, "chair_fabric_peacock", 4);
    nodeWithMaterial(j, wood, "chair_wood_black", 5);
    j.scenes[j.scene ?? 0].nodes = j.scenes[j.scene ?? 0].nodes.filter((n) => n !== label);
  },
});
await repack("IridescenceLamp", "lamp.glb");
// The sunglasses: the ear hooks' texture is two trademark logos (not covered by the licence): replaced by its black.
// The lenses are a clear, index-1 shell under an iridescent mirror film (KHR_materials_iridescence), which the engine
// does not read: as authored they come out as flat black holes. Both lens materials are replaced by what such a lens
// is without its film, dark polished glass that mirrors the room (opaque, index 1.5).
await repack("SunglassesKhronos", "sunglasses.glb", {
  flat: { 0: [28, 28, 28] },
  edit(j) {
    for (const m of j.materials.filter((m) => m.name.startsWith("lens_"))) {
      m.pbrMetallicRoughness = { baseColorFactor: [0.012, 0.012, 0.016, 1], metallicFactor: 0, roughnessFactor: 0.03 };
      m.extensions = { KHR_materials_ior: { ior: 1.5 } };
    }
  },
});
await repack("WaterBottle", "bottle.glb");
// The boom box: its display and lamps are emissive at glTF's 1 (nits here); given a strength that reads in a lit room.
await repack("BoomBox", "boombox.glb", {
  edit(j) {
    j.materials[0].extensions = { ...(j.materials[0].extensions ?? {}), KHR_materials_emissive_strength: { emissiveStrength: 400 } };
    j.extensionsUsed = [...new Set([...(j.extensionsUsed ?? []), "KHR_materials_emissive_strength"])];
  },
});
