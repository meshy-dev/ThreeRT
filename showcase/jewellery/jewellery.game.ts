// "The Jeweller's Window": a jeweller's product page as a live scene. Three rings and a pearl bracelet on a turntable
// of polished black stone in a small dark studio: a six-prong solitaire with a 2 ct brilliant (the hero, in the middle,
// on a velvet holder with the pearls around it), a half-eternity ring with seven small diamonds, and a plain
// comfort-fit band. Two soft boxes, a top panel and a fill panel light them (their cards are what the metal mirrors),
// two pin spots make the glints, a snooted lamp puts the glow on the paper behind. Everything is at real size (the
// rings are 2 cm across, the camera is 8 cm away) and built in code: no model or texture files.
//
// What tracing gives it: the polished metal mirrors the soft boxes, the other pieces and the pearls; each stone is a
// real brilliant cut (57 facets, flat) of clear glass with a diamond's index of refraction, so what you see in it is
// light that went in through the crown, bounced off the pavilion and came back out; the plinth mirrors all of it.
//
// How the stones are made to work live: `transmission: 1`, `ior` 2.42 (1.77 ruby / sapphire, 1.58 emerald), the
// colour as the tint. The live view traces 4 bounces, which is exactly a brilliant's light path (in, two reflections
// off the pavilion, out), so the cut returns the light panels and the stones are bright; the preset's 3 leave them
// dark, hence `maxBounces` below. Roughness 0.03 keeps the refraction sharp and lets a facet flash when it mirrors a
// pin spot. Small bright emissive spheres as sparkle lights put fireflies in the stones; spot lights do not. The
// pearls are a soft, slightly metallic base under a clear coat. The small stones stand clear of the band on prongs:
// set into the metal their pavilions had nothing to reflect and they went milky.
//
// How it turns: on screen the display turns in the studio, one turn in 80 s (a still of time t shows it where
// `f.time` puts it; live it advances by `f.dt` while `spin` is on). In the scene the display stands still and the
// studio (lights, paper) and the camera go around it, which is the same picture: see `frame`.
//
//   click a piece   the camera goes close to it and the table brings it to the front, swaying; click it again (or
//                   anything that is not a piece) to step back
//   metal  (M)      yellow gold -> rose gold -> platinum, on every piece
//   stone  (G)      the solitaire's stone: diamond -> ruby -> sapphire -> emerald
//   spin   (Space)  stops / resumes the turntable (stopped, the picture converges)
//   back   (B)      back to the whole display
import {
  BufferAttribute, BufferGeometry, Color, CylinderGeometry, Group, LatheGeometry, Mesh, MeshPhysicalMaterial, MeshStandardMaterial,
  Object3D, PerspectiveCamera, RectAreaLight, Scene, SkyColor, SphereGeometry, SpotLight, TorusGeometry, Vector3,
} from "@threert/scene";
import { defineGame, type FrameContext } from "@threert/gameplay";

type Vec3 = [number, number, number];
const MM = 0.001;
const DEG = Math.PI / 180;

const PIECES = ["solitaire", "pave", "band", "pearls"] as const;
type Piece = (typeof PIECES)[number];

interface State {
  /** Index into METALS / STONES. */
  metal: number;
  stone: number;
  /** The turntable turns. */
  spin: boolean;
  /** The piece the camera is close to ("" = the whole display). */
  focus: Piece | "";
}

// Linear reflectance of the polished alloys (18 ct yellow, 18 ct rose, platinum).
const METALS: { name: string; color: Vec3 }[] = [
  { name: "yellow gold", color: [1.0, 0.77, 0.36] },
  { name: "rose gold", color: [0.98, 0.66, 0.54] },
  { name: "platinum", color: [0.84, 0.83, 0.8] },
];
// The stone: its tint (what a pass through a facet keeps) and index of refraction.
const STONES: { name: string; color: Vec3; ior: number }[] = [
  { name: "diamond", color: [1, 1, 1], ior: 2.42 },
  { name: "ruby", color: [0.9, 0.02, 0.09], ior: 1.77 },
  { name: "sapphire", color: [0.02, 0.22, 0.8], ior: 1.77 },
  { name: "emerald", color: [0.04, 0.72, 0.3], ior: 1.58 },
];

// ---------------------------------------------------------------------------------------------------------------
// The display, in the turntable's frame (y = 0 is the top of the stone plinth; angles from +Z towards +X).

const PLINTH = { radius: 50 * MM, height: 12 * MM };
const RING_INNER = 8.6 * MM;          // a size 7 finger
const TURN_START = 0;                 // the table's angle in the studio in the first frame
const TURN_RATE = (2 * Math.PI) / 80; // one turn in 80 s
const SWAY = 0.35;                    // while a piece is close: the table sways this far either way (rad)

/** Where each ring stands: polar position of its holder, the holder's size, and the ring's own turn on it. */
const STAND: Record<Exclude<Piece, "pearls">, { at: number; r: number; block: [number, number]; yaw: number }> = {
  solitaire: { at: 0, r: 0, block: [9 * MM, 9 * MM], yaw: 0.62 },   // three-quarter view in the first frame
  pave: { at: -140 * DEG, r: 34 * MM, block: [9.5 * MM, 5 * MM], yaw: -140 * DEG - 0.1 },
  band: { at: 140 * DEG, r: 34 * MM, block: [9.5 * MM, 5 * MM], yaw: 140 * DEG + 0.1 },
};
/** The pearls: a bracelet lying around the solitaire's holder, 15 pearls and a gold clasp bead. */
const PEARLS = { count: 15, radius: 2.8 * MM, pitch: 5.75 * MM, clasp: 150 * DEG };
const BRACELET = ((PEARLS.count + 1) * PEARLS.pitch) / (2 * Math.PI);

/** The camera for the whole display and for each piece: what it looks at (turntable frame), from how far and how
 *  high, and the table angle that puts the piece in front. */
const VIEW: Record<Piece | "", { target: Vec3; dist: number; elev: number; front: number }> = {
  "": { target: [0, 17 * MM, 0], dist: 80 * MM, elev: 25 * DEG, front: TURN_START },
  solitaire: { target: [0, 26 * MM, 0], dist: 52 * MM, elev: 30 * DEG, front: TURN_START },
  pave: { target: on(STAND.pave.r, STAND.pave.at, 17 * MM), dist: 46 * MM, elev: 28 * DEG, front: -STAND.pave.at + 0.85 },
  band: { target: on(STAND.band.r, STAND.band.at, 14 * MM), dist: 50 * MM, elev: 26 * DEG, front: -STAND.band.at - 0.85 },
  pearls: { target: on(BRACELET * 0.6, 0.5, 7 * MM), dist: 46 * MM, elev: 24 * DEG, front: -0.5 },
};

function on(r: number, a: number, y: number): Vec3 {
  return [r * Math.sin(a), y, r * Math.cos(a)];
}
function polar(r: number, a: number): [number, number] {
  return [r * Math.sin(a), r * Math.cos(a)];
}

// ---------------------------------------------------------------------------------------------------------------
// Geometry

/** A geometry from triangles (9 numbers each), each with its own flat normal facing away from `inside`. */
function facets(tris: number[][], inside: Vec3): BufferGeometry {
  const pos: number[] = [], nor: number[] = [];
  for (const t of tris) {
    const [ax, ay, az, bx, by, bz, cx, cy, cz] = t;
    let nx = (by - ay) * (cz - az) - (bz - az) * (cy - ay), ny = (bz - az) * (cx - ax) - (bx - ax) * (cz - az), nz = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
    const l = Math.hypot(nx, ny, nz) || 1;
    nx /= l; ny /= l; nz /= l;
    const out = nx * ((ax + bx + cx) / 3 - inside[0]) + ny * ((ay + by + cy) / 3 - inside[1]) + nz * ((az + bz + cz) / 3 - inside[2]) >= 0;
    if (out) pos.push(...t);
    else { pos.push(ax, ay, az, cx, cy, cz, bx, by, bz); nx = -nx; ny = -ny; nz = -nz; }
    for (let k = 0; k < 3; k++) nor.push(nx, ny, nz);
  }
  const g = new BufferGeometry();
  g.setAttribute("position", new BufferAttribute(new Float32Array(pos), 3));
  g.setAttribute("normal", new BufferAttribute(new Float32Array(nor), 3));
  return g;
}

/**
 * A round brilliant cut, 57 facets + a faceted girdle, diameter 1, the table up (+Y), the girdle around y = 0:
 * table 57 %, crown 14.8 % (34.5 degrees), pavilion 43.1 % (40.75 degrees), the proportions of an ideal cut.
 */
function brilliantGeometry(): BufferGeometry {
  const R = 0.5, g = 0.03, table = 0.285, hc = 0.148, hp = 0.431, star = 0.394, low = 0.12;
  const c8 = Math.cos(22.5 * DEG);
  const p = (r: number, a: number, y: number): Vec3 => [r * Math.sin(a * DEG), y, r * Math.cos(a * DEG)];
  const T = (k: number) => p(table, k * 45, g / 2 + hc);
  // The star points lie in the bezel (kite) planes and the lower-girdle points in the pavilion mains', so both are flat.
  const S = (k: number) => p(star, (k + 0.5) * 45, g / 2 + (hc * (R - star * c8)) / (R - table));
  const GU = (j: number) => p(R, j * 22.5, g / 2);
  const GL = (j: number) => p(R, j * 22.5, -g / 2);
  const L = (k: number) => p(low, (k + 0.5) * 45, -g / 2 - (hp * (R - low * c8)) / R);
  const C: Vec3 = [0, -g / 2 - hp, 0];
  const tris: number[][] = [];
  const tri = (a: Vec3, b: Vec3, c: Vec3) => tris.push([...a, ...b, ...c]);
  for (let k = 1; k < 7; k++) tri(T(0), T(k), T(k + 1));
  for (let k = 0; k < 8; k++) {
    tri(T(k), T(k + 1), S(k));                                              // star
    tri(T(k), S(k - 1), GU(2 * k)); tri(T(k), GU(2 * k), S(k));             // bezel (kite)
    tri(S(k), GU(2 * k), GU(2 * k + 1)); tri(S(k), GU(2 * k + 1), GU(2 * k + 2)); // upper girdle
    tri(L(k), GL(2 * k), GL(2 * k + 1)); tri(L(k), GL(2 * k + 1), GL(2 * k + 2)); // lower girdle
    tri(C, L(k - 1), GL(2 * k)); tri(C, GL(2 * k), L(k));                   // pavilion main
  }
  for (let j = 0; j < 16; j++) { tri(GU(j), GU(j + 1), GL(j + 1)); tri(GU(j), GL(j + 1), GL(j)); }
  return facets(tris, [0, -0.1, 0]);
}

/**
 * A ring band around the Z axis (the finger's), its top at +Y: inner radius `inner`, a rounded comfort-fit section
 * `thick` x `width` (a superellipse, exponent `n`: 2 = oval, higher = flatter faces), `taper` narrowing it at the top.
 */
function bandGeometry(inner: number, thick: number, width: number, n = 2.6, taper = 0, around = 112, section = 20): BufferGeometry {
  const pos: number[] = [], idx: number[] = [];
  const pw = (v: number) => Math.sign(v) * Math.abs(v) ** (2 / n);
  for (let i = 0; i < around; i++) {
    const phi = (i / around) * 2 * Math.PI, top = Math.max(0, Math.sin(phi)) ** 2;
    const w = width * (1 - taper * top), t = thick * (1 - 0.5 * taper * top);
    for (let j = 0; j < section; j++) {
      const a = (j / section) * 2 * Math.PI;
      const r = inner + t / 2 + pw(Math.cos(a)) * (t / 2);
      pos.push(r * Math.cos(phi), r * Math.sin(phi), pw(Math.sin(a)) * (w / 2));
    }
  }
  for (let i = 0; i < around; i++)
    for (let j = 0; j < section; j++) {
      const a = i * section + j, b = ((i + 1) % around) * section + j, c = ((i + 1) % around) * section + ((j + 1) % section), d = i * section + ((j + 1) % section);
      idx.push(a, b, d, b, c, d);
    }
  const g = new BufferGeometry();
  g.setAttribute("position", new BufferAttribute(new Float32Array(pos), 3));
  g.setIndex(idx);
  g.computeVertexNormals();
  return g;
}

/** A round velvet holder: a low drum with a rounded shoulder, its base at y = 0. */
function holderGeometry(radius: number, height: number): BufferGeometry {
  const pts: { x: number; y: number }[] = [{ x: radius, y: 0 }];
  const c = Math.min(2.2 * MM, height * 0.45);
  for (let i = 0; i <= 8; i++) {
    const a = (i / 8) * 90 * DEG;
    pts.push({ x: radius - c + c * Math.cos(a), y: height - c + c * Math.sin(a) });
  }
  pts.push({ x: 0, y: height });
  return new LatheGeometry(pts, 56);
}

/** The backdrop: a floor that sweeps up into a wall with no corner (a paper cove), across X. */
function coveGeometry(): BufferGeometry {
  const prof: [number, number][] = [[0.7, 0]];
  const R = 0.28, zc = -0.3;
  for (let i = 0; i <= 14; i++) {
    const a = (i / 14) * 90 * DEG;
    prof.push([zc - R * Math.sin(a), R - R * Math.cos(a)]);
  }
  prof.push([zc - R, 1.0]);
  const pos: number[] = [], idx: number[] = [];
  prof.forEach(([z, y]) => pos.push(-1, y, z, 1, y, z));
  for (let i = 0; i < prof.length - 1; i++) idx.push(2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 1, 2 * i + 3, 2 * i + 2);
  const g = new BufferGeometry();
  g.setAttribute("position", new BufferAttribute(new Float32Array(pos), 3));
  g.setIndex(idx);
  g.computeVertexNormals();
  return g;
}

/** One geometry from several, each placed by its mesh's transform: a piece's metal is then one object to move. */
function merge(parts: Mesh[]): BufferGeometry {
  const pos: number[] = [], nor: number[] = [], idx: number[] = [];
  for (const m of parts) {
    m.updateMatrix();
    const g = m.geometry.clone().applyMatrix4(m.matrix), base = pos.length / 3;
    for (const v of g.positions) pos.push(v);
    for (const v of g.normals) nor.push(v);
    for (const i of g.triangleIndices()) idx.push(base + i);
  }
  const g = new BufferGeometry();
  g.setAttribute("position", new BufferAttribute(new Float32Array(pos), 3));
  g.setAttribute("normal", new BufferAttribute(new Float32Array(nor), 3));
  g.setIndex(idx);
  return g;
}

// ---------------------------------------------------------------------------------------------------------------
// The scene

function build() {
  const scene = new Scene();
  scene.environment = new SkyColor(0x000000, 0);
  const rgb = (c: Vec3) => new Color().setRGB(...c);

  const metal = new MeshStandardMaterial({ name: "metal", color: rgb(METALS[0].color), metalness: 1, roughness: 0.05 });
  const gem = new MeshPhysicalMaterial({ name: "gem", color: rgb(STONES[0].color), metalness: 0, roughness: 0.03, transmission: 1, ior: STONES[0].ior });
  const melee = new MeshPhysicalMaterial({ name: "melee", color: rgb([1, 1, 1]), metalness: 0, roughness: 0.03, transmission: 1, ior: 2.42 });
  const pearl = new MeshPhysicalMaterial({ name: "pearl", color: rgb([0.74, 0.65, 0.56]), metalness: 0.3, roughness: 0.2, clearcoat: 1, clearcoatRoughness: 0.03 });
  const velvet = new MeshStandardMaterial({ name: "velvet", color: rgb([0.010, 0.012, 0.024]), roughness: 1 });
  const stone = new MeshStandardMaterial({ name: "plinth", color: rgb([0.018, 0.018, 0.02]), roughness: 0.03 });
  const paper = new MeshStandardMaterial({ name: "backdrop", color: rgb([0.022, 0.023, 0.028]), roughness: 0.95, side: "double" });

  // The studio (the paper cove and the lights) is one group, the display another. On screen the display turns in the
  // studio; in the scene it is the studio, with the camera, that turns around the display (see `frame`).
  const studio = new Group();
  studio.id = "studio";
  scene.add(studio);
  const cove = new Mesh(coveGeometry(), paper);
  cove.id = "backdrop";
  studio.add(cove);

  const table = new Group();
  table.id = "table";
  table.position.y = PLINTH.height;
  scene.add(table);

  const add = (parent: Object3D, id: string, mesh: Mesh, at?: Vec3) => {
    mesh.id = id;
    if (at) mesh.position.set(...at);
    parent.add(mesh);
    return mesh;
  };

  // The plinth: a polished black stone disc with a chamfered edge.
  const ch = 1.2 * MM;
  const disc = new LatheGeometry([{ x: PLINTH.radius, y: -PLINTH.height }, { x: PLINTH.radius, y: -ch }, { x: PLINTH.radius - ch, y: 0 }, { x: 0, y: 0 }], 96);
  add(table, "plinth", new Mesh(disc, stone));

  const brilliant = brilliantGeometry();
  /** A part of a piece, placed in the piece's frame (not in the scene: `merge` makes one mesh of a piece's parts). */
  const part = (geometry: BufferGeometry, at: Vec3 = [0, 0, 0], scale = 1) => {
    const m = new Mesh(geometry, metal);
    m.position.set(...at);
    m.scale.set(scale, scale, scale);
    return m;
  };

  // A ring on its velvet holder: a group at the ring's centre, the band around its Z axis, the head at +Y.
  const ringOn = (piece: Exclude<Piece, "pearls">, outer: number) => {
    const s = STAND[piece], [x, z] = polar(s.r, s.at);
    add(table, `${piece}.holder`, new Mesh(holderGeometry(...s.block), velvet), [x, 0, z]);
    const ring = new Group();
    ring.id = `${piece}.ring`;
    ring.position.set(x, s.block[1] - 1.6 * MM + outer, z);   // 1.6 mm deep in the holder's slot
    ring.rotation.y = s.yaw;
    table.add(ring);
    return ring;
  };

  // 1. The solitaire: a tapered band, a six-prong head, an 8 mm (2 ct) brilliant.
  {
    const thick = 1.7 * MM, outer = RING_INNER + thick * 0.8, D = 8 * MM, yg = outer + 0.431 * D + 0.5 * MM;
    const ring = ringOn("solitaire", RING_INNER + thick);
    const rail = new TorusGeometry(2.75 * MM, 0.3 * MM, 10, 40);
    rail.rotateX(Math.PI / 2);
    const parts = [
      part(bandGeometry(RING_INNER, thick, 3.1 * MM, 2.6, 0.3)),
      part(new CylinderGeometry(1.5 * MM, 2.0 * MM, 1.6 * MM, 24), [0, outer + 0.3 * MM, 0]),   // the collet under the stone
      part(rail, [0, yg - 1.75 * MM, 0]),
    ];
    const tip = new SphereGeometry(0.52 * MM, 16, 10);
    for (let k = 0; k < 6; k++) {
      const a = (k * 60 + 30) * DEG, sx = Math.sin(a), sz = Math.cos(a);
      const at = (r: number, y: number) => new Vector3(sx * r, y, sz * r);
      const from = at(1.3 * MM, outer - 0.2 * MM), to = at(D / 2 + 0.2 * MM, yg + 0.25 * MM), len = from.distanceTo(to);
      const prong = part(new CylinderGeometry(0.36 * MM, 0.46 * MM, len, 12), from.clone().add(to).multiplyScalar(0.5).toArray());
      // (Its axis, +Y, along the prong: a turn about the horizontal axis square to it.)
      prong.quaternion.setFromAxisAngle(new Vector3(sz, 0, -sx), Math.acos((to.y - from.y) / len));
      parts.push(prong, part(tip, at(D / 2 - 0.1 * MM, yg + 0.5 * MM).toArray()));
    }
    add(ring, "solitaire.metal", new Mesh(merge(parts), metal));
    add(ring, "solitaire.stone", new Mesh(brilliant, gem), [0, yg, 0]).scale.set(D, D, D);
  }

  // 2. The half-eternity ring: seven 2.2 mm brilliants across the top, each pair sharing two prongs, so the stones
  // stand clear of the band with air under their pavilions (set into the metal they would have nothing to return).
  {
    const thick = 1.6 * MM, outer = RING_INNER + thick, D = 2.2 * MM, step = (2.42 * MM) / outer, lift = 0.95 * MM;
    const ring = ringOn("pave", outer);
    const bead = new SphereGeometry(0.36 * MM, 12, 8), post = new CylinderGeometry(0.24 * MM, 0.3 * MM, lift + 0.3 * MM, 8);
    /** At angle `a` from the top of the ring, `r` from its axis, `z` along it, its +Y pointing outwards. */
    const radial = (geometry: BufferGeometry, a: number, r: number, z: number, scale = 1) => {
      const m = part(geometry, [r * Math.sin(a), r * Math.cos(a), z], scale);
      m.rotation.z = -a;
      return m;
    };
    const parts = [part(bandGeometry(RING_INNER, thick, 2.5 * MM, 3.2))], stones: Mesh[] = [];
    for (let k = -3; k <= 3; k++) stones.push(radial(brilliant, k * step, outer + lift, 0, D));
    for (let k = -4; k <= 3; k++)
      for (const side of [-1, 1]) {
        const a = (k + 0.5) * step, z = side * 0.86 * MM;
        parts.push(radial(post, a, outer + (lift - 0.3 * MM) / 2, z), radial(bead, a, outer + lift + 0.12 * MM, z * 0.93));
      }
    add(ring, "pave.metal", new Mesh(merge(parts), metal));
    add(ring, "pave.stones", new Mesh(merge(stones), melee));
  }

  // 3. The plain band: 5 mm wide, domed outside, comfort-fit inside.
  {
    const thick = 1.9 * MM;
    add(ringOn("band", RING_INNER + thick), "band.metal", new Mesh(bandGeometry(RING_INNER, thick, 5 * MM, 2.3), metal));
  }

  // 4. The pearls: a bracelet on the stone around the middle holder, closed by a gold bead.
  {
    const ball = new SphereGeometry(PEARLS.radius, 32, 16), pearls: Mesh[] = [];
    for (let i = 0; i < PEARLS.count; i++) {
      const [x, z] = polar(BRACELET, PEARLS.clasp + ((i + 1) / (PEARLS.count + 1)) * 2 * Math.PI);
      pearls.push(part(ball, [x, PEARLS.radius, z]));
    }
    add(table, "pearls.strand", new Mesh(merge(pearls), pearl));
    const [x, z] = polar(BRACELET, PEARLS.clasp);
    add(table, "pearls.clasp", new Mesh(new SphereGeometry(2.2 * MM, 24, 14), metal), [x, 2.4 * MM, z]);
  }

  // The lights: two soft boxes, a top panel and a fill panel; their cards are what the metal and the stones mirror.
  const panel = (id: string, at: Vec3, size: [number, number], nits: number, color: Vec3) => {
    const l = new RectAreaLight(rgb(color), nits, ...size);
    l.id = id;
    l.position.set(...at);
    l.lookAt(0, 0.03, 0);
    studio.add(l);
  };
  panel("key", [-0.26, 0.3, 0.2], [0.34, 0.24], 9000, [1, 0.95, 0.88]);
  panel("strip", [0.32, 0.14, -0.06], [0.1, 0.42], 12000, [0.86, 0.93, 1]);
  panel("top", [0.03, 0.4, 0.0], [0.5, 0.4], 6000, [1, 0.98, 0.95]);
  panel("fill", [0.25, 0.12, 0.27], [0.3, 0.22], 3000, [1, 0.97, 0.93]);
  const spot = (id: string, at: Vec3, to: Vec3, candela: number, color: Vec3, angle: number, penumbra: number, radius: number) => {
    const l = new SpotLight(rgb(color), candela, 2, angle, penumbra), target = new Object3D();
    l.id = id;
    l.radius = radius;
    l.position.set(...at);
    target.position.set(...to);
    l.target = target;
    studio.add(l);
    studio.add(target);
  };
  // A snooted lamp on the paper behind the display: the glow the pieces stand against.
  spot("glow", [0, 0.55, 0], [0, 0, -0.17], 5500, [0.75, 0.82, 1], 16 * DEG, 1, 0.02);
  // Two pin spots, small and far: a facet or a point of metal lights up only where it mirrors one to the camera.
  spot("pin_a", [0.14, 0.36, 0.3], [0, 0.03, 0], 900, [1, 0.95, 0.85], 9 * DEG, 0.5, 0.012);
  spot("pin_b", [-0.1, 0.22, 0.42], [0, 0.03, 0], 700, [0.85, 0.93, 1], 9 * DEG, 0.5, 0.012);

  const camera = new PerspectiveCamera(32, 16 / 9, 0.005, 20);
  camera.id = "camera";
  camera.setManualExposure(11.2);
  scene.add(camera);
  return { scene, camera };
}

// ---------------------------------------------------------------------------------------------------------------
// The frame: the scene for the state. The table's angle and the camera ease towards where the state puts them.

/** The display's angle in the studio (rad), and the camera there: where it is and what it looks at. */
let turn = TURN_START;
const eye = new Vector3(), look = new Vector3();
const goalEye = new Vector3(), goalLook = new Vector3();
/** What the pieces are made of in the frame before, and the camera's nudge after that changes: its clock, time left. */
let shown = "", nudgeClock = 0, nudge = 0;

function frame(f: FrameContext<State>) {
  const s = f.state, view = VIEW[s.focus];

  const metal = (f.object("band.metal") as Mesh).material as MeshStandardMaterial;
  metal.color.setRGB(...METALS[s.metal % METALS.length].color);
  const gem = (f.object("solitaire.stone") as Mesh).material as MeshPhysicalMaterial;
  const st = STONES[s.stone % STONES.length];
  gem.color.setRGB(...st.color);
  if (gem.ior !== st.ior) { gem.ior = st.ior; gem.needsUpdate = true; }

  // The table: turning by itself, or holding the chosen piece in front of the camera and swaying a little.
  if (s.focus) {
    const goal = view.front + (s.spin ? SWAY * Math.sin(f.time * 0.4) : 0);
    const d = ((((goal - turn + Math.PI) % (2 * Math.PI)) + 2 * Math.PI) % (2 * Math.PI)) - Math.PI;
    turn = f.snap || Math.abs(d) < 1e-4 ? goal : turn + d * (1 - Math.exp(-f.dt * 2.5));
  } else if (f.snap) turn = TURN_START + (s.spin ? TURN_RATE * f.time : 0);
  else if (s.spin) turn += TURN_RATE * f.dt;

  // The camera in the studio: at the piece (which rides the table), from the front.
  const [tx, ty, tz] = view.target, c = Math.cos(turn), sn = Math.sin(turn);
  goalLook.set(tx * c + tz * sn, PLINTH.height + ty, -tx * sn + tz * c);
  goalEye.set(goalLook.x, goalLook.y + view.dist * Math.sin(view.elev), goalLook.z + view.dist * Math.cos(view.elev));
  // (Arrived means arrived: a camera that still creeps keeps the renderer from settling.)
  const k = f.snap || eye.distanceTo(goalEye) + look.distanceTo(goalLook) < 0.05 * MM ? 1 : 1 - Math.exp(-f.dt * 3.5);
  eye.lerp(goalEye, k);
  look.lerp(goalLook, k);

  // A new metal or stone: the camera shifts a fraction of a millimetre for a second. With everything dead still the
  // live view keeps its long history, and the stone would show patches of its last colour for a while.
  const showing = `${s.metal}/${s.stone}`;
  if (f.snap) nudge = nudgeClock = 0;
  else if (showing !== shown) nudge = 1;
  shown = showing;
  if (nudge > 0) { nudge -= f.dt; nudgeClock += f.dt; }
  const ex = eye.x + 0.4 * MM * Math.sin(nudgeClock * 1.3);

  // The display turns in the studio. Seen from the display, the studio (its lights and its paper) and the camera turn
  // the other way around it, and that is what the scene does: lights and a sheet of paper cost nothing to move, 40 000
  // triangles of jewellery do, and the live view follows a moving camera exactly but a moving mirror only roughly (a
  // turning ring left a trail on the backdrop below full render scale). The picture is the same.
  const studio = f.object("studio")!;
  if (studio.rotation.y !== -turn) studio.rotation.y = -turn;
  f.camera.position.set(ex * c - eye.z * sn, eye.y, ex * sn + eye.z * c);
  f.camera.lookAt(look.x * c - look.z * sn, look.y, look.x * sn + look.z * c);
}

export default defineGame<State>({
  state: { metal: 0, stone: 0, spin: true, focus: "" },

  setup() {
    const { scene, camera } = build();
    return { scene, camera, options: { name: "jewellery", preset: "preview" as const, width: 1280, height: 720, overrides: { native: { maxBounces: 8 } } } };
  },

  frame,

  handlers: {
    /** A piece (any part of it, or its holder): go close; the piece the camera is at, or anything else: step back. */
    click(tx, ev: { target?: string }) {
      const piece = PIECES.find((p) => ev.target?.startsWith(p + "."));
      tx.decide(piece !== undefined || tx.state.focus !== "", "click a ring or the pearls");
      tx.state.focus = piece === undefined || piece === tx.state.focus ? "" : piece;
    },
    metal(tx) {
      tx.state.metal = (tx.state.metal + 1) % METALS.length;
    },
    stone(tx) {
      tx.state.stone = (tx.state.stone + 1) % STONES.length;
    },
    spin(tx) {
      tx.state.spin = !tx.state.spin;
    },
    back(tx) {
      tx.decide(tx.state.focus !== "", "already looking at the whole display");
      tx.state.focus = "";
    },
  },

  keys: { M: "metal", G: "stone", Space: "spin", B: "back" },
});
