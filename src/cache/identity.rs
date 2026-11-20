//! Version-qualified identity evidence, stored separately from transaction links.
use super::*;

pub const MAX_EDGES: usize = 512;

#[derive(Debug, Clone)]
pub struct EvidenceEdge {
    pub relation: String,
    pub target_kind: String,
    pub target_id: String,
    pub detail: String,
    pub evidence: String,
}

#[derive(Debug, Clone)]
pub struct EvidenceRecord {
    pub system: String,
    pub identity: String,
    pub version_txid: String,
    pub version_height: u64,
    pub source_id: String,
    pub block_hash: Option<String>,
    pub registration: Option<String>,
    pub notes: Vec<String>,
    pub edges: Vec<EvidenceEdge>,
    pub observed_at: i64,
    pub membership: String,
}

impl Cache {
    pub async fn identity_evidence(
        &self,
        system: String,
        identity: String,
        version_txid: String,
        version_height: u64,
    ) -> Result<Option<EvidenceRecord>> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::IdentityEvidenceRead {
                system,
                identity,
                version_txid,
                version_height,
                reply,
            })
        })
        .await?
    }

    pub async fn record_identity_evidence(&self, record: EvidenceRecord) -> Result<()> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::IdentityEvidenceWrite { record, reply })
        })
        .await?
    }
}

impl StoreConnection {
    pub(super) fn identity_evidence(
        &self,
        system: &str,
        identity: &str,
        version_txid: &str,
        version_height: u64,
    ) -> Result<Option<EvidenceRecord>> {
        let metadata = self.connection.query_row(
            "select r.version_txid,r.version_height,r.source_id,r.block_hash,r.registration,
                    r.notes_json,r.observed_at,
                    coalesce((select membership from block_membership b where
                        b.system_id=r.system_id and b.source=r.source_id and b.block_hash=r.block_hash),
                        'unverified')
             from identity_evidence_reports r where r.system_id=?1 and r.identity_id=?2",
            params![system, identity],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        ).optional()?;
        let Some((
            stored_txid,
            stored_height,
            source_id,
            block_hash,
            registration,
            notes_json,
            observed_at,
            membership,
        )) = metadata
        else {
            return Ok(None);
        };
        if stored_txid != version_txid
            || stored_height != i64::try_from(version_height)?
            || membership == "orphan"
        {
            return Ok(None);
        }
        let mut statement = self.connection.prepare(
            "select relation,target_kind,target_id,detail,evidence from identity_evidence_edges
             where system_id=?1 and identity_id=?2 order by ordinal limit ?3",
        )?;
        let edges = statement
            .query_map(
                params![system, identity, i64::try_from(MAX_EDGES)?],
                |row| {
                    Ok(EvidenceEdge {
                        relation: row.get(0)?,
                        target_kind: row.get(1)?,
                        target_id: row.get(2)?,
                        detail: row.get(3)?,
                        evidence: row.get(4)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(EvidenceRecord {
            system: system.into(),
            identity: identity.into(),
            version_txid: stored_txid,
            version_height,
            source_id,
            block_hash,
            registration,
            notes: serde_json::from_str(&notes_json)?,
            edges,
            observed_at,
            membership,
        }))
    }

    pub(super) fn record_identity_evidence(&self, record: &EvidenceRecord) -> Result<()> {
        anyhow::ensure!(
            record.edges.len() <= MAX_EDGES,
            "too many identity evidence edges"
        );
        if let Some(block_hash) = &record.block_hash {
            let orphaned: bool = self.connection.query_row(
                "select exists(select 1 from block_membership where system_id=?1 and source=?2
                    and block_hash=?3 and membership='orphan')",
                params![record.system, record.source_id, block_hash],
                |row| row.get(0),
            )?;
            anyhow::ensure!(!orphaned, "identity evidence anchor is orphaned");
        }
        let tx = self.connection.unchecked_transaction()?;
        tx.execute(
            "insert into identity_evidence_reports values (?1,?2,?3,?4,?5,?6,?7,?8,unixepoch())
             on conflict(system_id,identity_id) do update set
               version_txid=excluded.version_txid,version_height=excluded.version_height,
               source_id=excluded.source_id,block_hash=excluded.block_hash,
               registration=excluded.registration,notes_json=excluded.notes_json,
               observed_at=excluded.observed_at",
            params![
                record.system,
                record.identity,
                record.version_txid,
                i64::try_from(record.version_height)?,
                record.source_id,
                record.block_hash,
                record.registration,
                serde_json::to_string(&record.notes)?
            ],
        )?;
        tx.execute(
            "delete from identity_evidence_edges where system_id=?1 and identity_id=?2",
            params![record.system, record.identity],
        )?;
        for (ordinal, edge) in record.edges.iter().enumerate() {
            tx.execute(
                "insert into identity_evidence_edges values (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![
                    record.system,
                    record.identity,
                    i64::try_from(ordinal)?,
                    edge.relation,
                    edge.target_kind,
                    edge.target_id,
                    edge.detail,
                    edge.evidence
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_edges_survive_restart_and_orphaned_anchor_is_not_reused() {
        let root = std::env::temp_dir().join(format!(
            "identity-evidence-{}-{}",
            std::process::id(),
            crate::rpc::next_request_order()
        ));
        let path = root.join("mainnet.sqlite3");
        let store = StoreConnection::open(&path).unwrap();
        let record = EvidenceRecord {
            system: "iSystem".into(),
            identity: "iSeed".into(),
            version_txid: "version".into(),
            version_height: 10,
            source_id: "local".into(),
            block_hash: Some("block-a".into()),
            registration: Some("Seed@".into()),
            notes: vec!["candidate only".into()],
            edges: vec![EvidenceEdge {
                relation: "Registration funding candidate".into(),
                target_kind: "address".into(),
                target_id: "RFunder".into(),
                detail: "input 0".into(),
                evidence: "funding:1".into(),
            }],
            observed_at: 0,
            membership: "unverified".into(),
        };
        store.record_identity_evidence(&record).unwrap();
        drop(store);
        let store = StoreConnection::open(&path).unwrap();
        let reopened = store
            .identity_evidence("iSystem", "iSeed", "version", 10)
            .unwrap()
            .unwrap();
        assert_eq!(reopened.edges[0].target_kind, "address");
        assert_eq!(reopened.edges[0].evidence, "funding:1");
        assert!(
            store
                .identity_evidence("iSystem", "iSeed", "different", 10)
                .unwrap()
                .is_none()
        );
        store
            .connection
            .execute(
                "insert into block_membership values (?1,?2,?3,?4,'orphan',unixepoch())",
                params!["iSystem", "local", "block-a", 10],
            )
            .unwrap();
        assert!(
            store
                .identity_evidence("iSystem", "iSeed", "version", 10)
                .unwrap()
                .is_none()
        );
        assert!(store.record_identity_evidence(&record).is_err());
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
