#!/usr/bin/env python3
"""Export the existing Sidera artwork with its rounded tile as the alpha edge.

Requires Pillow. Run from any directory; all shipped icon variants share this
source and mask, including the high-resolution bitmap embedded in the SVG.
"""
from base64 import b64encode
from io import BytesIO
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[1]


def main():
    artwork = Image.open(ROOT / "assets/source/sidera-artwork.png").convert("RGBA")
    if artwork.size != (512, 512):
        raise ValueError("The original artwork must be 512 by 512 pixels")

    # Follow the original tile's outside contour, retaining its fine outline.
    # Supersampling avoids stair steps; no background colour is baked into alpha.
    samples = 8
    mask = Image.new("L", (512 * samples, 512 * samples), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        tuple(int(v * samples) for v in (66, 66, 446, 446)),
        radius=80 * samples,
        fill=255,
    )
    artwork.putalpha(mask.resize(artwork.size, Image.Resampling.LANCZOS))
    tile = artwork.crop((64, 64, 448, 448)).resize((480, 480), Image.Resampling.LANCZOS)
    icon = Image.new("RGBA", (512, 512))
    icon.alpha_composite(tile, (16, 16))

    icon.resize((256, 256), Image.Resampling.LANCZOS).save(ROOT / "sidera.png")
    icon.resize((88, 88), Image.Resampling.LANCZOS).save(
        ROOT / "docs/assets/screenshots/sidera-icon.png"
    )
    icon.save(
        ROOT / "windows/assets/sidera.ico",
        sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)],
    )
    png = BytesIO()
    icon.save(png, format="PNG")
    encoded = b64encode(png.getvalue()).decode("ascii")
    (ROOT / "sidera.svg").write_text(
        '<svg xmlns="http://www.w3.org/2000/svg" '
        'xmlns:xlink="http://www.w3.org/1999/xlink" '
        'width="512" height="512" viewBox="0 0 512 512">'
        '<title>Sidera</title>'
        '<desc>Original Sidera artwork with a transparent, antialiased rounded tile. '
        'This SVG contains a high-resolution bitmap.</desc>'
        '<image width="512" height="512" '
        f'xlink:href="data:image/png;base64,{encoded}"/></svg>\n',
        encoding="utf-8",
    )
    print("Updated PNG, SVG, Windows ICO and documentation icon.")


if __name__ == "__main__":
    main()
