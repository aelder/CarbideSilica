//! Strict ordered replay of supported import-output branches in Verus
//! `AddReserveTransferImportOutputs`. Each preceding transfer must be covered
//! and match exactly; unsupported branches return `None`.

mod gateway;
mod launch;
use serde_json::Value;
use sha2::{Digest, Sha256};

const SATOSHI: i128 = 100_000_000;
const SUCCESS_FEE: i128 = 25_000;

pub(crate) fn amount_sats(value: &Value) -> Option<i128> {
    signed_amount_sats(value).filter(|&amount| amount >= 0)
}

pub(super) fn signed_amount_sats(value: &Value) -> Option<i128> {
    let raw = value.as_number()?.to_string();
    let (mantissa, exponent) = if let Some((m, e)) = raw.split_once(['e', 'E']) {
        (m, e.parse::<i32>().ok()?)
    } else {
        (raw.as_str(), 0)
    };
    let (whole, fractional) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let digits = format!("{whole}{fractional}").parse::<i128>().ok()?;
    let scale = 8_i32
        .checked_add(exponent)?
        .checked_sub(fractional.len() as i32)?;
    if scale >= 0 {
        digits.checked_mul(10_i128.checked_pow(scale as u32)?)
    } else {
        let divisor = 10_i128.checked_pow(scale.unsigned_abs())?;
        (digits % divisor == 0).then_some(digits / divisor)
    }
}

pub(super) fn conversion_fee(input: i128, conversions: i128) -> Option<i128> {
    if !(0..=i128::from(i64::MAX)).contains(&input) || !matches!(conversions, 1 | 2) {
        return None;
    }
    Some((input.checked_mul(SUCCESS_FEE)? / SATOSHI * conversions).min(input))
}

pub(crate) fn address_payload(address: &str) -> Option<(u8, Vec<u8>)> {
    if address.len() > 35 {
        return None;
    }
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut little_endian = vec![0_u8];
    for symbol in address.bytes() {
        let mut carry = u32::try_from(ALPHABET.iter().position(|&c| c == symbol)?).ok()?;
        for byte in &mut little_endian {
            carry += u32::from(*byte) * 58;
            *byte = carry as u8;
            carry >>= 8;
        }
        while carry > 0 {
            little_endian.push(carry as u8);
            carry >>= 8;
        }
    }
    let mut decoded = vec![0; address.bytes().take_while(|&c| c == b'1').count()];
    decoded.extend(little_endian.into_iter().rev());
    if decoded.len() != 25 {
        return None;
    }
    let check = Sha256::digest(Sha256::digest(&decoded[..21]));
    if decoded[21..] != check[..4] {
        return None;
    }
    Some((decoded[0], decoded[1..21].to_vec()))
}

fn payload_hex(payload: &[u8]) -> String {
    payload.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn p2pkh_script(address: &str) -> Option<String> {
    let (_, payload) = address_payload(address)?;
    Some(format!("76a914{}88ac", payload_hex(&payload)))
}

fn identity_hash(address: &str) -> Option<String> {
    let (version, payload) = address_payload(address)?;
    (version == 102 && address.starts_with('i')).then(|| payload_hex(&payload))
}

// Version-3, one-of-one EVAL_NONE identity destination emitted by the builder.
// Restrict the new native CC case to this exact serialized script, not an
// arbitrary decoded address annotation or a multi-signature condition.
fn native_identity_refund_matches(destination: &Value, output: &Value) -> bool {
    let Some(address) = destination["address"].as_str() else {
        return false;
    };
    let Some(hash) = identity_hash(address) else {
        return false;
    };
    let script = &output["scriptPubKey"];
    let expected = format!("050403000000cc1b04030001011504{hash}75");
    script["type"].as_str() == Some("cryptocondition")
        && script["evalnone"].as_str() == Some("valid")
        && script["reqSigs"].as_u64() == Some(1)
        && script["spendableoutput"].as_bool() == Some(true)
        && script["addresses"]
            .as_array()
            .is_some_and(|addresses| addresses.len() == 1 && addresses[0].as_str() == Some(address))
        && script["hex"]
            .as_str()
            .is_some_and(|hex| hex.eq_ignore_ascii_case(&expected))
}

fn conversion_sats(
    transfer: &Value,
    import: &Value,
    notarization: &Value,
    output_currency: &str,
) -> Option<(i128, ReplayKind)> {
    if transfer["destination"].get("gateway").is_some()
        || !matches!(transfer["destination"]["type"].as_u64(), Some(2 | 66))
    {
        return None;
    }
    conversion_value(transfer, import, notarization, output_currency)
}

fn conversion_value(
    transfer: &Value,
    import: &Value,
    notarization: &Value,
    output_currency: &str,
) -> Option<(i128, ReplayKind)> {
    let flags = transfer["flags"].as_u64()?;
    let reserve_to_reserve = matches!(flags, 1027 | 1091);
    let pool = import["importcurrencyid"].as_str()?;
    let state = &notarization["currencystate"];
    if transfer["version"].as_u64() != Some(1)
        || !matches!(flags, 3 | 67 | 515 | 579 | 1027 | 1091)
        || transfer["convert"].as_bool() != Some(true)
        || super::transfer_import_currency(transfer) != Some(pool)
        || transfer["importtosource"]
            .as_bool()
            .is_some_and(|value| value != (flags & 0x200 != 0))
        || (transfer["reservetoreserve"].as_bool() == Some(true)) != reserve_to_reserve
        || transfer["destinationcurrencyid"].as_str() != Some(output_currency)
        || state["currencyid"].as_str() != Some(pool)
        || state["flags"]
            .as_u64()
            .is_none_or(|flags| flags & 1 == 0 || flags & 6 != 0)
    {
        return None;
    }
    let values = transfer["currencyvalues"].as_object()?;
    if values.len() != 1 {
        return None;
    }
    let (source_currency, amount) = values.iter().next()?;
    if source_currency == output_currency {
        return None;
    }
    let input = amount_sats(amount)?;
    if input <= 0 || input > i128::from(i64::MAX) {
        return None;
    }
    let fee = conversion_fee(input, if reserve_to_reserve { 2 } else { 1 })?;
    let net = input.checked_sub(fee)?;
    let currencies = state["currencies"].as_object()?;
    let rate = |currency: &str, field: &str| -> Option<i128> {
        let rate = amount_sats(&currencies.get(currency)?[field])?;
        (rate > 0 && rate <= i128::from(i64::MAX)).then_some(rate)
    };
    let (result, kind) = if reserve_to_reserve {
        if transfer["via"].as_str() != Some(pool) {
            return None;
        }
        let fractional = net
            .checked_mul(SATOSHI)?
            .checked_div(rate(source_currency, "lastconversionprice")?)?;
        if fractional > i128::from(i64::MAX) {
            return None;
        }
        (
            fractional
                .checked_mul(rate(output_currency, "viaconversionprice")?)?
                .checked_div(SATOSHI)?,
            ReplayKind::ReserveToReserve,
        )
    } else if output_currency == pool {
        (
            net.checked_mul(SATOSHI)?
                .checked_div(rate(source_currency, "lastconversionprice")?)?,
            ReplayKind::ReserveToFractional,
        )
    } else if source_currency == pool {
        (
            net.checked_mul(rate(output_currency, "lastconversionprice")?)?
                .checked_div(SATOSHI)?,
            ReplayKind::FractionalToReserve,
        )
    } else {
        return None;
    };
    (result > 0 && result <= i128::from(i64::MAX)).then_some((result, kind))
}

fn plain_native_sats(transfer: &Value, destination_system: &str) -> Option<i128> {
    if transfer["version"].as_u64() != Some(1)
        || !matches!(transfer["flags"].as_u64(), Some(1 | 65))
        || transfer["destination"].get("gateway").is_some()
        || transfer["convert"].as_bool() == Some(true)
    {
        return None;
    }
    let values = transfer["currencyvalues"].as_object()?;
    if values.len() != 1 {
        return None;
    }
    let amount = amount_sats(values.get(destination_system)?)?;
    (amount > 0).then_some(amount)
}

fn amount_vector(value: &Value) -> Option<std::collections::BTreeMap<String, i128>> {
    let values = value.as_object()?;
    if values.is_empty() {
        return None;
    }
    values
        .iter()
        .map(|(currency, amount)| {
            let satoshis = amount_sats(amount)?;
            (satoshis > 0).then_some((currency.clone(), satoshis))
        })
        .collect()
}

fn plain_reserve_output_matches(
    transfer: &Value,
    output: &Value,
    destination_system: &str,
) -> bool {
    if transfer["version"].as_u64() != Some(1)
        || !matches!(transfer["flags"].as_u64(), Some(1 | 65))
        || transfer["convert"].as_bool() == Some(true)
        || transfer["refund"].as_bool() == Some(true)
        || transfer["destination"].get("gateway").is_some()
        || output["valueSat"].as_i64() != Some(0)
    {
        return false;
    }
    let Some(expected) = amount_vector(&transfer["currencyvalues"]) else {
        return false;
    };
    if expected.contains_key(destination_system) {
        return false;
    }
    reserve_output_matches(transfer, output, &expected)
}

fn reserve_output_matches(
    transfer: &Value,
    output: &Value,
    expected: &std::collections::BTreeMap<String, i128>,
) -> bool {
    if output["valueSat"].as_i64() != Some(0) {
        return false;
    }
    let script = &output["scriptPubKey"];
    let Some(actual) = amount_vector(&script["reserveoutput"]["currencyvalues"]) else {
        return false;
    };
    let Some(address) = transfer["destination"]["address"].as_str() else {
        return false;
    };
    &actual == expected
        && script["addresses"]
            .as_array()
            .is_some_and(|addresses| addresses.len() == 1 && addresses[0].as_str() == Some(address))
        && script.get("reservetransfer").is_none()
}

/// Forwarding through a settled fractional pool, with optional principal
/// conversion and a separate converted gateway fee budget. The next payload
/// and native custody must both match. Identity/currency exports, mixed
/// principal and serialized nested destinations require other branches.
fn gateway_result(
    transfer: &Value,
    output: &Value,
    import: &Value,
    notarization: &Value,
    system: &str,
) -> Option<ReplayKind> {
    if transfer["version"].as_u64()? != 1
        || !matches!(
            transfer["flags"].as_u64(),
            Some(1 | 3 | 65 | 67 | 515 | 579 | 1027 | 1091)
        )
    {
        return None;
    }
    let state = &notarization["currencystate"];
    let pool = import["importcurrencyid"].as_str()?;
    let state_flags = state["flags"].as_u64()?;
    if state["currencyid"].as_str()? != pool || state_flags & 0x31 != 0x31 || state_flags & 6 != 0 {
        return None;
    }
    let (principal, kind) = if transfer["convert"].as_bool() == Some(true) {
        let currency = transfer["destinationcurrencyid"].as_str()?;
        let (amount, kind) = conversion_value(transfer, import, notarization, currency)?;
        let kind = match kind {
            ReplayKind::ReserveToReserve => ReplayKind::NextLegReserveToReserve,
            ReplayKind::ReserveToFractional => ReplayKind::NextLegReserveToFractional,
            ReplayKind::FractionalToReserve => ReplayKind::NextLegFractionalToReserve,
            _ => return None,
        };
        (
            std::collections::BTreeMap::from([(currency.to_string(), amount)]),
            kind,
        )
    } else {
        if !matches!(transfer["flags"].as_u64(), Some(1 | 65))
            || transfer["destinationcurrencyid"].as_str()? != pool
        {
            return None;
        }
        (
            amount_vector(&transfer["currencyvalues"])?,
            ReplayKind::NextLeg,
        )
    };
    if principal.len() != 1 || principal.values().any(|n| *n > i128::from(i64::MAX)) {
        return None;
    }
    let native = principal.get(system).copied().unwrap_or(0);
    if output["valueSat"].as_i64().map(i128::from)? != native {
        return None;
    }
    let dest = transfer["destination"].as_object()?;
    if dest.keys().any(|key| {
        !matches!(
            key.as_str(),
            "type" | "address" | "auxdests" | "gateway" | "fees"
        )
    }) {
        return None;
    }
    let destination_type = dest.get("type")?.as_u64()?;
    if destination_type > 255
        || destination_type & 128 == 0
        || !matches!(destination_type & 63, 2 | 4 | 9)
    {
        return None;
    }
    let gateway = dest.get("gateway")?.as_str()?;
    if gateway == system {
        return None;
    }
    let budget = amount_sats(dest.get("fees")?)?;
    if budget <= 0 || budget > i128::from(i64::MAX) {
        return None;
    }
    let fee_currency = transfer["feecurrencyid"].as_str()?;
    let currencies = state["currencies"].as_object()?;
    let rate = |currency: &str, field: &str| -> Option<i128> {
        let rate = amount_sats(&currencies.get(currency)?[field])?;
        (rate > 0 && rate <= i128::from(i64::MAX)).then_some(rate)
    };
    let fee = if fee_currency == gateway {
        budget
    } else {
        let conversion_fee = conversion_fee(budget, if fee_currency == pool { 1 } else { 2 })?;
        let net = budget.checked_sub(conversion_fee)?;
        if fee_currency == pool {
            net.checked_mul(rate(gateway, "lastconversionprice")?)?
                .checked_div(SATOSHI)?
        } else {
            let fractional = net
                .checked_mul(SATOSHI)?
                .checked_div(rate(fee_currency, "lastconversionprice")?)?;
            if fractional > i128::from(i64::MAX) {
                return None;
            }
            fractional
                .checked_mul(rate(gateway, "viaconversionprice")?)?
                .checked_div(SATOSHI)?
        }
    };
    if fee <= 0 || fee > i128::from(i64::MAX) {
        return None;
    }
    let mut expected_dest = dest.clone();
    expected_dest.remove("gateway");
    expected_dest.remove("fees");
    expected_dest.insert("type".into(), Value::from(destination_type & !128));
    let next = output
        .pointer("/scriptPubKey/reservetransfer")?
        .as_object()?;
    if next.keys().any(|key| {
        !matches!(
            key.as_str(),
            "version"
                | "currencyvalues"
                | "flags"
                | "crosssystem"
                | "exportto"
                | "feecurrencyid"
                | "fees"
                | "destinationcurrencyid"
                | "destination"
        )
    }) || next.get("version")?.as_u64()? != 1
        || next.get("flags")?.as_u64()? != 65
        || !next.get("crosssystem")?.as_bool()?
        || next.get("exportto")?.as_str()? != gateway
        || next.get("destinationcurrencyid")?.as_str()? != gateway
        || next.get("feecurrencyid")?.as_str()? != gateway
        || amount_sats(next.get("fees")?)? != fee
        || amount_vector(next.get("currencyvalues")?)? != principal
        || next.get("destination")?.as_object()? != &expected_dest
    {
        return None;
    }
    Some(kind)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReplayKind {
    Preconversion,
    GatewayFallback,
    ReserveToReserve,
    ReserveToFractional,
    FractionalToReserve,
    NextLeg,
    NextLegReserveToReserve,
    NextLegReserveToFractional,
    NextLegFractionalToReserve,
    RefundNative,
    RefundReserve,
    Plain,
    ReservePlain,
    ReserveConversion,
}

impl ReplayKind {
    pub(super) fn is_forwarded(self) -> bool {
        matches!(
            self,
            Self::NextLeg
                | Self::NextLegReserveToReserve
                | Self::NextLegReserveToFractional
                | Self::NextLegFractionalToReserve
        )
    }
    pub(super) fn is_refund(self) -> bool {
        matches!(self, Self::RefundNative | Self::RefundReserve)
    }
    pub(super) fn conversion_steps(self) -> i128 {
        if matches!(
            self,
            Self::ReserveToReserve | Self::ReserveConversion | Self::NextLegReserveToReserve
        ) {
            2
        } else {
            1
        }
    }
    pub(super) fn is_conversion(self) -> bool {
        matches!(
            self,
            Self::Preconversion
                | Self::ReserveToReserve
                | Self::ReserveConversion
                | Self::ReserveToFractional
                | Self::FractionalToReserve
                | Self::NextLegReserveToReserve
                | Self::NextLegReserveToFractional
                | Self::NextLegFractionalToReserve
        )
    }
}

fn refunding(notarization: &Value) -> bool {
    notarization["currencystate"]["refunding"].as_bool() == Some(true)
        || notarization["currencystate"]["flags"]
            .as_u64()
            .is_some_and(|flags| flags & 4 != 0)
}

// Match GetCompatibleAuxDestination's primary-first ordering for PBaaS.
// Other compatible kinds must stop the search: skipping an SH/PK to a later
// supported destination would attribute money to the wrong recipient. Unsupported kinds remain
// unresolved, including the chain-identity fallback when none is compatible.
fn refund_destination(destination: &Value) -> Option<Value> {
    let kind = destination["type"].as_u64()?;
    if kind > 255 || kind & 48 != 0 || !matches!(kind & 63, 2 | 4 | 9) {
        return None;
    }
    let gateway = kind & 128 != 0;
    if gateway != destination.get("gateway").is_some()
        || (!gateway && destination.get("fees").is_some())
    {
        return None;
    }
    if gateway {
        crate::world::SystemId::try_from(destination["gateway"].as_str()?.to_owned()).ok()?;
        let fees = amount_sats(&destination["fees"])?;
        if fees > i128::from(i64::MAX) {
            return None;
        }
    }
    let aux = if kind & 64 != 0 {
        let aux = destination["auxdests"].as_array()?;
        // Never inspect an unbounded fallback list or truncate its semantics.
        if aux.len() > 8 {
            return None;
        }
        aux.as_slice()
    } else {
        if destination.get("auxdests").is_some() {
            return None;
        }
        &[]
    };
    for candidate in std::iter::once(destination).chain(aux) {
        let candidate_kind = candidate["type"].as_u64()?;
        if candidate_kind > 255 || candidate_kind & 48 != 0 {
            return None;
        }
        match candidate_kind & 63 {
            2 => {
                let address = candidate["address"].as_str()?;
                if !address.starts_with('R') {
                    return None;
                }
                p2pkh_script(address)?;
                return Some(serde_json::json!({"type":2,"address":address}));
            }
            4 => {
                let address = candidate["address"].as_str()?;
                identity_hash(address)?;
                return Some(serde_json::json!({"type":4,"address":address}));
            }
            9 => continue,
            _ => return None,
        }
    }
    None
}

// GetRefundTransfer clears conversion instructions and emits the full original
// principal. P2PKH and identity destinations include compatible auxiliary
// destinations when a refund cancels a gateway leg. Fees are not principal.
fn refund_transfer(transfer: &Value, import: &Value, system: &str) -> Option<Value> {
    let flags = transfer["flags"].as_u64()?;
    let base = flags & !0x800;
    let cross_system = base & 64 != 0;
    let source = import["sourcesystemid"].as_str()?;
    let route_matches = if source == system {
        !cross_system && transfer.get("exportto").is_none()
    } else {
        // Narrow verified return route. Launch-system redirection may retain an
        // older exportto; that needs historical definition evidence separately.
        cross_system
            && transfer["exportto"].as_str() == Some(system)
            && super::transfer_import_currency(transfer) == import["importcurrencyid"].as_str()
    };
    if transfer["version"].as_u64() != Some(1)
        || !matches!(base & !64, 1 | 3 | 7 | 1027 | 1025)
        || !route_matches
        || transfer["crosssystem"]
            .as_bool()
            .is_some_and(|value| value != cross_system)
        || amount_vector(&transfer["currencyvalues"])?.len() != 1
    {
        return None;
    }
    let mut refunded = transfer.clone();
    refunded["flags"] = Value::from(1);
    refunded["convert"] = Value::from(false);
    refunded["preconvert"] = Value::from(false);
    refunded["refund"] = Value::from(false);
    refunded["reservetoreserve"] = Value::from(false);
    refunded["crosssystem"] = Value::from(false);
    // Replay only the local GetTxOut principal, not a serialized continuation.
    // CROSS_SYSTEM is not HasNextLeg; refund selection strips gateway routing.
    refunded.as_object_mut()?.remove("exportto");
    refunded["destination"] = refund_destination(&transfer["destination"])?;
    Some(refunded)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ReplayProof {
    pub offset: usize,
    pub proceeds: Option<usize>,
    pub satoshis: Option<i128>,
    pub kind: ReplayKind,
}

/// Replay the whole prefix through `selected`. The consensus builder appends
/// these normal recipient outputs during each transfer's iteration and emits
/// fee/reward outputs only after the transfer loop. Supported launch branches
/// consume their proceeds output before the customer principal. Native results
/// require exact satoshis and serialized script. Reserve results require the
/// exact decoded vector and destination; mixed local gateway payouts also
/// verify their native component at the same generated offset.
pub(super) fn replay_supported_prefix(
    batch: &[Value],
    selected: usize,
    import: &Value,
    notarization: &Value,
    destination_system: &str,
    generated_outputs: &[Value],
) -> Option<ReplayProof> {
    replay_with_launch(
        batch,
        selected,
        import,
        notarization,
        destination_system,
        generated_outputs,
        None,
    )
}

pub(super) fn replay_with_launch(
    batch: &[Value],
    selected: usize,
    import: &Value,
    notarization: &Value,
    destination_system: &str,
    generated_outputs: &[Value],
    launch_context: Option<(&Value, &Value)>,
) -> Option<ReplayProof> {
    replay_with_evidence(
        batch,
        selected,
        import,
        notarization,
        destination_system,
        generated_outputs,
        launch_context.map_or(Evidence::None, |(definition, export)| {
            Evidence::Launch(definition, export)
        }),
    )
}

pub(super) enum Evidence<'a> {
    None,
    Launch(&'a Value, &'a Value),
    Gateways(&'a std::collections::BTreeMap<String, Value>, &'a Value),
}

pub(super) fn replay_with_evidence(
    batch: &[Value],
    selected: usize,
    import: &Value,
    notarization: &Value,
    destination_system: &str,
    generated_outputs: &[Value],
    evidence: Evidence<'_>,
) -> Option<ReplayProof> {
    let launch_context = match evidence {
        Evidence::Launch(definition, export) => Some((definition, export)),
        _ => None,
    };
    let prelaunch = launch_context.and_then(|(definition, export)| {
        launch::phase(import, notarization, export, definition, destination_system)
            .map(|fractional| (definition, fractional))
    });
    let settled = notarization["launchcomplete"].as_bool() == Some(true)
        && notarization["launchconfirmed"].as_bool() == Some(true);
    let failed_launch = notarization["currencystate"]["flags"]
        .as_u64()
        .is_some_and(|flags| flags & 4 != 0 && flags & 2 == 0)
        && notarization["currencystate"]["currencyid"] == import["importcurrencyid"]
        && import["importcurrencyid"].is_string();
    if selected >= batch.len()
        || selected >= generated_outputs.len()
        || (!settled && !failed_launch && prelaunch.is_none())
        || notarization["currencystate"]["version"].as_u64() != Some(1)
    {
        return None;
    }
    let mut proof = None;
    let mut offset = 0;
    for transfer in batch.iter().take(selected + 1) {
        if let Some((definition, fractional)) = prelaunch {
            let (receipt, next) = launch::receipt(
                transfer,
                (import, notarization, definition),
                fractional,
                destination_system,
                generated_outputs,
                offset,
            )?;
            offset = next;
            proof = Some(receipt);
            continue;
        }
        let output = generated_outputs.get(offset)?;
        let explicit_refund = transfer["flags"]
            .as_u64()
            .is_some_and(|flags| flags & 0x800 != 0);
        if transfer["refund"].as_bool() == Some(true) && !explicit_refund {
            return None;
        }
        let refunded = if refunding(notarization) || explicit_refund {
            if notarization["currencystate"]["currencyid"].as_str()
                != import["importcurrencyid"].as_str()
                || import["importcurrencyid"].as_str().is_none()
            {
                return None;
            }
            Some(refund_transfer(transfer, import, destination_system)?)
        } else {
            None
        };
        let transfer = refunded.as_ref().unwrap_or(transfer);
        let native =
            conversion_sats(transfer, import, notarization, destination_system).or_else(|| {
                plain_native_sats(transfer, destination_system)
                    .map(|amount| (amount, ReplayKind::Plain))
            });
        let fallback = match &evidence {
            Evidence::Gateways(definitions, definition) if refunded.is_none() => gateway::fallback(
                transfer,
                output,
                import,
                notarization,
                destination_system,
                definitions,
                definition,
            ),
            _ => None,
        };
        let (satoshis, kind) = if let Some(native) = fallback {
            (native, ReplayKind::GatewayFallback)
        } else if let Some((satoshis, kind)) = native {
            if satoshis <= 0 {
                return None;
            }
            let destination = &transfer["destination"];
            let script = &output["scriptPubKey"];
            let matches = match destination["type"].as_u64() {
                Some(2 | 66) => {
                    let expected_script = p2pkh_script(destination["address"].as_str()?)?;
                    script["type"].as_str() == Some("pubkeyhash")
                        && script["hex"]
                            .as_str()?
                            .eq_ignore_ascii_case(&expected_script)
                }
                Some(4) if refunded.is_some() => {
                    native_identity_refund_matches(destination, output)
                }
                _ => false,
            };
            if !matches || output["valueSat"].as_i64().map(i128::from) != Some(satoshis) {
                return None;
            }
            (
                Some(satoshis),
                if refunded.is_some() {
                    ReplayKind::RefundNative
                } else {
                    kind
                },
            )
        } else if plain_reserve_output_matches(transfer, output, destination_system) {
            if transfer["destination"]["type"].as_u64() == Some(4) && refunded.is_some() {
                let script = &output["scriptPubKey"];
                let hash = identity_hash(transfer["destination"]["address"].as_str()?)?;
                let prefix = format!("1b04030001011504{hash}cc");
                if script["type"].as_str() != Some("cryptocondition")
                    || script["reqSigs"].as_u64() != Some(1)
                    || script["spendableoutput"].as_bool() != Some(true)
                    || script["reserveoutput"]["version"].as_u64() != Some(1)
                    || !script["hex"]
                        .as_str()?
                        .to_ascii_lowercase()
                        .starts_with(&prefix)
                {
                    return None;
                }
            }
            (
                None,
                if refunded.is_some() {
                    ReplayKind::RefundReserve
                } else {
                    ReplayKind::ReservePlain
                },
            )
        } else if let Some(kind) =
            gateway_result(transfer, output, import, notarization, destination_system)
        {
            (None, kind)
        } else {
            let currency = transfer["destinationcurrencyid"].as_str()?;
            if currency == destination_system
                || transfer["destination"]["type"]
                    .as_u64()
                    .map(|kind| kind & 15)
                    != Some(2)
            {
                return None;
            }
            let (amount, kind) = conversion_sats(transfer, import, notarization, currency)?;
            if amount <= 0
                || !reserve_output_matches(
                    transfer,
                    output,
                    &std::collections::BTreeMap::from([(currency.to_string(), amount)]),
                )
            {
                return None;
            }
            (
                None,
                if kind == ReplayKind::ReserveToReserve {
                    ReplayKind::ReserveConversion
                } else {
                    kind
                },
            )
        };
        proof = Some(ReplayProof {
            proceeds: None,
            offset,
            satoshis,
            kind,
        });
        offset += 1;
    }
    proof
}

pub(super) fn replay_launch_allocations(
    import: &Value,
    notarization: &Value,
    export: &Value,
    definition: &Value,
    transfers: &[Value],
    system: &str,
    outputs: &[Value],
) -> Option<Vec<(usize, String, i128)>> {
    launch::allocations(
        import,
        notarization,
        export,
        definition,
        transfers,
        system,
        outputs,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decimal_amounts_do_not_round_fractional_satoshis() {
        assert_eq!(amount_sats(&json!(0.00395291)), Some(395291));
        assert_eq!(amount_sats(&json!(1e-8)), Some(1));
        assert_eq!(amount_sats(&json!(1e-9)), None);
        assert_eq!(amount_sats(&json!(-1)), None);
    }

    #[test]
    fn gateway_conversions_replay_principal_and_native_custody_independently_of_fees() {
        let pool = "iPool";
        let system = "iReserve";
        let gateway = "iGateway";
        let import = json!({"importcurrencyid":pool,"sourcesystemid":system});
        let state = json!({"launchcomplete":true,"launchconfirmed":true,"currencystate":{
            "version":1,"flags":49,"currencyid":pool,"currencies":{
                (system):{"lastconversionprice":2},"iOther":{"viaconversionprice":4}}}});
        for (flags, source, input, dest, result, native, kind) in [
            (
                3,
                system,
                100,
                pool,
                49.9875,
                0,
                ReplayKind::NextLegReserveToFractional,
            ),
            (
                515,
                pool,
                50,
                system,
                99.975,
                9997500000_i64,
                ReplayKind::NextLegFractionalToReserve,
            ),
            (
                1027,
                system,
                100,
                "iOther",
                199.9,
                0,
                ReplayKind::NextLegReserveToReserve,
            ),
            (1, system, 1, pool, 1.0, 100000000, ReplayKind::NextLeg),
        ] {
            let mut transfer = json!({"version":1,"flags":flags,"currencyvalues":{(source):input},
                "feecurrencyid":gateway,"fees":0.0002,"destinationcurrencyid":dest,
                "destination":{"type":201,"address":"0xrecipient","gateway":gateway,"fees":0.01,
                    "auxdests":[{"type":2,"address":"Raux"}]}});
            if flags != 1 {
                transfer["convert"] = json!(true);
            }
            if flags == 1027 {
                transfer["reservetoreserve"] = json!(true);
                transfer["via"] = json!(pool);
            }
            let result_currency = if flags == 1 { source } else { dest };
            let output = json!({"valueSat":native,"scriptPubKey":{"reservetransfer":{
                "version":1,"flags":65,"crosssystem":true,"exportto":gateway,
                "currencyvalues":{(result_currency):result},"feecurrencyid":gateway,"fees":0.01,
                "destinationcurrencyid":gateway,"destination":{"type":73,"address":"0xrecipient",
                    "auxdests":[{"type":2,"address":"Raux"}]}}}});
            let check = |output: &Value| {
                replay_supported_prefix(
                    std::slice::from_ref(&transfer),
                    0,
                    &import,
                    &state,
                    system,
                    std::slice::from_ref(output),
                )
            };
            assert_eq!(check(&output).unwrap().kind, kind);
            let mut changed = output.clone();
            changed["scriptPubKey"]["reservetransfer"]["currencyvalues"][result_currency] =
                json!(result + 0.00000001);
            assert!(check(&changed).is_none());
            changed = output.clone();
            changed["valueSat"] = json!(native + 1);
            assert!(check(&changed).is_none());
            changed = output;
            changed["scriptPubKey"]["reservetransfer"]["fees"] = json!(0.01000001);
            assert!(check(&changed).is_none());
        }
    }

    #[test]
    fn settled_local_refunds_preserve_full_principal_and_reject_other_return_paths() {
        let system = "iReserve";
        let pool = "iPool";
        let import = json!({"sourcesystemid":system,"importcurrencyid":pool});
        let state = json!({"launchcomplete":true,"launchconfirmed":true,"currencystate":{"version":1,"flags":49,"currencyid":pool}});
        let transfer = json!({"version":1,"flags":2051,"refund":true,"convert":true,
            "currencyvalues":{(system):10},"destinationcurrencyid":pool,
            "destination":{"type":2,"address":"RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp"}});
        let output = json!({"valueSat":1000000000,"scriptPubKey":{"type":"pubkeyhash",
            "hex":"76a9144f01c06114c899baab0522499ead597bf883e78988ac"}});
        let check = |transfer: &Value, import: &Value, state: &Value, output: &Value| {
            replay_supported_prefix(
                std::slice::from_ref(transfer),
                0,
                import,
                state,
                system,
                std::slice::from_ref(output),
            )
        };
        let proof = check(&transfer, &import, &state, &output).unwrap();
        assert_eq!(proof.kind, ReplayKind::RefundNative);
        assert_eq!(proof.satoshis, Some(1000000000));
        let mut normal = transfer.clone();
        normal["flags"] = json!(3);
        normal.as_object_mut().unwrap().remove("refund");
        let mut refunding = state.clone();
        refunding["currencystate"]["flags"] = json!(53);
        assert_eq!(
            check(&normal, &import, &refunding, &output).unwrap().kind,
            ReplayKind::RefundNative
        );
        let mut reserve = transfer.clone();
        reserve["currencyvalues"] = json!({"iToken":10});
        let reserve_output = json!({"valueSat":0,"scriptPubKey":{"addresses":["RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp"],"reserveoutput":{"currencyvalues":{"iToken":10}}}});
        assert_eq!(
            check(&reserve, &import, &state, &reserve_output)
                .unwrap()
                .kind,
            ReplayKind::RefundReserve
        );
        let mut changed = output.clone();
        changed["valueSat"] = json!(999750000);
        assert!(check(&transfer, &import, &state, &changed).is_none());
        let mut foreign = import.clone();
        foreign["sourcesystemid"] = json!("iForeign");
        assert!(check(&transfer, &foreign, &state, &output).is_none());
        for (path, value) in [
            ("/flags", json!(2115)),
            ("/flags", json!(1)),
            ("/destination/type", json!(66)),
        ] {
            let mut changed = transfer.clone();
            *changed.pointer_mut(path).unwrap() = value;
            assert!(check(&changed, &import, &state, &output).is_none());
        }
        let mut gateway = transfer.clone();
        gateway["destination"]["gateway"] = json!("iGateway");
        assert!(check(&gateway, &import, &state, &output).is_none());
        let mut launch = state;
        launch["launchcomplete"] = json!(false);
        assert!(check(&transfer, &import, &launch, &output).is_none());
    }

    #[test]
    fn cancelled_gateway_refund_replays_full_prefix_and_first_compatible_destination() {
        // Synthetic refund branch based on a real gateway batch. This is not
        // evidence that the original historical forwarding transaction refunded.
        let mut fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/vrsc-gateway-leg.json"))
                .unwrap();
        let system = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        let record = &mut fixture["record"];
        record["transfers"][1]["flags"] = serde_json::json!(2049);
        record["transfers"][1]["refund"] = serde_json::json!(true);
        let address = record["transfers"][1]["destination"]["auxdests"][0]["address"].clone();
        fixture["transaction"]["vout"][3] = serde_json::json!({"valueSat":0,"scriptPubKey":{
            "addresses":[address],"reserveoutput":{"currencyvalues":record["transfers"][1]["currencyvalues"].clone()}}});
        let check = |fixture: &Value| {
            replay_supported_prefix(
                fixture["record"]["transfers"].as_array().unwrap(),
                1,
                &fixture["record"]["import"],
                &fixture["record"]["importnotarization"],
                system,
                &fixture["transaction"]["vout"].as_array().unwrap()[2..4],
            )
        };
        assert_eq!(check(&fixture).unwrap().kind, ReplayKind::RefundReserve);
        let mut changed = fixture.clone();
        changed["transaction"]["vout"][3]["scriptPubKey"]["reserveoutput"]["currencyvalues"]["i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X"] =
            serde_json::json!(0.593);
        assert!(
            check(&changed).is_none(),
            "unused forwarding fees are not principal"
        );
        changed = fixture.clone();
        changed["transaction"]["vout"][2]["valueSat"] = serde_json::json!(1);
        assert!(
            check(&changed).is_none(),
            "earlier batch members must still replay"
        );
        changed = fixture.clone();
        changed["record"]["transfers"][1]["destination"]["auxdests"] = serde_json::json!([
            {"type":4,"address":system},{"type":2,"address":address}]);
        assert!(
            check(&changed).is_none(),
            "do not skip an earlier compatible identity"
        );
        changed = fixture.clone();
        changed["record"]["transfers"][1]["destination"]["type"] = serde_json::json!(137);
        assert!(
            check(&changed).is_none(),
            "unflagged auxiliary list is inconsistent"
        );
        changed = fixture.clone();
        changed["record"]["import"]["sourcesystemid"] = serde_json::json!("iForeign");
        assert!(check(&changed).is_none());
        changed = fixture.clone();
        changed["record"]["transfers"][1]["flags"] = serde_json::json!(1);
        changed["record"]["transfers"][1]
            .as_object_mut()
            .unwrap()
            .remove("refund");
        assert!(
            check(&changed).is_none(),
            "a matching output alone cannot establish a refund"
        );
    }

    #[test]
    fn cross_system_refund_replays_local_principal_not_a_forwarded_instruction() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/defi-refunds/refund-bots.json"
        ))
        .unwrap();
        let system = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        let mut import = fixture["record"]["import"].clone();
        let state = &fixture["record"]["importnotarization"];
        let mut transfers = fixture["record"]["transfers"].as_array().unwrap().clone();
        let (_, region) = super::super::generated_region(
            &fixture["transaction"],
            &import,
            import["importoutput"].as_u64().unwrap_or(0) as u32,
        )
        .unwrap();
        // Source-derived case based on the real receipt amounts/scripts. This is
        // not claimed as a captured cross-system refund transaction.
        import["sourcesystemid"] = Value::from("foreign-system");
        for transfer in &mut transfers {
            let flags = transfer["flags"].as_u64().unwrap();
            transfer["flags"] = Value::from(flags | 64);
            transfer["crosssystem"] = Value::from(true);
            transfer["exportto"] = Value::from(system);
        }
        let proof = replay_supported_prefix(&transfers, 1, &import, state, system, region).unwrap();
        assert_eq!(proof.kind, ReplayKind::RefundReserve);
        assert!(!proof.kind.is_forwarded());
        transfers[0]["exportto"] = Value::from("unproven-launch-redirection");
        assert!(replay_supported_prefix(&transfers, 1, &import, state, system, region).is_none());
        transfers[0]["exportto"] = Value::from(system);
        transfers[0]["crosssystem"] = Value::from(false);
        assert!(replay_supported_prefix(&transfers, 1, &import, state, system, region).is_none());
    }

    #[test]
    fn refund_identity_destination_precedes_a_later_pubkeyhash_auxiliary() {
        let id = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        let mut destination = json!({"type":68,"address":id,
            "auxdests":[{"type":2,"address":"RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp"}]});
        let selected = refund_destination(&destination).unwrap();
        assert_eq!(selected, json!({"type":4,"address":id}));
        let mut invalid = id.to_string();
        invalid.pop();
        invalid.push('X');
        destination["address"] = json!(invalid);
        assert!(
            refund_destination(&destination).is_none(),
            "an invalid primary identity must not fall through to an auxiliary"
        );
    }

    #[test]
    fn refund_destination_uses_primary_before_aux_and_rejects_bad_or_missing_fallbacks() {
        let address = "RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp";
        let primary = json!({"type":66,"address":address,
            "auxdests":[{"type":2,"address":"RMPmexyLnaCaqX9qgds1Rv3C81TauMb6VS"}]});
        assert_eq!(refund_destination(&primary).unwrap()["address"], address);
        let external = json!({"type":73,"address":"0x1234","auxdests":[
            {"type":9,"address":"0x5678"},{"type":2,"address":address}]});
        assert_eq!(refund_destination(&external).unwrap()["address"], address);
        let mut malformed = external.clone();
        malformed["auxdests"][0] = json!({"type":2,"address":"RInvalid"});
        assert!(
            refund_destination(&malformed).is_none(),
            "a bad compatible address stops selection"
        );
        malformed["auxdests"] = json!([]);
        assert!(
            refund_destination(&malformed).is_none(),
            "chain identity fallback needs another branch"
        );
        malformed["auxdests"] = json!(vec![json!({"type":9,"address":"0x1234"}); 9]);
        assert!(refund_destination(&malformed).is_none());
    }

    #[test]
    fn live_pool_currency_purchase_matches_the_exact_reserve_receipt() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/vrsc-pool-buy.json")).unwrap();
        let record = &fixture["record"];
        let outputs = fixture["transaction"]["vout"].as_array().unwrap();
        let proof = replay_supported_prefix(
            record["transfers"].as_array().unwrap(),
            0,
            &record["import"],
            &record["importnotarization"],
            "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV",
            &outputs[2..5],
        );
        assert!(
            proof.is_some(),
            "110 VRSC must replay to 7.62258126 Bridge.vETH"
        );
    }

    #[test]
    fn ordinary_pool_buys_and_sells_require_exact_outputs_and_pool_context() {
        let address = "RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp";
        let system = "iReserve";
        let buy = json!({"version":1,"flags":3,"convert":true,"currencyvalues":{"iReserve":100},
            "destinationcurrencyid":"iPool","destination":{"type":2,"address":address}});
        let sell = json!({"version":1,"flags":579,"convert":true,"importtosource":true,"crosssystem":true,"exportto":system,
            "currencyvalues":{"iPool":50},"destinationcurrencyid":system,"destination":{"type":2,"address":address}});
        let state = json!({"launchcomplete":true,"launchconfirmed":true,"currencystate":{
            "currencyid":"iPool","version":1,"flags":49,"currencies":{"iReserve":{"lastconversionprice":2}}}});
        let import = json!({"importcurrencyid":"iPool"});
        let outputs = [
            json!({"valueSat":0,"scriptPubKey":{"addresses":[address],"reserveoutput":{"currencyvalues":{"iPool":49.9875}}}}),
            json!({"valueSat":9997500000_i64,"scriptPubKey":{"type":"pubkeyhash","hex":"76a9144f01c06114c899baab0522499ead597bf883e78988ac"}}),
        ];
        let batch = [buy.clone(), sell];
        let proof = replay_supported_prefix(&batch, 1, &import, &state, system, &outputs).unwrap();
        assert_eq!(proof.kind, ReplayKind::FractionalToReserve);
        assert_eq!(proof.satoshis, Some(9997500000));
        let mut changed = outputs.clone();
        changed[0]["scriptPubKey"]["reserveoutput"]["currencyvalues"]["iPool"] = json!(49.98750001);
        assert!(replay_supported_prefix(&batch, 1, &import, &state, system, &changed).is_none());
        for (field, value) in [
            ("currencyid", json!("iWrong")),
            ("flags", json!(48)),
            ("flags", json!(53)),
            ("flags", json!(51)),
        ] {
            let mut other = state.clone();
            other["currencystate"][field] = value;
            assert!(
                replay_supported_prefix(&batch, 1, &import, &other, system, &outputs).is_none()
            );
        }
        let mut wrong_route = buy;
        wrong_route["destinationcurrencyid"] = json!("iNeitherPoolNorSource");
        assert!(
            replay_supported_prefix(&[wrong_route], 0, &import, &state, system, &outputs).is_none()
        );
    }

    #[test]
    fn canonical_pool_sells_match_native_and_reserve_receipts_and_reject_wrong_routes() {
        let address = "RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp";
        let state = json!({"launchcomplete":true,"launchconfirmed":true,"currencystate":{
            "currencyid":"pool","version":1,"flags":49,
            "currencies":{"reserve":{"lastconversionprice":2}}}});
        let import = json!({"importcurrencyid":"pool"});
        for flags in [515, 579] {
            let transfer = json!({"version":1,"flags":flags,"convert":true,"importtosource":true,
                "currencyvalues":{"pool":50},"destinationcurrencyid":"reserve",
                "destination":{"type":2,"address":address}});
            for system in ["reserve", "other-system"] {
                let native = system == "reserve";
                let output = if native {
                    json!({"valueSat":9997500000_i64,"scriptPubKey":{"type":"pubkeyhash",
                        "hex":"76a9144f01c06114c899baab0522499ead597bf883e78988ac"}})
                } else {
                    json!({"valueSat":0,"scriptPubKey":{"addresses":[address],
                        "reserveoutput":{"currencyvalues":{"reserve":99.975}}}})
                };
                let check = |transfer: &Value, output: &Value, import: &Value| {
                    replay_supported_prefix(
                        std::slice::from_ref(transfer),
                        0,
                        import,
                        &state,
                        system,
                        std::slice::from_ref(output),
                    )
                };
                let proof = check(&transfer, &output, &import).unwrap();
                assert_eq!(proof.kind, ReplayKind::FractionalToReserve);
                assert_eq!(proof.satoshis, native.then_some(9997500000));
                let mut wrong = transfer.clone();
                wrong["flags"] = json!(flags & !0x200);
                assert!(check(&wrong, &output, &import).is_none());
                wrong = transfer.clone();
                wrong["importtosource"] = json!(false);
                assert!(check(&wrong, &output, &import).is_none());
                wrong = transfer.clone();
                wrong["flags"] = json!(flags | 0x4000);
                assert!(check(&wrong, &output, &import).is_none());
                let mut altered = output.clone();
                if native {
                    altered["valueSat"] = json!(9997500001_i64);
                } else {
                    altered["scriptPubKey"]["reserveoutput"]["currencyvalues"]["reserve"] =
                        json!(99.97500001);
                }
                assert!(check(&transfer, &altered, &import).is_none());
                assert!(
                    check(&transfer, &output, &json!({"importcurrencyid":"reserve"})).is_none()
                );
            }
        }
    }

    #[test]
    fn real_gateway_leg_is_attributed_after_the_verified_conversion_prefix() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/vrsc-gateway-leg.json"))
                .unwrap();
        let r = &fixture["record"];
        let outputs = fixture["transaction"]["vout"].as_array().unwrap();
        let proof = replay_supported_prefix(
            r["transfers"].as_array().unwrap(),
            1,
            &r["import"],
            &r["importnotarization"],
            "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV",
            &outputs[2..6],
        );
        assert!(
            proof.is_some(),
            "The 0.59293245 vETH principal and 0.00210011 vETH forwarding fee must match"
        );
    }

    #[test]
    fn gateway_replay_rejects_changes_to_principal_fees_destination_and_prefix() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/vrsc-gateway-leg.json"))
                .unwrap();
        let r = &fixture["record"];
        let batch = r["transfers"].as_array().unwrap();
        let outputs = &fixture["transaction"]["vout"].as_array().unwrap()[2..6];
        let check = |batch: &[Value], outputs: &[Value]| {
            replay_supported_prefix(
                batch,
                1,
                &r["import"],
                &r["importnotarization"],
                "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV",
                outputs,
            )
        };
        assert_eq!(check(batch, outputs).unwrap().kind, ReplayKind::NextLeg);
        for (path, value) in [
            ("/scriptPubKey/reservetransfer/fees", json!(0.00210012)),
            (
                "/scriptPubKey/reservetransfer/currencyvalues/i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X",
                json!(0.59293246),
            ),
            (
                "/scriptPubKey/reservetransfer/destination/address",
                json!("RWrong"),
            ),
            ("/scriptPubKey/reservetransfer/destination/type", json!(201)),
            ("/scriptPubKey/reservetransfer/exportto", json!("iWrong")),
            ("/scriptPubKey/reservetransfer/flags", json!(1)),
            ("/valueSat", json!(1)),
        ] {
            let mut changed = outputs.to_vec();
            *changed[1].pointer_mut(path).unwrap() = value;
            assert!(check(batch, &changed).is_none(), "accepted changed {path}");
        }
        let mut changed = outputs.to_vec();
        changed[0]["valueSat"] = json!(1001692153);
        assert!(check(batch, &changed).is_none());
        changed.swap(0, 1);
        assert!(check(batch, &changed).is_none());
        let mut unsupported = batch.to_vec();
        unsupported[1]["destination"]["type"] = json!(208);
        assert!(check(&unsupported, outputs).is_none());
        unsupported = batch.to_vec();
        unsupported[1]["destination"]["fees"] = json!(5.4);
        assert!(check(&unsupported, outputs).is_none());
    }

    #[test]
    fn documented_conversion_matches_exact_satoshis_and_script() {
        let vrsc = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        let veth = "i9nwxtKuVYX4MSbeULLiK2ttVi6rUEhh4X";
        let pool = "i3f7tSctFkiPpiedY8QR5Tep9p4qDVebDx";
        let transfer = json!({"version":1,"flags":1027,"convert":true,"reservetoreserve":true,
            "currencyvalues":{(veth):0.00395291},"via":pool,"destinationcurrencyid":vrsc,
            "destination":{"type":66,"address":"RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp"}});
        let import = json!({"importcurrencyid":pool});
        let notarization = json!({"launchcomplete":true,"launchconfirmed":true,
            "currencystate":{"version":1,"flags":49,"currencyid":pool,"currencies":{
                (veth):{"lastconversionprice":0.00569406},
                (vrsc):{"viaconversionprice":14.43626273}}}});
        let output = json!({"valueSat":1001692152,"scriptPubKey":{"type":"pubkeyhash",
            "hex":"76a9144f01c06114c899baab0522499ead597bf883e78988ac"}});
        assert_eq!(
            replay_supported_prefix(
                std::slice::from_ref(&transfer),
                0,
                &import,
                &notarization,
                vrsc,
                std::slice::from_ref(&output)
            ),
            Some(ReplayProof {
                proceeds: None,
                offset: 0,
                satoshis: Some(1001692152),
                kind: ReplayKind::ReserveToReserve
            })
        );
        let mut altered = output.clone();
        altered["valueSat"] = json!(1001692153);
        assert_eq!(
            replay_supported_prefix(
                std::slice::from_ref(&transfer),
                0,
                &import,
                &notarization,
                vrsc,
                std::slice::from_ref(&altered)
            ),
            None
        );
        altered = output.clone();
        altered["scriptPubKey"]["hex"] =
            json!("76a914000000000000000000000000000000000000000088ac");
        assert_eq!(
            replay_supported_prefix(
                std::slice::from_ref(&transfer),
                0,
                &import,
                &notarization,
                vrsc,
                std::slice::from_ref(&altered)
            ),
            None
        );
        let later = json!({"version":1,"flags":1027,"convert":true,"reservetoreserve":true,
            "currencyvalues":{(veth):0.00790582},"via":pool,"destinationcurrencyid":vrsc,
            "destination":{"type":66,"address":"RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp"}});
        let later_output = json!({"valueSat":2003379223,"scriptPubKey":{"type":"pubkeyhash",
            "hex":"76a9144f01c06114c899baab0522499ead597bf883e78988ac"}});
        assert_eq!(
            replay_supported_prefix(
                &[transfer.clone(), later.clone()],
                1,
                &import,
                &notarization,
                vrsc,
                &[output.clone(), later_output.clone()]
            ),
            Some(ReplayProof {
                proceeds: None,
                offset: 1,
                satoshis: Some(2003379223),
                kind: ReplayKind::ReserveToReserve
            })
        );
        assert_eq!(
            replay_supported_prefix(
                &[transfer.clone(), later],
                1,
                &import,
                &notarization,
                vrsc,
                &[later_output, output.clone()]
            ),
            None
        );
        let unsupported = json!({"version":1,"flags":1,"currencyvalues":{(veth):1.0},
            "destination":{"type":2,"address":"RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp"}});
        assert_eq!(
            replay_supported_prefix(
                &[unsupported, transfer],
                1,
                &import,
                &notarization,
                vrsc,
                &[output.clone(), output]
            ),
            None
        );
        let plain = json!({"version":1,"flags":1,"currencyvalues":{(vrsc):0.5},
            "destination":{"type":2,"address":"RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp"}});
        let plain_output = json!({"valueSat":50000000,"scriptPubKey":{"type":"pubkeyhash",
            "hex":"76a9144f01c06114c899baab0522499ead597bf883e78988ac"}});
        let conversion = json!({"version":1,"flags":1027,"convert":true,"reservetoreserve":true,
            "currencyvalues":{(veth):0.00395291},"via":pool,"destinationcurrencyid":vrsc,
            "destination":{"type":66,"address":"RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp"}});
        let converted = json!({"valueSat":1001692152,"scriptPubKey":{"type":"pubkeyhash",
            "hex":"76a9144f01c06114c899baab0522499ead597bf883e78988ac"}});
        assert_eq!(
            replay_supported_prefix(
                &[plain, conversion],
                1,
                &import,
                &notarization,
                vrsc,
                &[plain_output, converted]
            ),
            Some(ReplayProof {
                proceeds: None,
                offset: 1,
                satoshis: Some(1001692152),
                kind: ReplayKind::ReserveToReserve
            })
        );
    }

    #[test]
    fn reserve_conversion_requires_exact_vector_and_a_supported_prefix() {
        let system = "iExecutingSystem";
        let address = "RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp";
        let transfer = json!({"version":1,"flags":1027,"convert":true,
            "reservetoreserve":true,"via":"iPool","destinationcurrencyid":"iReserve",
            "currencyvalues":{"iSource":0.00395291},
            "destination":{"type":2,"address":address}});
        let import = json!({"importcurrencyid":"iPool"});
        let notarization = json!({"launchcomplete":true,"launchconfirmed":true,
            "currencystate":{"version":1,"flags":49,"currencyid":"iPool","currencies":{
                "iSource":{"lastconversionprice":0.00569406},
                "iReserve":{"viaconversionprice":14.43626273}}}});
        let output = json!({"valueSat":0,"scriptPubKey":{"addresses":[address],
            "reserveoutput":{"currencyvalues":{"iReserve":10.01692152}}}});
        let check = |batch: &[Value], selected, outputs: &[Value]| {
            replay_supported_prefix(batch, selected, &import, &notarization, system, outputs)
        };
        assert_eq!(
            check(
                std::slice::from_ref(&transfer),
                0,
                std::slice::from_ref(&output)
            ),
            Some(ReplayProof {
                proceeds: None,
                offset: 0,
                satoshis: None,
                kind: ReplayKind::ReserveConversion
            })
        );
        for altered in [
            json!({"valueSat":0,"scriptPubKey":{"addresses":[address],
                "reserveoutput":{"currencyvalues":{"iReserve":10.01692153}}}}),
            json!({"valueSat":1,"scriptPubKey":{"addresses":[address],
                "reserveoutput":{"currencyvalues":{"iReserve":10.01692152}}}}),
            json!({"valueSat":0,"scriptPubKey":{"addresses":["RWrong"],
                "reserveoutput":{"currencyvalues":{"iReserve":10.01692152}}}}),
            json!({"valueSat":0,"scriptPubKey":{"addresses":[address],
                "reserveoutput":{"currencyvalues":{"iReserve":10.01692152,"iExtra":1}}}}),
        ] {
            assert_eq!(check(std::slice::from_ref(&transfer), 0, &[altered]), None);
        }
        let mut gateway = transfer.clone();
        gateway["destination"]["gateway"] = json!("iGateway");
        assert_eq!(
            check(&[gateway.clone()], 0, std::slice::from_ref(&output)),
            None
        );
        assert_eq!(
            check(
                &[gateway, transfer.clone()],
                1,
                &[output.clone(), output.clone()]
            ),
            None
        );
        assert_eq!(
            check(&[transfer.clone(), transfer], 1, &[output.clone(), output]),
            Some(ReplayProof {
                proceeds: None,
                offset: 1,
                satoshis: None,
                kind: ReplayKind::ReserveConversion
            })
        );
    }

    #[test]
    fn mixed_prefix_attributes_plain_reserve_only_with_exact_amount_and_destination() {
        let native = "iHog9UCTrn95qpUBFCZ7kKz7qWdMA8MQ6N";
        let reserve = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        let address = "RGUwZNAYW8tByUGzhxJywiXmVwKvwUfQMp";
        let first = json!({"version":1,"flags":1,"currencyvalues":{(native):0.5},
            "destination":{"type":2,"address":address}});
        let second = json!({"version":1,"flags":65,"currencyvalues":{(reserve):100.0},
            "destination":{"type":2,"address":address}});
        let first_output = json!({"valueSat":50000000,"scriptPubKey":{"type":"pubkeyhash",
            "hex":"76a9144f01c06114c899baab0522499ead597bf883e78988ac"}});
        let reserve_output = json!({"valueSat":0,"scriptPubKey":{"reserveoutput":{
            "currencyvalues":{(reserve):100.0}},"addresses":[address]}});
        let notarization = json!({"launchcomplete":true,"launchconfirmed":true,
            "currencystate":{"version":1}});
        let batch = [first, second];
        let outputs = [first_output, reserve_output.clone()];
        assert_eq!(
            replay_supported_prefix(&batch, 1, &json!({}), &notarization, native, &outputs),
            Some(ReplayProof {
                proceeds: None,
                offset: 1,
                satoshis: None,
                kind: ReplayKind::ReservePlain
            })
        );
        let mut altered = outputs.clone();
        altered[1]["scriptPubKey"]["reserveoutput"]["currencyvalues"][reserve] = json!(99.99999999);
        assert_eq!(
            replay_supported_prefix(&batch, 1, &json!({}), &notarization, native, &altered),
            None
        );
        altered[1] = reserve_output;
        altered[1]["scriptPubKey"]["addresses"] = json!([address, "RAnother"]);
        assert_eq!(
            replay_supported_prefix(&batch, 1, &json!({}), &notarization, native, &altered),
            None
        );
    }
}
