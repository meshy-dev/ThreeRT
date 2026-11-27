// three.js r187 AnimationMixer / AnimationAction / PropertyBinding / PropertyMixer (src/animation/): each scenario
// runs on ThreeRT objects and on three.js objects, and what the mixer writes to them must agree.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as THREE from "three";
import * as TRT from "../src/index.ts";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Lib = any;
const r = (v: number) => +v.toFixed(5);

function both(name: string, scenario: (T: Lib) => unknown): void {
  test(name, () => {
    const warn = console.warn, error = console.error;
    console.warn = console.error = () => {};
    try {
      const ours = scenario(TRT), theirs = scenario(THREE);
      assert.deepEqual(ours, theirs);
    } finally {
      console.warn = warn;
      console.error = error;
    }
  });
}

/** A root with a box (a mesh) and a lamp under it. */
function rig(T: Lib) {
  const root = new T.Group();
  root.name = "rig";
  const box = new T.Mesh(new T.BoxGeometry(1, 1, 1), new T.MeshStandardMaterial({ color: 0x808080 }));
  box.name = "box";
  box.position.set(1, 2, 3);
  const lamp = new T.Object3D();
  lamp.name = "lamp";
  root.add(box);
  box.add(lamp);
  return { root, box, lamp };
}

function state(o: { box: Lib; lamp: Lib }) {
  const { box, lamp } = o;
  return [
    ...box.position.toArray().map(r),
    ...box.quaternion.toArray().map(r),
    ...box.scale.toArray().map(r),
    box.visible,
    r(box.material.opacity),
    ...[box.material.color.r, box.material.color.g, box.material.color.b].map(r),
    ...lamp.position.toArray().map(r),
  ];
}

function clips(T: Lib) {
  const s = Math.SQRT1_2;
  const move = new T.AnimationClip("move", -1, [
    new T.VectorKeyframeTrack("box.position", [0, 1, 2], [0, 0, 0, 2, 4, 6, 0, 0, 0]),
    new T.QuaternionKeyframeTrack("box.quaternion", [0, 2], [0, 0, 0, 1, 0, s, 0, s]),
    new T.VectorKeyframeTrack("lamp.position", [0, 2], [0, 1, 0, 0, 3, 0], T.InterpolateSmooth),
  ]);
  const look = new T.AnimationClip("look", -1, [
    new T.VectorKeyframeTrack("box.position", [0, 3], [5, 5, 5, -5, -5, -5]),
    new T.VectorKeyframeTrack("box.scale", [0, 3], [1, 1, 1, 2, 3, 4]),
    new T.NumberKeyframeTrack("box.material.opacity", [0, 3], [1, 0]),
    new T.ColorKeyframeTrack("box.material.color", [0, 3], [1, 0, 0, 0, 0, 1]),
    new T.BooleanKeyframeTrack(".children[0].visible", [0], [true]), // an unresolved path: warns, binds nothing
    new T.BooleanKeyframeTrack("box.visible", [0, 1.5], [true, false]),
  ]);
  return { move, look };
}

function run(mixer: Lib, o: { box: Lib; lamp: Lib }, steps: number[]) {
  return steps.map((dt) => (mixer.update(dt), state(o)));
}

both("play: a repeating clip moves named descendants, loops, and stop restores them", (T) => {
  const o = rig(T), { move } = clips(T), mixer = new T.AnimationMixer(o.root), events: unknown[] = [];
  mixer.addEventListener("loop", (e: Lib) => events.push([e.type, e.loopDelta, e.action.getClip().name]));
  const a = mixer.clipAction(move).play();
  const frames = run(mixer, o, [0, 0.25, 0.5, 0.75, 0.6, 1.3, 2.2]);
  const running = [a.isRunning(), a.isScheduled(), r(a.time), mixer.stats.actions.inUse, mixer.stats.bindings.inUse];
  a.stop();
  return [frames, running, events, state(o), a.isScheduled(), mixer.stats.bindings.inUse];
});

both("LoopOnce with clampWhenFinished, LoopPingPong with repetitions, and running backwards", (T) => {
  const o = rig(T), { move } = clips(T), mixer = new T.AnimationMixer(o.root), events: unknown[] = [];
  mixer.addEventListener("finished", (e: Lib) => events.push(["finished", e.direction, r(e.action.time)]));
  mixer.addEventListener("loop", (e: Lib) => events.push(["loop", e.loopDelta]));
  const a = mixer.clipAction(move);
  a.setLoop(T.LoopOnce, 1);
  a.clampWhenFinished = true;
  a.play();
  const once = run(mixer, o, [0.5, 1, 1, 1]);
  const after = [a.paused, a.enabled, a.isRunning()];
  a.reset().setLoop(T.LoopPingPong, 3);
  a.clampWhenFinished = false;
  const pp = run(mixer, o, [0.7, 0.7, 0.7, 0.7, 0.7, 0.7, 0.7, 0.7, 0.7, 0.7]);
  a.reset().setLoop(T.LoopRepeat, Infinity);
  a.timeScale = -1;
  const back = run(mixer, o, [0.3, 0.3, 1.9]);
  return [once, after, pp, [a.enabled], back, events];
});

both("crossFadeTo with warp, fadeIn, weights and effective time scale", (T) => {
  const o = rig(T), { move, look } = clips(T), mixer = new T.AnimationMixer(o.root);
  const a = mixer.clipAction(move).play(), b = mixer.clipAction(look);
  run(mixer, o, [0.4]);
  b.play();
  a.crossFadeTo(b, 1, true);
  const frames = run(mixer, o, [0.1, 0.3, 0.3, 0.3, 0.3, 0.5]).map((s, i) => [...s, i]);
  return [frames, r(a.getEffectiveWeight()), r(b.getEffectiveWeight()), r(a.timeScale), r(b.timeScale), a.enabled, mixer.stats.controlInterpolants.total];
});

both("additive blending over a base action, and partial weight against the original", (T) => {
  const o = rig(T), { move, look } = clips(T), mixer = new T.AnimationMixer(o.root);
  mixer.clipAction(look).setEffectiveWeight(0.5).play();
  const add = new T.AnimationClip("nudge", -1, [
    new T.VectorKeyframeTrack("box.position", [0, 1], [0, 0, 0, 1, 1, 1]),
    new T.QuaternionKeyframeTrack("box.quaternion", [0, 1], [0, 0, 0, 1, 0, 0, Math.sin(0.4), Math.cos(0.4)]),
  ], T.AdditiveAnimationBlendMode);
  const n = mixer.clipAction(add).play();
  n.weight = 0.75;
  return [n.blendMode, run(mixer, o, [0.25, 0.25, 0.25]), mixer.existingAction(move)];
});

both("setTime, startAt, halt, setDuration, timeScale, clipAction by name, uncache", (T) => {
  const o = rig(T), { move, look } = clips(T), mixer = new T.AnimationMixer(o.root);
  o.root.animations = [move, look];
  const a = mixer.clipAction("move");
  const same = mixer.clipAction(move) === a && mixer.existingAction("move") === a && mixer.clipAction("nope") === null;
  a.setDuration(4).startAt(0.5).play();
  mixer.timeScale = 2;
  const scheduled = run(mixer, o, [0.1, 0.3, 0.3]);
  mixer.setTime(1);
  const atOne = [state(o), r(mixer.time), r(a.time)];
  a.halt(1);
  const halting = run(mixer, o, [0.2, 0.2, 0.2, 0.2]);
  const halted = [a.paused, r(a.timeScale)];
  mixer.timeScale = 1;
  const b = mixer.clipAction(look).play();
  run(mixer, o, [0.5]);
  mixer.uncacheClip(look);
  const afterClip = [mixer.stats.actions.total, mixer.stats.bindings.total, b.isScheduled(), state(o)];
  mixer.stopAllAction();
  mixer.uncacheRoot(o.root);
  return [same, scheduled, atOne, halting, halted, afterClip, mixer.stats.actions.total, mixer.stats.bindings.total, state(o), mixer.getRoot() === o.root];
});

both("PropertyBinding.parseTrackName and sanitizeNodeName", (T) =>
  ["box.position", ".quaternion", "box.material.opacity", "a/b/node.position[x]", "lamp.materials[2].color", "uuid-1.morphTargetInfluences[smile]", ".bones[hip].position"].map((n) => {
    const p = T.PropertyBinding.parseTrackName(n);
    return [p.nodeName, p.objectName, p.objectIndex, p.propertyName, p.propertyIndex];
  }).concat([T.PropertyBinding.sanitizeNodeName("my node.a[1]:b/c")]),
);

test("the mixer writes ThreeRT positions; a bad track name throws", () => {
  const o = rig(TRT), { move } = clips(TRT), mixer = new TRT.AnimationMixer(o.root);
  mixer.clipAction(move)!.play();
  mixer.update(0.25);
  assert.deepEqual(o.box.position.toArray(), [0.5, 1, 1.5]);
  assert.throws(() => TRT.PropertyBinding.parseTrackName("box"), /Cannot parse/);
});
