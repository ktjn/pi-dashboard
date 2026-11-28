use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use time::{Duration, OffsetDateTime};

pub const STANDINGS_STALE_AFTER: Duration = Duration::hours(2);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StandingsRow {
    pub competition: String,
    pub season: String,
    pub position: u32,
    pub team: String,
    pub played: u32,
    pub goal_difference: i32,
    pub points: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StandingsFetchError {
    InvalidJson(String),
    MissingData(String),
}

impl fmt::Display for StandingsFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(message) => {
                write!(f, "standings response is not valid JSON: {message}")
            }
            Self::MissingData(message) => {
                write!(f, "standings response is missing usable data: {message}")
            }
        }
    }
}

impl std::error::Error for StandingsFetchError {}

/// One trait for both providers (football-data.org for Serie A, Highlightly
/// for Allsvenskan) so panel rendering doesn't care which backs a given
/// competition - mirrors `WeatherSource`/`DeparturesSource`.
pub trait StandingsSource {
    fn fetch_standings(
        &mut self,
        competition_code: &str,
    ) -> Result<Vec<StandingsRow>, StandingsFetchError>;
}

#[cfg(feature = "fixtures")]
pub struct FixtureStandingsSource {
    responses: HashMap<String, Result<Vec<StandingsRow>, StandingsFetchError>>,
}

#[cfg(feature = "fixtures")]
impl FixtureStandingsSource {
    pub fn new(responses: HashMap<String, Result<Vec<StandingsRow>, StandingsFetchError>>) -> Self {
        Self { responses }
    }
}

#[cfg(feature = "fixtures")]
impl StandingsSource for FixtureStandingsSource {
    fn fetch_standings(
        &mut self,
        competition_code: &str,
    ) -> Result<Vec<StandingsRow>, StandingsFetchError> {
        self.responses
            .get(competition_code)
            .cloned()
            .ok_or_else(|| {
                StandingsFetchError::MissingData(format!(
                    "no fixture response for competition '{competition_code}'"
                ))
            })?
    }
}

// --- football-data.org (Serie A) -------------------------------------
//
// Response shape per the public docs at
// https://docs.football-data.org/general/v4/competition.html
// (`GET /v4/competitions/{id}/standings`, auth header `X-Auth-Token`,
// free tier 10 req/min): a `season` object with `startDate`/`endDate`,
// and a `standings` array with one entry per `type` (`TOTAL`/`HOME`/
// `AWAY`), each holding a `table` of rows already ranked by the
// provider. Unverified against a live response - no API key was
// available in this environment; built against the documented/example
// shape only.

#[derive(Debug, Deserialize)]
struct FootballDataResponse {
    competition: FootballDataCompetition,
    season: FootballDataSeason,
    standings: Vec<FootballDataStandingGroup>,
}

#[derive(Debug, Deserialize)]
struct FootballDataCompetition {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FootballDataSeason {
    start_date: String,
    end_date: String,
}

#[derive(Debug, Deserialize)]
struct FootballDataStandingGroup {
    #[serde(rename = "type")]
    standing_type: Option<String>,
    table: Vec<FootballDataRow>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FootballDataRow {
    position: u32,
    team: FootballDataTeam,
    played_games: u32,
    points: i32,
    goal_difference: i32,
}

#[derive(Debug, Deserialize)]
struct FootballDataTeam {
    name: String,
}

fn season_year(date: &str) -> Option<&str> {
    date.get(0..4)
}

pub fn parse_football_data_standings_response(
    json: &str,
) -> Result<Vec<StandingsRow>, StandingsFetchError> {
    let parsed: FootballDataResponse = serde_json::from_str(json)
        .map_err(|error| StandingsFetchError::InvalidJson(error.to_string()))?;

    let group = parsed
        .standings
        .iter()
        .find(|group| group.standing_type.as_deref() == Some("TOTAL"))
        .ok_or_else(|| {
            StandingsFetchError::MissingData(
                "standings: no 'TOTAL' standings group present (only HOME/AWAY or none)"
                    .to_string(),
            )
        })?;

    let start_year = season_year(&parsed.season.start_date).ok_or_else(|| {
        StandingsFetchError::MissingData(format!(
            "season.startDate: '{}' is not a usable date",
            parsed.season.start_date
        ))
    })?;
    let end_year = season_year(&parsed.season.end_date).ok_or_else(|| {
        StandingsFetchError::MissingData(format!(
            "season.endDate: '{}' is not a usable date",
            parsed.season.end_date
        ))
    })?;
    let season = format!("{start_year}/{end_year}");

    // Deliberately not sorted: the provider's own `table` order is
    // preserved verbatim, tiebreakers included - re-sorting by points
    // alone would silently drop tiebreakers we can't replicate.
    Ok(group
        .table
        .iter()
        .map(|row| StandingsRow {
            competition: parsed.competition.name.clone(),
            season: season.clone(),
            position: row.position,
            team: row.team.name.clone(),
            played: row.played_games,
            goal_difference: row.goal_difference,
            points: row.points,
        })
        .collect())
}

// --- Highlightly Football API (Allsvenskan) ---------------------------
//
// Response shape per the public docs at
// https://highlightly.net/football-api/documentation/standings/
// (`GET https://soccer.highlightly.net/standings?leagueId=&season=`,
// auth header `x-rapidapi-key`): a `groups` array (each group a
// section/pool of the table) and a top-level `league` object carrying
// the season. There is no `goalDifference` field; it's derived from
// `total.scoredGoals - total.receivedGoals`. Unverified against a live
// response - no API key was available in this environment; built
// against the documented/example shape only.

#[derive(Debug, Deserialize)]
struct HighlightlyResponse {
    groups: Vec<HighlightlyGroup>,
    league: HighlightlyLeague,
}

#[derive(Debug, Deserialize)]
struct HighlightlyGroup {
    standings: Vec<HighlightlyStanding>,
}

#[derive(Debug, Deserialize)]
struct HighlightlyStanding {
    position: u32,
    points: i32,
    team: HighlightlyTeam,
    total: HighlightlyTotal,
}

#[derive(Debug, Deserialize)]
struct HighlightlyTeam {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HighlightlyTotal {
    games: u32,
    scored_goals: i32,
    received_goals: i32,
}

#[derive(Debug, Deserialize)]
struct HighlightlyLeague {
    name: String,
    season: serde_json::Value,
}

pub fn parse_highlightly_standings_response(
    json: &str,
) -> Result<Vec<StandingsRow>, StandingsFetchError> {
    let parsed: HighlightlyResponse = serde_json::from_str(json)
        .map_err(|error| StandingsFetchError::InvalidJson(error.to_string()))?;

    let group = parsed.groups.first().ok_or_else(|| {
        StandingsFetchError::MissingData("groups: no standings groups present".to_string())
    })?;

    let season = match &parsed.league.season {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Number(value) => value.to_string(),
        other => {
            return Err(StandingsFetchError::MissingData(format!(
                "league.season: '{other}' is not a usable season value"
            )));
        }
    };

    // Deliberately not sorted, same reasoning as the football-data.org
    // parser above: the provider's own `standings` order is preserved.
    Ok(group
        .standings
        .iter()
        .map(|row| StandingsRow {
            competition: parsed.league.name.clone(),
            season: season.clone(),
            position: row.position,
            team: row.team.name.clone(),
            played: row.total.games,
            goal_difference: row.total.scored_goals - row.total.received_goals,
            points: row.points,
        })
        .collect())
}

// --- Highlightly Football API: current-season resolution --------------
//
// Highlightly's standings endpoint requires an explicit numeric `season`
// rather than offering a "current" shorthand, and there is no reliable
// way to derive which season is current from the calendar alone -
// Allsvenskan runs within a single calendar year while most of Europe's
// leagues split across two, and guessing from "now" is silently wrong
// for part of the year (the removed `highlightly_season_guess` in
// `dashboard-pi`). Instead, the season is resolved from a real lookup
// against `GET https://soccer.highlightly.net/leagues/{id}`, which the
// builder's research found returns a `seasons` array. Unverified
// against a live response - no API key was available in this
// environment; built against the documented/example shape only. This
// parser only trusts a season explicitly marked `current: true`; if none
// is marked, that's treated as missing data (`Unavailable`) rather than
// guessing the newest entry, since "newest" and "currently active" are
// not guaranteed to be the same season.

#[derive(Debug, Deserialize)]
struct HighlightlyLeagueResponse {
    seasons: Vec<HighlightlySeasonEntry>,
}

#[derive(Debug, Deserialize)]
struct HighlightlySeasonEntry {
    season: serde_json::Value,
    #[serde(default)]
    current: bool,
}

pub fn parse_highlightly_current_season_response(
    json: &str,
) -> Result<String, StandingsFetchError> {
    let parsed: HighlightlyLeagueResponse = serde_json::from_str(json)
        .map_err(|error| StandingsFetchError::InvalidJson(error.to_string()))?;

    let entry = parsed
        .seasons
        .iter()
        .find(|entry| entry.current)
        .ok_or_else(|| {
            StandingsFetchError::MissingData("seasons: no season is marked current".to_string())
        })?;

    match &entry.season {
        serde_json::Value::String(value) => Ok(value.clone()),
        serde_json::Value::Number(value) => Ok(value.to_string()),
        other => Err(StandingsFetchError::MissingData(format!(
            "seasons[].season: '{other}' is not a usable season value"
        ))),
    }
}

/// Highlightly-specific analogue of `StandingsSource`: fetching a table
/// is a two-step process (resolve the current season, then fetch that
/// season's table), and a failure at either step must fail the whole
/// fetch to `Unavailable` rather than falling back to a guess. Kept as
/// its own trait (rather than folding into `StandingsSource`) so
/// `fetch_highlightly_standings` below stays generic and testable
/// against `FixtureHighlightlySource`, independent of
/// `dashboard-pi`'s real HTTP client.
pub trait HighlightlySeasonSource {
    fn resolve_current_season(&mut self, league_id: &str) -> Result<String, StandingsFetchError>;

    fn fetch_standings_for_season(
        &mut self,
        league_id: &str,
        season: &str,
    ) -> Result<Vec<StandingsRow>, StandingsFetchError>;
}

/// Resolves the current season, then fetches that season's table.
/// `dashboard-pi`'s real `HighlightlyStandingsSource` drives this over
/// HTTP once per hourly refresh cycle (not per render frame) - Highlightly's
/// free tier is 100 req/day, and two calls/hour is 48/day, well within
/// budget. Kept generic here so the resolve-then-fetch sequencing itself
/// is unit-testable without a live key.
pub fn fetch_highlightly_standings<S: HighlightlySeasonSource>(
    source: &mut S,
    league_id: &str,
) -> Result<Vec<StandingsRow>, StandingsFetchError> {
    let season = source.resolve_current_season(league_id)?;
    source.fetch_standings_for_season(league_id, &season)
}

#[cfg(feature = "fixtures")]
pub struct FixtureHighlightlySource {
    season_responses: HashMap<String, Result<String, StandingsFetchError>>,
    standings_responses: HashMap<(String, String), Result<Vec<StandingsRow>, StandingsFetchError>>,
    pub requested_seasons: Vec<String>,
}

#[cfg(feature = "fixtures")]
impl FixtureHighlightlySource {
    pub fn new(
        season_responses: HashMap<String, Result<String, StandingsFetchError>>,
        standings_responses: HashMap<
            (String, String),
            Result<Vec<StandingsRow>, StandingsFetchError>,
        >,
    ) -> Self {
        Self {
            season_responses,
            standings_responses,
            requested_seasons: Vec::new(),
        }
    }
}

#[cfg(feature = "fixtures")]
impl HighlightlySeasonSource for FixtureHighlightlySource {
    fn resolve_current_season(&mut self, league_id: &str) -> Result<String, StandingsFetchError> {
        self.season_responses
            .get(league_id)
            .cloned()
            .ok_or_else(|| {
                StandingsFetchError::MissingData(format!(
                    "no fixture season response for league '{league_id}'"
                ))
            })?
    }

    fn fetch_standings_for_season(
        &mut self,
        league_id: &str,
        season: &str,
    ) -> Result<Vec<StandingsRow>, StandingsFetchError> {
        self.requested_seasons.push(season.to_string());
        self.standings_responses
            .get(&(league_id.to_string(), season.to_string()))
            .cloned()
            .ok_or_else(|| {
                StandingsFetchError::MissingData(format!(
                    "no fixture standings response for league '{league_id}' season '{season}'"
                ))
            })?
    }
}

#[derive(Debug, Clone)]
struct CompetitionSnapshot {
    rows: Vec<StandingsRow>,
    fetched_at: time::OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StandingsPanelState {
    Fresh(Vec<StandingsRow>),
    Stale {
        rows: Vec<StandingsRow>,
        age: Duration,
    },
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CachedCompetitionStandings {
    pub competition_id: String,
    pub fetched_at_unix: i64,
    pub rows: Vec<StandingsRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct StandingsCacheSnapshot {
    pub competitions: Vec<CachedCompetitionStandings>,
}

#[derive(Debug, Default)]
pub struct StandingsBoard {
    competitions: HashMap<String, CompetitionSnapshot>,
}

impl StandingsBoard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_success(
        &mut self,
        competition_id: &str,
        rows: Vec<StandingsRow>,
        fetched_at: OffsetDateTime,
    ) {
        self.competitions.insert(
            competition_id.to_string(),
            CompetitionSnapshot { rows, fetched_at },
        );
    }

    pub fn export_snapshot(&self) -> StandingsCacheSnapshot {
        let mut competitions: Vec<CachedCompetitionStandings> = self
            .competitions
            .iter()
            .map(|(competition_id, snapshot)| CachedCompetitionStandings {
                competition_id: competition_id.clone(),
                fetched_at_unix: snapshot.fetched_at.unix_timestamp(),
                rows: snapshot.rows.clone(),
            })
            .collect();
        competitions.sort_by(|left, right| left.competition_id.cmp(&right.competition_id));
        StandingsCacheSnapshot { competitions }
    }

    pub fn restore_snapshot(&mut self, snapshot: StandingsCacheSnapshot) {
        self.competitions.clear();
        for entry in snapshot.competitions {
            let Ok(fetched_at) = OffsetDateTime::from_unix_timestamp(entry.fetched_at_unix) else {
                continue;
            };
            self.competitions.insert(
                entry.competition_id,
                CompetitionSnapshot {
                    rows: entry.rows,
                    fetched_at,
                },
            );
        }
    }

    pub fn competition_state(
        &self,
        competition_id: &str,
        now: OffsetDateTime,
    ) -> StandingsPanelState {
        let Some(snapshot) = self.competitions.get(competition_id) else {
            return StandingsPanelState::Unavailable;
        };

        let age = now - snapshot.fetched_at;
        if age <= STANDINGS_STALE_AFTER {
            StandingsPanelState::Fresh(snapshot.rows.clone())
        } else {
            StandingsPanelState::Stale {
                rows: snapshot.rows.clone(),
                age,
            }
        }
    }
}

/// Alternates between the configured competitions on a fixed interval, so
/// both are readable within one rotation without a general animation
/// engine: a pure function of `now` and `interval_seconds`, with no
/// internal mutable state to drift or need driving from the render loop.
#[derive(Debug, Clone)]
pub struct StandingsRotation {
    competition_ids: Vec<String>,
    interval_seconds: u32,
}

impl StandingsRotation {
    pub fn new(competition_ids: Vec<String>, interval_seconds: u32) -> Self {
        Self {
            competition_ids,
            interval_seconds,
        }
    }

    /// Returns the competition id that should be shown at `now`. Panics
    /// only if constructed with zero competitions, which is a programmer
    /// error (`board_config.rs` already rejects an empty `competitions`
    /// list at `board.toml` load time - `panel_rotation.panels` itself is
    /// unused here) - not a possible runtime/network input.
    pub fn active_competition(&self, now: time::OffsetDateTime) -> &str {
        let len = self.competition_ids.len();
        assert!(
            len > 0,
            "StandingsRotation requires at least one competition"
        );
        if len == 1 || self.interval_seconds == 0 {
            return &self.competition_ids[0];
        }
        let slot = now.unix_timestamp() / i64::from(self.interval_seconds);
        let index = slot.rem_euclid(len as i64) as usize;
        &self.competition_ids[index]
    }

    pub fn rotation_progress(&self, now: time::OffsetDateTime) -> f32 {
        if self.competition_ids.len() <= 1 || self.interval_seconds == 0 {
            return 0.0;
        }
        let interval_nanos = i128::from(self.interval_seconds) * 1_000_000_000;
        let elapsed_nanos = now.unix_timestamp_nanos().rem_euclid(interval_nanos);
        let progress = (elapsed_nanos as f64 / interval_nanos as f64) as f32;
        if progress >= 1.0 {
            0.0
        } else {
            progress.max(0.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_never_panics_with_a_zero_interval() {
        let rotation =
            StandingsRotation::new(vec!["allsvenskan".to_string(), "serie-a".to_string()], 0);
        assert_eq!(
            rotation.active_competition(time::OffsetDateTime::UNIX_EPOCH),
            "allsvenskan"
        );
    }

    #[test]
    fn rotation_progress_tracks_subsecond_elapsed_window_in_unit_half_open_range() {
        let multi =
            StandingsRotation::new(vec!["allsvenskan".to_string(), "serie-a".to_string()], 15);
        let single = StandingsRotation::new(vec!["allsvenskan".to_string()], 15);
        let zero_interval =
            StandingsRotation::new(vec!["allsvenskan".to_string(), "serie-a".to_string()], 0);
        let empty = StandingsRotation::new(Vec::new(), 15);

        let base = time::OffsetDateTime::UNIX_EPOCH + Duration::seconds(1_782_037_800);
        assert_eq!(single.rotation_progress(base + Duration::seconds(5)), 0.0);
        assert_eq!(
            zero_interval.rotation_progress(base + Duration::seconds(5)),
            0.0
        );
        assert_eq!(empty.rotation_progress(base + Duration::seconds(5)), 0.0);

        assert!((multi.rotation_progress(base) - 0.0).abs() < 1e-6);
        assert!((multi.rotation_progress(base + Duration::seconds(3)) - 0.2).abs() < 1e-6);
        assert!((multi.rotation_progress(base + Duration::milliseconds(7_500)) - 0.5).abs() < 1e-6);
        assert!((multi.rotation_progress(base + Duration::seconds(12)) - 0.8).abs() < 1e-6);
        assert!((multi.rotation_progress(base + Duration::seconds(15)) - 0.0).abs() < 1e-6);

        let just_before_wrap =
            multi.rotation_progress(base + Duration::seconds(15) - Duration::nanoseconds(1));
        assert!((0.0..1.0).contains(&just_before_wrap));

        let before_epoch = time::OffsetDateTime::UNIX_EPOCH - Duration::seconds(5);
        assert!((multi.rotation_progress(before_epoch) - (10.0 / 15.0)).abs() < 1e-6);
    }

    #[test]
    fn parses_the_current_season_marked_true_in_the_documented_leagues_response_shape() {
        let json =
            r#"{"seasons":[{"season":2024,"current":false},{"season":2025,"current":true}]}"#;
        let season = parse_highlightly_current_season_response(json).expect("must parse");
        assert_eq!(season, "2025");
    }

    #[test]
    fn rejects_a_leagues_response_with_no_season_marked_current() {
        let json = r#"{"seasons":[{"season":2024,"current":false},{"season":2025}]}"#;
        let error = parse_highlightly_current_season_response(json).unwrap_err();
        assert!(matches!(error, StandingsFetchError::MissingData(_)));
    }

    #[test]
    fn rejects_malformed_leagues_json() {
        let error = parse_highlightly_current_season_response("{not json").unwrap_err();
        assert!(matches!(error, StandingsFetchError::InvalidJson(_)));
    }

    #[cfg(feature = "fixtures")]
    #[test]
    fn fetch_highlightly_standings_uses_the_resolved_season_to_fetch_the_table() {
        let row = StandingsRow {
            competition: "Allsvenskan".to_string(),
            season: "2025".to_string(),
            position: 1,
            team: "Malmö FF".to_string(),
            played: 30,
            goal_difference: 20,
            points: 55,
        };
        let mut season_responses = HashMap::new();
        season_responses.insert("113".to_string(), Ok("2025".to_string()));
        let mut standings_responses = HashMap::new();
        standings_responses.insert(
            ("113".to_string(), "2025".to_string()),
            Ok(vec![row.clone()]),
        );
        let mut source = FixtureHighlightlySource::new(season_responses, standings_responses);

        let rows = fetch_highlightly_standings(&mut source, "113").expect("must succeed");

        assert_eq!(rows, vec![row]);
        assert_eq!(source.requested_seasons, vec!["2025".to_string()]);
    }

    #[cfg(feature = "fixtures")]
    #[test]
    fn fetch_highlightly_standings_fails_when_season_resolution_fails() {
        let mut season_responses = HashMap::new();
        season_responses.insert(
            "113".to_string(),
            Err(StandingsFetchError::MissingData(
                "leagues lookup failed".to_string(),
            )),
        );
        let mut source = FixtureHighlightlySource::new(season_responses, HashMap::new());

        let error = fetch_highlightly_standings(&mut source, "113").unwrap_err();

        assert!(matches!(error, StandingsFetchError::MissingData(_)));
        // The standings endpoint must never be called once season
        // resolution has already failed - no stale/guessed season used.
        assert!(source.requested_seasons.is_empty());
    }
}
