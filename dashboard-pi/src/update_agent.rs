use crate::atomic_fs;
use sha2::{Digest, Sha256};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MAX_BUNDLE_BYTES: u64 = 64 * 1024 * 1024;

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Debug)]
pub enum UpdateAgentIoError {
    Io(String),
    OversizedBundle { max_bytes: u64 },
    InvalidVersion(String),
}

impl fmt::Display for UpdateAgentIoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(f, "update agent I/O failed: {message}"),
            Self::OversizedBundle { max_bytes } => {
                write!(
                    f,
                    "release bundle exceeds maximum size of {max_bytes} bytes"
                )
            }
            Self::InvalidVersion(version) => {
                write!(f, "release version '{version}' is not a valid filename")
            }
        }
    }
}

impl std::error::Error for UpdateAgentIoError {}

pub fn read_bounded_bytes<R: Read>(
    reader: R,
    max_bytes: u64,
) -> Result<Vec<u8>, UpdateAgentIoError> {
    let mut bytes = Vec::new();
    let limit = max_bytes.saturating_add(1);
    reader
        .take(limit)
        .read_to_end(&mut bytes)
        .map_err(|error| UpdateAgentIoError::Io(error.to_string()))?;
    if bytes.len() as u64 > max_bytes {
        return Err(UpdateAgentIoError::OversizedBundle { max_bytes });
    }
    Ok(bytes)
}

fn is_valid_version_segment(version: &str) -> bool {
    !version.is_empty()
        && version != "."
        && !version.contains("..")
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

pub fn stage_bundle_atomic(
    staging_dir: &Path,
    version: &str,
    bundle_bytes: &[u8],
) -> Result<PathBuf, UpdateAgentIoError> {
    if !is_valid_version_segment(version) {
        return Err(UpdateAgentIoError::InvalidVersion(version.to_string()));
    }
    let bundle_path = staging_dir.join(format!("{version}.tar.gz"));
    atomic_fs::write_bytes_atomic(&bundle_path, bundle_bytes)
        .map_err(|error| UpdateAgentIoError::Io(error.to_string()))?;
    Ok(bundle_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64 as TestCounter, Ordering as TestOrdering};

    static UNIQUE_SUFFIX: TestCounter = TestCounter::new(0);

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
    fn read_bounded_bytes_accepts_input_up_to_limit() {
        let data = b"exact-size";
        let result = read_bounded_bytes(&data[..], data.len() as u64).expect("within limit");
        assert_eq!(result, data);
    }

    #[test]
    fn read_bounded_bytes_rejects_input_exceeding_limit() {
        let data = b"too-many-bytes";
        let err = read_bounded_bytes(&data[..], 4).expect_err("must reject oversized input");
        assert!(matches!(
            err,
            UpdateAgentIoError::OversizedBundle { max_bytes: 4 }
        ));
    }

    #[test]
    fn stage_bundle_atomic_writes_verified_bundle_to_staging_dir() {
        let suffix = UNIQUE_SUFFIX.fetch_add(1, TestOrdering::Relaxed);
        let staging_dir = std::env::temp_dir().join(format!(
            "pi-dashboard-staging-test-{}-{suffix}",
            std::process::id()
        ));
        let payload = b"fake-tar-gz-bytes";

        let staged_path =
            stage_bundle_atomic(&staging_dir, "v1.2.3", payload).expect("staging must succeed");

        assert_eq!(staged_path, staging_dir.join("v1.2.3.tar.gz"));
        assert_eq!(
            std::fs::read(&staged_path).expect("staged bundle must be readable"),
            payload
        );
        let _ = std::fs::remove_dir_all(&staging_dir);
    }

    #[test]
    fn stage_bundle_atomic_rejects_path_traversal_and_invalid_versions() {
        let staging_dir = std::env::temp_dir().join("pi-dashboard-staging-invalid-test");
        for invalid in [
            "",
            ".",
            "..",
            "../escape",
            "a/b",
            "a\\b",
            "v1..2",
            " v1.0.0 ",
            "C:evil",
            "v1:0",
        ] {
            let err = stage_bundle_atomic(&staging_dir, invalid, b"bytes")
                .expect_err("invalid version must be rejected");
            assert!(
                matches!(err, UpdateAgentIoError::InvalidVersion(_)),
                "expected InvalidVersion for {invalid:?}, got {err:?}"
            );
        }
    }
}
