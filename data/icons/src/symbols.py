import math
import re

from common import INK, Shape, fmt

W = 1.6
DIM = 0.32


def S():
    return Shape()


def layers(*items):
    out = []
    for item in items:
        if isinstance(item, Shape):
            out.append((item, 1, None))
        elif len(item) == 2:
            out.append((item[0], item[1], None))
        else:
            out.append(item)
    return out


def render(parts, color=INK):
    body = ''
    for shape, opacity, cls in parts:
        attrs = f' class="{cls}"' if cls else ''
        if opacity != 1:
            attrs += f' opacity="{fmt(opacity)}"'
        body += f'<path d="{shape.d()}" fill="{color}"{attrs}/>'
    return (f'<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">'
            f'{body}</svg>\n')


def slash():
    return S().line(2.5, 2.2, 13.8, 13.5, W)


WIFI = (3.2, 6.3, 9.4)


def wifi(lit, off=False):
    parts = [(S().circle(8, 13, 1.45), 1 if lit > 0 else DIM)]
    for index, radius in enumerate(WIFI):
        parts.append((S().arc(8, 13, radius, -135, -45, W), 1 if lit > index + 1 else DIM))
    if off:
        parts = [(shape, DIM) for shape, _ in parts] + [(slash(), 1)]
    return layers(*parts)


def wired(off=False):
    socket = S().frame(1.6, 2.6, 12.8, 10.8, 2, 1.4)
    socket.polygon([(4.4, 5.6), (11.6, 5.6), (11.6, 9.4), (10, 9.4), (10, 10.8), (6, 10.8), (6, 9.4), (4.4, 9.4)])
    if off:
        return layers((socket, DIM), slash())
    return layers(socket)


def shield_path():
    return 'M8 1.2L13.6 3.2V7.6C13.6 11 11.4 13.4 8 14.8C4.6 13.4 2.4 11 2.4 7.6V3.2Z'


def vpn():
    shape = S().raw(shield_path())
    shape.circle(8, 6.9, 1.5, clockwise=False)
    shape.polygon([(7.35, 7.4), (6.9, 10.8), (9.1, 10.8), (8.65, 7.4)], clockwise=False)
    return layers(shape)


def airplane():
    body = S().polygon([(7.2, 1.8), (8.8, 1.8), (8.9, 6.2), (14.5, 9.2), (14.5, 10.8), (8.9, 9.4), (8.8, 12.4),
                        (10.6, 13.6), (10.6, 14.6), (8, 14), (5.4, 14.6), (5.4, 13.6), (7.2, 12.4), (7.1, 9.4),
                        (1.5, 10.8), (1.5, 9.2), (7.1, 6.2)])
    body.circle(8, 2.4, 0.8)
    return layers(body)


SPEAKER = [(1.6, 5.7), (4.4, 5.7), (7.6, 2.6), (7.6, 13.4), (4.4, 10.3), (1.6, 10.3)]


def speaker():
    return S().polygon(SPEAKER).polyline(SPEAKER, 1.0, closed=True)


def volume(lit, muted=False):
    parts = [(speaker(), 1)]
    if muted:
        parts.append((S().line(10.3, 5.8, 14.5, 10, W).line(14.5, 5.8, 10.3, 10, W), 1))
        return layers(*parts)
    for index, radius in enumerate((2.8, 5.0, 7.2)):
        parts.append((S().arc(7.6, 8, radius, -48, 48, W), 1 if lit > index else DIM))
    return layers(*parts)


def microphone(off=False):
    shape = S().rrect(5.6, 1.4, 4.8, 8, 2.4).arc(8, 7.2, 4.4, 0, 180, W).line(8, 11.6, 8, 14.4, W)
    shape.line(5.4, 14.4, 10.6, 14.4, W)
    if off:
        return layers((shape, DIM), slash())
    return layers(shape)


def bolt(cx=8, cy=8, scale=1.0):
    points = [(0.6, -5.5), (-3, 0.8), (-0.4, 0.8), (-0.9, 5.5), (3, -1.2), (0.4, -1.2)]
    return S().polygon([(cx + x * scale, cy + y * scale) for x, y in points])


def battery(level, charging=False, missing=False):
    frame = S().frame(1.2, 4.2, 12.2, 7.6, 2.2, 1.3).rrect(14.3, 6.4, 1.5, 3.2, 0.6)
    if missing:
        return layers((frame, DIM), (S().line(5.6, 6, 9, 10, W).line(9, 6, 5.6, 10, W), 1))
    width = 8.8 * level / 100
    cls = 'error' if level <= 10 and not charging else 'warning' if level <= 20 and not charging else None
    parts = [(frame, 1, None)]
    if width > 0:
        parts.append((S().rrect(2.9, 5.9, max(width, 1.4), 4.2, 0.9), 0.45 if charging else 1, cls))
    if charging:
        parts.append((bolt(7.8, 8, 0.78), 1, None))
    return layers(*parts)


def bluetooth(off=False):
    rune = S().lines('M4.3,4.9 11.5,10.9 8,14.1 8,1.9 11.5,5.1 4.3,11.1', W)
    if off:
        return layers((rune, DIM), slash())
    return layers(rune)


def sun():
    shape = S().circle(8, 8, 3)
    for index in range(8):
        angle = math.radians(index * 45)
        shape.line(8 + 5 * math.cos(angle), 8 + 5 * math.sin(angle), 8 + 6.6 * math.cos(angle),
                   8 + 6.6 * math.sin(angle), 1.5)
    return layers(shape)


def crescent(cx=8, cy=8, r=6.2, ox=3.4, oy=-2.6, cut=5.2):
    dx, dy = ox, oy
    d = math.hypot(dx, dy)
    a = (r * r - cut * cut + d * d) / (2 * d)
    h = math.sqrt(max(r * r - a * a, 0))
    mx, my = cx + a * dx / d, cy + a * dy / d
    p1 = (mx + h * dy / d, my - h * dx / d)
    p2 = (mx - h * dy / d, my + h * dx / d)
    return S().raw(f'M{fmt(p1[0])} {fmt(p1[1])}A{fmt(r)} {fmt(r)} 0 1 0 {fmt(p2[0])} {fmt(p2[1])}'
                   f'A{fmt(cut)} {fmt(cut)} 0 0 1 {fmt(p1[0])} {fmt(p1[1])}Z')


def moon():
    return layers(crescent())


BELL = 'M8 1.6C10.6 1.6 12.3 3.6 12.3 6.2V9.2L13.8 11.4V12.2H2.2V11.4L3.7 9.2V6.2C3.7 3.6 5.4 1.6 8 1.6Z'


def bell(off=False):
    shape = S().raw(BELL).rrect(6.1, 12.9, 3.8, 2, 1)
    if off:
        return layers((shape, DIM), slash())
    return layers(shape)


def control_center():
    return layers(S().frame(1.4, 2.2, 13.2, 4.8, 2.4, 1.3).circle(12.2, 4.6, 1.5)
                  .frame(1.4, 9, 13.2, 4.8, 2.4, 1.3).circle(3.8, 11.4, 1.5))


def keyboard():
    shape = S().frame(1.2, 3.4, 13.6, 9.2, 2, 1.3)
    for row, y in enumerate((5.8, 8)):
        for column in range(5):
            shape.rrect(3.3 + column * 2.1, y - 0.55, 1.2, 1.1, 0.3)
    shape.line(5.4, 10.4, 10.6, 10.4, 1.2)
    return layers(shape)


def magnifier(extra=None):
    shape = S().ring(6.8, 6.8, 4.6, W).line(10.2, 10.2, 14, 14, 2.0)
    if extra == 'in':
        shape.line(4.6, 6.8, 9, 6.8, 1.3).line(6.8, 4.6, 6.8, 9, 1.3)
    elif extra == 'out':
        shape.line(4.6, 6.8, 9, 6.8, 1.3)
    return layers(shape)


def view_list():
    shape = S()
    for y in (3.5, 8, 12.5):
        shape.circle(2.6, y, 1.2).line(5.8, y, 14, y, W)
    return layers(shape)


def view_grid():
    shape = S()
    for x in (1.5, 8.8):
        for y in (1.5, 8.8):
            shape.rrect(x, y, 5.7, 5.7, 1.4)
    return layers(shape)


def view_columns():
    return layers(S().frame(1.4, 2.4, 13.2, 11.2, 2, 1.3).line(6, 2.4, 6, 13.6, 1.3, False)
                  .line(10.2, 2.4, 10.2, 13.6, 1.3, False))


def view_more(vertical=False):
    shape = S()
    for t in (3, 8, 13):
        shape.circle(8 if vertical else t, t if vertical else 8, 1.5)
    return layers(shape)


def menu():
    return layers(S().line(2.5, 4, 13.5, 4, W).line(2.5, 8, 13.5, 8, W).line(2.5, 12, 13.5, 12, W))


def sidebar():
    return layers(S().frame(1.4, 2.4, 13.2, 11.2, 2, 1.3).line(6, 2.4, 6, 13.6, 1.3, False),
                  (S().rrect(2.05, 3.05, 3.95, 9.9, 1.4), 0.35))


def circular_arrow(clockwise=True):
    shape = S().arc(8, 8.4, 5.4, -60 if clockwise else -120, 230 if clockwise else 410, W, caps=False)
    if clockwise:
        a = math.radians(-60)
        tip = (8 + 5.4 * math.cos(a), 8.4 + 5.4 * math.sin(a))
        shape.polygon([(tip[0] + 2.6, tip[1] - 1.6), (tip[0] + 0.9, tip[1] + 2.6), (tip[0] - 2.2, tip[1] - 1.0)])
    else:
        a = math.radians(-120)
        tip = (8 + 5.4 * math.cos(a), 8.4 + 5.4 * math.sin(a))
        shape.polygon([(tip[0] - 2.6, tip[1] - 1.6), (tip[0] - 0.9, tip[1] + 2.6), (tip[0] + 2.2, tip[1] - 1.0)])
    end = math.radians(230 if clockwise else 410)
    shape.circle(8 + 5.4 * math.cos(end), 8.4 + 5.4 * math.sin(end), W / 2)
    return layers(shape)


def rotate(points, turns):
    out = []
    for x, y in points:
        for _ in range(turns):
            x, y = 16 - y, x
        out.append((x, y))
    return out


def arrow(turns):
    shaft = rotate([(13.2, 8), (2.8, 8)], turns)
    head = rotate([(7.4, 3.2), (2.6, 8), (7.4, 12.8)], turns)
    return layers(S().polyline(shaft, W).polyline(head, W))


def chevron(turns):
    return layers(S().polyline(rotate([(10, 3.2), (5.2, 8), (10, 12.8)], turns), 1.8))


def cross():
    return layers(S().line(3.6, 3.6, 12.4, 12.4, W).line(12.4, 3.6, 3.6, 12.4, W))


def minimize():
    return layers(S().line(3.2, 8, 12.8, 8, W))


def maximize():
    return layers(S().frame(2.6, 2.6, 10.8, 10.8, 2, W))


def restore():
    return layers(S().frame(2.4, 5.2, 8.4, 8.4, 1.6, 1.4).polyline([(5.2, 3.2), (5.6, 2.4), (12.4, 2.4), (13.6, 3.6), (13.6, 10.4), (12.8, 10.8)], 1.4))


def plus(minus=False):
    shape = S().line(2.6, 8, 13.4, 8, W)
    if not minus:
        shape.line(8, 2.6, 8, 13.4, W)
    return layers(shape)


def trash(full=False):
    can = S().line(1.8, 3.6, 14.2, 3.6, W).polyline([(5.8, 3.6), (6.2, 1.6), (9.8, 1.6), (10.2, 3.6)], 1.3)
    can.polyline([(3.2, 3.8), (4.2, 13.4), (4.8, 14.4), (11.2, 14.4), (11.8, 13.4), (12.8, 3.8)], W)
    if full:
        return layers(can, (S().polygon([(4.5, 5.6), (11.5, 5.6), (10.8, 12.8), (5.2, 12.8)]), 0.55))
    return layers(can.line(6.4, 6.4, 6.6, 11.6, 1.2).line(9.6, 6.4, 9.4, 11.6, 1.2))


def copy():
    return layers(S().frame(5.2, 5.2, 9, 9.4, 1.8, 1.4).polyline([(2.2, 10.2), (2.2, 3.4), (3.4, 2), (9.8, 2)], 1.4))


def cut():
    return layers(S().ring(4.6, 11.8, 2.3, 1.4).ring(11.4, 11.8, 2.3, 1.4)
                  .line(6.1, 10.1, 11.6, 1.8, 1.4).line(9.9, 10.1, 4.4, 1.8, 1.4))


def paste():
    return layers(S().frame(2.8, 3, 10.4, 11.6, 1.8, 1.4).rrect(5.2, 1.2, 5.6, 3.4, 1.2)
                  .line(5.6, 8, 10.4, 8, 1.2).line(5.6, 10.8, 9, 10.8, 1.2))


def undo(redo=False):
    shape = S().polyline([(4.4, 6.4), (10, 6.4)], W).arc(10, 9.6, 3.2, -90, 90, W, caps=False).polyline([(10, 12.8), (6.6, 12.8)], W)
    shape.polyline([(6.8, 3.6), (4, 6.4), (6.8, 9.2)], W)
    parts = layers(shape)
    return mirrored(parts) if redo else parts


def mirror(d):
    tokens = re.findall(r'[A-Za-z]|-?\d*\.?\d+', d)
    out, index, command = [], 0, None
    arity = {'M': 2, 'L': 2, 'H': 1, 'V': 1, 'A': 7, 'Z': 0, 'C': 6, 'Q': 4}
    while index < len(tokens):
        token = tokens[index]
        if token.isalpha():
            command = token
            out.append(token)
            index += 1
            if command == 'Z':
                continue
        values = [float(v) for v in tokens[index:index + arity[command]]]
        index += arity[command]
        if command in 'MLCQ':
            values = [16 - v if i % 2 == 0 else v for i, v in enumerate(values)]
        elif command == 'H':
            values = [16 - values[0]]
        elif command == 'A':
            values[4] = 1 - values[4]
            values[5] = 16 - values[5]
        out.append(' '.join(fmt(v) for v in values))
    return ''.join(out)


def mirrored(parts):
    return [(S().raw(mirror(p.d())), o, c) for p, o, c in parts]


def clear():
    tag = S().polyline([(5.2, 3), (13.8, 3), (13.8, 13), (5.2, 13), (1.4, 8)], 1.4, closed=True)
    tag.line(7.6, 5.8, 11.2, 10.2, 1.4).line(11.2, 5.8, 7.6, 10.2, 1.4)
    return layers(tag)


def check():
    return layers(S().polyline([(2.6, 8.6), (6.2, 12), (13.4, 4.2)], 1.9))


PAGE = [(3, 1.6), (9.6, 1.6), (13, 5), (13, 14.4), (3, 14.4)]


def page(extra=None):
    shape = S().polyline(PAGE, 1.4, closed=True).polyline([(9.4, 1.8), (9.4, 5.2), (12.8, 5.2)], 1.2)
    if extra == 'plus':
        shape.line(8, 7.6, 8, 12.4, 1.4).line(5.6, 10, 10.4, 10, 1.4)
    elif extra == 'lines':
        shape.line(5.6, 8.2, 10.4, 8.2, 1.2).line(5.6, 11, 10.4, 11, 1.2)
    return layers(shape)


FOLDER = [(1.6, 3.4), (1.8, 2.6), (2.6, 2.2), (6, 2.2), (7.6, 4), (13.4, 4), (14.4, 5), (14.4, 12.8), (13.4, 13.8),
          (2.6, 13.8), (1.6, 12.8)]


def folder(extra=None):
    shape = S().polyline(FOLDER, 1.4, closed=True).line(1.6, 6.4, 14.4, 6.4, 1.2, False)
    if extra == 'plus':
        shape.line(8, 8, 8, 12, 1.4).line(6, 10, 10, 10, 1.4)
    return layers(shape)


def folder_open():
    shape = S().polyline([(1.6, 12.6), (1.6, 3.4), (2.6, 2.2), (6, 2.2), (7.6, 4), (12.6, 4), (12.8, 6.2)], 1.4)
    shape.polyline([(1.6, 12.6), (4.2, 6.6), (15, 6.6), (12.6, 13.6), (2.4, 13.6), (1.6, 12.6)], 1.4)
    return layers(shape)


def save():
    return layers(S().line(8, 1.8, 8, 9.6, W).polyline([(4.6, 6.4), (8, 9.8), (11.4, 6.4)], W)
                  .polyline([(1.8, 10), (1.8, 13.6), (2.6, 14.4), (13.4, 14.4), (14.2, 13.6), (14.2, 10)], W))


def clock():
    return layers(S().ring(8, 8, 6.2, 1.5).polyline([(8, 4.4), (8, 8.4), (10.8, 10)], 1.5))


STAR = [(8, 1.4), (9.95, 5.4), (14.3, 5.95), (11.1, 9), (11.9, 13.4), (8, 11.25), (4.1, 13.4), (4.9, 9), (1.7, 5.95),
        (6.05, 5.4)]


def star(filled=True):
    shape = S().polyline(STAR, 1.3, closed=True)
    if filled:
        shape.polygon(STAR)
    return layers(shape)


def share():
    return layers(S().polyline([(5.6, 6.4), (2.4, 6.4), (2.4, 14.4), (13.6, 14.4), (13.6, 6.4), (10.4, 6.4)], 1.4)
                  .line(8, 1.6, 8, 10, W).polyline([(5, 4.4), (8, 1.4), (11, 4.4)], W))


def eject():
    return layers(S().polygon([(8, 2.4), (13.4, 8.6), (2.6, 8.6)]).polyline([(8, 2.4), (13.4, 8.6), (2.6, 8.6)], 1.2, closed=True)
                  .rrect(2.6, 11, 10.8, 2.4, 0.8))


def lock(unlocked=False):
    shape = S().rrect(2.6, 7, 10.8, 7.8, 2)
    shape.circle(8, 10.2, 1.3, clockwise=False).polygon([(7.4, 10.6), (7.1, 12.8), (8.9, 12.8), (8.6, 10.6)], clockwise=False)
    if unlocked:
        shape.arc(10.8, 4.6, 2.8, 180, 360, W, caps=False).line(13.6, 4.6, 13.6, 5.4, W).line(8, 4.6, 8, 7.4, W, False)
    else:
        shape.arc(8, 5.2, 3.2, 180, 360, W, caps=False).line(4.8, 5.2, 4.8, 7.4, W, False).line(11.2, 5.2, 11.2, 7.4, W, False)
    return layers(shape)


def power():
    return layers(S().arc(8, 8.6, 5.6, -55, 235, W).line(8, 1.6, 8, 7.6, W))


def logout():
    return layers(S().polyline([(9.6, 4.6), (9.6, 2.2), (2.2, 2.2), (2.2, 13.8), (9.6, 13.8), (9.6, 11.4)], 1.4)
                  .line(6, 8, 14, 8, W).polyline([(11.4, 5.4), (14, 8), (11.4, 10.6)], W))


def play():
    return layers(S().polygon([(4.4, 2.6), (13.2, 8), (4.4, 13.4)]).polyline([(4.4, 2.6), (13.2, 8), (4.4, 13.4)], 1.4, closed=True))


def pause():
    return layers(S().rrect(3.4, 2.4, 3.4, 11.2, 1).rrect(9.2, 2.4, 3.4, 11.2, 1))


def stop():
    return layers(S().rrect(2.8, 2.8, 10.4, 10.4, 2))


def skip(forward=True):
    shape = S().polygon([(2.4, 3.2), (9.4, 8), (2.4, 12.8)]).polyline([(2.4, 3.2), (9.4, 8), (2.4, 12.8)], 1.2, closed=True)
    shape.rrect(10.8, 2.8, 2.6, 10.4, 0.9)
    parts = layers(shape)
    return parts if forward else mirrored(parts)


def info():
    return layers(S().ring(8, 8, 6.3, 1.5).line(8, 7.2, 8, 11.4, 1.7).circle(8, 4.9, 1.1))


def warning():
    triangle = [(8, 2), (14.4, 13.4), (1.6, 13.4)]
    return layers(S().polyline(triangle, 1.5, closed=True).line(8, 6.2, 8, 9.2, 1.7).circle(8, 11.3, 1.05))


def error():
    return layers(S().ring(8, 8, 6.3, 1.5).line(5.6, 5.6, 10.4, 10.4, 1.6).line(10.4, 5.6, 5.6, 10.4, 1.6))


def faders():
    shape = S()
    for y, knob in ((3.6, 10.6), (8, 5), (12.4, 8.8)):
        shape.line(1.8, y, 14.2, y, 1.3).circle(knob, y, 2.3)
    cut = S()
    for y, knob in ((3.6, 10.6), (8, 5), (12.4, 8.8)):
        cut.circle(knob, y, 1.0, clockwise=False)
    return layers(shape.raw(cut.d()))


def avatar():
    return layers(S().circle(8, 5.4, 3.2).raw('M2.2 14.6C2.6 11.4 4.8 9.6 8 9.6C11.2 9.6 13.4 11.4 13.8 14.6Z'))


def camera():
    body = S().raw('M2.6 4.6H4.8L6 2.8H10L11.2 4.6H13.4A1.6 1.6 0 0 1 15 6.2V12.4A1.6 1.6 0 0 1 13.4 14H2.6A1.6 1.6 0 0 1 1 12.4V6.2A1.6 1.6 0 0 1 2.6 4.6Z')
    body.circle(8, 9.2, 3.2, clockwise=False).circle(8, 9.2, 1.9)
    return layers(body)


def brackets():
    shape = S()
    for sx, sy in ((1, 1), (-1, 1), (1, -1), (-1, -1)):
        cx, cy = 8 - sx * 5.8, 8 - sy * 5.8
        shape.polyline([(cx, cy + sy * 3.4), (cx, cy), (cx + sx * 3.4, cy)], W)
    return layers(shape, (S().rrect(5.6, 5.6, 4.8, 4.8, 0.6), 1, None))


def home():
    house = S().polyline([(1.6, 7.6), (8, 1.8), (14.4, 7.6)], W)
    house.polyline([(3.4, 6.4), (3.4, 14), (6.4, 14), (6.4, 10), (9.6, 10), (9.6, 14), (12.6, 14), (12.6, 6.4)], 1.4)
    return layers(house)


def desktop():
    return layers(S().frame(1.4, 2, 13.2, 9, 1.8, 1.4).line(8, 11, 8, 14, 1.4, False).line(4.8, 14.2, 11.2, 14.2, W))


def laptop():
    return layers(S().frame(2.8, 3, 10.4, 7.6, 1.4, 1.4).polyline([(1, 12.6), (15, 12.6)], W)
                  .polyline([(1.4, 12.6), (2.2, 10.8)], 1.2).polyline([(14.6, 12.6), (13.8, 10.8)], 1.2))


def download():
    return layers(S().line(8, 1.8, 8, 10.6, W).polyline([(4.2, 7), (8, 10.8), (11.8, 7)], W).line(2.4, 14, 13.6, 14, W))


def music():
    return layers(S().line(6, 3.6, 6, 12, 1.4).line(13, 2.2, 13, 10.6, 1.4)
                  .polygon([(5.3, 3), (13.7, 1.4), (13.7, 4.2), (5.3, 5.8)])
                  .circle(4.3, 12.2, 2.1).circle(11.3, 10.8, 2.1))


def picture():
    return layers(S().frame(1.4, 2.6, 13.2, 10.8, 1.8, 1.4).circle(10.6, 6, 1.4)
                  .polyline([(2.2, 12), (6, 7.6), (9.4, 11), (11, 9.6), (13.8, 12.2)], 1.4))


def video():
    shape = S().frame(1.4, 2.6, 13.2, 10.8, 1.8, 1.4)
    shape.polygon([(6.4, 5.4), (11, 8), (6.4, 10.6)]).polyline([(6.4, 5.4), (11, 8), (6.4, 10.6)], 1.0, closed=True)
    return layers(shape)


def templates():
    shape = S().polyline(PAGE, 1.4, closed=True).polyline([(9.4, 1.8), (9.4, 5.2), (12.8, 5.2)], 1.2)
    shape.rrect(5.4, 7.6, 5.2, 4.4, 0.6)
    return layers(shape)


def people():
    return layers(S().circle(5.6, 5.2, 2.5).raw('M1 13.6C1.3 10.8 3.1 9.2 5.6 9.2C8.1 9.2 9.9 10.8 10.2 13.6Z'),
                  (S().circle(11, 4.8, 2.1).raw('M10.9 8.4C13.3 8.4 14.8 10 15 12.6H11.2C11 10.8 10.4 9.4 9.4 8.6C9.9 8.5 10.4 8.4 10.9 8.4Z'), 0.6))


def globe():
    return layers(S().ring(8, 8, 6.3, 1.4).line(1.8, 8, 14.2, 8, 1.2, False)
                  .raw('M8 1.4C9.9 3.2 10.9 5.5 10.9 8C10.9 10.5 9.9 12.8 8 14.6C6.1 12.8 5.1 10.5 5.1 8C5.1 5.5 6.1 3.2 8 1.4Z'
                       'M8 2.7C6.7 4.2 6.2 6.1 6.2 8C6.2 9.9 6.7 11.8 8 13.3C9.3 11.8 9.8 9.9 9.8 8C9.8 6.1 9.3 4.2 8 2.7Z'))


def server():
    shape = S().frame(1.8, 2, 12.4, 5, 1.4, 1.3).frame(1.8, 9, 12.4, 5, 1.4, 1.3)
    shape.circle(4.6, 4.5, 0.9).circle(4.6, 11.5, 0.9).line(8.2, 4.5, 11.4, 4.5, 1.2).line(8.2, 11.5, 11.4, 11.5, 1.2)
    return layers(shape)


def harddisk():
    return layers(S().frame(1.4, 4.4, 13.2, 7.2, 1.8, 1.4).circle(11.4, 8, 1.1).line(3.8, 8, 7.6, 8, 1.3))


def removable():
    shape = S().frame(4.2, 6.4, 7.6, 8.2, 1.6, 1.4).frame(5.6, 1.6, 4.8, 4.8, 0.6, 1.2)
    shape.rrect(6.8, 3, 0.9, 1.2, 0.2).rrect(8.3, 3, 0.9, 1.2, 0.2)
    return layers(shape)


def optical():
    return layers(S().ring(8, 8, 6.3, 1.4).ring(8, 8, 2, 1.4), (S().arc(8, 8, 4.2, 200, 250, 1.2), 0.6))


def headphones():
    shape = S().arc(8, 9, 5.8, 180, 360, W, caps=False)
    shape.rrect(1.4, 8.6, 3.6, 5.8, 1.4).rrect(11, 8.6, 3.6, 5.8, 1.4)
    return layers(shape)


def mouse():
    return layers(S().frame(3.6, 1.6, 8.8, 12.8, 4.4, 1.4).line(8, 3.8, 8, 6.4, 1.6))


def touchpad():
    return layers(S().frame(1.4, 2.2, 13.2, 11.6, 2, 1.4).line(2, 10.2, 14, 10.2, 1.2).line(8, 10.2, 8, 13.2, 1.2))


def printer():
    shape = S().frame(1.4, 5.6, 13.2, 6, 1.6, 1.4).polyline([(4.4, 5.4), (4.4, 1.6), (11.6, 1.6), (11.6, 5.4)], 1.3)
    shape.frame(4.6, 9.6, 6.8, 4.8, 0.4, 1.3).circle(12, 7.8, 0.8)
    return layers(shape)


def phone():
    return layers(S().frame(4, 1.4, 8, 13.2, 1.8, 1.4).line(6.8, 12.2, 9.2, 12.2, 1.2))


def bande_mark():
    return layers(S().arc(8, 8.6, 4.6, 0, 180, 2.0, caps=False).line(3.4, 2.4, 3.4, 8.6, 2.0).line(12.6, 2.4, 12.6, 8.6, 2.0)
                  .circle(8, 8, 1.3))


BATTERY_LEVELS = range(0, 101, 10)


def battery_icons():
    icons = {}
    for level in BATTERY_LEVELS:
        icons[f'battery-level-{level}-symbolic'] = lambda level=level: battery(level)
        icons[f'battery-level-{level}-charging-symbolic'] = lambda level=level: battery(level, charging=True)
    icons['battery-level-100-charged-symbolic'] = lambda: battery(100, charging=True)
    for name, level in (('full', 100), ('good', 70), ('low', 30), ('caution', 10), ('empty', 0)):
        icons[f'battery-{name}-symbolic'] = lambda level=level: battery(level)
        icons[f'battery-{name}-charging-symbolic'] = lambda level=level: battery(level, charging=True)
    icons['battery-full-charged-symbolic'] = lambda: battery(100, charging=True)
    icons['battery-symbolic'] = lambda: battery(70)
    icons['battery-missing-symbolic'] = lambda: battery(0, missing=True)
    return icons


STATUS = {
    'network-wireless-signal-none-symbolic': lambda: wifi(0),
    'network-wireless-signal-weak-symbolic': lambda: wifi(1),
    'network-wireless-signal-ok-symbolic': lambda: wifi(2),
    'network-wireless-signal-good-symbolic': lambda: wifi(3),
    'network-wireless-signal-excellent-symbolic': lambda: wifi(4),
    'network-wireless-connected-symbolic': lambda: wifi(4),
    'network-wireless-symbolic': lambda: wifi(4),
    'network-wireless-acquiring-symbolic': lambda: wifi(1),
    'network-wireless-offline-symbolic': lambda: wifi(0, off=True),
    'network-wireless-disabled-symbolic': lambda: wifi(0, off=True),
    'network-wireless-no-route-symbolic': lambda: wifi(0),
    'network-wired-symbolic': wired,
    'network-wired-disconnected-symbolic': lambda: wired(off=True),
    'network-vpn-symbolic': vpn,
    'airplane-mode-symbolic': airplane,
    'audio-volume-muted-symbolic': lambda: volume(0, muted=True),
    'audio-volume-low-symbolic': lambda: volume(1),
    'audio-volume-medium-symbolic': lambda: volume(2),
    'audio-volume-high-symbolic': lambda: volume(3),
    'audio-volume-overamplified-symbolic': lambda: volume(3),
    'audio-speakers-symbolic': lambda: volume(3),
    'audio-input-microphone-symbolic': microphone,
    'microphone-sensitivity-muted-symbolic': lambda: microphone(off=True),
    'microphone-disabled-symbolic': lambda: microphone(off=True),
    'bluetooth-active-symbolic': bluetooth,
    'bluetooth-symbolic': bluetooth,
    'bluetooth-disabled-symbolic': lambda: bluetooth(off=True),
    'bluetooth-disconnected-symbolic': lambda: bluetooth(off=True),
    'display-brightness-symbolic': sun,
    'night-light-symbolic': moon,
    'weather-clear-night-symbolic': moon,
    'notifications-symbolic': bell,
    'preferences-system-notifications-symbolic': bell,
    'notifications-disabled-symbolic': lambda: bell(off=True),
    'bande-control-center-symbolic': control_center,
    'input-keyboard-symbolic': keyboard,
    'dialog-information-symbolic': info,
    'help-about-symbolic': info,
    'dialog-warning-symbolic': warning,
    'dialog-error-symbolic': error,
    'system-lock-screen-symbolic': lock,
    'changes-prevent-symbolic': lock,
    'changes-allow-symbolic': lambda: lock(unlocked=True),
    'system-shutdown-symbolic': power,
    'system-reboot-symbolic': lambda: circular_arrow(True),
    'system-log-out-symbolic': logout,
    'system-suspend-symbolic': moon,
    'avatar-default-symbolic': avatar,
    'bande-symbolic': bande_mark,
}

ACTIONS = {
    'system-search-symbolic': magnifier,
    'edit-find-symbolic': magnifier,
    'zoom-in-symbolic': lambda: magnifier('in'),
    'zoom-out-symbolic': lambda: magnifier('out'),
    'view-list-symbolic': view_list,
    'view-grid-symbolic': view_grid,
    'view-app-grid-symbolic': view_grid,
    'view-column-symbolic': view_columns,
    'view-more-symbolic': view_more,
    'view-more-horizontal-symbolic': view_more,
    'view-more-vertical-symbolic': lambda: view_more(True),
    'open-menu-symbolic': menu,
    'sidebar-show-symbolic': sidebar,
    'view-sidebar-start-symbolic': sidebar,
    'view-refresh-symbolic': lambda: circular_arrow(True),
    'go-previous-symbolic': lambda: arrow(0),
    'go-up-symbolic': lambda: arrow(1),
    'go-next-symbolic': lambda: arrow(2),
    'go-down-symbolic': lambda: arrow(3),
    'go-home-symbolic': home,
    'pan-start-symbolic': lambda: chevron(0),
    'pan-up-symbolic': lambda: chevron(1),
    'pan-end-symbolic': lambda: chevron(2),
    'pan-down-symbolic': lambda: chevron(3),
    'window-close-symbolic': cross,
    'window-minimize-symbolic': minimize,
    'window-maximize-symbolic': maximize,
    'window-restore-symbolic': restore,
    'list-add-symbolic': plus,
    'list-remove-symbolic': lambda: plus(True),
    'edit-delete-symbolic': trash,
    'edit-copy-symbolic': copy,
    'edit-cut-symbolic': cut,
    'edit-paste-symbolic': paste,
    'edit-undo-symbolic': undo,
    'edit-redo-symbolic': lambda: undo(True),
    'edit-clear-symbolic': clear,
    'object-select-symbolic': check,
    'document-new-symbolic': lambda: page('plus'),
    'document-open-symbolic': folder_open,
    'document-save-symbolic': save,
    'document-properties-symbolic': info,
    'folder-new-symbolic': lambda: folder('plus'),
    'document-open-recent-symbolic': clock,
    'starred-symbolic': star,
    'non-starred-symbolic': lambda: star(False),
    'emblem-favorite-symbolic': star,
    'send-to-symbolic': share,
    'emblem-shared-symbolic': share,
    'media-eject-symbolic': eject,
    'media-playback-start-symbolic': play,
    'media-playback-pause-symbolic': pause,
    'media-playback-stop-symbolic': stop,
    'media-skip-forward-symbolic': skip,
    'media-skip-backward-symbolic': lambda: skip(False),
    'preferences-system-symbolic': faders,
    'emblem-system-symbolic': faders,
    'camera-photo-symbolic': camera,
    'screenshot-recorded-symbolic': brackets,
}

PLACES = {
    'folder-symbolic': folder,
    'inode-directory-symbolic': folder,
    'user-home-symbolic': home,
    'user-desktop-symbolic': desktop,
    'folder-documents-symbolic': lambda: page('lines'),
    'folder-download-symbolic': download,
    'folder-music-symbolic': music,
    'folder-pictures-symbolic': picture,
    'folder-videos-symbolic': video,
    'folder-templates-symbolic': templates,
    'folder-publicshare-symbolic': people,
    'user-trash-symbolic': trash,
    'user-trash-full-symbolic': lambda: trash(True),
    'network-workgroup-symbolic': globe,
    'folder-remote-symbolic': globe,
    'network-server-symbolic': server,
}

DEVICES = {
    'drive-harddisk-symbolic': harddisk,
    'drive-removable-media-symbolic': removable,
    'media-removable-symbolic': removable,
    'media-optical-symbolic': optical,
    'computer-symbolic': laptop,
    'video-display-symbolic': desktop,
    'audio-headphones-symbolic': headphones,
    'input-mouse-symbolic': mouse,
    'input-touchpad-symbolic': touchpad,
    'printer-symbolic': printer,
    'phone-symbolic': phone,
}

CONTEXTS = {
    'status': {**STATUS, **battery_icons()},
    'actions': ACTIONS,
    'places': PLACES,
    'devices': DEVICES,
}

SVGS = {context: {name: (lambda draw=draw: render(draw())) for name, draw in icons.items()}
        for context, icons in CONTEXTS.items()}
