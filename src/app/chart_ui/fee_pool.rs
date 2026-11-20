//! Read bounded coinbase checkpoints; fee pools are state, not spendable outputs.
use super::*;
use rand::{Rng, seq::SliceRandom};

const MAX_POINTS: usize = 100;
// Two endpoints, up to one exact seek target, and four midpoint passes.
const INDIVIDUAL_UPDATES: usize = 2 + 1 + 1 + 2 + 4 + 8;
const UNIT: i128 = 100_000_000;

// Positive symmetric-log scale: linear from zero to the protocol's 0.0001
// minimum share, then logarithmic. Unlike log(balance), zero remains defined.
const LOG_KNEE: f64 = 0.0001;
pub(super) fn scale(value: f64, logarithmic: bool) -> f64 {
    if !logarithmic {
        value
    } else if value <= LOG_KNEE {
        value / LOG_KNEE
    } else {
        1.0 + (value / LOG_KNEE).log10()
    }
}
pub(super) fn axis_label(value: f64) -> String {
    let coins = if value <= 1.0 {
        value * LOG_KNEE
    } else {
        LOG_KNEE * 10_f64.powf(value - 1.0)
    };
    if coins >= 1_000_000.0 {
        return format!("{coins:.2e}");
    }
    format!("{coins:.8}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

pub(super) fn log_ticks(data: &[(f64, f64)], count: usize) -> Vec<f64> {
    let top = data
        .iter()
        .map(|p| p.1)
        .reduce(f64::max)
        .unwrap_or(1.0)
        .ceil()
        .max(1.0);
    // Integer transformed ticks are powers of ten in coin units, rather than
    // long decimal labels such as 316.22776602 on a generic 2.5-unit grid.
    let step = (top / count.max(2) as f64).ceil().max(1.0);
    let mut ticks = Vec::new();
    let mut tick = 0.0;
    while tick < top {
        ticks.push(tick);
        tick += step;
    }
    ticks.push(top);
    ticks
}

#[derive(Debug, Clone)]
pub(super) struct Series {
    pub labels: [String; 2],
    pub reserve: bool,
    pub current: Option<(u64, u64, [i128; 2])>,
    pub requested: usize,
    pub loaded: usize,
}

pub(super) fn amount(value: i128) -> String {
    format!("{}.{:08}", value / UNIT, value % UNIT)
}

pub(super) fn balances(values: [i128; 2], series: &Series) -> String {
    let native = format!("{} {}", amount(values[0]), series.labels[0]);
    if series.reserve {
        format!("{native} | {} {}", amount(values[1]), series.labels[1])
    } else {
        native
    }
}

// A valid, present checkpoint's omitted currency is zero. Absent/malformed
// checkpoints are unknown, including pre-activation blocks.
fn checkpoint(tx: &Value, native: &str, reserve: &str) -> Result<Option<[i128; 2]>> {
    anyhow::ensure!(
        tx["vin"]
            .as_array()
            .is_some_and(|vin| vin.len() == 1 && vin[0].get("coinbase").is_some()),
        "Expected a coinbase transaction"
    );
    let outputs = tx["vout"]
        .as_array()
        .context("Coinbase outputs unavailable")?;
    let pools = outputs
        .iter()
        .filter_map(|out| out["scriptPubKey"].get("feepool"))
        .collect::<Vec<_>>();
    if pools.is_empty() {
        return Ok(None);
    }
    anyhow::ensure!(pools.len() == 1, "Ambiguous fee-pool checkpoints");
    let pool = pools[0];
    anyhow::ensure!(
        pool["version"].as_u64() == Some(1),
        "Unsupported fee-pool version"
    );
    let entries = pool["currencyvalues"]
        .as_object()
        .context("Fee-pool amounts unavailable")?;
    let mut result = [0; 2];
    for (id, value) in entries {
        let value = value
            .as_number()
            .and_then(|_| decimal_value_to_zat(value))
            .filter(|v| (0..=i128::from(i64::MAX)).contains(v))
            .context("Invalid fee-pool amount")?;
        anyhow::ensure!(
            id == native || id == reserve || value == 0,
            "Unexpected fee-pool currency"
        );
        if id == native {
            result[0] = value;
        } else if id == reserve {
            result[1] = value;
        }
    }
    Ok(Some(result))
}

fn indices(count: usize, selected: Option<usize>, limit: usize) -> Vec<usize> {
    if count <= limit {
        return (0..count).collect();
    }
    // Keep a stable grid across paging/selection; an exact seek replaces only
    // one interior point rather than changing every previously cached height.
    let mut result = (0..limit)
        .map(|i| i * (count - 1) / (limit - 1))
        .collect::<Vec<_>>();
    if let Some(selected) = selected.filter(|i| *i < count)
        && !result.contains(&selected)
    {
        let nearest = (1..limit - 1)
            .min_by_key(|&i| result[i].abs_diff(selected))
            .unwrap();
        result[nearest] = selected;
        result.sort_unstable();
    }
    result
}

fn fetch_order(indices: &[usize], selected: Option<usize>, rng: &mut impl Rng) -> Vec<usize> {
    if indices.len() < 2 {
        return indices.to_vec();
    }
    let last = indices.len() - 1;
    let mut order = vec![indices[0], indices[last]];
    if let Some(selected) = selected
        && indices[1..last].contains(&selected)
    {
        order.push(selected);
    }

    let mut intervals = vec![(0, last)];
    while !intervals.is_empty() {
        let mut next = Vec::new();
        let start = order.len();
        for (left, right) in intervals {
            if right - left > 1 {
                let middle = left + (right - left) / 2;
                if Some(indices[middle]) != selected {
                    order.push(indices[middle]);
                }
                next.extend([(left, middle), (middle, right)]);
            }
        }
        order[start..].shuffle(rng);
        intervals = next;
    }
    order
}

async fn read(
    client: &RpcClient,
    cache: &Cache,
    config: &Config,
    source: &str,
    method: &str,
    params: &[Value],
) -> Result<Value> {
    if let Some(raw) = cache
        .raw_rpc_response_async(
            source,
            &config.chain_cache_key(),
            method,
            &json!(params).to_string(),
        )
        .await?
    {
        return rpc_result_value(&raw);
    }
    let response = client.call_with_raw::<Value>(method, params).await?;
    anyhow::ensure!(
        response.source_id == source,
        "Source changed while reading fee pool"
    );
    Ok(response.result)
}

async fn at(
    client: &RpcClient,
    cache: &Cache,
    config: &Config,
    source: &str,
    sample: &Sample,
    currencies: (&str, &str),
) -> Result<Option<[i128; 2]>> {
    let block = read(
        client,
        cache,
        config,
        source,
        "getblock",
        &[json!(sample.hash), json!(1)],
    )
    .await?;
    anyhow::ensure!(
        block["hash"].as_str() == Some(&sample.hash)
            && block["height"].as_u64() == Some(sample.height),
        "Fee-pool block mismatch"
    );
    let txid = block["tx"]
        .as_array()
        .and_then(|txs| txs.first())
        .and_then(Value::as_str)
        .filter(|id| valid_hash(id))
        .context("Coinbase reference unavailable")?;
    let tx = read(
        client,
        cache,
        config,
        source,
        "getrawtransaction",
        &[json!(txid), json!(1)],
    )
    .await?;
    anyhow::ensure!(
        tx["txid"].as_str() == Some(txid) && tx["blockhash"].as_str() == Some(&sample.hash),
        "Fee-pool coinbase/block mismatch"
    );
    checkpoint(&tx, currencies.0, currencies.1)
}

pub(super) async fn load(
    config: &Config,
    view: &View,
    page: &mut Page,
    events: &UnboundedSender<AppEvent>,
) -> Result<()> {
    let client = RpcClient::for_config(config)?;
    let cache = Cache::open(&config.cache_path)?;
    let info = client.get_info().await?;
    anyhow::ensure!(
        info.source_id == page.source,
        "Source changed before loading fee pool"
    );
    let native = info.result["chainid"]
        .as_str()
        .or_else(|| config.chain.system_id.get().map(|id| id.as_ref()))
        .context("Native currency ID unavailable")?;
    if let Some(expected) = config.chain.system_id.get() {
        anyhow::ensure!(native == expected.as_ref(), "Fee-pool system mismatch");
    }
    // Coinbase fee pools allow the local currency and the network's Verus
    // currency. The daemon's notary chain does not determine eligibility.
    let network = crate::world::Network::from_testnet(config.chain.testnet);
    let reserve_id = crate::world::system_id("VRSC", network)?;
    let reserve = reserve_id.as_ref();
    let reserve_label = if config.chain.testnet {
        "VRSCTEST"
    } else {
        "VRSC"
    };
    let native_label = if native == reserve {
        reserve_label
    } else {
        info.result["name"].as_str().unwrap_or(&config.chain.name)
    };
    let selected = view
        .selected
        .as_ref()
        .or_else(|| view.focus.as_ref().map(|s| &s.hash));
    let selected = selected.and_then(|hash| page.samples.iter().position(|s| &s.hash == hash));
    let indices = indices(
        page.samples.len(),
        selected,
        if view.pool_detail { 2000 } else { MAX_POINTS },
    );
    let order = fetch_order(&indices, selected, &mut rand::rng());
    for &index in &indices {
        page.samples[index].pool_scheduled = true;
    }
    page.pool = Some(Series {
        labels: [native_label.into(), reserve_label.into()],
        reserve: reserve != native,
        current: None,
        requested: indices.len(),
        loaded: 0,
    });

    // The current balance always refers to a fresh tip observation, even when the
    // displayed chart is historical. Recheck that exact tip after reading it.
    let tip = client.get_blockchain_info().await?;
    anyhow::ensure!(
        tip.source_id == page.source,
        "Source changed while locating latest fee pool"
    );
    let height = tip.result["blocks"]
        .as_u64()
        .context("Tip height unavailable")?;
    let hash = tip.result["bestblockhash"]
        .as_str()
        .filter(|h| valid_hash(h))
        .context("Tip hash unavailable")?;
    let header = read(
        &client,
        &cache,
        config,
        &page.source,
        "getblock",
        &[json!(hash), json!(1)],
    )
    .await?;
    let current = check_header(&header, hash, height)?;
    let values = at(
        &client,
        &cache,
        config,
        &page.source,
        &current,
        (native, reserve),
    )
    .await?;
    let canonical = client
        .call_with_raw::<String>("getblockhash", &[json!(height)])
        .await?;
    anyhow::ensure!(
        canonical.source_id == page.source && canonical.result == hash,
        "Latest fee-pool block changed; F5 retries"
    );
    if let Some(values) = values {
        page.pool.as_mut().unwrap().current = Some((height, current.time, values));
    }
    for (ordinal, index) in order.into_iter().enumerate() {
        let values = if page.samples[index].hash == current.hash {
            values
        } else {
            at(
                &client,
                &cache,
                config,
                &page.source,
                &page.samples[index],
                (native, reserve),
            )
            .await?
        };
        page.samples[index].pool_requested = true;
        page.samples[index].pool = values;
        if values.is_some() {
            page.pool.as_mut().unwrap().loaded += 1;
        }
        if ordinal < INDIVIDUAL_UPDATES || (ordinal + 1).is_multiple_of(8) {
            let _ = events.send(AppEvent::ChartUpdated {
                request: view.request,
                page: Some(page.clone()),
                done: false,
                error: None,
            });
        }
    }
    // Cached header windows start unverified; qualify their anchor after all
    // checkpoint reads. A historical orphan stays explicitly labeled as such.
    let canonical = client
        .call_with_raw::<String>("getblockhash", &[json!(page.last)])
        .await?;
    anyhow::ensure!(
        canonical.source_id == page.source,
        "Source changed while checking fee-pool history"
    );
    page.status = if page
        .samples
        .last()
        .is_some_and(|s| s.hash == canonical.result)
    {
        "Observed active chain"
    } else {
        "Orphan anchor; historical branch"
    }
    .into();
    Ok(())
}

pub(super) fn segments(page: &Page, currency: usize) -> Vec<Vec<(f64, f64)>> {
    let mut segments = Vec::new();
    let mut current = Vec::new();
    for sample in page.samples.iter().filter(|s| s.pool_scheduled) {
        let point = sample
            .pool
            .map(|p| (sample.time as f64, p[currency] as f64 / UNIT as f64));
        if (point.is_none()
            || current
                .last()
                .is_some_and(|p: &(f64, f64)| p.0 >= sample.time as f64))
            && !current.is_empty()
        {
            segments.push(std::mem::take(&mut current));
        }
        if let Some(point) = point {
            current.push(point);
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoints_distinguish_zero_missing_malformed_and_reserve() {
        let native = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        let reserve = "iHog9UCTrn95qpUBFCZ7kKz7qWdMA8MQ6N";
        let mut tx = json!({"vin":[{"coinbase":"00"}],"vout":[{"scriptPubKey":{"feepool":{"version":1,"currencyvalues":{native:1.00000001,reserve:2.5}}}}]});
        assert_eq!(
            checkpoint(&tx, native, reserve).unwrap(),
            Some([100000001, 250000000])
        );
        assert_eq!(amount(100000001), "1.00000001");
        tx["vout"][0]["scriptPubKey"]["feepool"]["currencyvalues"] = json!({});
        assert_eq!(checkpoint(&tx, native, reserve).unwrap(), Some([0, 0]));
        for value in [json!(-1), json!(0.000000001), json!("2"), json!(null)] {
            tx["vout"][0]["scriptPubKey"]["feepool"]["currencyvalues"] = json!({native:value});
            assert!(checkpoint(&tx, native, reserve).is_err());
        }
        tx["vout"] = json!([]);
        assert_eq!(checkpoint(&tx, native, reserve).unwrap(), None);
        tx["vin"] = json!([{"txid":"00"}]);
        assert!(checkpoint(&tx, native, reserve).is_err());
    }

    #[test]
    fn sampling_is_bounded_and_keeps_endpoints_and_requested_block() {
        for count in [0, 1, 99, 100, 101, 1441, 20000] {
            let selected = count / 2;
            let result = indices(count, Some(selected), MAX_POINTS);
            assert!(result.len() <= MAX_POINTS);
            if count > 0 {
                assert_eq!(result.first(), Some(&0));
                assert_eq!(result.last(), Some(&(count - 1)));
                assert!(result.contains(&selected));
            }
        }
    }

    #[test]
    fn fetch_order_shuffles_only_within_each_midpoint_pass() {
        use rand::{SeedableRng, rngs::StdRng};
        let grid = [0, 10, 20, 30, 40, 50, 60, 70, 80];
        let mut permutations = std::collections::BTreeSet::new();
        for seed in 0..16 {
            let mut rng = StdRng::seed_from_u64(seed);
            let order = fetch_order(&grid, None, &mut rng);
            assert_eq!(order[..3], [0, 80, 40]);
            let mut quarters = order[3..5].to_vec();
            quarters.sort_unstable();
            assert_eq!(quarters, [20, 60]);
            let mut remainder = order[5..].to_vec();
            remainder.sort_unstable();
            assert_eq!(remainder, [10, 30, 50, 70]);
            permutations.insert(order[5..].to_vec());

            let sought = fetch_order(&grid, Some(20), &mut rng);
            assert_eq!(sought[..5], [0, 80, 20, 40, 60]);
            let mut sorted = sought;
            sorted.sort_unstable();
            assert_eq!(sorted, grid);
        }
        assert!(permutations.len() > 1);
        let mut rng = StdRng::seed_from_u64(0);
        assert!(fetch_order(&[], None, &mut rng).is_empty());
        assert_eq!(fetch_order(&[3], Some(3), &mut rng), [3]);
        assert_eq!(fetch_order(&[3, 4], Some(4), &mut rng), [3, 4]);
    }
}
