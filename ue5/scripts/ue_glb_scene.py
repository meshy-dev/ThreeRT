"""UE 5.8 ground truth for a static glTF scene of the render suite's layer B (scenes/khronos/studio.ts): one GLB in the
mannequin plaza's daylight studio, built natively and rendered with Movie Render Queue.

    THREERT_JOB=/abs/job.json UnrealEditor <scratch>.uproject -ExecCmds="py ue5/scripts/ue_glb_scene.py" -unattended ...
job.json: {"name": "khronos-clearcoat", "glb": "/abs/ClearCoatTest.glb", "lift": 6.25, "eye": [x, y, z], "target": [x, y, z],
           "vfov": 35, "ev100": 14.6, "sun": {"lux": 80000, "elevation": 38, "azimuth": -35}, "outRoot": "/abs/out/ue",
           "work": "/abs/out/ue-glb", "methods": ["lumen", "pathtracer"], "presetsDir": "/abs/presets", "width": 1280,
           "height": 720, "quit": true}
Driven by ue5/glb-scene.ts (scratch project as ue5/mannequin.ts; the job comes from tools/ue-glb-scene.ts).

Scene (mirror of scenes/khronos/studio.ts; keep in sync):
  - The GLB imported with Interchange as a scene (actors at the node transforms, the glTF material instances: clearcoat,
    transmission, metallic-roughness), raised by `lift` metres onto the ground.
  - 2 km ground plane (linear albedo 0.35, roughness 0.6); sun 80000 lux at the top of the atmosphere (Atmosphere Sun
    Light, source angle 0.5357 deg) from setSunAngles(elevation, azimuth); SkyAtmosphere defaults; SkyLight real-time
    capture. CineCamera 16:9 at `eye` looking at `target`, vertical fov `vfov`, no DOF; manual exposure `ev100`.
Coordinates: ours are Y-up metres, UE = 100 * (x, z, y) cm (Interchange glTF convention).
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


def ground_material():
    path = f"{ROOT}/M_ThreeRT_Ground"
    if EAL.does_asset_exist(path):
        return EAL.load_asset(path)
    mat = AT.create_asset("M_ThreeRT_Ground", ROOT, unreal.Material, unreal.MaterialFactoryNew())
    for i, (prop, value) in enumerate(((unreal.MaterialProperty.MP_BASE_COLOR, (0.35, 0.35, 0.35)), (unreal.MaterialProperty.MP_ROUGHNESS, 0.6),
                                       (unreal.MaterialProperty.MP_METALLIC, 0.0), (unreal.MaterialProperty.MP_SPECULAR, 0.5))):
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


def start_import(path):
    """Interchange scene import into the current level (actors at the glTF node transforms, glTF material instances).
    It finishes on the game thread over the next ticks: Runner polls `importing()`."""
    mgr = unreal.InterchangeManager.get_interchange_manager_scripted()
    params = unreal.ImportAssetParameters()
    params.is_automated = True
    if not mgr.import_scene(f"{ROOT}/{JOB['name'].replace('-', '_')}", unreal.InterchangeManager.create_source_data(path), params):
        raise RuntimeError(f"import_scene {path} failed")


def importing():
    return unreal.InterchangeManager.get_interchange_manager_scripted().is_interchange_active()


def static_mesh_actor(mesh, loc, rot, scale, mat, label):
    a = spawn(unreal.StaticMeshActor, loc, rot, label)
    c = a.static_mesh_component
    c.set_mobility(unreal.ComponentMobility.MOVABLE)
    c.set_static_mesh(mesh)
    a.set_actor_scale3d(scale)
    for i in range(c.get_num_materials()):
        c.set_material(i, mat)
    return a


def sun_rotation():
    el, az = math.radians(JOB["sun"]["elevation"]), math.radians(JOB["sun"]["azimuth"])
    to_sun = (math.sin(az) * math.cos(el), math.sin(el), math.cos(az) * math.cos(el))  # DirectionalLight.setSunAngles
    d = ue([-c for c in to_sun], 1.0)  # direction the light travels, UE axes
    return unreal.Rotator(roll=0.0, pitch=math.degrees(math.asin(d.z)), yaw=math.degrees(math.atan2(d.y, d.x)))


def lights():
    a = spawn(unreal.DirectionalLight, unreal.Vector(0, 0, 1000), sun_rotation(), "Sun")
    c = a.light_component
    c.set_mobility(unreal.ComponentMobility.MOVABLE)
    c.set_editor_property("intensity", float(JOB["sun"]["lux"]))
    c.set_editor_property("light_source_angle", 0.5357)
    c.set_editor_property("atmosphere_sun_light", True)
    c.set_editor_property("cast_shadows", True)
    c.set_editor_property("use_temperature", False)
    c.set_light_color(unreal.LinearColor(1, 1, 1, 1), True)
    spawn(unreal.SkyAtmosphere, None, None, "SkyAtmosphere")
    sl = spawn(unreal.SkyLight, None, None, "SkyLight")
    c = sl.light_component
    c.set_mobility(unreal.ComponentMobility.MOVABLE)
    c.set_editor_property("real_time_capture", True)
    c.set_editor_property("source_type", unreal.SkyLightSourceType.SLS_CAPTURED_SCENE)
    c.set_editor_property("intensity", 1.0)


def camera(w, h):
    loc, aim = ue(JOB["eye"]), ue(JOB["target"])
    d = aim - loc
    rot = unreal.Rotator(roll=0.0, pitch=math.degrees(math.atan2(d.z, math.hypot(d.x, d.y))), yaw=math.degrees(math.atan2(d.y, d.x)))
    a = spawn(unreal.CineCameraActor, loc, rot, "Camera")
    c = a.get_cine_camera_component()
    fb = c.get_editor_property("filmback")
    sensor_w = 36.0
    sensor_h = sensor_w * h / w
    fb.set_editor_property("sensor_width", sensor_w)
    fb.set_editor_property("sensor_height", sensor_h)
    c.set_editor_property("filmback", fb)
    c.set_editor_property("current_focal_length", sensor_h / (2.0 * math.tan(math.radians(JOB["vfov"]) / 2.0)))
    fs = c.get_editor_property("focus_settings")
    fs.set_editor_property("focus_method", unreal.CameraFocusMethod.DISABLE)
    c.set_editor_property("focus_settings", fs)
    c.set_editor_property("constrain_aspect_ratio", False)
    log(f"camera loc={loc} rot={rot} focal={c.get_editor_property('current_focal_length'):.3f}")
    return a


def set_pp(s, name, value):
    s.set_editor_property(name, value)
    try:
        s.set_editor_property("override_" + name, True)
    except Exception:
        pass


def post(preset):
    """Former IR adapter setup_post (manual exposure branch) with both the Lumen and the path tracer settings."""
    ppv = spawn(unreal.PostProcessVolume, None, None, "PostProcess")
    ppv.set_editor_property("unbound", True)
    ppv.set_editor_property("priority", 10.0)
    s = ppv.get_editor_property("settings")
    set_pp(s, "auto_exposure_method", unreal.AutoExposureMethod.AEM_MANUAL)
    set_pp(s, "auto_exposure_apply_physical_camera_exposure", False)
    set_pp(s, "auto_exposure_bias", -float(JOB["ev100"]))
    p = preset["post"]
    tm, cg = p["tonemapper"], p["colorGrading"]
    for k, v in (("film_slope", tm["slope"]), ("film_toe", tm["toe"]), ("film_shoulder", tm["shoulder"]),
                 ("film_black_clip", tm["blackClip"]), ("film_white_clip", tm["whiteClip"]), ("tone_curve_amount", tm["toneCurveAmount"]),
                 ("expand_gamut", cg["expandGamut"]), ("blue_correction", cg["blueCorrection"]),
                 ("white_temp", cg.get("whiteTemp", 6500.0)), ("white_tint", cg.get("whiteTint", 0.0)),
                 ("bloom_intensity", p["bloom"]["intensity"]), ("bloom_threshold", p["bloom"]["threshold"]),
                 ("bloom_size_scale", p["bloom"]["sizeScale"]), ("vignette_intensity", p["vignette"]),
                 ("lens_flare_intensity", p.get("lensFlare", 0.0)), ("film_grain_intensity", p.get("filmGrain", 0.0)),
                 ("scene_fringe_intensity", p.get("chromaticAberration", 0.0))):
        set_pp(s, k, float(v))
    for k, v in (("color_saturation", cg.get("saturation", 1.0)), ("color_contrast", cg.get("contrast", 1.0)),
                 ("color_gain_shadows", cg.get("shadowsGain", 1.0)), ("color_gain_highlights", cg.get("highlightsGain", 1.0))):
        set_pp(s, k, unreal.Vector4(1.0, 1.0, 1.0, float(v)))
    for i, (size, tint) in enumerate(zip(p["bloom"]["sizes"], p["bloom"]["tints"]), start=1):
        set_pp(s, f"bloom{i}_size", float(size))
        set_pp(s, f"bloom{i}_tint", unreal.LinearColor(tint, tint, tint, 1))
    set_pp(s, "bloom_method", unreal.BloomMethod.BM_SOG)
    set_pp(s, "motion_blur_amount", 0.0)
    # Lumen (ignored by the path tracer)
    set_pp(s, "dynamic_global_illumination_method", unreal.DynamicGlobalIlluminationMethod.LUMEN)
    set_pp(s, "reflection_method", unreal.ReflectionMethod.LUMEN)
    for k, v in preset["ue"]["postProcessVolume"].items():
        if isinstance(v, str):
            v = getattr(unreal.LumenRayLightingModeOverride, v)
        try:
            set_pp(s, k, v)
        except Exception as e:
            log(f"warn: pp {k}: {e}")
    # Path tracer (ignored by Lumen)
    pt = preset["ue"].get("pathTracer", {})
    set_pp(s, "path_tracing_max_bounces", int(pt.get("maxBounces", preset["native"]["maxBounces"])))
    set_pp(s, "path_tracing_samples_per_pixel", int(pt.get("spp", 512)))
    set_pp(s, "path_tracing_max_path_intensity", float(pt.get("maxPathIntensity", 64.0)))
    set_pp(s, "path_tracing_enable_denoiser", bool(pt.get("denoise", True)))
    set_pp(s, "path_tracing_include_emissive", True)
    ppv.set_editor_property("settings", s)


def make_sequence(cam):
    name = "Seq_" + JOB["name"].replace("-", "_")
    path = f"{ROOT}/{name}"
    if EAL.does_asset_exist(path):
        EAL.delete_asset(path)
    seq = AT.create_asset(name, ROOT, unreal.LevelSequence, unreal.LevelSequenceFactoryNew())
    seq.set_display_rate(unreal.FrameRate(24, 1))
    seq.set_playback_start(0)
    seq.set_playback_end(1)
    binding = seq.add_spawnable_from_instance(cam)
    section = seq.add_track(unreal.MovieSceneCameraCutTrack).add_section()
    section.set_range(0, 1)
    section.set_camera_binding_id(unreal.MovieSceneSequenceExtensions.get_binding_id(seq, binding))
    EAS.destroy_actor(cam)
    EAL.save_loaded_asset(seq)
    return seq


def begin_level(job):
    map_path = f"{ROOT}/Map_{job['name'].replace('-', '_')}_{int(time.time())}"
    if not LES.new_level(map_path):
        raise RuntimeError(f"new_level {map_path} failed")
    plane = EAL.load_asset("/Engine/BasicShapes/Plane")
    static_mesh_actor(plane, unreal.Vector(0, 0, 0), unreal.Rotator(0, 0, 0), unreal.Vector(2000, 2000, 1), ground_material(), "ground")
    return map_path


def finish_level(job, preset, imported):
    lift = unreal.Vector(0, 0, float(job["lift"]) * 100.0)
    roots = [a for a in imported if a.get_attach_parent_actor() is None]
    for a in roots:
        a.set_actor_location(a.get_actor_location() + lift, False, False)
    log(f"{len(imported)} imported actors, {len(roots)} roots raised {job['lift']} m")
    lights()
    post(preset)
    seq = make_sequence(camera(job["width"], job["height"]))
    if not LES.save_current_level():
        raise RuntimeError("save level failed")
    unreal.EditorLoadingAndSavingUtils.save_dirty_packages(True, True)
    return seq


def configure(j, job, preset, method):
    cfg = j.get_configuration()
    out = cfg.find_or_add_setting_by_class(unreal.MoviePipelineOutputSetting)
    out.set_editor_property("output_directory", unreal.DirectoryPath(os.path.join(job["outRoot"], method)))
    out.set_editor_property("file_name_format", job["name"])
    out.set_editor_property("output_resolution", unreal.IntPoint(job["width"], job["height"]))
    out.set_editor_property("zero_pad_frame_numbers", 0)
    out.set_editor_property("flush_disk_writes_per_shot", True)
    out.set_editor_property("override_existing_output", True)
    pt = method == "pathtracer"
    cfg.find_or_add_setting_by_class(unreal.MoviePipelineDeferredPass_PathTracer if pt else unreal.MoviePipelineDeferredPassBase)
    cfg.find_or_add_setting_by_class(unreal.MoviePipelineImageSequenceOutput_PNG)
    aa = cfg.find_or_add_setting_by_class(unreal.MoviePipelineAntiAliasingSetting)
    aa.set_editor_property("override_anti_aliasing", True)
    mrq = preset["ue"]["mrq"]
    if pt:
        aa.set_editor_property("spatial_sample_count", int(preset["ue"]["pathTracer"]["spp"]))
        aa.set_editor_property("temporal_sample_count", 1)
        aa.set_editor_property("anti_aliasing_method", unreal.AntiAliasingMethod.AAM_NONE)
        aa.set_editor_property("engine_warm_up_count", 8)
        aa.set_editor_property("render_warm_up_count", 8)
    else:
        aa.set_editor_property("spatial_sample_count", int(mrq["spatialSamples"]))
        aa.set_editor_property("temporal_sample_count", int(mrq["temporalSamples"]))
        aa.set_editor_property("anti_aliasing_method", getattr(unreal.AntiAliasingMethod, "AAM_" + mrq["antiAliasing"]))
        aa.set_editor_property("engine_warm_up_count", int(mrq["warmupFrames"]))
        aa.set_editor_property("render_warm_up_count", int(mrq["warmupFrames"]))
    aa.set_editor_property("use_camera_cut_for_warm_up", False)
    cv = cfg.find_or_add_setting_by_class(unreal.MoviePipelineConsoleVariableSetting)
    for k, v in preset["ue"]["cvars"].items():
        if v is not None:
            cv.add_or_update_console_variable(k, float(v))
    game = cfg.find_or_add_setting_by_class(unreal.MoviePipelineGameOverrideSetting)
    game.set_editor_property("cinematic_quality_settings", True)
    game.set_editor_property("texture_streaming", unreal.MoviePipelineTextureStreamingMethod.DISABLED)
    game.set_editor_property("use_lod_zero", True)
    game.set_editor_property("disable_hlods", True)
    game.set_editor_property("flush_streaming_managers", True)


class Runner:
    def run(self, job):
        self.job, self.t0, self.timings = job, time.time(), {}
        self.preset = json.load(open(os.path.join(job["presetsDir"], "cinematic.json")))
        self.map_path = begin_level(job)
        self.before = set(a.get_name() for a in EAS.get_all_level_actors())
        start_import(job["glb"])
        self.idle = 0
        self.tick = unreal.register_slate_post_tick_callback(self.poll)

    def poll(self, dt):
        # Wait for the import, then a few more ticks for its actors to settle.
        try:
            self.idle = 0 if importing() else self.idle + 1
            if self.idle < 10:
                return
            unreal.unregister_slate_post_tick_callback(self.tick)
            imported = [a for a in EAS.get_all_level_actors() if a.get_name() not in self.before]
            seq = finish_level(self.job, self.preset, imported)
            self.timings["build_s"] = round(time.time() - self.t0, 2)
            log(f"built {self.map_path} in {self.timings['build_s']}s ({len(EAS.get_all_level_actors())} actors)")
            self.render(seq)
        except Exception:
            log("FAILED\n" + traceback.format_exc())
            if self.job.get("quit", True):
                unreal.SystemLibrary.quit_editor()

    def render(self, seq):
        qs = unreal.get_editor_subsystem(unreal.MoviePipelineQueueSubsystem)
        q = qs.get_queue()
        q.delete_all_jobs()
        for m in self.job["methods"]:
            j = q.allocate_new_job(unreal.MoviePipelineExecutorJob)
            j.set_editor_property("job_name", m)
            j.set_editor_property("map", unreal.SoftObjectPath(self.map_path))
            j.set_editor_property("sequence", unreal.SoftObjectPath(seq.get_path_name()))
            configure(j, self.job, self.preset, m)
        self.ex = unreal.MoviePipelinePIEExecutor()
        self.ex.on_executor_finished_delegate.add_callable_unique(self.done)
        self.ex.on_individual_job_work_finished_delegate.add_callable_unique(self.job_done)
        self.t_last = time.time()
        qs.render_queue_with_executor_instance(self.ex)
        log("render started")

    def job_done(self, params):
        now = time.time()
        try:
            name = params.job.get_editor_property("job_name")
        except Exception:
            name = f"job{len(self.timings)}"
        self.timings[f"{name}_render_s"] = round(now - self.t_last, 2)
        self.t_last = now
        log(f"job {name} done in {self.timings[f'{name}_render_s']}s")

    def done(self, executor, success):
        self.timings["total_s"] = round(time.time() - self.t0, 2)
        os.makedirs(self.job["work"], exist_ok=True)
        json.dump(self.timings, open(os.path.join(self.job["work"], "timings.json"), "w"), indent=1)
        log(f"all done success={success} timings={json.dumps(self.timings)}")
        if self.job.get("quit", True):
            unreal.SystemLibrary.quit_editor()


_runner = Runner()
try:
    _runner.run(JOB)
except Exception:
    log("FAILED\n" + traceback.format_exc())
    if JOB.get("quit", True):
        unreal.SystemLibrary.quit_editor()
