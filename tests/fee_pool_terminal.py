"""F9 fee-pool sampling, dual currencies, current vs historical, cache and cancellation."""
import http.server
import json
from pathlib import Path
import tempfile
import threading
import time

from bookmarks_terminal import Terminal, VRSC, VDEX
from charts_terminal import header, block_hash, wait

VETH = 'i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X'


def coinbase_id(height):
    return f'{100000 + height:064x}'


class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method, params = request['method'], request['params']
        self.server.calls.append((method, params))
        native = self.server.native
        value = None
        if method == 'getinfo':
            value = dict(chainid=native, name='VRSC' if native == VRSC else 'vDEX',
                         notarychainid=VETH, testnet=False, blocks=5000,
                         longestchain=5000, connections=1, tiptime=1700300000)
        elif method == 'getblockcount':
            value = 5000
        elif method == 'getblockchaininfo':
            value = dict(blocks=5000, bestblockhash=block_hash(5000))
        elif method == 'getblockhash':
            height = int(params[0])
            value = block_hash(height + (1 if height == self.server.orphan_height else 0))
        elif method in ('getblockheader', 'getblock'):
            height = int(params[0], 16)-1 if len(str(params[0])) == 64 else int(params[0])
            value = header(height)
            value['tx'] = [coinbase_id(height)]
        elif method == 'getrawtransaction':
            height = int(params[0], 16)-100000
            if self.server.delay:
                time.sleep(self.server.delay)
            values = {native: 50 + height / 100000000}
            if native != VRSC:
                values[VRSC] = 2.25
            script = {} if height < self.server.activation else dict(feepool=dict(version=1, currencyvalues=values))
            value = dict(txid=params[0], blockhash=block_hash(height),
                         height=height, confirmations=5001-height,
                         vin=[dict(coinbase='00')],
                         vout=[dict(n=0, value=0, scriptPubKey=script)])
        elif method == 'getcurrency':
            names = {VRSC: 'VRSC', VDEX: 'vDEX', VETH: 'vETH'}
            value = dict(currencyid=params[0], name=names.get(params[0], 'unknown'),
                         systemid=params[0], launchsystemid=VRSC)
        body = json.dumps(dict(result=value, error=None if value is not None else
            dict(code=-1, message='mock unsupported'), id=request['id'])).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass


def run():
    for native in (VRSC, VDEX):
        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Daemon)
        server.native, server.calls = native, []
        server.delay, server.activation, server.orphan_height = 0, 0, None
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            with tempfile.TemporaryDirectory(prefix='explorer-fee-pool-') as directory:
                root = Path(directory)
                conf = root/'daemon.conf'
                conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
                terminal = Terminal(root, conf, chain='VRSC' if native == VRSC else 'vDEX')
                try:
                    wait(terminal, '5000')
                    terminal.key(b'\x1b[20~', .3)
                    wait(terminal, 'Observed active chain')
                    # Header-only metrics must not fetch fee-pool transactions.
                    assert not any(m == 'getrawtransaction' for m, _ in server.calls)
                    before = sum(m == 'getrawtransaction' for m, _ in server.calls)
                    terminal.key(b'5', .3)
                    wait(terminal, 'Fee pool balances')
                    wait(terminal, '100/100 sampled checkpoints')
                    screen = terminal.screen()
                    assert 'Current fee pool (observed #5000)' in screen, screen
                    assert 'vETH' not in screen, screen
                    assert '50.00005000 ' + ('VRSC' if native == VRSC else 'vDEX') in screen, screen
                    if native == VDEX:
                        assert '2.25000000 VRSC' in screen, screen
                    else:
                        assert '50.00005000 VRSC |' not in screen, screen
                    heights = [int(params[0], 16) - 100000 for m, params in server.calls
                               if m == 'getrawtransaction'][before:]
                    scheduled = [3560 + i * 1440 // 99 for i in range(100)]
                    # The current-balance read supplies the tip endpoint first.
                    assert heights[:3] == [5000, 3560, scheduled[49]], heights
                    assert set(heights[3:5]) == {scheduled[24], scheduled[74]}, heights
                    assert sorted(heights) == scheduled, heights
                    count = sum(m == 'getrawtransaction' for m, _ in server.calls)
                    terminal.key(b'z', .15)
                    wait(terminal, 'fit range')
                    terminal.key(b'z', .15)
                    wait(terminal, 'Log-like coins')
                    if native == VDEX:
                        terminal.key(b'v', .15)
                        wait(terminal, 'currency: native')
                        terminal.key(b'v', .15)
                        wait(terminal, 'currency: reserve')
                        terminal.key(b'v', .15)
                    assert sum(m == 'getrawtransaction' for m, _ in server.calls) == count
                    if native == VDEX:
                        terminal.key(b't', .2)
                        wait(terminal, '1441/1441 sampled checkpoints')
                        terminal.key(b't', .2)
                        wait(terminal, '100/100 sampled checkpoints')
                    terminal.key(b'\x1b[H', .2)
                    wait(terminal, 'Selected block 3560')
                    assert '50.00003560' in terminal.screen(), terminal.screen()
                    # Paging changes historical balances, not the current balance.
                    terminal.key(b'[', .3)
                    wait(terminal, 'heights 2119..3559')
                    wait(terminal, '100/100 sampled checkpoints')
                    assert 'Current fee pool (observed #5000)' in terminal.screen()
                    count = sum(m == 'getrawtransaction' for m, _ in server.calls)
                    terminal.key(b']', .3)
                    wait(terminal, 'heights 3560..5000')
                    wait(terminal, '100/100 sampled checkpoints')
                    assert sum(m == 'getrawtransaction' for m, _ in server.calls) <= count + 1
                    # Seek keeps the metric and includes the exact requested point.
                    terminal.key(b'gheight:4201\r', .3)
                    wait(terminal, 'Selected block 4201')
                    assert 'Fee pool balances' in terminal.screen()
                    wait(terminal, '50.00004201')
                    # No-pool historical blocks remain unknown, never fabricated zero.
                    server.activation = 2000
                    terminal.key(b'gheight:500\r', .3)
                    wait(terminal, 'No fee-pool checkpoints available')
                    assert 'Current fee pool (observed #5000)' in terminal.screen()
                    assert '0.00000000 VRSC' not in terminal.screen(), terminal.screen()
                    # Cancel uncached transaction reads by switching metrics.
                    server.activation = 0
                    server.delay = .3
                    terminal.key(b'gheight:1800\r', .05)
                    baseline = sum(m == 'getrawtransaction' for m, _ in server.calls)
                    deadline = time.monotonic() + 60
                    while sum(m == 'getrawtransaction' for m, _ in server.calls) == baseline:
                        terminal.read(.05)
                        assert time.monotonic() < deadline, terminal.screen()
                    terminal.key(b'1', .1)
                    count = sum(m == 'getrawtransaction' for m, _ in server.calls)
                    terminal.read(.8)
                    assert sum(m == 'getrawtransaction' for m, _ in server.calls) <= count + 1
                    assert 'Block intervals (seconds)' in terminal.screen(), terminal.screen()
                    assert 'Approve' not in terminal.screen(), terminal.screen()
                finally:
                    terminal.close()
        finally:
            server.shutdown()
            server.server_close()
    print('Fee-pool terminal checks passed (VRSC and PBaaS).')


if __name__ == '__main__':
    run()
