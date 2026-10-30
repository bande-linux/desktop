from kit import *
def sheet(p, ox, oy, w):
    out = rect(ox, oy, w, 400, 0, p['sheet'])
    out += text(ox+32, oy+44, 'Apps — Disc', 18, p['ink'], 700, DISPLAY)
    out += text(ox+32, oy+64, '48 grid · 44 disc · flat field · one symbol · one square signal', 12, p['ink3'])
    apps = [('Files', icon_files()), ('Terminal', icon_terminal()), ('Settings', icon_settings()),
            ('Monitor', icon_monitor()), ('Text', icon_text()), ('Capture', icon_capture())]
    for i, (n, ic) in enumerate(apps):
        x = ox + 32 + i * 92
        out += mini(ic, x, oy + 84, 72)
        out += text(x + 36, oy + 176, n, 12, p['ink2'], 500, UI, 'middle')
    out += text(ox+32, oy+234, 'Files — Sheet + label', 18, p['ink'], 700, DISPLAY)
    files = [('Folder', folder()), ('Downloads', folder('down')), ('report.pdf', file_pdf()),
             ('photo.png', file_image()), ('notes.txt', file_text()), ('src.zip', file_archive())]
    for i, (n, ic) in enumerate(files):
        x = ox + 32 + i * 92
        out += mini(ic, x, oy + 252, 72)
        out += text(x + 36, oy + 344, n, 12, p['ink2'], 500, UI, 'middle')
    # small sizes
    for i, (_, ic) in enumerate(apps + files):
        out += mini(ic, ox + 32 + i * 26, oy + 362, 20)
    return out
body = sheet(scheme('light'), 0, 0, 600) + sheet(scheme('dark'), 600, 0, 600)
open('icons.svg','w').write(doc(1200, 400, body))
