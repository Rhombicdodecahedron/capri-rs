"""Builds the README media from frames rendered by the real renderer.

    cargo run --release --example readme_frames -- target/readme-frames
    python tools/readme_media.py target/readme-frames
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[1]
FRAMES = Path(sys.argv[1] if len(sys.argv) > 1 else ROOT / "target" / "readme-frames")
OUT = ROOT / "docs" / "media"
FONT = ImageFont.truetype(str(ROOT / "assets" / "fonts" / "ShareTechMono-Regular.ttf"), 15)
FONT_S = ImageFont.truetype(str(ROOT / "assets" / "fonts" / "ShareTechMono-Regular.ttf"), 13)
ANCHOR = (240, 460)  # where her anchor sits on a rendered frame (printed by the example)
RED, CYAN, NIGHT = (255, 0, 60), (0, 240, 255), (10, 10, 18)


def wallpaper(w, h):
    """Night-city gradient with faint scanlines, like a desktop at 2 a.m."""
    img = Image.new("RGB", (w, h))
    d = ImageDraw.Draw(img)
    for y in range(h):
        k = y / h
        d.line([(0, y), (w, y)], fill=(int(14 + 30 * k), int(10 + 6 * k), int(30 + 28 * k)))
    for y in range(0, h, 3):
        d.line([(0, y), (w, y)], fill=(0, 0, 0))
    # skyline
    x = 0
    heights = [60, 110, 80, 150, 95, 70, 130, 88, 170, 66, 120, 90, 140, 75, 100]
    i = 0
    while x < w:
        bw = 40 + (i * 37) % 50
        bh = heights[i % len(heights)]
        d.rectangle([x, h - bh, x + bw, h], fill=(8, 8, 16))
        for wy in range(h - bh + 10, h - 8, 14):
            for wx in range(x + 6, x + bw - 6, 12):
                if (wx * 7 + wy * 3 + i) % 5 == 0:
                    d.rectangle([wx, wy, wx + 4, wy + 6], fill=(255, 0, 60) if (wx + wy) % 3 == 0 else (0, 160, 180))
        x += bw + 4
        i += 1
    return img


def window(d, box, title, lines):
    x0, y0, x1, y1 = box
    d.rectangle(box, fill=(13, 14, 22), outline=(0, 120, 135))
    d.rectangle([x0, y0, x1, y0 + 26], fill=(20, 21, 34))
    d.text((x0 + 12, y0 + 5), title, font=FONT_S, fill=(150, 160, 175))
    for k, c in enumerate([(255, 95, 86), (255, 189, 46), (39, 201, 63)]):
        d.ellipse([x1 - 70 + k * 20, y0 + 8, x1 - 60 + k * 20, y0 + 18], fill=c)
    for k, (txt, col) in enumerate(lines):
        d.text((x0 + 14, y0 + 40 + k * 21), txt, font=FONT, fill=col)


def place(base, frame, ax, ay):
    """Pastes a rendered frame so her anchor lands on (ax, ay)."""
    base.alpha_composite(frame, (ax - ANCHOR[0], ay - ANCHOR[1]))


def save_gif(frames, path, fps=12):
    pal = [f.convert("RGB").quantize(colors=128, method=Image.Quantize.MEDIANCUT, dither=Image.Dither.NONE) for f in frames]
    pal[0].save(path, save_all=True, append_images=pal[1:], duration=int(1000 / fps), loop=0, optimize=True)


def hero():
    w, h = 960, 440
    bg = wallpaper(w, h).convert("RGBA")
    d = ImageDraw.Draw(bg)
    win = (120, 190, 700, 410)
    window(d, win, "~/capri-rs - cargo", [
        ("$ cargo run --release", (230, 247, 255)),
        ("   Compiling capri-domain v0.1.0", (120, 200, 140)),
        ("   Compiling capri-desk-winit v0.1.0", (120, 200, 140)),
        ("    Finished `release` in 41.2s", (120, 200, 140)),
        ("     Running `target/release/capri`", (230, 247, 255)),
        ("INFO capri: Capri is on the desk", (0, 240, 255)),
    ])
    # taskbar
    d.rectangle([0, h - 30, w, h], fill=(6, 6, 11))
    d.line([(0, h - 30), (w, h - 30)], fill=RED, width=1)
    d.text((14, h - 23), "CAPRI // ONLINE", font=FONT_S, fill=RED)
    frames = []
    for i in range(48):
        f = Image.open(FRAMES / f"perch-{i:03d}.png").convert("RGBA")
        img = bg.copy()
        place(img, f, 430, win[1])
        frames.append(img)
    save_gif(frames, OUT / "hero.gif")


def talk():
    w, h = 560, 400
    bg = wallpaper(w, h).convert("RGBA")
    d = ImageDraw.Draw(bg)
    d.rectangle([0, h - 30, w, h], fill=(6, 6, 11))
    d.line([(0, h - 30), (w, h - 30)], fill=CYAN, width=1)
    frames = []
    for i in range(48):
        f = Image.open(FRAMES / f"talk-{i:03d}.png").convert("RGBA")
        img = bg.copy()
        place(img, f, 280, h - 30)
        frames.append(img)
    save_gif(frames, OUT / "talk.gif")


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    hero()
    talk()
    for p in sorted(OUT.iterdir()):
        print(p.name, p.stat().st_size // 1024, "KB")
