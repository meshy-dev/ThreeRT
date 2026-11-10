// A grid program's sources (ROADMAP § 6 step 5b): a grid of zeros whose program adds to every cell inside a source's
// sphere what the source gives (`s.a.x += source(k).a.x`). The game emits one source of 0.6 over the whole box in
// setup. A frame is two half steps, and the source reaches the first alone, so the grid holds 0.6, not 1.2: a pure
// absorber of extinction 2/m x the grid, 1 m thick, in front of a 2-nit wall reads 2 e^-1.2 = 0.6024 nits after 6
// frames (0.1 s). (Added in both half steps, the grid holds 1.2: the extinction is clamped to the majorant, 2 e^-2 = 0.27.)
import { BoxGeometry, DensityGrid, GridProgram, Mesh, MeshStandardMaterial, VolumeMesh } from "@threert/scene";
import { CENTER, camera, result, sceneWithSky, type TestMeta } from "./lib.ts";
import { GRID_MEDIUM } from "./volume-grid-uniform.ts";

export const test: TestMeta = {
  id: "volume/grid-source", features: ["volume", "density-grid"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 2 * Math.exp(-1.2), tolerance: 0.02 },
  spp: 256,
  time: 0.1,
  note: "box 2 x 2 x 1 m at the origin, wall at z = -2, camera at z = 5",
};

const ADD = new GridProgram("test_add_source", {
  code: `
    for (var k = 0u; k < i.sources; k++) {
        let src = source(k);
        if (distance(i.p, src.p) < src.radius) { s.a.x += src.a.x; }
    }`,
});

export default function () {
  const scene = sceneWithSky(0);
  const wall = new Mesh(new BoxGeometry(8, 8, 0.1), new MeshStandardMaterial({ name: "wall", color: 0x000000, emissive: 0xffffff, emissiveIntensity: 2 }));
  wall.position.set(0, 0, -2);
  const grid = new DensityGrid({ size: [16, 16, 8], program: ADD.with() });
  const box = new VolumeMesh({ volume: GRID_MEDIUM.with({ Density: 2 }), majorant: 2, grid });
  box.scale.set(2, 2, 1);
  grid.emit([0, 0, 0, 5, 0.6, 0, 0, 0, 0, 0, 0, 0]);
  return result(scene, camera(scene, [0, 0, 5], [0, 0, 0]), [wall, box]);
}
