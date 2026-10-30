import sys
sys.path.insert(0, '/tmp/claude-1000/-home-mikael-unbound-linux/c03b543e-455e-464d-abfa-484ad0a962a0/scratchpad/dl')
from colors import T

UI = "Inter"
DISPLAY = "Manrope"
MONO = "JetBrains Mono"


def scheme(name, accent='ultramarine'):
    p = {k.split('.', 1)[1]: v for k, v in T.items() if k.startswith(name + '.')}
    p['accent'] = T[f'accent.{name}.{accent}']
    p['on_accent'] = '#FFFFFF' if name == 'light' else T['dark.chassis']
    p['danger'] = T[f'sem.{name}.danger']
    p['warning'] = T[f'sem.{name}.warning']
    p['success'] = T[f'sem.{name}.success']
    p['name'] = name
    p['shadow'] = 0.16 if name == 'light' else 0.5
    p['select_soft'] = mix(p['accent'], p['sheet'], 0.84 if name == 'light' else 0.78)
    return p


def mix(a, b, t):
    pa = [int(a[i:i + 2], 16) for i in (1, 3, 5)]
    pb = [int(b[i:i + 2], 16) for i in (1, 3, 5)]
    return '#%02X%02X%02X' % tuple(round(x + (y - x) * t) for x, y in zip(pa, pb))


def doc(w, h, body, bg=None):
    defs = f'''<defs>
<filter id="sh-window" x="-20%" y="-20%" width="140%" height="150%">
<feDropShadow dx="0" dy="1" stdDeviation="1" flood-color="#000" flood-opacity="0.10"/>
<feDropShadow dx="0" dy="12" stdDeviation="16" flood-color="#000" flood-opacity="0.22"/>
</filter>
<filter id="sh-overlay" x="-30%" y="-30%" width="160%" height="170%">
<feDropShadow dx="0" dy="1" stdDeviation="0.75" flood-color="#000" flood-opacity="0.14"/>
<feDropShadow dx="0" dy="6" stdDeviation="9" flood-color="#000" flood-opacity="0.18"/>
</filter>
<filter id="sh-small" x="-30%" y="-30%" width="160%" height="170%">
<feDropShadow dx="0" dy="1" stdDeviation="0.6" flood-color="#000" flood-opacity="0.20"/>
</filter>
<filter id="sh-dark" x="-30%" y="-30%" width="160%" height="170%">
<feDropShadow dx="0" dy="1" stdDeviation="1" flood-color="#000" flood-opacity="0.5"/>
<feDropShadow dx="0" dy="14" stdDeviation="18" flood-color="#000" flood-opacity="0.55"/>
</filter>
</defs>'''
    b = f'<rect width="{w}" height="{h}" fill="{bg}"/>' if bg else ''
    return f'<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">{defs}{b}{body}</svg>'


def esc(s):
    return s.replace('&', '&amp;').replace('<', '&lt;').replace('>', '&gt;')


def text(x, y, s, size=13, fill='#000', weight=400, family=UI, anchor='start', ls=0, opacity=1):
    l = f' letter-spacing="{ls}"' if ls else ''
    o = f' opacity="{opacity}"' if opacity != 1 else ''
    return f'<text x="{x}" y="{y}" font-family="{family}" font-size="{size}" font-weight="{weight}" fill="{fill}" text-anchor="{anchor}"{l}{o}>{esc(s)}</text>'


def label(x, y, s, p, anchor='start'):
    return text(x, y, s.upper(), 10.5, p['ink3'], 600, UI, anchor, 0.7)


def rect(x, y, w, h, r=0, fill='none', stroke=None, sw=1, extra=''):
    st = f' stroke="{stroke}" stroke-width="{sw}"' if stroke else ''
    return f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{fill}"{st} {extra}/>'


def line(x1, y1, x2, y2, c, w=1, cap='round', op=1):
    return f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{c}" stroke-width="{w}" stroke-linecap="{cap}" opacity="{op}"/>'


def g(x, y, s=1, body=''):
    return f'<g transform="translate({x} {y}) scale({s})">{body}</g>'


def window_controls(x, y, p, state='rest', active=True):
    well = p['well'] if active else 'none'
    edge = p['hairline']
    out = rect(x, y, 66, 24, 12, well, None if active else edge, 1)
    cells = [('close', x + 12), ('min', x + 33), ('zoom', x + 54)]
    for kind, cx in cells:
        cy = y + 12
        col = p['ink2'] if active else p['ink3']
        if state == 'hover':
            col = p['ink']
        if state == 'hover' and kind == 'close':
            out += f'<circle cx="{cx}" cy="{cy}" r="8" fill="{p["danger"]}"/>'
            col = '#FFFFFF'
        if kind == 'close':
            out += line(cx - 3.5, cy - 3.5, cx + 3.5, cy + 3.5, col, 1.5)
            out += line(cx - 3.5, cy + 3.5, cx + 3.5, cy - 3.5, col, 1.5)
        elif kind == 'min':
            out += line(cx - 4, cy, cx + 4, cy, col, 1.5)
        else:
            d = f'M{cx-4.5} {cy-1} V{cy-4.5} H{cx-1} M{cx+4.5} {cy+1} V{cy+4.5} H{cx+1}'
            out += f'<path d="{d}" fill="none" stroke="{col}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>'
    return out


def disc(color, body, edge='#000000', edge_op=0.10):
    return (f'<circle cx="24" cy="24" r="22" fill="{color}"/>'
            f'<circle cx="24" cy="24" r="21.5" fill="none" stroke="{edge}" stroke-opacity="{edge_op}" stroke-width="1"/>' + body)


PAPER = T['light.sheet']
INK = T['light.ink']
EMBER = T['accent.light.ember']
ULTRA = T['accent.light.ultramarine']
STONE = '#D9D3CA'
STONE_DK = '#B9B1A5'


def icon_files():
    body = (f'<path d="M13 17.5 a2 2 0 0 1 2 -2 h6 l2.5 2.5 h10 a2 2 0 0 1 2 2 V31 a2 2 0 0 1 -2 2 H15 a2 2 0 0 1 -2 -2 Z" fill="{PAPER}"/>'
            f'<rect x="13" y="21" width="22" height="1.6" fill="{ULTRA}" opacity="0.35"/>')
    return disc(ULTRA, body)


def icon_terminal():
    body = (f'<path d="M14.5 18 L20 23.5 L14.5 29" fill="none" stroke="{PAPER}" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"/>'
            f'<rect x="23" y="27" width="10" height="2.6" rx="1.3" fill="{PAPER}"/>'
            f'<rect x="32" y="16.5" width="3.2" height="3.2" fill="{EMBER}"/>')
    return disc(INK, body, '#FFFFFF', 0.22)


def icon_settings():
    body = ''
    for i, (yy, kx) in enumerate(((17, 28), (24, 18), (31, 25))):
        body += f'<rect x="13" y="{yy-1.2}" width="22" height="2.4" rx="1.2" fill="{INK}" opacity="0.85"/>'
        fill = EMBER if i == 1 else PAPER
        body += f'<circle cx="{kx}" cy="{yy}" r="3.6" fill="{fill}" stroke="{INK}" stroke-width="2"/>'
    return disc('#E4E0DA', body, '#000000', 0.12)


def icon_monitor():
    teal = T['accent.light.teal']
    body = (f'<path d="M12 26 H17 L20 19 L24.5 31 L28 22 L30.5 26 H36" fill="none" stroke="{PAPER}" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>'
            f'<rect x="34.3" y="24.3" width="3.4" height="3.4" fill="{T["accent.dark.amber"]}"/>')
    return disc(teal, body)


def icon_text():
    body = ''
    for i, w in enumerate((20, 20, 14)):
        body += f'<rect x="14" y="{17+i*6}" width="{w}" height="2.2" rx="1.1" fill="{INK}" opacity="0.8"/>'
    body += f'<rect x="29.5" y="27.5" width="2.2" height="8" fill="{EMBER}"/>'
    return disc(PAPER, body, '#000000', 0.14)


def icon_capture():
    plum = T['accent.light.plum']
    body = ''
    for sx, sy in ((1, 1), (-1, 1), (1, -1), (-1, -1)):
        cx, cy = 24 - sx * 10, 24 - sy * 9
        body += f'<path d="M{cx} {cy + sy*5} V{cy} H{cx + sx*5}" fill="none" stroke="{PAPER}" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>'
    body += f'<rect x="21" y="21" width="6" height="6" fill="{EMBER}"/>'
    return disc(plum, body)


def folder(symbol=None, accent=ULTRA, body_col=STONE):
    out = (f'<path d="M5 13 a3 3 0 0 1 3 -3 h10 l3 3 h19 a3 3 0 0 1 3 3 v2 H5 Z" fill="{accent}"/>'
           f'<rect x="5" y="16" width="38" height="26" rx="3" fill="{body_col}"/>'
           f'<rect x="5" y="16" width="38" height="1" fill="#FFFFFF" opacity="0.5"/>')
    if symbol == 'down':
        out += f'<path d="M24 22 V34 M19 29.5 L24 34.5 L29 29.5" fill="none" stroke="#6E665B" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>'
    return out


def sheet_icon(kind, color, hint):
    out = (f'<rect x="9" y="4" width="30" height="40" rx="3" fill="#FFFFFF" stroke="#000" stroke-opacity="0.16" stroke-width="1"/>'
           + hint +
           f'<path d="M9 31 H39 V41 a3 3 0 0 1 -3 3 H12 a3 3 0 0 1 -3 -3 Z" fill="{color}"/>'
           + text(24, 40.2, kind, 8, '#FFFFFF', 700, MONO, 'middle', 0.4))
    return out


def file_pdf():
    hint = ''.join(f'<rect x="14" y="{10+i*4}" width="{20 if i%3 else 12}" height="1.6" fill="{INK}" opacity="0.45"/>' for i in range(5))
    return sheet_icon('PDF', T['sem.light.danger'], hint)


def file_image():
    hint = (f'<rect x="13" y="8" width="22" height="19" rx="1.5" fill="#E9E4DC"/>'
            f'<path d="M13 24 L20 16 L25 21 L28 18 L35 25 V25.5 a1.5 1.5 0 0 1 -1.5 1.5 H14.5 a1.5 1.5 0 0 1 -1.5 -1.5 Z" fill="{T["accent.light.moss"]}"/>'
            f'<rect x="28" y="10.5" width="3.6" height="3.6" fill="{EMBER}"/>')
    return sheet_icon('PNG', T['accent.light.moss'], hint)


def file_text():
    hint = ''.join(f'<rect x="14" y="{10+i*4}" width="{[20,18,20,9,16][i]}" height="1.6" fill="{INK}" opacity="0.45"/>' for i in range(5))
    return sheet_icon('TXT', '#6E6A64', hint)


def file_archive():
    hint = ''.join(f'<rect x="{22 + (i%2)*2}" y="{6+i*3}" width="2" height="2" fill="{INK}" opacity="0.6"/>' for i in range(7))
    hint += rect(21, 27, 6, 3, 1, INK, extra='opacity="0.6"')
    return sheet_icon('ZIP', T['accent.light.amber'], hint)


def mini_folder(x, y, s=20, accent=ULTRA):
    return g(x, y, s / 48, folder(accent=accent))


def mini(icon, x, y, s):
    return g(x, y, s / 48, icon)


SYM = {
    'home': 'M2.5 8 L9 2.5 L15.5 8 V15.5 H11 V11 H7 V15.5 H2.5 Z',
    'desktop': 'M2 3 H16 V12 H2 Z M6.5 15.5 H11.5 M9 12 V15.5',
    'doc': 'M4 1.75 H11 L14.5 5.25 V16.25 H4 Z M11 1.75 V5.25 H14.5 M6.75 9 H11.75 M6.75 12 H11.75',
    'down': 'M9 2 V12 M4.5 7.75 L9 12.25 L13.5 7.75 M3 15.75 H15',
    'image': 'M2 3 H16 V15 H2 Z M2 12 L6.5 7.5 L10 11 L12 9 L16 13 M11.5 5.5 H12.5',
    'music': 'M6.5 13.5 V3.5 L15 2 V12 M6.5 13.5 a2.25 2.25 0 1 1 -2.25 -2.25 a2.25 2.25 0 0 1 2.25 2.25 Z M15 12 a2.25 2.25 0 1 1 -2.25 -2.25 a2.25 2.25 0 0 1 2.25 2.25 Z',
    'drive': 'M2 5 H16 V13 H2 Z M5 10 H7',
    'trash': 'M2.5 4.5 H15.5 M6.5 4.5 V2.5 H11.5 V4.5 M4 4.5 L5 16 H13 L14 4.5',
}


def sym(name, x, y, color, w=1.5):
    return f'<path transform="translate({x} {y})" d="{SYM[name]}" fill="none" stroke="{color}" stroke-width="{w}" stroke-linecap="round" stroke-linejoin="round"/>'
