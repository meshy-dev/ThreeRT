# Converts ORCA (Falcor) DDS / TGA textures to PNG / JPEG for glTF. Run by tools/orca/fbx2gltf.py (inside Blender)
# with a Python that has Pillow and numpy (scripts/orca.ts makes that venv):
#
#   python -I tools/orca/textures.py jobs.json results.json
#
# jobs.json: [{"src", "dst" (without extension), "role": "color" | "orm" | "normal" | "emissive", "max": 2048}]
# results.json: {dst: {"file": the written path, "alpha": "opaque" | "mask", "size": [w, h]}}
#
# The ORCA convention (Bistro and Zero Day README): BaseColor RGB + opacity in alpha; Specular R occlusion (always 0 in
# both scenes, so unused), G roughness, B metalness, which is glTF's metallicRoughnessTexture layout as is; Normal in
# the DirectX convention (BC5: x, y only); Emissive RGB.
#  - color: alpha below 250 anywhere -> PNG RGBA (alpha-tested in Falcor: glTF MASK), else JPEG RGB;
#  - orm: JPEG RGB, 4:4:4 (no chroma subsampling across the roughness / metalness channels), R cleared;
#  - normal: OpenGL convention for glTF (green flipped), z rebuilt from x and y, JPEG 4:4:4;
#  - emissive: JPEG RGB.
# Images of 64 texels or fewer (the scenes' many constant 16 x 16 maps) are PNG, lossless. Larger than `max` on the long
# side -> downscaled (Lanczos), aspect kept; power-of-two sides of 512 or more then lose 32 texels (the renderer's page
# gutters).
import json
import os
import sys
from multiprocessing import Pool

import numpy as np
from PIL import Image

QUALITY = {"color": 90, "orm": 92, "normal": 95, "emissive": 92}


def convert(job):
    src, dst, role, cap = job["src"], job["dst"], job["role"], job.get("max", 2048)
    im = Image.open(src)
    im.load()
    has_alpha = im.mode in ("RGBA", "LA") or "transparency" in im.info
    im = im.convert("RGBA")
    w, h = im.size
    if max(w, h) > cap:
        k = cap / max(w, h)
        im = im.resize((max(1, round(w * k)), max(1, round(h * k))), Image.LANCZOS)
    # The renderer's texture pages give a power-of-two side of 512 or more up 2 x 16 gutter texels (renderer/src/
    # texpack.rs GUTTER) so four 2K textures fill a 4K page; done here once (Lanczos) instead of at every load (Bistro's
    # 337 2K maps took ~1 s of each first frame).
    w, h = im.size
    nw = w - 32 if w >= 512 and w & (w - 1) == 0 else w
    nh = h - 32 if h >= 512 and h & (h - 1) == 0 else h
    if (nw, nh) != (w, h):
        im = im.resize((nw, nh), Image.LANCZOS)
    a = np.asarray(im)
    alpha = "opaque"
    if role == "color" and has_alpha and a[..., 3].min() < 250:
        alpha = "mask"
    rgb = a[..., :3].copy()
    if role == "normal":
        xy = rgb[..., :2].astype(np.float32) / 127.5 - 1.0
        xy[..., 1] = -xy[..., 1]  # DirectX -> OpenGL
        z = np.sqrt(np.clip(1.0 - (xy ** 2).sum(-1), 0.0, 1.0))
        n = np.dstack([xy, z])
        n /= np.maximum(np.linalg.norm(n, axis=-1, keepdims=True), 1e-6)
        rgb = np.clip(np.round((n + 1.0) * 127.5), 0, 255).astype(np.uint8)
    elif role == "orm":
        rgb[..., 0] = 255  # occlusion unused (0 in the sources): white, harmless if a reader takes it
    small = max(im.size) <= 64
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    if alpha == "mask":
        out = dst + ".png"
        Image.fromarray(np.dstack([rgb, a[..., 3]]), "RGBA").save(out, optimize=True)
    elif small:
        out = dst + ".png"
        Image.fromarray(rgb, "RGB").save(out, optimize=True)
    else:
        out = dst + ".jpg"
        sub = 2 if role == "color" or role == "emissive" else 0
        Image.fromarray(rgb, "RGB").save(out, quality=QUALITY[role], subsampling=sub, optimize=True)
    return dst, {"file": out, "alpha": alpha, "size": list(im.size)}


def main():
    jobs = json.load(open(sys.argv[1]))
    with Pool(os.cpu_count()) as pool:
        results = dict(pool.imap_unordered(convert, jobs, chunksize=4))
    json.dump(results, open(sys.argv[2], "w"), indent=1)
    print(f"textures: {len(results)} converted", file=sys.stderr)


if __name__ == "__main__":
    main()
