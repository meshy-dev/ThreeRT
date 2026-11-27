// tools/lib/chrome.ts: CHROME first, then the macOS app, then Chrome or Chromium on PATH (Linux, #73).
import { test } from "node:test";
import assert from "node:assert/strict";
import { findChrome } from "../chrome.ts";

const MAC = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const only = (...files: string[]) => (p: string) => files.includes(p);

test("CHROME wins, as given", () => {
  assert.equal(findChrome({ env: { CHROME: "/opt/chrome/chrome", PATH: "/usr/bin" }, platform: "linux", exists: only("/usr/bin/chromium") }), "/opt/chrome/chrome");
});

test("macOS: the app bundle", () => {
  assert.equal(findChrome({ env: { PATH: "/usr/bin" }, platform: "darwin", exists: only(MAC) }), MAC);
});

test("Linux: google-chrome or chromium on PATH, in PATH order", () => {
  const env = { PATH: "/usr/local/bin:/usr/bin:/snap/bin" };
  assert.equal(findChrome({ env, platform: "linux", exists: only("/usr/bin/google-chrome-stable", "/snap/bin/chromium") }), "/usr/bin/google-chrome-stable");
  assert.equal(findChrome({ env, platform: "linux", exists: only("/snap/bin/chromium") }), "/snap/bin/chromium");
  assert.equal(findChrome({ env, platform: "linux", exists: only("/usr/bin/chromium-browser") }), "/usr/bin/chromium-browser");
});

test("Linux never answers with the macOS path", () => {
  assert.equal(findChrome({ env: { PATH: "/usr/bin" }, platform: "linux", exists: only(MAC) }), null);
});

test("none found: null", () => {
  assert.equal(findChrome({ env: {}, platform: "linux", exists: () => false }), null);
  assert.equal(findChrome({ env: { PATH: "/usr/bin" }, platform: "darwin", exists: () => false }), null);
});
