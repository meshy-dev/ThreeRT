// Shared pieces of the atomic rendering tests (tools/test-render.ts runs them; see docs/ROADMAP.md § 3).
//
// A test module exports `test` (metadata + the metric) and a default scene builder. Metrics read the renderer's linear
// HDR radiance (nits, before exposure / post) over regions of the image given as [u0, v0, u1, v1] in 0..1 (v down).
import { Color, Mesh, MeshPhysicalMaterial, PerspectiveCamera, PlaneGeometry, Scene, SkyColor, type MeshStandardMaterial, type Object3D } from "@threert/scene";

export type Region = [number, number, number, number];
export type Metric =
  /** Mean luminance (or RGB) of a region equals `expected` within a relative `tolerance`. */
  | { kind: "radiance"; region: Region; expected: number | [number, number, number]; tolerance: number }
  /** mean(region a) / mean(region b) equals `expected` within an absolute `tolerance`. */
  | { kind: "ratio"; a: Region; b: Region; expected: number; tolerance: number }
  /** The exposure the image was shown at (auto exposure: its converged / adapted value), EV100, within `tolerance` EV. */
  | { kind: "exposure"; expected: number; tolerance: number };

export interface TestMeta {
  id: string;
  features: string[];
  /** Where the expected value comes from: physics (analytic), a converged render, UE. */
  reference: "analytic" | "converged" | "ue";
  metric: Metric;
  temporal: boolean;
  /** Samples per pixel (default 64; noise must stay well inside the tolerance). */
  spp?: number;
  /** Render with the real-time renderer for this many frames (radiance cache, temporal passes) instead. */
  live?: number;
  /** Scene time, s (animated materials; default 0). */
  time?: number;
  /** Asset files the test needs (unsupported when missing). */
  requires?: string[];
  /** Why the expected value is what it is. */
  note?: string;
}

export const SIZE = { width: 160, height: 90 };

/** A pure Lambert surface (no specular lobe): radiance = albedo x irradiance / pi. */
export const lambert = (albedo: number, name = "lambert") =>
  new MeshPhysicalMaterial({ name, color: new Color().setRGB(albedo, albedo, albedo), roughness: 1, metalness: 0, specularIntensity: 0 });

export function plane(size: number, material: MeshStandardMaterial, name = "plane"): Mesh {
  const m = new Mesh(new PlaneGeometry(size, size), material);
  m.name = name;
  m.rotation.x = -Math.PI / 2;
  return m;
}

/** A scene with a uniform sky of `nits` (0 = black). */
export function sceneWithSky(nits: number): Scene {
  const scene = new Scene();
  scene.environment = new SkyColor(0xffffff, nits);
  return scene;
}

export function camera(scene: Scene, position: [number, number, number], target: [number, number, number], fov = 30): PerspectiveCamera {
  const c = new PerspectiveCamera(fov, 16 / 9, 0.01, 1000);
  c.position.set(...position);
  c.lookAt(...target);
  c.setManualExposure(0);
  scene.add(c);
  return c;
}

/** Test scene result with the defaults (small image, offline settings for an unbiased estimate). */
export function result(scene: Scene, cam: PerspectiveCamera, extra: Object3D[] = [], overrides: Record<string, unknown> = {}) {
  for (const o of extra) scene.add(o);
  return { scene, camera: cam, options: { name: "test", ...SIZE, preset: "cinematic" as const, overrides: { native: { energyCompensation: 0, ...overrides } } } };
}

export const CENTER: Region = [0.45, 0.45, 0.55, 0.55];
