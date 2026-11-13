// threert.app on the landing page: which of the platform's apps it shows, and what a tile of one looks like. No DOM
// and no imports, so the page (live, from the platform's API) and site/tools/platform.ts (which bakes the same tiles
// into index.html, for a page that is complete without JavaScript or the platform) build the very same markup.
//
// The API (CORS-open for GET): GET https://threert.app/api/v1/apps?sort=new|top|playing&kind=game|app&limit=N ->
//   {items: [{name, title, summary, kind, tags, author: {login, name, firstParty}, cover, urls: {page, base, open},
//             stats: {plays, likes, playing}, multiplayer?, webPlayable}], next}
// and GET /api/v1/apps/<name> -> {app: {..., screenshots, listed}}.

export const PLATFORM = "https://threert.app";
/** The game the page gives its own section: left out of the tiles under it. */
export const FEATURED = "openstrike";
/** Tiles in the grid; the eighth cell is the invitation to publish. */
export const TILES = 7;
/** The list the tiles are picked from: the platform's "popular", more than the grid holds (some are left out). */
export const LIST = `${PLATFORM}/api/v1/apps?sort=top&limit=24`;

export const esc = (s) => String(s ?? "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const https = (u) => typeof u === "string" && /^https:\/\/[^\s"'<>]+$/.test(u);

/**
 * The apps the grid shows, in the platform's order: listed apps with a cover and a page, without the featured game
 * (it is the section above) and without the starters (`starter`: the templates a project begins from, which the
 * page's hello world already shows).
 */
export function pick(items, n = TILES) {
  return (Array.isArray(items) ? items : [])
    .filter((a) => a && typeof a.name === "string" && a.title && https(a.cover) && https(a.urls?.page))
    .filter((a) => a.name !== FEATURED && !(a.tags ?? []).includes("starter"))
    .slice(0, n);
}

/** "App · education": the kind and the first tag that says what it is for. */
export function label(app) {
  const generic = new Set(["showcase", "starter", "scene", "game", "app", "first-party"]);
  const field = (app.tags ?? []).find((t) => !generic.has(t));
  return (app.kind === "game" ? "Game" : "App") + (field ? ` · ${String(field).replace(/-/g, " ")}` : "");
}

/** The app's address as people say it: jewellery.threert.app */
export const address = (app) => app.urls.page.replace(/^https:\/\//, "").replace(/\/$/, "");

/** A tile. `img`: the picture's URL (a copy on this site when there is one, else the platform's cover). */
export function tile(app, img) {
  const playing = app.stats?.playing > 0 ? `<i class="now">${esc(app.stats.playing)} playing</i>` : "";
  return `<a class="app" href="${esc(app.urls.page)}" rel="noopener" data-name="${esc(app.name)}" data-cover="${esc(app.cover)}">` +
    `<span class="pic"><img src="${esc(img)}" width="640" height="360" alt="${esc(app.title)}, rendered by ThreeRT" loading="lazy" decoding="async">${playing}</span>` +
    `<span class="t"><small>${esc(label(app))}</small><b>${esc(app.title)}</b><span>${esc(app.summary)}</span><em>${esc(address(app))}</em></span></a>`;
}
