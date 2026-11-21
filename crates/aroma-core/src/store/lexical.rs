//! `lexical`: mmapped CSR `lexical.bin`, the BM25 identifier index (T23, D30).
//!
//! Layout (every integer little-endian; every array starts on an 8-byte boundary
//! because the header is 56 bytes and the arrays before `doc_len` have 8-byte elements):
//!
//! ```text
//! offset  size  field
//!      0     8  magic            b"AROMALEX"
//!      8     4  version          u32 = 1
//!     12     4  endian marker    u32 = 0x0102_0304
//!     16     8  n_terms          u64
//!     24     8  n_entries        u64
//!     32     4  n_units          u32 (every stored unit id is < n_units)
//!     36     4  reserved         u32 = 0
//!     40     8  avgdl            f64 bits = sum(doc_len) / n_units (0 when no units)
//!     48     8  header check     u64 = first 8 bytes (LE) of blake3(bytes 0..48)
//!     56        terms            [u64; n_terms]         strictly increasing term ids
//!      .        offsets          [u64; n_terms + 1]     offsets[0] = 0, non-decreasing,
//!                                                       offsets[n_terms] = n_entries
//!      .        entries          [(unit u32, tf u32); n_entries]
//!                                                       unit strictly increasing per
//!                                                       term, tf >= 1
//!      .        doc_len          [u32; n_units]         sum of the unit's tf
//! ```
//!
//! The postings of `terms[i]` are `entries[offsets[i]..offsets[i + 1]]`. The file length
//! must equal `56 + 8 * n_terms + 8 * (n_terms + 1) + 8 * n_entries + 4 * n_units`.
//! [`MmapLexical::open`] is O(n_terms); [`MmapLexical::verify`] scans everything.

use crate::lexical::{mean_doc_len, LexicalSource, TermId};
use crate::store::postings::header_check;
use crate::store::StoreError;
use crate::unit::UnitId;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

pub const MAGIC: &[u8; 8] = b"AROMALEX";
pub const VERSION: u32 = 1;
pub const ENDIAN_MARKER: u32 = 0x0102_0304;
pub const HEADER_LEN: usize = 56;
const CHECKED: usize = 48;
const FILE: &str = "lexical.bin";

fn tmp_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

fn invalid(msg: String) -> StoreError {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, msg).into()
}

/// Write `lexical.bin` from `(term, unit, tf)` triples and the per-unit document
/// lengths (`n_units = doc_len.len()`). Triples are sorted, equal `(term, unit)` pairs
/// are summed and `tf == 0` triples dropped. Fails with `ErrorKind::InvalidInput` when a
/// unit id is `>= n_units` or the term frequencies do not sum to `sum(doc_len)`.
/// Written to `<path>.tmp`, fsynced and renamed over `path`.
pub fn write_lexical(
    path: &Path,
    mut entries: Vec<(TermId, UnitId, u32)>,
    doc_len: &[u32],
) -> Result<(), StoreError> {
    let n_units =
        u32::try_from(doc_len.len()).map_err(|_| invalid("more than u32::MAX units".into()))?;
    entries.retain(|e| e.2 > 0);
    entries.sort_unstable();
    entries.dedup_by(|b, a| {
        if (a.0, a.1) == (b.0, b.1) {
            a.2 += b.2;
            true
        } else {
            false
        }
    });
    if let Some(&(t, u, _)) = entries.iter().find(|e| e.1 >= n_units) {
        return Err(invalid(format!(
            "unit id {u} of term {t:#018x} is >= n_units {n_units}"
        )));
    }
    let tf_sum: u64 = entries.iter().map(|e| u64::from(e.2)).sum();
    let dl_sum: u64 = doc_len.iter().map(|&d| u64::from(d)).sum();
    if tf_sum != dl_sum {
        return Err(invalid(format!(
            "term frequencies sum to {tf_sum}, document lengths to {dl_sum}"
        )));
    }
    let mut terms: Vec<TermId> = Vec::new();
    let mut offsets: Vec<u64> = vec![0];
    for (i, &(t, _, _)) in entries.iter().enumerate() {
        if terms.last() != Some(&t) {
            if !terms.is_empty() {
                offsets.push(i as u64);
            }
            terms.push(t);
        }
    }
    if !terms.is_empty() {
        offsets.push(entries.len() as u64);
    }

    let mut header = [0u8; HEADER_LEN];
    header[0..8].copy_from_slice(MAGIC);
    header[8..12].copy_from_slice(&VERSION.to_le_bytes());
    header[12..16].copy_from_slice(&ENDIAN_MARKER.to_le_bytes());
    header[16..24].copy_from_slice(&(terms.len() as u64).to_le_bytes());
    header[24..32].copy_from_slice(&(entries.len() as u64).to_le_bytes());
    header[32..36].copy_from_slice(&n_units.to_le_bytes());
    header[36..40].copy_from_slice(&0u32.to_le_bytes());
    header[40..48].copy_from_slice(&mean_doc_len(doc_len).to_bits().to_le_bytes());
    let check = header_check(&header[0..CHECKED]);
    header[48..56].copy_from_slice(&check.to_le_bytes());

    let tmp = tmp_path(path);
    let mut w = BufWriter::new(File::create(&tmp)?);
    w.write_all(&header)?;
    for t in &terms {
        w.write_all(&t.to_le_bytes())?;
    }
    for o in &offsets {
        w.write_all(&o.to_le_bytes())?;
    }
    for &(_, u, tf) in &entries {
        w.write_all(&u.to_le_bytes())?;
        w.write_all(&tf.to_le_bytes())?;
    }
    for d in doc_len {
        w.write_all(&d.to_le_bytes())?;
    }
    let file = w.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&tmp, path)?;
    Ok(())
}

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

/// Read-only, memory-mapped `lexical.bin`. `open` checks the header, the length, the
/// term array and the offsets (O(n_terms)); entries and document lengths are checked
/// only by [`MmapLexical::verify`] (never on the query path). Lookups binary-search the
/// term array and return a borrowed slice of the map.
#[derive(Debug)]
pub struct MmapLexical {
    map: memmap2::Mmap,
    n_terms: usize,
    n_entries: usize,
    n_units: u32,
    avgdl: f64,
}

type Arrays<'a> = (&'a [u64], &'a [u64], &'a [[u32; 2]], &'a [u32]);

impl MmapLexical {
    /// Map `path` and validate header, length, terms and offsets in O(n_terms); any
    /// violated invariant is `StoreError::Corrupt`.
    pub fn open(path: &Path) -> Result<MmapLexical, StoreError> {
        if cfg!(target_endian = "big") {
            return Err(StoreError::Incompatible(
                "lexical.bin is mapped in place and needs a little-endian host".into(),
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
        // writer modifies or truncates a published lexical.bin while it is mapped.
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
        if le_u64(b, 48) != header_check(&b[0..CHECKED]) {
            return Err(corrupt("header check mismatch".into()));
        }
        let nt = le_u64(b, 16);
        let ne = le_u64(b, 24);
        let n_units = le_u32(b, 32);
        let avgdl = f64::from_bits(le_u64(b, 40));
        if !avgdl.is_finite() || avgdl < 0.0 {
            return Err(corrupt(format!("avgdl {avgdl} is not a finite length")));
        }
        let expected =
            (nt as u128) * 16 + 8 + (ne as u128) * 8 + u128::from(n_units) * 4 + HEADER_LEN as u128;
        if expected != len as u128 {
            return Err(corrupt(format!(
                "file is {len} bytes, header implies {expected}"
            )));
        }
        let l = MmapLexical {
            map,
            n_terms: nt as usize,
            n_entries: ne as usize,
            n_units,
            avgdl,
        };
        l.validate()?;
        Ok(l)
    }

    /// O(n_terms): the term and offset arrays only.
    fn validate(&self) -> Result<(), StoreError> {
        let (ts, offs, _, _) = self.arrays()?;
        if let Some(i) = ts.windows(2).position(|w| w[0] >= w[1]) {
            return Err(corrupt(format!(
                "term ids not strictly increasing at index {}",
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
        if offs[self.n_terms] != ne {
            return Err(corrupt(format!(
                "last offset {} (expected n_entries {ne})",
                offs[self.n_terms]
            )));
        }
        Ok(())
    }

    /// Full scan: every unit id `< n_units` and strictly increasing within its term,
    /// every tf `>= 1`, term frequencies summing to the document lengths, and `avgdl`
    /// equal to their mean. For `aroma inspect-index` / tests; never on the query path.
    pub fn verify(&self) -> Result<(), StoreError> {
        let (_, offs, entries, doc_len) = self.arrays()?;
        let mut tf_sum = 0u64;
        for (i, w) in offs.windows(2).enumerate() {
            let list = &entries[w[0] as usize..w[1] as usize];
            if let Some(&[u, _]) = list.iter().find(|p| p[0] >= self.n_units) {
                return Err(corrupt(format!(
                    "unit id {u} of term index {i} is >= n_units {}",
                    self.n_units
                )));
            }
            if list.windows(2).any(|p| p[0][0] >= p[1][0]) {
                return Err(corrupt(format!(
                    "unit ids of term index {i} not strictly increasing"
                )));
            }
            if list.iter().any(|p| p[1] == 0) {
                return Err(corrupt(format!("zero term frequency in term index {i}")));
            }
            tf_sum += list.iter().map(|p| u64::from(p[1])).sum::<u64>();
        }
        let dl_sum: u64 = doc_len.iter().map(|&d| u64::from(d)).sum();
        if tf_sum != dl_sum {
            return Err(corrupt(format!(
                "term frequencies sum to {tf_sum}, document lengths to {dl_sum}"
            )));
        }
        let mean = mean_doc_len(doc_len);
        if mean.to_bits() != self.avgdl.to_bits() {
            return Err(corrupt(format!(
                "avgdl {} differs from the mean document length {mean}",
                self.avgdl
            )));
        }
        Ok(())
    }

    /// Typed slices of the map. Alignment holds by construction (page-aligned map,
    /// 8-byte-aligned array starts); a failed cast is reported as corruption.
    fn arrays(&self) -> Result<Arrays<'_>, StoreError> {
        let b: &[u8] = &self.map;
        let t_end = HEADER_LEN + 8 * self.n_terms;
        let o_end = t_end + 8 * (self.n_terms + 1);
        let e_end = o_end + 8 * self.n_entries;
        let d_end = e_end + 4 * self.n_units as usize;
        let cast = |e: bytemuck::PodCastError| corrupt(format!("array cast failed: {e:?}"));
        Ok((
            bytemuck::try_cast_slice(&b[HEADER_LEN..t_end]).map_err(cast)?,
            bytemuck::try_cast_slice(&b[t_end..o_end]).map_err(cast)?,
            bytemuck::try_cast_slice(&b[o_end..e_end]).map_err(cast)?,
            bytemuck::try_cast_slice(&b[e_end..d_end]).map_err(cast)?,
        ))
    }

    pub fn n_terms(&self) -> u64 {
        self.n_terms as u64
    }

    pub fn n_entries(&self) -> u64 {
        self.n_entries as u64
    }
}

impl LexicalSource for MmapLexical {
    fn n_units(&self) -> u32 {
        self.n_units
    }

    fn avgdl(&self) -> f64 {
        self.avgdl
    }

    fn doc_len(&self, unit: UnitId) -> u32 {
        // Hot path (once per posting entry): a direct read instead of `arrays()`.
        if unit >= self.n_units {
            return 0;
        }
        let at = HEADER_LEN + 16 * self.n_terms + 8 + 8 * self.n_entries + 4 * unit as usize;
        le_u32(&self.map, at)
    }

    fn postings(&self, term: TermId) -> &[[u32; 2]] {
        // `open` proved the casts succeed and the offsets are in range; the bounds check
        // keeps this panic-free without the full `verify`.
        let Ok((ts, offs, entries, _)) = self.arrays() else {
            return &[];
        };
        let Ok(i) = ts.binary_search(&term) else {
            return &[];
        };
        let (start, end) = (offs[i], offs[i + 1]);
        if start <= end && end <= self.n_entries as u64 {
            &entries[start as usize..end as usize]
        } else {
            &[]
        }
    }
}
