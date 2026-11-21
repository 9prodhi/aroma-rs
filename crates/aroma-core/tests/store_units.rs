//! T09: `units.bin` + `files.json` unit table and `spt.bin` SPT store.

mod unit_table {
    use aroma_core::lang::Lang;
    use aroma_core::store::unit_table::{write_unit_table, UnitTable};
    use aroma_core::store::StoreError;
    use aroma_core::unit::{UnitKind, UnitRecord};
    use std::path::{Path, PathBuf};

    const KINDS: [UnitKind; 6] = [
        UnitKind::Method,
        UnitKind::Constructor,
        UnitKind::Trigger,
        UnitKind::Accessor,
        UnitKind::Initializer,
        UnitKind::Snippet,
    ];

    fn rec(id: u32, lang: Lang, kind: UnitKind, file: u32) -> UnitRecord {
        UnitRecord {
            id,
            lang,
            kind,
            file,
            bytes: 100 * id..100 * id + 57,
            lines: 3 * id + 1..3 * id + 4,
            n_leaves: 10 + id,
            body_hash: 0x1111_2222_3333_4444 ^ u64::from(id),
            is_test: id % 2 == 1,
        }
    }

    fn files() -> Vec<PathBuf> {
        vec![
            PathBuf::from("src/A.java"),
            PathBuf::from("force-app/b/B.cls"),
        ]
    }

    fn sample(dir: &Path) -> Vec<UnitRecord> {
        let records = vec![
            rec(0, Lang::Java, UnitKind::Method, 0),
            rec(1, Lang::Apex, UnitKind::Trigger, 1),
            rec(2, Lang::Apex, UnitKind::Accessor, 1),
        ];
        write_unit_table(dir, &records, &files()).unwrap();
        records
    }

    fn patch(path: &Path, at: usize, bytes: &[u8]) {
        let mut b = std::fs::read(path).unwrap();
        b[at..at + bytes.len()].copy_from_slice(bytes);
        std::fs::write(path, b).unwrap();
    }

    fn corrupt(dir: &Path) -> (&'static str, String) {
        match UnitTable::open(dir) {
            Err(StoreError::Corrupt { file, reason }) => (file, reason),
            Err(other) => panic!("expected Corrupt, got {other:?}"),
            Ok(_) => panic!("expected Corrupt, got Ok"),
        }
    }

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let records = sample(dir.path());
        let t = UnitTable::open(dir.path()).unwrap();
        assert_eq!(t.n_units(), 3);
        assert_eq!(t.n_files(), 2);
        for r in &records {
            assert_eq!(&t.record(r.id), r);
        }
        assert_eq!(t.file_path(0), Path::new("src/A.java"));
        assert_eq!(t.file_path(1), Path::new("force-app/b/B.cls"));
    }

    #[test]
    fn every_lang_and_kind_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let mut records = Vec::new();
        for lang in [Lang::Java, Lang::Apex] {
            for kind in KINDS {
                records.push(rec(records.len() as u32, lang, kind, 0));
            }
        }
        write_unit_table(dir.path(), &records, &files()).unwrap();
        let t = UnitTable::open(dir.path()).unwrap();
        for r in &records {
            assert_eq!(&t.record(r.id), r);
        }
    }

    #[test]
    fn new_language_and_unit_codes_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut records = Vec::new();
        for lang in Lang::ALL {
            for kind in [
                UnitKind::Template,
                UnitKind::MarkupBlock,
                UnitKind::WireMember,
                UnitKind::FlowElement,
                UnitKind::Field,
                UnitKind::ValidationRule,
                UnitKind::MetadataRecord,
                UnitKind::Page,
                UnitKind::Component,
            ] {
                records.push(rec(records.len() as u32, lang, kind, 0));
            }
        }
        write_unit_table(dir.path(), &records, &files()).unwrap();
        let table = UnitTable::open(dir.path()).unwrap();
        for record in records {
            assert_eq!(table.record(record.id), record);
        }
    }

    #[test]
    fn exact_layout() {
        let dir = tempfile::tempdir().unwrap();
        let r = UnitRecord {
            id: 0,
            lang: Lang::Apex,
            kind: UnitKind::Trigger,
            file: 1,
            bytes: 10..90,
            lines: 2..9,
            n_leaves: 41,
            body_hash: 0x0102_0304_0506_0708,
            is_test: true,
        };
        write_unit_table(dir.path(), &[r], &files()).unwrap();
        let b = std::fs::read(dir.path().join("units.bin")).unwrap();
        assert_eq!(b.len(), 16 + 48);
        assert_eq!(&b[0..8], b"AROMAUNT");
        assert_eq!(u32_at(&b, 8), 1);
        assert_eq!(u32_at(&b, 12), 1);
        assert_eq!(u32_at(&b, 16), 0); // id
        assert_eq!(&b[20..24], &[1, 2, 1, 0]); // lang apex, kind trigger, is_test, pad
        let words: Vec<u32> = (0..6).map(|i| u32_at(&b, 24 + 4 * i)).collect();
        assert_eq!(words, vec![1, 10, 90, 2, 9, 41]); // file, bytes, lines, n_leaves
        assert_eq!(&b[48..56], &[8, 7, 6, 5, 4, 3, 2, 1]); // body_hash LE
        assert_eq!(&b[56..64], &[0; 8]); // reserved
    }

    #[test]
    fn files_json_is_ordered_path_list() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        let text = std::fs::read_to_string(dir.path().join("files.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v, serde_json::json!(["src/A.java", "force-app/b/B.cls"]));
    }

    #[test]
    fn empty_table() {
        let dir = tempfile::tempdir().unwrap();
        write_unit_table(dir.path(), &[], &[]).unwrap();
        let t = UnitTable::open(dir.path()).unwrap();
        assert_eq!(t.n_units(), 0);
        assert_eq!(t.n_files(), 0);
        assert_eq!(
            std::fs::read(dir.path().join("units.bin")).unwrap().len(),
            16
        );
    }

    #[test]
    #[should_panic]
    fn record_out_of_range_panics() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        let t = UnitTable::open(dir.path()).unwrap();
        let _ = t.record(3);
    }

    #[test]
    fn writer_rejects_id_not_equal_to_position() {
        let dir = tempfile::tempdir().unwrap();
        let records = vec![rec(1, Lang::Java, UnitKind::Method, 0)];
        match write_unit_table(dir.path(), &records, &files()) {
            Err(StoreError::Io(e)) => {
                assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
                assert_eq!(e.to_string(), "record 0 has id 1");
            }
            other => panic!("expected Io(InvalidInput), got {other:?}"),
        }
    }

    #[test]
    fn writer_rejects_unknown_file() {
        let dir = tempfile::tempdir().unwrap();
        let records = vec![rec(0, Lang::Java, UnitKind::Method, 2)];
        match write_unit_table(dir.path(), &records, &files()) {
            Err(StoreError::Io(e)) => assert_eq!(e.to_string(), "record 0 names file 2 of 2"),
            other => panic!("expected Io(InvalidInput), got {other:?}"),
        }
    }

    #[test]
    fn missing_units_bin_is_io_not_found() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        std::fs::remove_file(dir.path().join("units.bin")).unwrap();
        match UnitTable::open(dir.path()) {
            Err(StoreError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            Err(other) => panic!("expected Io, got {other:?}"),
            Ok(_) => panic!("expected Io, got Ok"),
        }
    }

    #[test]
    fn malformed_files_json_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        std::fs::write(dir.path().join("files.json"), "{\"a\": 1}").unwrap();
        assert_eq!(corrupt(dir.path()).0, "files.json");
    }

    #[test]
    fn truncated_header_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        std::fs::write(dir.path().join("units.bin"), b"AROMAUNT").unwrap();
        assert_eq!(
            corrupt(dir.path()),
            (
                "units.bin",
                "file is 8 bytes, shorter than the 16-byte header".to_string()
            )
        );
    }

    #[test]
    fn wrong_magic_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        patch(&dir.path().join("units.bin"), 0, b"AROMAXXX");
        assert_eq!(corrupt(dir.path()), ("units.bin", "bad magic".to_string()));
    }

    #[test]
    fn wrong_version_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        patch(&dir.path().join("units.bin"), 8, &9u32.to_le_bytes());
        assert_eq!(corrupt(dir.path()).1, "unsupported version 9 (expected 1)");
    }

    #[test]
    fn length_mismatch_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        let path = dir.path().join("units.bin");
        let b = std::fs::read(&path).unwrap();
        std::fs::write(&path, &b[..b.len() - 1]).unwrap();
        assert_eq!(
            corrupt(dir.path()).1,
            "file is 159 bytes, header implies 160"
        );
    }

    /// Record 1 starts at byte 16 + 48 = 64.
    const R1: usize = 64;

    #[test]
    fn id_mismatch_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        patch(&dir.path().join("units.bin"), R1, &7u32.to_le_bytes());
        assert_eq!(corrupt(dir.path()).1, "record 1: id 7");
    }

    #[test]
    fn bad_codes_are_corrupt() {
        for (at, byte, reason) in [
            (R1 + 4, 8u8, "record 1: lang code 8"),
            (R1 + 5, 15u8, "record 1: kind code 15"),
            (R1 + 6, 2u8, "record 1: is_test byte 2"),
            (R1 + 7, 1u8, "record 1: non-zero padding"),
            (R1 + 40, 1u8, "record 1: non-zero padding"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            sample(dir.path());
            patch(&dir.path().join("units.bin"), at, &[byte]);
            assert_eq!(corrupt(dir.path()).1, reason);
        }
    }

    #[test]
    fn reversed_ranges_are_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        // record 1: bytes 100..157 -> 200..157
        patch(
            &dir.path().join("units.bin"),
            R1 + 12,
            &200u32.to_le_bytes(),
        );
        assert_eq!(corrupt(dir.path()).1, "record 1: bytes 200..157");
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        // record 1: lines 4..7 -> 8..7
        patch(&dir.path().join("units.bin"), R1 + 20, &8u32.to_le_bytes());
        assert_eq!(corrupt(dir.path()).1, "record 1: lines 8..7");
    }

    #[test]
    fn file_index_beyond_files_json_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        sample(dir.path());
        std::fs::write(dir.path().join("files.json"), "[\"src/A.java\"]").unwrap();
        assert_eq!(
            corrupt(dir.path()),
            ("units.bin", "record 1: file 1 of 1".to_string())
        );
    }
}

mod spt_store {
    use aroma_core::spt::{Elem, Leaf, Node, Spt, SymbolId};
    use aroma_core::store::spt_store::{write_spt_store, SptStore};
    use aroma_core::store::StoreError;
    use std::path::{Path, PathBuf};

    fn kw(s: &str) -> Elem {
        Elem::Keyword(s.into())
    }

    /// `if (x) return y;` with `x` a bound local.
    fn if_return() -> Spt {
        Spt {
            nodes: vec![
                Node {
                    label: "if(#)#".into(),
                    children: vec![kw("if"), kw("("), Elem::Leaf(0), kw(")"), Elem::Tree(1)],
                },
                Node {
                    label: "return#;".into(),
                    children: vec![kw("return"), Elem::Leaf(1), kw(";")],
                },
            ],
            leaves: vec![
                Leaf {
                    text: "x".into(),
                    is_var: true,
                    binding: Some(SymbolId(0)),
                    bytes: 4..5,
                    line: 1,
                },
                Leaf {
                    text: "y".into(),
                    is_var: false,
                    binding: None,
                    bytes: 14..15,
                    line: 2,
                },
            ],
            root: 0,
        }
    }

    /// `foo();`
    fn call() -> Spt {
        Spt {
            nodes: vec![Node {
                label: "#();".into(),
                children: vec![Elem::Leaf(0), kw("("), kw(")"), kw(";")],
            }],
            leaves: vec![Leaf {
                text: "foo".into(),
                is_var: false,
                binding: None,
                bytes: 0..3,
                line: 7,
            }],
            root: 0,
        }
    }

    fn written(spts: &[Spt]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spt.bin");
        write_spt_store(&path, spts).unwrap();
        (dir, path)
    }

    fn patch(path: &Path, at: usize, bytes: &[u8]) {
        let mut b = std::fs::read(path).unwrap();
        b[at..at + bytes.len()].copy_from_slice(bytes);
        std::fs::write(path, b).unwrap();
    }

    /// A hand-built spt.bin holding `blobs` verbatim.
    fn raw_file(path: &Path, blobs: &[Vec<u8>]) {
        let mut b = b"AROMASPT".to_vec();
        b.extend_from_slice(&1u32.to_le_bytes());
        b.extend_from_slice(&(blobs.len() as u32).to_le_bytes());
        let mut off = 0u64;
        b.extend_from_slice(&off.to_le_bytes());
        for blob in blobs {
            off += blob.len() as u64;
            b.extend_from_slice(&off.to_le_bytes());
        }
        for blob in blobs {
            b.extend_from_slice(blob);
        }
        std::fs::write(path, b).unwrap();
    }

    fn open_corrupt(path: &Path) -> String {
        match SptStore::open(path) {
            Err(StoreError::Corrupt { file, reason }) => {
                assert_eq!(file, "spt.bin");
                reason
            }
            Err(other) => panic!("expected Corrupt, got {other:?}"),
            Ok(_) => panic!("expected Corrupt, got Ok"),
        }
    }

    fn get_corrupt(store: &SptStore, id: u32) -> String {
        match store.get(id) {
            Err(StoreError::Corrupt { file, reason }) => {
                assert_eq!(file, "spt.bin");
                reason
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn round_trip() {
        let spts = vec![if_return(), call(), if_return()];
        let (_d, path) = written(&spts);
        let s = SptStore::open(&path).unwrap();
        assert_eq!(s.len(), 3);
        assert!(!s.is_empty());
        for (i, t) in spts.iter().enumerate() {
            assert_eq!(&s.get(i as u32).unwrap(), t);
        }
        assert!(!path.with_file_name("spt.bin.tmp").exists());
    }

    #[test]
    fn exact_layout() {
        let spts = vec![if_return(), call()];
        let (_d, path) = written(&spts);
        let b = std::fs::read(&path).unwrap();
        let blob0 = postcard::to_stdvec(&spts[0]).unwrap();
        let blob1 = postcard::to_stdvec(&spts[1]).unwrap();
        assert_eq!(&b[0..8], b"AROMASPT");
        assert_eq!(&b[8..12], &1u32.to_le_bytes());
        assert_eq!(&b[12..16], &2u32.to_le_bytes());
        let off = |i: usize| u64::from_le_bytes(b[16 + 8 * i..24 + 8 * i].try_into().unwrap());
        assert_eq!(
            [off(0), off(1), off(2)],
            [0, blob0.len() as u64, (blob0.len() + blob1.len()) as u64]
        );
        assert_eq!(&b[40..40 + blob0.len()], &blob0[..]);
        assert_eq!(&b[40 + blob0.len()..], &blob1[..]);
    }

    #[test]
    fn empty_store() {
        let (_d, path) = written(&[]);
        let s = SptStore::open(&path).unwrap();
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert_eq!(std::fs::read(&path).unwrap().len(), 24);
        assert_eq!(get_corrupt(&s, 0), "unit id 0 out of range (len 0)");
    }

    #[test]
    fn get_out_of_range_is_corrupt() {
        let (_d, path) = written(&[call()]);
        let s = SptStore::open(&path).unwrap();
        assert_eq!(get_corrupt(&s, 1), "unit id 1 out of range (len 1)");
    }

    #[test]
    fn missing_file_is_io_not_found() {
        let dir = tempfile::tempdir().unwrap();
        match SptStore::open(&dir.path().join("spt.bin")) {
            Err(StoreError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            Err(other) => panic!("expected Io, got {other:?}"),
            Ok(_) => panic!("expected Io, got Ok"),
        }
    }

    #[test]
    fn truncated_header_is_corrupt() {
        let (_d, path) = written(&[call()]);
        std::fs::write(&path, b"AROMASPT\x01\x00").unwrap();
        assert_eq!(
            open_corrupt(&path),
            "file is 10 bytes, shorter than the 16-byte header"
        );
    }

    #[test]
    fn wrong_magic_is_corrupt() {
        let (_d, path) = written(&[call()]);
        patch(&path, 0, b"AROMAPST");
        assert_eq!(open_corrupt(&path), "bad magic");
    }

    #[test]
    fn wrong_version_is_corrupt() {
        let (_d, path) = written(&[call()]);
        patch(&path, 8, &2u32.to_le_bytes());
        assert_eq!(open_corrupt(&path), "unsupported version 2 (expected 1)");
    }

    #[test]
    fn count_beyond_offset_table_is_corrupt() {
        let (_d, path) = written(&[]);
        patch(&path, 12, &5u32.to_le_bytes());
        assert_eq!(
            open_corrupt(&path),
            "file is 24 bytes, offset table needs 64"
        );
    }

    #[test]
    fn nonzero_first_offset_is_corrupt() {
        let (_d, path) = written(&[call(), call()]);
        patch(&path, 16, &1u64.to_le_bytes());
        assert_eq!(open_corrupt(&path), "offset 0 = 1 (expected 0)");
    }

    #[test]
    fn decreasing_offsets_are_corrupt() {
        let (_d, path) = written(&[call(), call()]);
        let b = std::fs::read(&path).unwrap();
        let last = u64::from_le_bytes(b[32..40].try_into().unwrap());
        patch(&path, 24, &(last + 1).to_le_bytes());
        assert_eq!(open_corrupt(&path), "offsets decrease at index 2");
    }

    #[test]
    fn trailing_bytes_after_last_blob_are_corrupt() {
        let (_d, path) = written(&[call()]);
        let mut b = std::fs::read(&path).unwrap();
        let blob_len = b.len() - 32;
        b.push(0);
        std::fs::write(&path, b).unwrap();
        assert_eq!(
            open_corrupt(&path),
            format!(
                "last offset {blob_len} (expected blob length {})",
                blob_len + 1
            )
        );
    }

    #[test]
    fn undecodable_blob_is_corrupt_on_get_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spt.bin");
        let good = postcard::to_stdvec(&call()).unwrap();
        raw_file(&path, &[good, vec![0xff; 3]]);
        let s = SptStore::open(&path).unwrap();
        assert_eq!(s.get(0).unwrap(), call());
        assert!(get_corrupt(&s, 1).starts_with("unit 1: "));
    }

    #[test]
    fn blob_with_trailing_bytes_is_corrupt() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("spt.bin");
        let mut blob = postcard::to_stdvec(&call()).unwrap();
        blob.extend_from_slice(&[0, 0]);
        raw_file(&path, &[blob]);
        let s = SptStore::open(&path).unwrap();
        assert_eq!(
            get_corrupt(&s, 0),
            "unit 0: 2 trailing bytes after the tree"
        );
    }

    #[test]
    fn bad_arena_references_are_corrupt() {
        let mut self_loop = call();
        self_loop.nodes[0].children.push(Elem::Tree(0));
        let mut bad_leaf = call();
        bad_leaf.nodes[0].children.push(Elem::Leaf(1));
        let mut bad_root = call();
        bad_root.root = 1;
        let mut forward_ok = if_return();
        forward_ok.nodes[1].children.push(Elem::Keyword("".into()));
        let (_d, path) = written(&[self_loop, bad_leaf, bad_root, forward_ok.clone()]);
        let s = SptStore::open(&path).unwrap();
        assert_eq!(get_corrupt(&s, 0), "unit 0: node 0: child node 0 of 1");
        assert_eq!(get_corrupt(&s, 1), "unit 1: node 0: leaf 1 of 1");
        assert_eq!(get_corrupt(&s, 2), "unit 2: root 1 of 1 nodes");
        assert_eq!(s.get(3).unwrap(), forward_ok);
    }

    #[test]
    fn every_single_byte_truncation_is_rejected_or_harmless() {
        let (_d, path) = written(&[if_return(), call()]);
        let full = std::fs::read(&path).unwrap();
        for len in 0..full.len() {
            std::fs::write(&path, &full[..len]).unwrap();
            if let Ok(s) = SptStore::open(&path) {
                // Only possible if the length still matches the offsets: it never does.
                panic!("length {len} opened with {} trees", s.len());
            }
        }
    }
}
