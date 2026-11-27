#!/usr/bin/env python3
"""Run the Linux explorer against the synthetic chain used by the chart tests."""
import argparse
import base64
from functools import partial
from hashlib import sha256
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import secrets
import subprocess
from tempfile import TemporaryDirectory
from threading import Thread
import time

from bookmarks_terminal import VRSC
from charts_terminal import header as chart_header

ROOT = Path(__file__).resolve().parents[1]
TIP = 5000
TIP_TIME = int(time.time())
BLOCK_HASHES = [sha256(f"synthetic VRSC block {h}".encode()).hexdigest() for h in range(TIP + 1)]
HEIGHT_BY_HASH = {digest: h for h, digest in enumerate(BLOCK_HASHES)}
RECIPIENT = "REit4Djn6K456BLuBGAEHqAMVeysLjGM7A"


def header(height):
    block = chart_header(height)
    block.update(hash=BLOCK_HASHES[height], time=TIP_TIME - (TIP - height) * 60)
    if height:
        block["previousblockhash"] = BLOCK_HASHES[height - 1]
    if height < TIP:
        block["nextblockhash"] = BLOCK_HASHES[height + 1]
    return block


# Give transactions a separate namespace from the chart fixture's block hashes.
def txid(height):
    return f"c{height:063x}"


def height_for(selector):
    text = str(selector)
    height = HEIGHT_BY_HASH.get(text, -1) if len(text) == 64 else int(text)
    if not 0 <= height <= TIP:
        raise ValueError("Unknown synthetic block")
    return height


def transaction(height):
    block = header(height)
    return dict(txid=txid(height), version=4, size=100, blockhash=block["hash"],
                height=height, blocktime=block["time"], time=block["time"],
                confirmations=block["confirmations"], vin=[dict(coinbase="00")],
                vout=[dict(n=0, value=3, valueSat=300_000_000, scriptPubKey=dict(type="pubkeyhash",
                          addresses=[RECIPIENT]))])


def rpc(method, params):
    tip = header(TIP)
    if method == "getinfo":
        return dict(chainid=VRSC, name="VRSC", VRSCversion="synthetic-demo",
                    testnet=False, blocks=TIP, longestchain=TIP, connections=1,
                    difficulty=tip["difficulty"], tiptime=tip["time"])
    if method == "getblockcount":
        return TIP
    if method == "getblockchaininfo":
        return dict(blocks=TIP, headers=TIP, bestblockhash=tip["hash"])
    if method == "getchaintips":
        return [dict(height=TIP, hash=tip["hash"], branchlen=0, status="active")]
    if method == "getblockhash":
        return BLOCK_HASHES[height_for(params[0])]
    if method in ("getblock", "getblockheader"):
        height = height_for(params[0])
        block = header(height)
        if method == "getblock":
            decoded = len(params) > 1 and params[1] == 2
            block["tx"] = [transaction(height) if decoded else txid(height)]
            block["size"] = 220
        else:
            del block["tx"]
        return block
    if method == "getrawtransaction":
        query = str(params[0])
        height = int(query[1:], 16)
        if not 0 <= height <= TIP or txid(height) != query:
            raise ValueError("Unknown synthetic transaction")
        return transaction(height)
    if method == "getpeerinfo":
        return [dict(id=1, addr="192.0.2.1:27485", subver="/Synthetic:1/",
                     inbound=False, synced_headers=TIP, synced_blocks=TIP)]
    if method == "listbanned":
        return []
    if method == "getnettotals":
        return dict(totalbytesrecv=24000, totalbytessent=12000, timemillis=int(time.time() * 1000))
    if method == "getmempoolinfo":
        return dict(size=0, bytes=0, usage=0)
    if method == "getrawmempool":
        return {} if params and params[0] else []
    if method == "getcurrency" and params[0] in ("VRSC", "VRSC@", VRSC):
        return dict(currencyid=VRSC, name="VRSC", systemid=VRSC)
    raise NotImplementedError(method)


class Handler(BaseHTTPRequestHandler):
    def __init__(self, *args, auth, **kwargs):
        self.auth = auth
        super().__init__(*args, **kwargs)

    def log_message(self, *_):
        pass

    def do_POST(self):
        if self.headers.get("Authorization") != self.auth:
            self.send_error(401)
            return
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        result, error = None, None
        try:
            result = rpc(request["method"], request.get("params", []))
        except NotImplementedError:
            error = dict(code=-32601, message="Method unavailable in the synthetic demo")
        except (ValueError, TypeError, IndexError) as exc:
            error = dict(code=-8, message=str(exc))
        body = json.dumps(dict(result=result, error=error, id=request.get("id"))).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "linux/target/debug/carbide-silica")
    parser.add_argument("--open", help="Open a fixture height, block hash or transaction ID")
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not os.access(binary, os.X_OK):
        parser.error(f"Build the Linux app first (missing executable at {binary})")
    password = secrets.token_hex(16)
    auth = "Basic " + base64.b64encode(f"demo:{password}".encode()).decode()
    with TemporaryDirectory(prefix="carbide-synthetic-") as tmp, ThreadingHTTPServer(
        ("127.0.0.1", 0), partial(Handler, auth=auth)
    ) as server:
        conf = Path(tmp) / "VRSC.conf"
        conf.write_text(f"rpcconnect=127.0.0.1\nrpcport={server.server_port}\n"
                        f"rpcuser=demo\nrpcpassword={password}\n")
        env = dict(os.environ, HOME=tmp, XDG_CONFIG_HOME=f"{tmp}/config",
                   XDG_DATA_HOME=f"{tmp}/data", XDG_CACHE_HOME=f"{tmp}/cache",
                   CARBIDE_SILICA_SETTINGS=f"{tmp}/settings.json",
                   CARBIDE_SILICA_RESOURCES=str(ROOT / "linux/resources"))
        settings = Path(env["XDG_CONFIG_HOME"]) / "verus-explorer-tui/config.toml"
        settings.parent.mkdir(parents=True)
        settings.write_text('[[connections]]\nnetwork = "mainnet"\nchain = "VRSC"\n'
                            f'config_file = {json.dumps(str(conf))}\nstate = "monitored"\n')
        Thread(target=server.serve_forever, daemon=True).start()
        command = [str(binary), f"-conf={conf}"]
        if args.open:
            command.append(f"-open={args.open}")
        print(f"Synthetic VRSC at 127.0.0.1:{server.server_port}, tip {TIP}. "
              "Close the explorer to stop the demo.", flush=True)
        try:
            return subprocess.run(command, env=env).returncode
        finally:
            server.shutdown()


if __name__ == "__main__":
    raise SystemExit(main())
