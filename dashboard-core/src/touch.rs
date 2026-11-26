use time::{Duration, OffsetDateTime};

/// `[touch]` in `board.toml`. `enabled = false` (or the table absent, see
/// `board_config.rs`) disables all touch handling per `req-touch-resilience`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchConfig {
    pub enabled: bool,
    pub wake_seconds: u32,
    pub screensaver_dismiss_seconds: u32,
    pub idle_timeout_seconds: u32,
}

impl TouchConfig {
    pub const DEFAULT_WAKE_SECONDS: u32 = 120;
    pub const DEFAULT_SCREENSAVER_DISMISS_SECONDS: u32 = 45;
    pub const DEFAULT_IDLE_TIMEOUT_SECONDS: u32 = 45;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchMode {
    Default,
    Wake,
    ScreensaverDismissed,
}

/// `spec.yaml`'s `touch-state` data model. `last_input_at` is a single,
/// global "most recent touch of any kind" timestamp rather than one per
/// field - `revert_idle` uses it to expire `football_index` and
/// `expanded_departure` together, matching the one-field shape the spec
/// actually defines instead of inventing a per-widget timer each.
/// `wake_until`/`screensaver_dismissed_until` are independent absolute
/// expiries set once at entry, never slid forward by later taps - see
/// `touch_never_extends_wake_past_configured_duration`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TouchState {
    pub mode: TouchMode,
    pub wake_until: Option<OffsetDateTime>,
    pub screensaver_dismissed_until: Option<OffsetDateTime>,
    pub football_index: Option<usize>,
    pub expanded_departure: Option<String>,
    pub last_input_at: Option<OffsetDateTime>,
}

impl TouchState {
    pub fn new() -> Self {
        Self {
            mode: TouchMode::Default,
            wake_until: None,
            screensaver_dismissed_until: None,
            football_index: None,
            expanded_departure: None,
            last_input_at: None,
        }
    }
}

impl Default for TouchState {
    fn default() -> Self {
        Self::new()
    }
}

/// What the screen was showing when the touch landed, decided by the caller
/// from the already-resolved schedule/override/screensaver state - `touch.rs`
/// has no access to and no need for that state itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayContext {
    Blanked,
    Screensaver,
    Dashboard,
}

/// What was tapped, only meaningful in `DisplayContext::Dashboard` - a tap
/// while `Blanked` or showing the `Screensaver` wakes/dismisses regardless of
/// target, since nothing panel-specific is visible to tap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TouchTarget {
    Header,
    FootballPanel { panel_count: usize },
    DepartureRow { departure_id: String },
    StalePanel { provider: String },
}

/// A side effect for the caller (`dashboard-pi`) to carry out. `touch.rs`
/// itself does no I/O - see `req-touch-security`: touch can only drive view
/// state and these bounded, temporary requests, never configuration,
/// credentials or permanent power state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TouchEffect {
    WakeDisplay,
    RefreshDeparturesRequested,
    DismissScreensaverView,
    RequestPanelRefetch { provider: String },
}

/// Applies one touch event to `state`. Pure: takes every input it needs as a
/// plain value, including `now`, rather than reading a clock itself.
pub fn apply_touch(
    state: &TouchState,
    context: DisplayContext,
    target: TouchTarget,
    now: OffsetDateTime,
    config: &TouchConfig,
) -> (TouchState, Vec<TouchEffect>) {
    if !config.enabled {
        return (state.clone(), Vec::new());
    }

    match context {
        DisplayContext::Blanked => wake(state, now, config),
        DisplayContext::Screensaver => dismiss_screensaver(state, now, config),
        DisplayContext::Dashboard => dashboard_tap(state, target, now),
    }
}

fn wake(state: &TouchState, now: OffsetDateTime, config: &TouchConfig) -> (TouchState, Vec<TouchEffect>) {
    if state.mode == TouchMode::Wake && state.wake_until.is_some_and(|until| now < until) {
        return (state.clone(), Vec::new());
    }
    let mut next = state.clone();
    next.mode = TouchMode::Wake;
    next.wake_until = Some(now + Duration::seconds(i64::from(config.wake_seconds)));
    next.last_input_at = Some(now);
    (
        next,
        vec![TouchEffect::WakeDisplay, TouchEffect::RefreshDeparturesRequested],
    )
}

fn dismiss_screensaver(
    state: &TouchState,
    now: OffsetDateTime,
    config: &TouchConfig,
) -> (TouchState, Vec<TouchEffect>) {
    if state.mode == TouchMode::ScreensaverDismissed
        && state.screensaver_dismissed_until.is_some_and(|until| now < until)
    {
        return (state.clone(), Vec::new());
    }
    let mut next = state.clone();
    next.mode = TouchMode::ScreensaverDismissed;
    next.screensaver_dismissed_until =
        Some(now + Duration::seconds(i64::from(config.screensaver_dismiss_seconds)));
    next.last_input_at = Some(now);
    (next, vec![TouchEffect::DismissScreensaverView])
}

fn dashboard_tap(
    state: &TouchState,
    target: TouchTarget,
    now: OffsetDateTime,
) -> (TouchState, Vec<TouchEffect>) {
    match target {
        TouchTarget::Header => (state.clone(), Vec::new()),
        TouchTarget::FootballPanel { panel_count } => {
            if panel_count == 0 {
                return (state.clone(), Vec::new());
            }
            let mut next = state.clone();
            let current = state.football_index.unwrap_or(0);
            next.football_index = Some((current + 1) % panel_count);
            next.last_input_at = Some(now);
            (next, Vec::new())
        }
        TouchTarget::DepartureRow { departure_id } => {
            let mut next = state.clone();
            next.expanded_departure = if state.expanded_departure.as_deref() == Some(departure_id.as_str())
            {
                None
            } else {
                Some(departure_id)
            };
            next.last_input_at = Some(now);
            (next, Vec::new())
        }
        TouchTarget::StalePanel { provider } => {
            let mut next = state.clone();
            next.last_input_at = Some(now);
            (next, vec![TouchEffect::RequestPanelRefetch { provider }])
        }
    }
}

/// Clears any touch-driven state whose timeout has passed, independent of
/// new input - the caller ticks this once a loop. This is where
/// `touch_state_reverts_after_idle_timeout` actually takes effect.
pub fn revert_idle(state: &TouchState, now: OffsetDateTime, config: &TouchConfig) -> TouchState {
    let mut next = state.clone();

    if next.mode == TouchMode::Wake && next.wake_until.is_some_and(|until| now >= until) {
        next.mode = TouchMode::Default;
        next.wake_until = None;
    }
    if next.mode == TouchMode::ScreensaverDismissed
        && next
            .screensaver_dismissed_until
            .is_some_and(|until| now >= until)
    {
        next.mode = TouchMode::Default;
        next.screensaver_dismissed_until = None;
    }

    let idle_expired = next.last_input_at.is_some_and(|last_input_at| {
        now >= last_input_at + Duration::seconds(i64::from(config.idle_timeout_seconds))
    });
    if idle_expired {
        next.football_index = None;
        next.expanded_departure = None;
    }

    next
}
