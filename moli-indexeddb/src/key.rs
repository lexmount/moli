use std::cmp::Ordering as CmpOrdering;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Key {
    String(String),
    Integer(i64),
    Array(Vec<Key>),
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        match (self, other) {
            (Self::Integer(left), Self::Integer(right)) => left.cmp(right),
            (Self::String(left), Self::String(right)) => left.cmp(right),
            (Self::Array(left), Self::Array(right)) => left.cmp(right),
            (Self::Integer(_), Self::String(_)) => CmpOrdering::Less,
            (Self::Integer(_), Self::Array(_)) => CmpOrdering::Less,
            (Self::String(_), Self::Integer(_)) => CmpOrdering::Greater,
            (Self::String(_), Self::Array(_)) => CmpOrdering::Less,
            (Self::Array(_), Self::Integer(_) | Self::String(_)) => CmpOrdering::Greater,
        }
    }
}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<i64> for Key {
    fn from(value: i64) -> Self {
        Self::Integer(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum KeyPath {
    String(String),
    Sequence(Vec<String>),
}

impl KeyPath {
    /// Key paths contain identifiers separated by dots, or a non-empty list
    /// of such strings. An empty string selects the entire stored value.
    pub fn is_valid(&self) -> bool {
        match self {
            Self::String(path) => string_key_path_is_valid(path),
            Self::Sequence(paths) => {
                !paths.is_empty() && paths.iter().all(|path| string_key_path_is_valid(path))
            }
        }
    }

    pub fn is_sequence(&self) -> bool {
        matches!(self, Self::Sequence(_))
    }

    pub fn is_empty_string(&self) -> bool {
        matches!(self, Self::String(value) if value.is_empty())
    }
}

fn string_key_path_is_valid(path: &str) -> bool {
    path.is_empty()
        || path.split('.').all(|identifier| {
            let mut chars = identifier.chars();
            chars
                .next()
                .is_some_and(|ch| matches!(ch, '$' | '_') || unicode_id_start::is_id_start(ch))
                && chars.all(|ch| {
                    matches!(ch, '$' | '_' | '\u{200c}' | '\u{200d}')
                        || unicode_id_start::is_id_continue(ch)
                })
        })
}

impl From<String> for KeyPath {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for KeyPath {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::KeyPath;

    #[test]
    fn key_paths_accept_unicode_identifiers_without_normalizing_them() {
        for path in [
            "",
            "id",
            "true.$",
            "delete._",
            "a0.b9",
            "my.køi",
            "名字.值",
            "a\u{0301}",
            "a\u{200c}\u{200d}",
            "\u{2118}",
            "\u{309b}",
            "\u{10400}.id",
        ] {
            assert!(KeyPath::from(path).is_valid(), "{path:?}");
            assert!(KeyPath::Sequence(vec![path.into()]).is_valid(), "{path:?}");
        }
        assert!(KeyPath::Sequence(vec!["id".into(), "meta.name".into()]).is_valid());
    }

    #[test]
    fn key_paths_reject_empty_sequences_and_invalid_identifier_segments() {
        for path in [
            ".",
            ".id",
            "id.",
            "id..name",
            "0id",
            "id.1",
            "with space",
            "id-name",
            "id[0]",
            "a,b",
            "\u{0301}a",
            "\u{200c}a",
            "\u{200d}a",
            "\u{fffd}",
            "a\0b",
            "a\nb",
            "a\u{feff}",
            "😀",
            "\\u0061",
            "a.\\u0062",
        ] {
            assert!(!KeyPath::from(path).is_valid(), "{path:?}");
            assert!(
                !KeyPath::Sequence(vec!["id".into(), path.into()]).is_valid(),
                "{path:?}"
            );
        }
        assert!(!KeyPath::Sequence(vec![]).is_valid());
    }
}
