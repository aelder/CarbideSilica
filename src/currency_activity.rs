//! Explicit bounded discovery. Import/export records are batch evidence;
//! individual transfer outpoints come only from a verified export input range.
use crate::{
    cache::{BlockContext, Cache},
    config::Config,
    rpc::{BlockId, RpcClient},
    world::{OutpointRef, SystemId},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const WINDOW: u64 = 1_000;
pub const MAX_RECORDS: usize = 80;
pub const MAX_INPUTS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub first: u64,
    pub last: u64,
}
impl Range {
    pub fn ending_at(last: u64) -> Self {
        Self {
            first: last.saturating_sub(WINDOW - 1),
            last,
        }
    }
    pub fn older(self) -> Option<Self> {
        self.first.checked_sub(1).map(Self::ending_at)
    }
    pub fn newer(self, tip: u64) -> Option<Self> {
        (self.last < tip).then(|| Self {
            first: self.last + 1,
            last: self.last.saturating_add(WINDOW).min(tip),
        })
    }
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.first <= self.last && self.last - self.first < WINDOW,
            "activity range exceeds {WINDOW} blocks"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Import,
    Export,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub height: u64,
    pub kind: Kind,
    pub output: OutpointRef,
    pub export: Option<OutpointRef>,
    pub transfer_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub system: String,
    pub currency: String,
    pub source: String,
    pub anchor: BlockContext,
    pub range: Range,
    pub records: Vec<Record>,
    pub truncated: bool,
}

#[derive(Debug, Clone)]
pub struct StoredPage {
    pub page: Page,
    pub membership: String,
}

fn txid(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|id| id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()))
}

pub fn records(
    system: &SystemId,
    range: Range,
    imports: &[Value],
    exports: &[Value],
) -> (Vec<Record>, bool) {
    let mut records = Vec::new();
    for (kind, items, height_key, tx_key, output_key) in [
        (
            Kind::Import,
            imports,
            "importheight",
            "importtxid",
            "importvout",
        ),
        (Kind::Export, exports, "height", "txid", "txoutnum"),
    ] {
        for item in items {
            let Some(height) = item[height_key]
                .as_u64()
                .filter(|height| (range.first..=range.last).contains(height))
            else {
                continue;
            };
            let Some(id) = txid(&item[tx_key]) else {
                continue;
            };
            let Some(vout) = item[output_key].as_u64().and_then(|n| n.try_into().ok()) else {
                continue;
            };
            let output = OutpointRef {
                system_id: system.clone(),
                txid: id.into(),
                vout,
            };
            let export = if kind == Kind::Export {
                Some(output.clone())
            } else {
                let info = &item["import"];
                info["sourcesystemid"]
                    .as_str()
                    .and_then(|id| SystemId::try_from(id.to_owned()).ok())
                    .zip(txid(&info["exporttxid"]))
                    .zip(
                        info["exporttxout"]
                            .as_u64()
                            .and_then(|n| u32::try_from(n).ok()),
                    )
                    .map(|((system_id, txid), vout)| OutpointRef {
                        system_id,
                        txid: txid.into(),
                        vout,
                    })
            };
            let info = &item[if kind == Kind::Import {
                "import"
            } else {
                "exportinfo"
            }];
            records.push(Record {
                height,
                kind,
                output,
                export,
                transfer_hash: txid(&info["hashtransfers"]).map(str::to_owned),
            });
        }
    }
    records.sort_by(|a, b| {
        b.height
            .cmp(&a.height)
            .then(a.output.txid.cmp(&b.output.txid))
            .then(a.output.vout.cmp(&b.output.vout))
            .then((a.kind as u8).cmp(&(b.kind as u8)))
    });
    records.dedup();
    let truncated = records.len() > MAX_RECORDS;
    records.truncate(MAX_RECORDS);
    (records, truncated)
}

pub async fn load(
    config: Config,
    currency: String,
    range: Range,
    source: String,
    anchor: BlockContext,
) -> Result<Page> {
    range.validate()?;
    ensure!(
        range.last <= anchor.height,
        "activity range exceeds its anchor"
    );
    let client = RpcClient::for_config(&config.noninteractive())?;
    let before = client.get_block(BlockId::Height(anchor.height), 1).await?;
    ensure!(
        before.source_id == source
            && before.result.hash == anchor.hash
            && before.result.height == anchor.height,
        "activity anchor changed; refresh the currency state"
    );
    let params = [json!(currency), json!(range.first), json!(range.last)];
    let imports = client
        .call_nullable_with_raw::<Vec<Value>>("getimports", &params)
        .await?;
    let exports = client
        .call_nullable_with_raw::<Vec<Value>>("getexports", &params)
        .await?;
    let after = client.get_block(BlockId::Height(anchor.height), 1).await?;
    ensure!(
        imports.source_id == source
            && exports.source_id == source
            && after.source_id == source
            && after.result.hash == anchor.hash
            && after.result.height == anchor.height,
        "activity source or branch changed during lookup"
    );
    let system = config
        .chain
        .system_id
        .get()
        .context("activity system unresolved")?;
    let (records, truncated) = records(
        system,
        range,
        imports.result.as_deref().unwrap_or_default(),
        exports.result.as_deref().unwrap_or_default(),
    );
    let page = Page {
        system: system.to_string(),
        currency,
        source,
        anchor,
        range,
        records,
        truncated,
    };
    let cache = Cache::open(&config.cache_path)?;
    cache
        .record_currency_activity(
            page.clone(),
            after
                .receipt
                .context("activity lookup lacked storage receipt")?,
        )
        .await?;
    Ok(page)
}

/// Only the export's declared input slice supplies candidates. Loading the
/// selected source output later must still verify it is a reserve transfer.
pub fn export_inputs(record: &Record, transaction: &Value) -> Result<(Vec<OutpointRef>, bool)> {
    let target = record
        .export
        .as_ref()
        .context("batch has no source export reference")?;
    ensure!(
        transaction["txid"].as_str() == Some(&target.txid),
        "export transaction ID mismatch"
    );
    ensure!(
        transaction["confirmations"].as_i64().is_none_or(|n| n > 0),
        "export is unconfirmed or outside the active chain"
    );
    let info = transaction["vout"]
        .as_array()
        .and_then(|outs| outs.get(target.vout as usize))
        .and_then(|out| out["scriptPubKey"].get("crosschainexport"))
        .context("referenced output is not an export")?;
    ensure!(
        info["sourcesystemid"].as_str() == Some(target.system_id.as_ref()),
        "export source system mismatch"
    );
    if let Some(hash) = &record.transfer_hash {
        ensure!(
            info["hashtransfers"].as_str() == Some(hash),
            "export transfer hash differs from activity evidence"
        );
    }
    let first = info["firstinput"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .context("export first input missing")?;
    let count = info["numinputs"]
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .context("export input count missing")?;
    let end = first
        .checked_add(count)
        .context("export input range overflow")?;
    let inputs = transaction["vin"]
        .as_array()
        .and_then(|inputs| inputs.get(first..end))
        .context("export input range exceeds transaction inputs")?;
    let mut outputs = Vec::new();
    for input in inputs.iter().take(MAX_INPUTS) {
        outputs.push(OutpointRef {
            system_id: target.system_id.clone(),
            txid: txid(&input["txid"])
                .context("invalid export input transaction")?
                .into(),
            vout: input["vout"]
                .as_u64()
                .and_then(|n| n.try_into().ok())
                .context("invalid export input output index")?,
        });
    }
    Ok((outputs, count > MAX_INPUTS))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_are_bounded_contiguous_and_stop_at_genesis_or_anchor() {
        let range = Range::ending_at(3962166);
        assert_eq!(range.last - range.first + 1, WINDOW);
        let older = range.older().unwrap();
        assert_eq!(older.last + 1, range.first);
        assert_eq!(older.newer(range.last), Some(range));
        assert!(range.newer(range.last).is_none());
        assert!(Range::ending_at(0).older().is_none());
        assert!(
            Range {
                first: 0,
                last: 1000
            }
            .validate()
            .is_err()
        );
        assert!(Range { first: 2, last: 1 }.validate().is_err());
    }
    #[test]
    fn activity_keeps_system_qualified_outpoints_and_import_export_direction() {
        let system = SystemId::try_from("iDestination".to_string()).unwrap();
        let import = json!({"importheight":10,"importtxid":"a".repeat(64),"importvout":3,
            "import":{"sourcesystemid":"iSource","exporttxid":"b".repeat(64),"exporttxout":7}});
        let export = json!({"height":9,"txid":"c".repeat(64),"txoutnum":4});
        let outside = json!({"height":11,"txid":"d".repeat(64),"txoutnum":0});
        let (rows, truncated) = records(
            &system,
            Range { first: 9, last: 10 },
            &[import.clone(), import],
            &[
                export,
                outside,
                json!({"height":9,"txid":"bad","txoutnum":0}),
            ],
        );
        assert!(!truncated);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].output.vout, 3);
        assert_eq!(rows[0].output.system_id, system);
        assert_eq!(
            rows[0].export.as_ref().unwrap().system_id.as_ref(),
            "iSource"
        );
        assert_eq!(rows[0].export.as_ref().unwrap().vout, 7);
        assert_eq!(rows[1].kind, Kind::Export);
        assert_eq!(rows[1].export.as_ref(), Some(&rows[1].output));
        let many: Vec<_> = (0..MAX_RECORDS + 1)
            .map(|n| json!({"height":10,"txid":format!("{n:064x}"),"txoutnum":0}))
            .collect();
        let (rows, truncated) = records(
            &system,
            Range {
                first: 10,
                last: 10,
            },
            &[],
            &many,
        );
        assert!(truncated);
        assert_eq!(rows.len(), MAX_RECORDS);
    }
    #[test]
    fn transfer_candidates_use_only_the_exact_export_input_range() {
        let system = SystemId::try_from("iSource".to_string()).unwrap();
        let record = Record {
            height: 10,
            kind: Kind::Export,
            output: OutpointRef {
                system_id: system,
                txid: "b".repeat(64),
                vout: 1,
            },
            export: None,
            transfer_hash: Some("hash".into()),
        };
        let record = Record {
            export: Some(record.output.clone()),
            ..record
        };
        let tx = json!({"txid":"b".repeat(64),"confirmations":1,"vin":[{"coinbase":"00"},{"txid":"a".repeat(64),"vout":7},{"txid":"c".repeat(64),"vout":2}],
            "vout":[{}, {"scriptPubKey":{"crosschainexport":{"sourcesystemid":"iSource","firstinput":1,"numinputs":1,"hashtransfers":"hash"}}}]});
        let (inputs, truncated) = export_inputs(&record, &tx).unwrap();
        assert!(!truncated);
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].vout, 7);
        for (path, value) in [
            ("/txid", json!("wrong")),
            ("/confirmations", json!(0)),
            ("/vout/1/scriptPubKey/crosschainexport/numinputs", json!(3)),
            (
                "/vout/1/scriptPubKey/crosschainexport/hashtransfers",
                json!("wrong"),
            ),
            (
                "/vout/1/scriptPubKey/crosschainexport/sourcesystemid",
                json!("iWrong"),
            ),
        ] {
            let mut changed = tx.clone();
            *changed.pointer_mut(path).unwrap() = value;
            assert!(export_inputs(&record, &changed).is_err(), "accepted {path}");
        }
    }
}
