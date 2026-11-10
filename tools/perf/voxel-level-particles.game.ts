// The benchmark level by day (games/voxel-level/level-native.game.ts) with a fountain of stateful GPU particles ahead of
// the camera (ROADMAP § 6 step 5a's acceptance, tools/bench-gpu-particles.ts): `count` particles alive at once (born at
// count / 4 a second, each living 4 s), thrown up in a cone, falling under gravity onto the level's voxels and the
// ground, lit (their colour the material function's `i.custom`). Every particle is updated, collided and placed each
// frame, and the dynamic BLAS of all of them rebuilt.
//   node tools/bundle-game.ts tools/perf/voxel-level-particles.game.ts            (100,000)
//   node tools/bundle-game.ts tools/perf/voxel-level-particles-10k.game.ts        (10,000)
import { GpuParticles, MaterialFunction, MeshStandardMaterial, ParticleProgram } from "@threert/scene";
import { defineGame } from "@threert/gameplay";
import { nativeLevelGame } from "../../games/voxel-level/level-native.game.ts";
import { cameraPath, groundHeight, SIZE, VOXEL } from "../../games/voxel-level/plan.ts";

const FOUNTAIN = new ParticleProgram("perf_fountain", {
  params: { Speed: 6, Spread: 0.5, Size: 0.025, Life: 4 },
  emit: `
    let ct = 1.0 - i.rand.x * (1.0 - cos($Spread));
    let st = sqrt(max(0.0, 1.0 - ct * ct));
    let ph = 6.2831853 * i.rand.y;
    s.vel = vec3<f32>(st * cos(ph), ct, st * sin(ph)) * ($Speed * (0.5 + 0.5 * i.rand.z));
    s.life = $Life;
    s.size = $Size;
    s.color = vec4<f32>(mix(vec3<f32>(0.8, 0.35, 0.1), vec3<f32>(0.9, 0.8, 0.6), i.rand.w), 1.0);`,
  update: `
    s.vel.y -= 9.81 * i.dt;`,
});

const SURFACE = new MaterialFunction("perf_fountain_surface", { code: `
    o.base = i.custom.rgb;` });

export function particlesGame(count: number) {
  const base = nativeLevelGame("material", 1, 0.2);
  return defineGame<Record<string, never>>({
    ...base,
    setup(ctx) {
      const r = base.setup(ctx);
      const key = cameraPath(1)[0];
      const [cx, cy, cz] = key.pos, [tx, , tz] = key.target;
      const len = Math.hypot(tx - cx, tz - cz);
      const [x, z] = [cx + ((tx - cx) / len) * 9, cz + ((tz - cz) / len) * 9];
      const ground = groundHeight(1, Math.round(x / VOXEL + SIZE / 2), Math.round(z / VOXEL + SIZE / 2)) * VOXEL;
      const material = new MeshStandardMaterial({ name: "fountain", color: 0xffffff, roughness: 0.8 });
      material.shader = SURFACE.with({});
      const p = new GpuParticles({ capacity: count, program: FOUNTAIN.with(), material, rate: count / 4, collide: { voxels: true, plane: Math.min(ground, cy - 10), restitution: 0.3, friction: 0.4 } });
      p.name = "fountain";
      p.position.set(x, ground + 0.3, z);
      r.scene.add(p);
      return { ...r, options: { ...r.options, name: `voxel-level-particles-${count}` } };
    },
  });
}

export default particlesGame(100_000);
