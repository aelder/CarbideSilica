//! Launch state comes from CCoinbaseCurrencyState, not definition options,
//! balances, dates, or the distinct notarization flag bitfield.
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchStatus {
    Launched,
    Prelaunch,
    Failed,
    Unknown,
}

impl LaunchStatus {
    pub const ALL: [Self; 4] = [Self::Launched, Self::Prelaunch, Self::Failed, Self::Unknown];
    pub const ALL_MASK: u8 = 15;
    pub const DEFAULT_MASK: u8 = Self::ALL_MASK & !Self::Failed.bit();

    pub const fn bit(self) -> u8 {
        match self {
            Self::Launched => 1,
            Self::Prelaunch => 2,
            Self::Failed => 4,
            Self::Unknown => 8,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Launched => "Launched",
            Self::Prelaunch => "Prelaunch",
            Self::Failed => "Failed launch (refunding)",
            Self::Unknown => "Launch status unknown",
        }
    }
}

pub fn classify(state: &Value) -> LaunchStatus {
    let Some(flags) = state["flags"].as_u64() else {
        return LaunchStatus::Unknown;
    };
    // Refunds take precedence even when a launch-complete marker is set.
    if flags & 4 != 0 {
        LaunchStatus::Failed
    } else if flags & 2 != 0 {
        LaunchStatus::Prelaunch
    } else if flags & 0x10 != 0 {
        LaunchStatus::Launched
    } else {
        LaunchStatus::Unknown
    }
}

/// Keep the best observation when present; fall back only if it is absent.
pub fn reported_state(record: &Value) -> &Value {
    [
        &record["bestcurrencystate"],
        &record["lastconfirmedcurrencystate"],
        &record["lastconfirmednotarization"]["currencystate"],
    ]
    .into_iter()
    .find(|state| state.is_object())
    .unwrap_or(&Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bots_refunding_states_remain_failed_after_launch_completion() {
        // Observed Bots launch-clear, refund-batch and closing states.
        for flags in [13, 5, 37] {
            assert_eq!(classify(&json!({"flags":flags})), LaunchStatus::Failed);
        }
        // Completion alone is not evidence of successful launch.
        assert_eq!(classify(&json!({"flags":33})), LaunchStatus::Unknown);
        assert_eq!(classify(&json!({"flags":49})), LaunchStatus::Launched);
    }

    #[test]
    fn explicit_state_flags_outweigh_balances_and_confirmed_prelaunch_is_still_pending() {
        assert_eq!(
            classify(&json!({"flags":3,"supply":100})),
            LaunchStatus::Prelaunch
        );
        assert_eq!(classify(&json!({"flags":19})), LaunchStatus::Prelaunch);
        assert_eq!(
            classify(&json!({"flags":49,"supply":0,"reservecurrencies":[]})),
            LaunchStatus::Launched
        );
        assert_eq!(classify(&json!({"supply":0})), LaunchStatus::Unknown);
        assert_eq!(classify(&json!({"flags":0})), LaunchStatus::Unknown);
    }

    #[test]
    fn state_fallback_preserves_best_observation_and_notarization_flags_are_not_used() {
        let record = json!({"lastconfirmednotarization":{"flags":4,"currencystate":{"flags":49}}});
        assert_eq!(classify(reported_state(&record)), LaunchStatus::Launched);
        let record = json!({"bestcurrencystate":{},"lastconfirmedcurrencystate":{"flags":4}});
        assert_eq!(classify(reported_state(&record)), LaunchStatus::Unknown);
    }
}
