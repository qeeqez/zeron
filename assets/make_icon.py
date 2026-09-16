#!/usr/bin/env python3
"""Regenerate the Rixl Code app icon (assets/icon.png + assets/icon.icns).

Draws the mark programmatically — a mint `>_` terminal glyph on a dark
rounded-rect gradient — using signed-distance fields, so every size renders
with analytic antialiasing (no resampling). Writes a PNG for embedding and a
.iconset -> .icns via `iconutil` for a future .app bundle.

Usage: python3 assets/make_icon.py   (run from the repo root; macOS for .icns)
"""

import math
import os
import shutil
import struct
import subprocess
import sys
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
ICONSET = os.path.join(HERE, "icon.iconset")

# Icon geometry in a 1024-unit square. The rounded rect bleeds to the edges —
# macOS applies its own mask — and the glyph is a `>` chevron plus `_` bar.
RADIUS = 232.0
TOP = (0x1B, 0x24, 0x37)  # gradient top: slate blue
BOTTOM = (0x0D, 0x11, 0x17)  # gradient bottom: near-black
INK = (0x34, 0xD3, 0x99)  # glyph: mint
CHEVRON = ((268.0, 296.0), (502.0, 468.0), (268.0, 640.0))  # apex mid-right
BAR = (584.0, 812.0, 640.0)  # x0, x1, center-y
STROKE = 112.0


def sd_segment(px, py, ax, ay, bx, by):
    """Distance from (px, py) to the segment a->b."""
    vx, vy = bx - ax, by - ay
    t = max(0.0, min(1.0, ((px - ax) * vx + (py - ay) * vy) / (vx * vx + vy * vy)))
    dx, dy = px - (ax + t * vx), py - (ay + t * vy)
    return math.hypot(dx, dy)


def sd_rounded_box(px, py, half, r):
    """Distance to a rounded box centered on the canvas, half-size `half`."""
    qx, qy = abs(px - half * 2) - (half * 2 - r), abs(py - half * 2) - (half * 2 - r)
    ax, ay = max(qx, 0.0), max(qy, 0.0)
    return math.hypot(ax, ay) + min(max(qx, qy), 0.0) - r


def coverage(d):
    """Antialiased coverage for a signed distance in pixels."""
    return max(0.0, min(1.0, 0.5 - d))


def render(size):
    """Render the icon at `size`x`size`; returns RGBA bytes."""
    scale = size / 1024.0
    half = 512.0 * scale
    radius = RADIUS * scale
    stroke = STROKE * scale / 2.0
    (ax, ay), (bx, by), (cx_, cy) = [(x * scale, y * scale) for x, y in CHEVRON]
    bar = (BAR[0] * scale, BAR[1] * scale, BAR[2] * scale)
    out = bytearray(size * size * 4)
    i = 0
    for y in range(size):
        py = y + 0.5
        t = py / size
        bg = tuple(round(TOP[c] + (BOTTOM[c] - TOP[c]) * t) for c in range(3))
        for x in range(size):
            px = x + 0.5
            a_bg = coverage(sd_rounded_box(px, py, half, radius))
            d_ink = min(
                sd_segment(px, py, ax, ay, bx, by),
                sd_segment(px, py, bx, by, cx_, cy),
                sd_segment(px, py, bar[0], bar[2], bar[1], bar[2]),
            ) - stroke
            a_ink = coverage(d_ink) * a_bg
            r = INK[0] * a_ink + bg[0] * (a_bg - a_ink)
            g = INK[1] * a_ink + bg[1] * (a_bg - a_ink)
            b = INK[2] * a_ink + bg[2] * (a_bg - a_ink)
            out[i : i + 4] = bytes((round(r), round(g), round(b), round(a_bg * 255)))
            i += 4
    return bytes(out)


def write_png(path, size, rgba):
    """Write RGBA bytes as a PNG (no dependencies — zlib + struct only)."""
    def chunk(tag, data):
        body = tag + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    rows = b"".join(b"\x00" + rgba[y * size * 4 : (y + 1) * size * 4] for y in range(size))
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", ihdr) + chunk(b"IDAT", zlib.compress(rows, 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)


def main():
    sizes = [16, 32, 64, 128, 256, 512, 1024]
    os.makedirs(ICONSET, exist_ok=True)
    for s in sizes:
        write_png(os.path.join(ICONSET, f"icon_{s}x{s}.png"), s, render(s))
        print(f"iconset: {s}x{s}")
    # The embedded runtime icon (dock + About) — 512 is plenty for both.
    write_png(os.path.join(HERE, "icon.png"), 512, render(512))
    print("assets/icon.png (512x512)")
    if sys.platform == "darwin" and shutil.which("iconutil"):
        subprocess.run(["iconutil", "-c", "icns", ICONSET, "-o", os.path.join(HERE, "icon.icns")], check=True)
        print("assets/icon.icns")
    else:
        print("iconutil unavailable — skipped icon.icns", file=sys.stderr)
    shutil.rmtree(ICONSET)


if __name__ == "__main__":
    main()
