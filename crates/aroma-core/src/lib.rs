//! Aroma (Luan et al., "Aroma: Code Recommendation via Structural Code Search",
//! OOPSLA 2019, arXiv 1812.01158): the language-agnostic algorithm and on-disk index.
//!
//! ```text
//!  Raw (from aroma-lang) --[spt]--> Spt --[features]--> per-leaf F(n) --> Bag of FeatureId
//!                                                                          |
//!  [index] Phase I: overlap > min_ratio * known query features, top cap <--+
//!      v
//!  [prune] greedy leaf selection --> rerank (Jaccard > min_pruned)
//!      v
//!  [cluster] tuples (tau3, tau2) --> intersect --> RecGate   (--experimental-recommend)
//!      v
//!  [render] full lines + "// your code ..." + kept byte spans
//! ```
//!
//! This crate never depends on tree-sitter; adapters hand it `spt::Raw` trees.
pub mod cluster;
pub mod deadline;
pub mod explain;
pub mod features;
pub mod index;
pub mod lang;
pub mod lexical;
pub mod multiset;
pub mod profile;
pub mod prune;
pub mod query;
pub mod render;
pub mod spt;
pub mod store;
pub mod unit;
