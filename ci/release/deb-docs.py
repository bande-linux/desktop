#!/usr/bin/python3
# deb-docs.py <package> <version> <out dir>: the Debian documentation a cargo-deb package ships,
# generated at build: <out dir>/changelog.gz (one entry for <version>, dated by the last commit of
# the package's crate) and <out dir>/man/<program>.1.gz for each program it installs in /usr/bin
# ($auto or a listed asset), from the Cargo description of the crate that builds it, or
# data/<program>.1 in that crate, which a package's second program needs.
import gzip
import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

root = Path(__file__).resolve().parent
while not (root / "Cargo.lock").exists():
    root = root.parent


def compressed(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "wb") as file:
        with gzip.GzipFile(fileobj=file, mode="wb", mtime=0, filename="") as out:
            out.write(text.encode())


def roff(text):
    lines = []
    for line in text.replace("\\", "\\e").replace("-", "\\-").splitlines():
        lines.append("\\&" + line if line.startswith((".", "'")) else line)
    return "\n".join(lines)


def programs(package, packages):
    deb = package["metadata"]["deb"]
    assets = list(deb.get("assets", []))
    for variant in deb.get("variants", {}).values():
        assets += variant.get("assets", [])
    built = {target["name"]: crate
             for crate in packages for target in crate["targets"] if "bin" in target["kind"]}
    found = {}
    for asset in assets:
        if asset == "$auto":
            for target in package["targets"]:
                if "bin" in target["kind"]:
                    found[target["name"]] = package
            continue
        if isinstance(asset, dict):
            source, destination = asset["source"], asset["dest"]
        else:
            source, destination = asset[:2]
        if destination.startswith(("usr/bin/", "usr/sbin/")):
            name = Path(source).name if destination.endswith("/") else Path(destination).name
            found[name] = built.get(name, package)
    return found


def manual(name, crate, package, version, date):
    summary = crate.get("description") or package["description"]
    described = crate.get("metadata", {}).get("deb", {}).get("extended-description")
    if crate is package or not described:
        described = package["metadata"]["deb"].get("extended-description") or package["description"]
    return "\n".join([
        f'.TH "{name.upper()}" "1" "{date}" "{package["name"]} {version}" "User Commands"',
        ".SH NAME",
        f"{roff(name)} \\- {roff(summary.rstrip('.'))}",
        ".SH SYNOPSIS",
        f".B {roff(name)}",
        ".SH DESCRIPTION",
        roff(" ".join(described.split())),
        "",
    ])


def main():
    name, version, out = sys.argv[1], sys.argv[2], Path(sys.argv[3])
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
        cwd=root, check=True, capture_output=True, text=True).stdout)
    packages = metadata["packages"]
    package = next(crate for crate in packages if crate["name"] == name)
    crate_dir = Path(package["manifest_path"]).parent
    stamp = subprocess.run(["git", "log", "-1", "--format=%ct", "--", str(crate_dir)],
                           cwd=root, check=True, capture_output=True, text=True).stdout.strip()
    when = int(stamp)
    committed = datetime.fromtimestamp(when, timezone.utc)
    date = f"{committed:%Y-%m-%d}"
    maintainer = package["metadata"]["deb"]["maintainer"]
    compressed(out / "changelog.gz", "\n".join([
        f"{name} ({version}) unstable; urgency=medium",
        "",
        f"  * Built from {package['repository']}.",
        "",
        f" -- {maintainer}  {committed:%a, %d %b %Y %H:%M:%S} +0000",
        "",
    ]))
    installed = programs(package, packages)
    for program, crate in installed.items():
        written = Path(crate["manifest_path"]).parent / "data" / f"{program}.1"
        if written.exists():
            page = written.read_text()
        elif crate is package and program != name and len(installed) > 1:
            sys.exit(f"deb-docs: {name} installs {program}: describe it in {written}")
        else:
            page = manual(program, crate, package, version, date)
        compressed(out / "man" / f"{program}.1.gz", page)


main()
