//! Same-system nonfractional issuer burns. Bind the indexed transfer order to
//! the export's actual prevouts, then reconcile the linked supply checkpoints.
use super::{Amount, RpcClient, Value, matches_import, output, replay, valid_txid};

#[derive(Debug, Clone)]
pub struct SupplyBurn {
    pub selected: Amount,
    pub batch_total: Amount,
}

pub(crate) fn issuer_instruction(
    transfer: &Value,
    issuer: &str,
    system: &str,
) -> Option<(i128, i128)> {
    let values = transfer["currencyvalues"].as_object()?;
    let destination = transfer["destination"].as_object()?;
    if issuer == system
        || values.len() != 1
        || transfer["version"] != 1
        || transfer["flags"] != 129
        || transfer["burnchangeprice"] != true
        || transfer["feecurrencyid"] != system
        || transfer["destinationcurrencyid"] != issuer
        || destination.len() != 2
        || destination.get("type")?.as_u64()? != 4
        || destination.get("address")?.as_str()? != issuer
    {
        return None;
    }
    let principal = replay::amount_sats(values.get(issuer)?)?;
    let budget = replay::amount_sats(&transfer["fees"])?;
    (principal > 0 && budget > 0).then_some((principal, budget))
}

pub(super) struct Evidence<'a> {
    pub source: &'a str,
    pub system: &'a str,
    pub issuer: &'a str,
    pub export_tx: &'a Value,
    pub export_vout: u32,
    pub import_tx: &'a Value,
    pub import_vout: u32,
    pub record: &'a Value,
    pub selected: usize,
    pub region: &'a [Value],
}

fn confirmed_height(tx: &Value) -> Option<u64> {
    (tx["confirmations"].as_u64()? > 0).then_some(tx["height"].as_u64()?)
}

fn out(tx: &Value, vout: u32) -> Option<&Value> {
    output(tx, vout).filter(|out| out["n"].as_u64() == Some(vout.into()))
}

fn previous_outpoint(notarization: &Value) -> Option<(&str, u32)> {
    Some((
        notarization["prevnotarizationtxid"]
            .as_str()
            .filter(|id| valid_txid(id))?,
        u32::try_from(notarization["prevnotarizationout"].as_u64()?).ok()?,
    ))
}

fn spends(tx: &Value, id: &str, vout: u32) -> bool {
    tx["vin"].as_array().is_some_and(|inputs| {
        inputs
            .iter()
            .filter(|input| {
                input["txid"].as_str() == Some(id) && input["vout"].as_u64() == Some(vout.into())
            })
            .count()
            == 1
    })
}

fn active_state<'a>(notarization: &'a Value, issuer: &str) -> Option<&'a Value> {
    let state = &notarization["currencystate"];
    (notarization["currencyid"] == issuer
        && state["currencyid"] == issuer
        && state["version"] == 1
        && state["flags"] == 48
        && state["launchcurrencies"].as_array()?.is_empty()
        && state["reservecurrencies"]
            .as_array()
            .is_none_or(Vec::is_empty)
        && [
            "initialsupply",
            "emitted",
            "preconvertedout",
            "primarycurrencyfees",
            "primarycurrencyconversionfees",
        ]
        .iter()
        .all(|key| replay::amount_sats(&state[*key]) == Some(0)))
    .then_some(state)
}

fn supply_change(latest: &Value, previous: &Value, issuer: &str, batch: i128) -> Option<()> {
    let state = active_state(latest, issuer)?;
    let prior_state = active_state(previous, issuer)?;
    if previous["notarizationheight"].as_u64()? != latest["prevheight"].as_u64()?
        || previous["notarizationheight"].as_u64()? >= latest["notarizationheight"].as_u64()?
        || replay::amount_sats(&prior_state["supply"])?
            .checked_sub(replay::amount_sats(&state["supply"])?)?
            != batch
        || replay::signed_amount_sats(&state["primarycurrencyout"]) != Some(-batch)
    {
        return None;
    }
    Some(())
}

fn native_service_output(output: &Value) -> bool {
    let script = &output["scriptPubKey"];
    script["spendableoutput"] == true
        && (matches!(
            script["type"].as_str(),
            Some("pubkey" | "pubkeyhash" | "scripthash")
        ) || (script["type"] == "cryptocondition" && script["evalnone"] == "valid"))
        && [
            "reserveoutput",
            "reserve_balance",
            "reservedeposit",
            "reservetransfer",
        ]
        .iter()
        .all(|field| script.get(field).is_none())
        && replay::amount_sats(&output["value"]).is_some()
}

fn batch_amounts(batch: &[Value], issuer: &str, system: &str) -> Option<(i128, i128)> {
    batch
        .iter()
        .try_fold((0_i128, 0_i128), |(principal, fees), instruction| {
            let (amount, budget) = issuer_instruction(instruction, issuer, system)?;
            Some((principal.checked_add(amount)?, fees.checked_add(budget)?))
        })
}

fn matches_custody(
    values: &Value,
    issuer: &str,
    system: &str,
    principal: i128,
    fees: i128,
) -> bool {
    values.as_object().is_some_and(|map| map.len() == 2)
        && replay::amount_sats(&values[issuer]) == Some(principal)
        && replay::amount_sats(&values[system]) == Some(fees)
}

async fn fetch(client: &RpcClient, source: &str, id: &str) -> Option<Value> {
    let tx = client
        .call_on(source, "getrawtransaction", &[id.into(), 1.into()])
        .await
        .ok()?;
    (tx["txid"] == id && confirmed_height(&tx).is_some()).then_some(tx)
}

impl Evidence<'_> {
    pub(super) async fn verify(&self, client: &RpcClient) -> Option<SupplyBurn> {
        let export_id = self.export_tx["txid"].as_str()?;
        let export = &out(self.export_tx, self.export_vout)?["scriptPubKey"]["crosschainexport"];
        let import = &out(self.import_tx, self.import_vout)?["scriptPubKey"]["crosschainimport"];
        let batch = self.record["transfers"].as_array()?;
        let hash = export["hashtransfers"]
            .as_str()
            .filter(|hash| valid_txid(hash))?;
        let first = usize::try_from(export["firstinput"].as_u64()?).ok()?;
        let count = usize::try_from(export["numinputs"].as_u64()?).ok()?;
        let inputs = self.export_tx["vin"]
            .as_array()?
            .get(first..first.checked_add(count)?)?;
        let height = confirmed_height(self.import_tx)?;
        let source_start = export["sourceheightstart"].as_u64()?;
        let source_end = export["sourceheightend"].as_u64()?;
        let notarization_vout = self.import_vout.checked_add(1)?;
        let latest =
            &out(self.import_tx, notarization_vout)?["scriptPubKey"]["acceptednotarization"];
        if export["flags"] != 128
            || export["version"] != 1
            || export["sourcesystemid"] != self.system
            || export["destinationsystemid"] != self.system
            || export["destinationcurrencyid"] != self.issuer
            || import["version"] != 1
            || import["flags"] != 12
            || import["importcurrencyid"] != self.issuer
            || import["hashtransfers"] != hash
            || import["sourceheight"].as_u64() != Some(source_end)
            || !matches_import(import, self.system, export_id, self.export_vout)
            || import != &self.record["import"]
            || !import["tokensout"].as_object()?.is_empty()
            || latest != &self.record["importnotarization"]
            || latest["notarizationheight"].as_u64() != Some(source_end)
            || self.record["importheight"].as_u64() != Some(height)
            || self.record["importtxid"] != self.import_tx["txid"]
            || self.record["importvout"].as_u64() != Some(self.import_vout.into())
            || confirmed_height(self.export_tx)? != height
            || source_end > height
            || !(1..=24).contains(&count)
            || count != batch.len()
            || self.import_tx["vout"].as_array()?.len()
                != notarization_vout as usize + 1 + self.region.len()
            || !self.region.iter().all(native_service_output)
        {
            return None;
        }

        let (selected, _) =
            issuer_instruction(batch.get(self.selected)?, self.issuer, self.system)?;
        let (principal, fees) = batch_amounts(batch, self.issuer, self.system)?;
        if [&import["valuein"], &export["totalamounts"]]
            .into_iter()
            .any(|values| !matches_custody(values, self.issuer, self.system, principal, fees))
        {
            return None;
        }
        let inputs_spent = self.import_tx["vin"].as_array()?;
        let mut deposits = inputs_spent.iter().filter_map(|input| {
            if input["txid"] != export_id {
                return None;
            }
            let index = u32::try_from(input["vout"].as_u64()?).ok()?;
            let output = out(self.export_tx, index)?;
            Some((output, output["scriptPubKey"].get("reservedeposit")?))
        });
        let (deposit, balance) = deposits.next()?;
        if deposits.next().is_some()
            || balance["controllingcurrencyid"] != self.issuer
            || !matches_custody(
                &balance["currencyvalues"],
                self.issuer,
                self.system,
                principal,
                fees,
            )
            || replay::amount_sats(&deposit["value"]) != Some(fees)
        {
            return None;
        }

        let definition = client
            .call_on(self.source, "getcurrency", &[self.issuer.into()])
            .await
            .ok()?;
        if definition["currencyid"] != self.issuer
            || definition["options"] != 40
            || definition["proofprotocol"] != 1
            || definition["systemid"] != self.system
            || definition["launchsystemid"] != self.system
        {
            return None;
        }
        let (prior_id, prior_index) = previous_outpoint(latest)?;
        if !spends(self.import_tx, prior_id, prior_index) {
            return None;
        }
        let prior = fetch(client, self.source, prior_id).await?;
        if confirmed_height(&prior)? >= height {
            return None;
        }
        let previous = &out(&prior, prior_index)?["scriptPubKey"]["acceptednotarization"];
        supply_change(latest, previous, self.issuer, principal)?;

        // Resolve every prevout on the same source. Each indexed member must
        // agree at the same ordinal, even if some members have identical values.
        let mut seen = std::collections::HashSet::new();
        for (input, item) in inputs.iter().zip(batch) {
            let id = input["txid"].as_str().filter(|id| valid_txid(id))?;
            let vout = u32::try_from(input["vout"].as_u64()?).ok()?;
            if !seen.insert((id, vout)) {
                return None;
            }
            let parent = fetch(client, self.source, id).await?;
            let output = out(&parent, vout)?;
            let (_, budget) = issuer_instruction(item, self.issuer, self.system)?;
            if !(source_start..=source_end).contains(&confirmed_height(&parent)?)
                || &output["scriptPubKey"]["reservetransfer"] != item
                || replay::amount_sats(&output["value"]) != Some(budget)
            {
                return None;
            }
        }

        let amount = |satoshis| Amount {
            currency: self.issuer.into(),
            satoshis,
        };
        Some(SupplyBurn {
            selected: amount(selected),
            batch_total: amount(principal),
        })
    }
}
