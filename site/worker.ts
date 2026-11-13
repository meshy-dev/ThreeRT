// The site on Cloudflare Workers: static assets (out/site, built by site/tools/build.ts) on threert.dev, the clients'
// disk images from R2, and byte ranges for the clips (media/**.mp4), which the assets alone do not answer.
// www.threert.dev redirects to the apex, and the old Showcase page to threert.app.
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

/** The bytes a `Range: bytes=...` header asks of a file of `size` bytes: [start, end] inclusive, "unsatisfiable", or
 *  null when there is no single range to answer (no header, another unit, several ranges: the whole file then). */
export function byteRange(header: string | null, size: number): [number, number] | "unsatisfiable" | null {
  const m = /^bytes=(\d*)-(\d*)$/.exec(header?.trim() ?? "");
  if (!m || (m[1] === "" && m[2] === "")) return null;
  if (m[1] === "") {
    const n = Number(m[2]);
    return n > 0 && size > 0 ? [Math.max(0, size - n), size - 1] : "unsatisfiable";
  }
  const start = Number(m[1]), end = m[2] === "" ? size - 1 : Math.min(Number(m[2]), size - 1);
  return start < size && start <= end ? [start, end] : "unsatisfiable";
}

/** media/**.mp4 -> true: the clips the pages play. */
export const isClip = (pathname: string) => /^\/media\/[A-Za-z0-9._\/-]+\.mp4$/.test(pathname);

/**
 * A clip, with byte ranges. Static assets answer a Range request with the whole file, and Safari (every browser on an
 * iPhone) plays a video only from a server that answers 206: it asks for bytes 0-1 first and gives up otherwise. The
 * clips are a few MiB, so the range is cut out of the asset here.
 */
async function clip(request: Request, env: Env): Promise<Response> {
  const headers = new Headers(request.headers);
  headers.delete("range");
  const asset = await env.ASSETS.fetch(new Request(request.url, { method: request.method, headers }));
  const out = new Headers(asset.headers);
  out.set("accept-ranges", "bytes");
  if (asset.status !== 200 || request.method !== "GET") return new Response(asset.body, { status: asset.status, headers: out });
  if (!request.headers.has("range")) return new Response(asset.body, { headers: out });
  const bytes = await asset.arrayBuffer();
  const range = byteRange(request.headers.get("range"), bytes.byteLength);
  if (range === null) return new Response(bytes, { headers: out });
  if (range === "unsatisfiable") {
    out.set("content-range", `bytes */${bytes.byteLength}`);
    out.delete("content-length");
    return new Response(null, { status: 416, headers: out });
  }
  out.set("content-range", `bytes ${range[0]}-${range[1]}/${bytes.byteLength}`);
  out.set("content-length", String(range[1] - range[0] + 1));
  return new Response(bytes.slice(range[0], range[1] + 1), { status: 206, headers: out });
}

/**
 * The Showcase page's address: /showcase, /showcase/, /showcase/index.html. What it listed is now published on
 * threert.app, so old links (the npm packages' READMEs, the pages of packages built before, earlier apps) go to the
 * catalogue there. Only the page: showcase/community.json stays an asset, because clients before 0.4.0 read it.
 */
export const isShowcase = (pathname: string) => /^\/showcase(\/(index\.html)?)?$/.test(pathname);
export const PLATFORM = "https://threert.app/";

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (url.hostname === "www.threert.dev") {
      url.hostname = "threert.dev";
      return Response.redirect(url.toString(), 301);
    }
    if (isShowcase(url.pathname)) return Response.redirect(PLATFORM, 301);
    const key = imageKey(url.pathname);
    if (key) return image(request, env, key);
    if (isClip(url.pathname)) return clip(request, env);
    return env.ASSETS.fetch(request);
  },
};
