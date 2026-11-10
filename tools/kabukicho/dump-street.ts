// Dumps the Kabukichō street as the DSL builds it (kind, position, scale per object) for verify-port.py.
//   node tools/kabukicho/dump-street.ts > out/kb-street.json
import { GLTFModel, Light, lightDesc, Mesh } from "@threert/scene";
import { buildStreet } from "../../games/kabukicho/street.ts";

const nodes: { id: string; kind: string; position: number[]; scale: number[] }[] = [];
for (const o of buildStreet(4).root.children) {
  const kind = o instanceof Light ? `light:${lightDesc(o).type}` : o instanceof GLTFModel ? "model" : o instanceof Mesh ? `geometry:${o.geometry.type}` : null;
  if (kind) nodes.push({ id: o.stableId ?? o.name, kind, position: o.position.toArray(), scale: o.scale.toArray() });
}
process.stdout.write(JSON.stringify({ nodes }));
