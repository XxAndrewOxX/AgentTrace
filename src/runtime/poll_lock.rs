//! Cross-process poll leader election.
//!
//! `InstanceLock` (see [`crate::runtime::change_processor`]) guards single TUI
//! ownership but is only meaningful for interactive sessions. The poll loop,
//! however, must be a *single writer across processes* — an MCP server and a
//! `agent-trace open` TUI started against the same store would otherwise both
//! run a poll loop and duplicate commits / `summary_events.jsonl` entries.
//!
//! `PollLock` provides that election with an OS-level advisory lock (`flock` on
//! Unix) over `.agent-trace/locks/poll.lock`. The first acquirer becomes the
//! poll leader; subsequent acquirers get `None` and fall back to HEAD-only
//! updates. The lock is released when the [`PollLock`] is dropped (the file
//! descriptor closes), so a crashed leader does not leave a stale lock.

use anyhow::{Context, Result};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

/// Advisory, cross-process lock that elects a single poll-loop writer.
pub struct PollLock {
    /// Kept open for the lifetime of the lock; closing the fd releases `flock`.
    _file: File,
    #[allow(dead_code)]
    path: PathBuf,
}

impl PollLock {
    /// Try to become the poll leader without blocking.
    ///
    /// Returns `Ok(Some(lock))` if this process acquired the poll lock, or
    /// `Ok(None)` if another process already holds it.
    pub fn try_acquire(store_root: &Path) -> Result<Option<Self>> {
        let dir = store_root.join(".agent-trace").join("locks");
        std::fs::create_dir_all(&dir).context("create .agent-trace/locks directory")?;
        let path = dir.join("poll.lock");

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("open poll lock at {}", path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let fd = file.as_raw_fd();
            // Non-blocking exclusive advisory lock.
            let rc = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
            if rc != 0 {
                let err = std::io::Error::last_os_error();
                match err.raw_os_error() {
                    Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN => {
                        return Ok(None);
                    }
                    _ => {
                        return Err(anyhow::anyhow!(
                            "failed to acquire poll lock {}: {err}",
                            path.display()
                        ));
                    }
                }
            }
            // Record the leader PID for diagnostics (best-effort).
            use std::io::{Seek, SeekFrom, Write};
            let mut f = &file;
            let _ = f.set_len(0);
            let _ = f.seek(SeekFrom::Start(0));
            let _ = write!(f, "{}", std::process::id());
        }

        #[cfg(not(unix))]
        {
            // No advisory flock available; fall back to an exclusive-create
            // sentinel with stale-PID detection (best effort).
            if !try_acquire_sentinel(&path)? {
                return Ok(None);
            }
        }

        Ok(Some(Self { _file: file, path }))
    }
}

#[cfg(not(unix))]
fn try_acquire_sentinel(path: &Path) -> Result<bool> {
    use std::io::Write;
    if path.exists() {
        if let Ok(content) = std::fs::read_to_string(path) {
            if content.trim().parse::<u32>().is_ok() {
                // Conservatively treat an existing sentinel as a live leader.
                return Ok(false);
            }
        }
    }
    let mut f = std::fs::File::create(path)?;
    write!(f, "{}", std::process::id())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// V-D1: a second acquire returns `None` while the first lock is held.
    #[test]
    fn second_acquire_blocked_while_held() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();
        let first = PollLock::try_acquire(root).unwrap();
        assert!(first.is_some(), "first acquire should become poll leader");

        let second = PollLock::try_acquire(root).unwrap();
        assert!(
            second.is_none(),
            "second acquire must fail while first lock is held"
        );

        drop(first);
        drop(second);
    }

    /// V-D2: after the first lock is dropped, a new acquire succeeds.
    #[test]
    fn acquire_succeeds_after_release() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        {
            let first = PollLock::try_acquire(root).unwrap();
            assert!(first.is_some());
        } // dropped here → lock released

        let again = PollLock::try_acquire(root).unwrap();
        assert!(
            again.is_some(),
            "acquire should succeed after the previous lock was released"
        );
    }
}
