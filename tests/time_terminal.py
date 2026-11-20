"""Time lookup through the real search interface, using isolated mock RPC only."""
import http.server
import tempfile
import threading
import time
from pathlib import Path

from bookmarks_terminal import Terminal
from charts_terminal import Daemon, block_hash, wait


def run():
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Daemon)
    server.calls, server.delay, server.bad_height, server.orphan_height = [], 0, None, None
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-time-') as directory:
            root = Path(directory)
            conf = root / 'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal = Terminal(root, conf)
            try:
                wait(terminal, 'Recent Blocks 1-')
                for query, height in [
                    ('time:1700150030', 2500),  # Halfway: prefer earlier time.
                    ('1700150060', 2501),  # Bare Unix seconds beyond the tip and genesis boundary.
                    ('1700000000', 0),  # The genesis timestamp is included in the Unix range.
                    ('2023-11-16', 1547),  # Midnight UTC, closest header.
                    ('time:Nov 16 2023 at 12:00 am UTC', 1547),
                    ('time:2023-11-16T02:00:00+02:00', 1547),
                    ('time:1701000000', 5000),  # Past date beyond this old fixture's tip.
                    ('time:-1', 0),
                ]:
                    terminal.key(b'/' + query.encode() + b'\r', .2)
                    quality = 'exact timestamp' if query in ('1700150060', '1700000000') else 'nearest of'
                    wait(terminal, quality)
                    wait(terminal, f'Height: {height} ')
                    wait(terminal, block_hash(height))
                    assert 'nearest of' in terminal.screen() or 'exact timestamp' in terminal.screen(), terminal.screen()
                    assert 'API communication requires permission' not in terminal.screen(), terminal.screen()
                # Numbers in the known block-height range still open blocks directly.
                terminal.key(b'/123\r', .2)
                wait(terminal, 'Height: 123 ')
                terminal.key(b'/1700150030000\r', .2)
                wait(terminal, 'Use Unix seconds, not milliseconds')
                count = len(server.calls)
                terminal.key(b'/time:11/16/2023\r', .2)
                wait(terminal, 'numeric dates require year first')
                assert len(server.calls) == count, 'Invalid dates must not trigger RPC'
                # A cold lookup can be cancelled without later replacing the screen.
                server.delay = .2
                terminal.key(b'/time:1700060000\r', .15)
                wait(terminal, 'Finding block near')
                terminal.key(b'\x1b', .2)
                count = sum(method == 'getblockheader' for method, _ in server.calls)
                terminal.read(.6)
                assert sum(method == 'getblockheader' for method, _ in server.calls) <= count + 1
                assert 'Height: 123 ' in terminal.screen(), terminal.screen()
                # A new navigation root also cancels in-flight time requests.
                terminal.key(b'/time:1700072000\r', .15)
                wait(terminal, 'Finding block near')
                terminal.key(b'd', .2)
                terminal.read(.5)
                assert 'Dashboard' in terminal.screen() and 'Block Header' not in terminal.screen(), terminal.screen()
                # Replace the observed tip during a cold lookup: refuse the result.
                server.delay = .02
                count = sum(method == 'getblockheader' for method, _ in server.calls)
                terminal.key(b'/time:1700042000\r', .1)
                deadline = time.monotonic() + 10
                while sum(method == 'getblockheader' for method, _ in server.calls) < count + 5:
                    assert time.monotonic() < deadline, terminal.screen()
                    terminal.read(.05)
                server.orphan_height = 5000
                wait(terminal, 'Chain or source changed during time lookup')
                assert 'Dashboard' in terminal.screen() and 'Height: 700 ' not in terminal.screen(), terminal.screen()
                assert not any(method == 'getblockhashes' for method, _ in server.calls), 'No timestamp index probe'
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()
    print('Time-search PTY checks passed: Unix/plain dates, UTC/offsets, numeric height preservation, boundaries, cancellation, reorg rejection and no timestamp-index/API consent probes.')


if __name__ == '__main__':
    run()
