// The files the pages import from npm packages: three.js (site/index.html's import map: "three" and "three/addons/")
// and the QR encoder (js/site.js), served from the repo's own devDependencies so no copy is committed. serve.ts maps
// these URLs; build.ts copies the files.
import { join, resolve } from "node:path";

const modules = resolve(import.meta.dir, "../../node_modules");
const three = join(modules, "three");

export const VENDOR: Record<string, string> = {
  "vendor/three/three.module.js": join(three, "build/three.module.js"),
  "vendor/three/three.core.js": join(three, "build/three.core.js"),
  "vendor/three/addons/RoomEnvironment.js": join(three, "examples/jsm/environments/RoomEnvironment.js"),
  "vendor/three/addons/RoundedBoxGeometry.js": join(three, "examples/jsm/geometries/RoundedBoxGeometry.js"),
  "vendor/three/LICENSE": join(three, "LICENSE"),
  // qrcode-generator (Kazuhiko Arase, MIT: the licence is the file's header).
  "vendor/qrcode.mjs": join(modules, "qrcode-generator/dist/qrcode.mjs"),
};
