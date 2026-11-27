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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayState {
    Normal,
    ScheduledStandby,
    ManualOverride(OverridePower),
    Screensaver,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverridePower {
    On,
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverrideState {
    pub power: OverridePower,
    pub expires_at: OffsetDateTime,
}

pub fn active_override_power(
    override_state: Option<OverrideState>,
    now_utc: OffsetDateTime,
) -> Option<OverridePower> {
    override_state
        .filter(|state| now_utc < state.expires_at)
        .map(|state| state.power)
}

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

pub trait DisplayPowerControl {
    fn set_power(&mut self, on: bool) -> Result<(), DisplayPowerError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct LoggingDisplayPowerControl;

impl LoggingDisplayPowerControl {
    pub fn new() -> Self {
        Self
    }
}

impl DisplayPowerControl for LoggingDisplayPowerControl {
    fn set_power(&mut self, on: bool) -> Result<(), DisplayPowerError> {
        eprintln!("display power (logging backend): set_power({on})");
        Ok(())
    }
}

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
