use dashboard_core::clock::{ClockError, LocalClock, local_clock};
use time::macros::datetime;

#[test]
fn computes_local_time_for_a_configured_time_zone() {
    let now_utc = datetime!(2026-11-24 20:30:15 UTC);
    let clock = local_clock("Europe/Stockholm", now_utc).expect("time zone must resolve");

    assert_eq!(
        clock,
        LocalClock {
            year: 2026,
            month: 11,
            day: 24,
            hour: 21,
            minute: 30,
            second: 15,
        }
    );
}

#[test]
fn applies_daylight_saving_offset_in_summer() {
    let now_utc = datetime!(2026-07-01 12:00:00 UTC);
    let clock = local_clock("Europe/Stockholm", now_utc).expect("time zone must resolve");

    assert_eq!(clock.hour, 14);
}

#[test]
fn advances_through_an_hour_boundary() {
    let just_before =
        local_clock("UTC", datetime!(2026-11-24 20:59:59 UTC)).expect("UTC must resolve");
    let just_after =
        local_clock("UTC", datetime!(2026-11-24 21:00:00 UTC)).expect("UTC must resolve");

    assert_eq!(just_before.hour, 20);
    assert_eq!(just_before.minute, 59);
    assert_eq!(just_after.hour, 21);
    assert_eq!(just_after.minute, 0);
}

#[test]
fn rejects_an_unrecognized_time_zone_without_panicking() {
    let error = local_clock("Not/AZone", datetime!(2026-11-24 20:00:00 UTC)).unwrap_err();
    assert!(matches!(error, ClockError::UnknownTimeZone(_)));
}
