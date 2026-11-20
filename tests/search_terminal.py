"""A local block-hash search must not ask for API access after the TX lookup misses."""
import http.server
import json
from pathlib import Path
import tempfile
import threading
from bookmarks_terminal import Terminal, VRSC

TIP = 'b' * 64
TARGET = 'c' * 64
IDENTITY = 'i4NpJp1vqrXgDvSNBXkNYvTR1VF2HMkeDA'
MISSING_TX = 'd' * 64
TRANSACTION = 'e' * 64
ADDRESS = 'RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp'
OTHER_CURRENCY = 'i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X'

class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method, params = request['method'], request['params']
        self.server.calls.append((method, params))
        value, error = None, None
        if method == 'getinfo':
            value = dict(chainid=VRSC, testnet=False, blocks=1, longestchain=1, connections=0)
        elif method == 'getblockcount':
            value = 1
        elif method == 'getblockchaininfo':
            value = dict(blocks=1, bestblockhash=TIP)
        elif method == 'getidentity' and len(params) == 1:
            name = 'Other' if params[0] == 'Other@' else 'Test'
            identity = OTHER_CURRENCY if name == 'Other' else IDENTITY
            value = dict(identity=dict(identityaddress=identity, name=name, version=3,
                flags=1, primaryaddresses=['RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp'],
                revocationauthority=identity, recoveryauthority=identity),
                fullyqualifiedname=name + '@', blockheight=1, txid=MISSING_TX, status='active')
        elif method == 'getidentity':
            error = dict(code=-1 if self.server.history_unavailable else -5,
                         message='Historical identity lookup unavailable' if self.server.history_unavailable else 'Identity not found')
        elif method == 'getrawtransaction' and params[0] == TRANSACTION:
            value = dict(txid=TRANSACTION, height=1, blockhash=TIP, blocktime=1700000000,
                confirmations=1, size=10, vin=[dict(txid=MISSING_TX, vout=0)],
                vout=[dict(n=0, value=1, scriptPubKey=dict(type='pubkeyhash', addresses=[ADDRESS],
                    reservetransfer=dict(version=1, flags=1, currencyvalues={OTHER_CURRENCY: 1},
                        destinationcurrencyid=OTHER_CURRENCY, destination=dict(type=2, address=ADDRESS)))),
                    dict(n=1, value=2, scriptPubKey=dict(type='pubkeyhash', addresses=['RAnotherDestination']))])
        elif method == 'getaddresstxids' and params[0]['addresses'] == [ADDRESS]:
            value = [TRANSACTION]
        elif method == 'getcurrency' and params[0] != OTHER_CURRENCY:
            value = dict(currencyid=params[0], name='VRSC', systemid=VRSC)
        elif method in ('getblock', 'getblockheader'):
            block = TARGET if params[0] == TARGET else TIP
            value = dict(hash=block, height=0 if block == TARGET else 1,
                         time=1700000000, tx=[], size=10, confirmations=1)
        else:
            error = dict(code=-5, message='No information available about transaction')
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
    server.history_unavailable = False
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-search-') as directory:
            root = Path(directory)
            conf = root / 'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal = Terminal(root, conf)
            try:
                terminal.key(b'/' + TARGET.encode() + b'\r', 1)
                screen = terminal.screen()
                assert 'API communication requires permission' not in screen, screen
                assert any(method == 'getblock' and params[0] == TARGET for method, params in server.calls), server.calls
                assert TARGET in screen, screen
                assert ('getrawtransaction', [TARGET, 1]) in server.calls, server.calls
                terminal.key(b'/Test@\r', 1)
                screen = terminal.screen()
                assert 'API communication requires permission' not in screen, screen
                assert 'Primary addresses:' in screen, screen
                assert ('getrawtransaction', [MISSING_TX, 1]) in server.calls, server.calls

                for query in [IDENTITY, TRANSACTION, ADDRESS, '1', VRSC + '@@']:
                    terminal.key(b'/' + query.encode() + b'\r', 1)
                    screen = terminal.read(0.3)
                    assert 'API communication requires permission' not in screen, (query, screen)
                server.history_unavailable = True
                # A current identity remains usable when older versions cannot load.
                terminal.key(b'/Other@\r', 1)
                screen = terminal.screen()
                assert 'API communication requires permission' not in screen, screen
                assert 'Primary addresses:' in screen, screen
                methods = {method for method, _ in server.calls}
                for method in ['getaddressbalance', 'getaddressutxos', 'getaddresstxids',
                               'getoffers', 'gettxout']:
                    assert method in methods, (method, server.calls)
                assert ('getcurrency', [OTHER_CURRENCY]) in server.calls, server.calls
                assert not any(method.startswith('getidentitieswith') for method, _ in server.calls), server.calls
                assert 'RPC error' not in screen and '-idindex' not in screen, screen
            finally:
                terminal.key(b'\x1b', 0.1)
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()

if __name__ == '__main__':
    import sys
    if '--forecast-only' not in sys.argv:
        run()
        from time_terminal import run as run_time_checks
        run_time_checks()
    from forecast_terminal import run as run_forecast_checks
    run_forecast_checks()
