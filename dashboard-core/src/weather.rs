use serde::{Deserialize, Serialize};
use std::fmt;
use time::macros::format_description;
use time::{Duration, OffsetDateTime, PrimitiveDateTime};

pub const WEATHER_STALE_AFTER: Duration = Duration::minutes(20);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeatherReading {
    pub temp_c: f64,
    pub precip_probability_pct: u8,
    pub observed_at: OffsetDateTime,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WeatherFetchError {
    InvalidJson(String),
    MissingData(String),
}

impl fmt::Display for WeatherFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(message) => {
                write!(f, "weather response is not valid JSON: {message}")
            }
            Self::MissingData(message) => {
                write!(f, "weather response is missing usable data: {message}")
            }
        }
    }
}

impl std::error::Error for WeatherFetchError {}

pub trait WeatherSource {
    fn fetch(&mut self) -> Result<WeatherReading, WeatherFetchError>;
}

#[cfg(feature = "fixtures")]
pub struct FixtureWeatherSource {
    responses: Vec<Result<WeatherReading, WeatherFetchError>>,
    index: usize,
}

#[cfg(feature = "fixtures")]
impl FixtureWeatherSource {
    pub fn new(responses: Vec<Result<WeatherReading, WeatherFetchError>>) -> Self {
        Self {
            responses,
            index: 0,
        }
    }
}

#[cfg(feature = "fixtures")]
impl WeatherSource for FixtureWeatherSource {
    fn fetch(&mut self) -> Result<WeatherReading, WeatherFetchError> {
        let last_index = self.responses.len().saturating_sub(1);
        let response = self
            .responses
            .get(self.index.min(last_index))
            .cloned()
            .ok_or_else(|| {
                WeatherFetchError::MissingData("no fixture responses configured".to_string())
            })?;
        if self.index < last_index {
            self.index += 1;
        }
        response
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WeatherCacheSnapshot {
    pub temp_c: f64,
    pub precip_probability_pct: u8,
    pub observed_at_unix: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WeatherPanelState {
    Fresh {
        reading: WeatherReading,
        age: Duration,
    },
    Stale {
        reading: WeatherReading,
        age: Duration,
    },
    Unavailable,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WeatherSnapshotStore {
    last_successful: Option<WeatherReading>,
}

impl WeatherSnapshotStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_success(&mut self, reading: WeatherReading, _fetched_at: OffsetDateTime) {
        self.last_successful = Some(reading);
    }

    pub fn export_snapshot(&self) -> Option<WeatherCacheSnapshot> {
        self.last_successful.map(|reading| WeatherCacheSnapshot {
            temp_c: reading.temp_c,
            precip_probability_pct: reading.precip_probability_pct,
            observed_at_unix: reading.observed_at.unix_timestamp(),
        })
    }

    pub fn restore_snapshot(&mut self, snapshot: WeatherCacheSnapshot) {
        if let Ok(observed_at) = OffsetDateTime::from_unix_timestamp(snapshot.observed_at_unix) {
            self.last_successful = Some(WeatherReading {
                temp_c: snapshot.temp_c,
                precip_probability_pct: snapshot.precip_probability_pct,
                observed_at,
            });
        }
    }

    pub fn state(&self, now: OffsetDateTime) -> WeatherPanelState {
        match self.last_successful {
            None => WeatherPanelState::Unavailable,
            Some(reading) => {
                let age = now - reading.observed_at;
                if age <= WEATHER_STALE_AFTER {
                    WeatherPanelState::Fresh { reading, age }
                } else {
                    WeatherPanelState::Stale { reading, age }
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenMeteoResponse {
    current: OpenMeteoCurrent,
    hourly: OpenMeteoHourly,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoCurrent {
    time: String,
    temperature_2m: f64,
}

#[derive(Debug, Deserialize)]
struct OpenMeteoHourly {
    time: Vec<String>,
    precipitation_probability: Vec<i64>,
}

fn parse_open_meteo_time(value: &str) -> Result<OffsetDateTime, WeatherFetchError> {
    let format = format_description!("[year]-[month]-[day]T[hour]:[minute]");
    let naive = PrimitiveDateTime::parse(value, &format).map_err(|error| {
        WeatherFetchError::InvalidJson(format!("current.time: '{value}' {error}"))
    })?;
    Ok(naive.assume_utc())
}

/// Parses an Open-Meteo `/v1/forecast` response fetched with
/// `current=temperature_2m&hourly=precipitation_probability&forecast_hours=2&timezone=UTC`.
/// Assumes the request used `timezone=UTC`, so `current.time`/`hourly.time`
/// values are interpreted as UTC wall-clock timestamps.
pub fn parse_open_meteo_response(json: &str) -> Result<WeatherReading, WeatherFetchError> {
    let parsed: OpenMeteoResponse = serde_json::from_str(json)
        .map_err(|error| WeatherFetchError::InvalidJson(error.to_string()))?;

    let observed_at = parse_open_meteo_time(&parsed.current.time)?;

    let next_hour_probability = parsed
        .hourly
        .time
        .iter()
        .zip(parsed.hourly.precipitation_probability.iter())
        .find(|(time, _)| time.as_str() > parsed.current.time.as_str())
        .map(|(_, probability)| probability)
        .or_else(|| parsed.hourly.precipitation_probability.last())
        .ok_or_else(|| {
            WeatherFetchError::MissingData("hourly.precipitation_probability is empty".to_string())
        })?;

    if !(0..=100).contains(next_hour_probability) {
        return Err(WeatherFetchError::MissingData(format!(
            "hourly.precipitation_probability value {next_hour_probability} is out of the expected 0-100 range"
        )));
    }
    let precip_probability_pct = *next_hour_probability as u8;

    Ok(WeatherReading {
        temp_c: parsed.current.temperature_2m,
        precip_probability_pct,
        observed_at,
    })
}
