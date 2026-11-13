// Geometry edits applied in place (live/edits.rs `edit_geometry`; ROADMAP § 6 step 2c-4): after a few frames the frame script
// removes a procedural box that hid the centre, swaps a `dynamicGeometry` mesh's tiny plane for a 4 m one across the
// centre, and adds a procedural box and a `dynamicGeometry` mesh behind it (with the materials already in the scene: a
// new material rebuilds the scene). The centre then shows the swapped plane:
// Lambert, albedo 0.5, facing a uniform sky of 1 nit with nothing in its hemisphere, so 0.5. The same frames with
// THREERT_EDIT_REBUILD=1 (every edit rebuilds the scene) are byte-identical (the PR's A/B).
import { BoxGeometry, Mesh, PlaneGeometry, ProceduralMesh, proceduralBoxes } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "geometry/edit-live", features: ["procedural", "instancing", "edits"], reference: "analytic", temporal: true,
  metric: { kind: "radiance", region: CENTER, expected: 0.5, tolerance: 0.02 },
  live: 60,
  note: "a box removed, a mesh's geometry swapped, a box and a mesh added, all in place: a Lambert 0.5 plane under a 1 nit sky",
};

let elapsed = 0;
let edited = false;
export default defineGame<Record<string, never>>({
  state: {},
  setup: () => {
    elapsed = 0;
    edited = false;
    const scene = sceneWithSky(1);
    const cover = new ProceduralMesh(proceduralBoxes([{ min: [-1, -1, 1], max: [1, 1, 1.2] }]), lambert(0.2, "cover"));
    cover.name = "cover";
    const swap = new Mesh(new PlaneGeometry(0.05, 0.05), lambert(0.5, "swap"));
    swap.name = "swap";
    swap.dynamicGeometry = true;
    return result(scene, camera(scene, [0, 0, 6], [0, 0, 0]), [cover, swap]);
  },
  frame(f) {
    elapsed += f.dt;
    if (edited || elapsed <= 3 / 60) return;
    edited = true;
    const cover = f.scene.getObjectByName("cover") as ProceduralMesh;
    const swap = f.scene.getObjectByName("swap") as Mesh;
    cover.parent!.remove(cover);
    swap.geometry = new PlaneGeometry(4, 4);
    const box = new ProceduralMesh(proceduralBoxes([{ min: [-0.5, -0.5, -0.5], max: [0.5, 0.5, 0.5] }]), cover.materials);
    box.position.set(2, 0, -3);
    const mesh = new Mesh(new BoxGeometry(1, 1, 1), swap.material);
    mesh.dynamicGeometry = true;
    mesh.position.set(-2, 0, -3);
    f.scene.add(box, mesh);
  },
  handlers: {},
});
