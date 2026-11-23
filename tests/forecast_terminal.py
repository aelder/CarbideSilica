"""Future estimation and lazy height dates in isolated PTYs and fake local RPC."""
import datetime
import http.server
import json
import tempfile
import threading
import time
from pathlib import Path
from bookmarks_terminal import Terminal, VRSC
from charts_terminal import wait

# Use the currency identity to exercise PBaaS namespace formatting.
STAKER = 'i6j1rzjgrDhSmUYiTtp21J8Msiudv5hgt9'
STAKER_NAME = 'Bridge.vDEX@'
AUTHORITY = 'i5v3h9FWVdRFbNHU7DfcpGykQjRaHtMqu7'
AUTHORITY_NAME = 'Verus Coin Foundation@'
TIP, START, TX, TX2, TX3 = 4_000_000, 2_549_920, 'a' * 64, 'b' * 64, 'c' * 64


def block_hash(height):
    return f'{height:064x}'


class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method, params = request['method'], request['params']
        self.server.calls.append((method, params))
        value, error = None, None
        timestamp = lambda h: self.server.now - (TIP - h) * 62
        if method == 'getinfo':
            value = dict(chainid=VRSC, testnet=False, blocks=TIP, longestchain=TIP,
                         connections=5, tiptime=self.server.now)
        elif method == 'getblockchaininfo':
            value = dict(blocks=TIP, headers=TIP, bestblockhash=block_hash(TIP),
                         initialblockdownload=self.server.catching_up)
        elif method == 'getblockcount':
            value = TIP
        elif method == 'getblockhash':
            height = params[0]
            if height == self.server.fail_height:
                error = dict(code=-1, message='Header unavailable')
            else:
                value = block_hash(height)
        elif method in ('getblock', 'getblockheader'):
            h = int(params[0], 16) if len(str(params[0])) == 64 else int(params[0])
            if h == self.server.delay_height:
                time.sleep(1)
            value = dict(hash=block_hash(h), height=h, previousblockhash=block_hash(max(0, h-1)),
                         time=timestamp(h), tx=[], size=10, confirmations=TIP-h+1,
                         validationtype='work')
            if h == TIP-100:
                value.update(validationtype='stake', postxddest=STAKER, posrewarddest=STAKER)
        elif method == 'getrawtransaction' and params[0] in (TX,TX2,TX3):
            value = dict(txid=params[0], height=TIP-20, blockhash=block_hash(TIP-20),
                         blocktime=timestamp(TIP-20), confirmations=21, size=10,
                         expiryheight=self.server.expiry, locktime=0,
                         vin=[dict(coinbase='00')], vout=[])
        elif method == 'getidentity' and len(params) == 1 and params[0] in (STAKER,AUTHORITY,STAKER_NAME):
            identity_address = STAKER if params[0] == STAKER_NAME else params[0]
            name = STAKER_NAME if identity_address == STAKER else AUTHORITY_NAME
            value = dict(identity=dict(identityaddress=identity_address, name=name.split('.')[0], version=3, flags=1,
                                      revocationauthority=AUTHORITY, recoveryauthority=AUTHORITY),
                         friendlyname=name, fullyqualifiedname=name[:-1]+'.VRSC@', blockheight=TIP-200)
        elif method == 'getcurrency':
            value = dict(currencyid=params[0], name='VRSC', systemid=VRSC)
        elif method in ('getrawmempool', 'getpeerinfo'):
            value = []
        else:
            error = dict(code=-1, message='Not supplied by fake daemon')
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
    server.now = int(time.time()) - 20
    server.calls, server.expiry = [], TIP+100
    server.fail_height, server.delay_height, server.catching_up = None, None, False
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-forecast-') as directory:
            root = Path(directory)
            conf = root / 'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal = Terminal(root, conf)
            try:
                wait(terminal, 'Recent Blocks 1-')
                terminal.key(f'/{TIP+200}\r'.encode(), .2)
                wait(terminal, 'Future block estimate')
                wait(terminal, '62.000000 seconds per block')
                assert f'Source: Local daemon — 127.0.0.1:{server.server_port}' in terminal.screen(),terminal.screen()
                assert 'local-daemon:' not in terminal.screen(),terminal.screen()
                assert f'#{START}' in terminal.screen(), terminal.screen()
                assert not any(m == 'getblock' and str(p[0]) == str(TIP+200) for m,p in server.calls)
                target_time = int(time.time()) + 120 * 62 + 30
                terminal.key(f'/{target_time}\r'.encode(), .2)
                wait(terminal, f'estimated height {TIP+120}')
                # F10 is a convenient entry point; relative input freezes on submission.
                terminal.key(b'\x1b[21~in 2 hours\r', .2)
                wait(terminal, f'estimated height {TIP+116}')
                terminal.key(f'/{TX}\r'.encode(), .2)
                wait(terminal, 'Transaction Overview')
                assert f'Expiry: {TIP+100}' in terminal.screen(), terminal.screen()
                terminal.key(b'D', .2)
                wait(terminal, 'Expiry: est.')
                actual = datetime.datetime.fromtimestamp(server.now-20*62,datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
                assert actual in terminal.screen(), terminal.screen()
                assert 'API communication requires permission' not in terminal.screen(), terminal.screen()
                # Display resolves data values; status/navigation stay numeric.
                terminal.key(b'\t', .2)
                wait(terminal, 'expiryheight')
                screen=terminal.screen()
                assert 'est.' in next(line for line in screen.splitlines() if 'expiryheight:' in line),screen
                for label in ['height:', 'blocktime:', 'confirmations:']:
                    assert actual in next(line for line in screen.splitlines() if line.lstrip(' \u00a0>▾▸').startswith(label)),screen
                assert f'tip {TIP}/' in screen.splitlines()[0],screen
                # Disabling the preview restores the exact raw scalar values.
                terminal.key(b'D', .2)
                screen=terminal.screen()
                assert str(TIP+100) in next(line for line in screen.splitlines() if 'expiryheight:' in line),screen
                assert str(server.now-20*62) in next(line for line in screen.splitlines() if 'blocktime:' in line),screen
                terminal.key(b'D', .2)
                terminal.key(b'\tD', .2)
                wait(terminal, f'Expiry: {TIP+100}')
                # Past height conversion must fetch its actual header, never extrapolate.
                terminal.key(f'/{TIP-100}\r'.encode(), .2)
                wait(terminal, f'Height: {TIP-100}')
                terminal.key(b'D', .2)
                date = datetime.datetime.fromtimestamp(server.now-100*62,datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
                wait(terminal, 'Height: '+date)
                terminal.key(b'\t', .2)
                screen=terminal.screen()
                for label in ['height:', 'time:', 'confirmations:']:
                    assert date in next(line for line in screen.splitlines() if line.lstrip(' \u00a0>▾▸').startswith(label)),screen
                assert f'tip {TIP}/' in screen.splitlines()[0],screen
                assert f'Bk {TIP-100}' in screen.splitlines()[1],screen
                assert date not in screen.splitlines()[1],screen
                wait(terminal, STAKER_NAME)
                destline=lambda: next(line for line in terminal.screen().splitlines() if 'postxddest:' in line)
                assert STAKER_NAME in destline(),terminal.screen()
                assert ('getidentity',[STAKER]) in server.calls,server.calls
                terminal.key(b'I', .2)
                assert STAKER in destline() and STAKER_NAME not in destline(),terminal.screen()
                terminal.key(b'i', .2)
                assert STAKER in destline(),terminal.screen()
                terminal.key(b'I', .2)
                assert STAKER_NAME in destline(),terminal.screen()
                terminal.key(b'/I', .2)
                assert ' / I' in terminal.screen(),terminal.screen()
                terminal.key(b'\x1b', .2)
                assert STAKER_NAME in destline(),terminal.screen()
                terminal.key(b'\t', .2)

                # Unavailable historical headers never become past estimates or RPC errors.
                server.expiry,server.fail_height=TIP-77,TIP-77
                terminal.key(f'/{TX3}\r'.encode(), .2)
                wait(terminal, f'{TIP-77} (date unavailable)')
                assert 'RPC error' not in terminal.screen(),terminal.screen()
                # A catching-up source must refuse a new forecast.
                server.catching_up=True
                terminal.key(f'/estimate:{TIP+200}\r'.encode(), .2)
                wait(terminal, 'Source is catching up')
                server.catching_up=False
                # Slow date hydration does not block quitting or keyboard navigation.
                server.expiry,server.delay_height=TIP-5000,TIP-5000
                terminal.key(f'/{TX2}\r'.encode(), .15)
                deadline=time.monotonic()+4
                while not any(m=='getblockheader' and p[0]==block_hash(TIP-5000) for m,p in server.calls):
                    assert time.monotonic()<deadline, terminal.screen()
                    terminal.read(.05)
                terminal.key(b'jD', .2)
                terminal.key(f'/{STAKER_NAME}\r'.encode(), .2)
                wait(terminal, 'Primary addresses:')
                terminal.key(b'\t', .2)
                wait(terminal, 'friendlyname:')
                wait(terminal, AUTHORITY_NAME)
                field=lambda name: next(line for line in terminal.screen().splitlines() if line.lstrip(' \u00a0>▾▸').startswith(name+':'))
                assert STAKER_NAME in field('friendlyname') and '.VRSC' not in field('friendlyname'),terminal.screen()
                assert STAKER_NAME[:-1]+'.VRSC@' in field('fullyqualifiedname'),terminal.screen()
                assert STAKER in field('identityaddress'),terminal.screen()
                assert AUTHORITY_NAME in field('revocationauthority'),terminal.screen()
                terminal.key(b'I', .2)
                assert STAKER in field('identityaddress'),terminal.screen()
                assert AUTHORITY in field('revocationauthority'),terminal.screen()
                assert STAKER_NAME in field('friendlyname') and '.VRSC' not in field('friendlyname'),terminal.screen()
                terminal.key(b'I', .2)
                assert AUTHORITY_NAME in field('revocationauthority'),terminal.screen()
                assert STAKER in field('identityaddress'),terminal.screen()
                assert terminal.process.poll() is None
                assert 'API communication requires permission' not in terminal.screen()
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()
    print('Forecast PTY checks passed: shared calibration, future height/time, F10, observed dates, future labels, raw JSON, block destination names, Shift-I, text entry and responsive cancellation.')


if __name__ == '__main__':
    run()
