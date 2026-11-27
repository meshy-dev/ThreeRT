// tools/analytic/kernel.ts mirrors the path tracer's functions; this holds it to them. Each source function's text
// (comments and spacing aside) is hashed and compared with kernel.ts `PINS`: a change to one fails here until the
// mirror is updated and the pin with it (the failure prints the new hash).
import { test } from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { PINS } from "../kernel.ts";

const root = fileURLToPath(new URL("../../../", import.meta.url));

/** A function's text from `fn name(` (WGSL) or `pub fn name(` (Rust) to its closing brace. */
export function source(text: string, name: string) {
  const at = text.search(new RegExp(`^(pub )?fn ${name}\\(`, "m"));
  if (at < 0) return undefined;
  let depth = 0;
  for (let i = text.indexOf("{", at); i < text.length; i++) {
    if (text[i] === "{") depth++;
    else if (text[i] === "}" && --depth === 0) return text.slice(at, i + 1);
  }
  return undefined;
}

export const fingerprint = (fn: string) =>
  createHash("sha256").update(fn.replace(/\/\/.*$/gm, "").replace(/\s+/g, " ").trim()).digest("hex").slice(0, 16);

test("the mirrored kernel functions are the kernel's", () => {
  const stale: string[] = [];
  for (const [file, pins] of Object.entries(PINS)) {
    const text = readFileSync(root + file, "utf8");
    for (const [name, pin] of Object.entries(pins)) {
      const fn = source(text, name);
      assert.ok(fn, `${file}: no fn ${name}`);
      const now = fingerprint(fn);
      if (now !== pin) stale.push(`${file} fn ${name}: ${now} (pinned ${pin})`);
    }
  }
  assert.deepEqual(stale, [], "changed since tools/analytic/kernel.ts mirrored them: update the mirror and its PINS");
});
