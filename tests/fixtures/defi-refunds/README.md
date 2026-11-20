# Captured burns and failed-launch refunds

These JSON files contain public protocol examples from VRSC daemon 1.2.18.
Formatting and numeric rendering were normalized at capture time. An independent
Python Decimal verifier generated `verification.json`.
`SHA256SUMS` preserves the imported payloads and this catalog. No RPC is needed
to run the Rust regressions. The terminal test uses isolated mock daemons.

| File | Case |
| --- | --- |
| refund-superbasket.json | Native 5 VRSC repayment after failed launch |
| refund-tbtc.json | Independent native 5 VRSC repayment |
| refund-bots.json | Two reserve-token repayments in one batch |
| refund-eurc.json | Reserve-token repayment through a failed basket |
| refund-bots-launch.json | Five preconversions repaid to the Bots currency identity |
| burn-ethereum-dai.json | Ethereum-origin DAI reserve burn accepted by Bridge.vETH |

Refund bundles retain source transfers, the spending export, import record and
actual import outputs. Tests bind source outpoints to export input ordinals and
compare exact expected output indices and amounts. They exercise the same
generated-region, ordered-prefix replay and financial summary used by F8.
Negative cases change prefix amounts, destinations, scripts, currency-state
identity, phase and source system. A matching later destination cannot rescue an
unsupported or altered prefix.

The covered refund subset is same-system, one ordinary output per transfer.
Failed-state refunds do not require a successful-launch flag or completion of
all preconversions. Native P2PKH outputs require their exact serialized script;
native identity outputs require the standard version-3 one-of-one EVAL_NONE
script. Reserve refunds require the decoded complete amount vector and single
destination; identity reserve outputs additionally require the standard decoded
condition and its identity script prefix. This is bounded replay, not a complete
consensus verifier. Arbitrary CC scripts, other destination kinds, cross-system
returns and multi-output allocations for one transfer remain unsupported.

The DAI fixture verifies the supplemental transfer instruction, reserve custody,
linked prior pool state and unchanged supply. It deliberately produces no
replayed customer receipt. Its Burn tag describes typed protocol instruction
evidence. The external export digest is not assumed to be an Ethereum transaction
hash; no originating Ethereum receipt is included or independently confirmed.

Run from the application repository:

```sh
cargo test defi::fixtures
python3 tests/defi_fixtures_terminal.py
```

From this directory, verify payload integrity with `sha256sum -c SHA256SUMS`.
