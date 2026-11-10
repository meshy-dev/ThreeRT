# Scene runtime on the render node

> Split note (2026-11-10): this engine was extracted from `taichi-dev/serverless-agent-test` (main `42a51d6`). The serverless parts this document describes — `cloud/` (gateway, rooms, render pool), `render-node/`, `render-container/` and the deploy scripts — stay in that repository.

ThreeRT's goal is that an agent writes script-like code and gets a UE-quality world it can play at 60 fps. The
measure is how good the final work is for a given creative intent, so the platform does not need a cross-engine
interchange format. Round 2 (docs/PARITY.md) showed where expressiveness comes from: code. The UE session's look came
from Custom-HLSL materials, a C++ Director ticking every frame and gameplay switching material parameters. A field in
a scene schema is a ceiling.

So there is no scene document. The scene code runs where the scene is rendered, in the renderer's embedded JS
runtime, and drives the engine directly, the way a UE game drives its actors:

```
 Room (authority) ── committed game state ──┐
 handlers, OCC, log (state only)            ▼
                          ┌────────────── render node (threert-render) ───────────────┐
 game code ─────────────▶ │ ScriptHost (QuickJS): setup() builds the Three-like scene  │
 (bundle, by hash)        │   frame(f) derives it from the state, every frame          │
                          │   SceneBinding ──▶ engine API (__engine natives)           │
                          │ World (engine objects) ── changes ──▶ LiveCore             │──▶ pixels
                          │   parameters: GPU table entries in place (µs)              │
                          │   structure:  rebuild from the world (in memory)           │
                          └────────────────────────────────────────────────────────────┘
```

* **The unit of distribution is the game code.** Locally a bundle (`tools/bundle-game.ts`: game + `@threert/scene`
  + `@threert/gameplay` + the node host in one script); in the cloud the agent's ES module plus the shared runtime
  module. A node builds the scene from it (deterministic per room seed).
* **The world is the game state.** Handlers change `tx.state` only (OCC, log, replay). They never touch the scene.
* **What lives only where the engine is has its own handlers.** The physics world exists on the node that owns the
  scene, so an event that needs it (a click that casts a ray and adds a body) is handled there: `on[type](world, ev)`,
  a world handler, changes the scene as the event arrives (World handlers, below).
* **The scene is a function of the state.** `frame(f)` (an actor's Tick on a UE client) reads the committed state and
  sets the scene: transforms, visibility, `power`, light and material parameters, the environment, the camera, objects
  added or removed, plus animation (flicker, ramps). It runs every frame in Play, and with `dt = 0` after setup and
  after every commit. After setup or a replaced state `f.snap` is true: the scene shows the state settled, so a render
  of any room state is right.
* **No interchange format.** Nothing is exported. The UE ground truth for a level comes from that level in UE
  (docs/PARITY.md § Round 2); the old IR → UE adapter is gone.

## Pieces

| Piece | Where | What |
|---|---|---|
| Engine API | `packages/scene/src/engine.ts` (`Engine`), `renderer/src/script.rs` (`__engine` natives) | Handles (u32) for geometries (typed arrays), textures (asset path, encoded image or RGBA8 pixels), material-function code, materials, objects (group / mesh / model / light / camera) and their transform, visibility, power, light / camera parameters; the environment and render settings. Descriptors are JSON-shaped (`renderer/src/desc.rs`). |
| Scene binding | `packages/scene/src/engine.ts` `SceneBinding` | Keeps the engine in step with a DSL scene. Objects report their own changes (core.ts change tracking: transforms, visibility, power, shadow flag, mesh geometry / material, add / remove), so a sync visits only what changed. Lights, cameras and materials are compared against what was last sent. |
| World | `renderer/src/world.rs` | The engine's objects, resources, environment and settings, and what changed since the last commit: parameter changes (power, light / material parameters, environment, camera) or structural ones. Asset paths must stay inside the asset root (game code is untrusted on nodes). |
| Live commit | `renderer/src/live.rs` `LiveCore::commit`, `tables.rs` | Parameter changes rewrite GPU table entries in place (materials, lights, object power, emitter CDF, wetness / rain / fog, camera). Anything structural rebuilds the GPU scene from the world, in memory; history survives, validated per pixel. |
| Scene runtime | `renderer/src/script.rs` `ScriptHost` | QuickJS (rquickjs) with no I/O (no fs, network or timers). Loads a bundle (one script) or ES modules (`game.js` + `threert-runtime.js`, the cloud layout). Memory capped at 1 GiB. Time budgets: setup 120 s, event 1 s, frame 50 ms; the interrupt handler aborts overruns. A prelude adds `TextEncoder`, `structuredClone`, `atob` and `console`. |
| Node host | `packages/gameplay/src/node-host.ts` | `globalThis.__threert`: `setup(room, seed)`, `event(type, payload, ts)` (local authority: execute + commit), `commit(writes, seq)` / `setState(state, seq)` (room authority), `state()`, `frame(dt, now)`, and the editor's `outline()`, `inspect(name)`, `edit(name, path, value)`. |
| World handlers | `packages/gameplay/src/world.ts` | `on[type](world, ev)`: an event handled on this node, changing the scene at once. `WorldContext`: `scene`, `camera`, read-only `state`, `object(id)`, `time`, `now`, `random()`, `emit(type, payload)`. |
| Frame scripts | `packages/gameplay/src/frame.ts` | `FrameContext`: `time`, `dt`, `snap`, `now`, read-only `state`, `scene`, `camera`, `object(id)`, `random()`. |
| Game session | `renderer/src/script.rs` `GameSession` | A runtime + its live view: events, clicks (CPU pick → `click {target, point, ray: {origin, direction}}`, with `instanceId` on an `InstancedMesh`; `ray` is the view ray, for a `Raycaster`), frame script, the physics world's ticks, editor edits, each followed by a commit. Shared by `play`, `live` and the playground. |
| Physics | `renderer/physics-core` (`Physics` on Rapier, `bridge::Bridge`), `renderer/src/physics.rs`, `packages/scene` (`Object3D.physics`, joints, `Raycaster.intersectPhysics`, `Shapecaster`) | The session's physics world, made by the first physics call (none: nothing runs). The engine steps it at 1 / 60 s after the frame script (at most 5 ticks a frame; offline renders step to their scene time) and places the objects of the bodies that moved: instances, so a move is a TLAS transform. Scripts cannot step it. See "Physics" below. |

## Entry points

```
threert-render play <bundle.js> [--assets DIR]          native window; keys / clicks go to the game
threert-render live <bundle.js> [--assets DIR]          headless 60 fps; {"cmd":"event"} / {"cmd":"click"} on stdin
threert-render render <bundle.js> -o out.png [--state s.json] [--camera {position,target,fov}] [--time t] [--preset p]
threert-render serve                                    JSON-lines jobs {game, runtime?, assets, room, seed, state, ...}
threert-render setup <bundle.js>                        setup cost in the runtime
threert-render edit-check <bundle.js> --events [...]    in-place edits == a fresh build
node tools/bundle-game.ts <game.ts | scene.ts>          a scene module (default export () => {scene, camera, options})
node tools/build-scenes.ts                              bundles every catalog entry to out/bundles/<id>.js
```

`serve` keeps one runtime per (code version, room, seed): the first job of a room runs setup, later ones only apply
the job's state (`setState`, settled by the frame script). A job whose scene reads files that are not under the asset
root yet fails with `missing: [...]`; the node fetches them and retries.

The playground runs the opened scene the same way: its host holds the `GameSession`; the Node backend keeps the
catalog, the selection, the undo stack (Details edits as `{node, path, before, after}`) and the Play flow. Stop
re-runs setup and replays the editor's edits (UE PIE: Play changes are discarded).

## World handlers

A handler is a transaction on the JSON state and may run in a room, where there is no scene and no physics world; the
scene then follows the state in `frame`. For an event whose point is the scene itself, that is a detour: a click that
adds a box had to be recorded in the state by a handler and acted on by the frame script. A world handler is the
event's handler on the node that owns the scene:

```ts
export default defineGame({
  state: {},
  setup,
  on: {
    click(world, ev) {
      raycaster.set(new Vector3(...ev.ray.origin), new Vector3(...ev.ray.direction));
      const hit = raycaster.intersectPhysics()[0];
      if (!hit) return;
      const box = new Mesh(geometry, wood);
      box.position.copy(hit.point).addScaledVector(hit.normal, 1.5);
      box.physics = "dynamic";
      world.scene.add(box);                      // in the engine, with its body, when the handler returns
      world.emit("placed", { by: ev.player });   // a state transaction, if the game keeps a score: handlers.placed
    },
  },
});
```

- **Where:** on the node that owns the scene and the physics world (`play`, `live`, the playground: wherever events
  arrive at a node). Not in a room.
- **When:** as the event arrives, synchronously, before the engine's next physics tick. What it changed goes to the
  engine through the scene binding when it returns: no commit, and `frame` is not run again for it.
- **The context** (`world`): `scene`, `camera`, the committed `state` (read-only), `object(id)`, `time` (the frame
  clock, s), `now` (the event's time, ms), `random()` (the event's own stream; `Math.random` and `Date` in the handler
  are the event's too), and `emit(type, payload)`, which starts a state transaction (`handlers[type]`).
- **With `handlers[type]` for the same event:** the world handler runs first, on the state as committed before the
  event; then the handler's transaction; then what the world handler emitted, in order; then the frame script shows
  the new state. A room that is the authority runs `handlers` on its own, so neither may rely on the other having run.
- **`keys`** map to either; `describe()` lists them as `on`; a game of world handlers only needs no `handlers`.
- **Synchronous:** no await, as for handlers.
- **A world handler that throws** does not stop the world, as a frame script that throws does not: what it had changed
  is synced, the event's handler and the next tick run all the same, and the error is in the event's log.
- **In a room:** they do not run. A node that follows a room gets committed state, not events; an event with a world
  handler only, sent to a room, is an error there that says so. Who owns the physics when a room is the authority is
  not decided (ROADMAP § 6 step 9); today world handlers are for a node that is its own authority.
- **What they add is not in the game state.** A state restored on another node (`setState`, a render of a room's
  state, `f.snap`) does not bring it back. Every change they make passes through the scene binding, which is where it
  will be logged and replayed from (step 9); until then, keep in the state what must survive a restore.
- **Queries and the scene's queue** (this holds for the frame script too):
  - What a script changes in the scene (`scene.add`, a move, `physics = ...`, an impulse) reaches the engine when the
    script returns. A query in the same handler asks the engine as it was before: it does not see the box the handler
    just added.
  - A query sees everything the engine has been told, with or without a physics tick since: the next event's query
    sees that box, also when no frame passed between the two events, and a body moved by a script is found where it
    was put.
- An event with neither a handler nor a world handler is the error it was.

## Physics

Scene code describes bodies on objects, three.js style; the engine owns the world (ROADMAP § 6 step 7).

```ts
const crate = new Mesh(new BoxGeometry(0.5, 0.5, 0.5), wood);
crate.position.set(0, 3, 0);
crate.physics = "dynamic";                       // or { type, shape, density | mass, friction, restitution, ccd, ... }
scene.add(crate);                                // in the scene it has its body; scene.remove(crate) ends it

const frame = new Mesh(new BoxGeometry(0.1, 2, 0.1), wood), door = new Mesh(new BoxGeometry(1, 2, 0.05), wood);
frame.physics = "fixed";
door.physics = "dynamic";
door.position.set(0.55, 0, 0);
scene.add(frame, door, new HingeJoint(frame, door, { anchorA: new Vector3(0.05, 0, 0), anchorB: new Vector3(-0.5, 0, 0), limits: [-1.5, 1.5] }));

raycaster.set(origin, direction);                // or setFromCamera
const hit = raycaster.intersectPhysics()[0];     // { object, point, normal, distance }
if (hit) hit.object.applyImpulse(new Vector3(0, 40, 0), hit.point);
crate.position.y;                                // the body's pose, read when asked
crate.position.set(0, 5, 0);                     // a write moves the body there
```

- Bodies: `"dynamic"`, `"fixed"`, `"kinematic"` (moved by the script through its `position` / `quaternion`; it pushes
  dynamic bodies). The collider comes from the mesh's geometry (`BoxGeometry` a box, `SphereGeometry` a sphere,
  `CapsuleGeometry` a capsule, any other its convex hull, or its triangles for a fixed body) and the object's scale;
  `shape` overrides it. An object that is not a mesh takes a shape with sizes. A hidden object keeps its body.
- Methods of the object: `applyImpulse(impulse, point?)`, `applyTorqueImpulse`, `applyForce` / `applyTorque` (for the
  frame's ticks: call them every frame they act), `setLinearVelocity`, `setAngularVelocity`, `getLinearVelocity`,
  `getAngularVelocity`, `sleeping`. `scene.gravity` is the world's.
- Joints are objects: `FixedJoint`, `HingeJoint`, `SliderJoint`, `BallJoint`, `RopeJoint` (`anchorA` / `anchorB` in
  each object's space, `axis` in the first object's, `limits`, `motor`, `length`). They hold while they and both their
  objects are in the scene, and join the objects as they are turned when made (limits count from there).
- A `physics` no body can be made from (a flat geometry's hull, a size of 0, a number out of its bounds) throws from
  the sync once, names the object, and leaves it without a body until `physics` is set again.
- Game code is untrusted, so the engine checks every number (in `physics-core`, whoever calls it) and refuses a call
  with one outside its bounds, with the reason:

  | what | bound |
  | --- | --- |
  | positions (bodies, static colliders, joint anchors, impulse points, query origins, trimesh vertices) | within ±100 km |
  | sizes (half extents, radii) | 1 mm to 10 km; a capsule under 1 mm long is its sphere |
  | a collider's offset on its body, a convex hull's points | within ±10 km; a hull holds at least 1 mm³ |
  | velocities set by a call | 10 km/s, 1,000 rad/s (Rapier itself caps what a body reaches at 400 m/s) |
  | impulses, forces | 10⁶ N s (N m s), 10⁸ N (N m) |
  | a collider's mass, density | 0, or 1 g to 10,000 t; 0, or 1 g/m³ to 100 t/m³ |
  | gravity scale, gravity | ±100, ±1,000 m/s² a component |
  | damping, friction, restitution | 0 to 1,000, 0 to 1,000, 0 to 1 |
  | joint limits, a rope's length | within ±100 km (rad for a hinge), min ≤ max; 0 to 100 km |
  | a motor | ±10 km/s (rad/s), damping 0 to 10⁹, a force from 0 |
  | rotations, axes, directions | finite and not zero (any size: they are normalized) |
  | a query's reach | cut to 1,000 km |
  | counts | 20,000 bodies, 40,000 colliders and joints; a trimesh 1 M triangles, a hull 4,096 points, all shapes 4 M points and triangles; 256 hits a ray; 8,192 contacts held between frames (the oldest go) |

  A triangle mesh is for fixed bodies only; a joint joins two different bodies; queries take a box, sphere or capsule.
- What the simulation itself drives out of bounds is taken out of the physics world after a tick, with its colliders
  and joints. The object stays in the scene where its body last put it, without a body until its `physics` is set
  again:
  - A body more than 1,000 km from the origin left the world (it fell off the level; free fall gets there in 42
    minutes, Rapier capping a body at 400 m/s). Nothing is wrong: its object gets an `outofworld` event (before the frame
    script, as contacts do), and the game decides: `scene.remove(object)`, or put it back and set `physics` again.
  - A body whose pose or velocity is no longer finite is an error (the simulation blew up): the next sync throws once,
    naming the object and why.
- A frame script (or its sync) that throws does not stop the world: in a window and in `live` the error is printed,
  the scene is synced as the script left it, and that frame's physics ticks and commit are made all the same (so the
  next frame reads the bodies' new poses). A `serve` job with frames fails with the error.
- A call the engine refuses changes nothing (an impulse with a torque whose point is out of bounds applies neither).
- A tick has no time bound yet: thousands of bodies made at one point take seconds (ROADMAP § 6 step 7, Open).
- Queries: `Raycaster.intersectPhysics({ exclude })` (every body the ray passes through, nearest first, with the
  surface's normal) and `Shapecaster` (`overlap()`, `sweep(direction)`) for a box, sphere or capsule. They see the
  world as the engine has it, with or without a tick since the last change; what the running script itself changed
  reaches the engine when it returns (World handlers, above).
- Contacts (`physics.contacts`): once a frame, `contactstart` / `contactend` events on the two objects
  (`event.other`) and one `contacts` event on the scene with the whole batch. They come before the frame script when
  the game has one, and every frame without one too (a game of world handlers, or listeners added in `setup`): a
  scene with bodies has the node host run every frame.
- Cost: a body moves its object in the engine; no script runs for it. Poses reach the script only when read: the first
  read of a frame fetches one typed array of every body's pose, each object then takes its seven numbers on its own
  first read. Numbers: ROADMAP § 6 step 7.
- The engine call under it is `__engine.physics(op, args)` (JSON; `renderer/physics-core/src/bridge.rs` lists the
  calls, `faults` and `state`'s `{ removed }` among them) and `__engine.physicsPoses()`.

## The runtime's shape: Rust owns the world, the script engine is a guest

Decided by the user on 2026-11-10, with the measurements below.

- **Scripts are three.js-style TypeScript and nothing else.** An object is a `Mesh`; `mesh.physics = "dynamic"` and
  `scene.add(mesh)` put it in the physics world. No world, no step, no sync loop in the script.
- **Rust is the main runtime.** It owns the scene, the physics world, the voxel data and the fixed 1 / 60 s tick. The
  script decides what to add, push and remove, and when.
- **The boundary is four channels**, and it is the contract that stays when the script engine changes:

  | channel | direction | when | for |
  | --- | --- | --- | --- |
  | commands | script → Rust | queued in the script, sent once when it returns (the scene binding's sync) | many small writes (`scene.add`, impulses, joints) in a defined order; what step 9 logs |
  | poses | Rust → script | one table (7 floats a body), fetched at most once a tick; an object takes its own seven on first read | thousands of bodies move, the script reads a few; a body nobody reads costs the script nothing |
  | events | Rust → script | batched, once a frame | contacts, bodies that left the world: no polling |
  | queries | script ⇄ Rust | at once | rays, overlaps, sweeps: the script needs the answer to go on |

  A body's pose goes from the physics to the renderer (instance record, TLAS) without passing through the script.
- **The script engine can be swapped; the `Engine` interface is not tied to one.** Natively it is QuickJS embedded in
  the Rust application, as now, and nothing in the design needs a JIT.
- **On the web: layout C.** The Rust engine (scene, Rapier, voxels) is compiled to wasm, and the gameplay TypeScript
  runs in the browser's own JS engine, calling the wasm module's exports: the same game code and the same four
  channels, with the `Engine` interface implemented by the wasm exports and a thin layer of JS. Not built yet (it is
  what follows the world handlers; ROADMAP § 6). The layouts considered:

  | layout | what | why not / why |
  | --- | --- | --- |
  | A | everything in one wasm module: QuickJS compiled in beside the Rust engine | the same structure as native and a sandbox for free; scripts 1.5–1.8× slower than natively, heavy script code 190–478× slower than Rust, a C toolchain in the build; built and run in Node only (below) |
  | B | three.js as the runtime in the browser, physics in JS | physics, voxel edits and every later system written twice, and no determinism shared with the native build |
  | **C** | Rust engine in wasm, scripts in the browser's engine | **chosen.** One Rust engine, a pure Rust build, a call across the boundary costs about 1 ns, heavy script code about 3× Rust. The binding's glue is written a second time (wasm exports + thin JS), and isolating the script (a Worker or an iframe) is ours to do |
- **Heavy computation has standard ways out of the script**, because QuickJS is an interpreter (natively too):
  1. the engine's own operations in Rust (voxel edits, meshing, connectivity, procedural geometry, physics queries):
     one call from the script;
  2. WGSL program contracts (geometry functions, GPU particles, density-grid programs): the game gives a piece of
     shader, the engine runs it on the GPU;
  3. a wasm compute module the game brings (arrays in, arrays out): **not built**; it waits for a real need.

  A script's time is in the frame log, so an author sees when their TypeScript is the cost.

### The measurements behind it (runtime probe, 2026-11-10)

Conditions: Apple M5 Pro, macOS 26.5, rustc 1.96, release profile as the renderer's; rquickjs 0.12.2 (QuickJS-ng
0.15.1); **Node 24.21 (V8 13.6) stands in for a browser, and nothing was run in a real browser**; single-threaded,
medians, on a machine another agent was using (load 3–5): differences under about 10 % are noise, and ratios are worth
more than the milliseconds. The harness and the full report are not in the repository (a copy of the report is kept
beside the checkout, `out/runtime-probe/REPORT.md`).

| crossing the boundary | native QuickJS | QuickJS in wasm (in Node) | V8 calling a wasm export |
| --- | ---: | ---: | ---: |
| a call with no argument | 18 ns | 26 ns | 1.2 ns (3.0 ns without V8's inlining of the call) |
| a call with three f64 | 37 ns | 67 ns | 0.9 ns (3.6 ns) |
| reading 1,000 bodies' poses from one table | 0.096 ms | 0.148 ms | 0.005 ms |
| reading 10,000 bodies' poses from one table | 0.94 ms | 1.48 ms | 0.051 ms |
| reading 10,000 poses, a call and a new array per body | 4.24 ms | 4.18 ms | not measured |

One table is 4.5× cheaper than a call per body, which is why poses are a table. Of QuickJS's 0.94 ms for 10,000
poses, the script's own read loop is 0.97 ms (the same within noise): the interpreter reading the numbers is the
cost, not the crossing. For orchestration, all three are enough in a 16.67 ms frame.

| the same algorithm, ms (× native Rust) | heightfield 256², mesh | greedy mesh of 64³ voxels |
| --- | ---: | ---: |
| Rust, native | 0.58 (1×) | 1.25 (1×) |
| Rust in wasm32, in Node | 0.65 (1.1×) | 1.92 (1.5×) |
| JS in V8, warmed up | 1.64 (2.8×) | 4.05 (3.2×) |
| JS in native QuickJS | 171 (295×) | 135 (108×) |
| JS in QuickJS in wasm | 273–277 (472–478×) | 237–238 (190×) |
| JS in Boa (a JS engine in Rust), native | 696 (1,202×) | 696 (557×) |

Every configuration produced the same output hash. A 64³ mesh in QuickJS is 8 to 14 frames: hence the ways out
above, natively as much as on the web. Authoring is checked natively (QuickJS), so under layout C a script is only
ever faster for a player than it was for its author.

Building a script engine into the wasm module (layout A), as far as it was taken:
- rquickjs does not build for `wasm32-unknown-unknown` with plain cargo (no libc headers for the target). It builds
  for `wasm32-wasip1` (wasi-sdk, fetched by its build script), and into one `wasm32-unknown-unknown` module beside
  wasm-bindgen with a recipe (QuickJS's C compiled by wasi-sdk's clang, wasi-libc linked, five WASI imports shimmed in
  JS): 1.2 MB, run in Node, same hashes.
- Boa builds for `wasm32-unknown-unknown` with one feature (10.4 MB) and is 4–8× slower than QuickJS.
- Not verified by the probe: a real browser; wgpu in the same module as a script engine; the software BVH traversal's
  frame rate in a browser (WebGPU has no hardware ray queries: the web's largest unknown, whichever layout).

## Correctness

* `threert-render edit-check` builds the GPU tables both ways for a list of events: edited in place, and built fresh
  from the edited world. For Kabukichō (three moods, closing time, the gate, rain) materials, lights, object power, the
  emitter distribution and wetness / rain are bit-identical, and none of the edits is structural.
* Every reference scene and Kabukichō render from their bundles like they did from the old IR packages (same
  triangle, light, emitter and texture counts; at 64 spp mean ΔE 0.0 (material-grid, bit-identical) to 0.99, and
  1.43 for Kabukichō, where the error map is sampling noise only). The six UE cameras (`tools/kabukicho/views.py`,
  now `render --camera`) frame identically.
* `tools/kabukicho/verify-port.py` (against `node tools/kabukicho/dump-street.ts`) still matches every UE actor
  (2,158 shapes, 0.0000 m).
* Playground UI test (`threert-playground --uitest`): 14/14, including Play / Stop in Light Room.
* Cloud: `node cloud/test/e2e-local.ts` and `play-smoke.ts` pass (rooms hold state only; nodes run the game code
  against the room's state; a cold job fetched code, runtime and the missing asset, then rendered; a warm event →
  PNG takes 124 ms with 0.5 ms of script).
* Live temporal bench (`tools/bench-temporal.ts --scale 1`, same converged references as PR #9): camera motion
  unchanged (ΔE 5.29–6.25 vs 5.21–6.38); 4 s after closing time ΔE 4.00 vs 3.90. The earlier closing shots differ
  by design: each sign group now stays lit until its staggered delay and then flickers off, like the UE Director,
  where before the handler switched every group off at once.

## Kabukichō: the UE Director as a frame script

The UE session's `AKabukichoDirector` (C++) ticks every frame. Rain eases at 0.6 / s. Streets get wet at 0.35 / s
and dry at 0.06 / s. Fog eases toward the mood. Power groups that change state flicker like failing neon for
0.35–1.1 s (on) or 0.08–0.3 s (off), and switch with staggered delays when many change at once: moods, the 3 AM
closing, the power cut. `games/kabukicho/kabukicho.game.ts` ports it line for line:

- **Handlers** commit the Director's targets as game state: `rain` (R), `mood` (M), `blackout` (B), `lights` (L),
  `click` (a sign's power group, the gate), `viewpoint` (V).
- **`frame(f)`** makes the street follow them: group powers, gate emission, wetness / rain / fog, the camera marker;
  settled at once when `f.snap`.

![Kabukichō in the node runtime: start · power cut 0.15 s · 1.3 s · restoring 4.8 s](images/runtime-blackout.jpg)

## Measurements (Apple M5, 1280×720, Kabukichō: 4.0 M triangles, 2,665 objects, 147 lights)

| | IR patches (PR #8) | no IR |
|---|---:|---:|
| Scene setup in QuickJS | 345–381 ms (IR + generated GLBs / PNGs) | 255–266 ms (engine calls, no encoding) |
| Gameplay event → scene updated | patch 0.01–4.8 ms | handler + frame(0) + sync 6–13 ms; commit in place 0.00–0.45 ms |
| Frame script (Director port) + sync + commit, per frame | 0.12–1.1 ms (presentation writes) | 0.4–0.8 ms average, 2.1 ms max (7–8 ms before change tracking) |
| Generated-texture scene setup (glb-asset) | — | 43 ms (732 ms when DataTexture still encoded PNG in JS) |
| One-shot render of another room state (`serve`, warm, 640×360, 16 spp) | 0.88 s | 1.23 s (3.6 ms script) |
| Live open (playground) | 5.2 s | 5.8 s (setup 0.26 s, build 3.1 s, texture pages 1.3 s, upload 1.7 s) |

QuickJS is about 8× slower than V8 for this kind of code. It matters for setup-heavy content (large procedural
geometry in JS), where the runtime could move to V8 (deno_core) behind the same `ScriptHost` interface.

## Next

1. **Movable objects without rebuilds.** Transforms and visibility become TLAS instance updates for objects that
   scripts move (UE Mobility = Movable), e.g. the UE session's crowd (46 walkers, 14 loiterers) and vehicles. They
   are structural today: fine for small scenes or occasional changes, not every frame on a big street.
2. **Warm room sessions on nodes.** `serve` keeps runtimes per room already; a node could also keep the GPU scene of a
   room between jobs and apply state changes in place, as the live view does.
3. **Kernel variant warm-up.** An edit that enables a kernel feature (e.g. a dry street getting wet) compiles that
   variant on first use; pre-compile likely variants in the background.
4. **UE references for the simple scenes** came from the removed IR adapter (cached in out/ue). New ones need either
   the scenes rebuilt in UE by hand or an engine-API binding on the UE side (the scene code driving UE actors).
