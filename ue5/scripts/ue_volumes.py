"""UE 5.8 reference for the volume presets (ROADMAP § 5 item 6.3, part 3): tools/volume-scene.ts rebuilt in a scratch
project, a still from the Path Tracer and one from Lumen (MRQ, linear EXR + PNG).

    THREERT_JOB=/abs/job.json UnrealEditor <scratch>.uproject -ExecCmds="py ue5/scripts/ue_volumes.py" -unattended ...
job.json: {"out": "/abs/out/ue-volumes", "methods": ["pathtracer", "lumen"], "width": 1280, "height": 720,
           "spp": 1024, "ev100": 3}
Driven by ue5/volumes.ts.

Scene (mirror of tools/volume-scene.ts; keep in sync): a 30 x 30 m floor and a 14 x 7 m wall at z = -3 (linear albedo
0.35, roughness 0.8), no sky; a 60 cd lamp (0xffb070) in a fog sphere on the left, a 25-lux moon (0xb0c4ff) from
(4, 8, 6), a smoke column in the middle and a fireball 0.7 s into its life on the right; camera at (0, 1.6, 7) looking
at (0, 1.4, 0), 50-degree vertical field of view, manual exposure EV100 3. Coordinates: ours are Y-up metres,
UE = 100 * (x, z, y) cm.

The fog is a Heterogeneous Volume with a Local Fog Volume's density (VolumetricFog.usf's voxelization, as
packages/scene FogVolume): the path tracer does not render Local Fog Volumes, and the Local Fog Volume through Lumen's
volumetric fog hung the editor on this Mac ("lfv": true tries it). The smoke column and the
fireball are Heterogeneous Volumes in both: Volume-domain materials whose Custom nodes are our volume functions in
HLSL. A Heterogeneous Volume's extinction and emission are per local cm (before the actor's scale), measured: world
1/m = value x 100 / scale.
"""
import json
import math
import os
import time
import traceback

import unreal

EAL = unreal.EditorAssetLibrary
MEL = unreal.MaterialEditingLibrary
AT = unreal.AssetToolsHelpers.get_asset_tools()
EAS = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
LES = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
ROOT = "/Game/ThreeRT"
JOB = json.load(open(os.environ["THREERT_JOB"]))
MP = unreal.MaterialProperty

# tools/volume-scene.ts
GREY = 0.3515  # sRGB 0xa0
FOG = {"pos": [-3.2, 1.1, 0.0], "diameter": 2.4, "radial": 2.0, "height": 1.0, "falloff": 1000.0, "offset": 0.5, "albedo": 0.9, "g": 0.2}
LAMP = {"pos": [-3.2, 1.4, 0.0], "cd": 60.0, "color": 0xFFB070, "radius": 0.05}
MOON = {"from": [4.0, 8.0, 6.0], "lux": 25.0, "color": 0xB0C4FF}
SMOKE = {"pos": [0.0, 2.2, -0.5], "size": [1.6, 4.4, 1.6], "density": 4.0, "scale": 1.5, "rise": 0.6, "albedo": [0.55, 0.55, 0.58], "g": 0.2}
FIRE = {"pos": [3.2, 1.2, 0.0], "diameter": 2.0, "life": 1.6, "period": 3.0, "intensity": 40.0, "density": 6.0, "scale": 3.0, "start": -0.7}
TIME = 0.0
VOXEL_CM = 2.5  # Heterogeneous Volume voxel size (the actor's scale on a 1 cm grid)


def log(msg):
    unreal.log("THREERT: " + msg)


def ue(v, s=100.0):
    """ours (x, y, z) m -> UE (x, z, y) cm."""
    return unreal.Vector(v[0] * s, v[2] * s, v[1] * s)


def srgb(hex_color):
    def lin(c):
        c /= 255.0
        return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4
    return unreal.LinearColor(lin((hex_color >> 16) & 255), lin((hex_color >> 8) & 255), lin(hex_color & 255), 1.0)


def spawn(cls, loc=None, rot=None, label=None):
    a = EAS.spawn_actor_from_class(cls, loc or unreal.Vector(0, 0, 0), rot or unreal.Rotator(0, 0, 0))
    if label:
        a.set_actor_label(label)
    return a


def fresh(name, cls, factory):
    path = f"{ROOT}/{name}"
    if EAL.does_asset_exist(path):
        EAL.delete_asset(path)
    return AT.create_asset(name, ROOT, cls, factory)


def grey_material():
    mat = fresh("M_ThreeRT_Grey", unreal.Material, unreal.MaterialFactoryNew())
    for i, (prop, value) in enumerate(((MP.MP_BASE_COLOR, (GREY, GREY, GREY)), (MP.MP_ROUGHNESS, 0.8))):
        if isinstance(value, tuple):
            e = MEL.create_material_expression(mat, unreal.MaterialExpressionConstant3Vector, -400, 200 * i)
            e.set_editor_property("constant", unreal.LinearColor(value[0], value[1], value[2], 1.0))
        else:
            e = MEL.create_material_expression(mat, unreal.MaterialExpressionConstant, -400, 200 * i)
            e.set_editor_property("r", value)
        MEL.connect_material_property(e, "", prop)
    MEL.recompile_material(mat)
    EAL.save_loaded_asset(mat)
    return mat


# Our volume functions in HLSL (packages/scene/src/effects.ts NOISE, SMOKE, FIRE, LOCAL_FOG; keep in sync). `P` is the
# point in our coordinates (m), `LP` in the volume's unit box / sphere (half extent 0.5).
NOISE = """
struct VN {
    float hash(float3 p) {
        float3 q = frac(p * float3(0.1031, 0.1030, 0.0973));
        float3 r = q + dot(q, q.yxz + 33.33);
        return frac((r.x + r.y) * r.z);
    }
    float noise(float3 p) {
        float3 i = floor(p);
        float3 f = frac(p);
        float3 u = f * f * (3.0 - 2.0 * f);
        float a = lerp(lerp(hash(i), hash(i + float3(1, 0, 0)), u.x), lerp(hash(i + float3(0, 1, 0)), hash(i + float3(1, 1, 0)), u.x), u.y);
        float b = lerp(lerp(hash(i + float3(0, 0, 1)), hash(i + float3(1, 0, 1)), u.x), lerp(hash(i + float3(0, 1, 1)), hash(i + float3(1, 1, 1)), u.x), u.y);
        return lerp(a, b, u.z);
    }
    float fbm(float3 p) {
        float s = 0.0;
        float a = 0.5;
        float3 q = p;
        for (int k = 0; k < 3; k++) {
            s += a * noise(q);
            q = q * 2.03 + float3(1.7, 9.2, 4.1);
            a *= 0.5;
        }
        return s;
    }
};
VN vn;
float3 P = float3(WP.x, WP.z, WP.y) * 0.01;
float3 LP = (P - Center) / Size;
"""

BODIES = {
    "Fog": """
float3 U = LP * 2.0;
float fade = saturate(1.0 - length(U));
float ext = 0.0;
if (fade > 0.0) {
    float height = Height > 0.0 ? Height * exp(min(-Falloff * (U.y - Offset), 80.0)) : 0.0;
    float a = exp(-height);
    float b = exp(-Radial * pow(fade, 0.82));
    ext = max(-log(max(a - a * b + b, 1e-30)), 0.0);
}
float3 alb = Albedo;
float3 emi = 0;
""",
    "Smoke": """
float r = length(LP.xz) * 2.0;
float h = LP.y + 0.5;
float n = vn.fbm(P * NScale - float3(0.0, Time * Rise * NScale, 0.0));
float shape = max(1.0 - r * r * (1.0 + h), 0.0) * (1.0 - smoothstep(0.6, 1.0, h));
float ext = Density * shape * smoothstep(0.35, 0.75, n);
float3 alb = Albedo;
float3 emi = 0;
""",
    "Fire": """
float age = Time - Start;
if (Period > 0.0 && age >= 0.0) { age = age - floor(age / Period) * Period; }
float t = saturate(age / Life);
float grow = sqrt(t);
float r = length(LP) * 2.0 / max(grow, 1e-3);
float n = vn.fbm(P * NScale + float3(0.0, -age * 1.5, 0.0));
float body = max(1.0 - r, 0.0) * smoothstep(0.25, 0.7, n + 0.3 * (1.0 - r));
float alive = (age >= 0.0 && age < Life) ? 1.0 : 0.0;
float ext = Density * body * alive;
float temp = saturate((1.0 - r) * 1.6 - t * 1.2);
float3 c = lerp(float3(0.6, 0.06, 0.01), float3(1.0, 0.45, 0.08), smoothstep(0.1, 0.5, temp));
c = lerp(c, float3(1.0, 0.9, 0.6), smoothstep(0.5, 0.95, temp));
float3 alb = 0.08;
float3 emi = c * Intensity * temp * temp * ext;
""",
}
# Our extinction / emission are per metre; a Heterogeneous Volume's per local cm: x VOXEL_CM / 100.
OUTPUTS = {MP.MP_SUBSURFACE_COLOR: "return ext * PerCm;", MP.MP_BASE_COLOR: "return alb;", MP.MP_EMISSIVE_COLOR: "return emi * PerCm;"}


def volume_material(kind, scalars, vectors):
    """A Volume-domain material for Heterogeneous Volumes: three Custom nodes (extinction, albedo, emission)."""
    mat = fresh(f"M_ThreeRT_{kind}", unreal.Material, unreal.MaterialFactoryNew())
    # Additive first: the domain change compiles, and volume materials must be additive.
    mat.set_editor_property("blend_mode", unreal.BlendMode.BLEND_ADDITIVE)
    mat.set_editor_property("material_domain", unreal.MaterialDomain.MD_VOLUME)
    mat.set_editor_property("used_with_heterogeneous_volumes", True)
    wp = MEL.create_material_expression(mat, unreal.MaterialExpressionWorldPosition, -1400, 0)
    params = {}
    for i, (name, v) in enumerate(list(scalars.items()) + [("PerCm", VOXEL_CM / 100.0)]):
        p = MEL.create_material_expression(mat, unreal.MaterialExpressionScalarParameter, -1400, 120 * (i + 1))
        p.set_editor_property("parameter_name", name)
        p.set_editor_property("default_value", float(v))
        params[name] = p
    for i, (name, v) in enumerate(vectors.items()):
        p = MEL.create_material_expression(mat, unreal.MaterialExpressionVectorParameter, -1700, 160 * i)
        p.set_editor_property("parameter_name", name)
        p.set_editor_property("default_value", unreal.LinearColor(v[0], v[1], v[2], 0.0))
        params[name] = p
    for k, (prop, ret) in enumerate(OUTPUTS.items()):
        c = MEL.create_material_expression(mat, unreal.MaterialExpressionCustom, -600, 300 * k)
        c.set_editor_property("code", NOISE + BODIES[kind] + ret)
        c.set_editor_property("output_type", unreal.CustomMaterialOutputType.CMOT_FLOAT3)
        names = ["WP"] + list(params)
        inputs = []
        for n in names:
            ci = unreal.CustomInput()
            ci.set_editor_property("input_name", n)
            inputs.append(ci)
        c.set_editor_property("inputs", inputs)
        MEL.connect_material_expressions(wp, "", c, "WP")
        for n, p in params.items():
            MEL.connect_material_expressions(p, "", c, n)
        MEL.connect_material_property(c, "", prop)
    MEL.recompile_material(mat)
    EAL.save_loaded_asset(mat)
    return mat


def heterogeneous_volume(label, mat, center, size):
    """A Heterogeneous Volume over `size` (ours, m) at `center`: a 1 cm grid scaled by VOXEL_CM."""
    hv = spawn(unreal.HeterogeneousVolume, ue(center), None, label)
    hc = hv.get_editor_property("heterogeneous_volume_component")
    ext = ue(size)  # UE axes, cm
    hc.set_editor_property("volume_resolution", unreal.IntVector(*(max(1, int(round(abs(c) / VOXEL_CM))) for c in (ext.x, ext.y, ext.z))))
    hc.set_editor_property("pivot_at_centroid", True)
    hv.set_actor_scale3d(unreal.Vector(VOXEL_CM, VOXEL_CM, VOXEL_CM))
    hc.set_material(0, mat)
    return hv


def build_level(method):
    map_path = f"{ROOT}/L_Volumes_{method}"
    if EAL.does_asset_exist(map_path):
        EAL.delete_asset(map_path)
    if not LES.new_level(map_path):
        raise RuntimeError(f"new_level {map_path} failed")
    grey = grey_material()
    floor = spawn(unreal.StaticMeshActor, None, None, "Floor")
    floor.static_mesh_component.set_static_mesh(EAL.load_asset("/Engine/BasicShapes/Plane"))
    floor.set_actor_scale3d(unreal.Vector(30.0, 30.0, 1.0))
    floor.static_mesh_component.set_material(0, grey)
    wall = spawn(unreal.StaticMeshActor, ue([0, 3.5, -3]), unreal.Rotator(roll=90.0, pitch=0.0, yaw=0.0), "Wall")
    wall.static_mesh_component.set_static_mesh(EAL.load_asset("/Engine/BasicShapes/Plane"))
    wall.set_actor_scale3d(unreal.Vector(14.0, 7.0, 1.0))
    wall.static_mesh_component.set_material(0, grey)
    lamp = spawn(unreal.PointLight, ue(LAMP["pos"]), None, "Lamp")
    lc = lamp.point_light_component
    lc.set_editor_property("intensity_units", unreal.LightUnits.CANDELAS)
    lc.set_editor_property("intensity", LAMP["cd"])
    lc.set_light_color(srgb(LAMP["color"]), True)
    lc.set_editor_property("source_radius", LAMP["radius"] * 100.0)
    lc.set_editor_property("attenuation_radius", 4000.0)
    lc.set_editor_property("use_inverse_squared_falloff", True)
    # Point and spot lights do not shadow through volumes by default (directional and sky lights do): without it the
    # path tracer lit the media as if they let all of the lamp's light through (a lamp inside a sphere of medium read
    # 11.3 against the analytic 4.23 for single scattering; 4.07 with it).
    lc.set_editor_property("cast_volumetric_shadow", True)
    rot = unreal.MathLibrary.find_look_at_rotation(ue(MOON["from"]), unreal.Vector(0, 0, 0))
    moon = spawn(unreal.DirectionalLight, ue(MOON["from"]), rot, "Moon")
    mc = moon.light_component
    mc.set_editor_property("intensity", MOON["lux"])
    mc.set_light_color(srgb(MOON["color"]), True)
    mc.set_editor_property("light_source_angle", 0.5357)
    # Fog: a Heterogeneous Volume with the Local Fog Volume's density in both stills. The Local Fog Volume itself
    # ("lfv": true, Lumen only) renders through volumetric fog, which hung UE 5.8 on Metal (M5) at the first frame
    # after warm-up; without volumetric fog it did not show at all.
    if JOB.get("noFog"):
        pass
    elif method == "lumen" and JOB.get("lfv"):
        lfv = spawn(unreal.LocalFogVolume, ue(FOG["pos"]), None, "FogVolume")
        lfv.set_actor_scale3d(unreal.Vector(1, 1, 1) * (FOG["diameter"] / 2.0))
        fc = lfv.get_editor_property("local_fog_volume_volume")
        for k, v in (("radial_fog_extinction", FOG["radial"]), ("height_fog_extinction", FOG["height"]), ("height_fog_falloff", FOG["falloff"]),
                     ("height_fog_offset", FOG["offset"]), ("fog_phase_g", FOG["g"])):
            fc.set_editor_property(k, float(v))
        fc.set_editor_property("fog_albedo", unreal.LinearColor(FOG["albedo"], FOG["albedo"], FOG["albedo"], 1.0))
        hf = spawn(unreal.ExponentialHeightFog, unreal.Vector(0, 0, -100000), None, "HeightFog")
        h = hf.component
        # The scene only takes a height fog with some density; this one sits 1 km down with a steep falloff, so the
        # scene has none of it, but volumetric fog (which the Local Fog Volume renders through) is on.
        for k, v in (("fog_density", 1e-4), ("fog_height_falloff", 1.0), ("fog_max_opacity", 1.0), ("enable_volumetric_fog", True), ("volumetric_fog_scattering_distribution", FOG["g"]),
                     ("volumetric_fog_extinction_scale", 1.0), ("volumetric_fog_distance", 6000.0)):
            h.set_editor_property(k, v)
        h.set_editor_property("volumetric_fog_albedo", unreal.Color(255, 255, 255, 255))
    else:
        fog = volume_material("Fog", {"Radial": FOG["radial"], "Height": FOG["height"], "Falloff": FOG["falloff"], "Offset": FOG["offset"]},
                              {"Center": FOG["pos"], "Size": [FOG["diameter"]] * 3, "Albedo": [FOG["albedo"]] * 3})
        heterogeneous_volume("FogVolume", fog, FOG["pos"], [FOG["diameter"]] * 3)
    smoke = volume_material("Smoke", {"Density": SMOKE["density"], "NScale": SMOKE["scale"], "Rise": SMOKE["rise"], "Time": TIME},
                            {"Center": SMOKE["pos"], "Size": SMOKE["size"], "Albedo": SMOKE["albedo"]})
    heterogeneous_volume("Smoke", smoke, SMOKE["pos"], SMOKE["size"])
    fire = volume_material("Fire", {"Life": FIRE["life"], "Period": FIRE["period"], "Intensity": FIRE["intensity"], "Density": FIRE["density"],
                                    "NScale": FIRE["scale"], "Time": TIME, "Start": FIRE["start"]},
                           {"Center": FIRE["pos"], "Size": [FIRE["diameter"]] * 3})
    heterogeneous_volume("Fireball", fire, FIRE["pos"], [FIRE["diameter"]] * 3)
    ppv = spawn(unreal.PostProcessVolume, None, None, "PostProcess")
    ppv.set_editor_property("unbound", True)
    s = ppv.get_editor_property("settings")
    for k, v in (("auto_exposure_method", unreal.AutoExposureMethod.AEM_MANUAL), ("auto_exposure_apply_physical_camera_exposure", False),
                 ("auto_exposure_bias", -float(JOB.get("ev100", 3.0))), ("motion_blur_amount", 0.0), ("bloom_intensity", 0.0),
                 ("vignette_intensity", 0.0), ("local_exposure_highlight_contrast_scale", 1.0), ("local_exposure_shadow_contrast_scale", 1.0),
                 ("dynamic_global_illumination_method", unreal.DynamicGlobalIlluminationMethod.LUMEN), ("reflection_method", unreal.ReflectionMethod.LUMEN),
                 ("path_tracing_samples_per_pixel", int(JOB.get("spp", 1024))), ("path_tracing_max_bounces", int(JOB.get("bounces", 8))),
                 ("path_tracing_enable_denoiser", False), ("path_tracing_include_emissive", True)):
        s.set_editor_property(k, v)
        s.set_editor_property("override_" + k, True)
    ppv.set_editor_property("settings", s)
    LES.save_current_level()
    log(f"level {map_path}: floor, wall, lamp, moon, fog ({'Local Fog Volume' if method == 'lumen' else 'Heterogeneous Volume'}), smoke, fireball")
    return map_path


def make_sequence(w, h):
    seq = fresh("Seq_Volumes", unreal.LevelSequence, unreal.LevelSequenceFactoryNew())
    seq.set_display_rate(unreal.FrameRate(60, 1))
    seq.set_playback_start(0)
    seq.set_playback_end(1)
    loc, aim = ue([0, 1.6, 7]), ue([0, 1.4, 0])
    d = aim - loc
    rot = unreal.Rotator(roll=0.0, pitch=math.degrees(math.atan2(d.z, math.hypot(d.x, d.y))), yaw=math.degrees(math.atan2(d.y, d.x)))
    cam = spawn(unreal.CineCameraActor, loc, rot, "Camera")
    c = cam.get_cine_camera_component()
    fb = c.get_editor_property("filmback")
    sensor_h = 36.0 * h / w
    fb.set_editor_property("sensor_width", 36.0)
    fb.set_editor_property("sensor_height", sensor_h)
    c.set_editor_property("filmback", fb)
    c.set_editor_property("current_focal_length", sensor_h / (2.0 * math.tan(math.radians(50.0) / 2.0)))
    fs = c.get_editor_property("focus_settings")
    fs.set_editor_property("focus_method", unreal.CameraFocusMethod.DISABLE)
    c.set_editor_property("focus_settings", fs)
    c.set_editor_property("constrain_aspect_ratio", False)
    binding = seq.add_spawnable_from_instance(cam)
    cut = seq.add_track(unreal.MovieSceneCameraCutTrack).add_section()
    cut.set_range(0, 1)
    cut.set_camera_binding_id(unreal.MovieSceneSequenceExtensions.get_binding_id(seq, binding))
    EAS.destroy_actor(cam)
    EAL.save_loaded_asset(seq)
    return seq


def configure(j, method):
    cfg = j.get_configuration()
    out = cfg.find_or_add_setting_by_class(unreal.MoviePipelineOutputSetting)
    out.set_editor_property("output_directory", unreal.DirectoryPath(JOB["out"]))
    out.set_editor_property("file_name_format", method)
    out.set_editor_property("output_resolution", unreal.IntPoint(JOB.get("width", 1280), JOB.get("height", 720)))
    out.set_editor_property("override_existing_output", True)
    pt = method == "pathtracer"
    cfg.find_or_add_setting_by_class(unreal.MoviePipelineDeferredPass_PathTracer if pt else unreal.MoviePipelineDeferredPassBase)
    exr = cfg.find_or_add_setting_by_class(unreal.MoviePipelineImageSequenceOutput_EXR)
    exr.set_editor_property("compression", unreal.EXRCompressionFormat.ZIP)
    cfg.find_or_add_setting_by_class(unreal.MoviePipelineColorSetting).set_editor_property("disable_tone_curve", True)
    aa = cfg.find_or_add_setting_by_class(unreal.MoviePipelineAntiAliasingSetting)
    aa.set_editor_property("override_anti_aliasing", True)
    aa.set_editor_property("spatial_sample_count", int(JOB.get("spp", 1024)) if pt else 1)
    aa.set_editor_property("temporal_sample_count", 1 if pt else 16)
    aa.set_editor_property("anti_aliasing_method", unreal.AntiAliasingMethod.AAM_NONE if pt else unreal.AntiAliasingMethod.AAM_TSR)
    aa.set_editor_property("engine_warm_up_count", 8 if pt else 64)
    aa.set_editor_property("render_warm_up_count", 8 if pt else 64)
    aa.set_editor_property("use_camera_cut_for_warm_up", False)
    cv = cfg.find_or_add_setting_by_class(unreal.MoviePipelineConsoleVariableSetting)
    for k, v in {"r.MotionBlurQuality": 0, "r.HeterogeneousVolumes": 1, "r.VolumetricFog.GridPixelSize": 4, "r.VolumetricFog.GridSizeZ": 128}.items():
        cv.add_or_update_console_variable(k, float(v))
    game = cfg.find_or_add_setting_by_class(unreal.MoviePipelineGameOverrideSetting)
    game.set_editor_property("cinematic_quality_settings", True)


class Runner:
    def run(self):
        self.t0 = time.time()
        maps = {m: build_level(m) for m in JOB["methods"]}
        seq = make_sequence(JOB.get("width", 1280), JOB.get("height", 720))
        unreal.EditorLoadingAndSavingUtils.save_dirty_packages(True, True)
        qs = unreal.get_editor_subsystem(unreal.MoviePipelineQueueSubsystem)
        q = qs.get_queue()
        q.delete_all_jobs()
        for m, map_path in maps.items():
            j = q.allocate_new_job(unreal.MoviePipelineExecutorJob)
            j.set_editor_property("job_name", m)
            j.set_editor_property("map", unreal.SoftObjectPath(map_path))
            j.set_editor_property("sequence", unreal.SoftObjectPath(seq.get_path_name()))
            configure(j, m)
        self.ex = unreal.MoviePipelinePIEExecutor()
        self.ex.on_executor_finished_delegate.add_callable_unique(self.done)
        qs.render_queue_with_executor_instance(self.ex)
        log("render started")

    def done(self, executor, success):
        json.dump({"method": "UE 5.8 MRQ, linear EXR (tone curve off), manual exposure", "job": JOB, "total_s": round(time.time() - self.t0, 1)},
                  open(os.path.join(JOB["out"], "manifest.json"), "w"), indent=1)
        log(f"all done success={success} in {time.time() - self.t0:.0f}s")
        unreal.SystemLibrary.quit_editor()


_runner = Runner()
try:
    _runner.run()
except Exception:
    log("FAILED\n" + traceback.format_exc())
    unreal.SystemLibrary.quit_editor()
