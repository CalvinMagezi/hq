#!/usr/bin/env python3
"""Small network helpers for the tunnel harness, run inside the test container.

  helper.py echo ADDR PORT                    a threaded TCP echo server (stays up until killed)
  helper.py probe ADDR PORT [SECONDS]         one connection, one line out, the same line back;
                                              exit 0 on an echo, 1 otherwise; prints the milliseconds
  helper.py silent ADDR PORT [SECONDS]        exit 0 if the connection gets no answer at all (neither
                                              an accept nor a reset) within SECONDS: a packet dropped
                                              at the door, not a refusal
  helper.py stream ADDR PORT SECONDS          one connection held open, a line a second; prints a
                                              JSON summary (lines, errors, worst_ms, first_error_at)
  helper.py flood ADDR PORT SECONDS           random UDP datagrams as fast as possible
  helper.py spoof ADDR PORT SRC SRCPORT SECS  the same, with the given source address and port
                                              (a raw socket; needs root)
"""
import struct
import json
import os
import socket
import sys
import threading
import time

DEFAULT_TIMEOUT = 3.0
STREAM_TIMEOUT = 3.0
FLOOD_SIZES = (32, 92, 148, 1200)


def echo(addr, port):
    server = socket.socket()
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    server.bind((addr, port))
    server.listen(64)

    def serve(conn):
        with conn:
            while True:
                data = conn.recv(65536)
                if not data:
                    return
                conn.sendall(data)

    while True:
        conn, _ = server.accept()
        threading.Thread(target=serve, args=(conn,), daemon=True).start()


def probe(addr, port, seconds=DEFAULT_TIMEOUT):
    line = f"probe-{os.getpid()}-{time.time_ns()}\n".encode()
    start = time.monotonic()
    try:
        with socket.create_connection((addr, port), timeout=seconds) as conn:
            conn.settimeout(seconds)
            conn.sendall(line)
            got = b""
            while len(got) < len(line):
                chunk = conn.recv(len(line) - len(got))
                if not chunk:
                    break
                got += chunk
    except OSError:
        return 1
    if got != line:
        return 1
    print(int((time.monotonic() - start) * 1000))
    return 0


def silent(addr, port, seconds=DEFAULT_TIMEOUT):
    try:
        socket.create_connection((addr, port), timeout=seconds).close()
    except socket.timeout:
        return 0
    except OSError:
        return 1
    return 1


def stream(addr, port, seconds):
    summary = {"lines": 0, "errors": 0, "worst_ms": 0, "first_error_at": None}
    started = time.monotonic()
    deadline = started + seconds
    conn = None
    n = 0
    while time.monotonic() < deadline:
        tick = time.monotonic()
        try:
            if conn is None:
                conn = socket.create_connection((addr, port), timeout=STREAM_TIMEOUT)
                conn.settimeout(STREAM_TIMEOUT)
            line = f"line-{n}\n".encode()
            n += 1
            conn.sendall(line)
            got = b""
            while len(got) < len(line):
                chunk = conn.recv(len(line) - len(got))
                if not chunk:
                    raise ConnectionResetError("closed")
                got += chunk
            if got != line:
                raise ValueError("wrong bytes")
            summary["lines"] += 1
        except OSError as e:
            summary["errors"] += 1
            if summary["first_error_at"] is None:
                summary["first_error_at"] = int(time.monotonic() - started)
            summary.setdefault("kinds", {})
            summary["kinds"][type(e).__name__] = summary["kinds"].get(type(e).__name__, 0) + 1
            if conn is not None:
                conn.close()
            conn = None
        except ValueError:
            summary["errors"] += 1
            conn = None
        summary["worst_ms"] = max(summary["worst_ms"], int((time.monotonic() - tick) * 1000))
        time.sleep(max(0.0, 1.0 - (time.monotonic() - tick)))
    print(json.dumps(summary))
    return 0


def flood(addr, port, seconds):
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    deadline = time.monotonic() + seconds
    sent = 0
    sizes = len(FLOOD_SIZES)
    while time.monotonic() < deadline:
        for _ in range(500):
            body = os.urandom(FLOOD_SIZES[sent % sizes])
            # Looks like a handshake or a data packet to a parser that only reads the first byte.
            body = bytes([1 + sent % 4]) + b"\0\0\0" + body[4:]
            try:
                sock.sendto(body, (addr, port))
                sent += 1
            except OSError:
                pass
    print(sent)
    return 0


def ip_checksum(data):
    total = sum(struct.unpack(f"!{len(data) // 2}H", data))
    while total >> 16:
        total = (total & 0xFFFF) + (total >> 16)
    return ~total & 0xFFFF


def spoof(addr, port, src, src_port, seconds):
    sock = socket.socket(socket.AF_INET, socket.SOCK_RAW, socket.IPPROTO_RAW)
    sock.setsockopt(socket.IPPROTO_IP, socket.IP_HDRINCL, 1)
    src_raw, dst_raw = socket.inet_aton(src), socket.inet_aton(addr)
    deadline = time.monotonic() + seconds
    sent = 0
    while time.monotonic() < deadline:
        for _ in range(200):
            size = FLOOD_SIZES[sent % len(FLOOD_SIZES)]
            body = bytes([1 + sent % 4]) + b"\0\0\0" + os.urandom(size - 4)
            udp = struct.pack("!HHHH", src_port, port, 8 + len(body), 0) + body
            header = struct.pack("!BBHHHBBH4s4s", 0x45, 0, 20 + len(udp), sent & 0xFFFF, 0, 64, 17, 0, src_raw, dst_raw)
            header = header[:10] + struct.pack("!H", ip_checksum(header)) + header[12:]
            try:
                sock.sendto(header + udp, (addr, 0))
                sent += 1
            except OSError:
                pass
    print(sent)
    return 0


def main(argv):
    command, args = argv[1], argv[2:]
    if command == "echo":
        echo(args[0], int(args[1]))
    if command == "probe":
        return probe(args[0], int(args[1]), float(args[2]) if len(args) > 2 else DEFAULT_TIMEOUT)
    if command == "silent":
        return silent(args[0], int(args[1]), float(args[2]) if len(args) > 2 else DEFAULT_TIMEOUT)
    if command == "stream":
        return stream(args[0], int(args[1]), float(args[2]))
    if command == "flood":
        return flood(args[0], int(args[1]), float(args[2]))
    if command == "spoof":
        return spoof(args[0], int(args[1]), args[2], int(args[3]), float(args[4]))
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
