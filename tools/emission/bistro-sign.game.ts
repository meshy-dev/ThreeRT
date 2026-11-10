// Bistro with a sign that switches on (tools/bench-emission.ts; ROADMAP § 6 step 3): a panel beside the street view of
// a scene-code material that emits nothing until the "toggle" event, then 20 nits. Before step 3 its first switch-on
// rebuilt the scene (a material without emitter entries); now its triangles get them in place.
import { Color, Mesh, MeshStandardMaterial, PlaneGeometry } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import build, { STREET } from "../../scenes/bistro.ts";

const sign = new MeshStandardMaterial({ name: "sign", color: new Color(0.05, 0.05, 0.05), emissive: new Color(0, 0, 0), roughness: 0.6 });

export default defineGame<{ on: boolean }>({
  state: { on: false },
  setup: () => {
    const r = build();
    const panel = new Mesh(new PlaneGeometry(2, 1), sign);
    panel.name = "sign";
    panel.position.set(STREET.target[0] + 1.5, STREET.target[1] + 1.2, STREET.target[2] - 3);
    panel.lookAt(...STREET.position);
    r.scene.add(panel);
    return r;
  },
  frame(f) {
    const on = f.state.on;
    if (on !== sign.emissiveIntensity > 0) {
      sign.emissive = on ? new Color(1, 0.6, 0.3) : new Color(0, 0, 0);
      sign.emissiveIntensity = on ? 20 : 0;
    }
  },
  handlers: {
    toggle: (tx) => {
      tx.state.on = !tx.state.on;
    },
  },
});
