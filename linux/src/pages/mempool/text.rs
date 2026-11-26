//! Words and figures of the Mempool page (`MempoolText` and the small formats of
//! `MempoolView.swift`).

/// A length of time in words: seconds to two minutes, then minutes (`MempoolText.span`).
pub fn span(seconds: f64) -> String {
    let seconds = if seconds.is_finite() { seconds.round().max(0.0) as i64 } else { 0 };
    if seconds < 120 {
        return format!("{seconds} s");
    }
    if seconds < 3600 {
        return format!("{} min", seconds / 60);
    }
    format!("{} h {} min", seconds / 3600, seconds % 3600 / 60)
}

/// "1 transaction", "2 transactions" (`MempoolText.count`).
pub fn count(number: usize, one: &str, many: &str) -> String {
    format!("{number} {}", if number == 1 { one } else { many })
}

/// "6:08" for six minutes and eight seconds (`MempoolTimeRecord.clock`).
pub fn clock(seconds: f64) -> String {
    let whole = if seconds.is_finite() { seconds.max(0.0) as i64 } else { 0 };
    if whole >= 3600 {
        format!("{}:{:02}:{:02}", whole / 3600, whole / 60 % 60, whole % 60)
    } else {
        format!("{}:{:02}", whole / 60, whole % 60)
    }
}

/// The part of the next block that the waiting transactions fill, as the readout says it.
pub fn fill_percent(waiting: usize, share: f64) -> String {
    let percent = share * 100.0;
    if waiting == 0 {
        "0%".into()
    } else if percent < 1.0 {
        format!("{percent:.2}%")
    } else {
        format!("{percent:.0}%")
    }
}

/// "a, b and c" (`StatementText.list`).
pub fn list(parts: &[String]) -> String {
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        n => format!("{} and {}", parts[..n - 1].join(", "), parts[n - 1]),
    }
}

/// The first letter in capitals (`StatementText.capital`).
pub fn capital(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans() {
        assert_eq!(span(0.4), "0 s");
        assert_eq!(span(-5.0), "0 s");
        assert_eq!(span(119.4), "119 s");
        assert_eq!(span(120.0), "2 min");
        assert_eq!(span(3599.0), "59 min");
        assert_eq!(span(3600.0 + 7.0 * 60.0 + 30.0), "1 h 7 min");
    }

    #[test]
    fn counts_and_clocks() {
        assert_eq!(count(1, "transaction", "transactions"), "1 transaction");
        assert_eq!(count(0, "transaction", "transactions"), "0 transactions");
        assert_eq!(clock(368.9), "6:08");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(clock(-3.0), "0:00");
    }

    #[test]
    fn percents() {
        assert_eq!(fill_percent(0, 0.0), "0%");
        assert_eq!(fill_percent(2, 0.001_234), "0.12%");
        assert_eq!(fill_percent(2, 0.456), "46%");
        assert_eq!(fill_percent(2, 1.5), "150%");
    }

    #[test]
    fn lists() {
        let parts = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(list(&parts(&[])), "");
        assert_eq!(list(&parts(&["a"])), "a");
        assert_eq!(list(&parts(&["a", "b"])), "a and b");
        assert_eq!(list(&parts(&["a", "b", "c"])), "a, b and c");
        assert_eq!(capital("the pool"), "The pool");
        assert_eq!(capital(""), "");
    }
}
