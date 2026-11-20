use chrono::{DateTime, Local, SecondsFormat, Utc};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TimeDisplay {
    #[default]
    Utc,
    Local,
}

impl TimeDisplay {
    pub fn toggle(&mut self) {
        *self = match self {
            Self::Utc => Self::Local,
            Self::Local => Self::Utc,
        };
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Utc => "UTC",
            Self::Local => "Local (system timezone)",
        }
    }

    pub fn format(self, timestamp: u64) -> String {
        let Some(time) = i64::try_from(timestamp)
            .ok()
            .and_then(|t| DateTime::<Utc>::from_timestamp(t, 0))
        else {
            return "invalid timestamp".into();
        };
        match self {
            Self::Utc => time.to_rfc3339_opts(SecondsFormat::Secs, true),
            Self::Local => time
                .with_timezone(&Local)
                .to_rfc3339_opts(SecondsFormat::Secs, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_default_preserves_existing_format_and_local_preserves_the_instant() {
        assert_eq!(TimeDisplay::default().format(0), "1970-01-01T00:00:00Z");
        for timestamp in [0, 1_735_689_600, 1_751_328_000] {
            let local = TimeDisplay::Local.format(timestamp);
            assert_eq!(
                DateTime::parse_from_rfc3339(&local).unwrap().timestamp(),
                timestamp as i64
            );
        }
        assert_eq!(TimeDisplay::Local.format(u64::MAX), "invalid timestamp");
    }

    // Run in a separate process with TZ=America/Los_Angeles; don't mutate the
    // environment in a multithreaded test process.
    #[test]
    fn local_obeys_historical_daylight_saving_rules() {
        if std::env::var("TZ").as_deref() == Ok("America/Los_Angeles") {
            assert!(TimeDisplay::Local.format(1_735_689_600).ends_with("-08:00"));
            assert!(TimeDisplay::Local.format(1_751_328_000).ends_with("-07:00"));
        }
    }
}
