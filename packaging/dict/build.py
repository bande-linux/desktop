#!/usr/bin/env python3
"""Build dictionary packs and their .debs: build.py <version> <sources> <out> [package...], or build.py --pin <sources>. See docs/architecture/lookup.md."""
import hashlib
import json
import pathlib
import shutil
import struct
import subprocess
import sys
import xml.etree.ElementTree as ET

MAGIC = b"BDICT1\n\0"
MAX_ENTRIES_PER_FORM = 40
CC_BY_40 = "Creative Commons Attribution 4.0, https://creativecommons.org/licenses/by/4.0/"
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


PINS = pathlib.Path(__file__).with_name("sources")
PINS_HEADER = "# name sha256 url: the source data, checked by build.py, fetched from our copy (MIRROR) first; refreshed by build.py --pin\n"
MIRROR_RELEASE = "dict-sources"
MIRROR = f"https://github.com/bande-linux/desktop/releases/download/{MIRROR_RELEASE}/"
MIRROR_API = f"https://api.github.com/repos/bande-linux/desktop/releases/tags/{MIRROR_RELEASE}"


def pins():
    pinned = {}
    for line in PINS.read_text().splitlines():
        if line and not line.startswith("#"):
            name, sha256, url = line.split()
            pinned[name] = (sha256, url)
    return pinned


def digest(path):
    hashed = hashlib.sha256()
    with open(path, "rb") as data:
        for block in iter(lambda: data.read(1 << 20), b""):
            hashed.update(block)
    return hashed.hexdigest()


def download(url, path, *headers):
    command = ["curl", "-fsSL", "--retry", "5", "--retry-all-errors", "--retry-delay", "20"]
    for header in headers:
        command += ["-H", header]
    return subprocess.run([*command, "-o", str(path), url], check=False).returncode == 0


def mirror_assets(sources):
    listing = sources / "mirror.json"
    if not listing.exists() and not download(MIRROR_API, listing, "Accept: application/vnd.github+json"):
        return {}
    return {asset["name"]: asset["url"] for asset in json.loads(listing.read_text()).get("assets", [])}


def from_mirror(sources, sha256, path):
    packed = path.with_name(f"{path.name}.xz")
    name = f"{sha256}.xz"
    asset = mirror_assets(sources).get(name)
    fetched = (asset is not None and download(asset, packed, "Accept: application/octet-stream")) or download(
        MIRROR + name, packed
    )
    if not fetched:
        return False
    with open(path, "wb") as plain:
        unpacked = subprocess.run(["xz", "-dc", str(packed)], stdout=plain, check=False).returncode == 0
    packed.unlink()
    return unpacked


def source(sources, name):
    sha256, url = pins()[name]
    path = sources / name
    if path.exists() and digest(path) == sha256:
        return path
    if from_mirror(sources, sha256, path) and digest(path) == sha256:
        return path
    print(f"{name}: not in our copy at {MIRROR}, fetching {url}", file=sys.stderr)
    if not download(url, path):
        sys.exit(f"cannot fetch {name} from our copy or {url}")
    found = digest(path)
    if found != sha256:
        sys.exit(f"{name} changed upstream: {found}, pinned {sha256}; check it, run build.py --pin and add it to our copy")
    return path


def pin(sources):
    sources.mkdir(parents=True, exist_ok=True)
    lines = []
    for name, (_, url) in pins().items():
        path = sources / name
        path.unlink(missing_ok=True)
        if not download(url, path):
            sys.exit(f"cannot fetch {url}")
        sha256 = digest(path)
        with open(sources / f"{sha256}.xz", "wb") as packed:
            subprocess.run(["xz", "-T0", "-6", "-c", str(path)], stdout=packed, check=True)
        lines.append(f"{name} {sha256} {url}")
    PINS.write_text(PINS_HEADER + "\n".join(lines) + "\n")
    print(f"Upload the new {sources}/<sha256>.xz files to the {MIRROR_RELEASE} release, then commit packaging/dict/sources")


def wordnet(sources):
    root = sources / "oewn"
    if not root.exists():
        root.mkdir()
        archive = source(sources, "oewn-2025.zip")
        subprocess.run(["unzip", "-q", str(archive)], cwd=root, check=True)
    data = next(path for path in root.glob("oewn*") if path.is_dir())
    forms, dictionary, thesaurus = {}, [], []
    for pos in ("noun", "verb", "adj", "adv"):
        synsets = {}
        for line in open(data / f"data.{pos}", encoding="utf8"):
            if line.startswith("  ") or "|" not in line:
                continue
            head, gloss = line.split("|", 1)
            parts = head.split()
            count = int(parts[3], 16)
            words = [parts[4 + 2 * i].split("(")[0].replace("_", " ") for i in range(count)]
            synsets[parts[0]] = (words, clean(gloss))
        for line in open(data / f"index.{pos}", encoding="utf8"):
            if line.startswith("  ") or not line.strip():
                continue
            parts = line.split()
            word = parts[0].replace("_", " ")
            senses = int(parts[2])
            meanings, synonyms = [], []
            for offset in parts[-senses:]:
                words, gloss = synsets[offset]
                meanings.append(("D", gloss))
                others = [other for other in words if other.lower() != word]
                if others:
                    meanings.append(("S", ", ".join(others)))
                    synonyms.append(("S", ", ".join(others)))
            keys = [f"en:{word}"]
            weight = int(parts[-senses - 1])
            dictionary.append((keys, word, pos, meanings, weight))
            if synonyms:
                thesaurus.append((keys, word, pos, synonyms, weight))
            forms[(pos, word)] = keys
        for line in open(data / f"{pos}.exc", encoding="utf8"):
            if not line.strip():
                continue
            form, *bases = line.split()
            for base in bases:
                known = forms.get((pos, base.replace("_", " ")))
                if known is not None:
                    known.append(f"en:{form.replace('_', ' ')}")
    for found in (dictionary, thesaurus):
        found.sort(key=lambda entry: -entry[4])
    return [entry[:4] for entry in dictionary], [entry[:4] for entry in thesaurus]


def wordnet_dictionary(sources):
    return wordnet(sources)[0]


def wordnet_thesaurus(sources):
    return wordnet(sources)[1]


def synlex_entries(sources):
    root = sources / "mythes-sv"
    if not root.exists():
        root.mkdir()
        deb = source(sources, "mythes-sv.deb")
        subprocess.run(["dpkg-deb", "-x", str(deb), "."], cwd=root, check=True)
    lines = open(next(root.glob("usr/share/mythes/th_sv_SE*.dat")), encoding="latin1").read().splitlines()[1:]
    found, at = [], 0
    while at < len(lines):
        word, count = lines[at].rsplit("|", 1)
        meanings = []
        for meaning in lines[at + 1 : at + 1 + int(count)]:
            pos, *synonyms = meaning.split("|")
            synonyms = [synonym for synonym in synonyms if synonym and synonym.lower() != word.lower()]
            if synonyms:
                meanings.append(("S", ", ".join(synonyms)))
        at += 1 + int(count)
        if meanings:
            found.append(([f"sv:{word.lower()}"], word, "", meanings))
    return found


def folkets_entries(sources):
    found = []
    for name, language in (("folkets_sv_en_public.xml", "sv"), ("folkets_en_sv_public.xml", "en")):
        path = source(sources, name)
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


def wiktionary_entries(sources, language, name):
    path = source(sources, f"kaikki-{language}.jsonl")
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
        keys = [f"{language}:{form}" for form in sorted(forms) if form and "\t" not in form]
        found.append((keys, item["word"], item.get("pos", ""), lines))
    return found


PACKS = {
    "bande-dict-en-wiktionary": {
        "packs": [
            {
                "file": "wiktionary-en",
                "entries": lambda sources: wiktionary_entries(sources, "en", "English"),
                "meta": {
                    "id": "wiktionary-en",
                    "name": "English Wiktionary",
                    "attribution": "Wiktionary via kaikki.org, CC BY-SA 4.0",
                    "languages": "en",
                    "definitions.en": "en",
                    "kind.en": "definition",
                    "priority": "10",
                    "default": "off",
                },
            },
        ],
        "summary": "English words from Wiktionary for Dictionary",
        "licence": f"Wiktionary contributors, extracted by kaikki.org (wiktextract).\n{CC_BY_SA_40}\nConverted to the Bande pack format.\n",
    },
    "bande-dict-en": {
        "packs": [
            {
                "file": "oewn-dictionary",
                "entries": wordnet_dictionary,
                "meta": {
                    "id": "oewn-dictionary",
                    "name": "English Dictionary (Open English WordNet)",
                    "attribution": "Open English WordNet 2025, CC BY 4.0",
                    "languages": "en",
                    "definitions.en": "en",
                    "kind.en": "definition",
                    "priority": "20",
                    "rules.en": ENGLISH_RULES,
                },
            },
            {
                "file": "oewn-thesaurus",
                "entries": wordnet_thesaurus,
                "meta": {
                    "id": "oewn-thesaurus",
                    "name": "English Thesaurus (Open English WordNet)",
                    "attribution": "Open English WordNet 2025, CC BY 4.0",
                    "languages": "en",
                    "definitions.en": "en",
                    "kind.en": "thesaurus",
                    "priority": "20",
                    "rules.en": ENGLISH_RULES,
                },
            },
        ],
        "summary": "English dictionary and thesaurus for Dictionary (Open English WordNet)",
        "licence": f"Open English WordNet 2025, https://en-word.net/\n{CC_BY_40}\nConverted to the Bande pack format.\n",
    },
    "bande-dict-sv": {
        "packs": [
            {
                "file": "folkets",
                "entries": folkets_entries,
                "meta": {
                    "id": "folkets",
                    "name": "Folkets lexikon",
                    "attribution": "Folkets lexikon, KTH, CC BY-SA 2.5",
                    "languages": "sv en",
                    "definitions.sv": "sv",
                    "definitions.en": "sv",
                    "kind.sv": "translation",
                    "kind.en": "translation",
                    "priority": "10",
                    "compound.sv": "3",
                },
            },
            {
                "file": "synlex",
                "entries": synlex_entries,
                "meta": {
                    "id": "synlex",
                    "name": "Svenska synonymer (Synlex)",
                    "attribution": "Folkets synonymlexikon Synlex, Viggo Kann, KTH",
                    "languages": "sv",
                    "definitions.sv": "sv",
                    "kind.sv": "thesaurus",
                    "priority": "10",
                    "compound.sv": "3",
                },
            },
        ],
        "summary": "Swedish-English dictionary and Swedish thesaurus for Dictionary (Folkets lexikon, Synlex)",
        "licence": f"Folkets lexikon, KTH, https://folkets-lexikon.csc.kth.se/\n{CC_BY_SA_25}\n\nSynlex, Folkets synonymlexikon, 2009 Viggo Kann, KTH, as packaged in Debian's mythes-sv; its copyright file follows.\nConverted to the Bande pack format.\n\n",
        "licence_file": "mythes-sv/usr/share/doc/mythes-sv/copyright",
    },
    "bande-dict-sv-wiktionary": {
        "packs": [
            {
                "file": "wiktionary-sv",
                "entries": lambda sources: wiktionary_entries(sources, "sv", "Swedish"),
                "meta": {
                    "id": "wiktionary-sv",
                    "name": "Svenska ord (Wiktionary)",
                    "attribution": "Wiktionary via kaikki.org, CC BY-SA 4.0",
                    "languages": "sv",
                    "definitions.sv": "en",
                    "kind.sv": "definition",
                    "priority": "20",
                    "compound.sv": "3",
                },
            },
        ],
        "summary": "Swedish words from Wiktionary for Dictionary",
        "licence": f"Wiktionary contributors, extracted by kaikki.org (wiktextract).\n{CC_BY_SA_40}\nConverted to the Bande pack format.\n",
    },
}


def debian_arch_all_deb(package, spec, packs, version, out):
    root = out / f"{package}.root"
    shutil.rmtree(root, ignore_errors=True)
    folder = root / "usr/share/bande/dict"
    docs = root / f"usr/share/doc/{package}"
    folder.mkdir(parents=True)
    docs.mkdir(parents=True)
    (root / "DEBIAN").mkdir()
    for pack in packs:
        shutil.copy(pack, folder)
    (docs / "copyright").write_text(spec["licence"])
    (root / "DEBIAN/control").write_text(
        f"Package: {package}\nVersion: {version}\nArchitecture: all\n"
        f"Maintainer: Mikael Hug <mikaelhug@gmail.com>\nSection: text\nPriority: optional\n"
        f"Installed-Size: {sum(pack.stat().st_size for pack in packs) // 1024}\nDescription: {spec['summary']}\n"
    )
    deb = out / f"{package}_{version}_all.deb"
    subprocess.run(["dpkg-deb", "--root-owner-group", "-Zxz", "-b", str(root), str(deb)], check=True)
    shutil.rmtree(root)
    return deb


def main():
    if sys.argv[1] == "--pin":
        pin(pathlib.Path(sys.argv[2]))
        return
    version, sources, out = sys.argv[1], pathlib.Path(sys.argv[2]), pathlib.Path(sys.argv[3])
    sources.mkdir(parents=True, exist_ok=True)
    out.mkdir(parents=True, exist_ok=True)
    for package in sys.argv[4:] or PACKS:
        spec = PACKS[package]
        packs = []
        for part in spec["packs"]:
            pack = out / f"{part['file']}.bdict"
            write_pack(pack, part["meta"], part["entries"](sources))
            packs.append(pack)
        if "licence_file" in spec:
            spec["licence"] += (sources / spec["licence_file"]).read_text()
        deb = debian_arch_all_deb(package, spec, packs, version, out)
        size = sum(pack.stat().st_size for pack in packs) // 1000000
        print(package, size, "MB packs,", deb.stat().st_size // 1000000, "MB deb")

if __name__ == "__main__":
    main()
