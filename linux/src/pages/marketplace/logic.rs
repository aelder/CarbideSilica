//! The pure parts of the Marketplace (`MarketplaceView.swift`): what a name is made of,
//! one offer with the figures the catalogue filters by, the filters and their counts, the
//! prices that many offers share, the scale and the points of the price curve, and the
//! texts that depend on the figures.

use std::collections::{BTreeMap, HashMap};

use unicode_segmentation::UnicodeSegmentation;

use crate::bridge::api::{self, Offer, OfferKind, OfferSort};
use crate::format;

/// The filters of the catalogue, in their order on the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Facet {
    Offer,
    Name,
    Length,
    Price,
    Seller,
    Listed,
}

impl Facet {
    pub const ALL: [Facet; 6] = [Facet::Offer, Facet::Name, Facet::Length, Facet::Price, Facet::Seller, Facet::Listed];

    pub fn title(self) -> &'static str {
        match self {
            Facet::Offer => "Kind of offer",
            Facet::Name => "Kind of name",
            Facet::Length => "Length of name",
            Facet::Price => "Price",
            Facet::Seller => "Seller",
            Facet::Listed => "Offered",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    Latin,
    Emoji,
    Other,
    Digits,
}

impl NameKind {
    pub const ALL: [NameKind; 4] = [NameKind::Latin, NameKind::Emoji, NameKind::Other, NameKind::Digits];

    pub fn label(self) -> &'static str {
        match self {
            NameKind::Latin => "Latin letters",
            NameKind::Emoji => "Emoji and symbols",
            NameKind::Other => "Other scripts",
            NameKind::Digits => "Digits",
        }
    }
}

/// What a name is made of. The length is in characters as a person sees them: one emoji
/// built from several code points is one character.
#[derive(Clone, Debug, PartialEq)]
pub struct NameTraits {
    pub kind: NameKind,
    pub length: usize,
    pub scripts: Vec<String>,
    pub code_points: String,
}

impl NameTraits {
    pub fn new(name: &str) -> Self {
        let length = name.graphemes(true).count();
        let code_points = name.chars().map(|c| format!("U+{:04X}", c as u32)).collect::<Vec<_>>().join(" ");
        let letters: Vec<char> = name.chars().filter(|c| c.is_alphabetic()).collect();
        let mut scripts: Vec<String> = Vec::new();
        for letter in &letters {
            let script = script(*letter as u32);
            if !scripts.iter().any(|s| s == script) {
                scripts.push(script.to_string());
            }
        }
        let kind = if !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()) {
            NameKind::Digits
        } else if letters.is_empty() {
            NameKind::Emoji
        } else if scripts == ["Latin"] {
            NameKind::Latin
        } else {
            NameKind::Other
        };
        Self { kind, length, scripts, code_points }
    }
}

/// The script of a letter, for the scripts that names on the chain use.
fn script(value: u32) -> &'static str {
    match value {
        0x0041..=0x024F | 0x1E00..=0x1EFF | 0xFF21..=0xFF5A => "Latin",
        0x0370..=0x03FF | 0x1F00..=0x1FFF => "Greek",
        0x0400..=0x052F => "Cyrillic",
        0x0530..=0x058F => "Armenian",
        0x0590..=0x05FF => "Hebrew",
        0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF => "Arabic",
        0x0900..=0x097F => "Devanagari",
        0x0980..=0x09FF => "Bengali",
        0x0B80..=0x0BFF => "Tamil",
        0x0E00..=0x0E7F => "Thai",
        0x10A0..=0x10FF => "Georgian",
        0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF => "Hangul",
        0x3040..=0x309F => "Hiragana",
        0x30A0..=0x30FF | 0x31F0..=0x31FF => "Katakana",
        0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0x20000..=0x2A6DF => "Han",
        0x13000..=0x1342F => "Egyptian hieroglyphs",
        _ => "another script",
    }
}

/// One offer with the figures that the catalogue filters by.
#[derive(Clone, Debug)]
pub struct MarketItem {
    pub offer: Offer,
    /// The identity's name for an identity offer, otherwise the subject's name.
    pub title: String,
    /// The ask, when it is one amount of the price currency.
    pub price: Option<f64>,
    /// For an offer of a named identity.
    pub traits: Option<NameTraits>,
}

impl MarketItem {
    pub fn new(offer: Offer, price_currency: Option<&str>, name: impl Fn(&str) -> String) -> Self {
        let mut identity: Option<String> = None;
        for side in [&offer.offered, &offer.accepted] {
            if side.identity_id == offer.subject_id
                && let Some(found) = &side.identity_name
            {
                identity = Some(found.clone());
            }
        }
        let title = identity.clone().or_else(|| offer.subject_id.as_deref().map(&name)).unwrap_or_else(|| offer.category.clone());
        let traits = identity.as_deref().map(NameTraits::new);
        let price = match (&offer.ask_currency, offer.ask_sats) {
            (Some(currency), Some(sats)) if Some(currency.as_str()) == price_currency => Some(sats as f64 / 100_000_000.0),
            _ => None,
        };
        Self { offer, title, price, traits }
    }

    pub fn is_identity(&self) -> bool {
        self.traits.is_some()
    }

    pub fn kind_label(&self) -> &'static str {
        kind_label(self.offer.kind)
    }
}

pub fn kind_label(kind: OfferKind) -> &'static str {
    match kind {
        OfferKind::IdentityForSale => "For sale",
        OfferKind::IdentityWanted => "Wanted",
        OfferKind::CurrencySwap => "Swap",
    }
}

/// The currency that most offers ask for: the price currency.
pub fn price_currency(offers: &[Offer]) -> Option<String> {
    let mut asked: HashMap<&str, usize> = HashMap::new();
    for currency in offers.iter().filter_map(|o| o.ask_currency.as_deref()) {
        *asked.entry(currency).or_default() += 1;
    }
    asked.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0))).map(|(currency, _)| currency.to_string())
}

pub const PRICE_BANDS: [(&str, f64, f64); 6] = [
    ("21 or less", 0.0, 21.0),
    ("22 to 100", 21.0, 100.0),
    ("101 to 225", 100.0, 225.0),
    ("226 to 500", 225.0, 500.0),
    ("501 to 2,500", 500.0, 2_500.0),
    ("More than 2,500", 2_500.0, f64::INFINITY),
];

pub fn price_band(price: f64) -> Option<&'static str> {
    PRICE_BANDS.iter().find(|(_, low, high)| price > *low && price <= *high).map(|(label, _, _)| *label)
}

pub fn length_band(length: usize) -> String {
    if length >= 8 {
        "8 or more".into()
    } else if length >= 5 {
        "5 to 7".into()
    } else {
        length.to_string()
    }
}

/// A number rounded to a count of significant digits.
pub fn rounded_to_significant(value: f64, digits: i32) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let scale = 10f64.powf(digits as f64 - value.abs().log10().ceil());
    (value * scale).round() / scale
}

/// A time in hours as words: minutes, hours, days or years.
pub fn time_left(hours: f64) -> String {
    if hours < 1.0 {
        return format!("{} min", ((hours * 60.0) as i64).max(1));
    }
    if hours < 48.0 {
        return format!("{} h", hours as i64);
    }
    if hours < 24.0 * 365.0 * 2.0 {
        return format!("{} days", (hours / 24.0) as i64);
    }
    format!("{:.1} years", hours / 24.0 / 365.0)
}

/// The time since a Unix time, as `time_left`.
pub fn age(since: u64, now: f64) -> String {
    time_left((now - since as f64) / 3_600.0)
}

/// The option of the Offered filter for a listing time.
pub fn listed_band(time: u64, now: f64) -> &'static str {
    let days = now / 86_400.0 - time as f64 / 86_400.0;
    if days <= 90.0 {
        "In the last 90 days"
    } else if days <= 365.0 {
        "90 days to 1 year"
    } else if days <= 730.0 {
        "1 to 2 years"
    } else {
        "More than 2 years"
    }
}

pub const LISTED_OPTIONS: [&str; 4] = ["In the last 90 days", "90 days to 1 year", "1 to 2 years", "More than 2 years"];
pub const LENGTH_OPTIONS: [&str; 6] = ["1", "2", "3", "4", "5 to 7", "8 or more"];

/// The sellers by the number of their offers, most first, then by address.
pub fn sellers<'a>(holders: impl IntoIterator<Item = &'a str>) -> Vec<(String, usize)> {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for holder in holders {
        *counts.entry(holder).or_default() += 1;
    }
    let mut list: Vec<(String, usize)> = counts.into_iter().map(|(a, c)| (a.to_string(), c)).collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    list
}

/// The sellers that the Seller filter names: the five largest with five offers or more.
pub fn named_sellers(sellers: &[(String, usize)]) -> Vec<String> {
    sellers.iter().filter(|s| s.1 >= 5).take(5).map(|s| s.0.clone()).collect()
}

/// A seller's address as the filter and the rows show it.
pub fn short_seller(address: &str) -> String {
    format::short(address, 6, 4)
}

/// What the page knows of one offer beyond the offer itself.
#[derive(Clone, Copy, Default)]
pub struct Known<'a> {
    pub holder: Option<&'a str>,
    pub listed: Option<u64>,
}

/// The option of a filter that an offer belongs to.
pub fn value(facet: Facet, item: &MarketItem, known: Known, named: &[String], now: f64) -> Option<String> {
    match facet {
        Facet::Offer => Some(item.kind_label().to_string()),
        Facet::Name => item.traits.as_ref().map(|t| t.kind.label().to_string()),
        Facet::Length => item.traits.as_ref().map(|t| length_band(t.length)),
        Facet::Price => item.price.and_then(price_band).map(str::to_string),
        Facet::Seller => {
            let holder = known.holder?;
            Some(if named.iter().any(|n| n == holder) { short_seller(holder) } else { "Other sellers".into() })
        }
        Facet::Listed => known.listed.map(|time| listed_band(time, now).to_string()),
    }
}

pub type Values = HashMap<Facet, String>;

/// True when the offer's title has the search text and the offer is in each selected
/// option, except the option of `skipping`.
pub fn matches(title: &str, own: Option<&Values>, selected: &BTreeMap<Facet, String>, search: &str, skipping: Option<Facet>) -> bool {
    let query = search.trim();
    if !query.is_empty() && !title.to_lowercase().contains(&query.to_lowercase()) {
        return false;
    }
    for (facet, option) in selected {
        if Some(*facet) == skipping {
            continue;
        }
        if own.and_then(|o| o.get(facet)) != Some(option) {
            return false;
        }
    }
    true
}

/// How many offers each option of a filter leaves, with the other filters applied.
pub fn counts(items: &[MarketItem], values: &HashMap<String, Values>, selected: &BTreeMap<Facet, String>, search: &str, facet: Facet) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for item in items {
        let own = values.get(&item.offer.txid);
        if matches(&item.title, own, selected, search, Some(facet))
            && let Some(option) = own.and_then(|o| o.get(&facet))
        {
            *counts.entry(option.clone()).or_default() += 1;
        }
    }
    counts
}

/// The options of a filter. A filter with none is not shown.
pub fn options(facet: Facet, items: &[MarketItem], detailed: usize, sellers: &[(String, usize)]) -> Vec<String> {
    match facet {
        Facet::Offer => {
            let kinds: Vec<&str> = items.iter().map(MarketItem::kind_label).collect();
            let distinct = ["For sale", "Wanted", "Swap"].iter().filter(|k| kinds.contains(k)).count();
            if distinct > 1 {
                ["For sale", "Wanted", "Swap"].iter().filter(|k| kinds.contains(k)).map(|k| k.to_string()).collect()
            } else {
                vec![]
            }
        }
        Facet::Name => NameKind::ALL.iter().map(|k| k.label().to_string()).collect(),
        Facet::Length => LENGTH_OPTIONS.iter().map(|s| s.to_string()).collect(),
        Facet::Price => PRICE_BANDS.iter().map(|b| b.0.to_string()).collect(),
        Facet::Seller => {
            if detailed == 0 {
                return vec![];
            }
            let mut options: Vec<String> = named_sellers(sellers).iter().map(|s| short_seller(s)).collect();
            if sellers.len() > options.len() {
                options.push("Other sellers".into());
            }
            options
        }
        Facet::Listed => {
            if detailed > 0 {
                LISTED_OPTIONS.iter().map(|s| s.to_string()).collect()
            } else {
                vec![]
            }
        }
    }
}

/// The filters in words: "Name contains "x" · Price: 21 or less".
pub fn applied_text(selected: &BTreeMap<Facet, String>, search: &str) -> String {
    let mut parts: Vec<String> = Facet::ALL.iter().filter_map(|f| selected.get(f).map(|o| format!("{}: {o}", f.title()))).collect();
    let query = search.trim();
    if !query.is_empty() {
        parts.insert(0, format!("Name contains \"{query}\""));
    }
    if parts.is_empty() { "No filter is applied.".into() } else { parts.join(" · ") }
}

pub fn sort_note(sort: OfferSort, descending: bool, detailed: usize, count: usize) -> String {
    match sort {
        OfferSort::Expiry => (if descending { "Latest expiry first." } else { "Soonest expiry first." }).into(),
        OfferSort::Name => (if descending { "Names Z to A." } else { "Names A to Z." }).into(),
        OfferSort::AskAmount => format!(
            "Grouped by ask currency, {} first. Offers with no single ask are last.",
            if descending { "largest" } else { "smallest" }
        ),
        OfferSort::ListingAge => {
            let order = if descending { "Oldest listing first." } else { "Newest listing first." };
            if detailed < count { format!("{order} Listing dates: {detailed} of {count} read.") } else { order.into() }
        }
    }
}

/// One bar of the prices that many offers share.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub label: String,
    pub count: usize,
    pub other: bool,
}

/// The exact prices with the most offers, cheapest first, and one row for the rest.
pub fn price_steps(prices: &[f64], ticker: &str) -> Vec<Step> {
    let mut counts: Vec<(f64, usize)> = Vec::new();
    for price in prices {
        match counts.iter_mut().find(|(p, _)| p == price) {
            Some(entry) => entry.1 += 1,
            None => counts.push((*price, 1)),
        }
    }
    let floor = (prices.len() / 60).max(3);
    let mut common: Vec<(f64, usize)> = counts.into_iter().filter(|c| c.1 >= floor).collect();
    common.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)));
    common.truncate(9);
    common.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut steps: Vec<Step> =
        common.iter().map(|(price, count)| Step { label: format!("{} {ticker}", format::amount(*price)), count: *count, other: false }).collect();
    let rest = prices.len() - common.iter().map(|c| c.1).sum::<usize>();
    if rest > 0 {
        steps.push(Step { label: "each other price".into(), count: rest, other: true });
    }
    steps
}

/// The ends of the logarithmic price scale: whole powers of ten around the prices.
pub fn price_bounds(prices: &[f64]) -> (f64, f64) {
    let min = prices.iter().cloned().fold(f64::INFINITY, f64::min);
    let max = prices.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let low = (if min.is_finite() { min } else { 1.0 }).max(1e-8);
    let high = (if max.is_finite() { max } else { 10.0 }).max(low * 10.0);
    (10f64.powf(low.log10().floor()), 10f64.powf(high.log10().ceil()))
}

/// The left and right margins of the curve in its canvas.
pub const CURVE_LEFT: f64 = 46.0;
pub const CURVE_RIGHT_MARGIN: f64 = 10.0;
pub const CURVE_TOP: f64 = 8.0;
pub const CURVE_BOTTOM_MARGIN: f64 = 22.0;

/// The price under a point of the curve's canvas.
pub fn price_at_x(x: f64, width: f64, bounds: (f64, f64)) -> f64 {
    let (left, right) = (CURVE_LEFT, width - CURVE_RIGHT_MARGIN);
    let share = ((x - left) / (right - left)).clamp(0.0, 1.0);
    10f64.powf(bounds.0.log10() + share * (bounds.1.log10() - bounds.0.log10()))
}

/// The x of a price in the curve's canvas.
pub fn x_of_price(price: f64, width: f64, bounds: (f64, f64)) -> f64 {
    let (left, right) = (CURVE_LEFT, width - CURVE_RIGHT_MARGIN);
    left + (price.max(bounds.0).log10() - bounds.0.log10()) / (bounds.1.log10() - bounds.0.log10()) * (right - left)
}

/// The y of a count of offers in the curve's canvas.
pub fn y_of_count(count: usize, total: usize, height: f64) -> f64 {
    let base = height - CURVE_BOTTOM_MARGIN;
    base - count as f64 / total as f64 * (base - CURVE_TOP)
}

/// The points of the curve of offers at or below each price: a step up at each price,
/// from the bottom left to the top right. `sorted` is in ascending order.
pub fn curve_points(sorted: &[f64], width: f64, height: f64) -> Vec<(f64, f64)> {
    if sorted.is_empty() {
        return vec![];
    }
    let bounds = price_bounds(sorted);
    let base = height - CURVE_BOTTOM_MARGIN;
    let mut points = vec![(CURVE_LEFT, base)];
    for (index, price) in sorted.iter().enumerate() {
        let x = x_of_price(*price, width, bounds);
        points.push((x, y_of_count(index, sorted.len(), height)));
        points.push((x, y_of_count(index + 1, sorted.len(), height)));
    }
    points.push((width - CURVE_RIGHT_MARGIN, CURVE_TOP));
    points
}

/// The line under the curve.
pub fn supply_readout(sorted: &[f64], hovered: Option<f64>, ticker: &str) -> String {
    match hovered {
        None => match sorted.get(sorted.len() / 2) {
            Some(middle) => format!(
                "Half of the offers ask {} {ticker} or less. Move the pointer over the curve to read it.",
                format::amount(*middle)
            ),
            None => String::new(),
        },
        Some(price) => {
            let count = sorted.iter().filter(|p| **p <= price).count();
            format!(
                "{} offer{} at {} {ticker} or less.",
                format::integer(count as u64),
                if count == 1 { "" } else { "s" },
                format::amount(rounded_to_significant(price, 3))
            )
        }
    }
}

/// The note below the prices.
pub fn prices_note(unpriced: usize, ticker: &str) -> String {
    let mut text = String::from("Each step up in the curve is a price that many offers share. The price scale is logarithmic. ");
    if unpriced > 0 {
        text.push_str(&format!(
            "{unpriced} offer{} not ask for one amount of {ticker} and {} not in these figures.",
            if unpriced == 1 { " does" } else { "s do" },
            if unpriced == 1 { "is" } else { "are" }
        ));
    }
    text
}

/// The share of a bar on the same logarithmic scale as the price curve.
pub fn bar_share(price: f64, bounds: (f64, f64)) -> f64 {
    let (low, high) = (bounds.0.log10(), bounds.1.log10());
    if high <= low {
        return 1.0;
    }
    ((price.max(bounds.0).log10() - low) / (high - low)).clamp(0.02, 1.0)
}

/// A percentage of offers, rounded to a whole number.
pub fn percent(part: usize, whole: usize) -> String {
    format!("{}%", (part as f64 / whole.max(1) as f64 * 100.0).round() as i64)
}

/// The amounts of one side of an offer in words.
pub fn asset(side: &api::OfferAsset, identity: impl Fn(&str) -> String, ticker: impl Fn(&str) -> String) -> String {
    if let Some(id) = &side.identity_id {
        return format!("ID {}", identity(id));
    }
    if let Some(name) = &side.identity_name {
        return format!("ID {name}@");
    }
    if side.amounts.is_empty() {
        return "asset".into();
    }
    side.amounts.iter().map(|a| format!("{} {}", format::amount(a.amount), ticker(&a.currency))).collect::<Vec<_>>().join(" + ")
}

/// As Carbide: the exact asking amount, or what kind of ask it is.
pub fn ask(offer: &Offer, ticker: impl Fn(&str) -> String) -> String {
    if let (Some(currency), Some(sats)) = (&offer.ask_currency, offer.ask_sats) {
        return format!("{} {}", format::coins(sats), ticker(currency));
    }
    if offer.accepted.identity_id.is_some() || offer.accepted.identity_name.is_some() {
        return "Identity exchange".into();
    }
    "Asset bundle".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::api::{CurrencyAmount, OfferAsset};

    fn side(id: Option<&str>, name: Option<&str>) -> OfferAsset {
        OfferAsset { identity_id: id.map(Into::into), identity_name: name.map(Into::into), amounts: vec![] }
    }

    fn offer(txid: &str, name: &str, sats: Option<i64>, kind: OfferKind) -> Offer {
        Offer {
            txid: txid.into(),
            category: "ids_for_currency".into(),
            kind,
            subject_id: Some(format!("i{name}")),
            ask_currency: sats.map(|_| "iVRSC".into()),
            ask_sats: sats,
            offered: side(Some(&format!("i{name}")), Some(name)),
            accepted: OfferAsset { identity_id: None, identity_name: None, amounts: vec![CurrencyAmount { currency: "iVRSC".into(), amount: 1.0 }] },
            expiry_height: 0,
            links: vec![],
        }
    }

    #[test]
    fn traits_of_names() {
        let latin = NameTraits::new("sat");
        assert_eq!((latin.kind, latin.length), (NameKind::Latin, 3));
        assert_eq!(latin.code_points, "U+0073 U+0061 U+0074");
        assert_eq!(NameTraits::new("2024").kind, NameKind::Digits);
        let emoji = NameTraits::new("👨‍👩‍👧");
        assert_eq!((emoji.kind, emoji.length), (NameKind::Emoji, 1));
        assert!(emoji.scripts.is_empty());
        let arabic = NameTraits::new("صحراء");
        assert_eq!(arabic.kind, NameKind::Other);
        assert_eq!(arabic.scripts, vec!["Arabic".to_string()]);
        let mixed = NameTraits::new("aб");
        assert_eq!(mixed.scripts, vec!["Latin".to_string(), "Cyrillic".to_string()]);
        assert_eq!(mixed.kind, NameKind::Other);
        assert_eq!(NameTraits::new("금").scripts, vec!["Hangul".to_string()]);
    }

    #[test]
    fn bands() {
        assert_eq!(price_band(21.0), Some("21 or less"));
        assert_eq!(price_band(21.5), Some("22 to 100"));
        assert_eq!(price_band(2_500.0), Some("501 to 2,500"));
        assert_eq!(price_band(1e9), Some("More than 2,500"));
        assert_eq!(price_band(0.0), None);
        assert_eq!(length_band(1), "1");
        assert_eq!(length_band(5), "5 to 7");
        assert_eq!(length_band(8), "8 or more");
        let now = 1_000.0 * 86_400.0;
        assert_eq!(listed_band(999 * 86_400, now), "In the last 90 days");
        assert_eq!(listed_band(700 * 86_400, now), "90 days to 1 year");
        assert_eq!(listed_band(0, now), "More than 2 years");
    }

    #[test]
    fn items_and_prices() {
        let item = MarketItem::new(offer("t1", "sat", Some(21 * 100_000_000), OfferKind::IdentityForSale), Some("iVRSC"), |id| id.into());
        assert_eq!(item.title, "sat");
        assert_eq!(item.price, Some(21.0));
        assert!(item.is_identity());
        let other = MarketItem::new(offer("t2", "x", Some(5), OfferKind::CurrencySwap), Some("iOTHER"), |id| id.into());
        assert_eq!(other.price, None);
        assert_eq!(other.kind_label(), "Swap");
        let offers = vec![offer("a", "a", Some(1), OfferKind::IdentityForSale), offer("b", "b", None, OfferKind::IdentityForSale)];
        assert_eq!(price_currency(&offers).as_deref(), Some("iVRSC"));
    }

    #[test]
    fn filters_and_counts() {
        let items: Vec<MarketItem> = [("t1", "sat", 21), ("t2", "satoshi", 225), ("t3", "金", 21)]
            .iter()
            .map(|(t, n, p)| MarketItem::new(offer(t, n, Some(p * 100_000_000), OfferKind::IdentityForSale), Some("iVRSC"), |id| id.into()))
            .collect();
        let values: HashMap<String, Values> = items
            .iter()
            .map(|item| {
                let own: Values = Facet::ALL
                    .iter()
                    .filter_map(|f| value(*f, item, Known::default(), &[], 0.0).map(|v| (*f, v)))
                    .collect();
                (item.offer.txid.clone(), own)
            })
            .collect();
        let mut selected = BTreeMap::new();
        selected.insert(Facet::Price, "21 or less".to_string());
        let names = counts(&items, &values, &selected, "", Facet::Name);
        assert_eq!(names.get("Latin letters"), Some(&1));
        assert_eq!(names.get("Other scripts"), Some(&1));
        // The count of a filter's own options leaves that filter out.
        let prices = counts(&items, &values, &selected, "", Facet::Price);
        assert_eq!(prices.get("21 or less"), Some(&2));
        assert_eq!(prices.get("101 to 225"), Some(&1));
        assert!(matches("Satoshi", values.get("t2"), &BTreeMap::new(), " SAT ", None));
        assert!(!matches("Satoshi", values.get("t2"), &selected, "", None));
        assert_eq!(applied_text(&selected, " sat "), "Name contains \"sat\" · Price: 21 or less");
        assert_eq!(applied_text(&BTreeMap::new(), ""), "No filter is applied.");
        assert!(options(Facet::Offer, &items, 0, &[]).is_empty());
        assert!(options(Facet::Seller, &items, 0, &[]).is_empty());
        let sellers = sellers(["RAAAAAAAAAAAAAAA"; 6].into_iter().chain(["RBBBBBBBBBBBBBBB"]));
        assert_eq!(sellers[0].1, 6);
        assert_eq!(options(Facet::Seller, &items, 7, &sellers), vec!["RAAAAA…AAAA".to_string(), "Other sellers".to_string()]);
    }

    #[test]
    fn steps_and_curve() {
        let mut prices = vec![21.0; 10];
        prices.extend([225.0; 4]);
        prices.extend([1.0, 2.0]);
        let steps = price_steps(&prices, "VRSC");
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0], Step { label: "21 VRSC".into(), count: 10, other: false });
        assert_eq!(steps[1].label, "225 VRSC");
        assert_eq!(steps[2], Step { label: "each other price".into(), count: 2, other: true });

        prices.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(price_bounds(&prices), (1.0, 1_000.0));
        assert_eq!(price_bounds(&[]), (1.0, 10.0));
        let points = curve_points(&prices, 400.0, 190.0);
        assert_eq!(points.len(), prices.len() * 2 + 2);
        assert_eq!(points[0], (CURVE_LEFT, 168.0));
        assert_eq!(*points.last().unwrap(), (390.0, CURVE_TOP));
        // The first price is at the left end of the scale; the curve only rises.
        assert!((points[1].0 - CURVE_LEFT).abs() < 1e-9);
        assert!(points.windows(2).all(|w| w[1].1 <= w[0].1 && w[1].0 >= w[0].0));
        let x = x_of_price(225.0, 400.0, (1.0, 1_000.0));
        assert!((price_at_x(x, 400.0, (1.0, 1_000.0)) - 225.0).abs() < 1e-6);
        assert_eq!(supply_readout(&prices, Some(21.0), "VRSC"), "12 offers at 21 VRSC or less.");
        assert_eq!(supply_readout(&prices, Some(1.0), "VRSC"), "1 offer at 1 VRSC or less.");
        assert_eq!(
            supply_readout(&prices, None, "VRSC"),
            "Half of the offers ask 21 VRSC or less. Move the pointer over the curve to read it."
        );
        assert_eq!(bar_share(1.0, (1.0, 1_000.0)), 0.02);
        assert!((bar_share(1_000.0, (1.0, 1_000.0)) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn words() {
        assert_eq!(time_left(0.001), "1 min");
        assert_eq!(time_left(0.5), "30 min");
        assert_eq!(time_left(47.9), "47 h");
        assert_eq!(time_left(24.0 * 175.0), "175 days");
        assert_eq!(time_left(24.0 * 365.0 * 5.8), "5.8 years");
        assert_eq!(rounded_to_significant(1234.5, 3), 1230.0);
        assert_eq!(rounded_to_significant(0.012345, 3), 0.0123);
        assert_eq!(percent(93, 100), "93%");
        assert_eq!(percent(1, 0), "100%");
        assert_eq!(prices_note(1, "VRSC").ends_with("1 offer does not ask for one amount of VRSC and is not in these figures."), true);
        assert!(prices_note(2, "VRSC").ends_with("2 offers do not ask for one amount of VRSC and are not in these figures."));
        assert_eq!(sort_note(OfferSort::ListingAge, false, 3, 5), "Newest listing first. Listing dates: 3 of 5 read.");
        assert_eq!(sort_note(OfferSort::AskAmount, true, 0, 0), "Grouped by ask currency, largest first. Offers with no single ask are last.");
        let o = offer("t", "sat", Some(2_100_000_000), OfferKind::IdentityForSale);
        assert_eq!(ask(&o, |_| "VRSC".into()), "21 VRSC");
        assert_eq!(asset(&o.offered, |id| format!("{id}@"), |c| c.into()), "ID isat@");
        assert_eq!(asset(&o.accepted, |id| id.into(), |_| "VRSC".into()), "1 VRSC");
    }
}
