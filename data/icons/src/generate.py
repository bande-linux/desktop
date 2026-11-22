# generate.py [--check]: writes the Bande icon themes in data/icons and our apps' hicolor icons, or
# with --check fails when any of them differs from what it would write.
import glob
import os
import sys

sys.dont_write_bytecode = True

import apps
import files
import places
import symbols
from common import ROOT

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


def render():
    outputs = {}
    owned = []

    def theme_dir(theme, directory, icons):
        target = os.path.join('data', 'icons', theme, directory)
        owned.append(target)
        for name, draw in icons.items():
            outputs[os.path.join(target, f'{name}.svg')] = draw()

    directories = []
    for size in (16, 24):
        directory = f'{size}x{size}/mimetypes'
        theme_dir('Bande', directory, files.SMALL)
        directories.append((directory, 'MimeTypes', size))
    for directory, (context, icons) in SCALABLE.items():
        theme_dir('Bande', directory, icons)
        directories.append((directory, context, 48))
    for directory, (context, icons) in SYMBOLIC.items():
        theme_dir('Bande', directory, icons)
        directories.append((directory, context, 16))
    outputs[os.path.join('data', 'icons', 'Bande', 'index.theme')] = index(
        'Bande', 'The Bande desktop icons in the Datum language', 'hicolor', directories)
    for accent, icons in places.ACCENT_VARIANTS.items():
        theme = f'Bande-{accent.capitalize()}'
        theme_dir(theme, 'scalable/places', icons)
        outputs[os.path.join('data', 'icons', theme, 'index.theme')] = index(
            theme.replace('-', ' '), f'Bande folders with the {accent} accent', 'Bande',
            [('scalable/places', 'Places', 48)], hidden=True)
    for name, relative in apps.HICOLOR.items():
        outputs[relative] = apps.ICONS[name]()
    return outputs, owned


def stale(outputs, owned):
    return sorted(found for directory in owned
                  for found in glob.glob(os.path.join(ROOT, directory, '*.svg'))
                  if os.path.relpath(found, ROOT) not in outputs)


def check():
    outputs, owned = render()
    problems = [f'{relative} is not what the generator makes' for relative, text in sorted(outputs.items())
                if not os.path.exists(os.path.join(ROOT, relative))
                or open(os.path.join(ROOT, relative)).read() != text]
    problems += [f'{os.path.relpath(found, ROOT)} is not made by the generator' for found in stale(outputs, owned)]
    for problem in problems:
        print(f'generate.py: {problem}', file=sys.stderr)
    return 1 if problems else 0


def main():
    outputs, owned = render()
    for found in stale(outputs, owned):
        os.remove(found)
    for relative, text in outputs.items():
        target = os.path.join(ROOT, relative)
        os.makedirs(os.path.dirname(target), exist_ok=True)
        with open(target, 'w') as out:
            out.write(text)
    return 0


if __name__ == '__main__':
    sys.exit(check() if sys.argv[1:] == ['--check'] else main())
