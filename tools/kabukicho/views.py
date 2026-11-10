"""Renders a game / scene bundle at the UE ground-truth cameras (UE location cm + rotator), for same-camera comparisons
with ue5/capture.ts renders.  usage: views.py <bundle.js> <out-dir> <vfov> [time] [--spp N] [--assets DIR]
Writes <out-dir>/v1.png .. v6.png (threert-render render --camera)."""
import json, math, os, subprocess, sys

VIEWS = {"v1": ((-1150, -180, 170), (4, 4, 0)), "v2": ((700, 160, 170), (6, -2, 0)), "v3": ((6900, -250, 170), (8, 8, 0)),
         "v4": ((3600, 0, 2600), (-24, 0, 0)), "v5": ((5200, 180, 170), (6, 178, 0)), "v6": ((8550, -900, 300), (4, 90, 0))}
ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

def camera(loc, rot, fov):
    """UE location (cm, Z up) + rotator (pitch, yaw, roll in degrees) -> our eye position + look-at target (m, Y up)."""
    p, y = math.radians(rot[0]), math.radians(rot[1])
    fwd = (math.cos(p) * math.cos(y), math.cos(p) * math.sin(y), math.sin(p))  # UE forward (X axis)
    pos = (loc[0] / 100, loc[2] / 100, loc[1] / 100)
    return {"position": pos, "target": [pos[0] + fwd[0], pos[1] + fwd[2], pos[2] + fwd[1]], "fov": fov}

args = sys.argv[1:]
flag = lambda k, d: args[args.index(k) + 1] if k in args else d
pos = [a for i, a in enumerate(args) if not a.startswith("--") and (i == 0 or not args[i - 1].startswith("--"))]
bundle, out, fov = pos[0], pos[1], float(pos[2])
time = pos[3] if len(pos) > 3 else None
os.makedirs(out, exist_ok=True)
for name, (loc, rot) in VIEWS.items():
    cmd = [os.path.join(ROOT, "renderer/target/release/threert-render"), "render", bundle, "-o", os.path.join(out, f"{name}.png"),
           "--assets", flag("--assets", ROOT), "--camera", json.dumps(camera(loc, rot, fov))]
    if time is not None:
        cmd += ["--time", time]
    if "--spp" in args:
        cmd += ["--spp", flag("--spp", "")]
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)
print("ok", list(VIEWS))
