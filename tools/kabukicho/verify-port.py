"""Check the DSL port of the UE Kabukichō level actor by actor against a dump of the UE level (ue5/scripts/ue_dump_level.py).
usage: node tools/kabukicho/dump-street.ts > out/kb-street.json
       python3 tools/kabukicho/verify-port.py out/ue-kb/level.json out/kb-street.json"""
import json, math, sys
from collections import defaultdict

ue, street = json.load(open(sys.argv[1])), json.load(open(sys.argv[2]))
skip = ("KB/Gate", "KB/Environment", "KB/Markers")
def kind_ue(a):
    if "light" in a: return "light:" + a["light"]["kind"]
    if "mesh" in a:
        m = a["mesh"].split("/")[-1].split(".")[0]
        return "shape:" + m if m in ("Cube", "Plane", "Cylinder", "Sphere", "Cone") else "model"
    return None
def kind_ours(n):
    if n["kind"].startswith("geometry:"):
        g = n["kind"].split(":")[1]
        return "shape:" + {"BoxGeometry": "Cube", "MeshGeometry": "Plane", "PlaneGeometry": "Plane", "CylinderGeometry": "Cylinder", "SphereGeometry": "Sphere", "ConeGeometry": "Cone"}[g]
    return n["kind"]
ues = defaultdict(list)
for a in ue["actors"]:
    k = kind_ue(a)
    if k and not a["folder"].startswith(skip):
        x, y, z = a["location"]
        ues[k].append((x / 100, z / 100, y / 100, a))
ours = defaultdict(list)
for n in street["nodes"]:
    k = kind_ours(n)
    if k and not n["id"].startswith("gate") and n["id"] != "moon":
        ours[k].append((*n["position"], n))
worst = 0
for k in sorted(set(ues) | set(ours)):
    a, b = ues[k], ours[k]
    used = set(); dmax = 0; smax = 0
    grid = defaultdict(list)
    for i, p in enumerate(b): grid[(round(p[0]), round(p[1]), round(p[2]))].append(i)
    for p in a:
        best, bi = 1e9, None
        c = (round(p[0]), round(p[1]), round(p[2]))
        for dx in (-1, 0, 1):
            for dy in (-1, 0, 1):
                for dz in (-1, 0, 1):
                    for i in grid.get((c[0] + dx, c[1] + dy, c[2] + dz), []):
                        if i in used: continue
                        d = math.dist(p[:3], b[i][:3])
                        if d < best: best, bi = d, i
        if bi is not None:
            used.add(bi)
            ua, n = p[3], b[bi][3]
            us = ua["scale"]
            smax = max(smax, max(abs(us[0] - n["scale"][0]), abs(us[2] - n["scale"][1]), abs(us[1] - n["scale"][2])))
        dmax = max(dmax, best)
    worst = max(worst, dmax)
    print(f"{k:16s} ue={len(a):5d} ours={len(b):5d} matched={len(used):5d} max pos err={dmax:.4f} m  max scale err={smax:.4f}")
print("OK" if worst < 0.01 else "MISMATCH", f"(worst {worst:.4f} m)")
