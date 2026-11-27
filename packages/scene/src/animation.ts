// three.js r187's keyframe animation data: the interpolants (src/math/Interpolant.js, src/math/interpolants/),
// KeyframeTrack and its typed tracks (src/animation/KeyframeTrack.js, tracks/) and AnimationClip
// (src/animation/AnimationClip.js), ported as they are so that clips three.js code builds, or parses from JSON,
// sample to the same numbers.

export const LoopOnce = 2200;
export const LoopRepeat = 2201;
export const LoopPingPong = 2202;
export const InterpolateDiscrete = 2300;
export const InterpolateLinear = 2301;
export const InterpolateSmooth = 2302;
export const InterpolateBezier = 2303;
export const ZeroCurvatureEnding = 2400;
export const ZeroSlopeEnding = 2401;
export const WrapAroundEnding = 2402;
export const NormalAnimationBlendMode = 2500;
export const AdditiveAnimationBlendMode = 2501;

export type InterpolationMode = typeof InterpolateDiscrete | typeof InterpolateLinear | typeof InterpolateSmooth | typeof InterpolateBezier;
export type AnimationBlendMode = typeof NormalAnimationBlendMode | typeof AdditiveAnimationBlendMode;

/** A track's times or values: a typed array, or a plain array for boolean and string tracks. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type KeyframeArray = any[] | Float32Array | Float64Array;
type ArrayType = { new (n: number | ArrayLike<unknown>): KeyframeArray; BYTES_PER_ELEMENT?: number };

function convertArray(array: ArrayLike<unknown> | undefined, type: ArrayType): KeyframeArray {
  if (!array || array.constructor === type) return array as KeyframeArray;
  if (typeof type.BYTES_PER_ELEMENT === "number") return new type(array as ArrayLike<number>);
  return Array.prototype.slice.call(array);
}

function isTypedArray(a: unknown): boolean {
  return ArrayBuffer.isView(a) && !(a instanceof DataView);
}

/** Bezier tangents, as glTF and three.js's JSON carry them: (time, value) pairs per key and component. */
export interface KeyframeTangents {
  inTangents: KeyframeArray;
  outTangents: KeyframeArray;
}

function hasTangents(s: Partial<KeyframeTangents> | null | undefined): s is KeyframeTangents {
  return s != null && s.inTangents !== undefined && s.outTangents !== undefined;
}

let clipIds = 0;
function generateUUID(): string {
  const c = (globalThis as { crypto?: { randomUUID?: () => string } }).crypto;
  return c && typeof c.randomUUID === "function" ? c.randomUUID().toUpperCase() : `CLIP-${++clipIds}`;
}

// --- interpolants ---

/** three.js Interpolant: samples `sampleValues` (`sampleSize` per key) at a parameter, caching the last interval. */
export class Interpolant {
  parameterPositions: KeyframeArray;
  resultBuffer: KeyframeArray;
  sampleValues: KeyframeArray;
  valueSize: number;
  settings: Record<string, unknown> | null = null;
  DefaultSettings_: Record<string, unknown> = {};
  protected _cachedIndex = 0;

  constructor(parameterPositions: KeyframeArray, sampleValues: KeyframeArray, sampleSize: number, resultBuffer?: KeyframeArray) {
    this.parameterPositions = parameterPositions;
    this.resultBuffer = resultBuffer !== undefined ? resultBuffer : new (sampleValues.constructor as ArrayType)(sampleSize);
    this.sampleValues = sampleValues;
    this.valueSize = sampleSize;
  }

  /** The value at `t`: the first or last key's outside the keys, else the interpolation of its interval. */
  evaluate(t: number): KeyframeArray {
    const pp = this.parameterPositions;
    let i1 = this._cachedIndex, t1 = pp[i1], t0 = pp[i1 - 1];
    validate_interval: {
      seek: {
        let right: number;
        linear_scan: {
          forward_scan: if (!(t < t1)) {
            for (let giveUpAt = i1 + 2; ; ) {
              if (t1 === undefined) {
                if (t < t0) break forward_scan;
                i1 = pp.length;
                this._cachedIndex = i1;
                return this.copySampleValue_(i1 - 1);
              }
              if (i1 === giveUpAt) break;
              t0 = t1;
              t1 = pp[++i1];
              if (t < t1) break seek;
            }
            right = pp.length;
            break linear_scan;
          }
          if (!(t >= t0)) {
            const t1global = pp[1];
            if (t < t1global) {
              i1 = 2;
              t0 = t1global;
            }
            for (let giveUpAt = i1 - 2; ; ) {
              if (t0 === undefined) {
                this._cachedIndex = 0;
                return this.copySampleValue_(0);
              }
              if (i1 === giveUpAt) break;
              t1 = t0;
              t0 = pp[--i1 - 1];
              if (t >= t0) break seek;
            }
            right = i1;
            i1 = 0;
            break linear_scan;
          }
          break validate_interval;
        }
        while (i1 < right) {
          const mid = (i1 + right) >>> 1;
          if (t < pp[mid]) right = mid;
          else i1 = mid + 1;
        }
        t1 = pp[i1];
        t0 = pp[i1 - 1];
        if (t0 === undefined) {
          this._cachedIndex = 0;
          return this.copySampleValue_(0);
        }
        if (t1 === undefined) {
          i1 = pp.length;
          this._cachedIndex = i1;
          return this.copySampleValue_(i1 - 1);
        }
      }
      this._cachedIndex = i1;
      this.intervalChanged_(i1, t0, t1);
    }
    return this.interpolate_(i1, t0, t, t1);
  }

  getSettings_(): Record<string, unknown> {
    return this.settings || this.DefaultSettings_;
  }

  copySampleValue_(index: number): KeyframeArray {
    const result = this.resultBuffer, values = this.sampleValues, stride = this.valueSize, offset = index * stride;
    for (let i = 0; i !== stride; ++i) result[i] = values[offset + i];
    return result;
  }

  interpolate_(_i1: number, _t0: number, _t: number, _t1: number): KeyframeArray {
    throw new Error("THREE.Interpolant: Call to abstract method.");
  }

  intervalChanged_(_i1: number, _t0: number, _t1: number): void {}
}

export class DiscreteInterpolant extends Interpolant {
  override interpolate_(i1: number): KeyframeArray {
    return this.copySampleValue_(i1 - 1);
  }
}

export class LinearInterpolant extends Interpolant {
  override interpolate_(i1: number, t0: number, t: number, t1: number): KeyframeArray {
    const result = this.resultBuffer, values = this.sampleValues, stride = this.valueSize;
    const offset1 = i1 * stride, offset0 = offset1 - stride, weight1 = (t - t0) / (t1 - t0), weight0 = 1 - weight1;
    for (let i = 0; i !== stride; ++i) result[i] = values[offset0 + i] * weight0 + values[offset1 + i] * weight1;
    return result;
  }
}

/** Catmull-Rom-like cubic spline with three.js's end conditions (`settings.endingStart` / `endingEnd`). */
export class CubicInterpolant extends Interpolant {
  private _weightPrev = -0;
  private _offsetPrev = -0;
  private _weightNext = -0;
  private _offsetNext = -0;

  constructor(parameterPositions: KeyframeArray, sampleValues: KeyframeArray, sampleSize: number, resultBuffer?: KeyframeArray) {
    super(parameterPositions, sampleValues, sampleSize, resultBuffer);
    this.DefaultSettings_ = { endingStart: ZeroCurvatureEnding, endingEnd: ZeroCurvatureEnding };
  }

  override intervalChanged_(i1: number, t0: number, t1: number): void {
    const pp = this.parameterPositions;
    let iPrev = i1 - 2, iNext = i1 + 1, tPrev = pp[iPrev], tNext = pp[iNext];
    if (tPrev === undefined) {
      switch (this.getSettings_().endingStart) {
        case ZeroSlopeEnding:
          iPrev = i1;
          tPrev = 2 * t0 - t1;
          break;
        case WrapAroundEnding:
          iPrev = pp.length - 2;
          tPrev = t0 + pp[iPrev] - pp[iPrev + 1];
          break;
        default:
          iPrev = i1;
          tPrev = t1;
      }
    }
    if (tNext === undefined) {
      switch (this.getSettings_().endingEnd) {
        case ZeroSlopeEnding:
          iNext = i1;
          tNext = 2 * t1 - t0;
          break;
        case WrapAroundEnding:
          iNext = 1;
          tNext = t1 + pp[1] - pp[0];
          break;
        default:
          iNext = i1 - 1;
          tNext = t0;
      }
    }
    const halfDt = (t1 - t0) * 0.5, stride = this.valueSize;
    this._weightPrev = halfDt / (t0 - tPrev);
    this._weightNext = halfDt / (tNext - t1);
    this._offsetPrev = iPrev * stride;
    this._offsetNext = iNext * stride;
  }

  override interpolate_(i1: number, t0: number, t: number, t1: number): KeyframeArray {
    const result = this.resultBuffer, values = this.sampleValues, stride = this.valueSize;
    const o1 = i1 * stride, o0 = o1 - stride, oP = this._offsetPrev, oN = this._offsetNext;
    const wP = this._weightPrev, wN = this._weightNext;
    const p = (t - t0) / (t1 - t0), pp = p * p, ppp = pp * p;
    const sP = -wP * ppp + 2 * wP * pp - wP * p;
    const s0 = (1 + wP) * ppp + (-1.5 - 2 * wP) * pp + (-0.5 + wP) * p + 1;
    const s1 = (-1 - wN) * ppp + (1.5 + wN) * pp + 0.5 * p;
    const sN = wN * ppp - wN * pp;
    for (let i = 0; i !== stride; ++i) result[i] = sP * values[oP + i] + s0 * values[o0 + i] + s1 * values[o1 + i] + sN * values[oN + i];
    return result;
  }
}

function cubicBezier(s: number, p0: number, p1: number, p2: number, p3: number): number {
  const k = 1 - s;
  return k * k * k * p0 + 3 * k * k * s * p1 + 3 * k * s * s * p2 + s * s * s * p3;
}
function cubicBezierSlope(s: number, p0: number, p1: number, p2: number, p3: number): number {
  const k = 1 - s;
  return 3 * k * k * (p1 - p0) + 6 * k * s * (p2 - p1) + 3 * s * s * (p3 - p2);
}
function solveBezierParameter(x: number, x0: number, x1: number, x2: number, x3: number): number {
  let s = (x - x0) / (x3 - x0);
  for (let i = 0; i < 8; i++) {
    const error = cubicBezier(s, x0, x1, x2, x3) - x;
    if (Math.abs(error) < 1e-10) break;
    const slope = cubicBezierSlope(s, x0, x1, x2, x3);
    if (Math.abs(slope) < 1e-10) break;
    s = Math.max(0, Math.min(1, s - error / slope));
  }
  return s;
}

/** Cubic Bezier between keys, with (time, value) control points from `inTangents` / `outTangents`; linear without. */
export class BezierInterpolant extends Interpolant {
  inTangents?: KeyframeArray;
  outTangents?: KeyframeArray;

  override interpolate_(i1: number, t0: number, t: number, t1: number): KeyframeArray {
    const result = this.resultBuffer, values = this.sampleValues, stride = this.valueSize;
    const offset1 = i1 * stride, offset0 = offset1 - stride;
    const inTangents = this.inTangents, outTangents = this.outTangents;
    if (!inTangents || !outTangents) {
      const weight1 = (t - t0) / (t1 - t0), weight0 = 1 - weight1;
      for (let i = 0; i !== stride; ++i) result[i] = values[offset0 + i] * weight0 + values[offset1 + i] * weight1;
      return result;
    }
    const tangentStride = stride * 2, i0 = i1 - 1;
    for (let i = 0; i !== stride; ++i) {
      const v0 = values[offset0 + i], v1 = values[offset1 + i];
      const outOffset = i0 * tangentStride + i * 2, inOffset = i1 * tangentStride + i * 2;
      const s = solveBezierParameter(t, t0, outTangents[outOffset], inTangents[inOffset], t1);
      result[i] = cubicBezier(s, v0, outTangents[outOffset + 1], inTangents[inOffset + 1], v1);
    }
    return result;
  }
}

/** three.js Quaternion.slerpFlat, on flat arrays. */
function slerpFlat(dst: KeyframeArray, dstOffset: number, src0: KeyframeArray, srcOffset0: number, src1: KeyframeArray, srcOffset1: number, t: number): void {
  let x0 = src0[srcOffset0], y0 = src0[srcOffset0 + 1], z0 = src0[srcOffset0 + 2], w0 = src0[srcOffset0 + 3];
  let x1 = src1[srcOffset1], y1 = src1[srcOffset1 + 1], z1 = src1[srcOffset1 + 2], w1 = src1[srcOffset1 + 3];
  if (w0 !== w1 || x0 !== x1 || y0 !== y1 || z0 !== z1) {
    let dot = x0 * x1 + y0 * y1 + z0 * z1 + w0 * w1;
    if (dot < 0) {
      x1 = -x1;
      y1 = -y1;
      z1 = -z1;
      w1 = -w1;
      dot = -dot;
    }
    let s = 1 - t;
    if (dot < 0.9995) {
      const theta = Math.acos(dot), sin = Math.sin(theta);
      s = Math.sin(s * theta) / sin;
      t = Math.sin(t * theta) / sin;
      x0 = x0 * s + x1 * t;
      y0 = y0 * s + y1 * t;
      z0 = z0 * s + z1 * t;
      w0 = w0 * s + w1 * t;
    } else {
      x0 = x0 * s + x1 * t;
      y0 = y0 * s + y1 * t;
      z0 = z0 * s + z1 * t;
      w0 = w0 * s + w1 * t;
      const f = 1 / Math.sqrt(x0 * x0 + y0 * y0 + z0 * z0 + w0 * w0);
      x0 *= f;
      y0 *= f;
      z0 *= f;
      w0 *= f;
    }
  }
  dst[dstOffset] = x0;
  dst[dstOffset + 1] = y0;
  dst[dstOffset + 2] = z0;
  dst[dstOffset + 3] = w0;
}

/** Spherical interpolation of each quaternion (4 values) between keys. */
export class QuaternionLinearInterpolant extends Interpolant {
  override interpolate_(i1: number, t0: number, t: number, t1: number): KeyframeArray {
    const result = this.resultBuffer, values = this.sampleValues, stride = this.valueSize, alpha = (t - t0) / (t1 - t0);
    let offset = i1 * stride;
    for (let end = offset + stride; offset !== end; offset += 4) slerpFlat(result, 0, values, offset - stride, values, offset, alpha);
    return result;
  }
}

// --- tracks ---

type InterpolantFactory = ((this: KeyframeTrack, result?: KeyframeArray) => Interpolant) | undefined;

/** three.js KeyframeTrack: a property path (`"door.position"`, `".quaternion"`), its key times in seconds and its
 * values, flat (`getValueSize()` per key). */
export class KeyframeTrack {
  name: string;
  times: KeyframeArray;
  values: KeyframeArray;
  declare settings?: KeyframeTangents;
  /** Makes the interpolant `setInterpolation` chose. */
  createInterpolant!: (result?: KeyframeArray) => Interpolant;
  declare ValueTypeName: string;
  declare TimeBufferType: ArrayType;
  declare ValueBufferType: ArrayType;
  declare DefaultInterpolation: InterpolationMode;
  declare InterpolantFactoryMethodDiscrete: InterpolantFactory;
  declare InterpolantFactoryMethodLinear: InterpolantFactory;
  declare InterpolantFactoryMethodSmooth: InterpolantFactory;
  declare InterpolantFactoryMethodBezier: InterpolantFactory;

  constructor(name: string, times: ArrayLike<number>, values: ArrayLike<unknown>, interpolation?: InterpolationMode) {
    if (name === undefined) throw new Error("THREE.KeyframeTrack: track name is undefined");
    if (times === undefined || times.length === 0) throw new Error("THREE.KeyframeTrack: no keyframes in track named " + name);
    this.name = name;
    this.times = convertArray(times, this.TimeBufferType);
    this.values = convertArray(values, this.ValueBufferType);
    this.setInterpolation(interpolation || this.DefaultInterpolation);
  }

  /** The track as three.js's JSON: name, times, values, type, and the interpolation when not the default. */
  static toJSON(track: KeyframeTrack): KeyframeTrackJSON {
    const trackType = track.constructor as typeof KeyframeTrack;
    let json: KeyframeTrackJSON;
    if (trackType.toJSON !== this.toJSON) {
      json = trackType.toJSON(track);
    } else {
      json = { name: track.name, times: convertArray(track.times, Array) as number[], values: convertArray(track.values, Array) as unknown[] } as KeyframeTrackJSON;
      const interpolation = track.getInterpolation();
      if (interpolation !== track.DefaultInterpolation) json.interpolation = interpolation;
      if (hasTangents(track.settings)) {
        json.settings = { inTangents: convertArray(track.settings.inTangents, Array) as number[], outTangents: convertArray(track.settings.outTangents, Array) as number[] };
      }
    }
    json.type = track.ValueTypeName;
    return json;
  }

  /** Switches how values between keys are made; a mode the track type has no interpolant for warns and keeps the
   * current one (or the default, on construction). */
  setInterpolation(interpolation: InterpolationMode): this {
    let factoryMethod: InterpolantFactory;
    switch (interpolation) {
      case InterpolateDiscrete:
        factoryMethod = this.InterpolantFactoryMethodDiscrete;
        break;
      case InterpolateLinear:
        factoryMethod = this.InterpolantFactoryMethodLinear;
        break;
      case InterpolateSmooth:
        factoryMethod = this.InterpolantFactoryMethodSmooth;
        break;
      case InterpolateBezier:
        factoryMethod = this.InterpolantFactoryMethodBezier;
        break;
    }
    if (factoryMethod === undefined) {
      const message = "unsupported interpolation for " + this.ValueTypeName + " keyframe track named " + this.name;
      if (this.createInterpolant === undefined) {
        if (interpolation !== this.DefaultInterpolation) this.setInterpolation(this.DefaultInterpolation);
        else throw new Error(message);
      }
      console.warn("THREE.KeyframeTrack:", message);
      return this;
    }
    this.createInterpolant = factoryMethod as (result?: KeyframeArray) => Interpolant;
    return this;
  }

  getInterpolation(): InterpolationMode | undefined {
    switch (this.createInterpolant) {
      case this.InterpolantFactoryMethodDiscrete:
        return InterpolateDiscrete;
      case this.InterpolantFactoryMethodLinear:
        return InterpolateLinear;
      case this.InterpolantFactoryMethodSmooth:
        return InterpolateSmooth;
      case this.InterpolantFactoryMethodBezier:
        return InterpolateBezier;
    }
    return undefined;
  }

  getValueSize(): number {
    return this.values.length / this.times.length;
  }

  /** Moves every key (and tangent) by `timeOffset` seconds. */
  shift(timeOffset: number): this {
    if (timeOffset !== 0.0) {
      const times = this.times;
      for (let i = 0, n = times.length; i !== n; ++i) times[i] += timeOffset;
      if (hasTangents(this.settings)) {
        shiftTangentTimes(this.settings.inTangents, timeOffset);
        shiftTangentTimes(this.settings.outTangents, timeOffset);
      }
    }
    return this;
  }

  /** Multiplies every key time (and tangent time) by `timeScale`. */
  scale(timeScale: number): this {
    if (timeScale !== 1.0) {
      const times = this.times;
      for (let i = 0, n = times.length; i !== n; ++i) times[i] *= timeScale;
      if (hasTangents(this.settings)) {
        scaleTangentTimes(this.settings.inTangents, timeScale);
        scaleTangentTimes(this.settings.outTangents, timeScale);
      }
    }
    return this;
  }

  /** Drops the keys outside [startTime, endTime], keeping at least one. */
  trim(startTime: number, endTime: number): this {
    const times = this.times, nKeys = times.length;
    let from = 0, to = nKeys - 1;
    while (from !== nKeys && times[from] < startTime) ++from;
    while (to !== -1 && times[to] > endTime) --to;
    ++to;
    if (from !== 0 || to !== nKeys) {
      if (from >= to) {
        to = Math.max(to, 1);
        from = to - 1;
      }
      const stride = this.getValueSize();
      this.times = times.slice(from, to);
      this.values = this.values.slice(from * stride, to * stride);
    }
    return this;
  }

  /** Whether the value size is whole, the track has keys, its times are in order and its typed values are numbers. */
  validate(): boolean {
    let valid = true;
    const valueSize = this.getValueSize();
    if (valueSize - Math.floor(valueSize) !== 0) {
      console.error("THREE.KeyframeTrack: Invalid value size in track.", this);
      valid = false;
    }
    const times = this.times, values = this.values, nKeys = times.length;
    if (nKeys === 0) {
      console.error("THREE.KeyframeTrack: Track is empty.", this);
      valid = false;
    }
    let prevTime: number | null = null;
    for (let i = 0; i !== nKeys; i++) {
      const currTime = times[i];
      if (typeof currTime === "number" && isNaN(currTime)) {
        console.error("THREE.KeyframeTrack: Time is not a valid number.", this, i, currTime);
        valid = false;
        break;
      }
      if (prevTime !== null && prevTime > currTime) {
        console.error("THREE.KeyframeTrack: Out of order keys.", this, i, currTime, prevTime);
        valid = false;
        break;
      }
      prevTime = currTime;
    }
    if (values !== undefined && isTypedArray(values)) {
      for (let i = 0, n = values.length; i !== n; ++i) {
        const value = values[i];
        if (isNaN(value)) {
          console.error("THREE.KeyframeTrack: Value is not a valid number.", this, i, value);
          valid = false;
          break;
        }
      }
    }
    return valid;
  }

  /** Removes keys that repeat their neighbours (consecutive equal times, or a value equal on both sides). */
  optimize(): this {
    const times = this.times.slice(), values = this.values.slice(), stride = this.getValueSize();
    const smoothInterpolation = this.getInterpolation() === InterpolateSmooth, lastIndex = times.length - 1;
    let writeIndex = 1;
    for (let i = 1; i < lastIndex; ++i) {
      let keep = false;
      const time = times[i], timeNext = times[i + 1];
      if (time !== timeNext && (i !== 1 || time !== times[0])) {
        if (!smoothInterpolation) {
          const offset = i * stride, offsetP = offset - stride, offsetN = offset + stride;
          for (let j = 0; j !== stride; ++j) {
            const value = values[offset + j];
            if (value !== values[offsetP + j] || value !== values[offsetN + j]) {
              keep = true;
              break;
            }
          }
        } else {
          keep = true;
        }
      }
      if (keep) {
        if (i !== writeIndex) {
          times[writeIndex] = times[i];
          const readOffset = i * stride, writeOffset = writeIndex * stride;
          for (let j = 0; j !== stride; ++j) values[writeOffset + j] = values[readOffset + j];
        }
        ++writeIndex;
      }
    }
    if (lastIndex > 0) {
      times[writeIndex] = times[lastIndex];
      for (let readOffset = lastIndex * stride, writeOffset = writeIndex * stride, j = 0; j !== stride; ++j) values[writeOffset + j] = values[readOffset + j];
      ++writeIndex;
    }
    if (writeIndex !== times.length) {
      this.times = times.slice(0, writeIndex);
      this.values = values.slice(0, writeIndex * stride);
    } else {
      this.times = times;
      this.values = values;
    }
    return this;
  }

  clone(): this {
    const TypedKeyframeTrack = this.constructor as new (name: string, times: KeyframeArray, values: KeyframeArray) => this;
    const track = new TypedKeyframeTrack(this.name, this.times.slice(), this.values.slice());
    track.createInterpolant = this.createInterpolant;
    if (hasTangents(this.settings)) track.settings = { inTangents: this.settings.inTangents.slice(), outTangents: this.settings.outTangents.slice() };
    return track;
  }
}

function shiftTangentTimes(tangents: KeyframeArray, timeOffset: number): void {
  for (let i = 0, n = tangents.length; i !== n; i += 2) tangents[i] += timeOffset;
}
function scaleTangentTimes(tangents: KeyframeArray, timeScale: number): void {
  for (let i = 0, n = tangents.length; i !== n; i += 2) tangents[i] *= timeScale;
}

Object.assign(KeyframeTrack.prototype, {
  ValueTypeName: "",
  TimeBufferType: Float32Array,
  ValueBufferType: Float32Array,
  DefaultInterpolation: InterpolateLinear,
  InterpolantFactoryMethodDiscrete(this: KeyframeTrack, result?: KeyframeArray) {
    return new DiscreteInterpolant(this.times, this.values, this.getValueSize(), result);
  },
  InterpolantFactoryMethodLinear(this: KeyframeTrack, result?: KeyframeArray) {
    return new LinearInterpolant(this.times, this.values, this.getValueSize(), result);
  },
  InterpolantFactoryMethodSmooth(this: KeyframeTrack, result?: KeyframeArray) {
    return new CubicInterpolant(this.times, this.values, this.getValueSize(), result);
  },
  InterpolantFactoryMethodBezier(this: KeyframeTrack, result?: KeyframeArray) {
    const interpolant = new BezierInterpolant(this.times, this.values, this.getValueSize(), result);
    if (this.settings) {
      interpolant.inTangents = this.settings.inTangents;
      interpolant.outTangents = this.settings.outTangents;
    }
    return interpolant;
  },
});

export interface KeyframeTrackJSON {
  name: string;
  type: string;
  times?: number[];
  values?: unknown[];
  /** Keys as objects (`{ time, value }`), when `times` is left out. */
  keys?: { time: number; [k: string]: unknown }[];
  interpolation?: InterpolationMode;
  settings?: { inTangents: number[]; outTangents: number[] };
}

/** Discrete booleans (`visible`). */
export class BooleanKeyframeTrack extends KeyframeTrack {
  constructor(name: string, times: ArrayLike<number>, values: ArrayLike<boolean>) {
    super(name, times, values);
  }
}
Object.assign(BooleanKeyframeTrack.prototype, { ValueTypeName: "bool", ValueBufferType: Array, DefaultInterpolation: InterpolateDiscrete, InterpolantFactoryMethodLinear: undefined, InterpolantFactoryMethodSmooth: undefined });

/** RGB colors (3 values a key). */
export class ColorKeyframeTrack extends KeyframeTrack {}
ColorKeyframeTrack.prototype.ValueTypeName = "color";

/** Scalars. */
export class NumberKeyframeTrack extends KeyframeTrack {}
NumberKeyframeTrack.prototype.ValueTypeName = "number";

/** Quaternions (4 values a key), slerped. */
export class QuaternionKeyframeTrack extends KeyframeTrack {}
Object.assign(QuaternionKeyframeTrack.prototype, {
  ValueTypeName: "quaternion",
  InterpolantFactoryMethodLinear(this: KeyframeTrack, result?: KeyframeArray) {
    return new QuaternionLinearInterpolant(this.times, this.values, this.getValueSize(), result);
  },
  InterpolantFactoryMethodSmooth: undefined,
});

/** Discrete strings. */
export class StringKeyframeTrack extends KeyframeTrack {
  constructor(name: string, times: ArrayLike<number>, values: ArrayLike<string>) {
    super(name, times, values);
  }
}
Object.assign(StringKeyframeTrack.prototype, { ValueTypeName: "string", ValueBufferType: Array, DefaultInterpolation: InterpolateDiscrete, InterpolantFactoryMethodLinear: undefined, InterpolantFactoryMethodSmooth: undefined });

/** Vectors (`position`, `scale`). */
export class VectorKeyframeTrack extends KeyframeTrack {}
VectorKeyframeTrack.prototype.ValueTypeName = "vector";

// --- clips ---

export interface AnimationClipJSON {
  name: string;
  duration?: number;
  tracks: KeyframeTrackJSON[];
  uuid?: string;
  blendMode?: AnimationBlendMode;
  userData?: string;
  /** Key times in frames at this rate (parse only). */
  fps?: number;
}

/** three.js AnimationClip: named tracks that play together. A negative duration is the last key's time. */
export class AnimationClip {
  name: string;
  tracks: KeyframeTrack[];
  duration: number;
  blendMode: AnimationBlendMode;
  uuid = generateUUID();
  userData: Record<string, unknown> = {};

  constructor(name = "", duration = -1, tracks: KeyframeTrack[] = [], blendMode: AnimationBlendMode = NormalAnimationBlendMode) {
    this.name = name;
    this.tracks = tracks;
    this.duration = duration;
    this.blendMode = blendMode;
    if (this.duration < 0) this.resetDuration();
  }

  static parse(json: AnimationClipJSON): AnimationClip {
    const tracks: KeyframeTrack[] = [], frameTime = 1.0 / (json.fps || 1.0);
    for (const t of json.tracks) tracks.push(parseKeyframeTrack(t).scale(frameTime));
    const clip = new this(json.name, json.duration, tracks, json.blendMode);
    clip.uuid = json.uuid as string;
    clip.userData = JSON.parse(json.userData || "{}");
    return clip;
  }

  static toJSON(clip: AnimationClip): AnimationClipJSON {
    return {
      name: clip.name,
      duration: clip.duration,
      tracks: clip.tracks.map((t) => KeyframeTrack.toJSON(t)),
      uuid: clip.uuid,
      blendMode: clip.blendMode,
      userData: JSON.stringify(clip.userData),
    };
  }

  /** The clip called `name` in an array, or in an object's `animations` (or its geometry's); null if none. */
  static findByName(objectOrClipArray: AnimationClip[] | { animations?: AnimationClip[]; geometry?: { animations?: AnimationClip[] } }, name: string): AnimationClip | null {
    let clipArray = objectOrClipArray as AnimationClip[];
    if (!Array.isArray(objectOrClipArray)) {
      const o = objectOrClipArray;
      clipArray = ((o.geometry && o.geometry.animations) || o.animations) as AnimationClip[];
    }
    for (let i = 0; i < clipArray.length; i++) if (clipArray[i].name === name) return clipArray[i];
    return null;
  }

  /** A clip that blends each morph target in turn (`.morphTargetInfluences[name]` tracks), `fps` targets a second;
   * unless `noLoop`, it returns to the first. */
  static CreateFromMorphTargetSequence(name: string, morphTargetSequence: { name: string }[], fps: number, noLoop?: boolean): AnimationClip {
    const n = morphTargetSequence.length, tracks: KeyframeTrack[] = [];
    for (let i = 0; i < n; i++) {
      let times: number[] = [(i + n - 1) % n, i, (i + 1) % n], values = [0, 1, 0];
      const order = times.map((_, k) => k).sort((a, b) => times[a] - times[b]);
      times = order.map((k) => times[k]);
      values = order.map((k) => values[k]);
      if (!noLoop && times[0] === 0) {
        times.push(n);
        values.push(values[0]);
      }
      tracks.push(new NumberKeyframeTrack(".morphTargetInfluences[" + morphTargetSequence[i].name + "]", times, values).scale(1.0 / fps));
    }
    return new this(name, -1, tracks);
  }

  /** One clip per name prefix of numbered morph targets (`walk_001`, `walk_002`, `run_001` → walk_, run_). */
  static CreateClipsFromMorphTargetSequences(morphTargets: { name: string }[], fps: number, noLoop?: boolean): AnimationClip[] {
    const byName: Record<string, { name: string }[]> = {}, pattern = /^([\w-]*?)([\d]+)$/;
    for (const m of morphTargets) {
      const parts = m.name.match(pattern);
      if (parts && parts.length > 1) (byName[parts[1]] ??= []).push(m);
    }
    const clips: AnimationClip[] = [];
    for (const name in byName) clips.push(this.CreateFromMorphTargetSequence(name, byName[name], fps, noLoop));
    return clips;
  }

  resetDuration(): this {
    let duration = 0;
    for (const track of this.tracks) duration = Math.max(duration, track.times[track.times.length - 1]);
    this.duration = duration;
    return this;
  }

  trim(): this {
    for (const track of this.tracks) track.trim(0, this.duration);
    return this;
  }

  validate(): boolean {
    let valid = true;
    for (const track of this.tracks) valid = valid && track.validate();
    return valid;
  }

  optimize(): this {
    for (const track of this.tracks) track.optimize();
    return this;
  }

  clone(): this {
    const C = this.constructor as new (name: string, duration: number, tracks: KeyframeTrack[], blendMode: AnimationBlendMode) => this;
    const clip = new C(this.name, this.duration, this.tracks.map((t) => t.clone()), this.blendMode);
    clip.userData = JSON.parse(JSON.stringify(this.userData));
    return clip;
  }

  toJSON(): AnimationClipJSON {
    return (this.constructor as typeof AnimationClip).toJSON(this);
  }
}

function getTrackTypeForValueTypeName(typeName: string): typeof KeyframeTrack {
  switch (typeName.toLowerCase()) {
    case "scalar":
    case "double":
    case "float":
    case "number":
    case "integer":
      return NumberKeyframeTrack;
    case "vector":
    case "vector2":
    case "vector3":
    case "vector4":
      return VectorKeyframeTrack;
    case "color":
      return ColorKeyframeTrack;
    case "quaternion":
      return QuaternionKeyframeTrack;
    case "bool":
    case "boolean":
      return BooleanKeyframeTrack as typeof KeyframeTrack;
    case "string":
      return StringKeyframeTrack as typeof KeyframeTrack;
  }
  throw new Error("THREE.KeyframeTrack: Unsupported typeName: " + typeName);
}

function flattenJSON(jsonKeys: { time: number; [k: string]: unknown }[], times: number[], values: unknown[], valuePropertyName: string): void {
  let i = 1, key = jsonKeys[0];
  while (key !== undefined && key[valuePropertyName] === undefined) key = jsonKeys[i++];
  if (key === undefined) return;
  let value = key[valuePropertyName] as unknown;
  if (value === undefined) return;
  const toArray = (value as { toArray?: unknown }).toArray !== undefined;
  do {
    value = key[valuePropertyName];
    if (value !== undefined) {
      times.push(key.time);
      if (Array.isArray(value)) values.push(...value);
      else if (toArray) (value as { toArray(a: unknown[], o: number): void }).toArray(values, values.length);
      else values.push(value);
    }
    key = jsonKeys[i++];
  } while (key !== undefined);
}

function parseKeyframeTrack(json: KeyframeTrackJSON): KeyframeTrack {
  if (json.type === undefined) throw new Error("THREE.KeyframeTrack: track type undefined, can not parse");
  const trackType = getTrackTypeForValueTypeName(json.type) as typeof KeyframeTrack & { parse?: (j: KeyframeTrackJSON) => KeyframeTrack };
  if (json.times === undefined) {
    const times: number[] = [], values: unknown[] = [];
    flattenJSON(json.keys ?? [], times, values, "value");
    json.times = times;
    json.values = values;
  }
  const track = trackType.parse !== undefined ? trackType.parse(json) : new trackType(json.name, json.times!, json.values!, json.interpolation);
  if (hasTangents(json.settings)) {
    track.settings = { inTangents: convertArray(json.settings.inTangents, Float32Array), outTangents: convertArray(json.settings.outTangents, Float32Array) };
  }
  return track;
}
