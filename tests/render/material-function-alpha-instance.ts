// A cutout whose alpha is a material function's, on a mesh of its own: the alpha test of a candidate hit evaluates
// the function at the point as the instance places it (pathtrace.wgsl `alpha_pass`, `mat_in_at`). A cover (Lambert
// 0.2) 100 m from the origin hides the centre of a plane (Lambert 0.5); its function keeps it where the world x is
// over 50 and cuts it elsewhere (in the mesh's own space every point has |x| < 1). Under a uniform sky of 1 nit the
// centre reads the cover, 0.2; cut away it would read the plane.
import { MaterialFunction, Mesh, PlaneGeometry } from "@threert/scene";
import { CENTER, camera, lambert, result, sceneWithSky, type TestMeta } from "./lib.ts";

export const test: TestMeta = {
  id: "material/function-alpha-instance", features: ["material-function", "cutout", "instancing"], reference: "analytic", temporal: false,
  metric: { kind: "radiance", region: CENTER, expected: 0.2, tolerance: 0.02 },
  note: "a cover kept by its function's alpha where world x > 50, seen straight on under a 1 nit sky: 0.2",
};

const FAR_KEPT = new MaterialFunction("far_kept", {
  params: {},
  code: `
    o.opacity = select(0.0, 1.0, i.wp.x > 50.0);`,
});

export default () => {
  const scene = sceneWithSky(1);
  const back = new Mesh(new PlaneGeometry(4, 4), lambert(0.5, "plane"));
  back.position.x = 100;
  const material = lambert(0.2, "cover");
  material.alphaTest = 0.5;
  material.shader = FAR_KEPT.with({});
  const cover = new Mesh(new PlaneGeometry(1.5, 1.5), material);
  cover.name = "cover";
  cover.dynamicGeometry = true;
  cover.position.set(100, 0, 1);
  return result(scene, camera(scene, [100, 0, 6], [100, 0, 0]), [back, cover]);
};
