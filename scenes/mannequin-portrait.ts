// Mannequin portrait (depth of field): scenes/mannequin.ts through a portrait lens, as a UE CineCamera with a 36 mm filmback would take it: 20 deg vertical FOV (57 mm) at f/1.2, focused on the dancer 5.9 m away.
// UE ground truth: PORTRAIT=1 bun ue5/mannequin.ts (keep its numbers in sync). The plaza game's F key uses the same lens.
import build from "./mannequin.ts";

export const PORTRAIT = { target: [1.65, 0.9, -0.15] as const, fov: 20, dof: { focusDistance: 5.9, fStop: 1.2, sensorWidth: 36 } };

export default function () {
  const r = build();
  r.camera.lookAt(...PORTRAIT.target);
  r.camera.fov = PORTRAIT.fov;
  r.camera.dof = { ...PORTRAIT.dof };
  return { ...r, options: { ...r.options, name: "mannequin-portrait" } };
}
