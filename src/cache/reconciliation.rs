use super::*;
use crate::reconciliation::{Change, State};

impl Cache {
    pub async fn reconciliation_state(&self, system: &str, source: &str) -> Result<State> {
        let cache = self.clone();
        let (system, source) = (system.to_owned(), source.to_owned());
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::ReconciliationState {
                system,
                source,
                reply,
            })
        })
        .await?
    }
    pub async fn commit_reconciliation(
        &self,
        system: &str,
        source: &str,
        receipt: Receipt,
        expected: u64,
        change: Change,
    ) -> Result<()> {
        let cache = self.clone();
        let (system, source) = (system.to_owned(), source.to_owned());
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::Reconcile {
                system,
                source,
                receipt,
                expected,
                change,
                reply,
            })
        })
        .await?
    }
}
impl StoreConnection {
    pub(super) fn reconciliation_state(&self, system: &str, source: &str) -> Result<State> {
        self.connection.execute(
            "insert or ignore into reconciled_chain values(?1,?2,0,?3)",
            params![system, source, serde_json::to_string(&State::default())?],
        )?;
        let raw: Option<String> = self
            .connection
            .query_row(
                "select state_json from reconciled_chain where system_id=?1 and source=?2",
                params![system, source],
                |r| r.get(0),
            )
            .optional()?;
        let mut state: State = raw
            .map(|s| serde_json::from_str(&s))
            .transpose()?
            .unwrap_or_default();
        let mut statement = self.connection.prepare("select h.block_hash,h.height,h.parent_hash,h.block_time,b.membership from reconciliation_headers h join block_membership b on b.system_id=h.system_id and b.block_hash=h.block_hash where b.system_id=?1 and b.source=?2 and b.membership in('member','orphan')")?;
        let rows = statement.query_map(params![system, source], |r| {
            Ok((
                crate::reconciliation::Header {
                    hash: r.get(0)?,
                    height: r.get::<_, i64>(1)? as u64,
                    parent: r.get(2)?,
                    time: r.get::<_, i64>(3)? as u64,
                },
                r.get::<_, String>(4)?,
            ))
        })?;
        for row in rows {
            let (h, membership) = row?;
            if membership == "member" {
                state.canonical.insert(h.height, h);
            } else {
                state.detached.insert(h.hash.clone(), h);
            }
        }
        Ok(state)
    }
    pub(super) fn commit_reconciliation(
        &self,
        system: &str,
        source: &str,
        order: u64,
        expected: u64,
        change: &Change,
    ) -> Result<()> {
        let revision: Option<i64> = self
            .connection
            .query_row(
                "select revision from reconciled_chain where system_id=?1 and source=?2",
                params![system, source],
                |r| r.get(0),
            )
            .optional()?;
        anyhow::ensure!(
            revision == Some(i64::try_from(expected)?),
            "reconciled revision changed"
        );
        let observed: Option<(i64, String)> = self
            .connection
            .query_row(
                "select height,block_hash from source_status where system_id=?1 and source=?2",
                params![system, source],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((height, hash)) = observed {
            anyhow::ensure!(
                change.state.tip().is_some_and(
                    |tip| tip.hash == hash && i64::try_from(tip.height).ok() == Some(height)
                ),
                "observed tip changed before reconciliation commit"
            );
        }
        anyhow::ensure!(
            change.state.revision == expected || change.state.revision == expected + 1,
            "invalid reconciliation revision"
        );
        if change.state.revision == expected {
            return Ok(());
        }
        let tx = self.connection.unchecked_transaction()?;
        let order = i64::try_from(order)?;
        for h in change
            .attached
            .iter()
            .chain(change.detached.iter())
            .chain(change.ancestor.iter())
        {
            tx.execute(
                "insert or ignore into reconciliation_headers values(?1,?2,?3,?4,?5)",
                params![
                    system,
                    h.hash,
                    i64::try_from(h.height)?,
                    h.parent,
                    i64::try_from(h.time)?
                ],
            )?;
        }
        if expected == 0 {
            tx.execute("update block_membership set membership='unverified' where system_id=?1 and source=?2 and membership='member'",params![system,source])?;
        }
        for h in &change.detached {
            tx.execute("insert into block_membership values(?1,?2,?3,?4,'orphan',unixepoch()) on conflict(system_id,source,block_hash) do update set membership='orphan',observed_at=unixepoch()",params![system,source,h.hash,i64::try_from(h.height)?])?;
            Self::membership_transition(
                &tx,
                system,
                source,
                "block",
                &h.hash,
                "orphan",
                &h.height.to_string(),
                order,
            )?;
        }
        for h in change.attached.iter().chain(change.ancestor.iter()) {
            tx.execute("insert into block_membership values(?1,?2,?3,?4,'member',unixepoch()) on conflict(system_id,source,block_hash) do update set membership='member',observed_at=unixepoch()",params![system,source,h.hash,i64::try_from(h.height)?])?;
            Self::membership_transition(
                &tx,
                system,
                source,
                "block",
                &h.hash,
                "member",
                &h.height.to_string(),
                order,
            )?;
            tx.execute("insert into transaction_inclusions(system_id,source,txid,block_hash,height,confirmed,observed_at) select ?1,?2,j.value,?3,?4,1,unixepoch() from objects o join raw_blobs b on b.hash=o.payload_hash,json_each(b.payload,'$.tx') j where o.system_id=?1 and o.kind='block' and o.object_id=?3 and j.type='text' on conflict(system_id,source,txid,block_hash) do update set confirmed=1",params![system,source,h.hash,i64::try_from(h.height)?])?;
        }
        tx.execute("update transaction_inclusions set confirmed=exists(select 1 from block_membership b where b.system_id=?1 and b.source=?2 and b.block_hash=transaction_inclusions.block_hash and b.membership='member') where system_id=?1 and source=?2",params![system,source])?;
        // Persist the checkpoint separately so a new tip does not rewrite history.
        let mut checkpoint = State {
            revision: change.state.revision,
            ..Default::default()
        };
        if let Some(tip) = change.state.tip() {
            checkpoint.canonical.insert(tip.height, tip.clone());
        }
        tx.execute("insert into reconciled_chain values(?1,?2,?3,?4) on conflict(system_id,source) do update set revision=excluded.revision,state_json=excluded.state_json",params![system,source,i64::try_from(change.state.revision)?,serde_json::to_string(&checkpoint)?])?;
        if !change.detached.is_empty() {
            let mut message = format!(
                "Reorg on {system}: fork at {}; {} blocks detached, {} adopted ({source})",
                change.ancestor.as_ref().map(|h| h.height).unwrap_or(0),
                change.detached.len(),
                change.attached.len()
            );
            if let Some(previous) = change.detached.iter().max_by_key(|header| header.height) {
                message.push_str(&format!(
                    "\nPrevious tip at height {}\n{}",
                    previous.height, previous.hash
                ));
            }
            if let Some(tip) = change.state.tip() {
                message.push_str(&format!(
                    "\nAdopted tip at height {}\n{}",
                    tip.height, tip.hash
                ));
            }
            tx.execute(
                "insert or replace into significant_events values(?1,?2,unixepoch())",
                params![
                    format!("reorg:{system}:{source}:{}", change.state.revision),
                    message
                ],
            )?;
            // A historical identity RPC is keyed by height, not by branch hash.
            // Keep its version evidence, but invalidate the reusable height alias.
            tx.execute("delete from raw_rpc_cache where system_id=?1 and source=?2 and (retention<>'immutable' or method='getidentity')",params![system,source])?;
            tx.execute(
                "delete from mempool_membership where system_id=?1 and source=?2",
                params![system, source],
            )?;
        }
        tx.execute("delete from significant_events where event_key not in(select event_key from significant_events order by observed_at desc,event_key limit 200)",[])?;
        tx.commit()?;
        Ok(())
    }
}
