// Analytic procedural geometry: spheres and boxes as geometry functions (ProceduralGeometry), each one box of its own.
// They are exact where a triangle mesh is not (a sphere's silhouette and normals), which makes them the reference shapes
// of the procedural geometry contract's tests, and an example of a geometry function reading the game's data.
import { GeometryFunction, ProceduralGeometry } from "./core.ts";

/** Spheres: per box, data (centre xyz, radius as f32 bits, material index). */
export const ANALYTIC_SPHERE = new GeometryFunction("analytic_sphere", {
  code: `
    let k = i.data + i.prim * 5u;
    let c = vec3<f32>(gdataf(k), gdataf(k + 1u), gdataf(k + 2u));
    let r = gdataf(k + 3u);
    let oc = i.o - c;
    let a = dot(i.d, i.d);
    let b = dot(oc, i.d);
    let disc = b * b - a * (dot(oc, oc) - r * r);
    if (disc >= 0.0) {
        let q = sqrt(disc);
        var t = (-b - q) / a;
        if (t <= i.tmin) { t = (-b + q) / a; }
        if (t > i.tmin && t < i.tmax) {
            o.t = t;
            o.normal = i.o + i.d * t - c;
            o.material = gdata(k + 4u);
        }
    }`,
});

/** Boxes: each box is solid; per box, data (material index). */
export const ANALYTIC_BOX = new GeometryFunction("analytic_box", {
  code: `
    let inv = 1.0 / i.d;
    let t0 = (i.lo - i.o) * inv;
    let t1 = (i.hi - i.o) * inv;
    let tn = min(t0, t1);
    let tf = max(t0, t1);
    let a = max(max(tn.x, tn.y), tn.z);
    let b = min(min(tf.x, tf.y), tf.z);
    // From outside the entry face, from inside the exit face.
    let enter = a > i.tmin;
    let t = select(b, a, enter);
    if (a <= b && t > i.tmin && t < i.tmax) {
        o.t = t;
        let f = select(tf, tn, enter);
        let axis = select(select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 1.0, 0.0), f.y == t), vec3<f32>(1.0, 0.0, 0.0), f.x == t);
        o.normal = axis * select(sign(i.d), -sign(i.d), enter);
        o.material = gdata(i.data + i.prim);
    }`,
});

const bits = (v: number) => new Uint32Array(new Float32Array([v]).buffer)[0];

/** Spheres (centre xyz, radius; material index into the mesh's materials, default 0) as procedural geometry. */
export function proceduralSpheres(spheres: { center: [number, number, number]; radius: number; material?: number }[]): ProceduralGeometry {
  const boxes = spheres.flatMap(({ center: [x, y, z], radius: r }) => [x - r, y - r, z - r, x + r, y + r, z + r]);
  const data = spheres.flatMap(({ center: [x, y, z], radius, material = 0 }) => [bits(x), bits(y), bits(z), bits(radius), material]);
  return new ProceduralGeometry(ANALYTIC_SPHERE, boxes, data);
}

/** Axis-aligned boxes (min xyz, max xyz; material index, default 0) as procedural geometry. */
export function proceduralBoxes(list: { min: [number, number, number]; max: [number, number, number]; material?: number }[]): ProceduralGeometry {
  return new ProceduralGeometry(ANALYTIC_BOX, list.flatMap((b) => [...b.min, ...b.max]), list.map((b) => b.material ?? 0));
}
