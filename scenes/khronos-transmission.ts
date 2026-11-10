// Khronos glTF Sample Asset TransmissionTest (render suite layer B): KHR_materials_transmission: thin and solid transmissive shapes in front of a patterned backdrop.
import { studio, type Exhibit } from "./khronos/studio.ts";

export const exhibit: Exhibit = { name: "khronos-transmission", features: ["transmission", "refraction"], file: "TransmissionTest.glb", min: [-0.976, -0.4, -0.521], max: [0.826, 0.621, 1.368], distance: 2.2 };

export default () => studio(exhibit);
