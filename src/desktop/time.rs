use std::time::{SystemTime, UNIX_EPOCH};
pub(crate) fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// UTC civil time without a timezone service or platform locale dependency.
pub(crate) fn utc_timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = month_index + if month_index < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    let hour = seconds / 3_600 % 24;
    let minute = seconds / 60 % 60;
    let second = seconds % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn utc_dates_handle_epoch_leap_days_and_time_of_day() {
        assert_eq!(
            crate::desktop::time::utc_timestamp(0),
            "1970-01-01 00:00:00"
        );
        assert_eq!(
            crate::desktop::time::utc_timestamp(951_782_400),
            "2000-02-29 00:00:00"
        );
        assert_eq!(
            crate::desktop::time::utc_timestamp(1_709_251_261),
            "2024-03-01 00:01:01"
        );
    }
}
