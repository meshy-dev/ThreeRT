// Interior GI coverage: sun through a window, sky light through an opening, multi-bounce, emissive lamp.
import { Scene, PerspectiveCamera, Mesh, BoxGeometry, SphereGeometry, CylinderGeometry, MeshStandardMaterial, DirectionalLight, SkyAtmosphere, Color, Group } from "@threert/scene";

export default function build() {
  const scene = new Scene();
  const sun = new DirectionalLight(0xffffff, 90000);
  sun.name = "sun";
  sun.setSunAngles(28, 100); // low afternoon sun from +X through the window
  scene.add(sun);
  scene.environment = new SkyAtmosphere({ sun });

  const plaster = new MeshStandardMaterial({ name: "plaster", color: 0xd9d4cc, roughness: 0.92 });
  const floorMat = new MeshStandardMaterial({ name: "oak", color: 0x8c6239, roughness: 0.55 });
  const fabric = new MeshStandardMaterial({ name: "fabric_blue", color: 0x2e4a7a, roughness: 0.95 });
  const ceramic = new MeshStandardMaterial({ name: "ceramic", color: 0xf2efe8, roughness: 0.15 });
  const brass = new MeshStandardMaterial({ name: "brass", color: new Color().setRGB(0.91, 0.78, 0.42), metalness: 1, roughness: 0.3 });
  const lampShade = new MeshStandardMaterial({ name: "lamp_shade", color: 0xfff1dc, emissive: new Color().setRGB(1.0, 0.78, 0.5), emissiveIntensity: 2500, roughness: 0.9 });

  const room = new Group();
  room.name = "room";
  scene.add(room);
  const box = (name: string, w: number, h: number, d: number, x: number, y: number, z: number, mat: MeshStandardMaterial, parent = room) => {
    const m = new Mesh(new BoxGeometry(w, h, d), mat);
    m.name = name;
    m.position.set(x, y, z);
    parent.add(m);
    return m;
  };
  const W = 6, H = 3, D = 6, T = 0.3;
  box("floor", W + 2 * T, T, D + 2 * T, 0, -T / 2, 0, floorMat);
  box("ceiling", W + 2 * T, T, D + 2 * T, 0, H + T / 2, 0, plaster);
  box("wall_back", W + 2 * T, H, T, 0, H / 2, -D / 2 - T / 2, plaster);
  box("wall_front", W + 2 * T, H, T, 0, H / 2, D / 2 + T / 2, plaster);
  box("wall_left", T, H, D, -W / 2 - T / 2, H / 2, 0, plaster);
  // +X wall with a 2.2 x 1.6 m window centered at z = -0.6, sill at 0.8 m
  const wx = W / 2 + T / 2, winW = 2.2, winH = 1.6, sill = 0.8, wz = -0.6;
  const zMin = -D / 2, zMax = D / 2;
  box("wall_right_a", T, H, wz - winW / 2 - zMin, wx, H / 2, (zMin + wz - winW / 2) / 2, plaster);
  box("wall_right_b", T, H, zMax - (wz + winW / 2), wx, H / 2, (wz + winW / 2 + zMax) / 2, plaster);
  box("wall_right_sill", T, sill, winW, wx, sill / 2, wz, plaster);
  box("wall_right_lintel", T, H - sill - winH, winW, wx, (H + sill + winH) / 2, wz, plaster);

  // furniture
  box("sofa_base", 2.2, 0.45, 0.9, -1.4, 0.225, -2.2, fabric);
  box("sofa_back", 2.2, 0.5, 0.25, -1.4, 0.7, -2.55, fabric);
  box("table", 1.2, 0.06, 0.7, 0.2, 0.45, -0.9, floorMat);
  for (const [i, [x, z]] of [[-0.3, -1.15], [0.7, -1.15], [-0.3, -0.65], [0.7, -0.65]].entries()) {
    box(`table_leg_${i}`, 0.05, 0.42, 0.05, x, 0.21, z, floorMat);
  }
  const vase = new Mesh(new SphereGeometry(0.16, 64, 48), ceramic);
  vase.name = "vase";
  vase.position.set(0.0, 0.64, -0.9);
  scene.add(vase);
  const pole = new Mesh(new CylinderGeometry(0.02, 0.02, 1.5, 24), brass);
  pole.name = "lamp_pole";
  pole.position.set(-2.6, 0.75, -2.5);
  scene.add(pole);
  const shade = new Mesh(new CylinderGeometry(0.16, 0.22, 0.3, 48, 1, true), lampShade);
  shade.name = "lamp_shade";
  shade.material.side = "double";
  shade.position.set(-2.6, 1.6, -2.5);
  scene.add(shade);

  const camera = new PerspectiveCamera(58, 16 / 9, 0.05, 200);
  camera.name = "camera";
  camera.position.set(-2.3, 1.45, 2.4);
  camera.lookAt(1.0, 0.9, -1.2);
  camera.setManualExposure(10.2);
  scene.add(camera);
  return { scene, camera, options: { name: "sun-room" } };
}
