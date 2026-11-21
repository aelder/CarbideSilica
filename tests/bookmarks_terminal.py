"""Linux PTY integration check. Run after cargo build; uses only fake local daemons."""
import fcntl
import http.server
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import tempfile
import termios
import threading
import time
import unicodedata

VRSC = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV"
VDEX = "iHog9UCTrn95qpUBFCZ7kKz7qWdMA8MQ6N"
TXID = "a" * 64
VETH = "i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X"


class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        method = request["method"]
        self.server.calls.append(method)
        self.server.requests.append((method, request["params"]))
        value = None
        handler = getattr(self.server, "rpc_handlers", {}).get(method)
        if handler is not None:
            value = handler(request["params"])
        elif method == "getinfo":
            value = dict(chainid=self.server.system, testnet=False, blocks=0, longestchain=0, connections=0)
        elif method == "getblockchaininfo":
            value = dict(blocks=0, bestblockhash="b" * 64)
        elif method == "getrawtransaction" and not self.server.reject_transactions:
            value = dict(txid=request["params"][0], vin=[dict(coinbase="00")], vout=[], confirmations=0, size=10)
        elif method in ("getblock", "getblockheader"):
            value = dict(hash="b" * 64, height=0, time=1700000000, tx=[], size=10, confirmations=1)
        elif method == "getcurrency":
            target = request["params"][0]
            if target.lower() == "veth" or target == VETH:
                value = dict(currencyid=VETH, name="vETH", systemid=self.server.system)
            elif "@" not in target:
                value = dict(currencyid=target, name="VRSC" if self.server.system == VRSC else "vDEX", systemid=self.server.system)
        error = None if value is not None or handler is not None else dict(code=-1, message="Not provided by mock")
        body = json.dumps(dict(result=value, error=error, id=request["id"])).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def daemon(system):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Daemon)
    server.system, server.calls = system, []
    server.requests, server.reject_transactions = [], False
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class Terminal:
    def __init__(self, root, conf, testnet=False, poll_interval=60, chain=None):
        self.width, self.height = 160, 40
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 160, 0, 0))
        env = os.environ.copy()
        env.update(TERM="xterm-256color", HOME=str(root), XDG_DATA_HOME=str(root / "data"), XDG_CONFIG_HOME=str(root / "config"), XDG_CACHE_HOME=str(root / "cache"))
        command = ["target/debug/vcarbide", "--conf", str(conf), "--emoji", "off", "--poll-interval", str(poll_interval)]
        if chain is not None:
            command += ["--chain", chain]
        if testnet:
            command += ["-testnet"]
        self.process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave, env=env)
        os.close(slave)
        self.captured = bytearray()
        self.read(0.7)

    def read(self, seconds=0.25):
        end = time.monotonic() + seconds
        while time.monotonic() < end:
            if select.select([self.master], [], [], 0.025)[0]:
                try:
                    self.captured.extend(os.read(self.master, 65536))
                except OSError:
                    break
        return self.screen()

    def key(self, value, seconds=0.25):
        os.write(self.master, value)
        return self.read(seconds)

    def resize(self, width, height):
        self.read(0.05)
        self.width, self.height = width, height
        self.captured.clear()
        fcntl.ioctl(self.master, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
        # The child has a PTY but no controlling terminal in this harness.
        self.process.send_signal(signal.SIGWINCH)
        return self.read(0.20)

    def screen(self):
        # Reconstruct incremental Ratatui redraws; stripping ANSI loses unchanged cells.
        cells = [[" "] * self.width for _ in range(self.height)]
        row = col = 0
        tokens = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07|\x1b\\)|\x1b.|[^\x1b]")
        for match in tokens.finditer(self.captured.decode(errors="replace")):
            token = match.group()
            if token.startswith("\x1b["):
                command, values = token[-1], token[2:-1].lstrip("?")
                parts = [int(x or 0) for x in values.split(";")] if all(x.isdigit() or not x for x in values.split(";")) else [0]
                if command in "Hf":
                    row, col = (parts[0] or 1) - 1, ((parts[1] if len(parts) > 1 else 1) or 1) - 1
                elif command == "J" and parts[0] in (2, 3):
                    cells = [[" "] * self.width for _ in range(self.height)]
                elif command == "K" and 0 <= row < self.height:
                    for x in range(col if parts[0] == 0 else 0, self.width if parts[0] != 1 else col + 1):
                        if 0 <= x < self.width:
                            cells[row][x] = " "
                elif command == "G":
                    col = (parts[0] or 1) - 1
                elif command == "A":
                    row -= parts[0] or 1
                elif command == "B":
                    row += parts[0] or 1
                elif command == "C":
                    col += parts[0] or 1
                elif command == "D":
                    col -= parts[0] or 1
            elif token.startswith("\x1b"):
                continue
            elif token == "\r":
                col = 0
            elif token == "\n":
                row += 1
            elif token >= " ":
                if 0 <= row < self.height and 0 <= col < self.width:
                    cells[row][col] = token
                col += 2 if unicodedata.east_asian_width(token) in ("W", "F") else 1
        return "\n".join("".join(line) for line in cells)

    def close(self):
        self.key(b"q")
        try:
            self.process.wait(timeout=3)
            assert self.process.returncode == 0
        finally:
            if self.process.poll() is None:
                self.process.kill()
                self.process.wait()
            os.close(self.master)


def run():
    servers = [daemon(VRSC), daemon(VDEX)]
    with tempfile.TemporaryDirectory(prefix="explorer-bookmarks-") as directory:
        root = Path(directory)
        config = root / "config/verus-explorer-tui"
        config.mkdir(parents=True)
        confs = []
        for i, server in enumerate(servers):
            conf = root / f"daemon{i}.conf"
            conf.write_text(f"rpcconnect=127.0.0.1\nrpcport={server.server_port}\n")
            confs.append(conf)
        (config / "config.toml").write_text(f'[[connections]]\nnetwork="mainnet"\nchain="vDEX"\nconfig_file="{confs[1]}"\nstate="disabled"\n')
        data = root / "data/verus-explorer-tui"
        data.mkdir(parents=True)
        path = data / "bookmarks-mainnet.jsonl"
        foreign = dict(version=1, system_id=VDEX, kind="transaction", target=TXID, description="Foreign investigation", created_at=1700000000)
        path.write_text("malformed retained row\n" + json.dumps(foreign) + "\n")
        terminal = Terminal(root, confs[0])
        try:
            screen = terminal.key(b"\x1b[17~", 0.6)
            assert "Foreign investigation" in screen and "Uncached" in screen, screen
            lines = screen.splitlines()
            assert "VRSC" in lines[0] and "view bookmarks" in lines[1], screen
            assert any(line.startswith(">") and "Foreign investigation" in line for line in lines), screen
            assert any(line.startswith("controls j/k select") for line in lines[-4:]), screen
            assert "F2 sources" not in screen, screen
            assert "invalid line" in screen, screen
            assert servers[1].calls == [], servers[1].calls
            screen = terminal.key(b"\x1b[C", 0.5)
            assert "disabled daemon" in screen, screen
            assert "controls 1 connect once" in screen and "j/k select" not in screen, screen
            screen = terminal.key(b"1", 0.7)
            assert "Object system: vDEX" in screen and TXID[:16] in screen, screen
            assert screen.splitlines()[0].startswith(" VRSC") or "VRSC" in screen.splitlines()[0], screen
            screen = terminal.key(b"\x7f")
            assert "Object system: vDEX" not in screen, screen
            terminal.key(b"/" + TXID.encode() + b"\r", 0.7)
            screen = terminal.key(b"B")
            assert "Transaction Overview" in screen and "Add bookmark" in screen, screen
            assert "view bookmarks" not in screen and "F6 bookmarks" not in screen, screen
            terminal.key(b"Local q ")
            terminal.resize(68, 20)
            assert "Add bookmark" in terminal.screen() and "Local q _" in terminal.screen(), terminal.screen()
            terminal.resize(160, 40)
            screen = terminal.key(b"investigation\r", 0.6)
            assert "Transaction Overview" in screen and "Add bookmark" not in screen, screen
            rows = path.read_text().splitlines()
            assert rows[0] == "malformed retained row"
            assert json.loads(rows[-1])["system_id"] == VRSC
            assert json.loads(rows[-1])["description"] == "Local q investigation"
            screen = terminal.key(b"\x1b[17~e")
            assert "Edit bookmark description" in screen, screen
            assert "controls Enter save description | Esc cancel" in screen, screen
            assert "a bookmark current object" not in screen, screen
            terminal.key(b"!\r")
            assert json.loads(path.read_text().splitlines()[-1])["description"].endswith("!")
            terminal.key(b"x")
            terminal.key(b"n")
            assert len(path.read_text().splitlines()) == 3
            terminal.key(b"x")
            terminal.key(b"y")
            assert len(path.read_text().splitlines()) == 2
            terminal.key(b"n123\tManual block\r")
            manual = json.loads(path.read_text().splitlines()[-1])
            assert manual["target"] == "123" and manual["description"] == "Manual block", manual
            terminal.key(b"xy")
            assert len(path.read_text().splitlines()) == 2
        finally:
            terminal.close()
        # Enabled sources alone do not authorize startup population. With opt-in,
        # the cached bookmark is skipped and only one of two missing objects is fetched.
        extra = [dict(foreign, target="c" * 64, description="Missing one"), dict(foreign, target="d" * 64, description="Missing two")]
        path.write_text(path.read_text() + "".join(json.dumps(row) + "\n" for row in extra))
        connection = f'[[connections]]\nnetwork="mainnet"\nchain="vDEX"\nconfig_file="{confs[1]}"\nstate="enabled"\n'
        (config / "config.toml").write_text(connection)
        before = servers[1].calls.count("getrawtransaction")
        terminal = Terminal(root, confs[0])
        try:
            terminal.read(0.5)
            assert servers[1].calls.count("getrawtransaction") == before
        finally:
            terminal.close()
        (config / "config.toml").write_text('[bookmarks]\nfetch_missing_on_startup=true\nstartup_limit=1\n' + connection)
        terminal = Terminal(root, confs[0])
        try:
            screen = terminal.key(b"\x1b[17~", 0.8)
            assert "1 fetched, 1 attempted" in screen, screen
            assert servers[1].calls.count("getrawtransaction") == before + 1
            assert "Missing two" in screen and "Uncached" in screen, screen
        finally:
            terminal.close()
        # Cache remains useful when the source is no longer configured.
        (config / "config.toml").write_text("")
        confs[0].write_text("rpcconnect=127.0.0.1\nrpcport=1\n")
        before = len(servers[1].calls)
        terminal = Terminal(root, confs[0])
        try:
            screen = terminal.key(b"\x1b[17~", 0.6)
            assert "Cached observation" in screen and "Coinbase" in screen, screen
            screen = terminal.key(b"\r", 0.7)
            assert f"Object system: {VDEX}" in screen, screen
            assert len(servers[1].calls) == before
        finally:
            terminal.close()
        terminal = Terminal(root, confs[0], testnet=True)
        try:
            screen = terminal.key(b"\x1b[17~", 0.6)
            assert "No bookmarks" in screen and "Foreign investigation" not in screen, screen
        finally:
            terminal.close()
    for server in servers:
        server.shutdown()
        server.server_close()
    print("Bookmark PTY checks passed: CRUD, malformed preservation, consent, foreign navigation/Back/current chain, offline cache, tags, network isolation and opt-in bounded startup fetching.")


def run_currency_permission():
    local, first, second = [daemon(VRSC) for _ in range(3)]
    with tempfile.TemporaryDirectory(prefix="explorer-currency-permission-") as directory:
        root = Path(directory)
        conf = root / "VRSC.conf"
        conf.write_text(f"rpcconnect=127.0.0.1\nrpcport={local.server_port}\n")
        config = root / "config/verus-explorer-tui"
        config.mkdir(parents=True)
        (config / "config.toml").write_text("".join(f'[[apis]]\nnetwork="mainnet"\nchain="VRSC"\nendpoint="http://127.0.0.1:{api.server_port}"\nenabled=false\n' for api in (first, second)))
        data = root / "data/verus-explorer-tui"
        data.mkdir(parents=True)
        path = data / "bookmarks-mainnet.jsonl"
        legacy = dict(version=1, system_id=VRSC, kind="auto", target="vETH@@", description="Legacy currency", created_at=1700000000)
        transaction = dict(legacy, kind="transaction", target=TXID, description="Transaction requiring a retry")
        path.write_text(json.dumps(legacy) + "\n" + json.dumps(transaction) + "\n")
        local.reject_transactions = True
        terminal = Terminal(root, conf)
        try:
            screen = terminal.key(b"\x1b[17~", .35)
            assert "Legacy currency" in screen and "vETH@@" not in screen, screen
            screen = terminal.key(b"\r", .7)
            assert "Currency vETH" in screen and "API communication requires permission" not in screen, screen
            assert ("getcurrency", ["vETH"]) in local.requests, local.requests
            assert not any("vETH@@" in json.dumps(args) for _, args in local.requests)
            assert first.calls == second.calls == []
            assert json.loads(path.read_text().splitlines()[0])["target"] == "vETH@@"
            screen = terminal.key(b"B")
            assert "Add bookmark" in screen and "Currency vETH" in screen, screen
            assert "Kind: currency" in screen and "view bookmarks" not in screen, screen
            terminal.key(b"\x1b")
            assert len(path.read_text().splitlines()) == 2
            screen = terminal.key(b"BvETH detail\r")
            assert "Currency vETH" in screen and "Add bookmark" not in screen, screen
            saved = json.loads(path.read_text().splitlines()[-1])
            assert saved["kind"] == "currency" and saved["target"] == VETH, saved
            terminal.key(b"\x1b[17~", .25)
            terminal.key(b"\x1b[Hj\r", .5)
            screen = terminal.screen()
            assert "view bookmarks" in screen and "API communication requires permission" in screen, screen
            assert "getrawtransaction failed on VRSC" in screen and "Not provided by mock" in screen, screen
            assert "F2 Sources" in screen and "q quit" not in screen and "Esc cancel" in screen, screen
            assert f"127.0.0.1:{first.server_port}" in screen and f"127.0.0.1:{second.server_port}" in screen, screen
            terminal.key(b"q", .2)
            assert terminal.process.poll() is None and "API communication requires permission" in terminal.screen()
            count = local.calls.count("getrawtransaction")
            terminal.key(b"r", .4)
            assert local.calls.count("getrawtransaction") == count + 1
            assert first.calls == second.calls == []
            terminal.key(b"\x1b", .3)
            assert "API communication requires permission" not in terminal.screen(), terminal.screen()
            assert first.calls == second.calls == []
            terminal.key(b"\r", .4)
            terminal.key(b"j\r", .7)
            assert first.calls == [] and "getrawtransaction" in second.calls, (first.calls, second.calls)
            assert "Transaction Overview" in terminal.screen(), terminal.screen()
        finally:
            if "API communication requires permission" in terminal.screen():
                terminal.key(b"\x1b", .2)
            terminal.close()
            for server in (local, first, second):
                server.shutdown()
                server.server_close()
    print("Currency bookmark and API recovery PTY checks passed.")


if __name__ == "__main__":
    import sys
    if "--currency-permission-only" not in sys.argv:
        run()
    run_currency_permission()
