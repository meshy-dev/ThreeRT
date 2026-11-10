// Khronos glTF Sample Asset ClearCoatTest (render suite layer B): KHR_materials_clearcoat: rows of spheres with clearcoat amount, roughness and normal variations.
import { studio, type Exhibit } from "./khronos/studio.ts";

export const exhibit: Exhibit = { name: "khronos-clearcoat", features: ["clearcoat"], file: "ClearCoatTest.glb", min: [-7.358, -6.25, -0.06], max: [3.1, 6.855, 1.05], distance: 30 };

export default () => studio(exhibit);
