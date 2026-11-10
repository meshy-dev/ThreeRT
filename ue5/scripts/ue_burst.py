"""UE 5.8 reference for a burst of sparks and debris (ROADMAP § 5 item 6.2): UE's own SimpleExplosion Niagara template
("a sprite core, velocity aligned sparks, and oriented meshes"), unchanged, over a grey floor in the dark, captured
with Movie Render Queue at 60 fps.

    THREERT_JOB=/abs/job.json UnrealEditor <scratch>.uproject -ExecCmds="py ue5/scripts/ue_burst.py" -unattended ...
job.json: {"frames": "/abs/out/ue-burst/frames", "eye": [x, y, z], "target": [x, y, z], "vfov": 50, "ev100": 6,
           "fire": 10, "end": 150, "width": 1280, "height": 720, "screenPercentage": 100}
Driven by ue5/burst.ts (scratch project as ue5/mannequin.ts).

Scene (mirror of tools/burst-scene.ts; keep in sync): a 30 x 30 m floor (linear albedo 0.35, roughness 0.6), no lights
(the burst is the only light), manual exposure `ev100`, CineCamera 16:9 at `eye` looking at `target`. The system sits at
the origin, inactive; the per-frame driver activates it at sequence frame `fire`.
Coordinates: ours are Y-up metres, UE = 100 * (x, z, y) cm.
"""
import json
import math
import os
import shutil
import time
import traceback

import unreal

EAL = unreal.EditorAssetLibrary
MEL = unreal.MaterialEditingLibrary
AT = unreal.AssetToolsHelpers.get_asset_tools()
EAS = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
LES = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
UES = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem)
ROOT = "/Game/ThreeRT"
TEMPLATE = "/Niagara/DefaultAssets/Templates/Systems/SimpleExplosion"
JOB = json.load(open(os.environ["THREERT_JOB"]))
FPS = 60
FIRE = JOB.get("fire", 10)
END = JOB.get("end", 150)


def log(msg):
    unreal.log("THREERT: " + msg)


def ue(v, s=100.0):
    """ours (x, y, z) m -> UE (x, z, y) cm."""
    return unreal.Vector(v[0] * s, v[2] * s, v[1] * s)


def spawn(cls, loc=None, rot=None, label=None):
    a = EAS.spawn_actor_from_class(cls, loc or unreal.Vector(0, 0, 0), rot or unreal.Rotator(0, 0, 0))
    if label:
        a.set_actor_label(label)
    return a


def floor_material():
    path = f"{ROOT}/M_ThreeRT_Floor"
    if EAL.does_asset_exist(path):
        return EAL.load_asset(path)
    mat = AT.create_asset("M_ThreeRT_Floor", ROOT, unreal.Material, unreal.MaterialFactoryNew())
    for i, (prop, value) in enumerate(((unreal.MaterialProperty.MP_BASE_COLOR, (0.35, 0.35, 0.35)), (unreal.MaterialProperty.MP_ROUGHNESS, 0.6))):
        if isinstance(value, tuple):
            e = MEL.create_material_expression(mat, unreal.MaterialExpressionConstant3Vector, -400, 0)
            e.set_editor_property("constant", unreal.LinearColor(value[0], value[1], value[2], 1.0))
        else:
            e = MEL.create_material_expression(mat, unreal.MaterialExpressionConstant, -400, 200 * i)
            e.set_editor_property("r", value)
        MEL.connect_material_property(e, "", prop)
    MEL.recompile_material(mat)
    EAL.save_loaded_asset(mat)
    return mat


def build_level():
    map_path = f"{ROOT}/L_Burst"
    if EAL.does_asset_exist(map_path):
        EAL.delete_asset(map_path)
    if not LES.new_level(map_path):
        raise RuntimeError(f"new_level {map_path} failed")
    floor = spawn(unreal.StaticMeshActor, None, None, "Floor")
    c = floor.static_mesh_component
    c.set_static_mesh(EAL.load_asset("/Engine/BasicShapes/Plane"))
    floor.set_actor_scale3d(unreal.Vector(30.0, 30.0, 1.0))
    c.set_material(0, floor_material())
    # The template, unchanged (a copy in the project), inactive until the driver fires it.
    sys_path = f"{ROOT}/NS_Burst"
    if EAL.does_asset_exist(sys_path):
        EAL.delete_asset(sys_path)
    system = EAL.duplicate_asset(TEMPLATE, sys_path)
    if system is None:
        raise RuntimeError(f"duplicate {TEMPLATE} failed")
    na = spawn(unreal.NiagaraActor, unreal.Vector(0, 0, 5), None, "Burst")
    nc = na.niagara_component
    nc.set_asset(system)
    nc.set_auto_activate(False)
    na.tags = ["ThreeRTBurst"]
    # Manual exposure, no motion blur.
    ppv = spawn(unreal.PostProcessVolume, None, None, "PostProcess")
    ppv.set_editor_property("unbound", True)
    s = ppv.get_editor_property("settings")
    for k, v in (("auto_exposure_method", unreal.AutoExposureMethod.AEM_MANUAL), ("auto_exposure_apply_physical_camera_exposure", False),
                 ("auto_exposure_bias", -float(JOB.get("ev100", 6.0))), ("motion_blur_amount", 0.0)):
        s.set_editor_property(k, v)
        s.set_editor_property("override_" + k, True)
    ppv.set_editor_property("settings", s)
    LES.save_current_level()
    log(f"level {map_path}: floor, {system.get_path_name()} at the origin, EV100 {JOB.get('ev100', 6.0)}")
    return map_path


def make_sequence(w, h):
    path = f"{ROOT}/Seq_Burst"
    if EAL.does_asset_exist(path):
        EAL.delete_asset(path)
    seq = AT.create_asset("Seq_Burst", ROOT, unreal.LevelSequence, unreal.LevelSequenceFactoryNew())
    seq.set_display_rate(unreal.FrameRate(FPS, 1))
    seq.set_playback_start(0)
    seq.set_playback_end(END + 1)
    loc, aim = ue(JOB["eye"]), ue(JOB["target"])
    d = aim - loc
    rot = unreal.Rotator(roll=0.0, pitch=math.degrees(math.atan2(d.z, math.hypot(d.x, d.y))), yaw=math.degrees(math.atan2(d.y, d.x)))
    cam = spawn(unreal.CineCameraActor, loc, rot, "Camera")
    c = cam.get_cine_camera_component()
    fb = c.get_editor_property("filmback")
    sensor_h = 36.0 * h / w
    fb.set_editor_property("sensor_width", 36.0)
    fb.set_editor_property("sensor_height", sensor_h)
    c.set_editor_property("filmback", fb)
    c.set_editor_property("current_focal_length", sensor_h / (2.0 * math.tan(math.radians(JOB.get("vfov", 50.0)) / 2.0)))
    fs = c.get_editor_property("focus_settings")
    fs.set_editor_property("focus_method", unreal.CameraFocusMethod.DISABLE)
    c.set_editor_property("focus_settings", fs)
    c.set_editor_property("constrain_aspect_ratio", False)
    binding = seq.add_spawnable_from_instance(cam)
    cut = seq.add_track(unreal.MovieSceneCameraCutTrack).add_section()
    cut.set_range(0, END + 1)
    cut.set_camera_binding_id(unreal.MovieSceneSequenceExtensions.get_binding_id(seq, binding))
    EAS.destroy_actor(cam)
    EAL.save_loaded_asset(seq)
    log(f"sequence {seq.get_path_name()}: {END + 1} frames at {FPS} fps, camera {loc} {rot}")
    return seq


def configure(j):
    cfg = j.get_configuration()
    out = cfg.find_or_add_setting_by_class(unreal.MoviePipelineOutputSetting)
    out.set_editor_property("output_directory", unreal.DirectoryPath(JOB["frames"]))
    out.set_editor_property("file_name_format", "{frame_number}")
    out.set_editor_property("output_resolution", unreal.IntPoint(JOB.get("width", 1280), JOB.get("height", 720)))
    out.set_editor_property("zero_pad_frame_numbers", 4)
    out.set_editor_property("override_existing_output", True)
    cfg.find_or_add_setting_by_class(unreal.MoviePipelineDeferredPassBase)
    cfg.find_or_add_setting_by_class(unreal.MoviePipelineImageSequenceOutput_PNG)
    aa = cfg.find_or_add_setting_by_class(unreal.MoviePipelineAntiAliasingSetting)
    aa.set_editor_property("override_anti_aliasing", True)
    aa.set_editor_property("anti_aliasing_method", unreal.AntiAliasingMethod.AAM_TSR)
    aa.set_editor_property("spatial_sample_count", 1)
    aa.set_editor_property("temporal_sample_count", 1)
    aa.set_editor_property("engine_warm_up_count", 32)
    aa.set_editor_property("render_warm_up_count", 32)
    aa.set_editor_property("use_camera_cut_for_warm_up", False)
    cam = cfg.find_or_add_setting_by_class(unreal.MoviePipelineCameraSetting)
    cam.set_editor_property("shutter_timing", unreal.MoviePipelineShutterTiming.FRAME_OPEN)
    cv = cfg.find_or_add_setting_by_class(unreal.MoviePipelineConsoleVariableSetting)
    for k, v in {"r.MotionBlurQuality": 0, "r.ScreenPercentage": JOB.get("screenPercentage", 100), "sg.EffectsQuality": 3}.items():
        cv.add_or_update_console_variable(k, float(v))
    game = cfg.find_or_add_setting_by_class(unreal.MoviePipelineGameOverrideSetting)
    game.set_editor_property("cinematic_quality_settings", False)
    game.set_editor_property("texture_streaming", unreal.MoviePipelineTextureStreamingMethod.DISABLED)


class Driver:
    """Fires the system at sequence frame FIRE in the MRQ PIE world (after MRQ's warm-up)."""

    def __init__(self):
        self.world, self.fired = None, False
        self.handle = unreal.register_slate_post_tick_callback(self.tick)

    def frame_of(self, world):
        for a in unreal.GameplayStatics.get_all_actors_of_class(world, unreal.LevelSequenceActor):
            p = a.sequence_player
            if p is None:
                continue
            qt = p.get_current_time()
            rate = qt.rate.numerator / qt.rate.denominator
            return (qt.time.frame_number.value + qt.time.sub_frame) * FPS / rate
        return None

    def tick(self, dt):
        try:
            world = UES.get_game_world()
            if world is None:
                return
            if world != self.world:
                self.world, self.fired = world, False
            fr = self.frame_of(world)
            if fr is None or self.fired or fr < FIRE - 1 + 1e-3:
                return
            for a in unreal.GameplayStatics.get_all_actors_with_tag(world, "ThreeRTBurst"):
                a.niagara_component.activate(True)
            self.fired = True
            log(f"fired at sequence frame {fr:.3f}")
        except Exception:
            log("driver error\n" + traceback.format_exc())
            unreal.unregister_slate_post_tick_callback(self.handle)


class Runner:
    def run(self):
        self.t0 = time.time()
        map_path = build_level()
        seq = make_sequence(JOB.get("width", 1280), JOB.get("height", 720))
        LES.save_current_level()
        unreal.EditorLoadingAndSavingUtils.save_dirty_packages(True, True)
        if os.path.isdir(JOB["frames"]):
            shutil.rmtree(JOB["frames"])
        qs = unreal.get_editor_subsystem(unreal.MoviePipelineQueueSubsystem)
        q = qs.get_queue()
        q.delete_all_jobs()
        j = q.allocate_new_job(unreal.MoviePipelineExecutorJob)
        j.set_editor_property("job_name", "burst")
        j.set_editor_property("map", unreal.SoftObjectPath(map_path))
        j.set_editor_property("sequence", unreal.SoftObjectPath(seq.get_path_name()))
        configure(j)
        self.driver = Driver()
        self.ex = unreal.MoviePipelinePIEExecutor()
        self.ex.on_executor_finished_delegate.add_callable_unique(self.done)
        qs.render_queue_with_executor_instance(self.ex)
        log(f"render started ({END + 1} frames, fire at {FIRE})")

    def done(self, executor, success):
        n = len([x for x in os.listdir(JOB["frames"]) if x.endswith(".png")]) if os.path.isdir(JOB["frames"]) else 0
        json.dump({"method": "UE 5.8 MRQ, Niagara SimpleExplosion template unchanged, TSR 1 temporal sample / frame at 60 fps, motion blur off, manual exposure",
                   "template": TEMPLATE, "fire": FIRE, "end": END, "frames": n, "total_s": round(time.time() - self.t0, 1), "job": JOB},
                  open(os.path.join(JOB["frames"], "manifest.json"), "w"), indent=1)
        log(f"all done success={success} rendered {n} frames")
        unreal.SystemLibrary.quit_editor()


_runner = Runner()
try:
    _runner.run()
except Exception:
    log("FAILED\n" + traceback.format_exc())
    unreal.SystemLibrary.quit_editor()
