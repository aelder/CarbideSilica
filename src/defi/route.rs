//! Follow only individually replayed continuation outputs, with per-system
//! import heights and a hard work bound. Address matches never extend a route.
use super::*;

pub const MAX_ROUTE_LEGS: usize = 8;

#[derive(Debug, Clone, Default)]
pub struct SearchPlan {
    pub request: ImportSearch,
    pub target: Option<OutpointRef>,
    pub known_imports: Vec<(OutpointRef, u64)>,
}

impl SearchPlan {
    fn for_leg(&self, root: &OutpointRef, first: bool) -> ImportSearch {
        if self.target.as_ref() == Some(root) || (first && self.target.is_none()) {
            self.request
        } else if let Some((_, height)) = self.known_imports.iter().find(|(key, _)| key == root) {
            ImportSearch::WindowEndingAt(*height)
        } else {
            ImportSearch::Initial
        }
    }
}

#[derive(Clone)]
pub(super) struct Progress {
    pub prefix: Trail,
    pub sender: UnboundedSender<Trail>,
}

impl Progress {
    pub(super) fn send(&self, leg: Trail) {
        let mut report = self.prefix.clone();
        append(&mut report, leg);
        report.searching = true;
        let _ = self.sender.send(report);
    }
}

fn append(route: &mut Trail, mut leg: Trail) {
    route.entries.append(&mut leg.entries);
    route.legs.append(&mut leg.legs);
    route.notes.append(&mut leg.notes);
    route.import_heights.append(&mut leg.import_heights);
    route.search = leg.search;
    route.search_root = leg.search_root;
    route.searching = leg.searching;
    route.stop = leg.stop;
}

pub async fn investigate(
    configs: Vec<Config>,
    root: OutpointRef,
    seed: Value,
    plan: SearchPlan,
    updates: Option<UnboundedSender<Trail>>,
) -> Trail {
    let configs: Vec<_> = configs
        .into_iter()
        .map(|config| config.noninteractive())
        .collect();
    if output(&seed, root.vout).is_some_and(|out| out["scriptPubKey"]["feepool"].is_object()) {
        return super::fee_pool::inspect(&configs, &root).await;
    }
    if output(&seed, root.vout)
        .is_some_and(|out| out["scriptPubKey"]["crosschainimport"].is_object())
    {
        return super::import::inspect(&configs, &root, &seed).await;
    }
    let mut report = Trail::default();
    let mut current = root;
    let mut seed = seed;
    let mut visited = std::collections::HashSet::new();
    for hop in 0..MAX_ROUTE_LEGS {
        if !visited.insert(current.clone()) {
            report.stop = Some(Stop {
                message: "Continuation cycle detected; route stopped".into(),
            });
            break;
        }
        let progress = updates.as_ref().map(|sender| Progress {
            prefix: report.clone(),
            sender: sender.clone(),
        });
        let mut leg = investigate_leg(
            configs.clone(),
            current.clone(),
            seed,
            plan.for_leg(&current, hop == 0),
            progress,
            hop > 0,
        )
        .await;
        let next = leg.next_leg.take();
        report.edges.extend(lifecycle_edges(&current, &leg));
        append(&mut report, leg);
        if let Some(sender) = &updates {
            let mut progress = report.clone();
            progress.searching = true;
            let _ = sender.send(progress);
        }
        let Some(next) = next else {
            break;
        };
        if hop + 1 == MAX_ROUTE_LEGS {
            report.stop = Some(Stop {
                message: format!(
                    "Reached the {MAX_ROUTE_LEGS}-leg route limit; open the verified continuation to proceed"
                ),
            });
            break;
        }
        report.notes.push(format!(
            "Following verified continuation {}:{} on {}",
            next.output.txid, next.output.vout, next.output.system_id
        ));
        current = next.output;
        seed = next.transaction;
    }
    // Favor structural and attributed route evidence over optional candidates
    // when the persistent edge budget is reached.
    if report.edges.len() > 64 {
        report
            .edges
            .sort_by_key(|edge| edge.resolution == "candidate");
        report.edges.truncate(64);
        report.notes.push(
            "Retained evidence is limited to 64 edges; verified route edges are prioritized".into(),
        );
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intermediate_reports_keep_the_receiver_open_and_preserve_each_imports_results() {
        let system = crate::world::SystemId::try_from("iSystem".to_string()).unwrap();
        let at = |txid: &str| OutpointRef {
            system_id: system.clone(),
            txid: txid.into(),
            vout: 0,
        };
        let make = |export: &str, import: &str, label: &str| Trail {
            entries: vec![
                entry(&at(export), "Export", "export"),
                entry(&at(import), "Import", "import"),
                entry(&at(&format!("{import}-result")), label, "result"),
            ],
            ..Trail::default()
        };
        let mut first = make("export1", "import1", "Replayed next-leg transfer");
        first.edges = lifecycle_edges(&at("root"), &first);
        let second = make("export2", "import2", "Replayed reserve result");
        let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
        Progress {
            prefix: first.clone(),
            sender,
        }
        .send(second.clone());
        let progress = receiver.try_recv().unwrap();
        assert!(
            progress.searching,
            "a completed leg is not a completed route"
        );
        assert_eq!(progress.entries.len(), 6);
        first
            .edges
            .extend(lifecycle_edges(&at("import1-result"), &second));
        append(&mut first, second);
        let edges = lifecycle_edges(&at("root"), &first);
        let receipts: Vec<_> = edges
            .iter()
            .filter(|edge| edge.relation == "import-generated-output")
            .collect();
        assert_eq!(receipts.len(), 2);
        assert_eq!(receipts[0].source, at("import1"));
        assert_eq!(receipts[0].target, at("import1-result"));
        assert_eq!(receipts[1].source, at("import2"));
        assert_eq!(receipts[1].target, at("import2-result"));
        assert!(!first.searching);
    }

    #[test]
    fn destination_search_is_scoped_to_its_leg_and_retains_prior_import_heights() {
        let root = |name: &str| OutpointRef {
            system_id: crate::world::SystemId::try_from(format!("i{name}")).unwrap(),
            txid: name.into(),
            vout: 0,
        };
        let first = root("First");
        let next = root("Next");
        let plan = SearchPlan {
            request: ImportSearch::WindowEndingAt(82104),
            target: Some(next.clone()),
            known_imports: vec![(first.clone(), 3962166)],
        };
        assert!(matches!(
            plan.for_leg(&first, true),
            ImportSearch::WindowEndingAt(3962166)
        ));
        assert!(matches!(
            plan.for_leg(&next, false),
            ImportSearch::WindowEndingAt(82104)
        ));
        assert!(matches!(
            plan.for_leg(&root("Other"), false),
            ImportSearch::Initial
        ));
    }
}
