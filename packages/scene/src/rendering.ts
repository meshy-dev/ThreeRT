/**
 * The live view's rendering settings, one per gap between what a frame affords and what is shown (the renderer's
 * src/live/rendering.rs has the whole table):
 *
 * - `quality`: how many rays, as the render resolution (`"auto"`: dynamic resolution).
 * - `reconstruction`: the noise and the missing pixels. ThreeRT's own denoiser and upscaler (`"builtin"`), an upscaler
 *   that replaces only the upscaling (`"dlss"`, `"metalfx"`, `"fsr"`), or a joint denoiser that replaces both and
 *   reads the noisy signal itself (`"dlss-rr"`, `"metalfx-denoised"`). A list is an order of preference.
 * - `frameGeneration`: frames shown per frame rendered (none is integrated yet: it reports itself unavailable).
 * - `latency`: how far the CPU may run ahead of the GPU.
 *
 * Each is a request: the renderer runs what the device supports and `status()` says what ran and why not the rest.
 * They are device-local view settings (a player's graphics options), not scene state, and are not replicated.
 */

/** One way of reconstructing the frame. `custom:<id>` names a provider the embedding renderer registered. */
export type ReconstructionMethod = "builtin" | "off" | "fsr" | "metalfx" | "metalfx-denoised" | "dlss" | "dlss-rr" | `custom:${string}`;
export type ReconstructionRequest = "auto" | ReconstructionMethod | readonly ReconstructionMethod[];
export type QualityRequest = "auto" | "native" | "quality" | "balanced" | "performance" | "ultra-performance" | number;
export type FrameGenerationRequest = "off" | "auto" | "2x" | "3x" | "4x";
export type LatencyRequest = "off" | "auto" | "on";

/** The settings to change; those left out stay as they are. */
export interface RenderingRequest {
  quality?: QualityRequest;
  reconstruction?: ReconstructionRequest;
  frameGeneration?: FrameGenerationRequest;
  latency?: LatencyRequest;
}

export type SkipReason = "unavailable" | "unsupported" | "provider-error" | "diagnostic-view";

export interface RenderingStatus {
  readonly quality: { readonly requested: string; readonly scale: number; readonly dynamic: boolean };
  readonly reconstruction: {
    /** The request, as a comma-separated list or "auto". */
    readonly requested: string;
    /** The method the last frame ran; null before the first frame. */
    readonly active: ReconstructionMethod | null;
    /** Who removed the noise and who upscaled: "builtin", "none" or a method. */
    readonly denoise: string;
    readonly upscale: string;
    /** Why the first method asked for did not run (null when it did, or for "auto"). */
    readonly fallback: SkipReason | null;
    /** Every method tried before the one that ran, and why it did not. */
    readonly skipped: readonly { readonly method: string; readonly reason: SkipReason; readonly detail: string }[];
  };
  readonly frameGeneration: { readonly requested: FrameGenerationRequest; readonly active: "off"; readonly fallback: "unavailable" | null };
  readonly latency: { readonly requested: LatencyRequest; readonly active: "on" | "off"; readonly framesInFlight: number };
}

/** Wire methods on the scene's engine binding (native QuickJS or browser WASM): the request and status as JSON. */
export interface RenderingBinding {
  setRendering?(json: string): void;
  renderingStatus?(): string;
}

function binding(): RenderingBinding | undefined {
  const runtime: typeof globalThis & { __engine?: RenderingBinding } = globalThis;
  return runtime.__engine;
}

/** Device-local live-view settings; they do not edit scene settings or replicated game state. */
export const rendering = {
  /**
   * Queue a change for the next frame. False if there is no renderer binding (not a support check: read `status()`).
   * Throws a TypeError for an unknown setting or value.
   */
  set(request: RenderingRequest): boolean {
    const engine = binding();
    if (!engine?.setRendering) return false;
    engine.setRendering(JSON.stringify(request));
    return true;
  },
  /** What the last frame ran, with requests not yet applied shown as requested; undefined without a renderer. */
  status(): RenderingStatus | undefined {
    const text = binding()?.renderingStatus?.();
    if (text === undefined) return undefined;
    const status = JSON.parse(text) as RenderingStatus | null;
    return status ?? undefined;
  },
};
