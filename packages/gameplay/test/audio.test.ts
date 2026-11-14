// Sound (src/audio.ts): what the game asks for reaches the app's queue as the messages docs/RUNTIME.md "Sound"
// describes, points as arrays, defaults left out, and a sound that is not a path refused.
import { test } from "node:test";
import assert from "node:assert/strict";
import { Vector3 } from "@threert/scene";
import { audio } from "../src/index.ts";

function app(): { posts: { t: string; [k: string]: unknown }[]; close(): void } {
  const posts: { t: string }[] = [];
  const g = globalThis as { __host_post?: (text: string) => unknown };
  g.__host_post = (text) => void posts.push(JSON.parse(text));
  return { posts, close: () => void delete g.__host_post };
}

test("play, stop, move and preload post their messages", () => {
  const a = app();
  try {
    audio.preload(["assets/sounds/shot.ogg"]);
    audio.play("assets/sounds/shot.ogg", { at: new Vector3(1, 2.00049, -3), gain: 0.8, pitch: 1, falloff: 6 });
    audio.play("assets/sounds/tick.wav");
    audio.play("assets/sounds/fire.ogg", { at: [0, 1, 0], loop: true, key: "fire" });
    audio.move("fire", { x: 1, y: 1, z: 0 });
    audio.stop("fire", 0.5);
    audio.stop();
    assert.deepEqual(a.posts, [
      { t: "audio.preload", sounds: ["assets/sounds/shot.ogg"] },
      { t: "audio.play", sound: "assets/sounds/shot.ogg", at: [1, 2, -3], gain: 0.8, falloff: 6 },
      { t: "audio.play", sound: "assets/sounds/tick.wav" },
      { t: "audio.play", sound: "assets/sounds/fire.ogg", at: [0, 1, 0], loop: true, key: "fire" },
      { t: "audio.move", key: "fire", at: [1, 1, 0] },
      { t: "audio.stop", key: "fire", fade: 0.5 },
      { t: "audio.stop" },
    ]);
  } finally {
    a.close();
  }
});

test("a sound is a path and a point is three finite numbers", () => {
  assert.throws(() => audio.play(""), /a sound is a file's path/);
  assert.throws(() => audio.play("a.ogg", { at: [0, Number.NaN, 0] }), /not a point/);
  // Without an app nothing listens, and nothing throws.
  audio.play("a.ogg");
});
