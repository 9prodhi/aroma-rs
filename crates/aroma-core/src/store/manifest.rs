//! `manifest`: `manifest.json` with schema, feature fingerprint, profile, adapter and
//! grammar versions (D22c). Owned and implemented by task T08 (`Manifest`,
//! `INDEX_SCHEMA_VERSION`).
//!
//! A query refuses an index whose manifest differs from the one the running binary
//! would write in any field except `generation`, `n_units` and `n_files`
//! (`StoreError::Incompatible`, "re-index required").

use crate::profile::ProfileName;
use crate::store::StoreError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;

/// 3 since T24: shared adapter hooks and expanded language/unit identities.
/// Older indexes are incompatible ("re-index required") and rebuilt in full.
pub const INDEX_SCHEMA_VERSION: u32 = 3;
pub const MANIFEST_FILE: &str = "manifest.json";

/// Persisted metadata mode. Missing fields in old manifests mean Lexical.
/// New builds choose `BUILD_DEFAULT` explicitly instead of the serde default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SfmetaMode {
    Structural,
    #[default]
    Lexical,
}

impl SfmetaMode {
    pub const BUILD_DEFAULT: Self = Self::Structural;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub feature_schema: u32,
    pub feature_fingerprint: u64,
    pub profile: ProfileName,
    pub compat: bool,
    #[serde(default)]
    pub sfmeta_mode: SfmetaMode,
    /// Language name -> adapter version (`LangSpec::adapter_version`).
    pub adapters: BTreeMap<String, String>,
    /// Language name -> grammar version (`LangSpec::grammar_version`).
    pub grammars: BTreeMap<String, String>,
    pub generation: u64,
    pub n_units: u32,
    pub n_files: u32,
}

impl Manifest {
    /// Write `dir/manifest.json` (pretty JSON + trailing newline) via a fsynced
    /// temporary file and a rename.
    pub fn write(&self, dir: &Path) -> Result<(), StoreError> {
        let mut json =
            serde_json::to_string_pretty(self).map_err(|e| StoreError::Codec(e.to_string()))?;
        json.push('\n');
        let path = dir.join(MANIFEST_FILE);
        let tmp = dir.join("manifest.json.tmp");
        let mut f = File::create(&tmp)?;
        f.write_all(json.as_bytes())?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// Read `dir/manifest.json`. A missing file is `StoreError::Io` (NotFound); text
    /// that is not a manifest is `StoreError::Corrupt { file: "manifest.json", .. }`.
    pub fn read(dir: &Path) -> Result<Manifest, StoreError> {
        let text = std::fs::read_to_string(dir.join(MANIFEST_FILE))?;
        serde_json::from_str(&text).map_err(|e| StoreError::Corrupt {
            file: "manifest.json",
            reason: e.to_string(),
        })
    }

    /// `self` is the manifest on disk, `expected` the one the running binary would
    /// write. Every differing field except `generation`, `n_units` and `n_files` is
    /// listed, in declaration order, as `"<field>: index <have>, expected <want>"`,
    /// joined by `"; "`.
    pub fn check_compatible(&self, expected: &Manifest) -> Result<(), StoreError> {
        let mut diffs: Vec<String> = Vec::new();
        let mut field = |name: &str, have: String, want: String| {
            if have != want {
                diffs.push(format!("{name}: index {have}, expected {want}"));
            }
        };
        field(
            "schema_version",
            self.schema_version.to_string(),
            expected.schema_version.to_string(),
        );
        field(
            "feature_schema",
            self.feature_schema.to_string(),
            expected.feature_schema.to_string(),
        );
        field(
            "feature_fingerprint",
            format!("{:#018x}", self.feature_fingerprint),
            format!("{:#018x}", expected.feature_fingerprint),
        );
        field(
            "profile",
            format!("{:?}", self.profile),
            format!("{:?}", expected.profile),
        );
        field(
            "compat",
            self.compat.to_string(),
            expected.compat.to_string(),
        );
        field(
            "sfmeta_mode",
            format!("{:?}", self.sfmeta_mode),
            format!("{:?}", expected.sfmeta_mode),
        );
        field(
            "adapters",
            format!("{:?}", self.adapters),
            format!("{:?}", expected.adapters),
        );
        field(
            "grammars",
            format!("{:?}", self.grammars),
            format!("{:?}", expected.grammars),
        );
        if diffs.is_empty() {
            Ok(())
        } else {
            Err(StoreError::Incompatible(diffs.join("; ")))
        }
    }
}
