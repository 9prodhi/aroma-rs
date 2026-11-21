//! `lock`: exclusive writer lock on `.aroma/lock` (D10). Owned and implemented by task T10
//! (`WriterLock`).
//!
//! One writer at a time builds a generation; readers never lock. The OS advisory lock
//! (`flock` on Unix, `LockFileEx` on byte 0 on Windows, both via `fd-lock`) dies with
//! the process, so a crashed writer never leaves a stale lock behind.
//!
//! The lock file holds `#<pid>\n`. The pid starts at byte 1 because Windows forbids
//! other handles from reading the locked byte 0; a waiter reads from byte 1 to name
//! the holder in `StoreError::Locked`.

use crate::store::StoreError;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const LOCK_FILE: &str = "lock";
/// How often `acquire` retries while another writer holds the lock.
pub const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Proof that this process is the only writer of an `.aroma` directory. Dropping it
/// closes the lock file, which releases the OS lock.
#[derive(Debug)]
pub struct WriterLock {
    dir: PathBuf,
    // The write guard of `fd_lock::RwLock` borrows the lock, so it cannot live in the
    // same struct. `acquire` forgets the guard instead: the lock then stays held for as
    // long as this file handle is open and is released when `WriterLock` drops it.
    _file: fd_lock::RwLock<File>,
}

impl WriterLock {
    /// Create `aroma_dir` if needed and take the exclusive lock on `aroma_dir/lock`,
    /// retrying every `POLL_INTERVAL` until `timeout` has elapsed (one attempt when
    /// `timeout` is zero). On timeout returns `StoreError::Locked { pid }` with the pid
    /// recorded by the holder (0 when unreadable). On success records our own pid.
    pub fn acquire(aroma_dir: &Path, timeout: Duration) -> Result<WriterLock, StoreError> {
        std::fs::create_dir_all(aroma_dir)?;
        let path = aroma_dir.join(LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        let mut lock = fd_lock::RwLock::new(file);
        let deadline = Instant::now() + timeout;
        loop {
            match lock.try_write() {
                Ok(mut guard) => {
                    guard.set_len(0)?;
                    guard.seek(SeekFrom::Start(0))?;
                    guard.write_all(format!("#{}\n", std::process::id()).as_bytes())?;
                    guard.flush()?;
                    std::mem::forget(guard);
                    break;
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(StoreError::Locked {
                            pid: holder_pid(&path),
                        });
                    }
                    std::thread::sleep(POLL_INTERVAL.min(deadline - now));
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(WriterLock {
            dir: aroma_dir.to_path_buf(),
            _file: lock,
        })
    }

    /// The `.aroma` directory this lock guards (as passed to `acquire`).
    pub fn aroma_dir(&self) -> &Path {
        &self.dir
    }
}

/// Pid written by the current holder, read from byte 1 on; 0 when it cannot be read.
fn holder_pid(path: &Path) -> u32 {
    let read = || -> std::io::Result<String> {
        let mut f = File::open(path)?;
        f.seek(SeekFrom::Start(1))?;
        let mut s = String::new();
        f.read_to_string(&mut s)?;
        Ok(s)
    };
    read().ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}
