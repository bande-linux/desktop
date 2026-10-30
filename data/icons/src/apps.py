import math

from common import (EMBER, INK, PAPER, accent, circle, fmt, gradient, mix, path, rect,
                    stroke, svg, tone)

LIFT = ('<filter id="lift" x="-20%" y="-20%" width="140%" height="150%">'
        '<feDropShadow dx="0" dy="0.9" stdDeviation="0.6" flood-color="#000" flood-opacity="0.24"/></filter>')


def tile(top, bottom, body, edge=None, extra_defs=''):
    edge = edge or ('#000000', 0.10)
    defs = gradient('field', top, bottom) + LIFT + extra_defs
    out = rect(2, 2, 44, 44, 10, 'url(#field)')
    out += rect(2.5, 2.5, 43, 43, 9.5, 'none', f'stroke="{edge[0]}" stroke-opacity="{edge[1]}" stroke-width="1"')
    out += f'<g filter="url(#lift)">{body}</g>'
    return svg(48, out, defs)


def hue_field(hue, lightness, chroma):
    return tone(hue, lightness + 0.07, chroma * 0.85), tone(hue, lightness - 0.03, chroma)


def files():
    blue = accent('ultramarine')
    back = mix(PAPER, blue, 0.28)
    body = (path('M11 15.5a2.5 2.5 0 0 1 2.5-2.5h7.2l2.8 2.8h11a2.5 2.5 0 0 1 2.5 2.5V33a2.5 2.5 0 0 1-2.5 2.5h-21A2.5 2.5 0 0 1 11 33Z', back)
            + path('M11 20.5a2.5 2.5 0 0 1 2.5-2.5h21a2.5 2.5 0 0 1 2.5 2.5V33a2.5 2.5 0 0 1-2.5 2.5h-21A2.5 2.5 0 0 1 11 33Z', PAPER)
            + rect(11, 22, 26, 1.4, 0, blue, 'opacity="0.22"'))
    return tile(*hue_field(272, 0.56, 0.19), body)


def terminal():
    body = (rect(9.5, 12, 29, 24, 4, '#000000', 'opacity="0.22"')
            + stroke('M15 18.5l5 4.5-5 4.5', PAPER, 2.6)
            + rect(22.5, 26.4, 9, 2.6, 1.3, PAPER, 'opacity="0.78"')
            + rect(33, 16, 2.6, 6.5, 0.6, EMBER))
    return tile(tone(80, 0.36, 0.008), tone(80, 0.22, 0.008), body, ('#FFFFFF', 0.12))


def settings():
    track = tone(80, 0.36, 0.01)
    body = ''
    for index, (y, knob) in enumerate(((16, 29), (24, 18.5), (32, 25.5))):
        body += rect(12, y - 1.3, 24, 2.6, 1.3, track, 'opacity="0.55"')
        body += rect(12, y - 1.3, knob - 12, 2.6, 1.3, PAPER, 'opacity="0.9"')
        fill = EMBER if index == 1 else PAPER
        body += circle(knob, y, 3.9, fill)
        body += circle(knob, y, 3.9, 'none', f'stroke="{track}" stroke-opacity="0.35" stroke-width="0.8"')
    return tile(tone(80, 0.70, 0.012), tone(80, 0.54, 0.014), body)


def monitor():
    moss_top, moss_bottom = hue_field(160, 0.62, 0.13)
    line = [(11, 30), (16.5, 25), (21, 28), (27, 18), (31.5, 23), (37, 15)]
    area = 'M' + ' L'.join(f'{x} {y}' for x, y in line) + ' L37 35 L11 35Z'
    body = (path(area, PAPER, 'opacity="0.28"')
            + stroke('M' + ' L'.join(f'{x} {y}' for x, y in line), PAPER, 2.5)
            + rect(11, 35, 26, 1.6, 0.8, PAPER, 'opacity="0.55"')
            + rect(35.2, 13.2, 3.6, 3.6, 0.4, EMBER))
    return tile(moss_top, moss_bottom, body)


def mark_shape(color, dot, width=3.6):
    return (stroke('M16.5 13.5v10a7.5 7.5 0 0 0 15 0v-10', color, width)
            + circle(24, 24, 2.7, dot))


def launcher():
    return tile(tone(80, 0.995, 0.002), tone(80, 0.93, 0.006), mark_shape(INK, accent('ultramarine')), ('#000000', 0.14))


def about():
    return tile(*hue_field(272, 0.56, 0.19), mark_shape(PAPER, EMBER))


def screenshot():
    body = rect(15, 15, 18, 18, 1.5, PAPER, 'opacity="0.22"')
    for sx, sy in ((1, 1), (-1, 1), (1, -1), (-1, -1)):
        cx, cy = 24 - sx * 12, 24 - sy * 12
        body += stroke(f'M{cx} {cy + sy * 6}V{cy}H{cx + sx * 6}', PAPER, 2.6)
    body += rect(21.5, 21.5, 5, 5, 0.6, EMBER)
    return tile(*hue_field(320, 0.54, 0.16), body)


def web():
    white = PAPER
    body = (circle(24, 24, 12.5, white, 'opacity="0.2"')
            + circle(24, 24, 12.5, 'none', f'stroke="{white}" stroke-width="2.2"')
            + '<ellipse cx="24" cy="24" rx="5.4" ry="12.5" fill="none" stroke="#FCFBF9" stroke-width="1.8"/>'
            + stroke('M11.5 24h25M13.6 17.5h20.8M13.6 30.5h20.8', white, 1.6))
    return tile(tone(55, 0.78, 0.15), tone(38, 0.62, 0.19), body)


def text_editor():
    lines = tone(65, 0.42, 0.09)
    body = rect(12, 10, 24, 28, 3, PAPER, 'opacity="0.95"')
    for index, width in enumerate((16, 16, 11, 14)):
        body += rect(16, 15.5 + index * 4.8, width, 2.1, 1.05, lines, 'opacity="0.75"')
    body += rect(28.5, 27.5, 2.2, 7.5, 0.4, EMBER)
    return tile(tone(88, 0.90, 0.13), tone(78, 0.80, 0.15), body)


def image_viewer():
    moss = accent('moss')
    body = (rect(10, 12.5, 28, 23, 3, PAPER)
            + '<clipPath id="photo"><rect x="12.5" y="15" width="23" height="18" rx="1.5"/></clipPath>'
            + f'<g clip-path="url(#photo)">{rect(12.5, 15, 23, 18, 0, tone(230, 0.86, 0.05))}'
            + path('M12.5 29l6-7 4.6 4.8 3-3 9.4 8.2V33h-23Z', moss)
            + path('M12.5 33v-2.4l7.5-4.3 6 3.7 4-1.8 5.5 2.4V33Z', mix(moss, '#000000', 0.25))
            + '</g>'
            + rect(28.5, 17.5, 3.8, 3.8, 0.4, EMBER))
    return tile(*hue_field(195, 0.60, 0.11), body)


def document_viewer():
    body = (rect(15, 9.5, 21, 27, 2.5, PAPER, 'opacity="0.55"')
            + rect(12, 12, 21, 27, 2.5, PAPER))
    for index, width in enumerate((13, 13, 9, 13)):
        body += rect(15.5, 16.5 + index * 3.6, width, 1.6, 0.8, INK, 'opacity="0.45"')
    body += rect(15.5, 31, 13, 3.4, 0.6, accent('amber'), 'opacity="0.55"')
    return tile(tone(25, 0.68, 0.16), tone(25, 0.53, 0.19), body)


def video_player():
    frame = tone(285, 0.30, 0.06)
    body = rect(9.5, 13, 29, 22, 3.5, PAPER, 'opacity="0.96"')
    for index in range(5):
        body += rect(11.4 + index * 5.6, 14.6, 3, 2.2, 0.5, frame, 'opacity="0.35"')
        body += rect(11.4 + index * 5.6, 31.2, 3, 2.2, 0.5, frame, 'opacity="0.35"')
    body += path('M21 19.2v9.6a.8.8 0 0 0 1.2.7l8-4.8a.8.8 0 0 0 0-1.4l-8-4.8a.8.8 0 0 0-1.2.7Z', frame)
    return tile(*hue_field(5, 0.58, 0.18), body)


def calculator():
    body = rect(12, 10, 24, 7.5, 2, PAPER, 'opacity="0.9"')
    body += rect(25, 12.6, 8, 2.4, 1.2, INK, 'opacity="0.55"')
    for row in range(3):
        for column in range(3):
            fill, opacity = PAPER, 0.85 if column < 2 else 0.55
            if row == 2 and column == 2:
                fill, opacity = EMBER, 1
            body += rect(12 + column * 8.6, 20 + row * 6.3, 6.8, 4.9, 1.5, fill, f'opacity="{opacity}"')
    return tile(tone(80, 0.38, 0.01), tone(80, 0.24, 0.01), body, ('#FFFFFF', 0.12))


def clocks():
    face = PAPER
    body = circle(24, 24, 15, face) + circle(24, 24, 15, 'none', f'stroke="{INK}" stroke-opacity="0.12" stroke-width="1"')
    for hour in range(12):
        angle = math.radians(hour * 30)
        inner = 11.6 if hour % 3 == 0 else 12.6
        x1, y1 = 24 + inner * math.sin(angle), 24 - inner * math.cos(angle)
        x2, y2 = 24 + 13.4 * math.sin(angle), 24 - 13.4 * math.cos(angle)
        body += stroke(f'M{fmt(x1)} {fmt(y1)}L{fmt(x2)} {fmt(y2)}', INK, 1.3 if hour % 3 == 0 else 0.9, 'opacity="0.55"')
    body += stroke('M24 24L24 15.5', INK, 2.2) + stroke('M24 24L30 27.5', INK, 2.2)
    body += stroke('M24 24L17.5 31', EMBER, 1.1) + circle(24, 24, 1.8, EMBER)
    return tile(*hue_field(255, 0.46, 0.12), body)


ICONS = {
    'org.unbound.Files': files,
    'org.unbound.Terminal': terminal,
    'org.unbound.Settings': settings,
    'org.unbound.ActivityMonitor': monitor,
    'org.unbound.Launcher': launcher,
    'org.unbound.About': about,
    'org.unbound.Screenshot': screenshot,
    'firefox-esr': web,
    'firefox': web,
    'org.gnome.TextEditor': text_editor,
    'org.gnome.Loupe': image_viewer,
    'org.gnome.Papers': document_viewer,
    'org.gnome.Showtime': video_player,
    'org.gnome.Calculator': calculator,
    'org.gnome.clocks': clocks,
}

HICOLOR = {
    'org.unbound.Files': 'crates/apps/files/data/org.unbound.Files.svg',
    'org.unbound.Terminal': 'crates/apps/terminal/data/org.unbound.Terminal.svg',
    'org.unbound.Settings': 'crates/apps/settings/data/org.unbound.Settings.svg',
    'org.unbound.ActivityMonitor': 'crates/apps/monitor/data/org.unbound.ActivityMonitor.svg',
}
