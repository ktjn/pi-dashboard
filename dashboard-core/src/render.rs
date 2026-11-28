use crate::board_config::TransitMode;
use crate::clock::LocalClock;
use crate::daylight::DaylightWindow;
use crate::departures::{Departure, DeparturesPanelState};
use crate::scene3d::{
    PanelFlipTransition, SolarSceneState, SplitFlapBoard, classify_weather_glyph,
    render_screensaver_backdrop_3d, render_solar_diorama_3d, render_split_flap_clock_3d,
    render_weather_glyph_3d,
};
use crate::screensaver::CalendarAvailability;
use crate::standings::{StandingsPanelState, StandingsRow};
use crate::theme::{Theme, mode_badge_color, mode_label, palette, status_color};
use crate::touch::TouchTarget;
use crate::weather::WeatherPanelState;

pub const MIN_TOUCH_TARGET_PX: f32 = 48.0;
pub const HEADER_HEIGHT: f32 = 96.0;
pub const WEATHER_HEADER_X: f32 = 332.0;
pub const DEPARTURE_ROW_HEIGHT: f32 = 48.0;
pub const DAYLIGHT_STRIP_HEIGHT: f32 = 220.0;

const MAX_INLINE_DISRUPTION_CHARS: usize = 40;
const STATION_HEADER_HEIGHT: f32 = MIN_TOUCH_TARGET_PX;
const STATION_PANEL_GAP: f32 = 12.0;
const EXPANDED_DETAIL_LINE_HEIGHT: f32 = 28.0;
const EXPANDED_DETAIL_PADDING: f32 = 12.0;
const EXPANDED_DISRUPTION_WRAP_CHARS: usize = 72;
const STANDINGS_HEADER_HEIGHT: f32 = 48.0;
const STANDINGS_STALE_HEADER_HEIGHT: f32 = 56.0;
const STANDINGS_BODY_HEIGHT: f32 = 680.0;
const STANDINGS_ROW_HEIGHT: f32 = 27.0;

fn with_alpha(color: macroquad::color::Color, alpha: f32) -> macroquad::color::Color {
    macroquad::color::Color::new(color.r, color.g, color.b, alpha.clamp(0.0, 1.0))
}

fn shade_color(color: macroquad::color::Color, factor: f32) -> macroquad::color::Color {
    let k = factor.clamp(0.0, 1.0);
    macroquad::color::Color::new(color.r * k, color.g * k, color.b * k, color.a)
}

fn pill_badge_width(
    text: &str,
    font: &macroquad::text::Font,
    font_size: u16,
    pad_x: f32,
    min_width: f32,
) -> f32 {
    let dims = macroquad::text::measure_text(text, Some(font), font_size, 1.0);
    (dims.width + pad_x * 2.0).max(min_width)
}

fn status_dot_pill_width(text: &str, font: &macroquad::text::Font, font_size: u16) -> f32 {
    let dims = macroquad::text::measure_text(text, Some(font), font_size, 1.0);
    10.0 + 4.0 * 2.0 + 8.0 + dims.width + 10.0
}

fn fit_weather_header_pills(
    after_temp_x: f32,
    max_right: f32,
    precip_w: f32,
    fresh_w: f32,
) -> (Option<f32>, Option<f32>) {
    let comfy_precip_x = after_temp_x + 18.0;
    let comfy_fresh_x = comfy_precip_x + precip_w + 12.0;
    if comfy_fresh_x + fresh_w <= max_right {
        return (Some(comfy_precip_x), Some(comfy_fresh_x));
    }

    let tight_precip_x = after_temp_x + 8.0;
    let tight_fresh_x = tight_precip_x + precip_w + 8.0;
    if tight_fresh_x + fresh_w <= max_right {
        return (Some(tight_precip_x), Some(tight_fresh_x));
    }

    if tight_precip_x + precip_w <= max_right {
        return (Some(tight_precip_x), None);
    }

    if tight_precip_x + fresh_w <= max_right {
        return (None, Some(tight_precip_x));
    }

    (None, None)
}

#[allow(clippy::too_many_arguments)]
fn draw_pill_badge(
    x: f32,
    y: f32,
    height: f32,
    pad_x: f32,
    min_width: f32,
    text: &str,
    font: &macroquad::text::Font,
    font_size: u16,
    bg: macroquad::color::Color,
    border: Option<macroquad::color::Color>,
    text_color: macroquad::color::Color,
) -> f32 {
    use macroquad::shapes::{draw_rectangle, draw_rectangle_lines};
    use macroquad::text::{TextParams, draw_text_ex, measure_text};

    let dims = measure_text(text, Some(font), font_size, 1.0);
    let width = (dims.width + pad_x * 2.0).max(min_width);
    draw_rectangle(x, y, width, height, bg);
    if let Some(border_color) = border {
        draw_rectangle_lines(x, y, width, height, 1.0, border_color);
    }
    let text_x = x + ((width - dims.width) * 0.5).max(pad_x * 0.5);
    let text_y = y + (height + dims.offset_y) * 0.5;
    draw_text_ex(
        text,
        text_x,
        text_y,
        TextParams {
            font: Some(font),
            font_size,
            color: text_color,
            ..Default::default()
        },
    );
    width
}

#[allow(clippy::too_many_arguments)]
fn draw_status_dot_pill(
    x: f32,
    y: f32,
    height: f32,
    text: &str,
    font: &macroquad::text::Font,
    font_size: u16,
    dot_color: macroquad::color::Color,
    bg: macroquad::color::Color,
    border: macroquad::color::Color,
    text_color: macroquad::color::Color,
) -> f32 {
    use macroquad::shapes::{draw_circle, draw_rectangle, draw_rectangle_lines};
    use macroquad::text::{TextParams, draw_text_ex, measure_text};

    let dims = measure_text(text, Some(font), font_size, 1.0);
    let dot_radius = 4.0;
    let pad_left = 10.0;
    let dot_gap = 8.0;
    let pad_right = 10.0;
    let width = pad_left + dot_radius * 2.0 + dot_gap + dims.width + pad_right;

    draw_rectangle(x, y, width, height, bg);
    draw_rectangle_lines(x, y, width, height, 1.0, border);
    draw_circle(
        x + pad_left + dot_radius,
        y + height * 0.5,
        dot_radius,
        dot_color,
    );
    let text_x = x + pad_left + dot_radius * 2.0 + dot_gap;
    let text_y = y + (height + dims.offset_y) * 0.5;
    draw_text_ex(
        text,
        text_x,
        text_y,
        TextParams {
            font: Some(font),
            font_size,
            color: text_color,
            ..Default::default()
        },
    );
    width
}

fn truncate_text_to_width(
    text: &str,
    max_width: f32,
    font: &macroquad::text::Font,
    font_size: u16,
) -> String {
    use macroquad::text::measure_text;

    if max_width <= 0.0 {
        return String::new();
    }
    if measure_text(text, Some(font), font_size, 1.0).width <= max_width {
        return text.to_string();
    }
    let mut clipped = String::new();
    for ch in text.chars() {
        let mut candidate = clipped.clone();
        candidate.push(ch);
        candidate.push('…');
        if measure_text(&candidate, Some(font), font_size, 1.0).width > max_width {
            break;
        }
        clipped.push(ch);
    }
    if clipped.is_empty() {
        String::new()
    } else {
        format!("{}…", clipped.trim_end())
    }
}

pub fn format_local_date(clock: &LocalClock) -> String {
    let parsed = i8::try_from(clock.month)
        .ok()
        .zip(i8::try_from(clock.day).ok())
        .and_then(|(month, day)| jiff::civil::Date::new(clock.year, month, day).ok());

    let Some(date) = parsed else {
        return format!("{:04}-{:02}-{:02}", clock.year, clock.month, clock.day);
    };

    let weekday = match date.weekday() {
        jiff::civil::Weekday::Monday => "Mon",
        jiff::civil::Weekday::Tuesday => "Tue",
        jiff::civil::Weekday::Wednesday => "Wed",
        jiff::civil::Weekday::Thursday => "Thu",
        jiff::civil::Weekday::Friday => "Fri",
        jiff::civil::Weekday::Saturday => "Sat",
        jiff::civil::Weekday::Sunday => "Sun",
    };
    let month = match date.month() {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => return format!("{:04}-{:02}-{:02}", clock.year, clock.month, clock.day),
    };
    format!("{weekday} {} {month}", date.day())
}

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

pub fn weather_precip_probability(state: &WeatherPanelState) -> u8 {
    match state {
        WeatherPanelState::Fresh { reading, .. } | WeatherPanelState::Stale { reading, .. } => {
            reading.precip_probability_pct
        }
        WeatherPanelState::Unavailable => 0,
    }
}

pub fn render_clock_weather_header(
    clock: &LocalClock,
    split_flap: &SplitFlapBoard,
    weather_state: &WeatherPanelState,
    theme: &Theme,
) {
    use macroquad::shapes::{draw_circle, draw_line, draw_rectangle};
    use macroquad::text::{TextParams, draw_text_ex, measure_text};
    use macroquad::window::screen_width;

    let screen_w = screen_width();
    draw_rectangle(0.0, 0.0, screen_w, HEADER_HEIGHT, palette::PANEL);
    draw_line(
        0.0,
        HEADER_HEIGHT,
        screen_w,
        HEADER_HEIGHT,
        1.0,
        palette::RULE,
    );

    render_split_flap_clock_3d(split_flap, 16.0, 6.0, 300.0, 66.0);

    let date_text = format_local_date(clock);
    let date_dims = measure_text(&date_text, Some(theme.sans), 15, 1.0);
    let date_x = 16.0 + ((300.0 - date_dims.width) * 0.5).max(0.0);
    draw_text_ex(
        &date_text,
        date_x,
        88.0,
        TextParams {
            font: Some(theme.sans),
            font_size: 15,
            color: palette::MUTED,
            ..Default::default()
        },
    );

    let divider_x = WEATHER_HEADER_X - 8.0;
    draw_line(
        divider_x,
        14.0,
        divider_x,
        HEADER_HEIGHT - 14.0,
        1.0,
        palette::RULE,
    );

    match weather_state {
        WeatherPanelState::Fresh { reading, age } | WeatherPanelState::Stale { reading, age } => {
            let is_stale = matches!(weather_state, WeatherPanelState::Stale { .. });
            let glyph_kind = classify_weather_glyph(reading.temp_c, reading.precip_probability_pct);
            render_weather_glyph_3d(
                glyph_kind,
                macroquad::time::get_time() as f32,
                WEATHER_HEADER_X,
                14.0,
                68.0,
                68.0,
            );

            let content_x = WEATHER_HEADER_X + 80.0;
            let temp_text = format!("{:.0}°C", reading.temp_c);
            let temp_color = status_color(is_stale, false);
            draw_text_ex(
                &temp_text,
                content_x,
                59.0,
                TextParams {
                    font: Some(theme.mono),
                    font_size: 38,
                    color: temp_color,
                    ..Default::default()
                },
            );

            let temp_w = measure_text(&temp_text, Some(theme.mono), 38, 1.0).width;
            let precip_text = format!("{}% rain", reading.precip_probability_pct);
            let precip_accent = if is_stale {
                palette::STALE
            } else {
                palette::ACCENT_CYAN
            };
            let (dot_color, fresh_text, fresh_bg, fresh_border, fresh_text_color) = if is_stale {
                (
                    palette::STALE,
                    format!("stale {}m", age.whole_minutes()),
                    with_alpha(palette::STALE, 0.14),
                    with_alpha(palette::STALE, 0.55),
                    palette::STALE,
                )
            } else {
                (
                    palette::OK,
                    format!("{}m", age.whole_minutes()),
                    palette::SURFACE,
                    palette::RULE,
                    palette::MUTED,
                )
            };

            let precip_w = pill_badge_width(&precip_text, theme.sans, 17, 12.0, 0.0);
            let fresh_w = status_dot_pill_width(&fresh_text, theme.mono, 15);
            let (precip_pos, fresh_pos) =
                fit_weather_header_pills(content_x + temp_w, screen_w - 20.0, precip_w, fresh_w);

            if let Some(precip_x) = precip_pos {
                draw_pill_badge(
                    precip_x,
                    32.0,
                    30.0,
                    12.0,
                    0.0,
                    &precip_text,
                    theme.sans,
                    17,
                    with_alpha(precip_accent, 0.14),
                    Some(with_alpha(precip_accent, 0.45)),
                    precip_accent,
                );
            }

            if let Some(fresh_x) = fresh_pos {
                draw_status_dot_pill(
                    fresh_x,
                    32.0,
                    30.0,
                    &fresh_text,
                    theme.mono,
                    15,
                    dot_color,
                    fresh_bg,
                    fresh_border,
                    fresh_text_color,
                );
            }
        }
        WeatherPanelState::Unavailable => {
            draw_circle(WEATHER_HEADER_X + 8.0, 48.0, 5.0, palette::ALERT);
            draw_text_ex(
                "weather unavailable",
                WEATHER_HEADER_X + 24.0,
                56.0,
                TextParams {
                    font: Some(theme.sans),
                    font_size: 24,
                    color: palette::DOWN,
                    ..Default::default()
                },
            );
        }
    }
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

pub fn daylight_metric_cells(
    window: &DaylightWindow,
    daylight_fraction: f32,
) -> [(&'static str, String); 5] {
    let display = format_daylight_strip(window);
    let clamped = if daylight_fraction.is_finite() {
        daylight_fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pct = (clamped * 100.0).round() as u32;
    let daylight_value = if window.daylight_duration().is_ok() {
        format!("{} · {pct}%", display.duration_text)
    } else {
        display.duration_text
    };
    [
        ("DAWN", display.civil_dawn_text),
        ("SUNRISE", display.sunrise_text),
        ("DAYLIGHT", daylight_value),
        ("SUNSET", display.sunset_text),
        ("DUSK", display.civil_dusk_text),
    ]
}

pub fn daylight_cell_bounds(left: f32, total_width: f32) -> [(f32, f32); 5] {
    const WEIGHTS: [f32; 5] = [0.16, 0.16, 0.36, 0.16, 0.16];
    let safe_width = if total_width.is_finite() {
        total_width.max(0.0)
    } else {
        0.0
    };
    let mut bounds = [(left, 0.0); 5];
    let mut x = left;
    for (idx, weight) in WEIGHTS.iter().enumerate() {
        let w = safe_width * weight;
        bounds[idx] = (x, w);
        x += w;
    }
    bounds
}

pub fn render_daylight_strip_at(
    window: &DaylightWindow,
    solar_scene: &SolarSceneState,
    precip_probability_pct: u8,
    top: f32,
    theme: &Theme,
) -> f32 {
    use macroquad::shapes::{draw_line, draw_rectangle};
    use macroquad::text::{TextParams, draw_text_ex, measure_text};
    use macroquad::window::screen_width;

    let screen_w = screen_width();
    let bottom = top + DAYLIGHT_STRIP_HEIGHT;

    draw_rectangle(0.0, top, screen_w, DAYLIGHT_STRIP_HEIGHT, palette::PANEL);
    draw_line(0.0, top, screen_w, top, 1.0, palette::RULE);

    let stage_w = (screen_w - 40.0).max(1.0);
    render_solar_diorama_3d(
        solar_scene,
        precip_probability_pct,
        macroquad::time::get_time() as f32,
        20.0,
        top + 6.0,
        stage_w,
        174.0,
    );

    let metrics_top = top + 180.0;
    draw_line(
        20.0,
        metrics_top,
        screen_w - 20.0,
        metrics_top,
        1.0,
        palette::RULE,
    );

    let cells = daylight_metric_cells(window, solar_scene.daylight_fraction);
    let cell_bounds = daylight_cell_bounds(20.0, stage_w);
    let progress_fraction = if solar_scene.daylight_fraction.is_finite() {
        solar_scene.daylight_fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };

    for (idx, ((label, value), &(cell_x, cell_w))) in
        cells.iter().zip(cell_bounds.iter()).enumerate()
    {
        if idx > 0 {
            draw_line(
                cell_x,
                metrics_top + 5.0,
                cell_x,
                bottom - 5.0,
                1.0,
                palette::RULE,
            );
        }

        let label_dims = measure_text(label, Some(theme.sans), 11, 1.0);
        let label_x = cell_x + ((cell_w - label_dims.width) * 0.5).max(4.0);
        draw_text_ex(
            label,
            label_x,
            metrics_top + 13.0,
            TextParams {
                font: Some(theme.sans),
                font_size: 11,
                color: palette::MUTED,
                ..Default::default()
            },
        );

        let is_daylight_cell = idx == 2;
        let value_font = if is_daylight_cell {
            theme.sans
        } else {
            theme.mono
        };
        let value_size: u16 = if is_daylight_cell { 14 } else { 16 };
        let clipped_value =
            truncate_text_to_width(value, (cell_w - 12.0).max(0.0), value_font, value_size);
        let value_dims = measure_text(&clipped_value, Some(value_font), value_size, 1.0);
        let value_x = cell_x + ((cell_w - value_dims.width) * 0.5).max(4.0);
        let value_y = if is_daylight_cell {
            metrics_top + 28.0
        } else {
            metrics_top + 31.0
        };
        draw_text_ex(
            &clipped_value,
            value_x,
            value_y,
            TextParams {
                font: Some(value_font),
                font_size: value_size,
                color: palette::TEXT,
                ..Default::default()
            },
        );

        if is_daylight_cell {
            let bar_x = cell_x + 14.0;
            let bar_w = (cell_w - 28.0).max(0.0);
            let bar_y = metrics_top + 33.5;
            let bar_h = 3.0;
            draw_rectangle(bar_x, bar_y, bar_w, bar_h, palette::RULE);
            draw_rectangle(
                bar_x,
                bar_y,
                bar_w * progress_fraction,
                bar_h,
                palette::ACCENT_CYAN,
            );
        }
    }

    draw_line(0.0, bottom, screen_w, bottom, 1.0, palette::RULE);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountdownUrgency {
    Comfortable,
    LeaveNow,
    TooLate,
}

pub fn classify_countdown_urgency(countdown_min: i64) -> CountdownUrgency {
    if countdown_min < -1 {
        CountdownUrgency::TooLate
    } else if countdown_min <= 0 {
        CountdownUrgency::LeaveNow
    } else {
        CountdownUrgency::Comfortable
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StationHeaderMeta<'a> {
    pub name: &'a str,
    pub site_id: u32,
    pub mode: TransitMode,
    pub walk_time_min: u32,
}

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

fn expanded_disruption_lines(detail: &ExpandedDepartureDetail) -> Vec<String> {
    detail
        .full_disruption
        .as_deref()
        .map(|disruption| {
            let prefixed = format!("⚠ {disruption}");
            wrap_text(&prefixed, EXPANDED_DISRUPTION_WRAP_CHARS)
        })
        .unwrap_or_default()
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
    disruption_lines: Vec<String>,
    upcoming: Vec<String>,
}

struct ComputedDepartureRow {
    y: f32,
    departure_id: String,
    line: String,
    destination: String,
    countdown_min: i64,
    disruption: Option<String>,
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
                    let disruption_lines = expanded_disruption_lines(&detail);
                    let line_count =
                        disruption_lines.len() + usize::from(!detail.upcoming.is_empty());
                    let height = expanded_detail_height(line_count);
                    let detail_y = y;
                    y += height;
                    ComputedExpandedDetail {
                        y: detail_y,
                        height,
                        disruption_lines,
                        upcoming: detail.upcoming,
                    }
                })
            } else {
                None
            };

            rows.push(ComputedDepartureRow {
                y: row_y,
                departure_id,
                line: departure.line.clone(),
                destination: departure.destination.clone(),
                countdown_min: departure.countdown_min,
                disruption: departure.disruption.clone(),
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

pub fn render_departures_panel_rich(
    meta: StationHeaderMeta<'_>,
    state: &DeparturesPanelState,
    top: f32,
    expanded_departure: Option<&str>,
    theme: &Theme,
) -> f32 {
    use macroquad::shapes::{draw_circle, draw_line, draw_rectangle, draw_rectangle_lines};
    use macroquad::text::{TextParams, draw_text_ex, measure_text};
    use macroquad::window::screen_width;

    let screen_w = screen_width();
    let computed = compute_station_layout(meta.site_id, state, top, expanded_departure);
    let is_stale = matches!(state, DeparturesPanelState::Stale { .. });
    let is_unavailable = matches!(state, DeparturesPanelState::Unavailable);
    let color = status_color(is_stale, is_unavailable);
    let mode_color = mode_badge_color(meta.mode);

    draw_rectangle(0.0, top, screen_w, STATION_HEADER_HEIGHT, palette::SURFACE);

    let mode_w = draw_pill_badge(
        20.0,
        top + 11.0,
        26.0,
        10.0,
        64.0,
        mode_label(meta.mode),
        theme.sans,
        13,
        mode_color,
        None,
        palette::TEXT,
    );

    let name_x = 20.0 + mode_w + 14.0;
    draw_text_ex(
        meta.name,
        name_x,
        top + 32.0,
        TextParams {
            font: Some(theme.sans),
            font_size: 24,
            color,
            ..Default::default()
        },
    );

    let name_w = measure_text(meta.name, Some(theme.sans), 24, 1.0).width;
    let walk_text = format!("{} min walk", meta.walk_time_min);
    let walk_x = name_x + name_w + 14.0;
    let walk_w = draw_pill_badge(
        walk_x,
        top + 11.0,
        26.0,
        10.0,
        0.0,
        &walk_text,
        theme.sans,
        14,
        palette::PANEL,
        Some(palette::RULE),
        palette::MUTED,
    );

    if let Some(note) = &computed.stale_note {
        let note_dims = measure_text(note, Some(theme.sans), 14, 1.0);
        let pill_w = 10.0 + 8.0 + 8.0 + note_dims.width + 10.0;
        let stale_x = (screen_w - 20.0 - pill_w).max(walk_x + walk_w + 12.0);
        draw_status_dot_pill(
            stale_x,
            top + 11.0,
            26.0,
            note,
            theme.sans,
            14,
            palette::STALE,
            with_alpha(palette::STALE, 0.14),
            with_alpha(palette::STALE, 0.55),
            palette::STALE,
        );
    }

    let header_bottom = top + STATION_HEADER_HEIGHT;
    draw_line(
        0.0,
        header_bottom,
        screen_w,
        header_bottom,
        1.0,
        palette::RULE,
    );

    if let Some(status_text) = computed.empty_status_text {
        let dot_color = if is_unavailable {
            palette::ALERT
        } else {
            palette::MUTED
        };
        draw_circle(28.0, header_bottom + 24.0, 4.5, dot_color);
        draw_text_ex(
            status_text,
            42.0,
            header_bottom + 31.0,
            TextParams {
                font: Some(theme.sans),
                font_size: 22,
                color,
                ..Default::default()
            },
        );
        let row_bottom = header_bottom + DEPARTURE_ROW_HEIGHT;
        draw_line(
            20.0,
            row_bottom,
            screen_w - 20.0,
            row_bottom,
            1.0,
            palette::RULE,
        );
        return computed.bottom;
    }

    for (idx, row) in computed.rows.iter().enumerate() {
        let is_expanded = row.expanded.is_some();
        if is_expanded {
            draw_rectangle(0.0, row.y, screen_w, DEPARTURE_ROW_HEIGHT, palette::SURFACE);
            draw_rectangle(0.0, row.y, 3.0, DEPARTURE_ROW_HEIGHT, palette::ACCENT_CYAN);
            draw_rectangle_lines(
                0.0,
                row.y,
                screen_w,
                DEPARTURE_ROW_HEIGHT,
                1.0,
                with_alpha(palette::ACCENT_CYAN, 0.45),
            );
        } else if idx % 2 == 1 {
            draw_rectangle(
                0.0,
                row.y,
                screen_w,
                DEPARTURE_ROW_HEIGHT,
                palette::CARD_ALT,
            );
        }

        let line_w = draw_pill_badge(
            20.0,
            row.y + 9.0,
            30.0,
            10.0,
            52.0,
            &row.line,
            theme.mono,
            18,
            mode_color,
            None,
            palette::TEXT,
        );

        let countdown_text = format_countdown(row.countdown_min);
        let (cd_bg, cd_border, cd_text_color) = match classify_countdown_urgency(row.countdown_min)
        {
            CountdownUrgency::Comfortable => (
                with_alpha(palette::OK, 0.16),
                with_alpha(palette::OK, 0.50),
                palette::OK,
            ),
            CountdownUrgency::LeaveNow => (
                with_alpha(palette::URGENT, 0.24),
                palette::URGENT,
                palette::TEXT,
            ),
            CountdownUrgency::TooLate => (
                with_alpha(palette::ALERT, 0.14),
                with_alpha(palette::ALERT, 0.45),
                palette::ALERT,
            ),
        };
        let cd_dims = measure_text(&countdown_text, Some(theme.mono), 17, 1.0);
        let cd_pill_w = (cd_dims.width + 24.0).max(96.0);
        let cd_x = (screen_w - 20.0 - cd_pill_w).max(220.0);
        draw_pill_badge(
            cd_x,
            row.y + 9.0,
            30.0,
            12.0,
            96.0,
            &countdown_text,
            theme.mono,
            17,
            cd_bg,
            Some(cd_border),
            cd_text_color,
        );

        let dest_x = 20.0 + line_w + 14.0;
        let max_middle_x = (cd_x - 14.0).max(dest_x);
        let middle_span = (max_middle_x - dest_x).max(0.0);
        let dest_max_w = if row.disruption.is_some() {
            (middle_span * 0.52).max(120.0).min(middle_span)
        } else {
            middle_span
        };
        let dest_text = truncate_text_to_width(&row.destination, dest_max_w, theme.sans, 24);
        draw_text_ex(
            &dest_text,
            dest_x,
            row.y + 32.0,
            TextParams {
                font: Some(theme.sans),
                font_size: 24,
                color,
                ..Default::default()
            },
        );

        if let Some(disruption) = &row.disruption {
            let dest_w = measure_text(&dest_text, Some(theme.sans), 24, 1.0).width;
            let pill_x = dest_x + dest_w + 14.0;
            let avail_w = max_middle_x - pill_x;
            if avail_w >= 80.0 {
                let inline = format!("⚠ {}", truncate_inline_disruption(disruption));
                let clipped =
                    truncate_text_to_width(&inline, (avail_w - 20.0).max(0.0), theme.sans, 14);
                if !clipped.is_empty() {
                    draw_pill_badge(
                        pill_x,
                        row.y + 11.0,
                        26.0,
                        10.0,
                        0.0,
                        &clipped,
                        theme.sans,
                        14,
                        with_alpha(palette::STALE, 0.15),
                        Some(with_alpha(palette::STALE, 0.50)),
                        palette::STALE,
                    );
                }
            }
        }

        let rule_y = if let Some(expanded) = &row.expanded {
            let card_w = (screen_w - 40.0).max(0.0);
            draw_rectangle(20.0, expanded.y, card_w, expanded.height, palette::SURFACE);
            draw_rectangle(20.0, expanded.y, 4.0, expanded.height, palette::URGENT);

            let mut detail_y = expanded.y + 24.0;
            let max_line_w = (card_w - 28.0).max(0.0);
            for detail_line in &expanded.disruption_lines {
                let clipped_line = truncate_text_to_width(detail_line, max_line_w, theme.sans, 19);
                draw_text_ex(
                    &clipped_line,
                    36.0,
                    detail_y,
                    TextParams {
                        font: Some(theme.sans),
                        font_size: 19,
                        color: palette::STALE,
                        ..Default::default()
                    },
                );
                detail_y += EXPANDED_DETAIL_LINE_HEIGHT;
            }

            if !expanded.upcoming.is_empty() {
                let chip_row_top = detail_y - 16.0;
                draw_text_ex(
                    "NEXT",
                    36.0,
                    chip_row_top + 17.0,
                    TextParams {
                        font: Some(theme.sans),
                        font_size: 12,
                        color: palette::MUTED,
                        ..Default::default()
                    },
                );
                let mut chip_x = 84.0;
                let max_chip_right = screen_w - 28.0;
                for upcoming in &expanded.upcoming {
                    let chip_text_w = measure_text(upcoming, Some(theme.mono), 14, 1.0).width;
                    if chip_x + chip_text_w + 20.0 > max_chip_right {
                        break;
                    }
                    let chip_w = draw_pill_badge(
                        chip_x,
                        chip_row_top,
                        24.0,
                        10.0,
                        0.0,
                        upcoming,
                        theme.mono,
                        14,
                        palette::PANEL,
                        Some(palette::RULE),
                        palette::TEXT,
                    );
                    chip_x += chip_w + 10.0;
                }
            }

            expanded.y + expanded.height
        } else {
            row.y + DEPARTURE_ROW_HEIGHT
        };

        draw_line(20.0, rule_y, screen_w - 20.0, rule_y, 1.0, palette::RULE);
    }

    computed.bottom
}

#[derive(Debug, Clone, PartialEq)]
pub struct StandingsPanelDisplay {
    pub header: String,
    pub lines: Vec<String>,
    pub stale_note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StandingsZone {
    Leader,
    European,
    MidTable,
    Relegation,
}

pub fn classify_standings_zone(position: u32, total_rows: usize) -> StandingsZone {
    if position == 1 {
        StandingsZone::Leader
    } else if (2..=4).contains(&position) {
        StandingsZone::European
    } else if total_rows >= 6 && (position as usize) > total_rows - 2 {
        StandingsZone::Relegation
    } else {
        StandingsZone::MidTable
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StandingsRenderContext<'a> {
    pub competition_label: &'a str,
    pub panel_count: usize,
    pub active_index: usize,
    pub is_paused: bool,
    pub rotation_progress: f32,
    pub flip: Option<&'a PanelFlipTransition>,
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

fn standings_show_text(flip_scale: f32, table_w: f32) -> bool {
    flip_scale >= 0.45 && table_w >= 320.0
}

pub fn render_standings_panel_rich(
    ctx: StandingsRenderContext<'_>,
    state: &StandingsPanelState,
    top: f32,
    theme: &Theme,
) {
    use macroquad::shapes::{draw_circle, draw_line, draw_rectangle};
    use macroquad::text::{TextParams, draw_text_ex, measure_text};
    use macroquad::window::screen_width;

    let screen_w = screen_width();
    let is_stale = matches!(state, StandingsPanelState::Stale { .. });
    let is_unavailable = matches!(state, StandingsPanelState::Unavailable);
    let base_color = status_color(is_stale, is_unavailable);

    let (rows, stale_age_min): (&[StandingsRow], Option<i64>) = match state {
        StandingsPanelState::Fresh(rows) => (rows.as_slice(), None),
        StandingsPanelState::Stale { rows, age } => (rows.as_slice(), Some(age.whole_minutes())),
        StandingsPanelState::Unavailable => (&[][..], None),
    };

    let competition_name = rows
        .first()
        .map(|row| row.competition.as_str())
        .unwrap_or(ctx.competition_label);
    let season_label = rows
        .first()
        .map(|row| row.season.as_str())
        .filter(|season| !season.is_empty());

    let card_x = 16.0;
    let card_w = (screen_w - 32.0).max(1.0);
    draw_rectangle(
        card_x,
        top,
        card_w,
        STANDINGS_HEADER_HEIGHT,
        palette::SURFACE,
    );
    draw_line(
        card_x,
        top + STANDINGS_HEADER_HEIGHT,
        card_x + card_w,
        top + STANDINGS_HEADER_HEIGHT,
        1.0,
        palette::RULE,
    );

    let mut header_cursor_x = card_x + 16.0;
    draw_text_ex(
        competition_name,
        header_cursor_x,
        top + 32.0,
        TextParams {
            font: Some(theme.sans),
            font_size: 24,
            color: base_color,
            ..Default::default()
        },
    );
    header_cursor_x += measure_text(competition_name, Some(theme.sans), 24, 1.0).width + 14.0;

    if let Some(season) = season_label {
        let season_w = draw_pill_badge(
            header_cursor_x,
            top + 12.0,
            24.0,
            10.0,
            0.0,
            season,
            theme.mono,
            14,
            palette::PANEL,
            Some(palette::RULE),
            palette::MUTED,
        );
        header_cursor_x += season_w + 10.0;
    }

    if let Some(age_min) = stale_age_min {
        let stale_text = format!("stale {age_min}m");
        draw_status_dot_pill(
            header_cursor_x,
            top + 12.0,
            24.0,
            &stale_text,
            theme.sans,
            13,
            palette::STALE,
            with_alpha(palette::STALE, 0.14),
            with_alpha(palette::STALE, 0.55),
            palette::STALE,
        );
    }

    let mut right_cursor_x = card_x + card_w - 16.0;
    if ctx.is_paused {
        let paused_text = "PAUSED · tap";
        let paused_w = measure_text(paused_text, Some(theme.sans), 14, 1.0).width + 20.0;
        let paused_x = right_cursor_x - paused_w;
        draw_pill_badge(
            paused_x,
            top + 11.0,
            26.0,
            10.0,
            0.0,
            paused_text,
            theme.sans,
            14,
            with_alpha(palette::STALE, 0.16),
            Some(with_alpha(palette::STALE, 0.60)),
            palette::STALE,
        );
        right_cursor_x = paused_x - 16.0;
    } else if ctx.panel_count > 1 {
        let bar_w = 84.0;
        let bar_h = 6.0;
        let bar_x = right_cursor_x - bar_w;
        let bar_y = top + (STANDINGS_HEADER_HEIGHT - bar_h) * 0.5;
        let progress = if ctx.rotation_progress.is_finite() {
            ctx.rotation_progress.clamp(0.0, 1.0)
        } else {
            0.0
        };
        draw_rectangle(bar_x, bar_y, bar_w, bar_h, palette::RULE);
        draw_rectangle(bar_x, bar_y, bar_w * progress, bar_h, palette::ACCENT_CYAN);
        right_cursor_x = bar_x - 16.0;
    }

    if ctx.panel_count > 1 {
        let dot_spacing = 14.0;
        let dot_radius = 4.0;
        let dots_total_w =
            (ctx.panel_count.saturating_sub(1) as f32) * dot_spacing + dot_radius * 2.0;
        let dots_start_x = right_cursor_x - dots_total_w + dot_radius;
        let dot_y = top + STANDINGS_HEADER_HEIGHT * 0.5;
        for idx in 0..ctx.panel_count {
            let dot_color = if idx == ctx.active_index {
                palette::ACCENT_CYAN
            } else {
                palette::RULE
            };
            draw_circle(
                dots_start_x + idx as f32 * dot_spacing,
                dot_y,
                dot_radius,
                dot_color,
            );
        }
    }

    let (flip_scale, flip_shade) = match ctx.flip {
        Some(flip) if flip.is_animating() => (flip.horizontal_scale(), flip.shade_factor()),
        _ => (1.0, 1.0),
    };
    let table_w = (card_w * flip_scale).max(1.0);
    let table_x = card_x + (card_w - table_w) * 0.5;
    let body_top = top + STANDINGS_STALE_HEADER_HEIGHT;

    if is_unavailable || rows.is_empty() {
        let empty_text = if is_unavailable {
            "standings unavailable"
        } else {
            "no standings"
        };
        let dot_color = if is_unavailable {
            palette::ALERT
        } else {
            palette::MUTED
        };
        draw_rectangle(
            table_x,
            body_top,
            table_w,
            48.0,
            shade_color(palette::PANEL, flip_shade),
        );
        if flip_scale >= 0.35 {
            draw_circle(
                table_x + 20.0,
                body_top + 24.0,
                4.5,
                shade_color(dot_color, flip_shade),
            );
            draw_text_ex(
                empty_text,
                table_x + 34.0,
                body_top + 31.0,
                TextParams {
                    font: Some(theme.sans),
                    font_size: 20,
                    color: shade_color(base_color, flip_shade),
                    ..Default::default()
                },
            );
        }
        return;
    }

    let col_header_h = 24.0;
    draw_rectangle(
        table_x,
        body_top,
        table_w,
        col_header_h,
        shade_color(palette::PANEL, flip_shade),
    );
    draw_line(
        table_x,
        body_top + col_header_h,
        table_x + table_w,
        body_top + col_header_h,
        1.0,
        shade_color(palette::RULE, flip_shade),
    );

    let table_right = table_x + table_w - 14.0;
    let pts_pill_w = 54.0;
    let pts_x = table_right - pts_pill_w;
    let gd_right_x = pts_x - 18.0;
    let played_right_x = gd_right_x - 58.0;
    let pos_x = table_x + 14.0;
    let team_x = table_x + 52.0;
    let show_text = standings_show_text(flip_scale, table_w);

    if show_text {
        let muted = shade_color(palette::MUTED, flip_shade);
        let header_y = body_top + 16.0;
        for (label, x) in [("#", pos_x), ("TEAM", team_x)] {
            draw_text_ex(
                label,
                x,
                header_y,
                TextParams {
                    font: Some(theme.sans),
                    font_size: 12,
                    color: muted,
                    ..Default::default()
                },
            );
        }
        let p_w = measure_text("P", Some(theme.sans), 12, 1.0).width;
        draw_text_ex(
            "P",
            played_right_x - p_w,
            header_y,
            TextParams {
                font: Some(theme.sans),
                font_size: 12,
                color: muted,
                ..Default::default()
            },
        );
        let gd_w = measure_text("GD", Some(theme.sans), 12, 1.0).width;
        draw_text_ex(
            "GD",
            gd_right_x - gd_w,
            header_y,
            TextParams {
                font: Some(theme.sans),
                font_size: 12,
                color: muted,
                ..Default::default()
            },
        );
        let pts_hdr_w = measure_text("PTS", Some(theme.sans), 12, 1.0).width;
        draw_text_ex(
            "PTS",
            pts_x + (pts_pill_w - pts_hdr_w) * 0.5,
            header_y,
            TextParams {
                font: Some(theme.sans),
                font_size: 12,
                color: muted,
                ..Default::default()
            },
        );
    }

    let visible_rows: Vec<&StandingsRow> = rows.iter().take(MAX_STANDINGS_ROWS_PER_PANEL).collect();
    let total_rows = rows.len();
    let mut row_y = body_top + col_header_h;

    for (idx, row) in visible_rows.iter().enumerate() {
        if idx % 2 == 1 {
            draw_rectangle(
                table_x,
                row_y,
                table_w,
                STANDINGS_ROW_HEIGHT,
                shade_color(palette::CARD_ALT, flip_shade),
            );
        }

        let zone = classify_standings_zone(row.position, total_rows);
        let zone_accent = match zone {
            StandingsZone::Leader => Some(palette::GOLD),
            StandingsZone::European => Some(palette::ACCENT_CYAN),
            StandingsZone::Relegation => Some(palette::ALERT),
            StandingsZone::MidTable => None,
        };
        if let Some(accent) = zone_accent {
            draw_rectangle(
                table_x,
                row_y,
                4.0,
                STANDINGS_ROW_HEIGHT,
                shade_color(accent, flip_shade),
            );
        }

        if show_text {
            let text_y = row_y + 19.0;
            let row_text_color = shade_color(base_color, flip_shade);
            let pos_color = zone_accent
                .map(|accent| shade_color(accent, flip_shade))
                .unwrap_or_else(|| shade_color(palette::MUTED, flip_shade));

            let pos_text = format!("{:>2}", row.position);
            draw_text_ex(
                &pos_text,
                pos_x,
                text_y,
                TextParams {
                    font: Some(theme.mono),
                    font_size: 16,
                    color: pos_color,
                    ..Default::default()
                },
            );

            let team_max_w = (played_right_x - 36.0 - team_x).max(0.0);
            let team_text = truncate_text_to_width(&row.team, team_max_w, theme.sans, 17);
            draw_text_ex(
                &team_text,
                team_x,
                text_y,
                TextParams {
                    font: Some(theme.sans),
                    font_size: 17,
                    color: row_text_color,
                    ..Default::default()
                },
            );

            let played_text = format!("{}", row.played);
            let played_w = measure_text(&played_text, Some(theme.mono), 16, 1.0).width;
            draw_text_ex(
                &played_text,
                played_right_x - played_w,
                text_y,
                TextParams {
                    font: Some(theme.mono),
                    font_size: 16,
                    color: shade_color(palette::MUTED, flip_shade),
                    ..Default::default()
                },
            );

            let gd_text = format!("{:+}", row.goal_difference);
            let gd_w = measure_text(&gd_text, Some(theme.mono), 16, 1.0).width;
            let gd_color = if is_stale {
                row_text_color
            } else if row.goal_difference > 0 {
                shade_color(palette::OK, flip_shade)
            } else if row.goal_difference < 0 {
                shade_color(palette::ALERT, flip_shade)
            } else {
                shade_color(palette::MUTED, flip_shade)
            };
            draw_text_ex(
                &gd_text,
                gd_right_x - gd_w,
                text_y,
                TextParams {
                    font: Some(theme.mono),
                    font_size: 16,
                    color: gd_color,
                    ..Default::default()
                },
            );

            let pts_text = format!("{}", row.points);
            let (pts_bg, pts_border, pts_text_color) = match zone_accent {
                Some(accent) if !is_stale => (
                    with_alpha(shade_color(accent, flip_shade), 0.18),
                    Some(with_alpha(shade_color(accent, flip_shade), 0.50)),
                    shade_color(palette::TEXT, flip_shade),
                ),
                _ => (
                    shade_color(palette::SURFACE, flip_shade),
                    Some(shade_color(palette::RULE, flip_shade)),
                    row_text_color,
                ),
            };
            draw_pill_badge(
                pts_x,
                row_y + 3.0,
                STANDINGS_ROW_HEIGHT - 6.0,
                6.0,
                pts_pill_w,
                &pts_text,
                theme.mono,
                16,
                pts_bg,
                pts_border,
                pts_text_color,
            );
        }

        row_y += STANDINGS_ROW_HEIGHT;
        draw_line(
            table_x,
            row_y,
            table_x + table_w,
            row_y,
            1.0,
            shade_color(palette::RULE, flip_shade),
        );
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

fn screensaver_clock_rect(screen_width: f32, screen_height: f32) -> (f32, f32, f32, f32) {
    let width = (screen_width * 0.80).clamp(320.0, 680.0);
    let height = 190.0;
    let x = centered_text_x(screen_width, width);
    let y = ((screen_height - height) / 2.0).max(20.0);
    (x, y, width, height)
}

pub fn render_screensaver_view_rich(
    clock: &LocalClock,
    split_flap: &SplitFlapBoard,
    availability: &CalendarAvailability,
    theme: &Theme,
) {
    use macroquad::text::{TextParams, draw_text_ex, measure_text};
    use macroquad::window::{screen_height, screen_width};

    let screen_w = screen_width();
    let screen_h = screen_height();

    render_screensaver_backdrop_3d(macroquad::time::get_time() as f32, screen_w, screen_h);

    let (x, y, width, height) = screensaver_clock_rect(screen_w, screen_h);

    let date = format_local_date(clock);
    let date_dims = measure_text(&date, Some(theme.sans), 28, 1.0);
    let date_x = centered_text_x(screen_w, date_dims.width);
    let date_y = (y - 28.0).max(32.0);
    draw_text_ex(
        &date,
        date_x,
        date_y,
        TextParams {
            font: Some(theme.sans),
            font_size: 28,
            color: palette::MUTED,
            ..Default::default()
        },
    );

    render_split_flap_clock_3d(split_flap, x, y, width, height);

    let hint = "Focus mode · tap anywhere to peek";
    let hint_dims = measure_text(hint, Some(theme.sans), 20, 1.0);
    let hint_x = centered_text_x(screen_w, hint_dims.width);
    let hint_y = (y + height + 44.0).min(screen_h - 48.0);
    draw_text_ex(
        hint,
        hint_x,
        hint_y,
        TextParams {
            font: Some(theme.sans),
            font_size: 20,
            color: palette::MUTED,
            ..Default::default()
        },
    );

    render_calendar_availability_note(availability, screen_h - 20.0, theme);
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
                height: STANDINGS_HEADER_HEIGHT,
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
    fn weather_precip_probability_extracts_reading_or_defaults_to_zero() {
        let fresh = WeatherPanelState::Fresh {
            reading: WeatherReading {
                temp_c: 3.4,
                precip_probability_pct: 45,
                observed_at: datetime!(2026-11-24 20:00:00 UTC),
            },
            age: Duration::minutes(2),
        };
        let stale = WeatherPanelState::Stale {
            reading: WeatherReading {
                temp_c: -1.0,
                precip_probability_pct: 70,
                observed_at: datetime!(2026-11-24 20:00:00 UTC),
            },
            age: Duration::minutes(45),
        };

        assert_eq!(weather_precip_probability(&fresh), 45);
        assert_eq!(weather_precip_probability(&stale), 70);
        assert_eq!(
            weather_precip_probability(&WeatherPanelState::Unavailable),
            0
        );
    }

    #[test]
    fn hit_test_maps_stale_weather_text_region_at_360px_and_header_when_fresh() {
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

        assert_eq!(HEADER_HEIGHT, 96.0);
        assert_eq!(WEATHER_HEADER_X, 332.0);

        let stale_layout =
            compute_dashboard_layout(1200.0, true, &stations, None, false, 2, 0, "serie-a");
        assert_eq!(
            hit_test(&stale_layout, 360.0, 48.0),
            Some(TouchTarget::StalePanel {
                provider: "weather".to_string(),
            })
        );
        assert_eq!(
            hit_test(&stale_layout, 360.0, 88.0),
            Some(TouchTarget::StalePanel {
                provider: "weather".to_string(),
            })
        );
        assert_eq!(
            hit_test(&stale_layout, 300.0, 48.0),
            Some(TouchTarget::Header)
        );
        assert_eq!(
            hit_test(&stale_layout, 100.0, 48.0),
            Some(TouchTarget::Header)
        );

        let fresh_layout =
            compute_dashboard_layout(1200.0, false, &stations, None, false, 2, 0, "serie-a");
        assert_eq!(
            hit_test(&fresh_layout, 360.0, 48.0),
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

    #[test]
    fn daylight_strip_height_reserves_220px_for_solar_diorama_and_text_without_hit_target() {
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

        assert_eq!(DAYLIGHT_STRIP_HEIGHT, 220.0);

        let layout =
            compute_dashboard_layout(1200.0, false, &stations, None, false, 2, 0, "allsvenskan");
        assert_eq!(layout.standings_top - layout.daylight_top, 220.0);
        assert_eq!(hit_test(&layout, 100.0, layout.daylight_top + 180.0), None);
    }

    #[test]
    fn screensaver_clock_rect_centers_and_clamps_width_for_split_flap_3d() {
        assert_eq!(
            screensaver_clock_rect(1200.0, 1920.0),
            (260.0, 865.0, 680.0, 190.0)
        );
        let (x, y, width, height) = screensaver_clock_rect(600.0, 800.0);
        assert!((x - 60.0).abs() < 1e-3);
        assert_eq!(y, 305.0);
        assert!((width - 480.0).abs() < 1e-3);
        assert_eq!(height, 190.0);
        assert_eq!(
            screensaver_clock_rect(320.0, 480.0),
            (20.0, 145.0, 320.0, 190.0)
        );
    }

    #[test]
    fn format_local_date_formats_weekday_day_and_short_month() {
        let cases = [
            ((2026, 11, 28), "Sat 28 Nov"),
            ((2026, 1, 5), "Mon 5 Jan"),
            ((2024, 2, 29), "Thu 29 Feb"),
            ((2026, 7, 1), "Wed 1 Jul"),
            ((2026, 12, 31), "Thu 31 Dec"),
            ((2026, 5, 10), "Sun 10 May"),
        ];
        for ((year, month, day), expected) in cases {
            let local = LocalClock {
                year,
                month,
                day,
                hour: 12,
                minute: 0,
                second: 0,
            };
            assert_eq!(format_local_date(&local), expected);
        }
    }

    #[test]
    fn format_local_date_falls_back_to_iso_for_invalid_dates() {
        let invalid_month = LocalClock {
            year: 2026,
            month: 13,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        assert_eq!(format_local_date(&invalid_month), "2026-13-01");

        let invalid_day = LocalClock {
            year: 2026,
            month: 2,
            day: 30,
            hour: 0,
            minute: 0,
            second: 0,
        };
        assert_eq!(format_local_date(&invalid_day), "2026-02-30");

        let out_of_i8 = LocalClock {
            year: 2026,
            month: 200,
            day: 99,
            hour: 0,
            minute: 0,
            second: 0,
        };
        assert_eq!(format_local_date(&out_of_i8), "2026-200-99");
    }

    #[test]
    fn classify_countdown_urgency_maps_too_late_leave_now_and_comfortable() {
        assert_eq!(classify_countdown_urgency(-5), CountdownUrgency::TooLate);
        assert_eq!(classify_countdown_urgency(-2), CountdownUrgency::TooLate);
        assert_eq!(classify_countdown_urgency(-1), CountdownUrgency::LeaveNow);
        assert_eq!(classify_countdown_urgency(0), CountdownUrgency::LeaveNow);
        assert_eq!(classify_countdown_urgency(1), CountdownUrgency::Comfortable);
        assert_eq!(
            classify_countdown_urgency(12),
            CountdownUrgency::Comfortable
        );
    }

    #[test]
    fn daylight_metric_cells_returns_five_labeled_cells_with_daylight_progress() {
        use jiff::civil::date;

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

        let cells = daylight_metric_cells(&window, 0.5);
        assert_eq!(cells[0], ("DAWN", "01:58".to_string()));
        assert_eq!(cells[1], ("SUNRISE", "03:31".to_string()));
        assert_eq!(cells[2].0, "DAYLIGHT");
        assert!(cells[2].1.contains("18h 38m daylight"));
        assert!(cells[2].1.contains("50%"));
        assert_eq!(cells[3], ("SUNSET", "22:09".to_string()));
        assert_eq!(cells[4], ("DUSK", "23:41".to_string()));

        assert!(daylight_metric_cells(&window, -0.5)[2].1.contains("0%"));
        assert!(daylight_metric_cells(&window, 1.5)[2].1.contains("100%"));
        assert!(daylight_metric_cells(&window, f32::NAN)[2].1.contains("0%"));
    }

    #[test]
    fn classify_standings_zone_maps_leader_european_midtable_and_relegation() {
        assert_eq!(classify_standings_zone(1, 20), StandingsZone::Leader);
        assert_eq!(classify_standings_zone(2, 20), StandingsZone::European);
        assert_eq!(classify_standings_zone(4, 20), StandingsZone::European);
        assert_eq!(classify_standings_zone(5, 20), StandingsZone::MidTable);
        assert_eq!(classify_standings_zone(18, 20), StandingsZone::MidTable);
        assert_eq!(classify_standings_zone(19, 20), StandingsZone::Relegation);
        assert_eq!(classify_standings_zone(20, 20), StandingsZone::Relegation);

        assert_eq!(classify_standings_zone(1, 6), StandingsZone::Leader);
        assert_eq!(classify_standings_zone(4, 6), StandingsZone::European);
        assert_eq!(classify_standings_zone(5, 6), StandingsZone::Relegation);
        assert_eq!(classify_standings_zone(6, 6), StandingsZone::Relegation);

        assert_eq!(classify_standings_zone(5, 5), StandingsZone::MidTable);
    }

    #[test]
    fn daylight_cell_bounds_weights_center_cell_at_36_percent() {
        let bounds = daylight_cell_bounds(20.0, 760.0);
        assert!((bounds[0].0 - 20.0).abs() < 1e-4);
        assert!((bounds[0].1 - 121.6).abs() < 1e-3);
        assert!((bounds[1].1 - 121.6).abs() < 1e-3);
        assert!((bounds[2].1 - 273.6).abs() < 1e-3);
        assert!((bounds[3].1 - 121.6).abs() < 1e-3);
        assert!((bounds[4].1 - 121.6).abs() < 1e-3);

        for pair in bounds.windows(2) {
            assert!((pair[0].0 + pair[0].1 - pair[1].0).abs() < 1e-4);
        }
        assert!((bounds[4].0 + bounds[4].1 - 780.0).abs() < 1e-3);

        let zero_bounds = daylight_cell_bounds(20.0, -10.0);
        assert!(zero_bounds.iter().all(|&(x, w)| x == 20.0 && w == 0.0));
        let nan_bounds = daylight_cell_bounds(20.0, f32::NAN);
        assert!(nan_bounds.iter().all(|&(x, w)| x == 20.0 && w == 0.0));
    }

    #[test]
    fn fit_weather_header_pills_tightens_and_omits_overflowing_pills() {
        assert_eq!(
            fit_weather_header_pills(500.0, 1180.0, 110.0, 120.0),
            (Some(518.0), Some(640.0))
        );

        let (tight_precip, tight_fresh) = fit_weather_header_pills(522.0, 780.0, 109.0, 131.0);
        assert_eq!(tight_precip, Some(530.0));
        assert_eq!(tight_fresh, Some(647.0));
        assert!(tight_fresh.unwrap() + 131.0 <= 780.0);

        assert_eq!(
            fit_weather_header_pills(522.0, 650.0, 109.0, 131.0),
            (Some(530.0), None)
        );
        assert_eq!(
            fit_weather_header_pills(522.0, 620.0, 109.0, 80.0),
            (None, Some(530.0))
        );
        assert_eq!(
            fit_weather_header_pills(522.0, 560.0, 109.0, 80.0),
            (None, None)
        );
    }

    #[test]
    fn standings_show_text_requires_both_flip_scale_and_minimum_table_width() {
        assert!(standings_show_text(0.45, 320.0));
        assert!(standings_show_text(1.0, 600.0));
        assert!(!standings_show_text(0.44, 600.0));
        assert!(!standings_show_text(0.45, 319.9));
        assert!(!standings_show_text(1.0, 280.0));
    }

    #[test]
    fn stale_standings_hit_region_matches_48px_header_bar_height_exactly() {
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
            compute_dashboard_layout(1200.0, false, &stations, None, true, 2, 0, "serie-a");
        let stale_region = layout
            .hit_regions
            .iter()
            .find(|region| matches!(region.target, TouchTarget::StalePanel { .. }))
            .expect("stale standings hit region present");

        assert_eq!(stale_region.rect.y, layout.standings_top);
        assert_eq!(stale_region.rect.height, STANDINGS_HEADER_HEIGHT);
        assert_eq!(
            hit_test(&layout, 100.0, layout.standings_top + 47.5),
            Some(TouchTarget::StalePanel {
                provider: "standings:serie-a".to_string(),
            })
        );
        assert_eq!(hit_test(&layout, 100.0, layout.standings_top + 50.0), None);
        assert_eq!(
            hit_test(
                &layout,
                100.0,
                layout.standings_top + STANDINGS_STALE_HEADER_HEIGHT
            ),
            Some(TouchTarget::FootballPanel {
                panel_count: 2,
                current_index: 0,
            })
        );
    }
}
