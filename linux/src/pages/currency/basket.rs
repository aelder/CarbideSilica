//! The arithmetic of a basket (`BasketMath` in `CurrencyView.swift`): its prices are units
//! of a reserve for one unit of the basket. Also the choice of the unit reserve and the
//! sentence above the reserves table.

use crate::format;

/// What one unit of a reserve is worth in the unit reserve: the price of the basket in the
/// unit, divided by its price in the reserve.
pub fn worth(price: Option<f64>, unit_price: Option<f64>) -> Option<f64> {
    match (price, unit_price) {
        (Some(price), Some(unit_price)) if price > 0.0 && unit_price > 0.0 => Some(unit_price / price),
        _ => None,
    }
}

/// A figure with as many decimals as its size needs.
pub fn text(value: f64) -> String {
    let size = value.abs();
    let digits = if size >= 1000.0 {
        0
    } else if size >= 10.0 {
        2
    } else if size >= 1.0 {
        3
    } else if size >= 0.01 {
        5
    } else {
        8
    };
    format::decimal(value, digits)
}

/// The reserve that values are shown in: the choice, or the chain's coin when it is a
/// reserve, or the first reserve.
pub fn unit(ids: &[String], chosen: Option<&str>, coin: Option<&str>) -> Option<String> {
    if let Some(chosen) = chosen.filter(|c| ids.iter().any(|id| id == c)) {
        return Some(chosen.to_string());
    }
    if let Some(coin) = coin.filter(|c| ids.iter().any(|id| id == c)) {
        return Some(coin.to_string());
    }
    ids.first().cloned()
}

/// The price of the basket in a reserve, when it is above zero.
pub fn positive(price: Option<f64>) -> Option<f64> {
    price.filter(|p| *p > 0.0)
}

/// "Bridge is a basket of 4 reserves at 25% each. One Bridge is worth 18.47 VRSC. The pool
/// holds the value of 216,028 VRSC." None without a unit price or a supply.
pub fn sentence(name: &str, weights: &[Option<f64>], unit_price: Option<f64>, supply: Option<f64>, unit_name: &str) -> Option<String> {
    let (unit_price, supply) = (unit_price?, supply?);
    let count = weights.len();
    let mut distinct: Vec<i64> = weights.iter().flatten().map(|w| (w * 1_000_000.0).round() as i64).collect();
    distinct.sort_unstable();
    distinct.dedup();
    let mut sentence = format!("{name} is a basket of {count} {}", if count == 1 { "reserve" } else { "reserves" });
    if count > 1 && distinct.len() == 1 && let Some(Some(weight)) = weights.first() {
        sentence += &format!(" at {} each", format::percent(*weight));
    }
    let total: f64 = weights.iter().flatten().sum();
    sentence += &format!(". One {name} is worth {} {unit_name}. ", text(unit_price));
    sentence += &format!("The pool holds the value of {} {unit_name}.", text(supply * unit_price * total));
    Some(sentence)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worth_is_the_unit_price_over_the_price() {
        assert_eq!(worth(Some(2.0), Some(18.0)), Some(9.0));
        assert_eq!(worth(Some(0.0), Some(18.0)), None);
        assert_eq!(worth(None, Some(18.0)), None);
        assert_eq!(worth(Some(2.0), Some(-1.0)), None);
    }

    #[test]
    fn figures_have_the_decimals_their_size_needs() {
        assert_eq!(text(54_007.4), "54,007");
        assert_eq!(text(18.46869061), "18.47");
        assert_eq!(text(1.5712), "1.571");
        assert_eq!(text(0.078992), "0.07899");
        assert_eq!(text(0.0078992), "0.0078992");
        assert_eq!(text(0.000123456789), "0.00012346");
    }

    #[test]
    fn the_unit_is_the_choice_then_the_coin_then_the_first() {
        let ids = vec!["iA".to_string(), "iCoin".to_string()];
        assert_eq!(unit(&ids, Some("iA"), Some("iCoin")).as_deref(), Some("iA"));
        assert_eq!(unit(&ids, Some("iGone"), Some("iCoin")).as_deref(), Some("iCoin"));
        assert_eq!(unit(&ids, None, Some("iOther")).as_deref(), Some("iA"));
        assert_eq!(unit(&[], None, None), None);
    }

    #[test]
    fn the_sentence_names_equal_weights() {
        let weights = [Some(0.25); 4];
        assert_eq!(
            sentence("Bridge", &weights, Some(18.4687), Some(11_696.996), "VRSC").unwrap(),
            "Bridge is a basket of 4 reserves at 25% each. One Bridge is worth 18.47 VRSC. The pool holds the value of 216,028 VRSC."
        );
        let mixed = [Some(0.5), Some(0.3)];
        assert_eq!(
            sentence("Pool", &mixed, Some(2.0), Some(10.0), "A").unwrap(),
            "Pool is a basket of 2 reserves. One Pool is worth 2 A. The pool holds the value of 16 A."
        );
        assert_eq!(sentence("Pool", &mixed, None, Some(10.0), "A"), None);
    }
}
