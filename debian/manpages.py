#!/usr/bin/python3
# manpages.py <out> <version>: after dh_install, a manual page for each program a package installs
# in /usr/bin, in <out>/<package>/<program>.1: data/<program>.1 of the crate when it has one, or
# else written from the Cargo description of the crate that builds the program and the package's
# description in debian/control. A package's second program needs its own data/<program>.1.
import json
import os
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path


def roff(text):
    lines = []
    for line in text.replace("\\", "\\e").replace("-", "\\-").splitlines():
        lines.append("\\&" + line if line.startswith((".", "'")) else line)
    return "\n".join(lines)


def descriptions():
    found = {}
    for stanza in re.split(r"\n\s*\n", Path("debian/control").read_text()):
        package = re.search(r"^Package: (\S+)$", stanza, re.M)
        description = re.search(r"^Description: .*\n((?: .*\n?)*)", stanza, re.M)
        if package and description:
            body = [line[1:] for line in description.group(1).splitlines()]
            found[package.group(1)] = " ".join(" ".join(line for line in body if line != ".").split())
    return found


def main():
    out, version = Path(sys.argv[1]), sys.argv[2]
    date = f"{datetime.fromtimestamp(int(os.environ['SOURCE_DATE_EPOCH']), timezone.utc):%Y-%m-%d}"
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1"],
        check=True, capture_output=True, text=True).stdout)
    crates = {target["name"]: crate for crate in metadata["packages"]
              for target in crate["targets"] if "bin" in target["kind"]}
    written = {page.stem: page for page in Path("crates").glob("*/*/data/*.1")}
    packages = descriptions()
    for package, described in packages.items():
        programs = sorted(Path("debian", package, "usr/bin").glob("*"))
        for program in (path.name for path in programs):
            target = out / package / f"{program}.1"
            target.parent.mkdir(parents=True, exist_ok=True)
            if program in written:
                target.write_text(written[program].read_text())
                continue
            if program != package and len(programs) > 1:
                sys.exit(f"manpages: {package} installs {program}: describe it in its crate's data/{program}.1")
            crate = crates[program]
            target.write_text("\n".join([
                f'.TH "{program.upper()}" "1" "{date}" "{package} {version}" "User Commands"',
                ".SH NAME",
                f"{roff(program)} \\- {roff(crate['description'].rstrip('.'))}",
                ".SH SYNOPSIS",
                f".B {roff(program)}",
                ".SH DESCRIPTION",
                roff(described),
                "",
            ]))


main()
