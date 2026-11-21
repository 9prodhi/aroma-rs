//! `unit_table`: fixed 48-byte `units.bin` records + `files.json`. Owned and implemented
//! by task T09 (`write_unit_table`, `UnitTable`).
//!
//! `units.bin` (every integer little-endian):
//!
//! ```text
//! offset  size  field
//!      0     8  magic     b"AROMAUNT"
//!      8     4  version   u32 = 1
//!     12     4  count     u32
//!     16        records   [DiskRecord; count], 48 bytes each, record i has id i
//!
//! DiskRecord: id u32 | lang u8 | kind u8 | is_test u8 | pad u8 = 0 | file u32 |
//!             bytes_start u32 | bytes_end u32 | line_start u32 | line_end u32 |
//!             n_leaves u32 | body_hash u64 | reserved u64 = 0
//! ```
//!
//! `lang`: 0 = java, 1 = apex. `kind`: 0 = method, 1 = constructor, 2 = trigger,
//! 3 = accessor, 4 = initializer, 5 = snippet. `files.json` is a JSON array of the
//! file paths a record's `file` field indexes, in order.

use crate::lang::Lang;
use crate::store::StoreError;
use crate::unit::{UnitId, UnitKind, UnitRecord};
use bytemuck::{Pod, Zeroable};
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const UNITS_FILE: &str = "units.bin";
pub const FILES_FILE: &str = "files.json";
pub const MAGIC: &[u8; 8] = b"AROMAUNT";
pub const VERSION: u32 = 1;
pub const HEADER_LEN: usize = 16;
pub const RECORD_LEN: usize = 48;

/// On-disk record. Fields hold little-endian values (`to_le` on write, `from_le` on
/// read), so the layout is identical on every host.
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct DiskRecord {
    id: u32,
    lang: u8,
    kind: u8,
    is_test: u8,
    pad: u8,
    file: u32,
    bytes_start: u32,
    bytes_end: u32,
    line_start: u32,
    line_end: u32,
    n_leaves: u32,
    body_hash: u64,
    reserved: u64,
}

const _: () = assert!(std::mem::size_of::<DiskRecord>() == RECORD_LEN);
const _: () = assert!(std::mem::align_of::<DiskRecord>() == 8);
const _: () = assert!(HEADER_LEN.is_multiple_of(8));

fn lang_code(l: Lang) -> u8 {
    match l {
        Lang::Java => 0,
        Lang::Apex => 1,
        Lang::LwcJs => 2,
        Lang::LwcHtml => 3,
        Lang::SfMeta => 4,
        Lang::Aura => 5,
        Lang::AuraJs => 6,
        Lang::Vf => 7,
    }
}

fn lang_from(c: u8) -> Option<Lang> {
    match c {
        0 => Some(Lang::Java),
        1 => Some(Lang::Apex),
        2 => Some(Lang::LwcJs),
        3 => Some(Lang::LwcHtml),
        4 => Some(Lang::SfMeta),
        5 => Some(Lang::Aura),
        6 => Some(Lang::AuraJs),
        7 => Some(Lang::Vf),
        _ => None,
    }
}

fn kind_code(k: UnitKind) -> u8 {
    match k {
        UnitKind::Method => 0,
        UnitKind::Constructor => 1,
        UnitKind::Trigger => 2,
        UnitKind::Accessor => 3,
        UnitKind::Initializer => 4,
        UnitKind::Snippet => 5,
        UnitKind::Template => 6,
        UnitKind::MarkupBlock => 7,
        UnitKind::WireMember => 8,
        UnitKind::FlowElement => 9,
        UnitKind::Field => 10,
        UnitKind::ValidationRule => 11,
        UnitKind::MetadataRecord => 12,
        UnitKind::Page => 13,
        UnitKind::Component => 14,
    }
}

fn kind_from(c: u8) -> Option<UnitKind> {
    match c {
        0 => Some(UnitKind::Method),
        1 => Some(UnitKind::Constructor),
        2 => Some(UnitKind::Trigger),
        3 => Some(UnitKind::Accessor),
        4 => Some(UnitKind::Initializer),
        5 => Some(UnitKind::Snippet),
        6 => Some(UnitKind::Template),
        7 => Some(UnitKind::MarkupBlock),
        8 => Some(UnitKind::WireMember),
        9 => Some(UnitKind::FlowElement),
        10 => Some(UnitKind::Field),
        11 => Some(UnitKind::ValidationRule),
        12 => Some(UnitKind::MetadataRecord),
        13 => Some(UnitKind::Page),
        14 => Some(UnitKind::Component),
        _ => None,
    }
}

fn encode(r: &UnitRecord) -> DiskRecord {
    DiskRecord {
        id: r.id.to_le(),
        lang: lang_code(r.lang),
        kind: kind_code(r.kind),
        is_test: u8::from(r.is_test),
        pad: 0,
        file: r.file.to_le(),
        bytes_start: r.bytes.start.to_le(),
        bytes_end: r.bytes.end.to_le(),
        line_start: r.lines.start.to_le(),
        line_end: r.lines.end.to_le(),
        n_leaves: r.n_leaves.to_le(),
        body_hash: r.body_hash.to_le(),
        reserved: 0,
    }
}

/// Decode a record `open` has validated (codes in range, so the `unwrap_or` arms are
/// unreachable).
fn decode(d: &DiskRecord) -> UnitRecord {
    UnitRecord {
        id: u32::from_le(d.id),
        lang: lang_from(d.lang).unwrap_or(Lang::Java),
        kind: kind_from(d.kind).unwrap_or(UnitKind::Method),
        file: u32::from_le(d.file),
        bytes: u32::from_le(d.bytes_start)..u32::from_le(d.bytes_end),
        lines: u32::from_le(d.line_start)..u32::from_le(d.line_end),
        n_leaves: u32::from_le(d.n_leaves),
        body_hash: u64::from_le(d.body_hash),
        is_test: d.is_test == 1,
    }
}

fn invalid(msg: String) -> StoreError {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, msg).into()
}

fn corrupt(file: &'static str, reason: String) -> StoreError {
    StoreError::Corrupt { file, reason }
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let mut f = File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

/// Write `dir/units.bin` and `dir/files.json`. `records[i].id` must be `i` and every
/// `records[i].file` must index `files` (`ErrorKind::InvalidInput` otherwise); a path
/// that is not valid UTF-8 is `StoreError::Codec`. Both files are fsynced; the
/// generation commit (T10) makes them visible atomically.
pub fn write_unit_table(
    dir: &Path,
    records: &[UnitRecord],
    files: &[PathBuf],
) -> Result<(), StoreError> {
    let count = u32::try_from(records.len())
        .map_err(|_| invalid(format!("{} units exceed u32", records.len())))?;
    let mut buf = Vec::with_capacity(HEADER_LEN + RECORD_LEN * records.len());
    buf.extend_from_slice(MAGIC);
    buf.extend_from_slice(&VERSION.to_le_bytes());
    buf.extend_from_slice(&count.to_le_bytes());
    for (i, r) in records.iter().enumerate() {
        if r.id as usize != i {
            return Err(invalid(format!("record {i} has id {}", r.id)));
        }
        if r.file as usize >= files.len() {
            return Err(invalid(format!(
                "record {i} names file {} of {}",
                r.file,
                files.len()
            )));
        }
        buf.extend_from_slice(bytemuck::bytes_of(&encode(r)));
    }
    let mut json = serde_json::to_string_pretty(files)
        .map_err(|e| StoreError::Codec(format!("{FILES_FILE}: {e}")))?;
    json.push('\n');
    write_synced(&dir.join(UNITS_FILE), &buf)?;
    write_synced(&dir.join(FILES_FILE), json.as_bytes())?;
    Ok(())
}

/// Memory-mapped `units.bin` plus the decoded `files.json` path list.
#[derive(Debug)]
pub struct UnitTable {
    map: memmap2::Mmap,
    count: u32,
    files: Vec<PathBuf>,
}

impl UnitTable {
    /// Map `dir/units.bin`, read `dir/files.json` and validate every record (O(n)):
    /// magic, version, exact length, `id == index`, lang/kind/is_test codes, zero
    /// padding, `bytes`/`lines` ranges not reversed, and `file < files.len()`.
    /// Violations are `StoreError::Corrupt`.
    pub fn open(dir: &Path) -> Result<UnitTable, StoreError> {
        if cfg!(target_endian = "big") {
            return Err(StoreError::Incompatible(
                "units.bin is mapped in place and needs a little-endian host".into(),
            ));
        }
        let text = std::fs::read_to_string(dir.join(FILES_FILE))?;
        let files: Vec<PathBuf> =
            serde_json::from_str(&text).map_err(|e| corrupt(FILES_FILE, e.to_string()))?;

        let file = File::open(dir.join(UNITS_FILE))?;
        let len = file.metadata()?.len();
        if len < HEADER_LEN as u64 {
            return Err(corrupt(
                UNITS_FILE,
                format!("file is {len} bytes, shorter than the {HEADER_LEN}-byte header"),
            ));
        }
        // SAFETY: a published generation is immutable (D23d); nothing writes or truncates
        // units.bin while it is mapped.
        let map = unsafe { memmap2::Mmap::map(&file)? };
        if &map[0..8] != MAGIC {
            return Err(corrupt(UNITS_FILE, "bad magic".into()));
        }
        let version = u32::from_le_bytes([map[8], map[9], map[10], map[11]]);
        if version != VERSION {
            return Err(corrupt(
                UNITS_FILE,
                format!("unsupported version {version} (expected {VERSION})"),
            ));
        }
        let count = u32::from_le_bytes([map[12], map[13], map[14], map[15]]);
        let expected = HEADER_LEN as u64 + RECORD_LEN as u64 * u64::from(count);
        if len != expected {
            return Err(corrupt(
                UNITS_FILE,
                format!("file is {len} bytes, header implies {expected}"),
            ));
        }
        let table = UnitTable { map, count, files };
        for (i, d) in table.disk_records()?.iter().enumerate() {
            table.check(i, d)?;
        }
        Ok(table)
    }

    fn disk_records(&self) -> Result<&[DiskRecord], StoreError> {
        bytemuck::try_cast_slice(&self.map[HEADER_LEN..])
            .map_err(|e| corrupt(UNITS_FILE, format!("record cast failed: {e:?}")))
    }

    fn check(&self, i: usize, d: &DiskRecord) -> Result<(), StoreError> {
        let bad = |what: String| Err(corrupt(UNITS_FILE, format!("record {i}: {what}")));
        let id = u32::from_le(d.id);
        if id as usize != i {
            return bad(format!("id {id}"));
        }
        if lang_from(d.lang).is_none() {
            return bad(format!("lang code {}", d.lang));
        }
        if kind_from(d.kind).is_none() {
            return bad(format!("kind code {}", d.kind));
        }
        if d.is_test > 1 {
            return bad(format!("is_test byte {}", d.is_test));
        }
        if d.pad != 0 || d.reserved != 0 {
            return bad("non-zero padding".into());
        }
        let r = decode(d);
        if r.bytes.start > r.bytes.end {
            return bad(format!("bytes {}..{}", r.bytes.start, r.bytes.end));
        }
        if r.lines.start > r.lines.end {
            return bad(format!("lines {}..{}", r.lines.start, r.lines.end));
        }
        if r.file as usize >= self.files.len() {
            return bad(format!("file {} of {}", r.file, self.files.len()));
        }
        Ok(())
    }

    pub fn n_units(&self) -> u32 {
        self.count
    }

    /// Record of unit `id`. Ids come from `MmapPostings`, whose `open` proved every id
    /// `< n_units`, and the index facade checks the manifest's `n_units` equals
    /// `UnitTable::n_units`. An out-of-range id is a caller bug and panics.
    pub fn record(&self, id: UnitId) -> UnitRecord {
        debug_assert!(id < self.count, "unit id {id} >= n_units {}", self.count);
        let start = HEADER_LEN + RECORD_LEN * id as usize;
        let d: &DiskRecord = bytemuck::from_bytes(&self.map[start..start + RECORD_LEN]);
        decode(d)
    }

    /// Path of file `file` (as given to `write_unit_table`). Panics when out of range;
    /// every stored record's `file` was validated by `open`.
    pub fn file_path(&self, file: u32) -> &Path {
        &self.files[file as usize]
    }

    /// Number of entries in `files.json`.
    pub fn n_files(&self) -> u32 {
        self.files.len() as u32
    }
}
