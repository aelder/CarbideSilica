//! Per-leg instruction budgets and individually replayed receipts. Never sum
//! forwarding budgets across legs: the next leg spends the same funds.
use super::{OutpointRef, Value, replay};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amount {
    pub currency: String,
    pub satoshis: i128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Observed,
    Unconfirmed,
    Pending,
    Unspent,
    Exported,
    Imported,
    Received,
    Forwarded,
    Refunded,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Self::Observed => "Source observed",
            Self::Unconfirmed => "Unconfirmed source",
            Self::Pending => "Pending export",
            Self::Unspent => "Unspent; export unresolved",
            Self::Exported => "Exported; receipt unresolved",
            Self::Imported => "Imported; receipt unresolved",
            Self::Received => "Received output verified",
            Self::Forwarded => "Forwarded; next instruction verified",
            Self::Refunded => "Refunded output verified",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptKind {
    Received,
    Forwarded,
    Refunded,
}

#[derive(Debug, Clone)]
pub struct Receipt {
    pub output: OutpointRef,
    pub kind: ReceiptKind,
    pub amounts: Vec<Amount>,
    pub next_fee_budget: Option<Amount>,
}

#[derive(Debug, Clone)]
pub struct Execution {
    pub import: OutpointRef,
    pub height: Option<u64>,
    pub state: Value,
    pub previous: Option<OutpointRef>,
    pub source: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExecutionRate {
    pub purpose: &'static str,
    pub net_input: Amount,
    pub output: Amount,
    pub prices: Vec<(String, &'static str, i128)>,
}

#[derive(Debug, Clone)]
pub struct LegSummary {
    pub source: OutpointRef,
    pub stage: Stage,
    pub principal: Option<Vec<Amount>>,
    pub transfer_fee_budget: Option<Amount>,
    pub forwarding_budget: Option<Amount>,
    pub conversion_fee: Option<Amount>,
    pub forwarding_conversion_fee: Option<Amount>,
    pub pool: Option<String>,
    pub execution: Option<Execution>,
    pub receipt: Option<Receipt>,
    pub proceeds: Option<Receipt>,
}

fn amount(value: &Value, currency: &str) -> Option<Amount> {
    let satoshis = replay::amount_sats(value)?;
    (satoshis <= i128::from(i64::MAX)).then(|| Amount {
        currency: currency.into(),
        satoshis,
    })
}

fn amounts(values: &Value) -> Option<Vec<Amount>> {
    let values = values.as_object()?;
    if values.is_empty() {
        return None;
    }
    values.iter().map(|(id, value)| amount(value, id)).collect()
}

impl LegSummary {
    pub(crate) fn new(source: OutpointRef, transfer: &Value) -> Self {
        let fee_currency = transfer["feecurrencyid"].as_str();
        Self {
            source,
            stage: Stage::Observed,
            principal: amounts(&transfer["currencyvalues"]),
            transfer_fee_budget: fee_currency.and_then(|id| amount(&transfer["fees"], id)),
            forwarding_budget: fee_currency
                .and_then(|id| amount(&transfer["destination"]["fees"], id)),
            conversion_fee: None,
            forwarding_conversion_fee: None,
            pool: None,
            execution: None,
            receipt: None,
            proceeds: None,
        }
    }

    pub(crate) fn imported(&mut self, import: OutpointRef, record: &Value) {
        self.stage = Stage::Imported;
        let notarization = &record["importnotarization"];
        let previous = notarization["prevnotarizationtxid"]
            .as_str()
            .filter(|id| id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit()))
            .zip(
                notarization["prevnotarizationout"]
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok()),
            )
            .map(|(txid, vout)| OutpointRef {
                system_id: import.system_id.clone(),
                txid: txid.into(),
                vout,
            });
        self.execution = Some(Execution {
            import,
            height: record["importheight"].as_u64(),
            state: notarization["currencystate"].clone(),
            previous,
            source: None,
        });
        let pool = record["import"]["importcurrencyid"].as_str();
        let state = &record["importnotarization"]["currencystate"];
        if pool.is_some()
            && state["currencyid"].as_str() == pool
            && state["flags"].as_u64().is_some_and(|flags| flags & 1 != 0)
        {
            self.pool = pool.map(str::to_owned);
        }
    }

    pub fn currency_ids(&self) -> Vec<String> {
        let mut ids = std::collections::BTreeSet::new();
        ids.extend(self.pool.iter().cloned());
        if let Some(execution) = &self.execution {
            ids.extend(
                execution.state["reservecurrencies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|reserve| reserve["currencyid"].as_str())
                    .take(12)
                    .map(str::to_owned),
            );
        }
        for amount in self
            .principal
            .iter()
            .flatten()
            .chain(self.transfer_fee_budget.iter())
            .chain(self.forwarding_budget.iter())
            .chain(self.conversion_fee.iter())
            .chain(self.forwarding_conversion_fee.iter())
            .chain(self.receipt.iter().flat_map(|receipt| &receipt.amounts))
            .chain(
                self.receipt
                    .iter()
                    .flat_map(|receipt| receipt.next_fee_budget.iter()),
            )
        {
            ids.insert(amount.currency.clone());
        }
        ids.into_iter().collect()
    }

    /// Replayed amounts supply the net execution ratio; named prices are the
    /// import batch's builder fields, not current market quotes.
    pub fn execution_rates(&self) -> Vec<ExecutionRate> {
        let mut rates = Vec::new();
        let (Some(execution), Some(pool), Some(receipt)) =
            (&self.execution, &self.pool, &self.receipt)
        else {
            return rates;
        };
        let mut add = |purpose, input: &Amount, fee: &Amount, output: &Amount| {
            let Some(net) = input
                .satoshis
                .checked_sub(fee.satoshis)
                .filter(|net| *net > 0)
            else {
                return;
            };
            if input.currency == output.currency || input.currency != fee.currency {
                return;
            }
            let mut prices = Vec::new();
            let mut price = |currency: &str, field: &'static str| {
                if let Some(value) =
                    replay::amount_sats(&execution.state["currencies"][currency][field])
                        .filter(|value| *value > 0)
                {
                    prices.push((currency.to_owned(), field, value));
                }
            };
            if &input.currency != pool {
                price(&input.currency, "lastconversionprice");
            }
            if &output.currency != pool {
                price(
                    &output.currency,
                    if &input.currency == pool {
                        "lastconversionprice"
                    } else {
                        "viaconversionprice"
                    },
                );
            }
            rates.push(ExecutionRate {
                purpose,
                net_input: Amount {
                    currency: input.currency.clone(),
                    satoshis: net,
                },
                output: output.clone(),
                prices,
            });
        };
        if let (Some(inputs), Some(fee)) = (&self.principal, &self.conversion_fee)
            && inputs.len() == 1
            && receipt.amounts.len() == 1
        {
            add("Principal", &inputs[0], fee, &receipt.amounts[0]);
        }
        if let (Some(input), Some(fee), Some(output)) = (
            &self.forwarding_budget,
            &self.forwarding_conversion_fee,
            &receipt.next_fee_budget,
        ) {
            add("Forward budget", input, fee, output);
        }
        rates
    }

    pub(super) fn launch_proceeds(&mut self, output: OutpointRef, raw: &Value) {
        let values = if raw["scriptPubKey"].get("reserveoutput").is_some() {
            amounts(&raw["scriptPubKey"]["reserveoutput"]["currencyvalues"])
        } else {
            raw["valueSat"].as_i64().map(|value| {
                vec![Amount {
                    currency: output.system_id.to_string(),
                    satoshis: i128::from(value),
                }]
            })
        };
        self.proceeds = values.map(|amounts| Receipt {
            output,
            amounts,
            kind: ReceiptKind::Received,
            next_fee_budget: None,
        });
    }

    /// Called only after the complete ordered prefix has matched. Legacy
    /// structural direct matches and destination candidates do not call this.
    pub(super) fn replayed(
        &mut self,
        proof: replay::ReplayProof,
        output: OutpointRef,
        raw: &Value,
    ) {
        let forwarded = proof.kind.is_forwarded();
        let next = &raw["scriptPubKey"]["reservetransfer"];
        let result = if forwarded {
            amounts(&next["currencyvalues"])
        } else if raw["scriptPubKey"].get("reserveoutput").is_some() {
            amounts(&raw["scriptPubKey"]["reserveoutput"]["currencyvalues"]).map(|mut values| {
                if let Some(native) = raw["valueSat"].as_i64().filter(|value| *value > 0) {
                    values.push(Amount {
                        currency: output.system_id.to_string(),
                        satoshis: i128::from(native),
                    });
                }
                values
            })
        } else {
            proof.satoshis.map(|satoshis| {
                vec![Amount {
                    currency: output.system_id.to_string(),
                    satoshis,
                }]
            })
        };
        let Some(amounts) = result else { return };
        if proof.kind.is_conversion()
            && let Some(principal) = self.principal.as_ref().filter(|values| values.len() == 1)
        {
            let principal = &principal[0];
            let conversions = proof.kind.conversion_steps();
            self.conversion_fee =
                replay::conversion_fee(principal.satoshis, conversions).map(|satoshis| Amount {
                    currency: principal.currency.clone(),
                    satoshis,
                });
        }
        if forwarded
            && let Some(budget) = &self.forwarding_budget
            && Some(budget.currency.as_str()) != next["feecurrencyid"].as_str()
        {
            let conversions = if Some(budget.currency.as_str()) == self.pool.as_deref() {
                1
            } else {
                2
            };
            self.forwarding_conversion_fee = replay::conversion_fee(budget.satoshis, conversions)
                .map(|satoshis| Amount {
                    currency: budget.currency.clone(),
                    satoshis,
                });
        }
        self.receipt = Some(Receipt {
            output,
            kind: if forwarded {
                ReceiptKind::Forwarded
            } else if proof.kind.is_refund() {
                ReceiptKind::Refunded
            } else {
                ReceiptKind::Received
            },
            amounts,
            next_fee_budget: if forwarded {
                next["feecurrencyid"]
                    .as_str()
                    .and_then(|id| amount(&next["fees"], id))
            } else {
                None
            },
        });
        self.stage = if forwarded {
            Stage::Forwarded
        } else if proof.kind.is_refund() {
            Stage::Refunded
        } else {
            Stage::Received
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reference(txid: &str, vout: u32) -> OutpointRef {
        OutpointRef {
            system_id: crate::world::SystemId::try_from(
                "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV".to_string(),
            )
            .unwrap(),
            txid: txid.into(),
            vout,
        }
    }

    fn replay_fixture(fixture: &Value, selected: usize) -> LegSummary {
        let record = &fixture["record"];
        let batch = record["transfers"].as_array().unwrap();
        let mut leg = LegSummary::new(reference("source", 0), &batch[selected]);
        let import = reference(record["importtxid"].as_str().unwrap(), 0);
        leg.imported(import.clone(), record);
        let outputs = fixture["transaction"]["vout"].as_array().unwrap();
        let proof = replay::replay_supported_prefix(
            batch,
            selected,
            &record["import"],
            &record["importnotarization"],
            import.system_id.as_ref(),
            &outputs[2..],
        )
        .unwrap();
        leg.replayed(
            proof,
            reference(&import.txid, (2 + selected) as u32),
            &outputs[2 + selected],
        );
        leg
    }

    #[test]
    fn pool_buy_summary_separates_principal_conversion_fee_and_explicit_budget() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/vrsc-pool-buy.json")).unwrap();
        let leg = replay_fixture(&fixture, 0);
        let rates = leg.execution_rates();
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].net_input.satoshis, 10_997_250_000);
        assert_eq!(rates[0].output.satoshis, 762_258_126);
        assert_eq!(rates[0].prices[0].1, "lastconversionprice");
        assert_eq!(rates[0].prices[0].2, 1_442_719_942);
        assert_eq!(leg.stage, Stage::Received);
        assert_eq!(leg.principal.unwrap()[0].satoshis, 11_000_000_000);
        assert_eq!(leg.transfer_fee_budget.unwrap().satoshis, 20_010);
        assert_eq!(leg.conversion_fee.unwrap().satoshis, 2_750_000);
        assert_eq!(leg.execution.unwrap().height, Some(3_962_158));
        let receipt = leg.receipt.unwrap();
        assert_eq!(receipt.kind, ReceiptKind::Received);
        assert_eq!(receipt.amounts[0].satoshis, 762_258_126);
        assert_eq!(
            Some(receipt.amounts[0].currency.as_str()),
            leg.pool.as_deref()
        );
        assert!(leg.forwarding_budget.is_none());
    }

    #[test]
    fn gateway_summary_keeps_fee_conversion_and_next_instruction_separate() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/vrsc-gateway-leg.json"))
                .unwrap();
        let leg = replay_fixture(&fixture, 1);
        let rates = leg.execution_rates();
        assert_eq!(rates.len(), 1);
        assert_eq!(rates[0].purpose, "Forward budget");
        assert_eq!(rates[0].net_input.satoshis, 532_498_668);
        assert_eq!(rates[0].output.satoshis, 210_011);
        assert_eq!(rates[0].prices[1].1, "viaconversionprice");
        assert_eq!(leg.stage, Stage::Forwarded);
        assert_eq!(leg.transfer_fee_budget.unwrap().satoshis, 24_012);
        assert!(
            leg.conversion_fee.is_none(),
            "no principal conversion occurred"
        );
        let budget = leg.forwarding_budget.unwrap();
        assert_eq!(budget.satoshis, 532_765_050);
        let fee = leg.forwarding_conversion_fee.unwrap();
        assert_eq!(fee.satoshis, 266_382);
        assert_eq!(fee.currency, budget.currency);
        let receipt = leg.receipt.unwrap();
        assert_eq!(receipt.kind, ReceiptKind::Forwarded);
        assert_eq!(receipt.amounts[0].satoshis, 59_293_245);
        assert_eq!(receipt.next_fee_budget.unwrap().satoshis, 210_011);
        assert_eq!(receipt.output.vout, 3);
    }

    #[test]
    fn forwarded_conversion_records_two_separate_fee_calculations_and_refund_has_no_rate() {
        let mut fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/vrsc-gateway-leg.json"))
                .unwrap();
        let mut conversion = fixture["record"]["transfers"][0].clone();
        conversion["destination"] = fixture["record"]["transfers"][1]["destination"].clone();
        fixture["record"]["transfers"] = json!([conversion]);
        let mut forwarded = fixture["transaction"]["vout"][3].clone();
        forwarded["valueSat"] = json!(1001692152);
        forwarded["scriptPubKey"]["reservetransfer"]["currencyvalues"] =
            json!({"i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV":10.01692152});
        fixture["transaction"]["vout"][2] = forwarded;
        let leg = replay_fixture(&fixture, 0);
        assert_eq!(leg.stage, Stage::Forwarded);
        assert_eq!(leg.conversion_fee.as_ref().unwrap().satoshis, 196);
        assert_eq!(
            leg.forwarding_conversion_fee.as_ref().unwrap().satoshis,
            266382
        );
        let rates = leg.execution_rates();
        assert_eq!(rates.len(), 2);
        assert_eq!(rates[0].net_input.satoshis, 395095);
        assert_eq!(rates[0].output.satoshis, 1001692152);
        let refund = json!({"version":1,"flags":2051,"convert":true,"refund":true,
            "currencyvalues":{"i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV":10},
            "destinationcurrencyid":"i3f7tSctFkiPpiedY8QR5Tep9p4qDVebDx",
            "destination":{"type":2,"address":"RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp"}});
        fixture["record"]["transfers"] = json!([refund]);
        fixture["transaction"]["vout"][2] = json!({"valueSat":1000000000,"scriptPubKey":{"type":"pubkeyhash",
            "hex":"76a9144f01c06114c899baab0522499ead597bf883e78988ac"}});
        let leg = replay_fixture(&fixture, 0);
        assert_eq!(leg.stage, Stage::Refunded);
        assert_eq!(leg.receipt.as_ref().unwrap().kind, ReceiptKind::Refunded);
        assert!(leg.conversion_fee.is_none());
        assert!(leg.execution_rates().is_empty());
    }

    #[test]
    fn imported_candidate_has_no_receipt_or_inferred_fee_and_unknown_is_not_zero() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/vrsc-gateway-leg.json"))
                .unwrap();
        let record = &fixture["record"];
        let mut transfer = record["transfers"][1].clone();
        transfer["fees"] = Value::Null;
        let mut leg = LegSummary::new(reference("source", 0), &transfer);
        leg.imported(reference("import", 0), record);
        assert_eq!(leg.stage, Stage::Imported);
        assert!(leg.receipt.is_none());
        assert!(leg.conversion_fee.is_none());
        assert!(leg.forwarding_conversion_fee.is_none());
        assert!(leg.transfer_fee_budget.is_none());
        transfer["fees"] = json!(0);
        assert_eq!(
            LegSummary::new(reference("source", 0), &transfer)
                .transfer_fee_budget
                .unwrap()
                .satoshis,
            0
        );
        transfer["fees"] = json!(0.000000001);
        assert!(
            LegSummary::new(reference("source", 0), &transfer)
                .transfer_fee_budget
                .is_none()
        );
    }
}
