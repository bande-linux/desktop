#!/usr/bin/env python3
"""gut.py <source dir> <dir>...: removes a Firefox component's code but keeps its file names, so the
build and the many callers still resolve. Each module becomes a stub exporting the same names, doing
nothing; pages, scripts, styles and images become empty. Tests and build files stay."""
import os
import re
import sys

STUB_HEAD = """/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */

// Removed in Bande's Browser.
const nothing = new Proxy(function () {}, {
  get(target, key) {
    if (key === "then" || typeof key === "symbol") {
      return undefined;
    }
    if (/^(is|has|can|should|enabled|initialized|available)/.test(key)) {
      return () => false;
    }
    return nothing;
  },
  apply() {
    return Promise.resolve();
  },
  construct() {
    return {};
  },
});
"""
EXPORT = re.compile(r"^export\s+(?:async\s+)?(const|let|var|class|function\*?)\s+([A-Za-z_$][\w$]*)", re.M)
EXPORT_LIST = re.compile(r"^export\s*\{([^}]*)\}", re.M)
BLANK_SVG = '<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"/>\n'


def stub(text):
    names = {name for _, name in EXPORT.findall(text)}
    for group in EXPORT_LIST.findall(text):
        for item in group.split(","):
            item = item.strip()
            if item:
                names.add(item.split(" as ")[-1].strip())
    out = [STUB_HEAD]
    for name in sorted(names):
        if name.endswith("Child") or name.endswith("Parent"):
            base = "JSWindowActorChild" if name.endswith("Child") else "JSWindowActorParent"
            out.append(f"export class {name} extends {base} {{}}\n")
        else:
            out.append(f"export const {name} = nothing;\n")
    if "export default" in text:
        out.append("export default nothing;\n")
    return "".join(out)


def gut(path):
    name = os.path.basename(path)
    if name.endswith(".mjs"):
        with open(path, encoding="utf-8", errors="replace") as source:
            text = source.read()
        with open(path, "w", encoding="utf-8") as out:
            out.write(stub(text))
    elif name.endswith((".js", ".jsx", ".css", ".scss")):
        open(path, "w").close()
    elif name.endswith(".svg"):
        with open(path, "w") as out:
            out.write(BLANK_SVG)
    elif name.endswith((".png", ".webp", ".avif", ".jpg", ".gif", ".webm", ".mp4", ".wasm", ".onnx")):
        open(path, "wb").close()


def main():
    root = sys.argv[1]
    count = 0
    for folder in sys.argv[2:]:
        for base, dirs, files in os.walk(os.path.join(root, folder)):
            dirs[:] = [d for d in dirs if d not in ("test", "tests", "docs", "node_modules")]
            for name in files:
                gut(os.path.join(base, name))
                count += 1
    print(f"gut: {count} files emptied")


main()
