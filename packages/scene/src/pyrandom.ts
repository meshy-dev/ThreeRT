// Python-compatible `random.Random` (CPython's MT19937 + the exact float / integer / sequence algorithms), so a scene
// generator written in Python (e.g. an Unreal editor script) can be ported line by line and produce the same layout.
// Covers random, uniform, getrandbits(<=32), randrange / randint, choice, choices (weighted), shuffle and sample.

const N = 624;

export class PyRandom {
  private mt = new Uint32Array(N);
  private mti = N + 1;

  /** `random.Random(seed)` for a non-negative integer seed. */
  constructor(seed: number) {
    const key: number[] = [];
    let s = Math.abs(Math.trunc(seed));
    do {
      key.push(s % 0x1_0000_0000);
      s = Math.floor(s / 0x1_0000_0000);
    } while (s > 0);
    this.initByArray(key);
  }

  private initGenrand(s: number) {
    const mt = this.mt;
    mt[0] = s >>> 0;
    for (let i = 1; i < N; i++) {
      const prev = mt[i - 1] ^ (mt[i - 1] >>> 30);
      mt[i] = (Math.imul(1812433253, prev) + i) >>> 0;
    }
    this.mti = N;
  }

  private initByArray(key: number[]) {
    this.initGenrand(19650218);
    const mt = this.mt;
    let i = 1, j = 0;
    for (let k = Math.max(N, key.length); k > 0; k--) {
      const prev = mt[i - 1] ^ (mt[i - 1] >>> 30);
      mt[i] = ((mt[i] ^ Math.imul(prev, 1664525)) + key[j] + j) >>> 0;
      i++;
      j++;
      if (i >= N) {
        mt[0] = mt[N - 1];
        i = 1;
      }
      if (j >= key.length) j = 0;
    }
    for (let k = N - 1; k > 0; k--) {
      const prev = mt[i - 1] ^ (mt[i - 1] >>> 30);
      mt[i] = ((mt[i] ^ Math.imul(prev, 1566083941)) - i) >>> 0;
      i++;
      if (i >= N) {
        mt[0] = mt[N - 1];
        i = 1;
      }
    }
    mt[0] = 0x80000000;
  }

  private genrand(): number {
    const mt = this.mt;
    if (this.mti >= N) {
      for (let kk = 0; kk < N; kk++) {
        const y = (mt[kk] & 0x80000000) | (mt[(kk + 1) % N] & 0x7fffffff);
        mt[kk] = mt[(kk + 397) % N] ^ (y >>> 1) ^ (y & 1 ? 0x9908b0df : 0);
      }
      this.mti = 0;
    }
    let y = mt[this.mti++];
    y ^= y >>> 11;
    y ^= (y << 7) & 0x9d2c5680;
    y ^= (y << 15) & 0xefc60000;
    y ^= y >>> 18;
    return y >>> 0;
  }

  /** [0, 1) with 53 random bits (genrand_res53). */
  random(): number {
    const a = this.genrand() >>> 5, b = this.genrand() >>> 6;
    return (a * 67108864 + b) / 9007199254740992;
  }

  uniform(a: number, b: number): number {
    return a + (b - a) * this.random();
  }

  getrandbits(k: number): number {
    if (k <= 0 || k > 32) throw new RangeError("getrandbits: 1..32 bits supported");
    return this.genrand() >>> (32 - k);
  }

  private randbelow(n: number): number {
    if (n <= 0) throw new RangeError("empty range");
    const k = Math.floor(Math.log2(n)) + 1; // n.bit_length()
    let r = this.getrandbits(k);
    while (r >= n) r = this.getrandbits(k);
    return r;
  }

  randrange(start: number, stop?: number): number {
    if (stop === undefined) return this.randbelow(start);
    return start + this.randbelow(stop - start);
  }

  randint(a: number, b: number): number {
    return this.randrange(a, b + 1);
  }

  choice<T>(seq: readonly T[]): T {
    return seq[this.randbelow(seq.length)];
  }

  /** `choices(population, weights)[0]` (k = 1). */
  choices<T>(population: readonly T[], weights: readonly number[]): T {
    const cum: number[] = [];
    let acc = 0;
    for (const w of weights) cum.push((acc += w));
    const x = this.random() * acc;
    // bisect_right(cum, x, 0, n - 1)
    let lo = 0, hi = population.length - 1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (x < cum[mid]) hi = mid;
      else lo = mid + 1;
    }
    return population[lo];
  }

  shuffle<T>(x: T[]): void {
    for (let i = x.length - 1; i > 0; i--) {
      const j = this.randbelow(i + 1);
      [x[i], x[j]] = [x[j], x[i]];
    }
  }

  /** `sample(population, k)` (pool algorithm; CPython uses it whenever n <= the set-size threshold). */
  sample<T>(population: readonly T[], k: number): T[] {
    const n = population.length;
    let setsize = 21;
    if (k > 5) setsize += 4 ** Math.ceil(Math.log(k * 3) / Math.log(4));
    if (n > setsize) throw new Error("PyRandom.sample: set-based selection not implemented");
    const pool = [...population];
    const out: T[] = [];
    for (let i = 0; i < k; i++) {
      const j = this.randbelow(n - i);
      out.push(pool[j]);
      pool[j] = pool[n - i - 1];
    }
    return out;
  }
}
