# Material functions

A material function is a WGSL body attached to a material, run by the path tracer at every hit of that material
(camera hits, every bounce, shadow-ray alpha tests and emitter sampling). It is our equivalent of a UE material graph or
a Custom-HLSL node. The DSL declares it once with named parameters and texture slots, and each material instance binds
its own values. Expressiveness lives in code instead of in a growing list of built-in material fields, so a UE master
material can be ported line for line (the Kabukichō port in `games/kabukicho/street.ts` mirrors the UE session's nine
master materials this way).

```ts
import { MaterialFunction, MeshStandardMaterial, Texture } from "@threert/scene";

const SIGN = new MaterialFunction("sign", {
  params: { Intensity: 12, Tint: [1, 1, 1, 0], Pan: [0, 0, 0, 0], Roughness: 0.22 },  // number = scalar, array = vec4
  textures: ["SignTex"],                                                                // up to 8 slots
  helpers: "fn hash21(p: vec2<f32>) -> f32 { ... }",                                    // optional module-scope WGSL
  code: `
    let uv = i.uv + $Pan.xy * i.time;
    let t = msample_color(m, i, $SignTex, uv, exp2(i.uvd));
    o.emissive = t.rgb * $Tint.rgb * $Intensity * i.power;
    o.base = t.rgb * 0.08;
    o.roughness = $Roughness;
    o.opacity = t.a;`,
});

const mat = new MeshStandardMaterial({ name: "sign_ramen" });
mat.shader = SIGN.with({ Intensity: 20 }, { SignTex: new Texture("assets/kabukicho/textures/T_Sign_ramen.png") });
```

## Standard library

Most scenes need the same few materials, so `@threert/scene` ships them as Three-like constructors
(`packages/scene/src/materials.ts`): fill in parameters, no shading code. Nine are the Kabukichō master materials
above, taken line for line; `SurfaceMaterial` is the renderer's former built-in material fields (world mapping,
wetness, puddles, grime, facade windows), moved out of the kernel line for line. The constructors take metres and
three.js names (`map`, `roughnessMap`, `normalMap`, `color`, `roughness` as the roughness map's multiplier) and bind
neutral 1×1 maps for slots left empty.

| constructor | what it is | main parameters |
|---|---|---|
| `SurfaceMaterial` | any surface with procedural weathering: standard maps on mesh UVs or world-mapped, wet look, puddles with rain ripples, grime, lit windows with fake interiors | three.js standard parameters, `worldScale`, `wet`, `puddles`, `grime`, `windows` (`facadeWindows()`) |
| `WetGroundMaterial` | street under rain: darkens and turns glossy with `scene.wetness`, puddles fill, `scene.rain` ripples | `map`, `roughnessMap`, `normalMap`, `puddleMap`, `textureScale`, `puddleScale`, `puddles` |
| `FacadeMaterial` | tiling wall, world-mapped, with windows per storey and bay, some lit with a room behind | `map`…, `grimeMap`, `floorHeight`, `bayWidth`, `windowWidth/Height`, `groundHeight`, `sill`, `litChance`, `brightness`, `seed` |
| `TriplanarMaterial` | any tiling surface mapped by world position (no UVs): concrete, metal plates, shutters | `map`…, `textureScale`, `metalness`, `wet` |
| `PaintMaterial` | painted or bare props: one colour, grime, wet look | `color`, `roughness`, `metalness`, `grimeMap`, `grime`, `wet` |
| `NeonSignMaterial` | emissive sign artwork: neon, lightbox, lantern, marquee | `map`, `intensity` (nits), `flicker`, `blink`, `blinkPhase`, `dim`, `repeat`, `scroll`, `glow`, `masked` |
| `LedScreenMaterial` | video wall: picture on an LED grid, alternating with a plasma | `map`, `intensity`, `resolution`, `led`, `plasma`, `cycle`, `seed`, `glow` |
| `LightPanelMaterial` | glowing fitting: bulbs, panels, blinking warning lights | `color`, `intensity`, `blink`, `rate`, `phase` |
| `ShopWindowMaterial` | dark glass in front of an interior picture with parallax | `map`, `intensity`, `glow` |
| `RoadMarkingMaterial` | road paint, cut out by a mask, glossy when wet | `color`, `mask` |

```ts
import { NeonSignMaterial, Texture, WetGroundMaterial } from "@threert/scene";

const ph = (id: string, m: string) => new Texture(`assets/polyhaven/tex/${id}/${id}_${m}_2k.jpg`, m === "diff" ? "srgb" : "linear");
const street = new WetGroundMaterial({ map: ph("worn_asphalt", "diff"), roughnessMap: ph("worn_asphalt", "rough"),
  normalMap: ph("worn_asphalt", "nor_gl"), puddleMap: new Texture("assets/kabukicho/masks/T_PuddleMask.png", "linear"),
  textureScale: 2, puddles: 1.1, color: [0.85, 0.8, 0.8] });
const sign = new NeonSignMaterial({ map: new Texture("assets/kabukicho/textures/T_SignH_00.png"), intensity: 9, flicker: true, glow: [1, 0.5, 0.2] });
sign.set({ intensity: 14 });   // parameters are read when a material is made; set() changes them later
```

`glow` × `intensity` is what an emissive library material lights its surroundings with: the renderer samples emitters
by their base emissive, while the function does the actual emission. A material the library doesn't cover is a
`MaterialFunction` of its own; the library's functions are worked examples.

## Contract

The body runs inside `fn mf_<name>(i: MatIn, m: Material, base: MatOut) -> MatOut { var o = base; <code> return o; }`.
`base` is the material's built-in evaluation (colour, maps, roughness, emission), so a function can override all of it
or only adjust a part.

| `MatIn` field | meaning |
|---|---|
| `wp` | world position (m) |
| `n`, `ng` | vertex / geometric normal, facing the viewer |
| `t`, `b` | tangent (+u), bitangent (image up) |
| `uv`, `uvd` | mesh UV0 (v down); `exp2(uvd)` = UV units per metre (pass it to `msample` for the mip) |
| `view` | unit vector toward the ray origin |
| `op`, `ob` | object pivot and world bounds size (UE `ObjectPositionWS` / `ObjectBounds`) |
| `time`, `wetness`, `rain` | scene clock (s) and the scene's weather (0..1) |
| `lod`, `depth` | log2 ray footprint (m); path vertex (0 = camera hit; skip expensive detail when > 0) |
| `near` | seen directly or in mirror-like reflections (puddles, glass, chrome): where fine detail (normal maps, grime, windows) shows; false on diffuse / glossy bounces |
| `power` | node power (`tx.write("nodes/<id>/power", p)` from gameplay; multiply emission by it) |
| `custom` | per-instance data (UE `PerInstanceCustomData`): the node's `customData` (four floats, default 0; `engine.customData(id, v)` changes it live), for an `InstancedMesh`'s instance its `setCustomDataAt` data (else the mesh's `customData`), or for a particle its particle function's `o.custom` |

| `MatOut` field | meaning |
|---|---|
| `base`, `roughness`, `metallic`, `specular` | UE default-lit inputs (specular 0.5 = F0 0.04) |
| `emissive` | nits |
| `normal` | world-space shading normal (`tangent_normal(i, tn)` converts a tangent-space normal) |
| `opacity` | alpha-tested against the material's `alphaTest` |
| `animated` | the shading here moves with time (wave normals): the live view keeps a short history, as it does for animated emission (default false) |

Parameters are written `$Name`: scalars pack four to a vec4 slot, vec4s take a slot each (8 slots in total). A
texture name `$Tex` becomes its slot index. Helpers: `msample(m, i, slot, uv, uv_per_m)` (linear) /
`msample_color` (sRGB → linear) with ray-cone mips; `tangent_normal`; `ue_pos(p)` / `ue_vec(v)` / `from_ue_vec(v)`
convert to and from UE world space (cm, Z up), so hashes and world-aligned mappings ported from a UE material produce
the same patterns.

## Per-instance data and particles

`obj.customData = [a, b, c, d]` gives one node's surfaces a vec4 that any material function reads as `i.custom`: one
material, many looks (a sign's flicker phase, a window's lit state, a prop's tint) without a material per node.

An `InstancedMesh` gives each instance its own: `mesh.setCustomDataAt(i, [a, b, c, d])` (then
`mesh.instanceCustomData.needsUpdate = true`, as for `instanceMatrix`). Each instance is its own object to material
functions: `i.op` / `i.ob` are its pivot and bounds. `setColorAt` (three.js) multiplies the material's base colour per
instance without a function.

Particles use the same channel. A `ParticleFunction` is a WGSL body that places one particle per call, on the GPU,
every frame:

```ts
const DRIFT = new ParticleFunction("drift", {
  params: { Speed: 0.5, Size: 0.05 },
  code: `
    let y = fract(i.rand.y - i.time * $Speed * (0.5 + i.rand.z)) * 4.0;
    o.center = i.pivot + vec3<f32>(i.rand.x * 4.0 - 2.0, y, i.rand.w * 4.0 - 2.0);
    o.u = vec3<f32>($Size, 0.0, 0.0);
    o.v = vec3<f32>(0.0, $Size, 0.0);
    o.custom = vec4<f32>(i.rand.z, 0.0, 0.0, 0.0);`,
});
scene.add(new Particles({ count: 2000, emitter: DRIFT.with({ Speed: 0.8 }), material: dust }));
```

It runs inside `fn pf_<name>(i: PIn, p: array<vec4<f32>, 8>) -> POut { var o = POut(i.pivot, vec3(0), vec3(0), true,
vec4(0)); <code> return o; }`:

| `PIn` field | meaning |
|---|---|
| `index`, `count` | the particle (0 .. count) and the node's count |
| `time`, `dt` | scene clock and frame time (s) |
| `cam`, `fwd` | camera position and forward |
| `rain`, `wetness` | the scene's weather (as in `MatIn`) |
| `rand` | four uniform random numbers, fixed per particle |
| `pivot` | the node's world position |
| `custom` | the node's `customData`, live: a game sets it without a rebuild (a burst's start time, a wind gust) |

| `POut` field | meaning |
|---|---|
| `center` | world position |
| `u`, `v` | the quad's half extents: uv.x grows along `u`, image up (uv.y = 0) is at +`v` |
| `visible` | false collapses the quad |
| `custom` | the material function's `i.custom` for this particle; `i.op` / `i.ob` are its center and bounds |

Parameters are `$Name`, as in material functions. A particle function has no state: it is a function of time, the camera
and its random numbers, so every client places the same particles and a frame can be re-rendered. Each particle is two
dynamic triangles and an object of its own; a compute pass (`renderer/shaders/particles.wgsl`) writes them (positions,
shading normal and tangent from `u` / `v`), last frame's positions (motion vectors) and the BLAS input, then the dynamic
BLASes and the TLAS are rebuilt. Moving the node moves `i.pivot`. Offline renders
place the particles once, at the scene time. Particles need hardware ray tracing (with software traversal they stay
collapsed at the pivot) and cast no shadow by default.

Cost: 16 000 particles over Kabukichō take 0.12 ms for the pass and 0.6 ms for the BLAS + TLAS rebuild at the 0.5 floor
(`THREERT_PROFILE` lists both as `particles` and `blas`). With an opaque or alpha-tested material, particles are surfaces
the path tracer hits like any other.

### Emissive particles and sparks

Emissive particles light the scene: their surfaces are emitters like any other.
- Without `emitterArea`, their light reaches other surfaces only through those surfaces' BSDF rays. That is unbiased,
  but noisy for small bright particles.
- With `emitterArea` (m², a particle's typical size), they are also sampled as lights, which is what sparks need. The
  value only decides how often a particle is picked: the kernel samples its actual quad and weighs it against the BSDF
  rays with the same pdf.

Layer A covers it:
- `particles/emissive-spark`: a 1000-nit quad lights a floor by the analytic amount.
- `particles/emissive-spark-nominal`: the same with a 10× wrong `emitterArea`.
- `particles/emissive-unsampled`: an emissive particle without `emitterArea`, next to other light-sampled emitters.

`SparkBurst` (standard library) is a `Particles` node built this way:
- sparks fly from the node in a cone, with gravity and drag, as streaks along their velocity;
- they cool from white through yellow and orange to red, and die;
- the burst fires at `customData[0]` (scene time) and can repeat every `period` s.

`Debris` is a group of a few chunks that `fire(time)` / `update(time)` throw and land. It is plain movable meshes moved
by the frame script: a handful of objects need no particle system.

```ts
const sparks = new SparkBurst({ count: 300, speed: 6, intensity: 500 });
scene.add(sparks);
// An event handler records when (tx.state.boomAt = the scene time); the frame script shows it:
sparks.customData = [f.state.boomAt, 0, 0, 0];
```

### Translucent particles

A particle node whose material is `transparent` (three.js's flag) is drawn by the translucency pass instead
(`renderer/shaders/translucent.wgsl`), as UE draws translucent materials:

- **Outside the scene.** Its triangles are in no acceleration structure the path tracer uses, only in their own. They
  cast no shadow and show in no reflection, refraction or GI (UE's default for translucency; docs/ROADMAP.md, Known
  parity gaps).
- **Over the image.** Per output pixel, after the temporal filter (live) or over the converged image (offline), a ray
  goes as far as the opaque surface the pixel shows. Then:
  - nothing behind a wall shows;
  - the 4 nearest particle hits are blended front to back (ordered blending, not weighted OIT);
  - there is no history, so nothing trails.
- **Material.** The material's colour, maps and opacity (map alpha × `opacity`, or the material function's
  `o.opacity`) and its function, with `i.custom` per particle. Emission adds as usual.
- **Lighting.** Non-directional: `base × L + emissive`, where L is the mean radiance arriving at the hit from all
  directions, i.e. what a white isotropic scatterer there sends back (UE's "volumetric non-directional" translucency
  lighting).
  - Live, L comes from the radiance cache's volume voxels (below).
  - Offline, it is a fresh estimate per sample: the lights sampled for an isotropic receiver, plus one ray in a
    uniform direction for the rest.

```ts
const mat = new MeshStandardMaterial({ transparent: true, color: 0xffffff });
mat.shader = STREAK.with({});        // o.opacity from i.uv and i.custom, for example
scene.add(new Particles({ count: 4000, emitter: SNOW.with({}), material: mat }));
```

**Volume voxels.** The radiance cache keeps lighting in the air next to its surface voxels: a voxel keyed by position
alone (no surface side), 4× and 16× coarser than a surface voxel at the same distance. Its value is the mean incident
radiance above.
- The translucency pass trains them: 1 in 8 pixels showing a particle adds its nearest hit, at most 8192 points a
  frame (`vtrain` in `THREERT_PROFILE`).
- Lookups take the finer level once it has 4 samples, else the coarser.
- Nothing in the key or the lookup is specific to particles: participating media (ROADMAP item 6.3) can read and train
  the same voxels.
- Layer A `volume/lighting` checks the live voxels: a white particle at the centre of a closed cube with one lit
  wall reads 1/6 of the wall's radiance. `volume/lighting-offline` checks the same with the offline estimate.

`RainCurtain` (the standard library's rain, UE's AKabukichoRain + M_Rain) is a `Particles` node built this way: its
particle function wraps 16 000 streaks in a box around the camera, and its material function gives M_Rain's streak
profile, colour and distance fades.

### Stateful particles: particle programs

A `Particles` node's particle function places every particle from scratch each frame (its index, the time, its fixed
random numbers): nothing carries over. A `GpuParticles` node keeps its particles' state on the GPU instead, so they can
be born at any time, pushed by forces, collide and come to rest (ROADMAP § 6 step 5a;
`renderer/shaders/gpu_particles.wgsl`):

- **State.** Per particle: position and age, velocity and life, colour, size (half the quad's side). The node has
  `capacity` slots, a ring: new particles take the next slots, the oldest reused when it is full. The CPU keeps no
  particle; it sends only how many are born this frame.
- **Births.** At `rate` per second at the node's position, and the game's own: `node.spawn(data)`, 8 numbers a
  particle (position xyz, a free number `i.tag`, colour rgba), sent at the next sync, born at the next frame (up to
  65,536 a frame).
- **The program.** `ParticleProgram(name, { params, emit, update, helpers })`, two WGSL bodies, `$Name` parameters as
  in a material function:
  - `emit` fills `s` (a `PState`: `pos`, `vel`, `age`, `life`, `color`, `size`), which starts at `pos = i.at`,
    `vel = 0`, `life = 1` s, `color = i.color`, `size = 0.05` m. Inputs: `i.index`, `i.time`, `i.rand` (vec4, fresh for
    each birth), `i.pivot` (the node's position), `i.spawned`, `i.at`, `i.tag`, `i.color`.
  - `update` changes `s` once a frame: forces on `s.vel`, colour and size over `s.age / s.life`. Inputs: `i.index`,
    `i.time`, `i.dt`, `i.cam`, `i.pivot`.
  - The engine then moves the particle by its velocity, collides it, ages it and retires it at `s.age >= s.life`.
- **Collision** (`collide`): a ground plane (`plane`: its height) and the voxel level's occupancy grid (`voxels`: the
  cells of `voxelLevel({ occupancy: cell })`, set again where explosions carve and pieces settle). On contact the
  velocity along the normal is reflected and scaled by `restitution`, the rest scaled by `1 - friction`, and the
  particle is put against the surface, its centre `size` from it. Triangle scenes are not collided with (later).
- **Drawn** as camera-facing quads of two dynamic triangles, each its own object, in the dynamic BLAS rebuilt every
  frame (as `Particles`). Lit like any surface; emissive materials glow, with `emitterArea` as for sparks. The
  material function reads the particle's colour as `i.custom`. Opaque or alpha-tested materials only.
- **Offline**, a still at scene time t simulates round(t × 60) steps from the setup's spawns, then renders.
- A structural rebuild of the scene starts the particles afresh.

```ts
const DUST = new ParticleProgram("dust", {
  params: { Speed: 4, Size: 0.02 },
  emit: `
    s.vel = normalize(i.at - i.pivot) * $Speed * (0.5 + i.rand.x);
    s.life = 2.5 + 1.5 * i.rand.y;
    s.size = $Size;`,
  update: `
    s.vel.y -= 9.81 * i.dt;`,
});
const mat = new MeshStandardMaterial({ roughness: 0.9 });
mat.shader = new MaterialFunction("dust_surface", { code: "o.base = i.custom.rgb;" }).with({});
const dust = new GpuParticles({ capacity: 65536, program: DUST.with({}), material: mat, collide: { plane: 0, voxels: true } });
scene.add(dust);
// In the frame script: debris voxels (x, y, z, palette index each) turned to dust, thrown out from the explosion.
dust.position.set(...center);
dust.spawn(debrisAsDust(voxelDebris()));
```

`games/voxel-level/level-destruction-dust.game.ts` turns every debris voxel of the level's explosions into dust this
way.

## Volume functions

A `VolumeFunction` fills a participating medium (smoke, local fog, a fireball). It is a WGSL body that gives, at a
point, the extinction (1/m), albedo and emission (radiance per metre, absorption × emitted radiance):

```ts
const PUFF = new VolumeFunction("puff", {
  params: { Density: 2, Color: [0.8, 0.8, 0.85, 0] },
  code: `
    let r = length(i.lp) * 2.0;                 // 0 at the centre, 1 at the surface
    o.extinction = $Density * max(1.0 - r * r, 0.0);
    o.albedo = $Color.rgb;`,
});
const smoke = new VolumeMesh({ shape: "sphere", volume: PUFF.with({ Density: 3 }), majorant: 3 });
smoke.scale.set(2, 3, 2);                         // the unit sphere (diameter 1) under the node's transform
scene.add(smoke);
```

It runs inside `fn vf_<name>(i: VIn, p: array<vec4<f32>, 8>) -> VOut { var o = VOut(0.0, vec3(1.0), vec3(0.0)); <code>
return o; }`:

| `VIn` field | meaning |
|---|---|
| `p` | world position, m |
| `lp` | position in the volume's own space: the box [-0.5, 0.5]³ or the sphere of radius 0.5 |
| `time` | scene time, s |
| `custom` | the node's `customData` |

| `VOut` field | meaning |
|---|---|
| `extinction` | 1/m; clamped to the volume's `majorant` |
| `albedo` | scattering / extinction |
| `emission` | emitted radiance per metre |

A function whose medium changes with time is `animated`: by default, one whose code reads `i.time`; `animated: true` or
`false` in its options says otherwise (a medium that changes only through its node's `customData` sets it; one that
reads the time for something that does not show clears it). A volume with a simulated `DensityGrid` is animated
whatever its function. The live view keeps a short history where such a medium fills a quarter of a pixel or more
(four samples in the upscaler, as for a surface with `o.animated`): without it a fire shows the frames before it. A
still medium keeps the long history, and its noise falls with it.

A `VolumeMesh` has these settings:
- `shape`: box or sphere;
- `volume`: the function binding;
- `majorant`: an upper bound of the extinction anywhere in it, required;
- `anisotropy`: the Henyey-Greenstein g.

The path tracer samples collisions at the majorant's rate. A loose bound is slower, never wrong; a function above it is
clamped. Volumes may overlap (a fire inside smoke): where they do they are one medium, their coefficients summed, and a
collision there is one of theirs by its extinction. They are lit by every light, cast shadows on what is behind them and scatter light
any number of times:
- delta tracking for paths;
- ratio tracking for shadow rays (Novák et al. 2018);
- emission gathered along the way.

The kernel compiles all of it out when no scene node is a volume (feature bit F_MEDIA): existing scenes render
byte-identical and pay nothing.

`FogVolume` in the standard library is UE 5.8's Local Fog Volume, as UE voxelizes it into volumetric fog
(VolumetricFog.usf). It takes UE's settings under the same names and defaults: `radialExtinction`,
`heightExtinction`, `heightFalloff`, `heightOffset`, `albedo`, `emissive` and `phaseG`.
- In the unit sphere (radius 1), the radial part is RadialFogExtinction × saturate(1 − d)^0.82, and the height part is
  HeightFogExtinction × exp(−HeightFogFalloff × (z − HeightFogOffset)).
- The two combine as coverages, not as a sum: with A = e^−height and B = e^−radial, the extinction is
  −log(A + B − AB) per metre.
- So fog needs both parts: with no height part there is no fog. UE's default falloff of 1000 cuts the sphere flat at
  the offset.
- The node's scale is the diameter: UE's scale 1.2 (a 1.2 m radius) is `scale.set(2.4, 2.4, 2.4)`.
- `volume/local-fog` checks the density against the optical depth integrated numerically: 0.2823 for 0.2863.

Layer A checks it against analytic values:
- `volume/box-transmittance`: Beer–Lambert;
- `volume/box-emission`: an emitting absorber;
- `volume/box-single-scatter`: a point light's in-scattering against the integral, attenuation included.

Offline renders are unbiased (delta tracking). The live viewport marches media instead, so the surface behind stays each
pixel's first hit for the temporal filter:
- The march is a pass of its own after the trace (`media_composite`): per pixel, 8 jittered steps from the camera to
  the first hit. The transmittance scales everything the pixel sees behind the medium, and the medium's emission and
  in-scattering are added to it.
- In-scattering is the medium's albedo × extinction × the mean incident radiance from the radiance cache's volume voxels,
  the same voxels translucent particles read. Live in-scattering is therefore isotropic, and its detail is the voxels'
  (a point light inside fog shows as a soft disc, not a peak).
- The voxel lookup is jittered within a voxel, so the accumulation filters the voxels instead of showing blocks.
- Live, reflection, GI and shadow rays do not see media: no smoke shadows on the street, no fog in reflections. The
  megakernel has no media code. Inlined in it, the march and the shadow-ray tracking cost Kabukichō 0.4 ms of trace with
  a smoke column out of view, and as much again with it in view. Offline paths see media everywhere. The exception is
  homogeneous media (water, below), which every live ray sees.
- A pass of its own, `media_train`, trains the voxels with 4096 fresh estimates a frame at points where camera rays
  cross the media. Its shadow rays do see media, so smoke shadows itself.
- That training's sphere ray crosses the media too, attenuated and gathering their in-scattered light from the
  voxels. Frame after frame this feedback adds every scattering order: multiple scattering.
  - A lamp inside a sphere of albedo-0.9 medium reads 7.61 live against 6.61 offline (+15 %); single scattering
    alone read 1.44.
  - A non-absorbing medium in a uniform sky reads 0.96 live (energy conservation: 1).
  - With max bounces 0 the feedback is off and the voxels hold single scattering.

The live tests `volume/box-*-live` give the same analytic values: transmittance (of an emissive and of a lit wall) and
emission exact, single scattering from a point light and from a sun −2 %. The smoke column of `tools/volume-scene.ts`
in front of its wall, lit by the moon alone, reads +5.7 % live over the offline reference: two thirds from the
isotropic phase function (the smoke's is HG 0.2), a third from the 8-step march's transmittance.

The standard library has three volume presets:
- `FogVolume`: UE 5.8's Local Fog Volume (its density formula and settings, above).
- `SmokeColumn`: fBm noise drifting upward, thinning toward the edge and top.
- `Fireball`: grows from `customData[0]`, emits by temperature, cools to smoke, can repeat.

Emissive media light their surroundings only through the surfaces' BSDF rays, since they are not sampled as lights.

### Density grids

A volume function computes the medium from a position. A `DensityGrid` gives it stored data instead: a 3D grid of
numbers across a box `VolumeMesh`, from the game or simulated on the GPU (ROADMAP § 6 step 5b).

```ts
const MEDIUM = new VolumeFunction("grid_medium", {
  params: { Density: 2, Albedo: [0.6, 0.6, 0.6, 0] },
  code: `
    o.extinction = $Density * vgrid(i, i.lp).x;   // the grid's first number at this point, trilinear
    o.albedo = $Albedo.rgb;`,
});
const grid = new DensityGrid({ size: [64, 64, 64], data });        // one number a cell, x fastest, then y, then z
const cloud = new VolumeMesh({ volume: MEDIUM.with({}), majorant: 2, grid });
cloud.scale.set(8, 8, 8);
```

- **The grid.** `size` cells across the box (1 to 512 an axis, at most 8,388,608 cells; a scene's grids at most twice
  that together), four numbers a cell, called field A. `data` has one number a cell (`channels: 1`: A's first) or four.
  Half floats on the GPU; all the scene's grids share one 3D texture.
- **Reading it.** `vgrid(i, lp)` in a volume function gives A at `lp` (the volume's own space, as `i.lp`), trilinear
  between cell centres. What the four numbers mean is the function's: density, temperature, a colour. A node without
  a grid reads zeros. The majorant must bound the extinction the function makes of them.
- **Rendering.** As any medium: delta tracking and ratio tracking offline; live, the camera rays' march and the volume
  voxels' in-scattering (above). A medium with a grid is marched in 64 steps instead of 8 (a grid has detail 8 steps
  miss), and its empty steps skip the voxel lookup.
- **Changing it from the game.** Write `grid.data`, set `grid.needsUpdate`: uploaded in place, no rebuild.
- **Simulating it.** With a `GridProgram`, the grid changes on the GPU every frame (`renderer/shaders/grid_sim.wgsl`,
  WGSL compute only). The grid then holds a second field B of four numbers, the program's own (a velocity, say).
  - The program is a WGSL body run once per cell, with `$Name` parameters. It gets the cell's `s.a` and `s.b` and
    changes them; it reads both fields as they were, anywhere, with `ga(uvw)` and `gb(uvw)` (trilinear, uvw 0..1 across
    the grid). Advection, diffusion, forces and sources are therefore all the program's own code; the engine only
    runs it.
  - A frame is two half steps (`i.dt` is half the frame's time), so the fields the renderer reads are always current.
  - Inputs: `i.cell`, `i.dims`, `i.uvw`, `i.p` (the cell's centre in the world), `i.size` (a cell's size, m),
    `i.time`, `i.dt`, `i.rand`, `i.custom` (the node's customData, set live), `i.sources` and `source(k)`.
    `to_uvw(p)` and `to_world(uvw)` go between world positions and grid coordinates.
  - **Sources.** `grid.emit(data)`, 12 numbers each: a sphere in the world (centre, radius) and two values of four
    numbers, which the program reads as `source(k).p`, `.radius`, `.a`, `.b` in the next frame's first half step (once:
    a program that adds what a source gives adds it once). At most 64 a frame.
  - Offline, a still at scene time t runs the program for round(t × 60) frames (at most 36,000) from the setup's
    sources, then renders.
  - A rebuild of the scene starts the grid again from its data.

```ts
// The whole of an advection: each cell takes what was upstream of it.
const DRIFT = new GridProgram("drift", {
  params: { Wind: [1, 0, 0, 0] },
  code: `
    s.a = ga(to_uvw(i.p - $Wind.xyz * i.dt));`,
});
const grid = new DensityGrid({ size: [64, 64, 64], data, program: DRIFT.with({}) });
```

`SimulatedSmoke` in the standard library is smoke and fire built this way: A is (density, temperature), B the velocity
and a pressure. Per cell and half step: semi-Lagrangian advection; one Jacobi step of the pressure that takes the
divergence out of the velocity (kept from step to step: a relaxation, not a full solve); diffusion; buoyancy;
turbulence; drag toward the wind (`wind`, plus the node's `customData.xyz`, live); cooling and thinning; sources; open
sides. `smoke.puff(center, radius, { density, temperature, push })` adds smoke.

```ts
const smoke = new SimulatedSmoke({ size: [88, 56, 144], density: 2.5, wind: [0.8, 0, 0.3] });
smoke.position.set(x, y, z);
smoke.scale.set(22, 14, 36);                        // the box the smoke lives in, m
scene.add(smoke);
// In the frame script, at an explosion:
smoke.puff(center, 1.5, { temperature: 1, push: 0.6 });
```

Layer A:
- `volume/grid-uniform`: a grid of ones against a `HomogeneousVolume` of the same coefficients (transmittance and
  scattering), ratio 1;
- `volume/grid-transmittance`, `-dense`: a grid that varies (a ramp along the view, a denser half), against the
  integral;
- `volume/grid-data-live`: data changed at run time, in place;
- `volume/grid-program`: a program that fills the grid, against the amount it must hold after 0.4 s;
- `volume/grid-advect`: a block of density moved by a program, where it must be after 0.25 s;
- `volume/grid-source`: a program that adds what a source gives, added once;
- `volume/grid-smoke`: `SimulatedSmoke`'s puff rises (a regression value).

Not done: collision of the smoke with scene geometry or voxels; grids in sphere volumes; reflection, GI and shadow
rays through grid media live (as for every medium but the homogeneous ones). A live rebuild (any structural change) starts every
simulated grid again from its data: the smoke there vanishes (ROADMAP § 6 step 10 b, Open).

### Water

`WaterBody` is a body of water: a `WaterMaterial` surface at the node's origin (y = 0, facing up) over a
`HomogeneousVolume` box of its size below it.

```ts
import { WaterBody } from "@threert/scene";

const canal = new WaterBody({ width: 6, depth: 1.5, length: 40, extinction: 1.2, albedo: [0.12, 0.2, 0.18],
  roughness: 0.04, waves: { wavelength: 1.2, steepness: 0.06, speed: 1, direction: 0.6 } });
scene.add(canal);
```

- `WaterMaterial` is clear glass of IOR 1.33 whose normals are four summed directional waves, each half as long as the
  one before and moving at the deep-water speed of its wavelength. `steepness` is the longest wave's slope. There is
  no displacement: normal waves are what UE's Single Layer Water uses at street scale.
- `HomogeneousVolume` is a `VolumeMesh` of constant coefficients (`homogeneous: true`; its function's parameters are
  `Coeff` = albedo rgb + extinction and `Emission`). Offline it is a medium like any other. Live, its constant
  coefficients let the megakernel attenuate and light reflection, refraction, GI and shadow rays through it
  analytically (`media_homogeneous`), behind a kernel feature bit of its own (F_HMEDIA): other scenes pay nothing.
- Light under the surface comes in two ways, offline:
  - Shadow rays go straight through glass (no caustics), each glass surface letting through the BSDF's transmission
    share at the crossing angle (transmission × (1 − E_spec)) tinted by its colour, as UE's path tracer does. A lamp seen from
    under water beyond the critical angle still lights through the surface, as its refracted light does.
  - Shadow rays toward the sky stop at an odd number of glass surfaces (glass that bends them: a water surface). The
    sky under water is then the refracted paths' alone, which find it in Snell's window and count it in full.
  - A path that passes glass that does not bend (a pane) keeps its last vertex's MIS weights, so the sky and the
    directional lights it finds there are weighed against that vertex's samples. Shadow rays toward emissive surfaces
    stop at all glass, so emitters behind glass count in full.
  - Unattenuated and counted on both paths, light under a water surface came in twice: a white furnace through water
    read 1.53, then 1.25 with the MIS fixed. `water/furnace` reads 0.997 now.
- Live, shadow rays pass glass unattenuated (UE's raster shadows of translucency). In-scattering along a refracted ray
  comes from the volume voxels at two points stratified by transmittance. The voxels are trained at points along the
  camera rays through the surface, down to the first opaque surface, and their sphere rays pass the surface or reflect
  off it (transmission share as above, all past the critical angle).

Layer A:
- `water/fresnel`: reflectance at normal incidence, F0 = 0.0201: 0.0202 offline, 0.0200 live.
- `water/absorption`: transmittance through 0.5 / m over 1 m, 2 e^−0.5 (1 − F0) = 1.189: 1.192 offline, 1.190 live.
- `water/furnace`: albedo-1 water in a uniform sky reads 1: 0.997 offline.
- `water/scatter`: a lamp over a pool of albedo-0.5 water, nearly all of it in-scattered: 0.1336 offline (4096 spp),
  0.1279 live (−4.2 %).

The canal of `tools/water-scene.ts` live against offline, frozen time, mean luminance by region: the walls, the
walkways, the pier, the far water, the water under the pier and the neon sign's and the wall's reflections all lie
within ±4 %. Two places do not:
- The dark water near the camera reads +20 %. Half of it is the live shadow rays' missing Fresnel (+11 % with it).
- The lamps' glints read 11 % of offline. Live clamps every lighting sample at 64× white (display units), so a small
  light's mirror image on calm water is white but carries little energy, and its bloom is weaker than offline.

`docs/img/water-canal.jpg`: offline | live, time frozen | live, waves moving.

Moving waves change the normals every frame, so long history smeared the live reflections into streaks. The wave
function sets `o.animated` while its waves move, and the live view keeps those pixels' history at 4 frames, as for
animated emission. `bench-temporal --shot water` measures it: the camera holds still while the waves move, and each
frame is compared with a reference converged 1024 frames at the same scene time.

| canal, ΔE / SSIM against the converged frame (T = 6, 7, 8 s) | water | reflections |
|---|---|---|
| time held at T, 240 frames, long history | 0.70–0.74 / 0.98 | 1.20–1.23 / 0.98 |
| waves moving, long history (before) | 6.88–7.00 / 0.81 | 20.4–20.9 / 0.48 |
| waves moving, `o.animated` | 3.71–3.76 / 0.83 | 9.2–9.9 / 0.64 |

`docs/img/water-waves-motion.jpg`: converged | moving, before | moving, now. The cost is noise: with the time held at T
the 4-frame history reads 1.9–2.1 / 3.8–4.1. Waves at speed 0 do not set `o.animated`, so still water keeps its long
history.

Live multiple scattering in water is the voxels' feedback, one more scattering order per training generation. Light
trapped under the surface by total internal reflection needs many orders: albedo-1 water in a uniform sky reads 0.52 /
0.70 / 0.77 / 0.81 live after 120 / 480 / 960 / 1920 frames. Water's real albedo (0.1 to 0.3) converges within a few.

### Wind

`windSway(plant, { strength, frequency, stiffness, direction, phase })` sways a placed plant rigidly: the renderer leans
it about its origin with scene time, downwind, around the horizontal axis across the wind.

```ts
import { GLTFModel, Group, windSway } from "@threert/scene";

// A potted plant: the pot stays, the plant (its own glTF node) leans about the soil, 0.305 m up.
const pot = new GLTFModel("assets/polyhaven/models/potted_plant_02_1k.glb", { nodes: ["potted_plant_02_pot", "potted_plant_02_dirt"] });
const soil = new Group();
soil.position.y = 0.305;
const leaves = new GLTFModel("assets/polyhaven/models/potted_plant_02_1k.glb", { nodes: ["potted_plant_02_leaves"] });
leaves.position.y = -0.305;
soil.add(leaves);
scene.add(pot, windSway(soil, { strength: 0.09, frequency: 0.7, direction: 0 }));
```

- The lean is A (0.6 + 0.4 sin(w t + p) + 0.2 sin(2.7 w t + 2 p)) radians: a mean lean of 0.6 A with a sway and a faster
  flutter on it. A = strength / (1 + stiffness), w = 2 pi frequency sqrt(1 + stiffness): stiffer plants lean less and
  sway faster. `windAngle` is the function; renderer/src/world.rs `Sway` is the same.
- The phase p is the option, or a hash of the plant's world position, so a row of plants does not sway in step.
- It is a node property (`Object3D.sway`, the engine's `sway` call), not frame code. The renderer leans the plant every
  live frame and for an offline still at its time, so a render at time t shows the sway at t. Changing it, a stronger
  wind for a storm, is a parameter edit.
- The plant becomes movable. A glTF model is then a TLAS instance whose transform moves (hardware RT): no triangles
  re-transformed, no BLAS rebuilt, motion vectors from the previous transform. Scene-code meshes re-transform their
  triangles in place (CPU). Before the instances, the foliage scene's ~52 k swaying triangles cost 2.4 ms of CPU a
  frame.
- Layer A: `wind/pivot-sway(-live)` reads an emissive marker on a 1 m pole where `windAngle` puts it at t = 0.5 s (10 of 10,
  offline and live).
- Motion (`bench-temporal --shot foliage`, the camera still, the plants swaying): ΔE against the frame converged at the
  same time reads 1.68–1.71 on the plants and 1.60–1.67 on the ferns, against 1.41–1.43 / 1.19–1.20 with time held:
  +0.3 / +0.4. `docs/img/foliage-wind.jpg`: converged | swaying.

## Geometry functions

A `GeometryFunction` is procedural geometry: a WGSL body that intersects a ray with what one box holds. A
`ProceduralGeometry` is a list of boxes in the object's space and the game's own data (32-bit words); a
`ProceduralMesh` places it under its node's transform with a list of materials. The path tracer finds the boxes a ray
enters and runs the function for each:
- hardware ray tracing: one AABB BLAS per distinct geometry, however many meshes use it, kept across rebuilds by the
  geometry's hash; one TLAS instance per mesh, moved by its node;
- the offline renderer's software traversal: each mesh's bounds, then a BVH over its geometry's boxes (built the first
  time a software render needs it, the triangle BVH's binned SAH).

```ts
// Spheres, one per box: data per sphere = centre xyz, radius (f32 bits), material index.
const SPHERE = new GeometryFunction("sphere", {
  code: `
    let k = i.data + i.prim * 5u;
    let c = vec3<f32>(gdataf(k), gdataf(k + 1u), gdataf(k + 2u));
    let r = gdataf(k + 3u);
    let oc = i.o - c;
    let a = dot(i.d, i.d);
    let b = dot(oc, i.d);
    let disc = b * b - a * (dot(oc, oc) - r * r);
    if (disc >= 0.0) {
        var t = (-b - sqrt(disc)) / a;
        if (t <= i.tmin) { t = (-b + sqrt(disc)) / a; }
        if (t > i.tmin && t < i.tmax) { o.t = t; o.normal = i.o + i.d * t - c; o.material = gdata(k + 4u); }
    }`,
});
const balls = new ProceduralMesh(new ProceduralGeometry(SPHERE, boxes, data), [red, metal]);
```

`proceduralSpheres` and `proceduralBoxes` (packages/scene `procedural.ts`) build exactly these for analytic spheres and
boxes. The body runs inside `fn gf_<name>(i: GIn) -> GOut { var o = GOut(-1.0, vec3(0.0, 1.0, 0.0), 0u); <code> return
o; }`:

| `GIn` field | meaning |
|---|---|
| `o`, `d` | the ray in the object's space; `d` is not normalised, so `t` is the world ray's own parameter |
| `tmin`, `tmax` | a hit must lie strictly between them (`tmax`: the closest hit found so far) |
| `prim` | the box's index in the geometry |
| `lo`, `hi` | the box, object space |
| `data`, `count` | where the geometry's data starts and its length in words: `gdata(i.data + k)` reads word k as u32, `gdataf(i.data + k)` as f32; outside the object's own data they read 0 (the function is game code and the kernel has no bounds checks of its own) |
| `custom` | the node's `customData` |
| `time` | scene time, s |

| `GOut` field | meaning |
|---|---|
| `t` | the hit's ray parameter; none: outside (`tmin`, `tmax`), as the default −1 |
| `normal` | object space, any length, facing either way (the renderer transforms it by the node's inverse transpose and faces it to the ray) |
| `material` | an index into the mesh's materials (past the last: the last; at most 255) |
| `attr` | two numbers for the material function, which reads them as `i.uv` (a voxel's palette entry, triplanar coordinates); half floats with a 9-bit mantissa (exact integers up to 1024), default 0 |

The function must answer the same for the same inputs: camera, shadow and indirect rays, the live and the offline render
and both traversals run it, and a hit it reports for one box is not checked against the box. A geometry holds up to
2²³ − 1 boxes. Data words keep their exact bits, NaN patterns included (`procedural/data-bits`).

What procedural surfaces are, today (ROADMAP § 6 step 1):
- Opaque and untextured: the material's maps are not read; material functions run, with `i.uv` = the function's
  `o.attr`, and can work from `i.wp`, `i.n` and `i.custom` too. No transmission; clearcoat works.
- Not light-sampled: an emissive procedural surface lights others only through their BSDF rays (as emissive media).
- Always instances: moving a `ProceduralMesh` rewrites its record and TLAS transform, never rebuilds the scene; its
  motion vectors come from the previous transform. Replacing its boxes or data (`needsUpdate`) rebuilds.
- Live primary visibility is traced (the raster experiment draws triangles only), and a click picks the boxes (the CPU
  does not run the function).

Hits carry `PROC_BIT | material << 23 | box` as their triangle, the octahedral object-space normal (15 bits per
component) in u, the attribute in v and the mesh's record (pathtrace.wgsl `proc_hit`), so `surface()` takes them apart
without running the function again. u and v hold 30 bits each inside ordinary floats (exponent bits 01xxxxxx): a raw
packed word can be a NaN pattern (the +Y normal as two snorms is 0x7FFF0000), whose bits WGSL does not promise to keep
through a bitcast (`procedural/normal-up`). The
records follow the model instances' in `instances`; the headers, materials, boxes and data follow the environment's CDF
in its binding (`Params::procs`), since the kernel has no binding to spare (Metal's 29 with the translucency pass).
- Each distinct geometry (by the SHA-256 of its boxes and data) is stored once, however many meshes use it.
- Every upload builds its own buffer: a scene being replaced, or another room's on a render node, may still read the
  last one. When the geometries are the same as the last upload's (a rebuild for a sky change, a material or an object
  added), their data is copied GPU to GPU from there, and only the CDF and the headers are sent: with 256 MB of data a
  sky change cost a 88–91 ms frame (73 ms of CPU) before, 33 ms (17 ms) now, where the scene without data takes 28.

Everything
is behind the feature bit F_PROC: a scene without procedural geometry compiles none of it (`THREERT_FEATURES_ON=262144`
forces it in, to measure that).

On Metal (wgpu 30), AABB candidates come up under `RAY_FLAG_FORCE_OPAQUE` (that flag concerns triangles), so every
query of a scene with procedural geometry runs a candidate loop. A loop over AABB candidates alone is deterministic run
to run; the cutout-triangle candidates in the same loop are what is not (Known issues, `cutout-trace`). Layer A
`procedural/*` checks spheres and boxes against their triangle versions, a furnace, materials, customData, shadows, a
live move, data reads clamped to the object's own data and data bits kept exactly; `tools/unbiased-check.ts --scenes procedural` checks the hardware traversal against the software one.

### Procedural geometry files

`ProceduralGeometry.fromAsset(fn, uri)` reads the boxes and data from a file under the asset root when the scene is
built, so a large geometry (a voxel level's bricks, a baked SDF, a heightfield) never passes through the scene code and
its QuickJS limits (1 GiB, a 120 s setup). Offline tools write the files with `encodeProcAsset` (tools/lib/proc-asset.ts).
The format (renderer/src/procasset.rs), little-endian throughout:

| bytes | field |
|---|---|
| 0..8 | magic `TRTPROC\0` |
| 8..12 | format version, 1 |
| 12..16 | byte-order mark 0x01020304 |
| 16..20 | box count n (≤ 2²³ − 1) |
| 20..24 | data word count m (≤ 2³⁰) |
| 24..32 | reserved, 0 |
| 32..64 | SHA-256 of the payload (bytes 64 on) |
| 64.. | boxes: n × (min xyz, max xyz) as f32, finite, min ≤ max |
| 64 + 24 n.. | data: m u32 words |

The file is exactly 64 + 24 n + 4 m bytes. The renderer checks every field, the length and the hash before it uses
anything, and refuses the file with a message otherwise (it may come from an untrusted game); it never reads past the
end. The hash is also what identifies the data on the GPU (above), which is why it is checked rather than trusted.
Files are parsed once per file version (path, length, modification time). A render node fetches them like the scene's
other files (`World::files`). Layer A: `procedural/asset`, `procedural/asset-bits` (tests/render/data/proc-two-boxes.tpg),
`procedural/data-rebuild-live` (data moved by a rebuild).

## Compilation and cost

The scene binding sends each function to the engine once (`engine.shader(name, wgsl, helpers)`) and each binding with
its material (`shader`: function id, 32 parameter floats, texture handles). The renderer splices all of a scene's functions into one kernel module behind a
`material_function` switch and compiles a pipeline variant per (feature mask, code hash), so scenes without functions
pay nothing. On the Kabukichō port (nine functions on ~2.6 k nodes) they cost about 20 % of the live frame. The usual
lever is cheaper evaluation at `i.depth > 0` (indirect hits), where fine normal or grime detail is invisible.

Names must be WGSL identifiers. Two different bodies with the same name are rejected, and an unknown `$Name` or an
unbound texture slot is an error in the scene binding, before anything reaches the renderer.
