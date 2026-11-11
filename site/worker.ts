// The site on Cloudflare Workers: static assets (out/site, built by site/tools/build.ts) on threert.dev, and the
// clients' disk images from R2. www.threert.dev redirects to the apex.
//
// Why R2: the disk image (releases/ThreeRT-<version>-macos-arm64.dmg, about 32 MiB with the `threert` command line in
// it) is over the 25 MiB a file that static assets take, and the client is closed source, so it has no public
// repository whose releases could carry it. The bucket holds the images under their file names; this Worker serves
// them at the same URL the Download page already links, /releases/<file>, with ranges (a resumed download).
interface R2Range {
  offset?: number;
  length?: number;
  suffix?: number;
}
interface R2Object {
  size: number;
  httpEtag: string;
  range?: R2Range;
  writeHttpMetadata(headers: Headers): void;
}
interface R2ObjectBody extends R2Object {
  body: ReadableStream;
}
interface R2Bucket {
  head(key: string): Promise<R2Object | null>;
  get(key: string, options?: { range?: Headers; onlyIf?: Headers }): Promise<R2ObjectBody | R2Object | null>;
}
interface Env {
  ASSETS: { fetch(request: Request): Promise<Response> };
  RELEASES: R2Bucket;
}

/** /releases/<name>.dmg -> the bucket key (the file name), or null for everything else. */
export function imageKey(pathname: string): string | null {
  return /^\/releases\/([A-Za-z0-9][A-Za-z0-9._-]*\.dmg)$/.exec(pathname)?.[1] ?? null;
}

async function image(request: Request, env: Env, key: string): Promise<Response> {
  if (request.method !== "GET" && request.method !== "HEAD") return new Response("method not allowed", { status: 405, headers: { allow: "GET, HEAD" } });
  const headers = new Headers({
    "content-type": "application/x-apple-diskimage",
    "content-disposition": `attachment; filename="${key}"`,
    "accept-ranges": "bytes",
    // An hour, then revalidated by ETag: a build may be replaced under its name before its version is final.
    "cache-control": "public, max-age=3600",
  });
  if (request.method === "HEAD") {
    const head = await env.RELEASES.head(key);
    if (!head) return new Response("not found", { status: 404 });
    headers.set("etag", head.httpEtag);
    headers.set("content-length", String(head.size));
    return new Response(null, { headers });
  }
  const object = await env.RELEASES.get(key, { range: request.headers, onlyIf: request.headers });
  if (!object) return new Response("not found", { status: 404 });
  headers.set("etag", object.httpEtag);
  // A conditional request the object did not pass (If-None-Match on a cached copy): no body.
  if (!("body" in object)) return new Response(null, { status: 304, headers });
  const r = object.range;
  if (request.headers.has("range") && r) {
    const start = r.suffix !== undefined ? Math.max(0, object.size - r.suffix) : (r.offset ?? 0);
    const length = r.suffix !== undefined ? object.size - start : (r.length ?? object.size - start);
    headers.set("content-range", `bytes ${start}-${start + length - 1}/${object.size}`);
    headers.set("content-length", String(length));
    return new Response(object.body, { status: 206, headers });
  }
  headers.set("content-length", String(object.size));
  return new Response(object.body, { headers });
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (url.hostname === "www.threert.dev") {
      url.hostname = "threert.dev";
      return Response.redirect(url.toString(), 301);
    }
    const key = imageKey(url.pathname);
    if (key) return image(request, env, key);
    return env.ASSETS.fetch(request);
  },
};
