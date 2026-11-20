# Showreel

Small scenes of light and physics together, made for ThreeRT's launch video and explainers and now in the landing page's reel (`tools/site-reel.ts`, `site/tools/reel.ts`).

| Scene | What | Event |
|---|---|---|
| `cascade.game.ts` | Four hundred balls, a third of them glowing, poured down a polished staircase: each lights the steps it lands on | `go` |
| `wreck.game.ts` | A wrecking ball through a round tower of bricks at last light, a quarter of them glowing | `go` |
| `lanterns.game.ts` | A dark hall hung with 400 lanterns over a polished floor: every one lights and casts shadows | |

```bash
node tools/bundle-game.ts games/showreel/wreck.game.ts -o out/bundles/wreck.js
renderer/target/release/threert-render play out/bundles/wreck.js     # then send the `go` event (Space)
```
