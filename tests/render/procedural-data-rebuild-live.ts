// Procedural data survives a rebuild: the frame script swaps the black sky colour for a black sky gradient after a few
// frames (a rebuild, and the environment CDF before the procedural data grows from 2 words to a baked map's), so the
// geometries' data is copied on the GPU to a new place (gpu.rs `upload_procs`). A box that checks its data words
// (procedural/data-bits) still emits 2 nits after it, and its record points at the moved data.
import { GeometryFunction, MeshStandardMaterial, ProceduralGeometry, ProceduralMesh, SkyColor, SkyGradient } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { BOX_IF } from "./procedural-data-bounds.ts";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/data-rebuild-live", features: ["procedural", "movable"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 2, tolerance: 0.02 },
  live: 60,
  note: "data words checked after a rebuild that moves them (environment CDF 2 words -> a baked gradient's)",
};

const WORDS = [0xffffffff, 0x7fc00001, 0x7f800001, 0xff800000, 0x80000000, 0x00000001, 0x12345678];
const BITS = new GeometryFunction("proc_rebuild_bits", {
  code: BOX_IF(WORDS.map((w, k) => `gdata(i.data + ${k}u) == 0x${w.toString(16)}u`).join(" && ")),
});

let elapsed = 0;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    const scene = sceneWithSky(0);
    const mats = [new MeshStandardMaterial({ name: "black", color: 0x000000 }), new MeshStandardMaterial({ name: "ok", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 })];
    const box = new ProceduralMesh(new ProceduralGeometry(BITS, [-1, -1, -0.1, 1, 1, 0], WORDS), mats);
    return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [box]);
  },
  frame(f) {
    elapsed += f.dt;
    if (elapsed > 3 / 60 && f.scene.environment instanceof SkyColor) f.scene.environment = new SkyGradient({ zenith: 0x000000, horizon: 0x000000, ground: 0x000000, intensity: 0 });
  },
  handlers: {},
});
