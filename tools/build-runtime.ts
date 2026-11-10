// Bundles @threert/scene + @threert/gameplay into one ES module, out/threert-runtime.js: the runtime a game written as
// an ES module imports (`threert-render render|play <game.js> --runtime out/threert-runtime.js`). Game imports of
// @threert/* are rewritten to it, so game code and the runtime share one copy of the DSL classes (instanceof works).
//   node tools/build-runtime.ts
import { build } from "esbuild";
import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const outDir = join(import.meta.dirname, "..", "out");
const t0 = performance.now();
const result = await build({
  stdin: {
    contents: 'export * from "@threert/scene";\nexport * from "@threert/gameplay";\n',
    resolveDir: import.meta.dirname,
    sourcefile: "threert-runtime.js",
  },
  bundle: true,
  format: "esm",
  platform: "neutral",
  target: "es2023",
  minifySyntax: true,
  minifyWhitespace: true,
  legalComments: "none",
  metafile: true,
  write: false,
  logLevel: "warning",
});
const code = result.outputFiles[0].text;
const exported = Object.values(result.metafile.outputs)[0].exports;
mkdirSync(outDir, { recursive: true });
writeFileSync(join(outDir, "threert-runtime.js"), code);
console.log(`threert-runtime.js: ${(code.length / 1024).toFixed(0)} KiB, ${exported.length} exports, ${(performance.now() - t0).toFixed(0)} ms`);
