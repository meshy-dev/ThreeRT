// Builds the ThreeRT Playground: the PocketJS UI bundle (ui/ -> ui/dist/main.{js,pak}), the native host, and a
// macOS app bundle (dist/ThreeRT Playground.app).
//   bun playground/build.ts            POCKETJS_DIR=<checkout> to use an existing PocketJS clone
import { $ } from "bun";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const dir = import.meta.dir;
const home = process.env.HOME!;
$.env({ ...process.env, PATH: `${home}/.cargo/bin:${home}/.bun/bin:${process.env.PATH}` });
$.cwd(dir);
const PJS_REV = "2da29c19d23e0231b193a3da9c8d497d43abf81d";
const pjs = join(dir, "vendor/pocketjs");
if (!existsSync(pjs)) {
  mkdirSync(join(dir, "vendor"), { recursive: true });
  if (process.env.POCKETJS_DIR) symlinkSync(process.env.POCKETJS_DIR, pjs);
  else {
    await $`git clone https://github.com/pocket-stack/pocketjs.git ${pjs}`;
    await $`git -C ${pjs} checkout -q ${PJS_REV}`;
    await $`bun install`.cwd(pjs);
  }
}
const ui = join(dir, "ui");
// UI: Solid TSX -> QuickJS bundle + pak (styles, baked fonts, SVG icons) at 2x raster density for Retina.
await $`bun tools/build.ts ${join(ui, "main.tsx")} --density=2 --outdir=${join(ui, "dist")} --strict-classes`.cwd(pjs);
// Native host (links the threert renderer as a library).
await $`cargo build --release`;
// Scene bundles + catalog for the content browser (out/bundles/*, UE5 references, benchmark rows).
await $`node tools/build-scenes.ts > /dev/null && node tools/catalog.ts > /dev/null`.cwd(join(dir, "..")).nothrow();
// macOS app bundle.
const app = join(dir, "dist/ThreeRT Playground.app");
rmSync(app, { recursive: true, force: true });
mkdirSync(join(app, "Contents/MacOS"), { recursive: true });
mkdirSync(join(app, "Contents/Resources/ui"), { recursive: true });
// App icon: the Pocket3D mark on the macOS icon grid (icon/AppIcon-1024.png: 824 px body centred in 1024, with shadow).
const iconset = join(mkdtempSync(join(tmpdir(), "threert-icon-")), "AppIcon.iconset");
mkdirSync(iconset);
for (const s of [16, 32, 128, 256, 512]) {
  await $`sips -z ${s} ${s} icon/AppIcon-1024.png --out ${join(iconset, `icon_${s}x${s}.png`)}`.quiet();
  await $`sips -z ${s * 2} ${s * 2} icon/AppIcon-1024.png --out ${join(iconset, `icon_${s}x${s}@2x.png`)}`.quiet();
}
await $`iconutil -c icns ${iconset} -o ${join(app, "Contents/Resources/AppIcon.icns")}`;
copyFileSync(join(dir, "target/release/threert-playground"), join(app, "Contents/MacOS/threert-playground"));
await $`chmod +x ${join(app, "Contents/MacOS/threert-playground")}`;
for (const f of ["main.js", "main.pak"]) copyFileSync(join(ui, "dist", f), join(app, "Contents/Resources/ui", f));
writeFileSync(join(app, "Contents/Resources/node-path"), (await $`which node`.text()).trim() + "\n");
writeFileSync(join(app, "Contents/Info.plist"), `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>ThreeRT Playground</string>
  <key>CFBundleDisplayName</key><string>ThreeRT Playground</string>
  <key>CFBundleIdentifier</key><string>ai.meshy.threert.playground</string>
  <key>CFBundleExecutable</key><string>threert-playground</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>LSMinimumSystemVersion</key><string>14.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSRequiresAquaSystemAppearance</key><false/>
</dict></plist>
`);
console.log(`built ${app}`);
