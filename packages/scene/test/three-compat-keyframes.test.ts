// three.js r187 keyframe tracks, interpolants and AnimationClip (src/animation/KeyframeTrack.js, tracks/,
// AnimationClip.js, src/math/interpolants/): each scenario runs on ThreeRT's port and on three itself, and must agree.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Lib = any;
const SAMPLES = [-1, 0, 0.25, 0.5, 1, 1.3, 2, 2.7, 3, 5, 0.1, 2.9, 0.4]; // out of order on purpose: the interval cache
const round = (a: ArrayLike<unknown>) => Array.from(a, (v) => (typeof v === "number" ? +v.toFixed(6) : v));

function both(name: string, scenario: (T: Lib) => unknown): void {
  test(name, () => {
    const warn = console.warn, error = console.error;
    console.warn = console.error = () => {};
    try {
      assert.deepEqual(scenario(TRT), scenario(THREE));
    } finally {
      console.warn = warn;
      console.error = error;
    }
  });
}

function sample(track: { createInterpolant(): { evaluate(t: number): ArrayLike<unknown> } }) {
  const i = track.createInterpolant();
  return SAMPLES.map((t) => round(i.evaluate(t)));
}

const TIMES = [0, 1, 2, 3];
const VALUES = [0, 0, 0, 1, 2, 3, 4, -1, 2, 0, 0, 5];

both("VectorKeyframeTrack: linear, discrete and smooth sampling", (T) => [
  sample(new T.VectorKeyframeTrack("a.position", TIMES, VALUES)),
  sample(new T.VectorKeyframeTrack("a.position", TIMES, VALUES, T.InterpolateDiscrete)),
  sample(new T.VectorKeyframeTrack("a.position", TIMES, VALUES, T.InterpolateSmooth)),
]);

both("CubicInterpolant: zero-slope and wrap-around endings", (T) =>
  [T.ZeroSlopeEnding, T.WrapAroundEnding].map((e) => {
    const i = new T.NumberKeyframeTrack(".x", TIMES, [0, 3, 1, 4], T.InterpolateSmooth).createInterpolant();
    i.settings = { endingStart: e, endingEnd: e };
    return SAMPLES.map((t) => +i.evaluate(t)[0].toFixed(6));
  }),
);

both("QuaternionKeyframeTrack slerps; it has no smooth mode and keeps linear", (T) => {
  const s = Math.SQRT1_2;
  const q = new T.QuaternionKeyframeTrack(".quaternion", [0, 1, 2], [0, 0, 0, 1, 0, s, 0, s, 0, 0, -1, 0]);
  const before = sample(q);
  q.setInterpolation(T.InterpolateSmooth);
  return [before, q.getInterpolation(), q.getValueSize()];
});

both("BooleanKeyframeTrack and StringKeyframeTrack are discrete", (T) => {
  const b = new T.BooleanKeyframeTrack(".visible", [0, 1, 2], [true, false, true]);
  const str = new T.StringKeyframeTrack(".name", [0, 1], ["a", "b"]);
  b.setInterpolation(T.InterpolateLinear);
  return [sample(b), sample(str), b.getInterpolation(), Array.isArray(b.values)];
});

both("Bezier interpolation with tangents, and linear without", (T) => {
  const bz = new T.NumberKeyframeTrack(".x", [0, 1, 2], [0, 1, 0], T.InterpolateBezier);
  const plain = sample(bz);
  bz.settings = { inTangents: new Float32Array([0, 0, 0.6, 1.5, 1.7, 0.2]), outTangents: new Float32Array([0.3, 0.8, 1.4, 1, 2, 0]) };
  bz.setInterpolation(T.InterpolateBezier);
  const curved = sample(bz);
  bz.shift(1).scale(2);
  return [plain, curved, round(bz.times), round(bz.settings.inTangents), round(bz.settings.outTangents), T.KeyframeTrack.toJSON(bz)];
});

both("KeyframeTrack: optimize, trim, shift, scale, validate, clone", (T) => {
  const o = new T.NumberKeyframeTrack(".x", [0, 1, 1, 2, 3, 4, 5], [0, 1, 1, 1, 1, 2, 2]).optimize();
  const tr = new T.NumberKeyframeTrack(".x", TIMES, [0, 1, 2, 3]).trim(0.5, 2.5);
  const tr2 = new T.NumberKeyframeTrack(".x", TIMES, [0, 1, 2, 3]).trim(5, 6);
  const sh = new T.NumberKeyframeTrack(".x", [0, 1], [0, 1]).shift(2).scale(0.5);
  const c = new T.VectorKeyframeTrack(".scale", [0, 1], [1, 1, 1, 2, 2, 2], T.InterpolateDiscrete).clone();
  return [
    [round(o.times), round(o.values)],
    [round(tr.times), round(tr.values)],
    [round(tr2.times), round(tr2.values)],
    round(sh.times),
    [new T.NumberKeyframeTrack(".x", [0, 2, 1], [0, 1, 2]).validate(), new T.NumberKeyframeTrack(".x", [0, 1], [0, NaN]).validate(), new T.NumberKeyframeTrack(".x", [0, 1], [0, 1, 2]).validate()],
    [c.constructor.name, c.getInterpolation(), round(c.values), c.times instanceof Float32Array],
  ];
});

both("AnimationClip: duration from the tracks, toJSON, clone, findByName", (T) => {
  const clip = new T.AnimationClip("walk", -1, [
    new T.NumberKeyframeTrack(".x", [0, 1.5], [0, 1]),
    new T.VectorKeyframeTrack(".position", [0, 2.5], [0, 0, 0, 1, 1, 1], T.InterpolateSmooth),
    new T.BooleanKeyframeTrack(".visible", [0, 1], [true, false]),
  ]);
  clip.userData = { tag: 1 };
  const json = T.AnimationClip.toJSON(clip);
  delete json.uuid;
  const c = clip.clone();
  const trimmed = new T.AnimationClip("t", 1, [new T.NumberKeyframeTrack(".x", TIMES, [0, 1, 2, 3])]).trim();
  return [
    clip.duration,
    clip.validate(),
    json,
    [c.name, c.duration, c.blendMode, c.userData, c.tracks[1].getInterpolation(), c.tracks[0] !== clip.tracks[0]],
    T.AnimationClip.findByName([clip], "walk") === clip,
    T.AnimationClip.findByName({ animations: [clip] }, "nope"),
    round(trimmed.tracks[0].times),
    new T.AnimationClip("additive", 2, [], T.AdditiveAnimationBlendMode).blendMode,
  ];
});

both("AnimationClip.parse: fps, keyed JSON, track types by name", (T) => {
  const p = T.AnimationClip.parse({
    name: "p",
    fps: 10,
    uuid: "U",
    tracks: [
      { name: ".x", type: "number", keys: [{ time: 0, value: 1 }, { time: 10, value: 2 }, { time: 20 }, { time: 30, value: 3 }] },
      { name: ".position", type: "vector3", keys: [{ time: 0, value: [1, 2, 3] }, { time: 5, value: [4, 5, 6] }] },
      { name: ".quaternion", type: "quaternion", times: [0, 10], values: [0, 0, 0, 1, 0, 0, 1, 0], interpolation: T.InterpolateDiscrete },
      { name: ".visible", type: "boolean", times: [0], values: [false] },
    ],
  });
  return [p.name, p.duration, p.uuid, p.tracks.map((t: Lib) => [t.constructor.name, round(t.times), round(t.values), t.getInterpolation()])];
});

test("KeyframeTrack: no name or no keys throws, as three.js does", () => {
  assert.throws(() => new TRT.NumberKeyframeTrack(".x", [], []), /no keyframes/);
  assert.throws(() => new TRT.NumberKeyframeTrack(undefined as unknown as string, [0], [0]), /name is undefined/);
});

both("AnimationClip.CreateClipsFromMorphTargetSequences: one looping clip per name prefix", (T) =>
  T.AnimationClip.CreateClipsFromMorphTargetSequences([{ name: "walk_001" }, { name: "walk_002" }, { name: "walk_003" }, { name: "run1" }, { name: "idle" }], 10, false).map((c: Lib) => [
    c.name,
    +c.duration.toFixed(6),
    c.tracks.map((t: Lib) => [t.name, round(t.times), round(t.values)]),
  ]),
);

both("AnimationClip.CreateFromMorphTargetSequence with noLoop", (T) =>
  T.AnimationClip.CreateFromMorphTargetSequence("s", [{ name: "a" }, { name: "b" }], 2, true).tracks.map((t: Lib) => [t.name, round(t.times), round(t.values)]),
);

test("KeyframeTrack: no own settings field until one is set, as three.js", () => {
  assert.equal(Object.hasOwn(new TRT.NumberKeyframeTrack(".x", [0], [0]), "settings"), Object.hasOwn(new THREE.NumberKeyframeTrack(".x", [0], [0]), "settings"));
});
