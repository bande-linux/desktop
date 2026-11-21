#!/usr/bin/python3
# debian-gap.py <json out>: what building without the pkg.bande.vendor profile still lacks in
# this Debian release: each librust-*-dev Build-Depends that dpkg-checkbuilddeps finds unmet,
# with what Debian's APT lists offer: installable, too old, a feature not packaged, or not packaged
# at that major version (with the version Debian has of the crate). Writes a Markdown summary to
# stdout and the same as JSON to <json out>.
import json
import re
import subprocess
import sys
from pathlib import Path


def unmet():
    result = subprocess.run(["dpkg-checkbuilddeps"], capture_output=True, text=True)
    match = re.search(r"unmet build dependencies: (.*)", result.stderr)
    if not match:
        return []
    return re.findall(r"(librust-[^\s()]+-dev)(?: \(>= ([^)]+)\))?", match.group(1))


def offered(name):
    shown = subprocess.run(["apt-cache", "showpkg", name], capture_output=True, text=True).stdout
    versions = re.findall(r"^(\S+) \(/var/lib/apt", shown, re.M)
    provides = shown.split("Reverse Provides:", 1)[1] if "Reverse Provides:" in shown else ""
    versions += re.findall(r"^\S+ \S+ \(= (\S+)\)$", provides, re.M)
    return versions


def newest(versions):
    best = None
    for version in versions:
        if best is None or subprocess.run(["dpkg", "--compare-versions", version, "gt", best]).returncode == 0:
            best = version
    return best


def satisfies(version, floor):
    return subprocess.run(["dpkg", "--compare-versions", version, "ge", floor]).returncode == 0


def main():
    out = Path(sys.argv[1])
    gaps = []
    for name, floor in unmet():
        best = newest(offered(name))
        crate = re.sub(r"(\+.*)?-dev$", "", name)
        if best is None:
            base = newest(offered(f"{crate}-dev")) if "+" in name else None
            state = "feature not packaged" if base else "not packaged"
            best = base or newest(offered(re.sub(r"-[0-9.]+$", "-dev", crate)))
        elif floor and not satisfies(best, floor):
            state = "too old"
        else:
            state = "installable"
        gaps.append({"package": name, "needs": floor, "debian": best, "state": state})
    out.write_text(json.dumps(gaps, indent=1) + "\n")
    counts = {}
    for gap in gaps:
        counts[gap["state"]] = counts.get(gap["state"], 0) + 1
    print("## Building from Debian's packaged crates\n")
    if not gaps:
        print("Every librust Build-Depends is satisfied.")
        return
    print(", ".join(f"{count} {state}" for state, count in sorted(counts.items())) + "\n")
    print("| Package | Needs | Debian has | State |\n|---|---|---|---|")
    for gap in sorted((gap for gap in gaps if gap["state"] != "installable"),
                      key=lambda gap: (gap["state"], gap["package"])):
        print(f"| `{gap['package']}` | {gap['needs'] or ''} | {gap['debian'] or '–'} | {gap['state']} |")


main()
