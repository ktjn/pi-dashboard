mod remote_control;

use dashboard_core::board_config::{BoardConfig, FootballProvider, Screensaver, load_board_config};
use dashboard_core::clock::local_clock;
use dashboard_core::daylight::daylight_window;
use dashboard_core::departures::{
    DepartureFetchError, DeparturesBoard, DeparturesSource, DeviationsSource, RawDeparture,
    RawDeviation, parse_sl_departures_response, parse_sl_deviations_response,
};
use dashboard_core::render::{
    STATION_PANEL_HEIGHT, render_clock_weather_header, render_daylight_strip,
    render_departures_panel, render_screensaver_view, render_standings_panel,
};
use dashboard_core::schedule::{
    DisplayPowerControl, DisplayState, NullDisplayPowerControl, active_override_power,
    is_within_display_window, reconcile, resolved_power,
};
use dashboard_core::screensaver::{
    CalendarEvent, CalendarFetchError, CalendarPollState, CalendarSource,
    evaluate_screensaver_rules, parse_google_calendar_events_response,
};
use dashboard_core::standings::{
    HighlightlySeasonSource, StandingsBoard, StandingsFetchError, StandingsRotation, StandingsRow,
    StandingsSource, fetch_highlightly_standings, parse_football_data_standings_response,
    parse_highlightly_current_season_response, parse_highlightly_standings_response,
};
use dashboard_core::weather::{
    WeatherFetchError, WeatherReading, WeatherSnapshotStore, WeatherSource,
    parse_open_meteo_response,
};
use jiff::civil::Date;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const WEATHER_REFRESH_INTERVAL: StdDuration = StdDuration::from_secs(15 * 60);
const WEATHER_HTTP_TIMEOUT: StdDuration = StdDuration::from_secs(5);
const DEPARTURES_REFRESH_INTERVAL: StdDuration = StdDuration::from_secs(30);
const DEVIATIONS_REFRESH_INTERVAL: StdDuration = StdDuration::from_secs(120);
const SL_HTTP_TIMEOUT: StdDuration = StdDuration::from_secs(5);
/// Spec's "hourly refresh" for football standings.
const STANDINGS_REFRESH_INTERVAL: StdDuration = StdDuration::from_secs(60 * 60);
const STANDINGS_HTTP_TIMEOUT: StdDuration = StdDuration::from_secs(10);
/// Falls back to `board.toml`'s own default when no `[panel_rotation]` is
/// configured at all.
const STANDINGS_ROTATION_DEFAULT_INTERVAL_SECONDS: u32 = 15;
/// Short enough for the real "once a minute" reconciliation (PI-05's own
/// acceptance criterion - the systemd-timer side of that is explicitly out
/// of scope for this slice) while staying cheap to poll continuously from a
/// background thread.
const DISPLAY_POWER_RECONCILE_INTERVAL: StdDuration = StdDuration::from_secs(60);
/// Spec's "poll on an interval short enough to catch event start/end within
/// about a minute."
const CALENDAR_POLL_INTERVAL: StdDuration = StdDuration::from_secs(60);
const CALENDAR_HTTP_TIMEOUT: StdDuration = StdDuration::from_secs(10);
const CALENDAR_TOKEN_SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly";
const CALENDAR_TOKEN_LIFETIME_SECONDS: i64 = 3600;
/// Refreshes the cached access token a little before Google's own
/// `expires_in` elapses, so a slow request never straddles the actual
/// expiry.
const CALENDAR_TOKEN_REFRESH_SKEW_SECONDS: i64 = 30;
/// `events.list`'s `timeMin`/`timeMax` window, bracketing "now": wide enough
/// that an event whose start or end straddles the poll instant is never cut
/// off by the boundary, narrow enough that pagination (`nextPageToken`,
/// deliberately unimplemented - see PI-07's review) is never needed even on
/// a busy calendar.
const CALENDAR_QUERY_WINDOW: time::Duration = time::Duration::hours(1);
/// Where the HTTP server writes the manual-override state and the
/// display-power reconciliation thread reads it from - a stand-in today
/// for the eventual systemd-timer-reads-the-file design
/// (`pi-dashboard-spec.md`'s PI-08 section): this process's own background
/// thread plays the timer's role until the real systemd timer exists.
const OVERRIDE_FILE_PATH: &str = "override-state.json";
/// Per `pi-dashboard-spec.md`: "provisioned once in the same manually-created
/// systemd `EnvironmentFile` as other credentials" - same pattern as
/// `FOOTBALL_DATA_ORG_API_KEY`/`HIGHLIGHTLY_API_KEY`, never a config file.
const REMOTE_CONTROL_TOKEN_ENV_VAR: &str = "REMOTE_CONTROL_TOKEN";
const REMOTE_CONTROL_BIND_ADDRESS: &str = "0.0.0.0:8080";

/// Deserialized shape of a Google service-account JSON key file (the
/// credential format Google's console issues) - unverified live, built
/// from https://developers.google.com/identity/protocols/oauth2/service-account's
/// documented fields. `token_uri` is present in every real key file,
/// documented default retained only so a hand-trimmed fixture/key still
/// parses.
#[derive(Debug, Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".to_string()
}

#[derive(Debug, Serialize)]
struct ServiceAccountClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    exp: i64,
    iat: i64,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<i64>,
}

/// Reads the service-account key's file path from the environment variable
/// named by `board.toml`'s `screensaver.credential_ref` - per the spec,
/// "provision all provider credentials... separately on the Pi, never in
/// the repo." Missing/unreadable/malformed credentials fail with a typed
/// error (surfacing as a held-last-state calendar check, never a panic),
/// same pattern as `FootballDataStandingsSource`'s API-key handling.
fn load_service_account_key(credential_ref: &str) -> Result<ServiceAccountKey, CalendarFetchError> {
    let path = std::env::var(credential_ref)
        .map_err(|_| CalendarFetchError::MissingData(format!("{credential_ref} is not set")))?;
    let contents = std::fs::read_to_string(&path)
        .map_err(|error| CalendarFetchError::MissingData(format!("{path}: {error}")))?;
    serde_json::from_str(&contents)
        .map_err(|error| CalendarFetchError::InvalidJson(error.to_string()))
}

/// Real Google Calendar client: exchanges the service-account key for a
/// short-lived OAuth2 access token via the documented JWT-bearer flow
/// (https://developers.google.com/identity/protocols/oauth2/service-account),
/// then calls `events.list`
/// (https://developers.google.com/calendar/api/v3/reference/events/list).
/// Both unverified live - no service-account credential was available in
/// this environment; built against the documented/example shapes only.
struct GoogleCalendarSource {
    client: reqwest::blocking::Client,
    credential_ref: String,
    cached_token: Option<(String, OffsetDateTime)>,
}

impl GoogleCalendarSource {
    fn new(credential_ref: String) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(CALENDAR_HTTP_TIMEOUT)
            .build()
            .expect("reqwest client must build");
        Self {
            client,
            credential_ref,
            cached_token: None,
        }
    }

    fn access_token(&mut self) -> Result<String, CalendarFetchError> {
        let now = OffsetDateTime::now_utc();
        if let Some((token, expires_at)) = &self.cached_token
            && now < *expires_at
        {
            return Ok(token.clone());
        }

        let key = load_service_account_key(&self.credential_ref)?;
        let iat = now.unix_timestamp();
        let claims = ServiceAccountClaims {
            iss: &key.client_email,
            scope: CALENDAR_TOKEN_SCOPE,
            aud: &key.token_uri,
            exp: iat + CALENDAR_TOKEN_LIFETIME_SECONDS,
            iat,
        };
        let encoding_key =
            EncodingKey::from_rsa_pem(key.private_key.as_bytes()).map_err(|error| {
                CalendarFetchError::MissingData(format!("service account private key: {error}"))
            })?;
        let assertion =
            encode(&Header::new(Algorithm::RS256), &claims, &encoding_key).map_err(|error| {
                CalendarFetchError::MissingData(format!("JWT signing failed: {error}"))
            })?;

        let body = self
            .client
            .post(&key.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .map_err(|error| CalendarFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| CalendarFetchError::MissingData(error.to_string()))?;
        let token_response: TokenResponse = serde_json::from_str(&body)
            .map_err(|error| CalendarFetchError::InvalidJson(error.to_string()))?;

        let expires_in = token_response
            .expires_in
            .unwrap_or(CALENDAR_TOKEN_LIFETIME_SECONDS);
        let expires_at = now + time::Duration::seconds(expires_in)
            - time::Duration::seconds(CALENDAR_TOKEN_REFRESH_SKEW_SECONDS);
        self.cached_token = Some((token_response.access_token.clone(), expires_at));
        Ok(token_response.access_token)
    }
}

impl CalendarSource for GoogleCalendarSource {
    fn fetch_events(
        &mut self,
        calendar_id: &str,
    ) -> Result<Vec<CalendarEvent>, CalendarFetchError> {
        let token = self.access_token()?;
        let now = OffsetDateTime::now_utc();
        let time_min = (now - CALENDAR_QUERY_WINDOW)
            .format(&Rfc3339)
            .map_err(|error| CalendarFetchError::MissingData(error.to_string()))?;
        let time_max = (now + CALENDAR_QUERY_WINDOW)
            .format(&Rfc3339)
            .map_err(|error| CalendarFetchError::MissingData(error.to_string()))?;
        let url = format!("https://www.googleapis.com/calendar/v3/calendars/{calendar_id}/events");
        let body = self
            .client
            .get(&url)
            .bearer_auth(token)
            // `singleEvents=true` expands a recurring event into its
            // individual occurrences (default `false` would otherwise
            // return just the recurring series' first occurrence forever,
            // so a recurring "Focus" block would never retrigger after its
            // first occurrence); `orderBy=startTime` requires
            // `singleEvents=true` and guarantees a stable, chronological
            // order; `timeMin`/`timeMax` bracket "now" per
            // `CALENDAR_QUERY_WINDOW` so today's event is always within the
            // (unpaginated, well under the 250-result default) page
            // returned. See
            // https://developers.google.com/calendar/api/v3/reference/events/list.
            .query(&[
                ("singleEvents", "true"),
                ("orderBy", "startTime"),
                ("timeMin", time_min.as_str()),
                ("timeMax", time_max.as_str()),
            ])
            .send()
            .map_err(|error| CalendarFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| CalendarFetchError::MissingData(error.to_string()))?;
        parse_google_calendar_events_response(&body)
    }
}

/// Polls the configured calendar on a background thread, off the render
/// loop, same isolation the other `spawn_*_refresh_threads` give their
/// feeds. The HTTP fetch (`source.fetch_events`, up to ~20s across the
/// token request plus the events request at `CALENDAR_HTTP_TIMEOUT`) runs
/// outside the mutex, same pattern as `spawn_weather_refresh_thread` -
/// `state` is locked only briefly, to record the already-fetched result,
/// so the render loop and the power-reconciliation thread (which both lock
/// it every frame/tick) never block on the network call. Only logs the
/// resulting activation boolean and, on failure, the fetch error's
/// `Display` output - `calendar-event-cache` is classified `confidential`
/// in `spec.yaml`, so event titles never reach stderr.
fn spawn_screensaver_poll_thread(screensaver: Screensaver, state: Arc<Mutex<CalendarPollState>>) {
    std::thread::spawn(move || {
        let mut source = GoogleCalendarSource::new(screensaver.credential_ref.clone());
        loop {
            let now_utc = OffsetDateTime::now_utc();
            let active = match source.fetch_events(&screensaver.calendar_id) {
                Ok(events) => {
                    let active = evaluate_screensaver_rules(&screensaver.rules, &events, now_utc);
                    let mut guard = state.lock().expect("calendar poll state mutex poisoned");
                    guard.record_success(active, now_utc);
                    guard.screensaver_active()
                }
                Err(error) => {
                    eprintln!("calendar fetch failed: {error}");
                    let mut guard = state.lock().expect("calendar poll state mutex poisoned");
                    guard.record_failure();
                    guard.screensaver_active()
                }
            };
            eprintln!("calendar check: screensaver active: {active}");
            std::thread::sleep(CALENDAR_POLL_INTERVAL);
        }
    });
}

struct OpenMeteoWeatherSource {
    client: reqwest::blocking::Client,
    latitude: f64,
    longitude: f64,
}

impl OpenMeteoWeatherSource {
    fn new(latitude: f64, longitude: f64) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(WEATHER_HTTP_TIMEOUT)
            .build()
            .expect("reqwest client must build");
        Self {
            client,
            latitude,
            longitude,
        }
    }
}

impl WeatherSource for OpenMeteoWeatherSource {
    fn fetch(&mut self) -> Result<WeatherReading, WeatherFetchError> {
        let url = format!(
            "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m&hourly=precipitation_probability&forecast_hours=2&timezone=UTC",
            self.latitude, self.longitude
        );
        let body = self
            .client
            .get(&url)
            .send()
            .map_err(|error| WeatherFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| WeatherFetchError::MissingData(error.to_string()))?;
        parse_open_meteo_response(&body)
    }
}

/// Fetches weather on a background thread, off the render loop - the spec
/// requires fetching data outside the render loop. The render loop only
/// ever reads the shared store, never blocks on HTTP.
fn spawn_weather_refresh_thread(
    mut source: OpenMeteoWeatherSource,
    store: Arc<Mutex<WeatherSnapshotStore>>,
) {
    std::thread::spawn(move || {
        loop {
            match source.fetch() {
                Ok(reading) => {
                    let mut guard = store.lock().expect("weather store mutex poisoned");
                    guard.record_success(reading, OffsetDateTime::now_utc());
                }
                Err(error) => eprintln!("weather fetch failed: {error}"),
            }
            std::thread::sleep(WEATHER_REFRESH_INTERVAL);
        }
    });
}

struct SlDeparturesSource {
    client: reqwest::blocking::Client,
    time_zone: String,
}

impl SlDeparturesSource {
    fn new(time_zone: String) -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(SL_HTTP_TIMEOUT)
            .build()
            .expect("reqwest client must build");
        Self { client, time_zone }
    }
}

impl DeparturesSource for SlDeparturesSource {
    fn fetch_departures(&mut self, site_id: u32) -> Result<Vec<RawDeparture>, DepartureFetchError> {
        let url = format!("https://transport.integration.sl.se/v1/sites/{site_id}/departures");
        let body = self
            .client
            .get(&url)
            .send()
            .map_err(|error| DepartureFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| DepartureFetchError::MissingData(error.to_string()))?;
        parse_sl_departures_response(&body, &self.time_zone)
    }
}

struct SlDeviationsSource {
    client: reqwest::blocking::Client,
}

impl SlDeviationsSource {
    fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(SL_HTTP_TIMEOUT)
            .build()
            .expect("reqwest client must build");
        Self { client }
    }
}

impl DeviationsSource for SlDeviationsSource {
    fn fetch_deviations(&mut self, site_id: u32) -> Result<Vec<RawDeviation>, DepartureFetchError> {
        let url = format!("https://deviations.integration.sl.se/v1/messages?site={site_id}");
        let body = self
            .client
            .get(&url)
            .send()
            .map_err(|error| DepartureFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| DepartureFetchError::MissingData(error.to_string()))?;
        parse_sl_deviations_response(&body)
    }
}

/// Fetches departures/deviations on background threads, off the render
/// loop, once per distinct site (per the spec's "fetch once per distinct
/// site, filter locally" - several configured stations can share one
/// `site_id` under different modes/directions).
fn spawn_departures_refresh_threads(
    board_config: &BoardConfig,
    board: Arc<Mutex<DeparturesBoard>>,
) {
    let site_ids: HashSet<u32> = board_config
        .stations
        .iter()
        .map(|station| station.site_id)
        .collect();
    let time_zone = board_config.time_zone.clone();

    for site_id in site_ids.clone() {
        let board = Arc::clone(&board);
        let mut source = SlDeparturesSource::new(time_zone.clone());
        std::thread::spawn(move || {
            loop {
                match source.fetch_departures(site_id) {
                    Ok(departures) => {
                        let mut guard = board.lock().expect("departures board mutex poisoned");
                        guard.record_departures_success(
                            site_id,
                            departures,
                            OffsetDateTime::now_utc(),
                        );
                    }
                    Err(error) => eprintln!("departures fetch failed for site {site_id}: {error}"),
                }
                std::thread::sleep(DEPARTURES_REFRESH_INTERVAL);
            }
        });
    }

    for site_id in site_ids {
        let board = Arc::clone(&board);
        let mut source = SlDeviationsSource::new();
        std::thread::spawn(move || {
            loop {
                match source.fetch_deviations(site_id) {
                    Ok(deviations) => {
                        let mut guard = board.lock().expect("departures board mutex poisoned");
                        guard.record_deviations_success(site_id, deviations);
                    }
                    Err(error) => eprintln!("deviations fetch failed for site {site_id}: {error}"),
                }
                std::thread::sleep(DEVIATIONS_REFRESH_INTERVAL);
            }
        });
    }
}

/// Reads football-data.org's API key from the environment rather than
/// `board.toml` - per the spec, "provision all provider credentials...
/// separately on the Pi, never in the repo." Missing/invalid credentials
/// fail this one competition's fetch with a typed error (surfacing as
/// `StandingsPanelState::Unavailable`), never a panic.
struct FootballDataStandingsSource {
    client: reqwest::blocking::Client,
    api_key: Result<String, StandingsFetchError>,
}

impl FootballDataStandingsSource {
    fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(STANDINGS_HTTP_TIMEOUT)
            .build()
            .expect("reqwest client must build");
        let api_key = std::env::var("FOOTBALL_DATA_ORG_API_KEY").map_err(|_| {
            StandingsFetchError::MissingData("FOOTBALL_DATA_ORG_API_KEY is not set".to_string())
        });
        Self { client, api_key }
    }
}

impl StandingsSource for FootballDataStandingsSource {
    fn fetch_standings(
        &mut self,
        competition_code: &str,
    ) -> Result<Vec<StandingsRow>, StandingsFetchError> {
        let api_key = self.api_key.clone()?;
        // https://docs.football-data.org/general/v4/competition.html
        // `GET /v4/competitions/{id}/standings`, auth header
        // `X-Auth-Token`, free tier 10 req/min - unverified live.
        let url =
            format!("https://api.football-data.org/v4/competitions/{competition_code}/standings");
        let body = self
            .client
            .get(&url)
            .header("X-Auth-Token", api_key)
            .send()
            .map_err(|error| StandingsFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| StandingsFetchError::MissingData(error.to_string()))?;
        parse_football_data_standings_response(&body)
    }
}

/// Reads Highlightly's API key from the environment, same reasoning as
/// `FootballDataStandingsSource`.
struct HighlightlyStandingsSource {
    client: reqwest::blocking::Client,
    api_key: Result<String, StandingsFetchError>,
}

impl HighlightlyStandingsSource {
    fn new() -> Self {
        let client = reqwest::blocking::Client::builder()
            .timeout(STANDINGS_HTTP_TIMEOUT)
            .build()
            .expect("reqwest client must build");
        let api_key = std::env::var("HIGHLIGHTLY_API_KEY").map_err(|_| {
            StandingsFetchError::MissingData("HIGHLIGHTLY_API_KEY is not set".to_string())
        });
        Self { client, api_key }
    }
}

impl HighlightlySeasonSource for HighlightlyStandingsSource {
    fn resolve_current_season(&mut self, league_id: &str) -> Result<String, StandingsFetchError> {
        let api_key = self.api_key.clone()?;
        // https://highlightly.net/football-api/documentation/standings/
        // (the league-lookup side of the same API) - `GET
        // https://soccer.highlightly.net/leagues/{id}`, auth header
        // `x-rapidapi-key` - unverified live. Resolves the real current
        // season instead of guessing from the calendar, which is
        // silently wrong for part of the Allsvenskan season (see the
        // removed `highlightly_season_guess`).
        let url = format!("https://soccer.highlightly.net/leagues/{league_id}");
        let body = self
            .client
            .get(&url)
            .header("x-rapidapi-key", api_key)
            .send()
            .map_err(|error| StandingsFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| StandingsFetchError::MissingData(error.to_string()))?;
        parse_highlightly_current_season_response(&body)
    }

    fn fetch_standings_for_season(
        &mut self,
        league_id: &str,
        season: &str,
    ) -> Result<Vec<StandingsRow>, StandingsFetchError> {
        let api_key = self.api_key.clone()?;
        // https://highlightly.net/football-api/documentation/standings/
        // `GET https://soccer.highlightly.net/standings?leagueId=&season=`,
        // auth header `x-rapidapi-key` - unverified live. `leagueId` is
        // Highlightly's own numeric id (`board_config.rs`'s
        // `highlightly_league_id`), not football-data.org's alpha code.
        let url = format!(
            "https://soccer.highlightly.net/standings?leagueId={league_id}&season={season}"
        );
        let body = self
            .client
            .get(&url)
            .header("x-rapidapi-key", api_key)
            .send()
            .map_err(|error| StandingsFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| StandingsFetchError::MissingData(error.to_string()))?;
        parse_highlightly_standings_response(&body)
    }
}

impl StandingsSource for HighlightlyStandingsSource {
    fn fetch_standings(
        &mut self,
        league_id: &str,
    ) -> Result<Vec<StandingsRow>, StandingsFetchError> {
        // Resolves the current season once per call (i.e. once per
        // hourly refresh cycle - this is driven from
        // `spawn_standings_refresh_threads`'s loop, never per render
        // frame), then fetches that season's table. Two calls/hour is
        // 48/day, well within Highlightly's 100 req/day free-tier limit.
        fetch_highlightly_standings(self, league_id)
    }
}

/// Fetches each configured competition's table on its own background
/// thread, off the render loop, hourly per the spec - one competition
/// failing (e.g. a missing API key) never blocks or blanks the other's
/// panel, same isolation `spawn_departures_refresh_threads` gives per site.
fn spawn_standings_refresh_threads(board_config: &BoardConfig, board: Arc<Mutex<StandingsBoard>>) {
    for competition in &board_config.competitions {
        let board = Arc::clone(&board);
        let competition_id = competition.id.clone();
        let provider = competition.provider;
        // football-data.org identifies a competition by its alpha code
        // (`competition_code`); Highlightly requires its own numeric
        // `leagueId` instead (`highlightly_league_id`) - there is no
        // alpha-code scheme on Highlightly's side, so the two providers'
        // ids are never interchangeable. `board_config.rs` already
        // guarantees `highlightly_league_id` is present when
        // `provider = "highlightly"`.
        let provider_request_id = match provider {
            FootballProvider::FootballDataOrg => competition.competition_code.clone(),
            FootballProvider::Highlightly => competition
                .highlightly_league_id
                .expect("board_config.rs guarantees highlightly_league_id for highlightly")
                .to_string(),
        };
        std::thread::spawn(move || {
            let mut source: Box<dyn StandingsSource> = match provider {
                FootballProvider::FootballDataOrg => Box::new(FootballDataStandingsSource::new()),
                FootballProvider::Highlightly => Box::new(HighlightlyStandingsSource::new()),
            };
            loop {
                match source.fetch_standings(&provider_request_id) {
                    Ok(rows) => {
                        let mut guard = board.lock().expect("standings board mutex poisoned");
                        guard.record_success(&competition_id, rows, OffsetDateTime::now_utc());
                    }
                    Err(error) => {
                        eprintln!(
                            "standings fetch failed for competition {competition_id}: {error}"
                        )
                    }
                }
                std::thread::sleep(STANDINGS_REFRESH_INTERVAL);
            }
        });
    }
}

/// Reconciles display power on a background thread, off the render loop,
/// same isolation the other `spawn_*_refresh_threads` give their feeds.
/// `override_active` has no real trigger yet (PI-08's override file) - this
/// loop already reads it as a fixed snapshot per tick and passes that
/// single snapshot into `reconcile`, so wiring in the real source later is
/// a read at the top of this loop, not a reshape of the reconciliation
/// call itself. `screensaver_active` is read from the same
/// `CalendarPollState` the render loop reads, so both sides of the
/// standby-overrides-screensaver priority agree on the same snapshot
/// source (never a literal `false`, now that PI-07 exists). Only calls
/// `set_power` when the resolved power actually changes, to avoid spamming
/// the (currently null) backlight backend every tick.
fn spawn_display_power_reconciliation_thread(
    board_config: &BoardConfig,
    calendar_poll_state: Arc<Mutex<CalendarPollState>>,
    mut power_control: impl DisplayPowerControl + Send + 'static,
) {
    let time_zone = board_config.time_zone.clone();
    let display_windows = board_config.display_windows.clone();
    std::thread::spawn(move || {
        let mut last_power: Option<bool> = None;
        loop {
            let now_utc = OffsetDateTime::now_utc();
            let override_active = active_override_power(
                remote_control::read_override_state(Path::new(OVERRIDE_FILE_PATH)),
                now_utc,
            );
            let screensaver_active = calendar_poll_state
                .lock()
                .expect("calendar poll state mutex poisoned")
                .screensaver_active();

            match is_within_display_window(&display_windows, &time_zone, now_utc) {
                Ok(schedule_says_awake) => {
                    let state = reconcile(schedule_says_awake, override_active, screensaver_active);
                    let power_on = resolved_power(state);
                    if last_power != Some(power_on) {
                        if let Err(error) = power_control.set_power(power_on) {
                            eprintln!("display power control failed: {error}");
                        } else {
                            last_power = Some(power_on);
                        }
                    }
                }
                Err(error) => eprintln!("display-window evaluation failed: {error}"),
            }

            std::thread::sleep(DISPLAY_POWER_RECONCILE_INTERVAL);
        }
    });
}

/// Runs the remote-control HTTP server on its own OS thread, with its own
/// dedicated `tokio` runtime confined to that thread - `axum`/`tokio` are a
/// new async-runtime addition for this binary (everything else here uses
/// `macroquad`'s own executor for the render loop and plain
/// `std::thread`/`reqwest::blocking` for the background feeds). Keeping
/// `tokio` isolated to this one thread means the render loop and the other
/// background threads are completely unaffected by it; if the server
/// thread's runtime ever panics, every other part of the dashboard keeps
/// running.
fn spawn_remote_control_server(state: remote_control::RemoteControlState) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("remote-control server runtime failed to start: {error}");
                return;
            }
        };
        runtime.block_on(async move {
            let router = remote_control::remote_control_router(state);
            match tokio::net::TcpListener::bind(REMOTE_CONTROL_BIND_ADDRESS).await {
                Ok(listener) => {
                    if let Err(error) = axum::serve(listener, router).await {
                        eprintln!("remote-control server failed: {error}");
                    }
                }
                Err(error) => {
                    eprintln!("remote-control server failed to bind: {error}");
                }
            }
        });
    });
}

#[macroquad::main("pi-dashboard")]
async fn main() {
    // reqwest's `rustls-no-provider` feature (see dashboard-pi/Cargo.toml -
    // avoids aws-lc-sys's non-optional `cmake` build-dependency, which the
    // default `cross-rs` aarch64 image doesn't ship) ships no crypto
    // provider of its own: install `ring`'s before any reqwest client is
    // built, exactly as reqwest's own tls.rs docs instruct, or every
    // reqwest::blocking::Client::builder() call below panics at construction.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls ring crypto provider");

    let board_config = match load_board_config(Path::new("board.toml")) {
        Ok(board_config) => board_config,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    let calendar_poll_state = Arc::new(Mutex::new(CalendarPollState::new()));
    if let Some(screensaver) = board_config.screensaver.clone() {
        spawn_screensaver_poll_thread(screensaver, Arc::clone(&calendar_poll_state));
    }

    spawn_display_power_reconciliation_thread(
        &board_config,
        Arc::clone(&calendar_poll_state),
        NullDisplayPowerControl::new(),
    );

    let weather_store = Arc::new(Mutex::new(WeatherSnapshotStore::new()));
    let weather_source = OpenMeteoWeatherSource::new(board_config.latitude, board_config.longitude);
    spawn_weather_refresh_thread(weather_source, Arc::clone(&weather_store));

    let departures_board = Arc::new(Mutex::new(DeparturesBoard::new()));
    spawn_departures_refresh_threads(&board_config, Arc::clone(&departures_board));

    let standings_board = Arc::new(Mutex::new(StandingsBoard::new()));
    spawn_standings_refresh_threads(&board_config, Arc::clone(&standings_board));
    let standings_rotation = StandingsRotation::new(
        board_config
            .competitions
            .iter()
            .map(|competition| competition.id.clone())
            .collect(),
        board_config
            .panel_rotation
            .as_ref()
            .map(|rotation| rotation.interval_seconds)
            .unwrap_or(STANDINGS_ROTATION_DEFAULT_INTERVAL_SECONDS),
    );

    match std::env::var(REMOTE_CONTROL_TOKEN_ENV_VAR) {
        Ok(expected_token) => {
            spawn_remote_control_server(remote_control::RemoteControlState {
                expected_token: Arc::new(expected_token),
                override_path: Arc::new(Path::new(OVERRIDE_FILE_PATH).to_path_buf()),
                time_zone: Arc::new(board_config.time_zone.clone()),
                display_windows: Arc::new(board_config.display_windows.clone()),
                stations: Arc::new(board_config.stations.clone()),
                competition_ids: Arc::new(
                    board_config
                        .competitions
                        .iter()
                        .map(|competition| competition.id.clone())
                        .collect(),
                ),
                weather_store: Arc::clone(&weather_store),
                departures_board: Arc::clone(&departures_board),
                standings_board: Arc::clone(&standings_board),
                calendar_poll_state: Arc::clone(&calendar_poll_state),
                started_at: std::time::Instant::now(),
            });
        }
        Err(_) => {
            eprintln!("{REMOTE_CONTROL_TOKEN_ENV_VAR} is not set: remote-control server disabled")
        }
    }

    let mut daylight_date: Option<Date> = None;
    let mut daylight = None;

    loop {
        let now_utc = OffsetDateTime::now_utc();

        macroquad::window::clear_background(macroquad::color::BLACK);

        match local_clock(&board_config.time_zone, now_utc) {
            Ok(clock) => {
                // Mirrors `spawn_display_power_reconciliation_thread`'s own
                // `reconcile` call: same inputs, read independently here so
                // the render loop's "show the screensaver or the normal
                // layout" decision never depends on the power-reconciler
                // thread's timing.
                let screensaver_active = calendar_poll_state
                    .lock()
                    .expect("calendar poll state mutex poisoned")
                    .screensaver_active();
                let schedule_says_awake = is_within_display_window(
                    &board_config.display_windows,
                    &board_config.time_zone,
                    now_utc,
                )
                .inspect_err(|error| eprintln!("display-window evaluation failed: {error}"))
                .unwrap_or(true);
                let override_active = active_override_power(
                    remote_control::read_override_state(Path::new(OVERRIDE_FILE_PATH)),
                    now_utc,
                );
                let display_state =
                    reconcile(schedule_says_awake, override_active, screensaver_active);

                if display_state == DisplayState::Screensaver {
                    let availability = calendar_poll_state
                        .lock()
                        .expect("calendar poll state mutex poisoned")
                        .availability(now_utc);
                    render_screensaver_view(&clock, &availability);
                    macroquad::window::next_frame().await;
                    continue;
                }

                let weather_state = weather_store
                    .lock()
                    .expect("weather store mutex poisoned")
                    .state(now_utc);
                render_clock_weather_header(&clock, &weather_state);

                match Date::new(clock.year, clock.month as i8, clock.day as i8) {
                    Ok(today) => {
                        if daylight_date != Some(today) {
                            daylight = daylight_window(
                                &board_config.time_zone,
                                board_config.latitude,
                                board_config.longitude,
                                today,
                            )
                            .inspect_err(|error| eprintln!("daylight computation failed: {error}"))
                            .ok();
                            daylight_date = Some(today);
                        }
                        if let Some(window) = &daylight {
                            render_daylight_strip(window);
                        }
                    }
                    Err(error) => eprintln!("local date computation failed: {error}"),
                }

                let mut panel_top = 140.0;
                {
                    let board = departures_board
                        .lock()
                        .expect("departures board mutex poisoned");
                    for station in &board_config.stations {
                        let state = board.station_state(station, now_utc);
                        render_departures_panel(&state, panel_top);
                        panel_top += STATION_PANEL_HEIGHT;
                    }
                }

                let active_competition_id = standings_rotation.active_competition(now_utc);
                let standings_state = standings_board
                    .lock()
                    .expect("standings board mutex poisoned")
                    .competition_state(active_competition_id, now_utc);
                render_standings_panel(active_competition_id, &standings_state, panel_top);
            }
            Err(error) => eprintln!("clock computation failed: {error}"),
        }

        macroquad::window::next_frame().await;
    }
}
