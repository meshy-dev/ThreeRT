// three.js r187's clocks (src/core/Clock.js, src/core/Timer.js): seconds since start and between calls, from the
// runtime's monotonic clock (`performance.now`, or `Date.now` where the host has none). A frame script usually reads
// its frame's own time instead; these are for three.js code that brings its own clock.

const now: () => number = (() => {
  const p = (globalThis as { performance?: { now?: () => number } }).performance;
  if (p && typeof p.now === "function") return () => p.now!();
  return () => Date.now();
})();

/** three.js Clock (deprecated in three.js r183 for {@link Timer}, still exported there). */
export class Clock {
  autoStart: boolean;
  startTime = 0;
  oldTime = 0;
  elapsedTime = 0;
  running = false;
  constructor(autoStart = true) {
    this.autoStart = autoStart;
  }
  start(): void {
    this.startTime = now();
    this.oldTime = this.startTime;
    this.elapsedTime = 0;
    this.running = true;
  }
  stop(): void {
    this.getElapsedTime();
    this.running = false;
    this.autoStart = false;
  }
  getElapsedTime(): number {
    this.getDelta();
    return this.elapsedTime;
  }
  /** Seconds since the last call (0 on the call that auto-starts the clock). */
  getDelta(): number {
    if (this.autoStart && !this.running) {
      this.start();
      return 0;
    }
    let diff = 0;
    if (this.running) {
      const t = now();
      diff = (t - this.oldTime) / 1000;
      this.oldTime = t;
      this.elapsedTime += diff;
    }
    return diff;
  }
}

interface VisibilityDocument {
  hidden?: boolean;
  addEventListener(type: string, fn: () => void, capture?: boolean): void;
  removeEventListener(type: string, fn: () => void): void;
}

/** three.js Timer: `update(timestamp?)` once per frame, then `getDelta()` / `getElapsed()` in seconds, scaled by the
 * timescale. `connect(document)` zeroes the delta while the page is hidden. */
export class Timer {
  private _previousTime = 0;
  private _currentTime = 0;
  private _startTime = now();
  private _delta = 0;
  private _elapsed = 0;
  private _timescale = 1;
  private _document: VisibilityDocument | null = null;
  private _pageVisibilityHandler: (() => void) | null = null;
  connect(document: VisibilityDocument): void {
    this._document = document;
    if (document.hidden !== undefined) {
      this._pageVisibilityHandler = () => {
        if (this._document?.hidden === false) this.reset();
      };
      document.addEventListener("visibilitychange", this._pageVisibilityHandler, false);
    }
  }
  disconnect(): void {
    if (this._pageVisibilityHandler !== null) {
      this._document!.removeEventListener("visibilitychange", this._pageVisibilityHandler);
      this._pageVisibilityHandler = null;
    }
    this._document = null;
  }
  getDelta(): number {
    return this._delta / 1000;
  }
  getElapsed(): number {
    return this._elapsed / 1000;
  }
  getTimescale(): number {
    return this._timescale;
  }
  setTimescale(timescale: number): this {
    this._timescale = timescale;
    return this;
  }
  reset(): this {
    this._currentTime = now() - this._startTime;
    return this;
  }
  dispose(): void {
    this.disconnect();
  }
  /** `timestamp` in milliseconds (a requestAnimationFrame time); the runtime's clock when left out. */
  update(timestamp?: number): this {
    if (this._pageVisibilityHandler !== null && this._document!.hidden === true) {
      this._delta = 0;
    } else {
      this._previousTime = this._currentTime;
      this._currentTime = (timestamp !== undefined ? timestamp : now()) - this._startTime;
      this._delta = (this._currentTime - this._previousTime) * this._timescale;
      this._elapsed += this._delta;
    }
    return this;
  }
}
