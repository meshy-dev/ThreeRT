/// <reference lib="dom" />
// Browser entry of "three.js at its strongest" (docs/COMPARE.md, part 1): the same scene / game code as the native
// renderer, through the same ThreeEngine mapping as the WebGL baseline (tools/three/engine.ts), but on WebGPURenderer
// with three.js r187's own real-time GI stack, composed the way its official examples compose it
// (webgpu_postprocessing_ssgi + webgpu_postprocessing_ssr):
//
//   scene pass (MRT: colour, diffuse colour, normal, metalness / roughness, velocity; PCF soft shadow maps; PMREM IBL)
//   -> SSGI (screen-space indirect diffuse + AO, "High": 3 slices x 16 steps, temporal)
//   -> SSR (reflections on metals and dielectrics, roughness blur)
//   -> TRAA (temporal reprojection anti-aliasing, accumulates SSGI / SSR noise too)
//   -> exposure, bloom (UE-like: no threshold), the preset's vignette, ACES (OutputPass equivalent).
//
// URL parameters: ev100, camera, width / height, frames (timed frames, default 120), converge (frames rendered before
// the still, default 64), bloom (strength, default 0.04), gi (low|medium|high, default high), ssr (0 to skip),
// ssrMode (mirror: the classic metals-only mode), ssrDist / ssrQuality / ssrThickness (default 20 m / 1 / 0.1),
// probe (1: scene probe as the environment), post (0: the plain scene pass, for the cost split).
import * as THREE from "three/webgpu";
import { add, convertToTexture, diffuseColor, specularColorBlended, dot, float, metalness, mrt, normalView, output, packNormalToRGB, pass, roughness, sample, screenUV, unpackRGBToNormal, vec2, vec4, velocity } from "three/tsl";
import { ssgi } from "three/addons/tsl/display/SSGINode.js";
import { ssr } from "three/addons/tsl/display/SSRNode.js";
import { traa } from "three/addons/tsl/display/TRAANode.js";
import { bloom } from "three/addons/tsl/display/BloomNode.js";
import { SkyMesh } from "three/addons/objects/SkyMesh.js";
import { RectAreaLightTexturesLib } from "three/addons/lights/RectAreaLightTexturesLib.js";
import { defineGame, installNodeHost, type GameDefinition } from "@threert/gameplay";
import { ThreeEngine, type ThreeBackend } from "../three/engine.ts";

type SceneModule = (() => unknown) | GameDefinition<unknown>;

function webgpuBackend(renderer: THREE.WebGPURenderer): ThreeBackend {
  THREE.RectAreaLightNode.setLTC(RectAreaLightTexturesLib.init());
  const pmrem = new THREE.PMREMGenerator(renderer);
  return {
    pmrem: pmrem as unknown as ThreeBackend["pmrem"],
    maxAnisotropy: renderer.getMaxAnisotropy(),
    makeSky(p) {
      // SkyMesh: the TSL port of the same Preetham `Sky` addon; scaled to nits like the WebGL backend.
      const s = new SkyMesh();
      s.sunPosition.value.copy(p.toSun);
      s.cloudCoverage.value = 0;
      s.mieDirectionalG.value = p.mieAnisotropy;
      s.rayleigh.value = p.rayleigh;
      s.mieCoefficient.value = p.mieCoefficient;
      s.showSunDisc.value = p.forEnv ? 0 : 1;
      const m = s.material as THREE.NodeMaterial;
      m.colorNode = vec4((m.colorNode as any).xyz.mul(p.nits), 1);
      return s as unknown as THREE.Mesh;
    },
  };
}

const median = (v: number[]) => {
  const s = [...v].sort((a, b) => a - b);
  return s.length ? s[Math.floor(s.length / 2)] : NaN;
};

export async function run(mod: SceneModule): Promise<void> {
  const q = new URLSearchParams(location.search);
  const result: Record<string, any> = { ok: false, images: {}, notes: [] };
  try {
    const canvas = document.createElement("canvas");
    document.body.appendChild(canvas);
    const renderer = new THREE.WebGPURenderer({ canvas, antialias: false, trackTimestamp: true, powerPreference: "high-performance", requiredLimits: { maxColorAttachmentBytesPerSample: 64 } } as ConstructorParameters<typeof THREE.WebGPURenderer>[0]);
    await renderer.init();
    if (!(renderer.backend as { isWebGPUBackend?: boolean }).isWebGPUBackend) throw new Error("WebGPU unavailable (fell back to WebGL 2)");
    const device = (renderer.backend as unknown as { device: { queue: { onSubmittedWorkDone(): Promise<void> } } }).device;
    result.adapter = "WebGPU";
    renderer.setPixelRatio(1);
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.shadowMap.enabled = true;
    renderer.shadowMap.type = THREE.PCFSoftShadowMap;

    const engine = new ThreeEngine(webgpuBackend(renderer));
    const game = (typeof mod === "function" ? defineGame({ state: {}, setup: () => mod() as never, handlers: {} }) : mod) as GameDefinition<unknown>;
    let t = performance.now();
    installNodeHost(game, engine).setup("local");
    await engine.ready();
    result.loadMs = performance.now() - t;

    const s = engine.settingsDesc;
    const width = Number(q.get("width") ?? s?.width ?? 1280), height = Number(q.get("height") ?? s?.height ?? 720);
    renderer.setSize(width, height, true);
    const exp = engine.cameraDesc?.exposure ?? { mode: "manual" as const, ev100: 0 };
    let ev100: number;
    if (exp.mode === "manual") ev100 = exp.ev100;
    else if (q.get("ev100") !== null) ev100 = Number(q.get("ev100"));
    else throw new Error("auto-exposure camera: pass ?ev100= (three.js has no auto exposure)");
    result.ev100 = ev100;
    const camera = q.get("camera") ? JSON.parse(q.get("camera")!) : undefined;
    engine.prepare({ width, height, camera });
    const cam = engine.activeCamera;
    // Scene probe (the WebGL baseline's variant): the lit scene captured at the camera (two bounces) as the
    // environment, so the sky light no longer reaches inside a room unoccluded. A developer switches it on for
    // interiors; the comparison keeps whichever of with / without is closer to the reference per scene.
    if (q.get("probe") === "1") {
      renderer.render(engine.scene, cam); // shadow maps, pipelines
      engine.captureProbe(cam.getWorldPosition(new THREE.Vector3()));
      result.notes.push("scene probe");
    }
    const preset = s ? await (await fetch(`/presets/${s.preset}.json`)).json() : { post: { vignette: 0.4 } };
    const vignette = Number((s?.overrides as { post?: { vignette?: number } } | undefined)?.post?.vignette ?? preset.post?.vignette ?? 0);

    // ------------------------------------------------------------------ the post stack (official r187 composition)
    const giQ = ({ low: [1, 12], medium: [2, 8], high: [3, 16] } as Record<string, [number, number]>)[q.get("gi") ?? "high"];
    const usePost = q.get("post") !== "0";
    const pipeline = new THREE.RenderPipeline(renderer);
    const scenePass = pass(engine.scene, cam);
    // Metalness / roughness ride in the alpha of the diffuse / normal targets (WebGPU's default limit is 32 bytes of
    // colour attachments per sample).
    // F0 (specularColorBlended) feeds the stochastic SSR's GGX weights (28 bytes in all).
    scenePass.setMRT(mrt({ output, diffuseColor: vec4(diffuseColor.rgb, metalness), normal: vec4(packNormalToRGB(normalView), roughness), specular: vec4(specularColorBlended, 1), velocity }));
    const color = scenePass.getTextureNode("output");
    const depth = scenePass.getTextureNode("depth");
    const normalTex = scenePass.getTextureNode("normal");
    const diffuse = scenePass.getTextureNode("diffuseColor");
    const vel = scenePass.getTextureNode("velocity");
    scenePass.getTexture("diffuseColor").type = THREE.UnsignedByteType;
    scenePass.getTexture("normal").type = THREE.UnsignedByteType;
    scenePass.getTexture("specular").type = THREE.UnsignedByteType;
    const specular = scenePass.getTextureNode("specular");
    const sceneNormal = sample((uv) => unpackRGBToNormal(normalTex.sample(uv).rgb));

    const gi = ssgi(color, depth, sceneNormal, cam);
    gi.sliceCount.value = giQ[0];
    gi.stepCount.value = giQ[1];
    const composite = vec4(add(color.rgb.mul(gi.getAONode()), diffuse.rgb.mul(gi.getGINode().rgb)), 1);
    let lit = composite;
    let debugNode: ReturnType<typeof vec4> | null = null;
    if (q.get("debug") === "rough") debugNode = vec4(normalTex.a, normalTex.a, normalTex.a, 1);
    if (q.get("debug") === "metal") debugNode = vec4(diffuse.a, diffuse.a, diffuse.a, 1);
    if (q.get("ssr") !== "0") {
      // Stochastic (GGX-sampled, F0-weighted) is the mode that reflects on dielectrics too: the classic mode weights
      // every hit by metalness. TRAA below accumulates its noise.
      const stochastic = q.get("ssrMode") !== "mirror";
      const refl = ssr(convertToTexture(composite), depth, sceneNormal, { stochastic, metalnessNode: diffuse.a, roughnessNode: normalTex.a, specularNode: specular.rgb, reflectNonMetals: true, camera: cam });
      // Scene scale: r187's defaults (1 unit ray length, quality 0.5) fit its example's 1 m model, not a street.
      refl.maxDistance.value = Number(q.get("ssrDist") ?? 20);
      refl.quality.value = Number(q.get("ssrQuality") ?? 1);
      refl.thickness.value = Number(q.get("ssrThickness") ?? 0.1);
      lit = vec4(composite.rgb.add(refl.rgb), 1);
      if (q.get("debug") === "ssr") debugNode = vec4(refl.rgb.mul(Math.pow(2, -(Number(q.get("ev100") ?? (engine.cameraDesc?.exposure as { ev100?: number })?.ev100 ?? 0)))), 1);
    }
    const aa = traa(lit, depth, vel, cam);
    // Exposure here (not in the output transform) so the bloom works on exposed values like UE's.
    const exposure = Math.pow(2, -ev100);
    renderer.toneMappingExposure = 1;
    const exposed = vec4(aa.rgb.mul(exposure), 1);
    const bloomStrength = Number(q.get("bloom") ?? 0.04);
    let graded = exposed;
    if (bloomStrength > 0) graded = vec4(exposed.rgb.add(bloom(exposed, bloomStrength, 0.6, 0).rgb), 1);
    const aspect = height / width;
    const pos = screenUV.mul(2).sub(1).mul(vec2(1, aspect)).mul((Math.SQRT2 / Math.sqrt(1 + aspect * aspect)) * vignette);
    const k = float(1).div(dot(pos, pos).add(1));
    pipeline.outputNode = vignette > 0 ? vec4(graded.rgb.mul(k.mul(k)), 1) : graded;
    if (debugNode) {
      pipeline.outputNode = debugNode;
      renderer.toneMapping = THREE.NoToneMapping;
    }
    if (!usePost) {
      // Cost split / plain variant: the scene pass alone (shadows, IBL), exposure + vignette + ACES.
      pipeline.outputNode = vec4(color.rgb.mul(exposure).mul(vignette > 0 ? k.mul(k) : float(1)), 1);
    }
    result.chain = usePost ? `SSGI ${giQ[0]}x${giQ[1]}${q.get("ssr") !== "0" ? ` + SSR${q.get("ssrMode") === "mirror" ? " (mirror)" : ""}` : ""}${q.get("probe") === "1" ? " + scene probe" : ""} + TRAA + bloom ${bloomStrength} + vignette ${vignette} + ACES` : "scene pass + ACES";

    let sceneRenders = 0;
    engine.scene.onBeforeRender = () => void sceneRenders++;
    // One draw = one frame: r187 advances the node frame (which gates the per-frame passes: scene pass, SSGI, SSR,
    // TRAA) only in its rAF loop, so do what that loop does before each render.
    const nodeFrame = (renderer as unknown as { _nodes: { nodeFrame: { update(): void } } })._nodes.nodeFrame;
    const draw = () => {
      if (renderer.info.autoReset) renderer.info.reset();
      nodeFrame.update();
      pipeline.render();
    };
    t = performance.now();
    draw();
    await device.queue.onSubmittedWorkDone();
    result.compileMs = performance.now() - t;

    // Converge the temporal effects on the still camera, then capture (toDataURL in the task that rendered).
    const converge = Number(q.get("converge") ?? 64);
    for (let i = 0; i < converge; i++) {
      draw();
      if (i % 8 === 7) await device.queue.onSubmittedWorkDone();
    }
    draw();
    result.images.still = canvas.toDataURL("image/png");
    await device.queue.onSubmittedWorkDone();

    // Frame cost: back-to-back frames, one GPU sync at the end (the steady state), and GPU timestamps per frame.
    const frames = Number(q.get("frames") ?? 120);
    for (let i = 0; i < 10; i++) draw();
    await device.queue.onSubmittedWorkDone();
    const t0 = performance.now();
    for (let i = 0; i < frames; i++) draw();
    await device.queue.onSubmittedWorkDone();
    result.pipelinedMs = (performance.now() - t0) / frames;
    const ts: number[] = [];
    for (let i = 0; i < Math.min(frames, 60); i++) {
      draw();
      await renderer.resolveTimestampsAsync(THREE.TimestampQuery.RENDER);
      await renderer.resolveTimestampsAsync(THREE.TimestampQuery.COMPUTE);
      ts.push((renderer.info.render.timestamp ?? 0) + (renderer.info.compute.timestamp ?? 0));
    }
    result.timestampMs = median(ts);
    // Per-frame work actually issued, and a frame-by-frame synced wall time (cross-check of the pipelined figure).
    renderer.info.autoReset = false;
    renderer.info.reset();
    draw();
    result.drawCalls = renderer.info.render.drawCalls;
    result.triangles = renderer.info.render.triangles;
    renderer.info.autoReset = true;
    const sync: number[] = [];
    sceneRenders = 0;
    for (let i = 0; i < 30; i++) {
      const t1 = performance.now();
      draw();
      await device.queue.onSubmittedWorkDone();
      sync.push(performance.now() - t1);
    }
    sceneRenders /= 30;
    result.syncMs = median(sync);
    result.sceneRendersPerFrame = sceneRenders;
    result.width = width;
    result.height = height;
    result.vignette = vignette;
    result.dropped = engine.dropped;
    result.notes.push(...engine.notes);
    result.stats = engine.stats();
    result.ok = true;
  } catch (e) {
    result.error = e instanceof Error ? `${e.message}\n${e.stack}` : String(e);
  }
  (window as unknown as { __result: unknown }).__result = result;
}
