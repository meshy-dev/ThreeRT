# Gameplay vs frame scripts

A game has two kinds of logic. Put each behaviour on the right side.

| | handlers (`handlers`, `tick`) | frame script (`frame(f)`) |
|---|---|---|
| UE equivalent | server gameplay: replicated state, RPCs | an actor's Tick on a client, MPC / MID animation, Niagara |
| Runs | in the room (or locally in `play`), once per event | on the render node, every rendered frame, and with `dt = 0` after setup / commits |
| Reads / writes | the game state (`tx.state`): committed, logged, replayed | reads committed `f.state`; changes the scene (`f.scene`, `f.camera`, `f.object(id)`) |
| Use for | rules, scores, targets ("the shop is closed", "it is raining") | what the scene shows for that state, and how it gets there: flicker, ramps, staggered switching |
| Randomness / time | `tx.random()`, `tx.now` (deterministic) | `f.random()`, `f.time`, `f.dt`, `f.now`; module variables may hold animation state |

Handlers never touch the scene: the world is the state. The scene is a function of it, computed by `frame` on every
node that renders the room.

Pattern (the UE Kabukichō Director, `games/kabukicho/kabukicho.game.ts`):

- The handler commits a **target** in the state:
  ```ts
  rain(tx) {
    tx.state.rain = tx.state.rain > 0.5 ? 0 : 1;
  }
  ```
- The frame script eases toward it, and shows it settled when `f.snap` (after setup or a replaced state, e.g. a
  render of a room's state):
  ```ts
  frame(f) {
    cur = f.snap ? f.state.rain : constTo(cur, f.state.rain, f.dt, 0.6);
    f.scene.rain = cur;
  }
  ```
- Objects the state creates or removes (balls, coins): reconcile in `frame` (add what the state has and the scene
  lacks, remove the rest), keyed by ids stored in the state.
- For events with a duration (a power cut), store `tx.now` in the state. The frame script compares it with `f.now`.

Cost on the renderer (objects report their own changes, so untouched objects cost nothing):

- **Cheap, every frame is fine:** `power`, light `intensity` / `color`, material `emissiveIntensity` / `color` /
  `roughness` / …, material-function parameters (`material.shader.params`, `needsUpdate`), `scene.wetness` / `rain`,
  fog density / extinction, the camera. These rewrite GPU table entries in place.
- **Expensive, avoid per frame on big scenes:** `position` / `quaternion` / `scale` / `visible`, adding or removing
  objects. These rebuild the GPU scene (about 0.7 s for Kabukichō; milliseconds for small rooms).

Run and measure:

```bash
node tools/bundle-game.ts games/x/x.game.ts                                    # -> out/bundles/x.js
renderer/target/release/threert-render play out/bundles/x.js                   # window; the game's `keys` work
renderer/target/release/threert-render live out/bundles/x.js --scale 0.6       # headless; stdin:
#   {"cmd":"event","type":"rain"}  {"cmd":"click","u":0.5,"v":0.4}  {"cmd":"snapshot","path":"x.png"}
#   stderr reports the frame-script cost (script + commit) every 2 s; keep it under ~1 ms
renderer/target/release/threert-render edit-check out/bundles/x.js --events '[{"type":"rain"}]'   # in place == rebuilt
```
