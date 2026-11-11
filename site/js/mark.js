// The ThreeRT mark, live: "Three bounces" (brand/mark.svg says what it is) with one thing a file cannot do. On hover
// the light path turns inside the mesh: its three bounce points slide along the edges, out to the corners (for a
// moment the path is the mesh) and on to the next midpoints, a third of a turn, leaving afterimages that catch up
// when it stops. It ends as it began, so it can go round again for as long as the pointer stays.
//
//   mark()   the lockup's <img> on every page (a.logo: nav and footer) becomes this drawing; the landing page's large
//            mark (svg[data-mark]) gets the same turn. Without this script both stay as they are, still.

const NS = "http://www.w3.org/2000/svg";
/** The mesh triangle (bottom, top left, right), in the 64 box: the lockup's heavier drawing and the large one. */
const SMALL = [[18.89, 58.5], [5.77, 5.5], [58.23, 20.64]];
const LARGE = [[18.39, 59.5], [4.78, 4.5], [59.22, 20.21]];

const ease = (t) => (t < 0.5 ? 4 * t * t * t : 1 - (-2 * t + 2) ** 3 / 2);
const clamp = (t) => Math.max(0, Math.min(1, t));
/** The triangle inscribed in V whose corners have each gone `u` of the way along their edge (0.5: the light path at
 *  rest; 1: the mesh itself; 1.5: the path again, a third of a turn on). */
function inscribed(V, u) {
  const at = (s) => {
    s %= 3;
    const i = Math.floor(s), f = s - i, a = V[i], b = V[(i + 1) % 3];
    return `${(a[0] + (b[0] - a[0]) * f).toFixed(2)} ${(a[1] + (b[1] - a[1]) * f).toFixed(2)}`;
  };
  return `M${at(u)} L${at(1 + u)} L${at(2 + u)} Z`;
}

/**
 * Turns the light path of `svg` while the pointer is over `over`.
 *   [data-ray]         paths drawn along the light path (they all follow it)
 *   [data-ray="main"]  the one the afterimages copy their stroke from
 *   .trail             the <g> the afterimages go in
 * opts: V (the mesh), ghosts, lag (of each afterimage behind the one before, as a share of the turn), ms (a turn),
 *       lit(main, k): called with k in 0..1 (how far from rest), for what else changes while it turns.
 */
function turn(svg, over, { V, ghosts, lag, ms, lit }) {
  const rays = [...svg.querySelectorAll("[data-ray]")], main = svg.querySelector('[data-ray="main"]'), trail = svg.querySelector(".trail");
  const width = Number(main.getAttribute("stroke-width"));
  const gs = Array.from({ length: ghosts }, () => {
    const p = document.createElementNS(NS, "path");
    for (const [k, v] of Object.entries({ fill: "none", stroke: main.getAttribute("stroke"), "stroke-width": (width * 0.6).toFixed(2), "stroke-linejoin": "round", display: "none" })) p.setAttribute(k, v);
    trail.append(p);
    return p;
  });
  const rest = inscribed(V, 0.5), whole = 1 + ghosts * lag;
  let t0 = 0, raf = 0, hovering = false;
  function frame(now) {
    const tau = (now - t0) / ms;
    if (tau >= whole) {
      for (const p of rays) p.setAttribute("d", rest);
      for (const g of gs) g.setAttribute("display", "none");
      lit?.(main, 0);
      raf = 0;
      if (hovering) start();
      return;
    }
    const e = ease(clamp(tau)), d = inscribed(V, 0.5 + e);
    for (const p of rays) p.setAttribute("d", d);
    gs.forEach((g, k) => {
      const ek = ease(clamp(tau - (k + 1) * lag));
      if (e - ek < 0.003) return g.setAttribute("display", "none");
      g.removeAttribute("display");
      g.setAttribute("d", inscribed(V, 0.5 + ek));
      g.setAttribute("opacity", (0.6 * (1 - k / ghosts) ** 1.5).toFixed(3));
    });
    lit?.(main, Math.sin(Math.PI * clamp(tau)));
    raf = requestAnimationFrame(frame);
  }
  function start() {
    if (raf) return;
    t0 = performance.now();
    raf = requestAnimationFrame(frame);
  }
  over.addEventListener("pointerenter", () => { hovering = true; start(); });
  over.addEventListener("pointerleave", () => { hovering = false; });   // the turn in progress finishes
  over.addEventListener("focus", () => start());
}

let serial = 0;
/** The lockup's mark as an inline drawing (the small one: heavier strokes, the path filled). */
function lockupMark() {
  const id = `mark-ray-${serial++}`;
  const svg = document.createElementNS(NS, "svg");
  svg.setAttribute("viewBox", "0 0 64 64");
  svg.setAttribute("class", "mark");
  svg.setAttribute("aria-hidden", "true");
  svg.innerHTML =
    `<defs><linearGradient id="${id}" x1="12.33" y1="32" x2="32" y2="13.07" gradientUnits="userSpaceOnUse"><stop offset="0" style="stop-color:var(--ember)"/><stop offset=".5" style="stop-color:var(--tungsten)"/><stop offset="1" style="stop-color:var(--filament)"/></linearGradient></defs>` +
    `<path d="${inscribed(SMALL, 0)}" fill="none" style="stroke:var(--bone)" stroke-width="5.2" stroke-linejoin="round"/>` +
    `<g class="trail"></g>` +
    `<path data-ray="main" d="${inscribed(SMALL, 0.5)}" fill="url(#${id})" stroke="url(#${id})" stroke-width="4.4" stroke-linejoin="round"/>`;
  return svg;
}

export function mark(root = document) {
  const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
  for (const img of root.querySelectorAll("a.logo img")) {
    const svg = lockupMark();
    img.replaceWith(svg);
    // At rest the path is a filled shape; turning, it thins to its outline so the mesh and the afterimages show.
    if (!still) turn(svg, svg.closest("a.logo"), { V: SMALL, ghosts: 5, lag: 0.055, ms: 1050, lit: (main, k) => main.setAttribute("fill-opacity", (1 - 0.88 * Math.min(1, k * 2.4)).toFixed(3)) });
  }
  for (const svg of root.querySelectorAll("svg[data-mark]")) {
    if (still) continue;
    const base = Number(svg.querySelector('[data-ray="main"]').getAttribute("opacity") ?? 1);
    turn(svg, svg, { V: LARGE, ghosts: 10, lag: 0.034, ms: 1700, lit: (main, k) => main.setAttribute("opacity", (base + (1 - base) * Math.min(1, k * 1.6)).toFixed(3)) });
  }
}
