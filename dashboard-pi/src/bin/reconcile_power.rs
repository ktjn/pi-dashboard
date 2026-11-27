use dashboard_core::board_config::load_board_config;
use dashboard_core::schedule::{
    DisplayPowerControl, active_override_power, is_within_display_window, reconcile, resolved_power,
};
use dashboard_pi::power::{env_path, read_touch_wake_until, select_display_power_control};
use dashboard_pi::remote_control::read_override_state;
use std::path::Path;
use time::OffsetDateTime;

const DEFAULT_BOARD_CONFIG_PATH: &str = "board.toml";
const DEFAULT_OVERRIDE_FILE_PATH: &str = "override-state.json";
const DEFAULT_TOUCH_WAKE_FILE_PATH: &str = "touch-wake-until.txt";

fn reconcile_power_once(
    board_path: &Path,
    override_path: &Path,
    touch_wake_path: &Path,
    now_utc: OffsetDateTime,
    control: &mut dyn DisplayPowerControl,
) -> Result<bool, String> {
    let config = load_board_config(board_path).map_err(|error| error.to_string())?;
    let schedule_says_awake =
        is_within_display_window(&config.display_windows, &config.time_zone, now_utc)
            .map_err(|error| error.to_string())?;
    let override_state = read_override_state(override_path);
    let override_active = active_override_power(override_state, now_utc);
    let state = reconcile(schedule_says_awake, override_active, false);
    let touch_wake_active =
        read_touch_wake_until(touch_wake_path).is_some_and(|until| now_utc < until);
    let desired_power = resolved_power(state) || touch_wake_active;
    control
        .set_power(desired_power)
        .map_err(|error| error.to_string())?;
    Ok(desired_power)
}

fn main() {
    let board_path = env_path("BOARD_CONFIG_PATH", DEFAULT_BOARD_CONFIG_PATH);
    let override_path = env_path("OVERRIDE_FILE_PATH", DEFAULT_OVERRIDE_FILE_PATH);
    let touch_wake_path = env_path("TOUCH_WAKE_FILE_PATH", DEFAULT_TOUCH_WAKE_FILE_PATH);
    let mut control = select_display_power_control();
    let now_utc = OffsetDateTime::now_utc();

    if let Err(error) = reconcile_power_once(
        &board_path,
        &override_path,
        &touch_wake_path,
        now_utc,
        control.as_mut(),
    ) {
        eprintln!("reconcile_power failed: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_core::schedule::NullDisplayPowerControl;
    use dashboard_pi::power::write_touch_wake_until_atomic;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static UNIQUE_SUFFIX: AtomicU64 = AtomicU64::new(0);

    fn temp_path(label: &str, ext: &str) -> PathBuf {
        let suffix = UNIQUE_SUFFIX.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "pi-dashboard-reconcile-power-{label}-{suffix}.{ext}"
        ))
    }

    fn root_board_toml() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("board.toml")
    }

    fn utc_datetime(
        year: i32,
        month: time::Month,
        day: u8,
        hour: u8,
        minute: u8,
    ) -> OffsetDateTime {
        let date = time::Date::from_calendar_date(year, month, day).expect("valid date");
        let tod = time::Time::from_hms(hour, minute, 0).expect("valid time");
        time::PrimitiveDateTime::new(date, tod).assume_utc()
    }

    #[test]
    fn reconcile_power_turns_on_during_awake_window() {
        let board_path = root_board_toml();
        let override_path = temp_path("awake-no-override", "json");
        let touch_wake_path = temp_path("awake-no-touch-wake", "txt");
        let now_utc = utc_datetime(2026, time::Month::November, 25, 6, 30);
        let mut control = NullDisplayPowerControl::new();

        let power = reconcile_power_once(
            &board_path,
            &override_path,
            &touch_wake_path,
            now_utc,
            &mut control,
        )
        .expect("reconciliation must succeed");

        assert!(power);
        assert_eq!(control.calls(), &[true]);
    }

    #[test]
    fn reconcile_power_turns_off_during_standby_window() {
        let board_path = root_board_toml();
        let override_path = temp_path("standby-no-override", "json");
        let touch_wake_path = temp_path("standby-no-touch-wake", "txt");
        let now_utc = utc_datetime(2026, time::Month::November, 25, 11, 0);
        let mut control = NullDisplayPowerControl::new();

        let power = reconcile_power_once(
            &board_path,
            &override_path,
            &touch_wake_path,
            now_utc,
            &mut control,
        )
        .expect("reconciliation must succeed");

        assert!(!power);
        assert_eq!(control.calls(), &[false]);
    }

    #[test]
    fn reconcile_power_honors_active_override_off_during_awake_window() {
        let board_path = root_board_toml();
        let override_path = temp_path("awake-override-off", "json");
        let touch_wake_path = temp_path("awake-override-off-wake", "txt");
        std::fs::write(
            &override_path,
            r#"{"power":"off","override_until":"2026-11-25T08:00:00Z"}"#,
        )
        .expect("override fixture must write");
        let now_utc = utc_datetime(2026, time::Month::November, 25, 6, 30);
        let mut control = NullDisplayPowerControl::new();

        let power = reconcile_power_once(
            &board_path,
            &override_path,
            &touch_wake_path,
            now_utc,
            &mut control,
        )
        .expect("reconciliation must succeed");

        std::fs::remove_file(&override_path).ok();
        assert!(!power);
        assert_eq!(control.calls(), &[false]);
    }

    #[test]
    fn reconcile_power_falls_back_to_schedule_when_override_is_expired() {
        let board_path = root_board_toml();
        let override_path = temp_path("awake-override-expired", "json");
        let touch_wake_path = temp_path("awake-override-expired-wake", "txt");
        std::fs::write(
            &override_path,
            r#"{"power":"off","override_until":"2026-11-25T06:00:00Z"}"#,
        )
        .expect("override fixture must write");
        let now_utc = utc_datetime(2026, time::Month::November, 25, 6, 30);
        let mut control = NullDisplayPowerControl::new();

        let power = reconcile_power_once(
            &board_path,
            &override_path,
            &touch_wake_path,
            now_utc,
            &mut control,
        )
        .expect("reconciliation must succeed");

        std::fs::remove_file(&override_path).ok();
        assert!(power);
        assert_eq!(control.calls(), &[true]);
    }

    #[test]
    fn reconcile_power_honors_active_override_on_during_standby_window() {
        let board_path = root_board_toml();
        let override_path = temp_path("standby-override-on", "json");
        let touch_wake_path = temp_path("standby-override-on-wake", "txt");
        std::fs::write(
            &override_path,
            r#"{"power":"on","override_until":"2026-11-25T15:00:00Z"}"#,
        )
        .expect("override fixture must write");
        let now_utc = utc_datetime(2026, time::Month::November, 25, 11, 0);
        let mut control = NullDisplayPowerControl::new();

        let power = reconcile_power_once(
            &board_path,
            &override_path,
            &touch_wake_path,
            now_utc,
            &mut control,
        )
        .expect("reconciliation must succeed");

        std::fs::remove_file(&override_path).ok();
        assert!(power);
        assert_eq!(control.calls(), &[true]);
    }

    #[test]
    fn reconcile_power_falls_back_to_schedule_on_malformed_override_json() {
        let board_path = root_board_toml();
        let override_path = temp_path("malformed-override", "json");
        let touch_wake_path = temp_path("malformed-override-wake", "txt");
        std::fs::write(&override_path, "{not valid json").expect("override fixture must write");
        let now_utc = utc_datetime(2026, time::Month::November, 25, 6, 30);
        let mut control = NullDisplayPowerControl::new();

        let power = reconcile_power_once(
            &board_path,
            &override_path,
            &touch_wake_path,
            now_utc,
            &mut control,
        )
        .expect("reconciliation must succeed");

        std::fs::remove_file(&override_path).ok();
        assert!(power);
        assert_eq!(control.calls(), &[true]);
    }

    #[test]
    fn reconcile_power_honors_active_touch_wake_during_standby_and_reverts_on_expiry() {
        let board_path = root_board_toml();
        let override_path = temp_path("standby-touch-wake-override", "json");
        let touch_wake_path = temp_path("standby-touch-wake", "txt");
        let now_utc = utc_datetime(2026, time::Month::November, 25, 11, 0);
        let wake_until = now_utc + time::Duration::seconds(120);
        write_touch_wake_until_atomic(&touch_wake_path, wake_until)
            .expect("touch wake fixture must write");

        let mut control = NullDisplayPowerControl::new();
        let awake_power = reconcile_power_once(
            &board_path,
            &override_path,
            &touch_wake_path,
            now_utc,
            &mut control,
        )
        .expect("reconciliation must succeed");
        let expired_power = reconcile_power_once(
            &board_path,
            &override_path,
            &touch_wake_path,
            wake_until,
            &mut control,
        )
        .expect("reconciliation must succeed");

        std::fs::remove_file(&touch_wake_path).ok();
        assert!(awake_power);
        assert!(!expired_power);
        assert_eq!(control.calls(), &[true, false]);
    }

    #[test]
    fn reconcile_power_honors_active_touch_wake_during_manual_override_off_and_reverts_on_expiry() {
        let board_path = root_board_toml();
        let override_path = temp_path("override-off-touch-wake-override", "json");
        let touch_wake_path = temp_path("override-off-touch-wake", "txt");
        std::fs::write(
            &override_path,
            r#"{"power":"off","override_until":"2026-11-25T08:00:00Z"}"#,
        )
        .expect("override fixture must write");
        let now_utc = utc_datetime(2026, time::Month::November, 25, 6, 30);
        let wake_until = now_utc + time::Duration::seconds(120);
        write_touch_wake_until_atomic(&touch_wake_path, wake_until)
            .expect("touch wake fixture must write");

        let mut control = NullDisplayPowerControl::new();
        let awake_power = reconcile_power_once(
            &board_path,
            &override_path,
            &touch_wake_path,
            now_utc,
            &mut control,
        )
        .expect("reconciliation must succeed");
        let expired_power = reconcile_power_once(
            &board_path,
            &override_path,
            &touch_wake_path,
            wake_until,
            &mut control,
        )
        .expect("reconciliation must succeed");

        std::fs::remove_file(&override_path).ok();
        std::fs::remove_file(&touch_wake_path).ok();
        assert!(awake_power);
        assert!(!expired_power);
        assert_eq!(control.calls(), &[true, false]);
    }
}
