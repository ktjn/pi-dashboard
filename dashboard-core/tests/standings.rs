use dashboard_core::standings::{
    FixtureStandingsSource, StandingsBoard, StandingsFetchError, StandingsPanelState,
    StandingsRotation, StandingsSource, parse_football_data_standings_response,
    parse_highlightly_standings_response,
};
use std::collections::HashMap;
use time::macros::datetime;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{name}")).expect("fixture must exist")
}

#[test]
fn parses_the_documented_football_data_org_standings_response_shape() {
    let rows =
        parse_football_data_standings_response(&fixture("standings_football_data_serie_a.json"))
            .expect("valid response must parse");

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].competition, "Serie A");
    assert_eq!(rows[0].season, "2025/2026");
    assert_eq!(rows[0].team, "Internazionale");
    assert_eq!(rows[0].position, 1);
    assert_eq!(rows[0].played, 13);
    assert_eq!(rows[0].points, 32);
    assert_eq!(rows[0].goal_difference, 18);
}

#[test]
fn football_data_org_parsing_only_reads_the_total_table_not_home_or_away() {
    let rows =
        parse_football_data_standings_response(&fixture("standings_football_data_serie_a.json"))
            .expect("valid response must parse");

    // The fixture's HOME table has a single Juventus row with 16 points;
    // if that leaked in we'd see 4 rows or the wrong points for Juventus.
    assert_eq!(rows.len(), 3);
    let juventus = rows
        .iter()
        .find(|row| row.team == "Juventus")
        .expect("Juventus must be present from the TOTAL table");
    assert_eq!(juventus.points, 30);
}

#[test]
fn football_data_org_preserves_provider_order_including_tied_points() {
    let rows =
        parse_football_data_standings_response(&fixture("standings_football_data_serie_a.json"))
            .expect("valid response must parse");

    // Napoli and Juventus are tied on 30 points; the provider lists Napoli
    // ahead of Juventus (presumably by a tiebreaker we can't replicate).
    // The parser must preserve that order, not re-sort by points alone.
    let names: Vec<&str> = rows.iter().map(|row| row.team.as_str()).collect();
    assert_eq!(names, vec!["Internazionale", "Napoli", "Juventus"]);
}

#[test]
fn football_data_org_rejects_malformed_json() {
    let error = parse_football_data_standings_response("{not json").unwrap_err();
    assert!(matches!(error, StandingsFetchError::InvalidJson(_)));
}

#[test]
fn football_data_org_rejects_a_response_with_no_standings_groups() {
    let error = parse_football_data_standings_response(&fixture(
        "standings_football_data_empty_table.json",
    ))
    .unwrap_err();
    assert!(matches!(error, StandingsFetchError::MissingData(_)));
}

#[test]
fn football_data_org_rejects_a_response_with_only_home_and_away_groups() {
    // A response with HOME/AWAY groups but no TOTAL group must error
    // rather than silently rendering a HOME-only or AWAY-only table as if
    // it were the full standings.
    let error = parse_football_data_standings_response(&fixture(
        "standings_football_data_home_away_only.json",
    ))
    .unwrap_err();
    assert!(matches!(error, StandingsFetchError::MissingData(_)));
}

#[test]
fn parses_the_documented_highlightly_standings_response_shape() {
    let rows =
        parse_highlightly_standings_response(&fixture("standings_highlightly_allsvenskan.json"))
            .expect("valid response must parse");

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].competition, "Allsvenskan");
    assert_eq!(rows[0].season, "2025");
    assert_eq!(rows[0].team, "Malmö FF");
    assert_eq!(rows[0].position, 1);
    assert_eq!(rows[0].played, 30);
    assert_eq!(rows[0].points, 55);
    assert_eq!(rows[0].goal_difference, 20);
}

#[test]
fn highlightly_preserves_provider_order_including_tied_points() {
    let rows =
        parse_highlightly_standings_response(&fixture("standings_highlightly_allsvenskan.json"))
            .expect("valid response must parse");

    // Malmö FF and Djurgården are tied on 55 points; the provider ranks
    // Malmö first. The parser must not re-sort and flip that order.
    let names: Vec<&str> = rows.iter().map(|row| row.team.as_str()).collect();
    assert_eq!(names, vec!["Malmö FF", "Djurgården", "Hammarby"]);
}

#[test]
fn highlightly_computes_goal_difference_from_scored_and_received_goals() {
    let rows =
        parse_highlightly_standings_response(&fixture("standings_highlightly_allsvenskan.json"))
            .expect("valid response must parse");

    let hammarby = rows
        .iter()
        .find(|row| row.team == "Hammarby")
        .expect("Hammarby must be present");
    assert_eq!(hammarby.goal_difference, 40 - 29);
}

#[test]
fn highlightly_rejects_malformed_json() {
    let error = parse_highlightly_standings_response("{not json").unwrap_err();
    assert!(matches!(error, StandingsFetchError::InvalidJson(_)));
}

#[test]
fn highlightly_rejects_a_response_with_no_groups() {
    let error =
        parse_highlightly_standings_response(&fixture("standings_highlightly_empty_groups.json"))
            .unwrap_err();
    assert!(matches!(error, StandingsFetchError::MissingData(_)));
}

#[test]
fn panel_is_unavailable_before_any_successful_fetch() {
    let board = StandingsBoard::new();
    let state = board.competition_state("serie-a", datetime!(2026-11-24 20:00:00 UTC));
    assert_eq!(state, StandingsPanelState::Unavailable);
}

#[test]
fn panel_is_fresh_immediately_after_a_successful_fetch() {
    let mut board = StandingsBoard::new();
    let rows =
        parse_football_data_standings_response(&fixture("standings_football_data_serie_a.json"))
            .expect("valid response must parse");
    let fetched_at = datetime!(2026-11-24 20:00:00 UTC);
    board.record_success("serie-a", rows.clone(), fetched_at);

    match board.competition_state("serie-a", fetched_at) {
        StandingsPanelState::Fresh(fresh_rows) => assert_eq!(fresh_rows, rows),
        other => panic!("expected Fresh, got {other:?}"),
    }
}

#[test]
fn panel_becomes_stale_once_the_table_exceeds_the_staleness_threshold() {
    use dashboard_core::standings::STANDINGS_STALE_AFTER;

    let mut board = StandingsBoard::new();
    let rows =
        parse_football_data_standings_response(&fixture("standings_football_data_serie_a.json"))
            .expect("valid response must parse");
    let fetched_at = datetime!(2026-11-24 20:00:00 UTC);
    board.record_success("serie-a", rows, fetched_at);

    let now = fetched_at + STANDINGS_STALE_AFTER + time::Duration::seconds(1);
    match board.competition_state("serie-a", now) {
        StandingsPanelState::Stale { age, .. } => assert_eq!(age, now - fetched_at),
        other => panic!("expected Stale, got {other:?}"),
    }
}

#[test]
fn one_failing_competition_does_not_affect_the_others_panel() {
    let mut board = StandingsBoard::new();
    let rows =
        parse_football_data_standings_response(&fixture("standings_football_data_serie_a.json"))
            .expect("valid response must parse");
    let fetched_at = datetime!(2026-11-24 20:00:00 UTC);
    board.record_success("serie-a", rows, fetched_at);

    // "allsvenskan" was never recorded (its fetch is failing) - it must
    // read Unavailable while "serie-a" stays Fresh, same isolation
    // guarantee DeparturesBoard gives per-site.
    assert_eq!(
        board.competition_state("allsvenskan", fetched_at),
        StandingsPanelState::Unavailable
    );
    assert!(matches!(
        board.competition_state("serie-a", fetched_at),
        StandingsPanelState::Fresh(_)
    ));
}

#[test]
fn each_tables_season_survives_independently_of_now_during_the_off_season() {
    // Both rows carry their own `season` string straight from the
    // provider's response, not derived from "now" - so even read long
    // after the season has ended, the table still identifies which
    // season it belongs to.
    let rows =
        parse_highlightly_standings_response(&fixture("standings_highlightly_allsvenskan.json"))
            .expect("valid response must parse");
    let far_future_now = datetime!(2030-01-01 00:00:00 UTC);

    let mut board = StandingsBoard::new();
    board.record_success("allsvenskan", rows, datetime!(2025-11-01 00:00:00 UTC));

    match board.competition_state("allsvenskan", far_future_now) {
        StandingsPanelState::Stale { rows, .. } => {
            assert_eq!(rows[0].season, "2025");
        }
        other => panic!("expected Stale, got {other:?}"),
    }
}

#[test]
fn fixture_standings_source_is_available_behind_the_fixtures_feature() {
    let rows =
        parse_football_data_standings_response(&fixture("standings_football_data_serie_a.json"))
            .expect("valid response must parse");
    let mut responses = HashMap::new();
    responses.insert("serie-a".to_string(), Ok(rows.clone()));
    let mut source = FixtureStandingsSource::new(responses);

    assert_eq!(source.fetch_standings("serie-a").unwrap(), rows);
    let error = source.fetch_standings("unknown-competition").unwrap_err();
    assert!(matches!(error, StandingsFetchError::MissingData(_)));
}

#[test]
fn both_competitions_are_readable_within_one_rotation() {
    let competitions = vec!["allsvenskan".to_string(), "serie-a".to_string()];
    let rotation = StandingsRotation::new(competitions.clone(), 15);

    let allsvenskan_turn = datetime!(2026-11-24 20:00:00 UTC);
    let serie_a_turn = allsvenskan_turn + time::Duration::seconds(15);

    let first = rotation.active_competition(allsvenskan_turn);
    let second = rotation.active_competition(serie_a_turn);

    assert_ne!(
        first, second,
        "rotation must alternate, not get stuck on one competition"
    );
    assert!(competitions.contains(&first.to_string()));
    assert!(competitions.contains(&second.to_string()));
}

#[test]
fn rotation_stays_on_the_same_competition_within_one_interval() {
    let competitions = vec!["allsvenskan".to_string(), "serie-a".to_string()];
    let rotation = StandingsRotation::new(competitions, 15);

    let now = datetime!(2026-11-24 20:00:00 UTC);
    let still_same_interval = now + time::Duration::seconds(5);

    assert_eq!(
        rotation.active_competition(now),
        rotation.active_competition(still_same_interval)
    );
}

#[test]
fn rotation_with_a_single_competition_always_returns_it() {
    let rotation = StandingsRotation::new(vec!["serie-a".to_string()], 15);
    let now = datetime!(2026-11-24 20:00:00 UTC);
    assert_eq!(rotation.active_competition(now), "serie-a");
    assert_eq!(
        rotation.active_competition(now + time::Duration::seconds(100)),
        "serie-a"
    );
}
