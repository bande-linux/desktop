import math
import os
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), '..', '..', '..'))
sys.path.insert(0, os.path.join(ROOT, 'docs', 'design', 'datum', 'src'))

from colors import T, oklch

PAPER = T['light.sheet']
INK = T['light.ink']
INK2 = T['light.ink2']
INK3 = T['light.ink3']
EMBER = T['accent.light.ember']
STONE = '#D9D3CA'
STONE_SHADE = '#C9C2B7'
ENGRAVE = '#8A8276'

ACCENTS = ['ultramarine', 'teal', 'moss', 'amber', 'ember', 'rose', 'plum', 'graphite']


def accent(name):
    return T['accent.light.' + name]


def semantic(name):
    return T['sem.light.' + name]


def fmt(value):
    text = f'{value:.3f}'.rstrip('0').rstrip('.')
    return '0' if text in ('-0', '') else text


def svg(size, body, defs=''):
    defs = f'<defs>{defs}</defs>' if defs else ''
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" '
            f'viewBox="0 0 {size} {size}">{defs}{body}</svg>\n')


def gradient(ident, top, bottom, x1=0, y1=0, x2=0, y2=1):
    return (f'<linearGradient id="{ident}" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}">'
            f'<stop offset="0" stop-color="{top}"/><stop offset="1" stop-color="{bottom}"/></linearGradient>')


def tone(hue, lightness, chroma):
    return oklch(lightness, chroma, hue)


def mix(a, b, t):
    pa = [int(a[i:i + 2], 16) for i in (1, 3, 5)]
    pb = [int(b[i:i + 2], 16) for i in (1, 3, 5)]
    return '#%02X%02X%02X' % tuple(round(x + (y - x) * t) for x, y in zip(pa, pb))


def path(d, fill, extra=''):
    return f'<path d="{d}" fill="{fill}"{" " + extra if extra else ""}/>'


def stroke(d, color, width, extra=''):
    return (f'<path d="{d}" fill="none" stroke="{color}" stroke-width="{fmt(width)}" '
            f'stroke-linecap="round" stroke-linejoin="round"{" " + extra if extra else ""}/>')


def rect(x, y, w, h, r, fill, extra=''):
    return (f'<rect x="{fmt(x)}" y="{fmt(y)}" width="{fmt(w)}" height="{fmt(h)}" rx="{fmt(r)}" '
            f'fill="{fill}"{" " + extra if extra else ""}/>')


def circle(cx, cy, r, fill, extra=''):
    return f'<circle cx="{fmt(cx)}" cy="{fmt(cy)}" r="{fmt(r)}" fill="{fill}"{" " + extra if extra else ""}/>'


class Shape:
    def __init__(self):
        self.parts = []

    def d(self):
        return ''.join(self.parts)

    def polygon(self, points, clockwise=True):
        area = sum(x1 * y2 - x2 * y1 for (x1, y1), (x2, y2) in zip(points, points[1:] + points[:1]))
        if (area > 0) != clockwise:
            points = points[::-1]
        self.parts.append('M' + ' L'.join(f'{fmt(x)} {fmt(y)}' for x, y in points) + 'Z')
        return self

    def circle(self, cx, cy, r, clockwise=True):
        sweep = 1 if clockwise else 0
        self.parts.append(f'M{fmt(cx + r)} {fmt(cy)}A{fmt(r)} {fmt(r)} 0 1 {sweep} {fmt(cx - r)} {fmt(cy)}'
                          f'A{fmt(r)} {fmt(r)} 0 1 {sweep} {fmt(cx + r)} {fmt(cy)}Z')
        return self

    def rrect(self, x, y, w, h, r, clockwise=True):
        r = min(r, w / 2, h / 2)
        if r <= 0:
            return self.polygon([(x, y), (x + w, y), (x + w, y + h), (x, y + h)], clockwise)
        s = 1 if clockwise else 0
        if clockwise:
            d = (f'M{fmt(x + r)} {fmt(y)}H{fmt(x + w - r)}A{fmt(r)} {fmt(r)} 0 0 {s} {fmt(x + w)} {fmt(y + r)}'
                 f'V{fmt(y + h - r)}A{fmt(r)} {fmt(r)} 0 0 {s} {fmt(x + w - r)} {fmt(y + h)}'
                 f'H{fmt(x + r)}A{fmt(r)} {fmt(r)} 0 0 {s} {fmt(x)} {fmt(y + h - r)}'
                 f'V{fmt(y + r)}A{fmt(r)} {fmt(r)} 0 0 {s} {fmt(x + r)} {fmt(y)}Z')
        else:
            d = (f'M{fmt(x + r)} {fmt(y)}A{fmt(r)} {fmt(r)} 0 0 {s} {fmt(x)} {fmt(y + r)}'
                 f'V{fmt(y + h - r)}A{fmt(r)} {fmt(r)} 0 0 {s} {fmt(x + r)} {fmt(y + h)}'
                 f'H{fmt(x + w - r)}A{fmt(r)} {fmt(r)} 0 0 {s} {fmt(x + w)} {fmt(y + h - r)}'
                 f'V{fmt(y + r)}A{fmt(r)} {fmt(r)} 0 0 {s} {fmt(x + w - r)} {fmt(y)}Z')
        self.parts.append(d)
        return self

    def frame(self, x, y, w, h, r, width):
        half = width / 2
        self.rrect(x - half, y - half, w + width, h + width, r + half)
        self.rrect(x + half, y + half, w - width, h - width, max(r - half, 0), clockwise=False)
        return self

    def ring(self, cx, cy, r, width):
        self.circle(cx, cy, r + width / 2)
        self.circle(cx, cy, r - width / 2, clockwise=False)
        return self

    def line(self, x1, y1, x2, y2, width, caps=True):
        dx, dy = x2 - x1, y2 - y1
        length = math.hypot(dx, dy) or 1
        nx, ny = -dy / length * width / 2, dx / length * width / 2
        self.polygon([(x1 + nx, y1 + ny), (x2 + nx, y2 + ny), (x2 - nx, y2 - ny), (x1 - nx, y1 - ny)])
        if caps:
            self.circle(x1, y1, width / 2)
            self.circle(x2, y2, width / 2)
        return self

    def lines(self, d, width):
        for segment in d.split('M')[1:]:
            points = [tuple(float(v) for v in pair.split(',')) for pair in segment.split()]
            self.polyline(points, width)
        return self

    def polyline(self, points, width, closed=False):
        pairs = list(zip(points, points[1:] + (points[:1] if closed else [])))
        for (x1, y1), (x2, y2) in pairs:
            self.line(x1, y1, x2, y2, width, caps=False)
        for x, y in points:
            self.circle(x, y, width / 2)
        return self

    def arc(self, cx, cy, r, start, end, width, caps=True):
        a0, a1 = math.radians(start), math.radians(end)
        outer, inner = r + width / 2, r - width / 2
        large = 1 if (end - start) % 360 > 180 else 0
        p = lambda radius, a: (cx + radius * math.cos(a), cy + radius * math.sin(a))
        o0, o1, i1, i0 = p(outer, a0), p(outer, a1), p(inner, a1), p(inner, a0)
        self.parts.append(f'M{fmt(o0[0])} {fmt(o0[1])}A{fmt(outer)} {fmt(outer)} 0 {large} 1 {fmt(o1[0])} {fmt(o1[1])}'
                          f'L{fmt(i1[0])} {fmt(i1[1])}A{fmt(inner)} {fmt(inner)} 0 {large} 0 {fmt(i0[0])} {fmt(i0[1])}Z')
        if caps:
            for a in (a0, a1):
                c = p(r, a)
                self.circle(c[0], c[1], width / 2)
        return self

    def raw(self, d):
        self.parts.append(d)
        return self
