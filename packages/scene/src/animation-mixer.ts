// three.js r187's AnimationMixer, AnimationAction, PropertyBinding and PropertyMixer (src/animation/), ported as they
// are: a mixer plays clips on an object tree, blending actions by weight, with fades, warps, loops and the
// "loop" / "finished" events. Not ported: AnimationObjectGroup (PropertyBinding.Composite), and the skeleton and
// morph-target paths (`.bones[...]`, `.morphTargetInfluences[...]`), which ThreeRT scene objects do not have.
import {
  AnimationClip,
  LinearInterpolant,
  LoopOnce,
  LoopPingPong,
  LoopRepeat,
  NormalAnimationBlendMode,
  AdditiveAnimationBlendMode,
  WrapAroundEnding,
  ZeroCurvatureEnding,
  ZeroSlopeEnding,
  type AnimationBlendMode,
  type Interpolant,
  type KeyframeArray,
} from "./animation.ts";

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Any = any;
type AnimationRoot = Any;

/** three.js keys a root by `uuid`; ThreeRT objects have none, so roots without one get a key of their own. */
const rootKeys = new WeakMap<object, string>();
let rootKeyCount = 0;
function rootKey(root: AnimationRoot): string {
  if (typeof root.uuid === "string") return root.uuid;
  let k = rootKeys.get(root);
  if (k === undefined) rootKeys.set(root, (k = `root${++rootKeyCount}`));
  return k;
}

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

function multiplyQuaternionsFlat(dst: KeyframeArray, dstOffset: number, src0: KeyframeArray, srcOffset0: number, src1: KeyframeArray, srcOffset1: number): void {
  const x0 = src0[srcOffset0], y0 = src0[srcOffset0 + 1], z0 = src0[srcOffset0 + 2], w0 = src0[srcOffset0 + 3];
  const x1 = src1[srcOffset1], y1 = src1[srcOffset1 + 1], z1 = src1[srcOffset1 + 2], w1 = src1[srcOffset1 + 3];
  dst[dstOffset] = x0 * w1 + w0 * x1 + y0 * z1 - z0 * y1;
  dst[dstOffset + 1] = y0 * w1 + w0 * y1 + z0 * x1 - x0 * z1;
  dst[dstOffset + 2] = z0 * w1 + w0 * z1 + x0 * y1 - y0 * x1;
  dst[dstOffset + 3] = w0 * w1 - x0 * x1 - y0 * y1 - z0 * z1;
}

// --- PropertyBinding ---

const RESERVED = "\\[\\]\\.:\\/";
const reservedRe = new RegExp("[" + RESERVED + "]", "g");
const wordChar = "[^" + RESERVED + "]";
const wordCharOrDot = "[^" + RESERVED.replace("\\.", "") + "]";
const trackRe = new RegExp(
  "^" +
    /((?:WC+[\/:])*)/.source.replace("WC", wordChar) +
    /(WCOD+)?/.source.replace("WCOD", wordCharOrDot) +
    /(?:\.(WC+)(?:\[(.+)\])?)?/.source.replace("WC", wordChar) +
    /\.(WC+)(?:\[(.+)\])?/.source.replace("WC", wordChar) +
    "$",
);
const supportedObjectNames = ["material", "materials", "bones", "map"];

export interface ParsedTrackName {
  nodeName?: string;
  objectName?: string;
  objectIndex?: string | number;
  propertyName: string;
  propertyIndex?: string | number;
}

const BindingType = { Direct: 0, EntireArray: 1, ArrayElement: 2, HasFromToArray: 3 } as const;
const Versioning = { None: 0, NeedsUpdate: 1, MatrixWorldNeedsUpdate: 2 } as const;

/** three.js PropertyBinding: resolves a track name (`"door.position"`, `"lamp.material.opacity"`, `".scale[1]"`)
 * under a root to the property it animates, and reads and writes it from flat buffers. */
export class PropertyBinding {
  path: string;
  parsedPath: ParsedTrackName;
  node: Any;
  rootNode: AnimationRoot;
  targetObject: Any;
  resolvedProperty: Any;
  propertyName?: string;
  propertyIndex?: string | number;
  getValue: (buffer: KeyframeArray, offset: number) => void;
  setValue: (buffer: KeyframeArray, offset: number) => void;

  constructor(rootNode: AnimationRoot, path: string, parsedPath?: ParsedTrackName) {
    this.path = path;
    this.parsedPath = parsedPath || PropertyBinding.parseTrackName(path);
    this.node = PropertyBinding.findNode(rootNode, this.parsedPath.nodeName);
    this.rootNode = rootNode;
    this.getValue = this._getValue_unbound;
    this.setValue = this._setValue_unbound;
  }

  static create(root: AnimationRoot, path: string, parsedPath?: ParsedTrackName): PropertyBinding {
    return new PropertyBinding(root, path, parsedPath);
  }

  /** A node name usable in a track name: whitespace to underscores, the reserved characters `[]./:` removed. */
  static sanitizeNodeName(name: string): string {
    return name.replace(/\s/g, "_").replace(reservedRe, "");
  }

  static parseTrackName(trackName: string): ParsedTrackName {
    const matches = trackRe.exec(trackName);
    if (matches === null) throw new Error("THREE.PropertyBinding: Cannot parse trackName: " + trackName);
    const results: ParsedTrackName = { nodeName: matches[2], objectName: matches[3], objectIndex: matches[4], propertyName: matches[5], propertyIndex: matches[6] };
    const lastDot = results.nodeName ? results.nodeName.lastIndexOf(".") : undefined;
    if (lastDot !== undefined && lastDot !== -1) {
      const objectName = results.nodeName!.substring(lastDot + 1);
      if (supportedObjectNames.indexOf(objectName) !== -1) {
        results.nodeName = results.nodeName!.substring(0, lastDot);
        results.objectName = objectName;
      }
    }
    if (results.propertyName === null || results.propertyName.length === 0) throw new Error("THREE.PropertyBinding: can not parse propertyName from trackName: " + trackName);
    return results;
  }

  /** The root itself for an empty name (or its name or uuid), else the first descendant with that name or uuid. */
  static findNode(root: AnimationRoot, nodeName: string | number | undefined): Any {
    if (nodeName === undefined || nodeName === "" || nodeName === "." || nodeName === -1 || nodeName === root.name || nodeName === root.uuid) return root;
    if (root.skeleton) {
      const bone = root.skeleton.getBoneByName(nodeName);
      if (bone !== undefined) return bone;
    }
    if (root.children) {
      const search = (children: Any[]): Any => {
        for (const child of children) {
          if (child.name === nodeName || child.uuid === nodeName) return child;
          const result = search(child.children);
          if (result) return result;
        }
        return null;
      };
      const found = search(root.children);
      if (found) return found;
    }
    return null;
  }

  _getValue_unavailable(): void {}
  _setValue_unavailable(): void {}
  _getValue_direct(buffer: KeyframeArray, offset: number): void {
    buffer[offset] = this.targetObject[this.propertyName!];
  }
  _getValue_array(buffer: KeyframeArray, offset: number): void {
    const source = this.resolvedProperty;
    for (let i = 0, n = source.length; i !== n; ++i) buffer[offset++] = source[i];
  }
  _getValue_arrayElement(buffer: KeyframeArray, offset: number): void {
    buffer[offset] = this.resolvedProperty[this.propertyIndex!];
  }
  _getValue_toArray(buffer: KeyframeArray, offset: number): void {
    // ThreeRT's Vector3 / Quaternion toArray() returns a new array rather than writing into one.
    const r = this.resolvedProperty.toArray(buffer, offset);
    if (r !== buffer) for (let i = 0; i < r.length; i++) buffer[offset + i] = r[i];
  }
  _setValue_direct(buffer: KeyframeArray, offset: number): void {
    this.targetObject[this.propertyName!] = buffer[offset];
  }
  _setValue_array(buffer: KeyframeArray, offset: number): void {
    const dest = this.resolvedProperty;
    for (let i = 0, n = dest.length; i !== n; ++i) dest[i] = buffer[offset++];
  }
  _setValue_arrayElement(buffer: KeyframeArray, offset: number): void {
    this.resolvedProperty[this.propertyIndex!] = buffer[offset];
  }
  _setValue_fromArray(buffer: KeyframeArray, offset: number): void {
    this.resolvedProperty.fromArray(buffer, offset);
  }
  _getValue_unbound(buffer: KeyframeArray, offset: number): void {
    this.bind();
    this.getValue(buffer, offset);
  }
  _setValue_unbound(buffer: KeyframeArray, offset: number): void {
    this.bind();
    this.setValue(buffer, offset);
  }

  /** Resolves the track's property; a missing node or property warns and leaves the binding a no-op. */
  bind(): void {
    let targetObject = this.node;
    const parsedPath = this.parsedPath, objectName = parsedPath.objectName, propertyName = parsedPath.propertyName;
    let propertyIndex = parsedPath.propertyIndex;
    if (!targetObject) {
      targetObject = PropertyBinding.findNode(this.rootNode, parsedPath.nodeName);
      this.node = targetObject;
    }
    this.getValue = this._getValue_unavailable;
    this.setValue = this._setValue_unavailable;
    if (!targetObject) {
      console.warn("THREE.PropertyBinding: No target node found for track: " + this.path + ".");
      return;
    }
    if (objectName) {
      let objectIndex = parsedPath.objectIndex;
      switch (objectName) {
        case "materials":
          if (!targetObject.material) return void console.error("THREE.PropertyBinding: Can not bind to material as node does not have a material.", this);
          if (!targetObject.material.materials) return void console.error("THREE.PropertyBinding: Can not bind to material.materials as node.material does not have a materials array.", this);
          targetObject = targetObject.material.materials;
          break;
        case "bones":
          if (!targetObject.skeleton) return void console.error("THREE.PropertyBinding: Can not bind to bones as node does not have a skeleton.", this);
          targetObject = targetObject.skeleton.bones;
          for (let i = 0; i < targetObject.length; i++) {
            if (targetObject[i].name === objectIndex) {
              objectIndex = i;
              break;
            }
          }
          break;
        case "map":
          if ("map" in targetObject) {
            targetObject = targetObject.map;
            break;
          }
          if (!targetObject.material) return void console.error("THREE.PropertyBinding: Can not bind to material as node does not have a material.", this);
          if (!targetObject.material.map) return void console.error("THREE.PropertyBinding: Can not bind to material.map as node.material does not have a map.", this);
          targetObject = targetObject.material.map;
          break;
        default:
          if (targetObject[objectName] === undefined) return void console.error("THREE.PropertyBinding: Can not bind to objectName of node undefined.", this);
          targetObject = targetObject[objectName];
      }
      if (objectIndex !== undefined) {
        if (targetObject[objectIndex] === undefined) return void console.error("THREE.PropertyBinding: Trying to bind to objectIndex of objectName, but is undefined.", this, targetObject);
        targetObject = targetObject[objectIndex];
      }
    }
    const nodeProperty = targetObject[propertyName];
    if (nodeProperty === undefined) {
      console.error("THREE.PropertyBinding: Trying to update property for track: " + parsedPath.nodeName + "." + propertyName + " but it wasn't found.", targetObject);
      return;
    }
    let versioning: number = Versioning.None;
    this.targetObject = targetObject;
    if (targetObject.isMaterial === true) versioning = Versioning.NeedsUpdate;
    // three.js also flags an Object3D's matrixWorldNeedsUpdate; ThreeRT's position / quaternion / scale report their
    // own changes, and its objects have no such flag.
    else if (targetObject.isObject3D === true && "matrixWorldNeedsUpdate" in targetObject) versioning = Versioning.MatrixWorldNeedsUpdate;
    let bindingType: number = BindingType.Direct;
    if (propertyIndex !== undefined) {
      if (propertyName === "morphTargetInfluences") {
        if (!targetObject.geometry) return void console.error("THREE.PropertyBinding: Can not bind to morphTargetInfluences because node does not have a geometry.", this);
        if (!targetObject.geometry.morphAttributes) return void console.error("THREE.PropertyBinding: Can not bind to morphTargetInfluences because node does not have a geometry.morphAttributes.", this);
        if (targetObject.morphTargetDictionary[propertyIndex] !== undefined) propertyIndex = targetObject.morphTargetDictionary[propertyIndex];
      }
      bindingType = BindingType.ArrayElement;
      this.resolvedProperty = nodeProperty;
      this.propertyIndex = propertyIndex;
    } else if (nodeProperty.fromArray !== undefined && nodeProperty.toArray !== undefined) {
      bindingType = BindingType.HasFromToArray;
      this.resolvedProperty = nodeProperty;
    } else if (Array.isArray(nodeProperty)) {
      bindingType = BindingType.EntireArray;
      this.resolvedProperty = nodeProperty;
    } else {
      this.propertyName = propertyName;
    }
    const getters = [this._getValue_direct, this._getValue_array, this._getValue_arrayElement, this._getValue_toArray];
    const setters = [this._setValue_direct, this._setValue_array, this._setValue_arrayElement, this._setValue_fromArray];
    this.getValue = getters[bindingType];
    const set = setters[bindingType];
    this.setValue =
      versioning === Versioning.NeedsUpdate
        ? function (this: PropertyBinding, b, o) { set.call(this, b, o); this.targetObject.needsUpdate = true; }
        : versioning === Versioning.MatrixWorldNeedsUpdate
          ? function (this: PropertyBinding, b, o) { set.call(this, b, o); this.targetObject.matrixWorldNeedsUpdate = true; }
          : set;
  }

  unbind(): void {
    this.node = null;
    this.getValue = this._getValue_unbound;
    this.setValue = this._setValue_unbound;
  }
}

// --- PropertyMixer ---

type MixFn = (this: PropertyMixer, buffer: KeyframeArray, dstOffset: number, srcOffset: number, t: number, stride: number) => void;

/** three.js PropertyMixer: accumulates the weighted values every action gives one bound property, then applies them. */
export class PropertyMixer {
  binding: PropertyBinding;
  valueSize: number;
  buffer: KeyframeArray;
  cumulativeWeight = 0;
  cumulativeWeightAdditive = 0;
  useCount = 0;
  referenceCount = 0;
  _cacheIndex: number | null = null;
  private _workIndex = 0;
  private _origIndex = 3;
  private _addIndex = 4;
  private _mixBufferRegion: MixFn;
  private _mixBufferRegionAdditive: MixFn;
  private _setIdentity: () => void;

  constructor(binding: PropertyBinding, typeName: string, valueSize: number) {
    this.binding = binding;
    this.valueSize = valueSize;
    switch (typeName) {
      case "quaternion":
        this._mixBufferRegion = this._slerp;
        this._mixBufferRegionAdditive = this._slerpAdditive;
        this._setIdentity = this._setAdditiveIdentityQuaternion;
        this.buffer = new Float64Array(valueSize * 6);
        this._workIndex = 5;
        break;
      case "string":
      case "bool":
        this._mixBufferRegion = this._select;
        this._mixBufferRegionAdditive = this._select;
        this._setIdentity = this._setAdditiveIdentityOther;
        this.buffer = new Array(valueSize * 5);
        break;
      default:
        this._mixBufferRegion = this._lerp;
        this._mixBufferRegionAdditive = this._lerpAdditive;
        this._setIdentity = this._setAdditiveIdentityNumeric;
        this.buffer = new Float64Array(valueSize * 5);
    }
  }

  accumulate(accuIndex: number, weight: number): void {
    const buffer = this.buffer, stride = this.valueSize, offset = accuIndex * stride + stride;
    let currentWeight = this.cumulativeWeight;
    if (currentWeight === 0) {
      for (let i = 0; i !== stride; ++i) buffer[offset + i] = buffer[i];
      currentWeight = weight;
    } else {
      currentWeight += weight;
      this._mixBufferRegion(buffer, offset, 0, weight / currentWeight, stride);
    }
    this.cumulativeWeight = currentWeight;
  }

  accumulateAdditive(weight: number): void {
    const buffer = this.buffer, stride = this.valueSize, offset = stride * this._addIndex;
    if (this.cumulativeWeightAdditive === 0) this._setIdentity();
    this._mixBufferRegionAdditive(buffer, offset, 0, weight, stride);
    this.cumulativeWeightAdditive += weight;
  }

  apply(accuIndex: number): void {
    const stride = this.valueSize, buffer = this.buffer, offset = accuIndex * stride + stride;
    const weight = this.cumulativeWeight, weightAdditive = this.cumulativeWeightAdditive, binding = this.binding;
    this.cumulativeWeight = 0;
    this.cumulativeWeightAdditive = 0;
    if (weight < 1) this._mixBufferRegion(buffer, offset, stride * this._origIndex, 1 - weight, stride);
    if (weightAdditive > 0) this._mixBufferRegionAdditive(buffer, offset, this._addIndex * stride, 1, stride);
    for (let i = stride, e = stride + stride; i !== e; ++i) {
      if (buffer[i] !== buffer[i + stride]) {
        binding.setValue(buffer, offset);
        break;
      }
    }
  }

  saveOriginalState(): void {
    const buffer = this.buffer, stride = this.valueSize, originalValueOffset = stride * this._origIndex;
    this.binding.getValue(buffer, originalValueOffset);
    for (let i = stride, e = originalValueOffset; i !== e; ++i) buffer[i] = buffer[originalValueOffset + (i % stride)];
    this._setIdentity();
    this.cumulativeWeight = 0;
    this.cumulativeWeightAdditive = 0;
  }

  restoreOriginalState(): void {
    this.binding.setValue(this.buffer, this.valueSize * 3);
  }

  _setAdditiveIdentityNumeric(): void {
    const start = this._addIndex * this.valueSize, end = start + this.valueSize;
    for (let i = start; i < end; i++) this.buffer[i] = 0;
  }
  _setAdditiveIdentityQuaternion(): void {
    this._setAdditiveIdentityNumeric();
    this.buffer[this._addIndex * this.valueSize + 3] = 1;
  }
  _setAdditiveIdentityOther(): void {
    const start = this._origIndex * this.valueSize, target = this._addIndex * this.valueSize;
    for (let i = 0; i < this.valueSize; i++) this.buffer[target + i] = this.buffer[start + i];
  }
  _select(buffer: KeyframeArray, dstOffset: number, srcOffset: number, t: number, stride: number): void {
    if (t >= 0.5) for (let i = 0; i !== stride; ++i) buffer[dstOffset + i] = buffer[srcOffset + i];
  }
  _slerp(buffer: KeyframeArray, dstOffset: number, srcOffset: number, t: number): void {
    slerpFlat(buffer, dstOffset, buffer, dstOffset, buffer, srcOffset, t);
  }
  _slerpAdditive(buffer: KeyframeArray, dstOffset: number, srcOffset: number, t: number, stride: number): void {
    const workOffset = this._workIndex * stride;
    multiplyQuaternionsFlat(buffer, workOffset, buffer, dstOffset, buffer, srcOffset);
    slerpFlat(buffer, dstOffset, buffer, dstOffset, buffer, workOffset, t);
  }
  _lerp(buffer: KeyframeArray, dstOffset: number, srcOffset: number, t: number, stride: number): void {
    const s = 1 - t;
    for (let i = 0; i !== stride; ++i) {
      const j = dstOffset + i;
      buffer[j] = buffer[j] * s + buffer[srcOffset + i] * t;
    }
  }
  _lerpAdditive(buffer: KeyframeArray, dstOffset: number, srcOffset: number, t: number, stride: number): void {
    for (let i = 0; i !== stride; ++i) {
      const j = dstOffset + i;
      buffer[j] = buffer[j] + buffer[srcOffset + i] * t;
    }
  }
}

// --- AnimationAction ---

type ControlInterpolant = LinearInterpolant & { __cacheIndex: number };
type LoopMode = typeof LoopOnce | typeof LoopRepeat | typeof LoopPingPong;

/** three.js AnimationAction: one clip played by a mixer, with its own time, time scale, weight and loop mode. */
export class AnimationAction {
  blendMode: AnimationBlendMode;
  loop: LoopMode = LoopRepeat;
  time = 0;
  timeScale = 1;
  weight = 1;
  repetitions = Infinity;
  paused = false;
  enabled = true;
  clampWhenFinished = false;
  zeroSlopeAtStart = true;
  zeroSlopeAtEnd = true;
  _mixer: AnimationMixer;
  _clip: AnimationClip;
  _localRoot: AnimationRoot | null;
  _interpolantSettings: { endingStart: number; endingEnd: number };
  _interpolants: Interpolant[];
  _propertyBindings: PropertyMixer[];
  _cacheIndex: number | null = null;
  _byClipCacheIndex: number | null = null;
  _timeScaleInterpolant: ControlInterpolant | null = null;
  _restoreTimeScale: number | null = null;
  _weightInterpolant: ControlInterpolant | null = null;
  _loopCount = -1;
  _startTime: number | null = null;
  _effectiveTimeScale = 1;
  _effectiveWeight = 1;

  constructor(mixer: AnimationMixer, clip: AnimationClip, localRoot: AnimationRoot | null = null, blendMode: AnimationBlendMode = clip.blendMode) {
    this._mixer = mixer;
    this._clip = clip;
    this._localRoot = localRoot;
    this.blendMode = blendMode;
    const interpolantSettings = { endingStart: ZeroCurvatureEnding, endingEnd: ZeroCurvatureEnding };
    this._interpolants = clip.tracks.map((track) => {
      const interpolant = track.createInterpolant(null as unknown as undefined);
      interpolant.settings = interpolantSettings;
      return interpolant;
    });
    this._interpolantSettings = interpolantSettings;
    this._propertyBindings = new Array(clip.tracks.length);
  }

  play(): this {
    this._mixer._activateAction(this);
    return this;
  }
  /** Stops it, restores what it animated (when no other action uses it) and resets it. */
  stop(): this {
    this._mixer._deactivateAction(this);
    return this.reset();
  }
  reset(): this {
    this.paused = false;
    this.enabled = true;
    this.time = 0;
    this._loopCount = -1;
    this._startTime = null;
    return this.stopFading().stopWarping();
  }
  isRunning(): boolean {
    return this.enabled && !this.paused && this.timeScale !== 0 && this._startTime === null && this._mixer._isActiveAction(this);
  }
  isScheduled(): boolean {
    return this._mixer._isActiveAction(this);
  }
  /** Delays the start to the mixer time `time`. */
  startAt(time: number): this {
    this._startTime = time;
    return this;
  }
  setLoop(mode: LoopMode, repetitions: number): this {
    this.loop = mode;
    this.repetitions = repetitions;
    return this;
  }
  setEffectiveWeight(weight: number): this {
    this.weight = weight;
    this._effectiveWeight = this.enabled ? weight : 0;
    return this.stopFading();
  }
  getEffectiveWeight(): number {
    return this._effectiveWeight;
  }
  fadeIn(duration: number): this {
    return this._scheduleFading(duration, 0, 1);
  }
  fadeOut(duration: number): this {
    return this._scheduleFading(duration, 1, 0);
  }
  crossFadeFrom(fadeOutAction: AnimationAction, duration: number, warp = false): this {
    fadeOutAction.fadeOut(duration);
    this.fadeIn(duration);
    if (warp === true) {
      const fadeInDuration = this._clip.duration, fadeOutDuration = fadeOutAction._clip.duration;
      fadeOutAction._restoreTimeScale = fadeOutAction.timeScale;
      this._restoreTimeScale = this.timeScale;
      fadeOutAction.warp(1.0, fadeOutDuration / fadeInDuration, duration);
      this.warp(fadeInDuration / fadeOutDuration, 1.0, duration);
    }
    return this;
  }
  crossFadeTo(fadeInAction: AnimationAction, duration: number, warp = false): AnimationAction {
    return fadeInAction.crossFadeFrom(this, duration, warp);
  }
  stopFading(): this {
    const weightInterpolant = this._weightInterpolant;
    if (weightInterpolant !== null) {
      this._weightInterpolant = null;
      this._mixer._takeBackControlInterpolant(weightInterpolant);
    }
    return this;
  }
  setEffectiveTimeScale(timeScale: number): this {
    this.timeScale = timeScale;
    this._effectiveTimeScale = this.paused ? 0 : timeScale;
    return this.stopWarping();
  }
  getEffectiveTimeScale(): number {
    return this._effectiveTimeScale;
  }
  /** Sets the time scale so that one loop takes `duration` seconds. */
  setDuration(duration: number): this {
    this.timeScale = this._clip.duration / duration;
    return this.stopWarping();
  }
  syncWith(action: AnimationAction): this {
    this.time = action.time;
    this.timeScale = action.timeScale;
    return this.stopWarping();
  }
  /** Slows to a stop over `duration` seconds. */
  halt(duration: number): this {
    return this.warp(this._effectiveTimeScale, 0, duration);
  }
  warp(startTimeScale: number, endTimeScale: number, duration: number): this {
    const mixer = this._mixer, now = mixer.time, timeScale = this.timeScale;
    let interpolant = this._timeScaleInterpolant;
    if (interpolant === null) {
      interpolant = mixer._lendControlInterpolant();
      this._timeScaleInterpolant = interpolant;
    }
    const times = interpolant.parameterPositions, values = interpolant.sampleValues;
    times[0] = now;
    times[1] = now + duration;
    values[0] = startTimeScale / timeScale;
    values[1] = endTimeScale / timeScale;
    return this;
  }
  stopWarping(): this {
    const timeScaleInterpolant = this._timeScaleInterpolant;
    if (timeScaleInterpolant !== null) {
      this._timeScaleInterpolant = null;
      this._mixer._takeBackControlInterpolant(timeScaleInterpolant);
    }
    this._restoreTimeScale = null;
    return this;
  }
  getMixer(): AnimationMixer {
    return this._mixer;
  }
  getClip(): AnimationClip {
    return this._clip;
  }
  getRoot(): AnimationRoot {
    return this._localRoot || this._mixer._root;
  }

  _update(time: number, deltaTime: number, timeDirection: number, accuIndex: number): void {
    if (!this.enabled) {
      this._updateWeight(time);
      return;
    }
    const startTime = this._startTime;
    if (startTime !== null) {
      const timeRunning = (time - startTime) * timeDirection;
      if (timeRunning < 0 || timeDirection === 0) {
        deltaTime = 0;
      } else {
        this._startTime = null;
        deltaTime = timeDirection * timeRunning;
      }
    }
    deltaTime *= this._updateTimeScale(time);
    const clipTime = this._updateTime(deltaTime);
    const weight = this._updateWeight(time);
    if (weight > 0) {
      const interpolants = this._interpolants, propertyMixers = this._propertyBindings;
      if (this.blendMode === AdditiveAnimationBlendMode) {
        for (let j = 0, m = interpolants.length; j !== m; ++j) {
          interpolants[j].evaluate(clipTime);
          propertyMixers[j].accumulateAdditive(weight);
        }
      } else {
        for (let j = 0, m = interpolants.length; j !== m; ++j) {
          interpolants[j].evaluate(clipTime);
          propertyMixers[j].accumulate(accuIndex, weight);
        }
      }
    }
  }

  _updateWeight(time: number): number {
    let weight = 0;
    if (this.enabled) {
      weight = this.weight;
      const interpolant = this._weightInterpolant;
      if (interpolant !== null) {
        const interpolantValue = interpolant.evaluate(time)[0];
        weight *= interpolantValue;
        if (time > interpolant.parameterPositions[1]) {
          this.stopFading();
          if (interpolantValue === 0) this.enabled = false;
        }
      }
    }
    this._effectiveWeight = weight;
    return weight;
  }

  _updateTimeScale(time: number): number {
    let timeScale = 0;
    if (!this.paused) {
      timeScale = this.timeScale;
      const interpolant = this._timeScaleInterpolant;
      if (interpolant !== null) {
        const interpolantValue = interpolant.evaluate(time)[0];
        timeScale *= interpolantValue;
        if (time > interpolant.parameterPositions[1]) {
          if (timeScale === 0) {
            this.paused = true;
          } else {
            if (this._restoreTimeScale !== null) timeScale = this._restoreTimeScale;
            this.timeScale = timeScale;
          }
          this.stopWarping();
        }
      }
    }
    this._effectiveTimeScale = timeScale;
    return timeScale;
  }

  _updateTime(deltaTime: number): number {
    const duration = this._clip.duration, loop = this.loop;
    let time = this.time + deltaTime, loopCount = this._loopCount;
    const pingPong = loop === LoopPingPong;
    if (deltaTime === 0) {
      if (loopCount === -1) return time;
      return pingPong && (loopCount & 1) === 1 ? duration - time : time;
    }
    if (loop === LoopOnce) {
      if (loopCount === -1) {
        this._loopCount = 0;
        this._setEndings(true, true, false);
      }
      handle_stop: {
        if (time >= duration) time = duration;
        else if (time < 0) time = 0;
        else {
          this.time = time;
          break handle_stop;
        }
        if (this.clampWhenFinished) this.paused = true;
        else this.enabled = false;
        this.time = time;
        this._mixer.dispatchEvent({ type: "finished", action: this, direction: deltaTime < 0 ? -1 : 1 });
      }
    } else {
      if (loopCount === -1) {
        if (deltaTime >= 0) {
          loopCount = 0;
          this._setEndings(true, this.repetitions === 0, pingPong);
        } else {
          this._setEndings(this.repetitions === 0, true, pingPong);
        }
      }
      if (time >= duration || time < 0) {
        const loopDelta = Math.floor(time / duration);
        time -= duration * loopDelta;
        loopCount += Math.abs(loopDelta);
        const pending = this.repetitions - loopCount;
        if (pending <= 0) {
          if (this.clampWhenFinished) this.paused = true;
          else this.enabled = false;
          time = deltaTime > 0 ? duration : 0;
          this.time = time;
          this._mixer.dispatchEvent({ type: "finished", action: this, direction: deltaTime > 0 ? 1 : -1 });
        } else {
          if (pending === 1) {
            const atStart = deltaTime < 0;
            this._setEndings(atStart, !atStart, pingPong);
          } else {
            this._setEndings(false, false, pingPong);
          }
          this._loopCount = loopCount;
          this.time = time;
          this._mixer.dispatchEvent({ type: "loop", action: this, loopDelta });
        }
      } else {
        this._loopCount = loopCount;
        this.time = time;
      }
      if (pingPong && (loopCount & 1) === 1) return duration - time;
    }
    return time;
  }

  _setEndings(atStart: boolean, atEnd: boolean, pingPong: boolean): void {
    const settings = this._interpolantSettings;
    if (pingPong) {
      settings.endingStart = ZeroSlopeEnding;
      settings.endingEnd = ZeroSlopeEnding;
    } else {
      settings.endingStart = atStart ? (this.zeroSlopeAtStart ? ZeroSlopeEnding : ZeroCurvatureEnding) : WrapAroundEnding;
      settings.endingEnd = atEnd ? (this.zeroSlopeAtEnd ? ZeroSlopeEnding : ZeroCurvatureEnding) : WrapAroundEnding;
    }
  }

  _scheduleFading(duration: number, weightNow: number, weightThen: number): this {
    const mixer = this._mixer, now = mixer.time;
    let interpolant = this._weightInterpolant;
    if (interpolant === null) {
      interpolant = mixer._lendControlInterpolant();
      this._weightInterpolant = interpolant;
    }
    const times = interpolant.parameterPositions, values = interpolant.sampleValues;
    times[0] = now;
    values[0] = weightNow;
    times[1] = now + duration;
    values[1] = weightThen;
    return this;
  }
}

// --- AnimationMixer ---

export interface AnimationMixerEvent {
  type: "loop" | "finished" | (string & {});
  action?: AnimationAction;
  direction?: number;
  loopDelta?: number;
  target?: AnimationMixer;
  [k: string]: unknown;
}

const controlResultBuffer = new Float32Array(1);

/** three.js AnimationMixer: plays clips on `root` and the objects under it (found by name). Call `update(dt)` once a
 * frame; it writes the blended values to the objects' properties. */
export class AnimationMixer {
  time = 0;
  timeScale = 1.0;
  _root: AnimationRoot;
  _accuIndex = 0;
  _actions: AnimationAction[] = [];
  _nActiveActions = 0;
  _actionsByClip: Record<string, { knownActions: AnimationAction[]; actionByRoot: Record<string, AnimationAction> }> = {};
  _bindings: PropertyMixer[] = [];
  _nActiveBindings = 0;
  _bindingsByRootAndName: Record<string, Record<string, PropertyMixer>> = {};
  _controlInterpolants: ControlInterpolant[] = [];
  _nActiveControlInterpolants = 0;
  private _listeners?: Map<string, ((e: AnimationMixerEvent) => void)[]>;
  readonly stats: {
    actions: { readonly total: number; readonly inUse: number };
    bindings: { readonly total: number; readonly inUse: number };
    controlInterpolants: { readonly total: number; readonly inUse: number };
  };

  constructor(root: AnimationRoot) {
    this._root = root;
    const scope = this;
    this.stats = {
      actions: { get total() { return scope._actions.length; }, get inUse() { return scope._nActiveActions; } },
      bindings: { get total() { return scope._bindings.length; }, get inUse() { return scope._nActiveBindings; } },
      controlInterpolants: { get total() { return scope._controlInterpolants.length; }, get inUse() { return scope._nActiveControlInterpolants; } },
    };
  }

  // three.js's EventDispatcher: "loop" and "finished".
  addEventListener(type: string, listener: (e: AnimationMixerEvent) => void): void {
    const all = (this._listeners ??= new Map());
    const l = all.get(type);
    if (!l) all.set(type, [listener]);
    else if (!l.includes(listener)) l.push(listener);
  }
  hasEventListener(type: string, listener: (e: AnimationMixerEvent) => void): boolean {
    return this._listeners?.get(type)?.includes(listener) ?? false;
  }
  removeEventListener(type: string, listener: (e: AnimationMixerEvent) => void): void {
    const l = this._listeners?.get(type);
    if (l?.includes(listener)) l.splice(l.indexOf(listener), 1);
  }
  dispatchEvent(event: AnimationMixerEvent): void {
    const l = this._listeners?.get(event.type);
    if (!l?.length) return;
    event.target = this;
    for (const f of [...l]) f(event);
    event.target = undefined;
  }

  _bindAction(action: AnimationAction, prototypeAction?: AnimationAction | null): void {
    const root = action._localRoot || this._root, tracks = action._clip.tracks, bindings = action._propertyBindings;
    const interpolants = action._interpolants, rootUuid = rootKey(root), bindingsByRoot = this._bindingsByRootAndName;
    let bindingsByName = bindingsByRoot[rootUuid];
    if (bindingsByName === undefined) bindingsByRoot[rootUuid] = bindingsByName = {};
    for (let i = 0; i !== tracks.length; ++i) {
      const track = tracks[i], trackName = track.name;
      let binding = bindingsByName[trackName];
      if (binding !== undefined) {
        ++binding.referenceCount;
        bindings[i] = binding;
      } else {
        binding = bindings[i];
        if (binding !== undefined) {
          if (binding._cacheIndex === null) {
            ++binding.referenceCount;
            this._addInactiveBinding(binding, rootUuid, trackName);
          }
          continue;
        }
        const path = prototypeAction && prototypeAction._propertyBindings[i].binding.parsedPath;
        binding = new PropertyMixer(PropertyBinding.create(root, trackName, path || undefined), track.ValueTypeName, track.getValueSize());
        ++binding.referenceCount;
        this._addInactiveBinding(binding, rootUuid, trackName);
        bindings[i] = binding;
      }
      interpolants[i].resultBuffer = binding.buffer;
    }
  }

  _activateAction(action: AnimationAction): void {
    if (!this._isActiveAction(action)) {
      if (action._cacheIndex === null) {
        const rootUuid = rootKey(action._localRoot || this._root), clipUuid = action._clip.uuid, actionsForClip = this._actionsByClip[clipUuid];
        this._bindAction(action, actionsForClip && actionsForClip.knownActions[0]);
        this._addInactiveAction(action, clipUuid, rootUuid);
      }
      for (const binding of action._propertyBindings) {
        if (binding.useCount++ === 0) {
          this._lendBinding(binding);
          binding.saveOriginalState();
        }
      }
      this._lendAction(action);
    }
  }

  _deactivateAction(action: AnimationAction): void {
    if (this._isActiveAction(action)) {
      for (const binding of action._propertyBindings) {
        if (--binding.useCount === 0) {
          binding.restoreOriginalState();
          this._takeBackBinding(binding);
        }
      }
      this._takeBackAction(action);
    }
  }

  _isActiveAction(action: AnimationAction): boolean {
    const index = action._cacheIndex;
    return index !== null && index < this._nActiveActions;
  }

  _addInactiveAction(action: AnimationAction, clipUuid: string, rootUuid: string): void {
    const actions = this._actions, actionsByClip = this._actionsByClip;
    let actionsForClip = actionsByClip[clipUuid];
    if (actionsForClip === undefined) {
      actionsForClip = { knownActions: [action], actionByRoot: {} };
      action._byClipCacheIndex = 0;
      actionsByClip[clipUuid] = actionsForClip;
    } else {
      action._byClipCacheIndex = actionsForClip.knownActions.length;
      actionsForClip.knownActions.push(action);
    }
    action._cacheIndex = actions.length;
    actions.push(action);
    actionsForClip.actionByRoot[rootUuid] = action;
  }

  _removeInactiveAction(action: AnimationAction): void {
    const actions = this._actions, lastInactiveAction = actions[actions.length - 1], cacheIndex = action._cacheIndex!;
    lastInactiveAction._cacheIndex = cacheIndex;
    actions[cacheIndex] = lastInactiveAction;
    actions.pop();
    action._cacheIndex = null;
    const clipUuid = action._clip.uuid, actionsByClip = this._actionsByClip, actionsForClip = actionsByClip[clipUuid];
    const knownActionsForClip = actionsForClip.knownActions, lastKnownAction = knownActionsForClip[knownActionsForClip.length - 1];
    const byClipCacheIndex = action._byClipCacheIndex!;
    lastKnownAction._byClipCacheIndex = byClipCacheIndex;
    knownActionsForClip[byClipCacheIndex] = lastKnownAction;
    knownActionsForClip.pop();
    action._byClipCacheIndex = null;
    delete actionsForClip.actionByRoot[rootKey(action._localRoot || this._root)];
    if (knownActionsForClip.length === 0) delete actionsByClip[clipUuid];
    this._removeInactiveBindingsForAction(action);
  }

  _removeInactiveBindingsForAction(action: AnimationAction): void {
    for (const binding of action._propertyBindings) if (--binding.referenceCount === 0) this._removeInactiveBinding(binding);
  }

  _lendAction(action: AnimationAction): void {
    const actions = this._actions, prevIndex = action._cacheIndex!, lastActiveIndex = this._nActiveActions++, firstInactiveAction = actions[lastActiveIndex];
    action._cacheIndex = lastActiveIndex;
    actions[lastActiveIndex] = action;
    firstInactiveAction._cacheIndex = prevIndex;
    actions[prevIndex] = firstInactiveAction;
  }

  _takeBackAction(action: AnimationAction): void {
    const actions = this._actions, prevIndex = action._cacheIndex!, firstInactiveIndex = --this._nActiveActions, lastActiveAction = actions[firstInactiveIndex];
    action._cacheIndex = firstInactiveIndex;
    actions[firstInactiveIndex] = action;
    lastActiveAction._cacheIndex = prevIndex;
    actions[prevIndex] = lastActiveAction;
  }

  _addInactiveBinding(binding: PropertyMixer, rootUuid: string, trackName: string): void {
    const bindingsByRoot = this._bindingsByRootAndName, bindings = this._bindings;
    let bindingByName = bindingsByRoot[rootUuid];
    if (bindingByName === undefined) bindingsByRoot[rootUuid] = bindingByName = {};
    bindingByName[trackName] = binding;
    binding._cacheIndex = bindings.length;
    bindings.push(binding);
  }

  _removeInactiveBinding(binding: PropertyMixer): void {
    const bindings = this._bindings, propBinding = binding.binding, rootUuid = rootKey(propBinding.rootNode), trackName = propBinding.path;
    const bindingsByRoot = this._bindingsByRootAndName, bindingByName = bindingsByRoot[rootUuid];
    const lastInactiveBinding = bindings[bindings.length - 1], cacheIndex = binding._cacheIndex!;
    lastInactiveBinding._cacheIndex = cacheIndex;
    bindings[cacheIndex] = lastInactiveBinding;
    bindings.pop();
    delete bindingByName[trackName];
    if (Object.keys(bindingByName).length === 0) delete bindingsByRoot[rootUuid];
  }

  _lendBinding(binding: PropertyMixer): void {
    const bindings = this._bindings, prevIndex = binding._cacheIndex!, lastActiveIndex = this._nActiveBindings++, firstInactiveBinding = bindings[lastActiveIndex];
    binding._cacheIndex = lastActiveIndex;
    bindings[lastActiveIndex] = binding;
    firstInactiveBinding._cacheIndex = prevIndex;
    bindings[prevIndex] = firstInactiveBinding;
  }

  _takeBackBinding(binding: PropertyMixer): void {
    const bindings = this._bindings, prevIndex = binding._cacheIndex!, firstInactiveIndex = --this._nActiveBindings, lastActiveBinding = bindings[firstInactiveIndex];
    binding._cacheIndex = firstInactiveIndex;
    bindings[firstInactiveIndex] = binding;
    lastActiveBinding._cacheIndex = prevIndex;
    bindings[prevIndex] = lastActiveBinding;
  }

  _lendControlInterpolant(): ControlInterpolant {
    const interpolants = this._controlInterpolants, lastActiveIndex = this._nActiveControlInterpolants++;
    let interpolant = interpolants[lastActiveIndex];
    if (interpolant === undefined) {
      interpolant = new LinearInterpolant(new Float32Array(2), new Float32Array(2), 1, controlResultBuffer) as ControlInterpolant;
      interpolant.__cacheIndex = lastActiveIndex;
      interpolants[lastActiveIndex] = interpolant;
    }
    return interpolant;
  }

  _takeBackControlInterpolant(interpolant: ControlInterpolant): void {
    const interpolants = this._controlInterpolants, prevIndex = interpolant.__cacheIndex, firstInactiveIndex = --this._nActiveControlInterpolants;
    const lastActiveInterpolant = interpolants[firstInactiveIndex];
    interpolant.__cacheIndex = firstInactiveIndex;
    interpolants[firstInactiveIndex] = interpolant;
    lastActiveInterpolant.__cacheIndex = prevIndex;
    interpolants[prevIndex] = lastActiveInterpolant;
  }

  /** The action that plays `clip` (or the root's clip of that name) on `optionalRoot` or the mixer's root; the same
   * action on later calls. Null when no clip has that name. */
  clipAction(clip: AnimationClip | string, optionalRoot?: AnimationRoot, blendMode?: AnimationBlendMode): AnimationAction | null {
    const root = optionalRoot || this._root, rootUuid = rootKey(root);
    let clipObject = typeof clip === "string" ? AnimationClip.findByName(root, clip) : clip;
    const clipUuid = clipObject !== null ? clipObject.uuid : (clip as string);
    const actionsForClip = this._actionsByClip[clipUuid];
    let prototypeAction: AnimationAction | null = null;
    if (blendMode === undefined) blendMode = clipObject !== null ? clipObject.blendMode : NormalAnimationBlendMode;
    if (actionsForClip !== undefined) {
      const existingAction = actionsForClip.actionByRoot[rootUuid];
      if (existingAction !== undefined && existingAction.blendMode === blendMode) return existingAction;
      prototypeAction = actionsForClip.knownActions[0];
      if (clipObject === null) clipObject = prototypeAction._clip;
    }
    if (clipObject === null) return null;
    const newAction = new AnimationAction(this, clipObject, optionalRoot, blendMode);
    this._bindAction(newAction, prototypeAction);
    this._addInactiveAction(newAction, clipUuid, rootUuid);
    return newAction;
  }

  existingAction(clip: AnimationClip | string, optionalRoot?: AnimationRoot): AnimationAction | null {
    const root = optionalRoot || this._root, rootUuid = rootKey(root);
    const clipObject = typeof clip === "string" ? AnimationClip.findByName(root, clip) : clip;
    const clipUuid = clipObject ? clipObject.uuid : (clip as string), actionsForClip = this._actionsByClip[clipUuid];
    if (actionsForClip !== undefined) return actionsForClip.actionByRoot[rootUuid] || null;
    return null;
  }

  stopAllAction(): this {
    const actions = this._actions;
    for (let i = this._nActiveActions - 1; i >= 0; --i) actions[i].stop();
    return this;
  }

  /** Advances the mixer's time by `deltaTime` seconds (times `timeScale`) and applies every running action. */
  update(deltaTime: number): this {
    deltaTime *= this.timeScale;
    const actions = this._actions, nActions = this._nActiveActions, time = (this.time += deltaTime);
    const timeDirection = Math.sign(deltaTime), accuIndex = (this._accuIndex ^= 1);
    for (let i = 0; i !== nActions; ++i) actions[i]._update(time, deltaTime, timeDirection, accuIndex);
    const bindings = this._bindings, nBindings = this._nActiveBindings;
    for (let i = 0; i !== nBindings; ++i) bindings[i].apply(accuIndex);
    return this;
  }

  /** Puts the mixer and every action at `time` seconds and applies them. */
  setTime(time: number): this {
    this.time = 0;
    for (const action of this._actions) action.time = 0;
    return this.update(time);
  }

  getRoot(): AnimationRoot {
    return this._root;
  }

  /** Forgets every action of `clip` (stopping them) and their bindings. */
  uncacheClip(clip: AnimationClip): void {
    const actions = this._actions, clipUuid = clip.uuid, actionsByClip = this._actionsByClip, actionsForClip = actionsByClip[clipUuid];
    if (actionsForClip !== undefined) {
      for (const action of actionsForClip.knownActions) {
        this._deactivateAction(action);
        const cacheIndex = action._cacheIndex!, lastInactiveAction = actions[actions.length - 1];
        action._cacheIndex = null;
        action._byClipCacheIndex = null;
        lastInactiveAction._cacheIndex = cacheIndex;
        actions[cacheIndex] = lastInactiveAction;
        actions.pop();
        this._removeInactiveBindingsForAction(action);
      }
      delete actionsByClip[clipUuid];
    }
  }

  /** Forgets every action and binding on `root`, restoring what they animated. */
  uncacheRoot(root: AnimationRoot): void {
    const rootUuid = rootKey(root), actionsByClip = this._actionsByClip;
    for (const clipUuid in actionsByClip) {
      const action = actionsByClip[clipUuid].actionByRoot[rootUuid];
      if (action !== undefined) {
        this._deactivateAction(action);
        this._removeInactiveAction(action);
      }
    }
    const bindingByName = this._bindingsByRootAndName[rootUuid];
    if (bindingByName !== undefined) {
      for (const trackName in bindingByName) {
        const binding = bindingByName[trackName];
        binding.restoreOriginalState();
        this._removeInactiveBinding(binding);
      }
    }
  }

  uncacheAction(clip: AnimationClip | string, optionalRoot?: AnimationRoot): void {
    const action = this.existingAction(clip, optionalRoot);
    if (action !== null) {
      this._deactivateAction(action);
      this._removeInactiveAction(action);
    }
  }
}
