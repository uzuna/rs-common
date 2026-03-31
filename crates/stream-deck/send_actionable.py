#!/usr/bin/env python3
"""Stream Deck notification sender for UNIX socket PoC.

Usage:
  python crates/stream-deck/send_actionable.py \
    --summary "Data Analysis Finished" \
    --body "Click to open the result" \
    --payload "vscode://file/tmp/result.csv"
"""

from __future__ import annotations

import argparse
import json
import socket

DEFAULT_SOCKET = "/tmp/rs-common-stream-deck-notify.sock"


def main() -> int:
    parser = argparse.ArgumentParser(description="Send actionable notification to stream-deck")
    parser.add_argument("--socket", default=DEFAULT_SOCKET, help="UNIX socket path")
    parser.add_argument("--summary", default="Data Analysis Finished", help="notification summary")
    parser.add_argument("--body", default="Click to open the result", help="notification body")
    parser.add_argument(
        "--payload",
        default="vscode://file/tmp/result.csv",
        help="action payload (URL or local file path)",
    )
    args = parser.parse_args()

    packet = {
        "summary": args.summary,
        "body": args.body,
        "action_payload": args.payload,
    }
    raw = json.dumps(packet, ensure_ascii=True).encode("utf-8")

    with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as sock:
        sock.sendto(raw, args.socket)

    print("sent:", packet)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
