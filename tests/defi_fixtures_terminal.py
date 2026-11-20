"""Real captured refund batches and an Ethereum-origin burn; isolated mock RPC only."""
import copy
import json
from pathlib import Path
import tempfile
from bookmarks_terminal import Terminal, VRSC
from defi_terminal import daemon, wait

FIXTURES = Path(__file__).parent/'fixtures/defi-refunds'
EXPECTED = json.loads((FIXTURES/'verification.json').read_text())


def run_refunds(case, corrupt_prefix=False):
    capture = json.loads((FIXTURES/case['file']).read_text())
    record = capture['record']
    transactions = [capture['transaction'], capture['export_transaction'], *capture['source_transactions']]
    txs = {tx['txid']: copy.deepcopy(tx) for tx in transactions}
    if corrupt_prefix:
        txs[capture['transaction']['txid']]['vout'][2]['valueSat'] -= 1
    server = daemon(VRSC, record['importheight'], txs, record['importheight'], record)
    server.export_records, server.export_ranges = [], []
    receipts = case['receipts'][-1:] if corrupt_prefix else case['receipts']
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-captured-refunds-') as directory:
            root = Path(directory)
            conf = root/'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal = Terminal(root, conf)
            try:
                for receipt in receipts:
                    source = txs[receipt['source_txid']]
                    transfer_outputs = [out['n'] for out in source['vout'] if 'reservetransfer' in out.get('scriptPubKey', {})]
                    root_index = transfer_outputs.index(receipt['source_vout'])
                    terminal.key(b'/' + source['txid'].encode() + b'\r', .4)
                    wait(terminal, 'Txid: ' + source['txid'])
                    terminal.key(b'\x1b[19~', .15)
                    terminal.key(b'j'*root_index + b'\r', .4)
                    if corrupt_prefix:
                        wait(terminal, 'Imported; receipt unresolved')
                        assert 'Refunded output verified' not in terminal.screen(), terminal.screen()
                    else:
                        wait(terminal, 'Refunded output verified')
                        wait(terminal, 'Replayed refund result')
                        screen = terminal.screen()
                        assert case['txid'] in screen, screen
                        assert receipt['amount'].rstrip('0').rstrip('.') in screen, screen
                        assert 'Conversion fee' not in screen and 'Execution rate' not in screen, screen
                    assert 'API communication requires permission' not in terminal.screen(), terminal.screen()
                    terminal.key(b'\x1b', .15)
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()


def run_burn():
    capture = json.loads((FIXTURES/'burn-ethereum-dai.json').read_text())
    txs = {tx['txid']: tx for tx in (capture['transaction'], capture['previous_transaction'], capture['other_prevout_transaction'])}
    server = daemon(VRSC, capture['record']['importheight'], txs)
    server.requests, server.export_records, server.export_ranges = [], [], []
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-captured-burn-') as directory:
            root = Path(directory)
            conf = root/'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal = Terminal(root, conf)
            try:
                terminal.key(b'/' + capture['transaction']['txid'].encode() + b'\r', .5)
                wait(terminal, 'Transaction Overview')
                wait(terminal, ' Burn ')
                assert 'Consumption Burn' not in terminal.screen(), terminal.screen()
                # This identifier is external bridge export evidence, not a VRSC prevout.
                external_export = capture['record']['import']['exporttxid']
                assert not any(method == 'getrawtransaction' and params[0] == external_export for method, params in server.requests)
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()


def run():
    refunds = [case for case in EXPECTED['fixtures'] if case['kind'] == 'refund']
    for case in refunds:
        run_refunds(case)
    run_refunds(refunds[0], corrupt_prefix=True)
    run_burn()
    print('Captured DeFi PTY checks passed: ten real refund receipts, altered-prefix rejection, DAI import Burn tag and external-export boundary.')


if __name__ == '__main__':
    run()
