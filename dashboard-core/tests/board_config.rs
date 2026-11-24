use dashboard_core::board_config::{BoardConfigError, load_board_config, parse_board_config};
use std::path::Path;

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{name}")).expect("fixture must exist")
}

#[test]
fn parses_valid_board_toml() {
    let config = parse_board_config(&fixture("valid.toml")).expect("valid config must parse");

    assert_eq!(config.time_zone, "Europe/Stockholm");
    assert_eq!(config.stations.len(), 2);
    assert_eq!(config.stations[0].name, "Duvbo");
    assert_eq!(config.stations[0].walk_time_min, 10);
    assert_eq!(config.competitions.len(), 2);
    assert_eq!(config.display_windows.len(), 3);
    let screensaver = config.screensaver.expect("screensaver must be present");
    assert_eq!(screensaver.rules.len(), 2);
}

#[test]
fn rejects_missing_required_field() {
    let err = parse_board_config(&fixture("missing_field.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Schema(_)));
}

#[test]
fn rejects_wrong_field_type() {
    let err = parse_board_config(&fixture("wrong_type.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Schema(_)));
}

#[test]
fn rejects_out_of_range_latitude() {
    let err = parse_board_config(&fixture("out_of_range_latitude.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Invalid(_)));
}

#[test]
fn rejects_walk_time_that_does_not_fit_u32() {
    let err = parse_board_config(&fixture("out_of_range_walk_time.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Schema(_)));
}

#[test]
fn rejects_bad_time_format() {
    let err = parse_board_config(&fixture("bad_time_format.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Invalid(_)));
}

#[test]
fn rejects_invalid_transit_mode() {
    let err = parse_board_config(&fixture("invalid_mode.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Invalid(_)));
}

#[test]
fn rejects_display_window_ending_before_it_starts() {
    let err = parse_board_config(&fixture("invalid_display_window.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Invalid(_)));
}

#[test]
fn rejects_invalid_match_type() {
    let err = parse_board_config(&fixture("invalid_match_type.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Invalid(_)));
}

#[test]
fn rejects_invalid_football_provider() {
    let err = parse_board_config(&fixture("invalid_provider.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Invalid(_)));
}

#[test]
fn rejects_blank_time_zone() {
    let err = parse_board_config(&fixture("blank_time_zone.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Invalid(_)));
}

#[test]
fn error_messages_are_descriptive() {
    let err = parse_board_config(&fixture("invalid_mode.toml")).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("mode"));
    assert!(message.contains("ROCKET"));
}

#[test]
fn rejects_empty_display_windows() {
    let err = parse_board_config(&fixture("empty_display_windows.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Invalid(_)));
}

#[test]
fn rejects_invalid_content_mode() {
    let err = parse_board_config(&fixture("invalid_content_mode.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Invalid(_)));
}

#[test]
fn rejects_invalid_panel_rotation() {
    let err = parse_board_config(&fixture("invalid_panel_rotation.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Invalid(_)));
}

#[test]
fn load_board_config_reports_missing_file() {
    let err = load_board_config(Path::new("tests/fixtures/does_not_exist.toml")).unwrap_err();
    assert!(matches!(err, BoardConfigError::Io(_)));
}

#[test]
fn load_board_config_loads_the_real_root_board_toml() {
    let config = load_board_config(Path::new("../board.toml")).expect("root board.toml must load");

    assert_eq!(config.time_zone, "Europe/Stockholm");
    assert_eq!(config.stations.len(), 2);
    assert_eq!(config.competitions.len(), 2);
    assert_eq!(config.display_windows.len(), 3);
    let panel_rotation = config
        .panel_rotation
        .expect("panel_rotation must be present");
    assert_eq!(panel_rotation.panels, vec!["football".to_string()]);
}
