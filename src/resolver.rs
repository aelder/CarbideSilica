use crate::domain::ObjectRoute;

pub fn currency_query(input: &str) -> Option<&str> {
    let input = input.trim();
    input
        .strip_prefix("currency:")
        .or_else(|| input.strip_suffix("@@"))
        .map(|target| target.trim().trim_end_matches('@').trim_end())
        .filter(|target| !target.is_empty())
}

pub fn classify_start_object(value: Option<&str>) -> ObjectRoute {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return ObjectRoute::Tip;
    };

    if let Some((prefix, target)) = value.split_once(':')
        && prefix.eq_ignore_ascii_case("estimate")
    {
        return ObjectRoute::Estimate(target.trim().into());
    }
    if let Some((prefix, time)) = value.split_once(':')
        && ["time", "date", "unix"]
            .iter()
            .any(|name| prefix.eq_ignore_ascii_case(name))
    {
        return ObjectRoute::BlockTime(time.trim().to_string());
    }

    if let Ok(height) = value.parse::<u64>() {
        return ObjectRoute::BlockHeight(height);
    }

    if value.len() == 64 && value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return ObjectRoute::Hash(value.to_ascii_lowercase());
    }

    let date_shaped = !value.contains('@')
        && value
            .as_bytes()
            .get(4)
            .is_some_and(|byte| matches!(byte, b'-' | b'/'))
        && value
            .as_bytes()
            .get(..4)
            .is_some_and(|year| year.iter().all(u8::is_ascii_digit));
    if date_shaped || crate::block_time::parse(value).is_ok() {
        return ObjectRoute::BlockTime(value.to_string());
    }

    if value.ends_with("@@") {
        return ObjectRoute::Unknown(value.to_string());
    }
    if value.ends_with('@') || value.contains('.') && value.contains('@') {
        return ObjectRoute::Identity(value.to_string());
    }

    if looks_like_identity_address(value) {
        return ObjectRoute::Identity(value.to_string());
    }

    if looks_like_verus_address(value) {
        return ObjectRoute::Address(value.to_string());
    }

    ObjectRoute::Unknown(value.to_string())
}

fn looks_like_identity_address(value: &str) -> bool {
    value.starts_with('i') && value.len() == 34 && looks_like_verus_address(value)
}

fn looks_like_verus_address(value: &str) -> bool {
    let len = value.len();
    (30..=80).contains(&len)
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '@')
}

#[cfg(test)]
mod tests {
    use super::classify_start_object;
    use crate::domain::ObjectRoute;

    #[test]
    fn classifies_time_queries_without_stealing_numeric_heights() {
        for query in [
            "time:1700000000",
            "date:Nov 7 2026",
            "unix:-1",
            "time:",
            "time:bad input",
        ] {
            assert!(matches!(
                classify_start_object(Some(query)),
                ObjectRoute::BlockTime(_)
            ));
        }
        for query in ["2026-11-07", "November 7 2026", "yesterday"] {
            assert_eq!(
                classify_start_object(Some(query)),
                ObjectRoute::BlockTime(query.to_string())
            );
        }
        assert_eq!(
            classify_start_object(Some("1700000000")),
            ObjectRoute::BlockHeight(1700000000)
        );
    }

    #[test]
    fn classifies_height() {
        assert_eq!(
            classify_start_object(Some("123")),
            ObjectRoute::BlockHeight(123)
        );
    }

    #[test]
    fn classifies_hash() {
        let hash = "a".repeat(64);
        assert_eq!(classify_start_object(Some(&hash)), ObjectRoute::Hash(hash));
    }

    #[test]
    fn classifies_identity() {
        assert_eq!(
            classify_start_object(Some("VRSC@")),
            ObjectRoute::Identity("VRSC@".to_string())
        );
    }

    #[test]
    fn classifies_verus_address() {
        let address = "RC5eNCTFke2KxATzUMmdUiBYBGBHnEZfou";
        assert_eq!(
            classify_start_object(Some(address)),
            ObjectRoute::Address(address.to_string())
        );
    }

    #[test]
    fn classifies_identity_address() {
        let identity_address = "i5w5MuNik5NtLcYmNzcvaoixooEebB6MGV";
        assert_eq!(
            classify_start_object(Some(identity_address)),
            ObjectRoute::Identity(identity_address.to_string())
        );
    }
}
