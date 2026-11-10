# A lower level of detail of Quaternius' mannequin (UAL1_Standard.glb), skin and clips kept, run headless by
# tools/fetch-quaternius.ts:
#
#   blender -b --factory-startup --python-exit-code 1 --python tools/quaternius/lod.py -- IN.glb OUT.glb RATIO
#
# Every skinned character is re-skinned and rebuilt into the dynamic BLAS each frame (wgpu has no BLAS refit), so its
# cost goes with its triangles: the Bistro C slice's four walkers at 13,744 triangles each were 1.44 ms of the 2.44 ms
# rebuild (ROADMAP item 3 part 2). Blender's collapse decimation keeps the vertex groups (skin weights interpolated) and
# the UVs; the clips are written back one glTF animation per action, under their own names.
import sys

import bpy

src, dst, ratio = (sys.argv[sys.argv.index("--") + 1:] + [""] * 3)[:3]
if not src or not dst or not ratio:
    raise SystemExit("usage: ... -- IN.glb OUT.glb RATIO")

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)


def triangles(ob):
    return sum(len(p.vertices) - 2 for p in ob.data.polygons)


before = after = 0
for ob in [o for o in bpy.data.objects if o.type == "MESH"]:
    before += triangles(ob)
    m = ob.modifiers.new("lod", "DECIMATE")
    m.decimate_type = "COLLAPSE"
    m.ratio = float(ratio)
    m.use_collapse_triangulate = True
    with bpy.context.temp_override(object=ob, active_object=ob, selected_objects=[ob]):
        # Before the armature modifier: decimate the rest pose, keep the skinning live.
        bpy.ops.object.modifier_move_to_index(modifier=m.name, index=0)
        bpy.ops.object.modifier_apply(modifier=m.name)
    after += triangles(ob)

bpy.ops.export_scene.gltf(filepath=dst, export_format="GLB", export_animation_mode="ACTIONS", export_skins=True, export_apply=False)
print(f"lod: {before} -> {after} triangles ({len(bpy.data.actions)} clips)", flush=True)
