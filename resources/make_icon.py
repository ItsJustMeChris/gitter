"""Generates resources/gitter.ico and resources/gitter.png (the app icon).

Run with: python resources/make_icon.py
"""
from pathlib import Path

from PIL import Image, ImageDraw

S = 1024  # draw large, downsample for crisp edges
img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(img)

# Rounded square with a subtle vertical gradient.
top, bottom = (77, 141, 246), (47, 107, 214)
mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle((40, 40, S - 40, S - 40), radius=230, fill=255)
grad = Image.new("RGBA", (S, S))
gd = ImageDraw.Draw(grad)
for y in range(S):
    t = y / (S - 1)
    gd.line([(0, y), (S, y)], fill=tuple(int(top[i] + (bottom[i] - top[i]) * t) for i in range(3)) + (255,))
img.paste(grad, (0, 0), mask)

# Branch glyph: two commits on a trunk and a branch curving off to a third.
w = 64
white = (255, 255, 255, 255)
x1, x2 = 370, 660
y_top, y_bot, y_branch = 300, 724, 360
d.line([(x1, y_top), (x1, y_bot)], fill=white, width=w)
# Curve from the branch commit back into the trunk.
pts = []
for i in range(41):
    t = i / 40
    # cubic bezier from (x2, y_branch) to (x1, 620)
    p0, p1, p2, p3 = (x2, y_branch), (x2, 560), (x1, 480), (x1, 640)
    x = (1 - t) ** 3 * p0[0] + 3 * (1 - t) ** 2 * t * p1[0] + 3 * (1 - t) * t ** 2 * p2[0] + t ** 3 * p3[0]
    y = (1 - t) ** 3 * p0[1] + 3 * (1 - t) ** 2 * t * p1[1] + 3 * (1 - t) * t ** 2 * p2[1] + t ** 3 * p3[1]
    pts.append((x, y))
d.line(pts, fill=white, width=w, joint="curve")
for cx, cy in [(x1, y_top), (x1, y_bot), (x2, y_branch)]:
    r = 92
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=white)
    r2 = 44
    d.ellipse((cx - r2, cy - r2, cx + r2, cy + r2), fill=(47, 107, 214, 255))

out = Path(__file__).parent
big = img.resize((256, 256), Image.LANCZOS)
big.save(out / "gitter.png")
big.save(out / "gitter.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
print("wrote", out / "gitter.ico")
