# Storage architecture — schema 6

The explorer owns one SQLite database per network: `mainnet.sqlite3` and
`testnet.sqlite3` under `$XDG_CACHE_HOME/verus-carbide/` on Linux. If
`XDG_CACHE_HOME` is unset, the directory is `~/.cache/verus-carbide/`. System
IDs qualify objects inside each file. The selected navigation chain never
selects another network.

## Ownership and writes

`Cache` is the interface to a bounded, serialized database worker. Connections
use WAL and foreign keys. Handles in one process share a worker; on Unix an
exclusive advisory lock prevents another current explorer from opening the same
file. Non-Unix builds currently provide process-local serialization only.

Successful RPC responses pass through this worker before publication to live
observers. Each write carries the network, source, system, source permission
generation, request order, branch epoch and cache epoch. The worker rechecks these
when processing the command. Revocation, re-enabling, reorgs, or clearing the cache
invalidates earlier work. Request ordering prevents late responses from replacing
newer exact-key, object, currency, tip, height or mempool observations. Storage
failure stops the request instead of triggering an API fallback.

Writes, relationship lookup and asynchronous object/witness cache reads await
blocking-worker replies outside Tokio's runtime threads. Startup, configuration,
and some existing UI metadata lookups still use synchronous facade calls.

## Immutable data and observations

- `raw_blobs` deduplicates normalized JSON by SHA-256 content hash.
- `objects` stores normalized block and transaction bodies. Confirmations,
  inclusion context and spent-output annotations belong to observations. Block
  transaction arrays normalize to IDs; headers cannot replace a stored full block.
- `object_observations` preserves source attribution and differing body versions,
  replacing repeated observations of the same body with their latest raw response.
- `block_membership`, `transaction_inclusions`, `membership_history`,
  `source_status` and `mempool_membership` hold source-qualified chain observations.
  Mempool disappearance means absence, not confirmation. For a reconciled local
  daemon, only the reconciler changes the active branch. Other sources retain
  independent observations; they cannot replace the daemon's mapping.
- Identity content versions are retained separately from their latest outer RPC
  response. Conflicting identity locators and currency definitions retain evidence
  and create significant events.

Ordinary cache-first block reads require a hash. A height is a mutable alias and
must be fetched again. Explicit bookmark snapshots can still open an older
height response with cached/source attribution.

## Currency definitions and state

Definitions are global by currency ID and definition hash, with a separate table
recording observing systems and sources. Conflicting definitions coexist.

Best and last-confirmed state retain the daemon's reported height separately from
the observing system's block height and hash. `getcurrency.bestheight` can refer to
a remote notarization; it does not establish local freshness. A noninteractive
`getblockchaininfo` request before and after the currency call qualifies state only
when both tips and all three sources match. These context requests use already
enabled sources; failure leaves a replaceable, unanchored snapshot.

Anchored history retains identical payloads observed at different blocks. Current
state reads require the same source's current height and hash to match the anchor;
height proximity alone is insufficient. Currency reads remain conservative and
require an exact observing-block match even though branch ancestry is now tracked.

## Reorg reconciliation

One reconciler runs serially with each local daemon's dashboard poller, respecting
enabled/monitored state and existing RPC limits. It never falls back to an API.
It observes `getblockchaininfo`, walks hash-keyed headers, and rechecks the tip
before committing. Normal advances attach missing descendants; mismatches walk
both branches to their common ancestor without a fixed depth cutoff. Walks yield
between batches, check source state, and reuse persisted headers on retries.

Initial coverage is the latest 100 blocks. Opening older blocks or transactions
queues targeted height validation against the same daemon, bracketed by matching
tip observations; this avoids an automatic historical scan. Missing data leaves
reconciliation incomplete and confirmations unverified. A changed tip during the
walk, revoked permissions, a cache clear, or a changed persisted revision prevents
the candidate from committing.

`reconciliation_headers` retains header ancestry. `block_membership` is the
source's active/orphan mapping; `reconciled_chain` holds a small tip/revision
checkpoint. A single transaction applies displaced/adopted membership, known
transaction inclusions, checkpoint, mutable-response invalidation and reorg event.
Historical headers and transaction inclusions survive restart. Ordinary RPC
responses cannot reinstate a displaced mapping.

The UI applies the committed change, derives confirmations from active membership,
and preserves the open object. Branch epochs reject obsolete mutable loader
results. Address balances/history/UTXOs, current identities, offers and output
state refresh on demand; the visible view is prioritized by existing loaders.
Raw evidence and immutable relationship links remain retained. F3 linkage labels
continue to describe structural evidence rather than consensus finality.

Block navigation follows known branch links. Ambiguous/missing successors do not
silently jump branches. `a` opens the active block at the displayed height.
F7 browses persisted transaction inclusions or child branches. Read-only worker
queries keep membership source- and system-qualified, combine header/body child
evidence, and return pages of 100 observations. Empty block hashes (unconfirmed
transaction observations) are excluded. Missing block membership is unverified,
not inferred from a negative confirmation flag. The panel uses last-observed
labels and reloads after committed reconciliation revisions. No schema change or
cache reset is required.

## Relationships and decoding

Stored edges preserve carrier and target system IDs, transaction IDs and output
indices, source, decoder version and raw-payload provenance. F3 also reads reverse
references from disk after a restart. A lookup is bounded to 256 known carriers
and 256 returned links. Opening a stale transaction projection rebuilds that
transaction from stored raw observations. Reverse lookup rebuilds only known
carriers for the selected target. Neither operation scans historical RPC data or
claims signature, proof, quorum or consensus verification.

Identity evidence reports and typed links are retained by system, identity,
identity-version transaction and height. They include the observing source and
block hash. A version mismatch or explicitly orphaned anchor prevents reuse.
Historical reverse-index links record the query target and bounded height window;
they remain evidence of an association, not ownership. A retained report can be
shown while a fresh load runs.

DeFi lifecycle edges retain source and target outpoints, observing sources,
block hashes, heights, and whether a result was attributed or remains a candidate.
Both endpoint transactions must have block-qualified observations before an edge
is stored. Reopening F8 shows retained edges while it investigates again. Each
endpoint displays its last observed membership, including orphan or unverified;
retained edges alone do not establish current chain membership or payment finality.

## Retention

Maintenance runs every 64 accepted writes:

- Immutable object responses and graph evidence remain retained.
- Mutable raw responses have a combined 64 MiB payload budget (shared payloads
  count once per response reference for this conservative budget).
- Diagnostic responses expire after 10 minutes.
- Other mutable snapshots expire after 7 days or beyond the newest 10,000 rows.
- Mempool transition history expires after 7 days or beyond 10,000 rows.
- Unanchored currency state is replaceable; anchored state remains historical.
- Significant events retain at most 200 entries.
- Each maintenance pass removes up to 1,024 unreferenced blobs, checking all
  evidence references first.

These are maintenance thresholds, not a hard limit on total database size.
Immutable history, referenced evidence, and up to 63 pending writes can exceed
them; SQLite also reuses freed pages until explicit clearing compacts the file.

## Fresh-schema reset

Schemas 3, 4 and 5 upgrade additively to schema 6 on open, preserving existing data. There
is no general migration of older development caches. For an explicit reset, run
`cargo run -- --reset-cache` for mainnet or
`cargo run -- -testnet --reset-cache` for testnet. This replaces only the selected
network cache and exits without RPC requests. The reset refuses an active current
writer, mismatched network, unknown schema version or unrecognized tables. It
preserves configuration, permissions, bookmarks and the other network database.
Retired per-chain files are not opened by the new architecture.

Regression coverage lives in `src/reconciliation_tests.rs`,
`tests/reorg_terminal.py`, `src/cache/regression_tests.rs`, `src/cache.rs`,
`src/rpc.rs`, and `tests/bookmarks_terminal.py`.

## Bounded currency activity pages

`currency_activity_pages` indexes explicitly queried activity windows by observing
system, currency ID, source, anchor block hash and inclusive height range. Pages
include exact import/export outpoints, source-export references and transfer hashes
when supplied by the daemon. Queries span at most 1,000 blocks and retain at most
80 records, sorted newest first with an explicit truncation flag. Empty pages are
retained as evidence of the examined range, never as proof of no other activity.

A same-source block lookup before/after both RPCs must match the selected state's
anchor hash/height. Nullable empty import responses preserve endpoint provenance.
The writer rechecks permission generation, branch epoch, cache epoch and request
order. A late page cannot overwrite newer evidence for the same key. A targeted
index read returns the latest eight pages; storage retains at most 32 per system
and currency. Each retained page is labeled with its anchor membership or unknown
membership. It never becomes current merely because its height is near the tip.
No broad raw-response or transaction-table scan is used. Clearing/resetting the
cache includes the activity index; schema 5 upgrades additively on open.
