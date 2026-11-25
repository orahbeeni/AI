"""Makes the app icon from design/images/appimage.png (the logo with the word "Organise" under it).

Cuts the mark out on its navy background, continues the background where the word was, and rounds the
corners. Writes design/images/icon.png (1024 px, transparent corners) and app/public/logo.png (in-app logo).
Then the platform icons come from:  cd app && npx tauri icon ../design/images/icon.png

    python3 design/tools/make_icon.py
"""
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "design/images/appimage.png"
OUT = ROOT / "design/images/icon.png"
LOGO = ROOT / "app/public/logo.png"

img = np.asarray(Image.open(SRC).convert("RGB")).astype(float)
H, W, _ = img.shape
r, g, b = img[..., 0], img[..., 1], img[..., 2]

# the mark: teal/blue, well above the navy background (the word below it is white, so it doesn't match)
mark = ((g - r) > 45) & ((b - r) > 45) & (g + b > 230)
mark[int(H * 0.66):] = False
ys, xs = np.nonzero(mark)
cx, cy = (xs.min() + xs.max()) / 2, (ys.min() + ys.max()) / 2
side = max(np.ptp(xs), np.ptp(ys)) / 0.78  # the mark fills 78% of the tile
x0, y0 = int(round(cx - side / 2)), int(round(cy - side / 2))
n = int(round(side))

# the word starts below the mark: everything from just above it is replaced by background
white = (r > 170) & (g > 170) & (b > 170)
white[: ys.max() + 10] = False
word_top = np.nonzero(white.any(axis=1))[0].min()
fade_from, fade_to = word_top - 34, word_top - 14

# background as a smooth gradient fitted to pixels well away from the mark and the word
yy, xx = np.mgrid[0:H, 0:W]
away = ~mark & ~white
grow = 40
away &= ~((xx > xs.min() - grow) & (xx < xs.max() + grow) & (yy > ys.min() - grow) & (yy < word_top + 90))
pts = np.column_stack([xx[away], yy[away], np.ones(away.sum())])
coef = [np.linalg.lstsq(pts, img[..., c][away], rcond=None)[0] for c in range(3)]

ty, tx = np.mgrid[y0 : y0 + n, x0 : x0 + n]
bg = np.stack([coef[c][0] * tx + coef[c][1] * ty + coef[c][2] for c in range(3)], axis=-1)
crop = np.zeros((n, n, 3))
inside = (ty >= 0) & (ty < H) & (tx >= 0) & (tx < W)
crop[inside] = img[ty[inside], tx[inside]]
keep = np.clip((fade_to - ty) / (fade_to - fade_from), 0, 1)[..., None] * inside[..., None]
tile = crop * keep + bg * (1 - keep)

def rounded(size: int, margin: int) -> Image.Image:
    """The tile at `size` px with rounded corners and a transparent margin (anti-aliased)."""
    big = size * 4
    inner = big - 2 * margin * 4
    face = Image.fromarray(tile.clip(0, 255).astype(np.uint8)).resize((inner, inner), Image.LANCZOS)
    mask = Image.new("L", (inner, inner), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, inner - 1, inner - 1], radius=int(inner * 0.225), fill=255)
    out = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    out.paste(face, (margin * 4, margin * 4), mask)
    return out.resize((size, size), Image.LANCZOS)

rounded(1024, 40).save(OUT)
LOGO.parent.mkdir(parents=True, exist_ok=True)
rounded(128, 0).save(LOGO)
print(f"mark {xs.min()}-{xs.max()} x {ys.min()}-{ys.max()}, word from y {word_top}, tile {n}px -> {OUT.name}, {LOGO.name}")
