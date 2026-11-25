#!/usr/bin/env python3
# theme-colors.py [--check]: writes theme/colors.css, the browser's Datum colour tokens, from
# docs/design/datum/src/colors.py, and theme/controls.css, the window controls' geometry, from title_bar.rs;
# with --check it fails when a file differs from what it would write.
import importlib.util
import os
import re
import sys

sys.dont_write_bytecode = True

BROWSER = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
REPO = os.path.dirname(BROWSER)
SOURCE = os.path.join(REPO, 'docs/design/datum/src/colors.py')
TITLE_BAR = os.path.join(REPO, 'crates/toolkit/graphics/src/title_bar.rs')
NEUTRALS = ('chassis', 'sheet', 'overlay', 'well', 'hairline', 'edge', 'ink', 'ink2', 'ink3')

spec = importlib.util.spec_from_file_location('colors', SOURCE)
colors = importlib.util.module_from_spec(spec)
spec.loader.exec_module(colors)
T = colors.T


def dark_fill_lightness():
    white = [k for k in colors.fill if colors.wcag('#FFFFFF', T[f'fill.dark.{k}']) >= 4.5]
    return round(sum(colors.fill[k][0] for k in white) / len(white), 3)


def block(indent, tokens):
    return ''.join(f'{indent}--bande-{name}: {value};\n' for name, value in tokens)


def stylesheet():
    light = [(k, T[f'light.{k}']) for k in NEUTRALS] + [
        ('field-edge', 'transparent'),
        ('accent', 'AccentColor'),
        ('accent-fill', 'AccentColor'),
        ('on-accent', 'AccentColorText'),
    ]
    dark = [(k, T[f'dark.{k}']) for k in NEUTRALS] + [
        ('accent-fill', f'oklch(from AccentColor {dark_fill_lightness()} c h)'),
        ('on-accent', '#FFFFFF'),
    ]
    contrast = [(k, T[f'contrast.light.{k}']) for k in colors.contrast_light] + [('field-edge', 'var(--bande-edge)')]
    contrast_dark = [(k, T[f'contrast.dark.{k}']) for k in colors.contrast_dark]
    return (
        ':root {\n'
        + block('  ', light)
        + '  @media (prefers-color-scheme: dark) {\n' + block('    ', dark) + '  }\n'
        + '  @media (prefers-contrast: more) {\n' + block('    ', contrast) + '  }\n'
        + '  @media (prefers-contrast: more) and (prefers-color-scheme: dark) {\n' + block('    ', contrast_dark) + '  }\n'
        + '}\n'
    )


def controls():
    with open(TITLE_BAR) as source:
        text = source.read()

    def size(name):
        return float(re.search(rf'^(?:pub )?const {name}: \w+ = ([\d.]+);', text, re.M).group(1))

    return ':root {\n' + block('  ', [(name, f'{value:g}px') for name, value in (
        ('row-height', size('WINDOW_HEADER_HEIGHT')),
        ('row-leading', size('EDGE_INSET')),
        ('well-span', size('EDGE_INSET') + size('WELL_WIDTH')),
        ('menu-size', size('WELL_HEIGHT')),
        ('menu-gap', size('MENU_GAP')),
        ('menu-reach', size('MENU_REACH')),
        ('menu-dot', size('MENU_DOT')),
        ('menu-pitch', size('MENU_DOT_PITCH')),
        ('menu-disc', 2 * size('DISC_RADIUS')),
    )] + [
        ('menu-hover', f"{100 * size('HOVER_OPACITY'):g}%"),
        ('menu-pressed', f"{100 * size('PRESSED_OPACITY'):g}%"),
    ]) + '}\n'


SHEETS = {'theme/colors.css': stylesheet, 'theme/controls.css': controls}


def written(path):
    try:
        with open(path) as current:
            return current.read()
    except FileNotFoundError:
        return None


def main():
    for name, sheet in SHEETS.items():
        with open(os.path.join(BROWSER, name), 'w') as out:
            out.write(sheet())


def check():
    stale = []
    for name, sheet in SHEETS.items():
        path = os.path.join(BROWSER, name)
        if written(path) != sheet():
            stale.append(os.path.relpath(path))
    if stale:
        print(f"{', '.join(stale)} out of date: run browser/scripts/theme-colors.py", file=sys.stderr)
    return int(bool(stale))


sys.exit(check() if sys.argv[1:] == ['--check'] else main())
