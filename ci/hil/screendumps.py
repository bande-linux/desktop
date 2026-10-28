import base64
import pathlib
import re
import sys

console, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
text = console.read_bytes().decode("utf-8", "replace")
out.mkdir(parents=True, exist_ok=True)
for name, payload in re.findall(r"UNBOUND-PNG-BEGIN (\S+)\r?\n([A-Za-z0-9+/=\r\n]+?)\r?\nUNBOUND-PNG-END", text):
    (out / name).write_bytes(base64.b64decode("".join(payload.split())))
    print(out / name)
status = re.search(r"UNBOUND-SMOKE-STATUS (\d+)", text)
sys.exit(int(status.group(1)) if status else 1)
