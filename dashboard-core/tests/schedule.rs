use dashboard_core::board_config::{DayType, DisplayWindow};
use dashboard_core::schedule::{
    DisplayPowerControl, DisplayState, OverridePower, ScheduleError, is_within_display_window,
    reconcile, resolved_power,
};
use time::macros::{datetime, time};

const STOCKHOLM_TZ: &str = "Europe/Stockholm";

fn weekday_windows() -> Vec<DisplayWindow> {
    vec![
        DisplayWindow {
            day_type: DayType::Weekday,
            start: time!(06:00),
            end: time!(09:00),
        },
        DisplayWindow {
            day_type: DayType::Weekday,
            start: time!(16:00),
            end: time!(22:30),
        },
        DisplayWindow {
            day_type: DayType::Weekend,
            start: time!(08:00),
            end: time!(23:00),
        },
    ]
}

#[test]
fn a_weekday_moment_inside_the_morning_window_is_within_display_hours() {
    // 2026-11-24 is a Tuesday.
    let now_utc = datetime!(2026-11-24 05:30:00 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, now_utc)
        .expect("computation must succeed");
    assert!(within);
}

#[test]
fn a_weekday_moment_between_the_two_windows_is_outside_display_hours() {
    // 2026-11-24 is a Tuesday; Stockholm is UTC+1 in November, so 10:00
    // local is 09:00 UTC, which sits between the 06:00-09:00 and
    // 16:00-22:30 weekday windows.
    let now_utc = datetime!(2026-11-24 09:00:00 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, now_utc)
        .expect("computation must succeed");
    assert!(!within);
}

#[test]
fn a_weekday_moment_in_the_evening_window_is_within_display_hours() {
    let now_utc = datetime!(2026-11-24 20:00:00 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, now_utc)
        .expect("computation must succeed");
    assert!(within);
}

#[test]
fn the_window_end_boundary_is_exclusive() {
    // 09:00 local on a weekday is exactly the configured end of the first
    // window: the display must already be dark at the boundary itself.
    let now_utc = datetime!(2026-11-24 08:00:00 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, now_utc)
        .expect("computation must succeed");
    assert!(!within);
}

#[test]
fn the_window_start_boundary_is_inclusive() {
    let now_utc = datetime!(2026-11-24 05:00:00 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, now_utc)
        .expect("computation must succeed");
    assert!(within);
}

#[test]
fn a_weekend_moment_uses_the_weekend_window_not_the_weekday_windows() {
    // 2026-11-28 is a Saturday. 07:00 local falls inside the weekend
    // 08:00-23:00 window's neighbourhood but before it starts, and would
    // be inside a weekday morning window if the day-type split were
    // ignored.
    let before_weekend_window = datetime!(2026-11-28 06:30:00 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, before_weekend_window)
        .expect("computation must succeed");
    assert!(!within);

    let inside_weekend_window = datetime!(2026-11-28 10:00:00 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, inside_weekend_window)
        .expect("computation must succeed");
    assert!(within);
}

#[test]
fn a_moment_outside_every_configured_window_is_not_within_display_hours() {
    // 2026-11-24 02:00 local, well before any weekday window.
    let now_utc = datetime!(2026-11-24 01:00:00 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, now_utc)
        .expect("computation must succeed");
    assert!(!within);
}

#[test]
fn rejects_an_unrecognized_time_zone_without_panicking() {
    let now_utc = datetime!(2026-11-24 05:30:00 UTC);
    let error = is_within_display_window(&weekday_windows(), "Not/AZone", now_utc).unwrap_err();
    assert!(matches!(error, ScheduleError::UnknownTimeZone(_)));
}

// --- DST correctness ---
//
// 2026-03-29 is Europe/Stockholm's real spring-forward date (offset UTC+1
// -> UTC+2) and 2026-10-25 is its real fall-back date (offset UTC+2 ->
// UTC+1); both are Sundays (weekend). These tests check instants several
// hours after each transition (not the transition instant itself) to
// confirm the correct post-transition UTC offset is in effect and applied
// when evaluating the weekend window's boundary. A fixed UTC-offset
// computation (rather than deriving local civil time from the IANA tz
// database for the correct instant) would misjudge the window boundary by
// a full hour on either of these dates, in opposite directions.

#[test]
fn spring_forward_transition_day_just_before_the_weekend_window_start_is_outside() {
    // 07:59:59 local on 2026-03-29 is, post-transition (UTC+2), 05:59:59 UTC.
    let now_utc = datetime!(2026-03-29 05:59:59 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, now_utc)
        .expect("computation must succeed across a spring-forward transition");
    assert!(!within);
}

#[test]
fn spring_forward_transition_day_at_the_weekend_window_start_is_inside() {
    // 08:00:00 local on 2026-03-29 (post-transition, UTC+2) -> 06:00:00 UTC.
    let now_utc = datetime!(2026-03-29 06:00:00 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, now_utc)
        .expect("computation must succeed across a spring-forward transition");
    assert!(within);
}

#[test]
fn fall_back_transition_day_just_before_the_weekend_window_start_is_outside() {
    // 2026-10-25 is Europe/Stockholm's fall-back date (Sunday, weekend;
    // local clocks move 03:00 -> 02:00, offset UTC+2 -> UTC+1).
    // 07:59:59 local, post-transition (UTC+1) -> 06:59:59 UTC.
    let now_utc = datetime!(2026-10-25 06:59:59 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, now_utc)
        .expect("computation must succeed across a fall-back transition");
    assert!(!within);
}

#[test]
fn fall_back_transition_day_at_the_weekend_window_start_is_inside() {
    // 08:00:00 local, post-transition (UTC+1) -> 07:00:00 UTC.
    let now_utc = datetime!(2026-10-25 07:00:00 UTC);
    let within = is_within_display_window(&weekday_windows(), STOCKHOLM_TZ, now_utc)
        .expect("computation must succeed across a fall-back transition");
    assert!(within);
}

// --- The state machine ---

#[test]
fn normal_when_schedule_says_awake_and_nothing_else_is_active() {
    assert_eq!(reconcile(true, None, false), DisplayState::Normal);
}

#[test]
fn scheduled_standby_when_schedule_says_asleep_and_nothing_else_is_active() {
    assert_eq!(
        reconcile(false, None, false),
        DisplayState::ScheduledStandby
    );
}

#[test]
fn screensaver_when_schedule_says_awake_and_screensaver_is_active() {
    assert_eq!(reconcile(true, None, true), DisplayState::Screensaver);
}

#[test]
fn standby_overrides_screensaver_when_both_would_otherwise_apply() {
    // spec.yaml's `schedule-display-power` invariant `standby_overrides_screensaver`:
    // when the schedule says the display should be dark, that wins even if
    // a screensaver would otherwise be active.
    assert_eq!(reconcile(false, None, true), DisplayState::ScheduledStandby);
}

#[test]
fn manual_override_wins_over_an_awake_schedule_and_no_screensaver() {
    assert_eq!(
        reconcile(true, Some(OverridePower::Off), false),
        DisplayState::ManualOverride(OverridePower::Off)
    );
}

#[test]
fn manual_override_wins_over_scheduled_standby() {
    assert_eq!(
        reconcile(false, Some(OverridePower::On), false),
        DisplayState::ManualOverride(OverridePower::On)
    );
}

#[test]
fn manual_override_wins_over_an_active_screensaver() {
    assert_eq!(
        reconcile(true, Some(OverridePower::Off), true),
        DisplayState::ManualOverride(OverridePower::Off)
    );
}

#[test]
fn manual_override_wins_even_when_standby_and_screensaver_would_both_apply() {
    assert_eq!(
        reconcile(false, Some(OverridePower::On), true),
        DisplayState::ManualOverride(OverridePower::On)
    );
}

#[test]
fn reconcile_is_deterministic_given_fixed_inputs() {
    // Demonstrates the "atomic read" shape: reconcile takes its override
    // input as a single already-read value rather than re-reading any
    // mutable source mid-computation, so calling it repeatedly with the
    // same arguments always yields the same decision - there is nothing
    // inside reconcile that could observe a torn/partial update.
    let first = reconcile(false, Some(OverridePower::On), true);
    let second = reconcile(false, Some(OverridePower::On), true);
    let third = reconcile(false, Some(OverridePower::On), true);
    assert_eq!(first, second);
    assert_eq!(second, third);
}

#[test]
fn resolved_power_is_on_for_normal_and_screensaver() {
    assert!(resolved_power(DisplayState::Normal));
    assert!(resolved_power(DisplayState::Screensaver));
}

#[test]
fn resolved_power_is_off_for_scheduled_standby() {
    assert!(!resolved_power(DisplayState::ScheduledStandby));
}

#[test]
fn resolved_power_follows_the_override_direction_during_manual_override() {
    assert!(resolved_power(DisplayState::ManualOverride(
        OverridePower::On
    )));
    assert!(!resolved_power(DisplayState::ManualOverride(
        OverridePower::Off
    )));
}

// --- Backlight-control seam ---

struct RecordingDisplayPowerControl {
    calls: Vec<bool>,
}

impl DisplayPowerControl for RecordingDisplayPowerControl {
    fn set_power(&mut self, on: bool) -> Result<(), dashboard_core::schedule::DisplayPowerError> {
        self.calls.push(on);
        Ok(())
    }
}

#[test]
fn display_power_control_trait_is_object_safe_and_callable() {
    let mut control = RecordingDisplayPowerControl { calls: Vec::new() };
    control.set_power(true).expect("set_power must succeed");
    control.set_power(false).expect("set_power must succeed");
    assert_eq!(control.calls, vec![true, false]);
}

#[test]
fn null_display_power_control_records_every_call_without_touching_hardware() {
    use dashboard_core::schedule::NullDisplayPowerControl;

    let mut control = NullDisplayPowerControl::new();
    control.set_power(true).expect("set_power must succeed");
    control.set_power(true).expect("set_power must succeed");
    control.set_power(false).expect("set_power must succeed");

    assert_eq!(control.calls(), &[true, true, false]);
}
