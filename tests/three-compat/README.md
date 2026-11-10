# three.js compatibility set (ROADMAP § 6 step 2f)

Snippets of three.js scene code, as official examples and LLM-written code have it, run unchanged against the DSL by
`node tools/three-compat.ts` (pass rules there; packages/scene's tests run them without the render). Each file:
`// area:` (geometry, object3d, material, light, raycast, renderer, instancing), `// source:`, then the code as
written, the page around it (DOM, window, requestAnimationFrame, asset loaders) left out. Its default export returns
`{ scene, camera, frame?(t), check?(), expect? }`.

## Where ThreeRT differs, on purpose

- **Shadows are always on** (a path tracer's, from every light; `castShadow = false` is honoured).
  `renderer.shadowMap.*`, the lights' `shadow.*` and the rest of `WebGLRenderer` are accepted and ignored.
- **Materials are physically based**, the others mapped (each noted once, `compatNotes()`):
  `MeshLambertMaterial` = standard, roughness 1, metalness 0; `MeshPhongMaterial` = standard, roughness
  √(2 / (shininess + 2)), metalness 0; `MeshBasicMaterial` = unlit (its colour seen, lighting nothing);
  `MeshToonMaterial`, `MeshNormalMaterial`, `MeshDepthMaterial` = standard, with a warning.
- **Ambient light is an environment**: `AmbientLight` / `HemisphereLight` light the scene as a uniform / gradient sky
  of radiance colour x intensity / pi (three.js: irradiance colour x intensity), and are not seen. In such a scene the
  background is `scene.background` or black, and a camera whose exposure scene code did not set takes three.js's
  fixed one (EV 0).
- **Background and fog colours** are shown at the level of the scene's light (its ambient and directional lights'
  irradiance / pi): a white background is as bright as a white surface in the key light. `Fog` / `FogExp2` are uniform
  exponential fog (95 % at `far`; FogExp2's e-folding distance).
- **Light decay is physical** (inverse square) whatever `decay` says.
- **`object.id`** is the stable id scene code sets (picking, events); until one is set, a number, as three.js's.
- **Geometry that changes at run time**: `attribute.setUsage(DynamicDrawUsage)` gives the mesh its own acceleration
  structure; without it, the first change rebuilds the scene once and promotes the mesh (frame log `fallback`
  "promote").
- Not drawn (noted `unsupported`): vertex colours, wireframe, `BackSide` (drawn double-sided). Missing: `Shape` /
  `ExtrudeGeometry`.
