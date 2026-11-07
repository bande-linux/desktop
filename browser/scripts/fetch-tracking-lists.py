#!/usr/bin/env python3
"""fetch-tracking-lists.py <source dir>: packs Mozilla's Tracking Protection lists for this Firefox
version into the source, like the other Remote Settings dumps, so they arrive only with an update."""
import hashlib
import json
import os
import re
import sys
import urllib.request

SERVER = "https://firefox.settings.services.mozilla.com/v1"
COLLECTION = "tracking-protection-lists"


def get(url):
    with urllib.request.urlopen(url, timeout=120) as response:
        return response.read()


def version_parts(text):
    return [int(part) for part in re.findall(r"\d+", text)[:2]]


def applies(record, version):
    for wanted, op in re.findall(
        r"""env\.version\|versionCompare\(["']([^"']+)["']\)\s*(>=|<=|<|>)\s*0""",
        record.get("filter_expression", ""),
    ):
        here, there = version_parts(version), version_parts(wanted)
        if not {">=": here >= there, "<=": here <= there, "<": here < there, ">": here > there}[op]:
            return False
    return True


def main():
    src = sys.argv[1]
    with open(os.path.join(src, "browser/config/version.txt")) as source:
        version = source.read().strip()
    dumps = os.path.join(src, "services/settings/dumps/main")
    attachments = json.loads(get(SERVER + "/"))["capabilities"]["attachments"]["base_url"]
    records = json.loads(get(f"{SERVER}/buckets/main/collections/{COLLECTION}/records"))["data"]
    chosen = {}
    for record in records:
        if record.get("attachment") and applies(record, version):
            if record["Name"] not in chosen or record["Version"] > chosen[record["Name"]]["Version"]:
                chosen[record["Name"]] = record
    if not {"base-track-digest256", "content-track-digest256", "mozstd-trackwhite-digest256"} <= set(chosen):
        sys.exit(f"tracking lists for {version} are missing: {sorted(chosen)}")
    folder = os.path.join(dumps, COLLECTION)
    os.makedirs(folder)
    files = []
    for name, record in sorted(chosen.items()):
        data = get(attachments + record["attachment"]["location"])
        if hashlib.sha256(data).hexdigest() != record["attachment"]["hash"]:
            sys.exit(f"{name}: the download doesn't match its hash")
        with open(os.path.join(folder, name), "wb") as out:
            out.write(data)
        files.append(name)
        record.pop("filter_expression", None)
    with open(os.path.join(dumps, COLLECTION + ".json"), "w") as out:
        json.dump({"data": list(chosen.values()), "timestamp": max(r["last_modified"] for r in chosen.values())}, out)
    with open(os.path.join(dumps, "moz.build"), "a") as out:
        out.write(f'\nFINAL_TARGET_FILES.defaults.settings.main += ["{COLLECTION}.json"]\n')
        out.write(f'FINAL_TARGET_FILES.defaults.settings.main["{COLLECTION}"] += [\n')
        out.writelines(f'    "{COLLECTION}/{name}",\n' for name in files)
        out.write("]\n")
    print(f"tracking lists: {len(files)} for {version}")


main()
