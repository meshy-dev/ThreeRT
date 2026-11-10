// Procedural data through a rebuild (ROADMAP § 6 step 2a): four procedural boxes carrying `PROC_DATA_MB` of game data
// between them (random words: a voxel level's bricks stand-in), lit by a sky that the `sky` event recolours, which
// rebuilds the GPU scene. `node tools/bench-hitches.ts --bundle out/bundles/proc-data-scene.js --events
// '[[3,"sky"],[6,"sky"],[9,"sky"]]'` measures the hitch; proc-data-scene-none.ts is the same scene without data.
//   node tools/bundle-game.ts tools/proc-data-scene.ts -o out/bundles/proc-data-scene.js
import { DirectionalLight, GeometryFunction, MeshStandardMaterial, PerspectiveCamera, ProceduralGeometry, ProceduralMesh, Scene, SkyColor } from "@threert/scene";
import { defineGame } from "@threert/gameplay";

/** A solid box; it reads its first data word so the data is in use. */
const BOX = new GeometryFunction("data_box", {
  code: `
    let inv = 1.0 / i.d;
    let t0 = (i.lo - i.o) * inv;
    let t1 = (i.hi - i.o) * inv;
    let a = max(max(min(t0.x, t1.x), min(t0.y, t1.y)), min(t0.z, t1.z));
    let b = min(min(max(t0.x, t1.x), max(t0.y, t1.y)), max(t0.z, t1.z));
    if (a <= b && a > i.tmin && a < i.tmax) {
        let tn = min(t0, t1);
        o.t = a;
        o.normal = select(select(vec3<f32>(0.0, 0.0, -sign(i.d.z)), vec3<f32>(0.0, -sign(i.d.y), 0.0), tn.y == a), vec3<f32>(-sign(i.d.x), 0.0, 0.0), tn.x == a);
        o.material = gdata(i.data) & 1u;
    }`,
});

export function dataScene(mb: number) {
  return defineGame<{ warm: boolean }>({
    state: { warm: false },
    setup: () => {
      const scene = new Scene();
      scene.environment = new SkyColor(0xb0c8ff, 2000);
      const sun = new DirectionalLight(0xffffff, 50000);
      sun.position.set(0.4, 1, 0.3);
      scene.add(sun);
      const words = Math.floor((mb * 1024 * 1024) / 4 / 4);
      let s = 12345;
      const mats = [new MeshStandardMaterial({ name: "a", color: 0xc04030 }), new MeshStandardMaterial({ name: "b", color: 0x3060c0 })];
      for (let k = 0; k < 4; k++) {
        const data = new Uint32Array(Math.max(words, 1));
        for (let i = 0; i < data.length; i++) data[i] = s = (Math.imul(s, 1664525) + 1013904223) >>> 0;
        const m = new ProceduralMesh(new ProceduralGeometry(BOX, [-0.5, 0, -0.5, 0.5, 1, 0.5], data), mats);
        m.position.set(k * 1.5 - 2.25, 0, 0);
        scene.add(m);
      }
      const camera = new PerspectiveCamera(45, 16 / 9, 0.05, 100);
      camera.position.set(0, 1.5, 6);
      camera.lookAt(0, 0.5, 0);
      camera.setManualExposure(12);
      scene.add(camera);
      return { scene, camera, options: { name: "proc-data-scene", width: 1280, height: 720, preset: "cinematic" as const } };
    },
    frame(f) {
      (f.scene.environment as SkyColor).color.setHex(f.state.warm ? 0xffc890 : 0xb0c8ff);
    },
    handlers: {
      sky: (tx) => {
        tx.state.warm = !tx.state.warm;
      },
    },
  });
}

export default dataScene(Number(256));
