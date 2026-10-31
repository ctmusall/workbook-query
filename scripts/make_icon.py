"""Draw the Workbook Query icon: a spreadsheet with highlighted matching rows under a magnifier.

Writes wbquery/assets/icon.png (256 px, the window icon) and icon.ico (16-256 px, the Windows exe).
Run:  python scripts/make_icon.py
"""

from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "wbquery" / "assets"
S = 1024  # drawn large, then scaled down smoothly

GREEN = (30, 123, 74)
GRID = (203, 224, 210)
SHEET = (255, 255, 255)
MATCH = (255, 205, 64)
INK = (38, 53, 68)
GLASS = (214, 236, 255, 150)


def draw() -> Image.Image:
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # Sheet with a green header band.
    x0, y0, x1, y1 = 96, 80, 800, 880
    d.rounded_rectangle((x0, y0, x1, y1), radius=64, fill=SHEET, outline=GREEN, width=36)
    d.rounded_rectangle((x0, y0, x1, y0 + 170), radius=64, fill=GREEN)
    d.rectangle((x0, y0 + 110, x1, y0 + 170), fill=GREEN)

    # Grid: rows, with two "matched" rows highlighted.
    top, row_h = y0 + 170, 118
    rows = 6
    for r in range(rows):
        ry = top + r * row_h
        if r in (1, 3):
            d.rectangle((x0 + 18, ry + 6, x1 - 18, ry + row_h - 6), fill=MATCH)
        if r:
            d.line((x0 + 18, ry, x1 - 18, ry), fill=GRID, width=10)
    for cx in (x0 + 230, x0 + 470):
        d.line((cx, y0 + 170, cx, y1 - 20), fill=GRID, width=10)

    # Magnifier over the lower right: glass, ring, handle.
    cx, cy, r = 620, 640, 210
    glass = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(glass).ellipse((cx - r, cy - r, cx + r, cy + r), fill=GLASS)
    img.alpha_composite(glass)
    d = ImageDraw.Draw(img)
    d.ellipse((cx - r, cy - r, cx + r, cy + r), outline=INK, width=64)
    hx, hy = cx + int(r * 0.72), cy + int(r * 0.72)
    d.line((hx, hy, 960, 960), fill=INK, width=120)
    d.ellipse((960 - 60, 960 - 60, 960 + 60, 960 + 60), fill=INK)
    return img


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    img = draw()
    img.resize((256, 256), Image.LANCZOS).save(OUT / "icon.png")
    img.save(OUT / "icon.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])
    print(f"Wrote {OUT / 'icon.png'} and {OUT / 'icon.ico'}")


if __name__ == "__main__":
    main()
