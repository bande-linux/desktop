from common import (EMBER, INK, INK3, accent, circle, mix, path, rect, semantic,
                    stroke, svg, tone)
from mono import text_path

SHEET = '#FFFFFF'
HINT = INK
BAND_TOP = 31

COLORS = {
    'generic': INK3,
    'text': '#6E6A64',
    'markdown': tone(80, 0.36, 0.01),
    'code': accent('teal'),
    'html': tone(235, 0.52, 0.12),
    'pdf': semantic('danger'),
    'image': accent('moss'),
    'audio': accent('plum'),
    'video': accent('rose'),
    'archive': accent('amber'),
    'document': accent('ultramarine'),
    'spreadsheet': semantic('success'),
    'presentation': EMBER,
    'font': tone(55, 0.47, 0.07),
    'executable': tone(80, 0.30, 0.01),
    'disk': tone(250, 0.50, 0.035),
}


def lines(widths, top=9.5, step=3.6, left=14, opacity=0.42, height=1.6):
    return ''.join(rect(left, top + index * step, width, height, height / 2, HINT, f'opacity="{opacity}"')
                   for index, width in enumerate(widths) if width)


def hint_text():
    return lines((20, 18, 20, 9, 16, 19))


def hint_markdown():
    return (rect(14, 9, 3, 3, 0.4, HINT, 'opacity="0.7"') + rect(18.5, 9.3, 12, 2.4, 1.2, HINT, 'opacity="0.7"')
            + lines((20, 17, 20, 12), top=15))


def hint_code():
    code = accent('teal')
    return (rect(14, 9.5, 8, 1.6, 0.8, code, 'opacity="0.85"') + rect(23.5, 9.5, 8, 1.6, 0.8, HINT, 'opacity="0.42"')
            + lines((None, 12, 14, 8), left=17.5)
            + rect(14, 9.5 + 4 * 3.6, 4, 1.6, 0.8, code, 'opacity="0.85"')
            + rect(14, 9.5 + 5 * 3.6, 16, 1.6, 0.8, HINT, 'opacity="0.42"'))


def hint_script():
    return (stroke('M14.5 9.5l3 2.5-3 2.5', HINT, 1.5, 'opacity="0.7"') + rect(19.5, 13.2, 6, 1.6, 0.8, HINT, 'opacity="0.7"')
            + lines((18, 13, 19), top=18.5))


def hint_html():
    color = COLORS['html']
    return (stroke('M18 11l-4.5 4.5 4.5 4.5M30 11l4.5 4.5-4.5 4.5', color, 1.8)
            + stroke('M25.6 9.5l-3.2 12', HINT, 1.5, 'opacity="0.5"')
            + lines((20,), top=25))


def hint_pdf():
    return (rect(14, 9, 20, 7, 1, COLORS['pdf'], 'opacity="0.16"')
            + lines((14, 0), top=11.7, left=17, opacity=0.5)
            + lines((20, 17, 20), top=19))


def hint_image():
    moss = accent('moss')
    return (rect(13, 8, 22, 19, 1.5, tone(230, 0.90, 0.04))
            + '<clipPath id="pic"><rect x="13" y="8" width="22" height="19" rx="1.5"/></clipPath>'
            + f'<g clip-path="url(#pic)">'
            + path('M13 24l7-8 5 5 3-3 7 7V27H13Z', moss)
            + path('M13 27v-2.2l6.6-3.4 5.6 3 3.6-1.4 6.2 2.4V27Z', mix(moss, '#000000', 0.25))
            + '</g>'
            + rect(28, 10.5, 3.6, 3.6, 0.4, EMBER))


def hint_audio():
    plum = accent('plum')
    heights = (3, 7, 12, 8, 15, 10, 5, 9, 4)
    out = ''
    for index, height in enumerate(heights):
        out += rect(13.6 + index * 2.4, 18 - height / 2, 1.5, height, 0.75, plum, 'opacity="0.85"')
    return out + lines((20,), top=27.5)


def hint_video():
    out = rect(13, 8, 22, 18, 2, tone(5, 0.30, 0.05))
    for index in range(4):
        out += rect(14.6 + index * 5.3, 9.2, 2.6, 1.8, 0.4, '#FFFFFF', 'opacity="0.55"')
        out += rect(14.6 + index * 5.3, 23, 2.6, 1.8, 0.4, '#FFFFFF', 'opacity="0.55"')
    return out + path('M21.6 13.6v6.8a.6.6 0 0 0 .9.5l5.6-3.4a.6.6 0 0 0 0-1l-5.6-3.4a.6.6 0 0 0-.9.5Z', '#FFFFFF')


def hint_archive():
    out = ''
    for index in range(8):
        out += rect(22 + (index % 2) * 2, 4.5 + index * 2.6, 2, 1.8, 0.3, HINT, 'opacity="0.6"')
    return out + rect(20.8, 26, 6.4, 3.4, 1, HINT, 'opacity="0.7"') + rect(22.6, 27, 2.8, 1.4, 0.5, '#FFFFFF')


def hint_document():
    blue = accent('ultramarine')
    return (rect(14, 9, 12, 2.6, 1.3, blue, 'opacity="0.85"')
            + lines((20, 20, 18, 20, 10), top=14.5))


def hint_spreadsheet():
    green = semantic('success')
    out = rect(13, 8.5, 22, 19.5, 1.2, green, 'opacity="0.10"')
    out += rect(13, 8.5, 22, 4, 1.2, green, 'opacity="0.35"')
    for x in (19, 27):
        out += rect(x, 8.5, 0.9, 19.5, 0, green, 'opacity="0.55"')
    for y in (12.5, 17.5, 22.5):
        out += rect(13, y, 22, 0.9, 0, green, 'opacity="0.55"')
    return out


def hint_presentation():
    ember = EMBER
    return (rect(12.5, 8, 23, 16, 1.5, tone(55, 0.95, 0.03))
            + rect(15.5, 17, 3, 4.5, 0.4, ember, 'opacity="0.55"')
            + rect(20.2, 13.5, 3, 8, 0.4, ember, 'opacity="0.75"')
            + rect(24.9, 11, 3, 10.5, 0.4, ember)
            + rect(29.6, 15, 3, 6.5, 0.4, ember, 'opacity="0.55"')
            + lines((14,), top=27, left=17))


def hint_font():
    d, _ = text_path('Aa', 24, 23, 15, -0.6)
    return path(d, HINT, 'opacity="0.85"') + lines((20,), top=26.5)


def hint_executable():
    dark = COLORS['executable']
    return (rect(13.5, 8.5, 21, 17, 2.5, dark)
            + stroke('M17.5 13l3.5 3-3.5 3', '#FFFFFF', 1.6)
            + rect(22.5, 18.4, 6, 1.6, 0.8, '#FFFFFF', 'opacity="0.8"')
            + rect(30, 11, 2, 2, 0.3, EMBER))


def hint_disk():
    slate = COLORS['disk']
    return (circle(24, 17, 10, slate, 'opacity="0.18"')
            + circle(24, 17, 10, 'none', f'stroke="{slate}" stroke-width="1.2" stroke-opacity="0.7"')
            + circle(24, 17, 6, 'none', f'stroke="{slate}" stroke-width="0.8" stroke-opacity="0.35"')
            + circle(24, 17, 2.4, '#FFFFFF')
            + circle(24, 17, 2.4, 'none', f'stroke="{slate}" stroke-width="1"'))


def hint_generic():
    return lines((20, 14, 20, 17), opacity=0.25)


HINTS = {
    'generic': hint_generic, 'text': hint_text, 'markdown': hint_markdown, 'code': hint_code,
    'script': hint_script, 'html': hint_html, 'pdf': hint_pdf, 'image': hint_image, 'audio': hint_audio,
    'video': hint_video, 'archive': hint_archive, 'document': hint_document, 'spreadsheet': hint_spreadsheet,
    'presentation': hint_presentation, 'font': hint_font, 'executable': hint_executable, 'disk': hint_disk,
}


def sheet(kind, label, small=False):
    color = COLORS['code' if kind == 'script' else kind]
    body = (rect(9, 4, 30, 40, 3, SHEET)
            + rect(9.5, 4.5, 29, 39, 2.5, 'none', 'stroke="#000000" stroke-opacity="0.16" stroke-width="1"')
            + HINTS[kind]()
            + path(f'M9 {BAND_TOP}H39V41a3 3 0 0 1-3 3H12a3 3 0 0 1-3-3Z', color))
    if not small:
        size = 8 if len(label) <= 3 else 7.2 if len(label) == 4 else 6.2
        text, _ = text_path(label, 24, 40.6, size, 0.4 if len(label) <= 3 else 0.15)
        body += path(text, '#FFFFFF')
    return svg(48, body)


MIMETYPES = {
    'application-x-generic': ('generic', 'FILE'),
    'text-x-generic': ('text', 'TXT'),
    'text-plain': ('text', 'TXT'),
    'text-x-log': ('text', 'LOG'),
    'text-csv': ('spreadsheet', 'CSV'),
    'text-markdown': ('markdown', 'MD'),
    'text-x-markdown': ('markdown', 'MD'),
    'text-x-script': ('script', 'SH'),
    'application-x-shellscript': ('script', 'SH'),
    'text-x-python': ('code', 'PY'),
    'text-x-python3': ('code', 'PY'),
    'text-rust': ('code', 'RS'),
    'text-x-csrc': ('code', 'C'),
    'text-x-chdr': ('code', 'H'),
    'text-x-c++src': ('code', 'CPP'),
    'text-x-go': ('code', 'GO'),
    'text-x-java': ('code', 'JAVA'),
    'text-javascript': ('code', 'JS'),
    'application-javascript': ('code', 'JS'),
    'application-x-javascript': ('code', 'JS'),
    'application-json': ('code', 'JSON'),
    'application-toml': ('code', 'TOML'),
    'application-yaml': ('code', 'YAML'),
    'application-xml': ('code', 'XML'),
    'text-xml': ('code', 'XML'),
    'text-css': ('code', 'CSS'),
    'text-x-patch': ('code', 'DIFF'),
    'text-html': ('html', 'HTML'),
    'application-xhtml+xml': ('html', 'HTML'),
    'application-pdf': ('pdf', 'PDF'),
    'image-x-generic': ('image', 'IMG'),
    'image-png': ('image', 'PNG'),
    'image-jpeg': ('image', 'JPG'),
    'image-gif': ('image', 'GIF'),
    'image-webp': ('image', 'WEBP'),
    'image-svg+xml': ('image', 'SVG'),
    'image-bmp': ('image', 'BMP'),
    'image-tiff': ('image', 'TIFF'),
    'image-heif': ('image', 'HEIC'),
    'image-avif': ('image', 'AVIF'),
    'image-x-xcf': ('image', 'XCF'),
    'audio-x-generic': ('audio', 'AUD'),
    'audio-mpeg': ('audio', 'MP3'),
    'audio-flac': ('audio', 'FLAC'),
    'audio-x-flac': ('audio', 'FLAC'),
    'audio-ogg': ('audio', 'OGG'),
    'audio-x-vorbis+ogg': ('audio', 'OGG'),
    'audio-x-opus+ogg': ('audio', 'OPUS'),
    'audio-x-wav': ('audio', 'WAV'),
    'audio-mp4': ('audio', 'M4A'),
    'audio-aac': ('audio', 'AAC'),
    'video-x-generic': ('video', 'VID'),
    'video-mp4': ('video', 'MP4'),
    'video-x-matroska': ('video', 'MKV'),
    'video-webm': ('video', 'WEBM'),
    'video-quicktime': ('video', 'MOV'),
    'video-x-msvideo': ('video', 'AVI'),
    'video-ogg': ('video', 'OGV'),
    'package-x-generic': ('archive', 'ARC'),
    'application-zip': ('archive', 'ZIP'),
    'application-x-tar': ('archive', 'TAR'),
    'application-x-compressed-tar': ('archive', 'TGZ'),
    'application-x-xz-compressed-tar': ('archive', 'TXZ'),
    'application-x-bzip2-compressed-tar': ('archive', 'TBZ'),
    'application-x-zstd-compressed-tar': ('archive', 'TZST'),
    'application-gzip': ('archive', 'GZ'),
    'application-x-xz': ('archive', 'XZ'),
    'application-zstd': ('archive', 'ZST'),
    'application-x-7z-compressed': ('archive', '7Z'),
    'application-vnd.rar': ('archive', 'RAR'),
    'application-x-rar': ('archive', 'RAR'),
    'application-vnd.debian.binary-package': ('archive', 'DEB'),
    'x-office-document': ('document', 'DOC'),
    'application-vnd.oasis.opendocument.text': ('document', 'ODT'),
    'application-vnd.openxmlformats-officedocument.wordprocessingml.document': ('document', 'DOCX'),
    'application-msword': ('document', 'DOC'),
    'application-rtf': ('document', 'RTF'),
    'application-epub+zip': ('document', 'EPUB'),
    'x-office-spreadsheet': ('spreadsheet', 'XLS'),
    'application-vnd.oasis.opendocument.spreadsheet': ('spreadsheet', 'ODS'),
    'application-vnd.openxmlformats-officedocument.spreadsheetml.sheet': ('spreadsheet', 'XLSX'),
    'application-vnd.ms-excel': ('spreadsheet', 'XLS'),
    'x-office-presentation': ('presentation', 'PPT'),
    'application-vnd.oasis.opendocument.presentation': ('presentation', 'ODP'),
    'application-vnd.openxmlformats-officedocument.presentationml.presentation': ('presentation', 'PPTX'),
    'application-vnd.ms-powerpoint': ('presentation', 'PPT'),
    'font-x-generic': ('font', 'FONT'),
    'font-ttf': ('font', 'TTF'),
    'font-otf': ('font', 'OTF'),
    'font-woff': ('font', 'WOFF'),
    'font-woff2': ('font', 'WOF2'),
    'application-x-font-ttf': ('font', 'TTF'),
    'application-x-executable': ('executable', 'BIN'),
    'application-x-sharedlib': ('executable', 'LIB'),
    'application-x-appimage': ('executable', 'APP'),
    'application-x-desktop': ('executable', 'APP'),
    'application-x-cd-image': ('disk', 'ISO'),
    'application-x-raw-disk-image': ('disk', 'IMG'),
    'application-vnd.efi.iso': ('disk', 'ISO'),
    'application-vnd.efi.img': ('disk', 'IMG'),
    'application-x-qemu-disk': ('disk', 'QCOW'),
}

ICONS = {name: (lambda kind=kind, label=label: sheet(kind, label)) for name, (kind, label) in MIMETYPES.items()}
SMALL = {name: (lambda kind=kind, label=label: sheet(kind, label, small=True)) for name, (kind, label) in MIMETYPES.items()}
