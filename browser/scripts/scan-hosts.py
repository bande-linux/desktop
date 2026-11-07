#!/usr/bin/env python3
"""scan-hosts.py <unpacked /usr/lib/bande-browser> [more prefs files]: lists Google hosts left in the
browser's own code and prefs, and which file holds each. Exit 1 if any is outside the allowed files:
page-compatibility data, block lists, and the search engine the user may pick."""
import os
import re
import sys
import zipfile

HOSTS = re.compile(rb"[a-z0-9.-]*(?:googleapis\.com|google\.com|gstatic\.com|gvt1\.com|googlevideo\.com|"
                   rb"doubleclick\.net|googlesyndication\.com|googletagmanager\.com|youtube\.com|ytimg\.com)")
ALLOWED = re.compile(r"webcompat|InteractionsBlocklist|search-config|safebrowsing-ui|pdfjs|devtools|locale|"
                     r"localization|\.ftl|about|licen[cs]e|credits|\.dtd|\.properties|StripOnShare|gmp-sources|"
                     r"ONNXPipeline|translations-document|videocontrols")


def scan(name, data, found):
    for host in set(HOSTS.findall(data)):
        found.setdefault(name, set()).add(host.decode())


def main():
    root = sys.argv[1]
    found = {}
    for jar in ("omni.ja", "browser/omni.ja"):
        with zipfile.ZipFile(os.path.join(root, jar)) as archive:
            for item in archive.namelist():
                scan(f"{jar}!{item}", archive.read(item), found)
    for extra in sys.argv[2:]:
        with open(extra, "rb") as source:
            scan(extra, source.read(), found)
    bad = 0
    for name, hosts in sorted(found.items()):
        allowed = bool(ALLOWED.search(name))
        bad += not allowed
        print(("allowed " if allowed else "LEFT    ") + name + ": " + " ".join(sorted(hosts)[:6]))
    print(f"{bad} files need a look")
    sys.exit(1 if bad else 0)


main()
