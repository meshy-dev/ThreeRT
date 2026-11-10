// A geometry function cannot read past its own data: reads before it (the renderer's header and materials), just past
// it, far past it and at the largest index all give 0, while its own word reads back. A box emits 2 nits (material 1)
// only when every read is as expected, else it is black (material 0). Black sky, seen straight on.
import { GeometryFunction, MeshStandardMaterial, ProceduralGeometry, ProceduralMesh } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "procedural/data-bounds", features: ["procedural", "emission"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2, tolerance: 0.01 },
  note: "gdata outside i.data .. i.data + i.count - 1 reads 0; inside, the word itself",
};

/** The box itself (slab test), material 1 when the data reads are as expected. */
export const BOX_IF = (check: string) => `
    let inv = 1.0 / i.d;
    let t0 = (i.lo - i.o) * inv;
    let t1 = (i.hi - i.o) * inv;
    let a = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
    let b = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    if (a <= b && a > i.tmin && a < i.tmax) {
        o.t = a;
        o.normal = vec3<f32>(0.0, 0.0, 1.0);
        o.material = select(0u, 1u, ${check});
    }`;

const READS = new GeometryFunction("proc_data_bounds", {
  code: BOX_IF(`i.count == 2u && gdata(i.data) == 0x12345678u && gdata(i.data + 1u) == 0x9abcdef0u
        && gdata(i.data - 1u) == 0u && gdata(i.data - 12u) == 0u && gdata(i.data + 2u) == 0u
        && gdata(i.data + 100000000u) == 0u && gdata(0xffffffffu) == 0u && gdata(0u) == 0u && gdataf(i.data + 2u) == 0.0`),
});

export default function () {
  const scene = sceneWithSky(0);
  const mats = [new MeshStandardMaterial({ name: "black", color: 0x000000 }), new MeshStandardMaterial({ name: "ok", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 })];
  const box = new ProceduralMesh(new ProceduralGeometry(READS, [-1, -1, -0.1, 1, 1, 0], [0x12345678, 0x9abcdef0]), mats);
  return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [box]);
}
