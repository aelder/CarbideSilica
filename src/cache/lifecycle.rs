//! Projection, observation ordering and retention all run on the single writer.
use super::*;
use crate::world::SystemId;
#[cfg(unix)]
pub(super) type Ownership = nix::fcntl::Flock<std::fs::File>;
#[cfg(not(unix))]
pub(super) type Ownership = std::fs::File;

pub(super) fn acquire_ownership(path: &Path) -> Result<Ownership> {
    let parent = path.parent().context("database has no directory")?;
    fs::create_dir_all(parent)?;
    let lock = parent.canonicalize()?.join(format!(
        "{}.lock",
        path.file_name()
            .context("database has no filename")?
            .to_string_lossy()
    ));
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock)?;
    #[cfg(unix)]
    {
        nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusiveNonblock).map_err(
            |(_, error)| {
                anyhow::anyhow!(
                    "database is already open by another explorer process: {} ({error})",
                    path.display()
                )
            },
        )
    }
    #[cfg(not(unix))]
    {
        Ok(file)
    }
}
use rusqlite::Transaction;

pub(super) fn retention(method: &str, params: &Value) -> &'static str {
    match method {
        "getrawtransaction" | "decodescript" => "immutable",
        "getblock" | "getblockheader" if params[0].as_str().is_some_and(|s| s.len() == 64) => {
            "immutable"
        }
        "getidentity" if params[1].as_u64().is_some() => "immutable",
        "getrawmempool" | "getpeerinfo" | "getinfo" | "getblockchaininfo" | "getbestblockhash"
        | "getblockcount" => "diagnostic",
        _ => "snapshot",
    }
}

impl StoreConnection {
    pub(super) fn blob(tx: &Transaction<'_>, value: &Value) -> Result<String> {
        use sha2::{Digest, Sha256};
        let raw = serde_json::to_string(value)?;
        let hash = format!("{:x}", Sha256::digest(raw.as_bytes()));
        tx.execute(
            "insert or ignore into raw_blobs values(?1,?2)",
            params![hash, raw],
        )?;
        Ok(hash)
    }

    pub(super) fn advance_head(
        tx: &Transaction<'_>,
        system: &str,
        source: &str,
        subject: &str,
        order: i64,
    ) -> Result<bool> {
        let previous:Option<i64>=tx.query_row("select request_order from observation_heads where system_id=?1 and source=?2 and subject=?3",params![system,source,subject],|r|r.get(0)).optional()?;
        if previous.is_some_and(|p| p >= order) {
            return Ok(false);
        };
        tx.execute("insert into observation_heads values(?1,?2,?3,?4) on conflict(system_id,source,subject) do update set request_order=excluded.request_order",params![system,source,subject,order])?;
        Ok(true)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn membership_transition(
        tx: &Transaction<'_>,
        system: &str,
        source: &str,
        kind: &str,
        id: &str,
        state: &str,
        context: &str,
        order: i64,
    ) -> Result<()> {
        let last:Option<(String,String)>=tx.query_row("select state,context from membership_history where system_id=?1 and source=?2 and kind=?3 and object_id=?4 order by request_order desc limit 1",params![system,source,kind,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if last
            .as_ref()
            .is_none_or(|(s, c)| s != state || c != context)
        {
            tx.execute(
                "insert or ignore into membership_history values(?1,?2,?3,?4,?5,?6,?7,unixepoch())",
                params![system, source, kind, id, state, context, order],
            )?;
        }
        Ok(())
    }

    pub(super) fn project_source_state(
        tx: &Transaction<'_>,
        source: &str,
        system: &str,
        method: &str,
        result: &Value,
        order: i64,
        generation: i64,
    ) -> Result<()> {
        if method == "getblockchaininfo"
            && let (Some(height), Some(hash)) =
                (result["blocks"].as_i64(), result["bestblockhash"].as_str())
            && Self::advance_head(tx, system, source, "tip", order)?
        {
            let previous: Option<(i64, String)> = tx
                .query_row(
                    "select height,block_hash from source_status where system_id=?1 and source=?2",
                    params![system, source],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let managed: bool = tx.query_row(
                "select exists(select 1 from reconciled_chain where system_id=?1 and source=?2)",
                params![system, source],
                |r| r.get(0),
            )?;
            if let Some((old_height, old_hash)) = previous.filter(|(_, h)| h != hash)
                && height <= old_height
                && !managed
            {
                tx.execute("insert or replace into significant_events values(?1,?2,unixepoch())",params![format!("tip:{system}:{source}:{hash}"),format!("{system}: {source} reports tip change {old_height}:{old_hash} -> {height}:{hash}")])?;
                tx.execute("update block_membership set membership='unverified' where system_id=?1 and source=?2 and height>=?3",params![system,source,height])?;
                tx.execute("update transaction_inclusions set confirmed=0 where system_id=?1 and source=?2 and (height>=?3 or block_hash in(select block_hash from block_membership where system_id=?1 and source=?2 and membership='unverified'))",params![system,source,height])?;
            }
            tx.execute("insert into source_status values(?1,?2,?3,?4,?5,?6,unixepoch()) on conflict(system_id,source) do update set generation=excluded.generation,request_order=excluded.request_order,height=excluded.height,block_hash=excluded.block_hash,observed_at=excluded.observed_at",params![system,source,generation,order,height,hash])?;
        }
        if method == "getrawmempool" && Self::advance_head(tx, system, source, "mempool", order)? {
            let ids: std::collections::BTreeSet<String> = if let Some(map) = result.as_object() {
                map.keys().cloned().collect()
            } else {
                result
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            };
            let old: Vec<String> = {
                let mut statement = tx.prepare(
                    "select txid from mempool_membership where system_id=?1 and source=?2",
                )?;
                statement
                    .query_map(params![system, source], |r| r.get(0))?
                    .collect::<rusqlite::Result<_>>()?
            };
            for id in old {
                if !ids.contains(&id) {
                    Self::membership_transition(
                        tx, system, source, "mempool", &id, "absent", "", order,
                    )?;
                }
            }
            tx.execute(
                "delete from mempool_membership where system_id=?1 and source=?2",
                params![system, source],
            )?;
            for id in ids {
                Self::membership_transition(
                    tx, system, source, "mempool", &id, "present", "", order,
                )?;
                tx.execute(
                    "insert into mempool_membership values(?1,?2,?3,unixepoch())",
                    params![system, source, id],
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn project_links(
        tx: &Transaction<'_>,
        source: &str,
        system: &SystemId,
        raw: &Value,
        payload: &str,
    ) -> Result<()> {
        let Some(id) = raw["txid"].as_str() else {
            return Ok(());
        };
        tx.execute(
            "delete from relationships where system_id=?1 and carrier_id=?2 and source=?3",
            params![system.as_ref(), id, source],
        )?;
        for link in crate::relationships::transaction_links(system, raw) {
            tx.execute("insert or ignore into relationships(system_id,carrier_id,relation,target_system_id,target_id,target_vout,source,carrier_vout,label,payload_hash,decoder_version) values(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![system.as_ref(),id,link.field,link.target.system_id.as_ref(),link.target.txid,link.target.vout,source,link.carrier_vout,link.label,payload,DECODER_VERSION])?;
        }
        for input in raw["vin"].as_array().into_iter().flatten() {
            if let (Some(target), Some(vout)) = (input["txid"].as_str(), input["vout"].as_u64()) {
                tx.execute("insert or ignore into relationships(system_id,carrier_id,relation,target_system_id,target_id,target_vout,source,payload_hash,decoder_version) values(?1,?2,'spends',?1,?3,?4,?5,?6,?7)",params![system.as_ref(),id,target,i64::try_from(vout)?,source,payload,DECODER_VERSION])?;
            }
        }
        Ok(())
    }

    pub(super) fn reproject(&self, system: &str, id: &str) -> Result<()> {
        let version:Option<i64>=self.connection.query_row("select decoder_version from objects where system_id=?1 and kind='transaction' and object_id=?2",params![system,id],|r|r.get(0)).optional()?;
        if version.is_none_or(|v| v == DECODER_VERSION) {
            return Ok(());
        };
        let system_id = SystemId::try_from(system.to_owned())?;
        let tx = self.connection.unchecked_transaction()?;
        let rows: Vec<(String, String, String)> = {
            let mut statement=tx.prepare("select o.source,o.payload_hash,b.payload from object_observations o join raw_blobs b on b.hash=o.payload_hash where o.system_id=?1 and o.kind='transaction' and o.object_id=?2 order by o.request_order")?;
            statement
                .query_map(params![system, id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                })?
                .collect::<rusqlite::Result<_>>()?
        };
        for (source, hash, raw) in rows {
            let value: Value = serde_json::from_str(&raw)?;
            Self::project_links(&tx, &source, &system_id, &value["result"], &hash)?;
        }
        tx.execute("update objects set decoder_version=?1 where system_id=?2 and kind='transaction' and object_id=?3",params![DECODER_VERSION,system,id])?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn reverse_links(
        &self,
        system: &str,
        id: &str,
    ) -> Result<Vec<crate::relationships::Link>> {
        // Reproject only known carriers for this target. No historical graph scan.
        let carriers: Vec<(String, String)> = {
            let mut statement=self.connection.prepare("select distinct system_id,carrier_id from relationships where target_system_id=?1 and target_id=?2 limit 256")?;
            statement
                .query_map(params![system, id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        for (system, carrier) in carriers {
            self.reproject(&system, &carrier)?;
        }
        let mut statement=self.connection.prepare("select distinct system_id,carrier_id,relation,target_vout,carrier_vout,label from relationships where target_system_id=?1 and target_id=?2 and carrier_vout>=0 order by system_id,carrier_id,carrier_vout,relation limit 256")?;
        let rows = statement.query_map(params![system, id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
                r.get::<_, u32>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        rows.map(|row| {
            let (system, id, field, target_vout, carrier_vout, label) = row?;
            Ok(crate::relationships::Link {
                label: format!("Referenced by: {label} (stored observation)"),
                field: format!("reverse:{field}"),
                carrier_vout: target_vout,
                target: crate::world::OutpointRef {
                    system_id: SystemId::try_from(system)?,
                    txid: id,
                    vout: carrier_vout,
                },
            })
        })
        .collect()
    }

    pub(super) fn retain_snapshots(tx: &Transaction<'_>) -> Result<()> {
        tx.execute("delete from raw_rpc_cache where rowid in(select id from (
            select r.rowid as id,sum(length(b.payload)) over(order by r.fetched_at desc,r.rowid desc) as bytes
            from raw_rpc_cache r join raw_blobs b on b.hash=r.payload_hash where r.retention<>'immutable') where bytes>67108864)",[])?;
        tx.execute(
            "delete from raw_rpc_cache where retention='diagnostic' and fetched_at<unixepoch()-600",
            [],
        )?;
        tx.execute("delete from raw_rpc_cache where retention='snapshot' and (fetched_at<unixepoch()-604800 or rowid in(select rowid from raw_rpc_cache where retention='snapshot' order by fetched_at desc,rowid desc limit -1 offset 10000))",[])?;
        tx.execute("delete from membership_history where kind='mempool' and (observed_at<unixepoch()-604800 or rowid in(select rowid from membership_history where kind='mempool' order by request_order desc limit -1 offset 10000))",[])?;
        // Unanchored getcurrency snapshots are replaceable. Anchored history is evidence.
        tx.execute("delete from currency_states where observed_block_hash is null and request_order<(select max(newer.request_order) from currency_states newer where newer.system_id=currency_states.system_id and newer.source=currency_states.source and newer.currency_id=currency_states.currency_id and newer.state_kind=currency_states.state_kind)",[])?;
        Ok(())
    }

    pub(super) fn collect_blobs(tx: &Transaction<'_>, limit: usize) -> Result<()> {
        tx.execute("delete from raw_blobs where hash in(select hash from raw_blobs b
            where not exists(select 1 from raw_rpc_cache where payload_hash=b.hash)
            and not exists(select 1 from objects where payload_hash=b.hash)
            and not exists(select 1 from object_observations where payload_hash=b.hash or body_hash=b.hash)
            and not exists(select 1 from identity_observations where payload_hash=b.hash or body_hash=b.hash)
            and not exists(select 1 from currency_definitions where definition_hash=b.hash)
            and not exists(select 1 from currency_definition_observations where definition_hash=b.hash)
            and not exists(select 1 from currency_states where payload_hash=b.hash or state_hash=b.hash)
            and not exists(select 1 from relationships where payload_hash=b.hash) limit ?1)", [i64::try_from(limit)?])?;
        Ok(())
    }
}
