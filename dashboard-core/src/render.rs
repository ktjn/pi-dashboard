use crate::clock::LocalClock;
use crate::daylight::DaylightWindow;
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
}
