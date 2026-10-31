"""Draws utsuri/assets/utsuri.{png,ico,icns}: python3 tools/make_icon.py utsuri/assets/utsuri (needs Pillow and Noto Sans CJK)."""
import sys
from PIL import Image, ImageDraw, ImageFont, ImageFilter, ImageChops
S = 1024
out = sys.argv[1]
def lerp(a, b, t): return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3))
# Diagonal gradient: teal -> indigo -> violet.
grad = Image.new("RGB", (S, S))
px = grad.load()
c1, c2, c3 = (20, 184, 166), (79, 70, 229), (147, 51, 234)
for y in range(S):
    for x in range(S):
        t = (x + y) / (2 * S)
        px[x, y] = lerp(c1, c2, t / 0.6) if t < 0.6 else lerp(c2, c3, (t - 0.6) / 0.4)
pad, r = 64, 230
mask = Image.new("L", (S, S), 0)
ImageDraw.Draw(mask).rounded_rectangle((pad, pad, S - pad, S - pad), r, fill=255)
icon = Image.new("RGBA", (S, S), (0, 0, 0, 0))
# Soft shadow.
sh = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(sh).rounded_rectangle((pad, pad + 18, S - pad, S - pad + 18), r, fill=(0, 0, 0, 110))
icon = Image.alpha_composite(icon, sh.filter(ImageFilter.GaussianBlur(22)))
tile = grad.convert("RGBA"); tile.putalpha(mask)
icon = Image.alpha_composite(icon, tile)
# Glass highlight on the top half.
hl = Image.new("L", (S, S), 0)
ImageDraw.Draw(hl).ellipse((-S * 0.25, -S * 0.75, S * 1.25, S * 0.48), fill=60)
hl = ImageChops.multiply(hl, mask)
white = Image.new("RGBA", (S, S), (255, 255, 255, 0)); white.putalpha(hl)
icon = Image.alpha_composite(icon, white)
# The glyph 移 (utsuru: to move, to shift).
font = ImageFont.truetype("/usr/share/fonts/noto-cjk/NotoSansCJK-Bold.ttc", 600, index=0)
g = Image.new("RGBA", (S, S), (0, 0, 0, 0))
d = ImageDraw.Draw(g)
bb = d.textbbox((0, 0), "移", font=font)
x = (S - (bb[2] - bb[0])) / 2 - bb[0]; y = (S - (bb[3] - bb[1])) / 2 - bb[1]
gs = Image.new("RGBA", (S, S), (0, 0, 0, 0))
ImageDraw.Draw(gs).text((x, y + 14), "移", font=font, fill=(20, 10, 60, 120))
icon = Image.alpha_composite(icon, gs.filter(ImageFilter.GaussianBlur(14)))
d.text((x, y), "移", font=font, fill=(255, 255, 255, 255))
icon = Image.alpha_composite(icon, g)
icon.save(out + ".png")
icon.save(out + ".ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
icon.save(out + ".icns")
