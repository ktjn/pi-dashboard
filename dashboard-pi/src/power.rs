use crate::atomic_fs;
use dashboard_core::schedule::{
    DisplayPowerControl, DisplayPowerError, LoggingDisplayPowerControl,
};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

const DEFAULT_DISPLAY_OUTPUT: &str = "DSI-1";

pub fn env_path(var_name: &str, default_path: &str) -> PathBuf {
    std::env::var(var_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default_path))
}

pub fn write_touch_wake_until_atomic(path: &Path, until: OffsetDateTime) -> io::Result<()> {
    let formatted = until.format(&Rfc3339).map_err(io::Error::other)?;
    atomic_fs::write_bytes_atomic(path, formatted.as_bytes())
}

pub fn clear_touch_wake_file(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn read_touch_wake_until(path: &Path) -> Option<OffsetDateTime> {
    let contents = std::fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return None;
    }
    OffsetDateTime::parse(trimmed, &Rfc3339).ok()
}

pub struct WlrRandrDisplayPowerControl {
    output: String,
}

impl WlrRandrDisplayPowerControl {
    pub fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
        }
    }
}

pub fn wlr_randr_args(output: &str, on: bool) -> [String; 3] {
    [
        "--output".to_string(),
        output.to_string(),
        if on { "--on" } else { "--off" }.to_string(),
    ]
}

impl DisplayPowerControl for WlrRandrDisplayPowerControl {
    fn set_power(&mut self, on: bool) -> Result<(), DisplayPowerError> {
        let status = Command::new("wlr-randr")
            .args(wlr_randr_args(&self.output, on))
            .status()
            .map_err(|error| DisplayPowerError::BacklightControlUnavailable(error.to_string()))?;
        if !status.success() {
            return Err(DisplayPowerError::BacklightControlUnavailable(format!(
                "wlr-randr exited with {status}"
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PowerBackendChoice {
    WlrRandr { output: String },
    Logging,
}

pub fn resolve_power_backend_choice(
    backend_env: Option<&str>,
    output_env: Option<&str>,
    is_linux: bool,
) -> PowerBackendChoice {
    let default_backend = if is_linux { "wlr-randr" } else { "null" };
    let backend = backend_env
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default_backend);
    let output = output_env
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_DISPLAY_OUTPUT)
        .to_string();

    match backend {
        "wlr-randr" => PowerBackendChoice::WlrRandr { output },
        "null" => PowerBackendChoice::Logging,
        other => {
            eprintln!("unknown DISPLAY_POWER_BACKEND '{other}'; falling back to logging backend");
            PowerBackendChoice::Logging
        }
    }
}

pub fn select_display_power_control() -> Box<dyn DisplayPowerControl + Send> {
    let backend_env = std::env::var("DISPLAY_POWER_BACKEND").ok();
    let output_env = std::env::var("DISPLAY_OUTPUT").ok();
    match resolve_power_backend_choice(
        backend_env.as_deref(),
        output_env.as_deref(),
        cfg!(target_os = "linux"),
    ) {
        PowerBackendChoice::WlrRandr { output } => {
            Box::new(WlrRandrDisplayPowerControl::new(output))
        }
        PowerBackendChoice::Logging => Box::new(LoggingDisplayPowerControl::new()),
    }
}

#[derive(Clone, Default)]
pub struct WakeSignal {
    inner: Arc<(Mutex<u64>, Condvar)>,
}

impl WakeSignal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn notify(&self) {
        let (lock, cvar) = &*self.inner;
        let mut generation = lock.lock().expect("wake signal mutex poisoned");
        *generation = generation.wrapping_add(1);
        cvar.notify_all();
    }

    pub fn wait_timeout(&self, duration: Duration) -> bool {
        let (lock, cvar) = &*self.inner;
        let guard = lock.lock().expect("wake signal mutex poisoned");
        let start_generation = *guard;
        let (guard, _) = cvar
            .wait_timeout_while(guard, duration, |generation| {
                *generation == start_generation
            })
            .expect("wake signal mutex poisoned");
        *guard != start_generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wlr_randr_args_builds_on_and_off_commands() {
        assert_eq!(wlr_randr_args("DSI-1", true), ["--output", "DSI-1", "--on"]);
        assert_eq!(
            wlr_randr_args("HDMI-A-1", false),
            ["--output", "HDMI-A-1", "--off"]
        );
    }

    #[test]
    fn resolve_power_backend_choice_defaults_by_platform_and_respects_overrides() {
        assert_eq!(
            resolve_power_backend_choice(None, None, true),
            PowerBackendChoice::WlrRandr {
                output: "DSI-1".to_string()
            }
        );
        assert_eq!(
            resolve_power_backend_choice(None, None, false),
            PowerBackendChoice::Logging
        );
        assert_eq!(
            resolve_power_backend_choice(Some("wlr-randr"), Some("HDMI-A-1"), false),
            PowerBackendChoice::WlrRandr {
                output: "HDMI-A-1".to_string()
            }
        );
        assert_eq!(
            resolve_power_backend_choice(Some("null"), Some("DSI-1"), true),
            PowerBackendChoice::Logging
        );
    }

    #[test]
    fn wake_signal_returns_true_when_notified_before_timeout() {
        let signal = WakeSignal::new();
        let notifier = signal.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            notifier.notify();
        });

        let woken = signal.wait_timeout(Duration::from_secs(2));
        handle.join().expect("notifier thread must not panic");
        assert!(woken);
    }

    #[test]
    fn wake_signal_returns_false_when_timeout_elapses() {
        let signal = WakeSignal::new();
        assert!(!signal.wait_timeout(Duration::from_millis(20)));
    }

    #[test]
    fn touch_wake_file_round_trips_clears_and_ignores_missing_or_malformed() {
        let path = std::env::temp_dir().join(format!(
            "pi-dashboard-touch-wake-test-{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);

        assert_eq!(read_touch_wake_until(&path), None);
        clear_touch_wake_file(&path).expect("clearing missing file must succeed");

        let until = time::OffsetDateTime::from_unix_timestamp(1_795_600_800).expect("valid ts");
        write_touch_wake_until_atomic(&path, until).expect("atomic write must succeed");
        assert_eq!(read_touch_wake_until(&path), Some(until));

        clear_touch_wake_file(&path).expect("clearing existing file must succeed");
        assert_eq!(read_touch_wake_until(&path), None);

        std::fs::write(&path, "not-a-timestamp").expect("corrupt fixture must write");
        assert_eq!(read_touch_wake_until(&path), None);
        let _ = std::fs::remove_file(&path);
    }
}
