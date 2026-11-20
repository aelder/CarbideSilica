BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS database_identity(singleton INTEGER PRIMARY KEY CHECK(singleton=1),network TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS systems(system_id TEXT PRIMARY KEY, name TEXT);
CREATE TABLE IF NOT EXISTS source_endpoints(
    source_id TEXT NOT NULL, system_id TEXT NOT NULL, endpoint TEXT NOT NULL,
    PRIMARY KEY(source_id,system_id));
CREATE TABLE IF NOT EXISTS raw_blobs(hash TEXT PRIMARY KEY, payload TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS raw_rpc_cache(
    source TEXT NOT NULL, system_id TEXT NOT NULL REFERENCES systems(system_id),
    method TEXT NOT NULL, params_json TEXT NOT NULL,
    payload_hash TEXT NOT NULL REFERENCES raw_blobs(hash), fetched_at INTEGER NOT NULL,
    request_order INTEGER NOT NULL DEFAULT 0, generation INTEGER NOT NULL DEFAULT 0,
    retention TEXT NOT NULL DEFAULT 'snapshot',
    PRIMARY KEY(source,system_id,method,params_json));
CREATE INDEX IF NOT EXISTS raw_rpc_payload ON raw_rpc_cache(payload_hash);
CREATE TABLE IF NOT EXISTS objects(
    system_id TEXT NOT NULL REFERENCES systems(system_id), kind TEXT NOT NULL,
    object_id TEXT NOT NULL, payload_hash TEXT NOT NULL REFERENCES raw_blobs(hash),
    decoder_version INTEGER NOT NULL, PRIMARY KEY(system_id,kind,object_id));
CREATE INDEX IF NOT EXISTS object_payload ON objects(payload_hash);
CREATE TABLE IF NOT EXISTS object_observations(
    system_id TEXT NOT NULL, kind TEXT NOT NULL, object_id TEXT NOT NULL,
    source TEXT NOT NULL, body_hash TEXT NOT NULL REFERENCES raw_blobs(hash), payload_hash TEXT NOT NULL REFERENCES raw_blobs(hash), observed_at INTEGER NOT NULL,
    request_order INTEGER NOT NULL,
    PRIMARY KEY(system_id,kind,object_id,source,body_hash),
    FOREIGN KEY(system_id,kind,object_id) REFERENCES objects(system_id,kind,object_id));
CREATE INDEX IF NOT EXISTS observation_payload ON object_observations(payload_hash);
CREATE INDEX IF NOT EXISTS observation_body ON object_observations(body_hash);
CREATE TABLE IF NOT EXISTS relationships(
    system_id TEXT NOT NULL, carrier_id TEXT NOT NULL, relation TEXT NOT NULL,
    target_system_id TEXT NOT NULL, target_id TEXT NOT NULL, target_vout INTEGER NOT NULL DEFAULT -1,
    source TEXT NOT NULL,
    carrier_vout INTEGER NOT NULL DEFAULT -1, label TEXT NOT NULL DEFAULT '',
    payload_hash TEXT REFERENCES raw_blobs(hash), decoder_version INTEGER NOT NULL DEFAULT 1,
    resolution TEXT NOT NULL DEFAULT 'Linked from source side',
    PRIMARY KEY(system_id,carrier_id,carrier_vout,relation,target_system_id,target_id,target_vout,source));
CREATE INDEX IF NOT EXISTS relationship_target ON relationships(target_system_id,target_id,target_vout);
CREATE INDEX IF NOT EXISTS relationship_payload ON relationships(payload_hash);
CREATE TABLE IF NOT EXISTS currency_definitions(
    currency_id TEXT NOT NULL, name TEXT NOT NULL, definition_hash TEXT NOT NULL REFERENCES raw_blobs(hash),
    PRIMARY KEY(currency_id,definition_hash));
CREATE INDEX IF NOT EXISTS currency_name ON currency_definitions(name);
CREATE TABLE IF NOT EXISTS currency_definition_observations(
    system_id TEXT NOT NULL, source TEXT NOT NULL, currency_id TEXT NOT NULL,
    definition_hash TEXT NOT NULL REFERENCES raw_blobs(hash), observed_at INTEGER NOT NULL,
    PRIMARY KEY(system_id,source,currency_id,definition_hash));
CREATE TABLE IF NOT EXISTS currency_states(
    system_id TEXT NOT NULL, source TEXT NOT NULL, currency_id TEXT NOT NULL,
    state_kind TEXT NOT NULL, observation_key TEXT NOT NULL, reported_height INTEGER,
    observed_height INTEGER, observed_block_hash TEXT,
    payload_hash TEXT NOT NULL REFERENCES raw_blobs(hash), state_hash TEXT NOT NULL REFERENCES raw_blobs(hash),
    request_order INTEGER NOT NULL, observed_at INTEGER NOT NULL,
    PRIMARY KEY(system_id,source,currency_id,state_kind,observation_key));
CREATE INDEX IF NOT EXISTS currency_state_block ON currency_states(system_id,source,currency_id,observed_block_hash);
CREATE INDEX IF NOT EXISTS currency_state_payload ON currency_states(payload_hash);
CREATE INDEX IF NOT EXISTS currency_state_body ON currency_states(state_hash);
CREATE TABLE IF NOT EXISTS source_status(
    system_id TEXT NOT NULL, source TEXT NOT NULL, generation INTEGER NOT NULL,
    request_order INTEGER NOT NULL, height INTEGER, block_hash TEXT, observed_at INTEGER NOT NULL,
    PRIMARY KEY(system_id,source));
CREATE TABLE IF NOT EXISTS observation_heads(
    system_id TEXT NOT NULL, source TEXT NOT NULL, subject TEXT NOT NULL,
    request_order INTEGER NOT NULL, PRIMARY KEY(system_id,source,subject));
CREATE TABLE IF NOT EXISTS block_membership(
    system_id TEXT NOT NULL, source TEXT NOT NULL, block_hash TEXT NOT NULL, height INTEGER NOT NULL,
    membership TEXT NOT NULL, observed_at INTEGER NOT NULL,
    PRIMARY KEY(system_id,source,block_hash));
CREATE INDEX IF NOT EXISTS membership_height ON block_membership(system_id,height,membership);
CREATE TABLE IF NOT EXISTS transaction_inclusions(
    system_id TEXT NOT NULL, source TEXT NOT NULL, txid TEXT NOT NULL, block_hash TEXT NOT NULL,
    height INTEGER, confirmed INTEGER NOT NULL, observed_at INTEGER NOT NULL,
    PRIMARY KEY(system_id,source,txid,block_hash));
CREATE TABLE IF NOT EXISTS membership_history(
    system_id TEXT NOT NULL, source TEXT NOT NULL, kind TEXT NOT NULL, object_id TEXT NOT NULL,
    state TEXT NOT NULL, context TEXT NOT NULL, request_order INTEGER NOT NULL, observed_at INTEGER NOT NULL,
    PRIMARY KEY(system_id,source,kind,object_id,request_order));
CREATE TABLE IF NOT EXISTS mempool_membership(
    system_id TEXT NOT NULL, source TEXT NOT NULL, txid TEXT NOT NULL, observed_at INTEGER NOT NULL,
    PRIMARY KEY(system_id,source,txid));
CREATE TABLE IF NOT EXISTS identity_definitions(
    name TEXT NOT NULL PRIMARY KEY, identity_address TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS identity_locator ON identity_definitions(identity_address);
CREATE TABLE IF NOT EXISTS identity_observations(
    system_id TEXT NOT NULL, source TEXT NOT NULL, name TEXT NOT NULL,
    height INTEGER NOT NULL, payload_hash TEXT NOT NULL REFERENCES raw_blobs(hash), observed_at INTEGER NOT NULL,
    body_hash TEXT NOT NULL REFERENCES raw_blobs(hash),
    PRIMARY KEY(system_id,source,name,height,body_hash));
CREATE INDEX IF NOT EXISTS identity_payload ON identity_observations(payload_hash);
CREATE INDEX IF NOT EXISTS identity_body ON identity_observations(body_hash);
CREATE TABLE IF NOT EXISTS identity_evidence_reports(
    system_id TEXT NOT NULL, identity_id TEXT NOT NULL, version_txid TEXT NOT NULL,
    version_height INTEGER NOT NULL, source_id TEXT NOT NULL, block_hash TEXT,
    registration TEXT, notes_json TEXT NOT NULL, observed_at INTEGER NOT NULL,
    PRIMARY KEY(system_id,identity_id));
CREATE TABLE IF NOT EXISTS identity_evidence_edges(
    system_id TEXT NOT NULL, identity_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
    relation TEXT NOT NULL, target_kind TEXT NOT NULL, target_id TEXT NOT NULL,
    detail TEXT NOT NULL, evidence TEXT NOT NULL,
    PRIMARY KEY(system_id,identity_id,ordinal),
    FOREIGN KEY(system_id,identity_id) REFERENCES identity_evidence_reports(system_id,identity_id) ON DELETE CASCADE);
CREATE INDEX IF NOT EXISTS identity_evidence_target ON identity_evidence_edges(system_id,target_kind,target_id);
CREATE TABLE IF NOT EXISTS defi_lifecycle_edges(
    root_system_id TEXT NOT NULL, root_txid TEXT NOT NULL, root_vout INTEGER NOT NULL,
    relation TEXT NOT NULL, source_system_id TEXT NOT NULL, source_txid TEXT NOT NULL,
    source_vout INTEGER NOT NULL, target_system_id TEXT NOT NULL, target_txid TEXT NOT NULL,
    target_vout INTEGER NOT NULL, source_id TEXT NOT NULL, source_block_hash TEXT NOT NULL,
    source_height INTEGER NOT NULL, target_source_id TEXT NOT NULL,
    target_block_hash TEXT NOT NULL, target_height INTEGER NOT NULL,
    resolution TEXT NOT NULL, detail TEXT NOT NULL, observed_at INTEGER NOT NULL,
    PRIMARY KEY(root_system_id,root_txid,root_vout,relation,target_system_id,target_txid,target_vout));
CREATE INDEX IF NOT EXISTS defi_lifecycle_target ON defi_lifecycle_edges(target_system_id,target_txid,target_vout);
CREATE TABLE IF NOT EXISTS significant_events(
    event_key TEXT PRIMARY KEY, message TEXT NOT NULL, observed_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS reconciled_chain(system_id TEXT NOT NULL, source TEXT NOT NULL, revision INTEGER NOT NULL, state_json TEXT NOT NULL, PRIMARY KEY(system_id,source));
CREATE TABLE IF NOT EXISTS reconciliation_headers(system_id TEXT NOT NULL, block_hash TEXT NOT NULL, height INTEGER NOT NULL, parent_hash TEXT, block_time INTEGER NOT NULL, PRIMARY KEY(system_id,block_hash));
CREATE TABLE IF NOT EXISTS currency_activity_pages(
    system_id TEXT NOT NULL, currency_id TEXT NOT NULL, source TEXT NOT NULL,
    anchor_height INTEGER NOT NULL, anchor_hash TEXT NOT NULL,
    first_height INTEGER NOT NULL, last_height INTEGER NOT NULL,
    page_json TEXT NOT NULL, request_order INTEGER NOT NULL, observed_at INTEGER NOT NULL,
    PRIMARY KEY(system_id,currency_id,source,anchor_hash,first_height,last_height));
CREATE INDEX IF NOT EXISTS currency_activity_lookup ON currency_activity_pages(system_id,currency_id,request_order DESC);
PRAGMA user_version=6;
COMMIT;
