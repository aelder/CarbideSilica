//! Source and block qualified lifecycle edges for inspected DeFi transfers.
use super::*;
use crate::world::{OutpointRef, SystemId};

#[derive(Debug, Clone)]
pub struct StoredLifecycleEdge {
    pub source: OutpointRef,
    pub target: OutpointRef,
    pub relation: String,
    pub resolution: String,
    pub detail: String,
    pub source_id: String,
    pub source_block_hash: String,
    pub source_height: u64,
    pub target_source_id: String,
    pub target_block_hash: String,
    pub target_height: u64,
    pub source_membership: String,
    pub target_membership: String,
}

struct TxContext {
    source: String,
    block_hash: String,
    height: i64,
}

impl Cache {
    pub async fn defi_edges(&self, root: OutpointRef) -> Result<Vec<StoredLifecycleEdge>> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::DefiEdgesRead { root, reply })
        })
        .await?
    }

    pub async fn record_defi_edges(
        &self,
        root: OutpointRef,
        edges: Vec<crate::defi::LifecycleEdge>,
    ) -> Result<()> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::DefiEdgesWrite { root, edges, reply })
        })
        .await?
    }
}

impl StoreConnection {
    fn transaction_context(&self, outpoint: &OutpointRef) -> Result<Option<TxContext>> {
        self.connection
            .query_row(
                "select o.source,json_extract(b.payload,'$.result.blockhash'),
                    json_extract(b.payload,'$.result.height')
             from object_observations o join raw_blobs b on b.hash=o.payload_hash
             where o.system_id=?1 and o.kind='transaction' and o.object_id=?2
               and json_type(b.payload,'$.result.blockhash')='text'
               and json_type(b.payload,'$.result.height')='integer'
             order by o.request_order desc limit 1",
                params![outpoint.system_id.as_ref(), outpoint.txid],
                |row| {
                    Ok(TxContext {
                        source: row.get(0)?,
                        block_hash: row.get(1)?,
                        height: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub(super) fn record_defi_edges(
        &self,
        root: &OutpointRef,
        edges: &[crate::defi::LifecycleEdge],
    ) -> Result<()> {
        anyhow::ensure!(edges.len() <= 64, "too many DeFi lifecycle edges");
        let mut qualified = Vec::new();
        for edge in edges {
            if let (Some(source), Some(target)) = (
                self.transaction_context(&edge.source)?,
                self.transaction_context(&edge.target)?,
            ) {
                qualified.push((edge, source, target));
            }
        }
        if !edges.is_empty() && qualified.is_empty() {
            anyhow::bail!("no source/block-qualified DeFi edges were available to retain");
        }
        let tx = self.connection.unchecked_transaction()?;
        for (edge, source, target) in qualified {
            tx.execute(
                "insert into defi_lifecycle_edges values
                 (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,unixepoch())
                 on conflict(root_system_id,root_txid,root_vout,relation,target_system_id,target_txid,target_vout)
                 do update set source_id=excluded.source_id,source_block_hash=excluded.source_block_hash,
                   source_height=excluded.source_height,target_source_id=excluded.target_source_id,
                   target_block_hash=excluded.target_block_hash,target_height=excluded.target_height,
                   resolution=excluded.resolution,detail=excluded.detail,observed_at=excluded.observed_at",
                params![root.system_id.as_ref(), root.txid, i64::from(root.vout), edge.relation,
                    edge.source.system_id.as_ref(), edge.source.txid, i64::from(edge.source.vout),
                    edge.target.system_id.as_ref(), edge.target.txid, i64::from(edge.target.vout),
                    source.source, source.block_hash, source.height,
                    target.source, target.block_hash, target.height,
                    edge.resolution, edge.detail],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(super) fn defi_edges(&self, root: &OutpointRef) -> Result<Vec<StoredLifecycleEdge>> {
        let mut statement = self.connection.prepare(
            "select source_system_id,source_txid,source_vout,target_system_id,target_txid,target_vout,
                relation,resolution,detail,source_id,source_block_hash,source_height,
                target_source_id,target_block_hash,target_height,
                coalesce((select membership from block_membership b where b.system_id=e.source_system_id
                    and b.source=e.source_id and b.block_hash=e.source_block_hash),'unverified'),
                coalesce((select membership from block_membership b where b.system_id=e.target_system_id
                    and b.source=e.target_source_id and b.block_hash=e.target_block_hash),'unverified')
             from defi_lifecycle_edges e where root_system_id=?1 and root_txid=?2 and root_vout=?3
             order by observed_at desc,relation,target_txid,target_vout limit 64",
        )?;
        let rows = statement
            .query_map(
                params![root.system_id.as_ref(), root.txid, i64::from(root.vout)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, i64>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, String>(13)?,
                        row.get::<_, i64>(14)?,
                        row.get::<_, String>(15)?,
                        row.get::<_, String>(16)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|row| {
                let (
                    source_system,
                    source_txid,
                    source_vout,
                    target_system,
                    target_txid,
                    target_vout,
                    relation,
                    resolution,
                    detail,
                    source_id,
                    source_block_hash,
                    source_height,
                    target_source_id,
                    target_block_hash,
                    target_height,
                    source_membership,
                    target_membership,
                ) = row;
                Ok(StoredLifecycleEdge {
                    source: OutpointRef {
                        system_id: SystemId::try_from(source_system)?,
                        txid: source_txid,
                        vout: u32::try_from(source_vout)?,
                    },
                    target: OutpointRef {
                        system_id: SystemId::try_from(target_system)?,
                        txid: target_txid,
                        vout: u32::try_from(target_vout)?,
                    },
                    relation,
                    resolution,
                    detail,
                    source_id,
                    source_block_hash,
                    source_height: u64::try_from(source_height)?,
                    target_source_id,
                    target_block_hash,
                    target_height: u64::try_from(target_height)?,
                    source_membership,
                    target_membership,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lifecycle_edges_keep_candidate_status_and_report_orphaned_blocks() {
        let root_dir = std::env::temp_dir().join(format!(
            "defi-edges-{}-{}",
            std::process::id(),
            crate::rpc::next_request_order()
        ));
        let path = root_dir.join("mainnet.sqlite3");
        let store = StoreConnection::open(&path).unwrap();
        let source_tx = "a".repeat(64);
        let export_tx = "b".repeat(64);
        let import_tx = "c".repeat(64);
        let source_system = SystemId::try_from("iSource".to_string()).unwrap();
        let destination_system = SystemId::try_from("iDestination".to_string()).unwrap();
        for (system, txid, block, height) in [
            ("iSource", &source_tx, "block-a", 1),
            ("iSource", &export_tx, "block-b", 2),
            ("iDestination", &import_tx, "block-c", 3),
        ] {
            store
                .put_raw_rpc(
                    "local",
                    system,
                    "getrawtransaction",
                    &json!([txid, 1]).to_string(),
                    &json!({"result":{"txid":txid,"blockhash":block,"height":height,
                    "vin":[],"vout":[]}})
                    .to_string(),
                )
                .unwrap();
        }
        let root = OutpointRef {
            system_id: source_system.clone(),
            txid: source_tx,
            vout: 1,
        };
        let export = OutpointRef {
            system_id: source_system,
            txid: export_tx,
            vout: 0,
        };
        let imported = OutpointRef {
            system_id: destination_system,
            txid: import_tx,
            vout: 0,
        };
        let edges = vec![
            crate::defi::LifecycleEdge {
                source: root.clone(),
                target: export.clone(),
                relation: "spent-by-export".into(),
                resolution: "structural".into(),
                detail: "input 0".into(),
            },
            crate::defi::LifecycleEdge {
                source: export,
                target: imported,
                relation: "export-import".into(),
                resolution: "candidate".into(),
                detail: "matching batch".into(),
            },
        ];
        store.record_defi_edges(&root, &edges).unwrap();
        drop(store);
        let store = StoreConnection::open(&path).unwrap();
        let read = store.defi_edges(&root).unwrap();
        assert_eq!(read.len(), 2);
        assert!(crate::defi::retained_trail(&root, &read).unwrap().searching);
        assert!(
            read.iter()
                .any(|edge| edge.relation == "export-import" && edge.resolution == "candidate")
        );
        assert!(
            read.iter()
                .all(|edge| edge.target_membership == "unverified")
        );
        store
            .connection
            .execute(
                "insert into block_membership values (?1,?2,?3,?4,'orphan',unixepoch())",
                params!["iDestination", "local", "block-c", 3],
            )
            .unwrap();
        assert!(
            store
                .defi_edges(&root)
                .unwrap()
                .iter()
                .any(|edge| edge.relation == "export-import" && edge.target_membership == "orphan")
        );
        drop(store);
        fs::remove_dir_all(root_dir).unwrap();
    }
}
