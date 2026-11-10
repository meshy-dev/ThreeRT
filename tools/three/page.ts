/// <reference lib="dom" />
// Browser entry of the three.js baseline: runs a scene / game module (the same code the native renderer runs) through
// the ThreeRT runtime + SceneBinding into ThreeEngine, renders one still per tone mapper and measures the frame cost.
// Bundled per catalog entry by tools/bench-three.ts; the result is published as `window.__result`.
//
// Frames go through three.js's standard post stack: the scene renders into a linear HDR target (half float, 4x MSAA),
// a pass applies the render preset's vignette (the UE / native formula, the one post effect three.js can express
// exactly), and OutputPass applies the tone mapping (toneMappingExposure = 2^-EV100) and the sRGB encoding.
//
// URL parameters: ev100 (exposure for auto-exposure cameras: the EV100 the native renderer settled on), camera
// (JSON {position, target, fov}), frames (timed frames, default 120), width / height (default: the scene's settings),
// probe=0 (skip the scene-probe variant), path (JSON [{position, target, fov}, ...]: instead of the stills and timings,
// one JPEG per pose of a camera move, each prepared for its view as a still is; `images.path` = their data URLs).
import * as THREE from "three";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { ShaderPass } from "three/addons/postprocessing/ShaderPass.js";
import { defineGame, installNodeHost, type GameDefinition } from "@threert/gameplay";
import { ThreeEngine } from "./engine.ts";

/** renderer/src/post.rs `finish`: c *= 1 / (|pos|^2 + 1)^2, pos = NDC x (1, h/w) x sqrt2 / sqrt(1 + (h/w)^2) x vignette. */
export const VignetteShader = {
  name: "UEVignette",
  uniforms: { tDiffuse: { value: null }, intensity: { value: 0.4 }, aspect: { value: 9 / 16 } },
  vertexShader: /* glsl */ `varying vec2 vUv; void main() { vUv = uv; gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0); }`,
  fragmentShader: /* glsl */ `
    uniform sampler2D tDiffuse; uniform float intensity; uniform float aspect; varying vec2 vUv;
    void main() {
      vec4 c = texture2D(tDiffuse, vUv);
      vec2 pos = (vUv * 2.0 - 1.0) * vec2(1.0, aspect) * (sqrt(2.0) / sqrt(1.0 + aspect * aspect)) * intensity;
      float k = 1.0 / (dot(pos, pos) + 1.0);
      gl_FragColor = vec4(c.rgb * k * k, c.a);
    }`,
};

type SceneModule = (() => unknown) | GameDefinition<unknown>;

export interface PageResult {
  ok: boolean;
  error?: string;
  webgl: string;
  width: number;
  height: number;
  ev100: number;
  exposureMode: string;
  images: Record<string, string>;
  /** GPU time per frame (EXT_disjoint_timer_query_webgl2), median ms; null when the extension is unavailable. */
  gpuMs: number | null;
  /** Wall time per frame including a GPU sync (render + 1-pixel readback), median ms. */
  syncMs: number;
  /** Steady-state frame time: `frames` renders back to back, one GPU sync at the end, divided by `frames` (ms). */
  pipelinedMs: number;
  /** The same without shadow maps (how much of the frame the shadow passes cost). */
  pipelinedNoShadowMs: number;
  /** With `path`: each pose's steady-state frame time (ms). */
  pathMs?: number[];
  frames: number;
  setupMs: number;
  loadMs: number;
  compileMs: number;
  vignette: number;
  /** One-time cost of the scene-probe variant (two cube captures + PMREM). */
  probeMs?: number;
  drawCalls: number;
  triangles: number;
  programs: number;
  dropped: Record<string, number>;
  notes: string[];
  stats: Record<string, number>;
}

const TONEMAPS: [string, THREE.ToneMapping][] = [["aces", THREE.ACESFilmicToneMapping], ["agx", THREE.AgXToneMapping], ["neutral", THREE.NeutralToneMapping]];

const median = (v: number[]) => {
  const s = [...v].sort((a, b) => a - b);
  return s.length ? s[Math.floor(s.length / 2)] : NaN;
};
const nextFrame = () => new Promise<void>((r) => setTimeout(r, 0));

export async function run(mod: SceneModule): Promise<void> {
  const q = new URLSearchParams(location.search);
  const result: Partial<PageResult> = { ok: false, images: {}, dropped: {}, notes: [] };
  try {
    const canvas = document.createElement("canvas");
    document.body.style.margin = "0";
    document.body.appendChild(canvas);
    const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, preserveDrawingBuffer: true, powerPreference: "high-performance" }); // MSAA: in the HDR target
    const gl = renderer.getContext() as WebGL2RenderingContext;
    const dbg = gl.getExtension("WEBGL_debug_renderer_info");
    result.webgl = dbg ? String(gl.getParameter(dbg.UNMASKED_RENDERER_WEBGL)) : String(gl.getParameter(gl.RENDERER));
    renderer.setPixelRatio(1);
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.shadowMap.enabled = true;
    renderer.shadowMap.type = THREE.PCFShadowMap;

    const engine = new ThreeEngine(renderer);
    const game = (typeof mod === "function" ? defineGame({ state: {}, setup: () => mod() as never, handlers: {} }) : mod) as GameDefinition<unknown>;
    let t = performance.now();
    installNodeHost(game, engine).setup("local");
    result.setupMs = performance.now() - t;
    t = performance.now();
    await engine.ready();
    result.loadMs = performance.now() - t;

    const s = engine.settingsDesc;
    const width = Number(q.get("width") ?? s?.width ?? 1280), height = Number(q.get("height") ?? s?.height ?? 720);
    renderer.setSize(width, height, true);
    const exp = engine.cameraDesc?.exposure ?? { mode: "manual" as const, ev100: 0 };
    let ev100: number;
    if (exp.mode === "manual") ev100 = exp.ev100;
    else if (q.get("ev100") !== null) {
      ev100 = Number(q.get("ev100"));
      result.notes!.push(`auto exposure -> EV100 ${ev100.toFixed(3)} as settled by the native renderer (three.js has no auto exposure)`);
    } else throw new Error("auto-exposure camera: pass ?ev100= (three.js has no auto exposure)");
    result.ev100 = ev100;
    result.exposureMode = exp.mode;
    renderer.toneMappingExposure = Math.pow(2, -ev100);
    const camera = q.get("camera") ? JSON.parse(q.get("camera")!) : undefined;
    engine.prepare({ width, height, camera });

    // Post: the preset's vignette (+ the scene's overrides), then tone mapping in the output pass.
    const preset = s ? await (await fetch(`/presets/${s.preset}.json`)).json() : { post: { vignette: 0.4 } };
    const vignette = Number((s?.overrides as { post?: { vignette?: number } } | undefined)?.post?.vignette ?? preset.post?.vignette ?? 0);
    result.vignette = vignette;
    const target = new THREE.WebGLRenderTarget(width, height, { type: THREE.HalfFloatType, samples: 4 });
    const composer = new EffectComposer(renderer, target);
    composer.setPixelRatio(1);
    composer.setSize(width, height);
    composer.addPass(new RenderPass(engine.scene, engine.activeCamera));
    const vignettePass = new ShaderPass(VignetteShader);
    vignettePass.uniforms.intensity.value = vignette;
    vignettePass.uniforms.aspect.value = height / width;
    vignettePass.enabled = vignette > 0;
    composer.addPass(vignettePass);
    composer.addPass(new OutputPass());
    const draw = () => composer.render();

    t = performance.now();
    await renderer.compileAsync(engine.scene, engine.activeCamera);
    draw();
    gl.finish();
    result.compileMs = performance.now() - t;

    // A camera move (tools/milestone1.ts): every pose prepared like a still (shadow frustum, automatic lights), one
    // JPEG each, and its steady-state frame time (4 back-to-back frames, one sync) in `pathMs`.
    if (q.get("path")) {
      const poses = JSON.parse(q.get("path")!) as { position: [number, number, number]; target: [number, number, number]; fov?: number }[];
      const frames: string[] = [];
      const px = new Uint8Array(4);
      result.pathMs = [];
      for (const pose of poses) {
        engine.prepare({ width, height, camera: pose });
        draw();
        draw();
        frames.push(canvas.toDataURL("image/jpeg", 0.9));
        gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
        const t0 = performance.now();
        for (let i = 0; i < 4; i++) draw();
        gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
        result.pathMs.push(+((performance.now() - t0) / 4).toFixed(2));
        await nextFrame();
      }
      result.images!.path = JSON.stringify(frames);
      engine.prepare({ width, height, camera: poses[0] });
    }

    for (const [name, tm] of TONEMAPS) {
      renderer.toneMapping = tm;
      draw(); // program switch
      draw();
      result.images![name] = canvas.toDataURL("image/png");
      await nextFrame();
    }
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    if (vignette > 0) {
      vignettePass.enabled = false;
      draw();
      result.images!["novignette"] = canvas.toDataURL("image/png");
      vignettePass.enabled = true;
    }
    // Draw calls / triangles of a whole frame (shadow passes, scene, post), not just the last render() call.
    renderer.info.autoReset = false;
    renderer.info.reset();
    draw();
    result.drawCalls = renderer.info.render.calls;
    result.triangles = renderer.info.render.triangles;
    renderer.info.autoReset = true;
    result.programs = renderer.info.programs?.length ?? 0;

    // Frame cost: steady state (back-to-back frames, one sync), per-frame synced wall time, and GPU time from timer
    // queries when the browser exposes them (ANGLE's Metal timer queries read high; the pipelined time is the figure).
    const frames = Number(q.get("frames") ?? 120);
    const px = new Uint8Array(4);
    const pipelined = () => {
      for (let i = 0; i < 10; i++) draw();
      gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
      const t0 = performance.now();
      for (let i = 0; i < frames; i++) draw();
      gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
      return (performance.now() - t0) / frames;
    };
    result.pipelinedMs = pipelined();
    await nextFrame();
    renderer.shadowMap.enabled = false;
    result.pipelinedNoShadowMs = pipelined();
    renderer.shadowMap.enabled = true;
    draw(); // programs with shadows again
    await nextFrame();
    const sync: number[] = [];
    for (let i = 0; i < frames; i++) {
      const t0 = performance.now();
      draw();
      gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
      sync.push(performance.now() - t0);
      if (i % 16 === 15) await nextFrame();
    }
    result.syncMs = median(sync);
    const ext = gl.getExtension("EXT_disjoint_timer_query_webgl2") as { TIME_ELAPSED_EXT: number; GPU_DISJOINT_EXT: number } | null;
    result.gpuMs = null;
    if (ext) {
      const queries: WebGLQuery[] = [];
      for (let i = 0; i < frames; i++) {
        const qy = gl.createQuery()!;
        gl.beginQuery(ext.TIME_ELAPSED_EXT, qy);
        draw();
        gl.endQuery(ext.TIME_ELAPSED_EXT);
        queries.push(qy);
        if (i % 8 === 7) await nextFrame();
      }
      gl.finish();
      const ms: number[] = [];
      for (const qy of queries) {
        for (let k = 0; k < 200 && !gl.getQueryParameter(qy, gl.QUERY_RESULT_AVAILABLE); k++) await new Promise((r) => setTimeout(r, 5));
        if (!gl.getParameter(ext.GPU_DISJOINT_EXT) && gl.getQueryParameter(qy, gl.QUERY_RESULT_AVAILABLE)) ms.push(gl.getQueryParameter(qy, gl.QUERY_RESULT) / 1e6);
        gl.deleteQuery(qy);
      }
      if (ms.length) result.gpuMs = median(ms);
    }
    // Variant: scene probe as the environment (two bounces captured at the camera), ACES.
    if (q.get("probe") !== "0") {
      t = performance.now();
      engine.captureProbe(engine.activeCamera.getWorldPosition(new THREE.Vector3()));
      draw();
      gl.finish();
      result.probeMs = performance.now() - t;
      result.images!.probe = canvas.toDataURL("image/png");
    }
    result.frames = frames;
    result.width = width;
    result.height = height;
    result.dropped = engine.dropped;
    result.notes!.push(...engine.notes);
    result.stats = engine.stats();
    result.ok = true;
  } catch (e) {
    result.error = e instanceof Error ? `${e.message}\n${e.stack}` : String(e);
  }
  (window as unknown as { __result: unknown }).__result = result;
}
