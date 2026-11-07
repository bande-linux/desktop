#!/usr/bin/env python3
"""Build dictionary packs and their .debs: build.py <version> <sources> <out> [pack...]. See docs/architecture/lookup.md."""
import json
import pathlib
import shutil
import struct
import subprocess
import sys
import urllib.request
import xml.etree.ElementTree as ET

MAGIC = b"BDICT1\n\0"
MAX_ENTRIES_PER_FORM = 40
FOLKETS = "https://folkets-lexikon.csc.kth.se/folkets/"
KAIKKI = "https://kaikki.org/dictionary/Swedish/kaikki.org-dictionary-Swedish.jsonl"
CC_BY_SA_25 = "Creative Commons Attribution-ShareAlike 2.5, https://creativecommons.org/licenses/by-sa/2.5/"
CC_BY_SA_40 = "Creative Commons Attribution-ShareAlike 4.0, https://creativecommons.org/licenses/by-sa/4.0/"
ENGLISH_RULES = (
    "noun:s: noun:ses:s noun:xes:x noun:zes:z noun:ches:ch noun:shes:sh noun:men:man noun:ies:y "
    "verb:s: verb:ies:y verb:es:e verb:es: verb:ed:e verb:ed: verb:ing:e verb:ing: "
    "adj:er: adj:est: adj:er:e adj:est:e"
)
FOLKETS_PARTS = {"nn": "noun", "vb": "verb", "jj": "adj", "ab": "adv"}


def write_pack(path, meta, entries):
    meta = "".join(f"{key}={value}\n" for key, value in meta.items()).encode()
    bodies, places, forms, at = [], [], {}, 0
    for keys, word, pos, lines in entries:
        body = "\n".join([word, pos] + [f"{kind}\t{text}" for kind, text in lines]).encode()
        places.append(at)
        bodies.append(struct.pack("<I", len(body)) + body)
        at += 4 + len(body)
        for key in keys:
            forms.setdefault(key.encode(), []).append(len(places) - 1)
    keys = sorted(forms)
    offsets_at = 12 + len(meta) + 4
    record_at = offsets_at + 4 * len(keys)
    offsets, records = [], []
    for key in keys:
        chosen = forms[key][:MAX_ENTRIES_PER_FORM]
        offsets.append(record_at)
        records.append((key, chosen))
        record_at += len(key) + 3 + 4 * len(chosen)
    with open(path, "wb") as out:
        out.write(MAGIC + struct.pack("<I", len(meta)) + meta + struct.pack("<I", len(keys)))
        out.write(struct.pack(f"<{len(offsets)}I", *offsets))
        for key, chosen in records:
            out.write(key + b"\0" + struct.pack("<H", len(chosen)))
            out.write(struct.pack(f"<{len(chosen)}I", *[record_at + places[i] for i in chosen]))
        for body in bodies:
            out.write(body)


def clean(text):
    return " ".join(text.split())


def fetch(url, path):
    if not path.exists():
        urllib.request.urlretrieve(url, path)
    return path


def wordnet_entries(sources):
    root = sources / "wordnet"
    if not root.exists():
        root.mkdir()
        subprocess.run(["apt-get", "download", "wordnet-base"], cwd=root, check=True)
        deb = next(root.glob("wordnet-base_*.deb"))
        subprocess.run(["dpkg-deb", "-x", str(deb), "."], cwd=root, check=True)
    data = root / "usr/share/wordnet"
    forms, found = {}, []
    for pos, name in (("noun", "noun"), ("verb", "verb"), ("adj", "adj"), ("adv", "adv")):
        synsets = {}
        for line in open(data / f"data.{name}", encoding="latin1"):
            if line.startswith("  "):
                continue
            head, gloss = line.split("|", 1)
            parts = head.split()
            count = int(parts[3], 16)
            words = [parts[4 + 2 * i].split("(")[0].replace("_", " ") for i in range(count)]
            synsets[parts[0]] = (words, clean(gloss))
        for line in open(data / f"index.{name}", encoding="latin1"):
            if line.startswith("  "):
                continue
            parts = line.split()
            word = parts[0].replace("_", " ")
            senses = int(parts[2])
            lines = []
            for offset in parts[-senses:]:
                words, gloss = synsets[offset]
                lines.append(("D", gloss))
                others = [other for other in words if other.lower() != word]
                if others:
                    lines.append(("S", ", ".join(others)))
            found.append(([f"en:{word}"], word, pos, lines, int(parts[-senses - 1])))
            forms[(pos, word)] = found[-1][0]
        for line in open(data / f"{name}.exc", encoding="latin1"):
            form, *bases = line.split()
            for base in bases:
                known = forms.get((pos, base.replace("_", " ")))
                if known is not None:
                    known.append(f"en:{form.replace('_', ' ')}")
    found.sort(key=lambda entry: -entry[4])
    return [entry[:4] for entry in found]


def folkets_entries(sources):
    found = []
    for name, language in (("folkets_sv_en_public.xml", "sv"), ("folkets_en_sv_public.xml", "en")):
        path = fetch(FOLKETS + name, sources / name)
        for word in ET.parse(path).getroot().iter("word"):
            value = word.get("value")
            if not value:
                continue
            lines = []
            translations = [item.get("value") for item in word.findall("translation")]
            if translations:
                lines.append(("T", ", ".join(item for item in translations if item)))
            lines += [("D", item.get("value")) for item in word.findall("definition") if item.get("value")]
            lines += [("E", item.get("value")) for item in word.findall("example")[:2] if item.get("value")]
            if not lines:
                continue
            words = len(value.split())
            forms = {value.lower()}
            forms |= {
                item.get("value", "").lower()
                for item in word.findall("paradigm/inflection")
                if len(item.get("value", "").split()) == words
            }
            keys = [f"{language}:{form}" for form in sorted(forms) if form]
            found.append((keys, value, FOLKETS_PARTS.get((word.get("class") or "").split(".")[0], ""), lines))
    return found


def wiktionary_entries(sources):
    path = fetch(KAIKKI, sources / "kaikki-sv.jsonl")
    found = []
    for line in open(path, encoding="utf8"):
        item = json.loads(line)
        lines, target = [], None
        for sense in item.get("senses", []):
            lines += [("D", clean(gloss)) for gloss in sense.get("glosses", [])[:1]]
            for form in sense.get("form_of", []) + sense.get("alt_of", []):
                target = target or form.get("word")
        if not lines:
            continue
        lines = lines[:8]
        if target:
            lines.insert(0, ("O", clean(target)))
        forms = {item["word"].lower()} | {form.get("form", "").lower() for form in item.get("forms", [])}
        keys = [f"sv:{form}" for form in sorted(forms) if form and "\t" not in form]
        found.append((keys, item["word"], item.get("pos", ""), lines))
    return found


PACKS = {
    "bande-dict-en": {
        "file": "wordnet",
        "entries": wordnet_entries,
        "meta": {
            "id": "wordnet",
            "name": "WordNet",
            "attribution": "WordNet 3.0, Princeton University. Free with notice.",
            "languages": "en",
            "definitions.en": "en",
            "kind.en": "definition",
            "priority": "20",
            "rules.en": ENGLISH_RULES,
        },
        "summary": "English dictionary for Look Up (WordNet)",
        "licence": "WordNet 3.0 Copyright 2006 by Princeton University. See https://wordnet.princeton.edu/license-and-commercial-use\nThe licence text from Debian's wordnet-base follows.\n\n",
        "licence_file": "wordnet/usr/share/doc/wordnet-base/copyright",
    },
    "bande-dict-sv": {
        "file": "folkets",
        "entries": folkets_entries,
        "meta": {
            "id": "folkets",
            "name": "Folkets lexikon",
            "attribution": "Folkets lexikon, CC BY-SA 2.5",
            "languages": "sv en",
            "definitions.sv": "sv",
            "definitions.en": "sv",
            "kind.sv": "translation",
            "kind.en": "translation",
            "priority": "10",
            "compound.sv": "3",
        },
        "summary": "Swedish-English dictionary for Look Up (Folkets lexikon)",
        "licence": f"Folkets lexikon, KTH, https://folkets-lexikon.csc.kth.se/\n{CC_BY_SA_25}\nConverted to the Bande pack format.\n",
    },
    "bande-dict-sv-wiktionary": {
        "file": "wiktionary-sv",
        "entries": wiktionary_entries,
        "meta": {
            "id": "wiktionary-sv",
            "name": "Wiktionary",
            "attribution": "Wiktionary via kaikki.org, CC BY-SA",
            "languages": "sv",
            "definitions.sv": "en",
            "kind.sv": "definition",
            "priority": "20",
            "compound.sv": "3",
        },
        "summary": "Swedish words from Wiktionary for Look Up",
        "licence": f"Wiktionary contributors, extracted by kaikki.org (wiktextract).\n{CC_BY_SA_40}\nConverted to the Bande pack format.\n",
    },
}


def debian_arch_all_deb(package, spec, pack, version, out):
    root = out / f"{package}.root"
    shutil.rmtree(root, ignore_errors=True)
    folder = root / "usr/share/bande/dict"
    docs = root / f"usr/share/doc/{package}"
    folder.mkdir(parents=True)
    docs.mkdir(parents=True)
    (root / "DEBIAN").mkdir()
    shutil.copy(pack, folder)
    (docs / "copyright").write_text(spec["licence"])
    (root / "DEBIAN/control").write_text(
        f"Package: {package}\nVersion: {version}\nArchitecture: all\n"
        f"Maintainer: Mikael Hug <mikaelhug@gmail.com>\nSection: text\nPriority: optional\n"
        f"Installed-Size: {pack.stat().st_size // 1024}\nDescription: {spec['summary']}\n"
    )
    deb = out / f"{package}_{version}_all.deb"
    subprocess.run(["dpkg-deb", "--root-owner-group", "-Zxz", "-b", str(root), str(deb)], check=True)
    shutil.rmtree(root)
    return deb


def main():
    version, sources, out = sys.argv[1], pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3])
    sources.mkdir(parents=True, exist_ok=True)
    out.mkdir(parents=True, exist_ok=True)
    for package in sys.argv[4:] or PACKS:
        spec = PACKS[package]
        pack = out / f"{spec['file']}.bdict"
        write_pack(pack, spec["meta"], spec["entries"](sources))
        if "licence_file" in spec:
            spec["licence"] += (sources / spec["licence_file"]).read_text()
        deb = debian_arch_all_deb(package, spec, pack, version, out)
        print(package, pack.stat().st_size // 1000000, "MB pack,", deb.stat().st_size // 1000000, "MB deb")


if __name__ == "__main__":
    main()
