"""Draws the placeholder "demo" skin: an original neon construct, so the app runs before real art is added.

Same sheet format as the original capri-ai-desktop skins: one PNG per clip, a horizontal strip of frames,
standing frames 240x384, seated frames 384x384. Run: python tools/gen_demo_skin.py
"""
import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFilter

OUT = Path(__file__).resolve().parents[1] / "assets" / "skins" / "demo"
RED = (255, 0, 60)
CYAN = (0, 240, 255)
NIGHT = (14, 14, 26)
SKIN = (40, 40, 64)

CLIPS = {  # name: frames, frame width
    "idle": (16, 240), "blink": (4, 240), "blink_glitch": (4, 240), "grin": (16, 240), "glitch": (10, 240),
    "dangle": (24, 240), "sleep": (16, 240), "wake": (13, 240), "mouth": (5, 240),
    "perch": (16, 384), "perch_blink": (4, 384), "perch_blink_glitch": (4, 384), "perch_glitch": (10, 384),
}
H = 384


def glow(img):
    """Soft neon halo under the crisp drawing."""
    halo = img.filter(ImageFilter.GaussianBlur(6))
    out = Image.new("RGBA", img.size)
    out.alpha_composite(halo)
    out.alpha_composite(img)
    return out


def figure(d, cx, feet, t, eyes="open", arms="down", seated=False, mouth=None):
    """cx: centre x, feet: y of feet (standing) or hips (seated)."""
    bob = math.sin(t * 2 * math.pi) * 3
    hip_y = feet - (0 if seated else 120) + bob * 0.3
    # legs
    if seated:
        for dx in (-10, 12):
            d.line([(cx + dx, hip_y), (cx + dx + 46, hip_y + 4), (cx + dx + 50 + 6 * math.sin(t * 6.28 + dx), hip_y + 86)],
                   fill=SKIN, width=16, joint="curve")
            d.line([(cx + dx + 50, hip_y + 80), (cx + dx + 62, hip_y + 86)], fill=RED, width=8)
    else:
        swing = math.sin(t * 2 * math.pi) * (16 if arms == "up" else 0)
        for dx, s in ((-14, swing), (14, -swing)):
            d.line([(cx + dx, hip_y), (cx + dx + s, feet - 6)], fill=SKIN, width=18)
            d.line([(cx + dx + s - 10, feet - 4), (cx + dx + s + 10, feet - 4)], fill=RED, width=8)
    # torso
    top = hip_y - 110
    d.polygon([(cx - 34, top), (cx + 34, top), (cx + 26, hip_y), (cx - 26, hip_y)], fill=NIGHT, outline=CYAN, width=3)
    d.line([(cx - 20, top + 30), (cx + 20, top + 30)], fill=RED, width=4)
    # arms
    for side in (-1, 1):
        sx = cx + side * 36
        if arms == "up":
            d.line([(sx, top + 10), (sx + side * 18, top - 70 + bob)], fill=SKIN, width=14)
        else:
            d.line([(sx, top + 10), (sx + side * 10, top + 96 + bob)], fill=SKIN, width=14)
    # head
    hy = top - 70 + bob
    d.rounded_rectangle([cx - 40, hy, cx + 40, hy + 62], radius=18, fill=NIGHT, outline=CYAN, width=3)
    # hair spikes
    for i, dx in enumerate((-36, -18, 0, 18, 36)):
        d.polygon([(cx + dx - 10, hy + 6), (cx + dx + 10, hy + 6), (cx + dx + 4 * (i - 2), hy - 22 - (i % 2) * 10)], fill=RED)
    # visor
    vy = hy + 24
    if eyes == "open":
        d.rounded_rectangle([cx - 30, vy, cx + 30, vy + 14], radius=6, fill=CYAN)
        d.rectangle([cx - 18, vy + 4, cx - 10, vy + 10], fill=NIGHT)
        d.rectangle([cx + 10, vy + 4, cx + 18, vy + 10], fill=NIGHT)
    elif eyes == "grin":
        d.arc([cx - 28, vy - 10, cx + 28, vy + 16], 20, 160, fill=CYAN, width=6)
    else:
        d.line([(cx - 28, vy + 8), (cx + 28, vy + 8)], fill=CYAN, width=3)
    if mouth is not None:
        d.rectangle([cx - 10, hy + 46, cx + 10, hy + 46 + mouth], fill=RED)


def frame(name, i, n, fw):
    img = Image.new("RGBA", (fw, H))
    d = ImageDraw.Draw(img)
    t = i / n
    seated = name.startswith("perch")
    cx, base = (115, 273) if seated else (120, 378)
    if name == "mouth":
        figure(ImageDraw.Draw(Image.new("RGBA", (fw, H))), cx, base, 0)  # keep geometry identical, draw only the mouth
        hy = base - 120 - 110 - 70
        d.rectangle([cx - 10, hy + 46, cx + 10, hy + 46 + [0, 3, 6, 9, 5][i]], fill=RED)
        return img
    eyes = "open"
    if "blink" in name:
        eyes = "closed" if i in (1, 2) else "open"
    if name == "grin":
        eyes = "grin" if 3 <= i <= 12 else "open"
    if name == "sleep":
        eyes = "closed"
    if name == "wake":
        eyes = "closed" if i < 6 else "open"
    arms = "up" if name == "dangle" else "down"
    figure(d, cx, base, t, eyes=eyes, arms=arms, seated=seated)
    if name == "sleep":
        zz = (i % 8) * 4
        d.text((cx + 46, base - 330 - zz), "z", fill=CYAN)
        d.text((cx + 58, base - 346 - zz), "Z", fill=CYAN)
    img = glow(img)
    if "glitch" in name and (name.endswith("glitch") and (i % 3 != 2 or name in ("glitch", "perch_glitch"))):
        # slice the frame into bands and shift them, with a red/cyan split
        out = Image.new("RGBA", img.size)
        for y in range(0, H, 12):
            off = int(math.sin(i * 7 + y) * (14 if name in ("glitch", "perch_glitch") else 5))
            band = img.crop((0, y, fw, y + 12))
            out.alpha_composite(band, (max(0, off), y))
        r, g, b, a = out.split()
        img = Image.merge("RGBA", (r.transform(out.size, Image.AFFINE, (1, 0, 3, 0, 1, 0)), g, b, a))
    return img


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for name, (n, fw) in CLIPS.items():
        strip = Image.new("RGBA", (n * fw, H))
        for i in range(n):
            strip.alpha_composite(frame(name, i, n, fw), (i * fw, 0))
        strip.save(OUT / f"{name}.png", optimize=True)
    (OUT / "skin.toml").write_text(
        'name = "Demo construct"\n'
        'credits = "Placeholder art generated by tools/gen_demo_skin.py (MIT, part of this repository)"\n',
        encoding="utf-8",
    )
    print("wrote", OUT)


if __name__ == "__main__":
    main()
