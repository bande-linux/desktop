#!/usr/bin/env python3
"""Drives a Firefox-family browser through Marionette and asks shot.py for screenshots."""
import json
import os
import socket
import subprocess
import tempfile
import time


class Browser:
    def __init__(self, binary, prefs=None, xpi=None, env=None):
        self.profile = tempfile.mkdtemp(prefix="browser-profile")
        lines = ['user_pref("marionette.port", 2828);']
        for name, value in (prefs or {}).items():
            lines.append(f"user_pref({json.dumps(name)}, {json.dumps(value)});")
        with open(os.path.join(self.profile, "user.js"), "w") as out:
            out.write("\n".join(lines) + "\n")
        self.log = open(os.path.join(self.profile, "browser.log"), "w")
        full_env = dict(os.environ, MOZ_ENABLE_WAYLAND="1", **(env or {}))
        self.process = subprocess.Popen(
            [binary, "--marionette", "-remote-allow-system-access", "--no-remote", "--profile", self.profile],
            env=full_env, stdout=self.log, stderr=self.log,
        )
        self.id = 0
        for _ in range(240):
            try:
                self.sock = socket.create_connection(("127.0.0.1", 2828), timeout=2)
                break
            except OSError:
                time.sleep(0.5)
        else:
            raise RuntimeError("marionette never came up")
        self.sock.settimeout(120)
        self.buf = b""
        self.read()
        self.send("WebDriver:NewSession", {"capabilities": {}})

    def read(self):
        while b":" not in self.buf:
            self.buf += self.sock.recv(65536)
        size, rest = self.buf.split(b":", 1)
        size = int(size)
        while len(rest) < size:
            rest += self.sock.recv(65536)
        self.buf = rest[size:]
        return json.loads(rest[:size])

    def send(self, name, params=None):
        self.id += 1
        body = json.dumps([0, self.id, name, params or {}]).encode()
        self.sock.sendall(str(len(body)).encode() + b":" + body)
        reply = self.read()
        if reply[2]:
            raise RuntimeError(f"{name}: {reply[2].get('message')}")
        return reply[3]

    def chrome(self):
        self.send("Marionette:SetContext", {"value": "chrome"})

    def content(self):
        self.send("Marionette:SetContext", {"value": "content"})

    def js(self, script, *args):
        return self.send("WebDriver:ExecuteAsyncScript" if "arguments[arguments.length" in script else "WebDriver:ExecuteScript",
                         {"script": script, "args": list(args), "sandbox": "system"})["value"]

    def go(self, url):
        self.content()
        self.send("WebDriver:Navigate", {"url": url})

    def quit(self):
        self.process.terminate()
        try:
            self.process.wait(10)
        except subprocess.TimeoutExpired:
            self.process.kill()


counter = [0]


def shot(path):
    counter[0] += 1
    base = os.path.join(os.environ["SHOT_DIR"], str(counter[0]))
    with open(base + ".tmp", "w") as out:
        out.write(path)
    os.rename(base + ".tmp", base + ".req")
    for _ in range(600):
        if os.path.exists(base + ".done"):
            result = open(base + ".done").read()
            if result != "ok":
                raise RuntimeError(result)
            return
        time.sleep(0.1)
    raise TimeoutError("no screenshot")
