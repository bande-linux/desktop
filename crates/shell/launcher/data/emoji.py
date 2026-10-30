import html
import re
import sys

LATEST_SUPPORTED = 16.0
TONES = ["\U0001F3FB", "\U0001F3FC", "\U0001F3FD", "\U0001F3FE", "\U0001F3FF"]
SELECTOR = "\uFE0F"
SYMBOLS_GROUP = "Characters"


def slug(group):
    return group.lower().replace(" & ", "-").replace(" ", "-")


def annotations(paths):
    keywords, names = {}, {}
    pattern = re.compile(r'<annotation cp="([^"]+)"( type="tts")?>([^<]*)</annotation>')
    for path in paths:
        for match in pattern.finditer(open(path, encoding="utf-8").read()):
            character = html.unescape(match.group(1))
            text = html.unescape(match.group(3))
            if match.group(2):
                names[character] = text
            else:
                keywords[character] = [word.strip() for word in text.split("|")]
    return keywords, names


def emoji(test_path):
    group, entries = None, []
    line_pattern = re.compile(r"^([0-9A-F ]+)\s*; fully-qualified\s*# (\S+) E(\d+\.\d+) (.*)$")
    for line in open(test_path, encoding="utf-8"):
        if line.startswith("# group: "):
            group = line[len("# group: "):].strip()
            continue
        match = line_pattern.match(line.strip())
        if not match or group == "Component" or float(match.group(3)) > LATEST_SUPPORTED:
            continue
        sequence = "".join(chr(int(code, 16)) for code in match.group(1).split())
        entries.append((group, sequence, match.group(4)))
    return entries


def toned(sequence, tone):
    first = sequence[0]
    rest = sequence[1:]
    if rest.startswith(SELECTOR):
        rest = rest[1:]
    return first + tone + rest


def main(test_path, annotation_path, derived_path):
    keywords, names = annotations([annotation_path, derived_path])
    entries = emoji(test_path)
    known = {sequence for _, sequence, _ in entries}
    lines, group = [], None
    for entry_group, sequence, name in entries:
        if any(tone in sequence for tone in TONES):
            continue
        if entry_group != group:
            group = entry_group
            lines.append(slug(group))
        tones = all(toned(sequence, tone) in known for tone in TONES)
        bare = sequence.replace(SELECTOR, "")
        words = keywords.get(sequence) or keywords.get(bare) or []
        name_words = set(name.lower().split())
        extra = [word for word in words if word.lower() not in name_words and word.lower() != name.lower()]
        lines.append("\t".join([sequence, "1" if tones else "", name, "|".join(extra)]))
    lines.append(slug(SYMBOLS_GROUP))
    for character, name in names.items():
        if character in known or character + SELECTOR in known or character in TONES or len(character) != 1:
            continue
        if not character.isprintable() or character.isalnum() or ord(character) < 0x80:
            continue
        name_words = set(name.lower().split())
        extra = [word for word in keywords.get(character, []) if word.lower() not in name_words]
        lines.append("\t".join([character, "", name, "|".join(extra)]))
    sys.stdout.write("\n".join(lines) + "\n")


main(*sys.argv[1:4])
