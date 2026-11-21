# Cross-chain architecture

## Operating model

The explorer runs against one network per process and can navigate across every known system in that network. Mainnet and testnet use separate databases, source registries, information graphs, and sessions:

```text
~/.cache/verus-carbide/mainnet.sqlite3
~/.cache/verus-carbide/testnet.sqlite3
```

There is no global or focused operating mode. The user selects a **current chain** at invocation and may change it explicitly. It defaults to VRSC and controls Dashboard, Mempool, Peers, other chain workspaces, the status bar, and unqualified searches. Each blockchain object separately retains its owning system. Following an object on another system never changes the current chain.

The network is fixed at startup. The explorer never queries a source from the other network or connects the two networks in its graph.

## Identities and object references

`systemid` is the canonical chain identity. Network and human-readable names are immutable attributes and aliases.

Fully qualified identity names are immutable and consensus-enforced as globally unique. Identity-derived currency, gateway, and PBaaS chain names are consequently immutable and unique. Identity i-addresses are immutable protocol locators. An i-address hash collision is computationally infeasible to cause intentionally, but the store need not assert mathematical uniqueness. If different names with the same i-address are ever observed, preserve both source-qualified observations and expose the anomaly without adding an ordinary name-disambiguation workflow.

Replace naked IDs in storage, events, navigation, retries, and loading sets with explicit references:

```rust
enum Network { Mainnet, Testnet }
struct SystemId(String);
struct SourceId(String);

struct ObjectRef {
    system_id: SystemId,
    kind: ObjectKind,
    id: String,
}

struct OutpointRef {
    system_id: SystemId,
    txid: String,
    vout: u32,
}
```

A block or transaction belongs permanently to one system. Reorganizations change block membership and transaction inclusion, not ownership. Ordinary links within an object inherit its owning system. Only a decoded cross-chain relationship changes systems. Preserve an unresolved relationship when the target system cannot be determined reliably; never guess.

Store one immutable identity definition keyed by its fully qualified name, with its i-address indexed as a locator. Store mutable identity versions and activity under their owning systems. Currency definitions are immutable; currency state is qualified by the system where it was observed, height, and block hash.

## Source registry

The source registry is a deep module between loaders and transport adapters. Views request an operation for a system; they do not construct clients or select endpoints.

There is at most one local daemon per system and possibly several chain-specific API endpoints. Give every connection a stable `source_id`. Connection state is independent for each source:

| Source | Disabled | Enabled | Monitored |
| --- | --- | --- | --- |
| Local daemon | No communication | Explicit requests and visible-view refresh | Enabled plus background tip, mempool, and ZMQ activity |
| API | No communication | Explicit consented requests | Unavailable |

Persist source state in the application configuration, outside SQLite. Session changes are not persisted until an explicit save. At startup, restore persistent connections and states, then session-enable the invoked chain's discovered daemon. Only persistently monitored daemons begin background work.

Opening a foreign object does not enable monitoring. A disabled local daemon presents connect-once, enable-for-session, and cancel choices. An enabled daemon may refresh a visible Dashboard or Mempool; leaving the view stops that work unless the daemon is monitored.

### Daemon discovery

Read only daemon configuration files. Never inspect blockchain databases, wallets, logs, process arguments, sockets, ports, or arbitrary files.

Follow Verus conventions and precedence for `-datadir`, `-conf`, and `-testnet`. Unless an alternate location is explicitly supplied or configured, inspect only standard locations, including:

```text
~/.komodo/VRSC/VRSC.conf
~/.komodo/vrsctest/vrsctest.conf
~/.verus/pbaas/<hex-encoded-chain-id>/...
~/.verustest/pbaas/<hex-encoded-chain-id>/...
```

A configuration file with connection information and credentials creates an available connection. Newly discovered daemons are disabled unless persistently configured; the invocation chain is the session-only exception. Store the configuration path and non-secret metadata. Read credentials from that file when connecting and never copy them into SQLite, application configuration, logs, or diagnostics.

An installed chain without usable connection information remains unavailable. The explorer never starts a daemon. Arbitrary endpoint editing is deferred. Verify a daemon's reported system ID before attaching its observations to a system.

### API consent and pools

Never contact an API without permission and acknowledgement. Permission may be persistent configuration or a modal offering Allow once, Allow for this session, and Cancel. The modal identifies the endpoint, system, operation, and reason. Invocation with a chain does not authorize its API.

For now, assume API endpoints for a chain have matching capabilities. Spread calls across enabled endpoints with health-aware round robin, retrying another endpoint after transport failure or rate limiting. Record the actual endpoint for every observation. Do not query several successful endpoints merely to look for discrepancies. Purposeful comparison belongs to a later fork-analysis mode.

Authenticated API configuration is deferred until the new service exists. Credentials must remain outside SQLite, URLs, logs, and diagnostics.

### Routing

Resolve an ordinary request in this order:

1. Valid cached immutable object.
2. Enabled local daemon.
3. Enabled API pool when the daemon is absent, unavailable, or cannot answer.
4. API-consent modal when an eligible API is disabled.
5. Explicit unresolved-source state.

The local daemon supplies the reference view of block membership, confirmations, and mempool state. Preserve conflicting API observations and notify the user without interrupting navigation or predicting which fork will prevail. If APIs disagree while no daemon is available, preserve the result being viewed and use temporary source affinity for that investigation branch.

## World store

Use one SQLite writer task with typed commands and WAL mode. Fetch tasks send writes through this interface rather than opening competing write connections. Every write carries the source configuration generation; obsolete results may be retained as history but cannot update live state.

The schema needs these conceptual tables:

- `systems`: system ID, immutable names, network attributes, and aliases.
- `sources`: stable source ID, system ID, kind, endpoint fingerprint, and non-secret metadata.
- `source_status`: current availability, height, update age, and configuration generation.
- `raw_blobs`: content-addressed payloads.
- `raw_responses`: source, system, method, normalized parameters, payload hash, time, and retention class.
- `objects`: system, kind, object ID, payload hash, and decoder version.
- `object_observations`: source attribution, time, source height, and discrepancy metadata.
- `blocks` and `block_membership`: immutable blocks plus member/orphan observations.
- `transactions` and `transaction_inclusions`: immutable transactions plus confirmed/unconfirmed inclusion observations.
- `mempool_membership`: replaceable membership without mandatory transaction hydration.
- `identity_definitions` and `identity_versions`: immutable global names/locators plus system-owned history.
- `currency_definitions` and `currency_states`: immutable definitions plus system/height/block-qualified state.
- `relationships`: typed, system-qualified links with carrier evidence and resolution state.
- `significant_events`: bounded reorg, discrepancy, failure, and recovery history.

Retain raw immutable objects and graph evidence. Store identical payload bytes once with lightweight per-source observations. Replace repetitive mutable responses while preserving compact membership and reorg transitions. Bound mempool diagnostic history by size and age. Keep peer and health responses only for significant transitions. Record changed tips, same-height replacement, rollback, reorg, and connectivity transitions rather than identical polls.

Store decoder versions with raw payloads and reproject lazily when decoding changes. Do not scan historical data automatically. Preserve orphaned blocks, unconfirmed transactions, and their relationships; update membership state instead of deleting them.

No general migration is required. Build and test the new schema using temporary databases, then discard the existing development cache and create fresh network databases. Clearing affects only the active network database and preserves configuration, permissions, bookmarks, and the running in-memory session.

## Relationship resolution

Every relationship retains its source and target system-qualified references, carrier transaction/output or decoded field, source observation, decoder version, resolution state, and membership observations for both sides.

The explorer resolves relationships and checks its own linkage. It ensures that counterpart objects were fetched from the intended systems, decoded identifiers connect the expected objects, navigation targets are correct, and source attribution is preserved. Consensus acceptance and chain membership come from daemon responses.

Use restrained states:

- `Unresolved`
- `Counterpart unavailable`
- `Linked from source side`
- `Both sides resolved`
- `Daemon reports finalized`
- `Inactive after reorg`
- `Source discrepancy`

Cross-chain checks involve both systems: a notarization carried on one chain and referenced on the other, an export and its destination import, and evidence/finalization transactions with the notarizations they reference. Proof-related fields may be decoded for presentation and graph linkage, but the explorer makes no claim to have validated proofs or consensus behavior.

Expansion is demand-driven. Selecting a link fetches one counterpart plus small labeling metadata. A bounded expansion action may use visible depth and object limits. Monitoring does not hydrate every mempool or block transaction merely to populate the graph. Full raw transactions delivered through ZMQ may receive cheap structural classification without additional calls.

## Session interface

The current chain changes only through the chain switcher. It controls chain-level workspaces, the status bar, and unqualified search. Each foreign object displays its owning chain. `d` returns to the current chain's Dashboard. A one-request search override may target another chain without changing the current chain.

Keep separate selection and scroll state for each chain's Dashboard, Mempool, Peers, and related workspaces. The switcher lists current, monitored, enabled, and unavailable systems with system name/ID, connection state, latest height, freshness, and health.

API-only systems provide snapshot Dashboard data with explicit refresh. Their Mempool, Peers, and logs explain that a local daemon is required.

Significant events from monitored systems enter a global notification list. Reorgs, discrepancies, sustained failures, and recoveries produce unobtrusive notices. Ordinary blocks and mempool churn update registry counters without interrupting navigation.

## Scheduling

Retain a configurable process-wide request ceiling of four by default. User work has priority over background work. Apply per-source background limits, stagger polling, and back off failures independently with jitter.

Initial defaults are approximately two seconds for a visible Mempool, five seconds for background monitored mempool membership, and three seconds for tip/status. ZMQ improves promptness but does not replace slower reconciliation at startup, after reconnect or sequence gaps, and around reorganizations.

Disabling a source cancels queued background work and stops polling. A transmitted request may finish and be retained as history but cannot refresh live state. Cached objects remain visible with disabled or stale source status.

## Configuration and bookmarks

Use one human-editable application TOML file under the platform configuration directory, separated by network and system. Keep daemon credentials in daemon configuration files.

Bookmarks use separate network files in the platform user-data directory:

```text
bookmarks-mainnet.jsonl
bookmarks-testnet.jsonl
```

Each versioned line contains a system ID, object kind or `auto`, canonical search target, user description, and creation time. Derived tags come from the resolved object. Malformed lines do not prevent valid bookmarks from loading and are reported without silently rewriting the file.

Missing bookmarks remain visible as uncached. Selecting one uses ordinary source and consent rules. Optional bounded startup population fetches only missing bookmarks, defaults to false, starts after the UI, and never authorizes a source. Opening a bookmark does not change the current chain.

## Implementation stages

1. Add network databases, the writer task, and strongly typed system-qualified references.
2. Add the source registry, exact configuration discovery, system association, and connection states.
3. Qualify the existing store, events, loaders, retries, searches, and navigation while preserving single-chain behavior.
4. Add current-chain switching, per-chain workspace state, and unavailable-source interfaces.
5. Add multiple-daemon monitoring, scheduling priorities, source generations, and significant events.
6. Add API consent, endpoint pools, fallback, and provenance.
7. Persist normalized objects, membership observations, and demand-driven relationships.
8. Add two-sided linkage and navigation for notarizations, exports/imports, and reserve transfers.

The first visible milestone starts on VRSC, discovers a PBaaS daemon while leaving it disabled, follows a system-qualified relationship, enables the target daemon for the session, loads the foreign transaction without changing the current chain, returns through Back, and explicitly switches Dashboard to the PBaaS system.

Tests use two deterministic fake systems with colliding heights and ordinary addresses, independent reorganizations, disabled-source activation, API consent, and source failure. Real decoded fixtures cover both sides of notarization and export/import relationships. Assertions cover routing, decoding, association, source attribution, navigation, and reactions to daemon-reported membership.

## Live VRSC/vDEX check (2026-09-12)

The real-daemon terminal walkthrough passed for both routes:

- VRSC accepted notarization → vDEX earned output 1; enable-for-session prompt,
  reverse references among loaded objects, Back to the accepted transaction,
  unchanged VRSC Dashboard context, then explicit vDEX chain switching.
- vDEX import → VRSC export output 1; connect-once prompt, reverse import link,
  Back to the import, and unchanged vDEX Dashboard context.

Both runs used temporary application configuration and network databases. No API
permission was granted. Trimmed daemon-response fixtures cover the exact
transactions and include their capture provenance. The mirrored predecessor
remains on the earned object's system; finalization and null-hash same-transaction
references remain on the carrier's system. Resolved-link labels describe decoded
object association only.

## Implementation checkpoint (architecture branch)

Implemented foundations:

- Network-bound databases, shared serialized writer, content-addressed raw payloads, system-qualified object keys and per-source observations.
- Exact config discovery and daemon-compatible path selection; chain aliases converge on their system ID. Invocation activation remains session-only.
- Independent per-chain stores, loaders, monitoring state and workspaces; explicit current-chain selection through F2. Foreign transaction navigation preserves current-chain context and selects the referenced output.
- Individual daemon/API controls, explicit state saving, consent modals, endpoint rotation and cooldown, daemon system checks, API provenance, and no API polling or HTTP redirects.
- Request limits and background reservations; queued requests recheck permissions before transmission and reject responses after permission changes. Queued daemon observations, Dashboard, Mempool and Peers events also recheck their generation before updating live state.
- Per-source block membership and transaction inclusions, source-discrepancy records, and independently mutable identity observations with locator-anomaly provenance.
- F3 demand-driven explicit import source-export, mirror-aware notarization predecessor, evidence-target, finalization-target, evidence-input and evidence-output links. Unknown systems retain unresolved references.
- F6 network-specific JSONL bookmarks with descriptions, derived cached-object tags, source-qualified opening, and navigation history. Cached observations remain usable without an enabled/configured source. Optional bounded startup population defaults off and contacts only already-enabled sources.
- Schema 3 storage: global immutable currency definitions, source/block-qualified currency states, generation/order/epoch-checked RPC persistence, bounded mutable retention, lazy transaction reprojection, and persistent reverse F3 relationships. See [STORAGE.md](STORAGE.md) for exact policies and limitations.

Still required before the full architecture design is complete:

- Broader relationship assessment and richer graph resolution states. F4 now provides bounded witness/evidence traversal, multipart assembly through daemon decoding, signature grouping and provenance. Live VRSC/vDEX notarization and import/export fixtures cover explicit target checks; reverse references now survive restarts.
- Currency viewer consuming qualified state. Local-daemon branch reconciliation, atomic membership updates, persistent orphan navigation and mutable-view invalidation are implemented in schema 4; schema 3 upgrades additively. F7 now exposes source-qualified inclusion history and child-branch selection, with cache-only paging and Back navigation.
- Generation attribution across remaining object-loader UI events, API investigation affinity, source health/freshness counters and a dedicated global event browser. Persistent RPC writes already recheck generation at the writer.
- Configurable scheduling limits, polling jitter, exit handling for unsaved source settings, and broader end-to-end navigation tests with two fake daemons.
- Optional ZMQ subscriptions with reconnect/sequence-gap reconciliation. Polling remains the implemented transport.

This checkpoint does not perform consensus or proof verification. Schema 3 upgrades additively to schema 4. Older development schemas require explicit fresh-cache replacement; there is no general migration.

## Deferred work

- TUI editing for arbitrary daemon and API endpoints.
- Persistent cross-chain investigation/session restoration.
- Purposeful fork-analysis mode.
- Authenticated API configuration after the new service becomes available.
