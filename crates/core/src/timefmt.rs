/// Render a Unix second count (since the epoch, read as UTC) into
/// the shape `YYYY-MM-DD HH:MM`, using a 24-hour clock.
pub fn short_unix_timestamp(secs: u64) -> String {
    const SECONDS_PER_DAY: u64 = 86_400;
    let days = secs / SECONDS_PER_DAY;
    let secs_in_day = secs % SECONDS_PER_DAY;
    let (year, month, day) = civil_from_days(days);
    let hour = secs_in_day / 3600;
    let minute = (secs_in_day % 3600) / 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

/// Convert a day count from the Unix epoch into (year, month, day) via Howard Hinnant's
/// `civil_from_days` algorithm, integer arithmetic only.
fn civil_from_days(z: u64) -> (u64, u64, u64) {
    let z = z + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097; // day of era, [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year, [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (year + u64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::short_unix_timestamp;

    #[test]
    fn epoch() {
        assert_eq!(short_unix_timestamp(0), "1970-01-01 00:00");
    }

    #[test]
    fn ordinary_time() {
        assert_eq!(short_unix_timestamp(1_757_170_800), "2025-09-06 15:00");
    }

    #[test]
    fn leap_day_2000() {
        assert_eq!(short_unix_timestamp(951_782_400), "2000-02-29 00:00");
    }

    #[test]
    fn leap_day_2024() {
        assert_eq!(short_unix_timestamp(1_709_164_800), "2024-02-29 00:00");
    }
}
