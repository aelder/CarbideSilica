"""Synthetic daemon-builder branches with exact historical conversion rates."""
import copy
import json
from pathlib import Path
import tempfile
from bookmarks_terminal import Terminal, VRSC, VDEX
from defi_terminal import (SOURCE, EXPORT, IMPORT, BLOCK, CURRENCY, ADDRESS,
                           TRANSFER, IMPORT_INFO, TXS, daemon, wait, run_gateway_route)


def run_refund(gateway=False, wrong_destination=False):
    transfer=dict(TRANSFER, version=1, flags=2051, convert=True, refund=True,
                  destination=dict(type=2,address=ADDRESS))
    if gateway:
        transfer['destination']=dict(type=201,address='0x05c6b3a4ffda0dfe3daad775b265c3fa9cdb7209',
            gateway='i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X',fees=5.3276505,
            auxdests=[dict(type=2,address=ADDRESS)])
    txs=copy.deepcopy(TXS)
    txs[SOURCE]['vout'][0]['scriptPubKey']['reservetransfer']=transfer
    txs[IMPORT]['vout'][1].update(valueSat=100000000)
    txs[IMPORT]['vout'][1]['scriptPubKey']['hex']='76a9144f01c06114c899baab0522499ead597bf883e78988ac'
    if wrong_destination:
        txs[IMPORT]['vout'][1]['scriptPubKey']['hex']='76a914000000000000000000000000000000000000000088ac'
    record=dict(importheight=1,importtxid=IMPORT,importvout=0,
        importnotarization=dict(launchcomplete=True,launchconfirmed=True,
            currencystate=dict(version=1,flags=49,currencyid=CURRENCY)),
        transfers=[transfer],**{'import':IMPORT_INFO})
    server=daemon(VRSC,1,txs,1,record)
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-defi-refund-') as directory:
            root=Path(directory);conf=root/'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal=Terminal(root,conf)
            try:
                terminal.key(b'/'+SOURCE.encode()+b'\r',0.6)
                wait(terminal,SOURCE)
                terminal.key(b'\x1b[19~',0.2)
                terminal.key(b'\r',0.4)
                if wrong_destination:
                    wait(terminal,'Imported; receipt unresolved')
                    assert 'Refunded output verified' not in terminal.screen(),terminal.screen()
                    return
                wait(terminal,'Replayed refund result')
                screen=terminal.screen()
                assert 'Refunded output verified' in screen and 'Refunded' in screen,screen
                assert 'Execution rate' not in screen and 'Conversion fee' not in screen,screen
                assert 'Replayed conversion result' not in screen,screen
            finally:
                terminal.close()
    finally:
        server.shutdown();server.server_close()


def run_pool_sell(pending=False):
    transfer = dict(TRANSFER, version=1, flags=515, convert=True, importtosource=True,
                    currencyvalues={CURRENCY: 50}, destinationcurrencyid=VRSC,
                    destination=dict(type=2, address=ADDRESS))
    txs = copy.deepcopy(TXS)
    txs[SOURCE]['vout'][0]['scriptPubKey']['reservetransfer'] = transfer
    if pending:
        for key in ('spentTxId', 'spentIndex', 'spentHeight'):
            txs[SOURCE]['vout'][0].pop(key, None)
    txs[IMPORT]['vout'][1].update(value=99.975, valueSat=9997500000)
    txs[IMPORT]['vout'][1]['scriptPubKey']['hex'] = '76a9144f01c06114c899baab0522499ead597bf883e78988ac'
    record = dict(importheight=1, importtxid=IMPORT, importvout=0,
                  importnotarization=dict(launchcomplete=True, launchconfirmed=True,
                      currencystate=dict(version=1, flags=49, currencyid=CURRENCY,
                          currencies={VRSC: dict(lastconversionprice=2)})),
                  transfers=[transfer], **{'import': IMPORT_INFO})
    server = daemon(VRSC, 1, txs, 1, record)
    server.requests = []
    server.pending_currency = CURRENCY
    server.pending_records = [dict(txid=SOURCE, n=0, reservetransfer=transfer)]
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-pool-sell-') as directory:
            root = Path(directory)
            conf = root / 'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal = Terminal(root, conf)
            try:
                terminal.key(b'/' + SOURCE.encode() + b'\r', 0.6)
                wait(terminal, SOURCE)
                terminal.key(b'\x1b[19~', 0.2)
                terminal.key(b'\r', 0.4)
                if pending:
                    wait(terminal, 'Pending export')
                    queries = [params for method, params in server.requests
                               if method == 'getpendingtransfers']
                    assert queries == [[CURRENCY]], queries
                    assert 'getimports' not in server.calls, server.calls
                else:
                    wait(terminal, 'Replayed conversion result')
                    screen = terminal.screen()
                    assert 'Received output verified' in screen, screen
                    assert '99.975' in screen and 'Conversion fee' in screen, screen
                    assert 'Bridge.vETH' in screen, screen
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()


def run_captured_pool_sell():
    fixture = json.loads((Path(__file__).parent / 'fixtures/vrsc-pool-sell.json').read_text())
    record = fixture['record']
    source = fixture['source_transaction']
    txs = {tx['txid']: tx for tx in (source, fixture['export_transaction'], fixture['transaction'])}
    server = daemon(VRSC, record['importheight'], txs, record['importheight'], record)
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-captured-pool-sell-') as directory:
            root = Path(directory)
            conf = root / 'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal = Terminal(root, conf)
            try:
                terminal.key(b'/' + source['txid'].encode() + b'\r', 0.6)
                wait(terminal, source['txid'])
                terminal.key(b'\x1b[19~', 0.2)
                terminal.key(b'\r', 0.4)
                wait(terminal, 'Replayed conversion result')
                screen = terminal.screen()
                assert 'Received output verified' in screen, screen
                assert '5078.95612795' in screen, screen
                assert record['importtxid'] in screen, screen
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()


def run_preconversion(fractional=False, altered=False):
    # Source-derived builder cases; these are not captured successful launches.
    transfer = dict(TRANSFER, version=1, flags=7, convert=True, preconvert=True,
                    currencyvalues={VRSC: 100}, destination=dict(type=2, address=ADDRESS))
    txs = copy.deepcopy(TXS)
    source = txs[SOURCE]['vout'][0]
    source['scriptPubKey']['reservetransfer'] = transfer
    export = txs[EXPORT]['vout'][0]['scriptPubKey']['crosschainexport']
    export.update(version=1, flags=1)
    info = dict(IMPORT_INFO, version=1, flags=8, numoutputs=1 if fractional else 2)
    principal = dict(n=2 if not fractional else 1, value=0, valueSat=0,
                     scriptPubKey=dict(addresses=[ADDRESS], reserveoutput=dict(
                         version=1, currencyvalues={CURRENCY: 49.9875})))
    # Currency identity's standard V3 one-of-one native script, computed from
    # the currency i-address rather than a hard-coded arbitrary identity name.
    alphabet = '123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
    number = 0
    for ch in CURRENCY:
        number = number * 58 + alphabet.index(ch)
    payload = number.to_bytes(25, 'big')[1:21].hex()
    proceeds = dict(n=1, value=99.975, valueSat=9997500001 if altered else 9997500000,
                    scriptPubKey=dict(type='cryptocondition', evalnone='valid', reqSigs=1,
                        spendableoutput=True, addresses=[CURRENCY],
                        hex='050403000000cc1b04030001011504' + payload + '75'))
    txs[IMPORT]['vout'] = [dict(n=0,value=0,valueSat=0,
        scriptPubKey=dict(crosschainimport=info))] + ([] if fractional else [proceeds]) + [principal]
    state = dict(version=2, currencyid=CURRENCY, prelaunch=True,
                 currencystate=dict(version=1, flags=3 if fractional else 2, currencyid=CURRENCY,
                                    currencies={VRSC: dict(lastconversionprice=2)}))
    record = dict(importheight=1,importtxid=IMPORT,importvout=0,
                  importnotarization=state,transfers=[transfer], **{'import':info})
    server = daemon(VRSC,1,txs,1,record)
    server.currency_definitions = {CURRENCY: dict(version=1,currencyid=CURRENCY,
        name='Bridge.vETH',systemid=VRSC,launchsystemid=VRSC,
        options=33 if fractional else 32,currencies=[VRSC])}
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-preconversion-') as directory:
            root=Path(directory);conf=root/'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal=Terminal(root,conf)
            try:
                terminal.key(b'/'+SOURCE.encode()+b'\r',0.6)
                wait(terminal,'Txid: '+SOURCE)
                terminal.key(b'\x1b[19~',0.2)
                terminal.key(b'\r',0.6)
                if altered:
                    wait(terminal,'Imported; receipt unresolved')
                    assert 'Received output verified' not in terminal.screen(),terminal.screen()
                else:
                    wait(terminal,'Received output verified')
                    screen=terminal.screen()
                    assert '49.9875' in screen and 'Conversion fee' in screen,screen
                    if not fractional:
                        assert 'Currency proceeds' in screen and '99.975' in screen,screen
                    assert 'launch success' not in screen.lower() or 'does not establish' in screen,screen
            finally:
                terminal.close()
    finally:
        server.shutdown();server.server_close()


def run_launch_allocation(altered=False):
    txs=copy.deepcopy(TXS)
    export=txs[EXPORT]['vout'][0]['scriptPubKey']['crosschainexport']
    export.update(version=1,flags=3,numinputs=0)
    info=dict(IMPORT_INFO,version=1,flags=14,numoutputs=2)
    alphabet='123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
    number=0
    for ch in VRSC:
        number=number*58+alphabet.index(ch)
    payload=number.to_bytes(25,'big')[1:21].hex()
    allocation=dict(n=1,value=0,valueSat=0,scriptPubKey=dict(type='cryptocondition',reqSigs=1,
        spendableoutput=True,addresses=[VRSC],hex='1b04030001011504'+payload+'cc',
        reserveoutput=dict(version=1,currencyvalues={CURRENCY:12.00000001 if altered else 12})))
    txs[IMPORT]['vout']=[dict(n=0,value=0,valueSat=0,scriptPubKey=dict(crosschainimport=info)),
        allocation,dict(n=2,value=0.0001,valueSat=10000,scriptPubKey=dict(addresses=[ADDRESS]))]
    record=dict(importheight=1,importtxid=IMPORT,importvout=0,transfers=[],
        importnotarization=dict(version=2,currencyid=CURRENCY,launchcleared=True,launchconfirmed=True,
            currencystate=dict(version=1,flags=56,currencyid=CURRENCY)),**{'import':info})
    server=daemon(VRSC,1,txs,1,record)
    server.currency_definitions={CURRENCY:dict(version=1,currencyid=CURRENCY,name='Bridge.vETH',
        systemid=VRSC,launchsystemid=VRSC,options=32,preallocations=[{VRSC:12}])}
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-launch-allocation-') as directory:
            root=Path(directory);conf=root/'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal=Terminal(root,conf)
            try:
                terminal.key(b'/'+IMPORT.encode()+b'\r',0.6)
                wait(terminal,'Txid: '+IMPORT)
                terminal.key(b'\x1b[19~',0.2)
                wait(terminal,'Currency import')
                terminal.key(b'\r',0.6)
                wait(terminal,'Generated output (unattributed)' if altered else 'Verified launch preallocation')
                if altered:
                    assert 'Verified launch preallocation' not in terminal.screen(),terminal.screen()
                else:
                    assert '1200000000 satoshis' in terminal.screen(),terminal.screen()
                assert EXPORT in terminal.screen(),terminal.screen()
            finally:
                terminal.close()
    finally:
        server.shutdown();server.server_close()


def run_local_gateway_fallback():
    txs=copy.deepcopy(TXS)
    transfer=dict(TRANSFER,version=1,flags=1,feecurrencyid=VDEX,
        destination=dict(type=130,address=ADDRESS,gateway=VDEX,fees=0.0001))
    txs[SOURCE]['vout'][0]['scriptPubKey']['reservetransfer']=transfer
    alphabet='123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
    number=0
    for ch in ADDRESS:
        number=number*58+alphabet.index(ch)
    payload=number.to_bytes(25,'big')[1:21].hex()
    txs[IMPORT]['vout'][1]=dict(n=1,value=1,valueSat=100000000,scriptPubKey=dict(
        type='cryptocondition',reqSigs=1,spendableoutput=True,addresses=[ADDRESS],
        hex='1a040300010114'+payload+'cc',reserveoutput=dict(version=1,currencyvalues={VDEX:0.0001})))
    state=dict(launchcomplete=True,launchconfirmed=True,currencystate=dict(version=1,flags=49,currencyid=CURRENCY,currencies={VDEX:dict(lastconversionprice=1)}))
    record=dict(importheight=1,importtxid=IMPORT,importvout=0,importnotarization=state,
        transfers=[transfer],**{'import':IMPORT_INFO})
    server=daemon(VRSC,1,txs,1,record)
    server.currency_definitions={CURRENCY:dict(version=1,currencyid=CURRENCY,systemid=VRSC,
        name='Bridge.vETH',options=33,currencies=[VDEX]),VDEX:dict(version=1,currencyid=VDEX,systemid=VDEX,
        name='vDEX',options=256,proofprotocol=1,transactionimportfee=0.0002)}
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-local-gateway-') as directory:
            root=Path(directory);conf=root/'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal=Terminal(root,conf)
            try:
                terminal.key(b'/'+SOURCE.encode()+b'\r',0.6)
                wait(terminal,'Txid: '+SOURCE)
                terminal.key(b'\x1b[19~',0.2);terminal.key(b'\r',0.6)
                wait(terminal,'Received output verified')
                screen=terminal.screen()
                assert 'Received    0.0001 vDEX + 1 VRSC' in screen,screen
                assert 'Forwarded output' not in screen and 'Conversion fee' not in screen,screen
            finally:
                terminal.close()
    finally:
        server.shutdown();server.server_close()


def run_fee_pool_checkpoint(missing_previous=False):
    previous='9'*64
    previous_block='f'*64
    current=dict(txid=SOURCE,height=1,blockhash=BLOCK,confirmations=1,vin=[dict(coinbase='00')],vout=[
        dict(n=0,value=0,valueSat=0,scriptPubKey=dict(feepool=dict(version=1,currencyvalues={VRSC:100})))])
    prior=dict(txid=previous,height=0,blockhash=previous_block,confirmations=2,vin=[dict(coinbase='00')],vout=[
        dict(n=0,value=0,valueSat=0,scriptPubKey={} if missing_previous else dict(feepool=dict(version=1,currencyvalues={VRSC:98})))])
    server=daemon(VRSC,1,{SOURCE:current,previous:prior})
    server.block_hashes={1:BLOCK,0:previous_block}
    server.blocks={BLOCK:dict(hash=BLOCK,height=1,previousblockhash=previous_block,time=1700000060,
        tx=[SOURCE],size=10,confirmations=1),previous_block:dict(hash=previous_block,height=0,
        time=1700000000,tx=[previous],size=10,confirmations=2)}
    try:
        with tempfile.TemporaryDirectory(prefix='explorer-fee-pool-') as directory:
            root=Path(directory);conf=root/'VRSC.conf'
            conf.write_text(f'rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n')
            terminal=Terminal(root,conf)
            try:
                terminal.key(b'/'+SOURCE.encode()+b'\r',0.6)
                wait(terminal,'Txid: '+SOURCE)
                wait(terminal,'unspendable protocol checkpoint')
                terminal.key(b'\x1b[19~',0.2);terminal.key(b'\r',0.6)
                if missing_previous:
                    wait(terminal,'no zero balance is assumed')
                    assert 'Adjacent checkpoint delta' not in terminal.screen(),terminal.screen()
                else:
                    wait(terminal,'Adjacent checkpoint delta: +200000000')
                    assert previous in terminal.screen(),terminal.screen()
                    screen = terminal.screen()
                    assert 'payout is inferred from this delta' in ' '.join(screen.split()), screen
                assert 'getspentinfo' not in server.calls,server.calls
            finally:
                terminal.close()
    finally:
        server.shutdown();server.server_close()



def run_issuer_burn(change=None):
    prior, second, third, hash_transfers = (key * 64 for key in "1234")
    source_transfer = dict(version=1, flags=129, burnchangeprice=True,
        currencyvalues={CURRENCY: 0.6}, destinationcurrencyid=CURRENCY,
        feecurrencyid=VRSC, fees=0.0002, destination=dict(type=4, address=CURRENCY))
    second_transfer = dict(source_transfer, currencyvalues={CURRENCY: 0.1})
    third_transfer = dict(source_transfer, currencyvalues={CURRENCY: 0.3})
    custody = {CURRENCY: 1, VRSC: 0.0006}
    export = dict(version=1, flags=128, firstinput=0, numinputs=3,
        sourcesystemid=VRSC, destinationsystemid=VRSC, destinationcurrencyid=CURRENCY,
        sourceheightstart=8, sourceheightend=9, hashtransfers=hash_transfers,
        totalamounts=custody)
    imported = dict(version=1, flags=12, sourcesystemid=VRSC, sourceheight=9,
        importcurrencyid=CURRENCY, exporttxid=EXPORT, exporttxout=0,
        hashtransfers=hash_transfers, numoutputs=1, tokensout={}, valuein=custody)
    state = dict(version=1, flags=48, currencyid=CURRENCY, launchcurrencies=[],
        initialsupply=0, emitted=0, supply=10, preconvertedout=0,
        primarycurrencyfees=0, primarycurrencyconversionfees=0, primarycurrencyout=-1)
    before = dict(currencyid=CURRENCY, notarizationheight=7,
                  currencystate=dict(state, supply=11))
    after = dict(currencyid=CURRENCY, notarizationheight=9, prevheight=7,
        prevnotarizationtxid=prior, prevnotarizationout=0, currencystate=state)

    def tx(txid, height, inputs, outputs):
        return dict(txid=txid, height=height, confirmations=11-height,
                    blockhash=BLOCK, vin=inputs, vout=outputs)

    def output(n, value=0, **script):
        return dict(n=n, value=value, scriptPubKey=script)

    selected = output(0, 0.0002, reservetransfer=source_transfer)
    selected.update(spentTxId=EXPORT, spentIndex=0, spentHeight=10)
    txs = {
        SOURCE: tx(SOURCE, 8, [], [selected]),
        second: tx(second, 9, [], [output(0, 0.0002, reservetransfer=second_transfer)]),
        third: tx(third, 9, [], [output(0, 0.0002, reservetransfer=third_transfer)]),
        EXPORT: tx(EXPORT, 10, [dict(txid=id, vout=0) for id in (SOURCE, second, third)], [
            output(0, crosschainexport=export),
            output(1, 0.0006, reservedeposit=dict(controllingcurrencyid=CURRENCY,
                                                currencyvalues=custody))]),
        prior: tx(prior, 8, [], [output(0, acceptednotarization=before)]),
        IMPORT: tx(IMPORT, 10, [dict(txid=prior, vout=0), dict(txid=EXPORT, vout=1)], [
            output(0, crosschainimport=imported), output(1, acceptednotarization=after),
            output(2, 0.0001, type="pubkeyhash", spendableoutput=True)]),
    }
    record = dict(importheight=10, importtxid=IMPORT, importvout=0, importnotarization=after,
                  transfers=[source_transfer, second_transfer, third_transfer], **{"import": imported})
    if change == "batch":
        record["transfers"] = [source_transfer, third_transfer, second_transfer]
    elif change == "prior state":
        before["currencystate"]["supply"] = 10
    elif change == "hash":
        record["import"] = dict(imported, hashtransfers="5" * 64)
    server = daemon(VRSC, 10, txs, 10, record)
    server.currency_definitions = {CURRENCY: dict(name="Token", currencyid=CURRENCY,
        options=40, proofprotocol=1, systemid=VRSC, launchsystemid=VRSC)}
    try:
        with tempfile.TemporaryDirectory(prefix="carbide-issuer-burn-") as directory:
            root = Path(directory)
            conf = root / "VRSC.conf"
            conf.write_text(f"rpcconnect=127.0.0.1\nrpcport={server.server_port}\nrpcuser=test\nrpcpassword=test\n")
            terminal = Terminal(root, conf)
            try:
                terminal.key(b"/" + SOURCE.encode() + b"\r", 0.6)
                wait(terminal, "Txid: " + SOURCE)
                terminal.key(b"\x1b[19~", 0.2)
                terminal.key(b"\r", 0.6)
                if change:
                    wait(terminal, "Its supply reduction could not be verified")
                    screen = terminal.screen()
                    assert "Imported; supply burn unverified" in screen, screen
                    assert "Received" not in screen, screen
                else:
                    wait(terminal, "Supply burn verified")
                    screen = terminal.screen()
                    assert "Burned" in screen and "0.6 Token" in screen, screen
                    assert "Batch supply reduction  1 Token" in screen, screen
                    assert "Received" not in screen and "Burn execution state" in screen, screen
            finally:
                terminal.close()
    finally:
        server.shutdown()
        server.server_close()

if __name__=='__main__':
    run_issuer_burn()
    for change in ('batch', 'prior state', 'hash'):
        run_issuer_burn(change)
    run_fee_pool_checkpoint()
    run_fee_pool_checkpoint(missing_previous=True)
    run_local_gateway_fallback()
    run_launch_allocation()
    run_launch_allocation(altered=True)
    run_preconversion()
    run_preconversion(fractional=True)
    run_preconversion(altered=True)
    run_captured_pool_sell()
    run_pool_sell(pending=True)
    run_pool_sell()
    run_refund()
    run_refund(gateway=True)
    run_refund(gateway=True,wrong_destination=True)
    run_gateway_route(convert=True,pool_checks=True)
    run_gateway_route(pool_checks=True,wrong_pool=True)
