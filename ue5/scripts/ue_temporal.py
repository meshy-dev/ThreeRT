"""UE real-time motion references for tools/bench-temporal.ts, rendered with Movie Render Queue from the actual Kabukicho
level (scratch copy of the project; driven by ue5/temporal.ts).

One Level Sequence at 60 fps, rendered with 1 temporal sample per frame (TSR, no per-frame warm-up), so TSR / Lumen /
eye-adaptation history evolves exactly as in real time; every frame is rendered and the shots are kept:
    [0, LEAD)                       start pose (bench v1 pose 0): history, exposure and the rain fade settle
    LEAD + k, k = 0..59             camera move, pose(k) of tools/bench-temporal.ts  -> move20, move40, move59
    LEAD + 59 + 30                  stopped 0.5 s at pose 59                          -> stopped-0.5s
    H20 - 1, H40 - 1, CUT - 1       settled (3 s held) at pose 59, 20, 40   -> settled59, settled20, settled40: UE's own
                                    converged frames, so its temporal error can be measured like ours (motion frame vs
                                    the same renderer's converged frame, which cancels the port's differences)
    CUT                             back to the start pose, held
    EVENT = CUT + LEAD              closing time ("lights"): every other group of our street's group order goes dark,
                                    staggered like games/kabukicho/kabukicho.game.ts' frame script (port of
                                    AKabukichoDirector::Tick): delay U(0, 2.5) s, flicker U(0.08, 0.3) s in steps of
                                    U(0.03, 0.11) s at power U(0.6, 1) (p 0.3) or 0.02, then off
    EVENT + 0.3 / 1 / 2 / 4 / 10 s  -> closing-0.3s, closing-1s, closing-2s, closing-4s, closing-settled
With "move": "strafe" (tools/bench-ghosting.ts' strafe, at 60 fps like its lockstep camera steps), instead:
    [0, LEAD]                       start pose (view v1)                              -> start (3 s held)
    LEAD + k, k = 1..24             3 m to the right in 0.4 s (7.5 m/s)               -> strafe-mid (k = 12)
    + 30                            held
    BACK + k, k = 1..24             back to the start pose                            -> strafe-return+0.1s / +0.5s / +2s
                                                                                         (6 / 30 / 120 frames after)
    then the midpoint pose held 3 s                                                   -> settled-strafe-mid
No closing time in this sequence.
The Director (AKabukichoDirector) runs in the MRQ PIE world (BeginPlay creates the sign MIDs, Tick drives rain /
wetness / fog), but it has no closing-time function and SetGroupState is not exposed, so a Python per-frame callback
(slate post-tick) drives the groups' "Power" MID parameters and light intensities in the PIE world, keyed on the
sequence time (the callback runs after frame f is rendered, so it sets the state of frame f + 1). Like the bench,
rain is switched off at the start (ToggleRain; the street stays wet) and crowd / rain particles are hidden.

    THREERT_TEMPORAL=/abs/job.json UnrealEditor Kabukicho.uproject -ExecCmds="py ue5/scripts/ue_temporal.py" -unattended ...
job.json: {"map": "/Game/Kabukicho/Maps/L_Kabukicho", "frames": "/abs/tmp frame dir", "refs": "/abs/out/bench-temporal/refs-ue",
           "groups": [our group tags in order], "hide": ["KabukichoCrowd", "KabukichoRain"], "screenPercentage": 100,
           "seed": 1, "cvars": {...}, "keepFrames": false, "motionBlur": null | {"amount", "max", "targetFps"},
           "rain": false}
rain: true keeps the rain on (no ToggleRain); with "KabukichoRain" out of "hide" its falling streaks show (the rain
references of the particle pass, tools/rain-stats.ts).
motionBlur: an unbound post volume above the level's sets UE's motion blur (with r.MotionBlurQuality > 0 in cvars).
"""
import json
import math
import os
import random
import shutil
import time
import traceback

import unreal

EAL = unreal.EditorAssetLibrary
EAS = unreal.get_editor_subsystem(unreal.EditorActorSubsystem)
LES = unreal.get_editor_subsystem(unreal.LevelEditorSubsystem)
UES = unreal.get_editor_subsystem(unreal.UnrealEditorSubsystem)
AT = unreal.AssetToolsHelpers.get_asset_tools()
ROOT = "/Game/ThreeRTTemporal"
FPS = 60
N = 60  # bench-temporal N
LEAD = 180  # 3 s at the start pose before the move / before closing time (bench: sleep(3000))
MOVE0 = LEAD
STOP = MOVE0 + 59 + 30
HOLD = 3 * FPS
H20 = MOVE0 + 59 + HOLD  # pose 59 held until here, then pose 20, then pose 40
H40 = H20 + HOLD
CUT = H40 + HOLD
EVENT = CUT + LEAD
END = EVENT + 10 * FPS
SHOTS = {"move20": MOVE0 + 20, "move40": MOVE0 + 40, "move59": MOVE0 + 59, "stopped-0.5s": STOP,
         "closing-0.3s": EVENT + 18, "closing-1s": EVENT + 60, "closing-2s": EVENT + 120, "closing-4s": EVENT + 240,
         "closing-settled": EVENT + 600, "start": MOVE0 - 1, "settled59": H20 - 1, "settled20": H40 - 1, "settled40": CUT - 1}


def log(msg):
    unreal.log("THREERT: " + msg)


# --- tools/bench-temporal.ts pose(k, N) -> UE camera ---------------------------------------------------------------------

def to_ue(pos, fwd):
    # ours (x, y, z) m -> UE (x, z, y) cm; UE forward (cos yaw, sin yaw, 0.07) -> yaw = yaw, pitch = atan(0.07)
    loc = (pos[0] * 100.0, pos[2] * 100.0, pos[1] * 100.0)
    rot = (math.degrees(math.atan2(fwd[1], math.hypot(fwd[0], fwd[2]))), math.degrees(math.atan2(fwd[2], fwd[0])), 0.0)
    return loc, rot


def pose(k):
    f = k / N
    yaw = math.atan2(0.07, 0.995) - f * math.radians(12.0)
    return to_ue((-11.5 + 2.5 * f, 1.7, -1.8 + 0.3 * f), (math.cos(yaw), 0.07, math.sin(yaw)))


# --- tools/bench-ghosting.ts strafe: view v1 moved `side` metres to its right ---------------------------------------------

SWEEP = 24  # frames per sweep (0.4 s)


def strafe(side):
    yaw = math.atan2(0.07, 0.995)
    return to_ue((-11.5 - math.sin(yaw) * side, 1.7, -1.8 + math.cos(yaw) * side), (math.cos(yaw), 0.07, math.sin(yaw)))


OUT0 = LEAD  # last frame at the start pose; frames OUT0 + k step out
BACK0 = OUT0 + SWEEP + 30  # last frame held at 3 m; frames BACK0 + k step back
RET = BACK0 + SWEEP  # first frame back at the start pose
MID0 = RET + 121  # midpoint pose held from here


def strafe_at(frame):
    if frame <= OUT0 or RET <= frame < MID0:
        return strafe(0.0)
    if frame >= MID0:
        return strafe(1.5)
    if frame <= OUT0 + SWEEP:
        return strafe(3.0 * (frame - OUT0) / SWEEP)
    if frame <= BACK0:
        return strafe(3.0)
    return strafe(3.0 * (1.0 - (frame - BACK0) / SWEEP))


def use_strafe():
    """Switches the sequence, shots and per-frame driver to the strafe (no closing time)."""
    global pose_at, END, EVENT, SHOTS, STOP, H20, H40, CUT
    pose_at = strafe_at
    END = MID0 + HOLD - 1
    EVENT = END + 10 * FPS  # never
    STOP = H20 = H40 = CUT = OUT0
    SHOTS = {"start": OUT0, "strafe-mid": OUT0 + SWEEP // 2, "strafe-return+0.1s": RET + 6, "strafe-return+0.5s": RET + 30,
             "strafe-return+2s": RET + 120, "settled-strafe-mid": END}


def pose_at(frame):
    if frame < MOVE0 or frame >= CUT:
        return pose(0)
    if frame >= H40:
        return pose(40)
    if frame >= H20:
        return pose(20)
    return pose(min(frame - MOVE0, N - 1))


# --- closing-time schedule (kabukicho.game.ts frame(): many groups changing, not dark -> delay U(0, 2.5)) -------------

def closing_schedule(groups, seed):
    rng = random.Random(seed)
    sched = {}
    for g in groups[1::2]:
        delay = rng.uniform(0.0, 2.5)
        flicker = rng.uniform(0.08, 0.3)
        steps, t = [], delay
        while t < delay + flicker:
            steps.append((t, rng.uniform(0.6, 1.0) if rng.random() < 0.3 else 0.02))
            t += rng.uniform(0.03, 0.11)
        sched[g] = (delay, flicker, steps)
    return sched


def power_at(entry, tau):
    delay, flicker, steps = entry
    if tau < delay:
        return 1.0
    if tau >= delay + flicker:
        return 0.0
    p = steps[0][1]
    for t, v in steps:
        if t <= tau:
            p = v
    return p


# --- level + sequence ------------------------------------------------------------------------------------------------

def prepare_level(job):
    LES.load_level(job["map"])
    hidden = 0
    for a in EAS.get_all_level_actors():
        if a.get_class().get_name() in job.get("hide", []):
            a.set_actor_hidden_in_game(True)
            a.set_actor_tick_enabled(False)
            hidden += 1
    log(f"level {job['map']}: hid {hidden} actors")
    mb = job.get("motionBlur")
    if mb:
        ppv = EAS.spawn_actor_from_class(unreal.PostProcessVolume, unreal.Vector(0, 0, 0), unreal.Rotator(0, 0, 0))
        ppv.set_actor_label("ThreeRT_MotionBlur")
        ppv.set_editor_property("unbound", True)
        ppv.set_editor_property("priority", 1000.0)
        s = ppv.get_editor_property("settings")
        for k, v in (("motion_blur_amount", mb["amount"]), ("motion_blur_max", mb["max"]), ("motion_blur_target_fps", int(mb["targetFps"]))):
            s.set_editor_property(k, v)
            s.set_editor_property("override_" + k, True)
        ppv.set_editor_property("settings", s)
        log(f"motion blur {mb}")


def make_sequence(w, h):
    path = f"{ROOT}/Seq_Temporal"
    if EAL.does_asset_exist(path):
        EAL.delete_asset(path)
    seq = AT.create_asset("Seq_Temporal", ROOT, unreal.LevelSequence, unreal.LevelSequenceFactoryNew())
    seq.set_display_rate(unreal.FrameRate(FPS, 1))
    seq.set_playback_start(0)
    seq.set_playback_end(END + 1)
    loc, rot = pose(0)
    cam = EAS.spawn_actor_from_class(unreal.CineCameraActor, unreal.Vector(*loc), unreal.Rotator(roll=rot[2], pitch=rot[0], yaw=rot[1]))
    c = cam.get_cine_camera_component()
    fb = c.get_editor_property("filmback")
    sensor_w, sensor_h = 36.0, 36.0 * h / w
    fb.set_editor_property("sensor_width", sensor_w)
    fb.set_editor_property("sensor_height", sensor_h)
    c.set_editor_property("filmback", fb)
    c.set_editor_property("current_focal_length", sensor_h / (2.0 * math.tan(math.radians(55.0) / 2.0)))
    fs = c.get_editor_property("focus_settings")
    fs.set_editor_property("focus_method", unreal.CameraFocusMethod.DISABLE)
    c.set_editor_property("focus_settings", fs)
    c.set_editor_property("constrain_aspect_ratio", False)
    binding = seq.add_spawnable_from_instance(cam)
    cut = seq.add_track(unreal.MovieSceneCameraCutTrack).add_section()
    cut.set_range(0, END + 1)
    cut.set_camera_binding_id(unreal.MovieSceneSequenceExtensions.get_binding_id(seq, binding))
    # Constant keys on every frame where the pose changes: MRQ evaluates at sub-frame offsets (shutter), which then
    # still resolve to the frame's own pose.
    sec = binding.add_track(unreal.MovieScene3DTransformTrack).add_section()
    sec.set_range(0, END + 1)
    chans = {str(ch.channel_name): ch for ch in sec.get_all_channels()}
    keyed = 0
    prev = None
    for f in range(END + 1):
        p = pose_at(f)
        if p == prev:
            continue
        prev = p
        (x, y, z), (pitch, yaw, roll) = p
        for name, v in (("Location.X", x), ("Location.Y", y), ("Location.Z", z), ("Rotation.X", roll), ("Rotation.Y", pitch), ("Rotation.Z", yaw)):
            chans[name].add_key(unreal.FrameNumber(f), v, 0.0, unreal.MovieSceneTimeUnit.DISPLAY_RATE, unreal.MovieSceneKeyInterpolation.CONSTANT)
        keyed += 1
    for name in ("Scale.X", "Scale.Y", "Scale.Z"):
        chans[name].set_default(1.0)
    EAS.destroy_actor(cam)
    EAL.save_loaded_asset(seq)
    log(f"sequence {seq.get_path_name()}: {END + 1} frames at {FPS} fps, {keyed} camera keys, channels={sorted(chans)}")
    return seq


def configure(j, job):
    cfg = j.get_configuration()
    out = cfg.find_or_add_setting_by_class(unreal.MoviePipelineOutputSetting)
    out.set_editor_property("output_directory", unreal.DirectoryPath(job["frames"]))
    out.set_editor_property("file_name_format", "{frame_number}")
    out.set_editor_property("output_resolution", unreal.IntPoint(job.get("width", 1280), job.get("height", 720)))
    out.set_editor_property("zero_pad_frame_numbers", 4)
    out.set_editor_property("override_existing_output", True)
    cfg.find_or_add_setting_by_class(unreal.MoviePipelineDeferredPassBase)
    cfg.find_or_add_setting_by_class(unreal.MoviePipelineImageSequenceOutput_PNG)
    aa = cfg.find_or_add_setting_by_class(unreal.MoviePipelineAntiAliasingSetting)
    aa.set_editor_property("override_anti_aliasing", True)
    aa.set_editor_property("anti_aliasing_method", unreal.AntiAliasingMethod.AAM_TSR)
    aa.set_editor_property("spatial_sample_count", 1)
    aa.set_editor_property("temporal_sample_count", 1)
    # Warm-up only before frame 0 (start pose); the 3 s lead-in is rendered as real frames.
    aa.set_editor_property("engine_warm_up_count", 32)
    aa.set_editor_property("render_warm_up_count", 32)
    aa.set_editor_property("use_camera_cut_for_warm_up", False)
    camset = cfg.find_or_add_setting_by_class(unreal.MoviePipelineCameraSetting)
    camset.set_editor_property("shutter_timing", unreal.MoviePipelineShutterTiming.FRAME_OPEN)
    cv = cfg.find_or_add_setting_by_class(unreal.MoviePipelineConsoleVariableSetting)
    for k, v in job.get("cvars", {}).items():
        cv.add_or_update_console_variable(k, float(v))
    game = cfg.find_or_add_setting_by_class(unreal.MoviePipelineGameOverrideSetting)
    # Real-time scalability (the cvars pin sg.* = Epic), not MRQ's cinematic override.
    game.set_editor_property("cinematic_quality_settings", False)
    game.set_editor_property("texture_streaming", unreal.MoviePipelineTextureStreamingMethod.DISABLED)
    game.set_editor_property("use_lod_zero", False)
    game.set_editor_property("flush_streaming_managers", True)


# --- per-frame driver in the MRQ PIE world ---------------------------------------------------------------------------

class Driver:
    def __init__(self, job):
        self.job = job
        self.sched = closing_schedule(job["groups"], job.get("seed", 1))
        self.world = None
        self.handle = unreal.register_slate_post_tick_callback(self.tick)
        self.log_frames = {0, 1, 2, MOVE0 - 1, MOVE0, MOVE0 + 1, MOVE0 + 59, STOP, H20, H40, CUT, EVENT, EVENT + 1, EVENT + 18, END}

    def reset(self, world):
        self.world = world
        self.rain_toggled = False
        self.groups = None
        self.applied = {}
        self.last_frame = None

    def gather(self, world):
        groups = {}
        for g in self.sched:
            mids, lights = [], []
            for a in unreal.GameplayStatics.get_all_actors_with_tag(world, g):
                for comp in a.get_components_by_class(unreal.PrimitiveComponent):
                    for i in range(comp.get_num_materials()):
                        m = comp.get_material(i)
                        if isinstance(m, unreal.MaterialInstanceDynamic):
                            mids.append(m)
                for lc in a.get_components_by_class(unreal.LightComponent):
                    lights.append((lc, lc.get_editor_property("intensity")))
            groups[g] = (mids, lights)
        n_mid = sum(len(v[0]) for v in groups.values())
        n_light = sum(len(v[1]) for v in groups.values())
        log(f"closing groups: {len(groups)} ({n_mid} sign MIDs, {n_light} lights); first delays "
            + ", ".join(f"{g}:{self.sched[g][0]:.2f}s" for g in list(self.sched)[:6]))
        return groups

    def frame_of(self, world):
        actors = unreal.GameplayStatics.get_all_actors_of_class(world, unreal.LevelSequenceActor)
        for a in actors:
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
                self.reset(world)
            fr = self.frame_of(world)  # known once MRQ drives the sequence (after the world's BeginPlay)
            if fr is None:
                return
            if not self.rain_toggled and not self.job.get("rain"):
                d = unreal.GameplayStatics.get_actor_of_class(world, unreal.KabukichoDirector)
                if d is not None:
                    d.toggle_rain()  # bench: "rain" event first (rain off, the street stays wet)
                    log(f"PIE world {world.get_name()}: Director found, rain toggled off at frame {fr:.2f}")
                else:
                    log("no KabukichoDirector in the PIE world")
                self.rain_toggled = True
            sp = self.job.get("screenPercentage")
            if sp and abs(unreal.SystemLibrary.get_console_variable_float_value("r.ScreenPercentage") - sp) > 0.01:
                # AKabukichoDirector::BeginPlay applies its "Balanced" preset (r.ScreenPercentage 62).
                unreal.SystemLibrary.execute_console_command(world, f"r.ScreenPercentage {sp}")
                log(f"r.ScreenPercentage -> {sp}")
            f = int(math.floor(fr + 1e-3))
            if f != self.last_frame and f in self.log_frames:
                cm = unreal.GameplayStatics.get_player_camera_manager(world, 0)
                loc = cm.get_camera_location() if cm else None
                exp_loc, _ = pose_at(f)
                log(f"frame {fr:.3f} t={unreal.GameplayStatics.get_time_seconds(world):.3f} cam={loc} expected={tuple(round(v, 1) for v in exp_loc)}")
            self.last_frame = f
            if self.groups is None and f >= 60:
                self.groups = self.gather(world)
            if self.groups is None:
                return
            tau = (f + 1 - EVENT) / FPS  # state of the next rendered frame
            for g, (mids, lights) in self.groups.items():
                p = power_at(self.sched[g], tau) if tau >= 0 else 1.0
                if self.applied.get(g) == p:
                    continue
                self.applied[g] = p
                for m in mids:
                    m.set_scalar_parameter_value("Power", p)
                for lc, base in lights:
                    lc.set_intensity(base * p)
        except Exception:
            log("driver error\n" + traceback.format_exc())
            unreal.unregister_slate_post_tick_callback(self.handle)


class Runner:
    def run(self, job):
        self.job, self.t0 = job, time.time()
        prepare_level(job)
        seq = make_sequence(job.get("width", 1280), job.get("height", 720))
        LES.save_current_level()
        unreal.EditorLoadingAndSavingUtils.save_dirty_packages(True, True)
        if os.path.isdir(job["frames"]):
            shutil.rmtree(job["frames"])
        qs = unreal.get_editor_subsystem(unreal.MoviePipelineQueueSubsystem)
        q = qs.get_queue()
        q.delete_all_jobs()
        j = q.allocate_new_job(unreal.MoviePipelineExecutorJob)
        j.set_editor_property("job_name", "temporal")
        j.set_editor_property("map", unreal.SoftObjectPath(job["map"]))
        j.set_editor_property("sequence", unreal.SoftObjectPath(seq.get_path_name()))
        configure(j, job)
        self.driver = Driver(job)
        self.ex = unreal.MoviePipelinePIEExecutor()
        self.ex.on_executor_finished_delegate.add_callable_unique(self.done)
        self.t_render = time.time()
        qs.render_queue_with_executor_instance(self.ex)
        log(f"render started ({END + 1} frames)")

    def done(self, executor, success):
        render_s = time.time() - self.t_render
        job = self.job
        os.makedirs(job["refs"], exist_ok=True)
        kept = {}
        for name, f in SHOTS.items():
            src = os.path.join(job["frames"], f"{f:04d}.png")
            if os.path.exists(src):
                shutil.copyfile(src, os.path.join(job["refs"], f"{name}.png"))
                kept[name] = f
        n = len([x for x in os.listdir(job["frames"]) if x.endswith(".png")]) if os.path.isdir(job["frames"]) else 0
        manifest = {"method": "UE 5.8 MRQ, L_Kabukicho (scratch copy), TSR 1 temporal sample / frame at 60 fps, real-time scalability (sg Epic), "
                              "Lumen (project settings), histogram auto exposure, motion blur " + (f"on {job['motionBlur']}" if job.get("motionBlur") else "off") + ", hidden: " + ", ".join(job.get("hide", [])) + (", rain on" if job.get("rain") else ", rain off"),
                    "screenPercentage": job.get("screenPercentage"), "move": job.get("move", "v1"), "frames": {"lead": LEAD, "move0": MOVE0, "stop": STOP, "h20": H20, "h40": H40, "cut": CUT, "event": EVENT, "end": END},
                    "shots": kept, "rendered_frames": n, "render_s": round(render_s, 1), "total_s": round(time.time() - self.t0, 1), "seed": job.get("seed", 1),
                    "closing": {g: [round(d, 3), round(fl, 3)] for g, (d, fl, _) in self.driver.sched.items()}}
        json.dump(manifest, open(os.path.join(job["refs"], "manifest.json"), "w"), indent=1)
        if not job.get("keepFrames") and os.path.isdir(job["frames"]):
            shutil.rmtree(job["frames"])
        log(f"all done success={success} rendered {n} frames in {render_s:.0f}s, kept {sorted(kept)}")
        unreal.SystemLibrary.quit_editor()


_runner = Runner()
try:
    with open(os.environ["THREERT_TEMPORAL"]) as fh:
        _job = json.load(fh)
    if _job.get("move") == "strafe":
        use_strafe()
    _runner.run(_job)
except Exception:
    log("FAILED\n" + traceback.format_exc())
    unreal.SystemLibrary.quit_editor()
