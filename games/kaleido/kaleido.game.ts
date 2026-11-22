// Kaleido on ThreeRT (games/kaleido/sim.ts): a hall of two facing mirrors, glass and chrome, lit by nothing but
// hundreds of glowing orbs that pour from the ceiling, pile up and are blown apart. Every orb is a light with its own
// shadows (the light tree picks among them per pixel), so a pile is dark inside and lights the floor around it; the
// mirrors reflect each other and everything in the hall, the glass refracts it, the plaster and ceiling take the
// orbs' colours bounced off the floor and each other, and a blast's flash lights the whole hall at once.
//   node tools/kaleido/bench.ts           (both engines, measured)
//   node tools/bundle-game.ts games/kaleido/kaleido.game.ts -o out/bundles/kaleido.js
//   renderer/target/release/threert-render play out/bundles/kaleido.js --scale 0.5 --reconstruction metalfx-denoised
// Options: `wait` (the bench: nothing moves until a "go" event, so frame k of a lockstep run is the show's frame k),
// `record` (each frame's poses and glows on stdout, for the three.js page to replay), `still` (a recorded frame, held:
// the offline reference's scene).
import { BoxGeometry, Color, Mesh, MeshPhysicalMaterial, MeshStandardMaterial, PerspectiveCamera, Scene, SkyColor, SphereGeometry, Vector3 } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { BALLS, BLAST, BODY, GLOW_NITS, HALL, LOOK, MONOLITHS, PALETTE, PARK, STATIC, Show, blastAt, cameraAt, flashOf, parkedAt, type Actuator } from "./sim.ts";

const REPORT_EVERY = 60;

export interface Still { t: number; poses: number[]; glows: number[] }

/** Base64 of a float array's bytes (QuickJS has no btoa). */
function b64(a: Float32Array): string {
  const T = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
  const u = new Uint8Array(a.buffer, a.byteOffset, a.byteLength);
  let out = "";
  let i = 0;
  for (; i + 2 < u.length; i += 3) {
    const n = (u[i] << 16) | (u[i + 1] << 8) | u[i + 2];
    out += T[n >> 18] + T[(n >> 12) & 63] + T[(n >> 6) & 63] + T[n & 63];
  }
  if (i < u.length) {
    const n = (u[i] << 16) | ((i + 1 < u.length ? u[i + 1] : 0) << 8);
    out += T[n >> 18] + T[(n >> 12) & 63] + (i + 1 < u.length ? T[(n >> 6) & 63] : "=") + "=";
  }
  return out;
}

export function makeGame(count: number, o: { width?: number; height?: number; wait?: boolean; record?: boolean; still?: Still; logic?: boolean } = {}) {
  let meshes: Mesh[] = [];
  let show: Show | null = null;
  let camera: PerspectiveCamera | null = null;
  let blast: Mesh | null = null;
  let live: Uint8Array = new Uint8Array(0);
  const v = new Vector3(), zero = new Vector3();
  let pos = new Float32Array(0), rec = new Float32Array(0), recGlow = new Float32Array(0);
  const act: Actuator = {
    teleport(i, p, vel) {
      const m = meshes[i];
      m.position.set(p[0], p[1], p[2]);
      m.quaternion.set(0, 0, 0, 1);
      m.setLinearVelocity(v.set(vel[0], vel[1], vel[2]));
      m.setAngularVelocity(zero);
      if (!live[i] && show!.orbs[i].kind === "glow") m.power = show!.orbs[i].gain * show!.glow[i];
      live[i] = 1;
    },
    impulse(i, x, y, z) {
      meshes[i].applyImpulse(v.set(x, y, z));
    },
    glow(i, g) {
      if (live[i]) meshes[i].power = show!.orbs[i].gain * g;
    },
  };
  const positions = () => {
    for (let i = 0; i < meshes.length; i++) {
      const p = meshes[i].position;
      pos[i * 3] = p.x; pos[i * 3 + 1] = p.y; pos[i * 3 + 2] = p.z;
    }
    return pos;
  };
  /** The blast's fireball and the camera at time t. */
  const place = (t: number) => {
    const b = blastAt(t);
    if (b) {
      const f = flashOf(b);
      blast!.position.set(...b.at);
      blast!.scale.setScalar(f.radius);
      blast!.power = f.power;
    } else if (blast!.power !== 0) {
      blast!.power = 0;
      blast!.position.set(0, PARK.y - 5, PARK.z);
    }
    const c = cameraAt(t);
    camera!.position.set(...c.pos);
    camera!.lookAt(...c.target);
  };
  return defineGame({
    state: { go: !o.wait },
    handlers: { go(tx) { tx.state.go = true; } },
    setup() {
      const scene = new Scene();
      scene.environment = new SkyColor(0x000000, 0);
      const physical = !o.still;
      const box = (w: number, h: number, d: number, x: number, y: number, z: number, m: MeshStandardMaterial, name: string, yaw = 0) => {
        const mesh = new Mesh(new BoxGeometry(w, h, d), m);
        mesh.name = name;
        mesh.position.set(x, y, z);
        mesh.rotation.y = yaw;
        if (physical) mesh.physics = { type: "fixed", ...STATIC };
        scene.add(mesh);
        return mesh;
      };
      const { hx, z0, z1, hy } = HALL, len = z1 - z0, zc = (z0 + z1) / 2;
      const floor = new MeshStandardMaterial({ name: "floor", color: LOOK.floor.color, roughness: LOOK.floor.roughness });
      const mirror = new MeshStandardMaterial({ name: "mirror", color: LOOK.mirror.color, metalness: 1, roughness: LOOK.mirror.roughness });
      const plaster = new MeshStandardMaterial({ name: "plaster", color: LOOK.plaster.color, roughness: LOOK.plaster.roughness });
      const ceiling = new MeshStandardMaterial({ name: "ceiling", color: LOOK.ceiling.color, roughness: LOOK.ceiling.roughness });
      const glass = new MeshPhysicalMaterial({ name: "glass", color: 0xffffff, roughness: 0, transmission: 1, ior: LOOK.glass.ior });
      const chrome = new MeshStandardMaterial({ name: "chrome", color: LOOK.chrome.color, metalness: 1, roughness: LOOK.chrome.roughness });
      box(2 * hx + 0.4, 0.2, len + 0.4, 0, -0.1, zc, floor, "floor");
      box(2 * hx + 0.4, 0.2, len + 0.4, 0, hy + 0.1, zc, ceiling, "ceiling");
      box(0.1, hy, len, -hx - 0.05, hy / 2, zc, mirror, "mirrorL");
      box(0.1, hy, len, hx + 0.05, hy / 2, zc, mirror, "mirrorR");
      box(2 * hx, hy, 0.2, 0, hy / 2, z0 - 0.1, plaster, "back");
      box(2 * hx, hy, 0.2, 0, hy / 2, z1 + 0.1, plaster, "front");
      MONOLITHS.forEach((m, k) => box(m.half[0] * 2, m.half[1] * 2, m.half[2] * 2, m.at[0], m.half[1], m.at[2], glass, `monolith${k}`, m.yaw));
      BALLS.forEach((b, k) => {
        const mesh = new Mesh(new SphereGeometry(b.r, 64, 32), b.kind === "glass" ? glass : chrome);
        mesh.name = `ball${k}`;
        mesh.position.set(...b.at);
        if (physical) mesh.physics = { type: "fixed", ...STATIC };
        scene.add(mesh);
      });
      // Where the orbs wait, far below.
      box(2 * PARK.half, 0.5, 2 * PARK.half, 0, PARK.y, PARK.z, ceiling, "park");

      show = new Show(count);
      live = new Uint8Array(count);
      pos = new Float32Array(count * 3);
      const sphere = new SphereGeometry(1, 16, 8);
      const team = PALETTE.map((c, k) => new MeshStandardMaterial({ name: `orb${k}`, color: new Color(c).multiplyScalar(0.2), roughness: 0.35, emissive: c, emissiveIntensity: GLOW_NITS }));
      const orbChrome = new MeshStandardMaterial({ name: "orbChrome", color: LOOK.chrome.color, metalness: 1, roughness: LOOK.chrome.roughness });
      meshes = show.orbs.map((orb, i) => {
        const m = new Mesh(sphere, orb.kind === "glow" ? team[orb.team] : orb.kind === "chrome" ? orbChrome : glass);
        m.name = `orb${i}`;
        m.scale.setScalar(orb.r);
        const p = parkedAt(i);
        m.position.set(p[0], PARK.y + 0.25 + orb.r, p[2]);
        if (orb.kind === "glow") m.power = 0; // dark until it pours
        if (physical) m.physics = { type: "dynamic", density: BODY.density, friction: BODY.friction, restitution: BODY.restitution, linearDamping: BODY.linearDamping, angularDamping: BODY.angularDamping, ccd: true };
        scene.add(m);
        return m;
      });
      blast = new Mesh(new SphereGeometry(1, 24, 12), new MeshStandardMaterial({ name: "blast", color: 0x000000, emissive: 0xffe2c0, emissiveIntensity: BLAST.nits }));
      blast.name = "blast";
      blast.power = 0;
      blast.position.set(0, PARK.y - 5, PARK.z);
      scene.add(blast);

      camera = new PerspectiveCamera(cameraAt(0).fov, (o.width ?? 1920) / (o.height ?? 1080), 0.05, 200);
      camera.setManualExposure(LOOK.ev100);
      scene.add(camera);
      if (o.still) {
        // A recorded frame: the orbs where they were, as bright as they were.
        const s = o.still;
        meshes.forEach((m, i) => {
          m.position.set(s.poses[i * 7], s.poses[i * 7 + 1], s.poses[i * 7 + 2]);
          m.quaternion.set(s.poses[i * 7 + 3], s.poses[i * 7 + 4], s.poses[i * 7 + 5], s.poses[i * 7 + 6]);
          if (show!.orbs[i].kind === "glow") m.power = show!.orbs[i].gain * s.glows[i];
        });
        place(s.t);
      } else place(0);
      if (o.record) {
        rec = new Float32Array(count * 7);
        recGlow = new Float32Array(count);
      }
      return { scene, camera, options: { name: `kaleido-${count}`, width: o.width ?? 1920, height: o.height ?? 1080, preset: "preview" as const } };
    },
    frame(f) {
      if (!show || o.still || !f.state.go || f.dt <= 0) return;
      if (o.record) {
        // Before this frame's step: the poses the physics left after the last one, the glows and blast of the last
        // frame: what frame `show.frame` (the last one) showed.
        for (let i = 0; i < meshes.length; i++) {
          const m = meshes[i], k = i * 7;
          rec[k] = m.position.x; rec[k + 1] = m.position.y; rec[k + 2] = m.position.z;
          rec[k + 3] = m.quaternion.x; rec[k + 4] = m.quaternion.y; rec[k + 5] = m.quaternion.z; rec[k + 6] = m.quaternion.w;
          recGlow[i] = live[i] && show.orbs[i].kind === "glow" ? show.glow[i] : 0;
        }
        console.log(`kpose ${show.frame} ${b64(rec)} ${b64(recGlow)}`);
      }
      if (o.logic !== false) show.step(1 / 60, positions, act);
      else { show.t += 1 / 60; show.frame++; }
      place(show.t);
      const s = show.stats;
      if (show.frame % REPORT_EVERY === 0) console.log(`kaleido ${JSON.stringify({ frame: show.frame, released: show.released, blasts: show.blasts, glowSends: s.glowSends })}`);
    },
  });
}

export default makeGame(600);
