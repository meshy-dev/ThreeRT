# A transparent overlay of text labels on dark boxes (this machine's ffmpeg has no drawtext).
#   python3 overlay.py out.png width height font '[{"text": "...", "x": 24, "y": 24, "size": 40, "align": "left|right|center"}]'
import json, sys
from PIL import Image, ImageDraw, ImageFont

out, w, h, font, labels = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4], json.loads(sys.argv[5])
img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
d = ImageDraw.Draw(img)
for l in labels:
    f = ImageFont.truetype(font, l["size"])
    x0, y0, x1, y1 = d.textbbox((0, 0), l["text"], font=f)
    tw, th = x1 - x0, y1 - y0
    pad = max(8, l["size"] // 3)
    x = {"left": l["x"], "right": w - l["x"] - tw, "center": (w - tw) // 2}[l.get("align", "left")]
    y = l["y"] if l["y"] >= 0 else h + l["y"] - th
    d.rounded_rectangle((x - pad, y - pad, x + tw + pad, y + th + pad), radius=pad // 2, fill=(0, 0, 0, 150))
    d.text((x - x0, y - y0), l["text"], font=f, fill=(255, 255, 255, 255))
img.save(out)
