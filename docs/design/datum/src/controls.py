from kit import *


def raised(p):
    return p['overlay'] if p['name'] == 'light' else p['edge']


def button(x, y, w, s, p, kind='secondary', state='rest'):
    out = ''
    if kind == 'primary':
        fill = p['accent'] if state != 'press' else mix(p['accent'], '#000000', 0.15)
        out += rect(x, y, w, 28, 6, fill)
        out += text(x + w / 2, y + 18.5, s, 13, p['on_accent'], 600, UI, 'middle')
    elif kind == 'secondary':
        out += rect(x, y, w, 28, 6, p['well'] if state == 'rest' else mix(p['well'], p['ink'], 0.08))
        out += text(x + w / 2, y + 18.5, s, 13, p['ink'], 500, UI, 'middle')
    elif kind == 'quiet':
        if state == 'hover':
            out += rect(x, y, w, 28, 6, p['well'])
        out += text(x + w / 2, y + 18.5, s, 13, p['accent'], 600, UI, 'middle')
    elif kind == 'danger':
        out += rect(x, y, w, 28, 6, p['danger'])
        out += text(x + w / 2, y + 18.5, s, 13, '#FFFFFF' if p['name'] == 'light' else p['chassis'], 600, UI, 'middle')
    if state == 'focus':
        out += rect(x - 3, y - 3, w + 6, 34, 9, 'none', p['accent'], 2)
    return out


def field(x, y, w, p, value='', placeholder='', focus=False):
    out = rect(x, y, w, 28, 6, p['overlay'] if p['name'] == 'light' else p['well'])
    out += rect(x + 0.5, y + 0.5, w - 1, 27, 5.5, 'none', p['accent'] if focus else p['edge'], 1.5 if focus else 1)
    if focus:
        out += rect(x - 3, y - 3, w + 6, 34, 9, 'none', p['accent'], 2, 'stroke-opacity="0.35"')
    if value:
        out += text(x + 10, y + 18.5, value, 13, p['ink'])
        tw = len(value) * 6.75
        out += rect(x + 10 + tw, y + 7, 1.5, 14, 0, p['accent'])
    else:
        out += text(x + 10, y + 18.5, placeholder, 13, p['ink3'])
    return out


def switch(x, y, on, p):
    track = p['accent'] if on else p['edge']
    out = rect(x, y, 36, 20, 10, track)
    kx = x + 26 if on else x + 10
    out += f'<circle cx="{kx}" cy="{y+10}" r="7.5" fill="#FFFFFF" filter="url(#sh-small)"/>'
    return out


def checkbox(x, y, on, p, s=''):
    if on:
        out = rect(x, y, 16, 16, 4, p['accent'])
        out += f'<path d="M{x+4} {y+8.5} L{x+7} {y+11.5} L{x+12} {y+5}" fill="none" stroke="{p["on_accent"]}" stroke-width="2" stroke-linecap="square"/>'
    else:
        out = rect(x + 0.75, y + 0.75, 14.5, 14.5, 3.5, p['overlay'] if p['name'] == 'light' else p['well'], p['ink3'], 1.5)
    return out + text(x + 24, y + 12.5, s, 13, p['ink'])


def radio(x, y, on, p, s=''):
    if on:
        out = f'<circle cx="{x+8}" cy="{y+8}" r="8" fill="{p["accent"]}"/>' + rect(x + 5.5, y + 5.5, 5, 5, 0, p['on_accent'])
    else:
        out = f'<circle cx="{x+8}" cy="{y+8}" r="7.25" fill="none" stroke="{p["ink3"]}" stroke-width="1.5"/>'
    return out + text(x + 24, y + 12.5, s, 13, p['ink'])


def segmented(x, y, items, sel, p, w=72):
    out = rect(x, y, w * len(items) + 4, 28, 8, p['well'])
    out += rect(x + 2 + sel * w, y + 2, w, 24, 6, raised(p), extra='filter="url(#sh-small)"')
    for i, s in enumerate(items):
        out += text(x + 2 + i * w + w / 2, y + 18.5, s, 13, p['ink'] if i == sel else p['ink2'], 600 if i == sel else 500, UI, 'middle')
    return out


def slider(x, y, w, v, p, ticks=0):
    out = rect(x, y + 8, w, 4, 2, p['well'] if p['name'] == 'light' else p['edge'])
    out += rect(x, y + 8, w * v, 4, 2, p['accent'])
    for t in range(ticks):
        tx = x + w * t / (ticks - 1)
        out += rect(tx - 0.75, y + 18, 1.5, 4, 0, p['ink3'])
    kx = x + w * v
    out += rect(kx - 3.5, y, 7, 20, 3.5, '#FFFFFF', p['edge'] if p['name'] == 'light' else None, 1, 'filter="url(#sh-small)"')
    return out


def stepper(x, y, value, p):
    out = rect(x, y, 96, 28, 6, p['well'])
    out += line(x + 10, y + 14, x + 18, y + 14, p['ink2'], 1.5)
    out += line(x + 78, y + 14, x + 86, y + 14, p['ink2'], 1.5) + line(x + 82, y + 10, x + 82, y + 18, p['ink2'], 1.5)
    out += text(x + 48, y + 18.5, value, 13, p['ink'], 600, UI, 'middle')
    return out


def popup(x, y, w, s, p):
    out = rect(x, y, w, 28, 6, p['well'])
    out += text(x + 10, y + 18.5, s, 13, p['ink'], 500)
    out += f'<path d="M{x+w-18} {y+12} L{x+w-14} {y+16} L{x+w-10} {y+12}" fill="none" stroke="{p["ink2"]}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>'
    return out


def progress(x, y, w, v, p):
    segs = 24
    out = ''
    sw = (w - (segs - 1) * 2) / segs
    for i in range(segs):
        on = i < round(v * segs)
        out += rect(x + i * (sw + 2), y, sw, 6, 1, p['accent'] if on else (p['well'] if p['name'] == 'light' else p['edge']))
    return out


def tooltip(x, y, s, p):
    w = len(s) * 6.2 + 20
    bg = p['ink'] if p['name'] == 'light' else p['ink']
    out = rect(x, y, w, 24, 6, bg)
    out += text(x + 10, y + 16, s, 12, p['chassis'], 500)
    return out


def badge(x, y, n, p):
    return rect(x, y, 18, 18, 9, p['danger']) + text(x + 9, y + 13, str(n), 11, '#FFFFFF', 700, UI, 'middle')


def panel(p, ox):
    W = 640
    out = rect(ox, 0, W, 760, 0, p['sheet'])
    out += text(ox + 32, 44, 'Controls', 22, p['ink'], 700, DISPLAY)
    out += text(ox + 32 + 110, 44, p['name'], 13, p['ink3'], 500)
    y = 72
    out += label(ox + 32, y, 'Window controls — rest · hover · inactive', p)
    for i, (st, act) in enumerate((('rest', True), ('hover', True), ('rest', False))):
        bx = ox + 32 + i * 110
        out += rect(bx - 8, y + 12, 98, 40, 10, p['chassis'])
        out += window_controls(bx + 8, y + 20, p, st, act)
    y += 82
    out += label(ox + 32, y, 'Buttons', p)
    out += button(ox + 32, y + 12, 96, 'Save', p, 'primary')
    out += button(ox + 140, y + 12, 96, 'Cancel', p, 'secondary')
    out += button(ox + 248, y + 12, 96, 'Options', p, 'quiet', 'hover')
    out += button(ox + 356, y + 12, 112, 'Move to Trash', p, 'danger')
    out += button(ox + 484, y + 12, 96, 'Focused', p, 'secondary', 'focus')
    y += 66
    out += label(ox + 32, y, 'Fields', p)
    out += field(ox + 32, y + 12, 260, p, placeholder='Name')
    out += field(ox + 312, y + 12, 260, p, value='Field recordings', focus=True)
    y += 66
    out += label(ox + 32, y, 'Choice', p)
    out += switch(ox + 32, y + 16, True, p) + switch(ox + 80, y + 16, False, p)
    out += checkbox(ox + 140, y + 18, True, p, 'Show hidden files')
    out += checkbox(ox + 300, y + 18, False, p, 'Ask first')
    out += radio(ox + 410, y + 18, True, p, 'Name')
    out += radio(ox + 490, y + 18, False, p, 'Date')
    y += 62
    out += label(ox + 32, y, 'Segmented · pop-up · stepper', p)
    out += segmented(ox + 32, y + 12, ['Icons', 'List', 'Columns'], 1, p)
    out += popup(ox + 268, y + 12, 160, 'Date Modified', p)
    out += stepper(ox + 444, y + 12, '12 pt', p)
    y += 66
    out += label(ox + 32, y, 'Slider · progress', p)
    out += slider(ox + 32, y + 14, 260, 0.62, p)
    out += slider(ox + 312, y + 14, 260, 0.5, p, 5)
    out += progress(ox + 32, y + 50, 540, 0.58, p)
    y += 84
    out += label(ox + 32, y, 'Tooltip · badge · status', p)
    out += tooltip(ox + 32, y + 14, 'Quit the selected process', p)
    out += badge(ox + 236, y + 17, 3, p)
    for i, k in enumerate(('success', 'warning', 'danger')):
        out += rect(ox + 280 + i * 92, y + 22, 8, 8, 0, p[k]) + text(ox + 294 + i * 92, y + 30, k.capitalize(), 12, p['ink2'], 500)
    y += 62
    out += label(ox + 32, y, 'List selection — focused · unfocused · hover', p)
    for i, (fill, fg) in enumerate(((p['accent'], p['on_accent']), (p['well'], p['ink']), (mix(p['well'], p['sheet'], 0.5), p['ink']))):
        out += rect(ox + 32, y + 12 + i * 32, 300, 30, 6, fill)
        out += text(ox + 48, y + 31.5 + i * 32, ['Quarterly report.pdf', 'Harbour at dusk.png', 'Notes.txt'][i], 13, fg, 500)
    return out


body = panel(scheme('light'), 0) + panel(scheme('dark'), 640)
open('controls.svg', 'w').write(doc(1280, 760, body))
