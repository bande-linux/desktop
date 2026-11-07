#!/usr/bin/env python3
"""shot.py <dir>: serves screenshots over ext-image-copy-capture on the WAYLAND_SOCKET the compositor
gives its review client. A file <dir>/<n>.req holding a path asks for a PNG there; <n>.done answers."""
import mmap
import os
import select
import socket
import struct
import sys
import time
import zlib

XRGB8888 = 1


class Wayland:
    def __init__(self, fd):
        self.sock = socket.socket(fileno=fd)
        self.buf = b""
        self.next_id = 2
        self.fds = []

    def new_id(self):
        self.next_id += 1
        return self.next_id

    def send(self, obj, op, args=b"", fds=()):
        msg = struct.pack("=IHH", obj, op, 8 + len(args)) + args
        socket.send_fds(self.sock, [msg], list(fds))

    def event(self, timeout=30):
        while len(self.buf) < 8 or len(self.buf) < struct.unpack("=IHH", self.buf[:8])[2]:
            ready, _, _ = select.select([self.sock], [], [], timeout)
            if not ready:
                raise TimeoutError("no event from the compositor")
            data = self.sock.recv(65536)
            if not data:
                raise EOFError("compositor went away")
            self.buf += data
        obj, op, size = struct.unpack("=IHH", self.buf[:8])
        args, self.buf = self.buf[8:size], self.buf[size:]
        return obj, op, args


def wl_string(text):
    raw = text.encode() + b"\0"
    return struct.pack("=I", len(raw)) + raw + b"\0" * (-len(raw) % 4)


def read_string(args, at):
    size = struct.unpack_from("=I", args, at)[0]
    return args[at + 4 : at + 4 + size - 1].decode(), at + 4 + ((size + 3) & ~3)


def png(path, width, height, stride, data):
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        row = data[y * stride : y * stride + width * 4]
        rgb = bytearray(width * 3)
        rgb[0::3] = row[2::4]
        rgb[1::3] = row[1::4]
        rgb[2::3] = row[0::4]
        rows += rgb

    def chunk(tag, body):
        return struct.pack(">I", len(body)) + tag + body + struct.pack(">I", zlib.crc32(tag + body))

    with open(path, "wb") as out:
        out.write(b"\x89PNG\r\n\x1a\n")
        out.write(chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)))
        out.write(chunk(b"IDAT", zlib.compress(bytes(rows), 3)))
        out.write(chunk(b"IEND", b""))


def main():
    wl = Wayland(int(os.environ["WAYLAND_SOCKET"]))
    registry = 2
    wl.send(1, 1, struct.pack("=I", registry))
    sync = wl.new_id()
    wl.send(1, 0, struct.pack("=I", sync))
    globals_ = {}
    while True:
        obj, op, args = wl.event()
        if obj == registry and op == 0:
            name = struct.unpack_from("=I", args)[0]
            interface, at = read_string(args, 4)
            globals_[interface] = (name, struct.unpack_from("=I", args, at)[0])
        elif obj == sync:
            break

    def bind(interface, version):
        name = globals_[interface][0]
        new = wl.new_id()
        wl.send(registry, 0, struct.pack("=I", name) + wl_string(interface) + struct.pack("=II", version, new))
        return new

    shm = bind("wl_shm", 1)
    output = bind("wl_output", 4)
    sources = bind("ext_output_image_capture_source_manager_v1", 1)
    manager = bind("ext_image_copy_capture_manager_v1", 1)

    def capture(path):
        source = wl.new_id()
        wl.send(sources, 0, struct.pack("=II", source, output))
        session = wl.new_id()
        wl.send(manager, 0, struct.pack("=III", session, source, 0))
        width = height = 0
        while True:
            obj, op, args = wl.event()
            if obj == session and op == 0:
                width, height = struct.unpack("=II", args)
            elif obj == session and op == 4:
                break
        stride = width * 4
        size = stride * height
        fd = os.memfd_create("shot")
        os.ftruncate(fd, size)
        pool = wl.new_id()
        wl.send(shm, 0, struct.pack("=Ii", pool, size), fds=[fd])
        buffer = wl.new_id()
        wl.send(pool, 0, struct.pack("=IiiiiI", buffer, 0, width, height, stride, XRGB8888))
        frame = wl.new_id()
        wl.send(session, 0, struct.pack("=I", frame))
        wl.send(frame, 1, struct.pack("=I", buffer))
        wl.send(frame, 2, struct.pack("=iiii", 0, 0, 2147483647, 2147483647))
        wl.send(frame, 3)
        while True:
            obj, op, args = wl.event()
            if obj == frame and op == 3:
                break
            if obj == frame and op == 4:
                raise RuntimeError(f"capture failed: {args!r}")
        with mmap.mmap(fd, size) as mapped:
            png(path, width, height, stride, bytes(mapped))
        os.close(fd)
        wl.send(frame, 0)
        wl.send(session, 1)
        wl.send(pool, 1)

    requests = sys.argv[1]
    while True:
        for name in os.listdir(requests):
            if name.endswith(".req"):
                full = os.path.join(requests, name)
                path = open(full).read().strip()
                os.remove(full)
                try:
                    capture(path)
                    result = "ok"
                except (OSError, RuntimeError, TimeoutError, EOFError) as error:
                    result = str(error)
                with open(full[:-4] + ".done", "w") as done:
                    done.write(result)
        ready, _, _ = select.select([wl.sock], [], [], 0.1)
        if ready:
            if not wl.sock.recv(65536):
                return
        time.sleep(0.1)


main()
