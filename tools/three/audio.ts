/// <reference lib="dom" />
// The `audio.*` messages in a page (docs/RUNTIME.md "Sound"), on Web Audio: what the engine's library does natively
// (renderer/src/audio), kept to its numbers so a game sounds the same in a browser. Each voice is a buffer source,
// a low pass, and a gain for each ear; once a frame the listener (the camera) places every voice by the same rules as
// mix.rs `place` and `pan_gains`. The output goes through the same limiter.
//
// A browser plays nothing before the person has touched the page: the context resumes on the first key or click.
// Files are fetched relative to the page (`assets/sounds/shot.ogg`), so a package hosted under a path finds them.
import type { HostMessage } from "@threert/gameplay";

export const DEFAULT_FALLOFF = 2;
const FADE_FROM = 100, SILENT_AT = 120, MAX_PAN = 0.8, MAX_VOICES = 48, MAX_COPIES = 8, MAX_PLAYS_A_FRAME = 32;

type V3 = [number, number, number];
const sub = (a: V3, b: V3): V3 => [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
const dot = (a: V3, b: V3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2];

/** Gain, pan and the low pass's cutoff of a sound at `at` for the listener: mix.rs `place`, number for number. */
export function place(pos: V3, forward: V3, right: V3, at: V3, falloff: number): [number, number, number] {
  falloff = Math.max(falloff, 0.05);
  const to = sub(at, pos);
  const d = Math.hypot(...to);
  let gain = d <= falloff ? 1 : falloff / d;
  if (d > FADE_FROM) gain *= Math.min(1, Math.max(0, (SILENT_AT - d) / (SILENT_AT - FADE_FROM)));
  if (d < 1e-4) return [gain, 0, 18000];
  const dir: V3 = [to[0] / d, to[1] / d, to[2] / d];
  const pan = Math.min(1, Math.max(-1, dot(dir, right) * Math.min(1, d / falloff))) * MAX_PAN;
  const behind = Math.max(0, -dot(dir, forward));
  return [gain, pan, (18000 / (1 + d / 40)) * (1 - 0.5 * behind)];
}

/** Equal-power gains of a pan in −1..1, the middle at 1 on both sides: mix.rs `pan_gains`. */
export function panGains(pan: number): [number, number] {
  const th = (Math.min(1, Math.max(-1, pan)) + 1) * (Math.PI / 4);
  return [Math.cos(th) * Math.SQRT2, Math.sin(th) * Math.SQRT2];
}

interface Voice {
  sound: string;
  src: AudioBufferSourceNode;
  lp: BiquadFilterNode;
  left: GainNode;
  right: GainNode;
  fade: GainNode;
  at?: V3;
  gain: number;
  falloff: number;
  key?: string;
  looped: boolean;
  level: number;
  age: number;
  ended: boolean;
}

export class PageAudio {
  private ctx: AudioContext | null = null;
  private master: GainNode | null = null;
  private readonly buffers = new Map<string, Promise<AudioBuffer | null>>();
  private readonly ready = new Map<string, AudioBuffer | null>();
  private voices: Voice[] = [];
  private listener: { pos: V3; forward: V3; right: V3 } = { pos: [0, 0, 0], forward: [0, 0, -1], right: [1, 0, 0] };
  private plays = 0;
  private ages = 0;
  private readonly off: (() => void)[] = [];
  private readonly base: string;

  constructor(base = "") {
    this.base = base;
    const resume = () => void this.context().resume();
    for (const t of ["keydown", "pointerdown"] as const) {
      window.addEventListener(t, resume, { capture: true });
      this.off.push(() => window.removeEventListener(t, resume, { capture: true }));
    }
  }

  private context(): AudioContext {
    if (!this.ctx) {
      this.ctx = new AudioContext({ latencyHint: "interactive" });
      // The limiter of mix.rs: transparent under 0.8, a soft knee to 1.
      const shaper = this.ctx.createWaveShaper();
      const n = 4097, curve = new Float32Array(n);
      for (let i = 0; i < n; i++) {
        const x = (i / (n - 1)) * 4 - 2, a = Math.abs(x);
        curve[i] = a <= 0.8 ? x : Math.sign(x) * (0.8 + 0.2 * Math.tanh((a - 0.8) / 0.2));
      }
      shaper.curve = curve;
      this.master = this.ctx.createGain();
      this.master.connect(shaper).connect(this.ctx.destination);
    }
    return this.ctx;
  }

  /** The app's volume, 0..1. */
  set volume(v: number) {
    if (this.master) this.master.gain.value = Math.min(1, Math.max(0, v));
  }

  /** A message the game posted: false when it is not `audio.*`. */
  post(m: HostMessage): boolean {
    if (typeof m.t !== "string" || !m.t.startsWith("audio.")) return false;
    if (m.t === "audio.play") {
      this.play(m);
    } else if (m.t === "audio.stop") {
      this.stop(typeof m.key === "string" ? m.key : undefined, typeof m.fade === "number" ? m.fade : 0.05);
    } else if (m.t === "audio.move" && typeof m.key === "string" && Array.isArray(m.at)) {
      for (const v of this.voices) if (v.key === m.key) v.at = m.at as V3;
    } else if (m.t === "audio.preload" && Array.isArray(m.sounds)) {
      for (const s of m.sounds) if (typeof s === "string") void this.load(s);
    }
    return true;
  }

  private load(sound: string): Promise<AudioBuffer | null> {
    let p = this.buffers.get(sound);
    if (!p) {
      if (!sound || sound.split("/").some((c) => c === ".." || c === "") || sound.startsWith("/")) {
        console.warn(`sound ${sound}: a sound is a path inside the game's package`);
        p = Promise.resolve(null);
      } else {
        p = fetch(this.base + sound)
          .then((r) => (r.ok ? r.arrayBuffer() : Promise.reject(new Error(`${r.status}`))))
          .then((b) => this.context().decodeAudioData(b))
          .catch((e) => (console.warn(`sound ${sound}: ${e instanceof Error ? e.message : e}`), null));
      }
      p.then((b) => this.ready.set(sound, b));
      this.buffers.set(sound, p);
    }
    return p;
  }

  private play(m: HostMessage) {
    if (typeof m.sound !== "string" || this.plays >= MAX_PLAYS_A_FRAME) return;
    this.plays++;
    const num = (v: unknown, d: number, lo: number, hi: number) => (typeof v === "number" && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : d);
    const o = {
      at: Array.isArray(m.at) && m.at.length === 3 ? (m.at as V3) : undefined,
      gain: num(m.gain, 1, 0, 4),
      pitch: num(m.pitch, 1, 0.25, 4),
      looped: m.loop === true,
      key: typeof m.key === "string" ? m.key : undefined,
      falloff: num(m.falloff, DEFAULT_FALLOFF, 0.05, 1000),
    };
    const buffer = this.ready.get(m.sound);
    // (Not decoded yet: it starts when it is, if that is soon; a one-shot that would be late is dropped.)
    if (buffer === undefined) {
      const asked = performance.now();
      void this.load(m.sound).then((b) => b && (o.looped || performance.now() - asked < 150) && this.start(m.sound as string, b, o));
      return;
    }
    if (buffer) this.start(m.sound, buffer, o);
  }

  private start(sound: string, buffer: AudioBuffer, o: { at?: V3; gain: number; pitch: number; looped: boolean; key?: string; falloff: number }) {
    const ctx = this.context();
    const copies = this.voices.filter((v) => v.sound === sound && !v.ended);
    if (copies.length >= MAX_COPIES) this.end(copies.reduce((a, b) => (a.age < b.age ? a : b)), 0);
    if (this.voices.length >= MAX_VOICES) {
      const quiet = this.voices.filter((v) => !v.looped).sort((a, b) => a.level - b.level)[0];
      if (!quiet) return;
      this.end(quiet, 0);
    }
    const src = ctx.createBufferSource();
    src.buffer = buffer;
    src.loop = o.looped;
    src.playbackRate.value = o.pitch;
    const lp = ctx.createBiquadFilter();
    lp.type = "lowpass";
    lp.Q.value = 0.5;
    const left = ctx.createGain(), right = ctx.createGain(), fade = ctx.createGain();
    const merge = ctx.createChannelMerger(2);
    src.connect(lp);
    if (buffer.numberOfChannels > 1) {
      const split = ctx.createChannelSplitter(2);
      lp.connect(split);
      split.connect(left, 0);
      split.connect(right, 1);
    } else {
      lp.connect(left);
      lp.connect(right);
    }
    left.connect(merge, 0, 0);
    right.connect(merge, 0, 1);
    merge.connect(fade).connect(this.master!);
    const v: Voice = { sound, src, lp, left, right, fade, at: o.at, gain: o.gain, falloff: o.falloff, key: o.key, looped: o.looped, level: o.gain, age: ++this.ages, ended: false };
    this.apply(v, true);
    src.onended = () => {
      v.ended = true;
      this.voices = this.voices.filter((x) => x !== v);
      merge.disconnect();
    };
    src.start();
    this.voices.push(v);
  }

  private end(v: Voice, fade: number) {
    if (v.ended) return;
    v.ended = true;
    const t = this.ctx!.currentTime;
    v.fade.gain.setValueAtTime(v.fade.gain.value, t);
    v.fade.gain.linearRampToValueAtTime(0, t + Math.max(fade, 0.005));
    v.src.stop(t + Math.max(fade, 0.005));
  }

  stop(key?: string, fade = 0.05) {
    for (const v of this.voices) if (key === undefined || v.key === key) this.end(v, fade);
  }

  /** Where it is heard from this frame on (`now`: without a ramp). */
  private apply(v: Voice, now = false) {
    const l = this.listener;
    const [g, pan, cutoff] = v.at ? place(l.pos, l.forward, l.right, v.at, v.falloff) : [1, 0, 18000];
    const [pl, pr] = panGains(pan);
    v.level = v.gain * g;
    const t = this.ctx!.currentTime;
    const set = (p: AudioParam, x: number) => (now ? p.setValueAtTime(x, t) : p.setTargetAtTime(x, t, 0.01));
    set(v.left.gain, v.gain * g * pl);
    set(v.right.gain, v.gain * g * pr);
    set(v.lp.frequency, Math.min(cutoff, this.ctx!.sampleRate * 0.45));
  }

  /** Once a frame, after the game's messages: the ears are the camera (position, forward, right). */
  setListener(pos: V3, forward: V3, right: V3) {
    this.plays = 0;
    this.listener = { pos, forward, right };
    if (this.ctx) for (const v of this.voices) if (v.at) this.apply(v);
  }

  close() {
    this.stop(undefined, 0);
    for (const f of this.off.splice(0)) f();
    void this.ctx?.close();
    this.ctx = null;
  }
}
