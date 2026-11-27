use dashboard_core::touch::{
    DisplayContext, REFETCH_RATE_LIMIT_SECONDS, TouchConfig, TouchEffect, TouchMode, TouchState,
    TouchTarget, apply_touch, revert_idle,
};
use time::Duration;
use time::macros::datetime;

const ENABLED: TouchConfig = TouchConfig {
    enabled: true,
    wake_seconds: 120,
    screensaver_dismiss_seconds: 45,
    idle_timeout_seconds: 45,
};

const DISABLED: TouchConfig = TouchConfig {
    enabled: false,
    ..ENABLED
};

#[test]
fn a_tap_while_blanked_wakes_the_display_and_requests_a_departures_refresh() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (state, effects) = apply_touch(
        &TouchState::new(),
        DisplayContext::Blanked,
        TouchTarget::Header,
        now,
        &ENABLED,
    );

    assert_eq!(state.mode, TouchMode::Wake);
    assert_eq!(state.wake_until, Some(now + Duration::seconds(120)));
    assert_eq!(
        effects,
        vec![
            TouchEffect::WakeDisplay,
            TouchEffect::RefreshDeparturesRequested
        ]
    );
}

#[test]
fn a_second_tap_during_an_active_wake_window_never_extends_it() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (woken, _) = apply_touch(
        &TouchState::new(),
        DisplayContext::Blanked,
        TouchTarget::Header,
        now,
        &ENABLED,
    );

    let later = now + Duration::seconds(60);
    let (state, effects) = apply_touch(
        &woken,
        DisplayContext::Blanked,
        TouchTarget::Header,
        later,
        &ENABLED,
    );

    assert_eq!(state.wake_until, Some(now + Duration::seconds(120)));
    assert!(effects.is_empty());
}

#[test]
fn a_tap_during_an_active_display_window_with_the_screen_already_on_has_no_wake_effect() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (state, effects) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::Header,
        now,
        &ENABLED,
    );

    assert_eq!(state, TouchState::new());
    assert!(effects.is_empty());
}

#[test]
fn the_wake_window_reverts_to_default_once_the_configured_duration_elapses() {
    let woke_at = datetime!(2026-11-26 20:00:00 UTC);
    let (woken, _) = apply_touch(
        &TouchState::new(),
        DisplayContext::Blanked,
        TouchTarget::Header,
        woke_at,
        &ENABLED,
    );

    let expired = revert_idle(&woken, woke_at + Duration::seconds(120), &ENABLED);
    assert_eq!(expired.mode, TouchMode::Default);
    assert_eq!(expired.wake_until, None);
}

#[test]
fn a_tap_while_the_screensaver_is_active_shows_the_dashboard_without_touching_calendar_state() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (state, effects) = apply_touch(
        &TouchState::new(),
        DisplayContext::Screensaver,
        TouchTarget::Header,
        now,
        &ENABLED,
    );

    assert_eq!(state.mode, TouchMode::ScreensaverDismissed);
    assert_eq!(
        state.screensaver_dismissed_until,
        Some(now + Duration::seconds(45))
    );
    assert_eq!(effects, vec![TouchEffect::DismissScreensaverView]);
}

#[test]
fn the_screensaver_dismissal_reverts_once_its_configured_duration_elapses() {
    let dismissed_at = datetime!(2026-11-26 20:00:00 UTC);
    let (dismissed, _) = apply_touch(
        &TouchState::new(),
        DisplayContext::Screensaver,
        TouchTarget::Header,
        dismissed_at,
        &ENABLED,
    );

    let expired = revert_idle(&dismissed, dismissed_at + Duration::seconds(45), &ENABLED);
    assert_eq!(expired.mode, TouchMode::Default);
    assert_eq!(expired.screensaver_dismissed_until, None);
}

#[test]
fn a_tap_on_the_football_panel_advances_the_index_and_pauses_rotation() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (state, effects) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::FootballPanel {
            panel_count: 3,
            current_index: 0,
        },
        now,
        &ENABLED,
    );

    assert_eq!(state.football_index, Some(1));
    assert_eq!(state.last_input_at, Some(now));
    assert!(effects.is_empty());
}

#[test]
fn tapping_the_football_panel_while_auto_rotated_advances_from_the_displayed_index() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (state, effects) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::FootballPanel {
            panel_count: 3,
            current_index: 1,
        },
        now,
        &ENABLED,
    );

    assert_eq!(state.football_index, Some(2));
    assert!(effects.is_empty());

    let (wrapped, _) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::FootballPanel {
            panel_count: 2,
            current_index: 1,
        },
        now,
        &ENABLED,
    );
    assert_eq!(wrapped.football_index, Some(0));
}

#[test]
fn a_tap_on_the_football_panel_wraps_back_to_the_first_competition() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let paused = TouchState {
        football_index: Some(2),
        ..TouchState::new()
    };
    let (state, _) = apply_touch(
        &paused,
        DisplayContext::Dashboard,
        TouchTarget::FootballPanel {
            panel_count: 3,
            current_index: 0,
        },
        now,
        &ENABLED,
    );

    assert_eq!(state.football_index, Some(0));
}

#[test]
fn football_rotation_resumes_once_the_idle_timeout_elapses_with_no_further_input() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (paused, _) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::FootballPanel {
            panel_count: 3,
            current_index: 0,
        },
        now,
        &ENABLED,
    );

    let resumed = revert_idle(&paused, now + Duration::seconds(45), &ENABLED);
    assert_eq!(resumed.football_index, None);
}

#[test]
fn tapping_a_departure_row_expands_it() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (state, effects) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::DepartureRow {
            departure_id: "9324-stockholm-metro".to_string(),
        },
        now,
        &ENABLED,
    );

    assert_eq!(
        state.expanded_departure,
        Some("9324-stockholm-metro".to_string())
    );
    assert!(effects.is_empty());
}

#[test]
fn tapping_the_same_expanded_departure_row_again_collapses_it() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let expanded = TouchState {
        expanded_departure: Some("9324-stockholm-metro".to_string()),
        ..TouchState::new()
    };

    let (state, _) = apply_touch(
        &expanded,
        DisplayContext::Dashboard,
        TouchTarget::DepartureRow {
            departure_id: "9324-stockholm-metro".to_string(),
        },
        now,
        &ENABLED,
    );

    assert_eq!(state.expanded_departure, None);
}

#[test]
fn an_expanded_departure_row_collapses_once_the_idle_timeout_elapses() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let expanded = TouchState {
        expanded_departure: Some("9324-stockholm-metro".to_string()),
        last_input_at: Some(now),
        ..TouchState::new()
    };

    let collapsed = revert_idle(&expanded, now + Duration::seconds(45), &ENABLED);
    assert_eq!(collapsed.expanded_departure, None);
}

#[test]
fn tapping_the_header_has_no_effect() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (state, effects) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::Header,
        now,
        &ENABLED,
    );

    assert_eq!(state, TouchState::new());
    assert!(effects.is_empty());
}

#[test]
fn tapping_a_stale_panel_requests_a_refetch_for_that_provider() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (state, effects) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::StalePanel {
            provider: "weather".to_string(),
        },
        now,
        &ENABLED,
    );

    assert_eq!(
        effects,
        vec![TouchEffect::RequestPanelRefetch {
            provider: "weather".to_string()
        }]
    );
    assert_eq!(state.last_input_at, Some(now));
}

#[test]
fn a_disabled_touch_config_leaves_state_and_effects_untouched() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (state, effects) = apply_touch(
        &TouchState::new(),
        DisplayContext::Blanked,
        TouchTarget::Header,
        now,
        &DISABLED,
    );

    assert_eq!(state, TouchState::new());
    assert!(effects.is_empty());
}

#[test]
fn tapping_the_same_stale_panel_within_the_rate_limit_emits_no_refetch() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (first_state, first_effects) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::StalePanel {
            provider: "weather".to_string(),
        },
        now,
        &ENABLED,
    );
    assert_eq!(
        first_effects,
        vec![TouchEffect::RequestPanelRefetch {
            provider: "weather".to_string()
        }]
    );

    let within_limit = now + Duration::seconds(i64::from(REFETCH_RATE_LIMIT_SECONDS) - 1);
    let (second_state, second_effects) = apply_touch(
        &first_state,
        DisplayContext::Dashboard,
        TouchTarget::StalePanel {
            provider: "weather".to_string(),
        },
        within_limit,
        &ENABLED,
    );
    assert!(second_effects.is_empty());
    assert_eq!(second_state.last_refetch_at.get("weather"), Some(&now));
}

#[test]
fn tapping_a_different_stale_panel_is_not_blocked_by_another_providers_rate_limit() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (after_weather, _) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::StalePanel {
            provider: "weather".to_string(),
        },
        now,
        &ENABLED,
    );

    let later = now + Duration::seconds(5);
    let (after_departures, effects) = apply_touch(
        &after_weather,
        DisplayContext::Dashboard,
        TouchTarget::StalePanel {
            provider: "departures".to_string(),
        },
        later,
        &ENABLED,
    );
    assert_eq!(
        effects,
        vec![TouchEffect::RequestPanelRefetch {
            provider: "departures".to_string()
        }]
    );
    assert_eq!(after_departures.last_refetch_at.get("weather"), Some(&now));
    assert_eq!(
        after_departures.last_refetch_at.get("departures"),
        Some(&later)
    );
}

#[test]
fn tapping_a_stale_panel_again_after_the_rate_limit_elapses_requests_refetch() {
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (first_state, _) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::StalePanel {
            provider: "weather".to_string(),
        },
        now,
        &ENABLED,
    );

    let after_limit = now + Duration::seconds(i64::from(REFETCH_RATE_LIMIT_SECONDS));
    let (second_state, effects) = apply_touch(
        &first_state,
        DisplayContext::Dashboard,
        TouchTarget::StalePanel {
            provider: "weather".to_string(),
        },
        after_limit,
        &ENABLED,
    );
    assert_eq!(
        effects,
        vec![TouchEffect::RequestPanelRefetch {
            provider: "weather".to_string()
        }]
    );
    assert_eq!(
        second_state.last_refetch_at.get("weather"),
        Some(&after_limit)
    );
}

#[test]
fn revert_idle_preserves_active_refetch_rate_limit_and_prunes_expired_entries() {
    let short_idle = TouchConfig {
        idle_timeout_seconds: 10,
        ..ENABLED
    };
    let now = datetime!(2026-11-26 20:00:00 UTC);
    let (state, _) = apply_touch(
        &TouchState::new(),
        DisplayContext::Dashboard,
        TouchTarget::StalePanel {
            provider: "weather".to_string(),
        },
        now,
        &short_idle,
    );

    let after_idle_before_rate_limit = now + Duration::seconds(15);
    let still_limited = revert_idle(&state, after_idle_before_rate_limit, &short_idle);
    assert_eq!(still_limited.last_refetch_at.get("weather"), Some(&now));

    let after_rate_limit = now + Duration::seconds(i64::from(REFETCH_RATE_LIMIT_SECONDS));
    let pruned = revert_idle(&still_limited, after_rate_limit, &short_idle);
    assert!(pruned.last_refetch_at.is_empty());
}
