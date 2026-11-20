//! Decoded protocol roles and explicit navigation targets. Shared CC signing
//! addresses are routing machinery; address matches alone prove no operation.
use crate::world::SystemId;
use serde_json::Value;

pub const MAX_LINKS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Currency(String),
    Destination {
        address: String,
        system: Option<SystemId>,
    },
    Outpoint(crate::world::OutpointRef),
    LegacyNotarizedBlock {
        hash: String,
        height: u32,
    },
    OpReturn(crate::domain::OpReturnData),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub label: String,
    pub target: Target,
}

#[derive(Debug, Clone)]
pub struct Role {
    pub key: &'static str,
    pub eval: u8,
    pub label: &'static str,
    pub address: &'static str,
}

// Public addresses/eval codes from CCcustom.cpp and eval.h. Never include the
// signing material stored beside these addresses in the daemon source.
pub const ROLES: &[Role] = &[
    Role {
        key: "stakeguard",
        eval: 1,
        label: "Stake guard",
        address: "RCG8KwJNDVwpUBcdoa6AoHqHVJsA1uMYMR",
    },
    Role {
        key: "currencydefinition",
        eval: 2,
        label: "Currency definition",
        address: "RP7id3CzCnwvzNUZesYJM6ekvsxpEzMqB1",
    },
    Role {
        key: "notaryevidence",
        eval: 3,
        label: "Notary evidence",
        address: "RQWMeecjGFF3ZAVeSimRbyG9iMDUHPY5Ny",
    },
    Role {
        key: "earnednotarization",
        eval: 4,
        label: "Earned notarization",
        address: "RMYbaxFsCT1xfMmwLCCYAVf2DsxcDTtBmx",
    },
    Role {
        key: "acceptednotarization",
        eval: 5,
        label: "Accepted notarization",
        address: "RDTq9qn1Lthv7fvsdbWz36mGp8HK9XaruZ",
    },
    Role {
        key: "finalizenotarization",
        eval: 6,
        label: "Notarization finalization",
        address: "RRbKYitLH9EhQCvCo4bPZqJx3TWxASadxE",
    },
    Role {
        key: "currencystate",
        eval: 7,
        label: "Currency state",
        address: "REU1HKkmdwdxKMpfD3QoxeERYd9tfMN6n9",
    },
    Role {
        key: "reservetransfer",
        eval: 8,
        label: "Reserve transfer",
        address: "RTqQe58LSj2yr5CrwYFwcsAQ1edQwmrkUU",
    },
    Role {
        key: "reserveoutput",
        eval: 9,
        label: "Reserve output",
        address: "RMXeZGxxRuABFkT4uLSCeuJHLegBNGZq8D",
    },
    Role {
        key: "identityreservation",
        eval: 10,
        label: "Advanced identity reservation",
        address: "REuGNkgunnw1J4Zx6Y9UCp8YHVZqYATe9D",
    },
    Role {
        key: "reservedeposit",
        eval: 11,
        label: "Reserve deposit",
        address: "RFw9AVfgNKcHe2Vp2eyzHrX65aFD9Ky8df",
    },
    Role {
        key: "crosschainexport",
        eval: 12,
        label: "Cross-chain export",
        address: "RGkrs7SndcpsV61oKK2jYdMiU8PgkLU2qP",
    },
    Role {
        key: "crosschainimport",
        eval: 13,
        label: "Cross-chain import",
        address: "RKLN7wFhbrJFkPG8XkKteErAe5CjqoddTm",
    },
    Role {
        key: "identityprimary",
        eval: 14,
        label: "Identity primary",
        address: "RS545EBdK5AzPTaGHNUg78wFuuAzBb74FB",
    },
    Role {
        key: "identityrevoke",
        eval: 15,
        label: "Identity revocation",
        address: "RG6My2zwh9hBFSgUhZ5UmmUtxBap57aU4N",
    },
    Role {
        key: "identityrecover",
        eval: 16,
        label: "Identity recovery",
        address: "RRw9rJMPwdNqC1wgXn5vryJwMDyBgpXjYT",
    },
    Role {
        key: "commitmenthash",
        eval: 17,
        label: "Identity commitment",
        address: "RCySaThHfVBcHZgjJGoBw3un4vcsRJNPYw",
    },
    Role {
        key: "identityreservation",
        eval: 18,
        label: "Identity reservation",
        address: "RDbzJU8rEv4CkMABNUnKQoKDTfnikSm9fM",
    },
    Role {
        key: "finalizeexport",
        eval: 19,
        label: "Export finalization",
        address: "REL7oLNeaeoQB1XauiHfcvjKMZC52Uj5xF",
    },
    Role {
        key: "feepool",
        eval: 20,
        label: "Fee pool",
        address: "RQ55dLQ7uGnLx8scXfkaFV6QS6qVBGyxAG",
    },
];

pub fn known_address(address: &str) -> Option<&'static Role> {
    ROLES.iter().find(|role| role.address == address)
}

fn decoded(script: &Value, role: &Role) -> bool {
    script.get(role.key).is_some_and(|value| {
        value.as_object().is_some_and(|object| !object.is_empty())
            || (matches!(role.eval, 1 | 15 | 16)
                && script["type"] == "cryptocondition"
                && value.as_str() == Some(""))
    })
}

pub fn routing_label(script: &Value, address: &str) -> Option<String> {
    let role = known_address(address)?;
    let payload = ROLES.iter().find(|role| decoded(script, role));
    Some(if decoded(script, role) {
        format!("{} [protocol routing]", role.label)
    } else if let Some(payload) = payload {
        format!("{} address [decoded {} differs]", role.label, payload.label)
    } else {
        format!("{} address [operation unclassified]", role.label)
    })
}

/// Only the output's actual decoded script addresses are raw counterparties.
/// Nested proposers, transfer destinations and auxiliary addresses have roles.
pub fn raw_addresses(output: &Value) -> Vec<String> {
    let script = output.get("scriptPubKey").unwrap_or(output);
    let mut addresses = Vec::new();
    for value in script["addresses"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(script.get("address"))
    {
        if let Some(address) = value.as_str()
            && !addresses.iter().any(|known| known == address)
        {
            addresses.push(address.to_owned());
        }
    }
    addresses
}

fn identifier(value: &Value) -> Option<&str> {
    value.as_str().filter(|id| {
        id.len() == 34 && id.starts_with('i') && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
    })
}

fn system(value: &Value) -> Option<SystemId> {
    identifier(value).and_then(|id| SystemId::try_from(id.to_owned()).ok())
}

fn add(links: &mut Vec<Link>, label: &str, target: Target) {
    if links.len() < MAX_LINKS
        && !links
            .iter()
            .any(|link| link.label == label && link.target == target)
    {
        links.push(Link {
            label: label.into(),
            target,
        });
    }
}

fn currency(links: &mut Vec<Link>, payload: &Value, key: &str, label: &str) {
    if let Some(id) = identifier(&payload[key]) {
        add(links, label, Target::Currency(id.into()));
    }
}

fn destination(
    links: &mut Vec<Link>,
    notes: &mut Vec<String>,
    destination: &Value,
    label: &str,
    system: Option<SystemId>,
    routable: bool,
) {
    let Some(address) = destination["address"].as_str() else {
        return;
    };
    let kind = destination["type"]
        .as_u64()
        .filter(|kind| *kind <= 255)
        .map(|kind| kind & 63);
    let native = address.len() == 34 && address.bytes().all(|byte| byte.is_ascii_alphanumeric());
    if routable
        && matches!(
            (kind, address.chars().next()),
            (Some(2), Some('R')) | (Some(4), Some('i'))
        )
        && native
    {
        add(
            links,
            label,
            Target::Destination {
                address: address.into(),
                system,
            },
        );
    } else {
        notes.push(format!(
            "{label}: {address} ({}; see JSON)",
            if kind == Some(9) {
                "external destination"
            } else {
                "destination/system unresolved"
            }
        ));
    }
}

#[derive(Debug, Clone, Default)]
pub struct References {
    pub links: Vec<Link>,
    pub notes: Vec<String>,
}

pub fn references(output: &Value) -> References {
    let script = &output["scriptPubKey"];
    let mut result = References::default();
    let links = &mut result.links;
    const FIELDS: &[(&str, &[(&str, &str)])] = &[
        (
            "currencydefinition",
            &[
                ("currencyid", "Currency definition"),
                ("systemid", "Currency system"),
                ("launchsystemid", "Launch system"),
            ],
        ),
        (
            "crosschainexport",
            &[
                ("destinationcurrencyid", "Export currency"),
                ("destinationsystemid", "Export system"),
            ],
        ),
        (
            "crosschainimport",
            &[
                ("importcurrencyid", "Import currency"),
                ("sourcesystemid", "Source system"),
            ],
        ),
        (
            "reservedeposit",
            &[("controllingcurrencyid", "Deposit controller")],
        ),
        ("currencystate", &[("currencyid", "Currency state")]),
        (
            "acceptednotarization",
            &[("currencyid", "Notarized currency")],
        ),
        (
            "earnednotarization",
            &[("currencyid", "Notarized currency")],
        ),
        (
            "finalizenotarization",
            &[("currencyid", "Finalization currency")],
        ),
        (
            "finalizeexport",
            &[("currencyid", "Export finalization system")],
        ),
    ];
    for &(key, fields) in FIELDS {
        if script[key].is_object() {
            for &(field, label) in fields {
                currency(links, &script[key], field, label);
            }
        }
    }
    if let Some(transfer) = script
        .get("reservetransfer")
        .filter(|value| value.is_object())
    {
        for (key, label) in [
            ("via", "Conversion pool"),
            ("destinationcurrencyid", "Requested currency"),
            ("feecurrencyid", "Fee currency"),
        ] {
            currency(links, transfer, key, label);
        }
        let cross = transfer["flags"]
            .as_u64()
            .map(|flags| flags & 64 != 0)
            .unwrap_or(transfer["crosssystem"] == true);
        let consistent = transfer["crosssystem"].as_bool().is_none_or(|declared| {
            transfer["flags"]
                .as_u64()
                .is_none_or(|flags| declared == (flags & 64 != 0))
        });
        let first_system = if cross {
            system(&transfer["exportto"])
        } else {
            None
        };
        if cross {
            currency(links, transfer, "exportto", "Next system");
        }
        let dest = &transfer["destination"];
        let gateway = dest["type"].as_u64().is_some_and(|kind| kind & 128 != 0);
        if gateway {
            currency(links, dest, "gateway", "Gateway system");
        }
        let final_system = if gateway {
            system(&dest["gateway"])
        } else {
            first_system.clone()
        };
        let gateway_consistent = gateway == dest.get("gateway").is_some();
        let routable = consistent
            && gateway_consistent
            && if gateway {
                final_system.is_some()
            } else {
                !cross || first_system.is_some()
            };
        destination(
            links,
            &mut result.notes,
            dest,
            "Transfer destination",
            final_system,
            routable,
        );
        for aux in dest["auxdests"]
            .as_array()
            .filter(|_| {
                dest["type"]
                    .as_u64()
                    .is_some_and(|kind| kind <= 255 && kind & 64 != 0)
            })
            .into_iter()
            .flatten()
            .take(8)
        {
            destination(
                links,
                &mut result.notes,
                aux,
                "Fallback destination",
                first_system.clone(),
                consistent && (!cross || first_system.is_some()),
            );
        }
    }
    for key in ["reservetransfer", "reserveoutput", "reservedeposit"] {
        for id in script[key]["currencyvalues"]
            .as_object()
            .into_iter()
            .flat_map(|values| values.keys())
            .take(8)
        {
            if identifier(&Value::String(id.clone())).is_some() {
                add(links, "Currency", Target::Currency(id.clone()));
            }
        }
    }
    if let Some(identity) = script
        .get("identityprimary")
        .filter(|value| value.is_object())
    {
        for (key, label) in [
            ("identityaddress", "Identity"),
            ("revocationauthority", "Revocation authority"),
            ("recoveryauthority", "Recovery authority"),
        ] {
            if let Some(id) = identifier(&identity[key]) {
                add(
                    links,
                    label,
                    Target::Destination {
                        address: id.into(),
                        system: None,
                    },
                );
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const VRSC: &str = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
    const VDEX: &str = "iHog9UCTrn95qpUBFCZ7kKz7qWdMA8MQ6N";
    const VETH: &str = "i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X";
    const ADDRESS: &str = "RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp";

    #[test]
    fn known_address_does_not_turn_an_ordinary_payment_into_a_protocol_operation() {
        let address = ROLES.iter().find(|role| role.eval == 8).unwrap().address;
        let payment = json!({"scriptPubKey":{"type":"pubkeyhash","addresses":[address]}});
        assert!(
            routing_label(&payment["scriptPubKey"], address)
                .unwrap()
                .contains("operation unclassified")
        );
        assert!(references(&payment).links.is_empty());
        assert!(crate::domain::output_tags(&payment).is_empty());
        let mut invalid = payment.clone();
        invalid["scriptPubKey"]["reservetransfer"] = json!("invalid");
        assert!(references(&invalid).links.is_empty());
        let mismatch =
            json!({"scriptPubKey":{"crosschainimport":{"version":1},"addresses":[address]}});
        assert!(
            routing_label(&mismatch["scriptPubKey"], address)
                .unwrap()
                .contains("differs")
        );
        let addresses = ROLES
            .iter()
            .map(|role| role.address)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(addresses.len(), ROLES.len());
        assert!(!addresses.contains(""));
    }

    #[test]
    fn historical_gateway_output_separates_routing_external_target_and_fallback() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/vrsc-gateway-leg.json")).unwrap();
        let output = &fixture["transaction"]["vout"][3];
        let raw = raw_addresses(output);
        assert_eq!(raw, vec!["RTqQe58LSj2yr5CrwYFwcsAQ1edQwmrkUU"]);
        assert!(
            routing_label(&output["scriptPubKey"], &raw[0])
                .unwrap()
                .contains("protocol routing")
        );
        let refs = references(output);
        assert!(
            refs.notes
                .iter()
                .any(|note| note.contains("external destination"))
        );
        assert!(refs.links.iter().all(|link|!matches!(&link.target,Target::Destination{address,..} if address.starts_with("0x"))));
        assert!(
            refs.links
                .iter()
                .any(|link| matches!(&link.target,Target::Currency(id) if id==VETH))
        );
    }

    #[test]
    fn destination_links_keep_explicit_systems_and_never_guess_a_missing_system() {
        let make = |transfer: Value| json!({"scriptPubKey":{"reservetransfer":transfer}});
        let transfer = json!({"flags":65,"crosssystem":true,"exportto":VDEX,"destinationcurrencyid":VRSC,
            "destination":{"type":194,"address":ADDRESS,"gateway":VETH,
                "auxdests":[{"type":2,"address":ADDRESS}]}});
        let refs = references(&make(transfer.clone()));
        assert!(refs.links.iter().any(|link|link.label=="Transfer destination" && matches!(&link.target,Target::Destination {system:Some(system),..} if system.as_ref()==VETH)));
        assert!(refs.links.iter().any(|link|link.label=="Fallback destination" && matches!(&link.target,Target::Destination {system:Some(system),..} if system.as_ref()==VDEX)));
        let mut missing = transfer;
        missing["destination"] = json!({"type":2,"address":ADDRESS});
        missing.as_object_mut().unwrap().remove("exportto");
        let refs = references(&make(missing.clone()));
        assert!(
            !refs
                .links
                .iter()
                .any(|link| matches!(link.target, Target::Destination { .. }))
        );
        assert!(!refs.notes.is_empty());
        missing["flags"] = json!(1);
        assert!(
            !references(&make(missing))
                .links
                .iter()
                .any(|link| matches!(link.target, Target::Destination { .. })),
            "contradictory cross-system metadata is unresolved"
        );
    }

    #[test]
    fn content_and_proof_objects_do_not_create_counterparties_or_navigation_targets() {
        let output = json!({"scriptPubKey":{"type":"cryptocondition","addresses":[ADDRESS],"unknown":"", "contentmap":{
            "reservetransfer":{"destination":{"type":2,"address":"RNotARecipient"}},"txid":"a".repeat(64)}}});
        assert_eq!(raw_addresses(&output), vec![ADDRESS]);
        assert!(references(&output).links.is_empty());
        assert!(routing_label(&output["scriptPubKey"], ADDRESS).is_none());
    }
}
