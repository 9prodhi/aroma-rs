//! Kinded raw tree: stage 1 output and stage 2 input/output. Unlike `aroma_core::spt::Raw`
//! it still carries grammar kinds and field names, which normalizers and the D18
//! classifier need.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub enum KRaw {
    /// A terminal (or atomic) node. `named` is tree-sitter's named flag.
    Token {
        kind: &'static str,
        named: bool,
        text: String,
        bytes: Range<u32>,
        line: u32,
        field: Option<&'static str>,
    },
    /// An inner node with its non-comment children in source order.
    Node {
        kind: &'static str,
        field: Option<&'static str>,
        children: Vec<KRaw>,
    },
}

impl KRaw {
    pub fn kind(&self) -> &'static str {
        match self {
            KRaw::Token { kind, .. } | KRaw::Node { kind, .. } => kind,
        }
    }

    /// Field name under which this node hangs in its parent.
    pub fn field(&self) -> Option<&'static str> {
        match self {
            KRaw::Token { field, .. } | KRaw::Node { field, .. } => *field,
        }
    }

    pub fn with_field(self, f: Option<&'static str>) -> KRaw {
        match self {
            KRaw::Token {
                kind,
                named,
                text,
                bytes,
                line,
                ..
            } => KRaw::Token {
                kind,
                named,
                text,
                bytes,
                line,
                field: f,
            },
            KRaw::Node { kind, children, .. } => KRaw::Node {
                kind,
                field: f,
                children,
            },
        }
    }
}

/// Text of the single opaque keyword that replaces an ERROR subtree (D11).
pub const ERR_TOKEN: &str = "#ERR";

#[cfg(test)]
mod tests {
    use super::*;

    fn tok() -> KRaw {
        KRaw::Token {
            kind: "identifier",
            named: true,
            text: "x".into(),
            bytes: 4..5,
            line: 2,
            field: None,
        }
    }

    #[test]
    fn kind_and_field_accessors() {
        let t = tok();
        assert_eq!(t.kind(), "identifier");
        assert_eq!(t.field(), None);
        let n = KRaw::Node {
            kind: "block",
            field: Some("body"),
            children: vec![t],
        };
        assert_eq!(n.kind(), "block");
        assert_eq!(n.field(), Some("body"));
    }

    #[test]
    fn with_field_replaces_only_the_field() {
        let t = tok().with_field(Some("name"));
        assert_eq!(
            t,
            KRaw::Token {
                kind: "identifier",
                named: true,
                text: "x".into(),
                bytes: 4..5,
                line: 2,
                field: Some("name")
            }
        );
        let n = KRaw::Node {
            kind: "block",
            field: Some("body"),
            children: vec![tok()],
        }
        .with_field(None);
        assert_eq!(
            n,
            KRaw::Node {
                kind: "block",
                field: None,
                children: vec![tok()]
            }
        );
    }

    #[test]
    fn err_token_text() {
        assert_eq!(ERR_TOKEN, "#ERR");
    }
}
