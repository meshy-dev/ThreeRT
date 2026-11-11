// "Moon Phases": a tabletop orrery of the Sun, the Earth and the Moon, path-traced. The Sun is the only light in the
// scene, and everything the model teaches follows from that and the geometry: the lit half of the Moon, the phase it
// shows to the Earth, the Moon's shadow crossing the Earth (a solar eclipse), the Earth's shadow taking the Moon
// (a lunar eclipse), the faint earthshine on the Moon's dark side. None of it is scripted: the frame script only puts
// the three bodies where the clock says they are.
//
// A model, not the sky: sizes and distances are NOT to scale (a 36 cm Earth, a 10 cm Moon 70 cm from it, a 16 cm
// Sun 1.6 m away). So that most months still pass without an eclipse, the Moon's orbit is tilted 34 degrees instead
// of 5.1 (with bodies this big next to their orbits, eclipse seasons are still longer than the sky's). The Earth turns
// twice a month instead of 29.5, so the continents can be followed. With the Sun this near, the half moons come 64
// degrees from the Sun instead of 90, and the full moon is lit half as strongly as the Earth. And the Earth has no
// air: a totally eclipsed Moon goes black here, not red. Kept: the Earth's 23.4 degree tilt, the Moon keeping one
// face to the Earth, and the calendar (the month, the year, the eclipse year).
//
// The view turns with the Earth round the Sun, so the Sun stays on the left. Seen like that, the stars, the Earth's
// axis and the line where the Moon's tilted orbit crosses the Earth's (the nodes) swing slowly backwards, once a year.
// An eclipse needs a new or full moon while the nodes point at the Sun: twice a year, the eclipse seasons.
//
// Inputs
//   pause (Space)           stop / resume time
//   faster / slower (F / S) time at 1/8 .. 2 times the default (one orbit of the Moon in 25 s)
//   phase (P)               on to the next of new moon, half moon (waxing), full moon, half moon (waning); held 6 s
//   eclipse (E)             on to the next solar eclipse, in slow motion; pressed again, the lunar eclipse after it
//   view (V)                the orrery from above and to the side <-> standing on the Earth, looking up at the Moon
//   click                   the Earth or the Moon: switch the view; the Sun: pause; the sky: next phase
//
// Time: the state holds where the clock stood (`at`, in months since new moon 0), when that was (`since`) and the
// speed; `frame` computes every position from them and the clock. Until a control is touched the clock is the scene's
// own time (`f.time`), so the first frame, and a render at any `--time`, is always the same picture.
import { Color, CylinderGeometry, DataTexture, Mesh, MeshBasicMaterial, MeshStandardMaterial, PerspectiveCamera, PointLight, Quaternion, Scene, SkyColor, SphereGeometry, Texture, TorusGeometry, Vector3 } from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";

// ---------------------------------------------------------------------------------------------- the model (metres)
const R_EARTH = 0.18, R_MOON = 0.05, R_SUN = 0.08;
const D_MOON = 0.7;                        // Earth centre to Moon centre
const D_SUN = 1.6;                         // Earth centre to Sun centre (the Sun is at -X)
const TILT = (23.4 * Math.PI) / 180;       // the Earth's axis
const INCL = (34 * Math.PI) / 180;         // the Moon's orbit against the Earth's (5.1 degrees in the sky)
const SUN_NITS = 92000;                    // the Sun globe's luminance
const SUN_CD = SUN_NITS * Math.PI * R_SUN * R_SUN;   // the same globe as a light: luminance x its disc

// The calendar, in synodic months (new moon to new moon, 29.53 days).
const YEAR = 12.369;                       // the stars and the Earth's axis come round
const ECLIPSE_YEAR = 11.738;               // the nodes come round (346.6 days)
const SPINS = 2;                           // turns of the Earth a month (29.5 in the sky)
const TAU = Math.PI * 2;
/** The direction of the nodes, ahead of the Sun's: they point at the Sun at new moon 6. */
const node = (m: number) => (TAU * (6 - m)) / ECLIPSE_YEAR;
/** How far the Moon is above the Earth's orbital plane, -1..1 of its largest. */
const lift = (m: number) => Math.sin(TAU * m - node(m));

/** The half moons, as fractions of the month: where the Moon is half lit as seen from the Earth. With the Sun this
 *  near that is 64 degrees from the Sun, not the sky's 90. */
const HALF = Math.acos(D_MOON / D_SUN) / TAU;
const PHASES = [0, HALF, 0.5, 1 - HALF];

const START = 3.2;                         // a little after the waxing half moon, three months before an eclipse season
const RATES = [0.005, 0.01, 0.02, 0.04, 0.08];   // months a second
const NORMAL = 3;
const SLOW = 1;                            // an eclipse is watched at RATES[SLOW] at most
const HOLD = 6;                            // seconds a phase is held before time runs on
const ECLIPSE_HOLD = 2.5;                  // seconds before an eclipse starts to move (the sweep to it takes that long)

interface State {
  /** `tx.now` (ms) when the clock was last set (later than now: held until then); 0: untouched, the scene's time. */
  since: number;
  /** Months on the clock at `since`. */
  at: number;
  /** Months from `at` that pass in slow motion (an eclipse under way). */
  slow: number;
  /** Index into RATES. */
  speed: number;
  paused: boolean;
  /** `tx.now` of the first touch, while the clock still counts from wherever the Moon was then (0 once a jump has
   *  set it outright). Only the frame script knows how long the scene had run before that. */
  rel: number;
  /** Jumps so far (the frame script sweeps to a new one instead of cutting). */
  jumps: number;
  /** The eclipse shown last. */
  eclipse: "solar" | "lunar";
  view: "orrery" | "earth";
}

// ---------------------------------------------------------------------------------------------- the clock
/** The months on the clock at `now` (ms), as far as the state says. */
function months(s: Readonly<State>, now: number): number {
  if (s.paused || !s.since) return s.at;
  const t = Math.max(0, (now - s.since) / 1000);
  const rate = RATES[s.speed], slowRate = Math.min(rate, RATES[SLOW]);
  const tSlow = s.slow / slowRate;
  return t < tSlow ? s.at + slowRate * t : s.at + s.slow + rate * (t - tSlow);
}

/** Scene time (s) at the first touch, kept per touch: what the untouched clock had run before the state took over. */
let latch = { rel: 0, t: 0 };
function clock(f: FrameContext<State>): number {
  const s = f.state;
  if (!s.since) return s.at + RATES[s.speed] * f.time;
  if (!s.rel) return months(s, f.now);
  if (latch.rel !== s.rel) latch = { rel: s.rel, t: Math.max(0, f.time - (f.now - s.rel) / 1000) };
  return months(s, f.now) + RATES[NORMAL] * latch.t;
}

// ---------------------------------------------------------------------------------------------- the scene
const UP = new Vector3(0, 1, 0), Z = new Vector3(0, 0, 1);
const qa = new Quaternion(), qb = new Quaternion();
/** A direction in the Earth's orbital plane, `a` radians ahead of the Sun's (the way the Moon goes round). */
const dir = (a: number, out: Vector3) => out.set(-Math.cos(a), 0, Math.sin(a));
const X = new Vector3(1, 0, 0);
const va = new Vector3(), vb = new Vector3(), moonAt = new Vector3(), probe = new Vector3();

/** Where the Moon is at month `m`, on its tilted orbit: `u` from the ascending node, the node line `om` ahead of the
 *  Sun's direction. */
function moonPosition(m: number, out: Vector3): Vector3 {
  const om = node(m), u = TAU * m - om;
  dir(om, va);
  dir(om + Math.PI / 2, vb).multiplyScalar(Math.cos(INCL));
  vb.y = Math.sin(INCL);
  return out.set(0, 0, 0).addScaledVector(va, D_MOON * Math.cos(u)).addScaledVector(vb, D_MOON * Math.sin(u));
}

/** The Earth's full shadow at the Moon's distance (a cone that widens: this Sun is smaller than this Earth). */
const UMBRA = R_EARTH + (D_MOON * (R_EARTH - R_SUN)) / D_SUN;
/** How far the Moon's centre is from the axis of the Earth's shadow at month `m` (near a full moon). */
const offAxis = (m: number) => Math.hypot(moonPosition(m, probe).y, probe.z);

const CAM = { pos: [-1.1, 0.9, 1.75] as const, target: [-0.63, -0.1, 0] as const, fov: 44 };
const EARTH_VIEW_FOV = 30;
const EV = 8;

/** The sweep to a jump's target: from the months shown when it came. */
let shown = START;
let sweep = { jumps: 0, from: START, t0: 0, dur: 0 };
const smooth = (k: number) => k * k * (3 - 2 * k);

function frame(f: FrameContext<State>) {
  const s = f.state;
  const target = clock(f);
  if (f.snap) sweep = { jumps: s.jumps, from: target, t0: 0, dur: 0 };
  else if (sweep.jumps !== s.jumps) sweep = { jumps: s.jumps, from: shown, t0: f.time, dur: Math.min(2.4, 0.8 + 0.3 * Math.abs(target - shown)) };
  const k = sweep.dur > 0 ? (f.time - sweep.t0) / sweep.dur : 1;
  const m = k >= 0 && k < 1 ? sweep.from + (target - sweep.from) * smooth(k) : target;
  // The bodies moved, so the light on them changed: say so to the renderer (a light edit shortens the history its
  // live view averages over, which otherwise follows a turning surface and smears day into night).
  if (m !== shown) (f.object("sunlight") as PointLight).intensity = SUN_CD * (1 + 0.002 * Math.sin(f.time * 17));
  shown = m;

  moonPosition(m, moonAt);
  const orbit = f.object("orbit")!;
  orbit.quaternion.setFromAxisAngle(UP, Math.PI + node(m)).multiply(qb.setFromAxisAngle(X, INCL - Math.PI / 2));
  orbit.visible = s.view === "orrery";
  const moon = f.object("moon")!;
  moon.position.copy(moonAt);
  // One face to the Earth: the map's centre (the near side) is the sphere's +X.
  moon.quaternion.setFromAxisAngle(UP, Math.atan2(moonAt.z, -moonAt.x));

  // The Earth: turning on an axis that keeps pointing at the same stars.
  const year = (TAU * m) / YEAR;
  qa.setFromAxisAngle(UP, 0.7 + (TAU * START) / YEAR - year).multiply(qb.setFromAxisAngle(Z, TILT));
  f.object("earth_axis")!.quaternion.copy(qa);
  f.object("earth")!.quaternion.copy(qa).multiply(qb.setFromAxisAngle(UP, TAU * (m - START) * SPINS + 3.2));
  f.object("sky")!.quaternion.setFromAxisAngle(UP, -year);

  const cam = f.camera;
  if (s.view === "earth") {
    // Standing where the Moon is overhead, looking up at it.
    va.copy(moonAt).normalize();
    cam.position.copy(va).multiplyScalar(R_EARTH + 0.012);
    cam.fov = EARTH_VIEW_FOV;
    cam.lookAt(moonAt);
    cam.setManualExposure(EV - 0.8);          // eyes used to the night
  } else {
    cam.position.set(...CAM.pos);
    cam.fov = CAM.fov;
    cam.lookAt(...CAM.target);
    cam.setManualExposure(EV);
  }
}

/** A small deterministic generator (mulberry32). */
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** A tile of faint stars (made here: no file). */
function starTile(size: number, count: number): DataTexture {
  const px = new Uint8Array(size * size * 4);
  for (let i = 3; i < px.length; i += 4) px[i] = 255;
  const r = rng(20261112);
  const put = (x: number, y: number, c: [number, number, number], k: number) => {
    const o = (((y + size) % size) * size + ((x + size) % size)) * 4;
    for (let j = 0; j < 3; j++) px[o + j] = Math.min(255, px[o + j] + c[j] * k);
  };
  for (let i = 0; i < count; i++) {
    const x = Math.floor(r() * size), y = Math.floor(r() * size);
    const mag = Math.pow(r(), 5);                    // few bright, many faint
    const warm = r();
    const c: [number, number, number] = [200 + 55 * warm, 215, 255 - 70 * warm];
    const k = 0.16 + 0.84 * mag;
    put(x, y, c, k);
    if (mag > 0.25) for (const [dx, dy] of [[1, 0], [-1, 0], [0, 1], [0, -1]]) put(x + dx, y + dy, c, k * 0.35);
  }
  return new DataTexture(size, size, px);
}

/** A clock change: the state takes the clock over from the scene's time at the first one. */
function touch(tx: { state: State; now: number }) {
  const s = tx.state;
  if (!s.since) s.rel = tx.now;
  const cur = months(s, tx.now);
  s.slow = Math.max(0, s.slow - (cur - s.at));
  s.at = cur;
  s.since = tx.now;
}

/** The clock set outright to month `m`, held there for `hold` seconds; the `slow` months after it pass slowly. */
function jump(tx: { state: State; now: number }, m: number, hold: number, slow = 0) {
  const s = tx.state;
  s.at = m;
  s.slow = slow;
  s.since = tx.now + hold * 1000;
  s.rel = 0;
  s.jumps += 1;
}

type Tx = { state: State; now: number; decide(ok: unknown, why: string): void };
const pause = (tx: Tx) => {
  touch(tx);
  tx.state.paused = !tx.state.paused;
};
const phase = (tx: Tx) => {
  // The next named phase, unless the Moon is all but there already.
  const cur = months(tx.state, tx.now);
  let k = 0;
  const at = () => Math.floor(cur) + Math.floor(k / 4) + PHASES[k % 4];
  while (at() < cur + 0.03) k += 1;
  jump(tx, at(), HOLD);
};
const view = (tx: Tx) => {
  tx.state.view = tx.state.view === "orrery" ? "earth" : "orrery";
};

export default defineGame<State>({
  state: { since: 0, at: START, slow: 0, speed: NORMAL, paused: false, rel: 0, jumps: 0, eclipse: "lunar", view: "orrery" },

  setup() {
    const scene = new Scene();
    scene.environment = new SkyColor(0x000000, 0);

    // The Sun: a globe that is seen, and a light of the same size and luminance at its centre that does the lighting
    // (one emitter, sampled as a sphere: the live picture is clean at one sample a pixel).
    const sunColor = new Color().setRGB(1.0, 0.91, 0.78);
    const sun = new Mesh(new SphereGeometry(R_SUN, 64, 32), new MeshBasicMaterial({ name: "sun", color: sunColor.clone().multiplyScalar(SUN_NITS) }));
    sun.id = "sun";
    sun.position.set(-D_SUN, 0, 0);
    sun.castShadow = false;
    scene.add(sun);
    const sunlight = new PointLight(sunColor, SUN_CD, 0);
    sunlight.id = "sunlight";
    sunlight.radius = R_SUN;
    sunlight.position.set(-D_SUN, 0, 0);
    scene.add(sunlight);

    const earth = new Mesh(new SphereGeometry(R_EARTH, 96, 48), new MeshStandardMaterial({ name: "earth", map: new Texture("assets/showcase/moon-phases/earth_bmng_2k.jpg"), roughnessMap: new Texture("assets/showcase/moon-phases/earth_rough_1k.jpg", "linear"), roughness: 1 }));
    earth.id = "earth";
    scene.add(earth);
    // The pin the globe turns on (it does not turn itself: a thin thing that moves leaves a trail in the live view).
    const axis = new Mesh(new CylinderGeometry(0.005, 0.005, R_EARTH * 2.7, 12), new MeshStandardMaterial({ name: "pin", color: new Color().setRGB(0.8, 0.62, 0.34), roughness: 0.6 }));
    axis.id = "earth_axis";
    scene.add(axis);
    // The Moon's orbit, drawn: a faint line that is seen and neither lit nor lighting, like the stars.
    const orbit = new Mesh(new TorusGeometry(D_MOON, 0.003, 6, 240), new MeshBasicMaterial({ name: "orbit", color: new Color().setRGB(0.95, 0.8, 0.55).multiplyScalar(16) }));
    orbit.id = "orbit";
    orbit.castShadow = false;
    scene.add(orbit);

    const moon = new Mesh(new SphereGeometry(R_MOON, 64, 32), new MeshStandardMaterial({ name: "moon", map: new Texture("assets/showcase/moon-phases/moon_lroc_2k.jpg"), roughness: 1 }));
    moon.id = "moon";
    scene.add(moon);

    // The stars: seen, lighting nothing.
    const stars = starTile(512, 700);
    stars.repeat = [8, 4];
    const sky = new Mesh(new SphereGeometry(30, 48, 24), new MeshBasicMaterial({ name: "stars", map: stars, color: new Color().setRGB(150, 150, 150), side: "double" }));
    sky.id = "sky";
    sky.castShadow = false;
    scene.add(sky);

    const camera = new PerspectiveCamera(CAM.fov, 16 / 9, 0.01, 100);
    camera.id = "camera";
    camera.position.set(...CAM.pos);
    camera.lookAt(...CAM.target);
    camera.setManualExposure(EV);
    scene.add(camera);
    return { scene, camera, options: { name: "moon-phases", preset: "preview" as const, width: 1280, height: 720 } };
  },

  frame,

  handlers: {
    pause,
    faster(tx) {
      tx.decide(tx.state.speed < RATES.length - 1, "time is already at full speed");
      touch(tx);
      tx.state.speed += 1;
    },
    slower(tx) {
      tx.decide(tx.state.speed > 0, "time is already at its slowest");
      touch(tx);
      tx.state.speed -= 1;
    },
    phase,
    eclipse(tx) {
      const s = tx.state;
      const kind = s.eclipse === "solar" ? "lunar" : "solar";
      // The next new (solar) or full (lunar) moon that comes while the nodes point at the Sun.
      const half = kind === "solar" ? 0 : 0.5;
      let m = Math.ceil(months(s, tx.now) - half + 0.02) + half;
      while (Math.abs(lift(m)) > 0.28) m += 1;
      // Start a little before it and let it pass in slow motion: the Moon's shadow crossing the Earth; or the Moon
      // from just outside the Earth's full shadow until it is all the way in (then on, at the clock's own speed).
      let start = m - 0.03, slow = 0.06;
      if (kind === "lunar") {
        // The month before full moon `m` at which the Moon's centre is `d` from the shadow's axis.
        const reach = (d: number) => {
          let out = m - 0.15, inside = m;
          for (let i = 0; i < 24; i++) {
            const mid = (out + inside) / 2;
            if (offAxis(mid) > d) out = mid;
            else inside = mid;
          }
          return out;
        };
        start = reach(UMBRA + R_MOON * 1.5);
        slow = reach(UMBRA - R_MOON) - start;
      }
      s.eclipse = kind;
      s.paused = false;
      jump(tx, start, ECLIPSE_HOLD, slow);
    },
    view,
    click(tx, ev: { target?: string }) {
      if (ev.target === "earth" || ev.target === "earth_axis" || ev.target === "moon") view(tx);
      else if (ev.target === "sun") pause(tx);
      else phase(tx);
    },
  },

  keys: { Space: "pause", F: "faster", S: "slower", P: "phase", E: "eclipse", V: "view" },
});
