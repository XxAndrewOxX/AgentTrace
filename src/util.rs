use std::path::Path;
use anyhow::Result;

/// Write `content` to `path` atomically using a temp-file + rename pattern.
/// Prevents file corruption if the process is interrupted mid-write.
pub fn atomic_write(path: &Path, content: &str) -> Result<()> {
    let tmp_path = path.with_extension(
        format!("{}.tmp", path.extension().and_then(|e| e.to_str()).unwrap_or(""))
    );
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_atomic_write_creates_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.toml");
        atomic_write(&path, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn test_atomic_write_no_tmp_left_behind() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.toml");
        atomic_write(&path, "content").unwrap();
        let tmp_path = path.with_extension("toml.tmp");
        assert!(!tmp_path.exists(), "tmp file should not remain after atomic write");
    }

    #[test]
    fn test_atomic_write_overwrites_existing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("out.toml");
        atomic_write(&path, "first").unwrap();
        atomic_write(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
    }
}
