"""vDEX burn activity resolves reserve currency names from hydrated input prevouts."""
import http.server
import json
from pathlib import Path
import tempfile
import threading
import time
from bookmarks_terminal import Terminal, VDEX

OWNER = 'iExBJfZYK7KREDpuhj6PzZBzqMAKaFg7d2'
BRIDGE = 'i6j1rzjgrDhSmUYiTtp21J8Msiudv5hgt9'
PUMP = 'i7neKkvaZPnvfL6bcCnpe32vim4kaMDbWQ'
VRSC = 'i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV'
BURN = 'a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1'
SOURCE = 'a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2a2'
FEE = 'a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3a3'
BLOCK = 'b' * 64


def output(n, amount, currency=None):
    script = dict(addresses=[OWNER], type='cryptocondition', spendableoutput=True)
    if currency:
        script['reserveoutput'] = dict(version=1, currencyvalues={currency: amount})
    else:
        script['evalnone'] = 'valid'
    return dict(n=n, value=0 if currency else amount, valueSat=0 if currency else int(amount * 100000000), scriptPubKey=script)


def transaction(txid, vin, vout):
    return dict(txid=txid, height=1, blockhash=BLOCK, blocktime=1700000000,
                confirmations=1, expiryheight=21, size=10, vin=vin, vout=vout)


TXS = {
    BURN: transaction(BURN, [dict(txid=SOURCE, vout=5), dict(txid=SOURCE, vout=6),
                            dict(txid=FEE, vout=0)], [output(0, 0)]),
    SOURCE: transaction(SOURCE, [], [output(n, 0) for n in range(5)] +
                        [output(5, 12.12500001, BRIDGE), output(6, 34.37500002, PUMP)]),
    FEE: transaction(FEE, [], [output(0, 0.0001)]),
}

# Preserve the daemon's entire consumed-output decode, including reserve_balance
# metadata. The old synthetic fixture omitted it and missed the Burn regression.
FIXTURES = Path(__file__).parent / 'fixtures'
for source in json.loads((FIXTURES / 'vdex-burn-prevouts.json').read_text())['sources']:
    padded = [dict(n=n, valueSat=0, scriptPubKey={}) for n in range(max(o['n'] for o in source['vout']) + 1)]
    for out in source['vout']:
        padded[out['n']] = out
    TXS[source['txid']] = transaction(source['txid'], [], padded)
IMPORTED = json.loads((FIXTURES / 'vdex-identity-import.json').read_text())
IMPORT = IMPORTED['txid']
TXS[IMPORT] = transaction(IMPORT, IMPORTED['vin'], IMPORTED['vout'])


class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method, params = request['method'], request['params']
        self.server.calls.append((method, params))
        value, error = None, None
        if method == 'getinfo':
            value = dict(chainid=VDEX, name='vDEX', testnet=False, blocks=1, longestchain=1, connections=0)
        elif method == 'getblockcount':
            value = 1
        elif method == 'getblockchaininfo':
            value = dict(blocks=1, bestblockhash=BLOCK)
        elif method in ('getblock', 'getblockheader'):
            height = int(params[0]) if str(params[0]).isdigit() else 1
            value = dict(hash='0' * 64 if height == 0 else BLOCK, height=height,
                         previousblockhash='0' * 64 if height > 0 else None,
                         time=1700000000, tx=[], size=10, confirmations=1)
        elif method == 'getcurrency':
            name = {VDEX: 'vDEX', VRSC: 'VRSC', BRIDGE: 'Bridge.vDEX', PUMP: 'PUMP'}.get(params[0], params[0])
            value = dict(currencyid=params[0], name=name, fullyqualifiedname=name, systemid=VDEX)
        elif method == 'getidentity' and params[0] == VDEX:
            value = dict(identity=dict(identityaddress=VDEX, name='vDEX', version=3),
                         fullyqualifiedname='vDEX.VRSC@', blockheight=1,
                         txid='0' * 64, status='active')
        elif method == 'getidentity' and len(params) == 1 and params[0] in (OWNER, 'vARRR@'):
            value = dict(identity=IMPORTED['vout'][5]['scriptPubKey']['identityprimary'],
                         fullyqualifiedname='vARRR@',
                         blockheight=1, txid=IMPORT, vout=5, status='active')
        elif method == 'getidentity':
            error = dict(code=-5, message='Identity not found')
        elif method == 'getrawtransaction':
            value = TXS.get(params[0])
        elif method == 'getaddressbalance':
            value = dict(balance=0, received=10000)
        elif method == 'getaddresstxids':
            value = [BURN]
        elif method == 'getoffers':
            value = getattr(self.server, 'marketplace', {}) if params[0] in (VDEX, 'vDEX') and params[1] else []
        elif method == 'getaddressutxos':
            value = []
        elif method == 'getrawmempool':
            value = {}
        elif method != 'gettxout':
            error = dict(code=-32601, message='Method not found')
        body = json.dumps(dict(result=value, error=error, id=request['id'])).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass


def run():
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Daemon)
    server.calls = []
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-currency-names-') as directory:
            root = Path(directory)
            conf = root / 'vDEX.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal = Terminal(root, conf, chain='vDEX')
            try:
                deadline = time.monotonic() + 12
                while True:
                    screen = terminal.read(0.2)
                    if 'vDEX.VRSC@' in screen.splitlines()[0]:
                        break
                    assert time.monotonic() < deadline, screen
                terminal.key(b'\x1bOQ', 0.3)
                screen = terminal.read(0.3)
                assert 'Sources' in screen and 'vDEX.VRSC@' in screen, screen
                assert VDEX in screen, screen
                assert '53fe39eea8c06bba32f1a4e20db67e5524f0309d' in screen, screen
                assert 'Enabled' in screen and 'Monitor' in screen and 'Connection' in screen and 'Last OK' in screen, screen
                assert '+ Yes' in screen and ('Responding' in screen or 'Working' in screen), screen
                terminal.key(b'm', 0.3)
                screen = terminal.read(0.2)
                assert '~ Yes' in screen and 'Monitored — enabled' in screen, screen
                terminal.key(b'm', 0.3)

                terminal.key(b'\x1b', 0.3)
                terminal.key(b'/vARRR@\r', 1)
                deadline = time.monotonic() + 12
                while True:
                    screen = terminal.read(0.2)
                    assert 'API communication requires permission' not in screen, screen
                    if ('12.12500001 Bridge' in screen and '34.37500002 PUMP' in screen
                            and ' Consumption Burn ' in screen and 'Identity import transaction' in screen):
                        break
                    assert time.monotonic() < deadline, screen
                rows = screen.splitlines()
                evidence_note = next(i for i, row in enumerate(rows) if 'earlier versions may exist' in row)
                assert any('Transactions' in row for row in rows[evidence_note + 1:evidence_note + 3]), screen
                assert any(row.lstrip().startswith('Imported:') for row in rows), screen
                terminal.key(b'k\r', 1)
                screen = terminal.read(0.3)
                assert 'Transaction Overview' in screen and IMPORT in screen, screen
                terminal.key(('/' + BURN + '\r').encode(), 1)
                deadline = time.monotonic() + 12
                while True:
                    screen = terminal.read(0.2)
                    if ('Transaction Overview' in screen and ' Consumption Burn ' in screen
                            and screen.count('12.12500001 Bridge') >= 2
                            and screen.count('34.37500002 PUMP') >= 2):
                        break
                    assert time.monotonic() < deadline, screen
                rows = screen.splitlines()
                block_row = next(i for i, row in enumerate(rows) if 'Block:' in row)
                assert 'Expiry:' in rows[block_row + 1], screen
                block_number = rows[block_row].index('1', rows[block_row].index('Block:'))
                expiry_number = rows[block_row + 1].index('21', rows[block_row + 1].index('Expiry:')) + 1
                assert block_number == expiry_number, screen
                terminal.key(b'\x1b[Z', 0.3)
                assert 'y yanks value' in terminal.read(0.2), terminal.screen()
                terminal.key(b'\x1b[Z', 0.3)
                assert 'Transaction Overview' in terminal.read(0.2), terminal.screen()

                assert not any(method == 'getidentity' and params[0] in (BRIDGE, PUMP) for method, params in server.calls), server.calls
                assert not any(method.startswith('getidentitieswith') for method, _ in server.calls), server.calls
                terminal.close()
                terminal = Terminal(root, conf, chain='vDEX')
                terminal.key(b'/vARRR@\r', 1)
                deadline = time.monotonic() + 12
                while True:
                    screen = terminal.read(0.2)
                    assert 'API communication requires permission' not in screen, screen
                    if 'Identity import transaction' in screen and ' Consumption Burn ' in screen:
                        break
                    assert time.monotonic() < deadline, screen

                # A UI batch containing a synthetic bundle quote plus ordinary ID
                # offers exercises the actual sort keys and viewport reset.
                server.marketplace = json.loads((FIXTURES / 'marketplace-bundle.json').read_text())
                server.marketplace['id_offers'] = [dict(identityid=f'iMarket{n}', price=n + 1,
                    offer=dict(txid=f'{n + 1:064x}', offer=dict(identityid=f'iMarket{n}',
                        name='🐉Mushroom🍄' if n == 0 else f'Alpha{n:02d}'),
                        accept={VRSC:n + 1}, blockexpiry=100 + n)) for n in range(12)]
                for n in range(12):
                    txid = f'{n + 1:064x}'
                    TXS[txid] = transaction(txid, [], [])
                terminal.key(b'o', 1)
                deadline = time.monotonic() + 12
                while True:
                    screen = terminal.read(0.2)
                    if ('Marketplace' in screen and '13' in screen and 'Mushroom' in screen
                            and '🐉' in screen and '🍄' in screen):
                        break
                    assert time.monotonic() < deadline, screen
                terminal.key(b'aA\r', 0.5)
                screen = terminal.read(0.3)
                assert 'Transaction Overview' in screen and f'{1:064x}' in screen, screen
                terminal.key(b'h', 0.3)
                terminal.key(b'j' * 12, 0.5)
                screen = terminal.read(0.3)
                assert 'Ask 258 VRSC' in screen, screen
                assert 'price 0.00000039' not in screen, screen
                terminal.key(b's', 0.3)
                screen = terminal.read(0.3)
                assert 'Sort: name ascending' in screen and 'Open offers 1-' in screen, screen
                terminal.key(b'S', 0.3)
                screen = terminal.read(0.2)
                assert 'Sort: expiry ascending' in screen and 'Open offers 1-' in screen, screen
                terminal.key(b's', 0.3)

                terminal.key(b'j' * 8, 0.3)
                terminal.key(b'r', 0.3)
                screen = terminal.read(0.3)
                assert 'Sort: name descending' in screen and 'Open offers 1-' in screen, screen
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()


if __name__ == '__main__':
    run()
