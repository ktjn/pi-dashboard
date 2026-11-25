use dashboard_core::clock::local_clock;
use dashboard_core::weather::{
    FixtureWeatherSource, WEATHER_STALE_AFTER, WeatherFetchError, WeatherPanelState,
    WeatherReading, WeatherSnapshotStore, WeatherSource, parse_open_meteo_response,
};
use time::macros::datetime;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{name}")).expect("fixture must exist")
}

#[test]
fn parses_current_temperature_and_next_hour_precipitation_probability() {
    let reading = parse_open_meteo_response(&fixture("weather_current.json"))
        .expect("valid response must parse");

    assert_eq!(reading.temp_c, 0.5);
    assert_eq!(reading.precip_probability_pct, 35);
    assert_eq!(reading.observed_at, datetime!(2026-11-24 20:00:00 UTC));
}

#[test]
fn falls_back_to_last_hourly_entry_when_no_later_entry_exists() {
    let reading = parse_open_meteo_response(&fixture("weather_last_hour_only.json"))
        .expect("valid response must parse");

    assert_eq!(reading.precip_probability_pct, 10);
}

#[test]
fn rejects_malformed_json() {
    let error = parse_open_meteo_response("{not json").unwrap_err();
    assert!(matches!(error, WeatherFetchError::InvalidJson(_)));
}

#[test]
fn rejects_response_with_empty_hourly_data() {
    let error = parse_open_meteo_response(&fixture("weather_empty_hourly.json")).unwrap_err();
    assert!(matches!(error, WeatherFetchError::MissingData(_)));
}

#[test]
fn rejects_out_of_range_precipitation_probability() {
    let error =
        parse_open_meteo_response(&fixture("weather_out_of_range_probability.json")).unwrap_err();
    assert!(matches!(error, WeatherFetchError::MissingData(_)));
}

#[test]
fn panel_is_unavailable_before_any_successful_fetch() {
    let store = WeatherSnapshotStore::new();
    let state = store.state(datetime!(2026-11-24 20:00:00 UTC));
    assert_eq!(state, WeatherPanelState::Unavailable);
}

#[test]
fn panel_is_fresh_immediately_after_a_successful_fetch() {
    let mut store = WeatherSnapshotStore::new();
    let reading = WeatherReading {
        temp_c: 1.0,
        precip_probability_pct: 20,
        observed_at: datetime!(2026-11-24 20:00:00 UTC),
    };
    let fetched_at = datetime!(2026-11-24 20:00:00 UTC);
    store.record_success(reading, fetched_at);

    assert_eq!(
        store.state(reading.observed_at),
        WeatherPanelState::Fresh {
            reading,
            age: time::Duration::ZERO,
        }
    );
}

#[test]
fn fresh_panel_reports_the_age_of_its_last_reading_measured_from_observed_at() {
    // observed_at and fetched_at deliberately differ: the HTTP call happened
    // 5 minutes after Open-Meteo's own reading timestamp. Age must be
    // measured from observed_at, not from fetched_at.
    let mut store = WeatherSnapshotStore::new();
    let reading = WeatherReading {
        temp_c: 1.0,
        precip_probability_pct: 20,
        observed_at: datetime!(2026-11-24 20:00:00 UTC),
    };
    let fetched_at = datetime!(2026-11-24 20:05:00 UTC);
    store.record_success(reading, fetched_at);

    let now = datetime!(2026-11-24 20:07:00 UTC);

    match store.state(now) {
        WeatherPanelState::Fresh {
            reading: observed,
            age,
        } => {
            assert_eq!(observed, reading);
            assert_eq!(age, now - reading.observed_at);
            assert_ne!(age, now - fetched_at);
        }
        other => panic!("expected Fresh, got {other:?}"),
    }
}

#[test]
fn panel_becomes_stale_once_the_reading_exceeds_the_staleness_threshold() {
    let mut store = WeatherSnapshotStore::new();
    let reading = WeatherReading {
        temp_c: 1.0,
        precip_probability_pct: 20,
        observed_at: datetime!(2026-11-24 20:00:00 UTC),
    };
    let fetched_at = datetime!(2026-11-24 20:00:00 UTC);
    store.record_success(reading, fetched_at);

    let now = reading.observed_at + WEATHER_STALE_AFTER + time::Duration::seconds(1);

    assert_eq!(
        store.state(now),
        WeatherPanelState::Stale {
            reading,
            age: now - reading.observed_at,
        }
    );
}

#[test]
fn clock_keeps_advancing_through_an_hour_boundary_while_the_weather_endpoint_is_offline() {
    // A prior successful fetch is cached...
    let mut store = WeatherSnapshotStore::new();
    let reading = WeatherReading {
        temp_c: -2.0,
        precip_probability_pct: 5,
        observed_at: datetime!(2026-11-24 23:50:00 UTC),
    };
    store.record_success(reading, datetime!(2026-11-24 23:50:00 UTC));

    // ...and the weather endpoint is now offline.
    let mut offline_source = FixtureWeatherSource::new(vec![Err(WeatherFetchError::MissingData(
        "endpoint offline".to_string(),
    ))]);
    let fetch_result = offline_source.fetch();
    assert!(
        fetch_result.is_err(),
        "weather endpoint must be offline for this test"
    );

    let before_boundary = datetime!(2026-11-24 23:59:58 UTC);
    let after_boundary = datetime!(2026-11-25 00:00:02 UTC);

    let clock_before = local_clock("UTC", before_boundary).expect("UTC must resolve");
    let clock_after = local_clock("UTC", after_boundary).expect("UTC must resolve");

    assert_eq!(clock_before.hour, 23);
    assert_eq!(clock_before.minute, 59);
    assert_eq!(clock_after.hour, 0);
    assert_eq!(clock_after.day, 25);

    // The weather panel still shows the last known reading, consistently,
    // on both sides of the hour boundary - never Unavailable just because
    // the clock ticked over.
    for now in [before_boundary, after_boundary] {
        match store.state(now) {
            WeatherPanelState::Fresh { reading: r, .. }
            | WeatherPanelState::Stale { reading: r, .. } => {
                assert_eq!(r, reading);
            }
            WeatherPanelState::Unavailable => {
                panic!("expected the cached reading to still be shown while offline")
            }
        }
    }
}

#[test]
fn fixture_weather_source_replays_configured_responses_then_repeats_the_last_one() {
    let first = WeatherReading {
        temp_c: 3.0,
        precip_probability_pct: 10,
        observed_at: datetime!(2026-11-24 20:00:00 UTC),
    };
    let second = WeatherReading {
        temp_c: 4.0,
        precip_probability_pct: 15,
        observed_at: datetime!(2026-11-24 21:00:00 UTC),
    };
    let mut source = FixtureWeatherSource::new(vec![Ok(first), Ok(second)]);

    assert_eq!(source.fetch().unwrap(), first);
    assert_eq!(source.fetch().unwrap(), second);
    assert_eq!(source.fetch().unwrap(), second);
}

#[test]
fn fixture_weather_source_can_simulate_a_failing_fetch() {
    let mut source = FixtureWeatherSource::new(vec![Err(WeatherFetchError::MissingData(
        "endpoint offline".to_string(),
    ))]);

    let error = source.fetch().unwrap_err();
    assert!(matches!(error, WeatherFetchError::MissingData(_)));
}
