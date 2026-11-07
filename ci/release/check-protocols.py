#!/usr/bin/python3
import subprocess
import sys
import xml.etree.ElementTree as ElementTree
from pathlib import Path

PROTOCOLS = Path("crates/platform/protocols/protocols")
SAME_BUILD_ONLY = {"bande_shell_v1"}


def git(*arguments):
    return subprocess.run(["git", *arguments], capture_output=True, text=True).stdout.strip()


def messages(interface, kind):
    return [
        (
            message.get("name"),
            message.get("since", "1"),
            message.get("type"),
            [tuple(sorted(arg.attrib.items() - {("summary", arg.get("summary"))})) for arg in message.findall("arg")],
        )
        for message in interface.findall(kind)
    ]


def enums(interface):
    return {
        enum.get("name"): {entry.get("name"): entry.get("value") for entry in enum.findall("entry")}
        for enum in interface.findall("enum")
    }


def compare(file, before, after):
    problems = []
    now = {interface.get("name"): interface for interface in after.findall("interface")}
    for old in before.findall("interface"):
        name = old.get("name")
        if name in SAME_BUILD_ONLY:
            continue
        new = now.get(name)
        if new is None:
            problems.append(f"{file}: {name} was removed")
            continue
        old_version, new_version = int(old.get("version")), int(new.get("version"))
        grew = False
        for kind in ("request", "event"):
            was, is_now = messages(old, kind), messages(new, kind)
            if is_now[: len(was)] != was:
                problems.append(f"{file}: {name} changed an existing {kind}; append a new one instead")
            for message in is_now[len(was):]:
                grew = True
                if int(message[1]) <= old_version:
                    problems.append(f"{file}: {name}.{message[0]} needs since > {old_version}")
        if new_version < old_version or (grew and new_version == old_version):
            problems.append(f"{file}: {name} needs a higher version than {old_version}")
        new_enums = enums(new)
        for enum, entries in enums(old).items():
            kept = new_enums.get(enum, {})
            if any(kept.get(entry) != value for entry, value in entries.items()):
                problems.append(f"{file}: {name}.{enum} changed an existing entry")
    return problems


def main():
    release = git("describe", "--tags", "--abbrev=0", "--match", "v[0-9]*")
    if not release:
        print("no release tag here to compare the protocols with")
        return 0
    problems = []
    for path in sorted(PROTOCOLS.glob("*.xml")):
        released = subprocess.run(["git", "show", f"{release}:{path}"], capture_output=True, text=True)
        if released.returncode == 0:
            problems += compare(path.name, ElementTree.fromstring(released.stdout), ElementTree.parse(path).getroot())
    print(*problems, sep="\n", file=sys.stderr)
    if problems:
        print(f"the protocols must stay compatible with {release}: see docs/architecture/session.md", file=sys.stderr)
    return 1 if problems else 0


sys.exit(main())
