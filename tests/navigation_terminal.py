"""q exits outside input fields; Escape closes the visible layer and never exits."""
import http.server
import threading
import os
from pathlib import Path
import tempfile
from bookmarks_terminal import Terminal, VRSC, daemon
from defi_terminal import wait
from reorg_terminal import Daemon, A2, TXID

ESC = b'\x1b'
TX = b'/' + TXID.encode() + b'\r'


def dispose(terminal):
    if terminal.process.poll() is None:
        terminal.process.kill()
        terminal.process.wait(timeout=5)
    os.close(terminal.master)


def assert_quits(terminal):
    terminal.key(b'q', .1)
    terminal.process.wait(timeout=5)
    assert terminal.process.returncode == 0
    terminal.read(.1)
    assert b'\x1b[?1006l' in terminal.captured, 'Quit did not release terminal mouse mode'
    assert b'\x1b[?2004l' in terminal.captured, 'Quit did not release bracketed paste'
    assert b'\x1b[23;0t' in terminal.captured, 'Quit did not request title restoration'


def run_dashboard():
    server = daemon(VRSC)
    first, earlier, older = 'd' * 64, 'e' * 64, 'f' * 64
    tip_height = 14
    reported_tip = tip_height
    release_old_blocks = threading.Event()
    blocks = {
        height: dict(height=height, hash=f'{height + 1:064x}', time=1700000000 + height * 60,
                     tx=[], size=200)
        for height in range(tip_height + 3)
    }
    for height in range(1, tip_height + 3):
        blocks[height]['previousblockhash'] = blocks[height - 1]['hash']
    blocks[tip_height].update(tx=[earlier, first], size=850)
    blocks[tip_height - 1].update(tx=[older], size=500)
    by_hash = {block['hash']: block for block in blocks.values()}
    by_txid = {txid: block for block in blocks.values() for txid in block['tx']}

    def block(args):
        key = args[0]
        return by_hash[key] if key in by_hash else blocks[int(key)]

    def block_with_delayed_history(args):
        value = block(args)
        if value['height'] < 3:
            release_old_blocks.wait(timeout=10)
        return value

    def transaction(args):
        txid = args[0]
        block = by_txid[txid]
        return dict(txid=txid, vin=[dict(coinbase='00')], vout=[], size=105,
                    blockhash=block['hash'], height=block['height'], blocktime=block['time'])

    server.rpc_handlers = {
        'getinfo': lambda _: dict(chainid=VRSC, testnet=False, blocks=reported_tip, longestchain=reported_tip, connections=1),
        'getblockchaininfo': lambda _: dict(blocks=reported_tip, bestblockhash=blocks[reported_tip]['hash']),
        'getblockhash': lambda args: blocks[int(args[0])]['hash'],
        'getblock': block_with_delayed_history,
        'getblockheader': block,
        'getrawtransaction': transaction,
    }
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-dashboard-') as directory:
            root = Path(directory)
            conf = root / 'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\n')
            terminal = Terminal(root, conf, poll_interval=1)
            try:
                wait(terminal, 'ffffff...ffffff')
                screen = terminal.screen()
                heading = next(line for line in screen.splitlines() if 'Recent Blocks' in line)
                assert heading.index('Recent Blocks') < heading.index('Recent Transactions'), screen
                midpoint = len(heading) // 2
                assert heading.index('Recent Transactions') == midpoint + 2, screen
                assert heading[midpoint - 1:midpoint + 2] == '   ', screen
                tx_rows = [line for line in screen.splitlines() if 'txid ' in line]
                assert all(line[midpoint - 1:midpoint + 2] == '   ' for line in tx_rows), screen
                expected = ['dddddd...dddddd', 'eeeeee...eeeeee', 'ffffff...ffffff']
                assert len(tx_rows) == len(expected) and all(
                    short in line for short, line in zip(expected, tx_rows)
                ), screen
                assert '850 bytes' in screen and f'Block {tip_height}' in screen, screen
                terminal.key(b'\rl\x1b[C')
                screen = terminal.screen()
                assert '> Stream Blocks' in screen and 'Block Header' not in screen, screen
                reported_tip += 1
                wait(terminal, f'height {reported_tip}')
                lines = terminal.screen().splitlines()
                stream_row = next(index for index, line in enumerate(lines) if '> Stream Blocks' in line)
                assert f'height {reported_tip}' in lines[stream_row + 1], terminal.screen()
                terminal.key(b'\t')
                terminal.resize(65, 26)
                wait(terminal, 'Recent Transactions')
                assert 'Recent Blocks' not in terminal.screen()
                terminal.resize(160, 40)
                wait(terminal, 'Recent Blocks')
                terminal.key(b'\r')
                wait(terminal, 'Transaction Overview')
                wait(terminal, first)
                terminal.key(ESC)
                terminal.key(b'j\r')
                wait(terminal, 'Transaction Overview')
                wait(terminal, earlier)
                terminal.key(ESC)
                terminal.key(b'\x1b[Zj')
                selected_height = reported_tip
                reported_tip += 1
                wait(terminal, f'height {reported_tip}')
                terminal.key(b'\r')
                wait(terminal, 'Block Header')
                wait(terminal, f'Height: {selected_height}')
                terminal.key(ESC)
                terminal.key(b'\x1b[H')
                wait(terminal, '> Stream Blocks')
                terminal.key(b'\x1b[F')
                wait(terminal, 'loading older blocks...')
                frames = set()
                for _ in range(5):
                    screen = terminal.read(.13)
                    lines = screen.splitlines()
                    loading_row = next(index for index, line in enumerate(lines) if 'loading older blocks...' in line)
                    left_width = len(lines[loading_row]) // 2
                    assert loading_row > max(index for index, line in enumerate(lines) if 'height ' in line[:left_width])
                    frames.add(lines[loading_row].lstrip()[0])
                assert frames <= set('|/-\\') and len(frames) > 1, frames
                release_old_blocks.set()
                wait(terminal, 'genesis reached')
                screen = terminal.screen()
                assert 'loading older blocks...' not in screen, screen
                assert_quits(terminal)
            finally:
                dispose(terminal)
    finally:
        release_old_blocks.set()
        server.shutdown()
        server.server_close()


def run_mempool():
    server = daemon(VRSC)
    initial, newcomer = 'e' * 64, 'f' * 64
    entries = {initial: dict(size=450, fee=0.0002, time=1700000000, height=9, depends=[])}

    def transaction(txid):
        return dict(txid=txid, version=4, vin=[dict(coinbase='00')], vout=[], size=450)

    def mempool(params):
        return {
            txid: dict(entry, fulltx=transaction(txid)) if len(params) == 2 else entry
            for txid, entry in entries.copy().items()
        }

    server.rpc_handlers = {
        'getrawmempool': mempool,
        'getrawtransaction': lambda args: dict(transaction(args[0]), hex='aabbccddeeff'),
    }
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-mempool-') as directory:
            root = Path(directory)
            conf = root / 'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\n')
            terminal = Terminal(root, conf)
            try:
                wait(terminal, 'Recent Blocks')
                terminal.key(b'm')
                wait(terminal, 'Coinbase')
                wait(terminal, 'fee 0.00020000')
                bulk = ('getrawmempool', [True, {'fulltxes': True}])
                assert bulk in server.requests
                assert ('getrawtransaction', [initial, 1]) not in server.requests
                entries[newcomer] = dict(size=550, fee=0.0003, time=1700000001, height=9, depends=[initial])
                wait(terminal, '1 unconfirmed parent')
                assert ('getrawmempool', [True]) in server.requests
                assert 'Coinbase' in terminal.screen(), terminal.screen()
                assert ('getrawtransaction', [initial, 1]) not in server.requests
                assert server.requests.count(bulk) == 1
                terminal.key(b'\r')
                wait(terminal, 'Transaction Overview')
                assert ('getrawtransaction', [initial, 1]) in server.requests
                assert ('getrawtransaction', [newcomer, 1]) in server.requests
                terminal.key(b'\t')
                wait(terminal, 'aabbccddeeff')
                assert_quits(terminal)
            finally:
                dispose(terminal)
    finally:
        server.shutdown()
        server.server_close()


def run_peers():
    server = daemon(VRSC)
    address = '[2001:db8::1]:27486'
    bans = [dict(address='192.0.2.1/32')]

    def clear_bans(_):
        bans.clear()

    def ban(args):
        bans.append(dict(address=args[0]))

    server.rpc_handlers = {
        'getpeerinfo': lambda _: [dict(id=7, addr=address, version=170010, pingtime=0.012)],
        'listbanned': lambda _: list(bans),
        'getnettotals': lambda _: dict(totalbytesrecv=12345, totalbytessent=67890, timemillis=1700000000123),
        'addnode': lambda _: None,
        'clearbanned': clear_bans,
        'ping': lambda _: None,
        'disconnectnode': lambda _: None,
        'setban': ban,
    }
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-peer-controls-') as directory:
            root = Path(directory)
            conf = root / 'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\n')
            terminal = Terminal(root, conf)
            try:
                wait(terminal, 'Recent Blocks')
                assert 'getnettotals' not in server.calls
                terminal.key(b'`')
                wait(terminal, '1 connected · 1 banned')
                wait(terminal, 'Network totals snapshot')
                screen = terminal.screen()
                snapshot_border = next(line for line in screen.splitlines() if 'Network totals snapshot' in line)
                assert snapshot_border.index('┐') - snapshot_border.index('┌') + 1 == 52, screen
                wait(terminal, '12345 bytes')
                wait(terminal, '67890 bytes')
                terminal.key(b'a')
                wait(terminal, 'Add node')
                terminal.key(b'\r')
                wait(terminal, 'Enter one node address')
                terminal.key(b'\x1b[200~quartz.example:27486\x1b[201~\r')
                wait(terminal, "Added quartz.example:27486 to the daemon's node list")
                assert ('addnode', ['quartz.example:27486', 'add']) in server.requests
                terminal.key(b'B')
                assert 'Clear all bans on this daemon?' not in terminal.screen()
                terminal.key(b'c')
                wait(terminal, 'Clear all bans on this daemon?')
                terminal.key(ESC)
                assert 'clearbanned' not in server.calls
                terminal.key(b'cy')
                wait(terminal, '1 connected · 0 banned')
                wait(terminal, 'p ping all')
                terminal.key(b'p')
                wait(terminal, 'Ping queued for all connected peers')
                assert ('ping', []) in server.requests
                terminal.key(b'\rp')
                wait(terminal, 'Protocol version:')
                wait(terminal, 'Traffic and latency')
                wait(terminal, 'Last ping:')
                wait(terminal, '12ms')
                log = root / 'debug.log'
                wait(terminal, f'Could not read {log}')
                wait(terminal, 'No such file or directory')
                log.write_text('unrelated log entry\n')
                wait(terminal, 'No recent debug.log lines contain')
                with log.open('a') as handle:
                    handle.write(f'connected to {address}\n')
                wait(terminal, f'connected to {address}')
                log.unlink()
                wait(terminal, 'Previously read entries')
                wait(terminal, f'Could not read {log}')
                assert 'p ping all' not in terminal.screen(), terminal.screen()
                assert server.calls.count('ping') == 1, server.calls
                terminal.key(b'x')
                wait(terminal, f'Disconnect {address}?')
                terminal.key(b'y')
                wait(terminal, 'Disconnect requested')
                assert ('disconnectnode', [address]) in server.requests
                terminal.key(b'\tb')
                wait(terminal, "Ban 2001:db8::1 for the daemon's default duration?")
                terminal.key(b'yh')
                wait(terminal, '1 connected · 1 banned')
                assert ('setban', ['2001:db8::1', 'add']) in server.requests
                assert server.calls.count('getnettotals') == 1, server.calls
                assert_quits(terminal)
            finally:
                dispose(terminal)
    finally:
        server.shutdown()
        server.server_close()


def run():
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Daemon)
    server.tip, server.missing, server.flip_after_tip = A2['hash'], None, None
    server.calls = []
    threading.Thread(target=server.serve_forever, daemon=True).start()
    cases = [
        ('dashboard', []),
        ('sources', [(b'\x1bOQ', 'Sources')]),
        ('source activity', [(b'\x1bOQ', 'Sources'), (b'a', 'Source activity')]),
        ('relationships', [(TX, 'Transaction Overview'), (b'\x1bOR', 'Relationships')]),
        ('bookmarks', [(b'\x1b[17~', 'Bookmarks')]),
        ('help', [(b'\x1bOP', 'Verus Carbide help')]),
        ('peers', [(b'`', 'Peers')]),
        ('option filters', [(b'cF', 'Currency option flags')]),
        ('launch filters', [(b'cF\t', 'Currency launch status')]),
        ('cache confirmation', [(b'?', 'Verus Carbide help'), (b'c', 'Clear Cache')]),
        ('witnesses', [(TX, 'Transaction Overview'), (b'\x1bOS', 'Witness investigation')]),
        ('inclusions', [(TX, 'Transaction Overview'), (b'\x1b[18~', 'Transaction inclusion history')]),
        ('DeFi', [(TX, 'Transaction Overview'), (b'\x1b[19~', 'DeFi trail')]),
    ]
    try:
        for label, steps in cases:
            with tempfile.TemporaryDirectory(prefix='explorer-quit-') as directory:
                root = Path(directory)
                conf = root/'VRSC.conf'
                conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
                terminal = Terminal(root, conf)
                try:
                    wait(terminal, 'tip 2/2')
                    assert 'Verus Carbide' in terminal.screen().splitlines()[0]
                    assert b'\x1b[22;0t\x1b]0;Verus Carbide\x07' in terminal.captured
                    for keys, needle in steps:
                        terminal.key(keys, .25)
                        wait(terminal, needle)
                    for width, height in [(76, 24), (44, 13), (12, 4), (160, 40)]:
                        terminal.resize(width, height)
                        assert terminal.process.poll() is None, f'{label} exited at {width}x{height}'
                    wait(terminal, steps[-1][1] if steps else 'Recent Blocks')
                    assert_quits(terminal)
                except Exception as error:
                    raise AssertionError(f'q failed in {label}:\n{terminal.screen()}') from error
                finally:
                    dispose(terminal)

        with tempfile.TemporaryDirectory(prefix='explorer-escape-') as directory:
            root = Path(directory)
            conf = root/'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal = Terminal(root, conf)
            try:
                wait(terminal, 'Recent Blocks')
                for _ in range(3):
                    terminal.key(ESC, .15)
                    assert terminal.process.poll() is None, 'Escape at Dashboard exited'
                terminal.key(b'/1\r', .3)
                wait(terminal, 'Height: 1 ')
                terminal.key(b'\t', .15)
                terminal.key(ESC, .2)
                wait(terminal, 'Recent Blocks')
                assert terminal.process.poll() is None, 'Escape at a JSON navigation root exited'

                terminal.key(b'/q', .2)
                assert ' / q' in terminal.screen(), terminal.screen()
                assert terminal.process.poll() is None
                terminal.key(ESC, .2)

                terminal.key(b'cF', .2)
                wait(terminal, 'Currency option flags')
                terminal.key(ESC, .2)
                assert 'Currency option flags' not in terminal.screen(), terminal.screen()
                assert 'Currencies' in terminal.screen(), terminal.screen()
                terminal.key(b'fq', .15)
                assert 'Filter: q_' in terminal.screen(), terminal.screen()
                terminal.key(ESC, .2)
                assert terminal.process.poll() is None
                terminal.key(ESC, .2)
                wait(terminal, 'Recent Blocks')

                terminal.key(b'/' + VRSC.encode() + b'@@\r', .4)
                wait(terminal, 'Currency VRSC')
                terminal.key(b'gq', .2)
                wait(terminal, 'Historical height / date:')
                assert terminal.process.poll() is None, 'q in numeric entry exited'
                terminal.key(ESC, .2)
                assert 'Historical height / date:' not in terminal.screen(), terminal.screen()
                assert 'Currency VRSC' in terminal.screen(), terminal.screen()
                terminal.key(b'/1\r', .2)
                wait(terminal, 'Height: 1 ')
                terminal.key(b'\ts', .2)
                wait(terminal, 'Save JSON')
                terminal.key(b'q', .2)
                assert 'filename q.json' in terminal.screen(), terminal.screen()
                assert terminal.process.poll() is None
                terminal.key(ESC, .2)
                terminal.key(b'\x1b[17~', .2)
                wait(terminal, 'Bookmarks')
                terminal.key(b'nq', .2)
                wait(terminal, 'Add bookmark')
                assert terminal.process.poll() is None
                terminal.key(ESC, .2)
                assert 'Bookmarks' in terminal.screen(), terminal.screen()
                terminal.key(ESC, .2)
                assert 'Bookmarks' not in terminal.screen(), terminal.screen()
                assert_quits(terminal)
            finally:
                dispose(terminal)
    finally:
        server.shutdown()
        server.server_close()
    print('Navigation PTY checks passed: q exits 13 interface contexts, input q stays text, Escape closes one layer and keeps roots open, terminal modes restored.')


if __name__ == '__main__':
    import sys
    if '--mempool-only' in sys.argv:
        run_mempool()
    elif '--peers-only' in sys.argv:
        run_peers()
    elif '--dashboard-only' in sys.argv:
        run_dashboard()
    else:
        run()
        run_peers()
        run_mempool()
        run_dashboard()
