// ThreeRT landing page: shared components (no dependencies).
//   <div class="cmp" data-a=".." data-b=".." data-la=".." data-lb=".." data-sa=".." data-sb=".." data-x="50">
//   <div class="seqp" data-scene="media/bistro" data-n="60" data-la=".." data-lb="..">
// Wipe comparisons put the baseline (three.js) on the left and ThreeRT on the right, always the same camera and frame.

export const $ = (s, r = document) => r.querySelector(s);
export const $$ = (s, r = document) => [...r.querySelectorAll(s)];

const clamp = (v, a, b) => Math.min(b, Math.max(a, v));
const KNOB = `<svg width="18" height="12" viewBox="0 0 18 12" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"><path d="M5.5 1.5 1 6l4.5 4.5M12.5 1.5 17 6l-4.5 4.5"/></svg>`;
const label = (cls, l, s) => `<div class="lab ${cls}"><i></i><span>${l ?? ""}${s ? `<small>${s}</small>` : ""}</span></div>`;

/** Wipe comparison. Drag (or click, or arrow keys) to move the divider; `el.cmp.set({...})` swaps the pair. */
export function compare(el) {
  const d = el.dataset;
  el.tabIndex = 0;
  el.setAttribute("role", "slider");
  el.setAttribute("aria-valuemin", "0");
  el.setAttribute("aria-valuemax", "100");
  el.innerHTML = `<div class="pa"><img alt="" draggable="false"></div><div class="pb"><img alt="" draggable="false"></div><div class="bar"></div><div class="knob">${KNOB}</div>${label("a")}${label("b")}`;
  const ia = $(".pa img", el), ib = $(".pb img", el);
  let x = +(d.x ?? 50);
  const setX = (v) => {
    x = clamp(v, 0, 100);
    el.style.setProperty("--x", `${x}%`);
    el.setAttribute("aria-valuenow", String(Math.round(x)));
  };
  const set = (o) => {
    Object.assign(d, o);
    if (d.a && (o.a || !ia.getAttribute("src"))) ia.src = d.a;
    if (d.b && (o.b || !ib.getAttribute("src"))) ib.src = d.b;
    $(".lab.a", el).outerHTML = label("a", d.la, d.sa);
    $(".lab.b", el).outerHTML = label("b", d.lb, d.sb);
    el.setAttribute("aria-label", `Comparison: ${d.la} (left) and ${d.lb} (right)`);
    if (o.x !== undefined) setX(+o.x);
  };
  set({});
  setX(x);
  let drag = false;
  const at = (e) => setX(((e.clientX - el.getBoundingClientRect().left) / el.clientWidth) * 100);
  el.addEventListener("pointerdown", (e) => { drag = true; el.setPointerCapture(e.pointerId); at(e); });
  el.addEventListener("pointermove", (e) => { if (drag || d.hover !== undefined) at(e); });
  el.addEventListener("pointerup", () => (drag = false));
  el.addEventListener("pointercancel", () => (drag = false));
  el.addEventListener("keydown", (e) => {
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") { setX(x + (e.key === "ArrowLeft" ? -4 : 4)); e.preventDefault(); }
  });
  // A gentle first sweep so the reader sees it is interactive.
  if (d.intro !== undefined) {
    const io = new IntersectionObserver(([en]) => {
      if (!en.isIntersecting) return;
      io.disconnect();
      const from = x, t0 = performance.now();
      const step = (t) => {
        const k = clamp((t - t0) / 1600, 0, 1);
        if (drag) return;
        setX(from + Math.sin(k * Math.PI * 2) * 14 * (1 - k));
        if (k < 1) requestAnimationFrame(step);
      };
      requestAnimationFrame(step);
    }, { threshold: 0.5 });
    io.observe(el);
  }
  el.cmp = { set, setX, get x() { return x; }, imgs: [ia, ib] };
  return el.cmp;
}

/** Plays the 60-pose camera move of both engines in lockstep, wiped like a comparison. */
export function sequence(el) {
  const d = el.dataset, n = +(d.n ?? 60), fps = +(d.fps ?? 10);
  el.innerHTML = `<canvas></canvas><div class="bar"></div>${label("a", d.la, d.sa)}${label("b", d.lb, d.sb)}<div class="loading">loading 120 frames…</div>
    <div class="ctl"><button class="pp" aria-label="Play / pause"></button><div class="track"><b></b></div><span class="t">0.0 s</span></div>`;
  const cv = $("canvas", el), cx = cv.getContext("2d"), bar = $(".bar", el), pp = $(".pp", el), fill = $(".track b", el), tl = $(".t", el);
  let x = 50, k = 0, dir = 1, playing = true, last = 0, ready = false, frames = null;
  const icon = () => (pp.innerHTML = playing ? `<svg width="12" height="12" viewBox="0 0 12 12"><rect x="2" y="1.5" width="3" height="9" fill="currentColor"/><rect x="7" y="1.5" width="3" height="9" fill="currentColor"/></svg>` : `<svg width="12" height="12" viewBox="0 0 12 12"><path d="M3 1.5v9l7.5-4.5z" fill="currentColor"/></svg>`);
  icon();
  const draw = () => {
    if (!ready) return;
    const w = (cv.width = el.clientWidth * devicePixelRatio), h = (cv.height = el.clientHeight * devicePixelRatio);
    const [three, rt] = [frames.three[k], frames.rt[k]];
    cx.drawImage(three, 0, 0, w, h);
    const cut = (x / 100) * w;
    cx.drawImage(rt, (cut / w) * rt.naturalWidth, 0, rt.naturalWidth * (1 - cut / w), rt.naturalHeight, cut, 0, w - cut, h);
    bar.style.left = `${x}%`;
    fill.style.width = `${(k / (n - 1)) * 100}%`;
    tl.textContent = `${(k / fps).toFixed(1)} s`;
  };
  const load = () => {
    const pad = (i) => String(i).padStart(2, "0");
    const get = (tag) => Array.from({ length: n }, (_, i) => Object.assign(new Image(), { src: `${d.scene}/move/${tag}-${pad(i)}.webp` }));
    frames = { rt: get("rt"), three: get("three") };
    Promise.all([...frames.rt, ...frames.three].map((im) => im.decode().catch(() => {}))).then(() => {
      ready = true;
      $(".loading", el).remove();
      draw();
    });
  };
  const loop = (t) => {
    if (playing && ready && t - last > 1000 / fps) {
      last = t;
      k += dir;
      if (k >= n - 1 || k <= 0) dir = -dir; // ping-pong along the camera path
      k = clamp(k, 0, n - 1);
      draw();
    }
    requestAnimationFrame(loop);
  };
  const io = new IntersectionObserver(([en]) => { if (en.isIntersecting && !frames) { load(); requestAnimationFrame(loop); } }, { rootMargin: "400px" });
  io.observe(el);
  let drag = false;
  const at = (e) => { x = clamp(((e.clientX - el.getBoundingClientRect().left) / el.clientWidth) * 100, 0, 100); draw(); };
  el.addEventListener("pointerdown", (e) => { if (e.target.closest(".ctl")) return; drag = true; el.setPointerCapture(e.pointerId); at(e); });
  el.addEventListener("pointermove", (e) => drag && at(e));
  el.addEventListener("pointerup", () => (drag = false));
  pp.addEventListener("click", () => { playing = !playing; icon(); });
  $(".track", el).addEventListener("click", (e) => {
    const r = e.currentTarget.getBoundingClientRect();
    k = Math.round(clamp((e.clientX - r.left) / r.width, 0, 1) * (n - 1));
    draw();
  });
  addEventListener("resize", draw);
}

/** Segmented control: buttons with data-v; calls on(v) and marks the pressed one. */
export function seg(el, on) {
  const bs = $$("button", el);
  const pick = (b) => { for (const o of bs) o.setAttribute("aria-pressed", String(o === b)); on(b.dataset.v, b); };
  for (const b of bs) b.addEventListener("click", () => pick(b));
  return { pick: (v) => pick(bs.find((b) => b.dataset.v === v) ?? bs[0]) };
}

/** Fade-in on scroll for `.rv`. */
export function reveal() {
  const io = new IntersectionObserver((es) => { for (const e of es) if (e.isIntersecting) { e.target.classList.add("in"); io.unobserve(e.target); } }, { threshold: 0.12 });
  for (const el of $$(".rv")) io.observe(el);
}

/** Minimal TS/JS highlighter for the code panels (keywords, strings, numbers, comments, calls). */
export function highlight(src, hl = []) {
  const esc = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;");
  return src.split("\n").map((line, i) => {
    let out = "", m;
    const re = /(\/\/.*$)|("(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|`[^`]*`)|\b(import|from|export|default|function|const|let|new|return|for|of|true|false)\b|(\b0x[0-9a-f]+\b|\b\d+(?:\.\d+)?\b)|\b([A-Za-z_]\w*)(?=\()/gi;
    let at = 0;
    while ((m = re.exec(line))) {
      out += esc(line.slice(at, m.index));
      const cls = m[1] ? "c" : m[2] ? "s" : m[3] ? "k" : m[4] ? "n" : "f";
      out += `<span class="${cls}">${esc(m[0])}</span>`;
      at = m.index + m[0].length;
    }
    out += esc(line.slice(at));
    return hl.includes(i + 1) ? `<span class="hl">${out || " "}</span>` : out;
  }).join("\n");
}
