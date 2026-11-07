#!/usr/bin/env python3
import math
import struct
import sys
import zlib

INK = (0xE4, 0xE6, 0xEA)
BODY = (1.0, 1.0, 14.0, 14.0, 4.2)
STROKE = 1.9
SEGMENTS = [
    (5.33, 3.2, 5.33, 12.8),
    (5.33, 3.2, 7.47, 3.2),
    (7.47, 7.47, 5.33, 7.47),
    (5.33, 7.47, 8.0, 7.47),
    (8.0, 12.8, 5.33, 12.8),
]
ARCS = [(7.47, 5.335, 2.135), (8.0, 10.135, 2.665)]


def segment_distance(x, y, x0, y0, x1, y1):
    dx, dy = x1 - x0, y1 - y0
    t = ((x - x0) * dx + (y - y0) * dy) / (dx * dx + dy * dy)
    t = max(0.0, min(1.0, t))
    return math.hypot(x - x0 - t * dx, y - y0 - t * dy)


def arc_distance(x, y, cx, cy, r):
    if x >= cx:
        return abs(math.hypot(x - cx, y - cy) - r)
    return min(math.hypot(x - cx, y - cy - r), math.hypot(x - cx, y - cy + r))


def body_distance(x, y):
    bx, by, bw, bh, r = BODY
    qx = abs(x - bx - bw / 2) - bw / 2 + r
    qy = abs(y - by - bh / 2) - bh / 2 + r
    return math.hypot(max(qx, 0.0), max(qy, 0.0)) + min(max(qx, qy), 0.0) - r


def coverage(distance, units_per_pixel):
    return max(0.0, min(1.0, 0.5 - distance / units_per_pixel))


def mark(size):
    unit = 16.0 / size
    alpha = bytearray(size * size)
    for row in range(size):
        y = (row + 0.5) * unit
        for column in range(size):
            x = (column + 0.5) * unit
            body = coverage(body_distance(x, y), unit)
            if body <= 0.0:
                continue
            stroke = min(
                min(segment_distance(x, y, *segment) for segment in SEGMENTS),
                min(arc_distance(x, y, *arc) for arc in ARCS),
            )
            knocked = coverage(stroke - STROKE / 2, unit)
            alpha[row * size + column] = round(255 * body * (1.0 - knocked))
    return bytes(alpha)


def chunk(kind, data):
    return (
        struct.pack(">I", len(data))
        + kind
        + data
        + struct.pack(">I", zlib.crc32(kind + data))
    )


def on_black(alpha):
    return [tuple(channel * value // 255 for channel in INK) for value in alpha]


def write_bootloader(directory, size, alpha):
    pixels = on_black(alpha)
    with open(f"{directory}/bootlogo_{size}.bin", "wb") as raw:
        raw.write(b"".join(bytes((*pixel, 255)) for pixel in pixels))
    rows = bytearray()
    for row in range(size):
        rows.append(0)
        for value in alpha[row * size : (row + 1) * size]:
            rows += bytes((*INK, value))
    with open(f"{directory}/bootlogo_{size}.png", "wb") as png:
        png.write(b"\x89PNG\r\n\x1a\n")
        png.write(chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0)))
        png.write(chunk(b"IDAT", zlib.compress(bytes(rows), 9)))
        png.write(chunk(b"IEND", b""))


def write_console_logo(path, size, alpha):
    levels = sorted({value * 223 // 255 for value in alpha})
    with open(path, "w") as ppm:
        ppm.write(f"P3\n{size} {size}\n255\n")
        pixels = on_black(bytes(value * 223 // 255 * 255 // 223 for value in alpha))
        for start in range(0, len(pixels), size):
            ppm.write(" ".join(f"{r} {g} {b}" for r, g, b in pixels[start : start + size]))
            ppm.write("\n")
    return len(levels)


def main():
    bootloader, console, alpha_map = sys.argv[1:4]
    large = mark(256)
    with open(alpha_map, "wb") as out:
        out.write(large)
    for size in (48, 128, 256):
        write_bootloader(bootloader, size, mark(size))
    print(write_console_logo(console, 128, mark(128)), "console logo levels")


if __name__ == "__main__":
    main()
