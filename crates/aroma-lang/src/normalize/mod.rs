//! Stage 2 (D21): per-language tree rewrites on `KRaw` (Java virtual-ANTLR regrouping,
//! Apex SOQL/DML). A normalizer must be deterministic and context-free so a unit body
//! normalizes identically whether indexed or pasted as a snippet (D18).
pub mod apex;
pub mod aura;
pub mod aura_js;
pub mod expr;
pub mod java;
pub mod lwc_html;
pub mod lwc_js;
pub mod sfmeta;
pub mod vf;

use crate::kraw::KRaw;

pub trait Normalizer: Sync {
    fn normalize(&self, node: KRaw) -> KRaw;
}

/// Leaves the tree unchanged.
pub struct Identity;

impl Normalizer for Identity {
    fn normalize(&self, node: KRaw) -> KRaw {
        node
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> KRaw {
        KRaw::Node {
            kind: "block",
            field: Some("body"),
            children: vec![KRaw::Token {
                kind: "{",
                named: false,
                text: "{".into(),
                bytes: 0..1,
                line: 1,
                field: None,
            }],
        }
    }

    #[test]
    fn identity_normalizers_return_input() {
        assert_eq!(Identity.normalize(sample()), sample());
        assert_eq!(java::NORMALIZER.normalize(sample()), sample());
        assert_eq!(apex::NORMALIZER.normalize(sample()), sample());
    }
}
