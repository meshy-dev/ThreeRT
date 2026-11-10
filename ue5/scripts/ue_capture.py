"""Ground-truth stills of an existing Unreal level (any project) with Movie Render Queue, from fixed cameras.

Renders the level (run it on a scratch copy of the project: the level is saved with changes) with selected actor classes hidden (crowds, particle
rain ...), optional manual exposure, Lumen or the path tracer. Used as the reference for porting UE scenes to ThreeRT.

    THREERT_CAPTURE=/abs/job.json UnrealEditor <project>.uproject -ExecCmds="py ue5/scripts/ue_capture.py" \
        -unattended -nosplash -nopause -RenderOffscreen -NoSound

job.json: {"map": "/Game/.../L_X", "out": "/abs/dir", "method": "lumen" | "pathtracer", "width": 1280, "height": 720,
           "views": [{"name": "v0", "location": [x, y, z] (UE cm), "rotation": [pitch, yaw, roll], "fov": vertical deg}],
           "hide": ["KabukichoCrowd"], "ev100": null | number, "spp": 1024, "warmup": 64, "temporal": 16,
           "cvars": {"r.Lumen.HardwareRayTracing": 1}}
"""
import json
import math
import os
import time
import traceback

import unreal

EAL = unreal.EditorAssetLibrary
EAS = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
LES = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
AT = unreal.AssetToolsHelpers.get_asset_tools()
ROOT = "/Game/ThreeRTCapture"


def log(msg):
    unreal.log("THREERT: " + msg)


def prepare_level(job):
    # Run on a scratch copy of the project: the level is modified and saved in place (duplicating a loaded world
    # trips the editor's GC check on reload).
    dup = job["map"]
    LES.load_level(dup)
    hidden = 0
    for a in EAS.get_all_level_actors():
        if a.get_class().get_name() in job.get("hide", []):
            a.set_actor_hidden_in_game(True)
            a.set_actor_tick_enabled(False)
            hidden += 1
        if isinstance(a, unreal.PostProcessVolume) and job.get("ev100") is not None:
            s = a.get_editor_property("settings")
            for k, v in (("auto_exposure_method", unreal.AutoExposureMethod.AEM_MANUAL), ("auto_exposure_apply_physical_camera_exposure", False),
                         ("auto_exposure_bias", -float(job["ev100"]))):
                s.set_editor_property(k, v)
                s.set_editor_property("override_" + k, True)
            if job.get("noMotionBlur", True):
                s.set_editor_property("motion_blur_amount", 0.0)
                s.set_editor_property("override_motion_blur_amount", True)
            if job.get("linear"):
                # Radiometric capture: no bloom / vignette / grain / fringe / local exposure (tone curve off in MRQ).
                for k, v in (("bloom_intensity", 0.0), ("vignette_intensity", 0.0), ("film_grain_intensity", 0.0), ("scene_fringe_intensity", 0.0),
                             ("local_exposure_highlight_contrast_scale", 1.0), ("local_exposure_shadow_contrast_scale", 1.0)):
                    s.set_editor_property(k, v)
                    s.set_editor_property("override_" + k, True)
            a.set_editor_property("settings", s)
    log(f"level {dup}: hid {hidden} actors")
    return dup


def make_sequence(view, w, h):
    path = f"{ROOT}/Seq_{view['name']}"
    if EAL.does_asset_exist(path):
        EAL.delete_asset(path)
    seq = AT.create_asset(f"Seq_{view['name']}", ROOT, unreal.LevelSequence, unreal.LevelSequenceFactoryNew())
    seq.set_display_rate(unreal.FrameRate(24, 1))
    seq.set_playback_start(0)
    seq.set_playback_end(1)
    loc = unreal.Vector(*view["location"])
    p, y, r = view["rotation"]
    cam = EAS.spawn_actor_from_class(unreal.CineCameraActor, loc, unreal.Rotator(roll=r, pitch=p, yaw=y))
    c = cam.get_cine_camera_component()
    fb = c.get_editor_property("filmback")
    sensor_w, sensor_h = 36.0, 36.0 * h / w
    fb.set_editor_property("sensor_width", sensor_w)
    fb.set_editor_property("sensor_height", sensor_h)
    c.set_editor_property("filmback", fb)
    c.set_editor_property("current_focal_length", sensor_h / (2.0 * math.tan(math.radians(view["fov"]) / 2.0)))
    fs = c.get_editor_property("focus_settings")
    fs.set_editor_property("focus_method", unreal.CameraFocusMethod.DISABLE)
    c.set_editor_property("focus_settings", fs)
    c.set_editor_property("constrain_aspect_ratio", False)
    binding = seq.add_spawnable_from_instance(cam)
    section = seq.add_track(unreal.MovieSceneCameraCutTrack).add_section()
    section.set_range(0, 1)
    section.set_camera_binding_id(unreal.MovieSceneSequenceExtensions.get_binding_id(seq, binding))
    EAS.destroy_actor(cam)
    EAL.save_loaded_asset(seq)
    return seq


def configure(j, job, view):
    cfg = j.get_configuration()
    out = cfg.find_or_add_setting_by_class(unreal.MoviePipelineOutputSetting)
    out.set_editor_property("output_directory", unreal.DirectoryPath(job["out"]))
    out.set_editor_property("file_name_format", view["name"])
    out.set_editor_property("output_resolution", unreal.IntPoint(job["width"], job["height"]))
    out.set_editor_property("zero_pad_frame_numbers", 0)
    out.set_editor_property("override_existing_output", True)
    pt = job["method"] == "pathtracer"
    cfg.find_or_add_setting_by_class(unreal.MoviePipelineDeferredPass_PathTracer if pt else unreal.MoviePipelineDeferredPassBase)
    if job.get("linear"):
        cfg.find_or_add_setting_by_class(unreal.MoviePipelineImageSequenceOutput_EXR)
        cfg.find_or_add_setting_by_class(unreal.MoviePipelineColorSetting).set_editor_property("disable_tone_curve", True)
    else:
        cfg.find_or_add_setting_by_class(unreal.MoviePipelineImageSequenceOutput_PNG)
    aa = cfg.find_or_add_setting_by_class(unreal.MoviePipelineAntiAliasingSetting)
    aa.set_editor_property("override_anti_aliasing", True)
    if pt:
        aa.set_editor_property("spatial_sample_count", int(job.get("spp", 1024)))
        aa.set_editor_property("temporal_sample_count", 1)
        aa.set_editor_property("anti_aliasing_method", unreal.AntiAliasingMethod.AAM_NONE)
        aa.set_editor_property("engine_warm_up_count", 8)
        aa.set_editor_property("render_warm_up_count", 8)
    else:
        aa.set_editor_property("spatial_sample_count", 1)
        aa.set_editor_property("temporal_sample_count", int(job.get("temporal", 16)))
        aa.set_editor_property("anti_aliasing_method", unreal.AntiAliasingMethod.AAM_TSR)
        aa.set_editor_property("engine_warm_up_count", int(job.get("warmup", 64)))
        aa.set_editor_property("render_warm_up_count", int(job.get("warmup", 64)))
    aa.set_editor_property("use_camera_cut_for_warm_up", False)
    cv = cfg.find_or_add_setting_by_class(unreal.MoviePipelineConsoleVariableSetting)
    for k, v in job.get("cvars", {"r.Lumen.HardwareRayTracing": 1}).items():
        cv.add_or_update_console_variable(k, float(v))
    game = cfg.find_or_add_setting_by_class(unreal.MoviePipelineGameOverrideSetting)
    game.set_editor_property("cinematic_quality_settings", True)
    game.set_editor_property("texture_streaming", unreal.MoviePipelineTextureStreamingMethod.DISABLED)
    game.set_editor_property("use_lod_zero", True)
    game.set_editor_property("flush_streaming_managers", True)


class Runner:
    def run(self, job):
        self.job = job
        self.t0 = time.time()
        level = prepare_level(job)
        seqs = [(v, make_sequence(v, job["width"], job["height"])) for v in job["views"]]
        LES.save_current_level()
        unreal.EditorLoadingAndSavingUtils.save_dirty_packages(True, True)
        qs = unreal.get_editor_subsystem(unreal.MoviePipelineQueueSubsystem)
        q = qs.get_queue()
        q.delete_all_jobs()
        for v, seq in seqs:
            j = q.allocate_new_job(unreal.MoviePipelineExecutorJob)
            j.set_editor_property("job_name", v["name"])
            j.set_editor_property("map", unreal.SoftObjectPath(level))
            j.set_editor_property("sequence", unreal.SoftObjectPath(seq.get_path_name()))
            configure(j, job, v)
        self.ex = unreal.MoviePipelinePIEExecutor()
        self.ex.on_executor_finished_delegate.add_callable_unique(self.done)
        self.ex.on_individual_job_work_finished_delegate.add_callable_unique(lambda p: log(f"view done after {time.time() - self.t0:.0f}s"))
        qs.render_queue_with_executor_instance(self.ex)
        log("render started")

    def done(self, executor, success):
        log(f"all done success={success} in {time.time() - self.t0:.0f}s")
        unreal.SystemLibrary.quit_editor()


_runner = Runner()
try:
    with open(os.environ["THREERT_CAPTURE"]) as fh:
        _runner.run(json.load(fh))
except Exception:
    log("FAILED\n" + traceback.format_exc())
    unreal.SystemLibrary.quit_editor()
