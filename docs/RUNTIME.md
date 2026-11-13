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
| Live commit | `renderer/src/live/edits.rs` `LiveCore::commit`, `tables.rs` | Parameter changes rewrite GPU table entries in place (materials, lights, object power, emitter CDF, wetness / rain / fog, camera), and so does hiding or showing a volume, a light without a card or a particle system that is not light-sampled by its own `visible`. Anything structural rebuilds the GPU scene from the world, in memory; history survives, validated per pixel. |
| Scene runtime | `renderer/src/script.rs` `ScriptHost` | QuickJS (rquickjs) with no I/O (no fs, network or timers; what needs the outside is asked of the app, "The host channel"). Loads a bundle (one script) or ES modules (`game.js` + `threert-runtime.js`, the cloud layout). Memory capped at 1 GiB. Time budgets: setup 120 s, event 1 s, frame 50 ms; the interrupt handler aborts overruns. Every entry ends by running the promise jobs it left, inside its budget ("Promises", below). The time the engine spends inside a voxel call (`voxelExplode`, `voxelEdit`, `voxelFrame`) is the engine's, not the script's: the deadline moves by it as measured, up to 250 ms an entry; past that it counts against the script, and a voxel call made after the deadline throws "interrupted" at once. A prelude adds `TextEncoder`, `structuredClone`, `atob` and `console`. |
| Node host | `packages/gameplay/src/node-host.ts` | `globalThis.__threert`: `setup(room, seed)`, `event(type, payload, ts)` (local authority: execute + commit), `commit(writes, seq)` / `setState(state, seq)` (room authority), `state()`, `frame(dt, now)`, `hostMessages(list, ts)` (the app's messages), `sync()` (the engine follows the scene once more: after an entry's promise jobs), and the editor's `outline()`, `inspect(name)`, `edit(name, path, value)`. |
| World handlers | `packages/gameplay/src/world.ts` | `on[type](world, ev)`: an event handled on this node, changing the scene at once. `WorldContext`: `scene`, `camera`, read-only `state`, `object(id)`, `time`, `now`, `random()`, `emit(type, payload)`. |
| Frame scripts | `packages/gameplay/src/frame.ts` | `FrameContext`: `time`, `dt`, `snap`, `now`, read-only `state`, `scene`, `camera`, `object(id)`, `random()`, `input`. |
| Host channel | `packages/gameplay/src/host.ts`, `input.ts`, `hud.ts`; `renderer/src/host.rs`, `script.rs` (`__host_post`, `GameSession::take_posts` / `host_messages`) | Messages between game code and the app that embeds the engine: the app's input every frame (`f.input`), sockets and HTTP the app owns (`net.*`), the HUD the app draws (`hud.set`), how the game was opened (`launch`). See "The host channel" below. |
| Game session | `renderer/src/script.rs` `GameSession` | A runtime + its live view: events, clicks (CPU pick → `click {target, point, ray: {origin, direction}}`, with `instanceId` on an `InstancedMesh`; `ray` is the view ray, for a `Raycaster`), frame script, the physics world's ticks, editor edits, each followed by a commit. Shared by `play`, `live` and the playground. |
| Physics | `renderer/physics-core` (`Physics` on Rapier, `bridge::Bridge`), `renderer/src/physics.rs`, `packages/scene` (`Object3D.physics`, joints, `Raycaster.intersectPhysics`, `Shapecaster`) | The session's physics world, made by the first physics call (none: nothing runs). The engine steps it at 1 / 60 s after the frame script (at most 5 ticks a frame; offline renders step to their scene time) and places the objects of the bodies that moved: instances, so a move is a TLAS transform. Scripts cannot step it. See "Physics" below. |

### Promises

QuickJS has no event loop of its own: a promise's callbacks are jobs that somebody has to run. The runtime runs them
at the end of every entry into the script (`ScriptHost::enter`: the loading of the bundle, setup, an event, a frame,
a batch of host messages, `commit` / `setState`, `seek`, an editor call), until none is left: a `.then` callback, the
code after an `await`, and the jobs those queue in turn. That is when a browser runs them: when the script that was
called has returned.

```ts
// A request as a promise (what a library makes of `net.fetch`): the app's answer is a later message.
const answers = new Map<string, (m: HostEvents["net.fetched"]) => void>();
host.on("net.fetched", (m) => answers.get(m.id)?.(m));
const fetched = (id: string, url: string) =>
  new Promise<HostEvents["net.fetched"]>((resolve) => {
    answers.set(id, resolve);
    host.post({ t: "net.fetch", id, url });
  });

async function load(url: string, scene: Scene) {
  const m = await fetched("level", url);        // goes on in the entry of the batch that brings `net.fetched`
  scene.add(build(JSON.parse(m.body)));         // in the engine when that entry returns
}
host.on("launch", (m, world) => void load(String(m.params.level), world.scene));   // the listener returns no promise
```

- **A promise resolves at the end of the entry that resolved it.** `Promise.resolve(x).then(f)` in a frame script
  runs `f` in that frame, before the frame's physics ticks and its commit; so does the code after an `await` of a
  promise that is settled by then, in a frame script, a handler, a world handler, a listener or setup.
- **There are still no timers and no I/O**, so nothing resumes on its own. A promise only the app can settle (the
  request above) stays pending through the frames in between, and its awaiter runs in the entry that settles it: a
  host message, an event, a frame. One that nothing settles never goes on.
- **The jobs run when the entry's own code is over**, not inside it. For the code after an `await` the event has
  ended: `Math.random` and `Date` are the host's again, a `world` shows the state as it was, and its `emit` throws
  (the handler's emits were taken when it returned; start a state transaction while a handler or a listener runs,
  for example in the listener that hears the answer). The scene is the scene: what the jobs changed in it goes to the
  engine when they have run (the runtime calls the node host's `sync()` after them; the entry's own sync came
  before), so the commit that follows the entry applies it.
- **`setup`, handlers, world handlers and host listeners return no promise themselves.** An `async` function as one
  of them is the error it was (`handlers must be synchronous`, `world handlers must be synchronous`, `host listeners
  must be synchronous`), and an `async` setup is one now too (`setup must be synchronous: it returns { scene, camera
  }, not a promise`): setup is not awaited, a scene is built when it returns. What any of them starts and does not
  wait for (`void load()`, a `.then`) runs as above; from setup that is at the end of the setup entry, and may add to
  the scene setup returned. A frame script's return value is not looked at.
- **Inside the entry's budget.** The deadline of the entry holds for its jobs: one that spins is interrupted, and a
  chain that never ends (`const again = () => Promise.resolve().then(again)`, an `async` loop of awaits) is ended
  when the time is up. The entry then fails as one that overran does (`interrupted`). The jobs not run stay queued
  and run after the next entry, so a chain that queues itself for ever fails every entry until the script stops it.
- **Errors.** What a callback throws rejects its promise. A promise rejected that nothing has handled when the
  entry's jobs have run is reported once, as an error line with its reason: `unhandled rejection: TypeError: ... (at
  ...)` in the game's log (stderr `[game error]`, the log sink), and for an event in the event's log (`✗ unhandled
  rejection: ...`). At most eight an entry by their reasons; the rest are counted. One that gets its handler later
  in the same entry (a `catch` added by a later job, an `await` in a `try`) is not reported; one handled in a later
  entry was.
- **In a browser** (`renderer/web`, the three.js view) the page's engine runs the jobs, at the same point, so game
  code behaves the same there. Nothing calls `sync()` after them: what they changed in the scene goes to the engine
  with the next entry's sync (the next frame's, where frames run).
- Checked: `renderer/src/script.rs` tests (a `.then` chain and an `await` in a frame, an event and a batch of host
  messages run within the entry, and what they changed is synced after them; a promise a later host message resolves;
  a chain that never ends; rejections reported once), `packages/gameplay/test/promises.test.ts` (the node host's
  `sync`, `emit` after the handler, what stays synchronous), and `node tools/host-echo.ts` (`games/host/echo.game.ts`
  awaits its page as the promise above, in `live`, `play` and Chrome).

## Entry points

```
threert-render play <bundle.js> [--assets DIR]          native window; keys / clicks go to the game
                    [--launch JSON] [--hud-log] [--sensitivity N]     the host channel: input, sockets, the HUD printed
threert-render live <bundle.js> [--assets DIR]          headless 60 fps; {"cmd":"event"} / {"cmd":"click"} on stdin
                    [--launch JSON]                     {"cmd":"host","messages":[..]} in, {"host":[..]} lines out
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
  are the event's too), `emit(type, payload)`, which starts a state transaction (`handlers[type]`), and `input` (what
  the player holds, for a game that asked for input: "Input", below).
- **With `handlers[type]` for the same event:** the world handler runs first, on the state as committed before the
  event; then the handler's transaction; then what the world handler emitted, in order; then the frame script shows
  the new state. A room that is the authority runs `handlers` on its own, so neither may rely on the other having run.
- **`keys`** map to either; `describe()` lists them as `on`; a game of world handlers only needs no `handlers`.
- **Synchronous**, as handlers are: a world handler returns no promise (an `async` function as one is an error).
  What it starts with promises runs when the event is over ("Promises", above): the scene it changes there is
  synced, but `world.emit` throws.
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
- **Voxels** (ROADMAP § 6 step 8; docs/VOXEL.md "Voxel bodies"): a voxel level collides without a `physics` on
  anything. Its objects are fixed colliders of their voxels and what an edit cuts loose is a dynamic body, both of
  the engine's own shape (`physics_core::Shape::Voxels`: parry's voxel shape; a collider's cells are changed where
  it is by `Physics::edit_voxels`, at a cost by the cells that change). Rays, shape casts and contacts report them
  like any collider. They are **the engine's bodies**: their ids are from 2^30 up, scene code cannot make or remove
  one (`body`, `collider` and `remove` refuse those ids, and a `collider` on one of those bodies is refused too: the
  engine edits and splits them by their cells), and can name one in every other call. **A voxel body is an object
  to scene code** (ROADMAP § 6 step 10 a; docs/VOXEL.md "Voxel bodies as objects"): `voxelBodies()` (`packages/voxel`)
  returns `VoxelBody` objects, the same object for the same body, with the physics methods every body has (sent at
  once), its pose, contacts on request and its own edits; joints take it as one of their objects; a hit of
  `Raycaster.intersectPhysics` or of a `Shapecaster` on it has it for its `object` and carries `collider` and `body`.
  **On the voxel level `hit.object` is `engineColliders` (in no scene, `physics` null) and there is no `body`.** So
  hello world's test for something fixed, `hit.object.physics?.type === "fixed"`, does not find the level: it is
  `hit.object === engineColliders`. A level that collides is in the physics world, and the queries ask it, whether or
  not any scene object has `physics`; contacts with it reach a body's listeners with `other: engineColliders`.
  `voxelImpulse(body, impulse, point?)` (step 8's call by id) is kept, deprecated. A joint's parameters (limits,
  motor, a rope's length) change in place (`joint_set`); a joint may hold to the voxel level (`engineColliders`: the
  world's anchor, a fixed body with no collider); a joint on a voxel body or the level follows the cell its anchor is
  in when bodies are cut (its `bodyA` / `anchorA`, or `bodyB` / `anchorB`, then name the part it is on), and is
  `broken` (a "broken" event) when that cell is carved or its body goes. An explosion's blast pushes the bodies near
  it, and kicks the parts it cuts loose (`push`; docs/VOXEL.md "Joints, the level's anchor and the blast", step 10
  c1). An explosion's fire and smoke are
  `Explosions` (`packages/voxel`, step 10 b; docs/VOXEL.md "Fire and smoke"): scene objects pooled in setup, no engine
  call of their own. A voxel shape's limits: cells of 1 cm to 100 m, at most 2^24
  cells, each within 2^20 cells and 100 km of the shape's origin.
  - Two things the world does itself for voxel shapes, because Rapier 0.36 and parry 0.31 do them wrongly
    (`physics-core/src/world.rs` has the tests): rays are walked cell by cell (parry's ray misses every cell when it
    lies on a plane of the lattice and runs along it), and a ball against voxels goes through parry's general voxel
    contacts (its own function for that pair makes Rapier's solver panic when a contact goes). Neither touches a
    world without voxel shapes: step 7's worlds end in the bits they did.
  - Cost: asleep, nothing. While they move, Rapier's step on one thread: 1.4 ms a tick for a hundred jagged pieces
    of 200 voxels in a heap, under 0.5 ms for one building part of 96 k voxels landing. On the benchmark level
    (hardware traversal) one thread holds 60 fps to about a hundred large bodies moving at once (the step is then
    10–13 ms).
  - **Natively the world is stepped by Rapier's parallel solver**, in a pool of the physics' own: 4 threads, or the
    machine's performance cores when it has fewer; `THREERT_PHYSICS_THREADS` sets another count, and 1 (0, off) is
    sequential. It gives the sequential build's bits (which are wasm32's) at a third of the step's time: about
    450–500 bodies moving in a frame, and a level's cap of 300 voxel bodies (`maxBodies`) by measurement. wasm32
    is sequential. How far "the bits do not depend on the thread count" was checked, and what was not: docs/VOXEL.md
    "Voxel bodies", ROADMAP § 6 step 8 "Open".
  - **A step is waited for with a kick** (`physics.rs` `in_pool`): Rapier's parallel step, made on a pool's worker,
    can strand a job on that worker's deque and wait for it for good (about once in 10^5 steps when steps are
    microseconds apart). While the step is out, the waiting thread injects an empty job into the pool every 0.5 ms,
    which wakes a worker that then takes the stranded one. The frame log's `physics.kicks` counts them (long steps
    get a few; they change nothing). `THREERT_PHYSICS_KICK=off` waits without, to reproduce the hang;
    `cargo run --release --example physics_fuzz` is the CPU check, with a watchdog.
  - Ids in a call (`body`, `collider`, `skip`, ...) are whole numbers however they are written: scene code hands
    back an id it read from an answer as a double.

## The host channel

A script has no I/O and no timers, and the engine draws the scene and nothing else. What a game needs beyond that
belongs to the app that embeds the engine: the keys and the pointer, sockets, a HUD over the picture, how the game
was opened. The host channel is how the two talk: messages, JSON objects with a string `t`, queued one way and
handed over in batches the other.

```ts
import { defineGame, host, hud } from "@threert/gameplay";

host.on("launch", (m) => host.post({ t: "net.open", id: "room", url: `wss://example.org/rooms/${m.params.room}` }));
host.on("net.messages", (m, world) => {          // on the node that owns the scene, like a world handler
  for (const text of m.list) world.emit("remote", JSON.parse(text));   // a state transaction: handlers.remote
});

export default defineGame({
  pointer: "look",                               // this game reads input and owns its camera
  frame(f) {
    yaw -= f.input.look.dx;
    if (f.input.pressed("Mouse0")) host.post({ t: "net.send", id: "room", data: JSON.stringify({ fire: true }) });
    hud.set({ crosshair: { style: "cross" }, items: [{ type: "text", anchor: "top-left", label: "Score", text: String(f.state.score) }] });
  },
  ...
});
```

- **Script → app:** `host.post({ t, ... })` (`packages/gameplay/src/host.ts`) queues the message's JSON text through
  the native `__host_post`. The queue holds 1,024 messages of at most 1 MiB each until the app takes them
  (`GameSession::take_posts`); past that `post` returns false and the message is dropped, and where no app is there
  at all (a test, a page that set no `__host_post`) it returns false too. A session nobody drains (`render`, `serve`)
  keeps the first 1,024 and nothing else.
- **App → script:** the app hands a batch to the session (`GameSession::host_messages`), one script entry for the
  batch (the node host's `hostMessages(list, ts)`), under an event's time budget (1 s). Each message goes to the
  listeners added with `host.on(type, fn)`, in the order they were added; `on` returns the function that removes the
  listener.
- **Listeners run like world handlers** ("World handlers", above): on the node that owns the scene, with the same
  context as their second argument (`scene`, `camera`, the committed `state`, `object(id)`, `time`, `now`,
  `random()`, `emit`, `input`). What they change in the scene reaches the engine when the batch returns, once.
  What they `emit` are state transactions, run after the batch's listeners, followed by the frame script as after
  any event. A listener that throws does not stop the batch: the error is logged and the next listener runs. A
  listener returns no promise; the code after an `await` it started runs at the end of the batch's entry, or of the
  later entry that settles what it waits for ("Promises", above).
- **The same object is `globalThis.__threert_host`** (`{ post, on, launch }`), for a library that does not import
  `@threert/gameplay`. A second copy of the package in one runtime finds the first's channel.
- **It is not one of the four channels** of "The runtime's shape" below: those are between the script and the
  engine. This one is between the script and the app, and the engine only carries it.
- **Every family below is optional.** An app implements what it has; a message it does not know is ignored, both
  ways. A game must go on without an answer (the socket that never opens, the HUD nobody draws), and must not
  expect one within a frame.

Messages a game posts:

| `t` | fields | what the app does |
| --- | --- | --- |
| `net.open` | `id`, `url`, `protocols?` | opens a WebSocket (`ws:` / `wss:`) the game calls `id`; answers `net.opened`, or `net.closed` if it cannot |
| `net.send` | `id`, `data` | sends the text `data`; before `net.opened`, once the socket is open; to a socket that is gone, nothing |
| `net.close` | `id`, `code?`, `reason?` | closes it (1000 unless `code` is 3000–4999); answers `net.closed` |
| `net.fetch` | `id`, `url`, `method?`, `headers?`, `body?` | one HTTP request (`GET` unless `method`); answers `net.fetched` |
| `hud` | `hud` | draws the HUD it describes ("HUD", below). `hud.set` posts it; a game does not post it by hand |

Messages an app sends:

| `t` | fields | when |
| --- | --- | --- |
| `net.opened` | `id` | the socket is open |
| `net.messages` | `id`, `list: string[]` | the text messages that arrived on `id` since the last frame, in order: one batch a socket a frame |
| `net.closed` | `id`, `code`, `reason` | the socket closed (the close frame's code; 1005 without one), or failed (1006, `reason` says why) |
| `net.fetched` | `id`, `status`, `body`, `error?` | the response's status and body as text; `status` 0 and `error` when the request failed |
| `launch` | `params: { room?, name?, ... }` | once, after setup: how the app opened the game. Kept as `host.launch` (`{}` before it comes, and for an app that sends none) |
| `input` | below | once a frame, before the frame script, to a game that asked for input. The node host keeps it; it has no listeners |

`id` is the game's own name for a socket or a request (a string); the app's answers carry it back. Messages are text:
a binary WebSocket frame is dropped. Which addresses a game may reach is the app's decision, not the engine's.

**What an app does** (Rust: whoever holds the `GameSession`; `renderer/src/view.rs` is the example to read):

1. After the scene is built: `session.host_messages(&mut core, &[host::launch_message(params)])`.
2. Every frame, before `session.frame`: one `host_messages` with the frame's `input` message (if the game asked:
   `session.host.pointer()` is `Some("look")` or `Some("free")`) and what its services have for the game (a
   `net.messages` batch a socket, answers). An empty batch is no entry at all.
3. After the frame: `session.take_posts()`, each message to whatever implements its family.
4. For a game with `pointer`: leave the camera alone (a commit makes the view follow the scene's camera, and the
   game moves that) and give up its own keys and pointer gestures. For `"look"`: capture the pointer on a click
   (that click is not the game's), let go on Esc and when the window loses the keyboard, and send the motion as
   `dx` / `dy`.
5. A game without `pointer` gets no `input` and is shown as before: the app's own camera controls, `keys` and
   clicks as discrete events. `keys` and (unless the pointer is captured for look) clicks work for every game.

The apps in this repository:

| app | input | `net.*` | `launch` | HUD |
| --- | --- | --- | --- | --- |
| `threert-render play` (`view.rs`) | keys by physical position, mouse buttons, wheel; the pointer captured for `"look"` (a click; Esc lets go, Esc again quits) | WebSockets and HTTP on a thread each (`src/net.rs`: tungstenite and ureq on rustls; text frames of at most 1 MiB, bodies of at most 4 MiB, 16 sockets, 32 requests at once), any address | `--launch '{"room":"K7Q4MZ"}'` | not drawn; `--hud-log` prints each description |
| `threert-render live` (`live/session.rs`) | whatever the driver sends | none: the driver is the app | `--launch` | on stdout with every other post |
| the three.js view (`tools/three/view.ts`, `host.ts`, `hud.ts`) | keyboard, mouse, the Pointer Lock API | `WebSocket`, `fetch` (the browser's rules: same-origin policy, CORS) | the page's query (`?room=K7Q4MZ&name=Ada`) | DOM over the canvas |

- `live` speaks the channel as JSON lines, so a game's use of it is testable without a window: on stdin
  `{"cmd":"host","messages":[{"t":"net.opened","id":"room"}, ...]}`, on stdout one line `{"host":[...]}` with what
  the game posted since the last one. `play` takes the same command and writes `{"type":"host","messages":[...]}`.
- The sockets of `play` are in the binary and behind the `net` cargo feature (on by default). The library has no
  network in it; a program that embeds it and wants its dependency graph without the three crates sets
  `default-features = false`, as `renderer/web` does. A build without the feature answers every `net.open` and
  `net.fetch` with a failure that says so.
- The three.js view has three.js for the picture and the engine's own physics for the rest (`tools/three/physics.ts`):
  the world and the physics as a wasm32 module (`renderer/examples/physics_scene.rs`: the code `__engine` runs
  natively, without the renderer and without wasm-bindgen), which `node tools/three/open.ts <game>` builds for a
  game whose source names the physics. Bodies move and `intersectPhysics` / `Shapecaster` answer as in the renderer.
  Without the module the view is the baseline it was: bodies stay where the scene put them.
- Checked: `packages/gameplay/test/host.test.ts` (the channel, input, the HUD's change detection, in Node);
  `renderer/src/host.rs` and `script.rs` tests (the queue's bounds, a batch's delivery, the input message);
  `node tools/host-echo.ts` (`games/host/echo.game.ts` in `live` with the script as the app, in `play` against a
  WebSocket and HTTP server on the machine, and in the three.js view in Chrome); `node tools/fps-arena.ts` and
  `node tools/three-arena.ts` (below).
- Not built: the channel in the engine's own page (`renderer/web/www`: its `main.ts` sets no `__host_post` and sends
  no input); binary socket messages; gamepads and touch in the three apps above (the `input` message has their
  fields).

### Input

A game that reads input says so in its definition, and `describe()` reports it:

```ts
defineGame({ pointer: "look", ... })     // or "free"
```

| `pointer` | the app | the camera |
| --- | --- | --- |
| left out | sends no input; its own controls work as before | the app's (orbit, fly), following the game's when a handler moves it |
| `"free"` | sends input every frame; the pointer stays free, `input.pointer` is where it is; a click is still a `click` event | the game's |
| `"look"` | the same, and captures the pointer: its motion is `input.look`; no `click` events | the game's |

Game code reads it as `f.input` in the frame script and `world.input` in world handlers and host listeners
(`packages/gameplay/src/input.ts`):

| | |
| --- | --- |
| `input.down(key)` | the key is held |
| `input.pressed(key)`, `input.released(key)` | it went down / up since the last frame |
| `input.axis(negative, positive)` | -1, 0 or 1 from two keys |
| `input.move` | `{ x, y }`: the app's move stick if its last message had one, else W A S D; x right, y forward, never longer than 1 |
| `input.look` | `{ dx, dy }`, radians to turn this frame: right and down are positive |
| `input.button(n)` | mouse button `n` is held (0 left, 1 middle, 2 right); the same as `down("Mouse<n>")`, and `pressed("Mouse0")` is its edge |
| `input.pointer` | `{ u, v }`, 0..1 from the picture's top left; (0.5, 0.5) while captured |
| `input.wheel`, `input.captured` | wheel notches this frame (away from the player is positive); the app holds the pointer |

The message, every field optional but `t`:

| field | |
| --- | --- |
| `keys: string[]` | every key held now |
| `pressed`, `released: string[]` | keys (and `"Mouse<n>"`) that went down / up since the last message. Left out: derived from how `keys` and `buttons` changed, which misses a tap shorter than a frame |
| `buttons: number[]` | mouse buttons held now |
| `u`, `v` | the pointer |
| `dx`, `dy` | look since the last message, radians |
| `wheel` | notches since the last message |
| `captured` | the pointer is captured |
| `move: [x, y] \| null` | a move stick (gamepad, touch), each -1..1; `null` or left out: none, and `input.move` is the keys' (the one field a message does not keep from the message before: an app with a stick sends it every frame) |

- **Keys are named by their place on a US keyboard**, not by what the layout prints (W A S D stay where they are on
  AZERTY): `"A"`–`"Z"`, `"0"`–`"9"`, `"Space"`, `"Shift"`, `"Ctrl"`, `"Alt"`, `"Meta"`, `"Tab"`, `"Enter"`,
  `"Escape"`, `"Backspace"`, `"Left"`, `"Right"`, `"Up"`, `"Down"`, `"F1"`–`"F12"`, `"Minus"`, `"Equal"`,
  `"BracketLeft"`, `"BracketRight"`, `"Semicolon"`, `"Quote"`, `"Comma"`, `"Period"`, `"Slash"`, `"Backslash"`,
  `"Backquote"`. Either shift key is `"Shift"`. (The discrete `keys` of a game are matched by the character typed,
  as they were.)
- **Look is an angle, not counts.** The app turns its device into radians: a mouse at 0.0025 rad a count (a point on
  macOS, a CSS pixel in a browser) times the player's sensitivity (`play --sensitivity`, the view's `?sensitivity=`),
  a stick's rate times the frame's time. A game applies `dx` / `dy` as they are, and sensitivity is the app's
  setting, where the player's other settings are.
- **A gamepad or a touch screen** sends `move` for its move stick, its look in `dx` / `dy`, and its buttons under
  the names of the keys they stand for (`"Space"` for jump, `"Mouse0"` for fire): a game written for the keyboard
  needs nothing more.
- **Edges last one frame:** from the input message that brought them to the end of the next frame script, so a world
  handler that runs between the two sees them too. Two messages before one frame add up (their edges, their look,
  their wheel). A frame without a message: what was held stays held, nothing is pressed or released, `look` and
  `wheel` are 0.
- **Input is not state.** It is this node's, read by the frame script and world handlers; nothing of it is committed
  or replayed. What a room must agree on goes through `world.emit` (a handler) or to the app's socket.

`games/fps/arena.game.ts` is the reference: a first-person walk with mouse look, a capsule the frame script moves by
shape sweeps (`Shapecaster.sweep`: stop a skin short of what is met, slide along it with what is left, so a long
frame cannot pass a wall), a jump, targets tagged by the view ray (`Raycaster.intersectPhysics`), and its HUD.
Checked on numbers in the renderer with scripted input (`node tools/fps-arena.ts`: `live`, lockstep), on the same
numbers on three.js with the engine's physics in Node (`tools/three/test/physics.test.ts`), and in Chrome with the
events a keyboard and a mouse send (`node tools/three-arena.ts`).

### HUD

The engine draws no text. A game describes its HUD and the app draws it its own way (`packages/gameplay/src/hud.ts`):

```ts
hud.set({
  crosshair: { style: "cross", gap: 0.3 },
  items: [
    { type: "text", anchor: "top-left", label: "Ammo", text: "24 / 90", size: "large" },
    { type: "bar", anchor: "bottom-left", label: "Health", value: 0.72, tone: "good" },
    { type: "list", anchor: "top-right", lines: [{ id: "k17", text: "Ada tagged Lin", ttl: 4 }] },
    { type: "table", anchor: "center", title: "Round 2", columns: ["Player", "Score"], rows: [["Ada", 7], ["Lin", 5]], highlight: 0 },
  ],
  message: { title: "Round 2", subtitle: "Tag every target" },
});
hud.flash("damage", 1.2);        // the player was hit from 1.2 rad clockwise of straight ahead
```

- **The description is the whole HUD.** What is not in it is not shown. A game calls `hud.set` every frame with what
  the frame shows; `{ t: "hud", hud }` is posted only when the description differs from the last one posted (compared
  as canonical JSON), and `set` says whether it was.
- **Nine anchors:** `top-left`, `top`, `top-right`, `left`, `center`, `right`, `bottom-left`, `bottom`,
  `bottom-right`. Items of one anchor stack in their order.
- **Items:**

  | `type` | fields | |
  | --- | --- | --- |
  | `text` | `text`, `label?`, `tone?`, `size?` (`small`, `normal`, `large`) | a figure or a line, with a small label over it |
  | `bar` | `value` (0..1), `label?`, `text?`, `tone?` | health, a reload, a round's time; `text` is a figure beside it |
  | `list` | `lines: (string \| { text, tone?, ttl?, id? })[]` | short lines, newest last. A line with `ttl` is shown that many seconds from when the app first got it (the same `id`, or the same text), then left out, whether or not the game still lists it |
  | `table` | `columns`, `rows`, `title?`, `highlight?` | a scoreboard: listed while it is to be seen (`f.input.down("Tab")`); `highlight` is the row to set apart |
- **`crosshair`:** `style` (`dot`, `cross`, `circle`), `gap` (0..1: how far it opens, a weapon's spread), `tone?`.
  **`message`:** `title`, `subtitle?`, `tone?`, at the picture's centre.
- **`flash`** is the one event: `hud.flash(kind, angle?)` posts the HUD as last set with `flash: { kind, angle?, n }`,
  `n` one more each time. An app plays it when `n` changes: `hit` (a hit marker at the crosshair), `kill`, `damage`
  (the player was hit; `angle`, radians clockwise from straight ahead, is where from).
- **Tones** say what a value means, for the app to colour: `default`, `good`, `warn`, `bad`, `accent`, `dim`.
- **Text is text.** An app shows a game's strings as they are, never as markup.
- A scene built anew in the same runtime posts its first description whatever the last one was.

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
  channels, with the `Engine` interface implemented by the wasm exports and a thin layer of JS ("The engine in a
  browser" below: built, and measured in Chrome on the physics hello world). The layouts considered:

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

## The engine's library in a browser (wasm32)

Layout C's first half: `renderer/` builds as a library for `wasm32-unknown-unknown` (CI builds it). Natively nothing
changed: the same sources, the same shaders, byte-identical renders. What runs it in a page (the exports, the JS
`Engine`, the canvas) is `renderer/web`, the step after this one.

- **The native shell stays native:** the window (`view`, winit), the embedded script runtime (`script`, rquickjs: a
  browser runs the game's TypeScript in its own engine), the denoiser (OIDN), image comparison, the headless runner
  (`live::run`: stdin commands, frame log, snapshots), the offline render (`Gpu::render`) and the snapshot readbacks.
  They are not compiled for wasm32.
- **The GPU comes from futures:** `Gpu::new_async()`; the native `Gpu::new()` blocks on it. A browser resolves them
  from its event loop, and nothing in the library may block there.
- **Time** is `web_time::Instant`: std's natively (a re-export), `performance.now()` in a browser, where std's panics.
- **Files** (`files.rs`): the library's reads go through one module. Natively the file system, as before. In a
  browser the page fetches what the scene names (`World::files`) and hands the bytes over (`files::provide`) before
  the scene is built; the two presets are compiled in. A glTF's external buffers and images are still read from disk
  by the glTF crate: only GLB files (everything in one file) load in a browser.
- **Waiting for the GPU.** wgpu's `poll(wait)` returns at once in a browser, and a buffer is mapped only after the
  page's event loop turned. The 30 places that wait:

  | where | what | in a browser |
  | --- | --- | --- |
  | `LiveCore::render`: at most two frames in flight | waits for the oldest submission | no wait (the page's frame callback paces) |
  | `LiveCore::render`: GPU timestamps (`harvest`) | mapped asynchronously, read a later frame | works as it is |
  | `LiveCore::render`: without timestamp queries, a frame timed by waiting for it | wait | reads ~0: dynamic resolution needs the `timestamp-query` feature there, or a fixed scale |
  | `LiveCore::render`: `THREERT_PROFILE` | wait + map | not reachable (no environment) |
  | `repose`, `repose_groups`, `edit_geometry`: `THREERT_PROFILE_BLAS` | wait | not reachable |
  | `Gpu::upload`: after the uploads, and after the BLAS builds | wait | no wait; hardware RT is not in a browser |
  | `Gpu::render`, `render_translucent` (offline), `read_hdr`, `read_signals`, `read_ev100`, `live::run`'s snapshots | wait + map + read | native only |
- **Threads:** none in a browser. Scene builds (BVH, texture pages, model loading) run on the one thread (rayon falls
  back to it). The dynamic sky's bake thread cannot start, so a sky whose sun moves keeps the sky it was built with.
- Checked: the library's wasm32 build in CI; the physics' recorded hashes in the renderer's wasm32 graph
  (`tools/physics-determinism.ts`). Not yet run in a page: that is `renderer/web`'s to show.

## The engine in a browser (`renderer/web`)

Layout C's second half: the physics hello world runs in Chrome (2026-11-11), the same game file and the same numbers
as natively. Measured on hello world only; larger scenes, and scenes that edit themselves, wait for the two-level
tree (ROADMAP § 6).

- **The module** (`renderer/web`, a `cdylib` for `wasm32-unknown-unknown`, wasm-bindgen): the engine's world, the
  voxels, the physics and the live view. Its exports are of two kinds:
  - the engine API: one export for each function the native runtime installs as `__engine` (35; `script.rs`
    `install_engine`), under the same name. Both call the same code (`renderer/src/api.rs`); a descriptor crosses as
    JSON text, an array as a typed array;
  - the session, what `GameSession` and the native window do around the script: `start` (the GPU from futures, the
    view of the world the setup built, shown on a canvas), `frameBegin` / `frameEnd` around the node host's `frame`
    (the scene's time and sway; then the physics' ticks, the commit, the frame), `click` (the pick and the event's
    payload with the view ray), `commit`, `tick` (a frame's end with no view).
- **The page** (`renderer/web/www`): `engine.ts` makes `__engine` of the exports (typed against `packages/scene`'s
  `Engine`); `main.ts` loads **the game bundle the native runtime loads** (`tools/bundle-game.ts`, unchanged), calls
  the node host as `GameSession` does, and sends the canvas' clicks down the same path. `node tools/build-web.ts
  [game.ts] --serve` builds and serves it (needs the wasm-bindgen CLI of the crate's version).
- **The merged binding layout** (`renderer/src/merged.rs`). WebGPU gives a stage 8 storage buffers by default; the
  kernel binds 19. (The live passes need none of it: each has a layout of its own, of the bindings its entry point
  uses, `renderer/src/live/layout.rs` from live.wgsl as build.rs reads it, and none binds more than eight, a unit
  test's check; a page binds them as natively.) There is one source: the browser's kernel is written from
  `pathtrace.wgsl` as text when a module is compiled, and the native one is untouched.
  - The kernel's seven tables become the arrays of one struct in one buffer; their lengths are in the source, each a
    power of two above a floor, so a kernel is compiled again only when a table doubles. The renderer keeps writing
    its own buffers and a frame copies them into the struct's.
  - The kernel's six group-1 buffers (the radiance cache, what the live passes read of a trace) are ranges of one
    buffer, the arrays of a second struct; what the live passes read of it starts at multiples of 256 bytes, so they
    bind those ranges as buffers of their own.
  - A use of a merged name the generator does not know, a declaration not found exactly once, a name it would add
    that is taken: each stops the renderer, with the line. Locals that shadow a name are followed through their
    scopes. A test reads the result with naga: valid WGSL, 8 storage buffers for the kernel (19 as written), the
    structs' arrays where the Rust side puts them.
  - `THREERT_MERGED=1` runs it natively (software traversal, the device held to 8).
- **The loop guard** (`merged::guard`). The kernel's traversals follow indices they read from buffers and end only on
  sound data. On 2026-11-11 two dispatches that did not return took this machine's display down (WindowServer's
  watchdog; software traversal under the atomic tests, at two scenes without a triangle; the cause below is likely,
  not confirmed). A page's shader runs on the
  player's GPU beside everything else, so **a browser's kernel always has the guard**: every loop body counts the
  invocation's iterations and leaves past a limit (65,536; a page can set it). No formula changes, but the image is
  not bit-identical to the kernel's without the guard: the compiler arranges the arithmetic of the functions it is in
  differently (hello world, one sample: 354 of 43,200 numbers differ, by at most 2 × 10⁻⁷ of their value). What it
  stops it counts: the count is read back without waiting (`LiveCore::guard_trips`, the page's `guardTrips()`, the frame log), an offline
  render in which it moved is an error, and anything but 0 means wrong pixels. Natively `THREERT_LOOP_GUARD=<limit>`
  sets it; with nothing set a kernel with units has it (65,536; `THREERT_LOOP_GUARD=0` takes it out) and every other
  kernel has none, as before ("Software traversal with units"). The live passes need none: their loops are counted
  loops over constants and clamped parameters.
- **A tree over nothing** (`bvh.rs`). A node with `count: 0` is an interior node to the kernel, and the one-node tree
  of a scene without triangles had itself and the bytes after its buffer for children. It is now a leaf of triangle
  0, which an empty vertex buffer's zeros make a triangle without area; a procedural object without boxes is left out
  under software traversal. A test walks the kernel's loop on the CPU with every read checked. That read is certain;
  **that it is what kept the two dispatches from returning is not confirmed**: the sequence that hung, run again on
  a build without the fix and with the guard, passed (the bytes after the buffer ended the walk that time; a hang
  needs them to be a box the ray meets and a count or child indices that do not end).
- **What a browser does not have.** Hardware ray queries, so software traversal: no particles of either kind and no
  translucency pass. Edits in place it has had since the step after this one, for model instances ("Software
  traversal with units" below; before it every added object and every frame a body moved rebuilt the scene). The
  denoiser (OIDN, offline only), the offline render, snapshots and the profile's readbacks. The dynamic sky's bake
  thread (a sky whose sun moves keeps the sky it was built with).

Checked without a GPU:
- `tools/web-engine.ts` (CI): the module in Node (V8), `__engine` made by the page's code, the game bundle, the
  physics hello world's five clicks (as view rays) and frames, and the native test's checks; the engine API one for
  one with `install_engine` (35 functions natively, in the page's `__engine`, in the module's exports).
- The generator's and the guard's tests (naga), the tree walk, the web crate's wasm32 build (CI).

### Measured in a browser (2026-11-11)

This section is the build before units (every instance flattened). With units a frame in which a body moves costs
the page 0.84 ms and none rebuilds the scene: "Software traversal with units", "Measured", below.

Three things first:
- **A frame in which a rigid body moves cost the page 16.7 ms, 16.4–16.6 of it rebuilding the scene** (p50; natively
  under software traversal 9.0 ms, under hardware traversal nothing). The CPU side alone filled a 60 fps frame. It
  is what the next step, in-place edits under software traversal, was for.
- **The browser's image is not bit-identical to the native one.** A browser's kernel has the loop guard, and the
  kernel with the guard differs from the kernel without it in the last bits (rounding: 354 of 43,200 numbers of a
  one-sample hello world, by at most 2 × 10⁻⁷ of their value). Natively the guard is off by default, so native
  renders are what they were (the catalog is byte-identical to main). The game's numbers (the physics, the clicks)
  are the native run's exactly.
- **The first frame waits about 1.4 s for a kernel the browser has not compiled, and opening the view takes 1.1 s, of
  which the sky's bake on the page's one thread is 1.09 s.**

Apple M5 Pro, Chrome 157.0.8103.40 driven by Playwright (`node tools/web-hello.ts`; a Chrome of its own profile),
its WebGPU adapter `apple` / `metal-3`, not a fallback adapter (the machine's GPU). The adapter allows 10 storage
buffers a stage; the device was asked for 8 and got 8. Loop guard on (65,536; its count 0 in every run below).
1280 × 720 traced at half (the dynamic resolution's floor), other applications holding 15–40 % of the GPU. The runs
and their order are in `out/web-probe/gpu-runs.md` (kept beside the checkout).

**The scripted test** (`tools/physics-hello.ts`'s: 822 frames in lockstep, five clicks at the same view
coordinates through the page's click path, the same checks): passes at 640 × 360 and at 1280 × 720, after a first
step of 14 frames at 160 × 90. **The report and the clicks' log are the native run's, number for number**: boxes at rest at (−1.0002, 0.2498, −0.3052),
(−1.0109, 0.7481, −0.2996), (−1.0023, 1.2475, −0.3001), the kicked one at (1.7011, 0.2499, 1.6092), 0.913 m along the
kick (0.917 predicted), all asleep. The same ticks give the same bits: nothing to explain away. A real mouse click on
the canvas adds its box 1.1–1.3 cm from where it was aimed (a mouse lands on whole pixels). The canvas against a
native live snapshot of the same frame count ± 1: 1.2 of 255 a channel, mean.

**Frame times**, ms p50 / p95, the same five clicks in real time, content at 60 fps (two runs; the second's figures):

| | still | a body moves |
| --- | --- | --- |
| frame interval | 16.67 / 17.29 | 16.67 / 25.0 |
| the page's own time in a frame | 0.29 / 0.81 | 16.7 / 18.0 |
| of it: the node host's `frame` (script) | 0.08 / 0.20 | 0.05 / 0.08 |
| engine calls made by the script | 1 a frame (not timed apart: the table below) | the same |
| physics: step and scene sync, in wasm | 0.02 / 0.06 | 0.06 / 0.12 |
| commit | 0.005 / 0.01 | 16.5 / 17.7 |
| of it: **rebuilding the scene** (software traversal, every frame a body moves) | 0 | **16.5 / 17.7** |
| recording the frame (the passes, the canvas) | 0.16 / 0.46 | 0.09 / 0.12 |
| GPU | 6.6 / 8.2 | 6.1 / 8.1 |
| of it: the trace | 2.1 / 2.8 | 1.7 / 2.7 |

- Still, a frame is the GPU's 6.6 ms and 0.3 ms of the page. Natively, the same view under software traversal:
  6.9–8.0 ms of GPU (trace 1.5–1.7), so the browser's frame is the native one within the noise of this machine;
  hardware traversal 4.7 (1.0).
- **Every frame in which a body moves spends the whole 16.7 ms rebuilding the scene** in the page (natively under
  software traversal: 9.0 ms, on every core; hardware traversal: nothing, the body's instance moves in place). The
  interval holds 60 fps at p50 and slips at p95. This is the cost hello world ships with until software traversal
  edits in place.
- A click that adds a box: pick 0.03–0.3 ms, the handler 0.4–1.6 ms, the commit 16.6–24.1 ms (a rebuild; natively
  under software traversal 8.8–10.7 ms, under hardware traversal 0.2–0.6 ms in place after the first box).

**Opening the view:** the module fetched and instantiated 21–65 ms; `setup` 7.4–8.1 ms (28 engine calls); `start`
1.13 s, of which **1.08 s is the sky's bake on the page's one thread** (natively 37–40 ms on every core) and 3–4 ms
the adapter and the device. **The first frame:** its work done on the GPU 26 ms after it was recorded when the
browser has compiled the kernel before; **1.43–1.44 s** with a kernel it has not (measured by giving the page a guard
limit it had not had, a new constant in the kernel's source; a fresh browser profile is not enough, the system keeps
compiled shaders across profiles). The live passes' own first compile was not separated.

**The boundary in Chrome** (`?bench=1`; a world of 10,000 objects with a body each, ticked once), ns a call, the best
of five runs; beside it the same module in Node 24 (`tools/web-engine.ts --bench`):

| call | Chrome 157 | Node 24 |
| --- | ---: | ---: |
| `visible(id, bool)` | 38 | 39 |
| `power(id, number)` | 39 | 40 |
| `transform(id, Float32Array(10))` | 54 | 55 |
| `customData(id, [4])` | 95 | 98 |
| `material(id, desc)` (JSON) | 1,190–1,200 | 1,410 |
| `object(id, desc)` (JSON) | 790–800 | 910 |
| `physics("state", { body })` (JSON both ways) | 1,490–1,520 | 1,590 |
| `physics("impulse", { body, impulse })` | 590 | 610 |
| `physics("ray", …)` in that world | 1,700–1,740 | 1,860 |
| `physicsPoses()` for 10,000 bodies, not read (70,000 floats copied out) | 20,000–21,600 | 25,100 |
| 1,000 bodies' poses fetched and read | 0.05–0.07 ms | 0.05 ms |
| 10,000 bodies' poses fetched and read | 0.30–0.32 ms | 0.30 ms |

The runtime probe's "a call costs about 1 ns" was an empty export; these are the engine's own calls, with the work
they do (a lookup, the change sets, for a descriptor its JSON). A scalar call is 40 ns, a descriptor about a
microsecond, 10,000 poses 0.3 ms (QuickJS natively: 0.94 ms). One finding beside the boundary: a physics query
tests every collider added since the last tick one by one, so in a world that has never ticked a ray over 1,000
bodies costs 42 µs (after a tick: 1.7 µs over 10,000).

**Natively, the merged layout and the guard** (hello world, software traversal): offline, the merged layout's bytes
are the layout's as written (with the guard in both; and without it in both); live, the scripted test's report in
the merged layout with the guard is the hardware run's number for number. Their cost on this scene is inside the
run-to-run spread (GPU ms a frame p50, each twice: as written 8.0, 6.9; merged 6.3, 7.0; as written + guard 7.1,
6.1; merged + guard 10.2, 6.7): a 106-triangle frame is not where a trace's cost shows.

**Size:** the module 7.34 MiB (gzip 2.28 MiB, brotli 1.51 MiB; no `wasm-opt`), wasm-bindgen's JavaScript and the
page's 105 KiB (brotli 17 KiB), hello world's bundle 230 KiB (brotli 53 KiB).

**Not measured:** any scene but hello world in a browser; Safari and Firefox (nothing Chrome-only was written:
WebGPU through wgpu, default limits except buffer sizes, timestamp queries only if the adapter has them). Measured
since, natively (below, "Software traversal with units"): the merged layout's and the guard's cost on a scene whose
trace is the frame (Kabukichō), and the atomic tests under software traversal as a suite, in both layouts.

## Software traversal with units (in-place edits without ray queries)

ROADMAP § 6 steps 7b and 7c. **Status: runs natively and in Chrome on this machine's GPU** (2026-11-11; every run
with the loop guard, which stopped nothing). Step 7b brought the edits and the memory; a still frame then paid for
them (the trace 37 % / 71 % / 16 % longer on the three scenes below), and step 7c took that cost out.

**Where units are used.** In a live session and in a browser (the wasm build), by default. **An offline render
(`threert-render render`) under software traversal flattens every instance, as before step 7b: its kernel and its
bytes are main's with nothing set.** `THREERT_UNITS=1` gives an offline render units; `THREERT_UNITS=0` flattens a
live session too (the A/B). Why the line is there (decided 2026-11-11):
- an offline render under software traversal is the reference render, and what the catalog's byte gate compares; the
  gate runs with no variable set, and stays a comparison with main's bytes;
- what units give (the triangle data of each mesh held once, a faster load, edits in place) is what a session that
  edits and a browser need; an offline render builds its scene once and edits nothing.

**What a still frame costs** (software traversal, live, 1280 × 720 traced at half, this machine, guard on in both
columns; "Measured: the still frame" at the end of this section has every step between the two):

| scene | the trace: units as in step 7b → **now** / flat | frame GPU: 7b → **now** / flat | 60 Hz (16.7 ms) |
| --- | --- | --- | --- |
| hello world (natively) | 1.42 → **0.73** / 0.99 ms | 4.60 → **3.92** / 4.33 ms | held |
| the voxel level (10,459 units) | 7.83 → **4.17** / 4.54 ms | 11.16 → **7.52** / 7.87 ms | held |
| Kabukichō (352 units) | 13.98 → **11.8** / 12.2 ms | 17.4 → **15.2** / 15.6–15.9 ms | **held again** (a frame every 16.64 ms; step 7b: 17.3) |

With units the trace is now a little below the flattened scene's on these three, and on the catalog's small models;
on the two largest single models it is 4 % (`bistro`) and 13 % (`zero-day`) above. Most of what was taken out was
not traversal: it was arrays set to zero for every ray ("The stack", below).

**Not run**: a kernel with units without the guard; a pile of bodies at rest on one another on the GPU (no game
makes one yet; counted on the CPU, below); anything but hello world in a browser.

**What there was.** Software traversal (a browser's only one) walked one flat tree over every triangle in world
space. Instances were written out into it (`flatten_instances`), so adding or moving anything rebuilt the scene, and a
level of placed meshes held every placement's triangles. The kernel already shades a hit on an instance (`Hit.inst`
names a record with the instance's rows; hardware traversal fills it), and already walks a second level for
procedural objects (`trace_procs`: the ray into the object's space, a tree of its own).

**What there is now.** Under software traversal an instance stays an instance: a **unit**.
- **The still scene is the flat tree it was**, walked first, exactly as before. (A scene whose triangles are all
  units' has for it a leaf of one triangle without area, put after the meshes' triangles: the tree over nothing's
  leaf names triangle 0, which with units is a mesh's first, in model space.)
- **A mesh's tree** is built once over its triangles in model space, where they already are (the pool hardware
  traversal builds its BLASes from; under software traversal the mesh's triangles are put in its tree's order, a
  cutout triangle carrying its flag). Its nodes sit in the node buffer after the still tree's; a tree is relocatable
  (child indices count from its root, leaf triangles from its mesh's first).
- **A unit** is the instance's kernel record (160 B), as under hardware traversal, with three of its unused words
  filled: its mesh tree's root; whether it casts shadows; whether it is placed by a translation alone (the ray then
  keeps its direction and only its origin moves: 8,308 of the voxel level's 10,459 units).
- **The top tree** is over the units' world boxes, one unit a leaf. A unit's box is its mesh's tree's bounds under the
  unit's transform, read when the top tree is made (so a mesh written again where it was moves its units' boxes with
  it). It is written whole into its own range of the node buffer whenever a unit is added, moved or removed (as the
  TLAS is built under hardware traversal). Its root and the number of units reach the kernel in two words of the
  frame's parameters (`Params.motion.z`, `.w`, as bits; no binding, no buffer more).
- **How the top tree is made** (`units::Kept`). By the surface-area heuristic (binned, one unit a leaf, its depth held
  to 18 whatever the boxes are): median splits by centres, what step 7b had, put a wall and the lamp beside it in
  halves that both span the wall, and a ray read 82 top nodes to enter 2 units on the voxel level (62 now). Such a
  build is 3.8 ms for 10,000 units, so it is seldom made over all of them. The units are two sets: the **settled**,
  which are where a build over all last had them (or less than their own size from there), whose tree is that
  build's, kept, its boxes brought up to date (0.45 ms for 10,000) and leaves taken out as units go; and the
  **loose**, which have come since or travelled (pieces thrown, bodies in flight), whose own small tree is built
  whenever one of them comes, goes or travels on. The top tree is a root over the two (one level more: 19). One
  tree is built over all when the settled tree's nodes have grown past 1.3 times their area at the build, when the
  loose at rest are an eighth of the units, or when the scene is small (64 units: a build is microseconds). Tried
  and left out: putting a new unit into the one tree leaf by leaf; after a few thousand pieces had flown, rays
  through the piles read several times what a built tree costs them.
- **The kernel** walks the top tree after the still one, with the ray already cut to the still scene's hit; at a
  unit it takes the ray into the unit's space (as `trace_procs` does) and walks the mesh's tree. **The top tree and
  the units' trees are walked by one loop**, each turn a node of whichever tree the ray is in: a GPU runs a group of
  rays through a loop together, and with a unit's walk as a loop inside the top walk's, a ray that had left a unit
  waited at every unit for the slowest of its group (1.5 ms of the voxel level's 6.8). The top tree's root's box is
  tested first: a ray that meets no unit reads one node, and a scene of one unit has no tree to walk. The hit it
  returns is the hit hardware traversal returns (the triangle, the record), so nothing after the traversal differs.
- **The unit walk is `shaders/trace_units.wgsl`**, in the kernel under `@if(units)` (`F_UNITS`, a source feature,
  as vertex colours are; gpu.rs `kernel_flags`): the file's walk, a call at the end of the still walk, and the still
  walk's stack declaration taken out ("The stack"). A scene without units has none of it; hardware traversal's
  kernel never does.
- Still flattened, as before: meshes that deform (a pose shared by a crowd), every instance of a scene with more
  of them than a top tree holds (2^18), and every instance of an offline render (`prepare`; a live session's scene
  goes through `prepare_live`), unless `THREERT_UNITS=1`. `THREERT_UNITS=0` flattens everything, as before this
  step (the A/B).

**Edits in place.** The edits that hardware traversal makes in place go through one type (`gpu::Top`: the TLAS and
its BLASes, or the units), so the engine's path is the same under both traversals (`live/edits.rs` `move_instances`,
`update_instanced`, `edit_geometry`):

| edit | under software traversal with units |
| --- | --- |
| a unit moves (a rigid body's box, a piece of debris) | its record and the top tree |
| an instance of a mesh the scene has is added or removed | a record in the edit room, the top tree |
| a mesh of its own is added (a new chunk, a new box) | its triangles and attributes into the edit room in its tree's order, its tree into the mesh range, a record, the top tree |
| a mesh of its own changes (a voxel edit: the chunks it touched) | those meshes' trees made again (the old tree's nodes given back and filled), their records; no other unit is read |
| a mesh is removed | its record cleared, its tree given back and filled, the top tree |

The scene is rebuilt, as before this step, by: the first instanced mesh of a scene that has none (its kernel has no
unit walk: one rebuild, as hardware traversal's first instance is); an edit of a procedural object; a posed model or
a rigid group's pose (their triangles are in the still tree); an instance of a mesh that deforms; **room that is
full** (the edit room and the tree ranges are sized at the build, a quarter of the scene and at least 65,536
triangles and 1,024 records, and do not grow under software traversal: under hardware traversal they do; what an
edit replaces stays behind until the next rebuild, under both; a scene with units and no geometry of its own to
edit gets the records and object entries alone, so an instance of a mesh it has is added in place and a mesh it
has not rebuilds it); a top tree that is refused (below). The edit log names the reason (`rebuild (edit room:
records (software traversal's does not grow))`).

**Why the loops end.** A walk follows indices it reads, so the trees are checked on the CPU where they are written
(`units.rs`): every child index is greater than its parent's and inside its tree's range, a leaf's triangles inside
its mesh's and at most 16, a top leaf's record a unit's. A tree that fails is not uploaded: the scene is rebuilt.
A unit's record names its mesh's tree by its root, so a top tree is refused while a shown unit's record names a root
that is not its mesh's tree now (freed, or placed again elsewhere), or a unit sits past the kernel's records. Room
not yet used, and a tree given back, hold leaves that end (`units::FILL`), not zeros (zeros are an interior node
whose children are nodes 0 and 1). With no unit shown the count is 0 and the kernel does not enter the top range.

**The stack.** One stack of 64 entries for an invocation of a kernel with units: a private variable of the module,
which the still walk uses too (its own declaration is under `@if(!units)`). Why not a walk's own: an array a function declares is set to zero each time the function is
entered, whatever the ray then meets, and a ray's walk is entered many times a pixel. The unit walk's own 64 words
cost hello world 0.3 ms a frame, Kabukichō 0.75 and the voxel level 0.55; the still walk's own 48 cost them 0.36,
1.1 and 1.5 more, **and cost every flattened scene the same** (a kernel without units is the file as it is, so it
still pays: "Open" in ROADMAP § 6 step 7c). A walk reads only what it pushed, so what an earlier ray left is never
read. The still walk uses 48 entries, as before, and leaves the stack empty; a unit's walk pushes above what the
top walk has pending. The top tree is at most 19 deep (a built tree 18, which holds 2^18 units, the most a scene is
given units for; a root over two of them). That leaves 45 for a mesh tree. Binned SAH is not balanced: measured on
the CPU, its trees over the catalog's models are 17 deep for 37 k triangles, 20 for 502 k, 22 for a 31 k-triangle
building of Kabukichō, and 42 for the 5.86 M triangles of `zero-day` (a balanced tree there is 21), which as part of
a flattened scene fitted the still walk's 48. A mesh tree deeper than 45 is built again by median splits, whose
depth is log2(triangles / 4): a worse tree to walk, never a wrong one. A push past the stack is dropped (geometry
missed, never a hang), and the builders' bounds mean it is not reached; `the_builders_keep_their_depths` holds them
to it.

**Checked on the CPU** (`units.rs`, `lib.rs` `units_tests`; no GPU):
- the kernel's walks transcribed line for line (a test compares the transcription's source lines with the
  shaders'), with every buffer read through an accessor that refuses an index outside its buffer and every loop
  iteration counted;
- random scenes and random sequences of adding a mesh, adding, moving and removing units, removing a mesh: after each
  step, for closest, shadow and any-hit rays, no read outside a buffer, an end within the nodes there are, and the
  closest hit brute force over every triangle finds;
- a scene prepared with units (`prepare_live`, what a session uploads) against the same scene flattened, ray for ray:
  a hit where there was one, at the same distance to 2 × 10⁻⁴ (a unit's triangles are met in its own space), and
  the same shadow rays blocked;
- broken trees (a child at or before its parent, a cycle, a range outside, a leaf over 16, a record that is no unit)
  refused; a record naming a tree that is gone refused;
- the top tree's rule: kept while units move a little, the loose units' tree built when units come, go or travel,
  one tree over all when the settled one costs too much or the loose have come to rest; whichever it is, a tree
  `check_top` accepts, within a quarter of a built tree's cost through 200 frames of thrown pieces
  (`a_top_tree_is_kept_while_units_only_move`); the random edits above run through the same rule;
- what a ray reads, counted (`units_cost.rs`, measurements, not tests: `units_cost` over a bundle's scene with units
  and flattened; `units_pile`, pieces of debris thrown onto a scene);
- the sources handed to wgpu for every scene without units: the hashes main has
  (`the_default_sources_are_untouched`); the guarded, merged kernel with units is valid WGSL in 8 storage buffers;
- an offline render of a scene with instances: no unit, no `F_UNITS`, and the triangle and node bytes the flattened
  scene has, unless `THREERT_UNITS=1` (`an_offline_render_has_no_units_unless_told`);
- the guard's default: a kernel with units has it at 65,536 with nothing set and none at `THREERT_LOOP_GUARD=0`, a
  kernel without units is the source it was (`a_kernel_with_units_has_the_loop_guard`; naga validates the guarded
  kernel with units as written).

**The loop guard.** A browser's kernel always has it (`merged::guard`). **Natively a kernel with units has it with
nothing set** (65,536 iterations an invocation; `THREERT_LOOP_GUARD=0` takes it out, `=<limit>` sets another): what
hung on 2026-11-11 is not confirmed, the unit walk is new, and no kernel with units has run without it. A native
kernel without units has none unless `THREERT_LOOP_GUARD` says so, as before (its source and every existing byte
unchanged); whether all of native software traversal gets it is not decided here. Every GPU run of a kernel with
units so far had it (8,192 or 65,536), and it stopped nothing in any of them. On Kabukichō it costs the flattened
kernel 1.2 ms of trace a frame (11 %; below). What it does when it stops something is unchanged: an offline render
is an error, a live session counts it in the frame log (`guard`; `null` while the scene's kernel has none, as
before a scene's first unit), and `tools/test-render.ts` fails a run in which it stopped a live test's frames.
Under software traversal as written the guard's count has its binding and buffer whether or not the scene's kernel
names it (not at `THREERT_LOOP_GUARD=0`), so a scene's first unit brings the guard without another layout.

### Measured (step 7b, 2026-11-11)

(As step 7b left it: the top tree by median splits, a stack of the unit walk's own, two nested loops. What a still
frame costs now is in "Measured: the still frame" after this.)

Apple M5 Pro; natively `THREERT_HWRT=0` with `THREERT_LOOP_GUARD` 65,536 unless said; "flat" is the same build with
`THREERT_UNITS=0` (every instance flattened: main's kernel, and main's bytes on the catalog); live views 1280 × 720
traced at half. These runs were made while an offline render had units by default too; the offline figures with units
below are what `THREERT_UNITS=1` gives now. Run again with the defaults as they are (nothing set but
`THREERT_HWRT=0`): the last paragraph of this section. The runs, their order and their limits are in
`out/web-probe/gpu-runs-b.md` (kept beside the checkout). Other applications held part of the GPU throughout; figures
are p50, two launches where a spread is given.

**Hello world in Chrome** (`tools/web-hello.ts --frames`; Chrome 157, the page's guard), ms p50 / p95:

| | units | flat (the section above) |
| --- | --- | --- |
| a frame in which a body moves: the page's time | **0.84 / 1.07** | 16.73 / 17.99 |
| of it the commit | 0.08 / 0.10 | 16.49 / 17.68 (the rebuild) |
| frames that rebuilt the scene | 0 of 1,173 | 295 of about 1,180 |
| a click that adds a box: commit | the first 31.6 (one rebuild, into the kernel with units), then **0.79, 0.73, 0.64** | 18.5–24.1 |
| the trace, still / a body moves | 2.55 / 2.66 | 2.08 / 1.72 |
| frame GPU, still / a body moves | 6.59 / 6.73 | 6.60 / 6.15 |

The scripted test passes (640 × 360 and 1280 × 720), its report and clicks the native run's number for number; it
now checks that no frame rebuilds the scene and that only the first box's click does.

**Hello world natively** (`tools/physics-hello.ts`, merged layout): every check passes, the two that software
traversal failed among them (the commits `rebuild, in place × 4`; one rebuild). A frame in which a body moves:
commit 0.02 ms (flat 8.65), frame CPU 4.6 ms (8.9); a click's commit 0.25–0.35 ms in place (9.1–9.6). The trace is
1.40–1.42 ms where flat's is 1.01–1.04 (**+0.38 ms, 37 %**, the same with one box as with four: every ray reads the
top tree where the still scene is a ground and a wall).

**The voxel level** (10,459 units of 6,410 meshes; flattened, 13.2 M triangles):

| | units | flat |
| --- | --- | --- |
| triangle data (vertices, attributes, nodes) | **310.9 MiB** | 2,098.2 MiB |
| trees built at load (`bvh_ms`) | 333 ms | 5,378 ms |
| first frame after | 5.2 s | 11.6 s |
| an explosion (0.5–4 m; 4–20 chunk meshes placed) | **2.5–4.5 ms, in place** (1.8–2.0 of it the trees) | 6.9–7.9 s (the scene rebuilt) |
| still frame: the trace | **7.85 ms** | 4.58 ms |
| still frame: GPU | 11.2 ms | 7.9 ms |

The still frame is where units cost: **+71 % trace**. The level is 10,459 placed chunks side by side and stacked,
whose boxes touch and overlap, under a top tree built by median splits; flattened they are one SAH tree. Not done:
a better top tree (SAH), a unit whose transform is the identity walked without transforming the ray. Not measured:
a pile of debris (boxes that overlap deeply), explosions until the room is full.

**Kabukichō** (352 units of 19 building meshes):

| | units | flat |
| --- | --- | --- |
| offline, 320 × 180, 64 spp: the trace (units: `THREERT_UNITS=1`) | 394.8 ms | 336.2 ms (units +17 %) |
| trees built (`bvh_ms`) | 89 ms | 1,635 ms |
| triangle data | 55.0 MiB | 634.1 MiB |
| live, guard on: frame GPU / the trace | 17.18 / 13.88 ms | 15.35 / 12.01 ms (units **+1.87 ms, 16 %**) |
| the same in the merged layout | 17.44 / 14.10 ms | 15.66 / 12.28 ms |
| first frame, second launch | 1.55 s | 3.3 s |

With units the view misses 60 Hz on this machine (frame interval 17.2–17.5 ms; flat 16.65). The images are not the
same bytes as flat's (a unit's ray is intersected in model space, and the emitter table follows the triangles'
order): at 64 spp the means are 0.69371 and 0.69395, 20-pixel blocks 0.5 % apart (p50), unbiased; hardware
traversal's image of the same render is as far from both.

**The guard and the merged layout on a scene whose trace is the frame** (Kabukichō flat, main's kernel, live; frame
GPU / the trace, ms): as written 14.11 / 10.78; merged 14.29 / 10.91; as written with the guard 15.35 / 12.01;
merged with the guard 15.66 / 12.28. **The guard: +1.23 ms of trace (11 %). The merged layout: +0.13 ms, with the
guard +0.27.** (On hello world both were inside the spread.) A kernel with units was not run without the guard.

**The atomic tests under software traversal** (170, a family a command, guard on; the first whole run of the suite
under software traversal on this machine; made while offline tests had units too): 147 pass and 23 fail, as written
and in the merged layout alike, **and the same 23 fail on main's build, with the same values**:
- particles, which software traversal does not draw (docs/MATERIAL_FUNCTIONS.md: they need ray queries):
  `particles/count-zero`, `emissive-spark`, `emissive-spark-nominal`, `emissive-unsampled`, `follow-emitter`,
  `hidden-in-place-live`, `instance-data`, `lit-normal`, `lit-orientation`, `motion-repose`, `node-custom`,
  `quad-placement`, `shown-in-place-live` (13; `particles/burst-not-yet` expects nothing drawn and passes);
  `gpu-particles/beside-particles`, `plane`, `voxels`, `voxels-live`, `voxels-side` (5); `translucent/depth`,
  `translucent/order` (translucent particles);
- `volume/lighting`, `volume/lighting-offline` (6 for 1): their subject is one translucent particle in front of a
  6-nit wall, so these two are the same missing particles: the region reads the wall;
- `surface/windows` (0.3934 for 0.4234): not explained. Its scene is one plane with the library's facade windows
  and no instance, so its kernel on this branch is main's source byte for byte.
The last three are in ROADMAP "Known issues"; they are the other line's to fix. The suite found one fault of units:
`voxel/cut-falls-live` drew a falling part cut off at the box it had before it fell (a unit kept the bounds its
mesh had when added; now the box is the tree's, above). Before that the first GPU run found another: a scene of
units alone drew a mesh's first triangle at the origin (the still leaf, above). Both have CPU tests now.

**How each kind of edit went** (the atomic edit tests one by one, the edit log): instances added, moved and removed,
a model's second copy, a new mesh, vertex colours, emitters added and removed: in place. A rebuild, with its reason
in the log: 1,100 specks added to a scene with room for 1,024 records; a procedural object edited or moved.

**With the defaults as they are now** (units live and in a browser, flat offline, the guard with units; run after
the rule above was built in):
- Hello world natively with nothing set but `THREERT_HWRT=0` (`tools/physics-hello.ts`): every check; the commits
  `rebuild, in place × 4`; 812 of 823 frames' kernel has the guard (the eleven before the first box have a kernel
  without units, and the log's `guard` is `null`), and it stopped nothing.
- The catalog offline under software traversal, `THREERT_UNITS` not set: **23 of 23 bundles byte-identical to main**
  (HDR and PNG, 320 × 180, 4 spp). Both builds ran with `THREERT_LOOP_GUARD=65536`, as every GPU run of this step;
  that the kernel without the guard is main's too is the source hashes' test.
- `cargo test --release` whole (78 pass), the atomic tests under hardware traversal (170 pass), and the scripted
  test in Chrome (every check, the guard 0 in 826 frames, no frame rebuilt, the report the native run's): as before.
- Not run again: the perf smoke and the catalog under hardware traversal (18 of 23 identical; `bistro`,
  `bistro-slice`, `bistro-district`, `zero-day`, `zero-day-animated` differ from main in 1,884 / 133 / 80 / 9,507 /
  8,850 of 57,600 pixels, and main from main in 1,782 / 1,095 / 125 / 7,988 / 10,426: ROADMAP "Known issues",
  hardware traversal's ties). Hardware traversal's sources, layouts and buffers are not touched by the rule.

### Measured: the still frame (step 7c, 2026-11-11)

The same machine, settings and tools as above (software traversal, live, 1280 × 720 traced at half, guard 65,536;
p50; Kabukichō two launches of `tools/bench-layout.ts`). Each row is a build with one more change; "flat" is the
last build with `THREERT_UNITS=0`. The runs and their order: `out/web-probe/gpu-runs-c.md` (kept beside the
checkout).

| the trace, ms | hello world | the voxel level | Kabukichō |
| --- | --- | --- | --- |
| flat | 0.99 | 4.54 | 12.2 |
| step 7b | 1.42 | 7.83 | 13.98, 13.93 |
| the top tree by surface area | 1.43 | 6.90 | 14.25, 14.21 |
| + every top node's own box tested when the walk comes to it (taken out again) | 1.42 | 7.43 | 14.34, 14.24 |
| + a unit placed by a translation alone keeps the ray's direction | 1.41 | 7.39 | 13.94, 14.01 |
| + the unit walk uses the still walk's stack: one array zeroed a ray, not two | 1.13 | 6.84 | 13.24, 13.17 |
| + one loop for the top tree and the units' trees | 1.09 | 5.30 | 13.31, 13.02 |
| + the stack is the module's: none zeroed a ray | **0.73** | **4.17** | **11.83, 11.75** |

Tried and left out: the still tree in the same loop as the other two (the voxel level 5.47 for 5.30, Kabukichō 14.7
for 13.2); two other forms of the loop guard's count (at most 3 % of hello's trace, nothing on Kabukichō: the
guard's 1.2 ms there is not its form). In Chrome (`tools/web-hello.ts --frames`) hello's trace is 1.81 ms still and
1.75 with a body moving (step 7b: 2.55 and 2.66).

**The catalog's scenes with models, live with units** (the last frames of 120 in lockstep compared by
`threert-render compare`):

| scene (units) | the trace, units / flat | the picture against flat's |
| --- | --- | --- |
| `khronos-clearcoat` (1) | 1.84 / 2.11 ms | ΔE 0.01, SSIM 1.0000 |
| `khronos-metal-rough` (1) | 2.27 / 2.68 | ΔE 0.02, SSIM 1.0000 |
| `khronos-transmission` (1) | 5.28 / 5.37 | ΔE 0.03, SSIM 0.9999 |
| `instancing` (1,000, all moving) | 3.39 / 7.89; flat rebuilds the scene every frame (276 ms of CPU a frame; units 6.5) | offline at one instant, 64 spp: ΔE 0.37, SSIM 0.995 |
| `bistro` (1 unit of 2.83 M triangles) | 23.1 / 22.2 | ΔE 0.69, SSIM 0.990 |
| `zero-day` (1 unit of 5.87 M) | 14.5 / 12.8 | ΔE 1.20, SSIM 0.963: the scene's own spread, see below |
| `zero-day-animated` (3,991) | every frame rebuilds the scene (2.6 s), with units as flat: a rigid group's pose, as before step 7b | – |

`zero-day`'s picture: two runs of the flat kernel are ΔE 0.11 apart, so units' 1.20 is not noise of one kernel.
Offline at 1,024 spp the three traversals are as far from one another: units against flat ΔE 0.97, hardware
traversal against flat 0.87, hardware against units 0.91 (mean luminance within 0.3 %): speckle on the scene's
fine and coincident geometry (ROADMAP "Known issues"), nothing missing or misplaced.

**The voxel level under edits.** An explosion's commit (0.5–4 m, in place): 2.6–3.9 ms, of it the trees 0.7–1.4
(step 7b: 2.5–4.5 and 1.8–2.0). An explosion a frame for ten seconds (433 edits) does not fill the default room
(triangles 2.23 M of 2.36 M, records 10,516 of 13,073). With a fifth of the room it fills four times: each time
the scene is rebuilt in **0.48–0.64 s** (flattened, every explosion is 7 s), the room is laid out again around the
scene as it then is, and the edits after it are in place again; the last frame is the never-full run's, identical,
and against hardware traversal's of the same run ΔE 0.07, SSIM 0.9997.

**Debris as units.** Twenty explosions in a second throw 8,192 voxel cubes, each an instance for 4 s (the `debris`
game): with all of them in flight the trace is 4.19–4.30 ms against 4.11–4.13 without (**+2–4 %**); the CPU's
instance updates are 2.2–2.4 ms a frame, about 1.6 of it the top tree (the loose units' tree built each frame; one
build over all three times in 277 frames). Pieces at rest on one another cost a ray more than pieces in flight;
counted on the CPU (`units_pile`: lumps of 36 triangles in eight piles on the level): 1,000 pieces +15 % of the bytes
a pixel reads, 4,000 +43 %, 16,000 +76 %, a ray past a pile entering 2.2 / 2.8 / 4.0 units; the top tree in a frame
of flight 0.9 / 1.3 / 2.5 ms. **So step 8 has room for a few thousand debris bodies**, and what bounds it first is
the CPU's update of the instances, not the trace.

**The merge gate**: the catalog offline under software traversal with nothing set, 24 of 24 bundles byte-identical to
main; `cargo test --release` whole (79 pass); the atomic tests under hardware traversal (170 pass) and the 29 with
units under software traversal; hello world natively and in Chrome (every check, the guard 0, no frame rebuilt).

## The kernel's source

The path-tracing kernel is one WESL module: `pathtrace.wgsl`, then `trace_units.wgsl`, `trace_hw.wgsl` and
`translucent.wgsl` (gpu.rs `KERNEL_FILES`), every device's and scene's variant in it under WESL's conditional
translation (`@if(flag)` on a declaration or a statement). wesl-rs (WESL's compiler in Rust, natively and in a
browser alike) makes a kernel's WGSL from the flags (gpu.rs `kernel_flags`, `kernel_wgsl`): the traversal
(`hwrt`), a device's switches (`strict`, `cutout_lite`, `cutout_lite2`, `last_direct`, `light_pick`) and a scene's
features that change declarations (`vcolor`, `grid`, `tint`, `unlit`, `units`, `background`; the other features switch
code under the pipeline-overridable constant `FEAT`). pathtrace.wgsl's head names them; each is said where it changes
something, and a flag the source names that a kernel is not given is an error.
- Before that, still as text: a scene's material, volume and geometry functions between their markers in
  pathtrace.wgsl, the merged layout (merged.rs `kernel`, the browser), the uniform tables (merged.rs `uniform_tables`,
  Android) and the loop guard (merged.rs `guard`).
- Until 2026-11-14 the variants were patches of the source at anchors, whole lines of code found and replaced (gpu.rs
  `kernel_base`, `kernel_switches`, `kernel_source`). For every combination of the flags, 2,304, the Metal source naga
  wrote from the patched kernel and from the one with `@if` was the same, two rewrites allowed for: the light pick's
  emitters and environment take their conditions from a local (a statement is under `@if` whole, and those blocks are
  long), and vertex and instance colours multiply the base colour in turn rather than in one expression (the same
  products, in the same order).
- WESL's parser is the specification's grammar, as strict as a browser's (Chrome's Tint); naga, wgpu's front end, is
  not. Through it the scene library's surface materials did not compile: their hash read `a * b ^ c * d`, which WGSL
  forbids (a bitwise operator's operands are unary expressions) and which naga took. The same in the particle passes.
  So they never compiled in a browser either; fixed 2026-11-14 with the parentheses in the order naga read them, and
  the shader files outside the kernel are held to the grammar by a unit test (gpu.rs
  `the_other_shaders_are_wgsl_as_the_specification_writes_it`). A scene's own functions go through WESL with the
  kernel: WGSL a browser would refuse is refused natively too.
- The repository's Rust is pinned (`rust-toolchain.toml`, 1.100.0): wesl-rs 0.6 asks for 1.97.1 or newer, and the
  determinism checks' recorded hashes are of one compiler.

## Android (Adreno): what a phone's Vulkan driver needed

The engine's library builds for `aarch64-linux-android` and runs under an Android client on Vulkan with ray queries.
On Qualcomm's Adreno 740 (Snapdragon 8 Gen 2, Android 13; an AYN Thor handheld) the renderer as written did not
draw correctly: a model's textures came out as another table's numbers, emissive objects lit nothing, Light Room was
black. What follows is what fixed it there, on this engine's current source, and it has run on the device ("Checked
on the device" below). All of it is off on every other platform unless asked for: with nothing set the device, its
layout and its kernels' source are what they were (`gpu.rs` `the_options_default_to_the_device_as_written`,
`the_default_sources_are_untouched`).

**The figures in the two tables are the original author's, measured on that handheld with the engine at `b646a20`,**
on scenes that are not all at hand. What was measured again on this source, on the same handheld, is under "Checked
on the device", and differs where it says so.

- **Uniform tables** (`GpuOptions::uniform_tables`, `THREERT_UNIFORM_TABLES=1|0`; default on Android; never with the
  merged layout). The driver gave the kernel other buffers' contents for four of its read-only storage buffers:
  `tex_info` (binding 14), `objects` (15), `instances` (17), `media` (18). As uniform buffers they read right.
  - The kernel's source is rewritten when a module is compiled, as the merged layout's is (`merged::uniform_tables`):
    the four declarations become sized uniform arrays at the same bindings, and every index into them is held to the
    array (`min(u32(i), last)`), so nothing is read outside a uniform buffer.
  - The renderer keeps writing its storage buffers. `Gpu::sync_tables` copies the four into their uniform buffers in
    a frame's encoder, after the particle passes (which write their objects' entries on the GPU) and before the trace.
  - A uniform binding holds 64 KiB, so the arrays are bounded: 2,048 textures, 4,096 object entries (1,365 objects;
    1,024 with instance colours), 409 instance records (model instances, procedural objects, `dynamicGeometry`
    meshes), all 32 media. A scene with more reads the last entry for the rest, and the renderer says so on stderr
    when it uploads the scene and once when in-place edits pass a bound. `THREERT_UNIFORM_TABLES=0` binds storage
    buffers again.
- **The kernel's other storage buffers stay as they are**, each for its reason:

  | buffer | size | decision |
  |---|---|---|
  | `materials` (4), `lights` (5) | 288 B and 80 B an entry | storage: the device drew right with them as they are; 64 KiB would hold 227 materials |
  | `emitters` (6) | the emitter CDF and the light tree | storage: megabytes in a lit scene |
  | `env_cdf` (10) | the environment's CDF, then the procedural geometry's data | storage (megabytes); its light sampling is off on Android, below |
  | `tri_verts` (1), `attrs` (2), `nodes` (3), `prev_verts` (16) | per triangle | storage (megabytes) |
  | `accum` (11), group 1 (radiance cache, the trace's outputs, the loop guard's count) | per pixel, per cache entry | written by the kernel: cannot be uniform |
  | group 2, the translucency pass (`timg`, `vpts`, `vcount`, `tlow`) | per pixel | written by the pass |
  | the particle passes' `systems`, `spawns`, `grid`; the grid programs' `srcs` | small, read-only | storage: other shaders, of eleven bindings at most and no acceleration structure; not known to misread (the fix on the device left them alone) |
  | the live passes (`live.wgsl`: 25 storage buffers, `ae_bins` and `ae_state` among them; at most eight a pass) | per pixel; two tiny | storage: written by the passes; the device showed their output right |

  Two of these are what a device run should look at first: `prev_verts` is the one storage buffer left after the
  acceleration structure's binding (13) and feeds the motion vectors of moving objects and the vertex colours;
  `env_cdf` also holds the procedural geometry (voxel worlds).
- **The kernel's switches** (`gpu::KernelSwitches`; flags of the kernel's conditional translation, `@if` in its source, as a scene's features are: "The kernel's source" below):

  | switch | default | what it does | on the device |
  |---|---|---|---|
  | `THREERT_KERNEL_STRICT` | Android: 1 | a ray query with a ray that is not finite, or a length that is negative or NaN, is not made (the ray misses): the kernel's two and the translucency pass's; `env_pdf` does not form the index before the first entry | the odd such ray faulted the GPU's MMU thousands of times a second and doubled the trace |
  | `THREERT_ENV_NEE` | Android: 0 | 0 = a baked environment map (an HDRI, a `SkyAtmosphere`) is not light-sampled, at load and when a dynamic sky is baked again; the sky lights through the paths' own rays, the sun is a directional light as ever | its reads of `env_cdf` went to the wrong memory: tens of thousands of GPU page faults a second, a trace up to twice as slow |
  | `THREERT_CUTOUT_LITE` | Android: 1 | hardware traversal: 1 = shadow and any-hit rays take alpha cutouts as opaque; 2 = camera rays also step through at most 4 cutouts | a map with cutout foliage: the trace 235 ms, 41 ms without cutouts |
  | `THREERT_LAST_DIRECT` | Android: 1 | the path's last vertex (at the bounce limit, past the camera hit) takes its direct light | with one bounce and no radiance cache (which cost 30-60 % more trace) sunlit ground lit nothing and interiors were black |
  | `THREERT_LIGHT_PICK` | 0 | past the camera hit, the directional lights and one of the other kinds (local lights, emitters, the sky), weighted by their number: two shadow rays a vertex instead of four; unbiased, noisier | a handheld's host sets it with `THREERT_LAST_DIRECT` |

  `THREERT_CHECKED=1` (or a mask: 2 bounds, 4 loop bounding, 8 ray-query initialization) compiles the kernel with
  naga's runtime checks, to tell a driver's fault from the kernel's.
- **The pipeline cache** (`GpuOptions::pipeline_cache`, `THREERT_PIPELINE_CACHE=<directory>`; `pipeline_cache.rs`).
  The driver compiles a kernel variant for about ten seconds. With a directory given, the device has a wgpu
  `PipelineCache` (Vulkan only), every compute pipeline of the engine and the raster pass's are made with it, and it
  is saved after a kernel variant, a scene's passes and the live passes are compiled. Opening a game a second time
  went from 10.6 s to 0.4 s (the author's; on this source a scene's pipelines compile for 16 to 31 s the first time
  and are there in under a second from the cache, below).
  - One file per adapter, named by wgpu's key (backend, vendor, device). wgpu's header in the data holds the driver's
    cache identity: after a driver update the data is not used and the next save replaces the file.
  - The data is sealed (a magic, its length, its SHA-256): a file cut short or damaged is not handed to the driver.
  - Two processes may share the directory: a save writes a file of its own name and renames it over the old one. The
    last save wins; what the other compiled meanwhile is compiled again by whoever misses it.
  - The directory may be deleted at any time. A file past 256 MiB is not read.
- **What a client sets.** `Gpu::new()` reads the environment, so an Android client that sets
  `THREERT_PIPELINE_CACHE` to a directory of its own before opening the device has everything above. In code:
  `Gpu::new_with(GpuOptions { pipeline_cache: Some(dir), ..GpuOptions::from_env() })` (`from_env` is the platform's
  defaults under the environment; `GpuOptions::default()` has no switch on).
- **What the device affords.** The author's figure for `b646a20` was 640 × 360 at 30 fps with one or two bounces
  (a tenth of the rays an iPhone 15 Pro traces). This source does not reach it: the trace costs 0.42 to 0.65 µs a
  traced pixel at one bounce whatever the scene, which is 249 × 140 to 333 × 187 traced at 30 fps ("Checked on the
  device").
- **Dynamic resolution by the clock** (`THREERT_GOVERN_CLOCK=1|0`, default on Android; `live/mod.rs`
  `govern_resolution`). The render scale was governed in frames (a check every 30th, the urgent one after the first
  120), which is twice a second and 2 s only at 60 frames a second. A scene opens at its full scale, on this GPU at
  about 6 frames a second, so it was a slide show for 25 s. With the clock it is at its frame rate 3 s after its
  first frame.
- **Not carried over from the original change:** it also sorted bind group layout entries and bind group entries by
  binding, for a driver thought to mind their order. wgpu puts both in binding order itself before its backend sees
  them (wgpu-core 30 `bgl::EntryMap::from_entries`, `Device::create_bind_group`), so the sort changed nothing; a test
  holds wgpu to it (`wgpu_takes_bind_group_entries_in_any_order`).

**Checked on the device** (AYN Thor: Snapdragon 8 Gen 2, Adreno 740, Android 13, driver 0676.53; 2026-11-13, this
source under ThreeRT for Android 0.4.0, which sets `THREERT_PIPELINE_CACHE` and nothing else):

- **It draws right.** The live view's frame (the client's `POST /screenshot`, 1280 × 720 or 960 × 540, 32 to 88
  frames of history) against the same scene's picture from a Mac (the published cover, same camera): mean absolute
  difference per channel, of 255: Material balls 0.8, Light Room 1.3 (it was black), Damaged Helmet 1.6 (its
  textures were another table's numbers), Boutique Showroom 3.0 (its shelf strips and spots light the room).
  OpenStrikeRT's hall from the place its page's picture was taken: the same walls, pillars, crates, signs, tube
  lights, the sun through the skylight on the floor, the furnace's glow; soldiers and their weapons (instanced,
  posed) are whole and in their teams' colours. No GPU fault in the kernel log (`dmesg`, kgsl) in any run.
- **The bounds.** OpenStrikeRT (104,898 triangles, 271 objects) with nine bots stays inside the uniform tables:
  the renderer's warning does not appear.
- **The pipeline cache works.** A first open, after the package is on the device: OpenStrikeRT 31 s, Boutique
  Showroom 18 s, Material balls 16 s (kernel variants and the live passes compiling). The same after a restart of
  the app: 0.9 s, 1.7 s (1.1 s of it the scene's setup), 0.5 s.
- **What a frame costs.** GPU time at 640 × 360 traced (scale 1), one bounce, Android's defaults: Hello (31,362
  triangles) 101 ms, Material balls (217,730) 96 ms, Sunlit room (6,400) 149 ms, Boutique Showroom (131,200)
  139 ms, OpenStrikeRT 146 ms; with ray tracing off (the camera ray alone) OpenStrikeRT still takes 90 ms. Of
  Material balls' frame at 1280 × 720 out, 416 × 234 traced: the trace 40.7 ms, every other pass together 9 ms
  (upscale 1.7 ms, the denoisers 4 ms); at 960 × 540 out the others take about 5 ms. So the trace is the frame,
  it goes with the pixels traced and not with the scene, and 30 fps is 249 × 140 (OpenStrikeRT, the showroom) to
  333 × 187 (Material balls, two bounces) under a 960 × 540 view; 40 fps is 249 × 140 to 291 × 164 under
  853 × 480 (36 to 37 fps reached); 60 fps is 192 × 108 to 224 × 126 under 640 × 360 (54 to 58 reached).
- **The switches, one at a time** (OpenStrikeRT, 640 × 360 traced, one bounce; 146 ms as shipped; one run each):
  `THREERT_LIGHT_PICK=1` 137 ms; `THREERT_LAST_DIRECT=0` 116 ms; `THREERT_UNIFORM_TABLES=0` 135 ms (so the uniform
  tables and their index clamps cost about 8 %); `THREERT_KERNEL_STRICT=0` 151 ms (no slower with the guard, and no
  fault without it in these scenes); `THREERT_CACHE=0` with the light pick 135 ms (Material balls: 96.9 against
  96.5 ms: the radiance cache costs nothing measurable here). None of them is the factor of four to the author's
  figure.
- **Hardware against software traversal** (`THREERT_HWRT=0`, same size and bounce): Sunlit room 78 ms against
  149 ms and Material balls 74 against 96 are faster without ray queries; OpenStrikeRT is 336 against 146. A ray
  query has a high fixed cost on this driver; ray queries stay the default. `THREERT_RASTER=1` (raster primary
  visibility): Sunlit room 136 against 149 ms.
- **Not looked at on the device:** voxel worlds and procedural geometry (`env_cdf`'s second half), particles, media,
  a map with cutout foliage (`THREERT_CUTOUT_LITE`'s figures are the author's), `THREERT_ENV_NEE=1`, and the
  motion vectors of moving objects (`prev_verts`): soldiers were looked at standing, not frame by frame in motion.

**Checked on a Mac** (Apple M3 Max, Metal, 2026-11-13; main `f69b1bc` against this change):
- The atomic tests (`npm run test:render`, 187), run in full on main's build, on this build, and on this build
  with `THREERT_UNIFORM_TABLES=1` (the last two twice): every test passes in every run but `geometry/grow-live`
  (below). Their HDR outputs, byte for byte: all 126 offline renders are identical in all five runs. Of the 60 other
  live tests, 41 gave the same bytes in every run, with the uniform tables and without. The other 19 differ between
  two runs without the uniform tables, and by as much as with them (18 of them run three or four more times both
  ways: about as many differing numbers and as large a relative difference, 10⁻⁶ to 0.6 by test).
- Under software traversal (`THREERT_HWRT=0`) the same 24 tests fail with and without the uniform tables (particles,
  translucency: hardware traversal's), and 173 of 187 outputs are identical. The other 14 are live tests: 13 of them
  differ from run to run without the uniform tables too, and `animation/shared-pose-live` was identical both ways
  when run again.
- With the uniform tables and `THREERT_KERNEL_STRICT=1`: the 126 offline renders are still main's bytes. With all of
  Android's defaults: 187 pass, and 117 of the 126 are main's bytes (nine differ as the last vertex's direct light
  and the unsampled sky intend).
- `geometry/grow-live` adds 1,100 `dynamicGeometry` objects in a frame and often stops at the script's deadline
  ("interrupted") on this machine while other jobs run: on main's build in 4 of the 7 times it was run, on this
  build more often than not, whatever is set. Where it ran, it passed under hardware traversal in every configuration. Its 1,102 instance
  records are past the uniform tables' 409, which the renderer says; under software traversal with the uniform
  tables its centre reads 1 (the sky) instead of 0.5, both times it was run. That is the bound above, at work: a
  scene past it is not drawn right with the uniform tables.
- `cargo test --release` passes as written, with `THREERT_UNIFORM_TABLES=1`, and with all of Android's defaults.
  `live::the_uniform_tables_render_what_the_tables_do` renders a scene offline on a device with and without the
  uniform tables to the same bytes and reads the uniform buffers back against their tables, before and after an
  edit in place.
- The library checks for `aarch64-linux-android` (NDK 27.1, `cargo ndk -t arm64-v8a --platform 29 check --release
  --lib --features "winit/android-native-activity rquickjs/bindgen"`, with `LIBCLANG_PATH` and the NDK sysroot for
  bindgen) and for wasm32.
- Not checked here: the pipeline cache against a driver (Metal has none: its file handling is tested, its wgpu
  calls only compile), and anything on an Adreno.

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
