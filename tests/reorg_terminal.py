"""Reconciliation integration with a controllable local daemon and real SQLite/PTY."""
import http.server
import json
from pathlib import Path
import sqlite3
import tempfile
import threading
import time
from bookmarks_terminal import Terminal, VRSC

def block(number, height, parent=None):
    return dict(hash="a"+f"{number:063x}", height=height, previousblockhash=parent,
                time=1700000000+height*60, tx=[], size=10, confirmations=1)

GENESIS = block(1, 0)
A1 = block(2, 1, GENESIS["hash"])
A2 = block(3, 2, A1["hash"])
B1 = block(4, 1, GENESIS["hash"])
B2 = block(5, 2, B1["hash"])
B3 = block(6, 3, B2["hash"])
TXID = "c" * 64
A2["tx"] = [TXID]
B2["tx"] = [TXID]
HEADERS = {b["hash"]: b for b in [GENESIS, A1, A2, B1, B2, B3]}

class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        method, args = request["method"], request["params"]
        self.server.calls.append(method)
        tip = HEADERS[self.server.tip]
        chain = {}
        cursor = tip
        while cursor:
            chain[cursor["height"]] = cursor["hash"]
            cursor = HEADERS.get(cursor["previousblockhash"])
        value = None
        if method == "getinfo":
            value = dict(chainid=VRSC, testnet=False, blocks=tip["height"], longestchain=tip["height"], connections=1)
        elif method == "getblockchaininfo":
            value = dict(blocks=tip["height"], bestblockhash=tip["hash"])
            if self.server.flip_after_tip:
                self.server.tip = self.server.flip_after_tip
                self.server.flip_after_tip = None
        elif method == "getblockhash":
            value = chain.get(int(args[0]))
        elif method in ("getblock", "getblockheader"):
            key = args[0] if len(str(args[0])) == 64 else chain.get(int(args[0]))
            if key in HEADERS and key != self.server.missing:
                value = dict(HEADERS[key])
                value["confirmations"] = tip["height"]-value["height"]+1 if chain.get(value["height"]) == key else -1
        elif method == "getrawtransaction" and args[0] == TXID:
            inclusion = A2 if tip["hash"] == A2["hash"] else B2
            value = dict(txid=TXID, vin=[dict(coinbase="00")], vout=[], size=10,
                         blockhash=inclusion["hash"], height=2, blocktime=inclusion["time"],
                         confirmations=tip["height"]-1)
        elif method == "getcurrency":
            value = dict(currencyid=args[0], name="VRSC", systemid=VRSC)
        body = json.dumps(dict(result=value,error=None if value is not None else dict(code=-5,message="unavailable"),id=request["id"])).encode()
        self.send_response(200)
        self.send_header("Content-Length",str(len(body)))
        self.end_headers()
        self.wfile.write(body)

def wait(terminal, predicate):
    deadline = time.monotonic()+15
    while time.monotonic() < deadline:
        terminal.read(0.1)
        if predicate():
            return
    raise AssertionError(terminal.screen())

def displayed_membership(terminal, block_hash):
    lines = terminal.screen().splitlines()
    for index, line in enumerate(lines):
        if block_hash in line and index:
            return lines[index-1]
    return ""


def run():
    server = http.server.ThreadingHTTPServer(("127.0.0.1",0),Daemon)
    server.tip, server.missing, server.flip_after_tip = A2["hash"], None, None
    server.calls = []
    threading.Thread(target=server.serve_forever,daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix="explorer-reorg-") as directory:
            root = Path(directory)
            conf = root/"VRSC.conf"
            conf.write_text(f"rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n")
            path = root/"cache/verus-carbide/mainnet.sqlite3"
            def state():
                if not path.exists(): return {}
                with sqlite3.connect(path) as db:
                    rows = db.execute("select state_json from reconciled_chain").fetchall()
                    return json.loads(rows[0][0]) if rows else {}
            def tip_is(h):
                s=state()
                return s.get("canonical",{}).get(str(h["height"]),{}).get("hash") == h["hash"]
            terminal=Terminal(root,conf,poll_interval=1)
            try:
                wait(terminal,lambda:tip_is(A2))
                terminal.key(b"/"+A2["hash"].encode()+b"\r",0.7)
                # A missing new-branch header must leave the committed old branch intact.
                server.missing, server.tip = B1["hash"], B2["hash"]
                wait(terminal,lambda:"Reconciliation incomplete" in terminal.screen())
                assert tip_is(A2)
                terminal.key(b"\x1bOQ",0.3)  # F2
                wait(terminal,lambda:"Needs attention" in terminal.screen())
                server.missing=None
                wait(terminal,lambda:tip_is(B2))
                wait(terminal,lambda:any(
                    "ago" in line and "Reconciled" in line and "2 blocks detached, 2 adopted" in line
                    for line in terminal.screen().splitlines()
                ))
                assert "Needs attention" not in terminal.screen(),terminal.screen()
                terminal.key(b"a\r",0.4)
                wait(terminal,lambda:all(
                    text in terminal.screen()
                    for text in ("Source activity", "Observed", A2["hash"], B2["hash"])
                ))
                terminal.key(b"\x1b",0.2)  # Details → history
                terminal.key(b"\x1b",0.2)  # History → sources
                terminal.key(b"\x1b",0.2)  # Sources → block
                wait(terminal,lambda:"Orphaned" in terminal.screen())
                assert A2["hash"] in terminal.screen(),terminal.screen()
                terminal.key(b"p",0.6)
                assert A1["hash"] in terminal.screen(),terminal.screen()
                terminal.key(b"a",0.6)
                assert B1["hash"] in terminal.screen(),terminal.screen()
                # The same transaction has competing inclusions; F7 keeps their statuses separate.
                terminal.key(b"/"+TXID.encode()+b"\r",0.8)
                terminal.key(b"\x1b[18~",0.6)  # F7
                wait(terminal,lambda:"Transaction inclusion history" in terminal.screen() and A2["hash"] in terminal.screen() and B2["hash"] in terminal.screen())
                assert "Last observed active" in terminal.screen() and "Last observed orphaned" in terminal.screen(),terminal.screen()
                terminal.key(b"\r",0.6)
                assert A2["hash"] in terminal.screen() and "Orphaned" in terminal.screen(),terminal.screen()
                terminal.key(b"h",0.4)
                assert TXID in terminal.screen(),terminal.screen()
                terminal.key(b"\x1b[18~",0.4)
                terminal.key(b"j\r",0.6)
                assert B2["hash"] in terminal.screen(),terminal.screen()
                terminal.key(b"h",0.4)
                assert TXID in terminal.screen(),terminal.screen()
                # At a fork point, n must ask instead of following a stale nextblockhash.
                terminal.key(b"/"+GENESIS["hash"].encode()+b"\r",0.6)
                terminal.key(b"n",0.5)
                wait(terminal,lambda:"Child branches" in terminal.screen() and A1["hash"] in terminal.screen() and B1["hash"] in terminal.screen())
                terminal.key(b"\r",0.6)
                assert A1["hash"] in terminal.screen(),terminal.screen()
                terminal.key(b"h",0.4)
                assert GENESIS["hash"] in terminal.screen(),terminal.screen()
                with sqlite3.connect(path) as db:
                    assert db.execute("select count(*) from block_membership where membership='orphan'").fetchone()[0] == 2
                    assert db.execute("select count(*) from significant_events where message like 'Reorg%'").fetchone()[0] == 1
                    assert db.execute("pragma integrity_check").fetchone()[0] == "ok"
                    assert db.execute("pragma foreign_key_check").fetchall() == []
            finally:
                terminal.close()
            server.tip=B3["hash"]
            terminal=Terminal(root,conf,poll_interval=1)
            try:
                wait(terminal,lambda:tip_is(B3))
                revision = state()["revision"]
                server.flip_after_tip, server.tip = B3["hash"], A2["hash"]
                wait(terminal,lambda:"tip changed during reconciliation" in terminal.screen())
                assert tip_is(B3) and state()["revision"] == revision
                with sqlite3.connect(path) as db:
                    assert db.execute("select membership from block_membership where block_hash=?",(A2["hash"],)).fetchone()[0] == "orphan"
                terminal.key(b"/"+A2["hash"].encode()+b"\r",0.7)
                wait(terminal,lambda:"Orphaned" in terminal.screen())
                # Reopen stored inclusions and child branches after a process restart.
                terminal.key(b"/"+TXID.encode()+b"\r",0.6)
                terminal.key(b"\x1b[18~",0.6)
                wait(terminal,lambda:"Transaction inclusion history" in terminal.screen() and A2["hash"] in terminal.screen() and B2["hash"] in terminal.screen())
                # Keep the investigation open during a real branch replacement.
                server.tip=A2["hash"]
                wait(terminal,lambda:tip_is(A2))
                wait(terminal,lambda:"Last observed active" in displayed_membership(terminal,A2["hash"]) and "Last observed orphaned" in displayed_membership(terminal,B2["hash"]))
                terminal.key(b"\x1b",0.3)
                terminal.key(b"/"+GENESIS["hash"].encode()+b"\r",0.5)
                terminal.key(b"\x1b[18~",0.5)
                wait(terminal,lambda:"Child branches" in terminal.screen() and A1["hash"] in terminal.screen() and B1["hash"] in terminal.screen())
                terminal.key(b"\x1b",0.3)
                assert GENESIS["hash"] in terminal.screen(),terminal.screen()
                # Disable the source; cached branch browsing and opening must still work.
                terminal.key(b"\x1bOQ",0.3)  # F2
                terminal.key(b"e",0.3)
                terminal.key(b"\x1b",0.4)
                calls=len(server.calls)
                terminal.key(b"\x1b[18~",0.4)
                wait(terminal,lambda:"Child branches" in terminal.screen() and A1["hash"] in terminal.screen())
                terminal.key(b"r",0.4)
                terminal.key(b"\r",0.4)
                assert A1["hash"] in terminal.screen(),terminal.screen()
                assert len(server.calls)==calls,server.calls[calls:]
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()
    print("Reorg PTY checks passed: incomplete walk, atomic replacement, open orphan, branch navigation, active counterpart, restart, inclusion history, fork selection, Back navigation and SQLite integrity.")

if __name__ == "__main__":
    run()
