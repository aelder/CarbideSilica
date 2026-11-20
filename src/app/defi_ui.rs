//! Presentation of instruction budgets and exact per-leg receipts.
use super::*;
use crate::defi::{Amount, LegSummary, ReceiptKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AssetLink {
    pub currency: String,
    pub system: crate::world::SystemId,
    pub height: Option<u64>,
    pub pool: bool,
}

fn amount_label(amount: &Amount, store: &ObjectStore) -> String {
    currency_amount_label(
        currency_display_value(store, &amount.currency),
        amount.satoshis,
    )
}

fn amounts_label(amounts: Option<&[Amount]>, store: &ObjectStore) -> String {
    amounts
        .map(|amounts| {
            amounts
                .iter()
                .map(|a| amount_label(a, store))
                .collect::<Vec<_>>()
                .join(" + ")
        })
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn summary_lines(
    leg: &LegSummary,
    number: usize,
    store: &ObjectStore,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(format!(
        "    Leg {number}: {}",
        leg.stage.label()
    ))];
    let (label, result) = match &leg.receipt {
        Some(receipt) => (
            if receipt.kind == ReceiptKind::Forwarded {
                "Forwarded"
            } else if receipt.kind == ReceiptKind::Refunded {
                "Refunded"
            } else {
                "Received"
            },
            amounts_label(Some(&receipt.amounts), store),
        ),
        None => ("Received", "unresolved".into()),
    };
    lines.push(Line::from(format!(
        "    {:<12}{}   {label:<12}{result}",
        "Sent",
        amounts_label(leg.principal.as_deref(), store)
    )));
    lines.push(Line::from(format!(
        "    {:<12}{}{}",
        "Fee budget",
        leg.transfer_fee_budget
            .as_ref()
            .map(|a| amount_label(a, store))
            .unwrap_or_else(|| "unknown".into()),
        leg.conversion_fee
            .as_ref()
            .map(|a| format!("   Conversion fee  {}", amount_label(a, store)))
            .unwrap_or_default()
    )));
    if let Some(budget) = &leg.forwarding_budget {
        lines.push(Line::from(format!(
            "    Forward budget  {}{}",
            amount_label(budget, store),
            leg.forwarding_conversion_fee
                .as_ref()
                .map(|a| format!("   Conversion fee  {}", amount_label(a, store)))
                .unwrap_or_default()
        )));
    }
    if let Some(receipt) = &leg.receipt {
        if let Some(budget) = &receipt.next_fee_budget {
            lines.push(Line::from(format!(
                "    Next fee budget {} (carried into the next instruction)",
                amount_label(budget, store)
            )));
        }
        lines.push(Line::from(format!(
            "    {} output {}:{} on {}",
            label,
            receipt.output.txid,
            receipt.output.vout,
            currency_display_value(store, receipt.output.system_id.as_ref())
        )));
    }
    if let Some(proceeds) = &leg.proceeds {
        lines.push(Line::from(format!(
            "    Currency proceeds {} at {}:{} (currency identity)",
            amounts_label(Some(&proceeds.amounts), store),
            proceeds.output.txid,
            proceeds.output.vout
        )));
    }
    for rate in leg.execution_rates() {
        lines.push(Line::from(format!(
            "    {} net  {} -> {}",
            rate.purpose,
            amount_label(&rate.net_input, store),
            amount_label(&rate.output, store)
        )));
        if let Some(scaled) = rate.output.satoshis.checked_mul(100_000_000) {
            let ratio = scaled / rate.net_input.satoshis;
            lines.push(Line::from(format!(
                "    Execution rate {}{} {} per {}",
                if scaled % rate.net_input.satoshis == 0 {
                    "="
                } else {
                    "≈"
                },
                format_compact_zat(ratio),
                currency_display_value(store, &rate.output.currency),
                currency_display_value(store, &rate.net_input.currency)
            )));
        }
        for (currency, field, value) in rate.prices {
            lines.push(Line::from(format!(
                "      Import batch: {} {field}={}",
                currency_display_value(store, &currency),
                format_compact_zat(value)
            )));
        }
    }
    if let Some(pool) = &leg.pool {
        let at = leg
            .execution
            .as_ref()
            .and_then(|execution| execution.height)
            .map(|height| {
                format!(
                    " at height {}",
                    theme.format_height_on(
                        leg.execution.as_ref().unwrap().import.system_id.as_ref(),
                        height
                    )
                )
            })
            .unwrap_or_default();
        lines.push(Line::from(format!(
            "    Pool {}{at}",
            currency_display_value(store, pool)
        )));
    }
    lines
}

pub(super) fn pool_lines(
    panel: &DefiPanel,
    store: &ObjectStore,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled("Pool states", theme.focused()),
        Line::default(),
    ];
    let Some(result) = &panel.pool_comparison else {
        lines.push(Line::from("Loading the linked previous notarization…"));
        return lines;
    };
    let comparison = match result {
        Ok(comparison) => comparison,
        Err(error) => {
            lines.push(Line::from(error.clone()));
            return lines;
        }
    };
    lines.push(Line::from(
        "Pool-wide changes between notarizations include other transfers and fees.",
    ));
    let reference = |snapshot: &crate::defi::pool::Snapshot| {
        format!(
            "{}:{} on {} at height {}",
            snapshot.output.txid,
            snapshot.output.vout,
            currency_display_value(store, snapshot.output.system_id.as_ref()),
            snapshot
                .height
                .map(|h| theme.format_height_on(snapshot.output.system_id.as_ref(), h))
                .unwrap_or_else(|| "unknown".into())
        )
    };
    if let Some(previous) = &comparison.previous {
        lines.push(Line::from(format!(
            "Previous notarization: {}",
            reference(previous)
        )));
    }
    lines.push(Line::from(format!(
        "Import state: {}",
        reference(&comparison.imported)
    )));
    if let Some(note) = &comparison.note {
        lines.push(Line::from(note.clone()));
    }
    lines.push(Line::default());
    let before = comparison.previous.as_ref().map(|snapshot| &snapshot.state);
    let after = &comparison.imported.state;
    let number = |value: Option<&Value>| value.and_then(crate::defi::pool::state_amount);
    let pair = |label: String, old: Option<i128>, new: Option<i128>| {
        let show = |value: Option<i128>| {
            value
                .map(format_compact_zat)
                .unwrap_or_else(|| "unavailable".into())
        };
        let delta = old.zip(new).and_then(|(old, new)| new.checked_sub(old));
        Line::from(format!(
            "  {label}: {} -> {}   change {}",
            show(old),
            show(new),
            show(delta)
        ))
    };
    lines.push(pair(
        "Supply".into(),
        number(before.map(|state| &state["supply"])),
        number(after.get("supply")),
    ));
    let reserves = |state: &Value| -> BTreeMap<String, Value> {
        state["reservecurrencies"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|reserve| {
                Some((
                    reserve["currencyid"].as_str()?.to_owned(),
                    reserve["reserves"].clone(),
                ))
            })
            .collect()
    };
    let old = before.map(&reserves).unwrap_or_default();
    let new = reserves(after);
    let ids = old.keys().chain(new.keys()).collect::<BTreeSet<_>>();
    for id in ids.iter().take(12) {
        lines.push(pair(
            currency_display_value(store, id).to_string(),
            number(old.get(*id)),
            number(new.get(*id)),
        ));
    }
    if ids.len() > 12 {
        lines.push(Line::from(
            "First 12 reserves shown; open the pool for the complete state.",
        ));
    }
    lines
}

pub(super) fn selected_leg(panel: &DefiPanel) -> Option<&LegSummary> {
    let report = panel.report.as_ref()?;
    if report
        .entries
        .get(panel.selected_entry)
        .is_some_and(|entry| entry.label.starts_with("Retained "))
    {
        return None;
    }
    report
        .legs
        .iter()
        .rev()
        .find(|leg| {
            report
                .entries
                .iter()
                .position(|entry| {
                    entry.output == leg.source
                        && matches!(
                            entry.label.as_str(),
                            "Conversion" | "Reserve transfer" | "Cross-system transfer"
                        )
                })
                .is_some_and(|index| index <= panel.selected_entry)
        })
        .or_else(|| {
            report
                .entries
                .is_empty()
                .then(|| report.legs.first())
                .flatten()
        })
}

pub(super) fn assets(panel: &DefiPanel) -> Vec<AssetLink> {
    let Some(leg) = selected_leg(panel) else {
        return Vec::new();
    };
    let system = leg
        .execution
        .as_ref()
        .map(|e| &e.import.system_id)
        .unwrap_or(&leg.source.system_id);
    let mut links = Vec::new();
    let mut add = |currency: &str, pool: bool| {
        if !links
            .iter()
            .any(|link: &AssetLink| link.currency == currency)
        {
            links.push(AssetLink {
                currency: currency.into(),
                system: system.clone(),
                height: if pool {
                    leg.execution.as_ref().and_then(|e| e.height)
                } else {
                    None
                },
                pool,
            });
        }
    };
    if let Some(pool) = &leg.pool {
        add(pool, true);
    }
    for amount in leg
        .principal
        .iter()
        .flatten()
        .chain(leg.transfer_fee_budget.iter())
        .chain(leg.forwarding_budget.iter())
        .chain(leg.receipt.iter().flat_map(|r| &r.amounts))
        .chain(leg.proceeds.iter().flat_map(|r| &r.amounts))
        .chain(leg.receipt.iter().flat_map(|r| r.next_fee_budget.iter()))
    {
        add(&amount.currency, false);
    }
    links
}

pub(super) fn asset_lines(
    panel: &DefiPanel,
    store: &ObjectStore,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::styled("Currencies for selected leg", theme.focused()),
        Line::default(),
    ];
    if let Some(message) = &panel.message {
        lines.push(Line::from(message.clone()));
    }
    for (index, link) in assets(panel).iter().enumerate() {
        let selected = Some(index) == panel.asset_selection;
        lines.push(
            selection_line(
                theme,
                selected,
                format!(
                    "{} {}{}",
                    if link.pool { "Pool" } else { "Currency" },
                    currency_display_value(store, &link.currency),
                    link.height
                        .map(|h| format!(
                            " — execution height {}",
                            theme.format_height_on(link.system.as_ref(), h)
                        ))
                        .unwrap_or_default()
                ),
            )
            .style(theme.tile_body(selected)),
        );
        lines.push(Line::from(format!(
            "    {} on {}",
            link.currency, link.system
        )));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{OutpointRef, SystemId};

    #[test]
    fn pool_comparison_keeps_missing_values_unknown_and_resolves_reserve_names() {
        let output = OutpointRef {
            system_id: SystemId::try_from("iSystem".to_string()).unwrap(),
            txid: "import".into(),
            vout: 0,
        };
        let mut panel = DefiPanel::default();
        panel.pool_comparison = Some(Ok(crate::defi::pool::Comparison {
            previous: Some(crate::defi::pool::Snapshot {
                output: output.clone(),
                height: Some(1),
                state: json!({"reservecurrencies":[{"currencyid":"iReserve","reserves":2}]}),
            }),
            imported: crate::defi::pool::Snapshot {
                output,
                height: Some(2),
                state: json!({"supply":100.5,"reservecurrencies":[{"currencyid":"iReserve","reserves":2.2}]}),
            },
            note: None,
        }));
        let mut store = ObjectStore::default();
        store.insert_currency_definition(CurrencyDefinitionRecord {
            currency_id: "iReserve".into(),
            name: "Reserve🦉".into(),
            definition_json: Value::Null,
        });
        let text = pool_lines(&panel, &store, &Theme::desert(crate::cli::EmojiMode::Off))
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains("Supply: unavailable -> 100.5   change unavailable"),
            "{text}"
        );
        assert!(text.contains("Reserve🦉: 2 -> 2.2   change 0.2"), "{text}");
        assert!(text.contains("include other transfers and fees"));
    }

    #[test]
    fn later_leg_assets_use_its_executing_system_height_and_resolved_names() {
        let system = |id: &str| SystemId::try_from(id.to_string()).unwrap();
        let source = OutpointRef {
            system_id: system("iSource"),
            txid: "first".into(),
            vout: 0,
        };
        let next = OutpointRef {
            system_id: system("iSource"),
            txid: "continuation".into(),
            vout: 3,
        };
        let first = LegSummary::new(
            source.clone(),
            &json!({"currencyvalues":{"iReserve":1},"feecurrencyid":"iReserve","fees":0}),
        );
        let mut second = LegSummary::new(
            next.clone(),
            &json!({"currencyvalues":{"iReserve":2},"feecurrencyid":"iReserve","fees":0.0001}),
        );
        second.imported(
            OutpointRef {
                system_id: system("iDestination"),
                txid: "import".into(),
                vout: 0,
            },
            &json!({"importheight":82104,"import":{"importcurrencyid":"iPool"},
                "importnotarization":{"currencystate":{"currencyid":"iPool","flags":49}}}),
        );
        let mut store = ObjectStore::default();
        store.insert_currency_definition(CurrencyDefinitionRecord {
            currency_id: "iReserve".into(),
            name: "Reserve🦉".into(),
            definition_json: Value::Null,
        });
        let lines = summary_lines(
            &second,
            2,
            &store,
            &Theme::desert(crate::cli::EmojiMode::Off),
        )
        .iter()
        .map(Line::to_string)
        .collect::<Vec<_>>()
        .join("\n");
        assert!(lines.contains("2 Reserve🦉"));
        assert!(lines.contains("Received    unresolved"));
        assert!(lines.contains("0.0001 Reserve🦉"));
        assert!(!lines.contains("Conversion fee"));
        let mut panel = DefiPanel::default();
        panel.selected_entry = 2;
        panel.report = Some(crate::defi::Trail {
            entries: vec![
                crate::defi::Entry {
                    output: source,
                    label: "Reserve transfer".into(),
                    detail: String::new(),
                },
                crate::defi::Entry {
                    output: next.clone(),
                    label: "Replayed next-leg transfer".into(),
                    detail: String::new(),
                },
                crate::defi::Entry {
                    output: next,
                    label: "Cross-system transfer".into(),
                    detail: String::new(),
                },
            ],
            legs: vec![first, second],
            ..crate::defi::Trail::default()
        });
        let links = assets(&panel);
        assert_eq!(links.len(), 2, "principal and fees share one currency link");
        assert_eq!(links[0].currency, "iPool");
        assert_eq!(links[0].height, Some(82104));
        assert_eq!(links[0].system, system("iDestination"));
        assert_eq!(links[1].height, None);
        assert_eq!(links[1].system, system("iDestination"));
    }
}
