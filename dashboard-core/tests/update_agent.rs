use dashboard_core::update_agent::{
    ActivationDecision, DigestOutcome, HealthCheckEffect, PollEffect, ReleaseAssets,
    UpdateAgentState, UpdateAgentStatus, apply_health_check_result, decide_activation,
    evaluate_candidate, parse_github_latest_release, parse_sha256sum_digest,
};
use time::macros::datetime;

fn release(version: &str) -> ReleaseAssets {
    ReleaseAssets {
        version: version.to_string(),
        bundle_url: format!("https://example.invalid/{version}/bundle.tar.gz"),
        digest_url: format!("https://example.invalid/{version}/bundle.tar.gz.sha256"),
    }
}

#[test]
fn parses_the_github_latest_release_response_shape() {
    let json = r#"{
        "tag_name": "v1.2.3",
        "assets": [
            { "name": "pi-dashboard-v1.2.3-aarch64-unknown-linux-gnu.tar.gz", "browser_download_url": "https://example.invalid/bundle.tar.gz" },
            { "name": "pi-dashboard-v1.2.3-aarch64-unknown-linux-gnu.tar.gz.sha256", "browser_download_url": "https://example.invalid/bundle.tar.gz.sha256" }
        ]
    }"#;
    let release = parse_github_latest_release(json).expect("must parse");
    assert_eq!(release.version, "v1.2.3");
    assert_eq!(release.bundle_url, "https://example.invalid/bundle.tar.gz");
    assert_eq!(release.digest_url, "https://example.invalid/bundle.tar.gz.sha256");
}

#[test]
fn rejects_a_release_response_missing_the_digest_asset() {
    let json = r#"{
        "tag_name": "v1.2.3",
        "assets": [
            { "name": "pi-dashboard-v1.2.3-aarch64-unknown-linux-gnu.tar.gz", "browser_download_url": "https://example.invalid/bundle.tar.gz" }
        ]
    }"#;
    assert!(parse_github_latest_release(json).is_err());
}

#[test]
fn parses_the_hex_digest_out_of_a_sha256sum_line() {
    let line = "abc123\n";
    let digest = parse_sha256sum_digest(line).expect("must parse");
    assert_eq!(digest, "abc123");
}

#[test]
fn a_candidate_matching_the_currently_running_version_triggers_no_action() {
    let state = UpdateAgentState::starting_at("v1.0.0");
    let now = datetime!(2026-11-27 10:00:00 UTC);
    let (next, effect) = evaluate_candidate(&state, release("v1.0.0"), now);
    assert_eq!(effect, PollEffect::NoAction);
    assert_eq!(next.last_checked_at, Some(now));
    assert_eq!(next.candidate_version, None);
}

#[test]
fn a_newer_candidate_is_staged_for_download() {
    let state = UpdateAgentState::starting_at("v1.0.0");
    let now = datetime!(2026-11-27 10:00:00 UTC);
    let (next, effect) = evaluate_candidate(&state, release("v1.1.0"), now);
    assert_eq!(next.candidate_version, Some("v1.1.0".to_string()));
    assert_eq!(
        effect,
        PollEffect::DownloadRequested {
            version: "v1.1.0".to_string(),
            bundle_url: "https://example.invalid/v1.1.0/bundle.tar.gz".to_string(),
            digest_url: "https://example.invalid/v1.1.0/bundle.tar.gz.sha256".to_string(),
        }
    );
}

#[test]
fn a_version_that_already_failed_its_health_check_is_not_retried() {
    let state = UpdateAgentState {
        candidate_version: Some("v1.1.0".to_string()),
        status: UpdateAgentStatus::RolledBack,
        ..UpdateAgentState::starting_at("v1.0.0")
    };
    let now = datetime!(2026-11-27 10:00:00 UTC);
    let (next, effect) = evaluate_candidate(&state, release("v1.1.0"), now);
    assert_eq!(effect, PollEffect::NoAction);
    assert_eq!(next.candidate_version, Some("v1.1.0".to_string()));
}

#[test]
fn a_rolled_back_version_is_restaged_once_a_different_candidate_appears() {
    let state = UpdateAgentState {
        candidate_version: Some("v1.1.0".to_string()),
        status: UpdateAgentStatus::RolledBack,
        ..UpdateAgentState::starting_at("v1.0.0")
    };
    let now = datetime!(2026-11-27 10:00:00 UTC);
    let (next, effect) = evaluate_candidate(&state, release("v1.2.0"), now);
    assert_eq!(next.candidate_version, Some("v1.2.0".to_string()));
    assert!(matches!(effect, PollEffect::DownloadRequested { .. }));
}

#[test]
fn a_bundle_whose_digest_matches_the_published_digest_is_staged() {
    let state = UpdateAgentState {
        candidate_version: Some("v1.1.0".to_string()),
        ..UpdateAgentState::starting_at("v1.0.0")
    };
    let (next, outcome) = dashboard_core::update_agent::stage_if_digest_matches(
        &state,
        "deadbeef",
        "deadbeef",
    );
    assert_eq!(outcome, DigestOutcome::Verified);
    assert_eq!(next.status, UpdateAgentStatus::Staged);
    assert_eq!(next.candidate_digest, Some("deadbeef".to_string()));
}

#[test]
fn a_bundle_whose_digest_does_not_match_is_never_staged() {
    let state = UpdateAgentState {
        candidate_version: Some("v1.1.0".to_string()),
        ..UpdateAgentState::starting_at("v1.0.0")
    };
    let (next, outcome) = dashboard_core::update_agent::stage_if_digest_matches(
        &state,
        "deadbeef",
        "not-a-match",
    );
    assert_eq!(outcome, DigestOutcome::Mismatch);
    assert_eq!(next.status, UpdateAgentStatus::Idle);
    assert_eq!(next.candidate_version, None);
    assert_eq!(next.candidate_digest, None);
}

#[test]
fn a_staged_candidate_activates_immediately_while_the_schedule_is_awake() {
    let state = UpdateAgentState {
        status: UpdateAgentStatus::Staged,
        candidate_version: Some("v1.1.0".to_string()),
        ..UpdateAgentState::starting_at("v1.0.0")
    };
    assert_eq!(decide_activation(&state, true), ActivationDecision::ActivateNow);
}

#[test]
fn a_staged_candidate_defers_activation_during_scheduled_standby() {
    let state = UpdateAgentState {
        status: UpdateAgentStatus::Staged,
        candidate_version: Some("v1.1.0".to_string()),
        ..UpdateAgentState::starting_at("v1.0.0")
    };
    assert_eq!(
        decide_activation(&state, false),
        ActivationDecision::DeferUntilWake
    );
}

#[test]
fn activation_is_not_ready_without_a_staged_candidate() {
    let state = UpdateAgentState::starting_at("v1.0.0");
    assert_eq!(decide_activation(&state, true), ActivationDecision::NotReady);
}

#[test]
fn a_passing_health_check_promotes_the_candidate_to_current_and_last_known_good() {
    let state = UpdateAgentState {
        status: UpdateAgentStatus::Staged,
        candidate_version: Some("v1.1.0".to_string()),
        candidate_digest: Some("deadbeef".to_string()),
        ..UpdateAgentState::starting_at("v1.0.0")
    };
    let (next, effect) = apply_health_check_result(&state, true);
    assert_eq!(next.current_version, "v1.1.0");
    assert_eq!(next.last_known_good_version, "v1.1.0");
    assert_eq!(next.candidate_version, None);
    assert_eq!(next.candidate_digest, None);
    assert_eq!(next.status, UpdateAgentStatus::Idle);
    assert_eq!(
        effect,
        HealthCheckEffect::Activated {
            version: "v1.1.0".to_string()
        }
    );
}

#[test]
fn a_failing_health_check_rolls_back_to_the_last_known_good_version() {
    let state = UpdateAgentState {
        status: UpdateAgentStatus::Staged,
        candidate_version: Some("v1.1.0".to_string()),
        candidate_digest: Some("deadbeef".to_string()),
        ..UpdateAgentState::starting_at("v1.0.0")
    };
    let (next, effect) = apply_health_check_result(&state, false);
    assert_eq!(next.current_version, "v1.0.0");
    assert_eq!(next.last_known_good_version, "v1.0.0");
    assert_eq!(next.status, UpdateAgentStatus::RolledBack);
    assert_eq!(
        effect,
        HealthCheckEffect::RolledBack {
            restore_version: "v1.0.0".to_string()
        }
    );
}
