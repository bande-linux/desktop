#!/usr/bin/env python3
"""scan-hosts.py <unpacked /usr/lib/bande-browser> [more prefs files]: lists every Google host named in
the browser's own code and prefs, and the file naming it. Most are labels and page-compatibility
data, so a person reads the list; the capture is what proves nothing connects."""
import os
import re
import sys
import zipfile

HOSTS = re.compile(rb"[a-z0-9.-]*(?:googleapis\.com|google\.com|gstatic\.com|gvt1\.com|googlevideo\.com|"
                   rb"doubleclick\.net|googlesyndication\.com|googletagmanager\.com|youtube\.com|ytimg\.com)")

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
    for name, hosts in sorted(found.items()):
        print(name + ": " + " ".join(sorted(hosts)[:6]))
    print(f"{len(found)} files name a Google host")


main()
