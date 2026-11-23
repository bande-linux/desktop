#!/usr/bin/env python3
# theme-colors.py [--check]: writes theme/colors.css, the browser's Datum colour tokens, from
# docs/design/datum/src/colors.py, or with --check fails when the file differs from what it would write.
import importlib.util
import os
import sys

sys.dont_write_bytecode = True

BROWSER = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
SOURCE = os.path.join(os.path.dirname(BROWSER), 'docs/design/datum/src/colors.py')
TARGET = os.path.join(BROWSER, 'theme/colors.css')
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


def main():
    with open(TARGET, 'w') as out:
        out.write(stylesheet())


def check():
    with open(TARGET) as current:
        if current.read() == stylesheet():
            return 0
    print(f'{os.path.relpath(TARGET)} is out of date: run browser/scripts/theme-colors.py', file=sys.stderr)
    return 1


sys.exit(check() if sys.argv[1:] == ['--check'] else main())
