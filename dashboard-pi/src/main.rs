use dashboard_core::board_config::{BoardConfig, load_board_config};
use dashboard_core::clock::local_clock;
use dashboard_core::daylight::daylight_window;
use dashboard_core::departures::{
    DepartureFetchError, DeparturesBoard, DeparturesSource, DeviationsSource, RawDeparture,
    RawDeviation, parse_sl_departures_response, parse_sl_deviations_response,
};
use dashboard_core::render::{
    STATION_PANEL_HEIGHT, render_clock_weather_header, render_daylight_strip,
    render_departures_panel,
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

                let board = departures_board
                    .lock()
                    .expect("departures board mutex poisoned");
                let mut panel_top = 140.0;
                for station in &board_config.stations {
                    let state = board.station_state(station, now_utc);
                    render_departures_panel(&state, panel_top);
                    panel_top += STATION_PANEL_HEIGHT;
                }
            }
            Err(error) => eprintln!("clock computation failed: {error}"),
        }

        macroquad::window::next_frame().await;
    }
}
