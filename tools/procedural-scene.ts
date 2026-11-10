// The procedural geometry check scene (ROADMAP § 6 step 1; tools/unbiased-check.ts `procedural`): analytic spheres and
// boxes (packages/scene procedural.ts) beside their triangle versions on a triangle floor, in sun and sky. Covered: one
// geometry function for many boxes (the spheres' data), several materials per mesh, clearcoat, a material function, an
// emissive procedural sphere (lit surfaces get it by their BSDF rays only), a rotated, non-uniformly scaled instance
// (normals through the inverse transpose), a sphere inside a box's bounds (two functions overlapping) and shadows
// both ways between triangles and procedural geometry.
//   node tools/bundle-game.ts tools/procedural-scene.ts -o out/bundles/procedural-scene.js
import { BoxGeometry, DirectionalLight, MaterialFunction, Mesh, MeshPhysicalMaterial, MeshStandardMaterial, PerspectiveCamera, ProceduralMesh, Scene, SkyAtmosphere, SphereGeometry, proceduralBoxes, proceduralSpheres } from "@threert/scene";
import { defineGame } from "@threert/gameplay";

/** World-space stripes from a material function (procedural surfaces have no uv). */
const STRIPES = new MaterialFunction("proc_stripes", {
  params: {},
  code: `
    let s = step(0.5, fract(i.wp.y * 4.0));
    o.base = mix(o.base, vec3<f32>(0.9, 0.85, 0.2), s);`,
});

export default defineGame({
  state: {},
  setup: () => {
    const scene = new Scene();
    const sun = new DirectionalLight(0xffffff, 90000);
    sun.setSunAngles(40, 300);
    scene.add(sun);
    scene.environment = new SkyAtmosphere({ sun });
    const camera = new PerspectiveCamera(45, 16 / 9, 0.05, 300);
    camera.position.set(0, 2.2, 7.5);
    camera.lookAt(0, 0.7, 0);
    camera.setManualExposure(13);
    scene.add(camera);

    const floor = new Mesh(new BoxGeometry(40, 0.1, 40), new MeshStandardMaterial({ name: "floor", color: 0x8a8580, roughness: 0.7 }));
    floor.position.y = -0.05;
    scene.add(floor);

    const red = new MeshStandardMaterial({ name: "red", color: 0xb03020, roughness: 0.4 });
    const metal = new MeshStandardMaterial({ name: "metal", color: 0xd0d0d8, roughness: 0.25, metalness: 1 });
    const coat = new MeshPhysicalMaterial({ name: "coat", color: 0x2050a0, roughness: 0.6, clearcoat: 1, clearcoatRoughness: 0.05 });
    const glow = new MeshStandardMaterial({ name: "glow", color: 0x000000, emissive: 0xffa040, emissiveIntensity: 60000 });
    const striped = new MeshStandardMaterial({ name: "striped", color: 0x404040, roughness: 0.5 });
    striped.shader = STRIPES.with({});

    // Spheres of one geometry: three materials picked by their data.
    const balls = new ProceduralMesh(proceduralSpheres([
      { center: [-2.6, 0.6, 0], radius: 0.6, material: 0 },
      { center: [-1.2, 0.45, 0.8], radius: 0.45, material: 1 },
      { center: [-0.2, 0.35, 1.8], radius: 0.35, material: 2 },
    ]), [red, metal, coat]);
    balls.name = "balls";
    scene.add(balls);
    // Their triangle twin, mirrored across x = 0.
    const twin = new Mesh(new SphereGeometry(0.6, 96, 48), red);
    twin.name = "twin";
    twin.position.set(2.6, 0.6, 0);
    scene.add(twin);

    // A box rotated and scaled unevenly, with stripes; a small sphere sits inside its bounds (overlapping boxes).
    const crate = new ProceduralMesh(proceduralBoxes([{ min: [-0.5, 0, -0.5], max: [0.5, 1, 0.5] }]), striped);
    crate.name = "crate";
    crate.position.set(1.0, 0, -1.2);
    crate.rotation.y = 0.6;
    crate.scale.set(1.4, 1.2, 0.6);
    scene.add(crate);
    const pearl = new ProceduralMesh(proceduralSpheres([{ center: [1.2, 1.35, -1.2], radius: 0.25 }]), metal);
    pearl.name = "pearl";
    scene.add(pearl);
    // A triangle box in front of it: procedural geometry in its shadow, and the other way round.
    const block = new Mesh(new BoxGeometry(0.5, 0.5, 0.5), red);
    block.name = "block";
    block.position.set(0.6, 0.25, 0.4);
    scene.add(block);

    // An emissive procedural sphere near the floor (its light reaches the floor by BSDF rays alone).
    const lamp = new ProceduralMesh(proceduralSpheres([{ center: [2.2, 0.15, 1.6], radius: 0.15 }]), glow);
    lamp.name = "lamp";
    lamp.castShadow = false;
    scene.add(lamp);

    return { scene, camera, options: { name: "procedural-scene", width: 1280, height: 720, preset: "cinematic" as const } };
  },
  handlers: {},
});
