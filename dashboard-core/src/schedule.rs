use crate::board_config::{DayType, DisplayWindow};
use jiff::Timestamp;
use std::fmt;
use time::{OffsetDateTime, Time};

#[derive(Debug)]
pub enum ScheduleError {
    UnknownTimeZone(String),
    InvalidTimestamp(String),
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTimeZone(zone) => {
                write!(f, "'{zone}' is not a recognized IANA time zone")
            }
            Self::InvalidTimestamp(message) => write!(f, "system time is invalid: {message}"),
        }
    }
}

impl std::error::Error for ScheduleError {}

fn day_type_for(weekday: jiff::civil::Weekday) -> DayType {
    match weekday {
        jiff::civil::Weekday::Saturday | jiff::civil::Weekday::Sunday => DayType::Weekend,
        _ => DayType::Weekday,
    }
}

/// Evaluates whether `now_utc` falls inside one of the configured
/// `display_windows`, in `time_zone`'s local civil time. Derives the
/// weekday/weekend split and the local time-of-day via the bundled IANA tz
/// database for this exact instant (the same `jiff`-based pattern
/// `clock.rs`/`daylight.rs` use), so the result is correct across a DST
/// transition rather than relying on a fixed UTC offset. A window's `start`
/// is inclusive, `end` is exclusive (`board_config.rs` already rejects
/// `end <= start`, so no window wraps past midnight).
pub fn is_within_display_window(
    windows: &[DisplayWindow],
    time_zone: &str,
    now_utc: OffsetDateTime,
) -> Result<bool, ScheduleError> {
    let nanosecond = i32::try_from(now_utc.nanosecond())
        .map_err(|error| ScheduleError::InvalidTimestamp(error.to_string()))?;
    let timestamp = Timestamp::new(now_utc.unix_timestamp(), nanosecond)
        .map_err(|error| ScheduleError::InvalidTimestamp(error.to_string()))?;
    let zoned = timestamp
        .in_tz(time_zone)
        .map_err(|_| ScheduleError::UnknownTimeZone(time_zone.to_string()))?;
    let civil = zoned.datetime();

    let day_type = day_type_for(civil.weekday());
    let now_time = Time::from_hms(
        civil.hour() as u8,
        civil.minute() as u8,
        civil.second() as u8,
    )
    .map_err(|error| ScheduleError::InvalidTimestamp(error.to_string()))?;

    Ok(windows
        .iter()
        .filter(|window| window.day_type == day_type)
        .any(|window| now_time >= window.start && now_time < window.end))
}

/// The four states `spec.yaml`'s `remote-screen-control` capability names.
/// `ManualOverride` and `Screensaver` have no real trigger wired up yet
/// (PI-08 and PI-07 respectively) - `reconcile` already accepts them as
/// inputs so those slices plug in without reshaping this enum or the
/// priority logic below. `ManualOverride` carries the `OverridePower` that
/// produced it, so a resolved state and the override direction it implies
/// can never go out of sync between `reconcile` and `resolved_power`: there
/// is no `(ManualOverride, None)` combination to misresolve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayState {
    Normal,
    ScheduledStandby,
    ManualOverride(OverridePower),
    Screensaver,
}

/// Which way an active manual override wants the backlight to go. PI-08's
/// real override-file format isn't built yet; this is the minimal shape
/// `reconcile` needs today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverridePower {
    On,
    Off,
}

/// Decides the display state from already-resolved inputs for one instant.
/// Pure function, no hidden state: `schedule_says_awake` is the result of
/// `is_within_display_window`, `override_active` is a single already-read
/// snapshot of the (future) override file, and `screensaver_active` is the
/// (future) calendar-poll result. `req-override-concurrency`'s "the
/// reconciler always reads a consistent override state" is satisfied by
/// this signature's shape, not by anything `reconcile` does internally:
/// the caller performs one atomic read of the override source before
/// calling this function, and because `reconcile` is pure and takes that
/// read as a plain value rather than re-reading the source itself, there is
/// no window during this computation in which a concurrent override write
/// could be partially observed - the whole call sees either the old value
/// or the new one, never a mix.
///
/// Priority order: an active manual override always wins (it is, by
/// definition, a deliberate temporary override of whatever the schedule or
/// screensaver would otherwise decide - see `pi-dashboard-spec.md`'s PI-08
/// section). Otherwise, `spec.yaml`'s `schedule-display-power` invariant
/// `standby_overrides_screensaver` applies: the schedule saying "asleep"
/// wins over an active screensaver.
pub fn reconcile(
    schedule_says_awake: bool,
    override_active: Option<OverridePower>,
    screensaver_active: bool,
) -> DisplayState {
    if let Some(override_power) = override_active {
        return DisplayState::ManualOverride(override_power);
    }
    if !schedule_says_awake {
        return DisplayState::ScheduledStandby;
    }
    if screensaver_active {
        return DisplayState::Screensaver;
    }
    DisplayState::Normal
}

/// Maps a resolved `DisplayState` to the actual backlight power boolean to
/// hand to `DisplayPowerControl::set_power`. Kept separate from `reconcile`
/// so the *reported* state (what `status-report`'s `state` field shows)
/// and the *physical* power decision stay independently testable.
pub fn resolved_power(state: DisplayState) -> bool {
    match state {
        DisplayState::ScheduledStandby => false,
        DisplayState::ManualOverride(override_power) => override_power == OverridePower::On,
        DisplayState::Screensaver | DisplayState::Normal => true,
    }
}

#[derive(Debug)]
pub enum DisplayPowerError {
    BacklightControlUnavailable(String),
}

impl fmt::Display for DisplayPowerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BacklightControlUnavailable(message) => {
                write!(f, "backlight control is unavailable: {message}")
            }
        }
    }
}

impl std::error::Error for DisplayPowerError {}

/// Seam a future hardware-integration slice fills in with real
/// `wlr-randr` invocations (`req-resilience-backlight`, out of scope here -
/// it needs the physical device/`WAYLAND_DISPLAY`/`XDG_RUNTIME_DIR` from a
/// systemd-timer context to verify).
pub trait DisplayPowerControl {
    fn set_power(&mut self, on: bool) -> Result<(), DisplayPowerError>;
}

/// The only implementation that exists today: logs the decision instead of
/// touching any hardware. Unlike `FixtureWeatherSource`/
/// `FixtureDeparturesSource`, this is not gated behind the `fixtures`
/// feature: there is no real backlight implementation yet for it to stand
/// in for, so `dashboard-pi` uses it directly as its production
/// implementation until a hardware-integration slice replaces it. Tests use
/// it too.
#[derive(Debug, Default)]
pub struct NullDisplayPowerControl {
    calls: Vec<bool>,
}

impl NullDisplayPowerControl {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn calls(&self) -> &[bool] {
        &self.calls
    }
}

impl DisplayPowerControl for NullDisplayPowerControl {
    fn set_power(&mut self, on: bool) -> Result<(), DisplayPowerError> {
        self.calls.push(on);
        eprintln!("display power (null backend): set_power({on})");
        Ok(())
    }
}
