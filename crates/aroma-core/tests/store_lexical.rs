//! T23: `lexical.bin` CSR layout, mmap reader validation (`open` vs `verify`), round trip.

use aroma_core::lexical::{lexical_search, LexicalSource, MemLexical};
use aroma_core::store::lexical::{write_lexical, MmapLexical, ENDIAN_MARKER, MAGIC};
use aroma_core::store::postings::header_check;
use aroma_core::store::StoreError;
use proptest::prelude::*;
use std::path::{Path, PathBuf};

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn written(entries: Vec<(u64, u32, u32)>, doc_len: &[u32]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lexical.bin");
    write_lexical(&path, entries, doc_len).unwrap();
    (dir, path)
}

/// Terms 5 -> [(0, 2), (1, 1)], 9 -> [(0, 1), (1, 1), (2, 3)] over 3 units with
/// doc_len [3, 2, 3]. Layout: header 0..56, terms 56..72, offsets 72..96,
/// entries 96..136, doc_len 136..148.
fn sample() -> (tempfile::TempDir, PathBuf) {
    written(
        vec![(9, 2, 3), (5, 0, 2), (9, 0, 1), (5, 1, 1), (9, 1, 1)],
        &[3, 2, 3],
    )
}

fn patch(path: &Path, at: usize, bytes: &[u8]) {
    let mut b = std::fs::read(path).unwrap();
    b[at..at + bytes.len()].copy_from_slice(bytes);
    std::fs::write(path, b).unwrap();
}

fn reseal_header(path: &Path) {
    let mut b = std::fs::read(path).unwrap();
    let check = header_check(&b[0..48]);
    b[48..56].copy_from_slice(&check.to_le_bytes());
    std::fs::write(path, b).unwrap();
}

fn corrupt_reason(path: &Path) -> String {
    match MmapLexical::open(path) {
        Err(StoreError::Corrupt { file, reason }) => {
            assert_eq!(file, "lexical.bin");
            reason
        }
        Err(other) => panic!("expected Corrupt, got {other:?}"),
        Ok(_) => panic!("expected Corrupt, got Ok"),
    }
}

fn verify_reason(path: &Path) -> String {
    match MmapLexical::open(path).unwrap().verify().unwrap_err() {
        StoreError::Corrupt { file, reason } => {
            assert_eq!(file, "lexical.bin");
            reason
        }
        other => panic!("expected Corrupt, got {other:?}"),
    }
}

mod writer {
    use super::*;

    #[test]
    fn exact_layout() {
        let (dir, path) = sample();
        let b = std::fs::read(&path).unwrap();
        assert_eq!(b.len(), 56 + 16 + 24 + 40 + 12);
        assert_eq!(&b[0..8], MAGIC);
        assert_eq!(u32_at(&b, 8), 1);
        assert_eq!(u32_at(&b, 12), ENDIAN_MARKER);
        assert_eq!(u64_at(&b, 16), 2); // n_terms
        assert_eq!(u64_at(&b, 24), 5); // n_entries
        assert_eq!(u32_at(&b, 32), 3); // n_units
        assert_eq!(u32_at(&b, 36), 0); // reserved
        assert_eq!(f64::from_bits(u64_at(&b, 40)), 8.0 / 3.0); // avgdl
        assert_eq!(u64_at(&b, 48), header_check(&b[0..48]));
        assert_eq!([u64_at(&b, 56), u64_at(&b, 64)], [5, 9]);
        assert_eq!([u64_at(&b, 72), u64_at(&b, 80), u64_at(&b, 88)], [0, 2, 5]);
        let entries: Vec<u32> = (0..10).map(|i| u32_at(&b, 96 + 4 * i)).collect();
        assert_eq!(entries, [0, 2, 1, 1, 0, 1, 1, 1, 2, 3]);
        let dl: Vec<u32> = (0..3).map(|i| u32_at(&b, 136 + 4 * i)).collect();
        assert_eq!(dl, [3, 2, 3]);
        assert!(!dir.path().join("lexical.bin.tmp").exists());
    }

    #[test]
    fn duplicates_are_summed_and_zero_tf_dropped() {
        let (_d, path) = written(vec![(4, 0, 1), (4, 0, 1), (6, 0, 0)], &[2]);
        let l = MmapLexical::open(&path).unwrap();
        assert_eq!(l.postings(4), &[[0, 2]]);
        assert!(l.postings(6).is_empty());
        l.verify().unwrap();
    }

    #[test]
    fn empty_index_is_header_plus_one_offset() {
        let (_d, path) = written(Vec::new(), &[]);
        assert_eq!(std::fs::read(&path).unwrap().len(), 64);
        let l = MmapLexical::open(&path).unwrap();
        assert_eq!((l.n_units(), l.n_terms(), l.avgdl()), (0, 0, 0.0));
        l.verify().unwrap();
    }

    #[test]
    fn rejects_unit_id_not_below_n_units_and_bad_lengths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lexical.bin");
        match write_lexical(&path, vec![(1, 2, 1)], &[1, 0]).unwrap_err() {
            StoreError::Io(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
                assert!(e.to_string().contains("unit id 2"), "{e}");
            }
            other => panic!("expected InvalidInput, got {other:?}"),
        }
        match write_lexical(&path, vec![(1, 0, 2)], &[1]).unwrap_err() {
            StoreError::Io(e) => assert!(e.to_string().contains("sum to 2"), "{e}"),
            other => panic!("expected InvalidInput, got {other:?}"),
        }
        assert!(!path.exists());
    }
}

mod reader {
    use super::*;

    #[test]
    fn lookups() {
        let (_d, path) = sample();
        let l = MmapLexical::open(&path).unwrap();
        assert_eq!((l.n_units(), l.n_terms(), l.n_entries()), (3, 2, 5));
        assert_eq!(l.avgdl(), 8.0 / 3.0);
        assert_eq!(l.postings(5), &[[0, 2], [1, 1]]);
        assert_eq!(l.postings(9), &[[0, 1], [1, 1], [2, 3]]);
        for t in [0, 4, 6, 10, u64::MAX] {
            assert!(l.postings(t).is_empty(), "term {t}");
        }
        assert_eq!(
            [l.doc_len(0), l.doc_len(1), l.doc_len(2), l.doc_len(3)],
            [3, 2, 3, 0]
        );
        l.verify().unwrap();
    }

    #[test]
    fn missing_file_is_io_not_found() {
        let dir = tempfile::tempdir().unwrap();
        match MmapLexical::open(&dir.path().join("lexical.bin")) {
            Err(StoreError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            Err(other) => panic!("expected Io, got {other:?}"),
            Ok(_) => panic!("expected Io, got Ok"),
        }
    }

    #[test]
    fn truncation_and_trailing_bytes_are_corrupt() {
        let (_d, path) = sample();
        let b = std::fs::read(&path).unwrap();
        std::fs::write(&path, &b[..30]).unwrap();
        assert_eq!(
            corrupt_reason(&path),
            "file is 30 bytes, shorter than the 56-byte header"
        );
        std::fs::write(&path, &b[..b.len() - 4]).unwrap();
        assert_eq!(
            corrupt_reason(&path),
            "file is 144 bytes, header implies 148"
        );
        let mut longer = b.clone();
        longer.extend_from_slice(&[0; 8]);
        std::fs::write(&path, longer).unwrap();
        assert_eq!(
            corrupt_reason(&path),
            "file is 156 bytes, header implies 148"
        );
    }

    #[test]
    fn header_fields_are_checked() {
        let (_d, path) = sample();
        patch(&path, 0, b"AROMAPST");
        assert_eq!(corrupt_reason(&path), "bad magic");
        let (_d, path) = sample();
        patch(&path, 8, &2u32.to_le_bytes());
        assert_eq!(corrupt_reason(&path), "unsupported version 2 (expected 1)");
        let (_d, path) = sample();
        patch(&path, 12, &ENDIAN_MARKER.to_be_bytes());
        assert_eq!(
            corrupt_reason(&path),
            "endian marker 0x04030201 (expected 0x01020304)"
        );
        let (_d, path) = sample();
        patch(&path, 40, &1.0f64.to_bits().to_le_bytes()); // avgdl edit, no reseal
        assert_eq!(corrupt_reason(&path), "header check mismatch");
        reseal_header(&path);
        MmapLexical::open(&path).unwrap(); // open does not recompute avgdl ...
        assert_eq!(
            verify_reason(&path), // ... verify does
            "avgdl 1 differs from the mean document length 2.6666666666666665"
        );
        let (_d, path) = sample();
        patch(&path, 40, &f64::NAN.to_bits().to_le_bytes());
        reseal_header(&path);
        assert_eq!(corrupt_reason(&path), "avgdl NaN is not a finite length");
    }

    #[test]
    fn counts_disagreeing_with_length_are_corrupt() {
        let (_d, path) = sample();
        patch(&path, 32, &4u32.to_le_bytes()); // n_units 3 -> 4
        reseal_header(&path);
        assert_eq!(
            corrupt_reason(&path),
            "file is 148 bytes, header implies 152"
        );
    }

    #[test]
    fn terms_and_offsets_are_checked_by_open() {
        let (_d, path) = sample();
        patch(&path, 56, &9u64.to_le_bytes()); // terms [9, 9]
        assert_eq!(
            corrupt_reason(&path),
            "term ids not strictly increasing at index 1"
        );
        let (_d, path) = sample();
        patch(&path, 80, &6u64.to_le_bytes()); // offsets [0, 6, 5]
        assert_eq!(corrupt_reason(&path), "offset 1 = 6 exceeds n_entries 5");
        let (_d, path) = sample();
        patch(&path, 72, &1u64.to_le_bytes()); // offsets [1, 2, 5]
        assert_eq!(corrupt_reason(&path), "offset 0 = 1 (expected 0)");
        let (_d, path) = sample();
        patch(&path, 80, &5u64.to_le_bytes());
        patch(&path, 88, &4u64.to_le_bytes()); // offsets [0, 5, 4]
        assert_eq!(corrupt_reason(&path), "offsets decrease at index 2");
        let (_d, path) = sample();
        patch(&path, 88, &4u64.to_le_bytes()); // offsets [0, 2, 4]
        assert_eq!(
            corrupt_reason(&path),
            "last offset 4 (expected n_entries 5)"
        );
    }

    #[test]
    fn entries_and_lengths_are_checked_by_verify() {
        let (_d, path) = sample();
        patch(&path, 96 + 32, &3u32.to_le_bytes()); // last entry unit 2 -> 3
        assert_eq!(
            verify_reason(&path),
            "unit id 3 of term index 1 is >= n_units 3"
        );
        let (_d, path) = sample();
        patch(&path, 96, &1u32.to_le_bytes()); // term 5 -> units [1, 1]
        assert_eq!(
            verify_reason(&path),
            "unit ids of term index 0 not strictly increasing"
        );
        let (_d, path) = sample();
        patch(&path, 100, &0u32.to_le_bytes()); // tf 2 -> 0
        assert_eq!(verify_reason(&path), "zero term frequency in term index 0");
        let (_d, path) = sample();
        patch(&path, 136, &4u32.to_le_bytes()); // doc_len[0] 3 -> 4
        assert_eq!(
            verify_reason(&path),
            "term frequencies sum to 8, document lengths to 9"
        );
    }

    #[test]
    fn every_single_byte_truncation_is_rejected() {
        let (_d, path) = sample();
        let full = std::fs::read(&path).unwrap();
        for len in 0..full.len() {
            std::fs::write(&path, &full[..len]).unwrap();
            assert!(MmapLexical::open(&path).is_err(), "length {len}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn round_trip_matches_mem_lexical(
            units in prop::collection::vec(
                prop::collection::btree_map(0u64..30, 1u32..5, 0..10), 0..25),
            query in prop::collection::vec(0u64..35, 0..6),
        ) {
            let spread = |t: u64| t.wrapping_mul(0x9e37_79b9_7f4a_7c15);
            let mut mem = MemLexical::default();
            let mut entries = Vec::new();
            let mut doc_len = Vec::new();
            for (i, tf) in units.iter().enumerate() {
                let mut terms: Vec<(u64, u32)> = tf.iter().map(|(&t, &n)| (spread(t), n)).collect();
                terms.sort_unstable();
                mem.insert(i as u32, &terms);
                entries.extend(terms.iter().map(|&(t, n)| (t, i as u32, n)));
                doc_len.push(terms.iter().map(|&(_, n)| n).sum::<u32>());
            }
            entries.reverse(); // order does not matter
            let (_d, path) = written(entries, &doc_len);
            let l = MmapLexical::open(&path).unwrap();
            prop_assert!(l.verify().is_ok());
            prop_assert_eq!(l.n_units(), mem.n_units());
            prop_assert_eq!(l.avgdl().to_bits(), mem.avgdl().to_bits());
            for t in (0u64..35).map(spread) {
                prop_assert_eq!(l.postings(t), mem.postings(t));
            }
            for u in 0..=units.len() as u32 {
                prop_assert_eq!(l.doc_len(u), mem.doc_len(u));
            }
            let mut q: Vec<u64> = query.into_iter().map(spread).collect();
            q.sort_unstable();
            q.dedup();
            let a = lexical_search(&l, &q, 7, &|_| true).hits;
            let b = lexical_search(&mem, &q, 7, &|_| true).hits;
            prop_assert_eq!(a, b);
        }
    }
}
