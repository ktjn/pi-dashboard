use dashboard_core::board_config::{BoardConfig, FootballProvider, Screensaver, load_board_config};
use dashboard_core::clock::local_clock;
use dashboard_core::daylight::daylight_window;
use dashboard_core::departures::{
    DepartureFetchError, DeparturesBoard, DeparturesPanelState, DeparturesSource, DeviationsSource,
    RawDeparture, RawDeviation, parse_sl_departures_response, parse_sl_deviations_response,
};
use dashboard_core::render::{
    StationLayoutInput, compute_dashboard_layout, hit_test, render_calendar_availability_note,
    render_clock_weather_header, render_daylight_strip_at, render_departures_panel_with_header,
    render_screensaver_view, render_standings_panel,
};
use dashboard_core::schedule::{
    DisplayPowerControl, DisplayState, OverrideState, active_override_power,
    is_within_display_window, reconcile, resolved_power,
};
use dashboard_core::screensaver::{
    CalendarEvent, CalendarFetchError, CalendarPollState, CalendarSource,
    evaluate_screensaver_rules, parse_google_calendar_events_response,
};
use dashboard_core::standings::{
    HighlightlySeasonSource, StandingsBoard, StandingsFetchError, StandingsPanelState,
    StandingsRotation, StandingsRow, StandingsSource, fetch_highlightly_standings,
    parse_football_data_standings_response, parse_highlightly_current_season_response,
    parse_highlightly_standings_response,
};
use dashboard_core::theme::{Theme, load_fonts, palette};
use dashboard_core::touch::{
    DisplayContext, TouchConfig, TouchEffect, TouchMode, TouchState, TouchTarget, apply_touch,
    revert_idle,
};
use dashboard_core::update_agent::{
    ActivationDecision, DigestOutcome, PollEffect, ReleaseAssets, UpdateAgentState,
    decide_activation, evaluate_candidate, parse_github_latest_release, parse_sha256sum_digest,
    stage_if_digest_matches,
};
use dashboard_core::weather::{
    WeatherFetchError, WeatherPanelState, WeatherReading, WeatherSnapshotStore, WeatherSource,
    parse_open_meteo_response,
};
use dashboard_pi::cache::DashboardCache;
use dashboard_pi::power::WakeSignal;
use dashboard_pi::{atomic_fs, cache, power, remote_control, update_agent};
use jiff::civil::Date;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const WEATHER_REFRESH_INTERVAL: StdDuration = StdDuration::from_secs(15 * 60);
const WEATHER_HTTP_TIMEOUT: StdDuration = StdDuration::from_secs(5);
const DEPARTURES_REFRESH_INTERVAL: StdDuration = StdDuration::from_secs(30);
const DEVIATIONS_REFRESH_INTERVAL: StdDuration = StdDuration::from_secs(120);
const SL_HTTP_TIMEOUT: StdDuration = StdDuration::from_secs(5);
const STANDINGS_REFRESH_INTERVAL: StdDuration = StdDuration::from_secs(60 * 60);
const STANDINGS_HTTP_TIMEOUT: StdDuration = StdDuration::from_secs(10);
const STANDINGS_ROTATION_DEFAULT_INTERVAL_SECONDS: u32 = 15;
const DISPLAY_POWER_RECONCILE_INTERVAL: StdDuration = StdDuration::from_secs(60);
const CALENDAR_POLL_INTERVAL: StdDuration = StdDuration::from_secs(60);
const CALENDAR_HTTP_TIMEOUT: StdDuration = StdDuration::from_secs(10);
const CALENDAR_TOKEN_SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly";
const CALENDAR_TOKEN_LIFETIME_SECONDS: i64 = 3600;
const CALENDAR_TOKEN_REFRESH_SKEW_SECONDS: i64 = 30;
const CALENDAR_QUERY_WINDOW: time::Duration = time::Duration::hours(1);
const BOARD_CONFIG_PATH_ENV_VAR: &str = "BOARD_CONFIG_PATH";
const DEFAULT_BOARD_CONFIG_PATH: &str = "board.toml";
const OVERRIDE_FILE_PATH_ENV_VAR: &str = "OVERRIDE_FILE_PATH";
const DEFAULT_OVERRIDE_FILE_PATH: &str = "override-state.json";
const TOUCH_WAKE_FILE_PATH_ENV_VAR: &str = "TOUCH_WAKE_FILE_PATH";
const DEFAULT_TOUCH_WAKE_FILE_PATH: &str = "touch-wake-until.txt";
const DASHBOARD_CACHE_PATH_ENV_VAR: &str = "DASHBOARD_CACHE_PATH";
const DEFAULT_DASHBOARD_CACHE_PATH: &str = "dashboard-cache.json";
const REMOTE_CONTROL_TOKEN_ENV_VAR: &str = "REMOTE_CONTROL_TOKEN";
const REMOTE_CONTROL_BIND_ADDRESS: &str = "0.0.0.0:8080";
const UPDATE_AGENT_POLL_INTERVAL: StdDuration = StdDuration::from_secs(30 * 60);
const UPDATE_AGENT_HTTP_TIMEOUT: StdDuration = StdDuration::from_secs(30);
const UPDATE_AGENT_LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/ktjn/pi-dashboard/releases/latest";
const STAGED_VERSION_FILE_PATH: &str = "staged-version.txt";
const STAGING_DIR_PATH: &str = "staging";

#[derive(Clone)]
struct CachePersister {
    path: Arc<PathBuf>,
    save_lock: Arc<Mutex<()>>,
    weather_store: Arc<Mutex<WeatherSnapshotStore>>,
    departures_board: Arc<Mutex<DeparturesBoard>>,
    standings_board: Arc<Mutex<StandingsBoard>>,
    calendar_poll_state: Arc<Mutex<CalendarPollState>>,
}

impl CachePersister {
    fn save(&self) {
        let _save_guard = self.save_lock.lock().expect("cache save mutex poisoned");
        let weather = self
            .weather_store
            .lock()
            .expect("weather store mutex poisoned")
            .export_snapshot();
        let departures = self
            .departures_board
            .lock()
            .expect("departures board mutex poisoned")
            .export_snapshot();
        let standings = self
            .standings_board
            .lock()
            .expect("standings board mutex poisoned")
            .export_snapshot();
        let calendar = self
            .calendar_poll_state
            .lock()
            .expect("calendar poll state mutex poisoned")
            .export_snapshot();

        let snapshot = DashboardCache {
            weather,
            departures,
            standings,
            calendar,
        };
        if let Err(error) = cache::save_cache_atomic(&self.path, &snapshot) {
            eprintln!("dashboard cache save failed: {error}");
        }
    }
}

fn percent_encode_path_segment(segment: &str) -> String {
    let mut encoded = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char);
            }
            other => {
                use std::fmt::Write;
                let _ = write!(&mut encoded, "%{other:02X}");
            }
        }
    }
    encoded
}

fn parse_remote_control_token(raw: Option<String>) -> Option<String> {
    raw.filter(|token| !token.trim().is_empty())
}

fn effective_display_power(
    display_state: DisplayState,
    touch_state: &TouchState,
    now_utc: OffsetDateTime,
) -> bool {
    let touch_wake_active =
        touch_state.mode == TouchMode::Wake && touch_state.wake_until.is_some_and(|u| now_utc < u);
    resolved_power(display_state) || touch_wake_active
}

fn sync_touch_wake_file(previous_mode: TouchMode, next: &TouchState, touch_wake_path: &Path) {
    if next.mode == TouchMode::Wake {
        if previous_mode != TouchMode::Wake
            && let Some(until) = next.wake_until
            && let Err(error) = power::write_touch_wake_until_atomic(touch_wake_path, until)
        {
            eprintln!("touch-wake file write failed: {error}");
        }
    } else if previous_mode == TouchMode::Wake
        && let Err(error) = power::clear_touch_wake_file(touch_wake_path)
    {
        eprintln!("touch-wake file clear failed: {error}");
    }
}

fn notify_all_departures(departures_wakes: &HashMap<u32, WakeSignal>) {
    for signal in departures_wakes.values() {
        signal.notify();
    }
}

fn dispatch_touch_effects(
    effects: &[TouchEffect],
    power_wake: &WakeSignal,
    weather_wake: &WakeSignal,
    departures_wakes: &HashMap<u32, WakeSignal>,
    standings_wakes: &HashMap<String, WakeSignal>,
) {
    for effect in effects {
        match effect {
            TouchEffect::WakeDisplay => power_wake.notify(),
            TouchEffect::RefreshDeparturesRequested => notify_all_departures(departures_wakes),
            TouchEffect::DismissScreensaverView => {}
            TouchEffect::RequestPanelRefetch { provider } => {
                if provider == "weather" {
                    weather_wake.notify();
                } else if provider == "departures" {
                    notify_all_departures(departures_wakes);
                } else if let Some(site_str) = provider.strip_prefix("departures:") {
                    if let Ok(site_id) = site_str.parse::<u32>()
                        && let Some(signal) = departures_wakes.get(&site_id)
                    {
                        signal.notify();
                    }
                } else if provider == "standings" {
                    for signal in standings_wakes.values() {
                        signal.notify();
                    }
                } else if let Some(competition_id) = provider.strip_prefix("standings:")
                    && let Some(signal) = standings_wakes.get(competition_id)
                {
                    signal.notify();
                }
            }
        }
    }
}

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

fn load_service_account_key(credential_ref: &str) -> Result<ServiceAccountKey, CalendarFetchError> {
    let path = std::env::var(credential_ref)
        .map_err(|_| CalendarFetchError::MissingData(format!("{credential_ref} is not set")))?;
    let contents = std::fs::read_to_string(&path)
        .map_err(|error| CalendarFetchError::MissingData(format!("{path}: {error}")))?;
    serde_json::from_str(&contents)
        .map_err(|error| CalendarFetchError::InvalidJson(error.to_string()))
}

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
            .error_for_status()
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
        let encoded_calendar_id = percent_encode_path_segment(calendar_id);
        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/{encoded_calendar_id}/events"
        );
        let body = self
            .client
            .get(&url)
            .bearer_auth(token)
            .query(&[
                ("singleEvents", "true"),
                ("orderBy", "startTime"),
                ("timeMin", time_min.as_str()),
                ("timeMax", time_max.as_str()),
            ])
            .send()
            .map_err(|error| CalendarFetchError::MissingData(error.to_string()))?
            .error_for_status()
            .map_err(|error| CalendarFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| CalendarFetchError::MissingData(error.to_string()))?;
        parse_google_calendar_events_response(&body)
    }
}

fn spawn_screensaver_poll_thread(
    screensaver: Screensaver,
    state: Arc<Mutex<CalendarPollState>>,
    cache_persister: CachePersister,
) {
    std::thread::spawn(move || {
        let mut source = GoogleCalendarSource::new(screensaver.credential_ref.clone());
        loop {
            let now_utc = OffsetDateTime::now_utc();
            let (active, succeeded) = match source.fetch_events(&screensaver.calendar_id) {
                Ok(events) => {
                    let active = evaluate_screensaver_rules(&screensaver.rules, &events, now_utc);
                    let mut guard = state.lock().expect("calendar poll state mutex poisoned");
                    guard.record_success(active, now_utc);
                    (guard.screensaver_active(), true)
                }
                Err(error) => {
                    eprintln!("calendar fetch failed: {error}");
                    let mut guard = state.lock().expect("calendar poll state mutex poisoned");
                    guard.record_failure();
                    (guard.screensaver_active(), false)
                }
            };
            if succeeded {
                cache_persister.save();
            }
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
            .error_for_status()
            .map_err(|error| WeatherFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| WeatherFetchError::MissingData(error.to_string()))?;
        parse_open_meteo_response(&body)
    }
}

fn spawn_weather_refresh_thread(
    mut source: OpenMeteoWeatherSource,
    store: Arc<Mutex<WeatherSnapshotStore>>,
    wake_signal: WakeSignal,
    cache_persister: CachePersister,
) {
    std::thread::spawn(move || {
        loop {
            match source.fetch() {
                Ok(reading) => {
                    {
                        let mut guard = store.lock().expect("weather store mutex poisoned");
                        guard.record_success(reading, OffsetDateTime::now_utc());
                    }
                    cache_persister.save();
                }
                Err(error) => eprintln!("weather fetch failed: {error}"),
            }
            wake_signal.wait_timeout(WEATHER_REFRESH_INTERVAL);
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn process_update_release<F>(
    state: &UpdateAgentState,
    handed_off_version: &mut Option<String>,
    release: ReleaseAssets,
    now_utc: OffsetDateTime,
    schedule_awake: bool,
    staging_dir: &Path,
    staged_version_path: &Path,
    mut fetch_release_artifacts: F,
) -> Result<UpdateAgentState, String>
where
    F: FnMut(&str, &str) -> Result<(String, Vec<u8>), String>,
{
    let (mut next, effect) = evaluate_candidate(state, release, now_utc);
    if let PollEffect::DownloadRequested {
        bundle_url,
        digest_url,
        ..
    } = effect
    {
        let (published_digest, bundle_bytes) = fetch_release_artifacts(&digest_url, &bundle_url)?;
        let downloaded_digest = update_agent::sha256_hex(&bundle_bytes);

        let (staged, outcome) =
            stage_if_digest_matches(&next, &published_digest, &downloaded_digest);
        next = staged;
        if outcome == DigestOutcome::Mismatch {
            eprintln!("update agent: digest mismatch for candidate, dropping it");
            return Ok(next);
        }

        if let Some(version) = next.candidate_version.as_deref() {
            update_agent::stage_bundle_atomic(staging_dir, version, &bundle_bytes)
                .map_err(|error| error.to_string())?;
        }
        *handed_off_version = None;
    }

    if decide_activation(&next, schedule_awake) == ActivationDecision::ActivateNow
        && let Some(version) = next.candidate_version.as_deref()
        && handed_off_version.as_deref() != Some(version)
    {
        match atomic_fs::write_bytes_atomic(staged_version_path, version.as_bytes()) {
            Ok(()) => {
                *handed_off_version = Some(version.to_string());
                eprintln!("update agent: staged {version} for activation");
            }
            Err(error) => eprintln!("update agent: failed to write staged-version file: {error}"),
        }
    }

    Ok(next)
}

fn poll_update_agent_once(
    client: &reqwest::blocking::Client,
    state: &UpdateAgentState,
    handed_off_version: &mut Option<String>,
    schedule_awake: bool,
) -> Result<UpdateAgentState, String> {
    let latest_release_json = client
        .get(UPDATE_AGENT_LATEST_RELEASE_URL)
        .header("User-Agent", "pi-dashboard-update-agent")
        .send()
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .text()
        .map_err(|error| error.to_string())?;
    let release =
        parse_github_latest_release(&latest_release_json).map_err(|error| error.to_string())?;

    process_update_release(
        state,
        handed_off_version,
        release,
        OffsetDateTime::now_utc(),
        schedule_awake,
        Path::new(STAGING_DIR_PATH),
        Path::new(STAGED_VERSION_FILE_PATH),
        |digest_url, bundle_url| {
            let published_digest_raw = client
                .get(digest_url)
                .header("User-Agent", "pi-dashboard-update-agent")
                .send()
                .map_err(|error| error.to_string())?
                .error_for_status()
                .map_err(|error| error.to_string())?
                .text()
                .map_err(|error| error.to_string())?;
            let published_digest =
                parse_sha256sum_digest(&published_digest_raw).map_err(|error| error.to_string())?;

            let bundle_response = client
                .get(bundle_url)
                .header("User-Agent", "pi-dashboard-update-agent")
                .send()
                .map_err(|error| error.to_string())?
                .error_for_status()
                .map_err(|error| error.to_string())?;
            let bundle_bytes =
                update_agent::read_bounded_bytes(bundle_response, update_agent::MAX_BUNDLE_BYTES)
                    .map_err(|error| error.to_string())?;
            Ok((published_digest, bundle_bytes))
        },
    )
}

fn spawn_update_agent_thread(
    time_zone: String,
    display_windows: Vec<dashboard_core::board_config::DisplayWindow>,
) {
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(UPDATE_AGENT_HTTP_TIMEOUT)
            .build()
            .expect("reqwest client must build");
        let mut state = UpdateAgentState::starting_at(env!("CARGO_PKG_VERSION"));
        let mut handed_off_version: Option<String> = None;
        loop {
            let now_utc = OffsetDateTime::now_utc();
            let schedule_awake =
                is_within_display_window(&display_windows, &time_zone, now_utc).unwrap_or(true);
            match poll_update_agent_once(&client, &state, &mut handed_off_version, schedule_awake) {
                Ok(next) => state = next,
                Err(error) => eprintln!("update agent: poll failed: {error}"),
            }
            std::thread::sleep(UPDATE_AGENT_POLL_INTERVAL);
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
            .error_for_status()
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
            .error_for_status()
            .map_err(|error| DepartureFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| DepartureFetchError::MissingData(error.to_string()))?;
        parse_sl_deviations_response(&body)
    }
}

fn spawn_departures_refresh_threads(
    board_config: &BoardConfig,
    board: Arc<Mutex<DeparturesBoard>>,
    wake_signals: &HashMap<u32, WakeSignal>,
    cache_persister: CachePersister,
) {
    let site_ids: HashSet<u32> = board_config
        .stations
        .iter()
        .map(|station| station.site_id)
        .collect();
    let time_zone = board_config.time_zone.clone();

    for site_id in site_ids.clone() {
        let board = Arc::clone(&board);
        let wake_signal = wake_signals
            .get(&site_id)
            .cloned()
            .unwrap_or_else(WakeSignal::new);
        let persister = cache_persister.clone();
        let mut source = SlDeparturesSource::new(time_zone.clone());
        std::thread::spawn(move || {
            loop {
                match source.fetch_departures(site_id) {
                    Ok(departures) => {
                        {
                            let mut guard = board.lock().expect("departures board mutex poisoned");
                            guard.record_departures_success(
                                site_id,
                                departures,
                                OffsetDateTime::now_utc(),
                            );
                        }
                        persister.save();
                    }
                    Err(error) => eprintln!("departures fetch failed for site {site_id}: {error}"),
                }
                wake_signal.wait_timeout(DEPARTURES_REFRESH_INTERVAL);
            }
        });
    }

    for site_id in site_ids {
        let board = Arc::clone(&board);
        let persister = cache_persister.clone();
        let mut source = SlDeviationsSource::new();
        std::thread::spawn(move || {
            loop {
                match source.fetch_deviations(site_id) {
                    Ok(deviations) => {
                        {
                            let mut guard = board.lock().expect("departures board mutex poisoned");
                            guard.record_deviations_success_at(
                                site_id,
                                deviations,
                                OffsetDateTime::now_utc(),
                            );
                        }
                        persister.save();
                    }
                    Err(error) => eprintln!("deviations fetch failed for site {site_id}: {error}"),
                }
                std::thread::sleep(DEVIATIONS_REFRESH_INTERVAL);
            }
        });
    }
}

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
        let url =
            format!("https://api.football-data.org/v4/competitions/{competition_code}/standings");
        let body = self
            .client
            .get(&url)
            .header("X-Auth-Token", api_key)
            .send()
            .map_err(|error| StandingsFetchError::MissingData(error.to_string()))?
            .error_for_status()
            .map_err(|error| StandingsFetchError::MissingData(error.to_string()))?
            .text()
            .map_err(|error| StandingsFetchError::MissingData(error.to_string()))?;
        parse_football_data_standings_response(&body)
    }
}

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
        let url = format!("https://soccer.highlightly.net/leagues/{league_id}");
        let body = self
            .client
            .get(&url)
            .header("x-rapidapi-key", api_key)
            .send()
            .map_err(|error| StandingsFetchError::MissingData(error.to_string()))?
            .error_for_status()
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
        let url = format!(
            "https://soccer.highlightly.net/standings?leagueId={league_id}&season={season}"
        );
        let body = self
            .client
            .get(&url)
            .header("x-rapidapi-key", api_key)
            .send()
            .map_err(|error| StandingsFetchError::MissingData(error.to_string()))?
            .error_for_status()
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
        fetch_highlightly_standings(self, league_id)
    }
}

fn spawn_standings_refresh_threads(
    board_config: &BoardConfig,
    board: Arc<Mutex<StandingsBoard>>,
    wake_signals: &HashMap<String, WakeSignal>,
    cache_persister: CachePersister,
) {
    for competition in &board_config.competitions {
        let board = Arc::clone(&board);
        let competition_id = competition.id.clone();
        let provider = competition.provider;
        let wake_signal = wake_signals
            .get(&competition_id)
            .cloned()
            .unwrap_or_else(WakeSignal::new);
        let persister = cache_persister.clone();
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
                        {
                            let mut guard = board.lock().expect("standings board mutex poisoned");
                            guard.record_success(&competition_id, rows, OffsetDateTime::now_utc());
                        }
                        persister.save();
                    }
                    Err(error) => {
                        eprintln!(
                            "standings fetch failed for competition {competition_id}: {error}"
                        )
                    }
                }
                wake_signal.wait_timeout(STANDINGS_REFRESH_INTERVAL);
            }
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_display_power_reconciliation_thread(
    board_config: &BoardConfig,
    override_path: Arc<PathBuf>,
    calendar_poll_state: Arc<Mutex<CalendarPollState>>,
    override_cache: Arc<Mutex<Option<OverrideState>>>,
    touch_state: Arc<Mutex<TouchState>>,
    power_wake: WakeSignal,
    departures_wakes: HashMap<u32, WakeSignal>,
    mut power_control: Box<dyn DisplayPowerControl + Send>,
) {
    let time_zone = board_config.time_zone.clone();
    let display_windows = board_config.display_windows.clone();
    std::thread::spawn(move || {
        let mut last_power: Option<bool> = None;
        loop {
            let now_utc = OffsetDateTime::now_utc();
            let override_state = remote_control::read_override_state(&override_path);
            *override_cache
                .lock()
                .expect("override cache mutex poisoned") = override_state;
            let override_active = active_override_power(override_state, now_utc);
            let screensaver_active = calendar_poll_state
                .lock()
                .expect("calendar poll state mutex poisoned")
                .screensaver_active();
            let touch_snapshot = touch_state
                .lock()
                .expect("touch state mutex poisoned")
                .clone();

            match is_within_display_window(&display_windows, &time_zone, now_utc) {
                Ok(schedule_says_awake) => {
                    let state = reconcile(schedule_says_awake, override_active, screensaver_active);
                    let power_on = effective_display_power(state, &touch_snapshot, now_utc);
                    if last_power != Some(power_on) {
                        if let Err(error) = power_control.set_power(power_on) {
                            eprintln!("display power control failed: {error}");
                        } else {
                            if last_power == Some(false) && power_on {
                                notify_all_departures(&departures_wakes);
                            }
                            last_power = Some(power_on);
                        }
                    }
                }
                Err(error) => eprintln!("display-window evaluation failed: {error}"),
            }

            power_wake.wait_timeout(DISPLAY_POWER_RECONCILE_INTERVAL);
        }
    });
}

fn spawn_remote_control_server(state: remote_control::RemoteControlState) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_io()
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

fn collect_frame_taps() -> Vec<(f32, f32)> {
    let touch_points: Vec<(f32, f32)> = macroquad::input::touches()
        .into_iter()
        .filter(|touch| touch.phase == macroquad::input::TouchPhase::Started)
        .map(|touch| (touch.position.x, touch.position.y))
        .collect();
    if !touch_points.is_empty() {
        return touch_points;
    }
    if macroquad::input::is_mouse_button_pressed(macroquad::input::MouseButton::Left) {
        return vec![macroquad::input::mouse_position()];
    }
    Vec::new()
}

fn window_conf() -> macroquad::window::Conf {
    macroquad::window::Conf {
        window_title: "pi-dashboard".to_string(),
        window_width: 1200,
        window_height: 1920,
        fullscreen: std::env::var("PI_DASHBOARD_WINDOWED").is_err(),
        ..Default::default()
    }
}

#[macroquad::main(window_conf)]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls ring crypto provider");

    let board_config_path = power::env_path(BOARD_CONFIG_PATH_ENV_VAR, DEFAULT_BOARD_CONFIG_PATH);
    let override_path = Arc::new(power::env_path(
        OVERRIDE_FILE_PATH_ENV_VAR,
        DEFAULT_OVERRIDE_FILE_PATH,
    ));
    let touch_wake_path =
        power::env_path(TOUCH_WAKE_FILE_PATH_ENV_VAR, DEFAULT_TOUCH_WAKE_FILE_PATH);
    let _ = power::clear_touch_wake_file(&touch_wake_path);

    let board_config = match load_board_config(&board_config_path) {
        Ok(board_config) => board_config,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };

    let touch_config = board_config.touch.unwrap_or(TouchConfig {
        enabled: false,
        wake_seconds: TouchConfig::DEFAULT_WAKE_SECONDS,
        screensaver_dismiss_seconds: TouchConfig::DEFAULT_SCREENSAVER_DISMISS_SECONDS,
        idle_timeout_seconds: TouchConfig::DEFAULT_IDLE_TIMEOUT_SECONDS,
    });
    let touch_state = Arc::new(Mutex::new(TouchState::new()));
    let override_cache = Arc::new(Mutex::new(remote_control::read_override_state(
        &override_path,
    )));

    let weather_store = Arc::new(Mutex::new(WeatherSnapshotStore::new()));
    let departures_board = Arc::new(Mutex::new(DeparturesBoard::new()));
    let standings_board = Arc::new(Mutex::new(StandingsBoard::new()));
    let calendar_poll_state = Arc::new(Mutex::new(CalendarPollState::new()));

    let cache_path = power::env_path(DASHBOARD_CACHE_PATH_ENV_VAR, DEFAULT_DASHBOARD_CACHE_PATH);
    if let Some(cached) = cache::load_cache(&cache_path) {
        if let Some(weather_snapshot) = cached.weather {
            weather_store
                .lock()
                .expect("weather store mutex poisoned")
                .restore_snapshot(weather_snapshot);
        }
        departures_board
            .lock()
            .expect("departures board mutex poisoned")
            .restore_snapshot(cached.departures);
        standings_board
            .lock()
            .expect("standings board mutex poisoned")
            .restore_snapshot(cached.standings);
        calendar_poll_state
            .lock()
            .expect("calendar poll state mutex poisoned")
            .restore_snapshot(cached.calendar);
    }

    let cache_persister = CachePersister {
        path: Arc::new(cache_path),
        save_lock: Arc::new(Mutex::new(())),
        weather_store: Arc::clone(&weather_store),
        departures_board: Arc::clone(&departures_board),
        standings_board: Arc::clone(&standings_board),
        calendar_poll_state: Arc::clone(&calendar_poll_state),
    };

    let power_wake = WakeSignal::new();
    let weather_wake = WakeSignal::new();
    let departures_wakes: HashMap<u32, WakeSignal> = board_config
        .stations
        .iter()
        .map(|station| (station.site_id, WakeSignal::new()))
        .collect();
    let standings_wakes: HashMap<String, WakeSignal> = board_config
        .competitions
        .iter()
        .map(|competition| (competition.id.clone(), WakeSignal::new()))
        .collect();

    if let Some(screensaver) = board_config.screensaver.clone() {
        spawn_screensaver_poll_thread(
            screensaver,
            Arc::clone(&calendar_poll_state),
            cache_persister.clone(),
        );
    }

    spawn_display_power_reconciliation_thread(
        &board_config,
        Arc::clone(&override_path),
        Arc::clone(&calendar_poll_state),
        Arc::clone(&override_cache),
        Arc::clone(&touch_state),
        power_wake.clone(),
        departures_wakes.clone(),
        power::select_display_power_control(),
    );

    let weather_source = OpenMeteoWeatherSource::new(board_config.latitude, board_config.longitude);
    spawn_weather_refresh_thread(
        weather_source,
        Arc::clone(&weather_store),
        weather_wake.clone(),
        cache_persister.clone(),
    );

    spawn_departures_refresh_threads(
        &board_config,
        Arc::clone(&departures_board),
        &departures_wakes,
        cache_persister.clone(),
    );

    spawn_standings_refresh_threads(
        &board_config,
        Arc::clone(&standings_board),
        &standings_wakes,
        cache_persister,
    );

    spawn_update_agent_thread(
        board_config.time_zone.clone(),
        board_config.display_windows.clone(),
    );

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

    match parse_remote_control_token(std::env::var(REMOTE_CONTROL_TOKEN_ENV_VAR).ok()) {
        Some(expected_token) => {
            spawn_remote_control_server(remote_control::RemoteControlState {
                expected_token: Arc::new(expected_token),
                override_path: Arc::clone(&override_path),
                override_cache: Arc::clone(&override_cache),
                touch_state: Arc::clone(&touch_state),
                now: Arc::new(OffsetDateTime::now_utc),
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
        None => {
            eprintln!("{REMOTE_CONTROL_TOKEN_ENV_VAR} is not set: remote-control server disabled")
        }
    }

    let (mono_font, sans_font) = load_fonts().expect("vendored fonts must load");
    let theme = Theme {
        mono: &mono_font,
        sans: &sans_font,
    };

    let mut daylight_date: Option<Date> = None;
    let mut daylight = None;

    loop {
        let now_utc = OffsetDateTime::now_utc();
        let mut frame_taps = collect_frame_taps();

        let (mut current_touch, previous_mode) = {
            let mut guard = touch_state.lock().expect("touch state mutex poisoned");
            let previous_mode = guard.mode;
            let reverted = revert_idle(&guard, now_utc, &touch_config);
            *guard = reverted.clone();
            (reverted, previous_mode)
        };
        if previous_mode == TouchMode::Wake && current_touch.mode != TouchMode::Wake {
            sync_touch_wake_file(previous_mode, &current_touch, &touch_wake_path);
            power_wake.notify();
        }

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
        let override_state = *override_cache
            .lock()
            .expect("override cache mutex poisoned");
        let override_active = active_override_power(override_state, now_utc);
        let display_state = reconcile(schedule_says_awake, override_active, screensaver_active);

        let mut power_on = effective_display_power(display_state, &current_touch, now_utc);
        if !power_on {
            if !frame_taps.is_empty() {
                frame_taps.clear();
                let previous_mode = current_touch.mode;
                let (next_touch, effects) = apply_touch(
                    &current_touch,
                    DisplayContext::Blanked,
                    TouchTarget::Header,
                    now_utc,
                    &touch_config,
                );
                *touch_state.lock().expect("touch state mutex poisoned") = next_touch.clone();
                current_touch = next_touch;
                sync_touch_wake_file(previous_mode, &current_touch, &touch_wake_path);
                dispatch_touch_effects(
                    &effects,
                    &power_wake,
                    &weather_wake,
                    &departures_wakes,
                    &standings_wakes,
                );
                power_on = effective_display_power(display_state, &current_touch, now_utc);
            }
            if !power_on {
                macroquad::window::clear_background(macroquad::color::BLACK);
                macroquad::window::next_frame().await;
                continue;
            }
        }

        let screensaver_dismissed = current_touch.mode == TouchMode::ScreensaverDismissed
            && current_touch
                .screensaver_dismissed_until
                .is_some_and(|until| now_utc < until);
        let mut showing_screensaver =
            display_state == DisplayState::Screensaver && !screensaver_dismissed;
        if showing_screensaver && !frame_taps.is_empty() {
            frame_taps.clear();
            let previous_mode = current_touch.mode;
            let (next_touch, effects) = apply_touch(
                &current_touch,
                DisplayContext::Screensaver,
                TouchTarget::Header,
                now_utc,
                &touch_config,
            );
            *touch_state.lock().expect("touch state mutex poisoned") = next_touch.clone();
            current_touch = next_touch;
            sync_touch_wake_file(previous_mode, &current_touch, &touch_wake_path);
            dispatch_touch_effects(
                &effects,
                &power_wake,
                &weather_wake,
                &departures_wakes,
                &standings_wakes,
            );
            let dismissed_now = current_touch.mode == TouchMode::ScreensaverDismissed
                && current_touch
                    .screensaver_dismissed_until
                    .is_some_and(|until| now_utc < until);
            showing_screensaver = !dismissed_now;
        }

        macroquad::window::clear_background(palette::INK);

        match local_clock(&board_config.time_zone, now_utc) {
            Ok(clock) => {
                let calendar_availability = calendar_poll_state
                    .lock()
                    .expect("calendar poll state mutex poisoned")
                    .availability(now_utc);

                if showing_screensaver {
                    render_screensaver_view(&clock, &calendar_availability, &theme);
                    macroquad::window::next_frame().await;
                    continue;
                }

                let weather_state = weather_store
                    .lock()
                    .expect("weather store mutex poisoned")
                    .state(now_utc);

                let station_states: Vec<DeparturesPanelState> = {
                    let board = departures_board
                        .lock()
                        .expect("departures board mutex poisoned");
                    board_config
                        .stations
                        .iter()
                        .map(|station| board.station_state(station, now_utc))
                        .collect()
                };

                let rotation_id = standings_rotation.active_competition(now_utc);
                let rotation_index = board_config
                    .competitions
                    .iter()
                    .position(|competition| competition.id == rotation_id)
                    .unwrap_or(0);
                let mut active_index = current_touch
                    .football_index
                    .filter(|&index| index < board_config.competitions.len())
                    .unwrap_or(rotation_index);
                let mut active_competition_id = board_config.competitions[active_index].id.as_str();
                let mut standings_state = standings_board
                    .lock()
                    .expect("standings board mutex poisoned")
                    .competition_state(active_competition_id, now_utc);

                let station_inputs: Vec<StationLayoutInput<'_>> = board_config
                    .stations
                    .iter()
                    .zip(station_states.iter())
                    .map(|(station, state)| StationLayoutInput {
                        site_id: station.site_id,
                        state,
                    })
                    .collect();

                let mut layout = compute_dashboard_layout(
                    macroquad::window::screen_width(),
                    matches!(weather_state, WeatherPanelState::Stale { .. }),
                    &station_inputs,
                    current_touch.expanded_departure.as_deref(),
                    matches!(standings_state, StandingsPanelState::Stale { .. }),
                    board_config.competitions.len(),
                    active_index,
                    active_competition_id,
                );

                if !frame_taps.is_empty() {
                    let mut touched = false;
                    for (tap_x, tap_y) in frame_taps {
                        if let Some(target) = hit_test(&layout, tap_x, tap_y) {
                            let (next_touch, effects) = apply_touch(
                                &current_touch,
                                DisplayContext::Dashboard,
                                target,
                                now_utc,
                                &touch_config,
                            );
                            current_touch = next_touch;
                            dispatch_touch_effects(
                                &effects,
                                &power_wake,
                                &weather_wake,
                                &departures_wakes,
                                &standings_wakes,
                            );
                            touched = true;
                        }
                    }
                    if touched {
                        *touch_state.lock().expect("touch state mutex poisoned") =
                            current_touch.clone();
                        active_index = current_touch
                            .football_index
                            .filter(|&index| index < board_config.competitions.len())
                            .unwrap_or(rotation_index);
                        active_competition_id = board_config.competitions[active_index].id.as_str();
                        standings_state = standings_board
                            .lock()
                            .expect("standings board mutex poisoned")
                            .competition_state(active_competition_id, now_utc);
                        layout = compute_dashboard_layout(
                            macroquad::window::screen_width(),
                            matches!(weather_state, WeatherPanelState::Stale { .. }),
                            &station_inputs,
                            current_touch.expanded_departure.as_deref(),
                            matches!(standings_state, StandingsPanelState::Stale { .. }),
                            board_config.competitions.len(),
                            active_index,
                            active_competition_id,
                        );
                    }
                }

                render_clock_weather_header(&clock, &weather_state, &theme);

                for ((station, state), &panel_top) in board_config
                    .stations
                    .iter()
                    .zip(station_states.iter())
                    .zip(layout.station_tops.iter())
                {
                    render_departures_panel_with_header(
                        &station.name,
                        station.site_id,
                        state,
                        panel_top,
                        current_touch.expanded_departure.as_deref(),
                        &theme,
                    );
                }

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
                            let _ = render_daylight_strip_at(window, layout.daylight_top, &theme);
                        }
                    }
                    Err(error) => eprintln!("local date computation failed: {error}"),
                }

                render_standings_panel(
                    active_competition_id,
                    &standings_state,
                    layout.standings_top,
                    &theme,
                );

                if board_config.screensaver.is_some() {
                    render_calendar_availability_note(
                        &calendar_availability,
                        macroquad::window::screen_height() - 20.0,
                        &theme,
                    );
                }
            }
            Err(error) => eprintln!("clock computation failed: {error}"),
        }

        macroquad::window::next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_core::schedule::OverridePower;
    use dashboard_core::touch::{TouchEffect, TouchMode, TouchState};

    #[test]
    fn percent_encode_path_segment_preserves_unreserved_and_encodes_reserved_chars() {
        assert_eq!(percent_encode_path_segment("primary"), "primary");
        assert_eq!(
            percent_encode_path_segment("abc-123_456.789~"),
            "abc-123_456.789~"
        );
        assert_eq!(
            percent_encode_path_segment("en.swedish#holiday@group.v.calendar.google.com"),
            "en.swedish%23holiday%40group.v.calendar.google.com"
        );
        assert_eq!(
            percent_encode_path_segment("a/b c%20#?"),
            "a%2Fb%20c%2520%23%3F"
        );
    }

    #[test]
    fn parse_remote_control_token_treats_empty_and_whitespace_as_unset() {
        assert_eq!(parse_remote_control_token(None), None);
        assert_eq!(parse_remote_control_token(Some(String::new())), None);
        assert_eq!(
            parse_remote_control_token(Some("   \t\r\n  ".to_string())),
            None
        );
        assert_eq!(
            parse_remote_control_token(Some("secret-token".to_string())),
            Some("secret-token".to_string())
        );
    }

    #[test]
    fn effective_display_power_honors_touch_wake_until_expiry() {
        let now = OffsetDateTime::from_unix_timestamp(1_795_600_800).expect("valid timestamp");
        let idle = TouchState::new();
        assert!(!effective_display_power(
            DisplayState::ScheduledStandby,
            &idle,
            now
        ));
        assert!(!effective_display_power(
            DisplayState::ManualOverride(OverridePower::Off),
            &idle,
            now
        ));
        assert!(effective_display_power(DisplayState::Normal, &idle, now));

        let mut awake = TouchState::new();
        awake.mode = TouchMode::Wake;
        awake.wake_until = Some(now + time::Duration::seconds(60));
        assert!(effective_display_power(
            DisplayState::ScheduledStandby,
            &awake,
            now
        ));
        assert!(effective_display_power(
            DisplayState::ManualOverride(OverridePower::Off),
            &awake,
            now
        ));
        assert!(!effective_display_power(
            DisplayState::ScheduledStandby,
            &awake,
            now + time::Duration::seconds(60)
        ));
    }

    #[test]
    fn dispatch_touch_effects_notifies_matching_provider_wake_signals() {
        let power_wake = WakeSignal::new();
        let weather_wake = WakeSignal::new();
        let dep_9324 = WakeSignal::new();
        let dep_9325 = WakeSignal::new();
        let standings_sa = WakeSignal::new();

        let mut departures_wakes = HashMap::new();
        departures_wakes.insert(9324, dep_9324.clone());
        departures_wakes.insert(9325, dep_9325.clone());

        let mut standings_wakes = HashMap::new();
        standings_wakes.insert("serie-a".to_string(), standings_sa.clone());

        let pw = power_wake.clone();
        let ww = weather_wake.clone();
        let d1 = dep_9324.clone();
        let d2 = dep_9325.clone();
        let sw = standings_sa.clone();

        let h_power = std::thread::spawn(move || pw.wait_timeout(StdDuration::from_secs(2)));
        let h_weather = std::thread::spawn(move || ww.wait_timeout(StdDuration::from_secs(2)));
        let h_dep1 = std::thread::spawn(move || d1.wait_timeout(StdDuration::from_secs(2)));
        let h_dep2 = std::thread::spawn(move || d2.wait_timeout(StdDuration::from_millis(80)));
        let h_standings = std::thread::spawn(move || sw.wait_timeout(StdDuration::from_secs(2)));

        std::thread::sleep(StdDuration::from_millis(20));

        dispatch_touch_effects(
            &[
                TouchEffect::WakeDisplay,
                TouchEffect::RequestPanelRefetch {
                    provider: "weather".to_string(),
                },
                TouchEffect::RequestPanelRefetch {
                    provider: "departures:9324".to_string(),
                },
                TouchEffect::RequestPanelRefetch {
                    provider: "standings:serie-a".to_string(),
                },
            ],
            &power_wake,
            &weather_wake,
            &departures_wakes,
            &standings_wakes,
        );

        assert!(h_power.join().expect("thread must not panic"));
        assert!(h_weather.join().expect("thread must not panic"));
        assert!(h_dep1.join().expect("thread must not panic"));
        assert!(!h_dep2.join().expect("thread must not panic"));
        assert!(h_standings.join().expect("thread must not panic"));
    }

    #[test]
    fn dispatch_touch_effects_refresh_departures_wakes_all_sites() {
        let power_wake = WakeSignal::new();
        let weather_wake = WakeSignal::new();
        let dep_9324 = WakeSignal::new();
        let dep_9325 = WakeSignal::new();

        let mut departures_wakes = HashMap::new();
        departures_wakes.insert(9324, dep_9324.clone());
        departures_wakes.insert(9325, dep_9325.clone());
        let standings_wakes = HashMap::new();

        let d1 = dep_9324.clone();
        let d2 = dep_9325.clone();
        let h1 = std::thread::spawn(move || d1.wait_timeout(StdDuration::from_secs(2)));
        let h2 = std::thread::spawn(move || d2.wait_timeout(StdDuration::from_secs(2)));

        std::thread::sleep(StdDuration::from_millis(20));

        dispatch_touch_effects(
            &[TouchEffect::RefreshDeparturesRequested],
            &power_wake,
            &weather_wake,
            &departures_wakes,
            &standings_wakes,
        );

        assert!(h1.join().expect("thread must not panic"));
        assert!(h2.join().expect("thread must not panic"));
    }

    #[test]
    fn deferred_staged_candidate_activates_when_standby_ends_without_redownload() {
        let test_dir = std::env::temp_dir().join(format!(
            "pi-dashboard-update-deferred-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&test_dir);
        let staging_dir = test_dir.join("staging");
        let staged_version_path = test_dir.join("staged-version.txt");

        let bundle_bytes = b"release-v2-archive-bytes".to_vec();
        let digest = update_agent::sha256_hex(&bundle_bytes);
        let release = ReleaseAssets {
            version: "v2.0.0".to_string(),
            bundle_url: "https://example.invalid/v2.0.0.tar.gz".to_string(),
            digest_url: "https://example.invalid/v2.0.0.tar.gz.sha256".to_string(),
        };
        let now = OffsetDateTime::from_unix_timestamp(1_795_600_800).expect("valid timestamp");

        let state = UpdateAgentState::starting_at("0.1.0");
        let mut handed_off_version = None;
        let mut download_calls = 0usize;

        let staged_during_standby = process_update_release(
            &state,
            &mut handed_off_version,
            release.clone(),
            now,
            false,
            &staging_dir,
            &staged_version_path,
            |_, _| {
                download_calls += 1;
                Ok((digest.clone(), bundle_bytes.clone()))
            },
        )
        .expect("staging during standby must succeed");

        assert_eq!(download_calls, 1);
        assert_eq!(
            staged_during_standby.status,
            dashboard_core::update_agent::UpdateAgentStatus::Staged
        );
        assert!(!staged_version_path.exists());
        assert_eq!(handed_off_version, None);

        let activated_after_wake = process_update_release(
            &staged_during_standby,
            &mut handed_off_version,
            release.clone(),
            now + time::Duration::minutes(30),
            true,
            &staging_dir,
            &staged_version_path,
            |_, _| {
                download_calls += 1;
                Ok((digest.clone(), bundle_bytes.clone()))
            },
        )
        .expect("deferred activation after wake must succeed");

        assert_eq!(download_calls, 1);
        assert_eq!(
            std::fs::read_to_string(&staged_version_path).expect("staged version file must exist"),
            "v2.0.0"
        );
        assert_eq!(handed_off_version.as_deref(), Some("v2.0.0"));

        std::fs::remove_file(&staged_version_path).expect("cleanup must succeed");
        let _ = process_update_release(
            &activated_after_wake,
            &mut handed_off_version,
            release,
            now + time::Duration::minutes(60),
            true,
            &staging_dir,
            &staged_version_path,
            |_, _| {
                download_calls += 1;
                Ok((digest.clone(), bundle_bytes.clone()))
            },
        )
        .expect("subsequent poll must succeed");

        assert_eq!(download_calls, 1);
        assert!(!staged_version_path.exists());

        let _ = std::fs::remove_dir_all(&test_dir);
    }

    #[test]
    fn sync_touch_wake_file_writes_on_wake_entry_and_clears_on_expiry() {
        let path = std::env::temp_dir().join(format!(
            "pi-dashboard-main-touch-wake-{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let until = OffsetDateTime::from_unix_timestamp(1_795_600_920).expect("valid timestamp");

        let mut wake_state = TouchState::new();
        wake_state.mode = TouchMode::Wake;
        wake_state.wake_until = Some(until);

        sync_touch_wake_file(TouchMode::Default, &wake_state, &path);
        assert_eq!(power::read_touch_wake_until(&path), Some(until));

        sync_touch_wake_file(TouchMode::Wake, &TouchState::new(), &path);
        assert_eq!(power::read_touch_wake_until(&path), None);

        sync_touch_wake_file(TouchMode::Default, &wake_state, &path);
        assert_eq!(power::read_touch_wake_until(&path), Some(until));

        let mut dismissed_state = TouchState::new();
        dismissed_state.mode = TouchMode::ScreensaverDismissed;
        dismissed_state.screensaver_dismissed_until = Some(until);
        sync_touch_wake_file(TouchMode::Wake, &dismissed_state, &path);
        assert_eq!(power::read_touch_wake_until(&path), None);
    }
}
