use dashboard_core::board_config::{Station, TransitMode};
use dashboard_core::departures::{
    DEVIATIONS_STALE_AFTER, DepartureFetchError, DeparturesBoard, DeparturesCacheSnapshot,
    DeparturesPanelState, RawDeparture, RawDeviation, parse_sl_departures_response,
    parse_sl_deviations_response,
};
use time::macros::datetime;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{name}")).expect("fixture must exist")
}

fn duvbo(mode: TransitMode, walk_time_min: u32, direction: &str) -> Station {
    Station {
        site_id: 9324,
        name: "Duvbo".to_string(),
        mode,
        walk_time_min,
        direction: direction.to_string(),
    }
}

#[test]
fn parses_the_live_sl_departures_response_shape() {
    let departures =
        parse_sl_departures_response(&fixture("departures_duvbo.json"), "Europe/Stockholm")
            .expect("valid response must parse");

    assert_eq!(departures.len(), 3);
    let kungstradgarden = departures
        .iter()
        .find(|d| d.destination == "Kungsträdgården")
        .expect("Kungsträdgården departure must be present");
    assert_eq!(kungstradgarden.transport_mode, TransitMode::Metro);
    assert_eq!(kungstradgarden.line_designation, "10");
    // Stockholm is UTC+1 in late November (no DST), so 22:56:19 local is
    // 21:56:19 UTC.
    assert_eq!(
        kungstradgarden.expected,
        Some(datetime!(2026-11-24 21:56:19 UTC))
    );
    assert_eq!(
        kungstradgarden.scheduled,
        datetime!(2026-11-24 21:56:12 UTC)
    );
}

#[test]
fn rejects_malformed_json() {
    let error = parse_sl_departures_response("{not json", "Europe/Stockholm").unwrap_err();
    assert!(matches!(error, DepartureFetchError::InvalidJson(_)));
}

#[test]
fn skips_a_departure_with_an_unrecognized_transit_mode_instead_of_failing_the_batch() {
    let departures = parse_sl_departures_response(
        &fixture("departures_unrecognized_mode.json"),
        "Europe/Stockholm",
    )
    .expect("an unrecognized mode must not fail the whole response");
    assert!(
        departures.is_empty(),
        "the fixture's only departure has an unrecognized mode"
    );
}

#[test]
fn a_mixed_response_keeps_the_recognized_departure_and_drops_the_unrecognized_one() {
    let departures =
        parse_sl_departures_response(&fixture("departures_mixed_modes.json"), "Europe/Stockholm")
            .expect("one unrecognized mode among several must not fail the response");

    assert_eq!(departures.len(), 1);
    assert_eq!(departures[0].transport_mode, TransitMode::Bus);
    assert_eq!(departures[0].destination, "Solna");
}

#[test]
fn walking_time_countdown_reflects_the_configured_walk_time() {
    // Matches the spec's acceptance example literally: a departure 14
    // minutes away with a configured 10-minute walk reads as "4 min to
    // leave" ("Gå om 4 min").
    let departures =
        parse_sl_departures_response(&fixture("departures_duvbo.json"), "Europe/Stockholm")
            .expect("valid response must parse");

    let mut board = DeparturesBoard::new();
    let now = datetime!(2026-11-24 21:42:19 UTC);
    board.record_departures_success(9324, departures, now);

    let station = duvbo(TransitMode::Metro, 10, "stockholm");
    match board.station_state(&station, now) {
        DeparturesPanelState::Fresh(departures) => {
            assert_eq!(
                departures.len(),
                1,
                "only the Kungsträdgården departure matches direction=stockholm"
            );
            assert_eq!(departures[0].countdown_min, 4);
            assert_eq!(departures[0].walk_time_min, 10);
        }
        other => panic!("expected Fresh, got {other:?}"),
    }
}

#[test]
fn elapsed_departures_are_removed_from_view() {
    let departures =
        parse_sl_departures_response(&fixture("departures_duvbo.json"), "Europe/Stockholm")
            .expect("valid response must parse");

    let mut board = DeparturesBoard::new();
    let now = datetime!(2026-11-24 21:42:19 UTC);
    board.record_departures_success(9324, departures, now);

    // direction="all" so both Hjulsta-bound departures would otherwise be
    // visible; the one scheduled for 22:41 local (21:41 UTC) is already in
    // the past relative to `now` and must be filtered out, while the one
    // at 22:46 local (21:46 UTC) remains.
    let station = duvbo(TransitMode::Metro, 0, "all");
    match board.station_state(&station, now) {
        DeparturesPanelState::Fresh(departures) => {
            let hjulsta: Vec<_> = departures
                .iter()
                .filter(|d| d.destination == "Hjulsta")
                .collect();
            assert_eq!(
                hjulsta.len(),
                1,
                "the elapsed Hjulsta departure must be removed"
            );
            assert!(hjulsta[0].countdown_min >= 0);
        }
        other => panic!("expected Fresh, got {other:?}"),
    }
}

#[test]
fn filters_by_configured_mode() {
    let departures =
        parse_sl_departures_response(&fixture("departures_duvbo.json"), "Europe/Stockholm")
            .expect("valid response must parse");

    let mut board = DeparturesBoard::new();
    let now = datetime!(2026-11-24 21:42:19 UTC);
    board.record_departures_success(9324, departures, now);

    let station = duvbo(TransitMode::Train, 0, "all");
    match board.station_state(&station, now) {
        DeparturesPanelState::Fresh(departures) => {
            assert!(
                departures.is_empty(),
                "the fixture only has METRO departures"
            );
        }
        other => panic!("expected Fresh, got {other:?}"),
    }
}

#[test]
fn filters_by_configured_direction() {
    let departures =
        parse_sl_departures_response(&fixture("departures_duvbo.json"), "Europe/Stockholm")
            .expect("valid response must parse");

    let mut board = DeparturesBoard::new();
    let now = datetime!(2026-11-24 21:42:19 UTC);
    board.record_departures_success(9324, departures, now);

    let station = duvbo(TransitMode::Metro, 0, "hjulsta");
    match board.station_state(&station, now) {
        DeparturesPanelState::Fresh(departures) => {
            assert!(departures.iter().all(|d| d.destination == "Hjulsta"));
            assert!(!departures.is_empty());
        }
        other => panic!("expected Fresh, got {other:?}"),
    }
}

#[test]
fn panel_is_unavailable_before_any_successful_fetch() {
    let board = DeparturesBoard::new();
    let station = duvbo(TransitMode::Metro, 10, "stockholm");

    let state = board.station_state(&station, datetime!(2026-11-24 21:42:19 UTC));
    assert_eq!(state, DeparturesPanelState::Unavailable);
}

#[test]
fn panel_becomes_stale_once_departures_exceed_the_staleness_threshold() {
    let departures =
        parse_sl_departures_response(&fixture("departures_duvbo.json"), "Europe/Stockholm")
            .expect("valid response must parse");

    let mut board = DeparturesBoard::new();
    let fetched_at = datetime!(2026-11-24 21:42:19 UTC);
    board.record_departures_success(9324, departures, fetched_at);

    let now = fetched_at + time::Duration::minutes(2) + time::Duration::seconds(1);
    let station = duvbo(TransitMode::Metro, 10, "stockholm");

    match board.station_state(&station, now) {
        DeparturesPanelState::Stale { age, .. } => assert_eq!(age, now - fetched_at),
        other => panic!("expected Stale, got {other:?}"),
    }
}

#[test]
fn parses_the_live_sl_deviations_response_shape() {
    let deviations = parse_sl_deviations_response(&fixture("deviations_bus_closure.json"))
        .expect("valid response must parse");

    assert_eq!(deviations.len(), 1);
    let deviation = &deviations[0];
    assert_eq!(deviation.deviation_case_id, 93075231);
    assert_eq!(deviation.header, "Indragna hållplatser");
    assert_eq!(deviation.importance_level, 7);
    assert_eq!(deviation.affected_modes, vec![TransitMode::Bus]);
    assert_eq!(
        deviation.publish_from,
        Some(
            time::OffsetDateTime::parse(
                "2024-01-03T07:45:54.647+01:00",
                &time::format_description::well_known::Rfc3339
            )
            .unwrap()
        )
    );
}

#[test]
fn active_disruption_overlays_matching_mode_departures_and_skips_expired_ones() {
    let departures =
        parse_sl_departures_response(&fixture("departures_duvbo.json"), "Europe/Stockholm")
            .expect("valid departures response must parse");
    let deviations = parse_sl_deviations_response(&fixture("deviations_metro_disruption.json"))
        .expect("valid deviations response must parse");

    let mut board = DeparturesBoard::new();
    let now = datetime!(2026-11-24 21:42:19 UTC);
    board.record_departures_success(9324, departures, now);
    board.record_deviations_success(9324, deviations);

    let station = duvbo(TransitMode::Metro, 10, "stockholm");
    match board.station_state(&station, now) {
        DeparturesPanelState::Fresh(departures) => {
            assert!(!departures.is_empty());
            for departure in &departures {
                assert_eq!(
                    departure.disruption.as_deref(),
                    Some("Driftstörning på blå linjen"),
                    "the still-active metro disruption must be attached, the expired one skipped"
                );
            }
        }
        other => panic!("expected Fresh, got {other:?}"),
    }
}

#[test]
fn disruption_for_a_different_mode_is_not_surfaced() {
    let departures =
        parse_sl_departures_response(&fixture("departures_duvbo.json"), "Europe/Stockholm")
            .expect("valid departures response must parse");
    let deviations = parse_sl_deviations_response(&fixture("deviations_bus_closure.json"))
        .expect("valid deviations response must parse");

    let mut board = DeparturesBoard::new();
    let now = datetime!(2026-11-24 21:42:19 UTC);
    board.record_departures_success(9324, departures, now);
    board.record_deviations_success(9324, deviations);

    let station = duvbo(TransitMode::Metro, 10, "stockholm");
    match board.station_state(&station, now) {
        DeparturesPanelState::Fresh(departures) => {
            assert!(!departures.is_empty());
            for departure in &departures {
                assert_eq!(
                    departure.disruption, None,
                    "the bus-only disruption must not apply to METRO"
                );
            }
        }
        other => panic!("expected Fresh, got {other:?}"),
    }
}

#[test]
fn one_failing_site_does_not_affect_another_sites_panel() {
    let departures =
        parse_sl_departures_response(&fixture("departures_duvbo.json"), "Europe/Stockholm")
            .expect("valid response must parse");

    let mut board = DeparturesBoard::new();
    let now = datetime!(2026-11-24 21:42:19 UTC);
    // Site 9324 (Duvbo) gets a successful fetch recorded; site 9325
    // (Sundbyberg) never does, modeling its request having failed.
    board.record_departures_success(9324, departures, now);

    let duvbo_station = duvbo(TransitMode::Metro, 10, "stockholm");
    let sundbyberg_station = Station {
        site_id: 9325,
        name: "Sundbyberg".to_string(),
        mode: TransitMode::Train,
        walk_time_min: 15,
        direction: "stockholm".to_string(),
    };

    match board.station_state(&duvbo_station, now) {
        DeparturesPanelState::Fresh(departures) => assert!(!departures.is_empty()),
        other => panic!("expected Fresh for the site with a successful fetch, got {other:?}"),
    }
    assert_eq!(
        board.station_state(&sundbyberg_station, now),
        DeparturesPanelState::Unavailable,
        "the never-fetched site must stay Unavailable, independent of the other site's success"
    );
}

#[test]
fn fixture_sources_are_available_behind_the_fixtures_feature() {
    use dashboard_core::departures::{
        DeparturesSource, DeviationsSource, FixtureDeparturesSource, FixtureDeviationsSource,
    };
    use std::collections::HashMap;

    let raw = RawDeparture {
        destination: "Hjulsta".to_string(),
        direction: Some("Hjulsta".to_string()),
        transport_mode: TransitMode::Metro,
        line_designation: "10".to_string(),
        scheduled: datetime!(2026-11-24 21:46:18 UTC),
        expected: Some(datetime!(2026-11-24 21:46:18 UTC)),
    };
    let mut departures_responses = HashMap::new();
    departures_responses.insert(9324, Ok(vec![raw.clone()]));
    let mut departures_source = FixtureDeparturesSource::new(departures_responses);
    assert_eq!(departures_source.fetch_departures(9324).unwrap(), vec![raw]);
    assert!(departures_source.fetch_departures(1).is_err());

    let deviation = RawDeviation {
        deviation_case_id: 1,
        header: "test".to_string(),
        importance_level: 1,
        publish_from: None,
        publish_upto: None,
        affected_modes: vec![TransitMode::Metro],
    };
    let mut deviations_responses = HashMap::new();
    deviations_responses.insert(9324, Ok(vec![deviation.clone()]));
    let mut deviations_source = FixtureDeviationsSource::new(deviations_responses);
    assert_eq!(
        deviations_source.fetch_deviations(9324).unwrap(),
        vec![deviation]
    );
    // Unlike departures, an unconfigured site quietly yields no
    // deviations rather than an error: the 120s deviation feed is
    // best-effort and must not block the 30s departure feed.
    assert_eq!(deviations_source.fetch_deviations(1).unwrap(), Vec::new());
}

#[test]
fn undated_deviations_expire_once_their_fetch_exceeds_staleness_threshold() {
    let departures =
        parse_sl_departures_response(&fixture("departures_duvbo.json"), "Europe/Stockholm")
            .expect("valid departures response must parse");
    let fetched_at = datetime!(2026-11-24 21:42:19 UTC);
    let undated = RawDeviation {
        deviation_case_id: 101,
        header: "Undated warning".to_string(),
        importance_level: 5,
        publish_from: None,
        publish_upto: None,
        affected_modes: vec![TransitMode::Metro],
    };
    let dated = RawDeviation {
        deviation_case_id: 102,
        header: "Dated warning".to_string(),
        importance_level: 5,
        publish_from: Some(datetime!(2026-11-24 21:00:00 UTC)),
        publish_upto: Some(datetime!(2026-11-24 22:30:00 UTC)),
        affected_modes: vec![TransitMode::Metro],
    };

    let mut board = DeparturesBoard::new();
    board.record_departures_success(9324, departures, fetched_at);
    board.record_deviations_success_at(9324, vec![undated, dated], fetched_at);

    let station = duvbo(TransitMode::Metro, 10, "stockholm");
    let at_threshold = fetched_at + DEVIATIONS_STALE_AFTER;
    match board.station_state(&station, at_threshold) {
        DeparturesPanelState::Stale { departures, .. } => {
            assert_eq!(
                departures[0].disruption.as_deref(),
                Some("Undated warning; Dated warning")
            );
        }
        other => panic!("expected Stale departures at threshold, got {other:?}"),
    }

    let past_threshold = fetched_at + DEVIATIONS_STALE_AFTER + time::Duration::seconds(1);
    match board.station_state(&station, past_threshold) {
        DeparturesPanelState::Stale { departures, .. } => {
            assert_eq!(departures[0].disruption.as_deref(), Some("Dated warning"));
        }
        other => panic!("expected Stale departures past threshold, got {other:?}"),
    }
}

#[test]
fn snapshot_round_trips_through_json_preserving_departures_deviations_and_staleness() {
    let departures =
        parse_sl_departures_response(&fixture("departures_duvbo.json"), "Europe/Stockholm")
            .expect("valid departures response must parse");
    let deviations = parse_sl_deviations_response(&fixture("deviations_metro_disruption.json"))
        .expect("valid deviations response must parse");
    let fetched_at = datetime!(2026-11-24 21:42:19 UTC);

    let mut board = DeparturesBoard::new();
    board.record_departures_success(9324, departures, fetched_at);
    board.record_deviations_success_at(9324, deviations, fetched_at);

    let exported = board.export_snapshot();
    let json = serde_json::to_string(&exported).expect("must serialize");
    let decoded: DeparturesCacheSnapshot = serde_json::from_str(&json).expect("must deserialize");

    let mut restored = DeparturesBoard::new();
    restored.restore_snapshot(decoded);

    let station = duvbo(TransitMode::Metro, 10, "stockholm");
    match restored.station_state(&station, fetched_at + time::Duration::seconds(30)) {
        DeparturesPanelState::Fresh(items) => {
            assert_eq!(items.len(), 1);
            assert_eq!(
                items[0].disruption.as_deref(),
                Some("Driftstörning på blå linjen")
            );
        }
        other => panic!("expected Fresh after restore, got {other:?}"),
    }

    let stale_now = fetched_at + time::Duration::minutes(3);
    match restored.station_state(&station, stale_now) {
        DeparturesPanelState::Stale { departures, age } => {
            assert_eq!(departures.len(), 1);
            assert_eq!(age, time::Duration::minutes(3));
        }
        other => panic!("expected Stale after restore, got {other:?}"),
    }
}
