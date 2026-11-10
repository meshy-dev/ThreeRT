// Bistro night in the rain (wet, RainCurtain streaks around the camera): the perf suite's "bistro rain" lines, and the
// base of its effect combinations (tools/perf/bistro-sparks.ts, bistro-smoke.ts). The street is at y ≈ 0.31 m 8 m
// ahead of the camera (ray-cast on the converted glTF).
import { RainCurtain } from "@threert/scene";
import build from "../../scenes/bistro.ts";

/** A point on the street ahead of Bistro's camera (m), for the effect combinations. */
export const AHEAD = { x: 23.2, ground: 0.31, z: -53 };

export function rainyBistro() {
  const r = build();
  r.scene.wetness = 1;
  r.scene.rain = 1;
  r.scene.add(new RainCurtain());
  return r;
}

export default function () {
  const r = rainyBistro();
  return { ...r, options: { ...r.options, name: "bistro-rain" } };
}
