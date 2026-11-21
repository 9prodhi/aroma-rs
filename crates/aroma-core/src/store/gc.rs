//! `gc`: delete generations older than current-1 (D23d). Owned and implemented by task T10
//! (`collect_garbage`).
//!
//! With `CURRENT = n`, `gen-n` and `gen-(n-1)` stay (a reader that opened n-1 just
//! before the switch keeps working); every other `gen-<k>` is removed: `k < n-1` are
//! superseded, `k > n` were renamed by a writer that crashed before rewriting
//! `CURRENT`. Every `tmp-*` directory and a stray `CURRENT.tmp` are removed too (the
//! caller holds the writer lock, so none of them belongs to a live writer). Without
//! `CURRENT` no `gen-*` is touched.
//!
//! On Windows a file that a reader still has mapped cannot be deleted; such a failure
//! is skipped silently and retried by the next run (D23d). Elsewhere it is an error.

use crate::store::generation::{parse_numbered, IndexDir};
use crate::store::lock::WriterLock;
use crate::store::StoreError;
use std::io::ErrorKind;
use std::path::Path;

/// Remove garbage under `index_dir` (see the module docs). `lock` must guard it
/// (`ErrorKind::InvalidInput` otherwise). Returns the numbers of the `gen-<k>`
/// directories removed, ascending; removed `tmp-*` directories are not listed.
pub fn collect_garbage(index_dir: &IndexDir, lock: &WriterLock) -> Result<Vec<u64>, StoreError> {
    index_dir.check_lock(lock)?;
    let aroma = index_dir.aroma_dir();
    let entries = match std::fs::read_dir(aroma) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let current = index_dir.current()?.map(|(n, _)| n);
    let mut removed = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let path = entry.path();
        if name == "CURRENT.tmp" {
            remove(&path, false)?;
        } else if name.starts_with("tmp-") && entry.file_type()?.is_dir() {
            remove(&path, true)?;
        } else if let (Some(k), Some(n)) = (parse_numbered(name, "gen-"), current) {
            let keep = k == n || k + 1 == n;
            if !keep && entry.file_type()?.is_dir() && remove(&path, true)? {
                removed.push(k);
            }
        }
    }
    removed.sort_unstable();
    Ok(removed)
}

/// `Ok(true)` when removed, `Ok(false)` when already gone or (Windows only) deferred.
fn remove(path: &Path, dir: bool) -> Result<bool, StoreError> {
    let res = if dir {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    };
    match res {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(false),
        Err(e) if cfg!(windows) => {
            tracing::debug!(path = %path.display(), error = %e, "deferring removal to the next run");
            Ok(false)
        }
        Err(e) => Err(e.into()),
    }
}
