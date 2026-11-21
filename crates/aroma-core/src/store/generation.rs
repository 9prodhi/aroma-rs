//! `generation`: `.aroma/gen-<n>` directories, `CURRENT` pointer, atomic publish.
//! Owned and implemented by task T10 (`IndexDir`, `GenerationWriter`).
//!
//! ```text
//! <repo>/.aroma/
//!   lock            writer lock (store::lock)
//!   CURRENT         "<n>\n": the published generation readers open
//!   gen-<n>/        immutable generation n (manifest.json, postings.bin, ...)
//!   gen-<n-1>/      kept so readers that opened it before the switch keep working
//!   tmp-<m>/        generation being built (or left behind by a crashed writer)
//! ```
//!
//! Publishing: write every file into `tmp-<n>/`, fsync the files and the directory,
//! rename `tmp-<n>` to `gen-<n>`, then replace `CURRENT` through `CURRENT.tmp` +
//! fsync + rename. A crash at any point leaves `CURRENT` naming a complete generation.

use crate::store::lock::WriterLock;
use crate::store::StoreError;
use std::fs::File;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

pub const AROMA_DIR: &str = ".aroma";
pub const CURRENT_FILE: &str = "CURRENT";
const CURRENT_TMP: &str = "CURRENT.tmp";

/// Name of generation `n`'s published directory.
pub fn gen_dir_name(n: u64) -> String {
    format!("gen-{n}")
}

/// Name of generation `n`'s build directory.
pub fn tmp_dir_name(n: u64) -> String {
    format!("tmp-{n}")
}

/// `n` for a directory named `<prefix><n>` with `n` all ASCII digits.
pub fn parse_numbered(name: &str, prefix: &str) -> Option<u64> {
    let digits = name.strip_prefix(prefix)?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// The `.aroma` directory of one repository.
#[derive(Clone, Debug)]
pub struct IndexDir {
    aroma: PathBuf,
}

impl IndexDir {
    /// `<repo_root>/.aroma`; does not touch the disk.
    pub fn new(repo_root: &Path) -> IndexDir {
        IndexDir {
            aroma: repo_root.join(AROMA_DIR),
        }
    }

    pub fn aroma_dir(&self) -> &Path {
        &self.aroma
    }

    /// The published generation: `None` when `CURRENT` does not exist (never built).
    /// `CURRENT` that is not a decimal number, or that names a missing `gen-<n>`
    /// directory, is `StoreError::Corrupt { file: "CURRENT", .. }`.
    pub fn current(&self) -> Result<Option<(u64, PathBuf)>, StoreError> {
        let text = match std::fs::read_to_string(self.aroma.join(CURRENT_FILE)) {
            Ok(t) => t,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let n: u64 = text.trim().parse().map_err(|_| StoreError::Corrupt {
            file: "CURRENT",
            reason: format!("not a generation number: {:?}", text.trim()),
        })?;
        let dir = self.aroma.join(gen_dir_name(n));
        if !dir.is_dir() {
            return Err(StoreError::Corrupt {
                file: "CURRENT",
                reason: format!("names {} which does not exist", gen_dir_name(n)),
            });
        }
        Ok(Some((n, dir)))
    }

    /// Start generation `current + 1` (1 for a fresh index) in `.aroma/tmp-<n>/`.
    /// Leftovers named `tmp-<n>` or `gen-<n>` from a writer that crashed before
    /// publishing are deleted first; nothing reads them because `CURRENT` < n.
    /// `lock` must guard this directory (`WriterLock::acquire(self.aroma_dir(), ..)`).
    pub fn begin(&self, lock: &WriterLock) -> Result<GenerationWriter, StoreError> {
        self.check_lock(lock)?;
        std::fs::create_dir_all(&self.aroma)?;
        let number = self.current()?.map_or(1, |(n, _)| n + 1);
        let tmp = self.aroma.join(tmp_dir_name(number));
        remove_dir_if_exists(&tmp)?;
        remove_dir_if_exists(&self.aroma.join(gen_dir_name(number)))?;
        std::fs::create_dir(&tmp)?;
        Ok(GenerationWriter {
            aroma: self.aroma.clone(),
            number,
            tmp,
        })
    }

    /// `ErrorKind::InvalidInput` unless `lock` guards this `.aroma` directory.
    pub(crate) fn check_lock(&self, lock: &WriterLock) -> Result<(), StoreError> {
        if same_dir(lock.aroma_dir(), &self.aroma) {
            return Ok(());
        }
        Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "writer lock guards {} but the index is {}",
                lock.aroma_dir().display(),
                self.aroma.display()
            ),
        )
        .into())
    }
}

/// A generation being built. Dropping it without `commit` leaves `tmp-<n>/` on disk
/// (exactly what a crash leaves); the next `begin` or `collect_garbage` removes it.
#[derive(Debug)]
pub struct GenerationWriter {
    aroma: PathBuf,
    number: u64,
    tmp: PathBuf,
}

impl GenerationWriter {
    pub fn number(&self) -> u64 {
        self.number
    }

    /// Directory to write the generation's files into (`.aroma/tmp-<n>/`).
    pub fn dir(&self) -> &Path {
        &self.tmp
    }

    /// Fsync every file under `dir()` and the directory itself, rename it to
    /// `gen-<n>`, then atomically replace `CURRENT` with `n`. Returns `n`.
    pub fn commit(self) -> Result<u64, StoreError> {
        sync_tree(&self.tmp)?;
        let gen = self.aroma.join(gen_dir_name(self.number));
        std::fs::rename(&self.tmp, &gen)?;
        sync_dir(&self.aroma)?;
        let tmp_current = self.aroma.join(CURRENT_TMP);
        let mut f = File::create(&tmp_current)?;
        f.write_all(format!("{}\n", self.number).as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp_current, self.aroma.join(CURRENT_FILE))?;
        sync_dir(&self.aroma)?;
        Ok(self.number)
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

fn remove_dir_if_exists(dir: &Path) -> Result<(), StoreError> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Fsync every regular file below `dir` (recursively), then `dir` itself.
fn sync_tree(dir: &Path) -> Result<(), StoreError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            sync_tree(&entry.path())?;
        } else if ty.is_file() {
            File::open(entry.path())?.sync_all()?;
        }
    }
    sync_dir(dir)
}

/// Persist a directory's entries (renames, creations). Windows cannot open a
/// directory as a `File`; NTFS journals the metadata, so this is a no-op there.
#[cfg(unix)]
fn sync_dir(dir: &Path) -> Result<(), StoreError> {
    File::open(dir)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> Result<(), StoreError> {
    Ok(())
}
