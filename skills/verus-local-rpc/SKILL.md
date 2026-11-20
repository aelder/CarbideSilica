---
name: verus-local-rpc
description: Query a local Verus, VRSC, VRSCTEST, or PBaaS daemon through the constrained read-only RPC wrapper. Use for targeted blockchain research, daemon connectivity checks, and inspection of blocks, transactions, addresses, identities, currencies, imports, exports, offers, peers, or the mempool without granting unrestricted network or curl access.
---

# Verus Local RPC

Use the installed `verus-rpc` wrapper on PATH for local daemon RPC calls. Do not replace it
with `curl`, the Verus CLI, or another network client. The installed wrapper is
an externally controlled capability: it permits only loopback connections and
an allowlist of read-only methods.

## Call The Daemon

Run one direct wrapper command per tool call:

```bash
verus-rpc getinfo
verus-rpc getblock 4221288 1
verus-rpc getrawtransaction TXID 1
```

Do not append a pipeline or shell redirection. Parse the returned JSON from the
tool result. Set an output limit appropriate to the expected response.

Pass parameters that need exact JSON typing with `--raw-params` before the
method:

```bash
verus-rpc --raw-params '["VRSC",100,200]' getimports
```

Use `-chain=NAME`, `-testnet`, or `--conf PATH` before the method when the
default VRSC mainnet configuration is not appropriate. Prefer `--conf` when a
PBaaS chain's configuration directory is represented by a hex identifier.

## Minimize Daemon Load

1. Call `getinfo` first when chain height or connectivity matters.
2. Prefer direct object lookup and transaction traversal over broad scans.
3. Use `getcurrencyconverters CURRENCY` instead of broad `listcurrencies`
   queries when discovering liquidity pools for a reserve currency.
4. Give `getimports` and `getexports` tight height bounds. Start with at most
   1,000 blocks and use smaller windows on a slow daemon.
5. Treat chain-derived refresh intervals in blocks rather than wall-clock time
   unless inspecting volatile data such as peers or the mempool.
6. Do not retry immediately in a loop. The wrapper permits only one concurrent
   wrapper request and waits up to 75 seconds for an earlier call.
7. If a method is rejected by the allowlist, report the missing capability.
   Do not bypass the wrapper.

## Interpret Results

- Exit `0` with output `null` means the RPC succeeded and returned no value.
- Exit `64` means invalid wrapper arguments or a disallowed method.
- Exit `69` means the daemon returned a JSON-RPC error.
- Exit `70` means the daemon returned an invalid JSON-RPC envelope.
- Exit `75` means the wrapper timed out waiting for its serialization lock.
- Other nonzero exits from a transport failure preserve curl's exit status.

Never interpret a successful `null` response as an unexpected EOF or transport
failure.

## Preserve The Boundary

Use only read-only investigation methods. Never request that the wrapper be
expanded with wallet mutation, transaction submission, mining, staking,
administrative, or peer-control RPCs. Do not print RPC credentials or inspect
the wrapper's temporary authentication file.
