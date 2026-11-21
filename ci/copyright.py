#!/usr/bin/python3
# copyright.py [--check]: debian/copyright, the DEP-5 copyright file of the whole source package,
# on stdout, or with --check, fails when debian/copyright differs from it. ci/copyright covers
# this repository and ci/copyright.d/* the third-party material in it; every crate of Cargo.lock,
# which the source carries in vendor/<crate>-<version>, gets a paragraph with its authors and
# licence from its Cargo.toml; then the text of every licence named, from ci/copyright and
# ci/licenses/<name> (spaces as _). Fails on a licence with no text.
import json
import re
import subprocess
import sys
from pathlib import Path

ci = Path(__file__).resolve().parent


def paragraphs(text):
    return [p.strip("\n") for p in re.split(r"\n\s*\n", text) if p.strip()]


def dep5(spdx):
    expression = re.sub(r"\s+WITH\s+(\S+)-exception", r" with \1 exception", spdx.replace("/", " OR "))
    return expression.replace(" OR ", " or ").replace(" AND ", " and ")


def licences(paragraph):
    match = re.search(r"^License: (.*)$", paragraph, re.M)
    if not match or paragraph.startswith("License:"):
        return set()
    rest = paragraph[match.end():]
    if rest.startswith("\n "):
        return set()
    names = re.split(r"\s+(?:and|or)\s+|[(),]", match.group(1))
    return {name.strip() for name in names if name.strip()}


def indented(text):
    lines = text.rstrip().splitlines()
    return "\n".join(" " + line.rstrip() if line.strip() else " ." for line in lines)


def copyright():
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--offline"],
        cwd=ci.parent, check=True, capture_output=True, text=True).stdout)
    own = paragraphs((ci / "copyright").read_text())
    head = [p for p in own if not p.startswith("License:")]
    texts = {p.splitlines()[0].removeprefix("License: "): "\n".join(p.splitlines()[1:]) for p in own
             if p.startswith("License:")}
    for extra in sorted((ci / "copyright.d").iterdir()):
        head += paragraphs(extra.read_text())
    found = []
    for crate in sorted(metadata["packages"], key=lambda crate: (crate["name"], crate["version"])):
        if crate["source"] is None:
            continue
        name, version = crate["name"], crate["version"]
        if not crate["license"]:
            sys.exit(f"copyright: {name} {version} names no licence")
        holders = crate["authors"] or [f"the {name} authors"]
        found.append(f"Files: vendor/{name}-{version}/*\nCopyright: " + "\n ".join(holders)
                     + f"\nLicense: {dep5(crate['license'])}")
    used = sorted(set().union(*(licences(p) for p in head + found)))
    standalone = []
    for name in used:
        file = ci / "licenses" / name.replace(" ", "_")
        if name in texts:
            standalone.append(f"License: {name}\n{texts[name]}")
        elif file.exists():
            standalone.append(f"License: {name}\n{indented(file.read_text())}")
        else:
            sys.exit(f"copyright: no text for the licence {name}: add ci/licenses/{file.name}")
    return "\n\n".join(head + found + standalone) + "\n"


def main():
    text = copyright()
    if sys.argv[1:] == ["--check"]:
        if (ci.parent / "debian" / "copyright").read_text() != text:
            sys.exit("copyright: debian/copyright is stale: write it with ci/copyright.py > debian/copyright")
        return
    sys.stdout.write(text)


main()
