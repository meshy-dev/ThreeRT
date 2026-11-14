import { test } from "node:test";
import { strict as assert } from "node:assert";
import worker, { clientFile } from "./worker.ts";
import { releaseArtifacts } from "./tools/releases.ts";

test("Windows downloads are attachments from R2, with HEAD and resume support", async () => {
  const env = {
    ASSETS: { fetch: async () => new Response("unexpected asset", { status: 500 }) },
    RELEASES: {
      head: async () => ({ size: 100, httpEtag: '"abc"', writeHttpMetadata() {} }),
      get: async (_: string, options: any) => ({
        size: 100, httpEtag: '"abc"', writeHttpMetadata() {},
        range: options.range.has("range") ? { offset: 10, length: 3 } : undefined,
        body: new Blob([new Uint8Array(options.range.has("range") ? 3 : 100)]).stream(),
      }),
    },
  };
  for (const [name, type] of [["ThreeRT-0.4.3-windows-x64-setup.exe", "application/vnd.microsoft.portable-executable"], ["ThreeRT-0.4.3-windows-x64.zip", "application/zip"], ["ThreeRT-0.4.3-windows-x64-SHA256SUMS.txt", "text/plain; charset=utf-8"]]) {
    const url = `https://threert.dev/releases/${name}`;
    const head = await worker.fetch(new Request(url, { method: "HEAD" }), env);
    assert.equal(head.status, 200);
    assert.equal(head.headers.get("content-type"), type);
    assert.equal(head.headers.get("content-disposition"), `attachment; filename="${name}"`);
    assert.equal(head.headers.get("content-length"), "100");
    assert.equal(await head.text(), "");
    const range = await worker.fetch(new Request(url, { headers: { Range: "bytes=10-12" } }), env);
    assert.equal(range.status, 206);
    assert.equal(range.headers.get("content-range"), "bytes 10-12/100");
    assert.equal((await range.arrayBuffer()).byteLength, 3);
    assert.equal((await worker.fetch(new Request(url, { method: "POST" }), env)).status, 405);
  }
  assert.equal(clientFile("/releases/../secret.exe"), null);
  assert.equal(clientFile("/releases/config.txt"), null);
});

test("the site validates and uploads all three Windows artifacts", () => {
  const windows = { file: "a.exe", portable: { file: "a.zip" }, checksums: { file: "a-SHA256SUMS.txt" } };
  assert.deepEqual(releaseArtifacts({ windows }).filter(a => a.key.startsWith("windows")).map(a => a.entry.file), ["a.exe", "a.zip", "a-SHA256SUMS.txt"]);
});
