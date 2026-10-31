from kit import *

L, D = scheme('light'), scheme('dark')
W, H = 1280, 900
out = rect(0, 0, W, H, 0, L['sheet'])
out += text(40, 56, 'Datum', 34, L['ink'], 800, DISPLAY)
out += text(40, 82, 'Quiet surfaces · one signal · exact edges', 14, L['ink2'], 500)

y = 120
out += label(40, y, 'Neutrals — light', L)
roles = ['chassis', 'sheet', 'overlay', 'well', 'hairline', 'edge', 'ink3', 'ink2', 'ink']
for i, r in enumerate(roles):
    x = 40 + i * 66
    out += rect(x, y + 12, 58, 44, 6, L[r], L['hairline'], 1)
    out += text(x, y + 72, r, 11, L['ink'], 600)
    out += text(x, y + 86, L[r], 10, L['ink3'], 400, MONO)
out += rect(640, y - 16, 640, 124, 0, D['sheet'])
out += label(660, y, 'Neutrals — dark', D)
for i, r in enumerate(roles):
    x = 660 + i * 66
    out += rect(x, y + 12, 58, 44, 6, D[r], D['hairline'], 1)
    out += text(x, y + 72, r, 11, D['ink'], 600)
    out += text(x, y + 86, D[r], 10, D['ink3'], 400, MONO)

y = 250
out += label(40, y, 'Accents — light / dark  (default: ultramarine)', L)
names = ['ultramarine', 'teal', 'moss', 'amber', 'ember', 'rose', 'plum', 'graphite']
for i, n in enumerate(names):
    x = 40 + i * 74
    out += rect(x, y + 12, 32, 40, 6, T[f'accent.light.{n}'])
    out += rect(x + 34, y + 12, 32, 40, 6, T[f'accent.dark.{n}'])
    out += text(x, y + 68, n, 11, L['ink'], 600)
    out += text(x, y + 82, T[f'accent.light.{n}'], 10, L['ink3'], 400, MONO)
out += label(660, y, 'Semantic — light / dark', L)
for i, n in enumerate(['danger', 'warning', 'success']):
    x = 660 + i * 110
    out += rect(x, y + 12, 48, 40, 6, T[f'sem.light.{n}'])
    out += rect(x + 50, y + 12, 48, 40, 6, T[f'sem.dark.{n}'])
    out += text(x, y + 68, n, 11, L['ink'], 600)
    out += text(x, y + 82, T[f'sem.light.{n}'] + ' / ' + T[f'sem.dark.{n}'], 9.5, L['ink3'], 400, MONO)

y = 370
out += label(40, y, 'Type', L)
specs = [
    ('Display 28/34 · Manrope Bold', 'Settings', 28, 700, DISPLAY),
    ('Title 22/28 · Manrope Bold', 'Sound & Displays', 22, 700, DISPLAY),
    ('Heading 17/22 · Manrope Bold', 'Quit Terminal?', 17, 700, DISPLAY),
    ('Window title 15/20 · Manrope Bold', 'Documents', 15, 700, DISPLAY),
    ('Body 13/18 · Inter Regular', 'Two processes are still running in “build.sh”.', 13, 400, UI),
    ('Body strong 13/18 · Inter Semibold', 'Copy finished', 13, 600, UI),
    ('Callout 12/16 · Inter Regular', '8 items · 110.4 GB free · 14:32', 12, 400, UI),
    ('Label 10.5/14 · Inter Semibold caps +0.07em', 'PLACES · NAME · MODIFIED', 10.5, 600, UI),
    ('Mono 13/18 · JetBrains Mono', 'mikael@bande:~$ cargo test -p toolkit', 13, 400, MONO),
]
yy = y + 22
for meta, sample, size, wgt, fam in specs:
    out += text(40, yy + size * 0.8, sample, size, L['ink'], wgt, fam, 'start', 0.7 if 'caps' in meta else 0)
    out += text(560, yy + size * 0.8, meta, 11, L['ink3'], 500, UI)
    yy += max(size * 1.35, 24) + 6

x0, y0 = 860, 370
out += label(x0, y0, 'Radii — concentric', L)
for i, r in enumerate((4, 6, 8, 10, 14)):
    out += rect(x0 + i * 76, y0 + 14, 64, 48, r, L['well'])
    out += text(x0 + i * 76, y0 + 78, f'r{r}', 11, L['ink'], 600)
    out += text(x0 + i * 76 + 22, y0 + 78, ['check', 'control', 'sheet', 'menu', 'window'][i], 10.5, L['ink3'])
out += rect(x0, y0 + 100, 220, 120, 14, L['chassis'])
out += rect(x0 + 6, y0 + 106 + 22, 208, 86, 8, L['sheet'])
out += rect(x0 + 16, y0 + 106 + 34, 120, 28, 6, L['accent'])
out += text(x0 + 236, y0 + 122, 'window 14', 11, L['ink2'])
out += text(x0 + 236, y0 + 140, '− inset 6 = sheet 8', 11, L['ink2'])
out += text(x0 + 236, y0 + 158, 'menu 10 − inset 4 = item 6', 11, L['ink2'])
out += text(x0 + 236, y0 + 176, 'floor 4; pills only for', 11, L['ink2'])
out += text(x0 + 236, y0 + 192, 'switch, slider track, badge', 11, L['ink2'])

y = 640
out += label(x0, y, 'Spacing (pt)', L)
for i, s in enumerate((2, 4, 8, 12, 16, 24, 32, 48)):
    out += rect(x0 + i * 46, y + 14 + 48 - s, 36, s, 0, L['accent'])
    out += text(x0 + i * 46, y + 80, str(s), 11, L['ink'], 600)
out += label(x0, y + 110, 'Elevation', L)
for i, (n, f) in enumerate((('chassis', ''), ('sheet', ''), ('overlay', 'sh-overlay'), ('window', 'sh-window'))):
    x = x0 + i * 96
    fill = L[n] if n != 'window' else L['chassis']
    e = f'filter="url(#{f})"' if f else ''
    out += rect(x, y + 124, 80, 56, 10, fill, L['hairline'] if not f else None, 1, e)
    out += text(x, y + 198, n, 11, L['ink'], 600)

y = 790
out += label(40, y, 'Motifs', L)
out += f'<g transform="translate(40 {y+16})">' + ''.join(rect(c * 9, r * 9, 6, 6, 0, L['ink']) for c, r in ((0, 0), (0, 1), (0, 2), (1, 2), (2, 0), (2, 1), (2, 2))) + '</g>'
out += text(80, y + 34, 'mark: a U of seven squares', 12, L['ink2'])
out += rect(290, y + 20, 6, 6, 0, L['accent']) + text(304, y + 28, 'square signal: running, cursor, recording, unread', 12, L['ink2'])
out += rect(640, y + 14, 3, 18, 1.5, L['accent']) + text(652, y + 28, 'tick: the active app, the focused tab', 12, L['ink2'])
out += window_controls(40, y + 50, L) + text(120, y + 66, 'window controls: one well, three neutral glyphs; colour only on hover (close)', 12, L['ink2'])

open('sheet.svg', 'w').write(doc(W, H, out))
