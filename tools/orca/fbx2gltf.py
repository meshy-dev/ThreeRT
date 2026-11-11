# ORCA (Falcor) FBX -> glTF 2.0 (separate .gltf + .bin + textures/), run headless by scripts/orca.ts:
#
#   blender -b --factory-startup --python-exit-code 1 --python tools/orca/fbx2gltf.py -- \
#     --fbx IN.fbx --out DIR --name NAME --python VENV_PYTHON [--max 2048] [--no-animations]
#
# Blender's FBX importer wires each material as Falcor's FBX connections say (DiffuseColor, SpecularColor, NormalMap,
# EmissiveColor) onto a Principled BSDF; this script turns that into what glTF means:
#  - textures converted by tools/orca/textures.py (PNG / JPEG, <= --max, normals to the OpenGL convention), relinked;
#  - Specular (Falcor: R occlusion, unused; G roughness; B metalness) -> Separate Color -> Roughness / Metallic, the
#    glTF metallicRoughnessTexture as is (no repacking);
#  - BaseColor alpha: alpha-tested in Falcor -> a Round node (glTF MASK, cutoff 0.5) where the texture has any, else
#    opaque;
#  - emission: the Emissive texture x 1 (Falcor's emissiveFactor; the scene code scales it to nits);
#  - double-sided only for materials named *.DoubleSided (Falcor's convention), else back faces culled;
#  - texture transforms (the Mapping nodes the importer makes) -> KHR_texture_transform, by the exporter.
# The analytic sun of the FBX is dropped (night: emissive surfaces only); the camera and its path are kept. The FBX
# take becomes one glTF animation named NAME (the scene timeline: Bistro's wind and camera path, Zero Day's motion).
# Writes DIR/NAME.gltf, DIR/NAME.bin, DIR/textures/*, DIR/NAME.report.json.
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time

import bpy

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
ap = argparse.ArgumentParser()
ap.add_argument("--fbx", required=True)
ap.add_argument("--out", required=True)
ap.add_argument("--name", required=True)
ap.add_argument("--python", required=True)
ap.add_argument("--max", type=int, default=2048)
ap.add_argument("--no-animations", action="store_true")
args = ap.parse_args(argv)

t0 = time.time()
here = os.path.dirname(os.path.abspath(__file__))
out = os.path.abspath(args.out)
texdir = os.path.join(out, "textures")
os.makedirs(texdir, exist_ok=True)

bpy.ops.wm.read_factory_settings(use_empty=True)
# Blender 5.1's FBX importer sets lamp.cycles.cast_shadow when Cycles is loaded, and Cycles' light settings no longer
# have it: the import of any FBX with a light stops there. Nothing here renders with Cycles, so it is unloaded first
# (the importer checks for lamp.cycles); the lights themselves are removed below.
import addon_utils
if addon_utils.check("cycles")[1]:
    addon_utils.disable("cycles")
bpy.ops.import_scene.fbx(filepath=os.path.abspath(args.fbx), use_image_search=False, use_anim=not args.no_animations)
print(f"orca: imported {args.fbx} in {time.time() - t0:.1f} s", flush=True)

for o in [o for o in bpy.data.objects if o.type == "LIGHT"]:
    bpy.data.objects.remove(o, do_unlink=True)

# Meshes without texture coordinates sample their textures at (0, 0) in Falcor (Zero Day has a few): a zero UV layer
# says the same in glTF (the exporter would write an invalid texCoord otherwise).
no_uv = 0
for me in {o.data for o in bpy.data.objects if o.type == "MESH"}:
    if not me.uv_layers:
        me.uv_layers.new(name="UVMap")
        no_uv += 1


def bsdf_of(mat):
    for n in mat.node_tree.nodes:
        if n.type == "BSDF_PRINCIPLED":
            return n
    return None


def role_of(link):
    to = link.to_node
    if to.type == "BSDF_PRINCIPLED":
        return {"Base Color": "color", "Alpha": "alpha", "Specular IOR Level": "orm", "Emission Color": "emissive"}.get(link.to_socket.name)
    if to.type == "NORMAL_MAP" and link.to_socket.name == "Color":
        return "normal"
    return None


# (material, image node, role) for every texture the importer wired.
uses = []
missing = set()
for mat in bpy.data.materials:
    if not mat.node_tree:
        continue
    for link in list(mat.node_tree.links):
        n = link.from_node
        if n.type != "TEX_IMAGE" or not n.image:
            continue
        role = role_of(link)
        if role is None:
            continue
        path = bpy.path.abspath(n.image.filepath, library=n.image.library)
        if not os.path.isfile(path):
            missing.add(path)
            continue
        uses.append((mat, n, link, role, os.path.normpath(path)))

# One converted file per (source, role); names sanitised (glTF URIs), unique.
jobs, dst_of, taken = [], {}, set()
for _, _, _, role, path in uses:
    r = "color" if role == "alpha" else role
    if (path, r) in dst_of:
        continue
    stem = re.sub(r"[^A-Za-z0-9._-]+", "_", os.path.splitext(os.path.basename(path))[0]).strip("_")
    name, i = stem, 1
    while name.lower() in taken:
        i += 1
        name = f"{stem}_{i}"
    taken.add(name.lower())
    dst_of[(path, r)] = os.path.join(texdir, name)
    jobs.append({"src": path, "dst": dst_of[(path, r)], "role": r, "max": args.max})

with tempfile.TemporaryDirectory() as tmp:
    jf, rf = os.path.join(tmp, "jobs.json"), os.path.join(tmp, "results.json")
    json.dump(jobs, open(jf, "w"))
    t = time.time()
    subprocess.run([args.python, "-I", os.path.join(here, "textures.py"), jf, rf], check=True)
    results = json.load(open(rf))
    print(f"orca: {len(jobs)} textures in {time.time() - t:.1f} s", flush=True)

by_dst = results
for j in jobs:
    if j["dst"] not in by_dst:
        raise SystemExit(f"orca: {j['src']} ({j['role']}) was not converted")

images = {}


def image_for(path, role):
    dst = dst_of[(path, "color" if role == "alpha" else role)]
    r = by_dst[dst]
    img = images.get(dst)
    if img is None:
        img = bpy.data.images.load(r["file"], check_existing=True)
        img.colorspace_settings.name = "sRGB" if role in ("color", "alpha", "emissive") else "Non-Color"
        img.alpha_mode = "STRAIGHT"
        images[dst] = img
    return img, r


stats = {"materials": 0, "mask": [], "emissive": [], "double_sided": 0}
for mat in bpy.data.materials:
    if not mat.node_tree:
        continue
    stats["materials"] += 1
    nt = mat.node_tree
    bsdf = bsdf_of(mat)
    mine = [u for u in uses if u[0] == mat]
    for _, node, link, role, path in mine:
        img, r = image_for(path, role)
        node.image = img
        if role == "orm":
            sep = nt.nodes.new("ShaderNodeSeparateColor")
            sep.location = (node.location.x + 300, node.location.y)
            nt.links.remove(link)
            nt.links.new(node.outputs["Color"], sep.inputs["Color"])
            nt.links.new(sep.outputs["Green"], bsdf.inputs["Roughness"])
            nt.links.new(sep.outputs["Blue"], bsdf.inputs["Metallic"])
            bsdf.inputs["Specular IOR Level"].default_value = 0.5
        elif role == "alpha":
            nt.links.remove(link)
            if r["alpha"] == "mask":
                rnd = nt.nodes.new("ShaderNodeMath")
                rnd.operation = "ROUND"
                rnd.location = (node.location.x + 300, node.location.y - 200)
                nt.links.new(node.outputs["Alpha"], rnd.inputs[0])
                nt.links.new(rnd.outputs[0], bsdf.inputs["Alpha"])
                stats["mask"].append(mat.name)
            else:
                bsdf.inputs["Alpha"].default_value = 1.0
        elif role == "emissive":
            bsdf.inputs["Emission Strength"].default_value = 1.0
            stats["emissive"].append(mat.name)
    if bsdf and not bsdf.inputs["Alpha"].is_linked:
        bsdf.inputs["Alpha"].default_value = 1.0
    if bsdf and not any(u[3] == "orm" for u in mine):
        # no Specular map: Falcor's defaults for the metal-rough model (rough dielectric)
        bsdf.inputs["Metallic"].default_value = 0.0
        bsdf.inputs["Roughness"].default_value = 1.0
        bsdf.inputs["Specular IOR Level"].default_value = 0.5
    ds = mat.name.endswith(".DoubleSided")
    mat.use_backface_culling = not ds
    stats["double_sided"] += ds

# One animation clip named after the output (the FBX take on the scene timeline), not one per animated object. The
# importer leaves the timeline at Blender's default (1 .. 250): it spans every action, or the export cuts them short.
sc = bpy.context.scene
sc.name = args.name
ranges = [a.frame_range for a in bpy.data.actions]
if ranges:
    sc.frame_start = int(min(r[0] for r in ranges))
    sc.frame_end = int(max(r[1] for r in ranges) + 0.999)
gltf = os.path.join(out, f"{args.name}.gltf")
t = time.time()
bpy.ops.export_scene.gltf(
    filepath=gltf,
    export_format="GLTF_SEPARATE",
    export_keep_originals=True,
    export_image_format="AUTO",
    export_tangents=True,
    export_lights=False,
    export_cameras=True,
    export_animations=not args.no_animations,
    export_animation_mode="SCENE",
    export_anim_scene_split_object=False,
    export_yup=True,
    export_extras=False,
    export_apply=False,
)
print(f"orca: exported {gltf} in {time.time() - t:.1f} s", flush=True)

doc = json.load(open(gltf))
bad = [m["name"] for m in doc.get("materials", []) if re.search(r'"texCoord": -', json.dumps(m))]
if bad:
    raise SystemExit(f"orca: invalid texCoord in {bad[:5]}")

tris = sum(sum(len(p.vertices) - 2 for p in o.data.polygons) for o in bpy.data.objects if o.type == "MESH")
report = {
    "source": os.path.basename(args.fbx),
    "objects": len([o for o in bpy.data.objects if o.type == "MESH"]),
    "triangles": tris,
    "materials": stats["materials"],
    "textures": len(jobs),
    "texture_bytes": sum(os.path.getsize(r["file"]) for r in by_dst.values()),
    "alpha_mask": sorted(set(stats["mask"])),
    "emissive": sorted(set(stats["emissive"])),
    "double_sided": stats["double_sided"],
    "missing_textures": sorted(missing),
    "meshes_without_uv": no_uv,
    "animations": len(doc.get("animations", [])),
    "animated_objects": len([o for o in bpy.data.objects if o.animation_data and o.animation_data.action]) if not args.no_animations else 0,
    "frames": [bpy.context.scene.frame_start, bpy.context.scene.frame_end, bpy.context.scene.render.fps],
    "cameras": [o.name for o in bpy.data.objects if o.type == "CAMERA"],
    "seconds": round(time.time() - t0, 1),
}
json.dump(report, open(os.path.join(out, f"{args.name}.report.json"), "w"), indent=1)
print("orca:", json.dumps({k: v for k, v in report.items() if k not in ("alpha_mask", "missing_textures")}), flush=True)
