"""Currency discovery, styled details, currency-ID entry and retained offline lists."""
import http.server
import json
from pathlib import Path
import tempfile
import threading
from bookmarks_terminal import Terminal, VRSC
from mouse_terminal import wait

BRIDGE = 'i6j1rzjgrDhSmUYiTtp21J8Msiudv5hgt9'
BOTS = 'i582ifsBK7SUN32rrXU5inHfXB5YJq438p'
PRELAUNCH = 'i3f7tSctFkiPpiedY8QR5Tep9p4qDVebDx'
NAMES = {VRSC: 'VRSC', BRIDGE: 'Bridge.vDEX', PRELAUNCH: 'Bridge.vETH', BOTS: 'Bots'}
STATE_FLAGS = {VRSC: 49, BRIDGE: 49, PRELAUNCH: 3, BOTS: 37}
BLOCK = 'b' * 64


def definition(currency):
    return dict(currencyid=currency, name=NAMES[currency],
                fullyqualifiedname=NAMES[currency],
                options={VRSC:256, BRIDGE:545, PRELAUNCH:33, BOTS:41}[currency], parent=VRSC,
                systemid=VRSC, launchsystemid=VRSC, startblock=1, endblock=0,
                initialsupply=100, currencies=[VRSC] if currency != VRSC else [], weights=[1])


class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method, params = request['method'], request['params']
        self.server.calls.append((method, params))
        value = None
        if method == 'getinfo':
            value = dict(chainid=VRSC, name='VRSC', testnet=False, blocks=10, longestchain=10, connections=1)
        elif method == 'getblockchaininfo':
            value = dict(blocks=10, bestblockhash=BLOCK)
        elif method == 'getblockcount':
            value = 10
        elif method in ('getblock', 'getblockheader'):
            value = dict(hash=BLOCK, height=10, time=1700000000, tx=[], confirmations=1, size=10)
        elif method == 'getblockhash':
            value = BLOCK
        elif method == 'listcurrencies':
            value = [dict(currencydefinition=definition(currency), bestheight=10,
                          bestcurrencystate=dict(currencyid=currency, supply=0 if currency == BOTS else 100, flags=STATE_FLAGS[currency]))
                     for currency in (VRSC, BRIDGE, PRELAUNCH, BOTS)]
        elif method == 'getcurrency':
            currency = params[0] if params[0] in NAMES else VRSC
            value = definition(currency)
            value.update(bestheight=10, bestcurrencystate=dict(currencyid=currency, supply=0 if currency == BOTS else 100, flags=STATE_FLAGS[currency],
                         reservecurrencies=[dict(currencyid=VRSC, reserves=12.5, weight=1, priceinreserve=0.125)]),
                         currencynames={VRSC: 'VRSC'})
        elif method == 'getidentity' and len(params) == 1:
            currency = BRIDGE if params[0] == BRIDGE else VRSC
            name = 'Bridge.vDEX@' if currency == BRIDGE else 'VRSC@'
            value = dict(identity=dict(identityaddress=currency, name=name[:-1], version=3,
                         flags=1, primaryaddresses=[], revocationauthority=currency, recoveryauthority=currency),
                         fullyqualifiedname=name, status='active', blockheight=0)
        elif method == 'getaddressbalance':
            value = dict(balance=0, received=0)
        elif method in ('getaddressutxos', 'getaddresstxids', 'getpeerinfo'):
            value = []
        elif method in ('getrawmempool', 'getoffers'):
            value = {}
        data = json.dumps(dict(result=value, error=None if value is not None else
                              dict(code=-5, message='mock unavailable'), id=request['id'])).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(data)))
        self.end_headers()
        try:
            self.wfile.write(data)
        except (BrokenPipeError, ConnectionResetError):
            pass


def run():
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Daemon)
    server.calls = []
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-currency-browser-') as directory:
            root = Path(directory)
            conf = root/'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal = Terminal(root, conf)
            try:
                terminal.key(b'c', .5)
                wait(terminal, 'Currencies  2 of 4')
                wait(terminal, 'List observation')
                wait(terminal, '1 reserve · VRSC')
                assert 'Liquidity pools (Show Fractional)' in terminal.screen(), terminal.screen()
                list_calls = sum(method == 'listcurrencies' for method, _ in server.calls)
                assert 'All except failed' in terminal.screen() and 'Bots' not in terminal.screen(), terminal.screen()
                terminal.key(b'F\t', .2)
                wait(terminal, 'Currency launch status')
                terminal.key(b'f', .2)
                wait(terminal, 'Matches 1 of 4')
                terminal.key(b'\x1b', .2)
                wait(terminal, 'Bots')
                assert 'Failed launch (refunding)' in terminal.screen(), terminal.screen()
                terminal.key(b'\r', .3)
                wait(terminal, 'Currency Bots')
                wait(terminal, 'Failed launch (refunding)')
                terminal.key(b'\x1b', .2)
                wait(terminal, 'Bots')
                terminal.key(b'F\t', .2)
                terminal.key(b'p', .2)
                wait(terminal, 'Matches 1 of 4')
                terminal.key(b'\x1b', .2)
                wait(terminal, 'Bridge.vETH')
                terminal.key(b'F\t', .2)
                terminal.key(b'l', .2)
                wait(terminal, 'Matches 1 of 4')
                terminal.key(b'\x1b', .2)
                wait(terminal, 'Bridge.vDEX')
                terminal.key(b'F\t', .2)
                terminal.key(b'a', .2)
                wait(terminal, 'Matches 3 of 4')
                terminal.key(b'd', .2)
                wait(terminal, 'Matches 2 of 4')
                terminal.key(b'\x1b', .2)
                assert sum(method == 'listcurrencies' for method, _ in server.calls) == list_calls
                terminal.key(b'F', .2)
                wait(terminal, 'Currency option flags')
                wait(terminal, '[+] Show')
                terminal.key(b' ', .2)
                wait(terminal, '[-] Hide')
                wait(terminal, 'Matches 1 of 4')
                terminal.key(b' ', .2)
                wait(terminal, 'Matches 3 of 4')
                terminal.key(b'p', .2)
                wait(terminal, 'Matches 2 of 4')
                terminal.key(b'a', .2)
                wait(terminal, 'Matches 3 of 4')
                terminal.key(b'\x1b', .2)
                wait(terminal, 'Currencies  3 of 4')
                assert sum(method == 'listcurrencies' for method, _ in server.calls) == list_calls
                assert 'Fractional' in terminal.screen() and 'Gateway converter' in terminal.screen(), terminal.screen()
                terminal.key(b'fBridge.vDEX\r', .2)
                wait(terminal, 'Currencies  1 of 4')
                terminal.key(b'\r', .4)
                wait(terminal, 'Currency Bridge.vDEX')
                assert 'Definition' in terminal.screen() and 'Basket currencies' in terminal.screen(), terminal.screen()
                assert 'Fractional' in terminal.screen() and 'Token' in terminal.screen(), terminal.screen()
                wait(terminal, 'Price in reserve')  # Definition-only list cache still loads detail state online.
                assert 'Best state' in terminal.screen(), terminal.screen()
                terminal.key(b'\x1b', .2)
                wait(terminal, 'Currencies  1 of 4')  # Filter/selection survive Back.
                terminal.key(b'f' + b'\x7f'*11 + b'VRSC\r', .2)
                wait(terminal, 'Currencies  1 of 4')
                terminal.key(b'\r', .3)
                wait(terminal, 'Currency VRSC')
                terminal.key(b'/VRSC@\r', .5)
                wait(terminal, 'Currency Defined')
                assert 'c opens currency' in terminal.screen(), terminal.screen()
                terminal.key(b'c', .3)
                wait(terminal, 'Currency VRSC')
                wait(terminal, 'c open identity')
                assert 'path' in terminal.screen() and 'ID VRSC' in terminal.screen(), terminal.screen()
                assert 'API communication requires permission' not in terminal.screen(), terminal.screen()
                terminal.key(b'\tc', .3)  # Currency JSON to the original identity.
                wait(terminal, 'Currency Defined')
                terminal.key(b'\tc', .3)  # Identity JSON to currency.
                wait(terminal, 'c open identity')
                terminal.key(b'c\x1b', .3)
                wait(terminal, 'Recent Blocks')  # Round trips do not add duplicate history.

                terminal.key(f'/{BRIDGE}@@\r'.encode(), .4)
                wait(terminal, 'Currency Bridge.vDEX')
                wait(terminal, 'c open identity')
                terminal.key(b'c', .4)
                wait(terminal, 'Currency Defined')
                assert ('getidentity', [BRIDGE]) in server.calls, server.calls
                terminal.key(b'c', .3)
                wait(terminal, 'Currency Bridge.vDEX')
            finally:
                terminal.key(b'\x1b', .2)
                terminal.close()

            terminal = Terminal(root, conf)
            try:
                terminal.key(b'\x1bOQ', .2)  # F2: disable this daemon without enabling API.
                wait(terminal, 'Sources')
                terminal.key(b'e', .2)
                terminal.key(b'\x1b', .2)
                count = sum(method == 'listcurrencies' for method, _ in server.calls)
                terminal.key(b'/currencies\r', .4)
                wait(terminal, 'Currencies  2 of 4')
                assert 'Liquidity pools (Show Fractional)' in terminal.screen(), terminal.screen()
                wait(terminal, 'Retained list observation')
                assert sum(method == 'listcurrencies' for method, _ in server.calls) == count
                terminal.key(b'fBridge.vDEX\r', .2)
                terminal.key(b'\r', .3)
                wait(terminal, 'Currency Bridge.vDEX')
                assert 'Fractional' in terminal.screen(), terminal.screen()
                assert 'API communication requires permission' not in terminal.screen(), terminal.screen()
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()
    print('Currency browser PTY checks passed: default liquidity pools without failed launches, launch-status filters/Bots detail and Back, tri-state option filters/presets without RPC, text filter/Back, configuration tags, state tables, identity link and retained offline navigation.')


if __name__ == '__main__':
    run()
