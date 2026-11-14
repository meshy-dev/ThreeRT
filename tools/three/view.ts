/// <reference lib="dom" />
// Interactive three.js view of a scene / game module in an ordinary browser (node tools/three/open.ts <scene>): the
// same ThreeEngine mapping and post stack as the measured baseline (tools/three/page.ts: WebGLRenderer, linear HDR
// target with 4x MSAA, the preset's vignette, ACES in the output pass, toneMappingExposure = 2^-EV100), drawn every
// frame with orbit controls and an overlay: frame rate, GPU time (timer queries when the browser has them), draw calls,
// triangles, lights (the adapter's automatic emissive lights among them) and what three.js cannot express (dropped,
// notes).
//
// Mouse: drag to orbit, right-drag to pan, wheel to dolly; R: back to the scene's camera; H: hide the overlay.
//
// A game is played, not only looked at: its frame script runs every frame, its `keys` run their handlers, a click is
// its `click` event (the object under it, the view ray), and the page is the app of its host channel
// (docs/RUNTIME.md "The host channel"; tools/three/host.ts, hud.ts, audio.ts): `net.*` on the browser's WebSocket and
// fetch, `audio.*` on Web Audio by the native mixer's rules, `launch` from the page's query (`?room=K7Q4MZ&name=Ada`),
// the HUD drawn as DOM over the canvas. A game that reads input (`pointer` in its definition) has the keys, the pointer
// and the camera to itself: no orbit controls, and with `pointer: "look"` a click takes the pointer lock (Esc lets go).
// With `physics=<the module's path>` the page loads the engine's own physics (tools/three/physics.ts;
// tools/three/open.ts builds it and adds the parameter), so bodies move and the physics queries answer as in the
// renderer.
//
// URL parameters: ev100 (auto-exposure cameras), autoLights (the adapter's cap; 0 = none), width / height (a fixed
// canvas instead of the window), check (after 120 frames publish `window.__result`: the overlay text and the frame as a
// PNG data URL, for an automated look; tools/three/browser.ts `run`), play=0 (a game shown as its setup left it, as
// before games were played here), stats=1 (the overlay of figures, for a game that reads input), sensitivity (of the
// look), physics. Every other parameter is the game's (`launch`).
import * as THREE from "three";
import { OrbitControls } from "three/addons/controls/OrbitControls.js";
import { EffectComposer } from "three/addons/postprocessing/EffectComposer.js";
import { OutputPass } from "three/addons/postprocessing/OutputPass.js";
import { RenderPass } from "three/addons/postprocessing/RenderPass.js";
import { ShaderPass } from "three/addons/postprocessing/ShaderPass.js";
import { defineGame, installNodeHost, type GameDefinition, type HostMessage, type Hud } from "@threert/gameplay";
import { ThreeEngine } from "./engine.ts";
import { PageAudio } from "./audio.ts";
import { launchMessage, PageInput, PageNet } from "./host.ts";
import { HudOverlay } from "./hud.ts";
import { VignetteShader } from "./page.ts";
import { loadScenePhysics } from "./physics.ts";

/** The page's own URL parameters: the rest are the game's (`launch`). */
const OWN = ["ev100", "autoLights", "width", "height", "check", "play", "stats", "sensitivity", "physics"];

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
    // The page is the app of the game's host channel: what the game posts waits here until `flush`.
    const posts: HostMessage[] = [];
    (globalThis as { __host_post?: (text: string) => void }).__host_post = (text) => void posts.push(JSON.parse(text) as HostMessage);
    const net = new PageNet();
    const sound = new PageAudio();
    let hud: HudOverlay | null = null;
    const flush = () => {
      for (const m of posts.splice(0)) {
        if (m.t === "hud") (hud ??= new HudOverlay()).set((m.hud ?? {}) as Hud);
        else if (!sound.post(m)) net.post(m);
      }
    };
    // The engine's physics, when the page was opened with it.
    if (q.get("physics")) engine.bodies = await loadScenePhysics(await (await fetch(q.get("physics")!)).arrayBuffer());
    let t = performance.now();
    const node = installNodeHost(game, engine);
    const about = JSON.parse(node.describe()) as { handlers: string[]; on: string[]; frame: boolean; keys: Record<string, string>; pointer: "look" | "free" | null };
    node.setup("local");
    await engine.ready();
    const loadMs = performance.now() - t;
    // A game is played (its frame script, its events, its channel); a scene module, or `play=0`, is looked at.
    const play = typeof mod !== "function" && q.get("play") !== "0";
    // A game that reads input has the keys, the pointer and the camera to itself.
    const input = play && about.pointer ? new PageInput(canvas, about.pointer === "look", Number(q.get("sensitivity") ?? 1) || 1) : null;
    if (input && q.get("stats") !== "1") overlay.style.display = "none";
    if (play) {
      node.hostMessages(JSON.stringify([launchMessage(q, OWN)]), Date.now());
      flush();
    }

    const fixed = q.get("width") !== null;
    const size = () => (fixed ? { w: Number(q.get("width")), h: Number(q.get("height") ?? 720) } : { w: innerWidth, h: innerHeight });
    let { w, h } = size();
    renderer.setSize(w, h, true);
    const exp = engine.cameraDesc?.exposure ?? { mode: "manual" as const, ev100: 0 };
    const ev100 = exp.mode === "manual" ? exp.ev100 : Number(q.get("ev100") ?? 0);
    renderer.toneMappingExposure = Math.pow(2, -ev100);
    const autoLights = q.get("autoLights") !== null ? Number(q.get("autoLights")) : undefined;
    engine.prepare({ width: w, height: h, autoLights });

    // A free camera starting at the scene's camera; orbit about a point 10 m ahead. (A game that reads input is seen
    // through its own camera instead: `view` below.)
    const sceneCam = engine.activeCamera;
    const cam = new THREE.PerspectiveCamera(sceneCam.fov, w / h, sceneCam.near, sceneCam.far);
    const controls = new OrbitControls(cam, canvas);
    controls.enabled = !input;
    const view = () => (input ? engine.activeCamera : cam);
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
    const scenePass = new RenderPass(engine.scene, view());
    composer.addPass(scenePass);
    const vignettePass = new ShaderPass(VignetteShader);
    vignettePass.uniforms.intensity.value = vignette;
    vignettePass.uniforms.aspect.value = h / w;
    vignettePass.enabled = vignette > 0;
    composer.addPass(vignettePass);
    composer.addPass(new OutputPass());
    t = performance.now();
    await renderer.compileAsync(engine.scene, view());
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
    /** An event of the game on this page (local play: the page is the authority), then what it posted. */
    const event = (type: string, payload: object) => {
      try {
        node.event(type, JSON.stringify(payload), Date.now());
      } catch (e) {
        console.error(`${type}: ${e instanceof Error ? e.message : String(e)}`);
      }
      flush();
    };
    addEventListener("keydown", (e) => {
      // The game's `keys` first, named as the native window names them ("R", "Space", "Escape").
      const name = e.key === " " ? "Space" : e.key.length === 1 ? e.key.toUpperCase() : e.key;
      if (play && !e.repeat && about.keys[name]) return event(about.keys[name], {});
      if (input) return; // the other keys are the game's input
      if (e.key === "r" || e.key === "R") reset();
      if (e.key === "h" || e.key === "H") overlay.style.display = overlay.style.display === "none" ? "" : "none";
    });
    // A click (not a drag) is the game's `click` event: the object under it and the view ray, as the renderer sends
    // them. Not while the pointer is the game's look: there is no place it points at.
    if (play && about.pointer !== "look" && (about.handlers.includes("click") || about.on.includes("click"))) {
      let down: { x: number; y: number } | null = null;
      canvas.addEventListener("pointerdown", (e) => void (down = e.button === 0 ? { x: e.clientX, y: e.clientY } : null));
      canvas.addEventListener("pointerup", (e) => {
        if (!down || Math.hypot(e.clientX - down.x, e.clientY - down.y) > 4) return;
        const r = canvas.getBoundingClientRect();
        const ray = new THREE.Raycaster();
        ray.setFromCamera(new THREE.Vector2(((e.clientX - r.left) / r.width) * 2 - 1, 1 - ((e.clientY - r.top) / r.height) * 2), view());
        const hit = ray.intersectObjects(engine.scene.children, true).find((i) => (i.object as THREE.Mesh).isMesh && !i.object.name.startsWith("auto-light-"));
        if (!hit) return;
        // (The object's holder carries the name the scene gave it.)
        let named: THREE.Object3D | null = hit.object;
        while (named && !named.name) named = named.parent;
        event("click", { target: named?.name ?? "", point: hit.point.toArray(), player: "you", ray: { origin: ray.ray.origin.toArray(), direction: ray.ray.direction.toArray() } });
      });
    }

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
    let failed = "";
    const check = q.get("check") !== null;
    const frame = () => {
      frames++;
      const now = performance.now();
      intervals.push(now - last);
      if (intervals.length > 60) intervals.shift();
      const dt = Math.min((now - last) / 1000, 0.1);
      last = now;
      if (play) {
        // As the renderer's frame: the app's messages (this frame's input, what the sockets received), the frame
        // script, the physics' ticks, then what the game posted.
        try {
          const messages = [...(input ? [input.message()] : []), ...net.poll()];
          if (messages.length) node.hostMessages(JSON.stringify(messages), Date.now());
          node.frame(dt, Date.now());
        } catch (e) {
          // (A frame script that throws does not stop the world; its error is shown once.)
          const text = e instanceof Error ? e.message : String(e);
          if (text !== failed) console.error(`frame script: ${(failed = text)}`);
        }
        engine.step(dt);
        flush();
        // The game's camera, whichever it is now, at the canvas's shape.
        const c = view();
        if (c !== scenePass.camera || (input && c.aspect !== w / h)) {
          scenePass.camera = c;
          c.aspect = w / h;
          c.updateProjectionMatrix();
        }
      }
      controls.update();
      // The ears are the camera the frame is drawn from.
      {
        const c = scenePass.camera as THREE.Camera;
        const p = c.getWorldPosition(new THREE.Vector3()), f = c.getWorldDirection(new THREE.Vector3());
        const r = new THREE.Vector3().crossVectors(f, c.up).normalize();
        sound.setListener([p.x, p.y, p.z], [f.x, f.y, f.z], [r.x, r.y, r.z]);
      }
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
