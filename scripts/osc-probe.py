#!/usr/bin/env python3
"""Small dependency-free OSC float probe for Control Room acceptance sessions."""

from __future__ import annotations

import argparse
import json
import socket
import struct
import sys
import time
import urllib.request


def endpoint(value: str) -> tuple[str, int]:
    host, separator, port = value.rpartition(":")
    if not separator or not host:
        raise argparse.ArgumentTypeError("expected HOST:PORT")
    try:
        number = int(port)
    except ValueError as error:
        raise argparse.ArgumentTypeError("port must be an integer") from error
    if not 1 <= number <= 65535:
        raise argparse.ArgumentTypeError("port must be in 1..=65535")
    return host, number


def padded_string(value: str) -> bytes:
    encoded = value.encode("utf-8") + b"\0"
    return encoded + b"\0" * ((-len(encoded)) % 4)


def encode_float(address: str, value: float) -> bytes:
    if not address.startswith("/") or len(address) < 2:
        raise ValueError("OSC address must start with / and contain a name")
    return padded_string(address) + padded_string(",f") + struct.pack(">f", value)


def read_string(packet: bytes, offset: int) -> tuple[str, int]:
    end = packet.find(b"\0", offset)
    if end < 0:
        raise ValueError("unterminated OSC string")
    value = packet[offset:end].decode("utf-8")
    next_offset = (end + 4) & ~3
    if next_offset > len(packet):
        raise ValueError("truncated OSC string padding")
    return value, next_offset


def decode_float(packet: bytes) -> tuple[str, float]:
    address, offset = read_string(packet, 0)
    tags, offset = read_string(packet, offset)
    if tags != ",f":
        raise ValueError(f"expected one float argument, got {tags!r}")
    if len(packet) != offset + 4:
        raise ValueError("expected exactly one OSC float argument")
    return address, struct.unpack(">f", packet[offset:])[0]


def send(args: argparse.Namespace) -> int:
    packet = encode_float(args.address, args.value)
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.sendto(packet, args.target)
    print(json.dumps({"event": "sent", "address": args.address, "value": args.value}))
    return 0


def listen(args: argparse.Namespace) -> int:
    deadline = time.monotonic() + args.timeout
    received = 0
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind(args.bind)
        while received < args.count:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                print(
                    json.dumps(
                        {"event": "timeout", "received": received, "expected": args.count}
                    ),
                    file=sys.stderr,
                )
                return 1
            sock.settimeout(remaining)
            try:
                packet, peer = sock.recvfrom(65535)
            except TimeoutError:
                continue
            try:
                address, value = decode_float(packet)
            except (UnicodeDecodeError, ValueError) as error:
                print(json.dumps({"event": "ignored", "error": str(error)}), file=sys.stderr)
                continue
            if address != args.address:
                continue
            received += 1
            print(
                json.dumps(
                    {
                        "event": "received",
                        "address": address,
                        "value": value,
                        "peer": f"{peer[0]}:{peer[1]}",
                        "index": received,
                    }
                ),
                flush=True,
            )
    return 0


def self_test(_args: argparse.Namespace) -> int:
    for address, value in [("/control/cutoff", 0.0), ("/control/lfo", 0.75), ("/x", -1.5)]:
        decoded_address, decoded_value = decode_float(encode_float(address, value))
        if decoded_address != address or abs(decoded_value - value) > 1e-6:
            raise AssertionError((address, value, decoded_address, decoded_value))
    print("OSC probe codec self-test passed")
    return 0


def check_state(args: argparse.Namespace) -> int:
    deadline = time.monotonic() + args.timeout
    last_value: float | None = None
    while time.monotonic() < deadline:
        try:
            with urllib.request.urlopen(args.url, timeout=1.0) as response:
                snapshot = json.load(response)
            meter = next(
                (candidate for candidate in snapshot.get("meters", []) if candidate["id"] == args.node),
                None,
            )
            if meter is not None:
                last_value = float(meter["value"])
                if abs(last_value - args.expected) <= args.tolerance:
                    print(
                        json.dumps(
                            {
                                "event": "state_match",
                                "node": args.node,
                                "value": last_value,
                                "revision": snapshot.get("revision"),
                            }
                        )
                    )
                    return 0
        except (OSError, ValueError, KeyError, StopIteration):
            pass
        time.sleep(0.05)
    print(
        json.dumps(
            {
                "event": "state_mismatch",
                "node": args.node,
                "expected": args.expected,
                "last_value": last_value,
            }
        ),
        file=sys.stderr,
    )
    return 1


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    commands = root.add_subparsers(dest="command", required=True)

    sender = commands.add_parser("send", help="send one OSC float")
    sender.add_argument("--target", type=endpoint, required=True)
    sender.add_argument("--address", required=True)
    sender.add_argument("--value", type=float, required=True)
    sender.set_defaults(run=send)

    listener = commands.add_parser("listen", help="listen for OSC floats")
    listener.add_argument("--bind", type=endpoint, required=True)
    listener.add_argument("--address", required=True)
    listener.add_argument("--count", type=int, default=1)
    listener.add_argument("--timeout", type=float, default=10.0)
    listener.set_defaults(run=listen)

    check = commands.add_parser("self-test", help="verify the probe codec")
    check.set_defaults(run=self_test)

    state = commands.add_parser("check-state", help="wait for a graph meter value over HTTP")
    state.add_argument("--url", required=True)
    state.add_argument("--node", required=True)
    state.add_argument("--expected", type=float, required=True)
    state.add_argument("--tolerance", type=float, default=0.01)
    state.add_argument("--timeout", type=float, default=5.0)
    state.set_defaults(run=check_state)
    return root


def main() -> int:
    args = parser().parse_args()
    if getattr(args, "count", 1) < 1:
        parser().error("--count must be positive")
    if getattr(args, "timeout", 1.0) <= 0:
        parser().error("--timeout must be positive")
    return args.run(args)


if __name__ == "__main__":
    raise SystemExit(main())
