"""Mouse navigation, value copying and native selection through a real PTY."""
import base64
import http.server
import json
import os
import re
from pathlib import Path
import tempfile
import threading
import time
from unittest.mock import patch
from bookmarks_terminal import Terminal, VRSC

def wait(terminal, needle):
    deadline=time.monotonic()+10
    while time.monotonic()<deadline:
        if needle in terminal.read(.1):
            return
    raise AssertionError(f'Missing {needle}:\n{terminal.screen()}')

IDENTITY='i4NpJp1vqrXgDvSNBXkNYvTR1VF2HMkeDA'
ADDRESS='RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp'
TXS=[symbol*64 for symbol in 'abcd']
UP=b'\x1b[<64;20;15M'
DOWN=b'\x1b[<65;20;15M'
CLIPBOARD = re.compile(rb'\x1b\]52;c;([A-Za-z0-9+/=]+)\x07')


def click(terminal, line_contains, value, copies=True):
    rows = terminal.screen().splitlines()
    row, line = next((index, text) for index, text in enumerate(rows) if line_contains in text)
    column = line.rindex(value) + 1
    before = len(terminal.captured)
    terminal.key(f'\x1b[<0;{column};{row + 1}M'.encode(), .15)
    if not copies:
        assert not CLIPBOARD.search(terminal.captured[before:])
        return None
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if copied := CLIPBOARD.search(terminal.captured[before:]):
            return base64.b64decode(copied[1]).decode()
        terminal.read(.05)
    raise AssertionError(f'No clipboard write for {value!r}:\n{terminal.screen()}')


def block_hash(height):
    return f'{height+1:064x}'


class Daemon(http.server.BaseHTTPRequestHandler):
    def log_message(self,*_):
        pass

    def do_POST(self):
        request=json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        method,params=request['method'],request['params']
        value,error=None,None
        if method=='getinfo':
            value=dict(chainid=VRSC,testnet=False,blocks=10,longestchain=10,connections=1)
        elif method=='getblockcount':
            value=10
        elif method=='getblockchaininfo':
            value=dict(blocks=10,bestblockhash=block_hash(10))
        elif method=='getblockhash':
            value=block_hash(int(params[0]))
        elif method in ('getblock','getblockheader'):
            height=int(params[0],16)-1 if len(str(params[0]))==64 else int(params[0])
            value=dict(hash=block_hash(height),height=height,time=1700000000+height*60,
                previousblockhash=block_hash(max(height-1,0)),confirmations=11-height,
                tx=[TXS[height-1]] if 1<=height<=4 else [],size=10,blocktype='mined')
            value.update({f'z{index:03}':index for index in range(100)})
        elif method=='getidentity' and len(params)==1:
            value=dict(identity=dict(identityaddress=IDENTITY,name='Wheel',version=3,flags=1,
                primaryaddresses=[ADDRESS],revocationauthority=IDENTITY,recoveryauthority=IDENTITY),
                fullyqualifiedname='Wheel@',blockheight=4,txid=TXS[-1],status='active')
        elif method=='getrawtransaction' and params[0] in TXS:
            height=TXS.index(params[0])+1
            value=dict(txid=params[0],height=height,blockhash=block_hash(height),
                blocktime=1700000000+height*60,confirmations=11-height,size=10,
                vin=[dict(coinbase='00')],vout=[dict(n=0,value=1,scriptPubKey=dict(
                    type='cryptocondition',addresses=[IDENTITY],spendableoutput=True))])
        elif method=='getaddresstxids':
            value=TXS
        elif method=='getaddressbalance':
            value=dict(balance=400000000,received=400000000)
        elif method=='getaddressutxos':
            value=[]
        elif method=='getoffers':
            value={}
        elif method=='getcurrency':
            value=dict(currencyid=params[0],name='VRSC',systemid=VRSC)
        else:
            error=dict(code=-5,message='mock unavailable')
        body=json.dumps(dict(result=value,error=error,id=request['id'])).encode()
        self.send_response(200);self.send_header('Content-Length',str(len(body)));self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError,ConnectionResetError):
            pass


def run():
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Daemon)
    threading.Thread(target=server.serve_forever,daemon=True).start()
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-mouse-') as directory:
            root=Path(directory);conf=root/'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            with patch.dict(os.environ, {'VERUS_EXPLORER_CLIPBOARD':'osc52'}):
                terminal=Terminal(root,conf)
            try:
                assert b'\x1b[?1006h' in terminal.captured,'Mouse reporting was not enabled'
                wait(terminal,'10/10')
                wait(terminal,'Recent Blocks 1-')
                terminal.key(DOWN,.2)  # Leave Stream Blocks for the newest block.
                terminal.key(DOWN,.2)
                terminal.key(b'\r',.4)
                wait(terminal,'Height: 9 ')
                terminal.key(b'\x1b',.2)
                terminal.key(UP+b'\r',.4)
                wait(terminal,'Height: 10 ')
                assert any(line.startswith(' Block Header') for line in terminal.screen().splitlines())
                assert any(line.startswith(' Hash: ') for line in terminal.screen().splitlines())
                assert click(terminal, 'Hash: ', block_hash(10)) == block_hash(10)
                click(terminal, 'Hash: ', 'Hash:', copies=False)
                timestamp = next(line.split('Timestamp: ')[1].strip() for line in terminal.screen().splitlines()
                                 if 'Timestamp: ' in line)
                assert click(terminal, 'Timestamp: ', timestamp) == timestamp
                footer = terminal.screen().splitlines()
                view_controls = next(index for index, line in enumerate(footer) if line.startswith('controls '))
                assert 'F2 sources' not in footer[view_controls]
                assert 'F2 sources' in footer[view_controls + 1] and 'F6 bookmarks' in footer[view_controls + 1]
                assert str(conf) not in terminal.screen()
                assert b'\x1b[?2004h' in terminal.captured, 'Bracketed paste not enabled'
                terminal.key(b'M', .2)
                wait(terminal, 'Mouse: select/paste')
                click(terminal, 'Hash: ', block_hash(10), copies=False)
                assert b'\x1b[?1000l' in terminal.captured, 'Button capture not released'
                assert b'\x1b[?1006l' in terminal.captured, 'SGR capture not released'
                # PRIMARY/middle-click paste is text from the terminal, not an app mouse action.
                terminal.key(b'/10', .2)  # A terminal without bracketed paste sends plain characters.
                assert ' / 10' in terminal.screen(), terminal.screen()
                assert 'Enter submit search' in terminal.screen() and 'F2 sources' not in terminal.screen()
                terminal.key(b'\x1b', .2)
                terminal.key(b'/\x1b[200~10\rM\x1b[201~', .2)
                assert ' / 10M' in terminal.screen(), terminal.screen()
                assert 'Mouse: select/paste' in terminal.screen(), terminal.screen()
                terminal.key(b'\x1b', .2)
                terminal.key(b'/', .2)
                terminal.key(b'\x1b[200~10\r\n\x1b[201~', .2)
                assert ' / 10' in terminal.screen(), terminal.screen()
                terminal.key(b'\r', .3)
                wait(terminal, 'Height: 10 ')
                terminal.key(b'M', .2)
                wait(terminal, 'Mouse: scroll/copy')
                terminal.key(b'\t\x1b[F',.3)  # JSON, End
                wait(terminal,'z099:')
                assert click(terminal, 'z098:', '98') == '98'
                assert next(line for line in terminal.screen().splitlines() if 'z098:' in line).startswith('>')
                assert click(terminal, 'z099:', '99') == '99'
                def json_positions():
                    rows=[(row,line[1:]) for row,line in enumerate(terminal.screen().splitlines())
                          if re.match(r'^[ >]\s+z\d{3}:',line)]
                    selected=[row for row,line in enumerate(terminal.screen().splitlines())
                              if re.match(r'^>\s+z\d{3}:',line)]
                    assert rows and len(selected)==1,terminal.screen()
                    return rows,selected[0]
                before,cursor=json_positions()
                for key in [b'k',b'\x1b[A',UP]:
                    terminal.key(key,.15)
                    after,next_cursor=json_positions()
                    assert after==before,'Text moved before the cursor reached the middle'
                    assert next_cursor==cursor-1,'JSON cursor did not move one row'
                    cursor=next_cursor
                terminal.key(DOWN,.15)
                after,next_cursor=json_positions()
                assert after==before and next_cursor==cursor+1
                terminal.key(b'\t',.15)
                terminal.key(b'/Wheel@\r',.8)
                wait(terminal,'Primary addresses:')
                assert click(terminal, ADDRESS, ADDRESS) == ADDRESS
                wait(terminal,'Transactions')
                terminal.key(b'\r',.4)
                wait(terminal,'Txid: '+TXS[-1])
                assert any(line.startswith(' Transaction Overview') for line in terminal.screen().splitlines())
                assert any(line.startswith(' Txid: ') for line in terminal.screen().splitlines())
                assert click(terminal, 'Txid: ', TXS[-1]) == TXS[-1]
                terminal.key(b'\x1b',.2)
                terminal.key(DOWN+b'\r',.4)
                wait(terminal,'Txid: '+TXS[-2])
                terminal.key(b'\x1b',.2)
                terminal.key(UP+b'\r',.4)
                wait(terminal,'Txid: '+TXS[-1])
                terminal.key(b'\x1b',.2)
                # Deliberately fast separate notches remain separate increments.
                terminal.key(DOWN+DOWN+b'\r',.4)
                wait(terminal,'Txid: '+TXS[-3])
                terminal.key(b'\x1bOQ', .25)
                wait(terminal, 'Config file: ' + str(conf))
                assert 'F6 bookmarks' not in terminal.screen()
                terminal.key(b'\x1b', .15)
                terminal.key(b'?', .15)
                terminal.key(b'\x1b[F', .2)
                wait(terminal, 'Cache usage')
                assert 'mainnet.sqlite3' in terminal.screen()
            finally:
                terminal.close()
                assert b'\x1b[?1006l' in terminal.captured,'Mouse reporting was not disabled'
                assert b'\x1b[?2004l' in terminal.captured,'Bracketed paste was not disabled'
    finally:
        server.shutdown();server.server_close()
    print('Mouse/JSON PTY checks passed: Overview and JSON clicks copy values, labels and capture-off clicks are inert, footer controls and paths follow the view, wheel navigation and native paste still work.')


if __name__=='__main__':
    run()
