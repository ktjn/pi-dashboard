use dashboard_core::board_config::{MatchType, ScreensaverRule, Station, TransitMode};
use dashboard_core::clock::local_clock;
use dashboard_core::daylight::daylight_window;
use dashboard_core::departures::{
    DEPARTURES_STALE_AFTER, DeparturesBoard, DeparturesPanelState, DeparturesSource,
    DeviationsSource, FixtureDeparturesSource, FixtureDeviationsSource, RawDeparture,
};
use dashboard_core::render::{
    StationLayoutInput, compute_dashboard_layout, hit_test, render_calendar_availability_note,
    render_clock_weather_header, render_daylight_strip_at, render_departures_panel_with_header,
    render_screensaver_view, render_standings_panel, weather_precip_probability,
};
use dashboard_core::scene3d::{SplitFlapBoard, compute_solar_scene};
use dashboard_core::schedule::{DisplayState, reconcile};
use dashboard_core::screensaver::{
    CalendarEvent, CalendarPollState, FixtureCalendarSource, poll_calendar,
};
use dashboard_core::standings::{
    FixtureStandingsSource, STANDINGS_STALE_AFTER, StandingsBoard, StandingsPanelState,
    StandingsRotation, StandingsRow, StandingsSource,
};
use dashboard_core::theme::{Theme, load_fonts, palette};
use dashboard_core::touch::{
    DisplayContext, TouchConfig, TouchEffect, TouchMode, TouchState, TouchTarget, apply_touch,
    revert_idle,
};
use dashboard_core::weather::{
    FixtureWeatherSource, WEATHER_STALE_AFTER, WeatherPanelState, WeatherReading,
    WeatherSnapshotStore, WeatherSource,
};
use jiff::civil::Date;
use std::collections::HashMap;
use time::Duration;
use time::macros::datetime;

const STOCKHOLM_LAT: f64 = 59.3626;
const STOCKHOLM_LON: f64 = 17.9158;
const STOCKHOLM_TZ: &str = "Europe/Stockholm";

const DUVBO_SITE_ID: u32 = 9324;
const SUNDBYBERG_SITE_ID: u32 = 9325;

const ALLSVENSKAN_ID: &str = "allsvenskan";
const SERIE_A_ID: &str = "serie-a";
const STANDINGS_ROTATION_INTERVAL_SECONDS: u32 = 15;

const SCREENSAVER_CALENDAR_ID: &str = "primary";

const PLAYGROUND_CALENDAR_POLL_INTERVAL: Duration = Duration::seconds(15);
const PLAYGROUND_DEPARTURES_REFRESH_INTERVAL: Duration = Duration::seconds(30);
const PLAYGROUND_WEATHER_REFRESH_INTERVAL: Duration = Duration::minutes(10);
const PLAYGROUND_STANDINGS_REFRESH_INTERVAL: Duration = Duration::minutes(5);

const DEMO_SCREENSAVER_FROM: Duration = Duration::seconds(60);
const DEMO_SCREENSAVER_UNTIL: Duration = Duration::seconds(180);
const DEMO_STALE_FROM: Duration = Duration::seconds(200);
const DEMO_STALE_UNTIL: Duration = Duration::seconds(215);
const DEMO_UNAVAILABLE_FROM: Duration = Duration::seconds(225);
const DEMO_CYCLE_LENGTH: Duration = Duration::seconds(240);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DemoPhase {
    Unavailable,
    Fresh,
    Stale,
}

fn demo_phase(elapsed_since_start: Duration) -> DemoPhase {
    if elapsed_since_start >= DEMO_UNAVAILABLE_FROM {
        DemoPhase::Unavailable
    } else if elapsed_since_start >= DEMO_STALE_FROM && elapsed_since_start < DEMO_STALE_UNTIL {
        DemoPhase::Stale
    } else {
        DemoPhase::Fresh
    }
}

fn resolve_competition_index(
    ids: &[&str],
    active_id: &str,
    override_index: Option<usize>,
) -> usize {
    if ids.is_empty() {
        return 0;
    }
    match override_index {
        Some(index) => index % ids.len(),
        None => ids.iter().position(|&id| id == active_id).unwrap_or(0),
    }
}

fn fixture_duvbo_departures(now: time::OffsetDateTime) -> Vec<RawDeparture> {
    vec![
        RawDeparture {
            destination: "Kungsträdgården".to_string(),
            direction: Some("Kungsträdgården".to_string()),
            transport_mode: TransitMode::Metro,
            line_designation: "10".to_string(),
            scheduled: now + Duration::minutes(14),
            expected: Some(now + Duration::minutes(14)),
        },
        RawDeparture {
            destination: "Hjulsta".to_string(),
            direction: Some("Hjulsta".to_string()),
            transport_mode: TransitMode::Metro,
            line_designation: "10".to_string(),
            scheduled: now + Duration::minutes(2),
            expected: Some(now + Duration::minutes(2)),
        },
    ]
}

fn fixture_sundbyberg_departures(now: time::OffsetDateTime) -> Vec<RawDeparture> {
    vec![RawDeparture {
        destination: "Stockholm Central".to_string(),
        direction: Some("Stockholm Central".to_string()),
        transport_mode: TransitMode::Train,
        line_designation: "40".to_string(),
        scheduled: now + Duration::minutes(18),
        expected: Some(now + Duration::minutes(18)),
    }]
}

fn refresh_fixture_departures_for_site(
    board: &mut DeparturesBoard,
    site_id: u32,
    generation_time: time::OffsetDateTime,
    fetched_at: time::OffsetDateTime,
) -> bool {
    let departures = match site_id {
        DUVBO_SITE_ID => fixture_duvbo_departures(generation_time),
        SUNDBYBERG_SITE_ID => fixture_sundbyberg_departures(generation_time),
        _ => return false,
    };
    let mut departures_responses = HashMap::new();
    departures_responses.insert(site_id, Ok(departures));
    let mut departures_source = FixtureDeparturesSource::new(departures_responses);
    let mut deviations_source = FixtureDeviationsSource::new(HashMap::new());

    if let Ok(departures) = departures_source.fetch_departures(site_id) {
        board.record_departures_success(site_id, departures, fetched_at);
    }
    if let Ok(deviations) = deviations_source.fetch_deviations(site_id) {
        board.record_deviations_success_at(site_id, deviations, fetched_at);
    }
    true
}

fn refresh_fixture_departures(
    board: &mut DeparturesBoard,
    generation_time: time::OffsetDateTime,
    fetched_at: time::OffsetDateTime,
) {
    for site_id in [DUVBO_SITE_ID, SUNDBYBERG_SITE_ID] {
        refresh_fixture_departures_for_site(board, site_id, generation_time, fetched_at);
    }
}

fn refresh_fixture_weather(
    store: &mut WeatherSnapshotStore,
    observed_at: time::OffsetDateTime,
    fetched_at: time::OffsetDateTime,
) {
    let mut weather_source = FixtureWeatherSource::new(vec![Ok(WeatherReading {
        temp_c: 3.5,
        precip_probability_pct: 45,
        observed_at,
    })]);
    if let Ok(reading) = weather_source.fetch() {
        store.record_success(reading, fetched_at);
    }
}

fn fixture_allsvenskan_table() -> Vec<StandingsRow> {
    vec![
        StandingsRow {
            competition: "Allsvenskan".to_string(),
            season: "2026".to_string(),
            position: 1,
            team: "Malmö FF".to_string(),
            played: 30,
            goal_difference: 20,
            points: 55,
        },
        StandingsRow {
            competition: "Allsvenskan".to_string(),
            season: "2026".to_string(),
            position: 2,
            team: "Djurgården".to_string(),
            played: 30,
            goal_difference: 17,
            points: 55,
        },
    ]
}

fn fixture_serie_a_table() -> Vec<StandingsRow> {
    vec![
        StandingsRow {
            competition: "Serie A".to_string(),
            season: "2025/2026".to_string(),
            position: 1,
            team: "Internazionale".to_string(),
            played: 13,
            goal_difference: 18,
            points: 32,
        },
        StandingsRow {
            competition: "Serie A".to_string(),
            season: "2025/2026".to_string(),
            position: 2,
            team: "Napoli".to_string(),
            played: 13,
            goal_difference: 13,
            points: 30,
        },
    ]
}

fn refresh_fixture_standings_for_competition(
    board: &mut StandingsBoard,
    competition_id: &str,
    now: time::OffsetDateTime,
) -> bool {
    let rows = match competition_id {
        ALLSVENSKAN_ID => fixture_allsvenskan_table(),
        SERIE_A_ID => fixture_serie_a_table(),
        _ => return false,
    };
    let mut responses = HashMap::new();
    responses.insert(competition_id.to_string(), Ok(rows));
    let mut source = FixtureStandingsSource::new(responses);

    if let Ok(rows) = source.fetch_standings(competition_id) {
        board.record_success(competition_id, rows, now);
    }
    true
}

fn refresh_fixture_standings(board: &mut StandingsBoard, now: time::OffsetDateTime) {
    for competition_id in [ALLSVENSKAN_ID, SERIE_A_ID] {
        refresh_fixture_standings_for_competition(board, competition_id, now);
    }
}

fn apply_panel_refetch(
    provider: &str,
    now_utc: time::OffsetDateTime,
    weather_store: &mut WeatherSnapshotStore,
    departures_board: &mut DeparturesBoard,
    standings_board: &mut StandingsBoard,
) -> bool {
    if provider == "weather" {
        refresh_fixture_weather(weather_store, now_utc, now_utc);
        return true;
    }
    if let Some(site_str) = provider.strip_prefix("departures:") {
        return site_str.parse::<u32>().is_ok_and(|site_id| {
            refresh_fixture_departures_for_site(departures_board, site_id, now_utc, now_utc)
        });
    }
    if let Some(competition_id) = provider.strip_prefix("standings:") {
        return refresh_fixture_standings_for_competition(standings_board, competition_id, now_utc);
    }
    false
}

fn fixture_screensaver_rules() -> Vec<ScreensaverRule> {
    vec![ScreensaverRule {
        pattern: "focus".to_string(),
        match_type: MatchType::Substring,
    }]
}

fn fixture_calendar_events(cycle_start_utc: time::OffsetDateTime) -> Vec<CalendarEvent> {
    vec![CalendarEvent {
        event_id: "playground-focus-block".to_string(),
        title: "Focus 14:00-15:00".to_string(),
        start: cycle_start_utc + DEMO_SCREENSAVER_FROM,
        end: cycle_start_utc + DEMO_SCREENSAVER_UNTIL,
    }]
}

fn refresh_fixture_calendar(
    state: &mut CalendarPollState,
    rules: &[ScreensaverRule],
    cycle_start_utc: time::OffsetDateTime,
    now: time::OffsetDateTime,
) {
    let mut responses = HashMap::new();
    responses.insert(
        SCREENSAVER_CALENDAR_ID.to_string(),
        Ok(fixture_calendar_events(cycle_start_utc)),
    );
    let mut source = FixtureCalendarSource::new(responses);
    poll_calendar(&mut source, SCREENSAVER_CALENDAR_ID, rules, now, state);
}

fn collect_frame_taps() -> Vec<(f32, f32)> {
    let mut taps: Vec<(f32, f32)> = macroquad::input::touches()
        .into_iter()
        .filter(|touch| touch.phase == macroquad::input::TouchPhase::Started)
        .map(|touch| (touch.position.x, touch.position.y))
        .collect();
    if taps.is_empty()
        && macroquad::input::is_mouse_button_pressed(macroquad::input::MouseButton::Left)
    {
        taps.push(macroquad::input::mouse_position());
    }
    taps
}

#[macroquad::main("pi-dashboard (playground)")]
async fn main() {
    let (mono_font, sans_font) = load_fonts().expect("vendored fonts must load");
    let theme = Theme {
        mono: &mono_font,
        sans: &sans_font,
    };

    let base_utc = datetime!(2026-06-21 10:30:00 UTC);
    let start_time = macroquad::time::get_time();

    let mut weather_store = WeatherSnapshotStore::new();
    let mut last_weather_refresh = base_utc;

    let mut departures_board = DeparturesBoard::new();
    let mut last_departures_refresh = base_utc;

    let mut standings_board = StandingsBoard::new();
    let mut last_standings_refresh = base_utc;

    let mut last_demo_phase: Option<DemoPhase> = None;

    let competition_ids = [ALLSVENSKAN_ID, SERIE_A_ID];
    let standings_rotation = StandingsRotation::new(
        vec![ALLSVENSKAN_ID.to_string(), SERIE_A_ID.to_string()],
        STANDINGS_ROTATION_INTERVAL_SECONDS,
    );

    let screensaver_rules = fixture_screensaver_rules();
    let mut calendar_poll_state = CalendarPollState::new();
    refresh_fixture_calendar(
        &mut calendar_poll_state,
        &screensaver_rules,
        base_utc,
        base_utc,
    );
    let mut last_calendar_refresh = base_utc;

    let touch_config = TouchConfig {
        enabled: true,
        wake_seconds: TouchConfig::DEFAULT_WAKE_SECONDS,
        screensaver_dismiss_seconds: TouchConfig::DEFAULT_SCREENSAVER_DISMISS_SECONDS,
        idle_timeout_seconds: TouchConfig::DEFAULT_IDLE_TIMEOUT_SECONDS,
    };
    let mut touch_state = TouchState::new();

    let stations = [
        Station {
            site_id: DUVBO_SITE_ID,
            name: "Duvbo".to_string(),
            mode: TransitMode::Metro,
            walk_time_min: 10,
            direction: "stockholm".to_string(),
        },
        Station {
            site_id: SUNDBYBERG_SITE_ID,
            name: "Sundbyberg".to_string(),
            mode: TransitMode::Train,
            walk_time_min: 15,
            direction: "stockholm".to_string(),
        },
    ];

    let mut daylight_date: Option<Date> = None;
    let mut daylight = None;
    let mut split_flap = SplitFlapBoard::new("00:00:00");

    let cycle_length_seconds = DEMO_CYCLE_LENGTH.whole_seconds();

    loop {
        let elapsed_seconds = (macroquad::time::get_time() - start_time).max(0.0) as i64;
        let now_utc = base_utc + Duration::seconds(elapsed_seconds);
        let cycle_index = elapsed_seconds / cycle_length_seconds;
        let elapsed_in_cycle = elapsed_seconds - cycle_index * cycle_length_seconds;
        let cycle_start_utc = base_utc + Duration::seconds(cycle_index * cycle_length_seconds);
        let elapsed_since_start = Duration::seconds(elapsed_in_cycle);

        touch_state = revert_idle(&touch_state, now_utc, &touch_config);

        let phase = demo_phase(elapsed_since_start);
        if Some(phase) != last_demo_phase {
            last_demo_phase = Some(phase);
            match phase {
                DemoPhase::Unavailable => {
                    weather_store = WeatherSnapshotStore::new();
                    departures_board = DeparturesBoard::new();
                    standings_board = StandingsBoard::new();
                }
                DemoPhase::Fresh => {
                    refresh_fixture_weather(&mut weather_store, now_utc, now_utc);
                    last_weather_refresh = now_utc;
                    refresh_fixture_departures(&mut departures_board, now_utc, now_utc);
                    last_departures_refresh = now_utc;
                    refresh_fixture_standings(&mut standings_board, now_utc);
                    last_standings_refresh = now_utc;
                }
                DemoPhase::Stale => {
                    let weather_observed_at = now_utc - WEATHER_STALE_AFTER - Duration::minutes(5);
                    refresh_fixture_weather(&mut weather_store, weather_observed_at, now_utc);
                    let departures_fetched_at =
                        now_utc - DEPARTURES_STALE_AFTER - Duration::minutes(1);
                    refresh_fixture_departures(
                        &mut departures_board,
                        now_utc,
                        departures_fetched_at,
                    );
                    let standings_fetched_at = now_utc - STANDINGS_STALE_AFTER - Duration::hours(1);
                    refresh_fixture_standings(&mut standings_board, standings_fetched_at);
                }
            }
        }

        if phase == DemoPhase::Fresh
            && now_utc - last_departures_refresh >= PLAYGROUND_DEPARTURES_REFRESH_INTERVAL
        {
            refresh_fixture_departures(&mut departures_board, now_utc, now_utc);
            last_departures_refresh = now_utc;
        }
        if phase == DemoPhase::Fresh
            && now_utc - last_weather_refresh >= PLAYGROUND_WEATHER_REFRESH_INTERVAL
        {
            refresh_fixture_weather(&mut weather_store, now_utc, now_utc);
            last_weather_refresh = now_utc;
        }
        if phase == DemoPhase::Fresh
            && now_utc - last_standings_refresh >= PLAYGROUND_STANDINGS_REFRESH_INTERVAL
        {
            refresh_fixture_standings(&mut standings_board, now_utc);
            last_standings_refresh = now_utc;
        }
        if now_utc - last_calendar_refresh >= PLAYGROUND_CALENDAR_POLL_INTERVAL {
            refresh_fixture_calendar(
                &mut calendar_poll_state,
                &screensaver_rules,
                cycle_start_utc,
                now_utc,
            );
            last_calendar_refresh = now_utc;
        }

        let mut taps = collect_frame_taps();

        macroquad::window::clear_background(palette::INK);

        let clock = local_clock(STOCKHOLM_TZ, now_utc).expect("time zone must resolve");
        split_flap.update(
            &format!("{:02}:{:02}:{:02}", clock.hour, clock.minute, clock.second),
            macroquad::time::get_frame_time(),
        );
        let calendar_availability = calendar_poll_state.availability(now_utc);

        let display_state = reconcile(true, None, calendar_poll_state.screensaver_active());
        if display_state == DisplayState::Screensaver
            && touch_state.mode != TouchMode::ScreensaverDismissed
        {
            if !taps.is_empty() {
                let (next_state, _) = apply_touch(
                    &touch_state,
                    DisplayContext::Screensaver,
                    TouchTarget::Header,
                    now_utc,
                    &touch_config,
                );
                touch_state = next_state;
                taps.clear();
            }
            if touch_state.mode != TouchMode::ScreensaverDismissed {
                render_screensaver_view(&split_flap, &calendar_availability, &theme);
                macroquad::window::next_frame().await;
                continue;
            }
        }

        let screen_width = macroquad::window::screen_width();
        let auto_competition_id = standings_rotation.active_competition(now_utc);

        let mut weather_state = weather_store.state(now_utc);
        let mut station_states: Vec<DeparturesPanelState> = stations
            .iter()
            .map(|station| departures_board.station_state(station, now_utc))
            .collect();
        let mut active_competition_index = resolve_competition_index(
            &competition_ids,
            auto_competition_id,
            touch_state.football_index,
        );
        let mut active_competition_id = competition_ids[active_competition_index];
        let mut standings_state = standings_board.competition_state(active_competition_id, now_utc);

        let layout_inputs: Vec<StationLayoutInput<'_>> = stations
            .iter()
            .zip(station_states.iter())
            .map(|(station, state)| StationLayoutInput {
                site_id: station.site_id,
                state,
            })
            .collect();

        let mut layout = compute_dashboard_layout(
            screen_width,
            matches!(weather_state, WeatherPanelState::Stale { .. }),
            &layout_inputs,
            touch_state.expanded_departure.as_deref(),
            matches!(standings_state, StandingsPanelState::Stale { .. }),
            competition_ids.len(),
            active_competition_index,
            active_competition_id,
        );

        let mut state_updated = false;
        for (tap_x, tap_y) in taps {
            if let Some(target) = hit_test(&layout, tap_x, tap_y) {
                let (next_state, effects) = apply_touch(
                    &touch_state,
                    DisplayContext::Dashboard,
                    target,
                    now_utc,
                    &touch_config,
                );
                touch_state = next_state;
                state_updated = true;

                for effect in effects {
                    if let TouchEffect::RequestPanelRefetch { provider } = effect {
                        let _ = apply_panel_refetch(
                            &provider,
                            now_utc,
                            &mut weather_store,
                            &mut departures_board,
                            &mut standings_board,
                        );
                    }
                }
            }
        }

        if state_updated {
            weather_state = weather_store.state(now_utc);
            station_states = stations
                .iter()
                .map(|station| departures_board.station_state(station, now_utc))
                .collect();
            active_competition_index = resolve_competition_index(
                &competition_ids,
                auto_competition_id,
                touch_state.football_index,
            );
            active_competition_id = competition_ids[active_competition_index];
            standings_state = standings_board.competition_state(active_competition_id, now_utc);

            let updated_inputs: Vec<StationLayoutInput<'_>> = stations
                .iter()
                .zip(station_states.iter())
                .map(|(station, state)| StationLayoutInput {
                    site_id: station.site_id,
                    state,
                })
                .collect();

            layout = compute_dashboard_layout(
                screen_width,
                matches!(weather_state, WeatherPanelState::Stale { .. }),
                &updated_inputs,
                touch_state.expanded_departure.as_deref(),
                matches!(standings_state, StandingsPanelState::Stale { .. }),
                competition_ids.len(),
                active_competition_index,
                active_competition_id,
            );
        }

        render_clock_weather_header(&clock, &split_flap, &weather_state, &theme);

        for ((station, state), top) in stations
            .iter()
            .zip(station_states.iter())
            .zip(layout.station_tops.iter().copied())
        {
            let _ = render_departures_panel_with_header(
                &station.name,
                station.site_id,
                state,
                top,
                touch_state.expanded_departure.as_deref(),
                &theme,
            );
        }

        let today = Date::new(clock.year, clock.month as i8, clock.day as i8)
            .expect("local clock must yield a valid calendar date");
        if daylight_date != Some(today) {
            daylight = daylight_window(STOCKHOLM_TZ, STOCKHOLM_LAT, STOCKHOLM_LON, today).ok();
            daylight_date = Some(today);
        }
        if let Some(window) = &daylight {
            let solar_scene = compute_solar_scene(window, now_utc, STOCKHOLM_TZ);
            let _ = render_daylight_strip_at(
                window,
                &solar_scene,
                weather_precip_probability(&weather_state),
                layout.daylight_top,
                &theme,
            );
        }

        render_standings_panel(
            active_competition_id,
            &standings_state,
            layout.standings_top,
            &theme,
        );

        render_calendar_availability_note(
            &calendar_availability,
            macroquad::window::screen_height() - 20.0,
            &theme,
        );

        macroquad::window::next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_phase_transitions_at_expected_boundaries() {
        assert_eq!(demo_phase(Duration::seconds(0)), DemoPhase::Fresh);
        assert_eq!(demo_phase(Duration::seconds(10)), DemoPhase::Fresh);
        assert_eq!(demo_phase(Duration::seconds(199)), DemoPhase::Fresh);
        assert_eq!(demo_phase(Duration::seconds(200)), DemoPhase::Stale);
        assert_eq!(demo_phase(Duration::seconds(214)), DemoPhase::Stale);
        assert_eq!(demo_phase(Duration::seconds(215)), DemoPhase::Fresh);
        assert_eq!(demo_phase(Duration::seconds(224)), DemoPhase::Fresh);
        assert_eq!(demo_phase(Duration::seconds(225)), DemoPhase::Unavailable);
        assert_eq!(demo_phase(Duration::seconds(239)), DemoPhase::Unavailable);
    }

    #[test]
    fn fixture_duvbo_departures_generate_expected_metro_departures_relative_to_now() {
        let now = datetime!(2026-11-24 20:00:00 UTC);
        let departures = fixture_duvbo_departures(now);

        assert_eq!(departures.len(), 2);
        assert_eq!(departures[0].destination, "Kungsträdgården");
        assert_eq!(departures[0].transport_mode, TransitMode::Metro);
        assert_eq!(departures[0].line_designation, "10");
        assert_eq!(departures[0].scheduled, now + Duration::minutes(14));
        assert_eq!(departures[0].expected, Some(now + Duration::minutes(14)));

        assert_eq!(departures[1].destination, "Hjulsta");
        assert_eq!(departures[1].transport_mode, TransitMode::Metro);
        assert_eq!(departures[1].line_designation, "10");
        assert_eq!(departures[1].scheduled, now + Duration::minutes(2));
        assert_eq!(departures[1].expected, Some(now + Duration::minutes(2)));
    }

    #[test]
    fn fixture_sundbyberg_departures_generate_expected_train_departure_relative_to_now() {
        let now = datetime!(2026-11-24 20:00:00 UTC);
        let departures = fixture_sundbyberg_departures(now);

        assert_eq!(departures.len(), 1);
        assert_eq!(departures[0].destination, "Stockholm Central");
        assert_eq!(departures[0].transport_mode, TransitMode::Train);
        assert_eq!(departures[0].line_designation, "40");
        assert_eq!(departures[0].scheduled, now + Duration::minutes(18));
        assert_eq!(departures[0].expected, Some(now + Duration::minutes(18)));
    }

    #[test]
    fn fixture_calendar_events_align_with_demo_screensaver_window() {
        let cycle_start = datetime!(2026-11-24 20:00:00 UTC);
        let events = fixture_calendar_events(cycle_start);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].start, cycle_start + DEMO_SCREENSAVER_FROM);
        assert_eq!(events[0].end, cycle_start + DEMO_SCREENSAVER_UNTIL);
        assert!(DEMO_SCREENSAVER_UNTIL < DEMO_STALE_FROM);
        assert!(DEMO_STALE_UNTIL < DEMO_UNAVAILABLE_FROM);
        assert!(DEMO_UNAVAILABLE_FROM < DEMO_CYCLE_LENGTH);

        let rules = fixture_screensaver_rules();
        let mut poll_state = CalendarPollState::new();
        refresh_fixture_calendar(
            &mut poll_state,
            &rules,
            cycle_start,
            cycle_start + DEMO_SCREENSAVER_FROM,
        );
        assert!(poll_state.screensaver_active());

        refresh_fixture_calendar(
            &mut poll_state,
            &rules,
            cycle_start,
            cycle_start + DEMO_SCREENSAVER_UNTIL,
        );
        assert!(!poll_state.screensaver_active());
    }

    #[test]
    fn refresh_fixture_departures_records_deviations_timestamp() {
        let now = datetime!(2026-11-24 20:00:00 UTC);
        let fetched_at = now - Duration::seconds(30);
        let mut board = DeparturesBoard::new();

        refresh_fixture_departures(&mut board, now, fetched_at);

        let snapshot = board.export_snapshot();
        assert_eq!(snapshot.deviations.len(), 2);
        for site in snapshot.deviations {
            assert_eq!(site.fetched_at_unix, Some(fetched_at.unix_timestamp()));
        }
    }

    #[test]
    fn resolve_competition_index_uses_active_id_when_no_override() {
        let ids = [ALLSVENSKAN_ID, SERIE_A_ID];

        assert_eq!(resolve_competition_index(&ids, ALLSVENSKAN_ID, None), 0);
        assert_eq!(resolve_competition_index(&ids, SERIE_A_ID, None), 1);
        assert_eq!(resolve_competition_index(&ids, "unknown", None), 0);
        assert_eq!(resolve_competition_index(&[], ALLSVENSKAN_ID, None), 0);
    }

    #[test]
    fn resolve_competition_index_applies_override_and_wraps_out_of_bounds() {
        let ids = [ALLSVENSKAN_ID, SERIE_A_ID];

        assert_eq!(resolve_competition_index(&ids, ALLSVENSKAN_ID, Some(1)), 1);
        assert_eq!(resolve_competition_index(&ids, SERIE_A_ID, Some(0)), 0);
        assert_eq!(resolve_competition_index(&ids, ALLSVENSKAN_ID, Some(2)), 0);
        assert_eq!(resolve_competition_index(&ids, ALLSVENSKAN_ID, Some(5)), 1);
        assert_eq!(resolve_competition_index(&[], ALLSVENSKAN_ID, Some(3)), 0);
    }

    #[test]
    fn apply_panel_refetch_refreshes_matching_stale_provider_to_fresh() {
        let now = datetime!(2026-11-24 20:00:00 UTC);
        let mut weather_store = WeatherSnapshotStore::new();
        let mut departures_board = DeparturesBoard::new();
        let mut standings_board = StandingsBoard::new();

        refresh_fixture_weather(
            &mut weather_store,
            now - WEATHER_STALE_AFTER - Duration::minutes(5),
            now,
        );
        refresh_fixture_departures(
            &mut departures_board,
            now,
            now - DEPARTURES_STALE_AFTER - Duration::minutes(1),
        );
        refresh_fixture_standings(
            &mut standings_board,
            now - STANDINGS_STALE_AFTER - Duration::hours(1),
        );

        let duvbo = Station {
            site_id: DUVBO_SITE_ID,
            name: "Duvbo".to_string(),
            mode: TransitMode::Metro,
            walk_time_min: 10,
            direction: "stockholm".to_string(),
        };
        let sundbyberg = Station {
            site_id: SUNDBYBERG_SITE_ID,
            name: "Sundbyberg".to_string(),
            mode: TransitMode::Train,
            walk_time_min: 15,
            direction: "stockholm".to_string(),
        };

        assert!(matches!(
            weather_store.state(now),
            WeatherPanelState::Stale { .. }
        ));
        assert!(apply_panel_refetch(
            "weather",
            now,
            &mut weather_store,
            &mut departures_board,
            &mut standings_board,
        ));
        assert!(matches!(
            weather_store.state(now),
            WeatherPanelState::Fresh { .. }
        ));

        assert!(matches!(
            departures_board.station_state(&duvbo, now),
            DeparturesPanelState::Stale { .. }
        ));
        assert!(apply_panel_refetch(
            "departures:9324",
            now,
            &mut weather_store,
            &mut departures_board,
            &mut standings_board,
        ));
        assert!(matches!(
            departures_board.station_state(&duvbo, now),
            DeparturesPanelState::Fresh(_)
        ));
        assert!(matches!(
            departures_board.station_state(&sundbyberg, now),
            DeparturesPanelState::Stale { .. }
        ));

        assert!(apply_panel_refetch(
            "departures:9325",
            now,
            &mut weather_store,
            &mut departures_board,
            &mut standings_board,
        ));
        assert!(matches!(
            departures_board.station_state(&sundbyberg, now),
            DeparturesPanelState::Fresh(_)
        ));

        assert!(matches!(
            standings_board.competition_state(ALLSVENSKAN_ID, now),
            StandingsPanelState::Stale { .. }
        ));
        assert!(apply_panel_refetch(
            "standings:allsvenskan",
            now,
            &mut weather_store,
            &mut departures_board,
            &mut standings_board,
        ));
        assert!(matches!(
            standings_board.competition_state(ALLSVENSKAN_ID, now),
            StandingsPanelState::Fresh(_)
        ));
        assert!(matches!(
            standings_board.competition_state(SERIE_A_ID, now),
            StandingsPanelState::Stale { .. }
        ));

        assert!(apply_panel_refetch(
            "standings:serie-a",
            now,
            &mut weather_store,
            &mut departures_board,
            &mut standings_board,
        ));
        assert!(matches!(
            standings_board.competition_state(SERIE_A_ID, now),
            StandingsPanelState::Fresh(_)
        ));

        assert!(!apply_panel_refetch(
            "unknown:provider",
            now,
            &mut weather_store,
            &mut departures_board,
            &mut standings_board,
        ));
    }
}
