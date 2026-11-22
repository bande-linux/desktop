#!/usr/bin/python3
# metainfo.py <out> <metainfo>...: writes each metainfo to <out> with <releases> from debian/changelog,
# one release per upstream version, dated by its newest entry and described by its entries' items.
import re
import sys
from email.utils import parsedate_to_datetime
from pathlib import Path
from xml.sax.saxutils import escape

HEADER = re.compile(r"^\S+ \((?:\d+:)?([^)]+)\) ")
TRAILER = re.compile(r"^ -- .+?  (.+)$")


def upstream(version):
    version = version.rsplit("-", 1)[0] if "-" in version else version
    return re.split(r"[~+]", version)[0]


def releases(changelog):
    found = []
    version, items, item = None, [], None
    for line in changelog.splitlines():
        header = HEADER.match(line)
        trailer = TRAILER.match(line)
        if header:
            version, items, item = upstream(header.group(1)), [], None
        elif trailer and version:
            date = parsedate_to_datetime(trailer.group(1)).strftime("%Y-%m-%d")
            if found and found[-1][0] == version:
                found[-1][2].extend(items)
            else:
                found.append((version, date, items))
            version = None
        elif line.startswith("  * "):
            item = [line[4:].strip()]
            items.append(item)
        elif line.startswith("    ") and item is not None:
            item.append(line.strip())
    return found


def block(found):
    lines = ["  <releases>"]
    for version, date, items in found:
        lines.append(f'    <release version="{escape(version)}" date="{date}">')
        if items:
            lines += ["      <description>", "        <ul>"]
            lines += [f"          <li>{escape(' '.join(item))}</li>" for item in items]
            lines += ["        </ul>", "      </description>"]
        lines.append("    </release>")
    lines.append("  </releases>")
    return "\n".join(lines) + "\n"


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    added = block(releases(Path("debian/changelog").read_text()))
    for source in map(Path, sys.argv[2:]):
        text = source.read_text()
        if not text.rstrip().endswith("</component>"):
            sys.exit(f"{source}: does not end with </component>")
        cut = text.rstrip().rfind("</component>")
        (out / source.name).write_text(text[:cut] + added + "</component>\n")


main()
