//! tree-sitter language adapters for aroma (D21 three-stage adapter):
//! (1) `table`: shared CST -> `KRaw` walk driven by `spec::LangSpec` tables;
//! (2) `normalize`: per-language tree rewrites;
//! (3) `classify`: shared D18 `#VAR` classification and case folding -> `aroma_core::spt::Raw`.
//! `build` writes index generations and `index` is the query facade.
pub mod build;
pub mod classify;
pub mod globals;
pub mod index;
pub mod kraw;
pub mod normalize;
pub mod snippet;
pub mod spec;
pub mod table;

/// Path routing and snippet-sensitive language aliases for adapter clients.
pub use spec::{lang_for_path, resolve_query_lang};
