import cinematic from "../../../presets/cinematic.json" with { type: "json" };
import preview from "../../../presets/preview.json" with { type: "json" };
import type { PresetName } from "./engine.ts";

export type RenderPreset = typeof cinematic;

export const presets: Record<PresetName, RenderPreset> = { cinematic, preview: preview as unknown as RenderPreset };

/** A preset with per-scene overrides deep-merged in. */
export function resolvePreset(name: PresetName, overrides?: Record<string, unknown>): RenderPreset {
  const base = presets[name];
  if (!base) throw new Error(`unknown preset ${name}`);
  return overrides ? (deepMerge(structuredClone(base), overrides) as RenderPreset) : base;
}

function deepMerge(a: Record<string, unknown>, b: Record<string, unknown>): Record<string, unknown> {
  for (const [k, v] of Object.entries(b)) {
    const cur = a[k];
    if (v && typeof v === "object" && !Array.isArray(v) && cur && typeof cur === "object" && !Array.isArray(cur)) deepMerge(cur as Record<string, unknown>, v as Record<string, unknown>);
    else a[k] = v;
  }
  return a;
}
