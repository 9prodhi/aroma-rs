//! On-disk index (D15/D19/D22/D23): one immutable generation per build under
//! `.aroma/gen-<n>/` with `manifest.json`, `postings.bin`, `units.bin`, `files.json`,
//! `spt.bin`, `globals.json`; writer lock `.aroma/lock`; GC keeps n and n-1.
pub mod gc;
pub mod generation;
pub mod lexical;
pub mod lock;
pub mod manifest;
pub mod postings;
pub mod spt_store;
pub mod unit_table;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("corrupt {file}: {reason}")]
    Corrupt { file: &'static str, reason: String },
    #[error("incompatible index ({0}); re-index required")]
    Incompatible(String),
    #[error("index is locked by pid {pid}")]
    Locked { pid: u32 },
    #[error("codec error: {0}")]
    Codec(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages() {
        let e = StoreError::Corrupt {
            file: "postings.bin",
            reason: "short header".into(),
        };
        assert_eq!(e.to_string(), "corrupt postings.bin: short header");
        assert_eq!(
            StoreError::Locked { pid: 42 }.to_string(),
            "index is locked by pid 42"
        );
        assert_eq!(
            StoreError::Incompatible("grammar changed".into()).to_string(),
            "incompatible index (grammar changed); re-index required"
        );
        let io: StoreError = std::io::Error::new(std::io::ErrorKind::NotFound, "gone").into();
        assert_eq!(io.to_string(), "io error: gone");
    }
}
