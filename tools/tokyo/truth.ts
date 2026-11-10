// Our unbiased offline render of a Tokyo pose (4096 spp by default, no clamps, no denoiser): the truth the live viewport
// (tools/tokyo/step1.ts) and the source three.js app (tools/tokyo/three-app.ts) are measured against. Cached in
// out/cache/truth by bundle, room, pose and spp; rendered when missing.
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { rendererBin, repoRoot } from "../lib/bench.ts";
import type { POSES } from "../../games/tokyo/tokyo.game.ts";

type Pose = (typeof POSES)[string];

export function truth(bundle: string, room: string, name: string, pose: Pose, spp: number): string {
  const dir = join(repoRoot, "out/cache/truth");
  mkdirSync(dir, { recursive: true });
  const key = createHash("sha256").update(readFileSync(bundle)).update(`tokyo ${room} ${name} ${JSON.stringify(pose)} ${spp}`).digest("hex").slice(0, 12);
  const png = join(dir, `tokyo-${room}-${name}-${key}.png`);
  if (!existsSync(png)) {
    console.log(`  truth ${room} ${name} (${spp} spp)…`);
    const camera = { position: pose.position, target: pose.target, fov: pose.fov };
    execFileSync(rendererBin, ["render", bundle, "-o", png, "--assets", repoRoot, "--room", room, "--spp", String(spp), "--camera", JSON.stringify(camera), "--no-denoise", "--override", JSON.stringify({ native: { clampIndirect: 1e9 } })], { stdio: ["ignore", "ignore", "inherit"] });
  }
  return png;
}

export type Metrics = { mean_delta_e: number; ssim: number; luminance_ratio: number };

/** ΔE / SSIM / luminance of `png` against `reference` (the renderer's `compare`). */
export function compare(reference: string, png: string): Metrics {
  return JSON.parse(execFileSync(rendererBin, ["compare", reference, png]).toString().trim().split("\n").pop()!);
}
