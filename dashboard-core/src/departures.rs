use crate::board_config::{Station, TransitMode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};

pub const DEPARTURES_STALE_AFTER: Duration = Duration::minutes(2);
pub const DEVIATIONS_STALE_AFTER: Duration = Duration::minutes(10);

/// Mirrors `isTowardsStockholm` in the original React prototype
/// (`sl-dashboard/src/utils.ts`): a destination/direction substring list
/// that identifies "inbound to Stockholm" without a real routing graph.
const STOCKHOLM_DIRECTIONS: &[&str] = &[
    "kungsträdgården",
    "stockholm",
    "t-centralen",
    "slussen",
    "gullmarsplan",
    "farsta",
    "hagsätra",
    "skarpnäck",
    "bagarmossen",
    "nynäshamn",
    "västerhaninge",
    "jordbro",
    "haninge",
    "tumba",
    "södertälje",
    "gnesta",
    "sickla",
    "hammarby",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Departure {
    pub site_id: u32,
    pub line: String,
    pub destination: String,
    /// Minutes until the rider must leave: `(departure time - now) -
    /// walk_time_min`. Can be negative ("leave now"/"already late"); only
    /// departures whose actual departure time has already passed are
    /// removed from view entirely (see `DeparturesBoard::station_state`).
    pub countdown_min: i64,
    pub walk_time_min: u32,
    pub disruption: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeparturesPanelState {
    Fresh(Vec<Departure>),
    Stale {
        departures: Vec<Departure>,
        age: Duration,
    },
    Unavailable,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DepartureFetchError {
    InvalidJson(String),
    MissingData(String),
}

impl fmt::Display for DepartureFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(message) => {
                write!(f, "SL response is not valid JSON: {message}")
            }
            Self::MissingData(message) => {
                write!(f, "SL response is missing usable data: {message}")
            }
        }
    }
}

impl std::error::Error for DepartureFetchError {}

pub trait DeparturesSource {
    fn fetch_departures(&mut self, site_id: u32) -> Result<Vec<RawDeparture>, DepartureFetchError>;
}

pub trait DeviationsSource {
    fn fetch_deviations(&mut self, site_id: u32) -> Result<Vec<RawDeviation>, DepartureFetchError>;
}

#[cfg(feature = "fixtures")]
pub struct FixtureDeparturesSource {
    responses: HashMap<u32, Result<Vec<RawDeparture>, DepartureFetchError>>,
}

#[cfg(feature = "fixtures")]
impl FixtureDeparturesSource {
    pub fn new(responses: HashMap<u32, Result<Vec<RawDeparture>, DepartureFetchError>>) -> Self {
        Self { responses }
    }
}

#[cfg(feature = "fixtures")]
impl DeparturesSource for FixtureDeparturesSource {
    fn fetch_departures(&mut self, site_id: u32) -> Result<Vec<RawDeparture>, DepartureFetchError> {
        self.responses.get(&site_id).cloned().ok_or_else(|| {
            DepartureFetchError::MissingData(format!("no fixture response for site {site_id}"))
        })?
    }
}

#[cfg(feature = "fixtures")]
pub struct FixtureDeviationsSource {
    responses: HashMap<u32, Result<Vec<RawDeviation>, DepartureFetchError>>,
}

#[cfg(feature = "fixtures")]
impl FixtureDeviationsSource {
    pub fn new(responses: HashMap<u32, Result<Vec<RawDeviation>, DepartureFetchError>>) -> Self {
        Self { responses }
    }
}

#[cfg(feature = "fixtures")]
impl DeviationsSource for FixtureDeviationsSource {
    fn fetch_deviations(&mut self, site_id: u32) -> Result<Vec<RawDeviation>, DepartureFetchError> {
        self.responses
            .get(&site_id)
            .cloned()
            .unwrap_or_else(|| Ok(Vec::new()))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawDeparture {
    pub destination: String,
    pub direction: Option<String>,
    pub transport_mode: TransitMode,
    pub line_designation: String,
    pub scheduled: OffsetDateTime,
    pub expected: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawDeviation {
    pub deviation_case_id: i64,
    pub header: String,
    pub importance_level: i64,
    pub publish_from: Option<OffsetDateTime>,
    pub publish_upto: Option<OffsetDateTime>,
    pub affected_modes: Vec<TransitMode>,
}

#[derive(Debug, Deserialize)]
struct SlDeparturesResponse {
    departures: Vec<SlDeparture>,
}

#[derive(Debug, Deserialize)]
struct SlDeparture {
    destination: String,
    #[serde(default)]
    direction: Option<String>,
    scheduled: String,
    #[serde(default)]
    expected: Option<String>,
    line: SlLine,
}

#[derive(Debug, Deserialize)]
struct SlLine {
    designation: String,
    transport_mode: String,
}

/// Parses `GET https://transport.integration.sl.se/v1/sites/{site_id}/departures`.
/// `scheduled`/`expected` are local wall-clock timestamps with no offset
/// (confirmed against the live endpoint, e.g. `"2026-11-24T22:46:18"`), so
/// `time_zone` (the board's configured IANA zone) is required to resolve
/// them to an absolute instant.
pub fn parse_sl_departures_response(
    json: &str,
    time_zone: &str,
) -> Result<Vec<RawDeparture>, DepartureFetchError> {
    let parsed: SlDeparturesResponse = serde_json::from_str(json)
        .map_err(|error| DepartureFetchError::InvalidJson(error.to_string()))?;

    // Fail-open on an unrecognized `transport_mode`: SL's live vocabulary
    // includes values beyond METRO/TRAIN/TRAM/BUS/FERRY/SHIP (e.g. `TAXI`,
    // confirmed at a real endpoint), so one unrecognized departure must not
    // discard the whole batch. A malformed timestamp on an otherwise
    // recognized departure is still a genuine parse error.
    let mut departures = Vec::with_capacity(parsed.departures.len());
    for departure in parsed.departures {
        let Some(transport_mode) = TransitMode::parse(&departure.line.transport_mode) else {
            continue;
        };
        let scheduled = parse_sl_local_time(&departure.scheduled, time_zone)?;
        let expected = departure
            .expected
            .as_deref()
            .map(|value| parse_sl_local_time(value, time_zone))
            .transpose()?;
        departures.push(RawDeparture {
            destination: departure.destination,
            direction: departure.direction,
            transport_mode,
            line_designation: departure.line.designation,
            scheduled,
            expected,
        });
    }
    Ok(departures)
}

fn parse_sl_local_time(
    value: &str,
    time_zone: &str,
) -> Result<OffsetDateTime, DepartureFetchError> {
    let civil: jiff::civil::DateTime = value.parse().map_err(|error| {
        DepartureFetchError::InvalidJson(format!(
            "'{value}' is not a valid local datetime: {error}"
        ))
    })?;
    let zoned = civil.in_tz(time_zone).map_err(|_| {
        DepartureFetchError::MissingData(format!(
            "'{time_zone}' is not a recognized IANA time zone"
        ))
    })?;
    let timestamp = zoned.timestamp();
    OffsetDateTime::from_unix_timestamp(timestamp.as_second()).map_err(|error| {
        DepartureFetchError::MissingData(format!(
            "'{value}' resolved to an invalid instant: {error}"
        ))
    })
}

#[derive(Debug, Deserialize)]
struct SlDeviationMessage {
    deviation_case_id: i64,
    #[serde(default)]
    publish: Option<SlPublish>,
    #[serde(default)]
    priority: Option<SlPriority>,
    #[serde(default)]
    message_variants: Vec<SlMessageVariant>,
    #[serde(default)]
    scope: Option<SlScope>,
}

#[derive(Debug, Deserialize)]
struct SlPublish {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    upto: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlPriority {
    #[serde(default)]
    importance_level: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SlMessageVariant {
    header: String,
    #[serde(default)]
    language: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlScope {
    #[serde(default)]
    lines: Vec<SlScopeEntry>,
    #[serde(default)]
    stop_areas: Vec<SlScopeEntry>,
}

#[derive(Debug, Deserialize)]
struct SlScopeEntry {
    #[serde(default)]
    transport_mode: Option<String>,
}

/// Parses `GET https://deviations.integration.sl.se/v1/messages?site={site_id}`.
/// The response is a bare JSON array, not `{ "messages": [...] }`. Unlike
/// departures, `publish.from`/`publish.upto` carry an explicit UTC offset
/// (e.g. `"2024-01-03T07:45:54.647+01:00"`), confirmed against the live
/// endpoint, so they parse directly with no time-zone lookup.
pub fn parse_sl_deviations_response(json: &str) -> Result<Vec<RawDeviation>, DepartureFetchError> {
    let parsed: Vec<SlDeviationMessage> = serde_json::from_str(json)
        .map_err(|error| DepartureFetchError::InvalidJson(error.to_string()))?;

    parsed
        .into_iter()
        .map(|message| {
            let header = message
                .message_variants
                .iter()
                .find(|variant| variant.language.as_deref() == Some("sv"))
                .or_else(|| message.message_variants.first())
                .map(|variant| variant.header.clone())
                .ok_or_else(|| {
                    DepartureFetchError::MissingData(format!(
                        "deviation {}: message_variants is empty",
                        message.deviation_case_id
                    ))
                })?;

            let publish_from = message
                .publish
                .as_ref()
                .and_then(|publish| publish.from.as_deref())
                .map(parse_sl_offset_time)
                .transpose()?;
            let publish_upto = message
                .publish
                .as_ref()
                .and_then(|publish| publish.upto.as_deref())
                .map(parse_sl_offset_time)
                .transpose()?;

            let affected_modes = message
                .scope
                .iter()
                .flat_map(|scope| scope.lines.iter().chain(scope.stop_areas.iter()))
                .filter_map(|entry| entry.transport_mode.as_deref())
                .filter_map(TransitMode::parse)
                .collect();

            Ok(RawDeviation {
                deviation_case_id: message.deviation_case_id,
                header,
                importance_level: message
                    .priority
                    .and_then(|priority| priority.importance_level)
                    .unwrap_or(0),
                publish_from,
                publish_upto,
                affected_modes,
            })
        })
        .collect()
}

fn parse_sl_offset_time(value: &str) -> Result<OffsetDateTime, DepartureFetchError> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|error| DepartureFetchError::InvalidJson(format!("'{value}': {error}")))
}

fn matches_direction(departure: &RawDeparture, direction: &str) -> bool {
    if direction.is_empty() || direction.eq_ignore_ascii_case("all") {
        return true;
    }
    let destination = departure.destination.to_lowercase();
    let departure_direction = departure
        .direction
        .as_deref()
        .map(str::to_lowercase)
        .unwrap_or_default();
    if direction.eq_ignore_ascii_case("stockholm") {
        return STOCKHOLM_DIRECTIONS
            .iter()
            .any(|towards| destination.contains(towards) || departure_direction.contains(towards));
    }
    direction
        .split('|')
        .filter(|keyword| !keyword.is_empty())
        .any(|keyword| {
            if let Some(exact) = keyword.strip_prefix('=') {
                let exact = exact.to_lowercase();
                destination == exact || departure_direction == exact
            } else {
                let keyword = keyword.to_lowercase();
                destination.contains(&keyword) || departure_direction.contains(&keyword)
            }
        })
}

fn active_disruption_text(
    snapshot: &SiteDeviationsSnapshot,
    mode: TransitMode,
    now: OffsetDateTime,
) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    let headers: Vec<&str> = snapshot
        .deviations
        .iter()
        .filter(|deviation| {
            deviation.affected_modes.is_empty() || deviation.affected_modes.contains(&mode)
        })
        .filter(|deviation| is_deviation_active(deviation, snapshot.fetched_at, now))
        .map(|deviation| deviation.header.as_str())
        .filter(|header| seen.insert(*header))
        .collect();
    if headers.is_empty() {
        None
    } else {
        Some(headers.join("; "))
    }
}

fn is_deviation_active(
    deviation: &RawDeviation,
    fetched_at: Option<OffsetDateTime>,
    now: OffsetDateTime,
) -> bool {
    if let Some(from) = deviation.publish_from
        && from > now
    {
        return false;
    }
    if let Some(upto) = deviation.publish_upto {
        if upto < now {
            return false;
        }
    } else if fetched_at.is_some_and(|fetched| now - fetched > DEVIATIONS_STALE_AFTER) {
        return false;
    }
    true
}

#[derive(Debug, Clone)]
struct SiteDeparturesSnapshot {
    departures: Vec<RawDeparture>,
    fetched_at: OffsetDateTime,
}

#[derive(Debug, Clone)]
struct SiteDeviationsSnapshot {
    deviations: Vec<RawDeviation>,
    fetched_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedDeparture {
    pub destination: String,
    pub direction: Option<String>,
    pub transport_mode: TransitMode,
    pub line_designation: String,
    pub scheduled_unix: i64,
    pub expected_unix: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedSiteDepartures {
    pub site_id: u32,
    pub fetched_at_unix: i64,
    pub departures: Vec<CachedDeparture>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedDeviation {
    pub deviation_case_id: i64,
    pub header: String,
    pub importance_level: i64,
    pub publish_from_unix: Option<i64>,
    pub publish_upto_unix: Option<i64>,
    pub affected_modes: Vec<TransitMode>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedSiteDeviations {
    pub site_id: u32,
    pub fetched_at_unix: Option<i64>,
    pub deviations: Vec<CachedDeviation>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeparturesCacheSnapshot {
    pub departures: Vec<CachedSiteDepartures>,
    pub deviations: Vec<CachedSiteDeviations>,
}

#[derive(Debug, Default)]
pub struct DeparturesBoard {
    departures: HashMap<u32, SiteDeparturesSnapshot>,
    deviations: HashMap<u32, SiteDeviationsSnapshot>,
}

impl DeparturesBoard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_departures_success(
        &mut self,
        site_id: u32,
        departures: Vec<RawDeparture>,
        fetched_at: OffsetDateTime,
    ) {
        self.departures.insert(
            site_id,
            SiteDeparturesSnapshot {
                departures,
                fetched_at,
            },
        );
    }

    pub fn record_deviations_success(&mut self, site_id: u32, deviations: Vec<RawDeviation>) {
        self.deviations.insert(
            site_id,
            SiteDeviationsSnapshot {
                deviations,
                fetched_at: None,
            },
        );
    }

    pub fn record_deviations_success_at(
        &mut self,
        site_id: u32,
        deviations: Vec<RawDeviation>,
        fetched_at: OffsetDateTime,
    ) {
        self.deviations.insert(
            site_id,
            SiteDeviationsSnapshot {
                deviations,
                fetched_at: Some(fetched_at),
            },
        );
    }

    pub fn export_snapshot(&self) -> DeparturesCacheSnapshot {
        let mut departures: Vec<CachedSiteDepartures> = self
            .departures
            .iter()
            .map(|(&site_id, snapshot)| CachedSiteDepartures {
                site_id,
                fetched_at_unix: snapshot.fetched_at.unix_timestamp(),
                departures: snapshot
                    .departures
                    .iter()
                    .map(|raw| CachedDeparture {
                        destination: raw.destination.clone(),
                        direction: raw.direction.clone(),
                        transport_mode: raw.transport_mode,
                        line_designation: raw.line_designation.clone(),
                        scheduled_unix: raw.scheduled.unix_timestamp(),
                        expected_unix: raw.expected.map(OffsetDateTime::unix_timestamp),
                    })
                    .collect(),
            })
            .collect();
        departures.sort_by_key(|site| site.site_id);

        let mut deviations: Vec<CachedSiteDeviations> = self
            .deviations
            .iter()
            .map(|(&site_id, snapshot)| CachedSiteDeviations {
                site_id,
                fetched_at_unix: snapshot.fetched_at.map(OffsetDateTime::unix_timestamp),
                deviations: snapshot
                    .deviations
                    .iter()
                    .map(|raw| CachedDeviation {
                        deviation_case_id: raw.deviation_case_id,
                        header: raw.header.clone(),
                        importance_level: raw.importance_level,
                        publish_from_unix: raw.publish_from.map(OffsetDateTime::unix_timestamp),
                        publish_upto_unix: raw.publish_upto.map(OffsetDateTime::unix_timestamp),
                        affected_modes: raw.affected_modes.clone(),
                    })
                    .collect(),
            })
            .collect();
        deviations.sort_by_key(|site| site.site_id);

        DeparturesCacheSnapshot {
            departures,
            deviations,
        }
    }

    pub fn restore_snapshot(&mut self, snapshot: DeparturesCacheSnapshot) {
        self.departures.clear();
        for site in snapshot.departures {
            let Ok(fetched_at) = OffsetDateTime::from_unix_timestamp(site.fetched_at_unix) else {
                continue;
            };
            let departures: Vec<RawDeparture> = site
                .departures
                .into_iter()
                .filter_map(|raw| {
                    let scheduled = OffsetDateTime::from_unix_timestamp(raw.scheduled_unix).ok()?;
                    let expected = raw
                        .expected_unix
                        .map(OffsetDateTime::from_unix_timestamp)
                        .transpose()
                        .ok()?;
                    Some(RawDeparture {
                        destination: raw.destination,
                        direction: raw.direction,
                        transport_mode: raw.transport_mode,
                        line_designation: raw.line_designation,
                        scheduled,
                        expected,
                    })
                })
                .collect();
            self.departures.insert(
                site.site_id,
                SiteDeparturesSnapshot {
                    departures,
                    fetched_at,
                },
            );
        }

        self.deviations.clear();
        for site in snapshot.deviations {
            let Ok(fetched_at) = site
                .fetched_at_unix
                .map(OffsetDateTime::from_unix_timestamp)
                .transpose()
            else {
                continue;
            };
            let deviations: Vec<RawDeviation> = site
                .deviations
                .into_iter()
                .filter_map(|raw| {
                    let publish_from = raw
                        .publish_from_unix
                        .map(OffsetDateTime::from_unix_timestamp)
                        .transpose()
                        .ok()?;
                    let publish_upto = raw
                        .publish_upto_unix
                        .map(OffsetDateTime::from_unix_timestamp)
                        .transpose()
                        .ok()?;
                    Some(RawDeviation {
                        deviation_case_id: raw.deviation_case_id,
                        header: raw.header,
                        importance_level: raw.importance_level,
                        publish_from,
                        publish_upto,
                        affected_modes: raw.affected_modes,
                    })
                })
                .collect();
            self.deviations.insert(
                site.site_id,
                SiteDeviationsSnapshot {
                    deviations,
                    fetched_at,
                },
            );
        }
    }

    pub fn station_state(&self, station: &Station, now: OffsetDateTime) -> DeparturesPanelState {
        let Some(snapshot) = self.departures.get(&station.site_id) else {
            return DeparturesPanelState::Unavailable;
        };

        let disruption = self
            .deviations
            .get(&station.site_id)
            .and_then(|deviations| active_disruption_text(deviations, station.mode, now));

        let mut departures: Vec<Departure> = snapshot
            .departures
            .iter()
            .filter(|raw| raw.transport_mode == station.mode)
            .filter(|raw| matches_direction(raw, &station.direction))
            .filter_map(|raw| {
                let actual_departure = raw.expected.unwrap_or(raw.scheduled);
                if actual_departure <= now {
                    return None;
                }
                let minutes_to_departure = (actual_departure - now).whole_minutes();
                let countdown_min = minutes_to_departure - i64::from(station.walk_time_min);
                Some(Departure {
                    site_id: station.site_id,
                    line: raw.line_designation.clone(),
                    destination: raw.destination.clone(),
                    countdown_min,
                    walk_time_min: station.walk_time_min,
                    disruption: disruption.clone(),
                })
            })
            .collect();
        departures.sort_by_key(|departure| departure.countdown_min);

        let age = now - snapshot.fetched_at;
        if age <= DEPARTURES_STALE_AFTER {
            DeparturesPanelState::Fresh(departures)
        } else {
            DeparturesPanelState::Stale { departures, age }
        }
    }
}
