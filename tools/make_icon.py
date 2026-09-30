from __future__ import annotations

import math
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
S = 1024


def draw() -> Image.Image:
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    m = 70
    d.rounded_rectangle([m, m, S - m, S - m], radius=150, fill=(250, 248, 242, 255), outline=(60, 72, 110, 255), width=36)
    for y in range(330, S - 150, 150):
        d.line([(m + 60, y), (S - m - 60, y)], fill=(170, 195, 240, 255), width=14)
    d.line([(300, m + 30), (300, S - m - 30)], fill=(235, 150, 150, 255), width=12)
    pts = []
    for i in range(0, 361):
        t = i / 360
        x = 330 + 420 * t
        y = 640 - 90 * math.sin(t * math.pi * 3) - 40 * t
        pts.append((x, y))
    r = 17
    for i in range(len(pts) - 1):
        (x0, y0), (x1, y1) = pts[i], pts[i + 1]
        for k in range(4):
            x, y = x0 + (x1 - x0) * k / 4, y0 + (y1 - y0) * k / 4
            d.ellipse([x - r, y - r, x + r, y + r], fill=(35, 35, 45, 255))
    tip = pts[-1]
    ang = math.radians(-50)
    ux, uy = math.cos(ang), math.sin(ang)
    nx, ny = -uy, ux
    w = 62

    def at(dist, off):
        return (tip[0] + ux * dist + nx * off, tip[1] + uy * dist + ny * off)
    d.polygon([at(0, 0), at(95, -w), at(95, w)], fill=(240, 205, 150, 255))
    d.polygon([at(0, 0), at(34, -w * 0.36), at(34, w * 0.36)], fill=(40, 40, 50, 255))
    d.polygon([at(95, -w), at(430, -w), at(430, w), at(95, w)], fill=(245, 180, 40, 255))
    d.polygon([at(95, -w * 0.33), at(430, -w * 0.33), at(430, w * 0.33), at(95, w * 0.33)], fill=(230, 160, 30, 255))
    d.polygon([at(430, -w), at(480, -w), at(480, w), at(430, w)], fill=(170, 170, 180, 255))
    d.polygon([at(480, -w), at(540, -w), at(540, w), at(480, w)], fill=(230, 120, 140, 255))
    return img


def main():
    big = draw()
    png = ROOT / "handwriter" / "static" / "icon.png"
    big.resize((256, 256), Image.LANCZOS).save(png)
    ico = ROOT / "assets" / "HandWriter.ico"
    ico.parent.mkdir(exist_ok=True)
    big.resize((256, 256), Image.LANCZOS).save(ico, sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64),
                                                            (128, 128), (256, 256)])
    print(png, ico)


if __name__ == "__main__":
    main()
