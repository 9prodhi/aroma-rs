//! `spt_store`: `spt.bin` offset table + postcard SPT blobs (D19). Owned and implemented
//! by task T09 (`write_spt_store`, `SptStore`).
//!
//! `spt.bin` (every integer little-endian):
//!
//! ```text
//! offset  size  field
//!      0     8  magic     b"AROMASPT"
//!      8     4  version   u32 = 1
//!     12     4  count     u32
//!     16        offsets   [u64; count + 1], relative to the blob area:
//!                         offsets[0] = 0, non-decreasing, offsets[count] = blob length
//!      .        blobs     postcard(Spt) of unit i at blob[offsets[i]..offsets[i + 1]]
//! ```
//!
//! File length = `16 + 8 * (count + 1) + offsets[count]`. A query decodes only the
//! blobs of its candidates; `get` validates the decoded arena before returning it.

use crate::spt::{Elem, Spt};
use crate::store::StoreError;
use crate::unit::UnitId;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub const MAGIC: &[u8; 8] = b"AROMASPT";
pub const VERSION: u32 = 1;
pub const HEADER_LEN: usize = 16;
const FILE: &str = "spt.bin";

fn corrupt(reason: String) -> StoreError {
    StoreError::Corrupt { file: FILE, reason }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Write `spt.bin` for units `0..spts.len()` (blob `i` is unit `i`) to `<path>.tmp`,
/// fsync it and rename it over `path`. Encoding failures are `StoreError::Codec`.
pub fn write_spt_store(path: &Path, spts: &[Spt]) -> Result<(), StoreError> {
    let count = u32::try_from(spts.len()).map_err(|_| {
        StoreError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{} trees exceed u32", spts.len()),
        ))
    })?;
    let mut blobs = Vec::with_capacity(spts.len());
    for (i, s) in spts.iter().enumerate() {
        blobs
            .push(postcard::to_stdvec(s).map_err(|e| StoreError::Codec(format!("unit {i}: {e}")))?);
    }
    let tmp = tmp_path(path);
    let mut w = BufWriter::new(File::create(&tmp)?);
    w.write_all(MAGIC)?;
    w.write_all(&VERSION.to_le_bytes())?;
    w.write_all(&count.to_le_bytes())?;
    let mut off = 0u64;
    w.write_all(&off.to_le_bytes())?;
    for b in &blobs {
        off += b.len() as u64;
        w.write_all(&off.to_le_bytes())?;
    }
    for b in &blobs {
        w.write_all(b)?;
    }
    let file = w.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Memory-mapped `spt.bin`; the offset table is validated by `open`, blobs by `get`.
#[derive(Debug)]
pub struct SptStore {
    map: memmap2::Mmap,
    count: u32,
}

impl SptStore {
    /// Map `path` and validate magic, version, length and the offset table (O(count)).
    pub fn open(path: &Path) -> Result<SptStore, StoreError> {
        if cfg!(target_endian = "big") {
            return Err(StoreError::Incompatible(
                "spt.bin is mapped in place and needs a little-endian host".into(),
            ));
        }
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        if len < HEADER_LEN as u64 {
            return Err(corrupt(format!(
                "file is {len} bytes, shorter than the {HEADER_LEN}-byte header"
            )));
        }
        // SAFETY: a published generation is immutable (D23d); nothing writes or truncates
        // spt.bin while it is mapped.
        let map = unsafe { memmap2::Mmap::map(&file)? };
        if &map[0..8] != MAGIC {
            return Err(corrupt("bad magic".into()));
        }
        let version = u32::from_le_bytes([map[8], map[9], map[10], map[11]]);
        if version != VERSION {
            return Err(corrupt(format!(
                "unsupported version {version} (expected {VERSION})"
            )));
        }
        let count = u32::from_le_bytes([map[12], map[13], map[14], map[15]]);
        let table_end = HEADER_LEN as u64 + 8 * (u64::from(count) + 1);
        if len < table_end {
            return Err(corrupt(format!(
                "file is {len} bytes, offset table needs {table_end}"
            )));
        }
        let store = SptStore { map, count };
        let offs = store.offsets()?;
        if offs[0] != 0 {
            return Err(corrupt(format!("offset 0 = {} (expected 0)", offs[0])));
        }
        if let Some(i) = offs.windows(2).position(|w| w[0] > w[1]) {
            return Err(corrupt(format!("offsets decrease at index {}", i + 1)));
        }
        let blob_len = len - table_end;
        if offs[count as usize] != blob_len {
            return Err(corrupt(format!(
                "last offset {} (expected blob length {blob_len})",
                offs[count as usize]
            )));
        }
        Ok(store)
    }

    fn table_end(&self) -> usize {
        HEADER_LEN + 8 * (self.count as usize + 1)
    }

    fn offsets(&self) -> Result<&[u64], StoreError> {
        bytemuck::try_cast_slice(&self.map[HEADER_LEN..self.table_end()])
            .map_err(|e| corrupt(format!("offset table cast failed: {e:?}")))
    }

    pub fn len(&self) -> u32 {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Decode the tree of unit `id`. An id `>= len()`, a blob that is not exactly one
    /// postcard `Spt`, or a tree whose arena references are out of range or not in
    /// pre-order (child node id > parent id) is `StoreError::Corrupt`.
    pub fn get(&self, id: UnitId) -> Result<Spt, StoreError> {
        if id >= self.count {
            return Err(corrupt(format!(
                "unit id {id} out of range (len {})",
                self.count
            )));
        }
        let offs = self.offsets()?;
        let base = self.table_end();
        let (s, e) = (offs[id as usize] as usize, offs[id as usize + 1] as usize);
        let blob = &self.map[base + s..base + e];
        let (spt, rest): (Spt, &[u8]) =
            postcard::take_from_bytes(blob).map_err(|err| corrupt(format!("unit {id}: {err}")))?;
        if !rest.is_empty() {
            return Err(corrupt(format!(
                "unit {id}: {} trailing bytes after the tree",
                rest.len()
            )));
        }
        check_arena(&spt).map_err(|why| corrupt(format!("unit {id}: {why}")))?;
        Ok(spt)
    }
}

/// Every reference in the arena is in range and child nodes come after their parent
/// (pre-order, as `Spt::build` and `Spt::from_reference_json` produce), so walking the
/// tree can neither index out of bounds nor loop.
fn check_arena(spt: &Spt) -> Result<(), String> {
    let n_nodes = spt.nodes.len();
    let n_leaves = spt.leaves.len();
    if spt.root as usize >= n_nodes {
        return Err(format!("root {} of {n_nodes} nodes", spt.root));
    }
    for (i, node) in spt.nodes.iter().enumerate() {
        for c in &node.children {
            match *c {
                Elem::Keyword(_) => {}
                Elem::Leaf(l) if (l as usize) < n_leaves => {}
                Elem::Leaf(l) => return Err(format!("node {i}: leaf {l} of {n_leaves}")),
                Elem::Tree(t) if (t as usize) > i && (t as usize) < n_nodes => {}
                Elem::Tree(t) => return Err(format!("node {i}: child node {t} of {n_nodes}")),
            }
        }
    }
    Ok(())
}
