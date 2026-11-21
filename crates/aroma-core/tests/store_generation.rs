//! T10: writer lock, generation publish protocol, garbage collection.

mod lock {
    use aroma_core::store::lock::WriterLock;
    use aroma_core::store::StoreError;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    #[test]
    fn acquire_creates_dir_and_records_pid() {
        let root = tempfile::tempdir().unwrap();
        let aroma = root.path().join(".aroma");
        let lock = WriterLock::acquire(&aroma, Duration::ZERO).unwrap();
        assert_eq!(lock.aroma_dir(), aroma.as_path());
        drop(lock);
        let text = std::fs::read_to_string(aroma.join("lock")).unwrap();
        assert_eq!(text, format!("#{}\n", std::process::id()));
    }

    #[test]
    fn second_writer_times_out_with_holder_pid() {
        let root = tempfile::tempdir().unwrap();
        let aroma = root.path().join(".aroma");
        let (held_tx, held_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel::<()>();
        let a = aroma.clone();
        let holder = std::thread::spawn(move || {
            let lock = WriterLock::acquire(&a, Duration::ZERO).unwrap();
            held_tx.send(()).unwrap();
            done_rx.recv().unwrap();
            drop(lock);
        });
        held_rx.recv().unwrap();
        let start = Instant::now();
        let err = WriterLock::acquire(&aroma, Duration::from_millis(200)).unwrap_err();
        let waited = start.elapsed();
        match err {
            StoreError::Locked { pid } => assert_eq!(pid, std::process::id()),
            other => panic!("expected Locked, got {other:?}"),
        }
        assert!(waited >= Duration::from_millis(200), "waited {waited:?}");
        assert!(waited < Duration::from_secs(5), "waited {waited:?}");
        done_tx.send(()).unwrap();
        holder.join().unwrap();
    }

    #[test]
    fn zero_timeout_tries_once() {
        let root = tempfile::tempdir().unwrap();
        let aroma = root.path().join(".aroma");
        let _held = WriterLock::acquire(&aroma, Duration::ZERO).unwrap();
        let start = Instant::now();
        assert!(matches!(
            WriterLock::acquire(&aroma, Duration::ZERO),
            Err(StoreError::Locked { .. })
        ));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn released_on_drop() {
        let root = tempfile::tempdir().unwrap();
        let aroma = root.path().join(".aroma");
        drop(WriterLock::acquire(&aroma, Duration::ZERO).unwrap());
        drop(WriterLock::acquire(&aroma, Duration::ZERO).unwrap());
    }

    #[test]
    fn waiter_gets_lock_when_holder_releases() {
        let root = tempfile::tempdir().unwrap();
        let aroma = root.path().join(".aroma");
        let (held_tx, held_rx) = mpsc::channel();
        let a = aroma.clone();
        let holder = std::thread::spawn(move || {
            let lock = WriterLock::acquire(&a, Duration::ZERO).unwrap();
            held_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(150));
            drop(lock);
        });
        held_rx.recv().unwrap();
        let start = Instant::now();
        let lock = WriterLock::acquire(&aroma, Duration::from_secs(10)).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(50));
        drop(lock);
        holder.join().unwrap();
    }
}

mod generation {
    use aroma_core::store::generation::IndexDir;
    use aroma_core::store::lock::WriterLock;
    use aroma_core::store::StoreError;
    use std::path::Path;
    use std::time::Duration;

    fn setup(root: &Path) -> (IndexDir, WriterLock) {
        let idx = IndexDir::new(root);
        let lock = WriterLock::acquire(idx.aroma_dir(), Duration::ZERO).unwrap();
        (idx, lock)
    }

    fn publish(idx: &IndexDir, lock: &WriterLock, marker: &str) -> u64 {
        let w = idx.begin(lock).unwrap();
        std::fs::write(w.dir().join("marker.txt"), marker).unwrap();
        w.commit().unwrap()
    }

    fn corrupt_reason(idx: &IndexDir) -> String {
        match idx.current() {
            Err(StoreError::Corrupt { file, reason }) => {
                assert_eq!(file, "CURRENT");
                reason
            }
            other => panic!("expected Corrupt, got {other:?}"),
        }
    }

    #[test]
    fn new_does_not_touch_disk() {
        let root = tempfile::tempdir().unwrap();
        let idx = IndexDir::new(root.path());
        assert_eq!(idx.aroma_dir(), root.path().join(".aroma").as_path());
        assert!(!idx.aroma_dir().exists());
        assert!(idx.current().unwrap().is_none());
    }

    #[test]
    fn begin_uses_tmp_dir_and_next_number() {
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        let w = idx.begin(&lock).unwrap();
        assert_eq!(w.number(), 1);
        assert_eq!(w.dir(), idx.aroma_dir().join("tmp-1").as_path());
        assert!(w.dir().is_dir());
        assert!(idx.current().unwrap().is_none());
    }

    #[test]
    fn commit_makes_generation_current() {
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        assert_eq!(publish(&idx, &lock, "one"), 1);
        let aroma = idx.aroma_dir();
        assert_eq!(idx.current().unwrap(), Some((1, aroma.join("gen-1"))));
        assert_eq!(
            std::fs::read_to_string(aroma.join("gen-1/marker.txt")).unwrap(),
            "one"
        );
        assert_eq!(
            std::fs::read_to_string(aroma.join("CURRENT")).unwrap(),
            "1\n"
        );
        assert!(!aroma.join("tmp-1").exists());
        assert!(!aroma.join("CURRENT.tmp").exists());
        assert_eq!(publish(&idx, &lock, "two"), 2);
        assert_eq!(idx.current().unwrap(), Some((2, aroma.join("gen-2"))));
        assert!(aroma.join("gen-1").is_dir());
    }

    #[test]
    fn commit_syncs_nested_files() {
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        let w = idx.begin(&lock).unwrap();
        std::fs::create_dir(w.dir().join("sub")).unwrap();
        std::fs::write(w.dir().join("sub/x.bin"), [1, 2, 3]).unwrap();
        assert_eq!(w.commit().unwrap(), 1);
        assert_eq!(
            std::fs::read(idx.aroma_dir().join("gen-1/sub/x.bin")).unwrap(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn crash_before_commit_leaves_current_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        publish(&idx, &lock, "one");
        let w = idx.begin(&lock).unwrap();
        assert_eq!(w.number(), 2);
        std::fs::write(w.dir().join("marker.txt"), "half-written").unwrap();
        drop(w); // the writer dies without committing
        let aroma = idx.aroma_dir();
        assert_eq!(idx.current().unwrap(), Some((1, aroma.join("gen-1"))));
        assert!(aroma.join("tmp-2").is_dir());
        // The next writer reuses number 2 and starts from an empty directory.
        let w = idx.begin(&lock).unwrap();
        assert_eq!(w.number(), 2);
        assert!(!w.dir().join("marker.txt").exists());
    }

    #[test]
    fn begin_replaces_unpublished_gen_dir() {
        // A writer crashed after renaming tmp-2 -> gen-2 but before rewriting CURRENT.
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        publish(&idx, &lock, "one");
        let stale = idx.aroma_dir().join("gen-2");
        std::fs::create_dir(&stale).unwrap();
        std::fs::write(stale.join("stale.txt"), "x").unwrap();
        assert_eq!(publish(&idx, &lock, "two"), 2);
        assert!(!stale.join("stale.txt").exists());
        assert_eq!(
            std::fs::read_to_string(stale.join("marker.txt")).unwrap(),
            "two"
        );
    }

    #[test]
    fn current_with_garbage_is_corrupt() {
        let root = tempfile::tempdir().unwrap();
        let (idx, _lock) = setup(root.path());
        std::fs::write(idx.aroma_dir().join("CURRENT"), "seven\n").unwrap();
        assert_eq!(corrupt_reason(&idx), "not a generation number: \"seven\"");
    }

    #[test]
    fn current_naming_missing_dir_is_corrupt() {
        let root = tempfile::tempdir().unwrap();
        let (idx, _lock) = setup(root.path());
        std::fs::write(idx.aroma_dir().join("CURRENT"), "4\n").unwrap();
        assert_eq!(corrupt_reason(&idx), "names gen-4 which does not exist");
    }

    #[test]
    fn begin_rejects_lock_of_another_index() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let idx = IndexDir::new(a.path());
        let other = WriterLock::acquire(&b.path().join(".aroma"), Duration::ZERO).unwrap();
        match idx.begin(&other) {
            Err(StoreError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput),
            other => panic!("expected Io(InvalidInput), got {other:?}"),
        }
    }
}

mod gc {
    use aroma_core::store::gc::collect_garbage;
    use aroma_core::store::generation::IndexDir;
    use aroma_core::store::lock::WriterLock;
    use aroma_core::store::StoreError;
    use std::path::Path;
    use std::time::Duration;

    fn setup(root: &Path) -> (IndexDir, WriterLock) {
        let idx = IndexDir::new(root);
        let lock = WriterLock::acquire(idx.aroma_dir(), Duration::ZERO).unwrap();
        (idx, lock)
    }

    fn publish(idx: &IndexDir, lock: &WriterLock) -> u64 {
        let w = idx.begin(lock).unwrap();
        std::fs::write(w.dir().join("postings.bin"), [0u8; 8]).unwrap();
        w.commit().unwrap()
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn rejects_lock_of_another_index() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let idx = IndexDir::new(a.path());
        let other = WriterLock::acquire(&b.path().join(".aroma"), Duration::ZERO).unwrap();
        match collect_garbage(&idx, &other) {
            Err(StoreError::Io(e)) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput),
            other => panic!("expected Io(InvalidInput), got {other:?}"),
        }
    }

    #[test]
    fn keeps_current_and_previous() {
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        for _ in 0..4 {
            publish(&idx, &lock);
        }
        assert_eq!(collect_garbage(&idx, &lock).unwrap(), vec![1, 2]);
        assert_eq!(
            names(idx.aroma_dir()),
            vec!["CURRENT", "gen-3", "gen-4", "lock"]
        );
        assert_eq!(publish(&idx, &lock), 5);
        assert_eq!(collect_garbage(&idx, &lock).unwrap(), vec![3]);
        assert_eq!(collect_garbage(&idx, &lock).unwrap(), Vec::<u64>::new());
        assert_eq!(
            names(idx.aroma_dir()),
            vec!["CURRENT", "gen-4", "gen-5", "lock"]
        );
    }

    #[test]
    fn first_generation_has_nothing_to_collect() {
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        publish(&idx, &lock);
        assert_eq!(collect_garbage(&idx, &lock).unwrap(), Vec::<u64>::new());
        assert_eq!(names(idx.aroma_dir()), vec!["CURRENT", "gen-1", "lock"]);
    }

    #[test]
    fn crashed_writer_tmp_dir_is_collected() {
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        publish(&idx, &lock);
        let w = idx.begin(&lock).unwrap();
        std::fs::write(w.dir().join("postings.bin"), [1u8; 8]).unwrap();
        drop(w);
        std::fs::write(idx.aroma_dir().join("CURRENT.tmp"), "2\n").unwrap();
        assert_eq!(collect_garbage(&idx, &lock).unwrap(), Vec::<u64>::new());
        assert_eq!(names(idx.aroma_dir()), vec!["CURRENT", "gen-1", "lock"]);
        assert_eq!(idx.current().unwrap().unwrap().0, 1);
    }

    #[test]
    fn unpublished_newer_generation_is_collected() {
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        publish(&idx, &lock);
        publish(&idx, &lock);
        std::fs::create_dir(idx.aroma_dir().join("gen-3")).unwrap();
        assert_eq!(collect_garbage(&idx, &lock).unwrap(), vec![3]);
        assert_eq!(
            names(idx.aroma_dir()),
            vec!["CURRENT", "gen-1", "gen-2", "lock"]
        );
    }

    #[test]
    fn without_current_only_tmp_dirs_go() {
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        std::fs::create_dir(idx.aroma_dir().join("gen-7")).unwrap();
        std::fs::create_dir(idx.aroma_dir().join("tmp-8")).unwrap();
        assert_eq!(collect_garbage(&idx, &lock).unwrap(), Vec::<u64>::new());
        assert_eq!(names(idx.aroma_dir()), vec!["gen-7", "lock"]);
    }

    #[test]
    fn unrelated_entries_are_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        for _ in 0..3 {
            publish(&idx, &lock);
        }
        let aroma = idx.aroma_dir();
        std::fs::create_dir(aroma.join("gen-x")).unwrap();
        std::fs::create_dir(aroma.join("cache")).unwrap();
        std::fs::write(aroma.join("gen-1.txt"), "note").unwrap();
        assert_eq!(collect_garbage(&idx, &lock).unwrap(), vec![1]);
        assert_eq!(
            names(aroma),
            vec![
                "CURRENT",
                "cache",
                "gen-1.txt",
                "gen-2",
                "gen-3",
                "gen-x",
                "lock"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn mapped_files_of_collected_generation_stay_readable() {
        // Readers mmap gen-(n-2) files; on Unix the unlink leaves the mapping valid.
        let root = tempfile::tempdir().unwrap();
        let (idx, lock) = setup(root.path());
        publish(&idx, &lock);
        let f = std::fs::File::open(idx.aroma_dir().join("gen-1/postings.bin")).unwrap();
        // SAFETY: test-only; nobody writes the file while it is mapped.
        let map = unsafe { memmap2::Mmap::map(&f).unwrap() };
        publish(&idx, &lock);
        publish(&idx, &lock);
        assert_eq!(collect_garbage(&idx, &lock).unwrap(), vec![1]);
        assert_eq!(&map[..], &[0u8; 8]);
    }
}
