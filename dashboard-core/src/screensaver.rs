use crate::board_config::{MatchType, ScreensaverRule};
use regex::Regex;
use serde::{Deserialize, Serialize};
#[cfg(feature = "fixtures")]
use std::collections::HashMap;
use std::fmt;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

fn resolve_offsetless_date_time(
    date_time: &str,
    time_zone: Option<&str>,
) -> Result<OffsetDateTime, CalendarFetchError> {
    let time_zone = time_zone.ok_or_else(|| {
        CalendarFetchError::MissingData(format!(
            "'{date_time}' has no UTC offset and no sibling 'timeZone' to resolve it against"
        ))
    })?;
    let civil: jiff::civil::DateTime = date_time.parse().map_err(|error| {
        CalendarFetchError::MissingData(format!(
            "'{date_time}' is not a valid local datetime: {error}"
        ))
    })?;
    let zoned = civil.in_tz(time_zone).map_err(|_| {
        CalendarFetchError::MissingData(format!("'{time_zone}' is not a recognized IANA time zone"))
    })?;
    let timestamp = zoned.timestamp();
    OffsetDateTime::from_unix_timestamp(timestamp.as_second()).map_err(|error| {
        CalendarFetchError::MissingData(format!(
            "'{date_time}' resolved to an invalid instant: {error}"
        ))
    })
}

/// A poll stays "fresh" for roughly five poll intervals (the spec's ~1
/// minute cadence) before the "calendar unavailable" indicator shows -
/// long enough to absorb a couple of transient failures without flashing
/// the indicator on every missed tick, short enough that a real outage is
/// visible well within the spec's "catch event start/end within about a
/// minute" window.
pub const CALENDAR_STALE_AFTER: Duration = Duration::minutes(5);

/// `calendar-event-cache`'s fields (`spec.yaml`). Classification is
/// `confidential` - callers must not log `title` wholesale (see
/// `dashboard-pi`'s poll loop, which logs only the resulting activation
/// boolean and, on failure, the fetch error's `Display` output - never
/// event content).
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarEvent {
    pub event_id: String,
    pub title: String,
    pub start: OffsetDateTime,
    pub end: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CalendarFetchError {
    InvalidJson(String),
    MissingData(String),
}

impl fmt::Display for CalendarFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(message) => {
                write!(f, "calendar response is not valid JSON: {message}")
            }
            Self::MissingData(message) => {
                write!(f, "calendar response is missing usable data: {message}")
            }
        }
    }
}

impl std::error::Error for CalendarFetchError {}

/// One trait for the real Google Calendar client and the fixture replay,
/// mirroring `WeatherSource`/`StandingsSource`.
pub trait CalendarSource {
    fn fetch_events(&mut self, calendar_id: &str)
    -> Result<Vec<CalendarEvent>, CalendarFetchError>;
}

#[cfg(feature = "fixtures")]
pub struct FixtureCalendarSource {
    responses: HashMap<String, Result<Vec<CalendarEvent>, CalendarFetchError>>,
}

#[cfg(feature = "fixtures")]
impl FixtureCalendarSource {
    pub fn new(responses: HashMap<String, Result<Vec<CalendarEvent>, CalendarFetchError>>) -> Self {
        Self { responses }
    }
}

#[cfg(feature = "fixtures")]
impl CalendarSource for FixtureCalendarSource {
    fn fetch_events(
        &mut self,
        calendar_id: &str,
    ) -> Result<Vec<CalendarEvent>, CalendarFetchError> {
        self.responses.get(calendar_id).cloned().ok_or_else(|| {
            CalendarFetchError::MissingData(format!(
                "no fixture response for calendar '{calendar_id}'"
            ))
        })?
    }
}

// --- Google Calendar API v3 ------------------------------------------
//
// Response shape per the public docs at
// https://developers.google.com/calendar/api/v3/reference/events/list
// (`GET /calendars/{calendarId}/events`, OAuth2 bearer token): a top-level
// `items` array; each item carries `id`, `summary` (the event title - may
// be absent for a blank-titled event) and `start`/`end` objects. Timed
// events carry `dateTime`, usually RFC3339 and already offset-aware, but
// per the same docs `dateTime` may instead omit the offset when a sibling
// `timeZone` field carries the IANA zone name - that case is resolved via
// `resolve_offsetless_date_time` below rather than dropped. All-day events
// carry `date` (a bare `YYYY-MM-DD`) instead. Unverified against a live
// response - no service-account credential was available in this
// environment; built against the documented/example shape only.

#[derive(Debug, Deserialize)]
struct GoogleCalendarEventsResponse {
    #[serde(default)]
    items: Vec<GoogleCalendarEvent>,
}

#[derive(Debug, Deserialize)]
struct GoogleCalendarEvent {
    id: String,
    #[serde(default)]
    summary: Option<String>,
    start: GoogleCalendarEventTime,
    end: GoogleCalendarEventTime,
}

#[derive(Debug, Deserialize)]
struct GoogleCalendarEventTime {
    #[serde(rename = "dateTime", default)]
    date_time: Option<String>,
    #[serde(rename = "timeZone", default)]
    time_zone: Option<String>,
}

impl GoogleCalendarEventTime {
    /// `None` for an all-day event (no `dateTime` at all - the caller skips
    /// it rather than failing the batch, since a bare calendar date has no
    /// timezone-aware instant to evaluate "now falls within start/end"
    /// against). `Some(Err(_))` for a `dateTime` that is neither valid
    /// RFC3339 nor resolvable through `timeZone` - that must fail visibly
    /// rather than silently drop the event.
    fn resolve(&self) -> Option<Result<OffsetDateTime, CalendarFetchError>> {
        let date_time = self.date_time.as_deref()?;
        if let Ok(parsed) = OffsetDateTime::parse(date_time, &Rfc3339) {
            return Some(Ok(parsed));
        }
        Some(resolve_offsetless_date_time(
            date_time,
            self.time_zone.as_deref(),
        ))
    }
}

/// Parses a Google Calendar `events.list` response. All-day events (only
/// `date`, no `dateTime`) are skipped rather than failing the whole batch -
/// per the spec's resolved screensaver design, matching is against a
/// specific start/end instant, and a bare calendar date has no timezone-
/// aware instant to evaluate "now falls within start/end" against without
/// guessing one. An event missing a usable `summary` is also skipped: an
/// untitled event can never match a title rule, so it carries no
/// information for this feature. A `dateTime` that is present but
/// unresolvable (neither a valid offset nor a resolvable `timeZone`) fails
/// the whole response rather than silently dropping that one event.
pub fn parse_google_calendar_events_response(
    json: &str,
) -> Result<Vec<CalendarEvent>, CalendarFetchError> {
    let parsed: GoogleCalendarEventsResponse = serde_json::from_str(json)
        .map_err(|error| CalendarFetchError::InvalidJson(error.to_string()))?;

    let mut events = Vec::with_capacity(parsed.items.len());
    for item in parsed.items {
        let Some(title) = item.summary else {
            continue;
        };
        let Some(start) = item.start.resolve() else {
            continue;
        };
        let Some(end) = item.end.resolve() else {
            continue;
        };
        events.push(CalendarEvent {
            event_id: item.id,
            title,
            start: start?,
            end: end?,
        });
    }

    Ok(events)
}

fn matches_rule(title: &str, rule: &ScreensaverRule) -> bool {
    match rule.match_type {
        MatchType::Substring => title.to_lowercase().contains(&rule.pattern.to_lowercase()),
        MatchType::Regex => Regex::new(&rule.pattern)
            .map(|regex| regex.is_match(title))
            .unwrap_or(false),
    }
}

/// Per the spec: "if any event matches a rule and `now` falls within its
/// start/end, activate the screensaver. Multiple matching events behave as
/// a union (any match keeps it active)." `start` is inclusive, `end` is
/// exclusive, matching `schedule.rs`'s display-window convention.
pub fn evaluate_screensaver_rules(
    rules: &[ScreensaverRule],
    events: &[CalendarEvent],
    now: OffsetDateTime,
) -> bool {
    events.iter().any(|event| {
        now >= event.start
            && now < event.end
            && rules.iter().any(|rule| matches_rule(&event.title, rule))
    })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CalendarAvailability {
    Fresh,
    Stale { age: Duration },
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarCacheSnapshot {
    pub active: bool,
    pub last_success_at_unix: Option<i64>,
}

#[derive(Debug, Default)]
pub struct CalendarPollState {
    active: bool,
    last_success_at: Option<OffsetDateTime>,
}

impl CalendarPollState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_success(&mut self, active: bool, now: OffsetDateTime) {
        self.active = active;
        self.last_success_at = Some(now);
    }

    pub fn record_failure(&mut self) {}

    pub fn export_snapshot(&self) -> CalendarCacheSnapshot {
        CalendarCacheSnapshot {
            active: self.active,
            last_success_at_unix: self.last_success_at.map(OffsetDateTime::unix_timestamp),
        }
    }

    pub fn restore_snapshot(&mut self, snapshot: CalendarCacheSnapshot) {
        if let Ok(last_success_at) = snapshot
            .last_success_at_unix
            .map(OffsetDateTime::from_unix_timestamp)
            .transpose()
        {
            self.active = snapshot.active;
            self.last_success_at = last_success_at;
        }
    }

    pub fn screensaver_active(&self) -> bool {
        self.active
    }

    pub fn availability(&self, now: OffsetDateTime) -> CalendarAvailability {
        match self.last_success_at {
            None => CalendarAvailability::Unavailable,
            Some(last_success_at) => {
                let age = now - last_success_at;
                if age <= CALENDAR_STALE_AFTER {
                    CalendarAvailability::Fresh
                } else {
                    CalendarAvailability::Stale { age }
                }
            }
        }
    }
}

/// Drives one poll cycle: fetch, evaluate the configured rules against
/// whatever came back, and record the result into `state`. A fetch
/// failure calls `record_failure`, which holds `state`'s last known
/// `active` value unchanged - the caller never has to duplicate that
/// hold-last-state logic. `dashboard-pi`'s background poll thread and
/// `dashboard-web`'s fixture refresh both drive this same function, same
/// pattern as `standings.rs`'s `fetch_highlightly_standings`.
pub fn poll_calendar<S: CalendarSource>(
    source: &mut S,
    calendar_id: &str,
    rules: &[ScreensaverRule],
    now: OffsetDateTime,
    state: &mut CalendarPollState,
) {
    match source.fetch_events(calendar_id) {
        Ok(events) => {
            let active = evaluate_screensaver_rules(rules, &events, now);
            state.record_success(active, now);
        }
        Err(_) => state.record_failure(),
    }
}
