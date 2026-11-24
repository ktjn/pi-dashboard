use dashboard_core::board_config::{BoardConfig, FootballProvider, load_board_config};
use dashboard_core::clock::local_clock;
use dashboard_core::daylight::daylight_window;
use dashboard_core::departures::{
    DepartureFetchError, DeparturesBoard, DeparturesSource, DeviationsSource, RawDeparture,
    RawDeviation, parse_sl_departures_response, parse_sl_deviations_response,
};
use dashboard_core::render::{
    STATION_PANEL_HEIGHT, render_clock_weather_header, render_daylight_strip,
    render_departures_panel, render_standings_panel,
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
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;
use time::OffsetDateTime;

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

#[macroquad::main("pi-dashboard")]
async fn main() {
    let board_config = match load_board_config(Path::new("board.toml")) {
        Ok(board_config) => board_config,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

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

    let mut daylight_date: Option<Date> = None;
    let mut daylight = None;

    loop {
        let now_utc = OffsetDateTime::now_utc();

        macroquad::window::clear_background(macroquad::color::BLACK);

        match local_clock(&board_config.time_zone, now_utc) {
            Ok(clock) => {
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
