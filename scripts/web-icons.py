"""The sync server page's icons (src/icons/): the app's icon (src-tauri/icons/icon.png) on a square of
its dark outer ring's colour, so a phone's home screen shows the target edge to edge.

    python3 scripts/web-icons.py
"""

from pathlib import Path

from PIL import Image, ImageDraw

RING = (27, 26, 41, 255)  # #1b1a29, the outer ring
ROOT = Path(__file__).resolve().parent.parent

icon = Image.open(ROOT / "src-tauri/icons/icon.png").convert("RGBA")
square = Image.new("RGBA", icon.size, RING)
square.alpha_composite(icon)
square = square.convert("RGB")
# The target sits on a white disc: everything beyond the light blue ring (its edge at 30% of the
# width from the centre) takes the dark ring's colour, so the ring runs to the edges without seams.
centre, edge = icon.width / 2, icon.width * 0.31
outside = Image.new("L", icon.size, 255)
ImageDraw.Draw(outside).ellipse((centre - edge, centre - edge, centre + edge, centre + edge), fill=0)
square.paste(RING[:3], (0, 0), outside)
for size in (32, 180, 192, 512):
    square.resize((size, size), Image.LANCZOS).save(ROOT / f"src/icons/icon-{size}.png", optimize=True)
