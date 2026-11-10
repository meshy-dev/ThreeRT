// three.js names scene code reaches for that have no work to do in ThreeRT (ROADMAP § 6 step 2f): the renderer's
// set-up and its constants, accepted so that three.js code runs unchanged. ThreeRT renders the scene a game returns;
// shadows are always on (a path tracer's, from every light: the user's choice, a deliberate difference from three.js),
// tone mapping and colour output are the preset's.

export const BasicShadowMap = 0;
export const PCFShadowMap = 1;
export const PCFSoftShadowMap = 2;
export const VSMShadowMap = 3;
export const NoToneMapping = 0;
export const LinearToneMapping = 1;
export const ReinhardToneMapping = 2;
export const CineonToneMapping = 3;
export const ACESFilmicToneMapping = 4;
export const AgXToneMapping = 6;
export const NeutralToneMapping = 7;
export const SRGBColorSpace = "srgb";
export const LinearSRGBColorSpace = "srgb-linear";
export const NoColorSpace = "";
export const RepeatWrapping = 1000;
export const ClampToEdgeWrapping = 1001;
export const MirroredRepeatWrapping = 1002;

/** three.js WebGLRenderer: every setting accepted and ignored (`shadowMap.*` included); `render` does nothing. */
export class WebGLRenderer {
  readonly isWebGLRenderer = true;
  /** No canvas: ThreeRT draws on its own surface. */
  domElement = null;
  shadowMap = { enabled: false, type: PCFShadowMap, autoUpdate: true, needsUpdate: false };
  toneMapping = NoToneMapping;
  toneMappingExposure = 1;
  outputColorSpace = SRGBColorSpace;
  info = { render: { calls: 0, triangles: 0 }, memory: { geometries: 0, textures: 0 } };
  private pixelRatio = 1;
  private size = { width: 300, height: 150 };
  private loop: ((time: number) => void) | null = null;
  constructor(_parameters: Record<string, unknown> = {}) {}
  setSize(width: number, height: number): void {
    this.size = { width, height };
  }
  getSize<T extends { set(x: number, y: number): unknown }>(target: T): T {
    target.set(this.size.width, this.size.height);
    return target;
  }
  setPixelRatio(ratio: number): void {
    this.pixelRatio = ratio;
  }
  getPixelRatio(): number {
    return this.pixelRatio;
  }
  setClearColor(_color: unknown, _alpha?: number): void {}
  /** Kept, not called: a ThreeRT game's `frame` runs every frame instead. */
  setAnimationLoop(callback: ((time: number) => void) | null): void {
    this.loop = callback;
  }
  render(_scene: unknown, _camera: unknown): void {}
  compile(_scene: unknown, _camera: unknown): void {}
  dispose(): void {}
}
