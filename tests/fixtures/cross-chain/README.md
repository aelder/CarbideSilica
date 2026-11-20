# VRSC / vDEX navigation fixtures

Peer endpoints use documentation-range IP addresses. The navigation cases use trimmed
`getrawtransaction(txid, 1)` responses from the two mainnet daemons, not fabricated
transactions. Recursive `hex`, `asm`, and `evidence` fields were omitted to keep
these navigation fixtures small. Amounts were parsed as JSON numbers; do not use
these files for byte serialization or monetary precision tests. Confirmation
counts and spent-output observations describe the capture time.

| File | Owning system | Transaction height | Relevant outputs |
| --- | --- | --- | --- |
| vrsc-accepted.json | VRSC | 3241019 | accepted 0; foreign evidence 1–2; pending finalization 3; prior confirmed finalization 5 |
| vdex-earned.json | vDEX | 82025 | earned 1; multipart evidence carriers 2–3; pending finalization 4 |
| vdex-predecessor.json | vDEX | 82013 | earned predecessor 1 |
| vdex-import.json | vDEX | 82104 | currency import 0; system import 1 |
| vrsc-export.json | VRSC | 3241034 | currency export 1; system export 2 |

VRSC system ID: `i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV`.
vDEX system ID: `iHog9UCTrn95qpUBFCZ7kKz7qWdMA8MQ6N`.

The accepted notarization's evidence targets the vDEX earned transaction at
output 1. Its mirrored predecessor also belongs to vDEX: mirroring preserves
`prevnotarizationtxid`, while swapping the principal currency/state/root.
The predecessor was found on vDEX; VRSC returned RPC error -5 for the same ID.
The pending finalization still targets the local accepted output. Outpoints in
the daemon JSON use `voutnum`; finalization scripts use `finalizenotarization`.

The vDEX import points to the two distinct VRSC export outputs. Their supplied
source/destination system IDs and transfer identifiers agree. These assertions
check explorer association and navigation only, not proofs or consensus.

## Witness fixtures

`vrsc-witness.json` is a local VRSC response for
`8709b2dc58acd1f8f7912aa8723b194f9a8820b0d3b55cfd1666930c40a9da00`.
It retains decoded evidence and signatures, omitting recursive `hex` and `asm`
fields. Its evidence output 1 contains a confirming entry from
`iByTJrnrwStZyV2qnLg5jipp12rnAPfCZZ`, signing height 3,083,106.

`vrsc-multipart.json` contains synthetic fragments for testing evidence reassembly, condition-script reconstruction, and rejection of incomplete or reversed fragments.
