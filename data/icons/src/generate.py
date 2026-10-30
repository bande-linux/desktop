import glob
import os
import sys

sys.dont_write_bytecode = True

import apps
import files
import places
import symbols
from common import ROOT

ICONS = os.path.join(ROOT, 'data', 'icons')

SCALABLE = {
    'scalable/apps': ('Applications', apps.ICONS),
    'scalable/mimetypes': ('MimeTypes', files.ICONS),
    'scalable/places': ('Places', places.ICONS),
    'scalable/devices': ('Devices', places.DEVICES),
}

SYMBOLIC = {f'symbolic/{context}': (context.capitalize(), icons) for context, icons in symbols.SVGS.items()}


def section(directory, context, size):
    if directory.startswith('scalable') or directory.startswith('symbolic'):
        return f'\n[{directory}]\nContext={context}\nSize={size}\nMinSize=8\nMaxSize=512\nType=Scalable\n'
    return f'\n[{directory}]\nContext={context}\nSize={size}\nType=Fixed\n'


def index(name, comment, inherits, directories, hidden=False):
    text = (f'[Icon Theme]\nName={name}\nComment={comment}\nInherits={inherits}\n'
            f'Directories={",".join(directory for directory, _, _ in directories)}\n')
    if hidden:
        text += 'Hidden=true\n'
    return text + ''.join(section(*entry) for entry in directories)


def write(theme, directory, icons):
    target = os.path.join(ICONS, theme, directory)
    os.makedirs(target, exist_ok=True)
    for stale in glob.glob(os.path.join(target, '*.svg')):
        os.remove(stale)
    for name, draw in sorted(icons.items()):
        with open(os.path.join(target, f'{name}.svg'), 'w') as out:
            out.write(draw())


def main():
    directories = []
    for size in (16, 24):
        directory = f'{size}x{size}/mimetypes'
        write('Unbound', directory, files.SMALL)
        directories.append((directory, 'MimeTypes', size))
    for directory, (context, icons) in SCALABLE.items():
        write('Unbound', directory, icons)
        directories.append((directory, context, 48))
    for directory, (context, icons) in SYMBOLIC.items():
        write('Unbound', directory, icons)
        directories.append((directory, context, 16))
    with open(os.path.join(ICONS, 'Unbound', 'index.theme'), 'w') as out:
        out.write(index('Unbound', 'The Unbound desktop icons in the Datum language', 'Adwaita,hicolor', directories))
    for accent, icons in places.ACCENT_VARIANTS.items():
        theme = f'Unbound-{accent.capitalize()}'
        write(theme, 'scalable/places', icons)
        with open(os.path.join(ICONS, theme, 'index.theme'), 'w') as out:
            out.write(index(theme.replace('-', ' '), f'Unbound folders with the {accent} accent', 'Unbound',
                            [('scalable/places', 'Places', 48)], hidden=True))
    for name, relative in apps.HICOLOR.items():
        with open(os.path.join(ROOT, relative), 'w') as out:
            out.write(apps.ICONS[name]())


if __name__ == '__main__':
    main()
