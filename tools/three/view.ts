/// <reference lib="dom" />
// Interactive three.js view of a scene / game module in an ordinary browser (node tools/three/open.ts <scene>): the
// same ThreeEngine mapping and post stack as the measured baseline (tools/three/page.ts: WebGLRenderer, linear HDR
// target with 4x MSAA, the preset's vignette, ACES in the output pass, toneMappingExposure = 2^-EV100), drawn every
// frame with orbit controls and an overlay: frame rate, GPU time (timer queries when the browser has them), draw calls,
// triangles, lights (the adapter's automatic emissive lights among them) and what three.js cannot express (dropped,
// notes).
//
// Mouse: drag to orbit, right-drag to pan, wheel to dolly; R: back to the scene's camera; H: hide the overlay.
// URL parameters: ev100 (auto-exposure cameras), autoLights (the adapter's cap; 0 = none), width / height (a fixed
// canvas instead of the window), check (after 120 frames publish `window.__result`: the overlay text and the frame as a
// PNG data URL, for an automated look; tools/three/browser.ts `run`).
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { ShaderPass } from "three/addons/postprocessing/ShaderPass.js";
import { defineGame, installNodeHost, type GameDefinition } from "@threert/gameplay";
import { ThreeEngine } from "./engine.ts";
import { VignetteShader } from "./page.ts";

type SceneModule = (() => unknown) | GameDefinition<unknown>;

export async function run(mod: SceneModule): Promise<void> {
  const q = new URLSearchParams(location.search);
  document.body.style.cssText = "margin:0;background:#000;overflow:hidden;font:12px/1.35 ui-monospace,Menlo,monospace;color:#ddd";
  const overlay = document.createElement("pre");
  overlay.style.cssText = "position:fixed;left:8px;top:8px;margin:0;padding:6px 8px;background:rgba(0,0,0,.6);border-radius:4px;max-width:46vw;white-space:pre-wrap;pointer-events:none";
  overlay.textContent = "loading…";
  document.body.appendChild(overlay);
  try {
    const canvas = document.createElement("canvas");
    canvas.style.display = "block";
    document.body.appendChild(canvas);
    const renderer = new THREE.WebGLRenderer({ canvas, antialias: false, powerPreference: "high-performance" });
    const gl = renderer.getContext() as WebGL2RenderingContext;
    renderer.setPixelRatio(1);
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.shadowMap.enabled = true;
    renderer.shadowMap.type = THREE.PCFShadowMap;

    const engine = new ThreeEngine(renderer);
    const game = (typeof mod === "function" ? defineGame({ state: {}, setup: () => mod() as never, handlers: {} }) : mod) as GameDefinition<unknown>;
    let t = performance.now();
    installNodeHost(game, engine).setup("local");
    await engine.ready();
    const loadMs = performance.now() - t;

    const fixed = q.get("width") !== null;
    const size = () => (fixed ? { w: Number(q.get("width")), h: Number(q.get("height") ?? 720) } : { w: innerWidth, h: innerHeight });
    let { w, h } = size();
    renderer.setSize(w, h, true);
    const exp = engine.cameraDesc?.exposure ?? { mode: "manual" as const, ev100: 0 };
    const ev100 = exp.mode === "manual" ? exp.ev100 : Number(q.get("ev100") ?? 0);
    renderer.toneMappingExposure = Math.pow(2, -ev100);
    const autoLights = q.get("autoLights") !== null ? Number(q.get("autoLights")) : undefined;
    engine.prepare({ width: w, height: h, autoLights });

    // A free camera starting at the scene's camera; orbit about a point 10 m ahead.
    const sceneCam = engine.activeCamera;
    const cam = new THREE.PerspectiveCamera(sceneCam.fov, w / h, sceneCam.near, sceneCam.far);
    const controls = new OrbitControls(cam, canvas);
    const reset = () => {
      sceneCam.updateWorldMatrix(true, false);
      cam.position.setFromMatrixPosition(sceneCam.matrixWorld);
      cam.quaternion.setFromRotationMatrix(sceneCam.matrixWorld);
      controls.target.copy(cam.position).add(new THREE.Vector3(0, 0, -10).applyQuaternion(cam.quaternion));
      controls.update();
    };
    reset();

    const s = engine.settingsDesc;
    const preset = s ? await (await fetch(`/presets/${s.preset}.json`)).json() : { post: { vignette: 0.4 } };
    const vignette = Number((s?.overrides as { post?: { vignette?: number } } | undefined)?.post?.vignette ?? preset.post?.vignette ?? 0);
    const target = new THREE.WebGLRenderTarget(w, h, { type: THREE.HalfFloatType, samples: 4 });
    const composer = new EffectComposer(renderer, target);
    composer.setPixelRatio(1);
    composer.setSize(w, h);
    composer.addPass(new RenderPass(engine.scene, cam));
    const vignettePass = new ShaderPass(VignetteShader);
    vignettePass.uniforms.intensity.value = vignette;
    vignettePass.uniforms.aspect.value = h / w;
    vignettePass.enabled = vignette > 0;
    composer.addPass(vignettePass);
    composer.addPass(new OutputPass());
    t = performance.now();
    await renderer.compileAsync(engine.scene, cam);
    const compileMs = performance.now() - t;

    addEventListener("resize", () => {
      if (fixed) return;
      ({ w, h } = size());
      renderer.setSize(w, h, true);
      composer.setSize(w, h);
      cam.aspect = w / h;
      cam.updateProjectionMatrix();
      vignettePass.uniforms.aspect.value = h / w;
    });
    addEventListener("keydown", (e) => {
      if (e.key === "r" || e.key === "R") reset();
      if (e.key === "h" || e.key === "H") overlay.style.display = overlay.style.display === "none" ? "" : "none";
    });

    // GPU time from timer queries (ANGLE / Metal reads high; the frame interval is the figure that counts).
    const ext = gl.getExtension("EXT_disjoint_timer_query_webgl2") as { TIME_ELAPSED_EXT: number; GPU_DISJOINT_EXT: number } | null;
    const pending: WebGLQuery[] = [];
    let gpuMs = NaN;
    const intervals: number[] = [];
    let last = performance.now();
    const stats = engine.stats();
    const autoCount = engine.scene.children.filter((c) => c.name.startsWith("auto-light-")).length;
    const fixedText = [
      `${s?.name ?? "scene"} · three.js r${THREE.REVISION} WebGL · ${w}×${h} · EV100 ${ev100} · ACES`,
      `load ${(loadMs / 1000).toFixed(1)} s · compile ${(compileMs / 1000).toFixed(1)} s · ${stats.meshes} meshes · ${stats.lights} lights (${autoCount} automatic)`,
    ];
    const droppedText = Object.entries(engine.dropped).map(([k, v]) => `  ${v} × ${k}`);
    const notesText = [...engine.notes].map((n) => `  ${n}`);
    let frames = 0;
    const check = q.get("check") !== null;
    const frame = () => {
      frames++;
      const now = performance.now();
      intervals.push(now - last);
      if (intervals.length > 60) intervals.shift();
      last = now;
      controls.update();
      let qy: WebGLQuery | null = null;
      if (ext && pending.length < 4) {
        qy = gl.createQuery();
        gl.beginQuery(ext.TIME_ELAPSED_EXT, qy!);
      }
      renderer.info.autoReset = false;
      renderer.info.reset();
      composer.render();
      if (qy) {
        gl.endQuery(ext!.TIME_ELAPSED_EXT);
        pending.push(qy);
      }
      while (pending.length && gl.getQueryParameter(pending[0], gl.QUERY_RESULT_AVAILABLE)) {
        const done = pending.shift()!;
        if (!gl.getParameter(ext!.GPU_DISJOINT_EXT)) gpuMs = gl.getQueryParameter(done, gl.QUERY_RESULT) / 1e6;
        gl.deleteQuery(done);
      }
      if (overlay.style.display !== "none") {
        const avg = intervals.reduce((a, b) => a + b, 0) / intervals.length;
        overlay.textContent = [
          ...fixedText,
          `${(1000 / avg).toFixed(0)} fps (${avg.toFixed(1)} ms) · GPU ${Number.isFinite(gpuMs) ? gpuMs.toFixed(1) + " ms" : "n/a"} · ${renderer.info.render.calls} draws · ${(renderer.info.render.triangles / 1e6).toFixed(2)} M triangles`,
          droppedText.length ? "dropped (three.js cannot express):" : "",
          ...droppedText,
          notesText.length ? "notes:" : "",
          ...notesText,
          "drag: orbit · right-drag: pan · wheel: dolly · R: scene camera · H: hide",
        ].filter(Boolean).join("\n");
      }
      if (check && frames === 120) (window as unknown as { __result: unknown }).__result = { ok: true, text: overlay.textContent, image: canvas.toDataURL("image/png") };
      requestAnimationFrame(frame);
    };
    requestAnimationFrame(frame);
  } catch (e) {
    overlay.textContent = `error: ${e instanceof Error ? `${e.message}\n${e.stack}` : String(e)}`;
    (window as unknown as { __result: unknown }).__result = { ok: false, error: overlay.textContent };
    throw e;
  }
}
