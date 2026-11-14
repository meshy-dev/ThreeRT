// The engine binding: what a DSL scene sends to the engine on the first sync, and what later syncs send.
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  Scene, PerspectiveCamera, Mesh, BoxGeometry, SphereGeometry, MeshStandardMaterial, DirectionalLight, SpotLight, PointLight, Color, Vector3, Quaternion,
  checkerTexture, Group, Texture, SkyGradient, ExponentialHeightFog, facadeWindows, MaterialFunction, RecordingEngine, SceneBinding, SurfaceMaterial,
  ProceduralMesh, ProceduralGeometry, ANALYTIC_BOX, proceduralBoxes, proceduralSpheres,
  GpuParticles, ParticleProgram,
  DensityGrid, GridProgram, VolumeFunction, VolumeMesh, SimulatedSmoke,
  type MaterialDesc, type ObjectDesc,
} from "../src/index.ts";

function bind(scene: Scene, cam: PerspectiveCamera) {
  const engine = new RecordingEngine();
  const b = new SceneBinding(engine, scene, cam);
  b.sync();
  return { engine, b, calls: engine.take() };
}
const ofType = (calls: [string, ...unknown[]][], t: string) => calls.filter((c) => c[0] === t);

test("first sync creates everything once; parents precede children; shared geometry is sent once", () => {
  const scene = new Scene();
  const geo = new SphereGeometry(1, 16, 8);
  const g = new Group();
  g.name = "group";
  g.position.set(1, 0, 0);
  scene.add(g);
  for (let i = 0; i < 3; i++) {
    const m = new Mesh(geo, new MeshStandardMaterial({ color: 0xff0000, map: checkerTexture() }));
    m.position.x = i;
    g.add(m);
  }
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { b, calls } = bind(scene, cam);
  assert.equal(ofType(calls, "geometry").length, 1);
  assert.equal(ofType(calls, "material").length, 3);
  assert.equal(ofType(calls, "texture").length, 3);
  const objs = ofType(calls, "object") as [string, number, ObjectDesc][];
  assert.deepEqual(objs.map((o) => o[2].name), ["group", "mesh_1", "mesh_2", "mesh_3", "perspectivecamera_1"]);
  const group = objs[0][1];
  assert.ok(objs.slice(1, 4).every((o) => o[2].parent === group));
  assert.equal(b.objectNamed("mesh_2")?.position.x, 1);
  const settings = ofType(calls, "settings")[0][1] as { camera: number };
  assert.equal(settings.camera, objs[4][1]);
});

test("later syncs send only what changed", () => {
  const scene = new Scene();
  const mat = new MeshStandardMaterial({ emissive: 0xffffff, emissiveIntensity: 10 });
  const box = new Mesh(new BoxGeometry(1, 1, 1), mat);
  box.id = "box";
  scene.add(box);
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { engine, b } = bind(scene, cam);
  assert.deepEqual(engine.take(), [], "nothing changed");
  box.power = 0.5;
  mat.emissiveIntensity = 0;
  box.position.y = 2;
  b.sync();
  const calls = engine.take();
  assert.deepEqual(calls.map((c) => c[0]).sort(), ["material", "power", "transform"]);
  assert.equal((ofType(calls, "material")[0][2] as MaterialDesc).emissiveIntensity, 0);
  // add / remove objects
  const extra = new Mesh(new BoxGeometry(1, 1, 1), mat);
  scene.add(extra);
  b.sync();
  assert.deepEqual(engine.take().map((c) => c[0]), ["geometry", "object", "transform"]);
  scene.remove(box);
  b.sync();
  assert.deepEqual(engine.take().map((c) => c[0]), ["remove"]);
  // other material changes need needsUpdate
  mat.alphaTest = 0.4;
  b.sync();
  assert.equal(engine.take().length, 0);
  mat.needsUpdate = true;
  b.sync();
  assert.equal(((engine.take()[0][2]) as MaterialDesc).alphaTest, 0.4);
});

test("dynamicGeometry meshes: the flag reaches the engine; a new geometry re-creates the object with it", () => {
  const scene = new Scene();
  const mat = new MeshStandardMaterial();
  const piece = new Mesh(new BoxGeometry(1, 1, 1), mat);
  piece.dynamicGeometry = true;
  const fixed = new Mesh(new BoxGeometry(1, 1, 1), mat);
  scene.add(piece, fixed);
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { b, engine, calls } = bind(scene, cam);
  const objects = ofType(calls, "object").map((c) => c[2] as ObjectDesc & { dynamicGeometry?: boolean });
  assert.deepEqual(objects.filter((o) => o.kind === "mesh").map((o) => o.dynamicGeometry), [true, undefined]);
  piece.geometry = new SphereGeometry(1);
  b.sync();
  const again = engine.take();
  assert.deepEqual(again.map((c) => c[0]), ["geometry", "object", "transform"], "re-created where it is");
  assert.equal((again[1][2] as { dynamicGeometry?: boolean }).dynamicGeometry, true);
});

test("objects report their own changes: a sync visits only those", () => {
  const scene = new Scene();
  const group = new Group();
  scene.add(group);
  const boxes = Array.from({ length: 50 }, (_, k) => {
    const m = new Mesh(new BoxGeometry(1, 1, 1));
    m.position.x = k;
    group.add(m);
    return m;
  });
  const lamp = new PointLight(0xffffff, 100, 10);
  group.add(lamp);
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { engine, b } = bind(scene, cam);
  // through every write path: a vector component, set(), the Euler, the quaternion, flags, mesh material
  boxes[3].position.z += 1;
  boxes[4].scale.set(2, 2, 2);
  boxes[5].rotation.y = 1;
  boxes[6].quaternion.set(0, 0, 0.6, 0.8);
  boxes[7].visible = false;
  boxes[8].castShadow = false;
  boxes[9].material = new MeshStandardMaterial({ color: 0xff0000 });
  lamp.intensity = 50;
  lamp.color.setRGB(1, 0, 0);
  const s = b.sync();
  const calls = engine.take();
  assert.equal(ofType(calls, "transform").length, 4 + 2, "4 moved + the 2 re-created");
  assert.equal(ofType(calls, "visible").length, 1);
  assert.equal(ofType(calls, "object").length, 2, "shadow flag / material switch re-create the object");
  assert.equal(ofType(calls, "light").length, 1);
  assert.equal(s.created, 0);
  assert.equal(JSON.stringify(boxes[3].position), '{"x":3,"y":0,"z":1}', "plain JSON, no back-reference");
  // unchanged values cost a comparison, not a call
  boxes[3].position.z += 0;
  b.sync();
  assert.deepEqual(engine.take(), []);
  // a moved target re-aims its light without touching the light
  const spot = new SpotLight(0xffffff, 100, 10);
  spot.position.set(0, 5, 0);
  spot.target!.position.set(0, 0, 0);
  scene.add(spot, spot.target!);
  b.sync();
  engine.take();
  spot.target!.position.set(5, 0, 0);
  b.sync();
  assert.equal(ofType(engine.take(), "transform").length, 2, "target and light");
});

test("camera and targeted lights face -Z toward their target (three.js semantics)", () => {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 1000);
  sun.position.set(0, 10, 10);
  scene.add(sun);
  const spot = new SpotLight(0xffffff, 100);
  spot.position.set(5, 5, 0);
  spot.target!.position.set(0, 0, 0);
  scene.add(spot);
  const cam = new PerspectiveCamera(40, 1, 0.1, 100);
  cam.position.set(0, 0, 5);
  cam.lookAt(0, 0, 0);
  scene.add(cam);
  const { calls } = bind(scene, cam);
  const forward = (name: string) => {
    const id = (ofType(calls, "object") as [string, number, ObjectDesc][]).find((o) => o[2].name === name)![1];
    const trs = ofType(calls, "transform").find((c) => c[1] === id)![2] as number[];
    return new Vector3(0, 0, -1).applyQuaternion(new Quaternion(trs[3], trs[4], trs[5], trs[6]));
  };
  const close = (v: Vector3, b: number[]) => [v.x, v.y, v.z].forEach((x, i) => assert.ok(Math.abs(x - b[i]) < 1e-6, `${[v.x, v.y, v.z]} vs ${b}`));
  const s = Math.SQRT1_2;
  close(forward("perspectivecamera_1"), [0, 0, -1]);
  close(forward("directionallight_1"), [0, -s, -s]);
  close(forward("spotlight_1"), [-s, -s, 0]);
});

test("colors: hex is sRGB, setRGB is linear", () => {
  const c = new Color(0x808080);
  assert.ok(Math.abs(c.r - 0.2158605) < 1e-5);
  assert.equal(new Color().setRGB(0.5, 0.5, 0.5).r, 0.5);
  assert.ok(new Vector3(1, 2, 2).length() === 3);
});

test("material + environment extensions reach the engine", () => {
  const scene = new Scene();
  const t = new Texture("assets/polyhaven/tex/asphalt_02/asphalt_02_diff_1k.jpg");
  t.offset = [0.25, 0];
  const ground = new MeshStandardMaterial({ name: "ground", map: t, alphaTest: 0.5 });
  // The former built-in fields are the library's SurfaceMaterial: a material function binding.
  const wall = new SurfaceMaterial({ name: "wall", map: t, worldScale: 4, wet: 1, puddles: { scale: 3, amount: 0.6 }, windows: facadeWindows({ seed: 3 }) });
  scene.add(new Mesh(new BoxGeometry(1, 1, 1), ground), new Mesh(new BoxGeometry(1, 1, 1), wall));
  scene.environment = new SkyGradient({ intensity: 6 }).setGlow(0xff8040, 0.3, 0.15).setClouds(0.5, 0x403030);
  scene.fog = new ExponentialHeightFog(0.01, 0.05, 0x101018).setVolumetric(0.7, 0.5);
  scene.wetness = 1;
  scene.rain = 0.6;
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { calls } = bind(scene, cam);
  const mats = Object.fromEntries((ofType(calls, "material") as [string, number, MaterialDesc][]).map((c) => [c[2].name, c[2]]));
  assert.equal(mats.ground.alphaTest, 0.5);
  assert.equal(mats.wall.map, undefined);
  assert.equal(mats.wall.shader?.id, "std_surface");
  // WorldScale, NormalScale, Wet, Flags (map | windows); PuddleScale, PuddleAmount, GrimeAmount, GrimeScale;
  // WinTop, WinSeed; UV (the map's repeat, offset).
  assert.deepEqual(mats.wall.shader?.params.slice(0, 16), [4, 1, 1, 33, 3, 0.6, 0, 1, 1e4, 3, 0, 0, 1, 1, 0.25, 0]);
  const tex = ofType(calls, "texture").find((c) => c[1] === mats.ground.map)!;
  assert.deepEqual((tex[2] as { offset: number[] }).offset, [0.25, 0]);
  const env = ofType(calls, "environment")[0][1] as { sky: { type: string }; fog: { volumetric: unknown }; wetness: number; rain: number };
  assert.equal(env.sky.type, "gradient");
  assert.deepEqual(env.fog.volumetric, { scattering: 0.7, anisotropy: 0.5 });
  assert.equal(env.rain, 0.6);
  // the environment is re-sent when it changes
  const scene2 = scene;
  scene2.rain = 0;
});

test("material functions: params packed into vec4 slots, $names substituted, textures bound by slot", () => {
  const fn = new MaterialFunction("brick", {
    params: { Rough: 0.6, Tint: [1, 0.5, 0.25, 1], Scale: 2, Wear: 0.1 },
    textures: ["Albedo", "Normal"],
    code: "    o.base = msample_color(m, i, $Albedo, i.uv * $Scale, exp2(i.uvd) * $Scale).rgb * $Tint.rgb;\n    o.roughness = $Rough + $Wear;",
  });
  const scene = new Scene();
  const a = new MeshStandardMaterial({ name: "a" });
  a.shader = fn.with({ Wear: 0.3 }, { Albedo: new Texture("assets/a.png") });
  const b = new MeshStandardMaterial({ name: "b" });
  b.shader = fn.with();
  scene.add(new Mesh(new BoxGeometry(1, 1, 1), a), new Mesh(new BoxGeometry(1, 1, 1), b));
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { calls } = bind(scene, cam);
  const shaders = ofType(calls, "shader");
  assert.deepEqual(shaders.map((s) => s[1]), ["brick"]);
  const code = shaders[0][2] as string;
  // scalars share slot 0 (Rough .x, Scale .y, Wear .z); the vec4 takes its own slot 1; textures are slot indices
  assert.match(code, /^fn mf_brick\(i: MatIn, m: Material, base: MatOut\) -> MatOut \{/);
  assert.match(code, /msample_color\(m, i, 0u, i\.uv \* m\.mp\[0\]\.y, exp2\(i\.uvd\) \* m\.mp\[0\]\.y\)\.rgb \* m\.mp\[1\]\.rgb/);
  assert.match(code, /o\.roughness = m\.mp\[0\]\.x \+ m\.mp\[0\]\.z;/);
  const mats = Object.fromEntries((ofType(calls, "material") as [string, number, MaterialDesc][]).map((c) => [c[2].name, c[2]]));
  assert.deepEqual(mats.a.shader!.params.slice(0, 8), [0.6, 2, 0.3, 0, 1, 0.5, 0.25, 1]);
  assert.equal(mats.b.shader!.params[2], 0.1);
  assert.equal(typeof mats.a.shader!.textures[0], "number");
  assert.equal(mats.a.shader!.textures[1], null);
  const bad = new MeshStandardMaterial({ name: "c" });
  bad.shader = new MaterialFunction("oops", { code: "o.base = $Missing;" }).with();
  const s2 = new Scene();
  s2.add(new Mesh(new BoxGeometry(1, 1, 1), bad));
  assert.throws(() => new SceneBinding(new RecordingEngine(), s2, cam).sync(), /unknown \$Missing/);
});

test("transmissionMap reaches the engine as a texture of the material, and a new map re-sends it", async () => {
  const { DataTexture, MeshPhysicalMaterial } = await import("../src/index.ts");
  const scene = new Scene();
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const map = new DataTexture(1, 1, new Uint8Array([255, 255, 255, 255]), "linear");
  const mat = new MeshPhysicalMaterial({ transmission: 1, transmissionMap: map });
  scene.add(new Mesh(new BoxGeometry(1, 1, 1), mat));
  const { engine, b, calls } = bind(scene, cam);
  const d = ofType(calls, "material")[0][2] as MaterialDesc;
  const tex = ofType(calls, "texture").map((c) => c[1]);
  assert.ok(d.transmissionMap !== undefined && tex.includes(d.transmissionMap), "the map is sent as a texture and named by the material");
  mat.transmissionMap = new DataTexture(1, 1, new Uint8Array([0, 0, 0, 255]), "linear");
  b.sync();
  assert.ok(ofType(engine.take(), "material").length === 1, "a new map re-sends the material");
});

test("procedural meshes: the geometry function sent once, boxes and data per geometry, materials by index; a move is a transform", () => {
  const scene = new Scene();
  const geo = proceduralSpheres([{ center: [0, 1, 0], radius: 0.5, material: 1 }, { center: [2, 1, 0], radius: 0.25 }]);
  const a = new MeshStandardMaterial({ name: "a" });
  const b = new MeshStandardMaterial({ name: "b" });
  const p = new ProceduralMesh(geo, [a, b]);
  scene.add(p);
  scene.add(new ProceduralMesh(proceduralBoxes([{ min: [0, 0, 0], max: [1, 1, 1] }]), a));
  scene.add(new ProceduralMesh(geo, b));
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { engine, b: binding, calls } = bind(scene, cam);
  assert.deepEqual(ofType(calls, "geometryFunction").map((c) => c[1]), ["analytic_sphere", "analytic_box"]);
  assert.match(ofType(calls, "geometryFunction")[0][2] as string, /^fn gf_analytic_sphere\(i: GIn\) -> GOut \{/);
  const procs = ofType(calls, "procedural") as [string, number, string, number[], number[]][];
  assert.equal(procs.length, 2);
  assert.deepEqual(procs[0][3], [-0.5, 0.5, -0.5, 0.5, 1.5, 0.5, 1.75, 0.75, -0.25, 2.25, 1.25, 0.25]);
  assert.equal(procs[0][4].length, 10);
  assert.equal(procs[0][4][4], 1);
  assert.equal(new Float32Array(new Uint32Array(procs[0][4].slice(3, 4)).buffer)[0], 0.5);
  const objs = ofType(calls, "object") as [string, number, ObjectDesc][];
  const descs = objs.map((o) => o[2]).filter((d) => d.kind === "procedural") as Extract<ObjectDesc, { kind: "procedural" }>[];
  assert.equal(descs.length, 3);
  assert.equal(descs[0].geometry, descs[2].geometry);
  assert.equal(descs[0].materials.length, 2);
  assert.deepEqual(descs[2].materials, [descs[0].materials[1]]);
  p.position.x = 3;
  binding.sync();
  const later = engine.take();
  assert.deepEqual(later.map((c) => c[0]), ["transform"]);
  geo.needsUpdate = true;
  p.position.x = 4;
  binding.sync();
  assert.deepEqual(ofType(engine.take(), "procedural").length, 1);
  assert.throws(() => new ProceduralMesh(geo, []), /at least one material/);
});

test("procedural geometry from an asset file: the engine gets its path, not boxes or data", () => {
  const scene = new Scene();
  const geo = ProceduralGeometry.fromAsset(ANALYTIC_BOX, "voxel/level/house-1.tpg");
  scene.add(new ProceduralMesh(geo, new MeshStandardMaterial()));
  scene.add(new ProceduralMesh(geo, new MeshStandardMaterial()));
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { calls } = bind(scene, cam);
  assert.deepEqual(ofType(calls, "proceduralAsset").map((c) => c.slice(2)), [["analytic_box", "voxel/level/house-1.tpg"]]);
  assert.equal(ofType(calls, "procedural").length, 0);
});

test("procedural meshes sharing a palette send each material once", () => {
  const scene = new Scene();
  const palette = [0, 1, 2, 3].map((k) => new MeshStandardMaterial({ name: `p${k}` }));
  for (let k = 0; k < 5; k++) scene.add(new ProceduralMesh(proceduralBoxes([{ min: [k, 0, 0], max: [k + 1, 1, 1] }]), palette));
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { calls } = bind(scene, cam);
  assert.equal(ofType(calls, "material").length, 4);
  const descs = (ofType(calls, "object") as [string, number, ObjectDesc][]).map((o) => o[2]).filter((d) => d.kind === "procedural") as Extract<ObjectDesc, { kind: "procedural" }>[];
  assert.ok(descs.every((d) => JSON.stringify(d.materials) === JSON.stringify(descs[0].materials)), "every mesh names the same four material ids");
});

test("generated names: the smallest free number, also after removals (searched from a floor, not from 1)", () => {
  const scene = new Scene();
  const meshes = Array.from({ length: 4 }, () => new Mesh(new BoxGeometry(1, 1, 1)));
  scene.add(...meshes);
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { b } = bind(scene, cam);
  assert.deepEqual(meshes.map((m) => b.nameOf(m)), ["mesh_1", "mesh_2", "mesh_3", "mesh_4"]);
  scene.remove(meshes[1], meshes[2]);
  b.sync();
  const late = [new Mesh(new BoxGeometry(1, 1, 1)), new Mesh(new BoxGeometry(1, 1, 1)), new Mesh(new BoxGeometry(1, 1, 1))];
  scene.add(...late);
  b.sync();
  assert.deepEqual(late.map((m) => b.nameOf(m)), ["mesh_2", "mesh_3", "mesh_5"]);
});

test("GpuParticles: the program's WGSL is sent once, the node's desc carries capacity, rate, collision; spawns go after the object, once", () => {
  const scene = new Scene();
  const prog = new ParticleProgram("dust", { params: { Speed: 2, Tint: [1, 0.5, 0.25, 1] }, emit: "s.vel = vec3<f32>(0.0, $Speed, 0.0); s.color = $Tint;", update: "s.vel.y -= 9.81 * i.dt;" });
  const mat = new MeshStandardMaterial();
  const a = new GpuParticles({ capacity: 1000, program: prog.with({ Speed: 3 }), material: mat, rate: 50, collide: { plane: 0.5, voxels: true, restitution: 0.2 } });
  const b2 = new GpuParticles({ capacity: 10, program: prog.with(), material: mat });
  scene.add(a, b2);
  a.spawn([1, 2, 3, 0, 1, 1, 1, 1]);
  a.spawn(new Float32Array([4, 5, 6, 7, 0.5, 0.5, 0.5, 1]));
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { b, engine, calls } = bind(scene, cam);
  const progs = ofType(calls, "particleProgram");
  assert.equal(progs.length, 1);
  const [, name, emit, update] = progs[0] as [string, string, string, string];
  assert.equal(name, "dust");
  assert.match(emit, /^fn pe_dust\(i: EIn, p: array<vec4<f32>, 8>\) -> PState \{/);
  assert.match(emit, /s\.vel = vec3<f32>\(0\.0, p\[0\]\.x, 0\.0\); s\.color = p\[1\];/);
  assert.match(update, /^fn pu_dust\(s_in: PState, i: UIn, p: array<vec4<f32>, 8>\) -> PState \{\n    var s = s_in;\ns\.vel\.y -= 9\.81 \* i\.dt;/);
  const objs = ofType(calls, "object").map((c) => c[2] as ObjectDesc).filter((o) => o.kind === "gpuParticles") as Extract<ObjectDesc, { kind: "gpuParticles" }>[];
  assert.equal(objs.length, 2);
  assert.deepEqual({ capacity: objs[0].capacity, rate: objs[0].rate, collide: objs[0].collide, castShadow: objs[0].castShadow }, { capacity: 1000, rate: 50, collide: { plane: 0.5, voxels: true, restitution: 0.2 }, castShadow: false });
  assert.deepEqual(objs[0].program.params.slice(0, 8), [3, 0, 0, 0, 1, 0.5, 0.25, 1]);
  assert.equal(objs[1].program.params[0], 2);
  const spawns = ofType(calls, "particlesSpawn");
  assert.equal(spawns.length, 1);
  assert.deepEqual([...(spawns[0][2] as Float32Array)], [1, 2, 3, 0, 1, 1, 1, 1, 4, 5, 6, 7, 0.5, 0.5, 0.5, 1]);
  const at = (t: string) => calls.findIndex((c) => c[0] === t);
  assert.ok(at("particleProgram") < at("object") && calls.findIndex((c) => c[0] === "object" && (c[2] as ObjectDesc).kind === "gpuParticles") < at("particlesSpawn"));
  b.sync();
  assert.equal(ofType(engine.take(), "particlesSpawn").length, 0, "sent once");
  b2.spawn([0, 0, 0, 0, 1, 1, 1, 1]);
  b.sync();
  assert.deepEqual(engine.take().map((c) => c[0]), ["particlesSpawn"], "a later spawn: no rebuild");
  assert.throws(() => a.spawn([1, 2, 3]), /8 numbers/);
  assert.throws(() => new GpuParticles({ capacity: 0, program: prog.with(), material: mat }), /capacity/);
  assert.throws(() => new GpuParticles({ capacity: 4, program: prog.with(), material: mat, collide: { friction: 2 } }), /friction/);
});

test("volume functions: one that reads the time is animated, unless it says otherwise; the volume's description carries it", () => {
  const still = new VolumeFunction("still_fog", { code: "o.extinction = 1.0;" });
  const drift = new VolumeFunction("drift", { code: "o.extinction = fract(i.time);" });
  const helper = new VolumeFunction("drift_helper", { helpers: "fn dh(i: VIn) -> f32 { return fract(i.time); }", code: "o.extinction = dh(i);" });
  const custom = new VolumeFunction("by_custom", { code: "o.extinction = i.custom.x;", animated: true });
  const held = new VolumeFunction("held", { code: "o.extinction = 1.0 + 0.0 * i.time;", animated: false });
  assert.deepEqual([still, drift, helper, custom, held].map((f) => f.animated), [false, true, true, true, false]);
  const scene = new Scene();
  scene.add(new VolumeMesh({ volume: still.with(), majorant: 1 }), new VolumeMesh({ volume: drift.with(), majorant: 1 }));
  // (Delta tracking steps by -ln(1 - u) / majorant: an infinite one would never step.)
  for (const bad of [Infinity, NaN, 0, -1]) assert.throws(() => new VolumeMesh({ volume: still.with(), majorant: bad }), /finite number > 0/);
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { calls } = bind(scene, cam);
  const volumes = (ofType(calls, "object").map((c) => c[2]) as ObjectDesc[]).filter((o) => o.kind === "volume") as Extract<ObjectDesc, { kind: "volume" }>[];
  assert.deepEqual(volumes.map((v) => v.animated), [undefined, true]);
});

test("density grids: the grid goes before its volume, with its program's WGSL once; new data and sources go in place", () => {
  const scene = new Scene();
  const fn = new VolumeFunction("grid_reader", { params: { K: 2 }, code: "o.extinction = $K * vgrid(i, i.lp).x;" });
  const prog = new GridProgram("fill", { params: { Rate: 1, Wind: [1, 0, 0, 0] }, code: "s.a.x += $Rate * i.dt; s.b = $Wind;" });
  const data = new Float32Array(8).fill(0.5);
  const still = new VolumeMesh({ volume: fn.with(), majorant: 2, grid: new DensityGrid({ size: [2, 2, 2], data }) });
  const live = new VolumeMesh({ volume: fn.with({ K: 3 }), majorant: 3, grid: new DensityGrid({ size: [4, 2, 2], channels: 4, program: prog.with({ Rate: 5 }) }) });
  scene.add(still, live);
  const cam = new PerspectiveCamera();
  scene.add(cam);
  const { b, engine, calls } = bind(scene, cam);
  const progs = ofType(calls, "gridProgram");
  assert.equal(progs.length, 1);
  assert.match(progs[0][2] as string, /^fn gp_fill\(s_in: Cell, i: CIn, p: array<vec4<f32>, 8>\) -> Cell \{\n    var s = s_in;\ns\.a\.x \+= p\[0\]\.x \* i\.dt; s\.b = p\[1\];\n    return s;\n\}\n$/);
  const grids = ofType(calls, "densityGrid") as [string, number, { size: number[]; channels: number; program?: { id: string; params: number[] } }, Float32Array | null][];
  assert.equal(grids.length, 2);
  assert.deepEqual([grids[0][2], grids[0][3]], [{ size: [2, 2, 2], channels: 1 }, data]);
  assert.deepEqual([grids[1][2].size, grids[1][2].channels, grids[1][2].program!.id, grids[1][2].program!.params.slice(0, 8), grids[1][3]], [[4, 2, 2], 4, "fill", [5, 0, 0, 0, 1, 0, 0, 0], null]);
  const volumes = (ofType(calls, "object").map((c) => c[2]) as ObjectDesc[]).filter((o) => o.kind === "volume") as Extract<ObjectDesc, { kind: "volume" }>[];
  assert.deepEqual(volumes.map((v) => v.grid), [grids[0][1], grids[1][1]]);
  // A simulated grid's medium changes with time whatever its function reads; stored data alone does not.
  assert.deepEqual(volumes.map((v) => v.animated), [undefined, true]);
  const at = (pred: (c: [string, ...unknown[]]) => boolean) => calls.findIndex(pred);
  assert.ok(at((c) => c[0] === "densityGrid") < at((c) => c[0] === "object" && (c[2] as ObjectDesc).kind === "volume"), "the grid before the volume that names it");
  // New data: sent again (the engine uploads it in place); sources: after, once.
  data.fill(1);
  still.grid!.needsUpdate = true;
  live.grid!.emit([0, 1, 0, 0.5, 1, 1, 0, 0, 0, 0, 0, 2]);
  b.sync();
  const again = engine.take();
  assert.deepEqual(again.map((c) => c[0]), ["densityGrid", "gridSources"]);
  assert.equal((again[0][3] as Float32Array)[0], 1);
  assert.deepEqual([...(again[1][2] as Float32Array)], [0, 1, 0, 0.5, 1, 1, 0, 0, 0, 0, 0, 2]);
  b.sync();
  assert.equal(engine.take().length, 0);
  assert.throws(() => still.grid!.emit(new Array(12).fill(0)), /no program/);
  assert.throws(() => live.grid!.emit([1, 2, 3]), /12 numbers/);
  assert.throws(() => new DensityGrid({ size: [0, 1, 1] }), /size/);
  assert.throws(() => new DensityGrid({ size: [2, 2, 2], data: new Float32Array(7) }), /8 numbers/);
  assert.throws(() => new VolumeMesh({ shape: "sphere", volume: fn.with(), majorant: 1, grid: new DensityGrid({ size: [2, 2, 2] }) }), /box/);
  assert.throws(() => new VolumeMesh({ volume: fn.with(), majorant: 1, grid: still.grid }), /another volume/);
  // The standard library's smoke: a puff is one source.
  const smoke = new SimulatedSmoke({ size: [8, 8, 8] });
  smoke.puff([1, 2, 3], 0.5, { density: 0.7, push: 2 });
  assert.deepEqual([...smoke.grid!.pendingSources[0]].map((v) => +v.toFixed(3)), [1, 2, 3, 0.5, 0.7, 1, 0, 0, 0, 0, 0, 2]);
});
