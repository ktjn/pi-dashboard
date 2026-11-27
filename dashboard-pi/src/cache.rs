use crate::atomic_fs;
use dashboard_core::departures::DeparturesCacheSnapshot;
use dashboard_core::screensaver::CalendarCacheSnapshot;
use dashboard_core::standings::StandingsCacheSnapshot;
use dashboard_core::weather::WeatherCacheSnapshot;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DashboardCache {
    pub weather: Option<WeatherCacheSnapshot>,
    pub departures: DeparturesCacheSnapshot,
    pub standings: StandingsCacheSnapshot,
    pub calendar: CalendarCacheSnapshot,
}

pub fn save_cache_atomic(path: &Path, cache: &DashboardCache) -> io::Result<()> {
    let bytes = serde_json::to_vec(cache).map_err(io::Error::other)?;
    atomic_fs::write_bytes_atomic(path, &bytes)
}

pub fn load_cache(path: &Path) -> Option<DashboardCache> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_core::board_config::{Station, TransitMode};
    use dashboard_core::departures::{
        DEPARTURES_STALE_AFTER, DeparturesBoard, DeparturesPanelState, RawDeparture, RawDeviation,
    };
    use dashboard_core::screensaver::{
        CALENDAR_STALE_AFTER, CalendarAvailability, CalendarPollState,
    };
    use dashboard_core::standings::{
        STANDINGS_STALE_AFTER, StandingsBoard, StandingsPanelState, StandingsRow,
    };
    use dashboard_core::weather::{
        WEATHER_STALE_AFTER, WeatherPanelState, WeatherReading, WeatherSnapshotStore,
    };
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use time::{Duration, OffsetDateTime};

    static CACHE_TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_cache_path(label: &str) -> PathBuf {
        let id = CACHE_TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "pi-dashboard-cache-test-{label}-{}-{id}.json",
            std::process::id()
        ))
    }

    fn base_time() -> OffsetDateTime {
        OffsetDateTime::from_unix_timestamp(1_795_550_400).expect("valid timestamp")
    }

    fn sample_station() -> Station {
        Station {
            site_id: 9324,
            name: "Duvbo".to_string(),
            mode: TransitMode::Metro,
            walk_time_min: 10,
            direction: "stockholm".to_string(),
        }
    }

    fn populate_stores(
        now: OffsetDateTime,
    ) -> (
        WeatherSnapshotStore,
        DeparturesBoard,
        StandingsBoard,
        CalendarPollState,
    ) {
        let mut weather = WeatherSnapshotStore::new();
        weather.record_success(
            WeatherReading {
                temp_c: 4.5,
                precip_probability_pct: 30,
                observed_at: now,
            },
            now,
        );

        let mut departures = DeparturesBoard::new();
        departures.record_departures_success(
            9324,
            vec![RawDeparture {
                destination: "Kungsträdgården".to_string(),
                direction: Some("Kungsträdgården".to_string()),
                transport_mode: TransitMode::Metro,
                line_designation: "10".to_string(),
                scheduled: now + Duration::hours(4),
                expected: Some(now + Duration::hours(4)),
            }],
            now,
        );
        departures.record_deviations_success_at(
            9324,
            vec![RawDeviation {
                deviation_case_id: 42,
                header: "Minor delay".to_string(),
                importance_level: 3,
                publish_from: Some(now - Duration::minutes(5)),
                publish_upto: Some(now + Duration::hours(1)),
                affected_modes: vec![TransitMode::Metro],
            }],
            now,
        );

        let mut standings = StandingsBoard::new();
        standings.record_success(
            "serie-a",
            vec![StandingsRow {
                competition: "Serie A".to_string(),
                season: "2026/2027".to_string(),
                position: 1,
                team: "Internazionale".to_string(),
                played: 12,
                goal_difference: 15,
                points: 30,
            }],
            now,
        );

        let mut calendar = CalendarPollState::new();
        calendar.record_success(true, now);

        (weather, departures, standings, calendar)
    }

    #[test]
    fn save_and_load_cache_round_trips_all_four_snapshots() {
        let path = unique_cache_path("roundtrip");
        let now = base_time();
        let (weather, departures, standings, calendar) = populate_stores(now);

        let original = DashboardCache {
            weather: weather.export_snapshot(),
            departures: departures.export_snapshot(),
            standings: standings.export_snapshot(),
            calendar: calendar.export_snapshot(),
        };

        save_cache_atomic(&path, &original).expect("cache save must succeed");
        let loaded = load_cache(&path).expect("cache load must succeed");
        assert_eq!(loaded, original);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn cold_boot_hydration_preserves_fresh_and_stale_states_across_all_stores() {
        let path = unique_cache_path("hydration");
        let fetched_at = base_time();
        let (weather, departures, standings, calendar) = populate_stores(fetched_at);

        let cache = DashboardCache {
            weather: weather.export_snapshot(),
            departures: departures.export_snapshot(),
            standings: standings.export_snapshot(),
            calendar: calendar.export_snapshot(),
        };
        save_cache_atomic(&path, &cache).expect("cache save must succeed");

        let loaded = load_cache(&path).expect("cache load must succeed");

        let mut restored_weather = WeatherSnapshotStore::new();
        if let Some(snapshot) = loaded.weather {
            restored_weather.restore_snapshot(snapshot);
        }
        let mut restored_departures = DeparturesBoard::new();
        restored_departures.restore_snapshot(loaded.departures);
        let mut restored_standings = StandingsBoard::new();
        restored_standings.restore_snapshot(loaded.standings);
        let mut restored_calendar = CalendarPollState::new();
        restored_calendar.restore_snapshot(loaded.calendar);

        let fresh_now = fetched_at + Duration::seconds(30);
        let station = sample_station();
        assert!(matches!(
            restored_weather.state(fresh_now),
            WeatherPanelState::Fresh { .. }
        ));
        assert!(matches!(
            restored_departures.station_state(&station, fresh_now),
            DeparturesPanelState::Fresh(_)
        ));
        assert!(matches!(
            restored_standings.competition_state("serie-a", fresh_now),
            StandingsPanelState::Fresh(_)
        ));
        assert_eq!(
            restored_calendar.availability(fresh_now),
            CalendarAvailability::Fresh
        );
        assert!(restored_calendar.screensaver_active());

        let weather_stale_now = fetched_at + WEATHER_STALE_AFTER + Duration::minutes(1);
        assert!(matches!(
            restored_weather.state(weather_stale_now),
            WeatherPanelState::Stale { .. }
        ));
        let departures_stale_now = fetched_at + DEPARTURES_STALE_AFTER + Duration::minutes(1);
        assert!(matches!(
            restored_departures.station_state(&station, departures_stale_now),
            DeparturesPanelState::Stale { .. }
        ));
        let standings_stale_now = fetched_at + STANDINGS_STALE_AFTER + Duration::minutes(1);
        assert!(matches!(
            restored_standings.competition_state("serie-a", standings_stale_now),
            StandingsPanelState::Stale { .. }
        ));
        let calendar_stale_now = fetched_at + CALENDAR_STALE_AFTER + Duration::minutes(1);
        assert!(matches!(
            restored_calendar.availability(calendar_stale_now),
            CalendarAvailability::Stale { .. }
        ));

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_cache_returns_none_for_missing_file() {
        let path = unique_cache_path("missing");
        assert!(load_cache(&path).is_none());
    }

    #[test]
    fn load_cache_returns_none_for_corrupt_json() {
        let path = unique_cache_path("corrupt");
        std::fs::write(&path, b"{not valid json").expect("fixture write must succeed");
        assert!(load_cache(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }
}
