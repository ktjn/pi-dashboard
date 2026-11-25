use crate::board_config::{DayType, DisplayWindow};
use jiff::Timestamp;
use std::fmt;
use time::{OffsetDateTime, Time};

#[derive(Debug)]
pub enum ScheduleError {
    UnknownTimeZone(String),
    InvalidTimestamp(String),
    NoUpcomingBoundary,
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTimeZone(zone) => {
                write!(f, "'{zone}' is not a recognized IANA time zone")
            }
            Self::InvalidTimestamp(message) => write!(f, "system time is invalid: {message}"),
            Self::NoUpcomingBoundary => {
                write!(f, "no display-window boundary follows the given instant")
            }
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

/// Days ahead to search for the next window boundary. A single configured
/// window always recurs within 7 days (the weekday/weekend split repeats
/// weekly), so this is enough to always find a boundary when at least one
/// window is configured, while staying a small, bounded scan rather than an
/// unbounded search.
const BOUNDARY_SEARCH_HORIZON_DAYS: i32 = 8;

/// Computes the next instant, strictly after `now_utc`, at which `now_utc`
/// would cross into or out of one of the configured `display_windows` (a
/// window's `start` or `end`), evaluated in `time_zone`'s local civil time -
/// the same `jiff`-based IANA-zone resolution `is_within_display_window`
/// uses, so this stays DST-correct across a transition rather than assuming
/// a fixed UTC offset. `remote-screen-control`'s
/// `override_expires_at_next_schedule_boundary` invariant uses this to give
/// a manual override a real, bounded lifetime: it must clear itself at the
/// next boundary, not persist forever.
pub fn next_window_boundary(
    windows: &[DisplayWindow],
    time_zone: &str,
    now_utc: OffsetDateTime,
) -> Result<OffsetDateTime, ScheduleError> {
    let nanosecond = i32::try_from(now_utc.nanosecond())
        .map_err(|error| ScheduleError::InvalidTimestamp(error.to_string()))?;
    let timestamp = Timestamp::new(now_utc.unix_timestamp(), nanosecond)
        .map_err(|error| ScheduleError::InvalidTimestamp(error.to_string()))?;
    let zoned_now = timestamp
        .in_tz(time_zone)
        .map_err(|_| ScheduleError::UnknownTimeZone(time_zone.to_string()))?;
    let today = zoned_now.date();

    let mut earliest: Option<OffsetDateTime> = None;
    for day_offset in 0..BOUNDARY_SEARCH_HORIZON_DAYS {
        let date = today
            .checked_add(jiff::Span::new().days(day_offset))
            .map_err(|error| ScheduleError::InvalidTimestamp(error.to_string()))?;
        let day_type = day_type_for(date.weekday());

        for window in windows.iter().filter(|window| window.day_type == day_type) {
            for boundary_time in [window.start, window.end] {
                let civil_time = jiff::civil::Time::new(
                    boundary_time.hour() as i8,
                    boundary_time.minute() as i8,
                    boundary_time.second() as i8,
                    0,
                )
                .map_err(|error| ScheduleError::InvalidTimestamp(error.to_string()))?;
                let civil_datetime = jiff::civil::DateTime::from_parts(date, civil_time);
                let zoned_boundary = civil_datetime
                    .in_tz(time_zone)
                    .map_err(|_| ScheduleError::UnknownTimeZone(time_zone.to_string()))?;
                let boundary_instant = zoned_boundary.timestamp();
                let boundary_utc =
                    OffsetDateTime::from_unix_timestamp(boundary_instant.as_second())
                        .map_err(|error| ScheduleError::InvalidTimestamp(error.to_string()))?;

                if boundary_utc > now_utc && earliest.is_none_or(|current| boundary_utc < current) {
                    earliest = Some(boundary_utc);
                }
            }
        }
    }

    earliest.ok_or(ScheduleError::NoUpcomingBoundary)
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

/// An override read from the override file: the requested power direction
/// plus the instant (computed by `next_window_boundary` at write time) at
/// which it stops applying. Kept as plain data with no I/O of its own -
/// `dashboard-pi` reads/writes the actual file; this type and
/// `active_override_power` only decide whether an already-read value still
/// counts as active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverrideState {
    pub power: OverridePower,
    pub expires_at: OffsetDateTime,
}

/// Resolves an optionally-present `OverrideState` into the `Option<OverridePower>`
/// `reconcile` expects, treating an override whose `expires_at` has already
/// passed as if it were absent. This is where
/// `override_expires_at_next_schedule_boundary` actually takes effect: the
/// caller does this check once, from a single already-read snapshot of the
/// override file, before calling `reconcile` - so `reconcile` itself never
/// needs to know about expiry.
pub fn active_override_power(
    override_state: Option<OverrideState>,
    now_utc: OffsetDateTime,
) -> Option<OverridePower> {
    override_state
        .filter(|state| now_utc < state.expires_at)
        .map(|state| state.power)
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
