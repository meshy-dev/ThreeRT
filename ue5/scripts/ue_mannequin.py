"""UE 5.8 ground truth for scenes/mannequin.ts, built natively (no scene IR) and rendered with Movie Render Queue.

    THREERT_JOB=/abs/job.json UnrealEditor <scratch>.uproject -ExecCmds="py ue5/scripts/ue_mannequin.py" -unattended ...
job.json: {"glb": "/abs/UAL1_Standard.glb", "outRoot": "/abs/out/ue", "work": "/abs/out/ue-mannequin",
           "methods": ["lumen", "pathtracer"], "presetsDir": "/abs/presets", "width": 1280, "height": 720, "quit": true}
Driven by ue5/mannequin.ts (which creates the scratch project).

Scene (mirror of scenes/mannequin.ts; keep in sync):
  - UAL1_Standard.glb imported with Interchange (skeletal mesh + skeleton + 43 AnimSequences). Four SkeletalMeshActors in
    "Use Animation Asset" mode: clip, play rate 0, not playing, position = clip time (the evaluated pose MRQ renders).
  - 2 km ground plane (linear albedo 0.35, roughness 0.6), three blocks (albedo 0.6, roughness 0.5), mannequin slots
    M_Main / M_Joints with the GLB's base colour factors (roughness 0.5, non-metal); one simple master material.
  - Sun: 80000 lux at the top of the atmosphere (Atmosphere Sun Light), source angle 0.5357 deg, from setSunAngles(38, -35).
    SkyAtmosphere defaults; SkyLight real-time capture, intensity 1, lower hemisphere black (UE default). No fog.
  - CineCamera 16:9 filmback, focal length from the 38 deg vertical fov, no DOF, at (0.3, 1.45, 5.6) looking at (0, 0.85, 0).
  - Unbound PostProcessVolume: manual exposure EV100 14.6 and the `cinematic` preset post / Lumen settings, exactly like
    the former IR adapter (git show 15d7e35:ue5/scripts/threert_ue.py, setup_post / configure_job).
Coordinates: ours are Y-up metres, UE = 100 * (x, z, y) cm (Interchange glTF convention, GLTF::ConvertVec3); a rotation
of theta about our +Y is UE yaw -theta (handedness flip).
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
GLB_DIR = f"{ROOT}/UAL"

# --- scenes/mannequin.ts ---------------------------------------------------------------------------------------------
POSES = [  # clip, time (s), x, z, yaw (deg about +Y)
    ("idle", "Idle_Loop", 0.8, -1.65, 0.2, 20.0),
    ("walk", "Walk_Loop", 0.45, -0.55, -0.1, 35.0),
    ("jog", "Jog_Fwd_Loop", 0.3, 0.55, 0.15, -30.0),
    ("dance", "Dance_Loop", 0.5, 1.65, -0.15, -15.0),
]
BLOCKS = [("block_a", 1, 1, 1, -3.4, -2.2, 0.3), ("block_b", 2, 0.5, 1, 3.1, -2.6, -0.2), ("block_c", 0.6, 2.2, 0.6, 2.4, -1.4, 0.0)]
SUN = dict(lux=80000.0, elevation=38.0, azimuth=-35.0, angle=0.5357)
CAMERA = dict(pos=(0.3, 1.45, 5.6), target=(0.0, 0.85, 0.0), vfov=38.0, ev100=14.6)
MATERIALS = {  # linear base colour, roughness
    "MI_Ground": ((0.35, 0.35, 0.35), 0.6),
    "MI_Block": ((0.6, 0.6, 0.6), 0.5),
    "MI_Main": ((0.7990983128547668, 0.40197819471359253, 0.042311523109674454), 0.5),
    "MI_Joints": ((0.40197500586509705, 0.132868230342865, 0.7083759903907776), 0.5),
}


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


# --- assets ----------------------------------------------------------------------------------------------------------


def import_glb(path):
    """Interchange import (default pipeline: skeletal mesh, skeleton, physics asset, materials, animations) once."""
    def find():
        assets = [EAL.load_asset(p) for p in EAL.list_assets(GLB_DIR, recursive=True)] if EAL.does_directory_exist(GLB_DIR) else []
        return assets
    assets = find()
    if not any(isinstance(a, unreal.SkeletalMesh) for a in assets):
        task = unreal.AssetImportTask()
        task.set_editor_property("filename", path)
        task.set_editor_property("destination_path", GLB_DIR)
        task.set_editor_property("automated", True)
        task.set_editor_property("replace_existing", True)
        task.set_editor_property("save", True)
        t0 = time.time()
        AT.import_asset_tasks([task])
        assets = find()
        log(f"imported {os.path.basename(path)} in {time.time() - t0:.1f}s: " + ", ".join(f"{a.get_class().get_name()}:{a.get_name()}" for a in assets))
    meshes = [a for a in assets if isinstance(a, unreal.SkeletalMesh)]
    anims = {a.get_name(): a for a in assets if isinstance(a, unreal.AnimSequence)}
    if not meshes:
        raise RuntimeError(f"no skeletal mesh imported from {path}")
    return meshes[0], anims


def find_anim(anims, clip, prefix):
    """Interchange names the AnimSequences <skeletal mesh name><glTF animation name>."""
    for name in (clip, prefix + clip, prefix + "_" + clip):
        if name in anims:
            return anims[name]
    hits = [n for n in anims if n.endswith("_" + clip)]
    if len(hits) != 1:
        raise RuntimeError(f"clip {clip}: candidates {hits} in {sorted(anims)}")
    return anims[hits[0]]


def master_material():
    path = f"{ROOT}/M_ThreeRT_Lit"
    if EAL.does_asset_exist(path):
        return EAL.load_asset(path)
    mat = AT.create_asset("M_ThreeRT_Lit", ROOT, unreal.Material, unreal.MaterialFactoryNew())
    mat.set_editor_property("used_with_skeletal_mesh", True)
    for i, (name, prop, default) in enumerate((("BaseColor", unreal.MaterialProperty.MP_BASE_COLOR, None), ("Roughness", unreal.MaterialProperty.MP_ROUGHNESS, 0.5),
                                               ("Metallic", unreal.MaterialProperty.MP_METALLIC, 0.0), ("Specular", unreal.MaterialProperty.MP_SPECULAR, 0.5))):
        if default is None:
            e = MEL.create_material_expression(mat, unreal.MaterialExpressionVectorParameter, -400, 0)
            e.set_editor_property("default_value", unreal.LinearColor(0.5, 0.5, 0.5, 1.0))
        else:
            e = MEL.create_material_expression(mat, unreal.MaterialExpressionScalarParameter, -400, 200 * i)
            e.set_editor_property("default_value", default)
        e.set_editor_property("parameter_name", name)
        MEL.connect_material_property(e, "", prop)
    MEL.recompile_material(mat)
    EAL.save_loaded_asset(mat)
    return mat


def material_instances(parent):
    out = {}
    for name, (rgb, rough) in MATERIALS.items():
        path = f"{ROOT}/{name}"
        if EAL.does_asset_exist(path):
            EAL.delete_asset(path)
        mi = AT.create_asset(name, ROOT, unreal.MaterialInstanceConstant, unreal.MaterialInstanceConstantFactoryNew())
        MEL.set_material_instance_parent(mi, parent)
        MEL.set_material_instance_vector_parameter_value(mi, "BaseColor", unreal.LinearColor(rgb[0], rgb[1], rgb[2], 1.0))
        MEL.set_material_instance_scalar_parameter_value(mi, "Roughness", rough)
        MEL.set_material_instance_scalar_parameter_value(mi, "Metallic", 0.0)
        MEL.set_material_instance_scalar_parameter_value(mi, "Specular", 0.5)
        MEL.update_material_instance(mi)
        EAL.save_loaded_asset(mi)
        out[name] = mi
    return out


# --- level -----------------------------------------------------------------------------------------------------------


def static_mesh_actor(mesh, loc, rot, scale, mat, label):
    a = spawn(unreal.StaticMeshActor, loc, rot, label)
    c = a.static_mesh_component
    c.set_mobility(unreal.ComponentMobility.MOVABLE)
    c.set_static_mesh(mesh)
    a.set_actor_scale3d(scale)
    for i in range(c.get_num_materials()):
        c.set_material(i, mat)
    return a


def mannequins(mesh, anims, mats):
    slots = [str(m.get_editor_property("material_slot_name")) for m in mesh.get_editor_property("materials")]
    log(f"skeletal mesh {mesh.get_path_name()} slots={slots} bounds={mesh.get_bounds().box_extent}")
    for pid, clip, t, x, z, yaw in POSES:
        anim = find_anim(anims, clip, mesh.get_name())
        a = spawn(unreal.SkeletalMeshActor, ue((x, 0.0, z)), unreal.Rotator(roll=0.0, pitch=0.0, yaw=-yaw), pid)
        c = a.get_editor_property("skeletal_mesh_component")
        c.set_mobility(unreal.ComponentMobility.MOVABLE)
        try:
            c.set_skeletal_mesh_asset(mesh)
        except AttributeError:
            c.set_skinned_asset_and_update(mesh)
        for i, s in enumerate(slots):
            c.set_material(i, mats["MI_Joints"] if "joint" in s.lower() else mats["MI_Main"])
        c.set_animation_mode(unreal.AnimationMode.ANIMATION_SINGLE_NODE)
        c.override_animation_data(anim, True, False, float(t), 0.0)
        c.set_editor_property("visibility_based_anim_tick_option", unreal.VisibilityBasedAnimTickOption.ALWAYS_TICK_POSE_AND_REFRESH_BONES)
        ad = c.get_editor_property("animation_data")
        log(f"{pid}: {anim.get_name()} len={anim.get_play_length():.3f}s pos={ad.get_editor_property('saved_position')} "
            f"rate={ad.get_editor_property('saved_play_rate')} playing={ad.get_editor_property('saved_playing')} at {a.get_actor_location()} yaw={-yaw}")


def sun_rotation():
    el, az = math.radians(SUN["elevation"]), math.radians(SUN["azimuth"])
    to_sun = (math.sin(az) * math.cos(el), math.sin(el), math.cos(az) * math.cos(el))  # DirectionalLight.setSunAngles
    d = ue([-c for c in to_sun], 1.0)  # direction the light travels, UE axes
    return unreal.Rotator(roll=0.0, pitch=math.degrees(math.asin(d.z)), yaw=math.degrees(math.atan2(d.y, d.x)))


def lights():
    rot = sun_rotation()
    a = spawn(unreal.DirectionalLight, unreal.Vector(0, 0, 1000), rot, "Sun")
    c = a.light_component
    c.set_mobility(unreal.ComponentMobility.MOVABLE)
    c.set_editor_property("intensity", SUN["lux"])
    c.set_editor_property("light_source_angle", SUN["angle"])
    c.set_editor_property("atmosphere_sun_light", True)
    c.set_editor_property("cast_shadows", True)
    c.set_editor_property("use_temperature", False)
    c.set_light_color(unreal.LinearColor(1, 1, 1, 1), True)
    log(f"sun rotation {a.get_actor_rotation()} forward={a.get_actor_forward_vector()}")
    spawn(unreal.SkyAtmosphere, None, None, "SkyAtmosphere")
    sl = spawn(unreal.SkyLight, None, None, "SkyLight")
    c = sl.light_component
    c.set_mobility(unreal.ComponentMobility.MOVABLE)
    c.set_editor_property("real_time_capture", True)
    c.set_editor_property("source_type", unreal.SkyLightSourceType.SLS_CAPTURED_SCENE)
    c.set_editor_property("intensity", 1.0)
    log(f"sky light lower_hemisphere_is_black={c.get_editor_property('lower_hemisphere_is_black')}")


def camera(w, h, portrait=None):
    """portrait (scenes/mannequin-portrait.ts): {"target", "vfov", "focus" (m), "fstop"}: aimed at the dancer, manual focus."""
    pos, tgt = CAMERA["pos"], (portrait["target"] if portrait else CAMERA["target"])
    vfov = portrait["vfov"] if portrait else CAMERA["vfov"]
    loc, aim = ue(pos), ue(tgt)
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
    c.set_editor_property("current_focal_length", sensor_h / (2.0 * math.tan(math.radians(vfov) / 2.0)))
    fs = c.get_editor_property("focus_settings")
    if portrait:
        fs.set_editor_property("focus_method", unreal.CameraFocusMethod.MANUAL)
        fs.set_editor_property("manual_focus_distance", portrait["focus"] * 100.0)
        c.set_editor_property("current_aperture", portrait["fstop"])
    else:
        fs.set_editor_property("focus_method", unreal.CameraFocusMethod.DISABLE)
    c.set_editor_property("focus_settings", fs)
    c.set_editor_property("constrain_aspect_ratio", False)
    log(f"camera loc={loc} rot={rot} focal={c.get_editor_property('current_focal_length'):.3f} hfov={c.get_editor_property('field_of_view'):.3f}")
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
    set_pp(s, "auto_exposure_bias", -float(CAMERA["ev100"]))
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
    path = f"{ROOT}/Seq_Mannequin"
    if EAL.does_asset_exist(path):
        EAL.delete_asset(path)
    seq = AT.create_asset("Seq_Mannequin", ROOT, unreal.LevelSequence, unreal.LevelSequenceFactoryNew())
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


def build_level(job, preset):
    mesh, anims = import_glb(job["glb"])
    log(f"{len(anims)} anim sequences: {sorted(anims)[:8]}...")
    mats = material_instances(master_material())
    map_path = f"{ROOT}/Map_Mannequin_{int(time.time())}"
    if not LES.new_level(map_path):
        raise RuntimeError(f"new_level {map_path} failed")
    plane, cube = EAL.load_asset("/Engine/BasicShapes/Plane"), EAL.load_asset("/Engine/BasicShapes/Cube")
    static_mesh_actor(plane, unreal.Vector(0, 0, 0), unreal.Rotator(0, 0, 0), unreal.Vector(2000, 2000, 1), mats["MI_Ground"], "ground")
    for name, w, h, d, x, z, yaw in BLOCKS:
        static_mesh_actor(cube, ue((x, h / 2, z)), unreal.Rotator(roll=0.0, pitch=0.0, yaw=-math.degrees(yaw)), unreal.Vector(w, d, h), mats["MI_Block"], name)
    mannequins(mesh, anims, mats)
    lights()
    post(preset)
    seq = make_sequence(camera(job["width"], job["height"], job.get("portrait")))
    if not LES.save_current_level():
        raise RuntimeError("save level failed")
    unreal.EditorLoadingAndSavingUtils.save_dirty_packages(True, True)
    return map_path, seq


def configure(j, job, preset, method):
    cfg = j.get_configuration()
    out = cfg.find_or_add_setting_by_class(unreal.MoviePipelineOutputSetting)
    out.set_editor_property("output_directory", unreal.DirectoryPath(os.path.join(job["outRoot"], method)))
    out.set_editor_property("file_name_format", "mannequin-portrait" if job.get("portrait") else "mannequin")
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
        preset = json.load(open(os.path.join(job["presetsDir"], "cinematic.json")))
        map_path, seq = build_level(job, preset)
        self.timings["build_s"] = round(time.time() - self.t0, 2)
        log(f"built {map_path} in {self.timings['build_s']}s ({len(EAS.get_all_level_actors())} actors)")
        qs = unreal.get_editor_subsystem(unreal.MoviePipelineQueueSubsystem)
        q = qs.get_queue()
        q.delete_all_jobs()
        for m in job["methods"]:
            j = q.allocate_new_job(unreal.MoviePipelineExecutorJob)
            j.set_editor_property("job_name", m)
            j.set_editor_property("map", unreal.SoftObjectPath(map_path))
            j.set_editor_property("sequence", unreal.SoftObjectPath(seq.get_path_name()))
            configure(j, job, preset, m)
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
_job = json.load(open(os.environ["THREERT_JOB"]))
try:
    _runner.run(_job)
except Exception:
    log("FAILED\n" + traceback.format_exc())
    if _job.get("quit", True):
        unreal.SystemLibrary.quit_editor()
