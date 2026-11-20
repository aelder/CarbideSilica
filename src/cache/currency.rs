//! Currency definitions are global immutable observations. Reported state heights
//! are not the observing system's tip and never establish freshness on their own.
use super::*;
use rusqlite::Transaction;

impl StoreConnection {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn record_historical_currency(
        &self,
        system: &str,
        currency: &str,
        source: &str,
        block: &BlockContext,
        state: &Value,
        response: &Value,
        order: u64,
    ) -> Result<()> {
        anyhow::ensure!(
            state["currencyid"].as_str() == Some(currency),
            "historical currency ID mismatch"
        );
        let tx = self.connection.unchecked_transaction()?;
        let payload_hash = Self::blob(&tx, response)?;
        let state_hash = Self::blob(&tx, state)?;
        let observation_key = format!("{}:{state_hash}", block.hash);
        tx.execute("insert into currency_states(system_id,source,currency_id,state_kind,observation_key,reported_height,observed_height,observed_block_hash,payload_hash,state_hash,request_order,observed_at)
            values(?1,?2,?3,'historical',?4,?5,?5,?6,?7,?8,?9,unixepoch())
            on conflict(system_id,source,currency_id,state_kind,observation_key) do update set payload_hash=excluded.payload_hash,state_hash=excluded.state_hash,request_order=excluded.request_order,observed_at=excluded.observed_at",
            params![system,source,currency,observation_key,i64::try_from(block.height)?,block.hash,payload_hash,state_hash,i64::try_from(order)?])?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn currency_snapshots(
        &self,
        system: &str,
        currency: &str,
    ) -> Result<Vec<CurrencySnapshot>> {
        let mut statement = self.connection.prepare(
            "select s.source,s.state_kind,s.reported_height,s.observed_height,s.observed_block_hash,
                    state.payload,response.payload,status.height,status.block_hash
             from currency_states s
             join raw_blobs state on state.hash=s.state_hash
             join raw_blobs response on response.hash=s.payload_hash
             left join source_status status on status.system_id=s.system_id and status.source=s.source
             where s.system_id=?1 and s.currency_id=?2
             order by (s.state_kind='historical') asc,s.request_order desc,s.observed_at desc limit 64",
        )?;
        let rows = statement.query_map(params![system, currency], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })?;
        let mut snapshots = Vec::new();
        for row in rows {
            let (source, kind, reported, height, hash, state, response, tip_height, tip_hash) =
                row?;
            let observed = height.zip(hash).and_then(|(height, hash)| {
                Some(BlockContext {
                    height: u64::try_from(height).ok()?,
                    hash,
                })
            });
            let current = observed.as_ref().is_some_and(|block| {
                tip_height == i64::try_from(block.height).ok()
                    && tip_hash.as_deref() == Some(block.hash.as_str())
            });
            snapshots.push(CurrencySnapshot {
                source,
                kind,
                reported_height: reported.and_then(|n| u64::try_from(n).ok()),
                observed,
                current,
                state: serde_json::from_str(&state)?,
                response: serde_json::from_str(&response)?,
            });
        }
        Ok(snapshots)
    }

    pub(super) fn project_currency(
        tx: &Transaction<'_>,
        source: &str,
        system: &str,
        raw: &Value,
        payload: &str,
        order: i64,
    ) -> Result<()> {
        let id = raw["currencyid"].as_str().context("currency ID absent")?;
        if !Self::advance_head(tx, system, source, &format!("currency:{id}"), order)? {
            return Ok(());
        }
        let name = raw["fullyqualifiedname"]
            .as_str()
            .or_else(|| raw["name"].as_str())
            .unwrap_or(id);
        let definition = currency_definition_projection(raw);
        let hash = Self::blob(tx, &definition)?;
        let conflict:Option<String>=tx.query_row("select definition_hash from currency_definitions where currency_id=?1 and definition_hash<>?2 limit 1",params![id,hash],|r|r.get(0)).optional()?;
        if conflict.is_some() {
            tx.execute("insert or replace into significant_events values(?1,?2,unixepoch())",params![format!("currency:{system}:{source}:{id}:{hash}"),format!("Currency definition discrepancy for {id}: observation from {source} on {system} retained alongside earlier definitions")])?;
        }
        tx.execute(
            "insert or ignore into currency_definitions values(?1,?2,?3)",
            params![id, name, hash],
        )?;
        tx.execute("insert into currency_definition_observations values(?1,?2,?3,?4,unixepoch()) on conflict(system_id,source,currency_id,definition_hash) do nothing",params![system,source,id,hash])?;
        for (height_key, key) in [
            ("bestheight", "bestcurrencystate"),
            ("lastconfirmedheight", "lastconfirmedcurrencystate"),
        ] {
            if let Some(state) = raw.get(key).filter(|s| s.is_object()) {
                let state_hash = Self::blob(tx, state)?;
                tx.execute("insert into currency_states(system_id,source,currency_id,state_kind,observation_key,reported_height,payload_hash,state_hash,request_order,observed_at) values(?1,?2,?3,?4,'latest',?5,?6,?7,?8,unixepoch()) on conflict(system_id,source,currency_id,state_kind,observation_key) do update set reported_height=excluded.reported_height,payload_hash=excluded.payload_hash,state_hash=excluded.state_hash,request_order=excluded.request_order,observed_at=excluded.observed_at",params![system,source,id,key,raw[height_key].as_i64(),payload,state_hash,order])?;
            }
        }
        Ok(())
    }

    pub(super) fn qualify_currency(
        &self,
        source: &str,
        system: &str,
        raw: &Value,
        block: &BlockContext,
        order: u64,
    ) -> Result<()> {
        let id = raw["currencyid"].as_str().context("currency ID absent")?;
        let tx = self.connection.unchecked_transaction()?;
        // Same-source before/after observations have already established this anchor.
        // Never attach a later response to an older in-flight observation.
        tx.execute("insert into currency_states(system_id,source,currency_id,state_kind,observation_key,reported_height,observed_height,observed_block_hash,payload_hash,state_hash,request_order,observed_at)
            select system_id,source,currency_id,state_kind,?2||':'||payload_hash,reported_height,?1,?2,payload_hash,state_hash,request_order,observed_at
            from currency_states where system_id=?3 and source=?4 and currency_id=?5 and request_order=?6 and observation_key='latest'
            on conflict(system_id,source,currency_id,state_kind,observation_key) do update set observed_at=excluded.observed_at,request_order=excluded.request_order",params![i64::try_from(block.height)?,block.hash,system,source,id,i64::try_from(order)?])?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn currency_at(
        &self,
        source: &str,
        system: &str,
        id: &str,
        block: &BlockContext,
        ttl: u64,
    ) -> Result<Option<Value>> {
        let current:Option<(i64,String)>=self.connection.query_row("select height,block_hash from source_status where system_id=?1 and source=?2 and height is not null and block_hash is not null",params![system,source],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if current != Some((i64::try_from(block.height)?, block.hash.clone())) {
            return Ok(None);
        }
        let mut statement=self.connection.prepare("select s.observed_height,s.observed_block_hash,b.payload from currency_states s join raw_blobs b on b.hash=s.payload_hash where s.system_id=?1 and s.source=?2 and s.currency_id=?3 and s.observed_height is not null order by s.request_order desc limit 1")?;
        let row: Option<(i64, String, String)> = statement
            .query_row(params![system, source, id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .optional()?;
        let Some((height, hash, raw)) = row else {
            return Ok(None);
        };
        let height = u64::try_from(height)?;
        if height > block.height || block.height - height > ttl {
            return Ok(None);
        };
        // Until ancestry has been reconciled, even a nearby height is insufficient.
        if hash != block.hash {
            return Ok(None);
        }
        let response: Value = serde_json::from_str(&raw)?;
        Ok(Some(currency_current_projection(&response["result"])))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serde_json::json;

    #[test]
    fn historical_states_keep_source_block_and_conflicting_payloads() {
        let store = StoreConnection {
            connection: Connection::open_in_memory().unwrap(),
            _ownership: None,
            writes: Default::default(),
        };
        store.migrate().unwrap();
        let block = BlockContext {
            height: 3_962_166,
            hash: "block-a".into(),
        };
        for supply in [12, 13] {
            let state = json!({"currencyid":"iPool","supply":supply});
            store
                .record_historical_currency(
                    "iVRSC",
                    "iPool",
                    "local",
                    &block,
                    &state,
                    &json!([{"height":block.height,"currencystate":state}]),
                    supply,
                )
                .unwrap();
        }
        store
            .record_historical_currency(
                "iOther",
                "iPool",
                "local",
                &block,
                &json!({"currencyid":"iPool","supply":99}),
                &json!([]),
                1,
            )
            .unwrap();
        let snapshots = store.currency_snapshots("iVRSC", "iPool").unwrap();
        assert_eq!(snapshots.len(), 2);
        assert!(
            snapshots
                .iter()
                .all(|snapshot| snapshot.observed.as_ref() == Some(&block))
        );
        assert!(
            snapshots
                .iter()
                .all(|snapshot| snapshot.kind == "historical")
        );
        assert!(snapshots.iter().all(|snapshot| !snapshot.current));
        assert!(
            snapshots
                .iter()
                .any(|snapshot| snapshot.state["supply"] == 12)
        );
        assert!(
            snapshots
                .iter()
                .any(|snapshot| snapshot.state["supply"] == 13)
        );
        assert!(
            store
                .record_historical_currency(
                    "iVRSC",
                    "iPool",
                    "local",
                    &block,
                    &json!({"currencyid":"iWrong"}),
                    &json!([]),
                    2
                )
                .is_err()
        );
    }
}
