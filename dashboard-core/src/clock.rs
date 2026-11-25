use jiff::Timestamp;
use std::fmt;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalClock {
    pub year: i16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

#[derive(Debug)]
pub enum ClockError {
    UnknownTimeZone(String),
    InvalidTimestamp(String),
}

impl fmt::Display for ClockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTimeZone(zone) => {
                write!(f, "'{zone}' is not a recognized IANA time zone")
            }
            Self::InvalidTimestamp(message) => write!(f, "system time is invalid: {message}"),
        }
    }
}

impl std::error::Error for ClockError {}

/// Computes the local wall-clock time for `time_zone` at `now_utc`.
/// Derived purely from the supplied UTC instant and the bundled IANA tz
/// database (no network or remote clock involved), per the spec's "compute
/// clock locally, never from a remote clock" requirement.
pub fn local_clock(time_zone: &str, now_utc: OffsetDateTime) -> Result<LocalClock, ClockError> {
    let nanosecond = i32::try_from(now_utc.nanosecond())
        .map_err(|error| ClockError::InvalidTimestamp(error.to_string()))?;
    let timestamp = Timestamp::new(now_utc.unix_timestamp(), nanosecond)
        .map_err(|error| ClockError::InvalidTimestamp(error.to_string()))?;
    let zoned = timestamp
        .in_tz(time_zone)
        .map_err(|_| ClockError::UnknownTimeZone(time_zone.to_string()))?;
    let civil = zoned.datetime();

    Ok(LocalClock {
        year: civil.year(),
        month: civil.month() as u8,
        day: civil.day() as u8,
        hour: civil.hour() as u8,
        minute: civil.minute() as u8,
        second: civil.second() as u8,
    })
}
