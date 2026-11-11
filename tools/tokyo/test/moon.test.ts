// The moon over Tokyo (games/tokyo/moon.ts) at November 2026's phases (USNO times, UTC): new on the 9th at 07:02, first
// quarter on the 17th at 11:47, full on the 24th at 14:53.
import { test } from "node:test";
import assert from "node:assert/strict";
import { airTransmittance, moonAt } from "../../../games/tokyo/moon.ts";

test("the moon's phase at November 2026's new moon, first quarter and full moon", () => {
  const at = (iso: string) => moonAt(Date.parse(iso));
  assert.ok(at("2026-11-09T07:02:00Z").lit < 0.005, "new");
  assert.ok(Math.abs(at("2026-11-17T11:47:00Z").lit - 0.5) < 0.02, "first quarter");
  const full = at("2026-11-24T14:53:00Z");
  assert.ok(full.lit > 0.995, "full");
  // a full moon near perigee: about 0.25 lux above the atmosphere against the sun's 100 klx
  assert.ok(full.toSun > 2e-6 && full.toSun < 3e-6, `full moon ${full.toSun}`);
  assert.ok(at("2026-11-17T11:47:00Z").toSun < full.toSun / 8, "a half moon is far dimmer than half the full (opposition surge)");
});

test("the moon's place: the full moon high in the south near midnight, the new one down at night", () => {
  const full = moonAt(Date.parse("2026-11-24T23:53:00+09:00"));
  assert.ok(full.elevation > 65 && full.elevation < 85, `elevation ${full.elevation}`);
  // azimuth from the south towards the east: within 45 degrees of the south
  assert.ok(Math.min(full.azimuth, 360 - full.azimuth) < 45, `azimuth ${full.azimuth}`);
  assert.ok(moonAt(Date.parse("2026-11-09T21:00:00+09:00")).elevation < -30, "set with the sun");
  // the first quarter sets around midnight: up in the evening, in the south-west (azimuth west of south: past 270)
  const quarter = moonAt(Date.parse("2026-11-17T20:00:00+09:00"));
  assert.ok(quarter.elevation > 15 && quarter.azimuth > 270, `${quarter.elevation} ${quarter.azimuth}`);
});

test("the air between: clear-ish overhead, reddened and dim at the horizon, nothing below it", () => {
  const top = airTransmittance(90, 10), low = airTransmittance(5, 10);
  assert.ok(top[0] > 0.85 && top[2] > 0.65 && top[2] < top[0]);
  assert.ok(low[0] < 0.5 && low[2] < low[0] / 3);
  assert.deepEqual(airTransmittance(-1), [0, 0, 0]);
});

// CI routing check (temporary PR, closed unmerged): a failing test must fail `ts`, and `threert` with it.
import { test as failsOnPurpose } from "node:test";
failsOnPurpose("CI routing check: fails on purpose", () => { throw new Error("on purpose"); });
