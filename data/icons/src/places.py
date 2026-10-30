import symbols
from common import (ACCENTS, EMBER, INK, PAPER, STONE, STONE_SHADE, accent, circle, gradient, mix, path, rect,
                    stroke, svg, tone)

ENGRAVE = tone(80, 0.52, 0.02)


def engraved(draw, cx=24, cy=29.5, size=14.5):
    scale = size / 16
    body = ''.join(f'<path d="{shape.d()}" opacity="{opacity}"/>' for shape, opacity, _ in draw())
    place = f'translate({cx - size / 2} {cy - size / 2}) scale({scale})'
    return (f'<g transform="translate(0 0.7) {place}" fill="#FFFFFF" opacity="0.55">{body}</g>'
            f'<g transform="{place}" fill="{ENGRAVE}">{body}</g>')


def folder(tab, symbol=None):
    defs = gradient('body', mix(STONE, '#FFFFFF', 0.25), STONE)
    out = (path('M4 12a3 3 0 0 1 3-3h10.2a3 3 0 0 1 2.1.9L21.5 12H41a3 3 0 0 1 3 3v3H4Z', tab)
           + rect(4, 15, 40, 27, 3, mix(tab, '#000000', 0.18), 'opacity="0.35"')
           + rect(4, 16, 40, 27, 3, 'url(#body)')
           + rect(4, 16, 40, 1, 0, '#FFFFFF', 'opacity="0.6"')
           + rect(4.5, 16.5, 39, 26, 2.5, 'none', f'stroke="{STONE_SHADE}" stroke-width="1"'))
    if symbol:
        out += engraved(symbol)
    return svg(48, out, defs)


FOLDERS = {
    'folder': None,
    'inode-directory': None,
    'folder-drag-accept': None,
    'user-home': symbols.home,
    'user-desktop': symbols.desktop,
    'folder-documents': lambda: symbols.page('lines'),
    'folder-download': symbols.download,
    'folder-music': symbols.music,
    'folder-pictures': symbols.picture,
    'folder-videos': symbols.video,
    'folder-templates': symbols.templates,
    'folder-publicshare': symbols.people,
    'folder-remote': symbols.globe,
    'network-workgroup': symbols.globe,
}


def trash(full=False):
    body_top, body_bottom = tone(80, 0.93, 0.008), tone(80, 0.78, 0.012)
    rib = tone(80, 0.50, 0.014)
    defs = gradient('can', body_top, body_bottom, 0, 0, 1, 0) + gradient('lid', tone(80, 0.97, 0.004), tone(80, 0.86, 0.01))
    out = ''
    if full:
        out += ('<g transform="rotate(-8 20 14)">' + rect(13, 5, 15, 18, 1.5, PAPER, f'stroke="{rib}" stroke-opacity="0.4" stroke-width="0.8"')
                + ''.join(rect(15.5, 8.5 + i * 2.6, 9 if i % 2 else 10, 1.1, 0.55, INK, 'opacity="0.3"') for i in range(4)) + '</g>'
                + '<g transform="rotate(9 30 14)">' + rect(22, 6, 14, 17, 1.5, tone(272, 0.93, 0.03), f'stroke="{rib}" stroke-opacity="0.4" stroke-width="0.8"')
                + rect(25, 10, 8, 6, 0.8, accent('ultramarine'), 'opacity="0.35"') + '</g>')
    out += (path('M10 15h28l-2.3 25.2a3 3 0 0 1-3 2.8H15.3a3 3 0 0 1-3-2.8Z', 'url(#can)')
            + path('M10 15h28l-2.3 25.2a3 3 0 0 1-3 2.8H15.3a3 3 0 0 1-3-2.8Z', 'none', f'stroke="{rib}" stroke-opacity="0.6" stroke-width="1"'))
    for x1, x2 in ((17.5, 18.4), (24, 24), (30.5, 29.6)):
        out += stroke(f'M{x1} 20L{x2} 38', rib, 1.6, 'opacity="0.7"')
    if not full:
        out += (rect(8, 10.5, 32, 5, 2.5, 'url(#lid)') + rect(8.5, 11, 31, 4, 2, 'none', f'stroke="{rib}" stroke-opacity="0.6" stroke-width="1"')
                + rect(19, 8, 10, 3, 1.5, 'none', f'stroke="{rib}" stroke-width="1.4"'))
    else:
        out += rect(8.5, 13.5, 31, 3, 1.5, rib, 'opacity="0.35"')
    return svg(48, out, defs)


def harddisk():
    defs = gradient('top', tone(80, 0.94, 0.006), tone(80, 0.86, 0.008))
    return svg(48, path('M8 17l4-6h24l4 6Z', 'url(#top)')
               + rect(8, 17, 32, 18, 3, tone(80, 0.42, 0.01))
               + rect(8, 17, 32, 1.2, 0, '#FFFFFF', 'opacity="0.18"')
               + rect(12, 24.5, 14, 2.4, 1.2, '#FFFFFF', 'opacity="0.22"')
               + circle(34, 26, 1.8, EMBER)
               + path('M8 17l4-6h24l4 6Z', 'none', f'stroke="{tone(80, 0.62, 0.01)}" stroke-width="0.8" stroke-linejoin="round"'), defs)


def removable():
    defs = gradient('stick', tone(80, 0.42, 0.01), tone(80, 0.28, 0.01))
    return svg(48, rect(17, 5, 14, 12, 1.5, tone(80, 0.88, 0.006))
               + rect(17.5, 5.5, 13, 11, 1, 'none', f'stroke="{tone(80, 0.62, 0.01)}" stroke-width="1"')
               + rect(20, 9, 2.6, 2.6, 0.4, tone(80, 0.45, 0.01)) + rect(25.4, 9, 2.6, 2.6, 0.4, tone(80, 0.45, 0.01))
               + rect(14, 16, 20, 28, 4, 'url(#stick)')
               + rect(14, 16, 20, 1.2, 0, '#FFFFFF', 'opacity="0.2"')
               + rect(19, 24, 10, 12, 2, '#FFFFFF', 'opacity="0.12"')
               + circle(24, 39.5, 1.4, EMBER), defs)


def icons(tab_accent):
    tab = accent(tab_accent)
    return {name: (lambda symbol=symbol: folder(tab, symbol)) for name, symbol in FOLDERS.items()}


ICONS = {
    **icons('ultramarine'),
    'user-trash': trash,
    'user-trash-full': lambda: trash(True),
}

DEVICES = {
    'drive-harddisk': harddisk,
    'drive-removable-media': removable,
    'media-removable': removable,
}

ACCENT_VARIANTS = {name: icons(name) for name in ACCENTS if name != 'ultramarine'}
