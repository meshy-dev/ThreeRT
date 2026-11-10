// Time of day on a small scene (docs/ROADMAP.md, the dynamic sky): the sun room with its sun set by the frame script, a
// sweep from the afternoon through sunset into the night in 8 s, the exposure following it and the lamp switching on as
// the sun goes down. The sky is `dynamic`: the live view moves the sun's light and re-bakes the sky in place (no scene
// rebuild). tools/bench-sun.ts measures it.
//
// Room id:
//   sweep           the frame script moves the sun (scene time 0 .. 8 s), the exposure and the lamp;
//   fixed-<t>       the sun, the exposure and the lamp where the sweep has them at t s, set at load and left there: the
//                   reference an in-place move must match.
import { defineGame, type FrameContext } from "@threert/gameplay";
import { type DirectionalLight, type Mesh, SkyAtmosphere } from "@threert/scene";
import build from "../../scenes/sun-room.ts";

/** The sweep's length, s. */
export const SWEEP = 8;

/** Sun angles (degrees) at scene time t: from a 40 deg afternoon sun through the window to 8 deg below the horizon. */
export function sunAt(t: number): { elevation: number; azimuth: number } {
  const u = Math.min(1, Math.max(0, t / SWEEP));
  return { elevation: 40 - 48 * u, azimuth: 95 + 25 * u };
}

/** Manual exposure (EV100) for a sun elevation: the interior's 10.2 by day, down to 6 once the sun is well below. */
export function evAt(elevation: number): number {
  const u = Math.min(1, Math.max(0, (12 - elevation) / 18));
  return 10.2 - 4.2 * u * u * (3 - 2 * u);
}

/** The lamp's power for a sun elevation: off by day, on below 2 deg (a light switched on at dusk). */
export function lampAt(elevation: number): number {
  return elevation < 2 ? 1 : 0;
}

let sweeping = true;

function place(sun: DirectionalLight, lamp: Mesh, camera: { setManualExposure(ev: number): unknown }, t: number) {
  const s = sunAt(t);
  sun.setSunAngles(s.elevation, s.azimuth);
  camera.setManualExposure(evAt(s.elevation));
  lamp.power = lampAt(s.elevation);
}

export default defineGame<Record<string, never>>({
  state: {},
  keys: {},
  handlers: {},
  setup(ctx) {
    const r = build();
    const sun = r.scene.getObjectByName("sun") as DirectionalLight;
    const lamp = r.scene.getObjectByName("lamp_shade") as Mesh;
    const room = ctx.roomId || "sweep";
    const fixed = /^fixed-(-?[\d.]+)$/.exec(room);
    sweeping = !fixed;
    place(sun, lamp, r.camera, fixed ? Number(fixed[1]) : 0);
    r.scene.environment = new SkyAtmosphere({ sun, dynamic: true, glowIntensity: 0.02 });
    return { ...r, options: { ...r.options, name: "sun-sweep" } };
  },
  frame(f: FrameContext<Record<string, never>>) {
    if (!sweeping) return;
    place(f.object("sun") as DirectionalLight, f.object("lamp_shade") as Mesh, f.camera, f.time);
  },
});
