use sha2::{Digest, Sha256};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Hex-encoded SHA-256 of `bytes`, in the same lowercase-hex form
/// `sha256sum` writes - what `update_agent::stage_if_digest_matches`
/// compares the downloaded bundle's digest against.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug)]
pub enum ActiveVersionFileError {
    Io(String),
}

impl fmt::Display for ActiveVersionFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(f, "active-version file I/O failed: {message}"),
        }
    }
}

impl std::error::Error for ActiveVersionFileError {}

static TMP_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn tmp_path_for(path: &Path) -> PathBuf {
    let pid = std::process::id();
    let unique = TMP_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{pid}.{unique}.tmp"));
    PathBuf::from(tmp)
}

/// Records which version is active by atomically rewriting a pointer file
/// (write-to-temp, then `rename` over the real path) - the same
/// write-then-rename pattern `remote_control.rs`'s override file uses, so a
/// concurrent reader never observes a half-written version string. This is
/// the "atomic swap" `req-update-activation-func` calls for; which concrete
/// mechanism (pointer file vs. a real symlink) is an implementation detail,
/// not a contract requirement.
pub fn write_active_version_atomic(path: &Path, version: &str) -> Result<(), ActiveVersionFileError> {
    let tmp_path = tmp_path_for(path);
    std::fs::write(&tmp_path, version).map_err(|error| ActiveVersionFileError::Io(error.to_string()))?;
    std::fs::rename(&tmp_path, path).map_err(|error| ActiveVersionFileError::Io(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64 as TestCounter, Ordering as TestOrdering};
    use std::thread;

    static UNIQUE_SUFFIX: TestCounter = TestCounter::new(0);

    fn unique_path(label: &str) -> PathBuf {
        let suffix = UNIQUE_SUFFIX.fetch_add(1, TestOrdering::Relaxed);
        std::env::temp_dir().join(format!("pi-dashboard-update-agent-test-{label}-{suffix}.txt"))
    }

    #[test]
    fn sha256_hex_matches_a_known_test_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_hex_of_empty_input_matches_the_well_known_empty_digest() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn write_then_read_round_trips_the_active_version() {
        let path = unique_path("roundtrip");
        write_active_version_atomic(&path, "v1.2.3").expect("write must succeed");
        assert_eq!(
            std::fs::read_to_string(&path).expect("file must be readable"),
            "v1.2.3"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_active_version_atomic_reports_an_io_error_for_an_unwritable_path() {
        let path = PathBuf::from("/this/directory/does/not/exist/active-version.txt");
        assert!(write_active_version_atomic(&path, "v1.2.3").is_err());
    }

    #[test]
    fn write_active_version_atomic_leaves_no_temp_file_behind_under_concurrent_writers() {
        let path = unique_path("concurrent");
        let handles: Vec<_> = (0..8)
            .map(|index| {
                let path = path.clone();
                thread::spawn(move || {
                    write_active_version_atomic(&path, &format!("v1.0.{index}"))
                        .expect("write must succeed")
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("writer thread must not panic");
        }

        assert!(std::fs::read_to_string(&path).is_ok());
        let parent = path.parent().expect("path must have a parent");
        let leftover_tmp_files: Vec<_> = std::fs::read_dir(parent)
            .expect("temp dir must be readable")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&*path.file_name().unwrap().to_string_lossy())
                    && entry.file_name().to_string_lossy().ends_with(".tmp")
            })
            .collect();
        assert!(leftover_tmp_files.is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
