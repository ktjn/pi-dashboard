use serde::Deserialize;
use std::fmt;
use time::OffsetDateTime;

/// `spec.yaml`'s `update-agent-state` data model, backing
/// `deploy-release`'s `last_known_good_retained` invariant:
/// `last_known_good_version` is only ever overwritten by
/// `apply_health_check_result` on a passing check, never cleared on a
/// failure, so a candidate that never resolves (or fails) leaves it intact.
/// `candidate_version`/`candidate_digest` are deliberately *not* cleared on
/// rollback - they double as the "last known bad candidate" marker
/// `evaluate_candidate` checks against `req-update-rollback-func`'s "not
/// retried on a later poll", without a separate field the approved data
/// model doesn't have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAgentState {
    pub current_version: String,
    pub last_known_good_version: String,
    pub candidate_version: Option<String>,
    pub candidate_digest: Option<String>,
    pub status: UpdateAgentStatus,
    pub last_checked_at: Option<OffsetDateTime>,
}

impl UpdateAgentState {
    pub fn starting_at(current_version: impl Into<String>) -> Self {
        let current_version = current_version.into();
        Self {
            last_known_good_version: current_version.clone(),
            current_version,
            candidate_version: None,
            candidate_digest: None,
            status: UpdateAgentStatus::Idle,
            last_checked_at: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateAgentStatus {
    Idle,
    Staged,
    RolledBack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseCheckError {
    InvalidJson(String),
    MissingAsset(String),
}

impl fmt::Display for ReleaseCheckError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(message) => {
                write!(f, "release response is not valid JSON: {message}")
            }
            Self::MissingAsset(message) => write!(f, "release is missing an asset: {message}"),
        }
    }
}

impl std::error::Error for ReleaseCheckError {}

/// The bundle and digest download URLs for one GitHub release, resolved
/// from its `tag_name`/`assets` - the only two fields this agent needs out
/// of GitHub's much larger release object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAssets {
    pub version: String,
    pub bundle_url: String,
    pub digest_url: String,
}

#[derive(Deserialize)]
struct RawAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    assets: Vec<RawAsset>,
}

/// Parses GitHub's `GET /repos/.../releases/latest` response shape.
/// `ci.yml` publishes exactly two bundle-related assets per release - the
/// `.tar.gz` bundle and its `.tar.gz.sha256` digest (`req-ci-validates-release`:
/// "a published release includes a verifiable digest for its bundle") - so
/// this looks them up by suffix rather than a fixed index.
pub fn parse_github_latest_release(json: &str) -> Result<ReleaseAssets, ReleaseCheckError> {
    let release: RawRelease =
        serde_json::from_str(json).map_err(|error| ReleaseCheckError::InvalidJson(error.to_string()))?;

    let bundle_url = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(".tar.gz"))
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| ReleaseCheckError::MissingAsset("no .tar.gz bundle asset".to_string()))?;
    let digest_url = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(".tar.gz.sha256"))
        .map(|asset| asset.browser_download_url.clone())
        .ok_or_else(|| {
            ReleaseCheckError::MissingAsset("no .tar.gz.sha256 digest asset".to_string())
        })?;

    Ok(ReleaseAssets {
        version: release.tag_name,
        bundle_url,
        digest_url,
    })
}

/// Extracts the hex digest out of a `sha256sum`-style line (`<hex>  <filename>`
/// or just `<hex>`), matching how `ci.yml` writes the `.sha256` asset.
pub fn parse_sha256sum_digest(contents: &str) -> Result<String, ReleaseCheckError> {
    contents
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| ReleaseCheckError::InvalidJson("digest file is empty".to_string()))
}

/// What a poll of the Releases API should do next. No I/O happens inside
/// `evaluate_candidate` itself - the caller performs `DownloadRequested`'s
/// fetches and reports back via `stage_if_digest_matches`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollEffect {
    NoAction,
    DownloadRequested {
        version: String,
        bundle_url: String,
        digest_url: String,
    },
}

/// Decides what a discovered release means for `state`. Pure: `now` is
/// passed in rather than read from a clock.
pub fn evaluate_candidate(
    state: &UpdateAgentState,
    release: ReleaseAssets,
    now: OffsetDateTime,
) -> (UpdateAgentState, PollEffect) {
    let mut next = state.clone();
    next.last_checked_at = Some(now);

    if release.version == state.current_version {
        return (next, PollEffect::NoAction);
    }
    if state.status == UpdateAgentStatus::RolledBack
        && state.candidate_version.as_deref() == Some(release.version.as_str())
    {
        return (next, PollEffect::NoAction);
    }

    next.candidate_version = Some(release.version.clone());
    next.candidate_digest = None;
    next.status = UpdateAgentStatus::Idle;
    (
        next,
        PollEffect::DownloadRequested {
            version: release.version,
            bundle_url: release.bundle_url,
            digest_url: release.digest_url,
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestOutcome {
    Verified,
    Mismatch,
}

/// Compares the bundle's actually-downloaded digest against the one
/// published alongside it. `req-update-digest-func`: a mismatch is never
/// staged or activated - the candidate is dropped entirely rather than left
/// half-recorded, so a later poll re-stages it from scratch instead of
/// treating it as already seen.
pub fn stage_if_digest_matches(
    state: &UpdateAgentState,
    published_digest: &str,
    downloaded_digest: &str,
) -> (UpdateAgentState, DigestOutcome) {
    let mut next = state.clone();
    if published_digest == downloaded_digest {
        next.candidate_digest = Some(published_digest.to_string());
        next.status = UpdateAgentStatus::Staged;
        (next, DigestOutcome::Verified)
    } else {
        next.candidate_version = None;
        next.candidate_digest = None;
        next.status = UpdateAgentStatus::Idle;
        (next, DigestOutcome::Mismatch)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivationDecision {
    ActivateNow,
    DeferUntilWake,
    NotReady,
}

/// `req-update-activation-func`: a staged candidate activates immediately
/// while the schedule is awake, or defers until the next wake during
/// scheduled standby. `schedule_awake` is a single already-resolved
/// snapshot (the same `is_within_display_window` result `schedule.rs`'s
/// `reconcile` takes), not re-read here.
pub fn decide_activation(state: &UpdateAgentState, schedule_awake: bool) -> ActivationDecision {
    if state.status != UpdateAgentStatus::Staged {
        return ActivationDecision::NotReady;
    }
    if schedule_awake {
        ActivationDecision::ActivateNow
    } else {
        ActivationDecision::DeferUntilWake
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthCheckEffect {
    Activated { version: String },
    RolledBack { restore_version: String },
}

/// `req-update-rollback-func`: a failing health check rolls back to
/// `last_known_good_version` immediately. A passing check promotes the
/// candidate to both `current_version` and `last_known_good_version` -
/// `req-update-semantic`'s invariant holds throughout either path, since
/// `last_known_good_version` is only ever written on a pass, never cleared
/// on a failure.
pub fn apply_health_check_result(
    state: &UpdateAgentState,
    passed: bool,
) -> (UpdateAgentState, HealthCheckEffect) {
    let mut next = state.clone();
    if passed {
        let activated_version = state
            .candidate_version
            .clone()
            .unwrap_or_else(|| state.current_version.clone());
        next.current_version = activated_version.clone();
        next.last_known_good_version = activated_version.clone();
        next.candidate_version = None;
        next.candidate_digest = None;
        next.status = UpdateAgentStatus::Idle;
        (next, HealthCheckEffect::Activated { version: activated_version })
    } else {
        next.status = UpdateAgentStatus::RolledBack;
        (
            next,
            HealthCheckEffect::RolledBack {
                restore_version: state.last_known_good_version.clone(),
            },
        )
    }
}
