// The three.js files the page imports (site/index.html's import map: "three" and "three/addons/"), served from the
// repo's own three devDependency so no copy is committed. serve.ts maps these URLs; build.ts copies the files.
import { join, resolve } from "node:path";

const three = resolve(import.meta.dir, "../../node_modules/three");

export const VENDOR: Record<string, string> = {
  "vendor/three/three.module.js": join(three, "build/three.module.js"),
  "vendor/three/three.core.js": join(three, "build/three.core.js"),
  "vendor/three/addons/RoomEnvironment.js": join(three, "examples/jsm/environments/RoomEnvironment.js"),
  "vendor/three/addons/RoundedBoxGeometry.js": join(three, "examples/jsm/geometries/RoundedBoxGeometry.js"),
  "vendor/three/LICENSE": join(three, "LICENSE"),
};
