/// <reference lib="dom" />
// Browser entry of the hello-world comparison (docs/COMPARE.md, part 2): runs one three.js hello world
// (compare/hello/three/<id>.js) the way its README would: a WebGLRenderer with antialiasing, one render() call.
// ?tier=polish adds compare/hello/three/polish.js (tone mapping, shadows, environment); ?env=0 skips the environment,
// ?exposure= is the tone mapping exposure picked by eye for the scene.
import * as THREE from "three";
import { polish } from "../../compare/hello/three/polish.js";

type Hello = () => { scene: THREE.Scene; camera: THREE.PerspectiveCamera } | Promise<{ scene: THREE.Scene; camera: THREE.PerspectiveCamera }>;

export async function run(hello: Hello): Promise<void> {
  const q = new URLSearchParams(location.search);
  const result: Record<string, unknown> = { ok: false };
  try {
    // The hello world's own three lines (counted in its size): renderer, size, canvas.
    const renderer = new THREE.WebGLRenderer({ antialias: true, preserveDrawingBuffer: true });
    renderer.setSize(1280, 720);
    document.body.appendChild(renderer.domElement);
    const { scene, camera } = await hello();
    if (q.get("tier") === "polish") polish(renderer, scene, { environment: q.get("env") !== "0", exposure: Number(q.get("exposure") ?? 1) });
    renderer.render(scene, camera);
    result.image = renderer.domElement.toDataURL("image/png");
    const t0 = performance.now();
    for (let i = 0; i < 60; i++) renderer.render(scene, camera);
    renderer.getContext().readPixels(0, 0, 1, 1, 6408, 5121, new Uint8Array(4));
    result.frameMs = (performance.now() - t0) / 60;
    result.ok = true;
  } catch (e) {
    result.error = e instanceof Error ? `${e.message}\n${e.stack}` : String(e);
  }
  (window as unknown as { __result: unknown }).__result = result;
}
