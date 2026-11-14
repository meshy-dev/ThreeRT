// Sound: what the game plays, from its package's files, at a place in the world or in the player's head. The engine
// decodes, places and mixes it (renderer/src/audio): the same code in every app that runs the game, so a shot sounds
// the same in ThreeRT for macOS, on a Steam Deck and on an Android handheld. `audio.play(...)` posts
// `{ t: "audio.play", ... }` on the host channel (host.ts); docs/RUNTIME.md "Sound" is the contract.
//
//   setup() {
//     audio.preload(["assets/sounds/shot.ogg", "assets/sounds/step-1.ogg"]);   // decoded ahead, off the frame
//     ...
//   },
//   frame(f) {
//     if (fired) audio.play("assets/sounds/shot.ogg", { at: muzzle, pitch: 0.97 + 0.06 * rng() });
//     if (f.input.pressed("Escape")) audio.play("assets/sounds/ui-tick.ogg");            // no `at`: in the head
//     audio.play("assets/sounds/fire.ogg", { at: brazier, loop: true, key: "brazier" }); // once; then
//     audio.move("brazier", brazier); audio.stop("brazier", 0.5);
//   }
//
// - A sound is a file of the game (WAV, Ogg Vorbis, MP3 or FLAC), named as the scene names its files
//   ("assets/sounds/shot.ogg"); a package carries everything under assets/. It is decoded once and kept while the game
//   is open: when first played, or ahead after `preload` (do that in setup for what plays in the first seconds).
// - The ears are the camera the frame is drawn from: nothing to send. `at` is in metres, world space; the sound keeps
//   its level out to `falloff` metres (2 by default) and falls as falloff / distance beyond, panned by its direction.
// - `gain` (0 to 4, 1 by default) and `pitch` (0.25 to 4: a playback rate, 1.1 is a little higher and shorter) vary a
//   sound that repeats: a step, a shot.
// - Sound is presentation: play it from the frame script and from host listeners, never from a handler (handlers are
//   replayed and verified; a sound is not state). An app that plays no sound ignores the messages, and the game goes
//   on the same.
// - At most 32 plays a frame, 48 voices at once, 8 of one sound (the oldest goes); the rest is dropped.

import { host } from "./host.ts";

/** A point: `[x, y, z]` or anything with x, y, z (a Vector3). */
export type AudioPoint = readonly [number, number, number] | { x: number; y: number; z: number };

export interface PlayOptions {
  /** Where in the world, metres. Without it the sound is in the player's head (the middle, its own level). */
  at?: AudioPoint;
  /** 0 to 4; 1 by default. */
  gain?: number;
  /** Playback rate, 0.25 to 4; 1 by default. */
  pitch?: number;
  /** Plays until stopped (`key` names it for `stop` and `move`). */
  loop?: boolean;
  /** The name `stop` and `move` reach this sound by (several sounds may share one). */
  key?: string;
  /** Metres out to which the sound keeps its level (2 by default): larger for a loud one, a gunshot, an engine. */
  falloff?: number;
}

/** The messages, as posted. */
export type AudioPost =
  | { t: "audio.play"; sound: string; at?: [number, number, number]; gain?: number; pitch?: number; loop?: boolean; key?: string; falloff?: number }
  | { t: "audio.stop"; key?: string; fade?: number }
  | { t: "audio.move"; key: string; at: [number, number, number] }
  | { t: "audio.preload"; sounds: string[] };

export interface Audio {
  /** Plays `sound` (a file of the game). */
  play(sound: string, options?: PlayOptions): void;
  /** Fades out the sounds of `key` (every sound without one) over `fade` seconds (0.05 by default). */
  stop(key?: string, fade?: number): void;
  /** Moves the sounds of `key` (a loop that follows something). */
  move(key: string, at: AudioPoint): void;
  /** Decodes these files ahead, off the frame. */
  preload(sounds: readonly string[]): void;
}

const round = (v: number) => Math.round(v * 1000) / 1000;

function point(p: AudioPoint): [number, number, number] {
  const [x, y, z] = Array.isArray(p) ? (p as readonly number[]) : [(p as { x: number }).x, (p as { y: number }).y, (p as { z: number }).z];
  if (![x, y, z].every((v) => typeof v === "number" && Number.isFinite(v))) throw new Error(`audio: not a point: ${JSON.stringify(p)}`);
  return [round(x!), round(y!), round(z!)];
}

function name(sound: unknown): string {
  if (typeof sound !== "string" || sound === "") throw new Error(`audio: a sound is a file's path, not ${JSON.stringify(sound)}`);
  return sound;
}

export const audio: Audio = {
  play(sound, o = {}) {
    const m: Extract<AudioPost, { t: "audio.play" }> = { t: "audio.play", sound: name(sound) };
    if (o.at !== undefined) m.at = point(o.at);
    if (o.gain !== undefined && o.gain !== 1) m.gain = round(o.gain);
    if (o.pitch !== undefined && o.pitch !== 1) m.pitch = round(o.pitch);
    if (o.loop) m.loop = true;
    if (o.key !== undefined) m.key = String(o.key);
    if (o.falloff !== undefined) m.falloff = round(o.falloff);
    host.post(m);
  },
  stop(key, fade) {
    const m: Extract<AudioPost, { t: "audio.stop" }> = { t: "audio.stop" };
    if (key !== undefined) m.key = String(key);
    if (fade !== undefined) m.fade = round(fade);
    host.post(m);
  },
  move(key, at) {
    host.post({ t: "audio.move", key: String(key), at: point(at) });
  },
  preload(sounds) {
    host.post({ t: "audio.preload", sounds: sounds.map(name) });
  },
};
