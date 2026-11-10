"""Tileable noise masks (spectral synthesis => seamless wrap): puddles, grime, cloud."""
import os, sys
import numpy as np
from PIL import Image

OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)
rng = np.random.default_rng(7)


def spectral(n, beta, lo=1.0):
    fx = np.fft.fftfreq(n)[:, None] * n
    fy = np.fft.fftfreq(n)[None, :] * n
    f = np.sqrt(fx * fx + fy * fy)
    f[0, 0] = 1
    amp = 1.0 / np.power(np.maximum(f, lo), beta / 2.0)
    amp[0, 0] = 0
    phase = rng.uniform(0, 2 * np.pi, (n, n))
    spec = amp * np.exp(1j * phase)
    img = np.real(np.fft.ifft2(spec))
    img = (img - img.min()) / (img.max() - img.min())
    return img


def save(a, name):
    Image.fromarray(np.clip(a * 255, 0, 255).astype(np.uint8), "L").save(os.path.join(OUT, name + ".png"))


# puddles: large blobs + detail; R = puddle field (threshold in material)
p = spectral(1024, 3.2, 2.0) * 0.8 + spectral(1024, 2.2, 8.0) * 0.2
save(p, "T_PuddleMask")
# grime/streaks for walls: vertical streaking via anisotropic blur of noise
g = spectral(1024, 2.4, 3.0)
streak = np.cumsum(rng.uniform(0, 1, (1024, 1024)) ** 8, axis=0)
streak = (streak - streak.min()) / (streak.max() - streak.min())
save(np.clip(g * 0.75 + (np.roll(streak, 0, 0) % 1) * 0.0 + spectral(1024, 1.6, 16.0) * 0.25, 0, 1), "T_Grime")
save(spectral(1024, 3.6, 1.5), "T_Cloud")
print("ok")
