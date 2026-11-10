// Beeple's Zero-Day, Measure One, animated (ROADMAP § 0 step 8): scenes/zero-day.ts with the FBX take played on the
// game clock (13.7 s, looping): 551 objects move (the instruments, the camera's rig is not used: the scene's camera
// holds its first pose).
import { defineGame, type FrameContext } from "@threert/gameplay";
import type { GLTFModel } from "@threert/scene";
import build from "../../scenes/zero-day.ts";

/** The glTF clip (scripts/orca.ts names it after the file) and its length (s). */
const CLIP = { name: "MEASURE_ONE", length: 13.733 };

export default defineGame<Record<string, never>>({
  state: {},
  keys: {},
  setup() {
    const r = build();
    const room = r.scene.getObjectByName("zero_day") as GLTFModel;
    room.id = "zero_day";
    room.movable = true;
    room.animation = { clip: CLIP.name, time: 0 };
    return { ...r, options: { ...r.options, name: "zero-day-animated" } };
  },
  frame(f: FrameContext<Record<string, never>>) {
    (f.object("zero_day") as GLTFModel).animation = { clip: CLIP.name, time: f.time % CLIP.length };
  },
  handlers: {},
});
