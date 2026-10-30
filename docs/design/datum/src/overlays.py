from kit import *
from controls import button
from window import wallpaper


def super_glyph(x, y, c):
    return rect(x + 0.75, y + 0.75, 9.5, 9.5, 2.5, 'none', c, 1.3) + rect(x + 4, y + 4, 3, 3, 0, c)


def menu(x, y, w, items, p, hi=None):
    h = 8 + sum(9 if it is None else 28 for it in items)
    out = f'<g filter="url(#{"sh-overlay" if p["name"]=="light" else "sh-dark"})">{rect(x, y, w, h, 10, p["overlay"])}</g>'
    out += rect(x + 0.5, y + 0.5, w - 1, h - 1, 9.5, 'none', p['edge'], 1, 'stroke-opacity="0.7"')
    cy = y + 4
    for i, it in enumerate(items):
        if it is None:
            out += rect(x + 12, cy + 4, w - 24, 1, 0, p['hairline'])
            cy += 9
            continue
        name, key, flags = it
        on = i == hi
        if on:
            out += rect(x + 4, cy, w - 8, 28, 6, p['accent'])
        fg = p['on_accent'] if on else (p['ink3'] if 'disabled' in flags else p['ink'])
        if 'check' in flags:
            out += f'<path d="M{x+12} {cy+14} L{x+15.5} {cy+17.5} L{x+21} {cy+10.5}" fill="none" stroke="{fg}" stroke-width="1.8" stroke-linecap="square"/>'
        out += text(x + 28, cy + 18.5, name, 13, fg, 400)
        if key:
            kc = p['on_accent'] if on else p['ink3']
            out += super_glyph(x + w - 34, cy + 9, kc) + text(x + w - 12, cy + 18.5, key, 12.5, kc, 500, UI, 'end')
        if 'sub' in flags:
            out += f'<path d="M{x+w-17} {cy+10} L{x+w-13} {cy+14} L{x+w-17} {cy+18}" fill="none" stroke="{fg}" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/>'
        cy += 28
    return out


def alert(x, y, p):
    w, h = 400, 196
    out = f'<g filter="url(#{"sh-window" if p["name"]=="light" else "sh-dark"})">{rect(x, y, w, h, 14, p["chassis"])}</g>'
    out += rect(x + 0.5, y + 0.5, w - 1, h - 1, 13.5, 'none', p['edge'], 1, 'stroke-opacity="0.6"')
    out += rect(x, y, w, h - 56, 14, p['overlay'] if p['name'] == 'light' else p['sheet'])
    out += rect(x, y + h - 70, w, 14, 0, p['overlay'] if p['name'] == 'light' else p['sheet'])
    out += mini(icon_terminal(), x + 24, y + 24, 36)
    out += text(x + 76, y + 40, 'Quit Terminal?', 17, p['ink'], 700, DISPLAY)
    out += text(x + 76, y + 64, 'Two processes are still running in', 13, p['ink2'])
    out += text(x + 76, y + 82, '“build.sh”. They will stop if you quit.', 13, p['ink2'])
    out += f'<rect x="{x+76}" y="{y+100}" width="0" height="0"/>'
    out += text(x + 76, y + 114, 'Super Q twice skips this question', 12, p['ink3'])
    by = y + h - 42
    out += button(x + 20, by, 116, "Don't Quit", p, 'quiet')
    out += button(x + w - 220, by, 96, 'Cancel', p, 'secondary')
    out += button(x + w - 116, by, 96, 'Quit', p, 'primary')
    return out


def notification(x, y, p):
    w, h = 340, 86
    out = f'<g filter="url(#{"sh-overlay" if p["name"]=="light" else "sh-dark"})">{rect(x, y, w, h, 14, p["overlay"])}</g>'
    out += rect(x + 0.5, y + 0.5, w - 1, h - 1, 13.5, 'none', p['edge'], 1, 'stroke-opacity="0.6"')
    out += mini(icon_files(), x + 14, y + 14, 18)
    out += text(x + 40, y + 27, 'FILES', 10.5, p['ink3'], 600, UI, 'start', 0.7)
    out += text(x + w - 14, y + 27, 'now', 11.5, p['ink3'], 500, UI, 'end')
    out += text(x + 14, y + 52, 'Copy finished', 13.5, p['ink'], 600)
    out += text(x + 14, y + 71, '1,204 items copied to “Field recordings”', 13, p['ink2'])
    return out


def scene(name):
    p = scheme(name)
    W, H = 1000, 660
    body = wallpaper(p, W, H)
    wx, wy, ww, wh = 40, 40, 560, 560
    body += f'<g filter="url(#sh-window)">{rect(wx, wy, ww, wh, 14, p["chassis"])}</g>'
    body += window_controls(wx + 12, wy + 12, p, 'rest', True)
    body += text(wx + 92, wy + 29.5, 'Documents', 15, p['ink'], 700, DISPLAY)
    body += rect(wx + 6, wy + 48, ww - 12, wh - 54, 8, p['sheet'])
    for i, n in enumerate(['Projects', 'Quarterly report.pdf', 'Harbour at dusk.png', 'Notes.txt', 'Field recordings']):
        ry = wy + 60 + i * 32
        if i == 1:
            body += rect(wx + 14, ry, ww - 28, 30, 6, p['accent'])
        body += text(wx + 30, ry + 19.5, n, 13, p['on_accent'] if i == 1 else p['ink'], 500 if i == 1 else 400)
    body += menu(wx + 200, wy + 110, 236, [('Open', 'O', ''), ('Open With', '', 'sub'), None, ('Rename', '', ''),
                                           ('Compress', '', ''), ('Copy', 'C', ''), None, ('Sort By', '', 'sub'),
                                           ('Move to Trash', '⌫', '')], p, hi=7)
    body += menu(wx + 200 + 236 - 4, wy + 110 + 162 - 4, 170, [('Name', '', 'check'), ('Date Modified', '', ''), ('Size', '', ''), ('Kind', '', '')], p)
    body += notification(W - 360, 20, p)
    body += alert(W - 420, 420, p)
    open(f'overlays-{name}.svg', 'w').write(doc(W, H, body))


scene('light')
scene('dark')
