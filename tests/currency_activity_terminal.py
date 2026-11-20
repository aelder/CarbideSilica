"""Bounded activity, retained windows and exact-output F8 entry using a fake daemon."""
import copy
from pathlib import Path
import tempfile
import time
from bookmarks_terminal import Terminal, VRSC, VDEX
from defi_terminal import (SOURCE, EXPORT, IMPORT, BLOCK, CURRENCY, ADDRESS, HASH,
                           TRANSFER, EXPORT_INFO, IMPORT_INFO, TXS, daemon, wait)


def run(nontransfer=False, cross=False):
    transfer = dict(TRANSFER, version=1, destination=dict(address=ADDRESS, type=2))
    if cross:
        transfer.update(flags=65, crosssystem=True, exportto=VDEX)
    txs = copy.deepcopy(TXS)
    # The selected source has two transfer outputs; only :7 is in this export.
    txs[SOURCE]['vout'] = [dict(n=n, value=0, valueSat=0, scriptPubKey={}) for n in range(8)]
    txs[SOURCE]['vout'][0]['scriptPubKey']['reservetransfer'] = dict(transfer, currencyvalues={VRSC: 99})
    txs[SOURCE]['vout'][7].update(scriptPubKey={} if nontransfer else dict(reservetransfer=transfer),
                                spentTxId=EXPORT, spentIndex=1)
    export = dict(EXPORT_INFO, firstinput=1, destinationsystemid=VDEX if cross else VRSC)
    txs[EXPORT]['vin'] = [dict(coinbase='00'), dict(txid=SOURCE, vout=7)]
    txs[EXPORT]['vout'][0]['scriptPubKey']['crosschainexport'] = export
    txs[EXPORT]['height'] = 4800
    txs[IMPORT]['height'] = 4800
    txs[IMPORT]['vout'][1].update(valueSat=100000000)
    txs[IMPORT]['vout'][1]['scriptPubKey']['hex'] = '76a9144f01c06114c899baab0522499ead597bf883e78988ac'
    record = dict(importheight=4800, importtxid=IMPORT, importvout=0,
                  importnotarization=dict(launchcomplete=True, launchconfirmed=True, currencystate=dict(version=1)),
                  transfers=[transfer], **{'import': IMPORT_INFO})
    server = daemon(VRSC, 5000, txs, 4800, record)
    server.export_records = [dict(height=4800, txid=EXPORT, txoutnum=0, exportinfo=export)]
    server.export_ranges, server.requests, server.block_transactions = [], [], []
    source = server
    if cross:
        txs[IMPORT]['vout'][1].update(value=0, valueSat=0, scriptPubKey=dict(addresses=[ADDRESS], reserveoutput=dict(currencyvalues={VRSC: 1})))
        source.import_record = None
        server = daemon(VDEX, 5000, {IMPORT: txs[IMPORT]}, 4800, record)
        server.export_records, server.export_ranges, server.requests, server.block_transactions = [], [], [], []
    server.timed_headers = True
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-currency-activity-') as directory:
            root = Path(directory)
            conf = root / 'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            if cross:
                source_conf = root / 'source.conf'
                source_conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={source.server_port}\nrpcuser=test\nrpcpassword=test\n')
                config = root / 'config/verus-explorer-tui'
                config.mkdir(parents=True)
                (config / 'config.toml').write_text(f'[[connections]]\nnetwork="mainnet"\nchain="VRSC"\nconfig_file="{source_conf}"\nstate="enabled"\n')
            terminal = Terminal(root, conf, chain='vDEX' if cross else None)
            try:
                terminal.key(b'/' + CURRENCY.encode() + b'@@\r', 0.6)
                wait(terminal, 'Currency Bridge.vETH')
                # Explicit heights, Unix timestamps and text dates share the
                # chart target parser. Future targets must not query state.
                terminal.key(b'gheight:5000\r', 0.6)
                wait(terminal, 'Historical state')
                before_state = len(server.state_requests)
                terminal.key(b'gunix:1800000000\r', 0.6)
                wait(terminal, 'Enter a time between genesis')
                assert len(server.state_requests) == before_state
                terminal.key(b'g2023-11-14 22:13:20 UTC\r', 0.6)
                wait(terminal, 'Historical state')
                deadline = time.monotonic() + 12
                while len(server.state_requests) == before_state and time.monotonic() < deadline:
                    terminal.read(0.1)
                assert len(server.state_requests) == before_state + 1, server.state_requests
                wait(terminal, 'exact timestamp')
                terminal.key(b'w', 0.6)
                wait(terminal, 'Currency activity heights 4001..5000')
                assert server.import_ranges == [(4001, 5000)], server.import_ranges
                assert server.export_ranges == [(4001, 5000)], server.export_ranges
                assert not [r for r in server.requests if r[0] == 'getrawtransaction'], server.requests
                terminal.key(b'jjj', 0.3)  # Select the import; its source export is explicit.
                terminal.key(b'\x1b[19~', 0.5)
                wait(terminal, 'Export inputs from')
                terminal.key(b'jj\r', 0.6)
                if nontransfer:
                    wait(terminal, 'Output 7 is not a reserve transfer')
                    assert 'DeFi trail — F8' not in terminal.screen(), terminal.screen()
                    return
                wait(terminal, 'Selected output 7')
                wait(terminal, 'Replayed reserve result' if cross else 'Replayed transfer result')
                assert 'Selected output 0:' not in terminal.screen(), terminal.screen()
                terminal.key(b'\x1b', 0.2)  # Close F8.
                terminal.key(b'\x1b', 0.3)  # Back to currency activity.
                wait(terminal, 'Currency Bridge.vETH')
                terminal.key(b'[', 0.5)
                wait(terminal, 'Currency activity heights 3001..4000')
                wait(terminal, 'None found in this window')  # getimports returns null.
                terminal.key(b']', 0.5)
                wait(terminal, 'Currency activity heights 4001..5000')
                assert server.export_ranges[-2:] == [(3001, 4000), (4001, 5000)], server.export_ranges
                before = len(server.import_ranges)
                server.branch_hash = 'f' * 64
                terminal.key(b'w', 0.4)
                wait(terminal, 'activity anchor changed')
                assert len(server.import_ranges) == before, server.import_ranges
                server.branch_hash = BLOCK
            finally:
                terminal.close()
            # A fresh process renders retained pages without re-querying activity.
            before = (len(server.import_ranges), len(server.export_ranges))
            terminal = Terminal(root, conf, chain='vDEX' if cross else None)
            try:
                terminal.key(b'/' + CURRENCY.encode() + b'@@\r', 0.6)
                wait(terminal, 'Currency Bridge.vETH')
                terminal.key(b'j' * 20, 0.4)
                wait(terminal, 'retained observation')
                assert (len(server.import_ranges), len(server.export_ranges)) == before
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()
        if source is not server:
            source.shutdown()
            source.server_close()


if __name__ == '__main__':
    run()
    run(nontransfer=True)
    run(cross=True)
