use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TMP_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn tmp_path_for(path: &Path) -> PathBuf {
    let pid = std::process::id();
    let unique = TMP_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(".{pid}.{unique}.tmp"));
    PathBuf::from(tmp)
}

pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let tmp_path = tmp_path_for(path);
    std::fs::write(&tmp_path, bytes)?;
    if let Err(error) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn unique_test_dir(label: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "pi-dashboard-atomic-fs-test-{label}-{}-{id}",
            std::process::id()
        ))
    }

    #[test]
    fn tmp_path_for_produces_distinct_paths_on_each_call() {
        let base = PathBuf::from("example.json");
        let first = tmp_path_for(&base);
        let second = tmp_path_for(&base);
        assert_ne!(first, second);
        assert!(first.to_string_lossy().ends_with(".tmp"));
        assert!(second.to_string_lossy().ends_with(".tmp"));
    }

    #[test]
    fn write_bytes_atomic_creates_parent_directories_and_writes_contents() {
        let root = unique_test_dir("nested");
        let target = root.join("nested").join("dir").join("file.json");

        write_bytes_atomic(&target, b"hello world").expect("atomic write must succeed");

        assert_eq!(
            std::fs::read(&target).expect("written file must be readable"),
            b"hello world"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn write_bytes_atomic_cleans_up_temp_file_when_rename_fails() {
        let root = unique_test_dir("rename-fail");
        let target_dir = root.join("existing-dir");
        std::fs::create_dir_all(&target_dir).expect("setup dir must be created");

        let result = write_bytes_atomic(&target_dir, b"payload");
        assert!(result.is_err());

        let leftovers: Vec<PathBuf> = std::fs::read_dir(&root)
            .expect("root dir must be readable")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(".tmp"))
            })
            .collect();
        assert!(leftovers.is_empty());

        let _ = std::fs::remove_dir_all(&root);
    }
}
