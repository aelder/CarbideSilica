//! Cache-only investigation snapshots. A worker command reads a consistent database view.
use super::*;

pub const PAGE_SIZE: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Investigation {
    Inclusions(String),
    Children(String),
}

#[derive(Debug, Clone)]
pub struct InvestigationRow {
    pub hash: String,
    pub source: String,
    pub height: Option<u64>,
    pub timestamp: Option<u64>,
    pub membership: String,
    pub observed_at: Option<i64>,
    pub cached: bool,
}

#[derive(Debug, Clone)]
pub struct InvestigationPage {
    pub rows: Vec<InvestigationRow>,
    pub more: bool,
}

impl Cache {
    pub async fn investigate(
        &self,
        system: String,
        subject: Investigation,
        page: usize,
    ) -> Result<InvestigationPage> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::Investigate {
                system,
                subject,
                page,
                reply,
            })
        })
        .await?
    }
}

impl StoreConnection {
    pub(super) fn investigate(
        &self,
        system: &str,
        subject: &Investigation,
        page: usize,
    ) -> Result<InvestigationPage> {
        // Membership is joined on both source and system. A negative/absent inclusion
        // flag alone does not prove an orphan; only explicit block membership does.
        let (target, candidates) = match subject {
            Investigation::Inclusions(txid) => (
                txid,
                "select block_hash,source,height,observed_at from transaction_inclusions where system_id=?1 and txid=?2 and block_hash<>''",
            ),
            Investigation::Children(parent) => (
                parent,
                "
                select h.block_hash,b.source,h.height,b.observed_at
                from reconciliation_headers h left join block_membership b
                  on b.system_id=h.system_id and b.block_hash=h.block_hash
                where h.system_id=?1 and h.parent_hash=?2
                  and (b.source is not null or not exists(
                    select 1 from object_observations o where o.system_id=h.system_id
                      and o.kind='block' and o.object_id=h.block_hash))
                union
                select o.object_id,o.source,json_extract(v.payload,'$.height'),o.observed_at
                from object_observations o join raw_blobs v on v.hash=o.body_hash
                where o.system_id=?1 and o.kind='block'
                  and json_extract(v.payload,'$.previousblockhash')=?2",
            ),
        };
        let sql = format!(
            "
            with candidates as ({candidates}), grouped as (
                select block_hash,source,max(height) height,max(observed_at) observed_at
                from candidates group by block_hash,source)
            select g.block_hash,coalesce(g.source,'Unknown source'),coalesce(b.height,g.height),
                coalesce(b.membership,'unverified'),coalesce(b.observed_at,g.observed_at),
                exists(select 1 from objects o join raw_blobs v on v.hash=o.payload_hash
                       where o.system_id=?1 and o.kind='block' and o.object_id=g.block_hash
                       and json_type(v.payload,'$.tx')='array'),
                coalesce((select json_extract(v.payload,'$.time') from objects o
                    join raw_blobs v on v.hash=o.payload_hash
                    where o.system_id=?1 and o.kind='block' and o.object_id=g.block_hash),
                    (select block_time from reconciliation_headers h
                    where h.system_id=?1 and h.block_hash=g.block_hash))
            from grouped g left join block_membership b
              on b.system_id=?1 and b.source=g.source and b.block_hash=g.block_hash
            order by g.height,g.block_hash,g.source limit ?3 offset ?4"
        );
        let mut statement = self.connection.prepare(&sql)?;
        let mut rows = statement
            .query_map(
                params![
                    system,
                    target,
                    (PAGE_SIZE + 1) as i64,
                    i64::try_from(page.saturating_mul(PAGE_SIZE))?
                ],
                |r| {
                    Ok(InvestigationRow {
                        hash: r.get(0)?,
                        source: r.get(1)?,
                        height: r
                            .get::<_, Option<i64>>(2)?
                            .and_then(|h| u64::try_from(h).ok()),
                        membership: r.get(3)?,
                        observed_at: r.get(4)?,
                        cached: r.get(5)?,
                        timestamp: r
                            .get::<_, Option<i64>>(6)?
                            .and_then(|t| u64::try_from(t).ok()),
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = rows.len() > PAGE_SIZE;
        rows.truncate(PAGE_SIZE);
        Ok(InvestigationPage { rows, more })
    }
}
