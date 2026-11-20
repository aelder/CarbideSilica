#!/usr/bin/env python3
"""Interactive fake-daemon reorg demo. Run from any directory after cargo build."""
import argparse
from collections import deque
import http.server
import json
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import threading

from reorg_terminal import A2, B1, B2, B3, Daemon, GENESIS, TXID, VRSC

CODE = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve()


def prepare(root, port):
    conf = root / "VRSC.conf"
    conf.write_text(f"rpcconnect=127.0.0.1\nrpcport={port}\nrpcuser=demo\nrpcpassword=demo\n")
    data = root / "data/verus-explorer-tui"
    data.mkdir(parents=True)
    entries = [
        ("transaction", TXID, "DEMO: same transaction on both branches — F7 inclusions"),
        ("block", GENESIS["hash"], "DEMO: fork point at height 0 — F7 child branches"),
        ("block", A2["hash"], "DEMO: branch A tip — becomes orphaned after b"),
        ("block", B2["hash"], "DEMO: branch B height 2"),
    ]
    (data / "bookmarks-mainnet.jsonl").write_text("".join(
        json.dumps(dict(version=1, system_id=VRSC, kind=kind, target=target,
                        description=description, created_at=1700000000)) + "\n"
        for kind, target, description in entries
    ))
    (root / "reorg-demo.json").write_text(json.dumps(dict(port=port, fixture="reorg-demo-v1")))


def open_explorer(root):
    root = root.resolve()
    marker = json.loads((root / "reorg-demo.json").read_text())
    if marker.get("fixture") != "reorg-demo-v1":
        raise ValueError("This directory is not a reorg demo session")
    binary = CODE / "target/debug/vcarbide"
    if not binary.is_file():
        raise FileNotFoundError(f"Build first: cd {shlex.quote(str(CODE))} && cargo build")
    env = os.environ.copy()
    # Isolate daemon discovery, app settings, bookmarks and network cache.
    env.update(HOME=str(root), XDG_DATA_HOME=str(root / "data"),
               XDG_CONFIG_HOME=str(root / "config"), XDG_CACHE_HOME=str(root / "cache"))
    return subprocess.call([str(binary), "--conf", str(root / "VRSC.conf"),
                            "--poll-interval", "1", "--emoji", "off", "--open", TXID],
                           env=env, cwd=CODE)


def control(server, command):
    if command == "a":
        server.missing, server.flip_after_tip, server.tip = None, None, A2["hash"]
        return "Branch A active at height 2. Branch B becomes orphaned once reconciled."
    if command == "b":
        server.missing, server.flip_after_tip, server.tip = None, None, B2["hash"]
        return "Branch B active at height 2. Branch A becomes orphaned once reconciled."
    if command == "grow":
        server.missing, server.flip_after_tip, server.tip = None, None, B3["hash"]
        return "Branch B advanced to height 3."
    if command == "incomplete":
        server.missing, server.flip_after_tip, server.tip = B1["hash"], None, B2["hash"]
        return "Branch B advertised with a missing header. Use this before B is cached; recover restores it."
    if command == "recover":
        server.missing = None
        return "Missing header restored; reconciliation can finish."
    if command == "status":
        return f"Tip: {server.tip}\nMissing header: {server.missing or 'none'}"
    return "Commands: a | b | grow | incomplete | recover | status | quit"


def serve():
    with tempfile.TemporaryDirectory(prefix="verus-reorg-demo-") as directory:
        root = Path(directory)
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Daemon)
        server.tip, server.missing, server.flip_after_tip = A2["hash"], None, None
        server.calls = deque(maxlen=1000)
        prepare(root, server.server_port)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            launch = shlex.join([sys.executable, str(SCRIPT), "--open", str(root)])
            print("Synthetic reorg demo; no real daemon or normal explorer cache is used.")
            print(f"\nIn a second terminal, run:\n\n{launch}\n")
            print("Start on branch A: press F7 on the transaction, then enter b here.")
            print("Use F6 bookmarks to open the fork point or either branch tip.")
            print("Use a/b to switch branches, grow to extend B, and q in the TUI to exit.")
            print("Re-run the second-terminal command to test restart persistence.")
            print("For incomplete reconciliation, use a fresh demo: incomplete, then recover.")
            print("Close the TUI before quit here; quit removes the temporary demo files.")
            print(control(server, "help"))
            while True:
                try:
                    command = input("reorg-demo> ").strip().lower()
                except (EOFError, KeyboardInterrupt):
                    print()
                    break
                if command in ("quit", "q", "exit"):
                    break
                print(control(server, command))
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--open", type=Path, metavar="DEMO_DIRECTORY",
                        help="open the real TUI against an already-running demo")
    args = parser.parse_args()
    if args.open:
        return open_explorer(args.open)
    serve()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
