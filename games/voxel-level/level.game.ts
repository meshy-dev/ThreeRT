// The benchmark level (docs/VOXEL.md, "The benchmark level"; ROADMAP § 6 step 2): its objects placed from the plan
// (plan.ts), their voxels read by the renderer from the assets tools/voxel-level/generate.ts built (one directory per
// seed and box granularity; or the greedy-meshed triangles, one GLB per geometry), never through the scene runtime. Sun and
// sky only: the lamps' voxels emit, unsampled, as boxes (procedural surfaces are not light-sampled) and not at all as the
// GLB meshes, so the representations are lit alike (at night, lit by the lamps: level-night.game.ts, § 6 step 4). A
// ground plane runs from the level's edge to the horizon. The camera starts on the
// level's camera path (cameraPath; the `view` event steps through its keys).
//   node tools/voxel-level/generate.ts --granularity region && node tools/bundle-game.ts games/voxel-level/level.game.ts
import { BufferGeometry, DirectionalLight, GLTFModel, Mesh, MeshStandardMaterial, PerspectiveCamera, PlaneGeometry, ProceduralGeometry, ProceduralMesh, Scene, SkyAtmosphere } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { VOXEL_BRICKS } from "@threert/voxel";
import { BUILDING_PARTS, EDGE, PALETTE, type Variant, SIZE, VOXEL, assetDir, buildingGeometry, cameraPath, levelPlan, propGeometry, tileGeometry, toScene } from "./plan.ts";

/**
 * A heightfield of 64 x 64 voxel-sized cells (8192 triangles) whose waves are shifted by `phase`: the mesh the edits
 * measurement swaps (`edits`).
 */
function heightfield(phase: number): BufferGeometry {
  const n = 64, p: number[] = [], nm: number[] = [], uv: number[] = [], idx: number[] = [];
  for (let z = 0; z <= n; z++) for (let x = 0; x <= n; x++) {
    p.push(x * VOXEL, 0.3 * Math.sin(x / 7 + phase) * Math.cos(z / 9 - phase), z * VOXEL);
    nm.push(0, 1, 0);
    uv.push(x / n, z / n);
  }
  for (let z = 0; z < n; z++) for (let x = 0; x < n; x++) {
    const a = z * (n + 1) + x;
    idx.push(a, a + n + 1, a + 1, a + 1, a + n + 1, a + n + 2);
  }
  return new BufferGeometry().setData(p, nm, uv, idx);
}

/**
 * `edits` (ROADMAP § 6 step 2c-4, live/edits.rs `edit_geometry`): every half second, one object's geometry is replaced: with
 * box representations, the procedural tile under the camera's start swaps between its own voxels and its neighbour's;
 * with the mesh, a `dynamicGeometry` heightfield there swaps between two shapes. The frame log's `edit` / `rebuild`
 * time it (tools/bench-voxel.ts --edits).
 */
export function levelGame(granularity: Variant, seed = 1, distinct = false, edits = false) {
  const keys = cameraPath(seed);
  let shown = 0;
  let frames = 0;
  let swap: { mesh: ProceduralMesh | Mesh; shapes: (ProceduralGeometry | BufferGeometry)[]; k: number } | null = null;
  return defineGame<{ view: number }>({
    state: { view: 0 },
    setup: () => {
      const scene = new Scene();
      const sun = new DirectionalLight(0xffffff, 90000);
      sun.setSunAngles(38, 300);
      scene.add(sun);
      scene.environment = new SkyAtmosphere({ sun });
      const camera = new PerspectiveCamera(55, 16 / 9, 0.05, 2000);
      camera.name = "camera";
      camera.position.set(...keys[0].pos);
      camera.lookAt(...keys[0].target);
      camera.setManualExposure(13);
      scene.add(camera);

      const mats = PALETTE.map(([name, color, roughness, metalness, glow]) => new MeshStandardMaterial({ name, color, roughness, metalness, ...(glow > 0 ? { emissive: color, emissiveIntensity: glow } : {}) }));
      // Ground from the level's edge (where the terrain comes down to EDGE) to the horizon: four strips around the level,
      // never over it (its valleys lie below EDGE).
      const half = (SIZE / 2) * VOXEL, far = 100000;
      for (const [w, d, x, z] of [[2 * far, far - half, 0, -(far + half) / 2], [2 * far, far - half, 0, (far + half) / 2], [far - half, 2 * half, -(far + half) / 2, 0], [far - half, 2 * half, (far + half) / 2, 0]]) {
        const strip = new Mesh(new PlaneGeometry(w, d), mats[1]);
        strip.name = "horizon";
        strip.rotation.x = -Math.PI / 2;
        strip.position.set(x, EDGE * VOXEL, z);
        scene.add(strip);
      }

      const dir = assetDir(seed, granularity, distinct);
      const geometries = new Map<string, ProceduralGeometry>();
      const geometry = (name: string) => {
        let g = geometries.get(name);
        if (!g) geometries.set(name, (g = ProceduralGeometry.fromAsset(VOXEL_BRICKS, `${dir}/${name}.tpg`)));
        return g;
      };
      const plan = levelPlan(seed);
      const place = (name: string, geo: string, pos: [number, number, number], turn = 0) => {
        // The meshes: each file placed as a model (one BLAS per file, an instance per placement, as the procedural
        // geometries are shared).
        const m = granularity === "mesh" ? new GLTFModel(`${dir}/${geo}.glb`) : new ProceduralMesh(geometry(geo), mats);
        m.name = name;
        m.position.set(...pos);
        m.rotation.y = (turn * Math.PI) / 2;
        scene.add(m);
      };
      for (const t of plan.tiles) place(t.name, tileGeometry(seed, t), toScene(t.tx * 64, 0, t.tz * 64));
      for (const b of plan.buildings) for (const part of BUILDING_PARTS) place(`${b.name}-${part}`, buildingGeometry(b, part), toScene(b.x, b.base, b.z));
      plan.props.forEach((p, k) => place(`${p.kind}-${k}`, propGeometry(p, k, distinct), toScene(p.x, p.y, p.z), p.turn));
      if (edits) {
        const [x, , z] = keys[0].pos;
        const tx = Math.floor((x / VOXEL + SIZE / 2) / 64), tz = Math.floor((z / VOXEL + SIZE / 2) / 64);
        if (granularity === "mesh") {
          const mesh = new Mesh(heightfield(0), mats[1]);
          mesh.name = "edited";
          mesh.dynamicGeometry = true;
          mesh.position.set(...toScene(tx * 64, EDGE + 60, tz * 64));
          scene.add(mesh);
          swap = { mesh, shapes: [heightfield(0), heightfield(1.5)], k: 0 };
        } else {
          const tile = scene.getObjectByName(`tile-${tx}-${tz}`) as ProceduralMesh;
          const other = plan.tiles.find((t) => t.tx === tx + 1 && t.tz === tz)!;
          swap = { mesh: tile, shapes: [tile.geometry, geometry(tileGeometry(seed, other))], k: 0 };
        }
      }
      return { scene, camera, options: { name: `voxel-level-${granularity}${distinct ? "-distinct" : ""}${edits ? "-edits" : ""}`, width: 1280, height: 720, preset: "cinematic" as const } };
    },
    // Only when the view event asks: otherwise the camera is the viewer's (or a benchmark's camera commands).
    frame(f) {
      if (swap && ++frames % 30 === 0) {
        swap.k ^= 1;
        if (swap.mesh instanceof Mesh) swap.mesh.geometry = swap.shapes[swap.k] as BufferGeometry;
        else swap.mesh.geometry = swap.shapes[swap.k] as ProceduralGeometry;
      }
      if (f.state.view === shown) return;
      shown = f.state.view;
      const k = keys[shown % keys.length];
      f.camera.position.set(...k.pos);
      f.camera.lookAt(...k.target);
    },
    handlers: {
      view: (tx) => {
        tx.state.view++;
      },
    },
  });
}

export default levelGame("region");
