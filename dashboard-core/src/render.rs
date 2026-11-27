use crate::clock::LocalClock;
use crate::daylight::DaylightWindow;
use crate::departures::{Departure, DeparturesPanelState};
use crate::screensaver::CalendarAvailability;
use crate::standings::StandingsPanelState;
use crate::theme::{Theme, palette, status_color};
use crate::touch::TouchTarget;
use crate::weather::WeatherPanelState;

pub const MIN_TOUCH_TARGET_PX: f32 = 48.0;
pub const HEADER_HEIGHT: f32 = 80.0;
pub const WEATHER_HEADER_X: f32 = 240.0;
pub const DEPARTURE_ROW_HEIGHT: f32 = 48.0;
pub const DAYLIGHT_STRIP_HEIGHT: f32 = 48.0;

const MAX_INLINE_DISRUPTION_CHARS: usize = 40;
const STATION_HEADER_HEIGHT: f32 = MIN_TOUCH_TARGET_PX;
const STATION_PANEL_GAP: f32 = 12.0;
const EXPANDED_DETAIL_LINE_HEIGHT: f32 = 28.0;
const EXPANDED_DETAIL_PADDING: f32 = 12.0;
const EXPANDED_DISRUPTION_WRAP_CHARS: usize = 72;
const STANDINGS_STALE_HEADER_HEIGHT: f32 = 56.0;
const STANDINGS_BODY_HEIGHT: f32 = 680.0;

#[derive(Debug, Clone, PartialEq)]
pub struct ClockWeatherDisplay {
    pub time_text: String,
    pub weather_text: String,
}

pub fn format_clock_weather_header(
    clock: &LocalClock,
    weather_state: &WeatherPanelState,
) -> ClockWeatherDisplay {
    let time_text = format!("{:02}:{:02}:{:02}", clock.hour, clock.minute, clock.second);

    let weather_text = match weather_state {
        WeatherPanelState::Fresh { reading, age } => format!(
            "{:.0}°C  {}% rain ({}m)",
            reading.temp_c,
            reading.precip_probability_pct,
            age.whole_minutes()
        ),
        WeatherPanelState::Stale { reading, age } => format!(
            "{:.0}°C  {}% rain (stale, {}m old)",
            reading.temp_c,
            reading.precip_probability_pct,
            age.whole_minutes()
        ),
        WeatherPanelState::Unavailable => "weather unavailable".to_string(),
    };

    ClockWeatherDisplay {
        time_text,
        weather_text,
    }
}

pub fn render_clock_weather_header(
    clock: &LocalClock,
    weather_state: &WeatherPanelState,
    theme: &Theme,
) {
    use macroquad::shapes::{draw_line, draw_rectangle};
    use macroquad::text::{TextParams, draw_text_ex, measure_text};
    use macroquad::window::screen_width;

    const TIME_FONT_SIZE: u16 = 40;
    const WEATHER_GAP: f32 = 32.0;

    let display = format_clock_weather_header(clock, weather_state);

    draw_rectangle(0.0, 0.0, screen_width(), HEADER_HEIGHT, palette::PANEL);
    draw_line(
        0.0,
        HEADER_HEIGHT,
        screen_width(),
        HEADER_HEIGHT,
        1.0,
        palette::RULE,
    );

    draw_text_ex(
        &display.time_text,
        20.0,
        52.0,
        TextParams {
            font: Some(theme.mono),
            font_size: TIME_FONT_SIZE,
            color: palette::TEXT,
            ..Default::default()
        },
    );

    let time_width = measure_text(&display.time_text, Some(theme.mono), TIME_FONT_SIZE, 1.0).width;

    let weather_color = status_color(
        matches!(weather_state, WeatherPanelState::Stale { .. }),
        matches!(weather_state, WeatherPanelState::Unavailable),
    );
    draw_text_ex(
        &display.weather_text,
        (20.0 + time_width + WEATHER_GAP).max(WEATHER_HEADER_X),
        52.0,
        TextParams {
            font: Some(theme.sans),
            font_size: 32,
            color: weather_color,
            ..Default::default()
        },
    );
}

#[derive(Debug, Clone, PartialEq)]
pub struct DaylightStripDisplay {
    pub civil_dawn_text: String,
    pub sunrise_text: String,
    pub sunset_text: String,
    pub civil_dusk_text: String,
    pub duration_text: String,
}

fn format_hhmm(datetime: jiff::civil::DateTime) -> String {
    format!("{:02}:{:02}", datetime.hour(), datetime.minute())
}

pub fn format_daylight_strip(window: &DaylightWindow) -> DaylightStripDisplay {
    let duration_text = match window.daylight_duration() {
        Ok(span) => format!("{}h {:02}m daylight", span.get_hours(), span.get_minutes()),
        Err(_) => "daylight duration unavailable".to_string(),
    };

    DaylightStripDisplay {
        civil_dawn_text: format_hhmm(window.civil_dawn),
        sunrise_text: format_hhmm(window.sunrise),
        sunset_text: format_hhmm(window.sunset),
        civil_dusk_text: format_hhmm(window.civil_dusk),
        duration_text,
    }
}

pub fn render_daylight_strip_at(window: &DaylightWindow, top: f32, theme: &Theme) -> f32 {
    use macroquad::shapes::{draw_line, draw_rectangle};
    use macroquad::text::{TextParams, draw_text_ex};
    use macroquad::window::screen_width;

    let display = format_daylight_strip(window);
    let bottom = top + DAYLIGHT_STRIP_HEIGHT;

    draw_rectangle(
        0.0,
        top,
        screen_width(),
        DAYLIGHT_STRIP_HEIGHT,
        palette::PANEL,
    );
    draw_line(0.0, top, screen_width(), top, 1.0, palette::RULE);
    draw_line(0.0, bottom, screen_width(), bottom, 1.0, palette::RULE);
    let text = format!(
        "Dawn {}  Sunrise {}  Sunset {}  Dusk {}  ({})",
        display.civil_dawn_text,
        display.sunrise_text,
        display.sunset_text,
        display.civil_dusk_text,
        display.duration_text
    );
    draw_text_ex(
        &text,
        20.0,
        top + 31.0,
        TextParams {
            font: Some(theme.mono),
            font_size: 22,
            color: palette::TEXT,
            ..Default::default()
        },
    );
    bottom
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeparturesPanelDisplay {
    pub lines: Vec<String>,
    pub stale_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedDepartureDetail {
    pub full_disruption: Option<String>,
    pub upcoming: Vec<String>,
}

pub const MAX_DEPARTURES_PER_PANEL: usize = 6;

fn format_countdown(countdown_min: i64) -> String {
    if countdown_min < -1 {
        "För sent".to_string()
    } else if countdown_min <= 0 {
        "Gå nu".to_string()
    } else {
        format!("Gå om {countdown_min} min")
    }
}

fn truncate_inline_disruption(text: &str) -> String {
    if text.chars().count() <= MAX_INLINE_DISRUPTION_CHARS {
        return text.to_string();
    }
    let prefix: String = text.chars().take(MAX_INLINE_DISRUPTION_CHARS).collect();
    format!("{}…", prefix.trim_end())
}

fn format_departure_line(departure: &Departure) -> String {
    let countdown = format_countdown(departure.countdown_min);
    let disruption = departure
        .disruption
        .as_deref()
        .map(|text| format!("  ⚠ {}", truncate_inline_disruption(text)))
        .unwrap_or_default();
    format!(
        "{} {}  {}{}",
        departure.line, departure.destination, countdown, disruption
    )
}

pub fn departure_row_id(site_id: u32, line: &str, destination: &str) -> String {
    format!("{site_id}:{line}:{destination}")
}

pub fn format_expanded_departure_detail(
    state: &DeparturesPanelState,
    departure_id: &str,
) -> Option<ExpandedDepartureDetail> {
    let departures = match state {
        DeparturesPanelState::Fresh(departures)
        | DeparturesPanelState::Stale { departures, .. } => departures.as_slice(),
        DeparturesPanelState::Unavailable => return None,
    };
    if departures.is_empty() {
        return None;
    }

    let matching: Vec<&Departure> = departures
        .iter()
        .filter(|departure| {
            departure_row_id(departure.site_id, &departure.line, &departure.destination)
                == departure_id
        })
        .collect();

    let selected: Vec<&Departure> = if matching.is_empty() {
        departures.iter().collect()
    } else {
        matching
    };

    let full_disruption = selected
        .iter()
        .find_map(|departure| departure.disruption.clone());
    let upcoming = selected
        .iter()
        .take(3)
        .map(|departure| format_countdown(departure.countdown_min))
        .collect();

    Some(ExpandedDepartureDetail {
        full_disruption,
        upcoming,
    })
}

fn wrap_text(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        let current_len = current.chars().count();
        if !current.is_empty() && current_len + 1 + word_len > max_chars {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn format_expanded_detail_lines(detail: &ExpandedDepartureDetail) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(disruption) = &detail.full_disruption {
        let prefixed = format!("⚠ {disruption}");
        lines.extend(wrap_text(&prefixed, EXPANDED_DISRUPTION_WRAP_CHARS));
    }
    if !detail.upcoming.is_empty() {
        lines.push(format!("Next: {}", detail.upcoming.join("  ·  ")));
    }
    lines
}

fn expanded_detail_height(line_count: usize) -> f32 {
    if line_count == 0 {
        0.0
    } else {
        line_count as f32 * EXPANDED_DETAIL_LINE_HEIGHT + EXPANDED_DETAIL_PADDING
    }
}

struct ComputedExpandedDetail {
    y: f32,
    height: f32,
    lines: Vec<String>,
}

struct ComputedDepartureRow {
    y: f32,
    departure_id: String,
    text: String,
    expanded: Option<ComputedExpandedDetail>,
}

struct ComputedStationLayout {
    stale_note: Option<String>,
    rows: Vec<ComputedDepartureRow>,
    empty_status_text: Option<&'static str>,
    bottom: f32,
}

fn compute_station_layout(
    site_id: u32,
    state: &DeparturesPanelState,
    top: f32,
    expanded_departure: Option<&str>,
) -> ComputedStationLayout {
    let (departures, stale_note, empty_status_text) = match state {
        DeparturesPanelState::Fresh(departures) => {
            let empty = if departures.is_empty() {
                Some("no departures")
            } else {
                None
            };
            (departures.as_slice(), None, empty)
        }
        DeparturesPanelState::Stale { departures, age } => {
            let empty = if departures.is_empty() {
                Some("no departures")
            } else {
                None
            };
            (
                departures.as_slice(),
                Some(format!("stale, {}m old", age.whole_minutes())),
                empty,
            )
        }
        DeparturesPanelState::Unavailable => (&[][..], None, Some("departures unavailable")),
    };

    let mut y = top + STATION_HEADER_HEIGHT;
    let mut rows = Vec::new();

    if departures.is_empty() {
        y += DEPARTURE_ROW_HEIGHT;
    } else {
        let mut expanded_matched = false;
        for departure in departures.iter().take(MAX_DEPARTURES_PER_PANEL) {
            let departure_id = departure_row_id(site_id, &departure.line, &departure.destination);
            let row_y = y;
            y += DEPARTURE_ROW_HEIGHT;

            let expanded = if !expanded_matched && expanded_departure == Some(departure_id.as_str())
            {
                expanded_matched = true;
                format_expanded_departure_detail(state, &departure_id).map(|detail| {
                    let lines = format_expanded_detail_lines(&detail);
                    let height = expanded_detail_height(lines.len());
                    let detail_y = y;
                    y += height;
                    ComputedExpandedDetail {
                        y: detail_y,
                        height,
                        lines,
                    }
                })
            } else {
                None
            };

            rows.push(ComputedDepartureRow {
                y: row_y,
                departure_id,
                text: format_departure_line(departure),
                expanded,
            });
        }
    }

    ComputedStationLayout {
        stale_note,
        rows,
        empty_status_text,
        bottom: y + STATION_PANEL_GAP,
    }
}

pub fn format_departures_panel(state: &DeparturesPanelState) -> DeparturesPanelDisplay {
    let (departures, stale_note) = match state {
        DeparturesPanelState::Fresh(departures) => (departures.as_slice(), None),
        DeparturesPanelState::Stale { departures, age } => (
            departures.as_slice(),
            Some(format!("stale, {}m old", age.whole_minutes())),
        ),
        DeparturesPanelState::Unavailable => {
            return DeparturesPanelDisplay {
                lines: vec!["departures unavailable".to_string()],
                stale_note: None,
            };
        }
    };

    let mut lines: Vec<String> = departures
        .iter()
        .take(MAX_DEPARTURES_PER_PANEL)
        .map(format_departure_line)
        .collect();

    if lines.is_empty() {
        lines.push("no departures".to_string());
    }

    DeparturesPanelDisplay { lines, stale_note }
}

pub fn render_departures_panel_with_header(
    station_name: &str,
    site_id: u32,
    state: &DeparturesPanelState,
    top: f32,
    expanded_departure: Option<&str>,
    theme: &Theme,
) -> f32 {
    use macroquad::shapes::{draw_line, draw_rectangle};
    use macroquad::text::{TextParams, draw_text_ex, measure_text};
    use macroquad::window::screen_width;

    const HEADER_FONT_SIZE: u16 = 26;

    let computed = compute_station_layout(site_id, state, top, expanded_departure);
    let color = status_color(
        matches!(state, DeparturesPanelState::Stale { .. }),
        matches!(state, DeparturesPanelState::Unavailable),
    );

    draw_text_ex(
        station_name,
        20.0,
        top + 32.0,
        TextParams {
            font: Some(theme.sans),
            font_size: HEADER_FONT_SIZE,
            color,
            ..Default::default()
        },
    );

    if let Some(note) = &computed.stale_note {
        let name_width = measure_text(station_name, Some(theme.sans), HEADER_FONT_SIZE, 1.0).width;
        draw_text_ex(
            note,
            20.0 + name_width + 16.0,
            top + 32.0,
            TextParams {
                font: Some(theme.sans),
                font_size: 20,
                color: palette::STALE,
                ..Default::default()
            },
        );
    }

    let header_bottom = top + STATION_HEADER_HEIGHT;
    draw_line(
        20.0,
        header_bottom,
        screen_width() - 20.0,
        header_bottom,
        1.0,
        palette::RULE,
    );

    if let Some(status_text) = computed.empty_status_text {
        draw_text_ex(
            status_text,
            20.0,
            header_bottom + 31.0,
            TextParams {
                font: Some(theme.mono),
                font_size: 26,
                color,
                ..Default::default()
            },
        );
        let row_bottom = header_bottom + DEPARTURE_ROW_HEIGHT;
        draw_line(
            20.0,
            row_bottom,
            screen_width() - 20.0,
            row_bottom,
            1.0,
            palette::RULE,
        );
        return computed.bottom;
    }

    for row in &computed.rows {
        draw_text_ex(
            &row.text,
            20.0,
            row.y + 31.0,
            TextParams {
                font: Some(theme.mono),
                font_size: 26,
                color,
                ..Default::default()
            },
        );

        let rule_y = if let Some(expanded) = &row.expanded {
            draw_rectangle(
                20.0,
                expanded.y,
                (screen_width() - 40.0).max(0.0),
                expanded.height,
                palette::PANEL,
            );
            let mut detail_y = expanded.y + 24.0;
            for detail_line in &expanded.lines {
                let detail_color = if detail_line.starts_with('⚠') {
                    palette::STALE
                } else {
                    palette::MUTED
                };
                draw_text_ex(
                    detail_line,
                    32.0,
                    detail_y,
                    TextParams {
                        font: Some(theme.sans),
                        font_size: 20,
                        color: detail_color,
                        ..Default::default()
                    },
                );
                detail_y += EXPANDED_DETAIL_LINE_HEIGHT;
            }
            expanded.y + expanded.height
        } else {
            row.y + DEPARTURE_ROW_HEIGHT
        };

        draw_line(
            20.0,
            rule_y,
            screen_width() - 20.0,
            rule_y,
            1.0,
            palette::RULE,
        );
    }

    computed.bottom
}

#[derive(Debug, Clone, PartialEq)]
pub struct StandingsPanelDisplay {
    pub header: String,
    pub lines: Vec<String>,
    pub stale_note: Option<String>,
}

pub const MAX_STANDINGS_ROWS_PER_PANEL: usize = 24;

pub fn format_standings_panel(
    competition_label: &str,
    state: &StandingsPanelState,
) -> StandingsPanelDisplay {
    let (rows, stale_note) = match state {
        StandingsPanelState::Fresh(rows) => (rows.as_slice(), None),
        StandingsPanelState::Stale { rows, age } => (
            rows.as_slice(),
            Some(format!("stale, {}m old", age.whole_minutes())),
        ),
        StandingsPanelState::Unavailable => {
            return StandingsPanelDisplay {
                header: competition_label.to_string(),
                lines: vec!["standings unavailable".to_string()],
                stale_note: None,
            };
        }
    };

    let header = rows
        .first()
        .map(|row| format!("{}  {}", row.competition, row.season))
        .unwrap_or_else(|| competition_label.to_string());

    let mut lines: Vec<String> = rows
        .iter()
        .take(MAX_STANDINGS_ROWS_PER_PANEL)
        .map(|row| {
            format!(
                "{}. {}  P{} GD{:+} {}pts",
                row.position, row.team, row.played, row.goal_difference, row.points
            )
        })
        .collect();

    if lines.is_empty() {
        lines.push("no standings".to_string());
    }

    StandingsPanelDisplay {
        header,
        lines,
        stale_note,
    }
}

pub fn render_standings_panel(
    competition_label: &str,
    state: &StandingsPanelState,
    top: f32,
    theme: &Theme,
) {
    use macroquad::shapes::draw_line;
    use macroquad::text::{TextParams, draw_text_ex};
    use macroquad::window::screen_width;

    const LINE_HEIGHT: f32 = 28.0;

    let display = format_standings_panel(competition_label, state);

    let color = status_color(
        matches!(state, StandingsPanelState::Stale { .. }),
        matches!(state, StandingsPanelState::Unavailable),
    );

    let mut y = top + 24.0;
    draw_text_ex(
        &display.header,
        20.0,
        y,
        TextParams {
            font: Some(theme.sans),
            font_size: 26,
            color,
            ..Default::default()
        },
    );
    y += LINE_HEIGHT;

    if let Some(note) = &display.stale_note {
        draw_text_ex(
            note,
            20.0,
            y,
            TextParams {
                font: Some(theme.sans),
                font_size: 20,
                color: palette::STALE,
                ..Default::default()
            },
        );
        y += LINE_HEIGHT;
    }

    for line in &display.lines {
        draw_text_ex(
            line,
            20.0,
            y,
            TextParams {
                font: Some(theme.mono),
                font_size: 22,
                color,
                ..Default::default()
            },
        );
        y += LINE_HEIGHT * 0.6;
        draw_line(20.0, y, screen_width() - 20.0, y, 1.0, palette::RULE);
        y += LINE_HEIGHT * 0.4;
    }
}

pub fn format_calendar_availability_note(availability: &CalendarAvailability) -> Option<String> {
    match availability {
        CalendarAvailability::Fresh => None,
        CalendarAvailability::Stale { age } => {
            Some(format!("calendar unavailable ({}m)", age.whole_minutes()))
        }
        CalendarAvailability::Unavailable => Some("calendar unavailable".to_string()),
    }
}

pub fn render_calendar_availability_note(
    availability: &CalendarAvailability,
    y: f32,
    theme: &Theme,
) {
    use macroquad::text::{TextParams, draw_text_ex};

    if let Some(note) = format_calendar_availability_note(availability) {
        draw_text_ex(
            &note,
            20.0,
            y,
            TextParams {
                font: Some(theme.sans),
                font_size: 22,
                color: palette::MUTED,
                ..Default::default()
            },
        );
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScreensaverDisplay {
    pub time_text: String,
    pub availability_note: Option<String>,
}

pub fn format_screensaver_view(
    clock: &LocalClock,
    availability: &CalendarAvailability,
) -> ScreensaverDisplay {
    let time_text = format!("{:02}:{:02}:{:02}", clock.hour, clock.minute, clock.second);
    let availability_note = format_calendar_availability_note(availability);

    ScreensaverDisplay {
        time_text,
        availability_note,
    }
}

fn centered_text_x(screen_width: f32, text_width: f32) -> f32 {
    ((screen_width - text_width) / 2.0).max(20.0)
}

pub fn render_screensaver_view(
    clock: &LocalClock,
    availability: &CalendarAvailability,
    theme: &Theme,
) {
    use macroquad::shapes::draw_rectangle;
    use macroquad::text::{TextParams, draw_text_ex, measure_text};
    use macroquad::window::{screen_height, screen_width};

    let display = format_screensaver_view(clock, availability);

    draw_rectangle(0.0, 0.0, screen_width(), screen_height(), palette::INK);

    let metrics = measure_text(&display.time_text, Some(theme.mono), 80, 1.0);
    let center_x = centered_text_x(screen_width(), metrics.width);
    let center_y = screen_height() / 2.0;
    draw_text_ex(
        &display.time_text,
        center_x,
        center_y,
        TextParams {
            font: Some(theme.mono),
            font_size: 80,
            color: palette::TEXT,
            ..Default::default()
        },
    );

    render_calendar_availability_note(availability, screen_height() - 20.0, theme);
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HitRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl HitRect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DashboardHitRegion {
    pub rect: HitRect,
    pub target: TouchTarget,
}

#[derive(Debug, Clone, Copy)]
pub struct StationLayoutInput<'a> {
    pub site_id: u32,
    pub state: &'a DeparturesPanelState,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DashboardLayout {
    pub station_tops: Vec<f32>,
    pub daylight_top: f32,
    pub standings_top: f32,
    pub hit_regions: Vec<DashboardHitRegion>,
}

#[allow(clippy::too_many_arguments)]
pub fn compute_dashboard_layout(
    screen_width: f32,
    weather_is_stale: bool,
    stations: &[StationLayoutInput<'_>],
    expanded_departure: Option<&str>,
    standings_is_stale: bool,
    standings_panel_count: usize,
    standings_current_index: usize,
    active_competition_id: &str,
) -> DashboardLayout {
    let mut hit_regions = Vec::new();
    let weather_x = WEATHER_HEADER_X.min(screen_width / 2.0);

    hit_regions.push(DashboardHitRegion {
        rect: HitRect {
            x: 0.0,
            y: 0.0,
            width: weather_x,
            height: HEADER_HEIGHT,
        },
        target: TouchTarget::Header,
    });
    hit_regions.push(DashboardHitRegion {
        rect: HitRect {
            x: weather_x,
            y: 0.0,
            width: screen_width - weather_x,
            height: HEADER_HEIGHT,
        },
        target: if weather_is_stale {
            TouchTarget::StalePanel {
                provider: "weather".to_string(),
            }
        } else {
            TouchTarget::Header
        },
    });

    let mut y = HEADER_HEIGHT;
    let mut station_tops = Vec::with_capacity(stations.len());

    for station in stations {
        station_tops.push(y);
        let computed =
            compute_station_layout(station.site_id, station.state, y, expanded_departure);

        if matches!(station.state, DeparturesPanelState::Stale { .. }) {
            hit_regions.push(DashboardHitRegion {
                rect: HitRect {
                    x: 0.0,
                    y,
                    width: screen_width,
                    height: STATION_HEADER_HEIGHT,
                },
                target: TouchTarget::StalePanel {
                    provider: format!("departures:{}", station.site_id),
                },
            });
        }

        for row in &computed.rows {
            hit_regions.push(DashboardHitRegion {
                rect: HitRect {
                    x: 0.0,
                    y: row.y,
                    width: screen_width,
                    height: DEPARTURE_ROW_HEIGHT,
                },
                target: TouchTarget::DepartureRow {
                    departure_id: row.departure_id.clone(),
                },
            });
        }

        y = computed.bottom;
    }

    let daylight_top = y;
    let standings_top = daylight_top + DAYLIGHT_STRIP_HEIGHT;

    if standings_is_stale {
        hit_regions.push(DashboardHitRegion {
            rect: HitRect {
                x: 0.0,
                y: standings_top,
                width: screen_width,
                height: STANDINGS_STALE_HEADER_HEIGHT,
            },
            target: TouchTarget::StalePanel {
                provider: format!("standings:{active_competition_id}"),
            },
        });
        hit_regions.push(DashboardHitRegion {
            rect: HitRect {
                x: 0.0,
                y: standings_top + STANDINGS_STALE_HEADER_HEIGHT,
                width: screen_width,
                height: STANDINGS_BODY_HEIGHT,
            },
            target: TouchTarget::FootballPanel {
                panel_count: standings_panel_count,
                current_index: standings_current_index,
            },
        });
    } else {
        hit_regions.push(DashboardHitRegion {
            rect: HitRect {
                x: 0.0,
                y: standings_top,
                width: screen_width,
                height: STANDINGS_STALE_HEADER_HEIGHT + STANDINGS_BODY_HEIGHT,
            },
            target: TouchTarget::FootballPanel {
                panel_count: standings_panel_count,
                current_index: standings_current_index,
            },
        });
    }

    DashboardLayout {
        station_tops,
        daylight_top,
        standings_top,
        hit_regions,
    }
}

pub fn hit_test(layout: &DashboardLayout, x: f32, y: f32) -> Option<TouchTarget> {
    layout
        .hit_regions
        .iter()
        .find(|region| region.rect.contains(x, y))
        .map(|region| region.target.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::weather::WeatherReading;
    use time::Duration;
    use time::macros::datetime;

    fn clock() -> LocalClock {
        LocalClock {
            year: 2026,
            month: 11,
            day: 24,
            hour: 21,
            minute: 30,
            second: 15,
        }
    }

    #[test]
    fn formats_the_time_as_hh_mm_ss() {
        let display = format_clock_weather_header(&clock(), &WeatherPanelState::Unavailable);
        assert_eq!(display.time_text, "21:30:15");
    }

    #[test]
    fn formats_a_fresh_reading_with_temperature_and_precipitation() {
        let reading = WeatherReading {
            temp_c: 3.4,
            precip_probability_pct: 20,
            observed_at: datetime!(2026-11-24 20:00:00 UTC),
        };
        let state = WeatherPanelState::Fresh {
            reading,
            age: Duration::minutes(2),
        };

        let display = format_clock_weather_header(&clock(), &state);
        assert_eq!(display.weather_text, "3°C  20% rain (2m)");
        assert!(!display.weather_text.contains("stale"));
    }

    #[test]
    fn formats_a_stale_reading_with_its_age_distinct_from_fresh() {
        let reading = WeatherReading {
            temp_c: -1.0,
            precip_probability_pct: 5,
            observed_at: datetime!(2026-11-24 20:00:00 UTC),
        };
        let state = WeatherPanelState::Stale {
            reading,
            age: Duration::minutes(45),
        };

        let display = format_clock_weather_header(&clock(), &state);
        assert_eq!(display.weather_text, "-1°C  5% rain (stale, 45m old)");
    }

    #[test]
    fn formats_unavailable_as_unavailable_not_stale_garbage() {
        let display = format_clock_weather_header(&clock(), &WeatherPanelState::Unavailable);
        assert_eq!(display.weather_text, "weather unavailable");
    }

    #[test]
    fn formats_daylight_times_as_hh_mm_with_duration() {
        use jiff::civil::date;

        // Hand-constructed rather than driven through the real solar
        // engine, so this test only exercises formatting and doesn't flake
        // on future engine-precision changes.
        let sunrise = date(2026, 6, 21).at(3, 31, 0, 0);
        let sunset = date(2026, 6, 21).at(22, 9, 0, 0);
        let window = DaylightWindow {
            date: date(2026, 6, 21),
            civil_dawn: date(2026, 6, 21).at(1, 58, 0, 0),
            sunrise,
            sunset,
            civil_dusk: date(2026, 6, 21).at(23, 41, 0, 0),
            sunrise_utc: sunrise.in_tz("UTC").unwrap().timestamp(),
            sunset_utc: sunset.in_tz("UTC").unwrap().timestamp(),
        };

        let display = format_daylight_strip(&window);

        assert_eq!(display.civil_dawn_text, "01:58");
        assert_eq!(display.sunrise_text, "03:31");
        assert_eq!(display.sunset_text, "22:09");
        assert_eq!(display.civil_dusk_text, "23:41");
        assert_eq!(display.duration_text, "18h 38m daylight");
    }

    #[test]
    fn formats_a_departure_line_with_countdown_and_disruption() {
        use crate::departures::Departure;

        let state = DeparturesPanelState::Fresh(vec![Departure {
            site_id: 9324,
            line: "10".to_string(),
            destination: "Kungsträdgården".to_string(),
            countdown_min: 4,
            walk_time_min: 10,
            disruption: Some("Driftstörning".to_string()),
        }]);

        let display = format_departures_panel(&state);
        assert_eq!(display.stale_note, None);
        assert_eq!(
            display.lines,
            vec!["10 Kungsträdgården  Gå om 4 min  ⚠ Driftstörning".to_string()]
        );
    }

    #[test]
    fn formats_a_due_departure_as_leave_now_rather_than_a_negative_countdown() {
        use crate::departures::Departure;

        let state = DeparturesPanelState::Fresh(vec![Departure {
            site_id: 9324,
            line: "10".to_string(),
            destination: "Hjulsta".to_string(),
            countdown_min: 0,
            walk_time_min: 10,
            disruption: None,
        }]);

        let display = format_departures_panel(&state);
        assert_eq!(display.lines, vec!["10 Hjulsta  Gå nu".to_string()]);
    }

    #[test]
    fn formats_unavailable_departures_distinctly() {
        let display = format_departures_panel(&DeparturesPanelState::Unavailable);
        assert_eq!(display.lines, vec!["departures unavailable".to_string()]);
    }

    #[test]
    fn formats_a_successful_empty_result_distinctly_from_unavailable() {
        let display = format_departures_panel(&DeparturesPanelState::Fresh(Vec::new()));
        assert_eq!(display.lines, vec!["no departures".to_string()]);
        assert_ne!(display.lines, vec!["departures unavailable".to_string()]);
    }

    #[test]
    fn formats_a_meaningfully_negative_countdown_as_too_late_rather_than_leave_now() {
        use crate::departures::Departure;

        let state = DeparturesPanelState::Fresh(vec![Departure {
            site_id: 9324,
            line: "10".to_string(),
            destination: "Hjulsta".to_string(),
            countdown_min: -5,
            walk_time_min: 10,
            disruption: None,
        }]);

        let display = format_departures_panel(&state);
        assert_eq!(display.lines, vec!["10 Hjulsta  För sent".to_string()]);
    }

    #[test]
    fn a_countdown_of_exactly_minus_one_still_reads_as_leave_now() {
        use crate::departures::Departure;

        let state = DeparturesPanelState::Fresh(vec![Departure {
            site_id: 9324,
            line: "10".to_string(),
            destination: "Hjulsta".to_string(),
            countdown_min: -1,
            walk_time_min: 10,
            disruption: None,
        }]);

        let display = format_departures_panel(&state);
        assert_eq!(display.lines, vec!["10 Hjulsta  Gå nu".to_string()]);
    }

    #[test]
    fn caps_rendered_departures_at_six_keeping_the_nearest() {
        use crate::departures::Departure;

        let departures: Vec<Departure> = (0..10)
            .map(|i| Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: format!("Stop {i}"),
                countdown_min: i,
                walk_time_min: 10,
                disruption: None,
            })
            .collect();

        let display = format_departures_panel(&DeparturesPanelState::Fresh(departures));
        assert_eq!(display.lines.len(), MAX_DEPARTURES_PER_PANEL);
        assert!(display.lines[0].contains("Stop 0"));
        assert!(display.lines[5].contains("Stop 5"));
    }

    fn standings_row(position: u32, team: &str, points: i32) -> crate::standings::StandingsRow {
        crate::standings::StandingsRow {
            competition: "Serie A".to_string(),
            season: "2025/2026".to_string(),
            position,
            team: team.to_string(),
            played: 13,
            goal_difference: 10,
            points,
        }
    }

    #[test]
    fn formats_a_fresh_standings_panel_header_with_competition_and_season() {
        let state = StandingsPanelState::Fresh(vec![standings_row(1, "Internazionale", 32)]);
        let display = format_standings_panel("serie-a", &state);
        assert_eq!(display.header, "Serie A  2025/2026");
        assert_eq!(display.stale_note, None);
    }

    #[test]
    fn preserves_provider_order_including_tied_points_when_rendering() {
        let state = StandingsPanelState::Fresh(vec![
            standings_row(2, "Napoli", 30),
            standings_row(3, "Juventus", 30),
        ]);
        let display = format_standings_panel("serie-a", &state);
        assert!(display.lines[0].contains("Napoli"));
        assert!(display.lines[1].contains("Juventus"));
    }

    #[test]
    fn formats_a_stale_standings_panel_distinctly_from_fresh() {
        let state = StandingsPanelState::Stale {
            rows: vec![standings_row(1, "Internazionale", 32)],
            age: Duration::hours(3),
        };
        let display = format_standings_panel("serie-a", &state);
        assert_eq!(display.stale_note, Some("stale, 180m old".to_string()));
    }

    #[test]
    fn caps_rendered_standings_rows_keeping_the_top_of_the_table() {
        let rows: Vec<crate::standings::StandingsRow> = (1..=30)
            .map(|position| {
                standings_row(position, &format!("Team {position}"), 100 - position as i32)
            })
            .collect();

        let state = StandingsPanelState::Fresh(rows);
        let display = format_standings_panel("serie-a", &state);

        assert_eq!(display.lines.len(), MAX_STANDINGS_ROWS_PER_PANEL);
        assert!(display.lines[0].contains("Team 1"));
        assert!(
            display.lines[MAX_STANDINGS_ROWS_PER_PANEL - 1]
                .contains(&format!("Team {MAX_STANDINGS_ROWS_PER_PANEL}"))
        );
    }

    #[test]
    fn formats_unavailable_standings_using_the_configured_competition_label() {
        let display = format_standings_panel("allsvenskan", &StandingsPanelState::Unavailable);
        assert_eq!(display.header, "allsvenskan");
        assert_eq!(display.lines, vec!["standings unavailable".to_string()]);
    }

    #[test]
    fn screensaver_view_shows_the_clock_with_no_note_when_the_calendar_check_is_fresh() {
        let display = format_screensaver_view(&clock(), &CalendarAvailability::Fresh);
        assert_eq!(display.time_text, "21:30:15");
        assert_eq!(display.availability_note, None);
    }

    #[test]
    fn screensaver_view_surfaces_an_unobtrusive_unavailable_note_without_a_calendar_check_yet() {
        let display = format_screensaver_view(&clock(), &CalendarAvailability::Unavailable);
        assert_eq!(
            display.availability_note,
            Some("calendar unavailable".to_string())
        );
    }

    #[test]
    fn screensaver_view_surfaces_a_stale_note_with_its_age() {
        let display = format_screensaver_view(
            &clock(),
            &CalendarAvailability::Stale {
                age: Duration::minutes(12),
            },
        );
        assert_eq!(
            display.availability_note,
            Some("calendar unavailable (12m)".to_string())
        );
    }

    #[test]
    fn formats_calendar_availability_note_for_all_states() {
        assert_eq!(
            format_calendar_availability_note(&CalendarAvailability::Fresh),
            None
        );
        assert_eq!(
            format_calendar_availability_note(&CalendarAvailability::Stale {
                age: Duration::minutes(12),
            }),
            Some("calendar unavailable (12m)".to_string())
        );
        assert_eq!(
            format_calendar_availability_note(&CalendarAvailability::Unavailable),
            Some("calendar unavailable".to_string())
        );
    }

    #[test]
    fn centered_screensaver_clock_x_centers_measured_width_and_clamps_to_margin() {
        assert_eq!(centered_text_x(1200.0, 360.0), 420.0);
        assert_eq!(centered_text_x(300.0, 400.0), 20.0);
        assert_eq!(centered_text_x(400.0, 380.0), 20.0);
    }

    #[test]
    fn truncates_long_inline_disruption_text_on_collapsed_departure_rows() {
        use crate::departures::Departure;

        let long_disruption = "Inställda avgångar på blå linjen mellan Kungsträdgården och Hjulsta på grund av signalfel";
        let state = DeparturesPanelState::Fresh(vec![Departure {
            site_id: 9324,
            line: "10".to_string(),
            destination: "Kungsträdgården".to_string(),
            countdown_min: 4,
            walk_time_min: 10,
            disruption: Some(long_disruption.to_string()),
        }]);

        let display = format_departures_panel(&state);
        assert_eq!(display.lines.len(), 1);
        let line = &display.lines[0];
        assert!(line.ends_with('…'));
        assert!(!line.contains(long_disruption));
        let warning_part = line.split("⚠ ").nth(1).expect("disruption marker present");
        assert!(warning_part.chars().count() <= 41);
    }

    #[test]
    fn departure_row_id_combines_site_line_and_destination() {
        assert_eq!(
            departure_row_id(9324, "10", "Kungsträdgården"),
            "9324:10:Kungsträdgården"
        );
    }

    #[test]
    fn expanded_departure_detail_returns_full_disruption_and_next_three_matching_departures() {
        use crate::departures::Departure;

        let long_disruption = "Inställda avgångar på blå linjen mellan Kungsträdgården och Hjulsta på grund av signalfel";
        let state = DeparturesPanelState::Fresh(vec![
            Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: "Hjulsta".to_string(),
                countdown_min: 2,
                walk_time_min: 10,
                disruption: Some(long_disruption.to_string()),
            },
            Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: "Kungsträdgården".to_string(),
                countdown_min: 4,
                walk_time_min: 10,
                disruption: Some(long_disruption.to_string()),
            },
            Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: "Kungsträdgården".to_string(),
                countdown_min: 14,
                walk_time_min: 10,
                disruption: Some(long_disruption.to_string()),
            },
            Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: "Kungsträdgården".to_string(),
                countdown_min: 24,
                walk_time_min: 10,
                disruption: Some(long_disruption.to_string()),
            },
            Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: "Kungsträdgården".to_string(),
                countdown_min: 34,
                walk_time_min: 10,
                disruption: Some(long_disruption.to_string()),
            },
        ]);

        let detail = format_expanded_departure_detail(&state, "9324:10:Kungsträdgården")
            .expect("detail present");
        assert_eq!(detail.full_disruption, Some(long_disruption.to_string()));
        assert_eq!(
            detail.upcoming,
            vec![
                "Gå om 4 min".to_string(),
                "Gå om 14 min".to_string(),
                "Gå om 24 min".to_string(),
            ]
        );
    }

    #[test]
    fn expanded_departure_detail_falls_back_to_station_departures_and_handles_unavailable() {
        use crate::departures::Departure;

        let state = DeparturesPanelState::Fresh(vec![
            Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: "Hjulsta".to_string(),
                countdown_min: 0,
                walk_time_min: 10,
                disruption: None,
            },
            Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: "Kungsträdgården".to_string(),
                countdown_min: 6,
                walk_time_min: 10,
                disruption: None,
            },
        ]);

        let fallback = format_expanded_departure_detail(&state, "9324:10:Other")
            .expect("fallback detail present");
        assert_eq!(fallback.full_disruption, None);
        assert_eq!(
            fallback.upcoming,
            vec!["Gå nu".to_string(), "Gå om 6 min".to_string()]
        );

        assert_eq!(
            format_expanded_departure_detail(&DeparturesPanelState::Unavailable, "9324:10:Other"),
            None
        );
        assert_eq!(
            format_expanded_departure_detail(
                &DeparturesPanelState::Fresh(Vec::new()),
                "9324:10:Other"
            ),
            None
        );
    }

    #[test]
    fn hit_rect_contains_uses_half_open_bounds() {
        let rect = HitRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 48.0,
        };
        assert!(rect.contains(10.0, 20.0));
        assert!(rect.contains(50.0, 40.0));
        assert!(!rect.contains(110.0, 20.0));
        assert!(!rect.contains(10.0, 68.0));
        assert!(!rect.contains(9.9, 20.0));
        assert!(!rect.contains(10.0, 19.9));
    }

    fn sample_station_states() -> (DeparturesPanelState, DeparturesPanelState) {
        use crate::departures::Departure;

        let duvbo = DeparturesPanelState::Fresh(vec![
            Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: "Kungsträdgården".to_string(),
                countdown_min: 4,
                walk_time_min: 10,
                disruption: Some("Signalfel vid Fridhemsplan".to_string()),
            },
            Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: "Hjulsta".to_string(),
                countdown_min: 9,
                walk_time_min: 10,
                disruption: None,
            },
        ]);
        let sundbyberg = DeparturesPanelState::Fresh(vec![Departure {
            site_id: 9325,
            line: "40".to_string(),
            destination: "Stockholm Central".to_string(),
            countdown_min: 6,
            walk_time_min: 15,
            disruption: None,
        }]);
        (duvbo, sundbyberg)
    }

    #[test]
    fn dashboard_layout_orders_header_departures_daylight_then_standings() {
        let (duvbo, sundbyberg) = sample_station_states();
        let stations = [
            StationLayoutInput {
                site_id: 9324,
                state: &duvbo,
            },
            StationLayoutInput {
                site_id: 9325,
                state: &sundbyberg,
            },
        ];

        let layout =
            compute_dashboard_layout(1200.0, false, &stations, None, false, 2, 0, "allsvenskan");

        assert_eq!(layout.station_tops.len(), 2);
        assert!(layout.station_tops[0] >= HEADER_HEIGHT);
        assert!(layout.station_tops[1] > layout.station_tops[0]);
        assert!(layout.daylight_top > layout.station_tops[1]);
        assert!(layout.standings_top >= layout.daylight_top + DAYLIGHT_STRIP_HEIGHT);
    }

    #[test]
    fn dashboard_layout_all_touch_targets_meet_minimum_48px_height() {
        let (duvbo, sundbyberg) = sample_station_states();
        let stale_duvbo = match duvbo {
            DeparturesPanelState::Fresh(departures) => DeparturesPanelState::Stale {
                departures,
                age: Duration::minutes(5),
            },
            other => other,
        };
        let stations = [
            StationLayoutInput {
                site_id: 9324,
                state: &stale_duvbo,
            },
            StationLayoutInput {
                site_id: 9325,
                state: &sundbyberg,
            },
        ];

        let layout = compute_dashboard_layout(
            1200.0,
            true,
            &stations,
            Some("9324:10:Kungsträdgården"),
            true,
            2,
            0,
            "serie-a",
        );

        assert!(!layout.hit_regions.is_empty());
        for region in &layout.hit_regions {
            assert!(
                region.rect.height >= MIN_TOUCH_TARGET_PX,
                "target {:?} height {} < {}",
                region.target,
                region.rect.height,
                MIN_TOUCH_TARGET_PX
            );
            assert!(
                region.rect.width >= MIN_TOUCH_TARGET_PX,
                "target {:?} width {} < {}",
                region.target,
                region.rect.width,
                MIN_TOUCH_TARGET_PX
            );
        }
    }

    #[test]
    fn hit_test_maps_header_departures_and_standings_when_fresh() {
        use crate::touch::TouchTarget;

        let (duvbo, sundbyberg) = sample_station_states();
        let stations = [
            StationLayoutInput {
                site_id: 9324,
                state: &duvbo,
            },
            StationLayoutInput {
                site_id: 9325,
                state: &sundbyberg,
            },
        ];

        let layout =
            compute_dashboard_layout(1200.0, false, &stations, None, false, 2, 1, "serie-a");

        assert_eq!(hit_test(&layout, 100.0, 40.0), Some(TouchTarget::Header));
        assert_eq!(hit_test(&layout, 900.0, 40.0), Some(TouchTarget::Header));

        let duvbo_row_0_y = layout.station_tops[0] + DEPARTURE_ROW_HEIGHT + 10.0;
        assert_eq!(
            hit_test(&layout, 100.0, duvbo_row_0_y),
            Some(TouchTarget::DepartureRow {
                departure_id: "9324:10:Kungsträdgården".to_string(),
            })
        );

        let duvbo_row_1_y = duvbo_row_0_y + DEPARTURE_ROW_HEIGHT;
        assert_eq!(
            hit_test(&layout, 100.0, duvbo_row_1_y),
            Some(TouchTarget::DepartureRow {
                departure_id: "9324:10:Hjulsta".to_string(),
            })
        );

        let sundbyberg_row_0_y = layout.station_tops[1] + DEPARTURE_ROW_HEIGHT + 10.0;
        assert_eq!(
            hit_test(&layout, 100.0, sundbyberg_row_0_y),
            Some(TouchTarget::DepartureRow {
                departure_id: "9325:40:Stockholm Central".to_string(),
            })
        );

        assert_eq!(hit_test(&layout, 100.0, layout.daylight_top + 20.0), None);

        assert_eq!(
            hit_test(&layout, 100.0, layout.standings_top + 20.0),
            Some(TouchTarget::FootballPanel {
                panel_count: 2,
                current_index: 1,
            })
        );
        assert_eq!(hit_test(&layout, -5.0, 10.0), None);
    }

    #[test]
    fn hit_test_maps_stale_panels_to_refetch_targets_while_preserving_row_and_body_targets() {
        use crate::touch::TouchTarget;

        let (duvbo, sundbyberg) = sample_station_states();
        let stale_duvbo = match duvbo {
            DeparturesPanelState::Fresh(departures) => DeparturesPanelState::Stale {
                departures,
                age: Duration::minutes(4),
            },
            other => other,
        };
        let stations = [
            StationLayoutInput {
                site_id: 9324,
                state: &stale_duvbo,
            },
            StationLayoutInput {
                site_id: 9325,
                state: &sundbyberg,
            },
        ];

        let layout = compute_dashboard_layout(1200.0, true, &stations, None, true, 2, 0, "serie-a");

        assert_eq!(hit_test(&layout, 100.0, 40.0), Some(TouchTarget::Header));
        assert_eq!(
            hit_test(&layout, 900.0, 40.0),
            Some(TouchTarget::StalePanel {
                provider: "weather".to_string(),
            })
        );

        assert_eq!(
            hit_test(&layout, 100.0, layout.station_tops[0] + 10.0),
            Some(TouchTarget::StalePanel {
                provider: "departures:9324".to_string(),
            })
        );
        assert_eq!(
            hit_test(
                &layout,
                100.0,
                layout.station_tops[0] + DEPARTURE_ROW_HEIGHT + 10.0
            ),
            Some(TouchTarget::DepartureRow {
                departure_id: "9324:10:Kungsträdgården".to_string(),
            })
        );

        assert_eq!(
            hit_test(&layout, 100.0, layout.standings_top + 10.0),
            Some(TouchTarget::StalePanel {
                provider: "standings:serie-a".to_string(),
            })
        );
        assert_eq!(
            hit_test(&layout, 100.0, layout.standings_top + 100.0),
            Some(TouchTarget::FootballPanel {
                panel_count: 2,
                current_index: 0,
            })
        );
    }

    #[test]
    fn hit_test_maps_stale_weather_text_region_at_260px_and_header_when_fresh() {
        use crate::touch::TouchTarget;

        let (duvbo, sundbyberg) = sample_station_states();
        let stations = [
            StationLayoutInput {
                site_id: 9324,
                state: &duvbo,
            },
            StationLayoutInput {
                site_id: 9325,
                state: &sundbyberg,
            },
        ];

        let stale_layout =
            compute_dashboard_layout(1200.0, true, &stations, None, false, 2, 0, "serie-a");
        assert_eq!(
            hit_test(&stale_layout, 260.0, 40.0),
            Some(TouchTarget::StalePanel {
                provider: "weather".to_string(),
            })
        );
        assert_eq!(
            hit_test(&stale_layout, 100.0, 40.0),
            Some(TouchTarget::Header)
        );

        let fresh_layout =
            compute_dashboard_layout(1200.0, false, &stations, None, false, 2, 0, "serie-a");
        assert_eq!(
            hit_test(&fresh_layout, 260.0, 40.0),
            Some(TouchTarget::Header)
        );
    }

    #[test]
    fn expanding_a_departure_row_shifts_subsequent_rows_and_panels_down() {
        use crate::touch::TouchTarget;

        let (duvbo, sundbyberg) = sample_station_states();
        let stations = [
            StationLayoutInput {
                site_id: 9324,
                state: &duvbo,
            },
            StationLayoutInput {
                site_id: 9325,
                state: &sundbyberg,
            },
        ];

        let collapsed =
            compute_dashboard_layout(1200.0, false, &stations, None, false, 2, 0, "allsvenskan");
        let expanded = compute_dashboard_layout(
            1200.0,
            false,
            &stations,
            Some("9324:10:Kungsträdgården"),
            false,
            2,
            0,
            "allsvenskan",
        );

        assert_eq!(expanded.station_tops[0], collapsed.station_tops[0]);
        let delta = expanded.station_tops[1] - collapsed.station_tops[1];
        assert!(delta > 0.0);
        assert_eq!(expanded.daylight_top - collapsed.daylight_top, delta);
        assert_eq!(expanded.standings_top - collapsed.standings_top, delta);

        let shifted_row_1_y =
            expanded.station_tops[0] + DEPARTURE_ROW_HEIGHT + DEPARTURE_ROW_HEIGHT + delta + 10.0;
        assert_eq!(
            hit_test(&expanded, 100.0, shifted_row_1_y),
            Some(TouchTarget::DepartureRow {
                departure_id: "9324:10:Hjulsta".to_string(),
            })
        );
    }

    #[test]
    fn duplicate_departure_ids_only_expand_once_per_station() {
        use crate::departures::Departure;

        let duvbo = DeparturesPanelState::Fresh(vec![
            Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: "Kungsträdgården".to_string(),
                countdown_min: 4,
                walk_time_min: 10,
                disruption: Some("Signalfel".to_string()),
            },
            Departure {
                site_id: 9324,
                line: "10".to_string(),
                destination: "Kungsträdgården".to_string(),
                countdown_min: 14,
                walk_time_min: 10,
                disruption: Some("Signalfel".to_string()),
            },
        ]);
        let stations = [StationLayoutInput {
            site_id: 9324,
            state: &duvbo,
        }];

        let collapsed =
            compute_dashboard_layout(1200.0, false, &stations, None, false, 1, 0, "allsvenskan");
        let expanded = compute_dashboard_layout(
            1200.0,
            false,
            &stations,
            Some("9324:10:Kungsträdgården"),
            false,
            1,
            0,
            "allsvenskan",
        );

        let expected_detail_height = expanded_detail_height(2);
        assert_eq!(
            expanded.daylight_top - collapsed.daylight_top,
            expected_detail_height
        );
    }

    #[test]
    fn empty_and_unavailable_stations_produce_no_departure_row_hit_targets() {
        use crate::touch::TouchTarget;

        let empty = DeparturesPanelState::Fresh(Vec::new());
        let unavailable = DeparturesPanelState::Unavailable;
        let stations = [
            StationLayoutInput {
                site_id: 9324,
                state: &empty,
            },
            StationLayoutInput {
                site_id: 9325,
                state: &unavailable,
            },
        ];

        let layout =
            compute_dashboard_layout(1200.0, false, &stations, None, false, 2, 0, "allsvenskan");

        assert!(
            !layout
                .hit_regions
                .iter()
                .any(|region| matches!(region.target, TouchTarget::DepartureRow { .. }))
        );
        assert!(layout.station_tops[1] > layout.station_tops[0]);
        assert!(layout.daylight_top > layout.station_tops[1]);
        assert_eq!(
            layout.standings_top,
            layout.daylight_top + DAYLIGHT_STRIP_HEIGHT
        );
    }
}
