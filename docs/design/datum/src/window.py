from kit import *


def wallpaper(p, W, H):
    if p['name'] == 'light':
        a, b, c = '#D9DCE6', '#C7CCE0', '#AEB6DA'
    else:
        a, b, c = '#16171D', '#1E2030', '#2B2F55'
    return (f'<defs><linearGradient id="wp{p["name"]}" x1="0" y1="0" x2="0.3" y2="1">'
            f'<stop offset="0" stop-color="{a}"/><stop offset="0.6" stop-color="{b}"/><stop offset="1" stop-color="{c}"/></linearGradient></defs>'
            f'<rect width="{W}" height="{H}" fill="url(#wp{p["name"]})"/>'
            f'<circle cx="{W*0.86}" cy="{H*1.45}" r="{H*1.0}" fill="{p["accent"]}" opacity="0.22"/>'
            f'<circle cx="{W*0.86}" cy="{H*1.45}" r="{H*0.82}" fill="{p["accent"]}" opacity="0.22"/>')


def bar(p, H):
    x, y, w, h = 0, 0, 64, H
    out = rect(x, y, w, h, 0, p["chassis"])
    out += rect(w - 1, 0, 1, h, 0, p['edge'], extra='opacity="0.7"')
    cx = x + w / 2
    out += f'<g transform="translate({cx-9} {y+18})">' + ''.join(
        rect(c * 7, r * 7, 4.5, 4.5, 0, p['ink']) for c, r in ((0,0),(0,1),(0,2),(1,2),(2,0),(2,1),(2,2))) + '</g>'
    out += rect(cx - 6, y + 50, 12, 1, 0, p['hairline'])
    apps = [icon_files(), icon_terminal(), icon_settings(), icon_monitor(), icon_text()]
    running = {0: 'active', 1: 'run', 3: 'run'}
    for i, ic in enumerate(apps):
        iy = y + 66 + i * 52
        out += mini(ic, cx - 20, iy, 40)
        st = running.get(i)
        if st == 'active':
            out += rect(x, iy + 12, 3, 16, 0, p['accent'])
        elif st == 'run':
            out += rect(x + 1, iy + 18.5, 3, 3, 0, p['ink2'])
    ty = y + 66 + 5 * 52 + 4
    out += rect(cx - 6, ty, 12, 1, 0, p['hairline'])
    out += f'<g transform="translate({cx-11} {ty+14})"><path d="M3 5 H19 M7 5 V3 H15 V5 M5 5 L6.5 21 H15.5 L17 5" fill="none" stroke="{p["ink2"]}" stroke-width="1.6" stroke-linejoin="round" stroke-linecap="round"/></g>'
    by = y + h - 112
    out += f'<g transform="translate({cx-8} {by})" fill="none" stroke="{p["ink2"]}" stroke-width="1.6" stroke-linecap="round">' \
           f'<path d="M1 6 Q8 0 15 6"/><path d="M3.5 9 Q8 5.5 12.5 9"/><circle cx="8" cy="12" r="0.9" fill="{p["ink2"]}"/></g>'
    out += f'<g transform="translate({cx-10} {by+26})">{rect(0.75, 0.75, 17, 9.5, 3, "none", p["ink2"], 1.4)}{rect(3, 3, 10, 5, 1.2, p["ink2"])}{rect(18.5, 3.5, 1.6, 4, 0.8, p["ink2"])}</g>'
    out += rect(cx - 6, by + 50, 12, 1, 0, p['hairline'])
    out += text(cx, by + 76, '14', 15, p['ink'], 600, UI, 'middle')
    out += text(cx, by + 94, '32', 15, p['ink'], 600, UI, 'middle')
    return out


def sidebar_item(x, y, w, name, p, selected=False, accent=None):
    out = ''
    if selected:
        out += rect(x, y, w, 28, 6, p['well'])
    out += sym(accent, x + 8, y + 5, p['accent'] if selected else p['ink2'])
    out += text(x + 34, y + 18.5, name, 13, p['ink'], 500 if selected else 400)
    return out


def files_window(p, x, y, w, h, active=True):
    out = f'<g filter="url(#{"sh-window" if p["name"]=="light" else "sh-dark"})">{rect(x, y, w, h, 14, p["chassis"])}</g>'
    out += rect(x + 0.5, y + 0.5, w - 1, h - 1, 13.5, 'none', p['edge'], 1, 'stroke-opacity="0.55"')
    head = 48
    out += window_controls(x + 12, y + 12, p, 'rest', active)
    for i, d in enumerate(('M5 1 L1 5 L5 9', 'M1 1 L5 5 L1 9')):
        bx = x + 96 + i * 30
        out += f'<path transform="translate({bx} {y+19})" d="{d}" fill="none" stroke="{p["ink2"] if i==0 else p["ink3"]}" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>'
    out += text(x + 150, y + 29.5, 'Documents', 15, p['ink'], 700, DISPLAY)
    sx = x + w - 260
    seg = ['grid', 'list', 'cols']
    out += rect(sx - 104, y + 12, 90, 24, 8, p['well'])
    out += rect(sx - 104 + 30 + 2, y + 14, 28, 20, 6, p['overlay'] if p['name'] == 'light' else p['edge'], extra='filter="url(#sh-small)"')
    for i in range(3):
        cx = sx - 104 + 15 + i * 30
        c = p['ink'] if i == 1 else p['ink2']
        if i == 0:
            out += ''.join(rect(cx - 5 + (k % 2) * 6, y + 19 + (k // 2) * 6, 4, 4, 1, c) for k in range(4))
        elif i == 1:
            out += ''.join(line(cx - 5, y + 20 + k * 4, cx + 5, y + 20 + k * 4, c, 1.5) for k in range(3))
        else:
            out += ''.join(rect(cx - 6 + k * 4.5, y + 19, 3, 10, 1, 'none', c, 1.3) for k in range(3))
    out += rect(sx, y + 12, 248, 24, 8, p['well'])
    out += f'<circle cx="{sx+16}" cy="{y+23.5}" r="4.5" fill="none" stroke="{p["ink3"]}" stroke-width="1.5"/>' + line(sx + 19.5, y + 27, sx + 22, y + 29.5, p['ink3'], 1.5)
    out += text(sx + 30, y + 28.5, 'Search', 13, p['ink3'])
    sw = 208
    sy = y + head
    out += label(x + 20, sy + 18, 'Places', p)
    items = ['mikael', 'Desktop', 'Documents', 'Downloads', 'Pictures', 'Music']
    for i, n in enumerate(items):
        out += sidebar_item(x + 10, sy + 28 + i * 30, sw - 20, n, p, n == 'Documents', ['home','desktop','doc','down','image','music'][i])
    out += label(x + 20, sy + 230, 'Devices', p)
    out += sym('drive', x + 18, sy + 244, p['ink2'])
    out += text(x + 44, sy + 258.5, 'Internal', 13, p['ink'])
    out += sym('trash', x + 18, sy + 274, p['ink2'])
    out += text(x + 44, sy + 288.5, 'Trash', 13, p['ink'])
    cx0, cy0, cw, ch = x + sw, sy, w - sw - 6, h - head - 6
    out += rect(cx0, cy0, cw, ch, 8, p['sheet'])
    if p['name'] == 'dark':
        out += rect(cx0 + 0.5, cy0 + 0.5, cw - 1, ch - 1, 7.5, 'none', p['hairline'], 1)
    cols = [('Name', 0), ('Modified', cw * 0.50), ('Size', cw * 0.72), ('Kind', cw * 0.83)]
    for n, cx in cols:
        out += label(cx0 + 20 + cx + (24 if n == 'Name' else 0), cy0 + 24, n, p)
    out += rect(cx0 + 12, cy0 + 34, cw - 24, 1, 0, p['hairline'])
    rows = [('Projects', 'folder', 'Today, 09:12', '—', 'Folder'),
            ('Quarterly report.pdf', file_pdf(), 'Today, 08:47', '2.4 MB', 'PDF document'),
            ('Harbour at dusk.png', file_image(), 'Yesterday, 21:03', '6.1 MB', 'PNG image'),
            ('Notes from the workshop.txt', file_text(), '28 Oct, 16:20', '4 KB', 'Plain text'),
            ('Field recordings', 'folder', '27 Oct, 11:02', '—', 'Folder'),
            ('bande-src.zip', file_archive(), '24 Oct, 18:45', '88.2 MB', 'ZIP archive'),
            ('Invoice 0142.pdf', file_pdf(), '19 Oct, 10:15', '310 KB', 'PDF document'),
            ('Moodboard.png', file_image(), '12 Oct, 14:38', '3.3 MB', 'PNG image')]
    for i, (n, ic, m, s, k) in enumerate(rows):
        ry = cy0 + 42 + i * 32
        sel = i == 1
        fg = p['on_accent'] if sel and active else p['ink']
        fg2 = p['on_accent'] if sel and active else p['ink2']
        if sel:
            out += rect(cx0 + 8, ry, cw - 16, 30, 6, p['accent'] if active else p['well'])
        if ic == 'folder':
            out += mini_folder(cx0 + 18, ry + 4, 22, p['accent'])
        else:
            out += mini(ic, cx0 + 18, ry + 4, 22)
        out += text(cx0 + 48, ry + 19.5, n, 13, fg, 500 if sel else 400)
        out += text(cx0 + 20 + cols[1][1], ry + 19.5, m, 13, fg2)
        out += text(cx0 + 20 + cols[2][1] + 52, ry + 19.5, s, 13, fg2, 400, UI, 'end')
        out += text(cx0 + 20 + cols[3][1], ry + 19.5, k, 13, fg2)
    out += rect(cx0 + 12, cy0 + ch - 34, cw - 24, 1, 0, p['hairline'])
    out += text(cx0 + 20, cy0 + ch - 13, 'Documents', 12, p['ink2'], 500)
    out += text(cx0 + cw - 20, cy0 + ch - 13, '8 items · 110.4 GB free', 12, p['ink3'], 400, UI, 'end')
    return out


def scene(name):
    p = scheme(name)
    W, H = 1280, 800
    body = wallpaper(p, W, H) + bar(p, H) + files_window(p, 150, 90, 1040, 600)
    open(f'window-{name}.svg', 'w').write(doc(W, H, body))


scene('light')
scene('dark')
