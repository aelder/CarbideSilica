use super::*;
use crate::currency_activity::{MAX_RECORDS, Page, StoredPage};

impl Cache {
    pub fn currency_activity(&self, system: &str, currency: &str) -> Result<Vec<StoredPage>> {
        self.request(|reply| Command::CurrencyActivityRead {
            system: system.into(),
            currency: currency.into(),
            reply,
        })
    }
    pub async fn record_currency_activity(&self, page: Page, receipt: Receipt) -> Result<()> {
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            cache.request(|reply| Command::CurrencyActivityWrite {
                page,
                receipt,
                reply,
            })
        })
        .await?
    }
}

impl StoreConnection {
    pub(super) fn record_currency_activity(&self, page: &Page, order: u64) -> Result<()> {
        page.range.validate()?;
        anyhow::ensure!(
            page.range.last <= page.anchor.height && page.records.len() <= MAX_RECORDS,
            "invalid currency activity page"
        );
        let tx = self.connection.unchecked_transaction()?;
        tx.execute("insert into currency_activity_pages values(?1,?2,?3,?4,?5,?6,?7,?8,?9,unixepoch())
            on conflict(system_id,currency_id,source,anchor_hash,first_height,last_height)
            do update set page_json=excluded.page_json,request_order=excluded.request_order,observed_at=excluded.observed_at
            where excluded.request_order>currency_activity_pages.request_order",
            params![page.system,page.currency,page.source,i64::try_from(page.anchor.height)?,page.anchor.hash,
                i64::try_from(page.range.first)?,i64::try_from(page.range.last)?,serde_json::to_string(page)?,i64::try_from(order)?])?;
        // Keep a bounded window index per currency and observing system. This
        // includes empty pages so absence within an examined range survives restart.
        tx.execute("delete from currency_activity_pages where system_id=?1 and currency_id=?2 and rowid not in
            (select rowid from currency_activity_pages where system_id=?1 and currency_id=?2 order by request_order desc limit 32)",
            params![page.system,page.currency])?;
        tx.commit()?;
        Ok(())
    }
    pub(super) fn currency_activity(
        &self,
        system: &str,
        currency: &str,
    ) -> Result<Vec<StoredPage>> {
        let mut statement = self.connection.prepare("select p.page_json,coalesce(
            (select membership from block_membership b where b.system_id=p.system_id and b.source=p.source and b.block_hash=p.anchor_hash),'unverified')
            from currency_activity_pages p where p.system_id=?1 and p.currency_id=?2 order by p.request_order desc limit 8")?;
        statement
            .query_map(params![system, currency], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .map(|row| {
                let (json, membership) = row?;
                Ok(StoredPage {
                    page: serde_json::from_str(&json)?,
                    membership,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::currency_activity::Range;
    #[tokio::test]
    async fn activity_writer_rechecks_permission_branch_and_cache_epochs() {
        let root = std::env::temp_dir().join(format!(
            "currency-activity-receipts-{}",
            crate::rpc::next_request_order()
        ));
        let path = root.join("mainnet.sqlite3");
        let cache = Cache::open(&path).unwrap();
        let control = Arc::new(crate::sources::Control::default());
        let receipt = Receipt {
            branch_epoch: 0,
            control: control.clone(),
            generation: control.generation(),
            order: 1,
            epoch: cache.epoch(),
            once: false,
        };
        let page = Page {
            system: "iSystem".into(),
            currency: "iPool".into(),
            source: "local".into(),
            anchor: BlockContext {
                height: 0,
                hash: "block".into(),
            },
            range: Range::ending_at(0),
            records: Vec::new(),
            truncated: false,
        };
        cache
            .record_currency_activity(page.clone(), receipt.clone())
            .await
            .unwrap();
        control.set(crate::sources::ConnectionState::Disabled);
        control.set(crate::sources::ConnectionState::Enabled);
        assert!(
            cache
                .record_currency_activity(page.clone(), receipt.clone())
                .await
                .is_err()
        );
        let receipt = Receipt {
            generation: control.generation(),
            ..receipt
        };
        control.branch_epoch.store(1, Ordering::Release);
        assert!(
            cache
                .record_currency_activity(page.clone(), receipt.clone())
                .await
                .is_err()
        );
        let receipt = Receipt {
            branch_epoch: 1,
            ..receipt
        };
        Cache::clear_at(&path).unwrap();
        assert!(cache.record_currency_activity(page, receipt).await.is_err());
        assert!(
            cache
                .currency_activity("iSystem", "iPool")
                .unwrap()
                .is_empty()
        );
        drop(cache);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn activity_survives_restart_is_source_qualified_and_keeps_empty_windows() {
        let root = std::env::temp_dir().join(format!(
            "currency-activity-{}",
            crate::rpc::next_request_order()
        ));
        let path = root.join("mainnet.sqlite3");
        let store = StoreConnection::open(&path).unwrap();
        let mut page = Page {
            system: "iSystem".into(),
            currency: "iPool".into(),
            source: "local".into(),
            anchor: BlockContext {
                height: 1000,
                hash: "anchor".into(),
            },
            range: Range::ending_at(1000),
            records: Vec::new(),
            truncated: false,
        };
        store.record_currency_activity(&page, 2).unwrap();
        page.truncated = true;
        store.record_currency_activity(&page, 1).unwrap();
        page.source = "api".into();
        page.truncated = false;
        store.record_currency_activity(&page, 3).unwrap();
        drop(store);
        let store = StoreConnection::open(&path).unwrap();
        let pages = store.currency_activity("iSystem", "iPool").unwrap();
        assert_eq!(pages.len(), 2);
        assert!(
            pages
                .iter()
                .all(|p| p.page.records.is_empty() && !p.page.truncated)
        );
        assert!(
            store
                .currency_activity("iOther", "iPool")
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .currency_activity("iSystem", "iOther")
                .unwrap()
                .is_empty()
        );
        store
            .connection
            .execute(
                "insert into block_membership values('iSystem','local','anchor',1000,'orphan',123)",
                [],
            )
            .unwrap();
        assert_eq!(
            store
                .currency_activity("iSystem", "iPool")
                .unwrap()
                .iter()
                .find(|p| p.page.source == "local")
                .unwrap()
                .membership,
            "orphan"
        );
        for order in 4..40 {
            page.range = Range::ending_at(order);
            store.record_currency_activity(&page, order).unwrap();
        }
        let count: i64 = store
            .connection
            .query_row("select count(*) from currency_activity_pages", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 32);
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn schema_five_upgrades_activity_index_without_losing_existing_data() {
        let root = std::env::temp_dir().join(format!(
            "currency-activity-upgrade-{}",
            crate::rpc::next_request_order()
        ));
        let path = root.join("mainnet.sqlite3");
        let store = StoreConnection::open(&path).unwrap();
        store.connection.execute_batch("insert into significant_events values('keep','existing',123); drop table currency_activity_pages; pragma user_version=5;").unwrap();
        drop(store);
        let store = StoreConnection::open(&path).unwrap();
        let version: i64 = store
            .connection
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert!(
            store
                .currency_activity("iSystem", "iPool")
                .unwrap()
                .is_empty()
        );
        let count: i64 = store
            .connection
            .query_row(
                "select count(*) from significant_events where event_key='keep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        store.clear().unwrap();
        drop(store);
        fs::remove_dir_all(root).unwrap();
    }
}
