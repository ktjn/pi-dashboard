use crate::clock::LocalClock;
use crate::daylight::DaylightWindow;
use crate::departures::DeparturesPanelState;
use crate::screensaver::CalendarAvailability;
use crate::standings::StandingsPanelState;
use crate::theme::{Theme, palette};
use crate::weather::WeatherPanelState;

/// Text content for the clock + weather header, factored out of the
/// macroquad drawing calls so the formatting logic is unit-testable
/// without a window/GL context.
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
        WeatherPanelState::Fresh { reading, .. } => format!(
            "{:.0}°C  {}% rain",
            reading.temp_c, reading.precip_probability_pct
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

/// Draws the clock + weather header. Keep this thin: all text content
/// comes from `format_clock_weather_header`, which is covered by unit
/// tests that don't need a window.
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

    draw_rectangle(0.0, 0.0, screen_width(), 80.0, palette::PANEL);
    draw_line(0.0, 80.0, screen_width(), 80.0, 1.0, palette::RULE);

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

    let weather_color = match weather_state {
        WeatherPanelState::Fresh { .. } => palette::TEXT,
        WeatherPanelState::Stale { .. } => palette::STALE,
        WeatherPanelState::Unavailable => palette::DOWN,
    };
    draw_text_ex(
        &display.weather_text,
        20.0 + time_width + WEATHER_GAP,
        52.0,
        TextParams {
            font: Some(theme.sans),
            font_size: 32,
            color: weather_color,
            ..Default::default()
        },
    );
}

/// Text content for the daylight strip, factored out of the macroquad
/// drawing calls for the same reason as `format_clock_weather_header`.
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

/// Draws the daylight strip. Keep this thin: all text content comes from
/// `format_daylight_strip`, which is covered by unit tests that don't need
/// a window.
pub fn render_daylight_strip(window: &DaylightWindow, theme: &Theme) {
    use macroquad::shapes::{draw_line, draw_rectangle};
    use macroquad::text::{TextParams, draw_text_ex};
    use macroquad::window::screen_width;

    const STRIP_Y: f32 = 80.0;
    const STRIP_HEIGHT: f32 = 40.0;

    let display = format_daylight_strip(window);

    draw_rectangle(0.0, STRIP_Y, screen_width(), STRIP_HEIGHT, palette::PANEL);
    draw_line(
        0.0,
        STRIP_Y + STRIP_HEIGHT,
        screen_width(),
        STRIP_Y + STRIP_HEIGHT,
        1.0,
        palette::RULE,
    );
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
        STRIP_Y + 28.0,
        TextParams {
            font: Some(theme.mono),
            font_size: 22,
            color: palette::TEXT,
            ..Default::default()
        },
    );
}

/// One formatted line per departure, factored out of the macroquad drawing
/// calls for the same reason as the other panels. Elapsed departures are
/// never passed in here - `DeparturesBoard::station_state` already removed
/// them, so this only ever formats what should be visible.
#[derive(Debug, Clone, PartialEq)]
pub struct DeparturesPanelDisplay {
    pub lines: Vec<String>,
    pub stale_note: Option<String>,
}

/// Caps how many departure rows a single station panel draws, mirroring the
/// reference prototype (`sl-dashboard/src/components/TransportSection.tsx`:
/// `departures.slice(0, 6)`). Without this, a station with many departures
/// (confirmed live: one real SL site returned 47 bus + 30 metro departures
/// in a single response) overdraws the next station's panel.
pub const MAX_DEPARTURES_PER_PANEL: usize = 6;

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

    // `departures` is already sorted nearest-first by `DeparturesBoard`, so
    // capping to the first N keeps the soonest departures.
    let mut lines: Vec<String> = departures
        .iter()
        .take(MAX_DEPARTURES_PER_PANEL)
        .map(|departure| {
            // `countdown_min` truncates toward zero rather than rounding
            // (deliberate: a consistently-conservative "leave by" estimate
            // vs. the prototype's `Math.round`).
            let countdown = if departure.countdown_min < -1 {
                "För sent".to_string()
            } else if departure.countdown_min <= 0 {
                "Gå nu".to_string()
            } else {
                format!("Gå om {} min", departure.countdown_min)
            };
            let disruption = departure
                .disruption
                .as_deref()
                .map(|text| format!("  ⚠ {text}"))
                .unwrap_or_default();
            format!(
                "{} {}  {}{}",
                departure.line, departure.destination, countdown, disruption
            )
        })
        .collect();

    // A station with genuinely zero matching departures right now must
    // remain visibly distinct from `Unavailable` (dead feed), per the
    // spec's "distinguish a successful-empty result from an unavailable
    // feed."
    if lines.is_empty() {
        lines.push("no departures".to_string());
    }

    DeparturesPanelDisplay { lines, stale_note }
}

/// Vertical space one station panel needs: the top offset before the
/// first line, an optional stale-note line, and up to
/// `MAX_DEPARTURES_PER_PANEL` departure lines, plus a little bottom
/// padding so adjacent panels never overdraw each other. Callers that lay
/// out multiple stations (`dashboard-pi`/`dashboard-web` `main.rs`) should
/// advance by this amount per station rather than a hand-picked constant.
pub const STATION_PANEL_HEIGHT: f32 = 260.0;

/// Draws the departures panel. Keep this thin: all text content comes from
/// `format_departures_panel`, which is covered by unit tests that don't
/// need a window.
pub fn render_departures_panel(state: &DeparturesPanelState, top: f32, theme: &Theme) {
    use macroquad::shapes::draw_line;
    use macroquad::text::{TextParams, draw_text_ex};
    use macroquad::window::screen_width;

    const LINE_HEIGHT: f32 = 32.0;

    let display = format_departures_panel(state);

    let mut y = top + 24.0;
    if let Some(note) = &display.stale_note {
        draw_text_ex(
            note,
            20.0,
            y,
            TextParams {
                font: Some(theme.sans),
                font_size: 22,
                color: palette::STALE,
                ..Default::default()
            },
        );
        y += LINE_HEIGHT;
    }

    let color = match state {
        DeparturesPanelState::Fresh(_) => palette::TEXT,
        DeparturesPanelState::Stale { .. } => palette::STALE,
        DeparturesPanelState::Unavailable => palette::DOWN,
    };

    for line in &display.lines {
        draw_text_ex(
            line,
            20.0,
            y,
            TextParams {
                font: Some(theme.mono),
                font_size: 26,
                color,
                ..Default::default()
            },
        );
        y += LINE_HEIGHT * 0.6;
        draw_line(20.0, y, screen_width() - 20.0, y, 1.0, palette::RULE);
        y += LINE_HEIGHT * 0.4;
    }
}

/// One formatted line per standings row, factored out of the macroquad
/// drawing calls for the same reason as the other panels. `header` carries
/// the competition name and season (from the table itself when available,
/// so it keeps identifying its season during the off-season), falling back
/// to the configured competition id when no table has ever been fetched.
#[derive(Debug, Clone, PartialEq)]
pub struct StandingsPanelDisplay {
    pub header: String,
    pub lines: Vec<String>,
    pub stale_note: Option<String>,
}

/// Caps how many standings rows a single panel draws, mirroring
/// `MAX_DEPARTURES_PER_PANEL`'s precedent. Not a current overflow risk at
/// the target 1200x1920 resolution (Serie A's 20 rows land around
/// y≈1272), but defends against a future competition with more teams
/// ever overdrawing past the panel.
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

    // Rendered in the exact order `StandingsBoard` stored them, which is
    // the provider's own ranking (including tied points) - never
    // re-sorted here.
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

/// Draws the currently-rotated competition's standings panel. Keep this
/// thin: all text content comes from `format_standings_panel`, which is
/// covered by unit tests that don't need a window.
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

    let color = match state {
        StandingsPanelState::Fresh(_) => palette::TEXT,
        StandingsPanelState::Stale { .. } => palette::STALE,
        StandingsPanelState::Unavailable => palette::DOWN,
    };

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

/// Text content for the screensaver view, factored out of the macroquad
/// drawing calls for the same reason as the other panels. Per the spec's
/// resolved decision, v1 content is clock-only (photo rotation deferred);
/// `availability_note` surfaces the "calendar unavailable" indicator
/// (point 4 of the PI-07 slice) without flipping the screensaver on/off
/// state itself.
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

    let availability_note = match availability {
        CalendarAvailability::Fresh => None,
        CalendarAvailability::Stale { age } => {
            Some(format!("calendar unavailable ({}m)", age.whole_minutes()))
        }
        CalendarAvailability::Unavailable => Some("calendar unavailable".to_string()),
    };

    ScreensaverDisplay {
        time_text,
        availability_note,
    }
}

/// Draws the screensaver's clock-only view, replacing the normal layout
/// entirely while active. Keep this thin: all text content comes from
/// `format_screensaver_view`, which is covered by unit tests that don't
/// need a window.
pub fn render_screensaver_view(
    clock: &LocalClock,
    availability: &CalendarAvailability,
    theme: &Theme,
) {
    use macroquad::shapes::draw_rectangle;
    use macroquad::text::{TextParams, draw_text_ex};
    use macroquad::window::{screen_height, screen_width};

    let display = format_screensaver_view(clock, availability);

    draw_rectangle(0.0, 0.0, screen_width(), screen_height(), palette::INK);

    let center_x = screen_width() / 2.0 - 120.0;
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

    if let Some(note) = &display.availability_note {
        draw_text_ex(
            note,
            20.0,
            screen_height() - 20.0,
            TextParams {
                font: Some(theme.sans),
                font_size: 22,
                color: palette::MUTED,
                ..Default::default()
            },
        );
    }
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
        assert_eq!(display.weather_text, "3°C  20% rain");
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
        let window = DaylightWindow {
            date: date(2026, 6, 21),
            civil_dawn: date(2026, 6, 21).at(1, 58, 0, 0),
            sunrise: date(2026, 6, 21).at(3, 31, 0, 0),
            sunset: date(2026, 6, 21).at(22, 9, 0, 0),
            civil_dusk: date(2026, 6, 21).at(23, 41, 0, 0),
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
}
