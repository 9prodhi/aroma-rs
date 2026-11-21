//! Indexing units (method, constructor, trigger, accessor, initializer bodies) and the
//! read-only store a query uses to look them up.

use crate::lang::Lang;
use crate::spt::Spt;
use crate::store::StoreError;
use serde::{Deserialize, Serialize};
use std::ops::Range;
use std::path::Path;

pub type UnitId = u32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnitKind {
    Method,
    Constructor,
    Trigger,
    Accessor,
    Initializer,
    Snippet,
    Template,
    MarkupBlock,
    WireMember,
    FlowElement,
    Field,
    ValidationRule,
    MetadataRecord,
    Page,
    Component,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitRecord {
    pub id: UnitId,
    pub lang: Lang,
    pub kind: UnitKind,
    /// Index into the store's file list.
    pub file: u32,
    pub bytes: Range<u32>,
    /// 1-based source lines, end exclusive.
    pub lines: Range<u32>,
    pub n_leaves: u32,
    /// Hash of the unit's normalized token stream; equal hashes mark exact clones (D22).
    pub body_hash: u64,
    pub is_test: bool,
}

/// Read access to indexed units: implemented by the on-disk index and by test fixtures.
pub trait UnitStore: Sync {
    fn n_units(&self) -> u32;
    fn record(&self, id: UnitId) -> UnitRecord;
    fn spt(&self, id: UnitId) -> Result<Spt, StoreError>;
    fn file_path(&self, file: u32) -> &Path;
}

/// Units with more leaves than this are never pruned (D23 eligibility).
pub const MAX_PRUNE_LEAVES: u32 = 2000;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_kind_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&UnitKind::Constructor).unwrap(),
            "\"constructor\""
        );
        assert_eq!(
            serde_json::to_string(&UnitKind::Snippet).unwrap(),
            "\"snippet\""
        );
    }

    #[test]
    fn unit_record_round_trips_through_postcard() {
        let r = UnitRecord {
            id: 7,
            lang: Lang::Apex,
            kind: UnitKind::Trigger,
            file: 3,
            bytes: 10..90,
            lines: 2..9,
            n_leaves: 41,
            body_hash: 0xdead_beef,
            is_test: true,
        };
        let bytes = postcard::to_stdvec(&r).unwrap();
        let back: UnitRecord = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, r);
    }
}
