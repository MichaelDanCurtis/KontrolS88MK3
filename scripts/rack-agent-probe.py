#!/usr/bin/env python3
"""Exercise the localhost-only Rack Copilot bridge without an AI provider."""

import json
import socket
import sys


def request(payload: dict) -> dict:
    encoded = json.dumps(payload, separators=(",", ":")).encode() + b"\n"
    with socket.create_connection(("127.0.0.1", 9102), timeout=10) as peer:
        peer.sendall(encoded)
        response = bytearray()
        while b"\n" not in response:
            chunk = peer.recv(65536)
            if not chunk:
                break
            response.extend(chunk)
            if len(response) > 4 * 1024 * 1024:
                raise RuntimeError("Rack agent response exceeded 4 MiB")
    decoded = json.loads(response.partition(b"\n")[0])
    if not decoded.get("ok"):
        raise RuntimeError(decoded.get("error", "unknown Rack agent error"))
    return decoded


def smoke() -> None:
    before = request({"op": "snapshot"})["snapshot"]
    assert before["catalog"], "Rack catalog is empty"
    assert any(model["plugin"] == "Core" and model["model"] == "Notes" for model in before["catalog"])
    applied = request(
        {
            "op": "apply",
            "base_fingerprint": before["fingerprint"],
            "commands": [
                {
                    "type": "add_module",
                    "reference": "ai:smoke-notes",
                    "plugin": "Core",
                    "model": "Notes",
                    "x": 20,
                    "y": 0,
                }
            ],
        }
    )
    assert applied["fingerprint"] != before["fingerprint"]
    changed = request({"op": "snapshot"})["snapshot"]
    assert any(module["plugin"] == "Core" and module["model"] == "Notes" for module in changed["patch"]["modules"])
    undone = request({"op": "undo"})
    assert undone["fingerprint"] == before["fingerprint"]
    restored = request({"op": "snapshot"})["snapshot"]
    assert not any(module["plugin"] == "Core" and module["model"] == "Notes" for module in restored["patch"]["modules"])
    print(f"Rack agent verified {len(before['catalog'])} installed models with Apply + Undo")


if __name__ == "__main__":
    if sys.argv[1:] != ["smoke"]:
        raise SystemExit("usage: rack-agent-probe.py smoke")
    smoke()
