#!/usr/bin/python3
# copyright.py <crate>...: the DEP-5 copyright file of the package built from these crates, on
# stdout. ci/copyright covers this repository; each crate linked into them (cargo tree, without
# build, dev and proc-macro crates) gets a paragraph with its authors and licence from its Cargo.toml, or
# ci/copyright.d/<crate> instead ({version} is its version) where it needs more, such as bundled
# C code or embedded data; then the text of every licence named, from ci/copyright and
# ci/licenses/<name> (spaces as _). Fails on a licence with no text.
import json
import re
import subprocess
import sys
from pathlib import Path

ci = Path(__file__).resolve().parent


def cargo(*args):
    return subprocess.run(["cargo", *args, "--locked"], cwd=ci.parent, check=True,
                          capture_output=True, text=True).stdout


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


def main():
    crates = sys.argv[1:]
    metadata = json.loads(cargo("metadata", "--format-version", "1"))
    packages = {(p["name"], p["version"]): p for p in metadata["packages"]}
    selected = [argument for crate in crates for argument in ("-p", crate)]
    tree = cargo("tree", "-q", *selected, "-e", "normal,no-proc-macro", "--prefix", "none",
                 "--format", "{p}")
    linked = sorted({(line.split()[0], line.split()[1].lstrip("v")) for line in tree.splitlines() if line})

    own = paragraphs((ci / "copyright").read_text())
    head = [p for p in own if not p.startswith("License:")]
    texts = {p.splitlines()[0].removeprefix("License: "): "\n".join(p.splitlines()[1:]) for p in own
             if p.startswith("License:")}
    found = []
    for name, version in linked:
        crate = packages[(name, version)]
        extra = ci / "copyright.d" / name
        if extra.exists():
            found += paragraphs(extra.read_text().replace("{version}", version))
        elif crate["source"] is not None:
            if not crate["license"]:
                sys.exit(f"copyright: {name} {version} names no licence")
            holders = crate["authors"] or [f"the {name} authors"]
            found.append(f"Files: crates.io/{name}-{version}/*\nCopyright: " + "\n ".join(holders)
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
    print("\n\n".join(head + found + standalone))


main()
