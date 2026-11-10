"""Generate the Kabukicho signage/texture set (PNG + manifest.json) with Pillow.

Every image is designed as an emissive lightbox face: the material multiplies it by an intensity,
so pure black means "unlit". The manifest records each texture's kind and its average glow colour,
which the level builder uses to colour the light that each sign casts on the wet street.
"""
import json
import math
import os
import random
import sys

from PIL import Image, ImageDraw, ImageFilter, ImageFont, ImageChops

OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.path.dirname(__file__), "..", "generated", "textures")
os.makedirs(OUT, exist_ok=True)

FONT_DIR = "/System/Library/Fonts"
GOTHIC = {w: os.path.join(FONT_DIR, f"ヒラギノ角ゴシック W{w}.ttc") for w in range(0, 10)}
MINCHO = os.path.join(FONT_DIR, "ヒラギノ明朝 ProN.ttc")
MARU = os.path.join(FONT_DIR, "ヒラギノ丸ゴ ProN W4.ttc")

rng = random.Random(20261104)
manifest = []

_font_cache = {}


def font(path, size, index=0):
    key = (path, size, index)
    if key not in _font_cache:
        _font_cache[key] = ImageFont.truetype(path, size, index=index)
    return _font_cache[key]


def heavy(size):
    return font(GOTHIC[rng.choice([8, 9])], size)


def bold(size):
    return font(GOTHIC[rng.choice([6, 7])], size)


def mincho(size):
    return font(MINCHO, size, index=rng.choice([0, 1]) if False else 0)


def pick_font(style, size):
    if style == "mincho":
        return font(MINCHO, size)
    if style == "maru":
        return font(MARU, size)
    if style == "medium":
        return font(GOTHIC[5], size)
    return font(GOTHIC[8 if style == "heavy" else 7], size)


# --- palettes ---------------------------------------------------------------------------------
# (background, text, accent) - lightbox colours that read well when pushed into HDR.
LIGHTBOX = [
    ((250, 250, 245), (220, 20, 40), (20, 20, 20)),
    ((255, 238, 60), (200, 10, 20), (20, 20, 20)),
    ((230, 20, 40), (255, 255, 250), (255, 220, 40)),
    ((20, 60, 200), (255, 255, 255), (255, 230, 60)),
    ((10, 10, 12), (255, 230, 80), (230, 30, 60)),
    ((255, 120, 20), (255, 255, 255), (60, 20, 10)),
    ((250, 250, 245), (20, 70, 200), (220, 20, 40)),
    ((40, 160, 70), (255, 255, 255), (255, 240, 120)),
    ((255, 80, 160), (255, 255, 255), (60, 0, 40)),
    ((120, 30, 160), (255, 240, 120), (255, 255, 255)),
    ((250, 250, 245), (20, 20, 20), (220, 20, 40)),
    ((0, 150, 200), (255, 255, 255), (255, 240, 0)),
    ((180, 10, 20), (255, 230, 120), (255, 255, 255)),
    ((15, 15, 60), (80, 220, 255), (255, 80, 200)),
]
NEON = [(255, 40, 140), (40, 230, 255), (255, 60, 40), (255, 210, 40), (140, 80, 255), (60, 255, 120), (255, 120, 220), (255, 255, 255)]

SHOP_NAMES = ["夜鷹", "龍門", "花道", "月光", "金魚", "桜", "大黒", "侍", "虎", "一番", "百番", "銀河", "蘭", "華",
              "紅", "雅", "楽", "宴", "鶴", "亀", "鈴", "舞", "嵐", "朧", "灯", "凛", "響", "夢", "粋", "暁"]
SHOP_TYPES = [
    ("居酒屋", "mincho"), ("焼肉", "heavy"), ("らあめん", "heavy"), ("ラーメン", "heavy"), ("やきとり", "mincho"),
    ("寿司", "mincho"), ("餃子", "heavy"), ("もつ鍋", "bold"), ("牛タン", "heavy"), ("天ぷら", "mincho"),
    ("カラオケ", "heavy"), ("スナック", "maru"), ("麻雀", "heavy"), ("ダーツ", "bold"), ("漫画喫茶", "bold"),
    ("占い", "mincho"), ("マッサージ", "maru"), ("ホテル", "bold"), ("ゲーム", "heavy"), ("パチンコ", "heavy"),
    ("無料案内所", "heavy"), ("串カツ", "heavy"), ("おでん", "mincho"), ("蕎麦", "mincho"), ("立ち飲み", "bold"),
    ("中華料理", "heavy"), ("たこ焼", "maru"), ("BAR", "heavy"), ("LOUNGE", "bold"), ("CLUB", "heavy"),
]
FLOORS = ["B2F", "B1F", "1F", "2F", "3F", "4F", "5F", "6F", "7F", "8F", "9F"]


def glow(img, radius, strength=1.0):
    """Add a soft additive bloom of the image onto itself (neon tubes / bulbs)."""
    blur = img.filter(ImageFilter.GaussianBlur(radius))
    if strength != 1.0:
        blur = blur.point(lambda v: min(255, int(v * strength)))
    return ImageChops.add(img, blur)


def lightbox_shading(img, edge=0.18):
    """Diffuser falloff: lightboxes are brightest in the middle, dimmer at the frame."""
    w, h = img.size
    mask = Image.new("L", (w, h), 0)
    d = ImageDraw.Draw(mask)
    steps = 24
    for i in range(steps):
        t = i / steps
        v = int(255 * (1 - edge) + 255 * edge * t)
        inset_x = int(w * 0.5 * t * 0.9)
        inset_y = int(h * 0.5 * t * 0.9)
        d.rectangle([inset_x, inset_y, w - inset_x, h - inset_y], fill=v)
    mask = mask.filter(ImageFilter.GaussianBlur(min(w, h) * 0.08))
    black = Image.new("RGB", (w, h), (0, 0, 0))
    return Image.composite(img, black, mask)


def draw_vertical_text(d, text, box, fnt, fill, stroke=0, stroke_fill=None, spacing=1.0):
    """Japanese vertical writing: one glyph per row, centred in box; Latin runs are rotated."""
    x0, y0, x1, y1 = box
    chars = list(text)
    size = fnt.size
    total = len(chars) * size * spacing
    y = y0 + (y1 - y0 - total) / 2
    cx = (x0 + x1) / 2
    for ch in chars:
        if ch == "ー":
            # long vowel mark is drawn vertically in tategaki
            bw = size * 0.12
            d.rectangle([cx - bw / 2, y + size * 0.12, cx + bw / 2, y + size * spacing - size * 0.12], fill=fill)
        else:
            d.text((cx, y + size * spacing / 2), ch, font=fnt, fill=fill, anchor="mm", stroke_width=stroke, stroke_fill=stroke_fill)
        y += size * spacing


def fit_vertical(text, style, max_w, max_h, spacing=1.0):
    size = int(max_w)
    while size > 10:
        if len(text) * size * spacing <= max_h:
            return pick_font(style, size)
        size -= 2
    return pick_font(style, 10)


def fit_horizontal(text, style, max_w, max_h):
    size = int(max_h)
    while size > 8:
        f = pick_font(style, size)
        l, t, r, b = f.getbbox(text)
        if r - l <= max_w and b - t <= max_h:
            return f
        size -= 2
    return pick_font(style, 8)


def avg_glow(img):
    """Mean colour of the bright half of the image, used to tint the sign's light."""
    small = img.convert("RGB").resize((32, 32), Image.BILINEAR)
    px = list(small.getdata())
    px.sort(key=lambda p: sum(p), reverse=True)
    top = px[: len(px) // 2]
    r = sum(p[0] for p in top) / len(top) / 255.0
    g = sum(p[1] for p in top) / len(top) / 255.0
    b = sum(p[2] for p in top) / len(top) / 255.0
    return [round(r, 3), round(g, 3), round(b, 3)]


def save(img, name, kind, **extra):
    path = os.path.join(OUT, name + ".png")
    img.save(path, optimize=True)
    entry = {"name": name, "kind": kind, "size": list(img.size), "glow": avg_glow(img)}
    entry.update(extra)
    manifest.append(entry)


# --- vertical lightbox (tate-kanban) ------------------------------------------------------------
def vertical_sign(i):
    w, h = 256, 1024
    bg, fg, accent = rng.choice(LIGHTBOX)
    img = Image.new("RGB", (w, h), bg)
    d = ImageDraw.Draw(img)
    typ, style = rng.choice(SHOP_TYPES)
    name = rng.choice(SHOP_NAMES)
    layout = rng.random()
    border = rng.choice([0, 6, 10])
    if border:
        d.rectangle([border, border, w - border, h - border], outline=accent, width=border)
    if typ.isascii():
        # rotated latin wordmark + name
        txt = Image.new("RGBA", (h - 160, w - 40), (0, 0, 0, 0))
        td = ImageDraw.Draw(txt)
        f = fit_horizontal(typ + " " + name, "heavy", txt.width - 20, txt.height - 30)
        td.text((txt.width / 2, txt.height / 2), typ + "  " + name, font=f, fill=fg, anchor="mm")
        txt = txt.rotate(-90, expand=True)
        img.paste(txt, (20, 40), txt)
    elif layout < 0.55:
        # type big, name in a circle badge on top
        r = w * 0.36
        d.ellipse([w / 2 - r, 30, w / 2 + r, 30 + 2 * r], fill=accent)
        d.text((w / 2, 30 + r), name[0], font=pick_font("mincho", int(r * 1.3)), fill=bg, anchor="mm")
        top = 30 + 2 * r + 24
        f = fit_vertical(typ, style, w * 0.72, h - top - 110)
        draw_vertical_text(d, typ, (0, top, w, h - 100), f, fg)
    else:
        f = fit_vertical(name + typ, style, w * 0.68, h - 140)
        draw_vertical_text(d, typ + name, (0, 40, w, h - 110), f, fg, stroke=rng.choice([0, 0, 3]), stroke_fill=accent)
    # floor tag strip
    fl = rng.choice(FLOORS)
    d.rectangle([0, h - 92, w, h], fill=accent)
    d.text((w / 2, h - 46), fl, font=pick_font("heavy", 64), fill=bg if accent != bg else fg, anchor="mm")
    img = lightbox_shading(img)
    save(img, f"T_SignV_{i:02d}", "vertical")


# --- stacked building directory tower (several shops, one per floor) --------------------------
def tower_sign(i):
    w, h = 256, 1536
    n = rng.choice([4, 5, 6, 7])
    img = Image.new("RGB", (w, h), (0, 0, 0))
    d = ImageDraw.Draw(img)
    ph = h / n
    floors = FLOORS[2: 2 + n][::-1] if rng.random() < 0.7 else FLOORS[:n][::-1]
    for k in range(n):
        bg, fg, accent = rng.choice(LIGHTBOX)
        y0, y1 = int(k * ph), int((k + 1) * ph)
        d.rectangle([0, y0, w, y1], fill=bg)
        typ, style = rng.choice(SHOP_TYPES)
        name = rng.choice(SHOP_NAMES)
        label = typ if rng.random() < 0.5 else name + " " + typ if typ.isascii() else typ + name
        if label.isascii() or any(c.isascii() and c.isalpha() for c in label):
            f = fit_horizontal(label, "heavy", w - 30, (y1 - y0) * 0.35)
            d.text((w / 2, (y0 + y1) / 2 - 10), label, font=f, fill=fg, anchor="mm")
        else:
            f = fit_vertical(label[:5], style, w * 0.5, (y1 - y0) - 60, spacing=0.98)
            draw_vertical_text(d, label[:5], (0, y0 + 8, w, y1 - 46), f, fg)
        d.text((w / 2, y1 - 26), floors[k % len(floors)], font=pick_font("heavy", 36), fill=accent, anchor="mm")
        d.line([0, y1 - 2, w, y1 - 2], fill=(20, 20, 20), width=6)
    img = lightbox_shading(img, edge=0.1)
    save(img, f"T_SignTower_{i:02d}", "tower")


# --- horizontal fascia signs --------------------------------------------------------------------
HORIZ_COPY = [
    ("カラオケ", "KARAOKE  60分 ¥580~"), ("居酒屋", "全品 ¥299  飲み放題"), ("焼肉", "食べ放題 ¥2,980"),
    ("ラーメン", "とんこつ  替玉無料"), ("やきとり", "炭火焼  創業昭和"), ("寿司", "回転寿司  一皿 ¥130"),
    ("麻雀", "雀荘  24時間営業"), ("ダーツ", "DARTS & BAR"), ("漫画喫茶", "ネットカフェ  シャワー完備"),
    ("ゲームセンター", "UFO CATCHER"), ("無料案内所", "FREE INFORMATION"), ("牛丼", "並盛 ¥450"),
    ("餃子", "焼き・水・揚げ"), ("中華料理", "本格四川"), ("ホテル", "HOTEL  休憩 ¥4,000~"),
    ("占い", "手相・タロット"), ("立ち飲み", "せんべろ"), ("ドラッグストア", "DRUG STORE  免税"),
    ("BAR", "OPEN 19:00 - LAST"), ("HOST CLUB", "★ PREMIUM NIGHT ★"), ("バッティング", "BATTING CENTER"),
    ("串カツ", "二度づけ禁止"), ("もつ鍋", "博多の味"), ("天ぷら", "揚げたて"),
]


def horizontal_sign(i):
    w, h = 1024, 256
    jp, sub = HORIZ_COPY[i % len(HORIZ_COPY)]
    style = rng.random()
    if style < 0.6:
        bg, fg, accent = rng.choice(LIGHTBOX)
        img = Image.new("RGB", (w, h), bg)
        d = ImageDraw.Draw(img)
        d.rectangle([0, h - 70, w, h], fill=accent)
        f = fit_horizontal(jp, "heavy" if rng.random() < 0.7 else "mincho", w - 80, h - 100)
        d.text((w / 2, (h - 70) / 2 + 4), jp, font=f, fill=fg, anchor="mm")
        fs = fit_horizontal(sub, "bold", w - 60, 50)
        d.text((w / 2, h - 35), sub, font=fs, fill=bg, anchor="mm")
        img = lightbox_shading(img, edge=0.12)
        kind = "horizontal"
    else:
        # neon on dark backing board
        col = rng.choice(NEON)
        col2 = rng.choice([c for c in NEON if c != col])
        img = Image.new("RGB", (w, h), (8, 6, 10))
        d = ImageDraw.Draw(img)
        d.rounded_rectangle([12, 12, w - 12, h - 12], radius=30, outline=col2, width=6)
        f = fit_horizontal(jp, "maru" if rng.random() < 0.5 else "bold", w - 120, h - 110)
        d.text((w / 2, h / 2 - 22), jp, font=f, fill=col, anchor="mm")
        fs = fit_horizontal(sub, "medium", w - 140, 44)
        d.text((w / 2, h - 52), sub, font=fs, fill=col2, anchor="mm")
        img = glow(img, 6, 1.3)
        img = glow(img, 18, 0.8)
        kind = "neon"
    save(img, f"T_SignH_{i:02d}", kind)


# --- big rooftop / facade billboards ------------------------------------------------------------
BILLBOARDS = [
    ("新宿 夜祭", "SHINJUKU NIGHT FESTIVAL", "2026.11.04 - 11.30"),
    ("HOST CLUB 輝", "NO.1 PLAYER  RYU", "TONIGHT ★ OPEN"),
    ("歌舞伎町 劇場", "KABUKICHO THEATER", "LIVE  EVERY NIGHT"),
    ("激安の殿堂", "DISCOUNT TOWER", "24H OPEN"),
    ("TOKYO CYBER ARCADE", "ゲーム ・ プリクラ ・ UFO", "B1F - 5F"),
    ("ROBOT CABARET", "ロボット レストラン", "SHOW 18:00 / 20:00 / 22:00"),
    ("新宿 ラーメン横丁", "RAMEN ALLEY", "12店舗 集結"),
    ("CINEMA CITY", "シネマシティ 新宿", "IMAX ・ 4DX"),
    ("夜の動物園", "NIGHT ZOO BAR", "ONLY IN KABUKICHO"),
    ("KARAOKE 歌広場", "全室 防音 完全個室", "朝まで ¥1,980"),
]


def billboard(i):
    w, h = 1024, 512
    title, sub, foot = BILLBOARDS[i % len(BILLBOARDS)]
    img = Image.new("RGB", (w, h))
    d = ImageDraw.Draw(img)
    a = rng.choice(NEON)
    b = rng.choice([c for c in NEON if c != a])
    for y in range(h):
        t = y / h
        col = tuple(int(a[k] * (1 - t) * 0.55 + b[k] * t * 0.35) for k in range(3))
        d.line([0, y, w, y], fill=col)
    # graphic shapes
    for _ in range(rng.randint(3, 7)):
        cx, cy, r = rng.randint(0, w), rng.randint(0, h), rng.randint(40, 220)
        c = rng.choice(NEON)
        d.ellipse([cx - r, cy - r, cx + r, cy + r], outline=c, width=rng.randint(4, 14))
    for k in range(rng.randint(0, 12)):
        x = rng.randint(0, w)
        d.polygon([(x, 0), (x + 40, 0), (x - 160, h), (x - 200, h)], fill=tuple(min(255, v + 30) for v in b))
    img = img.filter(ImageFilter.GaussianBlur(2))
    d = ImageDraw.Draw(img)
    ft = fit_horizontal(title, "heavy", w - 80, 190)
    d.text((w / 2, 190), title, font=ft, fill=(255, 255, 255), anchor="mm", stroke_width=8, stroke_fill=(20, 0, 30))
    fs = fit_horizontal(sub, "bold", w - 160, 70)
    d.text((w / 2, 330), sub, font=fs, fill=(255, 245, 160), anchor="mm", stroke_width=4, stroke_fill=(20, 0, 30))
    d.rectangle([0, h - 80, w, h], fill=(10, 10, 14))
    ff = fit_horizontal(foot, "bold", w - 200, 48)
    d.text((w / 2, h - 40), foot, font=ff, fill=a, anchor="mm")
    img = lightbox_shading(img, edge=0.1)
    save(img, f"T_Billboard_{i:02d}", "billboard")


# --- Kabukicho Ichiban-gai gate -----------------------------------------------------------------
def gate_sign():
    w, h = 2048, 512
    img = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    # arch silhouette: a flat-topped arc
    def arch_y(x):
        t = (x / w) * 2 - 1
        return int(70 + 60 * t * t)
    poly = [(0, h)] + [(x, arch_y(x)) for x in range(0, w + 1, 16)] + [(w, h)]
    d.polygon(poly, fill=(170, 10, 18, 255))
    inner = [(30, h - 30)] + [(x, arch_y(x) + 34) for x in range(30, w - 29, 16)] + [(w - 30, h - 30)]
    d.polygon(inner, fill=(235, 30, 35, 255))
    # text plate
    txt = "歌舞伎町一番街"
    f = fit_horizontal(txt, "heavy", w - 260, 270)
    tl = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    td = ImageDraw.Draw(tl)
    td.text((w / 2, h * 0.58), txt, font=f, fill=(255, 250, 235, 255), anchor="mm", stroke_width=6, stroke_fill=(255, 210, 120, 255))
    tl_rgb = tl.convert("RGB")
    tl_glow = tl_rgb.filter(ImageFilter.GaussianBlur(10))
    base = img.convert("RGB")
    base = ImageChops.add(base, tl_glow.point(lambda v: int(v * 0.6)))
    base.paste(tl_rgb, (0, 0), tl)
    # bulbs along the arch and the bottom edge
    bulbs = Image.new("RGB", (w, h), (0, 0, 0))
    bd = ImageDraw.Draw(bulbs)
    for x in range(40, w - 39, 44):
        y = arch_y(x) + 17
        bd.ellipse([x - 9, y - 9, x + 9, y + 9], fill=(255, 236, 170))
    for x in range(40, w - 39, 44):
        bd.ellipse([x - 9, h - 26, x + 9, h - 8], fill=(255, 236, 170))
    bulbs = glow(bulbs, 7, 1.4)
    base = ImageChops.add(base, bulbs)
    out = base.convert("RGBA")
    out.putalpha(img.split()[3])
    save(out, "T_Gate_Ichibangai", "gate")
    # second face: "KABUKICHO" in latin on the back side, as on the real gate's rear
    img2 = Image.new("RGBA", (w, h), (0, 0, 0, 0))
    d2 = ImageDraw.Draw(img2)
    d2.polygon(poly, fill=(170, 10, 18, 255))
    d2.polygon(inner, fill=(235, 30, 35, 255))
    f2 = fit_horizontal("KABUKICHO  歌舞伎町", "heavy", w - 300, 220)
    d2.text((w / 2, h * 0.6), "KABUKICHO  歌舞伎町", font=f2, fill=(255, 250, 235, 255), anchor="mm")
    b2 = ImageChops.add(img2.convert("RGB"), bulbs)
    out2 = b2.convert("RGBA")
    out2.putalpha(img2.split()[3])
    save(out2, "T_Gate_Back", "gate")


# --- vending machine fronts --------------------------------------------------------------------
DRINK_COLS = [(200, 20, 30), (20, 60, 180), (240, 240, 240), (30, 140, 60), (120, 70, 30), (240, 150, 20),
              (230, 210, 40), (20, 20, 20), (90, 180, 230), (220, 80, 140), (160, 20, 20), (240, 240, 200)]


def vending(i):
    w, h = 512, 1024
    body = rng.choice([(235, 235, 238), (200, 20, 30), (20, 70, 170), (30, 30, 34), (230, 120, 20)])
    img = Image.new("RGB", (w, h), tuple(int(c * 0.55) for c in body))
    d = ImageDraw.Draw(img)
    # display window
    d.rectangle([24, 60, w - 24, 620], fill=(250, 250, 255))
    for row in range(4):
        y0 = 80 + row * 135
        d.rectangle([34, y0 + 100, w - 34, y0 + 108], fill=(150, 150, 160))
        n = 7 if row < 3 else 6
        cw = (w - 68) / n
        for k in range(n):
            x0 = 34 + k * cw + 6
            col = rng.choice(DRINK_COLS)
            tall = rng.random() < 0.4
            top = y0 + (6 if tall else 24)
            d.rounded_rectangle([x0, top, x0 + cw - 12, y0 + 98], radius=8, fill=col)
            d.rectangle([x0 + 4, top + 18, x0 + cw - 16, top + 34], fill=tuple(min(255, c + 70) for c in col))
            d.line([x0 + 6, top + 4, x0 + 6, y0 + 94], fill=(255, 255, 255), width=3)
            # price and button
            d.rectangle([x0, y0 + 112, x0 + cw - 12, y0 + 128], fill=(20, 20, 20))
            d.text((x0 + (cw - 12) / 2, y0 + 120), rng.choice(["130", "160", "110", "180", "200"]), font=pick_font("bold", 13), fill=(80, 255, 120), anchor="mm")
    # header
    d.rectangle([0, 0, w, 56], fill=body)
    d.text((w / 2, 28), rng.choice(["つめた〜い", "DRINK", "あったか〜い", "COLD & HOT"]), font=pick_font("heavy", 34), fill=(255, 255, 255) if body != (235, 235, 238) else (200, 20, 30), anchor="mm")
    # lower panel: coin slot, display, dispenser
    d.rectangle([w - 160, 650, w - 40, 760], fill=(30, 30, 30))
    d.rectangle([w - 150, 662, w - 50, 690], fill=(10, 40, 10))
    d.text((w - 100, 676), "¥ 0", font=pick_font("bold", 20), fill=(90, 255, 120), anchor="mm")
    d.rectangle([w - 120, 705, w - 80, 745], fill=(80, 80, 80))
    d.rectangle([40, 860, w - 40, 960], fill=(10, 10, 12))
    d.text((w / 2, 980), "とりだしぐち", font=pick_font("medium", 18), fill=(240, 240, 240), anchor="mm")
    d.rectangle([40, 650, w - 190, 830], fill=tuple(int(c * 0.8) for c in body))
    d.text((40 + (w - 230) / 2, 740), rng.choice(["冷", "温", "茶", "珈琲"]), font=pick_font("mincho", 110), fill=(255, 255, 255), anchor="mm")
    save(img, f"T_Vending_{i:02d}", "vending")


# --- chochin lanterns ----------------------------------------------------------------------------
def lantern(i):
    w, h = 1024, 512
    base = rng.choice([(220, 30, 25), (230, 40, 30), (245, 230, 200), (230, 120, 30)])
    img = Image.new("RGB", (w, h), base)
    d = ImageDraw.Draw(img)
    for y in range(0, h, 22):
        d.line([0, y, w, y], fill=tuple(int(c * 0.78) for c in base), width=3)
    d.rectangle([0, 0, w, 40], fill=(25, 20, 20))
    d.rectangle([0, h - 40, w, h], fill=(25, 20, 20))
    txt = rng.choice(["焼鳥", "居酒屋", "酒", "おでん", "祭", "串", "やきとり", "大衆酒場"])
    ink = (20, 15, 15) if base[0] > 200 and base[1] < 200 else (200, 20, 20)
    # text appears twice around the lantern
    for cx in (w * 0.25, w * 0.75):
        f = fit_vertical(txt, "mincho", 170, h - 120)
        draw_vertical_text(d, txt, (cx - 100, 50, cx + 100, h - 50), f, ink)
    img = lightbox_shading(img, edge=0.35)
    save(img, f"T_Lantern_{i:02d}", "lantern")


# --- noren curtains ------------------------------------------------------------------------------
def noren(i):
    w, h = 768, 512
    base = rng.choice([(22, 34, 80), (120, 20, 25), (20, 20, 22), (230, 225, 210)])
    img = Image.new("RGB", (w, h), base)
    d = ImageDraw.Draw(img)
    ink = (240, 240, 235) if base[0] < 200 else (25, 25, 30)
    txt = rng.choice(["らあめん", "そば", "うどん", "めし", "酒処", "やきとり"])
    n = len(txt)
    pw = w / n
    for k, ch in enumerate(txt):
        d.text((pw * k + pw / 2, h * 0.55), ch, font=pick_font("mincho", int(min(pw * 0.8, h * 0.5))), fill=ink, anchor="mm")
        if k:
            d.line([pw * k, 0, pw * k, h], fill=tuple(int(c * 0.5) for c in base), width=6)
    save(img, f"T_Noren_{i:02d}", "noren")


# --- shop interiors seen through glass ---------------------------------------------------------
def interior(i):
    """Back wall of a shop interior (seen ~4 m behind the shop front)."""
    w, h = 1024, 512
    kind = ["konbini", "izakaya", "ramen", "karaoke", "pachinko", "bar", "konbini", "izakaya"][i % 8]
    img = Image.new("RGB", (w, h), (0, 0, 0))
    d = ImageDraw.Draw(img)
    if kind == "konbini":
        d.rectangle([0, 0, w, h], fill=(225, 228, 232))
        for k, c in enumerate([(0, 120, 200), (40, 170, 70), (240, 130, 20)]):
            d.rectangle([0, 18 + k * 14, w, 30 + k * 14], fill=c)
        for row in range(5):
            y = 90 + row * 78
            d.rectangle([0, y + 62, w, y + 70], fill=(170, 170, 175))
            x = 4
            while x < w:
                pw = rng.randint(14, 34)
                ph = rng.randint(28, 58)
                col = rng.choice(DRINK_COLS + [(250, 250, 250), (255, 210, 0), (230, 60, 80)])
                d.rectangle([x, y + 62 - ph, x + pw - 3, y + 62], fill=col)
                d.rectangle([x + 2, y + 62 - ph + 6, x + pw - 5, y + 62 - ph + 12], fill=tuple(min(255, c + 60) for c in col))
                x += pw
    elif kind in ("izakaya", "ramen"):
        d.rectangle([0, 0, w, h], fill=(120, 72, 38))
        for x in range(0, w, 64):
            d.line([x, 0, x, h], fill=(95, 55, 28), width=3)
        menu = ["生ビール", "枝豆", "焼鳥", "唐揚げ", "刺身", "冷奴", "餃子", "ハイボール", "焼酎", "日本酒", "おでん", "ラーメン", "つけ麺", "替玉", "味玉"]
        y0 = 40
        for k in range(14):
            x = 30 + k * 70
            d.rectangle([x, y0, x + 56, y0 + 250], fill=(245, 238, 220))
            item = rng.choice(menu)
            f = fit_vertical(item[:5], "mincho", 44, 190, 0.95)
            draw_vertical_text(d, item[:5], (x, y0 + 6, x + 56, y0 + 200), f, (25, 20, 20), spacing=0.95)
            d.text((x + 28, y0 + 228), f"{rng.choice([280, 380, 480, 580])}", font=pick_font("bold", 18), fill=(190, 20, 20), anchor="mm")
        d.rectangle([0, 330, w, h], fill=(70, 40, 22))
        d.rectangle([0, 322, w, 338], fill=(150, 95, 50))
        for k in range(24):
            x = rng.randint(10, w - 30)
            d.rectangle([x, 345, x + 18, 400], fill=rng.choice([(30, 90, 40), (90, 50, 20), (200, 200, 210), (20, 20, 20)]))
        if kind == "izakaya":
            for x in (120, 520, 900):
                d.ellipse([x - 40, 280, x + 40, 380], fill=(230, 40, 30))
    elif kind == "karaoke":
        d.rectangle([0, 0, w, h], fill=(28, 10, 40))
        for k in range(6):
            x = 40 + k * 165
            c = rng.choice(NEON)
            d.rectangle([x, 60, x + 130, 260], fill=tuple(int(v * 0.5) for v in c))
            d.rectangle([x, 60, x + 130, 260], outline=c, width=4)
            d.text((x + 65, 160), rng.choice(["♪", "★", "MIC", "VIP", "歌", "LIVE"]), font=pick_font("heavy", 50), fill=(255, 255, 255), anchor="mm")
        d.rectangle([0, 300, w, 312], fill=(255, 60, 160))
        d.text((w / 2, 400), "KARAOKE  カラオケ  受付", font=pick_font("heavy", 52), fill=(255, 220, 255), anchor="mm")
    elif kind == "pachinko":
        d.rectangle([0, 0, w, h], fill=(60, 10, 10))
        for k in range(12):
            x = 10 + k * 84
            d.rectangle([x, 40, x + 74, 460], fill=(200, 200, 210))
            d.ellipse([x + 6, 70, x + 68, 200], fill=rng.choice(NEON))
            d.rectangle([x + 10, 220, x + 64, 300], fill=rng.choice(NEON))
            d.rectangle([x + 4, 380, x + 70, 450], fill=(30, 30, 30))
        d.rectangle([0, 0, w, 30], fill=(255, 200, 0))
    else:  # bar
        d.rectangle([0, 0, w, h], fill=(25, 15, 10))
        for row in range(3):
            y = 70 + row * 100
            d.rectangle([0, y + 80, w, y + 86], fill=(210, 150, 60))
            for k in range(30):
                x = 10 + k * 34
                bh = rng.randint(50, 78)
                d.rectangle([x, y + 80 - bh, x + 20, y + 80], fill=rng.choice([(140, 80, 20), (60, 90, 40), (180, 170, 140), (90, 20, 20)]))
        d.rectangle([0, 390, w, h], fill=(50, 28, 16))
    img = img.filter(ImageFilter.GaussianBlur(1.2))
    save(img, f"T_Interior_{i:02d}", "interior", interior=kind)


# --- LED dot-matrix marquee (scrolls in material) -----------------------------------------------
def marquee(i):
    w, h = 2048, 128
    txt = rng.choice([
        "★ いらっしゃいませ ★ 歌舞伎町へようこそ ★ WELCOME TO KABUKICHO ★ 本日も元気に営業中 ★",
        "◆ カラオケ 30分 ¥100 ◆ 学生割引 ◆ 飲み放題プラン ◆ 朝までコース ◆",
        "▶ 新宿 東口 徒歩3分 ▶ 24時間営業 ▶ TAX FREE ▶ 免税 ▶",
        "● ホスト求人 ● 未経験歓迎 ● 日払いOK ● 寮完備 ●",
    ])
    col = rng.choice([(255, 60, 30), (255, 200, 30), (60, 255, 90), (255, 255, 255)])
    txtimg = Image.new("L", (w, h), 0)
    td = ImageDraw.Draw(txtimg)
    f = fit_horizontal(txt, "bold", w - 20, 100)
    td.text((w / 2, h / 2), txt, font=f, fill=255, anchor="mm")
    small = txtimg.resize((w // 8, h // 8), Image.BILINEAR)
    img = Image.new("RGB", (w, h), (0, 0, 0))
    d = ImageDraw.Draw(img)
    for y in range(small.height):
        for x in range(small.width):
            v = small.getpixel((x, y))
            if v > 70:
                d.ellipse([x * 8 + 1, y * 8 + 1, x * 8 + 7, y * 8 + 7], fill=col)
            else:
                d.ellipse([x * 8 + 2, y * 8 + 2, x * 8 + 6, y * 8 + 6], fill=(18, 14, 12))
    save(img, f"T_Marquee_{i:02d}", "marquee")


# --- standing A-board signs ---------------------------------------------------------------------
def aboard(i):
    w, h = 256, 384
    bg, fg, accent = rng.choice(LIGHTBOX)
    img = Image.new("RGB", (w, h), bg)
    d = ImageDraw.Draw(img)
    typ, style = rng.choice(SHOP_TYPES)
    f = fit_horizontal(typ, style, w - 30, 110)
    d.text((w / 2, 90), typ, font=f, fill=fg, anchor="mm")
    d.rectangle([0, 170, w, 176], fill=accent)
    for k, line in enumerate(rng.sample(["生ビール ¥390", "ハイボール ¥290", "60分 ¥1,000", "お通しなし", "女性半額", "席あります", "→ 2F", "OPEN"], 3)):
        d.text((w / 2, 215 + k * 52), line, font=pick_font("bold", 30), fill=fg, anchor="mm")
    img = lightbox_shading(img, edge=0.15)
    save(img, f"T_ABoard_{i:02d}", "aboard")


# --- road markings -----------------------------------------------------------------------------
def road_marking():
    w, h = 1024, 512
    img = Image.new("L", (w, h), 0)
    d = ImageDraw.Draw(img)
    f = pick_font("heavy", 300)
    # 止まれ is painted elongated along the driving direction
    tmp = Image.new("L", (900, 360), 0)
    td = ImageDraw.Draw(tmp)
    td.text((450, 180), "止まれ", font=f, fill=255, anchor="mm")
    tmp = tmp.resize((w - 60, h - 80))
    img.paste(tmp, (30, 40))
    img.save(os.path.join(OUT, "T_Road_Tomare.png"))
    manifest.append({"name": "T_Road_Tomare", "kind": "decal", "size": [w, h], "glow": [1, 1, 1]})


if __name__ == "__main__":
    for i in range(44):
        vertical_sign(i)
    for i in range(14):
        tower_sign(i)
    for i in range(36):
        horizontal_sign(i)
    for i in range(10):
        billboard(i)
    gate_sign()
    for i in range(5):
        vending(i)
    for i in range(8):
        lantern(i)
    for i in range(5):
        noren(i)
    for i in range(8):
        interior(i)
    for i in range(4):
        marquee(i)
    for i in range(10):
        aboard(i)
    road_marking()
    with open(os.path.join(OUT, "manifest.json"), "w") as fh:
        json.dump(manifest, fh, indent=1, ensure_ascii=False)
    print(f"wrote {len(manifest)} textures to {OUT}")
