// 3D device lineups for the "Everywhere" section, rendered live with three.js.
// Devices are modelled from their real dimensions (1 unit = 100 mm); each screen shows a real render.
//   deviceShelf(el, [{ kind: "monitor" | "macbook" | "deck" | "iphone" | "laptop" | "tablet" | "phone",
//                      img, browser?, name, req, width }])
import * as THREE from "three";
import { RoomEnvironment } from "three/addons/RoomEnvironment.js";
import { RoundedBoxGeometry } from "three/addons/RoundedBoxGeometry.js";

const IMG_ASPECT = 16 / 9;

// ---------- geometry helpers ----------
function rrect(w, h, r) {
  const s = new THREE.Shape(), x = -w / 2, y = -h / 2;
  r = Math.min(r, w / 2, h / 2);
  s.moveTo(x + r, y);
  s.lineTo(x + w - r, y);
  s.quadraticCurveTo(x + w, y, x + w, y + r);
  s.lineTo(x + w, y + h - r);
  s.quadraticCurveTo(x + w, y + h, x + w - r, y + h);
  s.lineTo(x + r, y + h);
  s.quadraticCurveTo(x, y + h, x, y + h - r);
  s.lineTo(x, y + r);
  s.quadraticCurveTo(x, y, x + r, y);
  return s;
}
/** A rounded slab centred on the origin, front face at z = +d/2. */
function slab(w, h, d, r, b = Math.min(0.012, d / 4)) {
  const g = new THREE.ExtrudeGeometry(rrect(w - 2 * b, h - 2 * b, Math.max(0.001, r - b)), { depth: d - 2 * b, bevelEnabled: true, bevelThickness: b, bevelSize: b, bevelSegments: 4, curveSegments: 28 });
  g.translate(0, 0, -(d - 2 * b) / 2);
  return g;
}
/** Steam Deck front outline: flat top with rounded corners, grips that hang below a straight middle bottom edge. */
function deckShape() {
  const s = new THREE.Shape(), T = 0.55, B = -0.55, G = -0.62, X = 1.49;
  s.moveTo(0, T);
  s.lineTo(X - 0.3, T);
  s.quadraticCurveTo(X, T, X, T - 0.3);
  s.lineTo(X, -0.22);
  s.bezierCurveTo(X, G - 0.02, X - 0.14, G, X - 0.36, G);
  s.bezierCurveTo(X - 0.52, G, X - 0.58, B, X - 0.68, B);
  s.lineTo(-(X - 0.68), B);
  s.bezierCurveTo(-(X - 0.58), B, -(X - 0.52), G, -(X - 0.36), G);
  s.bezierCurveTo(-(X - 0.14), G, -X, G - 0.02, -X, -0.22);
  s.lineTo(-X, T - 0.3);
  s.quadraticCurveTo(-X, T, -(X - 0.3), T);
  s.lineTo(0, T);
  return s;
}
function extrude(shape, d, b) {
  const g = new THREE.ExtrudeGeometry(shape, { depth: d - 2 * b, bevelEnabled: true, bevelThickness: b, bevelSize: b * 0.6, bevelSegments: 5, curveSegments: 32 });
  g.translate(0, 0, -(d - 2 * b) / 2);
  return g;
}
/** A flat rounded rectangle with 0..1 UVs (for screens and glass). */
function plate(w, h, r) {
  const g = new THREE.ShapeGeometry(rrect(w, h, r), 28), p = g.attributes.position, uv = g.attributes.uv;
  for (let i = 0; i < p.count; i++) uv.setXY(i, (p.getX(i) + w / 2) / w, (p.getY(i) + h / 2) / h);
  return g;
}
const mesh = (g, m, x = 0, y = 0, z = 0) => { const o = new THREE.Mesh(g, m); o.position.set(x, y, z); o.castShadow = o.receiveShadow = true; return o; };

// ---------- materials ----------
// One set per shelf: each shelf has its own WebGL context and environment map, so materials (which carry the env map)
// must not be shared between shelves. The builders below read `M`; deviceShelf sets it before it builds its devices.
const makeMaterials = () => ({
  silver: new THREE.MeshStandardMaterial({ color: 0xc9cbce, metalness: 1, roughness: 0.32 }),
  graphite: new THREE.MeshStandardMaterial({ color: 0x3a3c41, metalness: 0.85, roughness: 0.42 }),
  spaceGray: new THREE.MeshStandardMaterial({ color: 0x5d6066, metalness: 1, roughness: 0.38 }),
  titanium: new THREE.MeshStandardMaterial({ color: 0xa29d94, metalness: 1, roughness: 0.3 }),
  monitor: new THREE.MeshStandardMaterial({ color: 0x24262a, metalness: 0.5, roughness: 0.45 }),
  deck: new THREE.MeshStandardMaterial({ color: 0x212327, metalness: 0, roughness: 0.62 }),
  deckDark: new THREE.MeshStandardMaterial({ color: 0x15161a, metalness: 0, roughness: 0.5 }),
  deckPad: new THREE.MeshStandardMaterial({ color: 0x23252a, metalness: 0, roughness: 0.42 }),
  glass: new THREE.MeshPhysicalMaterial({ color: 0x030304, metalness: 0, roughness: 0.12, clearcoat: 0.6, clearcoatRoughness: 0.1 }),
  sheen: new THREE.MeshPhysicalMaterial({ color: 0x000000, metalness: 0, roughness: 0.05, transparent: true, opacity: 0.1, depthWrite: false }),
  black: new THREE.MeshStandardMaterial({ color: 0x020203, metalness: 0, roughness: 0.3 }),
});
let M;

// ---------- textures ----------
const loader = new THREE.TextureLoader();
const cache = new Map();
function imageTex(src, aspect) {
  const key = `${src}@${aspect.toFixed(3)}`;
  if (cache.has(key)) return cache.get(key);
  const t = loader.load(src);
  t.colorSpace = THREE.SRGBColorSpace;
  t.anisotropy = 8;
  // cover-crop the 16:9 render into the screen's aspect
  if (aspect > IMG_ASPECT) { t.repeat.set(1, IMG_ASPECT / aspect); t.offset.set(0, (1 - t.repeat.y) / 2); }
  else { t.repeat.set(aspect / IMG_ASPECT, 1); t.offset.set((1 - t.repeat.x) / 2, 0); }
  cache.set(key, t);
  return t;
}
/** The render inside a browser window (address bar on top), for the web laptop. */
function browserTex(src, aspect) {
  const W = 1280, H = Math.round(W / aspect), bar = 62;
  const c = document.createElement("canvas"); c.width = W; c.height = H;
  const x = c.getContext("2d");
  const t = new THREE.CanvasTexture(c);
  t.colorSpace = THREE.SRGBColorSpace;
  t.anisotropy = 8;
  const img = new Image();
  img.onload = () => {
    const ih = H - bar, iw = (img.width / img.height) * ih, sx = Math.max(0, (iw - W) / 2) * (img.width / iw);
    x.drawImage(img, sx, 0, img.width - 2 * sx, img.height, 0, bar, W, ih);
    x.fillStyle = "#ece8e1"; x.fillRect(0, 0, W, bar);
    x.fillStyle = "#c4bfb6"; for (const cx of [30, 56, 82]) { x.beginPath(); x.arc(cx, bar / 2, 8, 0, 7); x.fill(); }
    x.fillStyle = "#ffffff"; x.beginPath(); x.roundRect(130, 14, W - 260, bar - 28, 17); x.fill();
    x.fillStyle = "#6b6e75"; x.font = "500 22px Geist, system-ui, sans-serif"; x.textBaseline = "middle"; x.fillText("your-game.com", 152, bar / 2 + 1);
    t.needsUpdate = true;
  };
  img.src = src;
  return t;
}
function keyboardTex(body, keys) {
  const c = document.createElement("canvas"); c.width = 1024; c.height = 420;
  const x = c.getContext("2d");
  x.fillStyle = body; x.fillRect(0, 0, 1024, 420);
  x.fillStyle = keys;
  const rows = [[14, 0.55], [14, 1], [14, 1], [13, 1], [12, 1], [9, 1]];
  let y = 8;
  for (const [n, hgt] of rows) {
    const kh = 56 * hgt, kw = (1024 - 16 - (n - 1) * 8) / n;
    for (let i = 0; i < n; i++) { x.beginPath(); x.roundRect(8 + i * (kw + 8), y, kw, kh, 7); x.fill(); }
    y += kh + 8;
  }
  const t = new THREE.CanvasTexture(c); t.colorSpace = THREE.SRGBColorSpace; t.anisotropy = 8;
  return t;
}
const screenMat = (tex) => new THREE.MeshBasicMaterial({ map: tex, toneMapped: false });

// ---------- devices (local origin: bottom centre, screen facing +z) ----------
function monitor(d) {
  const g = new THREE.Group(), W = 6.14, H = 3.62, D = 0.1, lift = 1.0;
  g.add(mesh(slab(W, H, D, 0.05), M.monitor, 0, lift + H / 2, 0));
  g.add(mesh(plate(W - 0.04, H - 0.04, 0.04), M.glass, 0, lift + H / 2, D / 2 + 0.001));
  const sw = W - 0.2, sh = sw / IMG_ASPECT;
  g.add(mesh(plate(sw, sh, 0.01), screenMat(imageTex(d.img, sw / sh)), 0, lift + H / 2 + 0.03, D / 2 + 0.002));
  g.add(mesh(new RoundedBoxGeometry(3.2, 2.0, 0.34, 4, 0.1), M.monitor, 0, lift + H / 2, -0.2));
  const neck = mesh(new RoundedBoxGeometry(0.55, 2.3, 0.2, 4, 0.06), M.monitor, 0, 1.15, -0.55);
  neck.rotation.x = -0.12;
  g.add(neck);
  g.add(mesh(new RoundedBoxGeometry(2.5, 0.08, 1.6, 4, 0.035), M.monitor, 0, 0.04, -0.35));
  return { g, w: W, h: lift + H };
}

function macbook(d, o = {}) {
  const g = new THREE.Group(), W = o.w ?? 3.126, Dp = o.d ?? 2.212, bt = 0.085, lt = 0.06, body = o.body ?? M.silver;
  const base = mesh(slab(W, Dp, bt, 0.09), body, 0, bt / 2, 0);
  base.rotation.x = -Math.PI / 2;
  g.add(base);
  const kb = mesh(new THREE.PlaneGeometry(W * 0.86, Dp * 0.42), Object.assign(new THREE.MeshStandardMaterial({ map: keyboardTex(o.well ?? "#62656b", "#0e0f11"), roughness: 0.7 }), { userData: { env: 0.1 } }), 0, bt + 0.001, -Dp * 0.17);
  kb.rotation.x = -Math.PI / 2; g.add(kb);
  const tp = mesh(plate(W * 0.42, Dp * 0.3, 0.05), Object.assign(new THREE.MeshStandardMaterial({ color: o.pad ?? 0x8e9196, metalness: 0.9, roughness: 0.3 }), { userData: { env: 0.6 } }), 0, bt + 0.001, Dp * 0.29);
  tp.rotation.x = -Math.PI / 2; g.add(tp);
  // lid on a hinge at the back edge, opened ~105°
  const hinge = new THREE.Group();
  hinge.position.set(0, bt, -Dp / 2 + 0.02);
  hinge.rotation.x = -0.26;
  hinge.add(mesh(slab(W, Dp, lt, 0.09), body, 0, Dp / 2, -lt / 2 + 0.01));
  const bez = o.bezel ?? 0.035, chin = o.chin ?? 0.06;
  hinge.add(mesh(plate(W - 0.02, Dp - 0.02, 0.08), M.glass, 0, Dp / 2, 0.012));
  const sw = W - 2 * bez - 0.02, sh = Dp - bez - chin - 0.02;
  hinge.add(mesh(plate(sw, sh, 0.03), screenMat(d.browser ? browserTex(d.img, sw / sh) : imageTex(d.img, sw / sh)), 0, chin + 0.01 + sh / 2, 0.013));
  if (o.notch !== false) hinge.add(mesh(plate(0.36, 0.075, 0.03), M.black, 0, Dp - bez - 0.035, 0.014));
  else hinge.add(mesh(new THREE.CircleGeometry(0.016, 20), M.black, 0, Dp - bez / 2 - 0.012, 0.014));
  g.add(hinge);
  return { g, w: W, h: bt + Dp * Math.cos(0.26) };
}

function deck(d) {
  // Steam Deck: 298 × 117 × 49 mm. 7" 16:10 screen in the middle; sticks inboard above 32.5 mm trackpads;
  // D-pad (left) and ABXY (right) outboard; View / Menu above, Steam / ··· below.
  const g = new THREE.Group(), body = new THREE.Group(), W = 2.98, H = 1.17, D = 0.22;
  body.add(mesh(extrude(deckShape(), D, 0.035), M.deck, 0, 0, 0));
  for (const s of [-1, 1]) {  // the grips bulge out at the back
    const grip = mesh(new THREE.SphereGeometry(1, 40, 24), M.deck, s * 1.1, -0.2, -0.13);
    grip.scale.set(0.36, 0.42, 0.2);
    body.add(grip);
  }
  const z = D / 2;
  body.add(mesh(plate(1.76, 1.04, 0.06), M.glass, 0, 0, z + 0.001));
  const sw = 1.508, sh = 0.9425;
  body.add(mesh(plate(sw, sh, 0.012), screenMat(imageTex(d.img, sw / sh)), 0, 0.01, z + 0.002));
  const cyl = (r, h, m, x, y, zz) => { const o = mesh(new THREE.CylinderGeometry(r, r, h, 40), m, x, y, zz); o.rotation.x = Math.PI / 2; return o; };
  for (const s of [-1, 1]) {
    // thumbstick: well, shaft, concave cap
    body.add(cyl(0.15, 0.012, M.deckDark, s * 1.07, 0.3, z));
    body.add(cyl(0.05, 0.08, M.deckDark, s * 1.07, 0.3, z + 0.04));
    body.add(cyl(0.118, 0.045, M.deckDark, s * 1.07, 0.3, z + 0.1));
    const rim = mesh(new THREE.TorusGeometry(0.108, 0.016, 12, 40), M.deckDark, s * 1.07, 0.3, z + 0.123);
    body.add(rim);
    // trackpad (32.5 mm square, slightly recessed)
    body.add(mesh(plate(0.345, 0.345, 0.05), M.deckDark, s * 1.11, -0.22, z + 0.001));
    body.add(mesh(plate(0.325, 0.325, 0.045), M.deckPad, s * 1.11, -0.22, z + 0.003));
    // View / Menu (top, inboard) and Steam / ··· (bottom, inboard)
    body.add(mesh(new RoundedBoxGeometry(0.07, 0.045, 0.02, 2, 0.01), M.deckDark, s * 0.95, 0.44, z + 0.008));
    body.add(mesh(new RoundedBoxGeometry(s < 0 ? 0.11 : 0.07, 0.05, 0.02, 2, 0.012), M.deckDark, s * 0.97, -0.44, z + 0.008));
    // bumpers on the top edge
    body.add(mesh(new RoundedBoxGeometry(0.5, 0.06, 0.16, 3, 0.025), M.deckDark, s * 1.08, 0.55, -0.02));
  }
  // D-pad
  const dp = new THREE.Group(); dp.position.set(-1.33, 0.2, z);
  dp.add(cyl(0.13, 0.008, M.deckDark, 0, 0, 0));
  dp.add(mesh(new RoundedBoxGeometry(0.21, 0.068, 0.04, 2, 0.012), M.deckDark, 0, 0, 0.022));
  dp.add(mesh(new RoundedBoxGeometry(0.068, 0.21, 0.04, 2, 0.012), M.deckDark, 0, 0, 0.022));
  body.add(dp);
  // ABXY
  for (const [dx, dy] of [[0, 0.085], [0.085, 0], [0, -0.085], [-0.085, 0]]) body.add(cyl(0.04, 0.035, M.deckDark, 1.33 + dx, 0.2 + dy, z + 0.018));
  const tilt = 0.17;
  body.rotation.x = -tilt;
  body.position.y = H / 2 * Math.cos(tilt) + 0.06;
  g.add(body);
  return { g, w: W, h: H + 0.06 };
}

function handheld(d, o) {
  // phones and tablets, landscape, standing on the long edge and leaning back
  const g = new THREE.Group(), b = new THREE.Group(), { W, H, D, r, frame, bezel } = o;
  b.add(mesh(slab(W, H, D, r, 0.01), frame));
  b.add(mesh(plate(W - 0.016, H - 0.016, r - 0.008), M.glass, 0, 0, D / 2 + 0.001));
  const sw = W - 2 * bezel, sh = H - 2 * bezel;
  b.add(mesh(plate(sw, sh, Math.max(0.01, r - bezel)), screenMat(imageTex(d.img, sw / sh)), 0, 0, D / 2 + 0.002));
  if (o.island) b.add(mesh(plate(0.066, 0.205, 0.033), M.black, -sw / 2 + 0.075, 0, D / 2 + 0.003));
  if (o.punch) b.add(mesh(new THREE.CircleGeometry(0.02, 24), M.black, -sw / 2 + 0.05, 0, D / 2 + 0.003));
  if (o.cam) b.add(mesh(new THREE.CircleGeometry(0.014, 20), M.black, 0, H / 2 - bezel / 2, D / 2 + 0.003));
  if (o.buttons) b.add(mesh(new RoundedBoxGeometry(0.17, 0.03, 0.035, 2, 0.012), frame, -0.25, H / 2 + 0.008, 0));
  const tilt = 0.14;
  b.rotation.x = -tilt;
  b.position.y = H / 2 * Math.cos(tilt) + D / 2 * Math.sin(tilt);
  g.add(b);
  return { g, w: W, h: H };
}

const BUILD = {
  monitor,
  macbook: (d) => macbook(d),
  laptop: (d) => macbook(d, { w: 3.3, d: 2.25, body: M.graphite, well: "#2c2e33", pad: 0x45484e, notch: false, bezel: 0.09, chin: 0.16 }),
  deck,
  iphone: (d) => handheld(d, { W: 1.466, H: 0.706, D: 0.0825, r: 0.11, frame: M.titanium, bezel: 0.024, island: true, buttons: true }),
  phone: (d) => handheld(d, { W: 1.62, H: 0.762, D: 0.085, r: 0.09, frame: M.graphite, bezel: 0.02, punch: true }),
  tablet: (d) => handheld(d, { W: 2.476, H: 1.785, D: 0.061, r: 0.18, frame: M.spaceGray, bezel: 0.095, cam: true }),
};

// ---------- the shelf ----------
export function deviceShelf(el, devices, opt = {}) {
  try { return build(el, devices, opt); } catch (e) {
    console.error("deviceShelf:", e);
    if (opt.fallback) { el.classList.add("fallback"); el.innerHTML = `<img src="${opt.fallback}" alt="${devices.map((d) => d.name).join(", ")}">`; }
  }
}

function build(el, devices, opt) {
  let renderer;
  try { renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true }); } catch {
    // no WebGL: show a pre-rendered still of the same lineup
    if (opt.fallback) { el.classList.add("fallback"); el.innerHTML = `<img src="${opt.fallback}" alt="${devices.map((d) => d.name).join(", ")}">`; }
    return;
  }
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = opt.exposure ?? 1.0;
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFShadowMap; // r187: PCFSoftShadowMap was removed; PCF + shadow.radius is soft
  el.appendChild(renderer.domElement);

  const scene = new THREE.Scene();
  const pmrem = new THREE.PMREMGenerator(renderer);
  M = makeMaterials();
  const env = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
  scene.environment = env;
  const ENV = { silver: 1.25, graphite: 0.7, spaceGray: 0.8, titanium: 0.9, monitor: 0.35, deck: 0.12, deckDark: 0.1, deckPad: 0.16, glass: 0.06, sheen: 0.3, black: 0.04 };
  for (const [k, m] of Object.entries(M)) { m.envMap = env; m.envMapIntensity = ENV[k] ?? 0.5; m.needsUpdate = true; }
  const key = new THREE.DirectionalLight(0xffffff, 1.3);
  key.position.set(-4, 9, 7);
  key.castShadow = true;
  key.shadow.mapSize.set(2048, 2048);
  key.shadow.radius = 6;
  key.shadow.bias = -0.0004;
  Object.assign(key.shadow.camera, { left: -12, right: 12, top: 8, bottom: -4, near: 1, far: 30 });
  scene.add(key);
  const rim = new THREE.DirectionalLight(0xdfe6ff, 1.1);  // separates dark devices from the dark page
  rim.position.set(2, 6, -8);
  scene.add(rim);
  scene.add(new THREE.HemisphereLight(0xffffff, 0x202020, 0.35));
  if (opt.warm) { const w = new THREE.PointLight(0xffa24a, 6, 14, 2); w.position.set(0, 3, 3); scene.add(w); }
  const ground = new THREE.Mesh(new THREE.PlaneGeometry(60, 30), new THREE.ShadowMaterial({ opacity: 0.38 }));
  ground.rotation.x = -Math.PI / 2;
  ground.receiveShadow = true;
  scene.add(ground);

  // lay the devices out in a row, each scaled to its display width, resting on the ground
  const row = new THREE.Group(), gap = opt.gap ?? 0.5, items = [];
  let x = 0;
  for (const d of devices) {
    const { g, w } = BUILD[d.kind](d);
    const s = d.width / w;
    g.scale.setScalar(s);
    g.updateMatrixWorld(true);
    const box = new THREE.Box3().setFromObject(g);
    g.position.y -= box.min.y;  // rest on the ground
    const h = box.max.y - box.min.y;
    const pivot = new THREE.Group();
    pivot.add(g);
    pivot.position.x = x + d.width / 2;
    row.add(pivot);
    items.push({ pivot, w: d.width, h, d });
    x += d.width + gap;
  }
  const total = x - gap;
  // materials made per device (keyboards, trackpads) get the studio too, at their own strength
  row.traverse((o) => { if (o.isMesh && !o.material.envMap && !o.material.isMeshBasicMaterial && !o.material.isShadowMaterial) { o.material.envMap = env; o.material.envMapIntensity = o.material.userData.env ?? 0.5; } });
  for (const it of items) it.pivot.position.x -= total / 2;
  scene.add(row);
  const maxH = Math.max(...items.map((i) => i.h));

  // captions under each device (HTML, positioned from the 3D layout)
  const caps = items.map((it) => {
    const c = document.createElement("figcaption");
    c.className = "dev3d-cap";
    c.innerHTML = `<b>${it.d.name}</b>${it.d.req}`;
    el.appendChild(c);
    return c;
  });

  const camera = new THREE.PerspectiveCamera(opt.fov ?? 24, 2, 0.1, 200);
  const look = new THREE.Vector3(0, maxH * 0.5, 0);
  const capH = opt.capH ?? 46;
  const fit = () => {
    const w = el.clientWidth, h = Math.max(80, el.clientHeight - capH);
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    // Fit the row's width, keep the ground ~10 % above the bottom edge and the tallest device inside the top.
    const vf = THREE.MathUtils.degToRad(camera.fov), hf = 2 * Math.atan(Math.tan(vf / 2) * camera.aspect);
    const dist = Math.max((total / 2 + 0.55) / Math.tan(hf / 2), maxH / 0.84 / (2 * Math.tan(vf / 2)));
    look.y = dist * Math.tan(vf / 2) * 0.8;
    camera.position.set(0, look.y + dist * 0.1, dist);
    camera.lookAt(look);
    camera.updateProjectionMatrix();
  };
  const placeCaps = () => {
    const v = new THREE.Vector3(), w = el.clientWidth;
    items.forEach((it, i) => { v.set(it.pivot.position.x, 0, 0.6).project(camera); caps[i].style.left = `${((v.x + 1) / 2) * w}px`; });
  };

  let mx = 0, my = 0, tx = 0, ty = 0, t0 = performance.now();
  el.addEventListener("pointermove", (e) => { const r = el.getBoundingClientRect(); tx = ((e.clientX - r.left) / r.width - 0.5) * 2; ty = ((e.clientY - r.top) / r.height - 0.5) * 2; });
  el.addEventListener("pointerleave", () => { tx = 0; ty = 0; });
  const frame = (now) => {
    const t = (now - t0) / 1000;
    mx += (tx - mx) * 0.06; my += (ty - my) * 0.06;
    items.forEach((it, i) => { it.pivot.rotation.y = -0.18 * (it.pivot.position.x / total) * 2 + mx * 0.12 + Math.sin(t * 0.5 + i) * 0.02; });
    camera.position.x = mx * 0.5;
    camera.lookAt(look);
    renderer.render(scene, camera);
  };
  fit(); placeCaps(); renderer.render(scene, camera);
  // setSize clears the canvas: draw again at once (with reduced motion no animation loop would).
  addEventListener("resize", () => { fit(); placeCaps(); renderer.render(scene, camera); });
  const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
  new IntersectionObserver(([e]) => renderer.setAnimationLoop(e.isIntersecting && !reduce ? frame : null), { rootMargin: "100px" }).observe(el);
  // textures arrive asynchronously; redraw a few times so a static (reduced-motion) shelf still shows them
  for (const ms of [300, 900, 2000]) setTimeout(() => renderer.render(scene, camera), ms);
}
