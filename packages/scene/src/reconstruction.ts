/** Live HDR reconstruction. `custom:<id>` names a provider registered by the embedding renderer. */
export type ReconstructionMode = "builtin" | "auto" | "dlss" | "fsr" | "metalfx" | "metalfx-denoised" | `custom:${string}`;
export type ActiveReconstructionMode = Exclude<ReconstructionMode, "auto">;

export interface ReconstructionStatus {
  /** The currently requested mode. It may not yet have been evaluated. */
  readonly requested: ReconstructionMode;
  /** The implementation used for the last submitted view frame; null before the first frame. */
  readonly active: ActiveReconstructionMode | null;
  /** Why the last frame used the built-in path instead of the requested provider, if applicable. */
  readonly fallback: "unavailable" | "diagnostic-view" | "provider-error" | null;
}

/** Wire methods on the scene's engine binding (native QuickJS or browser WASM). */
export interface ReconstructionBinding {
  setReconstruction?(mode: ReconstructionMode): void;
  reconstructionStatus?(): string;
}

function binding(): ReconstructionBinding | undefined {
  const runtime: typeof globalThis & { __engine?: ReconstructionBinding } = globalThis;
  return runtime.__engine;
}

/** Device-local live-view controls; they do not edit scene settings or replicated game state. */
export const reconstruction = {
  /** Queue a mode for the next frame. False if there is no renderer binding, not a hardware support check. */
  set(mode: ReconstructionMode): boolean {
    const engine = binding();
    if (!engine?.setReconstruction) return false;
    engine.setReconstruction(mode);
    return true;
  },
  /** Read the requested mode and the mode actually used, or undefined without a renderer binding. */
  status(): ReconstructionStatus | undefined {
    const text = binding()?.reconstructionStatus?.();
    return text === undefined ? undefined : JSON.parse(text) as ReconstructionStatus;
  },
};
