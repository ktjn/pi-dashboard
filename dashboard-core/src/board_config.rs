use crate::touch::TouchConfig;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use time::Time;
use time::macros::format_description;

#[derive(Debug, Deserialize)]
struct RawBoardConfig {
    location: RawLocation,
    #[serde(default)]
    stations: Vec<RawStation>,
    #[serde(default)]
    competitions: Vec<RawCompetition>,
    #[serde(default)]
    display_windows: Vec<RawDisplayWindow>,
    screensaver: Option<RawScreensaver>,
    panel_rotation: Option<RawPanelRotation>,
    touch: Option<RawTouch>,
}

#[derive(Debug, Deserialize)]
struct RawLocation {
    time_zone: String,
    latitude: f64,
    longitude: f64,
}

#[derive(Debug, Deserialize)]
struct RawStation {
    site_id: u32,
    name: String,
    mode: String,
    walk_time_min: u32,
    direction: String,
}

#[derive(Debug, Deserialize)]
struct RawCompetition {
    id: String,
    provider: String,
    competition_code: String,
    #[serde(default)]
    highlightly_league_id: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct RawDisplayWindow {
    day_type: String,
    start: String,
    end: String,
}

#[derive(Debug, Deserialize)]
struct RawScreensaver {
    calendar_id: String,
    credential_ref: String,
    #[serde(default)]
    content_mode: Option<String>,
    #[serde(default)]
    rules: Vec<RawScreensaverRule>,
}

#[derive(Debug, Deserialize)]
struct RawScreensaverRule {
    pattern: String,
    match_type: String,
}

#[derive(Debug, Deserialize)]
struct RawPanelRotation {
    interval_seconds: u32,
    #[serde(default)]
    panels: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct RawTouch {
    enabled: bool,
    #[serde(default)]
    wake_seconds: Option<u32>,
    #[serde(default)]
    screensaver_dismiss_seconds: Option<u32>,
    #[serde(default)]
    idle_timeout_seconds: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransitMode {
    Metro,
    Train,
    Tram,
    Bus,
    Ferry,
    Ship,
}

impl TransitMode {
    /// `pub(crate)`, not private: the SL departures/deviations client in
    /// `departures.rs` parses the same `METRO`/`TRAIN`/... vocabulary from
    /// the live API and must map onto this one enum rather than duplicating
    /// it.
    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "METRO" => Some(Self::Metro),
            "TRAIN" => Some(Self::Train),
            "TRAM" => Some(Self::Tram),
            "BUS" => Some(Self::Bus),
            "FERRY" => Some(Self::Ferry),
            "SHIP" => Some(Self::Ship),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayType {
    Weekday,
    Weekend,
}

impl DayType {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "weekday" => Some(Self::Weekday),
            "weekend" => Some(Self::Weekend),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FootballProvider {
    FootballDataOrg,
    Highlightly,
}

impl FootballProvider {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "football-data-org" => Some(Self::FootballDataOrg),
            "highlightly" => Some(Self::Highlightly),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchType {
    Substring,
    Regex,
}

impl MatchType {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "substring" => Some(Self::Substring),
            "regex" => Some(Self::Regex),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentMode {
    Clock,
}

impl ContentMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "clock" => Some(Self::Clock),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    pub site_id: u32,
    pub name: String,
    pub mode: TransitMode,
    pub walk_time_min: u32,
    pub direction: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Competition {
    pub id: String,
    pub provider: FootballProvider,
    pub competition_code: String,
    /// Highlightly's numeric `leagueId` (resolved offline via their
    /// `/leagues?leagueName=...&countryCode=...` lookup - there is no
    /// alpha-code scheme like football-data.org's). Required only for
    /// `provider = "highlightly"` competitions; football-data.org uses
    /// `competition_code` directly instead.
    pub highlightly_league_id: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DisplayWindow {
    pub day_type: DayType,
    pub start: Time,
    pub end: Time,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScreensaverRule {
    pub pattern: String,
    pub match_type: MatchType,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Screensaver {
    pub calendar_id: String,
    pub credential_ref: String,
    pub content_mode: ContentMode,
    pub rules: Vec<ScreensaverRule>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PanelRotation {
    pub interval_seconds: u32,
    pub panels: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoardConfig {
    pub time_zone: String,
    pub latitude: f64,
    pub longitude: f64,
    pub stations: Vec<Station>,
    pub competitions: Vec<Competition>,
    pub display_windows: Vec<DisplayWindow>,
    pub screensaver: Option<Screensaver>,
    pub panel_rotation: Option<PanelRotation>,
    pub touch: Option<TouchConfig>,
}

#[derive(Debug)]
pub enum BoardConfigError {
    Toml(String),
    Schema(String),
    Invalid(String),
    Io(String),
}

impl fmt::Display for BoardConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(message) => write!(f, "board.toml is not valid TOML: {message}"),
            Self::Schema(message) => {
                write!(
                    f,
                    "board.toml does not match the expected schema: {message}"
                )
            }
            Self::Invalid(message) => write!(f, "board.toml failed validation: {message}"),
            Self::Io(message) => write!(f, "board.toml could not be read: {message}"),
        }
    }
}

impl std::error::Error for BoardConfigError {}

fn parse_time(field: &str, value: &str) -> Result<Time, BoardConfigError> {
    let format = format_description!("[hour]:[minute]");
    Time::parse(value, &format).map_err(|_| {
        BoardConfigError::Invalid(format!("{field}: '{value}' is not a valid HH:MM time"))
    })
}

fn validate(raw: RawBoardConfig) -> Result<BoardConfig, BoardConfigError> {
    if raw.location.time_zone.trim().is_empty() {
        return Err(BoardConfigError::Invalid(
            "location.time_zone: must not be blank".to_string(),
        ));
    }
    if jiff::tz::TimeZone::get(&raw.location.time_zone).is_err() {
        return Err(BoardConfigError::Invalid(format!(
            "location.time_zone: '{}' is not a recognized IANA time zone",
            raw.location.time_zone
        )));
    }
    if !(-90.0..=90.0).contains(&raw.location.latitude) {
        return Err(BoardConfigError::Invalid(format!(
            "location.latitude: {} is out of range [-90, 90]",
            raw.location.latitude
        )));
    }
    if !(-180.0..=180.0).contains(&raw.location.longitude) {
        return Err(BoardConfigError::Invalid(format!(
            "location.longitude: {} is out of range [-180, 180]",
            raw.location.longitude
        )));
    }

    if raw.stations.is_empty() {
        return Err(BoardConfigError::Invalid(
            "stations: at least one station is required".to_string(),
        ));
    }
    let mut stations = Vec::with_capacity(raw.stations.len());
    for (index, station) in raw.stations.into_iter().enumerate() {
        let mode = TransitMode::parse(&station.mode).ok_or_else(|| {
            BoardConfigError::Invalid(format!(
                "stations[{index}].mode: '{}' is not a recognized mode (expected METRO, TRAIN, TRAM, BUS, FERRY or SHIP)",
                station.mode
            ))
        })?;
        stations.push(Station {
            site_id: station.site_id,
            name: station.name,
            mode,
            walk_time_min: station.walk_time_min,
            direction: station.direction,
        });
    }

    if raw.competitions.is_empty() {
        return Err(BoardConfigError::Invalid(
            "competitions: at least one competition is required".to_string(),
        ));
    }
    let mut competitions = Vec::with_capacity(raw.competitions.len());
    for (index, competition) in raw.competitions.into_iter().enumerate() {
        let provider = FootballProvider::parse(&competition.provider).ok_or_else(|| {
            BoardConfigError::Invalid(format!(
                "competitions[{index}].provider: '{}' is not a recognized provider (expected football-data-org or highlightly)",
                competition.provider
            ))
        })?;
        if provider == FootballProvider::Highlightly && competition.highlightly_league_id.is_none()
        {
            return Err(BoardConfigError::Invalid(format!(
                "competitions[{index}].highlightly_league_id: is required when provider is highlightly"
            )));
        }
        competitions.push(Competition {
            id: competition.id,
            provider,
            competition_code: competition.competition_code,
            highlightly_league_id: competition.highlightly_league_id,
        });
    }

    if raw.display_windows.is_empty() {
        return Err(BoardConfigError::Invalid(
            "display_windows: at least one display window is required".to_string(),
        ));
    }
    let mut display_windows = Vec::with_capacity(raw.display_windows.len());
    for (index, window) in raw.display_windows.into_iter().enumerate() {
        let day_type = DayType::parse(&window.day_type).ok_or_else(|| {
            BoardConfigError::Invalid(format!(
                "display_windows[{index}].day_type: '{}' is not a recognized day type (expected weekday or weekend)",
                window.day_type
            ))
        })?;
        let start = parse_time(&format!("display_windows[{index}].start"), &window.start)?;
        let end = parse_time(&format!("display_windows[{index}].end"), &window.end)?;
        if end <= start {
            return Err(BoardConfigError::Invalid(format!(
                "display_windows[{index}]: end ({}) must be after start ({})",
                window.end, window.start
            )));
        }
        display_windows.push(DisplayWindow {
            day_type,
            start,
            end,
        });
    }

    let screensaver = match raw.screensaver {
        Some(screensaver) => {
            let content_mode = match screensaver.content_mode {
                Some(value) => ContentMode::parse(&value).ok_or_else(|| {
                    BoardConfigError::Invalid(format!(
                        "screensaver.content_mode: '{value}' is not a recognized content mode (expected clock)"
                    ))
                })?,
                None => ContentMode::Clock,
            };
            let mut rules = Vec::with_capacity(screensaver.rules.len());
            for (index, rule) in screensaver.rules.into_iter().enumerate() {
                let match_type = MatchType::parse(&rule.match_type).ok_or_else(|| {
                    BoardConfigError::Invalid(format!(
                        "screensaver.rules[{index}].match_type: '{}' is not a recognized match type (expected substring or regex)",
                        rule.match_type
                    ))
                })?;
                if match_type == MatchType::Regex {
                    regex::Regex::new(&rule.pattern).map_err(|error| {
                        BoardConfigError::Invalid(format!(
                            "screensaver.rules[{index}].pattern: '{}' is not a valid regex: {error}",
                            rule.pattern
                        ))
                    })?;
                }
                rules.push(ScreensaverRule {
                    pattern: rule.pattern,
                    match_type,
                });
            }
            Some(Screensaver {
                calendar_id: screensaver.calendar_id,
                credential_ref: screensaver.credential_ref,
                content_mode,
                rules,
            })
        }
        None => None,
    };

    let panel_rotation = match raw.panel_rotation {
        Some(panel_rotation) => {
            if panel_rotation.interval_seconds == 0 {
                return Err(BoardConfigError::Invalid(
                    "panel_rotation.interval_seconds: must be greater than 0".to_string(),
                ));
            }
            if panel_rotation.panels.is_empty() {
                return Err(BoardConfigError::Invalid(
                    "panel_rotation.panels: at least one panel is required".to_string(),
                ));
            }
            Some(PanelRotation {
                interval_seconds: panel_rotation.interval_seconds,
                panels: panel_rotation.panels,
            })
        }
        None => None,
    };

    let touch = match raw.touch {
        Some(touch) => {
            let wake_seconds = touch
                .wake_seconds
                .unwrap_or(TouchConfig::DEFAULT_WAKE_SECONDS);
            let screensaver_dismiss_seconds = touch
                .screensaver_dismiss_seconds
                .unwrap_or(TouchConfig::DEFAULT_SCREENSAVER_DISMISS_SECONDS);
            let idle_timeout_seconds = touch
                .idle_timeout_seconds
                .unwrap_or(TouchConfig::DEFAULT_IDLE_TIMEOUT_SECONDS);
            if wake_seconds == 0 {
                return Err(BoardConfigError::Invalid(
                    "touch.wake_seconds: must be greater than 0".to_string(),
                ));
            }
            if screensaver_dismiss_seconds == 0 {
                return Err(BoardConfigError::Invalid(
                    "touch.screensaver_dismiss_seconds: must be greater than 0".to_string(),
                ));
            }
            if idle_timeout_seconds == 0 {
                return Err(BoardConfigError::Invalid(
                    "touch.idle_timeout_seconds: must be greater than 0".to_string(),
                ));
            }
            Some(TouchConfig {
                enabled: touch.enabled,
                wake_seconds,
                screensaver_dismiss_seconds,
                idle_timeout_seconds,
            })
        }
        None => None,
    };

    Ok(BoardConfig {
        time_zone: raw.location.time_zone,
        latitude: raw.location.latitude,
        longitude: raw.location.longitude,
        stations,
        competitions,
        display_windows,
        screensaver,
        panel_rotation,
        touch,
    })
}

pub fn parse_board_config(input: &str) -> Result<BoardConfig, BoardConfigError> {
    let value: toml::Value =
        toml::from_str(input).map_err(|error| BoardConfigError::Toml(error.to_string()))?;
    let raw: RawBoardConfig = value
        .try_into()
        .map_err(|error: toml::de::Error| BoardConfigError::Schema(error.to_string()))?;
    validate(raw)
}

pub fn load_board_config(path: &Path) -> Result<BoardConfig, BoardConfigError> {
    let input = std::fs::read_to_string(path)
        .map_err(|error| BoardConfigError::Io(format!("{}: {error}", path.display())))?;
    parse_board_config(&input)
}
