// Khronos glTF Sample Asset MetalRoughSpheres (render suite layer B): metallic x roughness grid of spheres (core glTF PBR).
import { studio, type Exhibit } from "./khronos/studio.ts";

export const exhibit: Exhibit = { name: "khronos-metal-rough", features: ["metal-rough", "fresnel"], file: "MetalRoughSpheres.glb", min: [-4.875, -4.944, -3.6], max: [4.75, 4.388, 0.4], distance: 16 };

export default () => studio(exhibit);
