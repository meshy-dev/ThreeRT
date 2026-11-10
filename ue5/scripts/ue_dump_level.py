"""Dump an Unreal level to JSON (any project): actors with transforms, static meshes + materials, material instance
parameters, lights, fog, sky light, post-process overrides and markers. Used to port UE scenes to the ThreeRT IR and to
check a port actor by actor.

    THREERT_DUMP='{"map": "/Game/Kabukicho/Maps/L_Kabukicho", "out": "/abs/path/level.json"}' \
      UnrealEditor <project>.uproject -ExecCmds="py ue5/scripts/ue_dump_level.py" -unattended -nosplash -NoSound
"""
import json
import os
import traceback

import unreal

EAL = unreal.EditorAssetLibrary
EAS = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
LES = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)


def log(msg):
    unreal.log("THREERT: " + msg)


def vec(v):
    return [round(v.x, 4), round(v.y, 4), round(v.z, 4)]


def rot(r):
    return [round(r.pitch, 4), round(r.yaw, 4), round(r.roll, 4)]


def color(c):
    if isinstance(c, unreal.LinearColor):
        return [c.r, c.g, c.b, c.a]
    return [c.r, c.g, c.b, c.a]  # unreal.Color (sRGB bytes)


def prop(obj, name, default=None):
    try:
        v = obj.get_editor_property(name)
    except Exception:
        return default
    if isinstance(v, (unreal.Vector,)):
        return vec(v)
    if isinstance(v, (unreal.LinearColor, unreal.Color)):
        return color(v)
    if isinstance(v, unreal.Rotator):
        return rot(v)
    if isinstance(v, (bool, int, float, str)):
        return v
    if isinstance(v, unreal.EnumBase) or hasattr(v, "name"):
        return str(v)
    return str(v)


materials = {}


def material_info(m):
    if m is None:
        return None
    path = m.get_path_name()
    if path in materials:
        return path
    info = {"class": m.get_class().get_name()}
    if isinstance(m, unreal.MaterialInstance):
        parent = m.get_editor_property("parent")
        info["parent"] = parent.get_path_name() if parent else None
        info["scalars"] = {str(p.parameter_info.name): p.parameter_value for p in m.get_editor_property("scalar_parameter_values")}
        info["vectors"] = {str(p.parameter_info.name): color(p.parameter_value) for p in m.get_editor_property("vector_parameter_values")}
        info["textures"] = {str(p.parameter_info.name): (p.parameter_value.get_path_name() if p.parameter_value else None)
                            for p in m.get_editor_property("texture_parameter_values")}
        if parent:
            material_info(parent)
    else:
        info["blend_mode"] = prop(m, "blend_mode")
        info["two_sided"] = prop(m, "two_sided")
        info["shading_model"] = prop(m, "shading_model")
    materials[path] = info
    return path


meshes = {}


def mesh_info(sm):
    path = sm.get_path_name()
    if path not in meshes:
        bb = sm.get_bounding_box()
        meshes[path] = {"min": vec(bb.min), "max": vec(bb.max), "materials": [material_info(s.material_interface) for s in sm.get_editor_property("static_materials")]}
    return path


LIGHT_PROPS = ["intensity", "intensity_units", "light_color", "attenuation_radius", "cast_shadows", "volumetric_scattering_intensity",
               "specular_scale", "source_radius", "soft_source_radius", "source_length", "inner_cone_angle", "outer_cone_angle",
               "source_width", "source_height", "barn_door_angle", "barn_door_length", "use_inverse_squared_falloff",
               "light_falloff_exponent", "indirect_lighting_intensity", "use_temperature", "temperature", "affects_world"]
FOG_PROPS = ["fog_density", "fog_height_falloff", "fog_inscattering_luminance", "directional_inscattering_luminance",
             "directional_inscattering_exponent", "directional_inscattering_start_distance", "fog_max_opacity", "start_distance",
             "fog_cutoff_distance", "enable_volumetric_fog", "volumetric_fog_scattering_distribution", "volumetric_fog_albedo",
             "volumetric_fog_emissive", "volumetric_fog_extinction_scale", "volumetric_fog_distance",
             "volumetric_fog_start_distance", "volumetric_fog_near_fade_in_distance", "second_fog_data"]


def component_props(c, names):
    return {n: prop(c, n) for n in names if prop(c, n) is not None}


def pp_overrides(settings):
    out = {}
    for name in dir(settings):
        if not name.startswith("override_"):
            continue
        try:
            if settings.get_editor_property(name):
                key = name[len("override_"):]
                out[key] = prop(settings, key)
        except Exception:
            pass
    return out


def dump(job):
    LES.load_level(job["map"])
    actors = []
    for a in EAS.get_all_level_actors():
        t = a.get_actor_transform()
        e = {"class": a.get_class().get_name(), "label": a.get_actor_label(), "folder": str(a.get_folder_path()),
             "tags": [str(x) for x in a.tags], "location": vec(t.translation), "rotation": rot(t.rotation.rotator()),
             "scale": vec(t.scale3d), "hidden": a.is_hidden_ed()}
        smc = a.get_component_by_class(unreal.StaticMeshComponent)
        if smc and smc.static_mesh:
            e["mesh"] = mesh_info(smc.static_mesh)
            e["materials"] = [material_info(smc.get_material(i)) for i in range(smc.get_num_materials())]
            e["cast_shadow"] = smc.get_editor_property("cast_shadow")
        for cls, kind in ((unreal.RectLightComponent, "rect"), (unreal.SpotLightComponent, "spot"), (unreal.PointLightComponent, "point"),
                          (unreal.DirectionalLightComponent, "directional")):
            c = a.get_component_by_class(cls)
            if c:
                e["light"] = dict(kind=kind, **component_props(c, LIGHT_PROPS))
                break
        c = a.get_component_by_class(unreal.SkyLightComponent)
        if c:
            e["skylight"] = component_props(c, ["intensity", "real_time_capture", "lower_hemisphere_is_black", "lower_hemisphere_color",
                                                "source_type", "indirect_lighting_intensity", "volumetric_scattering_intensity"])
        c = a.get_component_by_class(unreal.ExponentialHeightFogComponent)
        if c:
            e["fog"] = component_props(c, FOG_PROPS)
        if isinstance(a, unreal.PostProcessVolume):
            e["post"] = pp_overrides(a.get_editor_property("settings"))
            e["unbound"] = a.get_editor_property("unbound")
        actors.append(e)
    out = {"map": job["map"], "actors": actors, "meshes": meshes, "materials": materials}
    os.makedirs(os.path.dirname(job["out"]), exist_ok=True)
    with open(job["out"], "w") as fh:
        json.dump(out, fh, indent=0)
    log(f"dumped {len(actors)} actors, {len(meshes)} meshes, {len(materials)} materials -> {job['out']}")


try:
    dump(json.loads(os.environ["THREERT_DUMP"]))
    log("all done")
except Exception:
    log("FAILED\n" + traceback.format_exc())
unreal.SystemLibrary.quit_editor()
