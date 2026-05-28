use super::*;
use chrono::TimeZone;

#[test]
fn quiet_hours_delay_handles_overnight_window() {
    let quiet = QuietHoursUtc {
        start_hour: 22,
        end_hour: 7,
    };
    let now = chrono::Utc
        .with_ymd_and_hms(2026, 5, 27, 23, 30, 0)
        .unwrap();

    assert_eq!(quiet.delay_until_end(now), Some(7 * 3600 + 30 * 60));
}

#[test]
fn quiet_hours_delay_is_none_outside_window() {
    let quiet = QuietHoursUtc {
        start_hour: 22,
        end_hour: 7,
    };
    let now = chrono::Utc.with_ymd_and_hms(2026, 5, 27, 12, 0, 0).unwrap();

    assert_eq!(quiet.delay_until_end(now), None);
}
