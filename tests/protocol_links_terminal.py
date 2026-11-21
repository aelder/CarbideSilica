"""Output badge and typed protocol navigation against isolated mock daemons."""
import copy
import json
from pathlib import Path
import tempfile
from bookmarks_terminal import Terminal, VRSC, VDEX
from defi_terminal import SOURCE, EXPORT, BLOCK, CURRENCY, ADDRESS, TRANSFER, daemon, wait

ROUTING = 'RTqQe58LSj2yr5CrwYFwcsAQ1edQwmrkUU'


def run():
    transfer = dict(TRANSFER, version=1, destination=dict(type=2, address=ADDRESS))
    outputs = [dict(n=i, valueSat=0, scriptPubKey={}) for i in range(8)]
    for index in (0, 7):
        outputs[index] = dict(n=index, valueSat=10000,
            scriptPubKey=dict(addresses=[ROUTING], reservetransfer=copy.deepcopy(transfer)))
    tx = dict(txid=SOURCE, height=1, blockhash=BLOCK, confirmations=1, vin=[], vout=outputs)
    source = daemon(VRSC, 1, {SOURCE: tx})
    remote = daemon(VDEX, 1, {EXPORT: dict(txid=EXPORT, height=1, blockhash=BLOCK,
        confirmations=1, vin=[], vout=[dict(n=i, valueSat=0, scriptPubKey={}) for i in range(8)])})
    source.requests, remote.requests = [], []
    source.block_transactions = remote.block_transactions = []
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-protocol-links-') as directory:
            root = Path(directory)
            conf, other = root/'VRSC.conf', root/'vDEX.conf'
            for path, server in ((conf, source), (other, remote)):
                path.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            config = root/'config/verus-explorer-tui'
            config.mkdir(parents=True)
            (config/'config.toml').write_text(f'[[connections]]\nnetwork="mainnet"\nchain="vDEX"\nconfig_file="{other}"\nstate="enabled"\n')
            terminal = Terminal(root, conf)
            try:
                terminal.key(b'/'+SOURCE.encode()+b'\r', .5)
                wait(terminal, SOURCE)
                # Last output has four typed references after its Reserve Transfer badge.
                terminal.key(b'\x1b[Fkkkk\r', .5)
                wait(terminal, 'DeFi trail')
                screen = terminal.screen()
                assert SOURCE+':7' in screen, screen
                terminal.key(b'\x1b', .25)

                # Conversion badge opens a currency on the observing system.
                conversion = copy.deepcopy(tx)
                conversion['txid'] = '1'*64
                conversion['vout'] = [dict(n=0, valueSat=10000, scriptPubKey=dict(
                    addresses=[ROUTING], reservetransfer=dict(transfer, flags=3, convert=True)))]
                source.txs[conversion['txid']] = conversion
                terminal.key(b'/'+conversion['txid'].encode()+b'\r', .5)
                wait(terminal, conversion['txid'])
                terminal.key(b'\x1b[Fkkkk\r', .5)
                wait(terminal, 'Currency Bridge.vETH')
                assert CURRENCY in terminal.screen(), terminal.screen()
                assert not source.state_requests, source.state_requests
                terminal.key(b'\x1b', .25)
                wait(terminal, conversion['txid'])

                # Explicit cross-system destination: last reference is Currency,
                # immediately preceded by Transfer destination.
                cross = copy.deepcopy(tx)
                cross['txid'] = 'e'*64
                cross['vout'] = [dict(n=0, valueSat=10000, scriptPubKey=dict(
                    addresses=[ROUTING], reservetransfer=dict(transfer, flags=65,
                        crosssystem=True, exportto=VDEX)))]
                source.txs[cross['txid']] = cross
                terminal.key(b'/'+cross['txid'].encode()+b'\r', .5)
                wait(terminal, cross['txid'])
                terminal.key(b'\x1b[Fk\r', .5)
                wait(terminal, ADDRESS)
                assert any(method == 'getaddressbalance' for method, _ in remote.requests), remote.requests
                assert not any(method == 'getaddressbalance' and ADDRESS in str(params)
                               for method, params in source.requests), source.requests
                terminal.key(b'?', .2)
                wait(terminal, 'Verus Carbide help')
                terminal.key(b'\x1b', .2)
                assert ADDRESS in terminal.screen(), terminal.screen()
                assert 'Verus Carbide help' not in terminal.screen(), terminal.screen()
                terminal.key(b'\x1b', .3)
                wait(terminal, cross['txid'])

                # Source export is appended after decoded currency references.
                imported = dict(txid='f'*64, height=1, blockhash=BLOCK, confirmations=1,
                    vin=[], vout=[dict(n=0, valueSat=0, scriptPubKey=dict(crosschainimport=dict(
                        sourcesystemid=VDEX, importcurrencyid=CURRENCY, exporttxid=EXPORT, exporttxout=7)))])
                source.txs[imported['txid']] = imported
                terminal.key(b'/'+imported['txid'].encode()+b'\r', .5)
                wait(terminal, imported['txid'])
                terminal.key(b'\x1bOQ')
                wait(terminal, 'Sources')
                terminal.key(b'\x1bORj\r')
                wait(terminal, 'Transaction Overview')
                assert 'Relationships — F3' not in terminal.screen(), terminal.screen()
                terminal.key(b'\x1bOQk\r')  # Restore the original monitoring chain.
                terminal.key(b'\x1bOR', .3)
                wait(terminal, 'Relationships — F3')
                wait(terminal, 'Enter/Right/l follow reference')
                assert any(line.lstrip().startswith('>') and 'Source export' in line
                           for line in terminal.screen().splitlines()), terminal.screen()
                terminal.key(b'l', .5)
                wait(terminal, EXPORT)
                terminal.key(b'\x1b', .25)
                wait(terminal, imported['txid'])
                terminal.key(b'\x1b[F\r', .5)
                wait(terminal, EXPORT)
                assert any(line.strip().startswith('>') and 'OUTPUT 7' in line
                           for line in terminal.screen().splitlines()), terminal.screen()
                terminal.key(b'\x1b', .3)
                wait(terminal, imported['txid'])
                # Foreign destination without a configured source leaves the
                # transaction open; it must never fall back to the local daemon.
                missing = copy.deepcopy(cross)
                missing['txid'] = '2'*64
                unknown = 'i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X'
                missing['vout'][0]['scriptPubKey']['reservetransfer']['exportto'] = unknown
                source.txs[missing['txid']] = missing
                terminal.key(b'/'+missing['txid'].encode()+b'\r', .5)
                wait(terminal, missing['txid'])
                terminal.key(b'\x1b[Fk\r', .3)
                wait(terminal, 'No configured source')
                assert missing['txid'] in terminal.screen(), terminal.screen()
                assert not any(method == 'getaddressbalance' and ADDRESS in str(params)
                               for method, params in source.requests), source.requests

                evidence = json.loads((Path(__file__).parent / 'fixtures/output-presentation.json').read_text())
                carrier_hash = '4' * 64
                export = dict(evidence['export'], vin=[], blockhash=carrier_hash, height=228124, confirmations=1)
                legacy = dict(evidence['legacy'], vin=[], blockhash=carrier_hash, confirmations=1)
                source.txs.update({export['txid']: export, legacy['txid']: legacy})
                source.blocks = {
                    carrier_hash: dict(hash=carrier_hash, height=228124, time=1700000000,
                                       tx=[export['txid'], legacy['txid']], size=100, confirmations=1),
                    legacy['notarized_hash']: dict(hash=legacy['notarized_hash'],
                        height=legacy['notarized_height'], time=legacy['notarized_time'],
                        tx=[], size=100, confirmations=1)
                }

                terminal.key(b'/' + export['txid'].encode() + b'\r')
                wait(terminal, 'Pending Export Finalization')
                wait(terminal, 'Export referenced by finalization')
                screen = terminal.key(b'\x1b[F\r', .5)
                assert any(line.strip().startswith('>') and 'OUTPUT 1' in line
                           for line in screen.splitlines()), screen

                terminal.key(b'/' + legacy['txid'].encode() + b'\r')
                wait(terminal, 'Legacy Notarization')
                wait(terminal, 'Notarized VRSC block: #228120')
                wait(terminal, legacy['destination_txid'])
                terminal.key(b'\x1b[F\r')
                wait(terminal, 'Height: 228120')
                assert any(method == 'getblock' and params[0] == legacy['notarized_hash']
                           for method, params in source.requests), source.requests
                terminal.key(b'\x1b')
                wait(terminal, 'Txid: ' + legacy['txid'])
                terminal.key(b'b')
                wait(terminal, 'Height: 228124')
                wait(terminal, 'Legacy Notarization')
                wait(terminal, 'Pending Export Finalization')

                data_txid = '5' * 64
                payload = b'Hello, Verus!\x00\xff\n' + bytes(range(16))
                source.txs[data_txid] = dict(txid=data_txid, vin=[], confirmations=0,
                    vout=[dict(n=0, valueSat=0, scriptPubKey=dict(
                        type='nulldata', hex=(bytes([0x6a, len(payload)]) + payload).hex()))])
                terminal.key(b'/' + data_txid.encode() + b'\r')
                wait(terminal, 'View OP_RETURN bytes')
                terminal.key(b'\r')
                wait(terminal, 'OP_RETURN output #0')
                wait(terminal, '48 65 6C 6C 6F 2C')
                wait(terminal, 'Hello, Verus!...')
                terminal.key(b'l')
                wait(terminal, 'Offset 0x00000001 | 0x65')
                terminal.key(b'j')
                wait(terminal, 'Offset 0x00000011 | 0x01')
                terminal.key(b'\x1b')
                wait(terminal, 'Txid: ' + data_txid)

                assert 'Allow API' not in terminal.screen(), terminal.screen()
            finally:
                terminal.close()
    finally:
        for server in (source, remote):
            server.shutdown()
            server.server_close()


if __name__ == '__main__':
    run()
