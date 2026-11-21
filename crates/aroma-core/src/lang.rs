//! Supported source languages. Every feature id is namespaced by `Lang::as_str`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lang {
    Java,
    Apex,
    LwcJs,
    LwcHtml,
    SfMeta,
    Aura,
    AuraJs,
    Vf,
}

impl Lang {
    /// All stable language identities, including adapters planned for later releases.
    pub const ALL: [Lang; 8] = [
        Lang::Java,
        Lang::Apex,
        Lang::LwcJs,
        Lang::LwcHtml,
        Lang::SfMeta,
        Lang::Aura,
        Lang::AuraJs,
        Lang::Vf,
    ];

    /// Languages with an implemented adapter, in file-routing precedence order.
    pub const SPECCED: &[Lang] = &[
        Lang::Java,
        Lang::Apex,
        Lang::LwcJs,
        Lang::LwcHtml,
        Lang::SfMeta,
        Lang::AuraJs,
        Lang::Vf,
        Lang::Aura,
    ];

    /// Stable lower-case name; part of every feature id (D7).
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Java => "java",
            Lang::Apex => "apex",
            Lang::LwcJs => "lwcjs",
            Lang::LwcHtml => "lwchtml",
            Lang::SfMeta => "sfmeta",
            Lang::Aura => "aura",
            Lang::AuraJs => "aurajs",
            Lang::Vf => "vf",
        }
    }

    /// Inverse of [`Lang::as_str`].
    pub fn parse(s: &str) -> Option<Lang> {
        Lang::ALL.into_iter().find(|l| l.as_str() == s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_str_and_parse_round_trip() {
        for l in Lang::ALL {
            assert_eq!(Lang::parse(l.as_str()), Some(l));
        }
        assert_eq!(Lang::Java.as_str(), "java");
        assert_eq!(Lang::Apex.as_str(), "apex");
        assert_eq!(Lang::parse("Java"), None);
        assert_eq!(Lang::parse("kotlin"), None);
    }

    #[test]
    fn every_language_has_its_stable_serde_name() {
        for lang in Lang::ALL {
            let encoded = serde_json::to_string(&lang).unwrap();
            assert_eq!(encoded, format!("\"{}\"", lang.as_str()));
            assert_eq!(serde_json::from_str::<Lang>(&encoded).unwrap(), lang);
        }
    }

    #[test]
    fn serde_uses_lowercase_names() {
        assert_eq!(serde_json::to_string(&Lang::Apex).unwrap(), "\"apex\"");
        let l: Lang = serde_json::from_str("\"java\"").unwrap();
        assert_eq!(l, Lang::Java);
    }
}
