"""Where UE 5.8's default auto exposure lands on the night scene compare/hello/threert/neon-reflections.ts.

The scene is built natively (no glTF route carries its spot lights and 1800-nit neon): floor, wall, chrome sphere, the
gold torus knot as a gold sphere of similar size (UE has no knot primitive), copper box, three neon tubes, two spot
lights (candela, cone from three.js angle / penumbra), the same camera. Its dim sky (0.6 nits) is left out: it is out
of frame and negligible next to the neon and the spots. Rendered with Lumen through Movie Render Queue (64 warm-up
frames, so eye adaptation settles): once with the project's default auto exposure (no exposure override), then with
manual EV100 values; matching the auto image's brightness against the manual ones gives the EV100 UE chose.

    THREERT_JOB=/abs/job.json UnrealEditor <scratch>.uproject -ExecCmds="py ue5/scripts/ue_neon_ae.py" -unattended ...
job.json: {"out": "/abs/out/ue-neon-ae", "evs": [-3, -2, ...], "width": 1280, "height": 720, "quit": true}
Coordinates: ours are Y-up metres, UE = 100 * (x, z, y) cm.
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
ROOT = "/Game/ThreeRTNeon"
JOB = json.load(open(os.environ["THREERT_JOB"]))


def log(msg):
    unreal.log("THREERT: " + msg)


def ue(v, s=100.0):
    return unreal.Vector(v[0] * s, v[2] * s, v[1] * s)


def spawn(cls, loc=None, rot=None, label=None):
    a = EAS.spawn_actor_from_class(cls, loc or unreal.Vector(0, 0, 0), rot or unreal.Rotator(0, 0, 0))
    if label:
        a.set_actor_label(label)
    return a


def master():
    path = f"{ROOT}/M_ThreeRT_PBR"
    if EAL.does_asset_exist(path):
        return EAL.load_asset(path)
    m = AT.create_asset("M_ThreeRT_PBR", ROOT, unreal.Material, unreal.MaterialFactoryNew())
    for i, (name, prop, vec) in enumerate((("BaseColor", unreal.MaterialProperty.MP_BASE_COLOR, True), ("Metallic", unreal.MaterialProperty.MP_METALLIC, False),
                                          ("Roughness", unreal.MaterialProperty.MP_ROUGHNESS, False), ("Emissive", unreal.MaterialProperty.MP_EMISSIVE_COLOR, True))):
        e = MEL.create_material_expression(m, unreal.MaterialExpressionVectorParameter if vec else unreal.MaterialExpressionScalarParameter, -400, 200 * i)
        e.set_editor_property("parameter_name", name)
        MEL.connect_material_property(e, "", prop)
    MEL.recompile_material(m)
    EAL.save_loaded_asset(m)
    return m


def material(name, base, metallic, roughness, emissive=(0.0, 0.0, 0.0)):
    path = f"{ROOT}/MI_{name}"
    if EAL.does_asset_exist(path):
        EAL.delete_asset(path)
    mi = AT.create_asset(f"MI_{name}", ROOT, unreal.MaterialInstanceConstant, unreal.MaterialInstanceConstantFactoryNew())
    MEL.set_material_instance_parent(mi, master())
    MEL.set_material_instance_vector_parameter_value(mi, "BaseColor", unreal.LinearColor(base[0], base[1], base[2], 1.0))
    MEL.set_material_instance_scalar_parameter_value(mi, "Metallic", metallic)
    MEL.set_material_instance_scalar_parameter_value(mi, "Roughness", roughness)
    MEL.set_material_instance_vector_parameter_value(mi, "Emissive", unreal.LinearColor(emissive[0], emissive[1], emissive[2], 1.0))
    EAL.save_loaded_asset(mi)
    return mi


def srgb(h):
    """three.js hex colour (sRGB) -> linear."""
    c = [((h >> s) & 255) / 255.0 for s in (16, 8, 0)]
    return tuple(v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4 for v in c)


def shape(kind, pos, scale, mat, label, rot=None):
    a = spawn(unreal.StaticMeshActor, ue(pos), rot or unreal.Rotator(0, 0, 0), label)
    c = a.static_mesh_component
    c.set_mobility(unreal.ComponentMobility.MOVABLE)
    c.set_static_mesh(EAL.load_asset(f"/Engine/BasicShapes/{kind}"))
    a.set_actor_scale3d(unreal.Vector(*scale))
    c.set_material(0, mat)
    return a


def spot(label, color, cd, pos, target, angle, penumbra):
    d = ue([t - p for t, p in zip(target, pos)], 1.0)
    rot = unreal.Rotator(roll=0.0, pitch=math.degrees(math.atan2(d.z, math.hypot(d.x, d.y))), yaw=math.degrees(math.atan2(d.y, d.x)))
    a = spawn(unreal.SpotLight, ue(pos), rot, label)
    c = a.spot_light_component
    c.set_mobility(unreal.ComponentMobility.MOVABLE)
    c.set_editor_property("intensity_units", unreal.LightUnits.CANDELAS)
    c.set_editor_property("intensity", float(cd))
    c.set_editor_property("use_temperature", False)
    c.set_light_color(unreal.LinearColor(color[0], color[1], color[2], 1.0), True)
    c.set_editor_property("outer_cone_angle", math.degrees(angle))
    c.set_editor_property("inner_cone_angle", math.degrees(angle * (1.0 - penumbra)))
    c.set_editor_property("attenuation_radius", 10000.0)
    c.set_editor_property("cast_shadows", True)


def build():
    map_path = f"{ROOT}/Map_NeonAE_{int(time.time())}"
    if not LES.new_level(map_path):
        raise RuntimeError(f"new_level {map_path} failed")
    shape("Plane", (0, 0, 0), (40, 40, 1), material("floor", srgb(0x0C0C0E), 0.0, 0.06), "floor")
    shape("Cube", (0, 3, -3.2), (14, 0.4, 6), material("wall", srgb(0x3A3A3C), 0.0, 0.8), "wall")
    shape("Sphere", (-1.6, 0.75, -0.4), (1.5, 1.5, 1.5), material("chrome", srgb(0xF4F4F4), 1.0, 0.03), "chrome")
    shape("Sphere", (0.6, 1.0, -0.8), (1.2, 1.2, 1.2), material("gold", (1.0, 0.766, 0.336), 1.0, 0.22), "gold (knot)")
    shape("Cube", (2.3, 0.4, 0.2), (0.8, 0.8, 0.8), material("copper", (0.955, 0.637, 0.538), 1.0, 0.45), "copper", unreal.Rotator(0, -math.degrees(0.6), 0))
    for i, (col, x, y, ln, rz) in enumerate((((1.0, 0.08, 0.55), -2.2, 2.4, 2.6, math.pi / 2), ((0.05, 0.75, 1.0), 1.8, 2.9, 2.0, math.pi / 2), ((1.0, 0.45, 0.05), 3.3, 1.6, 1.8, 0.0))):
        # UE's cylinder is 100 cm tall along Z (our Y); a tube rotated pi/2 about our Z lies along X.
        rot = unreal.Rotator(roll=0.0, pitch=90.0 if rz else 0.0, yaw=0.0)
        shape("Cylinder", (x, y, -2.95), (0.07, 0.07, ln), material(f"neon{i}", (0.0, 0.0, 0.0), 0.0, 0.5, tuple(c * 1800.0 for c in col)), f"neon{i}", rot)
    spot("key", (1.0, 0.9, 0.8), 2500, (2.5, 4.5, 3.0), (0, 0.6, -0.5), 0.45, 0.35)
    spot("rim", (0.5, 0.6, 1.0), 1500, (-3.5, 3.5, -1.5), (-0.5, 0.5, -0.5), 0.6, 0.5)
    return map_path


def camera(w, h):
    loc, aim = ue((0.3, 1.5, 5.6)), ue((0.2, 0.9, -0.6))
    d = aim - loc
    rot = unreal.Rotator(roll=0.0, pitch=math.degrees(math.atan2(d.z, math.hypot(d.x, d.y))), yaw=math.degrees(math.atan2(d.y, d.x)))
    a = spawn(unreal.CineCameraActor, loc, rot, "Camera")
    c = a.get_cine_camera_component()
    fb = c.get_editor_property("filmback")
    sh = 36.0 * h / w
    fb.set_editor_property("sensor_width", 36.0)
    fb.set_editor_property("sensor_height", sh)
    c.set_editor_property("filmback", fb)
    c.set_editor_property("current_focal_length", sh / (2.0 * math.tan(math.radians(40.0) / 2.0)))
    fs = c.get_editor_property("focus_settings")
    fs.set_editor_property("focus_method", unreal.CameraFocusMethod.DISABLE)
    c.set_editor_property("focus_settings", fs)
    return a


def sequence(cam):
    path = f"{ROOT}/Seq_NeonAE"
    if EAL.does_asset_exist(path):
        EAL.delete_asset(path)
    seq = AT.create_asset("Seq_NeonAE", ROOT, unreal.LevelSequence, unreal.LevelSequenceFactoryNew())
    seq.set_display_rate(unreal.FrameRate(24, 1))
    seq.set_playback_start(0)
    seq.set_playback_end(1)
    binding = seq.add_spawnable_from_instance(cam)
    sec = seq.add_track(unreal.MovieSceneCameraCutTrack).add_section()
    sec.set_range(0, 1)
    sec.set_camera_binding_id(unreal.MovieSceneSequenceExtensions.get_binding_id(seq, binding))
    EAS.destroy_actor(cam)
    EAL.save_loaded_asset(seq)
    return seq


class Runner:
    def run(self):
        self.t0 = time.time()
        self.map_path = build()
        self.cam = camera(JOB["width"], JOB["height"])
        LES.save_current_level()
        self.seq = sequence(self.cam)
        unreal.EditorLoadingAndSavingUtils.save_dirty_packages(True, True)
        self.jobs = [("auto", None)] + [(f"ev{ev:+g}", ev) for ev in JOB["evs"]]
        self.next()

    def next(self):
        if not self.jobs:
            log(f"all done in {time.time() - self.t0:.0f}s")
            if JOB.get("quit", True):
                unreal.SystemLibrary.quit_editor()
            return
        name, ev = self.jobs.pop(0)
        # Manual EV: one unbound post volume with the exposure override, spawned before rendering this job.
        for a in EAS.get_all_level_actors():
            if a.get_actor_label() == "ThreeRT_Exposure":
                EAS.destroy_actor(a)
        if ev is not None:
            ppv = spawn(unreal.PostProcessVolume, None, None, "ThreeRT_Exposure")
            ppv.set_editor_property("unbound", True)
            s = ppv.get_editor_property("settings")
            for k, v in (("auto_exposure_method", unreal.AutoExposureMethod.AEM_MANUAL), ("auto_exposure_apply_physical_camera_exposure", False), ("auto_exposure_bias", -float(ev))):
                s.set_editor_property(k, v)
                s.set_editor_property("override_" + k, True)
            ppv.set_editor_property("settings", s)
        LES.save_current_level()
        qs = unreal.get_editor_subsystem(unreal.MoviePipelineQueueSubsystem)
        q = qs.get_queue()
        q.delete_all_jobs()
        j = q.allocate_new_job(unreal.MoviePipelineExecutorJob)
        j.set_editor_property("job_name", name)
        j.set_editor_property("map", unreal.SoftObjectPath(self.map_path))
        j.set_editor_property("sequence", unreal.SoftObjectPath(self.seq.get_path_name()))
        cfg = j.get_configuration()
        out = cfg.find_or_add_setting_by_class(unreal.MoviePipelineOutputSetting)
        out.set_editor_property("output_directory", unreal.DirectoryPath(JOB["out"]))
        out.set_editor_property("file_name_format", name)
        out.set_editor_property("output_resolution", unreal.IntPoint(JOB["width"], JOB["height"]))
        out.set_editor_property("zero_pad_frame_numbers", 0)
        out.set_editor_property("override_existing_output", True)
        cfg.find_or_add_setting_by_class(unreal.MoviePipelineDeferredPassBase)
        cfg.find_or_add_setting_by_class(unreal.MoviePipelineImageSequenceOutput_PNG)
        aa = cfg.find_or_add_setting_by_class(unreal.MoviePipelineAntiAliasingSetting)
        aa.set_editor_property("override_anti_aliasing", True)
        aa.set_editor_property("spatial_sample_count", 1)
        aa.set_editor_property("temporal_sample_count", 8)
        aa.set_editor_property("anti_aliasing_method", unreal.AntiAliasingMethod.AAM_TSR)
        aa.set_editor_property("engine_warm_up_count", 64)
        aa.set_editor_property("render_warm_up_count", 64)
        aa.set_editor_property("use_camera_cut_for_warm_up", False)
        self.name = name
        self.ex = unreal.MoviePipelinePIEExecutor()
        self.ex.on_executor_finished_delegate.add_callable_unique(self.done)
        qs.render_queue_with_executor_instance(self.ex)
        log(f"render {name} started")

    def done(self, executor, success):
        log(f"render {self.name} done success={success}")
        try:
            self.next()
        except Exception:
            log("FAILED\n" + traceback.format_exc())
            unreal.SystemLibrary.quit_editor()


_runner = Runner()
try:
    _runner.run()
except Exception:
    log("FAILED\n" + traceback.format_exc())
    if JOB.get("quit", True):
        unreal.SystemLibrary.quit_editor()
