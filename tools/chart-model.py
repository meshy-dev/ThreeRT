# A model of the live denoiser's change chart (renderer/shaders/live.wgsl, "The response of the histories"): the
# rules of `dtemporal`'s response on a grid of pixels fed synthetic one-sample noise, to set the chart's constants
# before a GPU is asked (the comment at CHART_A quotes its figures). Per kind of sample noise it prints
#   - with nothing changing: the mean history length, the share of pixels the chart signals on a frame, and the
#     histories' rms error (the cost of false signals: compare `--k 1e9`, the chart off);
#   - after the histories start again with the light unchanged (a surface uncovered): their mean length 4, 8, 16, 32
#     frames on (4, 8, 16, 32 if nothing signals) and the share of pixels signalled on a frame of those 32;
#   - the mean of the histories over the truth 30, 60, 120, 170 frames into a steady fall of the light by --ramp a
#     frame (what the drift is for: the chart's level follows a slow fall and stops seeing it);
#   - the mean of the histories over the truth 2, 4, 8, 16, 32 frames after the light is scaled by a factor.
# And, with --sd, the pooled average's standard deviation on white residuals (CHART_SD).
#
#   uv run --no-project --with numpy python tools/chart-model.py [--k 3.5] [--floor 0.04] [--gain 2] [--mature 4]
#                                                                 [--a 0.125] [--cap 24] [--sd] [--ramp 0.026]
#                                                                 [--drift 0.25] [--allow 2] [--allowup 4]
# (--drift 0: the chart alone.)
#
# Sample kinds (mean 1 before the change): steady (lognormal, sigma 0.2), noisy (exponential), rare0.1 / rare0.01 (one
# bright sample in 10 / 100, else dark), mix0.02 / mix0.1 (a steady half, the other half by one sample in 50 / 10).
import argparse
import numpy as np

ap = argparse.ArgumentParser()
ap.add_argument("--a", type=float, default=0.125)       # CHART_A
ap.add_argument("--sdk", type=float, default=0.063)     # CHART_SD
ap.add_argument("--k", type=float, default=3.5)         # CHART_K
ap.add_argument("--floor", type=float, default=0.04)    # CHART_FLOOR
ap.add_argument("--gain", type=float, default=2.0)      # CHART_GAIN
ap.add_argument("--mature", type=float, default=4.0)    # CHART_MATURE
ap.add_argument("--level", type=float, default=32.0)    # CHART_LEVEL
ap.add_argument("--channel", type=float, default=0.25)  # CHART_CHANNEL
ap.add_argument("--cap", type=float, default=24.0)      # the frame's bound (HISTORY_LIVE)
ap.add_argument("--drift", type=float, default=0.25)    # DRIFT_GAIN
ap.add_argument("--allow", type=float, default=2.0)     # DRIFT_ALLOW
ap.add_argument("--allowup", type=float, default=4.0)   # DRIFT_ALLOW_UP
ap.add_argument("--clip", type=float, default=7.0)      # DRIFT_CLIP
ap.add_argument("--ramp", type=float, default=0.026)    # the steady fall a frame (the Tokyo dusk's city: about that)
ap.add_argument("--first", type=float, default=1.0)     # 1: the first frame's share (against no history) is no part of the level
ap.add_argument("--sd", action="store_true")
o = ap.parse_args()
rng = np.random.default_rng(1)
N = 96
DARK = 1e-3


def taps(frame):  # `chart_tap`
    q = [(-2, -1), (1, -2), (2, 1), (-1, 2)]
    return [(-x, y) for x, y in q] if frame & 1 else q


def pool(frame, v):  # the pixel itself and the four around it
    s = v.copy()
    for dx, dy in taps(frame):
        s += np.roll(v, (dy, dx), (0, 1))
    return s / 5


def sample(kind, mean):
    if kind == "steady":
        return mean * rng.lognormal(-0.02, 0.2, (N, N))
    if kind == "noisy":
        return mean * rng.exponential(1.0, (N, N))
    if kind.startswith("rare"):
        p = float(kind[4:])
        return mean / p * (rng.random((N, N)) < p)
    p = float(kind[3:])
    return 0.5 * mean + 0.5 * mean / p * (rng.random((N, N)) < p)


def run(kind, factor, frames=340, edit=300, restart=False, ramp=0.0):
    h = np.zeros((N, N))
    count = np.zeros((N, N))
    level = np.zeros((N, N))
    e = np.zeros((N, N))      # the residual average, as stored
    rms = np.zeros((N, N))    # the root of the residuals' mean square, as stored
    dr = np.zeros((N, N))     # the drift: the pooled relative residual, as stored
    m1 = np.zeros((N, N))     # the samples' mean and mean square (`diffuse_mom`)
    m2 = np.zeros((N, N))
    mean = np.ones((N, N))
    out = []
    for t in range(frames):
        if t == edit:
            mean = mean * factor
            if restart:  # the histories start again (a surface uncovered): nothing is known there
                h[:] = 0
                count[:] = 0
                level[:] = 0
                e[:] = 0
                rms[:] = 0
                dr[:] = 0
                m1[:] = 0
                m2[:] = 0
        if t > edit and ramp:
            mean = mean * (1 - ramp)
        s = sample(kind, mean)
        pooled, spread, drift = pool(t, e), pool(t, rms * rms), pool(t, dr)
        # The drift: out of the history at DRIFT_GAIN a frame, less what the pixel's own noise allows.
        cv2 = np.maximum(m2 / np.maximum(m1 * m1, 1e-12) - 1, 0)
        trust = 1 / (1 + cv2)
        if o.drift > 0 and t >= 100:
            allow = np.where(drift > 0, o.allowup, o.allow) * o.sdk * trust * np.sqrt(cv2)
            f = 1 + o.drift * np.sign(drift) * np.maximum(np.abs(drift) - allow, 0) / np.maximum(trust, 0.5)
            h, m1, m2 = h * f, m1 * f, m2 * f * f
        share = s / (s + h * (1 + o.channel) + DARK)
        res = (share - level) * np.clip((count - 1) / (o.mature - 1), 0, 1)
        band = np.maximum(o.k * o.sdk * np.sqrt(spread), o.floor)
        drop = np.clip((np.abs(pooled) / band - 1) * o.gain, 0, 1)
        plain = 1 / np.maximum(np.minimum(count + 1, o.cap), 1)
        a = plain + drop * (1 - plain)
        rel = np.clip((s - h) / (h + DARK), -1, o.clip) * np.clip((count - 1) / (o.mature - 1), 0, 1) * trust
        h += (s - h) * a
        m1 += (s - m1) * a
        m2 += (s * s - m2) * a
        count = 1 / a
        level += (share - level) / np.clip(count - o.first, 1, o.level)
        level = np.floor(level * 1023 + rng.random((N, N))) / 1023
        e = np.round((pooled + (res - pooled) * o.a) * 127) / 127
        rms = np.round(np.sqrt(spread + (res * res - spread) * o.a) * 127) / 127
        dr = np.round(np.clip(drift + (rel - drift) * o.a, -0.5, 0.5) * (1 - drop) * 254) / 254
        out.append((h.mean() / mean.mean(), count.mean(), (drop > 0).mean(), np.sqrt(((h - mean) ** 2).mean()) / mean.mean()))
    return out


if o.sd:
    e = np.zeros((N, N))
    v = []
    for t in range(400):
        p = pool(t, e)
        if t > 100:
            v.append(p.var())
        e = p + (rng.standard_normal((N, N)) - p) * o.a
    print(f"pooled average: {np.sqrt(np.mean(v)):.4f} of a residual's deviation ({1 / np.mean(v):.0f} residuals behind it)")
for kind in ["steady", "noisy", "mix0.02", "mix0.1", "rare0.1", "rare0.01"]:
    still = run(kind, 1.0, frames=300, edit=1000)[150:]
    print(f"{kind:9s} nothing changes: {np.mean([x[1] for x in still]):5.1f} frames, signalled {100 * np.mean([x[2] for x in still]):5.2f} %, rms {np.mean([x[3] for x in still]):.3f}, mean {np.mean([x[0] for x in still]):.3f}")
    r = run(kind, 1.0, restart=True)
    print(f"          restarted: " + "  ".join(f"{k}: {r[300 + k - 1][1]:.1f} frames" for k in [4, 8, 16, 32]) + f", signalled {100 * np.mean([x[2] for x in r[300:332]]):.2f} %")
    for g in [o.ramp, -o.ramp]:
        r = run(kind, 1.0, frames=480, ramp=g)
        print(f"          {'falling' if g > 0 else 'rising'} {abs(g):.3g} a frame: " + "  ".join(f"{k}: {r[300 + k - 1][0]:.2f}" for k in [30, 60, 120, 170]))
    for f in [1 / 13, 0.5, 0.75, 2.0, 13.0]:
        r = run(kind, f)
        print(f"          x {f:<6.3g} " + "  ".join(f"{k}: {r[300 + k - 1][0]:.2f}" for k in [2, 4, 8, 16, 32]))
