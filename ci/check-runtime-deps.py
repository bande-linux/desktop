#!/usr/bin/env python3
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
TABLE = ROOT / "ci" / "runtime-deps"
KINDS = {"depends", "required", "optional"}
PATH = re.compile(r'"((?:/usr)?/(?:s?bin|libexec|share|lib)/[A-Za-z0-9._+/-]+)"')
PROGRAM = re.compile(r'Command::new\(\s*"([A-Za-z0-9._+-]+)"\s*\)')
CONSTANT = re.compile(r'const ([A-Z_]+): &str = "([A-Za-z0-9._+/-]+)";')
CONSTANT_PROGRAM = re.compile(r'Command::new\(\s*([A-Z_]+)\s*\)')
TEST_MODULE = re.compile(r"#\[cfg\(test\)\]\s*mod \w+ \{")
SKIPPED = {"tests", "testing", "benches", "examples", "fuzz"}


def referenced(crate_root):
    found = {}
    for source in sorted((crate_root / "src").rglob("*.rs")):
        relative = source.relative_to(ROOT)
        if SKIPPED & {*relative.parts, relative.stem}:
            continue
        text = TEST_MODULE.split(source.read_text(encoding="utf-8"))[0]
        constants = dict(CONSTANT.findall(text))
        items = PATH.findall(text) + PROGRAM.findall(text)
        items += [constants[name] for name in CONSTANT_PROGRAM.findall(text) if name in constants]
        for item in items:
            found.setdefault(item, str(relative))
    return found


def names(relation):
    for alternative in re.split(r"[,|]", relation):
        name = re.sub(r"\(.*?\)|\[.*?\]", "", alternative).strip()
        if name and not name.startswith("$"):
            yield name


def control():
    depends = {}
    for stanza in re.split(r"\n\s*\n", (ROOT / "debian" / "control").read_text(encoding="utf-8")):
        package = re.search(r"^Package: (\S+)$", stanza, re.M)
        if package:
            relation = re.search(r"^Depends:((?:.*)(?:\n .*)*)", stanza, re.M)
            depends[package.group(1)] = set(names(relation.group(1) if relation else ""))
    installers = {}
    for install in (ROOT / "debian").glob("*.install"):
        for line in install.read_text(encoding="utf-8").splitlines():
            if line.startswith("debian/build/bin/"):
                installers[line.split()[0].removeprefix("debian/build/bin/")] = install.stem
    return depends, installers


def packages():
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--offline", "--locked", "--no-deps"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout)
    depends, installers = control()
    crates = {}
    for package in metadata["packages"]:
        name = package["name"]
        binaries = [target["name"] for target in package["targets"] if "bin" in target["kind"]]
        deb = name if name in depends else next(
            (installers[binary] for binary in binaries if binary in installers), None)
        crates[name] = {
            "root": pathlib.Path(package["manifest_path"]).parent,
            "deb": (deb, depends[deb]) if deb else None,
            "uses": [dependency["name"] for dependency in package["dependencies"]
                     if dependency.get("path") and dependency["kind"] is None],
        }
    return crates


def shipped_in(crates, name, seen):
    if name in seen or name not in crates:
        return set()
    seen.add(name)
    crate = crates[name]
    found = {name}
    for used in crate["uses"]:
        if crates.get(used) and crates[used]["deb"] is None:
            found |= shipped_in(crates, used, seen)
    return found


def ours(package):
    return package.startswith(("bande-", "libpam-bande-", "xdg-desktop-portal-bande"))


def main():
    problems = []
    table = {}
    for number, line in enumerate(TABLE.read_text(encoding="utf-8").splitlines(), 1):
        fields = line.split()
        if len(fields) != 3 or fields[2] not in KINDS:
            problems.append(f"{TABLE.name}:{number}: expected <path or program> <package> <{'|'.join(sorted(KINDS))}>")
            continue
        table[fields[0]] = (fields[1], fields[2])
    crates = packages()
    shipping = {}
    for name, crate in crates.items():
        if crate["deb"] is not None:
            for library in shipped_in(crates, name, set()):
                shipping.setdefault(library, []).append(crate["deb"])
    for library, debs in sorted(shipping.items()):
        own = crates[library]["deb"]
        for item, source in sorted(referenced(crates[library]["root"]).items()):
            if item not in table:
                problems.append(f"{source}: runs or reads {item}, which ci/runtime-deps does not list")
                continue
            package, kind = table[item]
            if kind != "depends" or ours(package):
                continue
            if own is not None and package not in own[1]:
                problems.append(f"{source}: {own[0]} runs or reads {item} from {package}, not in its Depends")
            elif own is None and not any(package in depends for _, depends in debs):
                problems.append(f"{source}: runs or reads {item} from {package}, which no package using it Depends on")
    problems = list(dict.fromkeys(problems))
    for problem in problems:
        print(problem)
    if problems:
        print("every program the desktop runs and every system file it reads is listed in ci/runtime-deps:"
              " see docs/architecture/ci.md")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
