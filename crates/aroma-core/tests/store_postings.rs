//! T08: `postings.bin` CSR layout, mmap reader validation, `manifest.json`.

mod writer {
    use aroma_core::store::postings::{header_check, write_postings, ENDIAN_MARKER, MAGIC};
    use aroma_core::store::StoreError;

    fn u64_at(b: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
    }

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn exact_layout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("postings.bin");
        // Unsorted, with one duplicate: (9, 1) twice.
        let entries = vec![(9, 1), (5, 1), (9, 1), (5, 0), (9, 0)];
        write_postings(&path, entries, 3).unwrap();
        let b = std::fs::read(&path).unwrap();
        // 48 header + 2 features * 8 + 3 offsets * 8 + 4 entries * 4
        assert_eq!(b.len(), 48 + 16 + 24 + 16);
        assert_eq!(&b[0..8], MAGIC);
        assert_eq!(u32_at(&b, 8), 1);
        assert_eq!(u32_at(&b, 12), ENDIAN_MARKER);
        assert_eq!(&b[12..16], &[0x04, 0x03, 0x02, 0x01]);
        assert_eq!(u64_at(&b, 16), 2); // n_features
        assert_eq!(u64_at(&b, 24), 4); // n_entries after dedup
        assert_eq!(u32_at(&b, 32), 3); // n_units
        assert_eq!(u32_at(&b, 36), 0); // reserved
        assert_eq!(u64_at(&b, 40), header_check(&b[0..40]));
        assert_eq!([u64_at(&b, 48), u64_at(&b, 56)], [5, 9]);
        assert_eq!([u64_at(&b, 64), u64_at(&b, 72), u64_at(&b, 80)], [0, 2, 4]);
        let units: Vec<u32> = (0..4).map(|i| u32_at(&b, 88 + 4 * i)).collect();
        assert_eq!(units, vec![0, 1, 0, 1]);
        assert!(!dir.path().join("postings.bin.tmp").exists());
    }

    #[test]
    fn empty_index_has_one_offset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("postings.bin");
        write_postings(&path, Vec::new(), 0).unwrap();
        let b = std::fs::read(&path).unwrap();
        assert_eq!(b.len(), 56);
        assert_eq!(u64_at(&b, 16), 0);
        assert_eq!(u64_at(&b, 24), 0);
        assert_eq!(u64_at(&b, 48), 0);
    }

    #[test]
    fn header_check_is_blake3_prefix() {
        let h = blake3::hash(b"0123456789012345678901234567890123456789");
        let want = u64::from_le_bytes(h.as_bytes()[..8].try_into().unwrap());
        assert_eq!(
            header_check(b"0123456789012345678901234567890123456789"),
            want
        );
    }

    #[test]
    fn rejects_unit_id_not_below_n_units() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("postings.bin");
        let err = write_postings(&path, vec![(1, 0), (1, 2)], 2).unwrap_err();
        match err {
            StoreError::Io(e) => {
                assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
                assert!(e.to_string().contains("unit id 2"), "{e}");
            }
            other => panic!("expected Io(InvalidInput), got {other:?}"),
        }
        assert!(!path.exists());
    }

    #[test]
    fn overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("postings.bin");
        write_postings(&path, vec![(1, 0), (2, 0), (3, 0)], 1).unwrap();
        write_postings(&path, vec![(7, 0)], 1).unwrap();
        assert_eq!(std::fs::read(&path).unwrap().len(), 48 + 8 + 16 + 4);
    }
}

mod reader {
    use aroma_core::index::{MemIndex, PostingSource};
    use aroma_core::multiset::Bag;
    use aroma_core::store::postings::{header_check, write_postings, MmapPostings};
    use aroma_core::store::StoreError;
    use proptest::prelude::*;
    use std::path::{Path, PathBuf};

    fn written(entries: Vec<(u64, u32)>, n_units: u32) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("postings.bin");
        write_postings(&path, entries, n_units).unwrap();
        (dir, path)
    }

    /// Features 5 -> [0, 1], 9 -> [0, 1, 2] over 3 units (file length 48+16+24+20 = 108).
    fn sample() -> (tempfile::TempDir, PathBuf) {
        written(vec![(5, 0), (5, 1), (9, 0), (9, 1), (9, 2)], 3)
    }

    fn patch(path: &Path, at: usize, bytes: &[u8]) {
        let mut b = std::fs::read(path).unwrap();
        b[at..at + bytes.len()].copy_from_slice(bytes);
        std::fs::write(path, b).unwrap();
    }

    fn reseal_header(path: &Path) {
        let mut b = std::fs::read(path).unwrap();
        let check = header_check(&b[0..40]);
        b[40..48].copy_from_slice(&check.to_le_bytes());
        std::fs::write(path, b).unwrap();
    }

    fn corrupt_reason(path: &Path) -> String {
        match MmapPostings::open(path) {
            Err(StoreError::Corrupt { file, reason }) => {
                assert_eq!(file, "postings.bin");
                reason
            }
            Err(other) => panic!("expected Corrupt, got {other:?}"),
            Ok(_) => panic!("expected Corrupt, got Ok"),
        }
    }

    /// `open` succeeds (it is O(n_features)); the per-list check is `verify`'s.
    fn verify_reason(path: &Path) -> String {
        match MmapPostings::open(path).unwrap().verify().unwrap_err() {
            StoreError::Corrupt { file, reason } => {
                assert_eq!(file, "postings.bin");
                reason
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn lookups() {
        let (_d, path) = sample();
        let p = MmapPostings::open(&path).unwrap();
        assert_eq!(p.n_units(), 3);
        assert_eq!(p.n_features(), 2);
        assert_eq!(p.n_entries(), 5);
        assert_eq!(p.postings(5), &[0, 1]);
        assert_eq!(p.postings(9), &[0, 1, 2]);
        p.verify().unwrap();
    }

    #[test]
    fn feature_lookup_miss_returns_empty_slice() {
        let (_d, path) = sample();
        let p = MmapPostings::open(&path).unwrap();
        for f in [0, 4, 6, 8, 10, u64::MAX] {
            assert!(p.postings(f).is_empty(), "feature {f}");
        }
    }

    #[test]
    fn empty_index_opens() {
        let (_d, path) = written(Vec::new(), 0);
        let p = MmapPostings::open(&path).unwrap();
        assert_eq!(p.n_units(), 0);
        assert_eq!(p.n_features(), 0);
        assert!(p.postings(1).is_empty());
        p.verify().unwrap();
    }

    #[test]
    fn missing_file_is_io_not_found() {
        let dir = tempfile::tempdir().unwrap();
        match MmapPostings::open(&dir.path().join("postings.bin")) {
            Err(StoreError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            Err(other) => panic!("expected Io, got {other:?}"),
            Ok(_) => panic!("expected Io, got Ok"),
        }
    }

    #[test]
    fn truncated_header_is_corrupt() {
        let (_d, path) = sample();
        let b = std::fs::read(&path).unwrap();
        std::fs::write(&path, &b[..20]).unwrap();
        assert_eq!(
            corrupt_reason(&path),
            "file is 20 bytes, shorter than the 48-byte header"
        );
    }

    #[test]
    fn truncated_body_is_corrupt() {
        let (_d, path) = sample();
        let b = std::fs::read(&path).unwrap();
        std::fs::write(&path, &b[..b.len() - 4]).unwrap();
        assert_eq!(
            corrupt_reason(&path),
            "file is 104 bytes, header implies 108"
        );
    }

    #[test]
    fn trailing_bytes_are_corrupt() {
        let (_d, path) = sample();
        let mut b = std::fs::read(&path).unwrap();
        b.extend_from_slice(&[0; 8]);
        std::fs::write(&path, b).unwrap();
        assert_eq!(
            corrupt_reason(&path),
            "file is 116 bytes, header implies 108"
        );
    }

    #[test]
    fn wrong_magic_is_corrupt() {
        let (_d, path) = sample();
        patch(&path, 0, b"AROMAXXX");
        assert_eq!(corrupt_reason(&path), "bad magic");
    }

    #[test]
    fn wrong_version_is_corrupt() {
        let (_d, path) = sample();
        patch(&path, 8, &2u32.to_le_bytes());
        assert_eq!(corrupt_reason(&path), "unsupported version 2 (expected 1)");
    }

    #[test]
    fn flipped_endian_marker_is_corrupt() {
        let (_d, path) = sample();
        patch(&path, 12, &0x0102_0304u32.to_be_bytes());
        assert_eq!(
            corrupt_reason(&path),
            "endian marker 0x04030201 (expected 0x01020304)"
        );
    }

    #[test]
    fn header_edit_without_reseal_fails_check() {
        let (_d, path) = sample();
        patch(&path, 32, &2u32.to_le_bytes()); // n_units 3 -> 2
        assert_eq!(corrupt_reason(&path), "header check mismatch");
    }

    #[test]
    fn counts_disagreeing_with_length_are_corrupt() {
        let (_d, path) = sample();
        patch(&path, 24, &6u64.to_le_bytes()); // n_entries 5 -> 6
        reseal_header(&path);
        assert_eq!(
            corrupt_reason(&path),
            "file is 108 bytes, header implies 112"
        );
    }

    #[test]
    fn open_is_constant_time_and_lookups_survive_corrupt_arrays() {
        // `open` checks only the header and sizes (one-shot queries must not scan the
        // features/offsets arrays); lookups stay panic-free on corrupt arrays.
        let (_d, path) = sample();
        patch(&path, 72, &6u64.to_le_bytes()); // offset past n_entries
        patch(&path, 48, &9u64.to_le_bytes()); // unsorted features
        let p = MmapPostings::open(&path).unwrap();
        for f in 0..20u64 {
            let _ = p.postings(f);
        }
        assert!(p.verify().is_err());
    }

    #[test]
    fn unsorted_features_fail_verify() {
        let (_d, path) = sample();
        patch(&path, 48, &9u64.to_le_bytes()); // features [9, 9]
        assert_eq!(
            verify_reason(&path),
            "feature ids not strictly increasing at index 1"
        );
    }

    #[test]
    fn offsets_out_of_range_fail_verify() {
        let (_d, path) = sample();
        // offsets live at 64, 72, 80 = [0, 2, 5]; push the middle one past n_entries.
        patch(&path, 72, &6u64.to_le_bytes());
        assert_eq!(verify_reason(&path), "offset 1 = 6 exceeds n_entries 5");
    }

    #[test]
    fn decreasing_offsets_fail_verify() {
        let (_d, path) = sample();
        patch(&path, 64, &3u64.to_le_bytes()); // [3, 2, 5]
        assert_eq!(verify_reason(&path), "offset 0 = 3 (expected 0)");
        let (_d3, path3) = written(vec![(1, 0), (2, 0), (3, 0)], 1);
        // offsets at 72, 80, 88, 96 = [0, 1, 2, 3]; make them [0, 2, 1, 3].
        patch(&path3, 80, &2u64.to_le_bytes());
        patch(&path3, 88, &1u64.to_le_bytes());
        assert_eq!(verify_reason(&path3), "offsets decrease at index 2");
    }

    #[test]
    fn last_offset_must_equal_n_entries_on_verify() {
        let (_d, path) = sample();
        patch(&path, 80, &4u64.to_le_bytes()); // [0, 2, 4] with n_entries 5
        assert_eq!(verify_reason(&path), "last offset 4 (expected n_entries 5)");
    }

    #[test]
    fn unit_id_out_of_range_fails_verify() {
        let (_d, path) = sample();
        // unit ids start at 88: [0, 1 | 0, 1, 2]; set the last one to 3 (= n_units).
        patch(&path, 88 + 16, &3u32.to_le_bytes());
        assert_eq!(
            verify_reason(&path),
            "unit id 3 of feature index 1 is >= n_units 3"
        );
    }

    #[test]
    fn unsorted_unit_ids_fail_verify() {
        let (_d, path) = sample();
        patch(&path, 88, &1u32.to_le_bytes()); // feature 5 -> [1, 1]
        assert_eq!(
            verify_reason(&path),
            "unit ids of feature index 0 not strictly increasing"
        );
    }

    #[test]
    fn every_single_byte_truncation_is_rejected() {
        let (_d, path) = sample();
        let full = std::fs::read(&path).unwrap();
        for len in 0..full.len() {
            std::fs::write(&path, &full[..len]).unwrap();
            assert!(MmapPostings::open(&path).is_err(), "length {len}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        #[test]
        fn round_trip_matches_mem_index(
            units in prop::collection::vec(prop::collection::vec(0u64..40, 0..12), 0..30),
            probes in prop::collection::vec(any::<u64>(), 0..8),
        ) {
            let mut mem = MemIndex::default();
            let mut entries = Vec::new();
            for (i, ids) in units.iter().enumerate() {
                let bag = Bag::from_ids(ids.iter().map(|&f| f.wrapping_mul(0x9e37_79b9_7f4a_7c15)));
                mem.insert(i as u32, &bag);
                for f in bag.distinct() {
                    entries.push((f, i as u32));
                    entries.push((f, i as u32)); // duplicates are deduped by the writer
                }
            }
            entries.reverse(); // and order does not matter
            let (_d, path) = written(entries, units.len() as u32);
            let p = MmapPostings::open(&path).unwrap();
            prop_assert!(p.verify().is_ok());
            prop_assert_eq!(p.n_units(), mem.n_units());
            for f in (0u64..40).map(|f| f.wrapping_mul(0x9e37_79b9_7f4a_7c15)).chain(probes) {
                prop_assert_eq!(p.postings(f), mem.postings(f));
            }
        }
    }
}

mod manifest {
    use aroma_core::profile::ProfileName;
    use aroma_core::store::manifest::{Manifest, INDEX_SCHEMA_VERSION};
    use aroma_core::store::StoreError;
    use std::collections::BTreeMap;

    fn sample() -> Manifest {
        Manifest {
            schema_version: INDEX_SCHEMA_VERSION,
            feature_schema: 1,
            feature_fingerprint: 0x0123_4567_89ab_cdef,
            profile: ProfileName::Reference,
            compat: false,
            sfmeta_mode: Default::default(),
            adapters: BTreeMap::from([("java".to_string(), "java-1".to_string())]),
            grammars: BTreeMap::from([("java".to_string(), "0.23.5".to_string())]),
            generation: 3,
            n_units: 120,
            n_files: 9,
        }
    }

    fn incompatible(have: &Manifest, want: &Manifest) -> String {
        match have.check_compatible(want) {
            Err(StoreError::Incompatible(msg)) => msg,
            other => panic!("expected Incompatible, got {other:?}"),
        }
    }

    #[test]
    fn schema_version_is_three() {
        assert_eq!(INDEX_SCHEMA_VERSION, 3);
    }

    #[test]
    fn round_trips_through_manifest_json() {
        let dir = tempfile::tempdir().unwrap();
        let m = sample();
        m.write(dir.path()).unwrap();
        assert_eq!(Manifest::read(dir.path()).unwrap(), m);
        assert!(!dir.path().join("manifest.json.tmp").exists());
    }

    #[test]
    fn json_field_names_and_values() {
        let dir = tempfile::tempdir().unwrap();
        sample().write(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join("manifest.json")).unwrap();
        assert!(text.ends_with("}\n"));
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "schema_version": 3,
                "feature_schema": 1,
                "feature_fingerprint": 81985529216486895u64,
                "profile": "reference",
                "compat": false,
                "sfmeta_mode": "lexical",
                "adapters": { "java": "java-1" },
                "grammars": { "java": "0.23.5" },
                "generation": 3,
                "n_units": 120,
                "n_files": 9
            })
        );
    }

    #[test]
    fn missing_manifest_is_io_not_found() {
        let dir = tempfile::tempdir().unwrap();
        match Manifest::read(dir.path()) {
            Err(StoreError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected Io, got {other:?}"),
        }
    }

    #[test]
    fn malformed_manifest_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), "{\"schema_version\": 1").unwrap();
        match Manifest::read(dir.path()) {
            Err(StoreError::Corrupt { file, .. }) => assert_eq!(file, "manifest.json"),
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn compatible_ignores_generation_and_counts() {
        let have = sample();
        let want = Manifest {
            generation: 0,
            n_units: 0,
            n_files: 0,
            ..sample()
        };
        assert!(have.check_compatible(&want).is_ok());
        assert!(have.check_compatible(&have).is_ok());
    }

    #[test]
    fn schema_version_mismatch() {
        // A schema-1 index (before lexical.bin, T23) is rejected by this binary.
        let have = Manifest {
            schema_version: 1,
            ..sample()
        };
        assert_eq!(
            incompatible(&have, &sample()),
            "schema_version: index 1, expected 3"
        );
    }

    #[test]
    fn feature_schema_mismatch() {
        let want = Manifest {
            feature_schema: 2,
            ..sample()
        };
        assert_eq!(
            incompatible(&sample(), &want),
            "feature_schema: index 1, expected 2"
        );
    }

    #[test]
    fn feature_fingerprint_mismatch() {
        let want = Manifest {
            feature_fingerprint: 1,
            ..sample()
        };
        assert_eq!(
            incompatible(&sample(), &want),
            "feature_fingerprint: index 0x0123456789abcdef, expected 0x0000000000000001"
        );
    }

    #[test]
    fn profile_mismatch() {
        let want = Manifest {
            profile: ProfileName::Paper,
            ..sample()
        };
        assert_eq!(
            incompatible(&sample(), &want),
            "profile: index Reference, expected Paper"
        );
    }

    #[test]
    fn compat_mismatch() {
        let want = Manifest {
            compat: true,
            ..sample()
        };
        assert_eq!(
            incompatible(&sample(), &want),
            "compat: index false, expected true"
        );
    }

    #[test]
    fn adapters_mismatch() {
        let mut want = sample();
        want.adapters.insert("java".into(), "java-2".into());
        assert_eq!(
            incompatible(&sample(), &want),
            "adapters: index {\"java\": \"java-1\"}, expected {\"java\": \"java-2\"}"
        );
    }

    #[test]
    fn grammars_mismatch() {
        let mut want = sample();
        want.grammars.insert("apex".into(), "3.0.2".into());
        assert_eq!(
            incompatible(&sample(), &want),
            "grammars: index {\"java\": \"0.23.5\"}, expected {\"apex\": \"3.0.2\", \"java\": \"0.23.5\"}"
        );
    }

    #[test]
    fn all_mismatches_are_listed_in_field_order() {
        let have = Manifest {
            schema_version: 1,
            ..sample()
        };
        let want = Manifest {
            compat: true,
            generation: 99,
            ..sample()
        };
        assert_eq!(
            incompatible(&have, &want),
            "schema_version: index 1, expected 3; compat: index false, expected true"
        );
    }

    #[test]
    fn incompatible_error_message() {
        let want = Manifest {
            compat: true,
            ..sample()
        };
        let err = sample().check_compatible(&want).unwrap_err();
        assert_eq!(
            err.to_string(),
            "incompatible index (compat: index false, expected true); re-index required"
        );
    }
}
