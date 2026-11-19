// What the showreel's physics demos share (they were made for the launch video): the dark studio (a polished floor under a black sky, lit by what glows
// in the scene), the palette of glowing and plain materials, a seeded random.
import { BoxGeometry, Color, Mesh, MeshStandardMaterial, RectAreaLight, Scene, SkyColor } from "@threert/scene";

/** A seeded random (mulberry32): the same scene every run. */
export function seeded(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Glowing colours (linear-ish sRGB hex): tungsten first, the brand's light. */
export const GLOWS = [0xff7a1a, 0xffb454, 0xffe9c4, 0x35c8ff, 0xff3d8b, 0x8dff5a, 0x8a6bff];
/** A material that glows at `nits`. */
export const glow = (name: string, colour: number, nits: number) =>
  new MeshStandardMaterial({ name, color: new Color(colour).multiplyScalar(0.35), roughness: 0.5, emissive: colour, emissiveIntensity: nits });
/** Plain materials worth a ray: metals, lacquer, ceramic, rubber. */
export const plain = () => [
  new MeshStandardMaterial({ name: "gold", color: new Color().setRGB(1.0, 0.766, 0.336), metalness: 1, roughness: 0.12 }),
  new MeshStandardMaterial({ name: "chrome", color: 0xe6e8ea, metalness: 1, roughness: 0.04 }),
  new MeshStandardMaterial({ name: "copper", color: new Color().setRGB(0.955, 0.638, 0.538), metalness: 1, roughness: 0.22 }),
  new MeshStandardMaterial({ name: "lacquer", color: 0xc81e1e, roughness: 0.08 }),
  new MeshStandardMaterial({ name: "ceramic", color: 0xe9e6df, roughness: 0.3 }),
  new MeshStandardMaterial({ name: "rubber", color: 0x1b1d22, roughness: 0.75 }),
  new MeshStandardMaterial({ name: "enamel", color: 0x1f5fd0, roughness: 0.15 }),
];

/** The studio: a dark sky with a breath of cool light, a soft box overhead, a polished floor (a fixed body, its top at
 *  y = 0). What glows in the scene does the rest. */
export function studio(scene: Scene, o: { size?: number; floor?: number; roughness?: number; sky?: number; key?: number; keyAt?: [number, number, number]; keySize?: number } = {}): Mesh {
  scene.environment = new SkyColor(0x8fa3c8, o.sky ?? 0.5);
  const size = o.size ?? 60;
  const floor = new Mesh(new BoxGeometry(size, 1, size), new MeshStandardMaterial({ name: "floor", color: o.floor ?? 0x6a6e76, roughness: o.roughness ?? 0.22 }));
  floor.name = "floor";
  floor.position.set(0, -0.5, 0);
  floor.physics = { type: "fixed", friction: 0.9, restitution: 0.35 };
  scene.add(floor);
  if (o.key !== 0) {
    const key = new RectAreaLight(0xfff2e2, o.key ?? 700, o.keySize ?? 7, o.keySize ?? 7);
    key.position.set(...(o.keyAt ?? [1.5, 8, 2.5]));
    key.lookAt(0, 0, 0);
    scene.add(key);
  }
  return floor;
}
