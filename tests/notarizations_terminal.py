"""Notarization browsing and proof-root checks on paired mock VRSC/vDEX daemons."""
import http.server
import json
import tempfile
import threading
import os
from unittest.mock import patch
from pathlib import Path

from bookmarks_terminal import Terminal, VRSC, VDEX, VETH
from charts_terminal import wait
from mouse_terminal import click


def key(n):
    return f"{n:064x}"


def row(n, block, vrsc, vdex, root_hash, vout=0):
    return dict(txid=key(n), vout=vout, blockhash=block, notarization=dict(proofroots=[
        dict(systemid=VRSC, height=vrsc, blockhash=key(4000 + vrsc)),
        dict(systemid=VDEX, height=vdex, blockhash=root_hash),
    ]))


VRSC_ROWS = [
    row(100, key(500), 4800, 800, key(800), 1),
    row(101, key(501), 4810, 810, key(810)),
    row(102, key(502), 4820, 820, key(820)),
    row(103, key(503), 4830, 830, key(830)),
    row(104, key(504), 4840, 840, key(840)),
    row(105, key(505), 4850, 1200, key(1200)),
    row(106, key(506), 4860, 800, key(800), 2),
]
VRSC_ROWS[0]["notarization"].update(
    ismirror=True, currencyid=VDEX, prevnotarizationtxid=key(199), prevnotarizationout=2
)
VDEX_ROWS = [row(200, key(600), 4870, 880, key(880)), row(201, key(601), 4880, 890, key(890))]


def graph(rows, forks):
    return dict(version=1, lastconfirmed=0, bestchain=0, forks=forks,
                notarizations=[dict(item, index=i) for i, item in enumerate(rows)])


class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        req = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        method, params = req["method"], req["params"]
        server = self.server
        server.calls.append((method, params))
        vrsc = server.chain == VRSC
        tip = 5000 if vrsc else 1000
        error = None
        result = None
        if method == "getinfo":
            result = dict(chainid=server.chain, testnet=False, blocks=tip, longestchain=tip,
                          connections=1, name="VRSC" if vrsc else "vDEX")
        elif method == "getblockchaininfo":
            result = dict(blocks=tip, bestblockhash=key(tip + 9000))
        elif method == "getblockcount":
            result = tip
        elif method == "getblockhash":
            result = key(int(params[0]) + 9000)
        elif method == "getcurrency":
            subject = {"vrsc": VRSC, VRSC: VRSC, "veth": VETH, VETH: VETH}.get(
                params[0].lower() if not params[0].startswith("i") else params[0], VDEX
            )
            result = dict(currencyid=subject, systemid=subject, launchsystemid=VRSC,
                          options={VRSC: 0, VDEX: 256, VETH: 128}[subject],
                          name={VRSC: "VRSC", VDEX: "vDEX", VETH: "vETH"}[subject])
        elif method == "getidentity" and vrsc and params == [VRSC]:
            result = dict(identity=dict(identityaddress=VRSC, name="VRSC", primaryaddresses=[]),
                          fullyqualifiedname="VRSC@", status="active", blockheight=0)
        elif method == "getnotarizationdata":
            peer = VDEX if vrsc else VRSC
            if vrsc and params == [VETH, False, False]:
                gateway = row(110, key(500), 4800, 800, key(800))
                gateway["notarization"]["proofroots"][1]["systemid"] = VETH
                result = graph([gateway], [[0]])
            elif params != [peer, False, False]:
                error = dict(code=-32602, message="Wrong perspective or evidence flags")
            else:
                result = server.graph
                error = server.graph_error
        elif method in ("getblockheader", "getblock"):
            target = params[0]
            number = int(target, 16) if len(str(target)) == 64 else int(target) + 9000
            if vrsc and 500 <= number <= 506:
                height = 4900 + number - 500
            elif not vrsc and 600 <= number <= 601:
                height = 900 + number - 600
            elif vrsc and 8870 <= number <= 8880:
                height = number - 4000
            elif not vrsc and number in (800, 810, 840):
                height = number if number != 840 else 841
            elif number >= 9000:
                height = number - 9000
            elif not vrsc and number == 830:
                error = dict(code=-32603, message="Header unavailable")
            else:
                error = dict(code=-5, message="Block not found")
            if error is None:
                result = dict(hash=key(number), height=height, time=1700000000 + height,
                              confirmations=-1 if number == 810 else 2, tx=[], size=100)
        elif method == "getrawtransaction":
            result = dict(txid=params[0], confirmations=2, size=100,
                          vin=[dict(coinbase="00")], vout=[dict(n=i, value=0, valueSat=0, scriptPubKey={}) for i in range(4)])
            if not vrsc and params[0] in (key(200), key(201)):
                result["vout"][0]["scriptPubKey"]["acceptednotarization"] = dict(version=1, currencyid=VRSC)
                result["vout"][1]["scriptPubKey"]["finalizenotarization"] = dict(
                    finalizationtype=1, currencyid=VRSC, output=dict(txid=key(0), voutnum=0),
                    evidenceoutputs=[3],
                )
                result["vout"][2]["scriptPubKey"]["finalizenotarization"] = dict(
                    finalizationtype=129, currencyid=VRSC, output=dict(txid=key(198), voutnum=0)
                )
                result["vout"][3]["scriptPubKey"]["notaryevidence"] = dict(
                    type=1, systemid=VDEX, output=dict(txid=key(0), voutnum=0),
                    evidence=dict(chainobjects=[
                        dict(vdxftype="iP1QT5ee7EP63WSrfjiMFc1dVJVSAy85cT", value=dict(
                            systemid=system, output=dict(txid=key(100), voutnum=1), confirmed=True,
                            signatures={signer: dict(blockheight=height, signatures=["synthetic-signature"])}
                        ))
                        for system, signer, height in [(VRSC, VRSC, 4800), (VETH, VDEX, 4801)]
                    ]),
                )
        else:
            error = dict(code=-32601, message="Not provided by mock")

        body = json.dumps(dict(result=result, error=error, id=req["id"])).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass


def daemon(chain, data):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Daemon)
    server.chain, server.graph, server.graph_error, server.calls = chain, data, None, []
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def run():
    vrsc = daemon(VRSC, graph(VRSC_ROWS, [[0, 1, 2], [0, 1, 3], [0, 4, 5, 6]]))
    vdex = daemon(VDEX, graph(VDEX_ROWS, [[0, 1]]))
    try:
        with tempfile.TemporaryDirectory(prefix="explorer-notarizations-") as directory:
            root = Path(directory)
            conf = root / "VRSC.conf"
            child = root / "vDEX.conf"
            for config, server in [(conf, vrsc), (child, vdex)]:
                config.write_text(f"rpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n")
            settings = root / "config/verus-explorer-tui"
            settings.mkdir(parents=True)
            (settings / "config.toml").write_text(
                f'[[connections]]\nnetwork="mainnet"\nchain="vDEX"\nconfig_file="{child}"\nstate="enabled"\n'
            )
            with patch.dict(os.environ, {"VERUS_EXPLORER_CLIPBOARD": "osc52"}):
                terminal = Terminal(root, conf)
            try:
                terminal.key(b"/vETH@@\r")
                wait(terminal, "Currency vETH")
                terminal.key(b"n")
                wait(terminal, "Notarizations · vETH")
                wait(terminal, "Selected notarization")
                screen = terminal.screen()
                assert "Tab perspective" not in screen, screen
                assert any(line.startswith(">") and "Confirmed" in line for line in screen.splitlines()), screen
                terminal.key(b"/vDEX@@\r")
                wait(terminal, "Currency vDEX")
                wait(terminal, "Unconfirmed in current view: 6")
                wait(terminal, "Included: #4900 at 2023-11-14T23:35:00Z")
                wait(terminal, f"Last confirmed: {key(100)}:1")
                terminal.key(b"n")
                wait(terminal, "VRSC view")
                wait(terminal, "Notarizations · vDEX")
                wait(terminal, "Selected notarization")
                wait(terminal, "Proof roots  VRSC #4800 | vDEX #800")
                wait(terminal, "vDEX Active")
                wait(terminal, "Known off-chain")
                wait(terminal, "Not found • possible fork")
                wait(terminal, "Height mismatch (found #841)")
                wait(terminal, "vDEX Unavailable")
                wait(terminal, f"Previous on vDEX  {key(199)}:2")
                wait(terminal, "Present on local-daemon:")
                terminal.key(b"\x1b[F")
                wait(terminal, "source tip #1000 is behind")
                terminal.resize(68, 19)
                terminal.resize(160, 40)
                wait(terminal, "source tip #1000 is behind")
                terminal.key(b"\x1b[H")
                wait(terminal, f"Previous on vDEX  {key(199)}:2")
                assert sum(method == "getnotarizationdata" and params[0] == VDEX for method, params in vrsc.calls) == 1
                assert sum(method == "getblockheader" and params[0] == key(800) for method, params in vdex.calls) == 1
                terminal.key(b"b")
                wait(terminal, "Height: 800")
                terminal.key(b"\x1b")
                wait(terminal, "VRSC view")
                terminal.key(b"\t")
                wait(terminal, "vDEX view")
                wait(terminal, f"Confirmed  {key(200)}:0")
                wait(terminal, "VRSC Active")
                assert [params for method, params in vdex.calls if method == "getnotarizationdata"] == [[VRSC, False, False]]
                terminal.key(b"\r")
                wait(terminal, "Txid: " + key(200))
                wait(terminal, "Notarization output 0: confirmed in vDEX view")
                terminal.key(b"\x1bOS")
                wait(terminal, "Witness investigation")
                wait(terminal, "Search block height")
                assert any(line.startswith("path ") and " > Witnesses" in line
                           for line in terminal.screen().splitlines()), terminal.screen()
                terminal.key(b"\x1b[C")
                wait(terminal, "2 signature groups")
                wait(terminal, "Signing height 4800")
                terminal.resize(72, 22)
                wait(terminal, "Signing height 4800")
                assert any(line.lstrip().startswith(">") and "VRSC" in line for line in terminal.screen().splitlines()), terminal.screen()
                terminal.resize(160, 40)
                wait(terminal, "Evidence: ")
                selected = next(line for line in terminal.screen().splitlines()
                                if "Signing height 4800" in line)
                display = selected.strip().split("·", 1)[0].lstrip("> ").strip()
                assert display in ("VRSC", "VRSC@"), selected
                assert click(terminal, "Signing height 4800", display) == VRSC
                terminal.key(b"I")
                assert any(line.split("·", 1)[0].strip().lstrip("> ").strip() == VRSC and "Signing height 4800" in line
                           for line in terminal.screen().splitlines()), terminal.screen()
                terminal.key(b"Il")
                wait(terminal, "Primary addresses:")
                assert any(line.startswith("path ") and "Witnesses > ID" in line
                           for line in terminal.screen().splitlines()), terminal.screen()
                terminal.key(b"\x1b")
                wait(terminal, "2 signature groups")
                terminal.key(b"j\r")
                wait(terminal, "Signer identity unavailable")
                assert any(VDEX in line and "Signing height 4801" in line
                           for line in terminal.screen().splitlines()), terminal.screen()
                terminal.key(b"k]")
                assert any(line.lstrip().startswith(">") and "Evidence:" in line
                           for line in terminal.screen().splitlines()), terminal.screen()
                terminal.key(b"\r")
                wait(terminal, "Txid: " + key(200))
                terminal.key(b"\x1b")
                wait(terminal, "2 signature groups")
                assert any(line.lstrip().startswith(">") and "Evidence:" in line
                           for line in terminal.screen().splitlines()), terminal.screen()
                terminal.key(b"\t")
                wait(terminal, "report:")
                assert click(terminal, "status:", "complete") == "complete"
                terminal.key(b"s")
                wait(terminal, "Save JSON")
                output = root / "witness-report.json"
                terminal.key(str(output).encode() + b"\r")
                document = json.loads(output.read_text())
                assert document["root"]["txid"] == key(200), document
                signer = document["report"]["signatures"][0]
                assert signer["identity"] == VRSC and signer["system"] == VRSC, signer
                assert signer["key_signatures"] == ["synthetic-signature"], signer
                assert signer["origins"][0]["output"]["vout"] == 3, signer
                terminal.key(b"\t\x1b")
                wait(terminal, "Notarization output 0: confirmed in vDEX view")
                terminal.key(b"\x1b")
                wait(terminal, "vDEX view")
                terminal.key(b"j")
                # Move the selected pending outpoint to another index and
                # confirm that the refreshed browser still opens that outpoint.
                vdex.graph = graph([VDEX_ROWS[0], row(202, key(600), 4870, 880, key(880)), VDEX_ROWS[1]], [[0, 1], [0, 2]])
                terminal.key(b"\x1b[15~")
                wait(terminal, "3 rows · 2 unconfirmed")
                terminal.key(b"\r")
                wait(terminal, "Txid: " + key(201))
                wait(terminal, "Notarization output 0: unconfirmed in vDEX view")
                wait(terminal, "Pending Notarization Finalization")
                wait(terminal, "Confirms Earlier Notarization")
                terminal.key(b"\x1b")
                wait(terminal, "vDEX view")
                vdex.graph = graph(VDEX_ROWS, [[0, 1]])
                terminal.key(b"\t")
                wait(terminal, "VRSC view")
                terminal.key(b"\x1b")
                wait(terminal, "Unconfirmed in current view: 6")
                vrsc.graph = None
                terminal.key(b"\x1b[15~")
                wait(terminal, "No notarization data returned")
                wait(terminal, "Unconfirmed in current view: unavailable")
                vrsc.graph_error = dict(code=-32601, message="Method not found")
                terminal.key(b"r")
                wait(terminal, "Notarization data unavailable")
                assert "Approve" not in terminal.screen()
            finally:
                terminal.close()
            vrsc.graph_error = None
            vrsc.graph = graph(VRSC_ROWS, [[0, 1, 2], [0, 1, 3], [0, 4, 5, 6]])
            (settings / "config.toml").write_text(
                f'[[connections]]\nnetwork="mainnet"\nchain="VRSC"\nconfig_file="{conf}"\nstate="enabled"\n'
            )
            terminal = Terminal(root, child, chain="vDEX")
            try:
                terminal.key(b"/VRSC@@\r")
                wait(terminal, "Currency VRSC")
                wait(terminal, f"Last confirmed: {key(200)}:0")
                wait(terminal, "Included: #900 at 2023-11-14T22:28:20Z")
                wait(terminal, "Unconfirmed in current view: 1")
                terminal.key(b"n")
                wait(terminal, "vDEX view")
                terminal.key(b"\t")
                wait(terminal, "VRSC view")
                wait(terminal, f"Confirmed  {key(100)}:1")
                terminal.key(b"B")
                wait(terminal, "Add bookmark")
                wait(terminal, "Kind: notarizations")
                wait(terminal, f"System ID: {VRSC}")
                wait(terminal, f"Target: {VDEX}")
                terminal.key(b"vDEX from VRSC\r")
                wait(terminal, "Notarizations · vDEX")
                bookmark_file = root / "data/verus-explorer-tui/bookmarks-mainnet.jsonl"
                entries = [json.loads(line) for line in bookmark_file.read_text().splitlines()]
                assert [(b["kind"], b["system_id"], b["target"]) for b in entries] == [
                    ("notarizations", VRSC, VDEX)
                ], entries
                vrsc.graph = graph(VRSC_ROWS[:2], [[0, 1]])
                terminal.key(b"\x1b[17~\r")
                wait(terminal, "VRSC view")
                wait(terminal, "2 rows · 1 unconfirmed")
                terminal.key(b"\t")
                wait(terminal, "vDEX view")
                terminal.key(b"\x1b")
                wait(terminal, "VRSC view")
                assert "Notarizations · vDEX" in terminal.screen(), terminal.screen()
                terminal.key(b"\x1b[17~nnotarizations:VRSC\tVRSC from vDEX\r")
                wait(terminal, "VRSC from vDEX")
                terminal.key(b"\r")
                wait(terminal, "Notarizations · VRSC")
                wait(terminal, "vDEX view")
                wait(terminal, f"Confirmed  {key(200)}:0")
                terminal.key(b"/" + key(200).encode() + b"\r")
                wait(terminal, "Txid: " + key(200))
                terminal.key(b"\x1bOS/4800")
                wait(terminal, "/ 4800")
                terminal.key(b"\x1b")
                wait(terminal, "Search block height")
                wait(terminal, "Witness investigation")
            finally:
                terminal.close()
    finally:
        for server in (vrsc, vdex):
            server.shutdown()
            server.server_close()
    print("Notarization terminal checks passed on paired VRSC and vDEX sources.")


if __name__ == "__main__":
    run()
