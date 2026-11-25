use dashboard_core::board_config::{MatchType, ScreensaverRule};
use dashboard_core::screensaver::{
    CalendarAvailability, CalendarEvent, CalendarFetchError, CalendarPollState, CalendarSource,
    FixtureCalendarSource, evaluate_screensaver_rules, parse_google_calendar_events_response,
    poll_calendar,
};
use std::collections::HashMap;
use time::Duration;
use time::macros::datetime;

fn rule(pattern: &str, match_type: MatchType) -> ScreensaverRule {
    ScreensaverRule {
        pattern: pattern.to_string(),
        match_type,
    }
}

fn event(
    id: &str,
    title: &str,
    start: time::OffsetDateTime,
    end: time::OffsetDateTime,
) -> CalendarEvent {
    CalendarEvent {
        event_id: id.to_string(),
        title: title.to_string(),
        start,
        end,
    }
}

// Response shape documented at
// https://developers.google.com/calendar/api/v3/reference/events/list -
// an `items` array, each with `id`, `summary`, and `start`/`end` objects
// carrying `dateTime` (RFC3339, timed events) or `date` (all-day events).
// Unverified against a live response - no service-account credential was
// available in this environment.
const GOOGLE_CALENDAR_EVENTS_FIXTURE: &str = r#"{
  "kind": "calendar#events",
  "items": [
    {
      "kind": "calendar#event",
      "id": "abc123",
      "summary": "Focus 14:00-15:00",
      "start": { "dateTime": "2026-11-24T14:00:00+01:00" },
      "end": { "dateTime": "2026-11-24T15:00:00+01:00" }
    },
    {
      "kind": "calendar#event",
      "id": "def456",
      "summary": "Team offsite",
      "start": { "date": "2026-11-25" },
      "end": { "date": "2026-11-26" }
    }
  ]
}"#;

#[test]
fn parses_the_documented_google_calendar_events_list_response_shape() {
    let events =
        parse_google_calendar_events_response(GOOGLE_CALENDAR_EVENTS_FIXTURE).expect("must parse");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, "abc123");
    assert_eq!(events[0].title, "Focus 14:00-15:00");
}

#[test]
fn skips_all_day_events_lacking_a_datetime_rather_than_failing_the_batch() {
    let events =
        parse_google_calendar_events_response(GOOGLE_CALENDAR_EVENTS_FIXTURE).expect("must parse");

    assert!(!events.iter().any(|event| event.event_id == "def456"));
}

const GOOGLE_CALENDAR_OFFSETLESS_DATETIME_FIXTURE: &str = r#"{
  "items": [
    {
      "id": "zone789",
      "summary": "Focus 14:00-15:00",
      "start": { "dateTime": "2026-11-24T14:00:00", "timeZone": "Europe/Stockholm" },
      "end": { "dateTime": "2026-11-24T15:00:00", "timeZone": "Europe/Stockholm" }
    }
  ]
}"#;

#[test]
fn resolves_an_offsetless_datetime_through_its_sibling_timezone_field() {
    let events = parse_google_calendar_events_response(GOOGLE_CALENDAR_OFFSETLESS_DATETIME_FIXTURE)
        .expect("must parse");

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, "zone789");
    // Europe/Stockholm is UTC+1 in late November (no DST).
    assert_eq!(events[0].start, datetime!(2026-11-24 13:00:00 UTC));
    assert_eq!(events[0].end, datetime!(2026-11-24 14:00:00 UTC));
}

#[test]
fn an_offsetless_datetime_with_no_timezone_fails_visibly_rather_than_dropping_silently() {
    let json = r#"{
      "items": [
        {
          "id": "notz",
          "summary": "Focus",
          "start": { "dateTime": "2026-11-24T14:00:00" },
          "end": { "dateTime": "2026-11-24T15:00:00" }
        }
      ]
    }"#;

    let error = parse_google_calendar_events_response(json).unwrap_err();
    assert!(matches!(error, CalendarFetchError::MissingData(_)));
}

#[test]
fn rejects_malformed_json() {
    let error = parse_google_calendar_events_response("{not json").unwrap_err();
    assert!(matches!(error, CalendarFetchError::InvalidJson(_)));
}

#[test]
fn an_empty_items_array_parses_to_no_events() {
    let events = parse_google_calendar_events_response(r#"{"items":[]}"#).expect("must parse");
    assert!(events.is_empty());
}

#[test]
fn substring_match_is_case_insensitive() {
    let rules = vec![rule("focus", MatchType::Substring)];
    let events = vec![event(
        "1",
        "FOCUS time",
        datetime!(2026-11-24 13:00:00 UTC),
        datetime!(2026-11-24 14:00:00 UTC),
    )];

    let active = evaluate_screensaver_rules(&rules, &events, datetime!(2026-11-24 13:30:00 UTC));
    assert!(active);
}

#[test]
fn regex_match_type_matches_the_configured_pattern() {
    let rules = vec![rule("^Focus \\d{2}:\\d{2}", MatchType::Regex)];
    let events = vec![event(
        "1",
        "Focus 14:00-15:00",
        datetime!(2026-11-24 13:00:00 UTC),
        datetime!(2026-11-24 14:00:00 UTC),
    )];

    let active = evaluate_screensaver_rules(&rules, &events, datetime!(2026-11-24 13:30:00 UTC));
    assert!(active);
}

#[test]
fn an_invalid_regex_pattern_never_matches_but_does_not_panic() {
    let rules = vec![rule("(unclosed", MatchType::Regex)];
    let events = vec![event(
        "1",
        "(unclosed meeting",
        datetime!(2026-11-24 13:00:00 UTC),
        datetime!(2026-11-24 14:00:00 UTC),
    )];

    let active = evaluate_screensaver_rules(&rules, &events, datetime!(2026-11-24 13:30:00 UTC));
    assert!(!active);
}

#[test]
fn a_matching_event_outside_its_start_end_window_does_not_activate() {
    let rules = vec![rule("focus", MatchType::Substring)];
    let events = vec![event(
        "1",
        "Focus time",
        datetime!(2026-11-24 13:00:00 UTC),
        datetime!(2026-11-24 14:00:00 UTC),
    )];

    let before = evaluate_screensaver_rules(&rules, &events, datetime!(2026-11-24 12:59:00 UTC));
    let after = evaluate_screensaver_rules(&rules, &events, datetime!(2026-11-24 14:00:01 UTC));
    assert!(!before);
    assert!(!after);
}

#[test]
fn multiple_matching_events_behave_as_a_union_any_match_keeps_it_active() {
    let rules = vec![
        rule("focus", MatchType::Substring),
        rule("away", MatchType::Substring),
    ];
    let events = vec![
        event(
            "1",
            "Focus block",
            datetime!(2026-11-24 09:00:00 UTC),
            datetime!(2026-11-24 10:00:00 UTC),
        ),
        event(
            "2",
            "Away",
            datetime!(2026-11-24 10:00:00 UTC),
            datetime!(2026-11-24 11:00:00 UTC),
        ),
    ];

    assert!(evaluate_screensaver_rules(
        &rules,
        &events,
        datetime!(2026-11-24 09:30:00 UTC)
    ));
    assert!(evaluate_screensaver_rules(
        &rules,
        &events,
        datetime!(2026-11-24 10:30:00 UTC)
    ));
    assert!(!evaluate_screensaver_rules(
        &rules,
        &events,
        datetime!(2026-11-24 11:30:00 UTC)
    ));
}

#[test]
fn a_non_matching_event_title_does_not_activate() {
    let rules = vec![rule("focus", MatchType::Substring)];
    let events = vec![event(
        "1",
        "Standup",
        datetime!(2026-11-24 13:00:00 UTC),
        datetime!(2026-11-24 14:00:00 UTC),
    )];

    assert!(!evaluate_screensaver_rules(
        &rules,
        &events,
        datetime!(2026-11-24 13:30:00 UTC)
    ));
}

#[test]
fn poll_state_is_unavailable_and_inactive_before_any_successful_poll() {
    let state = CalendarPollState::new();
    assert!(!state.screensaver_active());
    assert_eq!(
        state.availability(datetime!(2026-11-24 13:30:00 UTC)),
        CalendarAvailability::Unavailable
    );
}

#[test]
fn a_successful_poll_records_the_computed_activation_and_becomes_fresh() {
    let mut state = CalendarPollState::new();
    state.record_success(true, datetime!(2026-11-24 13:00:00 UTC));

    assert!(state.screensaver_active());
    assert_eq!(
        state.availability(datetime!(2026-11-24 13:00:00 UTC)),
        CalendarAvailability::Fresh
    );
}

#[test]
fn a_failed_poll_holds_the_last_known_activation_state_rather_than_guessing() {
    let mut state = CalendarPollState::new();
    state.record_success(true, datetime!(2026-11-24 13:00:00 UTC));

    state.record_failure();

    assert!(state.screensaver_active());
}

#[test]
fn availability_becomes_stale_once_a_successful_poll_ages_past_the_threshold_even_though_activation_holds()
 {
    let mut state = CalendarPollState::new();
    state.record_success(true, datetime!(2026-11-24 13:00:00 UTC));
    state.record_failure();

    let later = datetime!(2026-11-24 13:00:00 UTC) + Duration::minutes(30);
    assert!(state.screensaver_active());
    assert!(matches!(
        state.availability(later),
        CalendarAvailability::Stale { .. }
    ));
}

#[cfg(feature = "fixtures")]
#[test]
fn poll_calendar_updates_state_from_a_successful_fixture_fetch() {
    let rules = vec![rule("focus", MatchType::Substring)];
    let mut responses = HashMap::new();
    responses.insert(
        "primary".to_string(),
        Ok(vec![event(
            "1",
            "Focus block",
            datetime!(2026-11-24 13:00:00 UTC),
            datetime!(2026-11-24 14:00:00 UTC),
        )]),
    );
    let mut source = FixtureCalendarSource::new(responses);
    let mut state = CalendarPollState::new();

    poll_calendar(
        &mut source,
        "primary",
        &rules,
        datetime!(2026-11-24 13:30:00 UTC),
        &mut state,
    );

    assert!(state.screensaver_active());
    assert_eq!(
        state.availability(datetime!(2026-11-24 13:30:00 UTC)),
        CalendarAvailability::Fresh
    );
}

#[cfg(feature = "fixtures")]
#[test]
fn poll_calendar_holds_last_state_when_the_fixture_fetch_fails() {
    let rules = vec![rule("focus", MatchType::Substring)];
    let mut state = CalendarPollState::new();
    state.record_success(true, datetime!(2026-11-24 13:00:00 UTC));

    let mut responses = HashMap::new();
    responses.insert(
        "primary".to_string(),
        Err(CalendarFetchError::MissingData(
            "calendar API unreachable".to_string(),
        )),
    );
    let mut source = FixtureCalendarSource::new(responses);

    poll_calendar(
        &mut source,
        "primary",
        &rules,
        datetime!(2026-11-24 13:45:00 UTC),
        &mut state,
    );

    assert!(state.screensaver_active());
    assert!(matches!(
        state.availability(datetime!(2026-11-24 13:45:00 UTC)),
        CalendarAvailability::Stale { .. }
    ));
}

#[cfg(feature = "fixtures")]
#[test]
fn fixture_calendar_source_reports_missing_data_for_an_unconfigured_calendar() {
    let mut source: Box<dyn CalendarSource> = Box::new(FixtureCalendarSource::new(HashMap::new()));
    let error = source.fetch_events("unknown").unwrap_err();
    assert!(matches!(error, CalendarFetchError::MissingData(_)));
}
