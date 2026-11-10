// Determinism helpers: handlers must produce identical write sets when re-executed (replay verification),
// so randomness and wall-clock time are derived from the room seed and the event instead of the host.

/** 32-bit FNV-1a; derives a default room seed from the room id. */
export function hashString(s: string): number {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193);
  return h >>> 0;
}

function mix32(x: number): number {
  x = Math.imul(x ^ (x >>> 16), 0x85ebca6b);
  x = Math.imul(x ^ (x >>> 13), 0xc2b2ae35);
  return (x ^ (x >>> 16)) >>> 0;
}

/** mulberry32 stream keyed by (seed, stream), e.g. (room seed, event id). Uniform in [0, 1). */
export function seededRandom(seed: number, stream = 0): () => number {
  let a = mix32((seed >>> 0) ^ mix32((stream >>> 0) + 0x9e3779b9));
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/**
 * Runs `fn` with `Math.random` and `Date` (now / no-arg constructor) replaced by deterministic versions.
 * Only sound because handlers are synchronous: nothing else can observe the swapped globals.
 */
export function withDeterminism<T>(now: number, random: () => number, fn: () => T): T {
  const realRandom = Math.random;
  const RealDate = globalThis.Date;
  class FixedDate extends RealDate {
    constructor(...args: unknown[]) {
      super(...((args.length ? args : [now]) as [number]));
    }
    static override now(): number {
      return now;
    }
  }
  Math.random = random;
  globalThis.Date = FixedDate as DateConstructor;
  try {
    return fn();
  } finally {
    Math.random = realRandom;
    globalThis.Date = RealDate;
  }
}
