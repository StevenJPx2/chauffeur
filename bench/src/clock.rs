//! UTC timestamps without a time crate.

use std::time::{SystemTime, UNIX_EPOCH};

/// A UTC civil time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Utc {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u64,
    pub minute: u64,
    pub second: u64,
}

impl Utc {
    /// The current time.
    #[must_use]
    pub fn now() -> Self {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());

        Self::from_unix(seconds)
    }

    /// Convert seconds since the Unix epoch (Howard Hinnant's
    /// `civil_from_days`).
    #[must_use]
    pub fn from_unix(seconds: u64) -> Self {
        let days = i64::try_from(seconds / 86_400).unwrap_or(0);
        let rest = seconds % 86_400;
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);

        Self {
            year,
            month: u32::try_from(month).unwrap_or(1),
            day: u32::try_from(day).unwrap_or(1),
            hour: rest / 3_600,
            minute: rest % 3_600 / 60,
            second: rest % 60,
        }
    }

    /// `YYYY-MM-DDTHH:MM:SSZ`.
    #[must_use]
    pub fn iso(&self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// `YYYYMMDDTHHMMSSZ`, safe as a directory name.
    #[must_use]
    pub fn compact(&self) -> String {
        format!(
            "{:04}{:02}{:02}T{:02}{:02}{:02}Z",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_known_instants() {
        assert_eq!(Utc::from_unix(0).iso(), "1970-01-01T00:00:00Z");
        assert_eq!(Utc::from_unix(951_782_400).iso(), "2000-02-29T00:00:00Z");
        assert_eq!(Utc::from_unix(1_790_590_245).compact(), "20260928T101045Z");
    }
}
