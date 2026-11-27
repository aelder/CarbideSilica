"""Synthetic VRSC chain fixtures shared by the demo and terminal tests."""

VRSC = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV"


def block_hash(height):
    return f'{height + 1:064x}'


def header(height):
    compact = 0x1c0f0f0f if height % 4 else 0x1c070707
    target = (compact & 0x7fffff) << (8 * ((compact >> 24) - 3))
    result = dict(hash=block_hash(height), height=height, time=1700000000 + height * 60,
                  validationtype='stake' if height % 2 else 'work',
                  blocktype='staked' if height % 2 else 'mined',
                  difficulty=100.5 + height, nonce=f'{compact:064x}',
                  version=65540, confirmations=5001-height, tx=[], size=10)
    if height % 2:
        result['postarget'] = f'{target:064x}'
    if height:
        result['previousblockhash'] = block_hash(height-1)
    if height < 5000:
        result['nextblockhash'] = block_hash(height+1)
    return result
