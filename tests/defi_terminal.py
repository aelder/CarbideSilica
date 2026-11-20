"""PTY check of the F8 reserve-transfer trail against a small fake daemon."""
import copy
import http.server
import json
from pathlib import Path
import tempfile
import threading
import time
from bookmarks_terminal import Terminal, VRSC, VDEX

SOURCE, EXPORT, IMPORT = "a" * 64, "b" * 64, "c" * 64
BLOCK = "d" * 64
CURRENCY = "i3f7tSctFkiPpiedY8QR5Tep9p4qDVebDx"
ADDRESS = "RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp"
HASH = "e" * 64
TRANSFER = dict(flags=1, currencyvalues={VRSC: 1.0}, fees=0.0001, feecurrencyid=VRSC,
                destinationcurrencyid=CURRENCY, destination=dict(address=ADDRESS))
EXPORT_INFO = dict(firstinput=0, numinputs=1, sourcesystemid=VRSC,
                   destinationsystemid=VRSC, destinationcurrencyid=CURRENCY,
                   hashtransfers=HASH)
IMPORT_INFO = dict(exporttxid=EXPORT, exporttxout=0, sourcesystemid=VRSC,
                   importcurrencyid=CURRENCY, hashtransfers=HASH, numoutputs=1)
TXS = {
    SOURCE: dict(txid=SOURCE, height=1, blockhash=BLOCK, confirmations=1,
                 vin=[dict(coinbase="00")], vout=[dict(n=0, value=0.0001,
                 scriptPubKey=dict(reservetransfer=TRANSFER, addresses=["contract"]),
                 spentTxId=EXPORT, spentIndex=0, spentHeight=1)]),
    EXPORT: dict(txid=EXPORT, height=1, blockhash=BLOCK, confirmations=1,
                 vin=[dict(txid=SOURCE, vout=0)],
                 vout=[dict(n=0, value=0, scriptPubKey=dict(crosschainexport=EXPORT_INFO))]),
    IMPORT: dict(txid=IMPORT, height=1, blockhash=BLOCK, confirmations=1,
                 vin=[dict(coinbase="00")], vout=[
                     dict(n=0, value=0, scriptPubKey=dict(crosschainimport=IMPORT_INFO)),
                     dict(n=1, value=1.0, scriptPubKey=dict(addresses=[ADDRESS], type="pubkeyhash", spendableoutput=True)),
                 ]),
}


class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        method, params = request["method"], request["params"]
        self.server.calls.append(method)
        if hasattr(self.server, "requests"):
            self.server.requests.append((method, params))
        value = None
        if method == "getinfo":
            value = dict(chainid=self.server.system, testnet=False, blocks=self.server.tip,
                         longestchain=self.server.tip, connections=0)
        elif method == "getblockcount":
            value = self.server.tip
        elif method == "getblockchaininfo":
            value = dict(blocks=self.server.tip, bestblockhash=getattr(self.server, "branch_hash", BLOCK))
        elif method == "getblockhash":
            height = int(params[0])
            value = (getattr(self.server, "branch_hash", BLOCK) if height == self.server.tip
                     else f'{height:064x}') if getattr(self.server, 'timed_headers', False) else BLOCK
            value = getattr(self.server, 'block_hashes', {}).get(height, value)
        elif method in ("getblock", "getblockheader"):
            height = int(params[0]) if str(params[0]).isdigit() and len(str(params[0])) < 10 else self.server.tip
            if getattr(self.server, 'timed_headers', False):
                tip_hash = getattr(self.server, "branch_hash", BLOCK)
                if len(str(params[0])) == 64:
                    height = self.server.tip if params[0] == tip_hash else int(params[0], 16)
                block_hash = tip_hash if height == self.server.tip else f'{height:064x}'
                value = dict(hash=block_hash, height=height, time=1700000000-(self.server.tip-height)*60,
                             previousblockhash=f'{max(0,height-1):064x}',
                             tx=getattr(self.server, "block_transactions", list(self.server.txs)),
                             size=10, confirmations=1)
            else:
                value = dict(hash=getattr(self.server, "branch_hash", BLOCK), height=height, time=1700000000, tx=getattr(self.server, "block_transactions", list(self.server.txs)),
                             size=10, confirmations=1)
            value = getattr(self.server, 'blocks', {}).get(str(params[0]), value)
        elif method == "getrawtransaction":
            value = self.server.txs.get(params[0])
        elif method == "getcurrency":
            self.server.currency_requests.append(params[0])
            names = {VRSC: "VRSC", VDEX: "vDEX", CURRENCY: "Bridge.vETH",
                     "i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X": "vETH"}
            value = dict(currencyid=params[0], name=names.get(params[0], params[0]),
                         systemid=self.server.system)
            value = getattr(self.server, "currency_definitions", {}).get(params[0], value)
        elif method == "getcurrencystate":
            self.server.state_requests.append(tuple(params))
            value = [dict(height=int(params[1]), currencystate=dict(currencyid=params[0], version=1, flags=49, supply=123))]
        elif method == "getpendingtransfers":
            value = (getattr(self.server, "pending_records", [])
                     if params[0] == getattr(self.server, "pending_currency", None) else None)
        elif method == "getimports":
            self.server.import_ranges.append(tuple(params[1:]))
            value = [self.server.import_record] if self.server.import_record and int(params[1]) <= self.server.import_height <= int(params[2]) else None
        elif method == "getexports":
            self.server.export_ranges.append(tuple(params[1:]))
            value = [record for record in self.server.export_records if int(params[1]) <= record["height"] <= int(params[2])]
        body = json.dumps(dict(result=value, error=None if value is not None or method == "getimports" else
                               dict(code=-5, message="mock unavailable"), id=request["id"])).encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


def wait(terminal, needle):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        if needle in terminal.read(0.1):
            return
    raise AssertionError(f"Missing {needle}:\n{terminal.screen()}")


def wait_scrolled(terminal, needle):
    deadline = time.monotonic() + 12
    while time.monotonic() < deadline:
        if needle in terminal.key(b"\x1b[6~", 0.15):
            return
    raise AssertionError(f"Missing {needle}:\n{terminal.screen()}")


def daemon(system, tip, txs, import_height=None, import_record=None):
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Daemon)
    server.system, server.tip, server.txs = system, tip, txs
    server.import_height, server.import_record = import_height, import_record
    server.calls, server.import_ranges, server.state_requests, server.currency_requests = [], [], [], []
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


def run():
    record = dict(importheight=1, importtxid=IMPORT, importvout=0,
                  importnotarization=dict(launchcomplete=True, launchconfirmed=True),
                  transfers=[TRANSFER], **{"import": IMPORT_INFO})
    server = daemon(VRSC, 1, TXS, 1, record)
    try:
        with tempfile.TemporaryDirectory(prefix="explorer-defi-") as directory:
            root = Path(directory)
            conf = root / "VRSC.conf"
            conf.write_text(f"rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n")
            terminal = Terminal(root, conf)
            try:
                terminal.key(b"/" + SOURCE.encode() + b"\r", 0.7)
                wait(terminal, SOURCE)
                terminal.key(b"\x1b[19~", 0.3)  # F8
                wait(terminal, "DeFi trail")
                terminal.key(b"\x1b[C", 0.4)
                wait(terminal, "Direct transfer result")
                assert "getimports" in server.calls
                assert EXPORT in terminal.screen() and IMPORT in terminal.screen()
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()


def run_cross_system(tip, search_keys, locate=True):
    cross_transfer = dict(TRANSFER, version=1, flags=65, crosssystem=True, exportto=VDEX)
    currency_export = dict(EXPORT_INFO, destinationsystemid=VDEX)
    system_export = dict(currency_export, destinationcurrencyid=VDEX)
    currency_import = dict(IMPORT_INFO, importcurrencyid=CURRENCY)
    system_import = dict(IMPORT_INFO, importcurrencyid=VRSC, exporttxout=1)
    source_txs = {
        SOURCE: dict(TXS[SOURCE], vout=[dict(n=0, value=0.0001,
            scriptPubKey=dict(reservetransfer=cross_transfer, addresses=["contract"]),
            spentTxId=EXPORT, spentIndex=0, spentHeight=1)]),
        EXPORT: dict(TXS[EXPORT], vout=[
            dict(n=0, value=0, scriptPubKey=dict(crosschainexport=currency_export)),
            dict(n=1, value=0, scriptPubKey=dict(crosschainexport=system_export))]),
    }
    destination_txs = {
        IMPORT: dict(TXS[IMPORT], height=82104, vout=[
            dict(n=0, value=0, scriptPubKey=dict(crosschainimport=currency_import)),
            dict(n=1, value=0, scriptPubKey=dict(crosschainimport=system_import)),
            dict(n=2, value=0, scriptPubKey=dict(acceptednotarization={})),
            dict(n=3, value=0, scriptPubKey=dict(notaryevidence={})),
            dict(n=4, value=0, scriptPubKey=dict(crosschainexport=dict(issupplemental=True))),
            dict(n=5, value=0, valueSat=0, scriptPubKey=dict(reserveoutput=dict(currencyvalues={VRSC: 1.0}), addresses=[ADDRESS])),
        ])
    }
    record = dict(importheight=82104, importtxid=IMPORT, importvout=0,
                  importnotarization=dict(launchcomplete=True, launchconfirmed=True,
                                          currencystate=dict(version=1)),
                  transfers=[cross_transfer], **{"import": currency_import})
    source = daemon(VRSC, 1, source_txs)
    destination = daemon(VDEX, tip, destination_txs, 82104, record if locate else None)
    try:
        with tempfile.TemporaryDirectory(prefix="explorer-defi-cross-") as directory:
            root = Path(directory)
            source_conf, destination_conf = root / "VRSC.conf", root / "vDEX.conf"
            for conf, server in [(source_conf, source), (destination_conf, destination)]:
                conf.write_text(f"rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n")
            config = root / "config/verus-explorer-tui"
            config.mkdir(parents=True)
            (config / "config.toml").write_text(f'[[connections]]\nnetwork="mainnet"\nchain="vDEX"\nconfig_file="{destination_conf}"\nstate="enabled"\n')
            terminal = Terminal(root, source_conf)
            try:
                terminal.key(b"/" + SOURCE.encode() + b"\r", 0.7)
                wait(terminal, SOURCE)
                terminal.key(b"\x1b[19~", 0.3)
                terminal.key(b"\r", 0.4)
                wait(terminal, "Matching import not in these windows")
                terminal.key(search_keys, 0.5)
                if locate:
                    wait(terminal, "Replayed reserve result")
                    screen = terminal.screen()
                    assert "Companion system export" in screen and "Companion system import" in screen, screen
                    assert "Import proof/evidence" in screen and "Supplemental transfer evidence" in screen, screen
                    assert IMPORT in screen and EXPORT in screen, screen
                    assert any(start <= 82104 <= end for start, end in destination.import_ranges), destination.import_ranges
                    assert len(destination.import_ranges) <= 33, destination.import_ranges
                else:
                    wait(terminal, "32 window(s)")
                    assert len(destination.import_ranges) == 33, destination.import_ranges
                    assert destination.import_ranges[0] == (tip - 999, tip)
                    for earlier, later in zip(destination.import_ranges, destination.import_ranges[1:]):
                        assert later[1] + 1 == earlier[0], destination.import_ranges
            finally:
                terminal.close()
    finally:
        for server in (source, destination):
            server.shutdown()
            server.server_close()


def run_conversion_batch(later=False, reserve=False):
    veth = "i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X"
    transfer = dict(version=1, flags=1027, convert=True, reservetoreserve=True,
                    currencyvalues={veth: 0.00395291}, fees=0.00020010,
                    feecurrencyid=VRSC, via=CURRENCY, destinationcurrencyid=VDEX if reserve else VRSC,
                    destination=dict(address=ADDRESS, type=66))
    other = (dict(transfer, currencyvalues={veth: 0.00790582}) if later else
             dict(version=1, flags=1, currencyvalues={veth: 0.59293245},
                  fees=0.00024012, feecurrencyid=VRSC, destinationcurrencyid=CURRENCY,
                  destination=dict(address="0x05c6b3a4ffda0dfe3daad775b265c3fa9cdb7209", type=201)))
    export = dict(EXPORT_INFO, firstinput=1, numinputs=2)
    imported = dict(IMPORT_INFO, numoutputs=4)
    txs = {
        SOURCE: dict(TXS[SOURCE], vout=[dict(n=0, value=0.00020010,
            scriptPubKey=dict(reservetransfer=transfer), spentTxId=EXPORT, spentIndex=1)] +
            ([dict(n=1, value=0.00020010, scriptPubKey=dict(reservetransfer=other),
                   spentTxId=EXPORT, spentIndex=2)] if later else [])),
        EXPORT: dict(TXS[EXPORT], vin=[dict(coinbase="00"),
            dict(txid=SOURCE, vout=0), dict(txid=SOURCE if later else "f" * 64, vout=1)],
            vout=[dict(n=0, value=0, scriptPubKey=dict(crosschainexport=export))]),
        IMPORT: dict(TXS[IMPORT], vout=[
            dict(n=0, value=0, scriptPubKey=dict(crosschainimport=imported)),
            dict(n=1, value=0, scriptPubKey=dict(acceptednotarization={})),
            dict(n=2, value=10.01692152, valueSat=1001692152,
                 scriptPubKey=dict(type="pubkeyhash", addresses=[ADDRESS], spendableoutput=True,
                     hex="76a9144f01c06114c899baab0522499ead597bf883e78988ac")),
            (dict(n=3, value=20.03379223, valueSat=2003379223,
                  scriptPubKey=dict(type="pubkeyhash", addresses=[ADDRESS], spendableoutput=True,
                      hex="76a9144f01c06114c899baab0522499ead597bf883e78988ac")) if later else
             dict(n=3, value=0, scriptPubKey=dict(reservetransfer=dict(destination=dict(address=other["destination"]["address"]))))),
            dict(n=4, value=0.0002, scriptPubKey=dict(type="pubkeyhash", addresses=["fee1"])),
            dict(n=5, value=0.0002, scriptPubKey=dict(type="pubkeyhash", addresses=["fee2"])),
        ]),
    }
    if reserve:
        for index, amount in [(2, 10.01692152), (3, 20.03379223)]:
            txs[IMPORT]["vout"][index] = dict(n=index, value=0, valueSat=0,
                scriptPubKey=dict(addresses=[ADDRESS],
                    reserveoutput=dict(currencyvalues={VDEX: amount})))
    record = dict(importheight=1, importtxid=IMPORT, importvout=0,
                  importnotarization=dict(launchcomplete=True, launchconfirmed=True,
                      currencystate=dict(version=1, flags=49, currencyid=CURRENCY, currencies={
                          veth: dict(lastconversionprice=0.00569406),
                          (VDEX if reserve else VRSC): dict(viaconversionprice=14.43626273)})),
                  transfers=[transfer, other], **{"import": imported})
    server = daemon(VRSC, 1, txs, 1, record)
    try:
        with tempfile.TemporaryDirectory(prefix="explorer-defi-replay-") as directory:
            root = Path(directory)
            conf = root / "VRSC.conf"
            conf.write_text(f"rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n")
            terminal = Terminal(root, conf)
            try:
                terminal.key(b"/" + SOURCE.encode() + b"\r", 0.7)
                wait(terminal, SOURCE)
                terminal.key(b"\x1b[19~", 0.3)
                if later:
                    terminal.key(b"j", 0.2)
                terminal.key(b"\r", 0.4)
                wait(terminal, "Replayed conversion result")
                assert "Conversion fee" in terminal.screen(), terminal.screen()
                assert "Received output verified" in terminal.screen(), terminal.screen()
                if later:
                    assert "batch members 0..1" in terminal.screen()
                assert "Destination-address candidate" not in terminal.screen()
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()


def run_gateway_route(tamper=False, locate=True, navigate_currency=False, convert=False, pool_checks=False, wrong_pool=False):
    # Synthetic two-chain route using the real gateway replay amounts/rates.
    fixture = json.loads((Path(__file__).parent / "fixtures/vrsc-gateway-leg.json").read_text())
    record, imported_tx = fixture["record"], fixture["transaction"]
    batch = record["transfers"]
    veth = next(iter(batch[1]["currencyvalues"]))
    batch[1]["destination"].update(address=ADDRESS, type=194, gateway=VDEX)
    batch[1]["destination"]["auxdests"] = [dict(address=ADDRESS, type=2)]
    state = record["importnotarization"]["currencystate"]
    state["currencies"][VDEX] = copy.deepcopy(state["currencies"][veth])
    next_transfer = imported_tx["vout"][3]["scriptPubKey"]["reservetransfer"]
    next_transfer.update(exportto=VDEX, destinationcurrencyid=VDEX, feecurrencyid=VDEX)
    next_transfer["destination"].update(address=ADDRESS, type=66, auxdests=[dict(address=ADDRESS, type=2)])
    if convert:
        batch[1].update(flags=1027, convert=True, reservetoreserve=True, via=CURRENCY,
                        destinationcurrencyid=VRSC, currencyvalues={veth: 0.00790582})
        next_transfer["currencyvalues"] = {VRSC: 20.03379223}
        imported_tx["vout"][3].update(value=20.03379223, valueSat=2003379223)
    if tamper:
        next_transfer["fees"] = 0.00210012
    record.update(importheight=1, importtxid=IMPORT, importvout=0)
    record["import"].update(exporttxid=EXPORT, exporttxout=0)
    imported_tx.update(txid=IMPORT, height=1, confirmations=1)
    imported_tx["vout"][0]["scriptPubKey"]["crosschainimport"] = record["import"]
    next_export, final_import = "1" * 64, "2" * 64
    imported_tx["vout"][3].update(spentTxId=next_export, spentIndex=0)
    first_export = dict(EXPORT_INFO, numinputs=2, hashtransfers=record["import"]["hashtransfers"])
    second_export = dict(EXPORT_INFO, destinationsystemid=VDEX, destinationcurrencyid=VDEX)
    second_import = dict(IMPORT_INFO, exporttxid=next_export, importcurrencyid=VDEX)
    source_txs = {
        SOURCE: dict(TXS[SOURCE], vout=[dict(n=0, value=0, valueSat=0,
            scriptPubKey=dict(reservetransfer=batch[1]), spentTxId=EXPORT, spentIndex=1)]),
        EXPORT: dict(TXS[EXPORT], vin=[dict(txid="f" * 64, vout=0), dict(txid=SOURCE, vout=0)],
            vout=[dict(n=0, value=0, scriptPubKey=dict(crosschainexport=first_export))]),
        IMPORT: imported_tx,
        next_export: dict(TXS[EXPORT], txid=next_export, vin=[dict(txid=IMPORT, vout=3)],
            vout=[dict(n=0, value=0, scriptPubKey=dict(crosschainexport=second_export))]),
    }
    destination_txs = {final_import: dict(TXS[IMPORT], txid=final_import, height=82104, vout=[
        dict(n=0, value=0, scriptPubKey=dict(crosschainimport=second_import)),
        dict(n=1, value=0, valueSat=0, scriptPubKey=dict(addresses=[ADDRESS],
             reserveoutput=dict(currencyvalues={VRSC: 20.03379223} if convert else {veth: 0.59293245})))])}
    final_record = dict(importheight=82104, importtxid=final_import, importvout=0,
        importnotarization=dict(launchcomplete=True, launchconfirmed=True, currencystate=dict(version=1)),
        transfers=[next_transfer], **{"import": second_import})
    source = daemon(VRSC, 1, source_txs, 1, record)
    if pool_checks:
        prior = json.loads((Path(__file__).parent / "fixtures/vrsc-pool-buy.json").read_text())["transaction"]
        prior_id = record["importnotarization"]["prevnotarizationtxid"]
        prior.update(txid=prior_id, height=0)
        if wrong_pool:
            prior["vout"][1]["scriptPubKey"]["acceptednotarization"]["currencyid"] = "iWrong"
        source.txs[prior_id] = prior
        source.requests, source.block_transactions = [], []
    destination = daemon(VDEX, 300000, destination_txs, 82104, final_record if locate else None)
    try:
        with tempfile.TemporaryDirectory(prefix="explorer-defi-route-") as directory:
            root = Path(directory)
            source_conf, destination_conf = root / "VRSC.conf", root / "vDEX.conf"
            for conf, server in [(source_conf, source), (destination_conf, destination)]:
                conf.write_text(f"rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n")
            config = root / "config/verus-explorer-tui"
            config.mkdir(parents=True)
            (config / "config.toml").write_text(f'[[connections]]\nnetwork="mainnet"\nchain="vDEX"\nconfig_file="{destination_conf}"\nstate="enabled"\n')
            terminal = Terminal(root, source_conf)
            try:
                terminal.key(b"/" + SOURCE.encode() + b"\r", 0.7)
                wait(terminal, SOURCE)
                terminal.key(b"\x1b[19~", 0.3)
                terminal.key(b"\r", 0.5)
                if tamper:
                    wait(terminal, "Destination-address candidate")
                    assert not destination.import_ranges, destination.import_ranges
                else:
                    # Scroll to the route end: two legs exceed one viewport.
                    terminal.key(b"\x1b[6~" * 5, 0.3)
                    wait_scrolled(terminal, "Matching import not in these windows")
                    terminal.key(b"g82104\r", 0.5)
                    terminal.key(b"\x1b[6~" * 5, 0.3)
                    if locate:
                        wait_scrolled(terminal, "Replayed reserve result")
                        terminal.key(b"j" * 12, 0.4)
                        wait(terminal, "> Replayed reserve result")
                        if navigate_currency:
                            terminal.resize(74, 22)
                            wait(terminal, "> Replayed reserve result")
                            terminal.resize(160, 40)
                            wait(terminal, "> Replayed reserve result")
                        # Currency navigation from a later leg must use vDEX.
                        terminal.key(b"c", 0.3)
                        wait(terminal, "Currencies for selected leg")
                        assert "Currency " + ("VRSC" if convert else "vETH") in terminal.screen(), terminal.screen()
                        assert " on " + VDEX in terminal.screen(), terminal.screen()
                        if navigate_currency:
                            terminal.key(b"\r", 0.4)
                            wait(terminal, "Definition")
                            assert f"Object system: vDEX ({VDEX})" in terminal.screen(), terminal.screen()
                            assert "Observing system: vDEX" in terminal.screen(), terminal.screen()
                            # Definitions may be shared from cache; explicit refresh must target vDEX.
                            terminal.key(b"\x1b[15~", 0.3)
                            deadline = time.monotonic() + 12
                            while veth not in destination.currency_requests and time.monotonic() < deadline:
                                terminal.read(0.1)
                            assert veth in destination.currency_requests, destination.currency_requests
                            assert "vETH" in terminal.screen(), terminal.screen()
                            assert "DeFi trail" not in terminal.screen()
                            return
                        terminal.key(b"\x1b", 0.2)
                        terminal.key(b"k" * 12, 0.4)
                        if convert:
                            assert "Principal net" in terminal.screen(), terminal.screen()
                            assert "Conversion fee" in terminal.screen(), terminal.screen()
                        if pool_checks:
                            assert not [p for m, p in source.requests if m == "getrawtransaction" and p[0] == prior_id], source.requests
                            terminal.key(b"v", 0.4)
                            wait(terminal, "Pool-wide changes between notarizations")
                            if wrong_pool:
                                wait(terminal, "Previous state unavailable")
                                assert "Supply: unavailable ->" in terminal.screen(), terminal.screen()
                            else:
                                wait(terminal, "Previous notarization:")
                                assert "Import state:" in terminal.screen() and "Supply:" in terminal.screen(), terminal.screen()
                            assert len([p for m, p in source.requests if m == "getrawtransaction" and p[0] == prior_id]) == 1, source.requests
                            terminal.key(b"v", 0.2)
                        terminal.key(b"p", 0.4)
                        wait(terminal, "Historical state")
                        assert source.state_requests == [(CURRENCY, 1)], source.state_requests
                        assert not destination.state_requests, destination.state_requests

                    else:
                        wait_scrolled(terminal, "Matching import not in these windows")
                    assert source.import_ranges == [(0, 1), (0, 1)], source.import_ranges
                    assert destination.import_ranges == [(299001, 300000), (81105, 82104)], destination.import_ranges
                    assert "Approve" not in terminal.screen()
            finally:
                terminal.close()
    finally:
        for server in (source, destination):
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    run()
    run_cross_system(300000, b"g82104\r")
    run_cross_system(84000, b"s")
    run_cross_system(84000, b"s", locate=False)
    run_conversion_batch()
    run_conversion_batch(later=True)
    run_conversion_batch(later=True, reserve=True)
    run_gateway_route()
    run_gateway_route(tamper=True)
    run_gateway_route(locate=False)
    run_gateway_route(navigate_currency=True)
