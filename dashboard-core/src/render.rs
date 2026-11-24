use crate::clock::LocalClock;
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
}
