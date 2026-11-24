use jiff::Timestamp;
use jiff::civil::{Date, DateTime};
use solar_positioning::{Horizon, Location, SolarEvents};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaylightWindow {
    pub date: Date,
    pub civil_dawn: DateTime,
    pub sunrise: DateTime,
    pub sunset: DateTime,
    pub civil_dusk: DateTime,
}

impl DaylightWindow {
    /// Always derived from sunset - sunrise (never stored or recomputed
    /// separately), so this structurally satisfies spec.yaml's
    /// `daylight_duration_equals_sunset_minus_sunrise` invariant.
    pub fn daylight_duration(&self) -> Result<jiff::Span, DaylightError> {
        self.sunset
            .since(self.sunrise)
            .map_err(|error| DaylightError::Computation(error.to_string()))
    }
}

#[derive(Debug)]
pub enum DaylightError {
    UnknownTimeZone(String),
    InvalidTimestamp(String),
    Computation(String),
    NoEvent(&'static str),
}

impl fmt::Display for DaylightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTimeZone(zone) => {
                write!(f, "'{zone}' is not a recognized IANA time zone")
            }
            Self::InvalidTimestamp(message) => {
                write!(f, "computed timestamp is invalid: {message}")
            }
            Self::Computation(message) => write!(f, "solar calculation failed: {message}"),
            Self::NoEvent(label) => write!(f, "no {label} crossing found for this date/location"),
        }
    }
}

impl std::error::Error for DaylightError {}

/// Unix-epoch Julian Date conversion (JD 2440587.5 = 1970-01-01T00:00:00Z),
/// the standard astronomical epoch used throughout `solar_positioning`.
fn timestamp_to_julian_day(timestamp: Timestamp) -> f64 {
    2_440_587.5
        + timestamp.as_second() as f64 / 86_400.0
        + f64::from(timestamp.subsec_nanosecond()) / 86_400e9
}

fn julian_day_to_timestamp(julian_day: f64) -> Result<Timestamp, DaylightError> {
    let total_seconds = (julian_day - 2_440_587.5) * 86_400.0;
    let whole_seconds = total_seconds.floor();
    let nanosecond = ((total_seconds - whole_seconds) * 1e9).round() as i32;
    Timestamp::new(whole_seconds as i64, nanosecond)
        .map_err(|error| DaylightError::InvalidTimestamp(error.to_string()))
}

fn local_datetime_for_event(julian_day: f64, time_zone: &str) -> Result<DateTime, DaylightError> {
    let timestamp = julian_day_to_timestamp(julian_day)?;
    let zoned = timestamp
        .in_tz(time_zone)
        .map_err(|_| DaylightError::UnknownTimeZone(time_zone.to_string()))?;
    Ok(zoned.datetime())
}

/// Picks the rise/set pair that brackets local solar noon for `date`, rather
/// than naively taking each list's first element independently.
///
/// `for_interval_from_julian` queries a local-midnight-to-local-midnight
/// window, so at high latitudes / far west of a time-zone meridian its
/// `rises`/`sets` lists can contain an event that is really the tail end of
/// the *previous* local day (e.g. a sunset at 00:01 local that belongs to
/// yesterday evening, not today). Pairing `rises.first()` with
/// `sets.first()` directly can then mispair a leaked near-midnight event
/// with the correct event on the other side of noon, producing a
/// nonsensical (even negative) span. Choosing the rise closest to (and no
/// later than) noon, and the set closest to (and no earlier than) noon,
/// correctly discards such leaked boundary events on either side.
fn pick_bracketing_pair(
    rises: &[f64],
    sets: &[f64],
    noon_jd: f64,
    rise_label: &'static str,
    set_label: &'static str,
) -> Result<(f64, f64), DaylightError> {
    let rise_jd = rises
        .iter()
        .copied()
        .filter(|&rise| rise <= noon_jd)
        .max_by(f64::total_cmp)
        .or_else(|| rises.iter().copied().min_by(f64::total_cmp))
        .ok_or(DaylightError::NoEvent(rise_label))?;

    let set_jd = sets
        .iter()
        .copied()
        .filter(|&set| set > noon_jd)
        .min_by(f64::total_cmp)
        .or_else(|| {
            sets.iter()
                .copied()
                .filter(|&set| set > rise_jd)
                .min_by(f64::total_cmp)
        })
        .ok_or(DaylightError::NoEvent(set_label))?;

    Ok((rise_jd, set_jd))
}

/// Computes civil dawn, sunrise, sunset and civil dusk for the local calendar
/// `date` at `(latitude, longitude)` in `time_zone`, using NREL's Solar
/// Position Algorithm (Reda & Andreas, 2003, as implemented by the
/// `solar-positioning` crate). Civil twilight is the sun 6 degrees below the
/// horizon. Pure function of its inputs: call again with a new local date to
/// recompute for the next day.
pub fn daylight_window(
    time_zone: &str,
    latitude: f64,
    longitude: f64,
    date: Date,
) -> Result<DaylightWindow, DaylightError> {
    let previous_date = date
        .yesterday()
        .map_err(|error| DaylightError::InvalidTimestamp(error.to_string()))?;
    let day_after_next = date
        .tomorrow()
        .and_then(|next| next.tomorrow())
        .map_err(|error| DaylightError::InvalidTimestamp(error.to_string()))?;

    // Query a 3-day window (yesterday through the day after tomorrow)
    // rather than just [date, date+1): at high latitudes / far west of a
    // time-zone meridian, the sunset that genuinely belongs to `date`'s
    // daylight cycle can fall just after local midnight, i.e. technically
    // on `date`'s calendar successor. Searching only one calendar day would
    // miss that event (or mispair with a leaked event from the day before);
    // `pick_bracketing_pair` below then picks the correct rise/set pair
    // around local solar noon from this wider pool.
    let query_start = previous_date
        .in_tz(time_zone)
        .map_err(|_| DaylightError::UnknownTimeZone(time_zone.to_string()))?
        .timestamp();
    let query_end = day_after_next
        .in_tz(time_zone)
        .map_err(|_| DaylightError::UnknownTimeZone(time_zone.to_string()))?
        .timestamp();

    let start_jd = timestamp_to_julian_day(query_start);
    let end_jd = timestamp_to_julian_day(query_end);

    let noon_timestamp = date
        .at(12, 0, 0, 0)
        .in_tz(time_zone)
        .map_err(|_| DaylightError::UnknownTimeZone(time_zone.to_string()))?
        .timestamp();
    let noon_jd = timestamp_to_julian_day(noon_timestamp);

    let delta_t = solar_positioning::delta_t::estimate(f64::from(date.year()))
        .map_err(|error| DaylightError::Computation(error.to_string()))?;
    let location = Location {
        latitude,
        longitude,
    };
    let events = SolarEvents::new();

    let sun_events = events
        .for_interval_from_julian(start_jd, end_jd, location, delta_t, Horizon::SunriseSunset)
        .map_err(|error| DaylightError::Computation(error.to_string()))?;
    let twilight_events = events
        .for_interval_from_julian(start_jd, end_jd, location, delta_t, Horizon::CivilTwilight)
        .map_err(|error| DaylightError::Computation(error.to_string()))?;

    let (sunrise_jd, sunset_jd) = pick_bracketing_pair(
        &sun_events.rises,
        &sun_events.sets,
        noon_jd,
        "sunrise",
        "sunset",
    )?;
    let (civil_dawn_jd, civil_dusk_jd) = pick_bracketing_pair(
        &twilight_events.rises,
        &twilight_events.sets,
        noon_jd,
        "civil dawn",
        "civil dusk",
    )?;

    Ok(DaylightWindow {
        date,
        civil_dawn: local_datetime_for_event(civil_dawn_jd, time_zone)?,
        sunrise: local_datetime_for_event(sunrise_jd, time_zone)?,
        sunset: local_datetime_for_event(sunset_jd, time_zone)?,
        civil_dusk: local_datetime_for_event(civil_dusk_jd, time_zone)?,
    })
}
