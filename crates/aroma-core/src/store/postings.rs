//! `postings`: mmapped CSR `postings.bin` (D15). Owned and implemented by task T08
//! (`write_postings`, `MmapPostings`).
//!
//! Layout (every integer little-endian; every array starts on an 8-byte boundary
//! because the header is 48 bytes and both u64 arrays have 8-byte elements):
//!
//! ```text
//! offset  size  field
//!      0     8  magic            b"AROMAPST"
//!      8     4  version          u32 = 1
//!     12     4  endian marker    u32 = 0x0102_0304 (reads 0x0403_0201 when byte-swapped)
//!     16     8  n_features       u64
//!     24     8  n_entries        u64
//!     32     4  n_units          u32 (every stored unit id is < n_units)
//!     36     4  reserved         u32 = 0
//!     40     8  header check     u64 = first 8 bytes (LE) of blake3(bytes 0..40)
//!     48        features         [u64; n_features]      strictly increasing
//!      .        offsets          [u64; n_features + 1]  offsets[0] = 0, non-decreasing,
//!                                                       offsets[n_features] = n_entries
//!      .        unit ids         [u32; n_entries]       strictly increasing per feature
//! ```
//!
//! The postings of `features[i]` are `unit_ids[offsets[i]..offsets[i + 1]]`. The file
//! length must equal `48 + 8 * n_features + 8 * (n_features + 1) + 4 * n_entries`.

use crate::multiset::FeatureId;
use crate::store::StoreError;
use crate::unit::UnitId;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub const MAGIC: &[u8; 8] = b"AROMAPST";
pub const VERSION: u32 = 1;
pub const ENDIAN_MARKER: u32 = 0x0102_0304;
pub const HEADER_LEN: usize = 48;

/// First 8 bytes (little-endian) of blake3 over the 40 header bytes before the check.
pub fn header_check(first_40: &[u8]) -> u64 {
    let hash = blake3::hash(first_40);
    let mut b = [0u8; 8];
    b.copy_from_slice(&hash.as_bytes()[..8]);
    u64::from_le_bytes(b)
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Write `postings.bin`: sorts and dedups `entries`, groups them by feature and writes
/// the CSR arrays to `<path>.tmp`, fsyncs it and renames it over `path`.
/// Fails with `ErrorKind::InvalidInput` when a unit id is `>= n_units`.
pub fn write_postings(
    path: &Path,
    mut entries: Vec<(FeatureId, UnitId)>,
    n_units: u32,
) -> Result<(), StoreError> {
    entries.sort_unstable();
    entries.dedup();
    if let Some(&(f, u)) = entries.iter().find(|&&(_, u)| u >= n_units) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("unit id {u} of feature {f:#018x} is >= n_units {n_units}"),
        )
        .into());
    }
    let mut features: Vec<FeatureId> = Vec::new();
    let mut offsets: Vec<u64> = vec![0];
    for (i, &(f, _)) in entries.iter().enumerate() {
        if features.last() != Some(&f) {
            if !features.is_empty() {
                offsets.push(i as u64);
            }
            features.push(f);
        }
    }
    if !features.is_empty() {
        offsets.push(entries.len() as u64);
    }

    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(MAGIC);
    header[8..12].copy_from_slice(&VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&ENDIAN_MARKER.to_le_bytes());
    header[16..24].copy_from_slice(&(features.len() as u64).to_le_bytes());
    header[24..32].copy_from_slice(&(entries.len() as u64).to_le_bytes());
    header[32..36].copy_from_slice(&n_units.to_le_bytes());
    header[36..40].copy_from_slice(&0u32.to_le_bytes());
    let check = header_check(&header[0..40]);
    header[40..48].copy_from_slice(&check.to_le_bytes());

    let tmp = tmp_path(path);
    let mut w = BufWriter::new(File::create(&tmp)?);
    w.write_all(&header)?;
    for f in &features {
        w.write_all(&f.to_le_bytes())?;
    }
    for o in &offsets {
        w.write_all(&o.to_le_bytes())?;
    }
    for &(_, u) in &entries {
        w.write_all(&u.to_le_bytes())?;
    }
    let file = w.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

const FILE: &str = "postings.bin";

fn corrupt(reason: String) -> StoreError {
    StoreError::Corrupt { file: FILE, reason }
}

fn le_u32(b: &[u8], at: usize) -> u32 {
    let mut a = [0u8; 4];
    a.copy_from_slice(&b[at..at + 4]);
    u32::from_le_bytes(a)
}

fn le_u64(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[at..at + 8]);
    u64::from_le_bytes(a)
}

/// Read-only, memory-mapped `postings.bin`. [`MmapPostings::open`] checks the header
/// and the length (O(1)); the feature/offset arrays and the per-list unit-id invariants
/// are checked only by [`MmapPostings::verify`] (O(n_entries), never on the query path). Lookups binary-search the feature array and return a borrowed
/// slice of the map (no deserialisation).
#[derive(Debug)]
pub struct MmapPostings {
    map: memmap2::Mmap,
    n_features: usize,
    n_entries: usize,
    n_units: u32,
}

impl MmapPostings {
    /// Map `path` and validate the header and the length in O(1); any violated
    /// invariant is `StoreError::Corrupt`. Arrays are checked by `verify`.
    pub fn open(path: &Path) -> Result<MmapPostings, StoreError> {
        if cfg!(target_endian = "big") {
            return Err(StoreError::Incompatible(
                "postings.bin is mapped in place and needs a little-endian host".into(),
            ));
        }
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        if len < HEADER_LEN as u64 {
            return Err(corrupt(format!(
                "file is {len} bytes, shorter than the {HEADER_LEN}-byte header"
            )));
        }
        // SAFETY: the index is immutable once its generation is published (D23d): no
        // writer modifies or truncates a published postings.bin while it is mapped.
        let map = unsafe { memmap2::Mmap::map(&file)? };
        let b: &[u8] = &map;
        if &b[0..8] != MAGIC {
            return Err(corrupt("bad magic".into()));
        }
        let version = le_u32(b, 8);
        if version != VERSION {
            return Err(corrupt(format!(
                "unsupported version {version} (expected {VERSION})"
            )));
        }
        let marker = le_u32(b, 12);
        if marker != ENDIAN_MARKER {
            return Err(corrupt(format!(
                "endian marker {marker:#010x} (expected {ENDIAN_MARKER:#010x})"
            )));
        }
        if le_u64(b, 40) != header_check(&b[0..40]) {
            return Err(corrupt("header check mismatch".into()));
        }
        let nf = le_u64(b, 16);
        let ne = le_u64(b, 24);
        let n_units = le_u32(b, 32);
        let expected = (nf as u128) * 16 + 8 + (ne as u128) * 4 + HEADER_LEN as u128;
        if expected != len as u128 {
            return Err(corrupt(format!(
                "file is {len} bytes, header implies {expected}"
            )));
        }
        // Both counts fit in usize: together they describe a file that exists.
        let (n_features, n_entries) = (nf as usize, ne as usize);
        let p = MmapPostings {
            map,
            n_features,
            n_entries,
            n_units,
        };
        Ok(p)
    }

    /// O(n_features): the features and offsets arrays (part of [`MmapPostings::verify`]).
    /// Not run by `open`: on the 233k-unit perf fixture it touched ~40 MB per one-shot
    /// query (7-10 ms), and lookups stay panic-free without it (see `postings`).
    fn validate(&self) -> Result<(), StoreError> {
        let (fs, offs, _) = self.arrays()?;
        if let Some(i) = fs.windows(2).position(|w| w[0] >= w[1]) {
            return Err(corrupt(format!(
                "feature ids not strictly increasing at index {}",
                i + 1
            )));
        }
        let ne = self.n_entries as u64;
        if let Some((i, &o)) = offs.iter().enumerate().find(|&(_, &o)| o > ne) {
            return Err(corrupt(format!("offset {i} = {o} exceeds n_entries {ne}")));
        }
        if offs[0] != 0 {
            return Err(corrupt(format!("offset 0 = {} (expected 0)", offs[0])));
        }
        if let Some(i) = offs.windows(2).position(|w| w[0] > w[1]) {
            return Err(corrupt(format!("offsets decrease at index {}", i + 1)));
        }
        if offs[self.n_features] != ne {
            return Err(corrupt(format!(
                "last offset {} (expected n_entries {ne})",
                offs[self.n_features]
            )));
        }
        Ok(())
    }

    /// Full O(n_entries) scan: every unit id `< n_units` and strictly increasing within
    /// its feature. Additive public item for `aroma inspect-index` / build (T15); the
    /// query path must not call it.
    pub fn verify(&self) -> Result<(), StoreError> {
        self.validate()?;
        let (_, offs, units) = self.arrays()?;
        for (i, w) in offs.windows(2).enumerate() {
            let list = &units[w[0] as usize..w[1] as usize];
            if let Some(&u) = list.iter().find(|&&u| u >= self.n_units) {
                return Err(corrupt(format!(
                    "unit id {u} of feature index {i} is >= n_units {}",
                    self.n_units
                )));
            }
            if list.windows(2).any(|p| p[0] >= p[1]) {
                return Err(corrupt(format!(
                    "unit ids of feature index {i} not strictly increasing"
                )));
            }
        }
        Ok(())
    }

    /// `(features, offsets, unit ids)` as typed slices of the map. Alignment holds by
    /// construction (page-aligned map, 8-byte-aligned array starts); a misaligned or
    /// odd-sized slice is reported as corruption rather than a panic.
    #[allow(clippy::type_complexity)]
    fn arrays(&self) -> Result<(&[u64], &[u64], &[u32]), StoreError> {
        let b: &[u8] = &self.map;
        let f_end = HEADER_LEN + 8 * self.n_features;
        let o_end = f_end + 8 * (self.n_features + 1);
        let u_end = o_end + 4 * self.n_entries;
        let cast = |e: bytemuck::PodCastError| corrupt(format!("array cast failed: {e:?}"));
        Ok((
            bytemuck::try_cast_slice(&b[HEADER_LEN..f_end]).map_err(cast)?,
            bytemuck::try_cast_slice(&b[f_end..o_end]).map_err(cast)?,
            bytemuck::try_cast_slice(&b[o_end..u_end]).map_err(cast)?,
        ))
    }

    pub fn n_features(&self) -> u64 {
        self.n_features as u64
    }

    pub fn n_entries(&self) -> u64 {
        self.n_entries as u64
    }
}

impl crate::index::PostingSource for MmapPostings {
    fn n_units(&self) -> u32 {
        self.n_units
    }

    fn postings(&self, f: FeatureId) -> &[UnitId] {
        // `open` proved the sizes, so the casts succeed and `offs` has n_features + 1
        // entries; the bounds check below keeps this panic-free without `verify` (corrupt
        // feature order can only make a lookup miss).
        let Ok((fs, offs, units)) = self.arrays() else {
            return &[];
        };
        let Ok(i) = fs.binary_search(&f) else {
            return &[];
        };
        let (start, end) = (offs[i], offs[i + 1]);
        if start <= end && end <= self.n_entries as u64 {
            &units[start as usize..end as usize]
        } else {
            &[]
        }
    }
}
