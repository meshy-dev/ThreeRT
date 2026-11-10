# Image work for tools/milestone1.ts (Pillow + numpy; run with the venv scripts/orca.ts makes:
# <assets>/.cache/orca-venv/bin/python -I tools/milestone1/compose.py job.json):
#   {"regions": {"ref": png, "images": {name: png}, "boxes": {region: [x0, y0, x1, y1] (fractions)}, "out": json}}
#     -> per image and region: mean ΔE76 (CIE Lab, D65, from sRGB) and the luminance ratio to the reference.
#   {"gif": {"tiles": [{"label": str, "frames": [png...] | "still": png}], "out": gif | webp, "max_bytes": n, "width": px,
#            "fps": n}}
#     -> one animation, the tiles side by side with their labels: an animated GIF, or an animated WebP when `out` ends
#        in .webp (lossy, several times smaller at the same size and frame count); smaller (GIF: fewer colours; WebP:
#        lower quality; then narrower, fewer frames) until it fits max_bytes.
import io
import json
import sys

import numpy as np
from PIL import Image, ImageDraw, ImageFont


def lab(png):
    a = np.asarray(Image.open(png).convert("RGB")).astype(np.float64) / 255.0
    a = np.where(a <= 0.04045, a / 12.92, ((a + 0.055) / 1.055) ** 2.4)
    m = np.array([[0.4124564, 0.3575761, 0.1804375], [0.2126729, 0.7151522, 0.0721750], [0.0193339, 0.1191920, 0.9503041]])
    xyz = a @ m.T / np.array([0.95047, 1.0, 1.08883])
    f = np.where(xyz > 216 / 24389, np.cbrt(xyz), (24389 / 27 * xyz + 16) / 116)
    L = 116 * f[..., 1] - 16
    return np.dstack([L, 500 * (f[..., 0] - f[..., 1]), 200 * (f[..., 1] - f[..., 2])]), a @ np.array([0.2126, 0.7152, 0.0722])


def regions(job):
    ref, ref_y = lab(job["ref"])
    h, w = ref.shape[:2]
    out = {}
    for name, png in job["images"].items():
        img, y = lab(png)
        if img.shape != ref.shape:
            img, y = lab_resized(png, w, h)
        de = np.linalg.norm(img - ref, axis=-1)
        r = {"all": {"delta_e": round(float(de.mean()), 2), "luminance_ratio": round(float(y.mean() / max(ref_y.mean(), 1e-9)), 3)}}
        for reg, (x0, y0, x1, y1) in job["boxes"].items():
            sl = (slice(int(y0 * h), int(y1 * h)), slice(int(x0 * w), int(x1 * w)))
            r[reg] = {"delta_e": round(float(de[sl].mean()), 2), "luminance_ratio": round(float(y[sl].mean() / max(ref_y[sl].mean(), 1e-9)), 3)}
        out[name] = r
    json.dump(out, open(job["out"], "w"), indent=1)


def lab_resized(png, w, h):
    tmp = io.BytesIO()
    Image.open(png).convert("RGB").resize((w, h), Image.LANCZOS).save(tmp, "PNG")
    tmp.seek(0)
    return lab(tmp)


def font(px):
    for f in ("/System/Library/Fonts/Helvetica.ttc", "/System/Library/Fonts/SFNS.ttf", "/Library/Fonts/Arial.ttf"):
        try:
            return ImageFont.truetype(f, px)
        except OSError:
            pass
    return ImageFont.load_default()


def gif(job):
    tiles = job["tiles"]
    n = max(len(t.get("frames", [])) for t in tiles) or 1
    webp = job["out"].lower().endswith(".webp")
    width, colors, step, quality = job.get("width", 1280), 256, 1, 80
    while True:
        tw = width // len(tiles)
        th = tw * 9 // 16
        lab_h = max(18, tw // 18)
        f = font(lab_h - 4)
        frames = []
        for k in range(0, n, step):
            canvas = Image.new("RGB", (tw * len(tiles), th + lab_h), (16, 18, 22))
            d = ImageDraw.Draw(canvas)
            for j, t in enumerate(tiles):
                src = t["frames"][min(k, len(t["frames"]) - 1)] if t.get("frames") else t.get("still")
                if src:
                    canvas.paste(Image.open(src).convert("RGB").resize((tw, th), Image.LANCZOS), (j * tw, lab_h))
                else:
                    d.text((j * tw + tw // 3, lab_h + th // 2), "pending", fill=(140, 140, 150), font=f)
                d.text((j * tw + 6, 2), t["label"], fill=(255, 255, 255), font=f)
            frames.append(canvas if webp else canvas.quantize(colors=colors, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE))
        buf = io.BytesIO()
        duration = int(1000 * step / job.get("fps", 10))
        if webp:
            frames[0].save(buf, "WEBP", save_all=True, append_images=frames[1:], duration=duration, loop=0, quality=quality, method=4)
        else:
            frames[0].save(buf, "GIF", save_all=True, append_images=frames[1:], duration=duration, loop=0, optimize=True)
        if buf.tell() <= job["max_bytes"] or width <= 480:
            open(job["out"], "wb").write(buf.getvalue())
            print(json.dumps({"bytes": buf.tell(), "width": tw * len(tiles), "frames": len(frames), **({"quality": quality} if webp else {"colors": colors})}))
            return
        # Too big: fewer colours (lower quality) first, then narrower, then every other frame.
        if webp and quality > 60:
            quality = 60
        elif not webp and colors > 128:
            colors = 128
        elif width > 960:
            width = int(width * 0.8)
        elif step == 1:
            step = 2
        else:
            width = int(width * 0.8)


if __name__ == "__main__":
    # numpy 2.0 with Accelerate reports spurious floating-point warnings in matmul of finite values.
    np.seterr(all="ignore")
    job = json.load(open(sys.argv[1]))
    if "regions" in job:
        regions(job["regions"])
    if "gif" in job:
        gif(job["gif"])
