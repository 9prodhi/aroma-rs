//! Aroma (Luan et al., "Aroma: Code Recommendation via Structural Code Search",
//! OOPSLA 2019, arXiv 1812.01158) expressed as Rust.
//!
//! Semantics follow the *reference profile*: the executable code in
//! facebookresearch/aroma-paper-artifacts (`ConvertJava.java`, `similar.py`).
//! Where the paper's math differs, the paper variant is noted inline.
//!
//! ```text
//!  source --tree-sitter--> CST --[ts_adapter]--> Raw --[spt]--> SimplifiedParseTree
//!                                                                     |
//!                                                          [features] per-leaf F(n)
//!                                                                     |
//!        corpus units: S(F(m)) --> [index] InvertedIndex <-- query F(q)
//!                                     | Phase I: overlap > 0.4*|S(F(q))|, top 100
//!                                     v
//!        [prune] greedy leaf selection (Jaccard) --> rerank, keep J > 0.65
//!                                     v
//!        [cluster] grow tuples (tau3 = 0.9, tau2 = 1.5), dedup Jaccard > 0.5, top 5
//!                                     v
//!        [cluster::intersect] sequential prune --> [render] "// your code ..."
//! ```
pub mod cluster;
pub mod features;
pub mod index;
pub mod multiset;
pub mod pipeline;
pub mod prune;
pub mod render;
pub mod spt;
pub mod ts_adapter;
