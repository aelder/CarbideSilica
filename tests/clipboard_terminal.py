"""Yank through a real, isolated tmux with its default external clipboard policy."""
import base64
import fcntl
import http.server
import json
import os
from pathlib import Path
import pty
import re
import shlex
import struct
import subprocess
import tempfile
import termios
import threading

from bookmarks_terminal import Terminal
from charts_terminal import Daemon, block_hash, wait


def run():
    cli_env = os.environ.copy()
    cli_env.pop('TMUX', None)
    cli_env.pop('TMUX_PANE', None)
    cli_env['VERUS_EXPLORER_CLIPBOARD'] = 'osc52'
    test = subprocess.run(['target/debug/vcarbide','--clipboard-test','--conf','/no-such-config'],
                          env=cli_env,stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=5)
    assert test.returncode == 0, test.stderr.decode()
    assert b'\x1b]52;c;' in test.stdout and b'Paste into a local editor' in test.stdout
    cli_env['VERUS_EXPLORER_CLIPBOARD'] = 'off'
    test = subprocess.run(['target/debug/vcarbide','--clipboard-test'],env=cli_env,
                          stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=5)
    assert test.returncode != 0 and b'\x1b]52;c;' not in test.stdout
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Daemon)
    server.calls, server.delay, server.bad_height, server.orphan_height = [], 0, None, None
    threading.Thread(target=server.serve_forever, daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-clipboard-') as directory:
            root = Path(directory)
            socket = str(root / 'tmux.sock')
            command = ['tmux', '-S', socket, '-f', '/dev/null']
            env = os.environ.copy()
            env.pop('TMUX', None)
            env.pop('VERUS_EXPLORER_CLIPBOARD', None)
            env.update(TERM='xterm-256color', HOME=str(root), XDG_DATA_HOME=str(root/'data'),
                       XDG_CONFIG_HOME=str(root/'config'), XDG_CACHE_HOME=str(root/'cache'))
            conf = root / 'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            app = [str(Path('target/debug/vcarbide').resolve()), '--conf', str(conf),
                   '--emoji', 'off', '--poll-interval', '60', '--open', '1']
            terminal = None
            try:
                subprocess.run(command+['new-session','-d','-s','copy-test','-x','160','-y','40',shlex.join(app)],env=env,check=True)
                subprocess.run(command+['set-option','-s','set-clipboard','external'],env=env,check=True)
                subprocess.run(command+['set-option','-g','allow-passthrough','off'],env=env,check=True)
                terminal = Terminal.__new__(Terminal)
                terminal.master, slave = pty.openpty()
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack('HHHH',40,160,0,0))
                terminal.process = subprocess.Popen(command+['attach-session','-t','copy-test'],stdin=slave,stdout=slave,stderr=slave,env=env)
                os.close(slave)
                terminal.captured = bytearray()
                wait(terminal,'Height: 1 ')
                terminal.key(b'\t',.2)
                terminal.key(b'y',.8)
                wait(terminal,'via tmux clipboard forwarding; paste to verify')
                copies = re.findall(rb'\x1b\]52;[^;]*;([A-Za-z0-9+/=]+)(?:\x07|\x1b\\)',terminal.captured)
                assert copies, 'tmux external did not forward the application yank as clipboard OSC 52'
                copied = json.loads(base64.b64decode(copies[-1]))
                assert copied['hash'] == block_hash(1), copied
                assert b'Yanked' not in terminal.captured, 'Do not claim unverified desktop clipboard delivery'
                terminal.key(b'?y',.8)
                wait(terminal,'Sent test phrase')
                copies = re.findall(rb'\x1b\]52;[^;]*;([A-Za-z0-9+/=]+)(?:\x07|\x1b\\)',terminal.captured)
                assert base64.b64decode(copies[-1]) == b'Verus Explorer clipboard test'
                assert subprocess.check_output(command+['show-options','-sv','set-clipboard'],env=env).strip() == b'external'
                assert subprocess.check_output(command+['show-options','-gv','allow-passthrough'],env=env).strip() == b'off'
                count = len(copies)
                subprocess.run(command+['set-option','-s','set-clipboard','off'],env=env,check=True)
                terminal.key(b'y',.6)
                wait(terminal,'tmux clipboard forwarding is off')
                copies = re.findall(rb'\x1b\]52;[^;]*;([A-Za-z0-9+/=]+)(?:\x07|\x1b\\)',terminal.captured)
                assert len(copies) == count, 'Respect disabled clipboard policy'
            finally:
                if terminal is not None:
                    terminal.close()
                subprocess.run(command+['kill-server'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    finally:
        server.shutdown()
        server.server_close()
    print('Clipboard PTY check passed: application -> tmux external -> outer OSC 52 payload, without passthrough or settings changes.')


if __name__ == '__main__':
    run()
