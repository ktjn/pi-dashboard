use dashboard_core::clock::local_clock;
use dashboard_core::daylight::daylight_window;
use dashboard_core::render::{render_clock_weather_header, render_daylight_strip};
use dashboard_core::weather::{
    FixtureWeatherSource, WeatherReading, WeatherSnapshotStore, WeatherSource,
};
use jiff::civil::Date;
use time::Duration;
use time::macros::datetime;

const STOCKHOLM_LAT: f64 = 59.3626;
const STOCKHOLM_LON: f64 = 17.9158;
const STOCKHOLM_TZ: &str = "Europe/Stockholm";

#[macroquad::main("pi-dashboard (playground)")]
async fn main() {
    // Fixture data in place of a real provider - the playground never
    // talks to a real network.
    let mut weather_source = FixtureWeatherSource::new(vec![Ok(WeatherReading {
        temp_c: 3.5,
        precip_probability_pct: 20,
        observed_at: datetime!(2026-11-24 20:00:00 UTC),
    })]);
    let mut weather_store = WeatherSnapshotStore::new();
    if let Ok(reading) = weather_source.fetch() {
        weather_store.record_success(reading, datetime!(2026-11-24 20:00:05 UTC));
    }

    let base_utc = datetime!(2026-11-24 20:00:00 UTC);
    let start_time = macroquad::time::get_time();

    let mut daylight_date: Option<Date> = None;
    let mut daylight = None;

    loop {
        let elapsed_seconds = (macroquad::time::get_time() - start_time).max(0.0) as i64;
        let now_utc = base_utc + Duration::seconds(elapsed_seconds);

        macroquad::window::clear_background(macroquad::color::BLACK);

        let clock = local_clock(STOCKHOLM_TZ, now_utc).expect("time zone must resolve");
        let weather_state = weather_store.state(now_utc);
        render_clock_weather_header(&clock, &weather_state);

        let today = Date::new(clock.year, clock.month as i8, clock.day as i8)
            .expect("local clock must yield a valid calendar date");
        if daylight_date != Some(today) {
            daylight = daylight_window(STOCKHOLM_TZ, STOCKHOLM_LAT, STOCKHOLM_LON, today).ok();
            daylight_date = Some(today);
        }
        if let Some(window) = &daylight {
            render_daylight_strip(window);
        }

        macroquad::window::next_frame().await;
    }
}
