#!/usr/bin/python3
# build-depends.py [--check]: the Build-Depends on Debian's librust-*-dev packages that a build
# without the pkg.bande.vendor profile needs, from the crates the workspace uses directly on
# Linux (amd64 and arm64) with the features it asks for. It writes them into debian/control
# between the "# librust:" comments, or with --check fails when they are out of date.
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CONTROL = ROOT / "debian" / "control"
BLOCK = re.compile(r"(# librust: written by ci/deb/build-depends.py\n)(.*?)(# librust: end\n)", re.S)
TRIPLES = ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")


def debian_name(name, version):
    major, minor, patch = (re.match(r"(\d+)\.(\d+)\.(\d+)", version).groups())
    compat = major if major != "0" else f"0.{minor}" if minor != "0" else f"0.0.{patch}"
    return f"librust-{name.replace('_', '-').lower()}-{compat}"


def minimum(requirement):
    match = re.search(r"(\d+)(?:\.(\d+))?(?:\.(\d+))?", requirement)
    return ".".join(part or "0" for part in match.groups())


def wanted():
    found = {}
    for triple in TRIPLES:
        metadata = json.loads(subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--locked", "--offline", "--filter-platform", triple],
            cwd=ROOT, check=True, capture_output=True, text=True).stdout)
        packages = {package["id"]: package for package in metadata["packages"]}
        for node in metadata["resolve"]["nodes"]:
            if node["id"] not in metadata["workspace_members"]:
                continue
            member = packages[node["id"]]
            for edge in node["deps"]:
                used = packages[edge["pkg"]]
                if used["source"] is None:
                    continue
                kinds = {kind["kind"] or "normal" for kind in edge["dep_kinds"]}
                declared = [dependency for dependency in member["dependencies"]
                            if dependency["name"] == used["name"] and (dependency["kind"] or "normal") in kinds]
                for dependency in declared:
                    base = debian_name(used["name"], minimum(dependency["req"]))
                    features = set(dependency["features"])
                    if dependency["uses_default_features"]:
                        features.add("default")
                    test = (dependency["kind"] or "normal") == "dev"
                    names = [base] + [f"{base}+{feature.replace('_', '-').lower()}" for feature in sorted(features)]
                    for name in names:
                        version, only_test = found.get(name, ("0", True))
                        floor = minimum(dependency["req"])
                        if [int(part) for part in floor.split(".")] > [int(part) for part in version.split(".")]:
                            version = floor
                        found[name] = (version, only_test and test)
    return found


def block():
    lines = []
    for name, (version, test) in sorted(wanted().items()):
        profiles = "<!nocheck !pkg.bande.vendor>" if test else "<!pkg.bande.vendor>"
        lines.append(f" {name}-dev (>= {version}-~~) {profiles},\n")
    return "".join(lines)


def main():
    control = CONTROL.read_text()
    match = BLOCK.search(control)
    if not match:
        sys.exit("build-depends: debian/control has no # librust: block")
    current = block()
    if sys.argv[1:] == ["--check"]:
        if match.group(2) != current:
            sys.exit("build-depends: the librust Build-Depends in debian/control are out of date:"
                     " run ci/deb/build-depends.py")
        return
    CONTROL.write_text(control[:match.start(2)] + current + control[match.end(2):])


main()
