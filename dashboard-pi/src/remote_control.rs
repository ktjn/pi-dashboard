use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use dashboard_core::departures::DeparturesPanelState;
use dashboard_core::schedule::{
    DisplayState, OverridePower, OverrideState, active_override_power, is_within_display_window,
    next_window_boundary, reconcile,
};
use dashboard_core::screensaver::CalendarPollState;
use dashboard_core::standings::StandingsPanelState;
use dashboard_core::weather::WeatherPanelState;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// `override-request`'s `action` vocabulary. `spec.yaml` only specifies the
/// field is a `string`; `pi-dashboard-spec.md`'s PI-08 section describes an
/// "off" request whose override "holds until the next schedule boundary",
/// so `Off`/`On` are the two power directions `reconcile` already knows
/// (`OverridePower`), and `Clear` is the minimal third action needed to let
/// an operator cancel an override early instead of waiting out its expiry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideAction {
    On,
    Off,
    Clear,
}

impl OverrideAction {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "on" => Some(Self::On),
            "off" => Some(Self::Off),
            "clear" => Some(Self::Clear),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct OverrideRequest {
    pub action: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StatusReport {
    pub state: String,
    pub panel_freshness: String,
    pub uptime_s: u64,
    pub version: String,
}

/// The override file's on-disk shape - `power-schedule-state`'s fields,
/// specialised to what an override actually needs to record (the file's
/// mere presence already implies `current_state = manual-override`).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct OverrideFile {
    power: String,
    override_until: String,
}

#[derive(Debug)]
pub enum OverrideFileError {
    Io(String),
    Malformed(String),
}

impl fmt::Display for OverrideFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(f, "override file I/O failed: {message}"),
            Self::Malformed(message) => write!(f, "override file is malformed: {message}"),
        }
    }
}

impl std::error::Error for OverrideFileError {}

/// Writes the override file atomically: serialize to a temp file in the
/// same directory, then `rename` over the real path. `rename` on both
/// Windows and POSIX replaces the destination in a single filesystem
/// operation, so a concurrent reader (the display-power reconciliation
/// thread, reading once a tick) only ever observes the file in its
/// complete old state or its complete new state - never a partially
/// written one. This is `req-override-concurrency`'s real mitigation: the
/// reconciler always reads a consistent override state because there is no
/// instant at which the file on disk is half-written.
fn write_override_atomic(
    path: &Path,
    power: OverridePower,
    expires_at: OffsetDateTime,
) -> Result<(), OverrideFileError> {
    let file = OverrideFile {
        power: match power {
            OverridePower::On => "on".to_string(),
            OverridePower::Off => "off".to_string(),
        },
        override_until: expires_at
            .format(&Rfc3339)
            .map_err(|error| OverrideFileError::Malformed(error.to_string()))?,
    };
    let contents = serde_json::to_vec(&file)
        .map_err(|error| OverrideFileError::Malformed(error.to_string()))?;

    let tmp_path = tmp_path_for(path);
    std::fs::write(&tmp_path, &contents)
        .map_err(|error| OverrideFileError::Io(error.to_string()))?;
    std::fs::rename(&tmp_path, path).map_err(|error| OverrideFileError::Io(error.to_string()))?;
    Ok(())
}

/// Per-call unique counter mixed into every temp-file name `tmp_path_for`
/// produces. A single fixed `<path>.tmp` name is unsafe under concurrent
/// writers: two `POST /control` requests landing on different `axum`/tokio
/// workers can interleave as `A:write(tmp) -> B:write(tmp) -> A:rename(tmp
/// -> path) -> B:rename(tmp -> path)`, where B's temp file has already been
/// renamed away by A by the time B tries to rename it, producing a spurious
/// I/O error (`ENOENT` on POSIX, "Access denied" on Windows) instead of a
/// successful write. Mixing in the process id as well keeps names unique
/// even across process restarts that might race on counter reuse.
static TMP_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_path_for(path: &Path) -> PathBuf {
    let pid = std::process::id();
    let unique = TMP_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{pid}.{unique}.tmp"));
    PathBuf::from(tmp)
}

/// Removes the override file for a `"clear"` action. A missing file is not
/// an error - "no override" is the already-cleared state.
fn clear_override_file(path: &Path) -> Result<(), OverrideFileError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(OverrideFileError::Io(error.to_string())),
    }
}

/// Reads the override file, if any. A missing file, unreadable file, or
/// malformed contents are all treated as "no override" rather than a
/// propagated error - same resilience posture as the other panels' feeds
/// (`req-resilience-offline`): a corrupt override file must never crash or
/// block the reconciler, it should simply fall back to normal scheduling.
pub fn read_override_state(path: &Path) -> Option<OverrideState> {
    let contents = std::fs::read(path).ok()?;
    let file: OverrideFile = serde_json::from_slice(&contents).ok()?;
    let power = match file.power.as_str() {
        "on" => OverridePower::On,
        "off" => OverridePower::Off,
        _ => return None,
    };
    let expires_at = OffsetDateTime::parse(&file.override_until, &Rfc3339).ok()?;
    Some(OverrideState { power, expires_at })
}

#[derive(Clone)]
pub struct RemoteControlState {
    pub expected_token: Arc<String>,
    pub override_path: Arc<PathBuf>,
    pub time_zone: Arc<String>,
    pub display_windows: Arc<Vec<dashboard_core::board_config::DisplayWindow>>,
    pub stations: Arc<Vec<dashboard_core::board_config::Station>>,
    pub competition_ids: Arc<Vec<String>>,
    pub weather_store: Arc<Mutex<dashboard_core::weather::WeatherSnapshotStore>>,
    pub departures_board: Arc<Mutex<dashboard_core::departures::DeparturesBoard>>,
    pub standings_board: Arc<Mutex<dashboard_core::standings::StandingsBoard>>,
    pub calendar_poll_state: Arc<Mutex<CalendarPollState>>,
    pub started_at: Instant,
}

pub fn remote_control_router(state: RemoteControlState) -> Router {
    Router::new()
        .route("/control", post(control_handler))
        .route("/status", get(status_handler))
        .with_state(state)
}

/// Bearer-token check, applied by hand inside each handler rather than as
/// separate `axum` middleware - with only two routes, a shared helper gives
/// the same guarantee (`req-remote-control-security`: "the control endpoint
/// rejects any request without a valid bearer token") without pulling in
/// `axum::middleware`'s extra indirection. The token is read once per
/// request from the already-resolved `expected_token` (sourced from the
/// environment at startup, never from the repo) and never logged - only
/// whether it matched.
fn authorize(headers: &HeaderMap, expected_token: &str) -> bool {
    let Some(header_value) = headers.get(header::AUTHORIZATION) else {
        return false;
    };
    let Ok(header_str) = header_value.to_str() else {
        return false;
    };
    let Some(presented) = header_str.strip_prefix("Bearer ") else {
        return false;
    };
    presented == expected_token
}

async fn control_handler(
    State(state): State<RemoteControlState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !authorize(&headers, &state.expected_token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let request: OverrideRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => return (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    };
    let Some(action) = OverrideAction::parse(&request.action) else {
        return (
            StatusCode::BAD_REQUEST,
            format!("'{}' is not a recognized action", request.action),
        )
            .into_response();
    };

    let now_utc = OffsetDateTime::now_utc();
    let write_result = match action {
        OverrideAction::Clear => clear_override_file(&state.override_path),
        OverrideAction::On | OverrideAction::Off => {
            let power = if action == OverrideAction::On {
                OverridePower::On
            } else {
                OverridePower::Off
            };
            match next_window_boundary(&state.display_windows, &state.time_zone, now_utc) {
                Ok(expires_at) => write_override_atomic(&state.override_path, power, expires_at),
                Err(error) => {
                    return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
                }
            }
        }
    };
    if let Err(error) = write_result {
        eprintln!("override file write failed: {error}");
        return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
    }

    Json(build_status_report(&state, now_utc)).into_response()
}

async fn status_handler(State(state): State<RemoteControlState>, headers: HeaderMap) -> Response {
    if !authorize(&headers, &state.expected_token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let now_utc = OffsetDateTime::now_utc();
    Json(build_status_report(&state, now_utc)).into_response()
}

fn panel_freshness_label(label: &str, fresh: bool, stale: bool) -> String {
    let status = if fresh {
        "fresh"
    } else if stale {
        "stale"
    } else {
        "unavailable"
    };
    format!("{label}={status}")
}

/// Builds `status-report` from each panel's already-tracked
/// Fresh/Stale/Unavailable state (`req-remote-control-observability`:
/// "status reports include per-panel data freshness, uptime and deployed
/// version"). `panel_freshness` is a plain string (`spec.yaml`'s
/// `status-report` data model declares it as `string`, not a nested
/// object), so panels are joined as `name=status` pairs rather than
/// re-encoding a second JSON document inside the field.
fn build_status_report(state: &RemoteControlState, now_utc: OffsetDateTime) -> StatusReport {
    let override_state = read_override_state(&state.override_path);
    let override_active = active_override_power(override_state, now_utc);
    let screensaver_active = state
        .calendar_poll_state
        .lock()
        .expect("calendar poll state mutex poisoned")
        .screensaver_active();
    let schedule_says_awake =
        is_within_display_window(&state.display_windows, &state.time_zone, now_utc).unwrap_or(true);
    let display_state = reconcile(schedule_says_awake, override_active, screensaver_active);

    let state_label = match display_state {
        DisplayState::Normal => "normal",
        DisplayState::ScheduledStandby => "scheduled-standby",
        DisplayState::ManualOverride(_) => "manual-override",
        DisplayState::Screensaver => "screensaver",
    };

    let weather = state
        .weather_store
        .lock()
        .expect("weather store mutex poisoned")
        .state(now_utc);
    let weather_label = panel_freshness_label(
        "weather",
        matches!(weather, WeatherPanelState::Fresh { .. }),
        matches!(weather, WeatherPanelState::Stale { .. }),
    );

    let departures_label = {
        let board = state
            .departures_board
            .lock()
            .expect("departures board mutex poisoned");
        let mut any_fresh = false;
        let mut any_stale = false;
        for station in state.stations.iter() {
            match board.station_state(station, now_utc) {
                DeparturesPanelState::Fresh(_) => any_fresh = true,
                DeparturesPanelState::Stale { .. } => any_stale = true,
                DeparturesPanelState::Unavailable => {}
            }
        }
        panel_freshness_label("departures", any_fresh, any_stale)
    };

    let standings_label = {
        let board = state
            .standings_board
            .lock()
            .expect("standings board mutex poisoned");
        let mut any_fresh = false;
        let mut any_stale = false;
        for competition_id in state.competition_ids.iter() {
            match board.competition_state(competition_id, now_utc) {
                StandingsPanelState::Fresh(_) => any_fresh = true,
                StandingsPanelState::Stale { .. } => any_stale = true,
                StandingsPanelState::Unavailable => {}
            }
        }
        panel_freshness_label("standings", any_fresh, any_stale)
    };

    let screensaver_label = {
        let availability = state
            .calendar_poll_state
            .lock()
            .expect("calendar poll state mutex poisoned")
            .availability(now_utc);
        panel_freshness_label(
            "screensaver",
            matches!(
                availability,
                dashboard_core::screensaver::CalendarAvailability::Fresh
            ),
            matches!(
                availability,
                dashboard_core::screensaver::CalendarAvailability::Stale { .. }
            ),
        )
    };

    let panel_freshness = [
        weather_label,
        departures_label,
        standings_label,
        screensaver_label,
    ]
    .join(",");

    StatusReport {
        state: state_label.to_string(),
        panel_freshness,
        uptime_s: state.started_at.elapsed().as_secs(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use dashboard_core::board_config::DayType;
    use dashboard_core::departures::DeparturesBoard;
    use dashboard_core::standings::StandingsBoard;
    use dashboard_core::weather::WeatherSnapshotStore;
    use http_body_util::BodyExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tower::ServiceExt;

    static UNIQUE_SUFFIX: AtomicU64 = AtomicU64::new(0);

    fn unique_override_path(label: &str) -> PathBuf {
        let suffix = UNIQUE_SUFFIX.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "pi-dashboard-remote-control-test-{label}-{suffix}.json"
        ))
    }

    fn weekday_windows() -> Vec<dashboard_core::board_config::DisplayWindow> {
        vec![dashboard_core::board_config::DisplayWindow {
            day_type: DayType::Weekday,
            start: time::Time::from_hms(6, 0, 0).expect("valid time"),
            end: time::Time::from_hms(22, 0, 0).expect("valid time"),
        }]
    }

    fn test_state(override_path: PathBuf, expected_token: &str) -> RemoteControlState {
        test_state_with_windows(override_path, expected_token, weekday_windows())
    }

    fn test_state_with_windows(
        override_path: PathBuf,
        expected_token: &str,
        display_windows: Vec<dashboard_core::board_config::DisplayWindow>,
    ) -> RemoteControlState {
        RemoteControlState {
            expected_token: Arc::new(expected_token.to_string()),
            override_path: Arc::new(override_path),
            time_zone: Arc::new("Europe/Stockholm".to_string()),
            display_windows: Arc::new(display_windows),
            stations: Arc::new(Vec::new()),
            competition_ids: Arc::new(Vec::new()),
            weather_store: Arc::new(Mutex::new(WeatherSnapshotStore::new())),
            departures_board: Arc::new(Mutex::new(DeparturesBoard::new())),
            standings_board: Arc::new(Mutex::new(StandingsBoard::new())),
            calendar_poll_state: Arc::new(Mutex::new(CalendarPollState::new())),
            started_at: Instant::now(),
        }
    }

    async fn body_bytes(response: Response) -> Vec<u8> {
        response
            .into_body()
            .collect()
            .await
            .expect("body must be readable")
            .to_bytes()
            .to_vec()
    }

    #[tokio::test]
    async fn control_rejects_a_request_with_no_authorization_header() {
        let override_path = unique_override_path("no-header");
        let router = remote_control_router(test_state(override_path, "secret-token"));
        let request = Request::builder()
            .method("POST")
            .uri("/control")
            .header("content-type", "application/json")
            .body(Body::from("{\"action\":\"off\"}"))
            .expect("request must build");
        let response = router.oneshot(request).await.expect("router must respond");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn control_rejects_a_request_with_an_invalid_token() {
        let override_path = unique_override_path("bad-token");
        let router = remote_control_router(test_state(override_path, "secret-token"));
        let request = Request::builder()
            .method("POST")
            .uri("/control")
            .header("authorization", "Bearer wrong-token")
            .header("content-type", "application/json")
            .body(Body::from("{\"action\":\"off\"}"))
            .expect("request must build");
        let response = router.oneshot(request).await.expect("router must respond");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn status_rejects_a_request_with_no_authorization_header() {
        let override_path = unique_override_path("status-no-header");
        let router = remote_control_router(test_state(override_path, "secret-token"));
        let request = Request::builder()
            .method("GET")
            .uri("/status")
            .body(Body::empty())
            .expect("request must build");
        let response = router.oneshot(request).await.expect("router must respond");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn status_accepts_a_request_with_a_valid_token() {
        let override_path = unique_override_path("status-valid-token");
        let router = remote_control_router(test_state(override_path, "secret-token"));
        let request = Request::builder()
            .method("GET")
            .uri("/status")
            .header("authorization", "Bearer secret-token")
            .body(Body::empty())
            .expect("request must build");
        let response = router.oneshot(request).await.expect("router must respond");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn control_off_writes_an_override_file_the_reconciler_can_read_back() {
        let override_path = unique_override_path("round-trip-off");
        let state = test_state(override_path.clone(), "secret-token");
        let router = remote_control_router(state);
        let request = Request::builder()
            .method("POST")
            .uri("/control")
            .header("authorization", "Bearer secret-token")
            .header("content-type", "application/json")
            .body(Body::from("{\"action\":\"off\"}"))
            .expect("request must build");
        let response = router.oneshot(request).await.expect("router must respond");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = body_bytes(response).await;
        let report: StatusReport =
            serde_json::from_slice(&bytes).expect("response body must be a valid status-report");
        assert_eq!(
            report.state, "manual-override",
            "response body must reflect the applied override state"
        );

        let override_state =
            read_override_state(&override_path).expect("override file must be readable back");
        assert_eq!(override_state.power, OverridePower::Off);
        assert!(override_state.expires_at > OffsetDateTime::now_utc());

        std::fs::remove_file(&override_path).ok();
    }

    #[tokio::test]
    async fn control_on_then_clear_removes_the_override_file() {
        let override_path = unique_override_path("round-trip-clear");
        let state = test_state(override_path.clone(), "secret-token");
        let router = remote_control_router(state.clone());
        let on_request = Request::builder()
            .method("POST")
            .uri("/control")
            .header("authorization", "Bearer secret-token")
            .header("content-type", "application/json")
            .body(Body::from("{\"action\":\"on\"}"))
            .expect("request must build");
        router
            .clone()
            .oneshot(on_request)
            .await
            .expect("router must respond");
        assert!(read_override_state(&override_path).is_some());

        let clear_request = Request::builder()
            .method("POST")
            .uri("/control")
            .header("authorization", "Bearer secret-token")
            .header("content-type", "application/json")
            .body(Body::from("{\"action\":\"clear\"}"))
            .expect("request must build");
        let response = router
            .oneshot(clear_request)
            .await
            .expect("router must respond");
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = body_bytes(response).await;
        let report: StatusReport =
            serde_json::from_slice(&bytes).expect("response body must be a valid status-report");
        assert_ne!(
            report.state, "manual-override",
            "response body must reflect that the override was cleared"
        );
        assert!(read_override_state(&override_path).is_none());
    }

    #[tokio::test]
    async fn control_rejects_an_unrecognized_action() {
        let override_path = unique_override_path("bad-action");
        let router = remote_control_router(test_state(override_path, "secret-token"));
        let request = Request::builder()
            .method("POST")
            .uri("/control")
            .header("authorization", "Bearer secret-token")
            .header("content-type", "application/json")
            .body(Body::from("{\"action\":\"explode\"}"))
            .expect("request must build");
        let response = router.oneshot(request).await.expect("router must respond");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// Any file in `path`'s parent directory whose name starts with `path`'s
    /// own file name and ends in `.tmp` - i.e. a leftover `tmp_path_for`
    /// staging file that was never cleaned up by a successful rename.
    fn leftover_tmp_files(path: &Path) -> Vec<PathBuf> {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            return Vec::new();
        };
        let Some(parent) = path.parent() else {
            return Vec::new();
        };
        let Ok(entries) = std::fs::read_dir(parent) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|candidate| {
                candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(file_name) && name.ends_with(".tmp"))
            })
            .collect()
    }

    #[test]
    fn write_override_atomic_leaves_no_temp_file_behind_under_concurrent_writers() {
        // Single-writer coverage alone can't discriminate here: a fixed
        // `<path>.tmp` name never collides, and a leftover temp file is
        // vacuously absent in a sequential test too. Spawning several
        // concurrent writer threads against the same path is what actually
        // exercises `tmp_path_for`'s per-call uniqueness (fix 2) and
        // confirms no staging file is ever left behind once every write has
        // completed - not just that a single write happens to clean up.
        let override_path = unique_override_path("atomic-write-concurrent");
        let writer_count = 8;
        let writes_per_writer = 50;

        let mut writers = Vec::new();
        for _ in 0..writer_count {
            let writer_path = override_path.clone();
            writers.push(std::thread::spawn(move || {
                for _ in 0..writes_per_writer {
                    write_override_atomic(
                        &writer_path,
                        OverridePower::On,
                        OffsetDateTime::now_utc() + time::Duration::hours(1),
                    )
                    .expect("write must succeed");
                }
            }));
        }
        for writer in writers {
            writer.join().expect("writer thread must not panic");
        }

        assert!(override_path.exists());
        let state = read_override_state(&override_path).expect("file must parse");
        assert_eq!(state.power, OverridePower::On);
        assert_eq!(
            leftover_tmp_files(&override_path),
            Vec::<PathBuf>::new(),
            "a staging temp file was left behind after all writers finished"
        );

        std::fs::remove_file(&override_path).ok();
    }

    #[test]
    fn concurrent_writers_to_the_same_override_path_never_hit_a_temp_file_rename_collision() {
        // `req-override-concurrency`'s companion failure mode to the
        // torn-read test below: this one is about writer-vs-writer
        // collision on the shared staging file, not writer-vs-reader
        // visibility. Before fix 2, a fixed `<path>.tmp` name let two
        // concurrent writers interleave as
        // `A:write(tmp) -> B:write(tmp) -> A:rename(tmp->path) ->
        // B:rename(tmp->path)`, so B's rename failed with ENOENT because
        // A's rename had already consumed the shared tmp file. Every write
        // across every thread must succeed once each writer's temp file is
        // unique.
        let override_path = unique_override_path("writer-collision");
        let writer_count = 8;
        let writes_per_writer = 125;

        let mut writers = Vec::new();
        for i in 0..writer_count {
            let writer_path = override_path.clone();
            writers.push(std::thread::spawn(move || {
                let mut errors = Vec::new();
                for j in 0..writes_per_writer {
                    let power = if (i + j) % 2 == 0 {
                        OverridePower::On
                    } else {
                        OverridePower::Off
                    };
                    let expires_at = OffsetDateTime::now_utc() + time::Duration::hours(1);
                    if let Err(error) = write_override_atomic(&writer_path, power, expires_at) {
                        errors.push(error.to_string());
                    }
                }
                errors
            }));
        }

        let mut all_errors = Vec::new();
        for writer in writers {
            all_errors.extend(writer.join().expect("writer thread must not panic"));
        }

        assert!(
            all_errors.is_empty(),
            "expected zero I/O errors from concurrent writers, got: {all_errors:?}"
        );

        std::fs::remove_file(&override_path).ok();
    }

    #[test]
    fn read_override_state_treats_a_missing_file_as_no_override() {
        let override_path = unique_override_path("missing");
        assert!(read_override_state(&override_path).is_none());
    }

    #[test]
    fn read_override_state_treats_malformed_contents_as_no_override_without_panicking() {
        let override_path = unique_override_path("malformed");
        std::fs::write(&override_path, b"not json").expect("fixture write must succeed");
        assert!(read_override_state(&override_path).is_none());
        std::fs::remove_file(&override_path).ok();
    }

    #[tokio::test]
    async fn status_reports_normal_state_when_nothing_else_is_active() {
        let override_path = unique_override_path("status-normal");
        let state = test_state(override_path, "secret-token");
        let router = remote_control_router(state);
        let request = Request::builder()
            .method("GET")
            .uri("/status")
            .header("authorization", "Bearer secret-token")
            .body(Body::empty())
            .expect("request must build");
        let response = router.oneshot(request).await.expect("router must respond");
        let bytes = body_bytes(response).await;
        let report: StatusReport =
            serde_json::from_slice(&bytes).expect("response must be a valid status-report");
        assert!(report.panel_freshness.contains("weather="));
        assert!(report.panel_freshness.contains("departures="));
        assert!(report.panel_freshness.contains("standings="));
        assert!(report.panel_freshness.contains("screensaver="));
        assert!(!report.version.is_empty());
        assert!(report.uptime_s < 60);
    }

    #[tokio::test]
    async fn status_reports_scheduled_standby_when_no_window_is_configured() {
        // No configured display windows means `is_within_display_window`
        // is always `false`, regardless of the real wall-clock time the
        // test happens to run at - this keeps the test deterministic
        // without needing to inject "now" through the HTTP layer.
        let override_path = unique_override_path("status-standby");
        let state = test_state_with_windows(override_path, "secret-token", Vec::new());
        let router = remote_control_router(state);
        let request = Request::builder()
            .method("GET")
            .uri("/status")
            .header("authorization", "Bearer secret-token")
            .body(Body::empty())
            .expect("request must build");
        let response = router.oneshot(request).await.expect("router must respond");
        let bytes = body_bytes(response).await;
        let report: StatusReport =
            serde_json::from_slice(&bytes).expect("response must be a valid status-report");
        assert_eq!(report.state, "scheduled-standby");
    }

    #[tokio::test]
    async fn status_reports_screensaver_when_the_schedule_is_awake_and_the_screensaver_is_active() {
        // A window spanning the whole day in both day-types makes
        // `is_within_display_window` `true` at any wall-clock time the
        // test happens to run at, so only `screensaver_active` decides
        // the outcome.
        let all_day_windows = vec![
            dashboard_core::board_config::DisplayWindow {
                day_type: DayType::Weekday,
                start: time::Time::from_hms(0, 0, 0).expect("valid time"),
                end: time::Time::from_hms(23, 59, 59).expect("valid time"),
            },
            dashboard_core::board_config::DisplayWindow {
                day_type: DayType::Weekend,
                start: time::Time::from_hms(0, 0, 0).expect("valid time"),
                end: time::Time::from_hms(23, 59, 59).expect("valid time"),
            },
        ];
        let override_path = unique_override_path("status-screensaver");
        let state = test_state_with_windows(override_path, "secret-token", all_day_windows);
        state
            .calendar_poll_state
            .lock()
            .expect("calendar poll state mutex poisoned")
            .record_success(true, OffsetDateTime::now_utc());
        let router = remote_control_router(state);
        let request = Request::builder()
            .method("GET")
            .uri("/status")
            .header("authorization", "Bearer secret-token")
            .body(Body::empty())
            .expect("request must build");
        let response = router.oneshot(request).await.expect("router must respond");
        let bytes = body_bytes(response).await;
        let report: StatusReport =
            serde_json::from_slice(&bytes).expect("response must be a valid status-report");
        assert_eq!(report.state, "screensaver");
    }

    #[tokio::test]
    async fn status_reports_manual_override_state_after_a_control_off_call() {
        let override_path = unique_override_path("status-override");
        let state = test_state(override_path.clone(), "secret-token");
        let router = remote_control_router(state.clone());
        let control_request = Request::builder()
            .method("POST")
            .uri("/control")
            .header("authorization", "Bearer secret-token")
            .header("content-type", "application/json")
            .body(Body::from("{\"action\":\"off\"}"))
            .expect("request must build");
        router
            .clone()
            .oneshot(control_request)
            .await
            .expect("router must respond");

        let status_request = Request::builder()
            .method("GET")
            .uri("/status")
            .header("authorization", "Bearer secret-token")
            .body(Body::empty())
            .expect("request must build");
        let response = router
            .oneshot(status_request)
            .await
            .expect("router must respond");
        let bytes = body_bytes(response).await;
        let report: StatusReport =
            serde_json::from_slice(&bytes).expect("response must be a valid status-report");
        assert_eq!(report.state, "manual-override");

        std::fs::remove_file(&override_path).ok();
    }

    // --- req-override-concurrency: a concurrent writer and reader never
    // observe a torn file ---

    #[test]
    fn concurrent_writes_and_reads_never_observe_a_torn_or_mixed_override_file() {
        // A discriminating race test: one thread repeatedly overwrites the
        // override file alternating between two distinct, fully-valid
        // states (`On` expiring in 1h, `Off` expiring in 2h) while several
        // reader threads repeatedly read it back. If the write were not
        // atomic (e.g. truncate-then-write-in-place instead of
        // write-temp-then-rename), a reader could observe a partially
        // written file - either a parse failure, a truncated read, or
        // (worse) a file whose bytes mix the two states' fields.
        //
        // The file is seeded with a known-good value *before* any
        // reader/writer thread is spawned. That seed is what gives the
        // `None` branch teeth: `read_override_state` maps every failure (an
        // I/O error, a truncated file, garbage bytes) to `None`, so if the
        // test only inspected `Some` results, the dominant real failure
        // mode under a non-atomic write - the reader seeing "no override"
        // mid-write - would be silently treated as a non-event. Once the
        // file is known to exist before the race starts, a `None` read can
        // only mean the write path tore the file; it is asserted as a
        // failure here, not skipped.
        let override_path = unique_override_path("race");
        let on_expires_at = OffsetDateTime::now_utc() + time::Duration::hours(1);
        let off_expires_at = OffsetDateTime::now_utc() + time::Duration::hours(2);
        let iterations = 500;

        write_override_atomic(&override_path, OverridePower::On, on_expires_at)
            .expect("seed write must succeed");

        let writer_path = override_path.clone();
        let writer = std::thread::spawn(move || {
            for i in 0..iterations {
                let (power, expires_at) = if i % 2 == 0 {
                    (OverridePower::On, on_expires_at)
                } else {
                    (OverridePower::Off, off_expires_at)
                };
                write_override_atomic(&writer_path, power, expires_at).expect("write must succeed");
            }
        });

        let mut readers = Vec::new();
        for _ in 0..4 {
            let reader_path = override_path.clone();
            readers.push(std::thread::spawn(move || {
                let mut none_reads = 0usize;
                for _ in 0..iterations {
                    match read_override_state(&reader_path) {
                        Some(state) => {
                            let is_known_good = (state.power == OverridePower::On
                                && state.expires_at == on_expires_at)
                                || (state.power == OverridePower::Off
                                    && state.expires_at == off_expires_at);
                            assert!(
                                is_known_good,
                                "observed a torn/mixed override state: {state:?}"
                            );
                        }
                        None => none_reads += 1,
                    }
                }
                none_reads
            }));
        }

        writer.join().expect("writer thread must not panic");
        let mut total_none_reads = 0usize;
        for reader in readers {
            total_none_reads += reader.join().expect("reader thread must not panic");
        }

        assert_eq!(
            total_none_reads, 0,
            "a read observed no override at all after the file was seeded with a known-good \
             value before the race started - the write path tore the file"
        );

        std::fs::remove_file(&override_path).ok();
    }
}
