import os
import struct

FONT = os.environ.get('BANDE_MONO_FONT', '/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Bold.ttf')


class Font:
    def __init__(self, path):
        self.data = open(path, 'rb').read()
        count = struct.unpack('>H', self.data[4:6])[0]
        self.tables = {}
        for index in range(count):
            tag, _, offset, length = struct.unpack('>4sIII', self.data[12 + 16 * index:28 + 16 * index])
            self.tables[tag.decode()] = (offset, length)
        head = self.tables['head'][0]
        self.units = struct.unpack('>H', self.data[head + 18:head + 20])[0]
        self.long_loca = struct.unpack('>h', self.data[head + 50:head + 52])[0] == 1
        hhea = self.tables['hhea'][0]
        self.metrics = struct.unpack('>H', self.data[hhea + 34:hhea + 36])[0]
        self.cmap = self.read_cmap()

    def u16(self, at):
        return struct.unpack('>H', self.data[at:at + 2])[0]

    def read_cmap(self):
        base = self.tables['cmap'][0]
        mapping = {}
        for index in range(self.u16(base + 2)):
            platform, encoding, offset = struct.unpack('>HHI', self.data[base + 4 + 8 * index:base + 12 + 8 * index])
            table = base + offset
            if self.u16(table) != 4 or (platform, encoding) not in ((3, 1), (0, 3), (0, 4)):
                continue
            segments = self.u16(table + 6) // 2
            ends = table + 14
            starts = ends + 2 * segments + 2
            deltas = starts + 2 * segments
            ranges = deltas + 2 * segments
            for segment in range(segments):
                end = self.u16(ends + 2 * segment)
                start = self.u16(starts + 2 * segment)
                delta = self.u16(deltas + 2 * segment)
                range_offset = self.u16(ranges + 2 * segment)
                for code in range(start, min(end, 0x7F) + 1):
                    if range_offset == 0:
                        glyph = (code + delta) & 0xFFFF
                    else:
                        at = ranges + 2 * segment + range_offset + 2 * (code - start)
                        glyph = self.u16(at)
                        glyph = (glyph + delta) & 0xFFFF if glyph else 0
                    mapping[chr(code)] = glyph
            return mapping
        return mapping

    def advance(self, glyph):
        hmtx = self.tables['hmtx'][0]
        glyph = min(glyph, self.metrics - 1)
        return self.u16(hmtx + 4 * glyph)

    def glyph_range(self, glyph):
        loca = self.tables['loca'][0]
        if self.long_loca:
            start, end = struct.unpack('>II', self.data[loca + 4 * glyph:loca + 4 * glyph + 8])
        else:
            start, end = (2 * value for value in struct.unpack('>HH', self.data[loca + 2 * glyph:loca + 2 * glyph + 4]))
        return self.tables['glyf'][0] + start, end - start

    def contours(self, glyph):
        at, length = self.glyph_range(glyph)
        if length == 0:
            return []
        count = struct.unpack('>h', self.data[at:at + 2])[0]
        if count < 0:
            return self.composite(at + 10)
        ends = [self.u16(at + 10 + 2 * index) for index in range(count)]
        points = ends[-1] + 1 if ends else 0
        cursor = at + 10 + 2 * count
        cursor += 2 + self.u16(cursor)
        flags = []
        while len(flags) < points:
            flag = self.data[cursor]
            cursor += 1
            flags.append(flag)
            if flag & 8:
                repeat = self.data[cursor]
                cursor += 1
                flags.extend([flag] * repeat)
        def axis(short, same):
            nonlocal cursor
            values, value = [], 0
            for flag in flags:
                if flag & short:
                    delta = self.data[cursor]
                    cursor += 1
                    value += delta if flag & same else -delta
                elif not flag & same:
                    value += struct.unpack('>h', self.data[cursor:cursor + 2])[0]
                    cursor += 2
                values.append(value)
            return values
        xs = axis(2, 16)
        ys = axis(4, 32)
        result, start = [], 0
        for end in ends:
            result.append([(xs[i], ys[i], bool(flags[i] & 1)) for i in range(start, end + 1)])
            start = end + 1
        return result

    def composite(self, at):
        result = []
        while True:
            flags, glyph = struct.unpack('>HH', self.data[at:at + 4])
            at += 4
            if flags & 1:
                dx, dy = struct.unpack('>hh', self.data[at:at + 4])
                at += 4
            else:
                dx, dy = struct.unpack('>bb', self.data[at:at + 2])
                at += 2
            if flags & 8:
                at += 2
            elif flags & 0x40:
                at += 4
            elif flags & 0x80:
                at += 8
            for contour in self.contours(glyph):
                result.append([(x + dx, y + dy, on) for x, y, on in contour])
            if not flags & 0x20:
                return result


def contour_path(contour, transform):
    points = [transform(x, y) + (on,) for x, y, on in contour]
    if not points[0][2]:
        last = points[-1]
        if last[2]:
            points = [last] + points[:-1]
        else:
            points = [((last[0] + points[0][0]) / 2, (last[1] + points[0][1]) / 2, True)] + points
    out = [f'M{points[0][0]:.2f} {points[0][1]:.2f}']
    count = len(points)
    index = 1
    while index <= count:
        x, y, on = points[index % count]
        if on:
            out.append(f'L{x:.2f} {y:.2f}')
            index += 1
            continue
        nx, ny, non = points[(index + 1) % count]
        if non:
            out.append(f'Q{x:.2f} {y:.2f} {nx:.2f} {ny:.2f}')
            index += 2
        else:
            out.append(f'Q{x:.2f} {y:.2f} {(x + nx) / 2:.2f} {(y + ny) / 2:.2f}')
            index += 1
    out.append('Z')
    return ''.join(out)


_font = None


def font():
    global _font
    if _font is None:
        _font = Font(FONT)
    return _font


def text_path(text, cx, baseline, size, tracking=0.0):
    face = font()
    scale = size / face.units
    advances = [face.advance(face.cmap.get(ch, 0)) * scale for ch in text]
    width = sum(advances) + tracking * (len(text) - 1)
    x = cx - width / 2
    parts = []
    for ch, advance in zip(text, advances):
        origin = x
        def transform(px, py, origin=origin):
            return (origin + px * scale, baseline - py * scale)
        for contour in face.contours(face.cmap.get(ch, 0)):
            parts.append(contour_path(contour, transform))
        x += advance + tracking
    return ''.join(parts), width
