use dashboard_core::daylight::{DaylightError, daylight_window};
use jiff::civil::date;

// Stockholm (Duvbo-area) coordinates from board.toml's [location].
const STOCKHOLM_LAT: f64 = 59.3626;
const STOCKHOLM_LON: f64 = 17.9158;
const STOCKHOLM_TZ: &str = "Europe/Stockholm";

// Stockholm city centre (Sergels torg), matching timeanddate.com's
// "Stockholm, Sweden" location used as the reference-value source below.
const STOCKHOLM_CITY_LAT: f64 = 59.3294;
const STOCKHOLM_CITY_LON: f64 = 18.0687;

// Bethel, Alaska: high-latitude and far west of its time-zone meridian, so
// sunset can fall just after local midnight. Exercises the rise/set
// bracketing fix in `daylight_window` (a naive "first rise, first set"
// pairing would otherwise mispair a leaked previous-day sunset with today's
// sunrise, yielding a negative duration).
const BETHEL_LAT: f64 = 60.7922;
const BETHEL_LON: f64 = -161.7558;
const BETHEL_TZ: &str = "America/Anchorage";

fn hhmm(dt: jiff::civil::DateTime) -> (i8, i8) {
    (dt.hour(), dt.minute())
}

#[test]
fn daylight_duration_equals_sunset_minus_sunrise_for_a_fixed_date() {
    let window = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 6, 21),
    )
    .expect("solar calculation must succeed for a valid date/location");

    let expected = window
        .sunset
        .since(window.sunrise)
        .expect("sunset is after sunrise");
    let actual = window
        .daylight_duration()
        .expect("duration must be computable");

    assert_eq!(jiff::SpanFieldwise(actual), jiff::SpanFieldwise(expected));
}

#[test]
fn matches_published_reference_values_for_stockholm_on_2026_06_21() {
    // Reference: timeanddate.com, Stockholm, Sweden (59.3294 N, 18.0687 E),
    // 2026-06-21 (CEST, UTC+2): civil dawn 01:59, sunrise 03:30, sunset
    // 22:08, civil dusk 23:40. Computed here at that same city-centre
    // location (STOCKHOLM_CITY_LAT/LON) so the published values are checked
    // against matching coordinates, not board.toml's Duvbo coordinates
    // (~1-1.5 minutes further north/west).
    let window = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_CITY_LAT,
        STOCKHOLM_CITY_LON,
        date(2026, 6, 21),
    )
    .expect("solar calculation must succeed for a valid date/location");

    assert_within_tolerance(hhmm(window.civil_dawn), (1, 59));
    assert_within_tolerance(hhmm(window.sunrise), (3, 30));
    assert_within_tolerance(hhmm(window.sunset), (22, 8));
    assert_within_tolerance(hhmm(window.civil_dusk), (23, 40));
}

fn assert_within_tolerance(actual: (i8, i8), expected: (i8, i8)) {
    let actual_minutes = i32::from(actual.0) * 60 + i32::from(actual.1);
    let expected_minutes = i32::from(expected.0) * 60 + i32::from(expected.1);
    let delta = (actual_minutes - expected_minutes).abs();
    // timeanddate.com publishes to the minute, so 1 minute of rounding
    // slack is the tightest tolerance that doesn't flake; an independent
    // from-scratch NOAA/Meeus re-derivation agreed with this engine to
    // within ~5-10 seconds at this location/date.
    assert!(
        delta <= 1,
        "expected {expected:?}, got {actual:?} (delta {delta} minutes)"
    );
}

#[test]
fn all_four_daylight_times_differ_and_keep_their_own_date_on_consecutive_days() {
    let today = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 11, 24),
    )
    .expect("solar calculation must succeed for a valid date/location");
    let tomorrow = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 11, 25),
    )
    .expect("solar calculation must succeed for a valid date/location");

    assert_eq!(today.date, date(2026, 11, 24));
    assert_eq!(tomorrow.date, date(2026, 11, 25));

    assert_eq!(today.civil_dawn.date(), date(2026, 11, 24));
    assert_eq!(today.sunrise.date(), date(2026, 11, 24));
    assert_eq!(today.sunset.date(), date(2026, 11, 24));
    assert_eq!(today.civil_dusk.date(), date(2026, 11, 24));

    assert_eq!(tomorrow.civil_dawn.date(), date(2026, 11, 25));
    assert_eq!(tomorrow.sunrise.date(), date(2026, 11, 25));
    assert_eq!(tomorrow.sunset.date(), date(2026, 11, 25));
    assert_eq!(tomorrow.civil_dusk.date(), date(2026, 11, 25));

    assert_ne!(today.civil_dawn, tomorrow.civil_dawn);
    assert_ne!(today.sunrise, tomorrow.sunrise);
    assert_ne!(today.sunset, tomorrow.sunset);
    assert_ne!(today.civil_dusk, tomorrow.civil_dusk);
}

#[test]
fn near_midnight_sunset_still_attaches_to_the_correct_local_calendar_date() {
    // Bethel, Alaska, 2026-06-01: because Bethel sits far west of the
    // America/Anchorage meridian, the sunset that belongs to this day's
    // sunrise-to-sunset cycle genuinely falls just after local midnight —
    // i.e. its correct local date is 2026-06-02, the day *after* `date`,
    // not 2026-05-31 (the previous day's leftover sunset that the
    // unfixed pairing bug used to misattribute as "today's" sunset).
    let window = daylight_window(BETHEL_TZ, BETHEL_LAT, BETHEL_LON, date(2026, 6, 1))
        .expect("solar calculation must succeed for a valid date/location");

    assert_eq!(window.date, date(2026, 6, 1));
    assert_eq!(window.sunrise.date(), date(2026, 6, 1));
    assert_eq!(window.civil_dawn.date(), date(2026, 6, 1));

    assert!(window.sunset > window.sunrise);
    assert_ne!(window.sunset.date(), date(2026, 5, 31));
    assert_eq!(window.sunset.date(), date(2026, 6, 2));
    assert_ne!(window.civil_dusk.date(), date(2026, 5, 31));
}

#[test]
fn daylight_is_longer_at_the_summer_solstice_than_the_winter_solstice() {
    let summer = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 6, 21),
    )
    .expect("solar calculation must succeed for a valid date/location");
    let winter = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 12, 21),
    )
    .expect("solar calculation must succeed for a valid date/location");

    let summer_duration = summer.daylight_duration().expect("must compute");
    let winter_duration = winter.daylight_duration().expect("must compute");

    assert!(summer_duration.compare(winter_duration).unwrap().is_gt());
}

#[test]
fn computes_correctly_either_side_of_a_real_dst_transition() {
    // 2026-03-29: Europe/Stockholm spring-forward (23-hour UTC day).
    let spring_forward = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 3, 29),
    )
    .expect("solar calculation must succeed across a spring-forward transition");
    // 2026-10-25: Europe/Stockholm fall-back (25-hour UTC day).
    let fall_back = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 10, 25),
    )
    .expect("solar calculation must succeed across a fall-back transition");

    assert!(
        spring_forward
            .daylight_duration()
            .expect("must compute")
            .get_hours()
            >= 0
    );
    assert!(
        fall_back
            .daylight_duration()
            .expect("must compute")
            .get_hours()
            >= 0
    );
    assert_eq!(spring_forward.date, date(2026, 3, 29));
    assert_eq!(fall_back.date, date(2026, 10, 25));
}

#[test]
fn rejects_an_unrecognized_time_zone_without_panicking() {
    let error =
        daylight_window("Not/AZone", STOCKHOLM_LAT, STOCKHOLM_LON, date(2026, 6, 21)).unwrap_err();
    assert!(matches!(error, DaylightError::UnknownTimeZone(_)));
}

#[test]
fn rejects_out_of_range_latitude_without_panicking() {
    let error = daylight_window(STOCKHOLM_TZ, 95.0, STOCKHOLM_LON, date(2026, 6, 21)).unwrap_err();
    assert!(matches!(error, DaylightError::Computation(_)));
}

#[test]
fn daylight_duration_is_never_negative() {
    let window = daylight_window(
        STOCKHOLM_TZ,
        STOCKHOLM_LAT,
        STOCKHOLM_LON,
        date(2026, 6, 21),
    )
    .expect("solar calculation must succeed for a valid date/location");
    let duration = window.daylight_duration().expect("must compute");

    assert!(duration.get_hours() >= 0);
}

#[test]
fn daylight_duration_is_never_negative_at_a_far_west_of_meridian_high_latitude_site() {
    // Reproduces the reviewer's Bethel, Alaska case: before the rise/set
    // bracketing fix, sunset computed as 00:01 local (a leaked previous-day
    // event) paired with that day's sunrise, giving a negative duration.
    let window = daylight_window(BETHEL_TZ, BETHEL_LAT, BETHEL_LON, date(2026, 6, 1))
        .expect("solar calculation must succeed for a valid date/location");
    let duration = window.daylight_duration().expect("must compute");

    assert!(window.sunset > window.sunrise);
    assert!(duration.get_hours() >= 0);
}
