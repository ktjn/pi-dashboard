use dashboard_core::board_config::{MatchType, ScreensaverRule, Station, TransitMode};
use dashboard_core::clock::local_clock;
use dashboard_core::daylight::daylight_window;
use dashboard_core::departures::{
    DEPARTURES_STALE_AFTER, DeparturesBoard, DeparturesSource, DeviationsSource,
    FixtureDeparturesSource, FixtureDeviationsSource, RawDeparture,
};
use dashboard_core::render::{
    STATION_PANEL_HEIGHT, render_clock_weather_header, render_daylight_strip,
    render_departures_panel, render_screensaver_view, render_standings_panel,
};
use dashboard_core::schedule::{DisplayState, reconcile};
use dashboard_core::screensaver::{
    CalendarEvent, CalendarPollState, FixtureCalendarSource, poll_calendar,
};
use dashboard_core::standings::{
    FixtureStandingsSource, STANDINGS_STALE_AFTER, StandingsBoard, StandingsRotation, StandingsRow,
    StandingsSource,
};
use dashboard_core::weather::{
    FixtureWeatherSource, WEATHER_STALE_AFTER, WeatherReading, WeatherSnapshotStore, WeatherSource,
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

/// How often the playground re-polls the fixture calendar relative to its
/// own advancing clock - frequent enough that a viewer watching the
/// playground actually sees the screensaver activate and clear within a
/// reasonable wait, same reasoning as the departures/weather refresh
/// intervals above.
const PLAYGROUND_CALENDAR_POLL_INTERVAL: Duration = Duration::seconds(15);

/// How often the playground re-generates fixture departures relative to
/// its own advancing clock (`now_utc`, not the frozen `base_utc`), so
/// departures keep refreshing instead of elapsing/going stale within a
/// couple of minutes of the playground being opened.
const PLAYGROUND_DEPARTURES_REFRESH_INTERVAL: Duration = Duration::seconds(30);

/// Same reasoning as `PLAYGROUND_DEPARTURES_REFRESH_INTERVAL`, applied to
/// the weather fixture: its `observed_at`/`fetched_at` must track the
/// playground's advancing `now_utc`, not a frozen timestamp, or the
/// weather panel silently goes permanently Stale then Unavailable within
/// minutes of being opened.
const PLAYGROUND_WEATHER_REFRESH_INTERVAL: Duration = Duration::minutes(10);

/// Same reasoning again, applied to the standings fixture - without a
/// periodic refresh here, `StandingsBoard` would only ever hold its
/// one-time startup snapshot and never demonstrate ongoing normal
/// operation.
const PLAYGROUND_STANDINGS_REFRESH_INTERVAL: Duration = Duration::minutes(5);

/// Phase 7 step 1 asks for fixture coverage of each panel's
/// normal/stale/unavailable states, not just "Fresh". Departures' (2 min),
/// weather's (20 min) and standings' (2h) real staleness thresholds are
/// too long to wait out live in a playground session, so instead of
/// literally waiting, each of those three panels is scripted through a
/// short, shared timeline that repeats every `DEMO_CYCLE_LENGTH` (so a
/// viewer who leaves the playground open keeps seeing every state rather
/// than settling into a permanent `Fresh` after one pass):
///
/// - `[0, DEMO_UNAVAILABLE_UNTIL)`: no fetch has been attempted yet (or
///   has been attempted since the current cycle started), so each
///   panel's board/store is empty and genuinely reports `Unavailable` -
///   the same state a real cold boot before the first successful fetch
///   would show.
/// - `[DEMO_UNAVAILABLE_UNTIL, DEMO_STALE_FROM)` and everything from
///   `DEMO_STALE_UNTIL` to `DEMO_CYCLE_LENGTH`: `Fresh`, refreshed
///   periodically per panel exactly as before this phase scripting was
///   added.
/// - `[DEMO_STALE_FROM, DEMO_STALE_UNTIL)`: each panel records one fetch
///   whose own `fetched_at`/`observed_at` is already older than that
///   panel's real staleness threshold - the same age computation a
///   provider outage would hit - without needing to wait out the real
///   threshold.
///
/// The screensaver's fixture calendar event (see `fixture_calendar_events`)
/// is active for `[DEMO_SCREENSAVER_FROM, DEMO_SCREENSAVER_UNTIL)` of the
/// same cycle. `DEMO_STALE_FROM`/`DEMO_STALE_UNTIL` are placed with a
/// comfortable margin after `DEMO_SCREENSAVER_UNTIL` - the screensaver
/// view fully masks every panel (`reconcile` returns `Screensaver` and the
/// render loop draws the screensaver and `continue`s before any panel
/// renders), so a Stale window that overlapped it would never actually be
/// visible to a viewer. The two windows must never overlap or be placed
/// adjacent with no buffer, since poll-timing jitter near a shared
/// boundary could make the Stale state flaky to observe.
///
/// The daylight panel has no such states: it is a pure local calculation
/// with no network dependency, so it is intentionally left out of this
/// scripting and always renders `Fresh`-equivalent (correct) values for
/// the real local date.
const DEMO_UNAVAILABLE_UNTIL: Duration = Duration::seconds(10);
const DEMO_SCREENSAVER_FROM: Duration = Duration::seconds(60);
const DEMO_SCREENSAVER_UNTIL: Duration = Duration::seconds(180);
const DEMO_STALE_FROM: Duration = Duration::seconds(200);
const DEMO_STALE_UNTIL: Duration = Duration::seconds(215);
const DEMO_CYCLE_LENGTH: Duration = Duration::seconds(240);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DemoPhase {
    Unavailable,
    Fresh,
    Stale,
}

fn demo_phase(elapsed_since_start: Duration) -> DemoPhase {
    if elapsed_since_start < DEMO_UNAVAILABLE_UNTIL {
        DemoPhase::Unavailable
    } else if elapsed_since_start >= DEMO_STALE_FROM && elapsed_since_start < DEMO_STALE_UNTIL {
        DemoPhase::Stale
    } else {
        DemoPhase::Fresh
    }
}

/// Fixture departures for the playground: no real network, mirrors the
/// `board.toml` default Duvbo station. Timestamps are generated relative
/// to whatever "now" is passed in, so periodic regeneration keeps them
/// usably fresh.
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

/// Fixture departures for the playground's second configured station
/// (`board.toml`'s Sundbyberg, TRAIN towards Stockholm).
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

/// Builds and drains the fixture departures sources for every configured
/// station, recording a success snapshot for each into `board` - the same
/// `DeparturesSource`/`record_departures_success` path the real
/// `dashboard-pi` refresh threads use, rather than poking `RawDeparture`
/// values into the board directly.
///
/// `generation_time` and `fetched_at` are deliberately separate: departure
/// times are always generated relative to `generation_time` (so they stay
/// sensibly in the future), while `fetched_at` is what
/// `DeparturesPanelState`'s staleness check actually ages against - letting
/// the demo-stale window (see `DEMO_STALE_FROM`) record a fetch that
/// *claims* to be older than `DEPARTURES_STALE_AFTER` without the cached
/// departures themselves looking already-elapsed.
fn refresh_fixture_departures(
    board: &mut DeparturesBoard,
    generation_time: time::OffsetDateTime,
    fetched_at: time::OffsetDateTime,
) {
    let mut departures_responses = HashMap::new();
    departures_responses.insert(DUVBO_SITE_ID, Ok(fixture_duvbo_departures(generation_time)));
    departures_responses.insert(
        SUNDBYBERG_SITE_ID,
        Ok(fixture_sundbyberg_departures(generation_time)),
    );
    let mut departures_source = FixtureDeparturesSource::new(departures_responses);

    let mut deviations_source = FixtureDeviationsSource::new(HashMap::new());

    for site_id in [DUVBO_SITE_ID, SUNDBYBERG_SITE_ID] {
        if let Ok(departures) = departures_source.fetch_departures(site_id) {
            board.record_departures_success(site_id, departures, fetched_at);
        }
        if let Ok(deviations) = deviations_source.fetch_deviations(site_id) {
            board.record_deviations_success(site_id, deviations);
        }
    }
}

/// Refreshes the fixture weather reading, through the same
/// `WeatherSource`/`record_success` path a real provider would use.
/// `observed_at` and `fetched_at` are deliberately separate - see
/// `refresh_fixture_departures` above for why; here `observed_at` is what
/// `WeatherPanelState`'s staleness check ages against, letting the
/// demo-stale window record a reading that claims an old observation time
/// without the surrounding fetch failing outright.
fn refresh_fixture_weather(
    store: &mut WeatherSnapshotStore,
    observed_at: time::OffsetDateTime,
    fetched_at: time::OffsetDateTime,
) {
    let mut weather_source = FixtureWeatherSource::new(vec![Ok(WeatherReading {
        temp_c: 3.5,
        precip_probability_pct: 20,
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

/// Builds and drains the fixture standings sources for both configured
/// competitions, recording a success snapshot for each into `board` -
/// the same `StandingsSource`/`record_success` path a real provider
/// client uses.
fn refresh_fixture_standings(board: &mut StandingsBoard, now: time::OffsetDateTime) {
    let mut responses = HashMap::new();
    responses.insert(ALLSVENSKAN_ID.to_string(), Ok(fixture_allsvenskan_table()));
    responses.insert(SERIE_A_ID.to_string(), Ok(fixture_serie_a_table()));
    let mut source = FixtureStandingsSource::new(responses);

    for competition_id in [ALLSVENSKAN_ID, SERIE_A_ID] {
        if let Ok(rows) = source.fetch_standings(competition_id) {
            board.record_success(competition_id, rows, now);
        }
    }
}

/// Mirrors a `board.toml` `[[screensaver.rules]]` entry so the playground
/// exercises the same substring-matching path a real `board.toml` would.
fn fixture_screensaver_rules() -> Vec<ScreensaverRule> {
    vec![ScreensaverRule {
        pattern: "focus".to_string(),
        match_type: MatchType::Substring,
    }]
}

/// One fixture calendar event, active for `[DEMO_SCREENSAVER_FROM,
/// DEMO_SCREENSAVER_UNTIL)` of the current demo cycle (see
/// `DEMO_CYCLE_LENGTH`'s doc comment), so a viewer watching it actually
/// sees the screensaver take over and the dashboard return - demonstrating
/// both halves of PI-07's acceptance check without needing a real
/// calendar. `cycle_start_utc` is the UTC instant the *current* cycle
/// began (not the playground's fixed startup time), so the event's
/// absolute window is re-derived every cycle and the screensaver keeps
/// recurring on each loop instead of only firing once.
fn fixture_calendar_events(cycle_start_utc: time::OffsetDateTime) -> Vec<CalendarEvent> {
    vec![CalendarEvent {
        event_id: "playground-focus-block".to_string(),
        title: "Focus 14:00-15:00".to_string(),
        start: cycle_start_utc + DEMO_SCREENSAVER_FROM,
        end: cycle_start_utc + DEMO_SCREENSAVER_UNTIL,
    }]
}

/// Drives the fixture calendar through the same `CalendarSource`/
/// `poll_calendar` path a real `dashboard-pi` poll thread uses.
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

#[macroquad::main("pi-dashboard (playground)")]
async fn main() {
    // Fixture data in place of a real provider - the playground never
    // talks to a real network.
    let base_utc = datetime!(2026-11-24 20:00:00 UTC);
    let start_time = macroquad::time::get_time();

    // Weather, departures and standings deliberately start empty rather
    // than eagerly populated: the `DEMO_UNAVAILABLE_UNTIL` window below
    // relies on each board/store genuinely never having recorded a
    // success yet, so their panels render `Unavailable` for the first
    // few seconds the playground is open, same as a real cold boot
    // before the first successful fetch.
    let mut weather_store = WeatherSnapshotStore::new();
    let mut last_weather_refresh = base_utc;

    let mut departures_board = DeparturesBoard::new();
    let mut last_departures_refresh = base_utc;

    let mut standings_board = StandingsBoard::new();
    let mut last_standings_refresh = base_utc;

    let mut last_demo_phase: Option<DemoPhase> = None;

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

    let cycle_length_seconds = DEMO_CYCLE_LENGTH.whole_seconds();

    loop {
        let elapsed_seconds = (macroquad::time::get_time() - start_time).max(0.0) as i64;
        let now_utc = base_utc + Duration::seconds(elapsed_seconds);
        let cycle_index = elapsed_seconds / cycle_length_seconds;
        let elapsed_in_cycle = elapsed_seconds - cycle_index * cycle_length_seconds;
        let cycle_start_utc = base_utc + Duration::seconds(cycle_index * cycle_length_seconds);
        let elapsed_since_start = Duration::seconds(elapsed_in_cycle);

        let phase = demo_phase(elapsed_since_start);
        if Some(phase) != last_demo_phase {
            last_demo_phase = Some(phase);
            match phase {
                // Reset every board/store back to empty so repeat cycles
                // genuinely report `Unavailable` again, rather than still
                // showing whatever Fresh/Stale data survived from the
                // previous cycle (the real staleness thresholds are long
                // enough - departures 2 min, weather 20 min, standings 2h
                // - that leftover data from moments ago would otherwise
                // still read as Fresh here).
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
                    // Each fetch claims a timestamp already older than
                    // that panel's own real staleness threshold, so the
                    // panel reports `Stale` immediately rather than
                    // after literally waiting out the threshold.
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

        macroquad::window::clear_background(macroquad::color::BLACK);

        let clock = local_clock(STOCKHOLM_TZ, now_utc).expect("time zone must resolve");

        // No display-window/override concept in the playground - only the
        // screensaver's own activation boolean drives this branch, same
        // `reconcile` call `dashboard-pi` uses.
        let display_state = reconcile(true, None, calendar_poll_state.screensaver_active());
        if display_state == DisplayState::Screensaver {
            render_screensaver_view(&clock, &calendar_poll_state.availability(now_utc));
            macroquad::window::next_frame().await;
            continue;
        }

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

        let mut panel_top = 140.0;
        for station in &stations {
            let departures_state = departures_board.station_state(station, now_utc);
            render_departures_panel(&departures_state, panel_top);
            panel_top += STATION_PANEL_HEIGHT;
        }

        let active_competition_id = standings_rotation.active_competition(now_utc);
        let standings_state = standings_board.competition_state(active_competition_id, now_utc);
        render_standings_panel(active_competition_id, &standings_state, panel_top);

        macroquad::window::next_frame().await;
    }
}
