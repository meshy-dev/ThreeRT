// The landing page on Cloudflare Workers: static assets (out/site, built by site/tools/build.ts) on threert.dev.
// www.threert.dev redirects to the apex; everything else is served from the assets as built.
interface Env {
  ASSETS: { fetch(request: Request): Promise<Response> };
}

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    if (url.hostname === "www.threert.dev") {
      url.hostname = "threert.dev";
      return Response.redirect(url.toString(), 301);
    }
    return env.ASSETS.fetch(request);
  },
};
