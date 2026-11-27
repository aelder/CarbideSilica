"""F9 header charts, history, block navigation and cancellation on mock RPC only."""
import http.server
import json
from pathlib import Path
import tempfile
import threading
import time
from bookmarks_terminal import Terminal
from synthetic_chain import VRSC, block_hash, header
from datetime import datetime, timezone

def wait(terminal, needle):
    deadline=time.monotonic()+60
    while time.monotonic()<deadline:
        if needle in terminal.read(.1):
            return
    raise AssertionError(f"Missing {needle}:\n{terminal.screen()}")


class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_POST(self):
        request = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method, params = request['method'], request['params']
        self.server.calls.append((method, params))
        value = None
        if method == 'getinfo':
            value = dict(chainid=VRSC, name='VRSC', testnet=False, blocks=5000,
                         longestchain=5000, connections=1, tiptime=1700300000)
        elif method == 'getblockcount':
            value = 5000
        elif method == 'getblockchaininfo':
            value = dict(blocks=5000, bestblockhash=block_hash(5000))
        elif method == 'getblockhash':
            value = block_hash(int(params[0]) + (1 if int(params[0]) == self.server.orphan_height else 0))
        elif method in ('getblockheader', 'getblock'):
            height = int(params[0], 16)-1 if len(str(params[0])) == 64 else int(params[0])
            if method == 'getblockheader' and self.server.delay:
                time.sleep(self.server.delay)
            value = header(height)
            if method == 'getblockheader' and height == self.server.bad_height:
                value['height'] += 1
        elif method == 'getcurrency':
            value = dict(currencyid=VRSC, name='VRSC', systemid=VRSC)
        body = json.dumps(dict(result=value, error=None if value is not None else
            dict(code=-1, message='mock unsupported'), id=request['id'])).encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError):
            pass  # A cancelled chart request must be allowed to disconnect.


def run():
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Daemon)
    server.calls, server.delay, server.bad_height, server.orphan_height = [], 0, None, None
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-charts-') as directory:
            root=Path(directory)
            conf=root/'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal=Terminal(root,conf)
            try:
                wait(terminal,'5000')
                terminal.key(b'\x1b[20~', .4)  # F9
                wait(terminal,'1441 headers')
                wait(terminal,'Observed active chain')
                assert 'heights 3560..5000' in terminal.screen(),terminal.screen()
                assert not any(m == 'getrawtransaction' for m, _ in server.calls),server.calls
                screen=terminal.screen()
                assert 'Span 1d' in screen,screen
                lines=screen.splitlines()
                assert next(i for i,line in enumerate(lines) if 'Block intervals (seconds)' in line)<next(i for i,line in enumerate(lines) if 'Charts  1 Intervals' in line),screen
                assert 'Start #3560' in screen and 'End   #5000' in screen,screen
                assert all(str(height) in screen for height in (3920,4280,4640)),screen
                for height in (3560,5000):
                    date=datetime.fromtimestamp(header(height)['time'],timezone.utc).isoformat().replace('+00:00','Z')
                    assert date in screen,screen
                terminal.key(b'2\x1b[Fj', .25)
                wait(terminal,'Selected block 4998')
                terminal.resize(42, 14)
                terminal.resize(160, 40)
                wait(terminal,'Selected block 4998')
                terminal.key(b'k', .25)
                wait(terminal,'Selected block 5000')
                terminal.key(b'\x1b[H', .25)
                wait(terminal,'Mining difficulty')
                wait(terminal,'Selected block 3560')
                wait(terminal,'MA100 3611.500 (50 samples)')
                wait(terminal,'Line: MA100')
                terminal.key(b'\r', .3)
                wait(terminal,block_hash(3560))
                assert 'Charts > Bk 3560' in terminal.screen(),terminal.screen()
                terminal.key(b'\x1b', .25)
                wait(terminal,'Selected block 3560')
                terminal.key(b'3\x1b[H', .25)
                wait(terminal,'Staking difficulty')
                wait(terminal,'Selected block 3561')
                terminal.key(b'\x1b[F', .2)
                wait(terminal,'Selected block 4999')
                assert 'PoS target' in terminal.screen(),terminal.screen()
                terminal.key(b'4\x1b[H', .3)
                wait(terminal,'PoW Ratio')
                wait(terminal,'Selected block 3560')
                wait(terminal,'100 blocks: 50 mined / 50 staked')
                wait(terminal,'1000 blocks: 500 mined / 500 staked')
                assert all(label in terminal.screen() for label in ('Mined','Staked','50/50','Time')),terminal.screen()
                terminal.key(b'\r', .3)
                wait(terminal,'Height: 3560 ')
                terminal.key(b'\x1b', .25)
                wait(terminal,'PoW Ratio')
                wait(terminal,'Selected block 3560')
                terminal.key(b'k', .2)
                wait(terminal,'Selected block 3561')
                terminal.key(b'3', .2)
                terminal.key(b'-', .3)
                wait(terminal,'Span 6h')
                wait(terminal,'361 headers')
                wait(terminal,'Retained observations')
                terminal.key(b'-', .3)
                wait(terminal,'Span 2h')
                wait(terminal,'121 headers')
                terminal.key(b'+', .3)
                wait(terminal,'Span 6h')
                wait(terminal,'361 headers')
                terminal.key(b'\x1b[5~', .4)  # PageUp: older range.
                wait(terminal,'heights 4279..4639')
                wait(terminal,'361 headers')
                header_calls=sum(m == 'getblockheader' for m, _ in server.calls)
                terminal.key(b'\x1b[6~', .4)  # PageDown: newer range.
                wait(terminal,'heights 4640..5000')
                wait(terminal,'361 headers')
                assert sum(m == 'getblockheader' for m, _ in server.calls) == header_calls
                terminal.key(b'\x1b[15~', .3)  # F5 qualifies cached headers again.
                wait(terminal,'Observed active chain')
                terminal.key(b'\x1b', .25)
                terminal.key(b'\x1b[20~', .3)
                wait(terminal,'Retained observations')
                assert 'Approve' not in terminal.screen(),terminal.screen()
                # Move to an older uncached window, then cancel the live request.
                server.delay=.3
                terminal.key(b'[', .05)
                time.sleep(.1)
                terminal.key(b'\x1b', .1)
                count=sum(m=='getblockheader' and int(params[0],16)<3560 for m,params in server.calls)
                terminal.read(.8)
                assert sum(m=='getblockheader' and int(params[0],16)<3560 for m,params in server.calls) <= count+1
                assert 'Charts  1 Intervals' not in terminal.screen(),terminal.screen()
                # Refreshing a malformed older header reports a gap, not a full window.
                server.delay=0
                server.bad_height=99
                terminal.key(b'/100\r', .3)
                wait(terminal,'Block Header')
                terminal.key(b'\x1b[20~', .4)
                wait(terminal,'Chart history unavailable')
                assert '101 headers' not in terminal.screen(),terminal.screen()
                server.bad_height=None
                server.orphan_height=100
                terminal.key(b'\x1b[15~', .4)
                wait(terminal,'101 headers')
                wait(terminal,'Orphan anchor; historical branch')
                assert 'Observed active chain' not in terminal.screen(),terminal.screen()
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()


def run_seeking():
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Daemon)
    server.calls, server.delay, server.bad_height, server.orphan_height = [], 0, None, None
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-chart-seek-') as directory:
            root = Path(directory)
            conf = root / 'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal = Terminal(root, conf)
            def seek(text, preview=None):
                terminal.key(b'g' + text.encode(), .15)
                wait(terminal, 'Go to block or date/time')
                if preview:
                    wait(terminal, preview)
                terminal.key(b'\r', .15)
            try:
                terminal.key(b'\x1b[20~', .3)
                wait(terminal, '1441 headers')
                terminal.key(b'--2', .3)
                wait(terminal, '121 headers')
                wait(terminal, 'Span 2h')
                terminal.key(b'g', .2)
                wait(terminal, 'Go to block or date/time')
                terminal.key(b'\x1b[17~', .2)  # F6 over the still-open seek prompt.
                wait(terminal, 'Bookmarks')
                terminal.key(b'\x1b', .2)
                assert 'Bookmarks' not in terminal.screen(), terminal.screen()
                wait(terminal, 'Go to block or date/time')
                terminal.key(b'q', .2)
                assert terminal.process.poll() is None, 'q in the seek input quit the application'
                terminal.key(b'\x1b', .2)
                wait(terminal, 'Span 2h')
                before = len(server.calls)
                seek('4998', 'Block 4998')
                wait(terminal, 'Selected block 4998')
                assert not any(method in ('getblockheader', 'getblockhash') for method, _ in server.calls[before:]), server.calls[before:]
                seek('2000', 'Block 2000')
                wait(terminal, 'heights 1940..2060')
                wait(terminal, 'Selected block 2000')
                wait(terminal, 'Span 2h')
                assert 'Mining difficulty' in terminal.screen() and 'Span 2h' in terminal.screen(), terminal.screen()
                terminal.key(b'\x1b[5~', .2)
                wait(terminal, 'heights 1819..1939')
                wait(terminal, 'Span 2h')
                terminal.key(b'\x1b[6~', .2)
                wait(terminal, 'heights 1940..2060')
                wait(terminal, 'Span 2h')
                # A loaded Unix timestamp moves selection without fetching headers.
                before = len(server.calls)
                seek(str(header(2002)['time']), 'Unix time')
                wait(terminal, 'Selected block 2002')
                wait(terminal, 'difference +0s')
                assert not any(method in ('getblockheader', 'getblockhash') for method, _ in server.calls[before:]), server.calls[before:]
                # Plain text dates are case-insensitive and route through the same parser.
                target = datetime.fromtimestamp(header(3000)['time'], timezone.utc)
                seek(target.strftime('%B %d %Y %H:%M:%S UTC'), 'Date/time')
                wait(terminal, 'Selected block 3000')
                wait(terminal, 'heights 2940..3060')
                terminal.key(b'\r', .25)
                wait(terminal, 'Height: 3000 ')
                terminal.key(b'\x1b', .2)
                wait(terminal, 'Selected block 3000')
                seek('2023-11-16', 'Date/time')  # No time means midnight UTC.
                wait(terminal, 'Selected block 1547')
                wait(terminal, '2023-11-16T00:00:00+00:00')
                # Ambiguous bare integers outside chain history explain both ranges.
                seek('6000')
                wait(terminal, 'Enter a height from 0 to 5000')
                terminal.key(b'\x15height:6000\r', .2)
                wait(terminal, 'No blocks available yet')
                terminal.key(b'\x1b', .2)
                assert 'Go to block or date/time' not in terminal.screen(), terminal.screen()
                seek('2099-01-01')
                wait(terminal, 'No blocks available yet')
                terminal.key(b'l', .2)
                wait(terminal, 'Selected block 5000')
                # Bad input stays editable. Explicit Unix zero selects genesis.
                seek('bad date')
                assert 'Go to block or date/time' in terminal.screen(), terminal.screen()
                terminal.key(b'\x15unix:0\r', .2)
                wait(terminal, 'Selected block 0')
                terminal.key(b'3', .2)
                seek('2001')
                wait(terminal, 'Selected block 2001')
                wait(terminal, 'Span 2h')
                assert 'Staking difficulty' in terminal.screen(), terminal.screen()
                terminal.key(b'4', .2)
                seek('4000')
                wait(terminal, 'Selected block 4000')
                wait(terminal, 'Span 2h')
                assert 'PoW Ratio' in terminal.screen(), terminal.screen()
                wait(terminal, '1000 blocks: 500 mined / 500 staked')
                seek('unix:0')
                wait(terminal, 'Selected block 0')
                wait(terminal, 'Span 2h')
                # Cancellation invalidates delayed lookup replies and preserves the chart.
                server.delay = .3
                seek('height:1000')
                terminal.key(b'\x1b', .05)
                terminal.read(.7)
                wait(terminal, 'Selected block 0')
                assert 'Go to block or date/time' not in terminal.screen(), terminal.screen()
                assert 'Approve' not in terminal.screen(), terminal.screen()
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()


if __name__ == '__main__':
    run()
    run_seeking()
