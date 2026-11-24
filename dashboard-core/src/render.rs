use crate::clock::LocalClock;
use crate::daylight::DaylightWindow;
use crate::departures::DeparturesPanelState;
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
pub fn render_clock_weather_header(clock: &LocalClock, weather_state: &WeatherPanelState) {
    use macroquad::color::{BLACK, GRAY, ORANGE, WHITE};
    use macroquad::shapes::draw_rectangle;
    use macroquad::text::draw_text;
    use macroquad::window::screen_width;

    let display = format_clock_weather_header(clock, weather_state);

    draw_rectangle(0.0, 0.0, screen_width(), 80.0, BLACK);
    draw_text(&display.time_text, 20.0, 50.0, 40.0, WHITE);

    let weather_color = match weather_state {
        WeatherPanelState::Fresh { .. } => WHITE,
        WeatherPanelState::Stale { .. } => ORANGE,
        WeatherPanelState::Unavailable => GRAY,
    };
    draw_text(&display.weather_text, 220.0, 50.0, 40.0, weather_color);
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
pub fn render_daylight_strip(window: &DaylightWindow) {
    use macroquad::color::{DARKGRAY, WHITE};
    use macroquad::shapes::draw_rectangle;
    use macroquad::text::draw_text;
    use macroquad::window::screen_width;

    const STRIP_Y: f32 = 80.0;
    const STRIP_HEIGHT: f32 = 40.0;

    let display = format_daylight_strip(window);

    draw_rectangle(0.0, STRIP_Y, screen_width(), STRIP_HEIGHT, DARKGRAY);
    let text = format!(
        "Dawn {}  Sunrise {}  Sunset {}  Dusk {}  ({})",
        display.civil_dawn_text,
        display.sunrise_text,
        display.sunset_text,
        display.civil_dusk_text,
        display.duration_text
    );
    draw_text(&text, 20.0, STRIP_Y + 28.0, 24.0, WHITE);
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
pub fn render_departures_panel(state: &DeparturesPanelState, top: f32) {
    use macroquad::color::{GRAY, ORANGE, WHITE};
    use macroquad::text::draw_text;

    const LINE_HEIGHT: f32 = 32.0;

    let display = format_departures_panel(state);

    let mut y = top + 24.0;
    if let Some(note) = &display.stale_note {
        draw_text(note, 20.0, y, 22.0, ORANGE);
        y += LINE_HEIGHT;
    }

    let color = match state {
        DeparturesPanelState::Fresh(_) => WHITE,
        DeparturesPanelState::Stale { .. } => ORANGE,
        DeparturesPanelState::Unavailable => GRAY,
    };

    for line in &display.lines {
        draw_text(line, 20.0, y, 28.0, color);
        y += LINE_HEIGHT;
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
}
