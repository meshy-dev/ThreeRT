// The benchmark level under explosions (ROADMAP § 6 steps 2c-4 and 6; tools/bench-destruction.ts): the level's voxels
// loaded by the renderer's native voxel objects (renderer/src/voxel.rs), as meshes or as procedural boxes, the camera
// still at the camera path's start. An explosion's voxels are gone (debris); what it leaves hanging on nothing falls
// as a new object and rests where it lands (connectivity: step 6). An event picks the scenario:
//   singles    one explosion of 0.5, 1, 2 and 4 m, two seconds apart, each at a fresh spot
//   sustained  7 explosions a second (1 m) for 10 s
//   peak       20 explosions (1 m) within one second
//   stress     one explosion (1 m) every frame for 10 s: where it breaks
//   endurance  one explosion (1 m) every frame for 60 s (ROADMAP § 6 step 5: the edit room's holes and growth)
//   show       one explosion (1.5 m) every 1.5 s for 12 s on the road straight ahead of the camera (to watch: dust)
//   crater     one explosion of 4 m on the open road ahead (the largest: 140,785 voxels of terrain, five times the
//              wall's 4 m of `singles`)
//   topple     one explosion of 0.6 m every half second at the trunks of the eight trees nearest ahead: each crown is
//              cut loose, falls and rests on its stump (explosions that split)
//   fell       a saw cut (a box 1 m thick, 30 x 30 m) through everything from 0.5 m above the road ahead: trees, lamp
//              posts, cars and the buildings there lose their footing and fall 1 m (the largest loose parts)
//   shatter    `fell`, then from two seconds on a cut 0.2 m thick every tenth of a second through what fell, 30 a
//              metre apart across the road and 30 along it: the bodies are cut into more and more bodies (ROADMAP
//              § 6 step 8: how many bodies a frame holds, and the cap on them)
//   shatter-dense  the same with the cuts half a metre apart and twenty a second: more bodies, sooner (past 500:
//              the level's cap is reached and held)
// With `debris`
// (level-destruction-debris.game.ts; ROADMAP § 6 step 2e), the voxels too few to make a piece are thrown out as
// instances of one unit cube (an `InstancedMesh`, coloured per instance from the palette), fall, bounce on the ground
// and are gone after 4 s; every frame rewrites their matrices, in place. With `debris` "dust"
// (level-destruction-dust.game.ts; ROADMAP § 6 step 5a) they become stateful GPU particles instead (`GpuParticles`):
// thrown out from the explosion, collided with the ground and the level's voxels (its occupancy grid), no CPU work
// per particle. With `smoke` (level-destruction-smoke.game.ts; ROADMAP § 6 step 5b), every explosion leaves a puff of
// simulated smoke (`SimulatedSmoke`: a density grid over the streets ahead of the camera, advected, diffused and lifted
// on the GPU every frame), which rises and drifts with the wind. With `flames` instead (level-destruction-fire.game.ts;
// ROADMAP § 6 step 10 b), every explosion is `Explosions.explode`: the voxel explosion, a fireball from a pool of 8
// (cooling to its own dark smoke), and a light following it. Spots alternate between the ground ahead
// of the camera and the walls of the buildings nearest to it. Each explosion
// logs what it did (`explosion {...}`, renderer timings included) for the bench. Sun and sky; the lamps do not glow (at
// night they do, and their pieces fly: level-night.game.ts).
//   node tools/voxel-level/generate.ts --granularity object && node tools/bundle-game.ts games/voxel-level/level-destruction.game.ts
import { BoxGeometry, Color, DirectionalLight, GpuParticles, InstancedMesh, MaterialFunction, Mesh, MeshStandardMaterial, ParticleProgram, SimulatedSmoke, PerspectiveCamera, PlaneGeometry, ProceduralGeometry, ProceduralMesh, Scene, SkyAtmosphere } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { Explosions, VOXEL_BRICKS, voxelBodies, voxelCarve, voxelDebris, voxelExplode, voxelFrame, voxelHash, voxelLevel, type VoxelFrame } from "@threert/voxel";
import { nativeObjects, nativePalette } from "./native.ts";
import { EDGE, PALETTE, SIZE, VOXEL, cameraPath, groundHeight, levelPlan, toScene } from "./plan.ts";

type Scenario = "singles" | "sustained" | "peak" | "stress" | "endurance" | "show" | "crater" | "fell" | "topple" | "shatter" | "shatter-dense";
interface State { scenario: Scenario | null; started: number }

/** Spots for explosions: alternately on the ground ahead of the camera and on the walls of the buildings nearest it
 *  (deterministic: the k-th spot is always the same). */
export function spots(seed: number): [number, number, number][] {
  const plan = levelPlan(seed);
  const key = cameraPath(seed)[0];
  const [cx, , cz] = key.pos, [tx, , tz] = key.target;
  const fwd = [tx - cx, tz - cz], len = Math.hypot(fwd[0], fwd[1]);
  const [fx, fz] = [fwd[0] / len, fwd[1] / len];
  const ground: [number, number, number][] = [];
  for (let k = 0; k < 160; k++) {
    const d = 6 + (k % 20) * 1.5, side = ((Math.floor(k / 20) % 8) - 3.5) * 2.5;
    const x = cx + fx * d - fz * side, z = cz + fz * d + fx * side;
    const vx = Math.round(x / VOXEL + SIZE / 2), vz = Math.round(z / VOXEL + SIZE / 2);
    ground.push([x, groundHeight(seed, vx, vz) * VOXEL, z]);
  }
  const near = plan.buildings.map((b) => ({ b, d: Math.hypot(toScene(b.x, 0, b.z)[0] - cx, toScene(b.x, 0, b.z)[2] - cz) })).sort((p, q) => p.d - q.d).slice(0, 8);
  const walls: [number, number, number][] = [];
  for (let k = 0; k < 160; k++) {
    const { b } = near[k % near.length];
    const along = ((k * 7) % 10) / 10, high = 1 + ((k * 3) % 5);
    const [x, y, z] = toScene(b.x + along * b.w, b.base + 6, b.z);
    walls.push([x, y + high, z]);
  }
  return ground.flatMap((g, k) => [g, walls[k]]);
}

/** Debris: up to `DEBRIS` voxels in flight, each `LIFE` seconds. */
const DEBRIS = 8192, LIFE = 4;

/** Thrown debris (`destructionGame`'s `debris`): one InstancedMesh of a unit cube; positions, velocities, floors and
 *  ages in typed arrays, the live ones first (a dead one is swapped with the last). */
class Debris {
  readonly mesh = new InstancedMesh(new BoxGeometry(1, 1, 1), new MeshStandardMaterial({ name: "debris", color: 0xffffff, roughness: 0.9 }), DEBRIS);
  private readonly p = new Float32Array(DEBRIS * 3);
  private readonly v = new Float32Array(DEBRIS * 3);
  private readonly floor = new Float32Array(DEBRIS);
  private readonly age = new Float32Array(DEBRIS);
  private readonly spin = new Float32Array(DEBRIS);
  private alive = 0;
  private rand = 0x9e3779b9;
  private readonly colors: Color[];
  private readonly seed: number;
  constructor(seed: number) {
    this.seed = seed;
    this.mesh.name = "debris";
    this.mesh.count = 0;
    this.mesh.setColorAt(0, new Color(0xffffff)); // colours from the start: their first use would rebuild the scene once
    this.colors = PALETTE.map(([, hex]) => new Color(hex));
  }
  private random(): number {
    this.rand = (this.rand + 0x6d2b79f5) | 0;
    let t = Math.imul(this.rand ^ (this.rand >>> 15), 1 | this.rand);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  }
  /** Throws `d` (`voxelDebris`: x, y, z, palette index each) out from the explosion at c. */
  spawn(d: Float32Array, c: [number, number, number]): number {
    let n = 0;
    for (let k = 0; k + 3 < d.length && this.alive < DEBRIS; k += 4, n++) {
      const i = this.alive++;
      const [x, y, z] = [d[k], d[k + 1], d[k + 2]];
      let [dx, dy, dz] = [x - c[0], y - c[1], z - c[2]];
      const len = Math.hypot(dx, dy, dz) || 1;
      const speed = 3 + 5 * this.random();
      [dx, dy, dz] = [(dx / len) * speed, (dy / len) * speed + 2 + 3 * this.random(), (dz / len) * speed];
      this.p.set([x, y, z], i * 3);
      this.v.set([dx, dy, dz], i * 3);
      this.floor[i] = groundHeight(this.seed, Math.round(x / VOXEL + SIZE / 2), Math.round(z / VOXEL + SIZE / 2)) * VOXEL + VOXEL / 2;
      this.age[i] = 0;
      this.spin[i] = (this.random() - 0.5) * 20;
      this.colors[d[k + 3]]?.toArray(this.mesh.instanceColor!.array, i * 3);
    }
    if (n) this.mesh.instanceColor!.needsUpdate = true;
    return n;
  }
  /** Moves the live debris one step (gravity, a damped bounce on the ground), retires the old, writes the matrices. */
  step(dt: number): void {
    if (this.alive === 0 && this.mesh.count === 0) return;
    const { p, v } = this;
    const m = this.mesh.instanceMatrix.array, col = this.mesh.instanceColor!.array;
    for (let i = 0; i < this.alive; ) {
      this.age[i] += dt;
      if (this.age[i] > LIFE) {
        const j = --this.alive;
        p.copyWithin(i * 3, j * 3, j * 3 + 3);
        v.copyWithin(i * 3, j * 3, j * 3 + 3);
        col.copyWithin(i * 3, j * 3, j * 3 + 3);
        [this.floor[i], this.age[i], this.spin[i]] = [this.floor[j], this.age[j], this.spin[j]];
        continue;
      }
      const o = i * 3;
      v[o + 1] -= 9.81 * dt;
      p[o] += v[o] * dt;
      p[o + 1] += v[o + 1] * dt;
      p[o + 2] += v[o + 2] * dt;
      if (p[o + 1] < this.floor[i]) {
        p[o + 1] = this.floor[i];
        v[o + 1] *= -0.3;
        v[o] *= 0.6;
        v[o + 2] *= 0.6;
        this.spin[i] *= 0.6;
      }
      const a = this.spin[i] * this.age[i], c = Math.cos(a) * VOXEL, s = Math.sin(a) * VOXEL, e = i * 16;
      m[e] = c; m[e + 1] = 0; m[e + 2] = -s; m[e + 3] = 0;
      m[e + 4] = 0; m[e + 5] = VOXEL; m[e + 6] = 0; m[e + 7] = 0;
      m[e + 8] = s; m[e + 9] = 0; m[e + 10] = c; m[e + 11] = 0;
      m[e + 12] = p[o]; m[e + 13] = p[o + 1]; m[e + 14] = p[o + 2]; m[e + 15] = 1;
      i++;
    }
    if (this.mesh.count !== this.alive) this.mesh.instanceColor!.needsUpdate = true; // swaps moved colours
    this.mesh.count = this.alive;
    this.mesh.instanceMatrix.needsUpdate = true;
  }
  get live(): number {
    return this.alive;
  }
}

/** Dust: a debris voxel thrown out from the explosion (the node's position when it was spawned, `i.pivot`), falling with a
 *  little drag, shrinking away at the end of its 2.5-4 s life. Collision (ground, voxels) is the engine's. */
const DUST = new ParticleProgram("voxel_dust", {
  params: { Speed: 4, Up: 2.5, Size: 0.022, Gravity: 9.81, Drag: 0.6 },
  emit: `
    let out = i.at - i.pivot;
    let d = select(vec3<f32>(0.0, 1.0, 0.0), normalize(out), dot(out, out) > 1e-8);
    s.vel = d * ($Speed * (0.5 + i.rand.x)) + vec3<f32>(0.0, $Up * (0.5 + i.rand.y), 0.0);
    s.life = 2.5 + 1.5 * i.rand.z;
    s.size = $Size * (0.7 + 0.6 * i.rand.w);
    s.color = vec4<f32>(i.color.rgb, s.size);`,
  update: `
    s.vel.y -= $Gravity * i.dt;
    s.vel *= exp(-$Drag * i.dt);
    s.size = s.color.a * (1.0 - smoothstep(0.75, 1.0, s.age / s.life));`,
});

/** Dust's surface: its voxel's colour (`i.custom`, the particle's colour). */
const DUST_SURFACE = new MaterialFunction("voxel_dust_surface", { code: `
    o.base = i.custom.rgb;` });

/** Debris as dust (`debris` "dust"): every voxel too few to make a piece, `PER` GPU particles of its colour spread over
 *  its cube. */
class Dust {
  readonly node: GpuParticles;
  private readonly colors: Color[];
  constructor() {
    const material = new MeshStandardMaterial({ name: "dust", color: 0xffffff, roughness: 0.95 });
    material.shader = DUST_SURFACE.with({});
    this.node = new GpuParticles({ capacity: 1 << 16, program: DUST.with(), material, collide: { plane: EDGE * VOXEL, voxels: true, restitution: 0.25, friction: 0.5 } });
    this.node.name = "dust";
    this.node.movable = true;
    this.colors = PALETTE.map(([, hex]) => new Color(hex));
  }
  static readonly PER = 4;
  /** `d` (`voxelDebris`: x, y, z, palette index each) thrown out from the explosion at c. */
  spawn(d: Float32Array, c: [number, number, number]): number {
    const n = Math.floor(d.length / 4), per = Dust.PER, data = new Float32Array(n * per * 8), q = VOXEL / 4;
    for (let k = 0; k < n; k++) {
      const col = this.colors[d[k * 4 + 3]] ?? this.colors[0];
      for (let j = 0; j < per; j++) {
        const o = [(j & 1 ? q : -q), (j & 2 ? q : -q), (j & 1) ^ ((j >> 1) & 1) ? q : -q];
        data.set([d[k * 4] + o[0], d[k * 4 + 1] + o[1], d[k * 4 + 2] + o[2], 0, col.r, col.g, col.b, 1], (k * per + j) * 8);
      }
    }
    this.node.position.set(...c);
    this.node.spawn(data);
    return n * per;
  }
}

/** `debris`: the removed voxels thrown out as instances (`Debris`; logged per explosion), or "dust": as GPU particles.
 *  `smoke`: the cells of the simulated smoke's grid. `physics` (ROADMAP § 6 step 8; on by default): the level
 *  collides and what comes loose is a rigid body, logged every half second while there are any as `bodies {...}`
 *  (how many, how many sleep, the fastest); off, loose parts fall through the lattice as in step 6 and are logged
 *  per frame as `batch {...}`. `maxBodies`: the level's cap on voxel bodies (300 unless given). `flames` (ROADMAP § 6
 *  step 10 b): every explosion through `Explosions` (fireball and light); not with `smoke` (volumes must not overlap). */
export function destructionGame(route: "mesh" | "boxes", seed = 1, debris: boolean | "dust" = false, smoke: false | [number, number, number] = false, physics = true, maxBodies?: number, flames = false) {
  const keys = cameraPath(seed);
  const at = spots(seed);
  // The ground points of the eight trees nearest ahead of the camera (`topple`).
  const [camX, , camZ] = keys[0].pos;
  const trunks = levelPlan(seed).props.filter((p) => p.kind === "tree").map((p) => toScene(p.x, p.y, p.z)).filter((p) => p[2] > camZ + 4).sort((a, b) => Math.hypot(a[0] - camX, a[2] - camZ) - Math.hypot(b[0] - camX, b[2] - camZ)).slice(0, 8);
  let next = 0, fired = 0, frames = 0, last = -1, hadBodies = false, now = 0;
  const flying = debris === true ? new Debris(seed) : null;
  const dust = debris === "dust" ? new Dust() : null;
  // Smoke: a box 22 x 14 x 36 m over the streets ahead of the camera (from 3 m ahead; the level's +z), `smoke` cells
  // across it, a light wind across the road.
  const [cx, cy, cz] = keys[0].pos;
  // Fireballs and the smoke grid are not both on: the grid's box is a volume, and the fireballs would be in it (volumes
  // must not overlap; docs/VOXEL.md, "Temporary ways round on the engine's shared paths").
  if (flames && smoke) throw new Error("destructionGame: fire and the smoke grid are not both on (volumes must not overlap)");
  const boom = flames ? new Explosions() : null;
  const cloud = smoke ? new SimulatedSmoke({ size: smoke, density: 2.5, wind: [0.8, 0, 0.3], buoyancy: 3.5, dissipation: 0.2, cooling: 0.9 }) : null;
  if (cloud) {
    cloud.position.set(cx, cy + 5, cz + 21);
    cloud.scale.set(22, 14, 36);
  }
  const fire = (r: number, spot?: number) => {
    const [x, y, z] = at[(spot ?? next++) % at.length];
    const e = boom ? boom.explode(x, y, z, r, now) : voxelExplode(x, y, z, r);
    const thrown = flying ? flying.spawn(voxelDebris(), [x, y, z]) : dust ? dust.spawn(voxelDebris(), [x, y, z]) : 0;
    cloud?.puff([x, y + 0.3 * r, z], 0.8 * r + 0.3, { density: 0.9, temperature: 1, push: 1.2 });
    // (With physics: the voxel bodies' top speed just after it, its blast's push included: step 10 c1.)
    const fastest = physics ? +voxelBodies().reduce((n, x) => Math.max(n, x.getLinearVelocity().length()), 0).toFixed(2) : undefined;
    console.log(`explosion ${JSON.stringify({ k: fired++, frame: frames, r, ...e, ...(flying ? { thrown, debris: flying.live } : dust ? { thrown } : {}), ...(boom ? { burning: boom.stats.burning, lit: boom.stats.lit } : {}), ...(fastest !== undefined ? { fastest } : {}) })}`);
  };
  return defineGame<State>({
    state: { scenario: null, started: 0 },
    setup: () => {
      const scene = new Scene();
      const sun = new DirectionalLight(0xffffff, 90000);
      sun.setSunAngles(38, 300);
      scene.add(sun);
      scene.environment = new SkyAtmosphere({ sun });
      const camera = new PerspectiveCamera(55, 16 / 9, 0.05, 2000);
      camera.name = "camera";
      camera.position.set(...keys[0].pos);
      camera.lookAt(...keys[0].target);
      camera.setManualExposure(13);
      scene.add(camera);
      const ground = new MeshStandardMaterial({ name: "ground", color: PALETTE[1][1], roughness: 0.9 });
      const half = (SIZE / 2) * VOXEL, far = 100000;
      for (const [w, d, x, z] of [[2 * far, far - half, 0, -(far + half) / 2], [2 * far, far - half, 0, (far + half) / 2], [far - half, 2 * half, -(far + half) / 2, 0], [far - half, 2 * half, (far + half) / 2, 0]]) {
        const strip = new Mesh(new PlaneGeometry(w, d), ground);
        strip.rotation.x = -Math.PI / 2;
        strip.position.set(x, EDGE * VOXEL, z);
        scene.add(strip);
      }
      // The boxes' geometry function, registered by a hidden procedural mesh.
      const fn = new ProceduralMesh(new ProceduralGeometry(VOXEL_BRICKS, [0, 0, 0, 0.1, 0.1, 0.1], new Array(12).fill(0)), ground);
      fn.visible = false;
      scene.add(fn);
      voxelLevel({ route, palette: nativePalette(false), function: VOXEL_BRICKS.name, objects: nativeObjects(seed), debris: !!debris, physics, ...(maxBodies === undefined ? {} : { maxBodies }), ...(dust ? { occupancy: 0.2 } : {}) });
      if (flying) scene.add(flying.mesh);
      if (dust) scene.add(dust.node);
      if (cloud) scene.add(cloud);
      if (boom) scene.add(boom);
      return { scene, camera, options: { name: `voxel-level-destruction-${route}${debris === true ? "-debris" : debris ? "-dust" : ""}${smoke ? "-smoke" : ""}${flames ? "-fire" : ""}${physics ? "" : "-lattice"}`, width: 1280, height: 720, preset: "cinematic" as const } };
    },
    frame(f) {
      frames++;
      now = f.time;
      boom?.step(now);
      flying?.step(f.dt);
      const b = voxelFrame(f.dt) as VoxelFrame; // (the level is loaded by the first frame)
      if (b.batches > 0 || (b.settled ?? 0) > 0) console.log(`batch ${JSON.stringify(b)}`);
      if (physics && frames % 30 === 0) {
        const bodies = voxelBodies();
        if (bodies.length > 0 || hadBodies) console.log(`bodies ${JSON.stringify({ frame: frames, n: bodies.length, asleep: bodies.filter((x) => x.asleep).length, voxels: bodies.reduce((n, x) => n + x.voxels, 0), largest: bodies.reduce((n, x) => Math.max(n, x.voxels), 0), fastest: +bodies.reduce((n, x) => Math.max(n, x.getLinearVelocity().length()), 0).toFixed(2) })}`);
        hadBodies = bodies.length > 0;
      }
      const s = f.state.scenario;
      if (!s) return;
      const t = (frames - f.state.started) / 60; // frames since the scenario began, at 60 per second of frame script
      if (s === "singles") {
        const k = Math.floor(t / 2);
        if (k < 4 && k !== last && t >= 1 + k * 2 - 1) {
          last = k;
          fire([0.5, 1, 2, 4][k]);
        }
      } else if (s === "sustained") {
        const k = Math.floor(t * 7);
        if (t < 10 && k !== last) {
          last = k;
          fire(1);
        }
      } else if (s === "peak") {
        const k = Math.floor(t * 20);
        if (t < 1 && k !== last) {
          last = k;
          fire(1);
        }
      } else if (s === "topple") {
        const k = Math.floor(t * 2);
        if (k < trunks.length && k !== last) {
          last = k;
          const [x, y, z] = trunks[k];
          const e = voxelExplode(x, y + 0.8, z, 0.6);
          const thrown = flying ? flying.spawn(voxelDebris(), [x, y + 0.8, z]) : dust ? dust.spawn(voxelDebris(), [x, y + 0.8, z]) : 0;
          console.log(`explosion ${JSON.stringify({ k: fired++, frame: frames, r: 0.6, ...e, ...(flying || dust ? { thrown } : {}) })}`);
        }
      } else if (s === "fell") {
        if (t >= 1 && last !== 0) {
          last = 0;
          const [x, y, z] = at[2 * 62];
          const e = voxelCarve({ box: [x - 15, y + 0.5, z - 6, x + 15, y + 1.5, z + 24] });
          console.log(`explosion ${JSON.stringify({ k: fired++, frame: frames, r: 0, cut: true, ...e })}`);
        }
      } else if (s === "shatter" || s === "shatter-dense") {
        const [x, y, z] = at[2 * 62];
        if (t >= 1 && last < 0) {
          last = 0;
          const e = voxelCarve({ box: [x - 15, y + 0.5, z - 6, x + 15, y + 1.5, z + 24] });
          console.log(`explosion ${JSON.stringify({ k: fired++, frame: frames, r: 0, cut: true, ...e })}`);
        }
        // (From 0.3 m above the road up: the cuts go through what stands and what fell, not into the ground.)
        // Cuts 0.2 m thick, `gap` m apart, `rate` a second: across the road's 30 m, then along them.
        const [gap, rate] = s === "shatter" ? [1, 10] : [0.5, 20];
        const n = 30 / gap, k = Math.floor((t - 3) * rate) + 1;
        if (t >= 3 && k <= 2 * n && k !== last) {
          last = k;
          const along = k <= n;
          const p = (along ? x - 15 : z - 6) + (along ? k : k - n) * gap - gap / 2;
          const e = voxelCarve({ box: along ? [p - 0.1, y + 0.3, z - 6, p + 0.1, y + 40, z + 24] : [x - 15, y + 0.3, p - 0.1, x + 15, y + 40, p + 0.1] });
          console.log(`explosion ${JSON.stringify({ k: fired++, frame: frames, r: 0, cut: true, ...e })}`);
        }
      } else if (s === "crater") {
        // Ground spot 62: 9 m ahead on the road.
        if (t >= 1 && last !== 0) {
          last = 0;
          fire(4, 2 * 62);
        }
      } else if (s === "show") {
        // Ground spots 60-79 (spots() interleaves ground and walls): 6 m ahead and on, just left of the camera's line.
        const k = Math.floor(t / 1.5);
        if (t < 12 && k !== last) {
          last = k;
          fire(1.5, 2 * (60 + k));
        }
      } else if (t < (s === "endurance" ? 60 : 10)) {
        fire(1);
      }
    },
    handlers: {
      singles: (tx) => { tx.state.scenario = "singles"; tx.state.started = frames; last = -1; },
      sustained: (tx) => { tx.state.scenario = "sustained"; tx.state.started = frames; last = -1; },
      peak: (tx) => { tx.state.scenario = "peak"; tx.state.started = frames; last = -1; },
      stress: (tx) => { tx.state.scenario = "stress"; tx.state.started = frames; last = -1; },
      endurance: (tx) => { tx.state.scenario = "endurance"; tx.state.started = frames; last = -1; },
      show: (tx) => { tx.state.scenario = "show"; tx.state.started = frames; last = -1; },
      crater: (tx) => { tx.state.scenario = "crater"; tx.state.started = frames; last = -1; },
      fell: (tx) => { tx.state.scenario = "fell"; tx.state.started = frames; last = -1; },
      shatter: (tx) => { tx.state.scenario = "shatter"; tx.state.started = frames; last = -1; },
      "shatter-dense": (tx) => { tx.state.scenario = "shatter-dense"; tx.state.started = frames; last = -1; },
      topple: (tx) => { tx.state.scenario = "topple"; tx.state.started = frames; last = -1; },
      // The level's hash, logged (`hash <16 hex digits>`): the same events at the same frames give the same one.
      // With physics, the physics world's hash too (the bodies' poses and velocities, bit for bit).
      hash: () => {
        const world = physics ? (JSON.parse((globalThis as { __engine?: { physics?: (op: string, a: object) => string } }).__engine?.physics?.("hash", {}) ?? "{}") as { hash?: string }).hash : undefined;
        console.log(`hash ${voxelHash()}${world ? ` physics ${world}` : ""} frame ${frames}`);
      },
    },
  });
}

export default destructionGame("mesh");
