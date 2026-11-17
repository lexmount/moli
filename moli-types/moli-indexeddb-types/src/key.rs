use std::cmp::Ordering;

use serde::{Deserialize, Serialize};

/// Maximum array nesting accepted by key conversion and persistent storage.
/// Keep recursive comparison, cloning, dropping and JS materialization bounded.
pub const MAX_KEY_DEPTH: usize = 256;
const KEY_DEPTH_ERROR: &str = "IndexedDB array key exceeds the maximum nesting depth";

/// A Number key cannot contain NaN. Preserve its bits for value roundtrips;
/// equality and ordering treat positive and negative zero as the same key.
#[derive(Debug, Clone, Copy)]
pub struct KeyNumber(u64);

impl KeyNumber {
    pub fn new(value: f64) -> Option<Self> {
        (!value.is_nan()).then_some(Self(value.to_bits()))
    }

    pub fn value(self) -> f64 {
        f64::from_bits(self.0)
    }
}

impl PartialEq for KeyNumber {
    fn eq(&self, other: &Self) -> bool {
        self.value() == other.value()
    }
}

impl Eq for KeyNumber {}

impl Ord for KeyNumber {
    fn cmp(&self, other: &Self) -> Ordering {
        if self == other {
            Ordering::Equal
        } else {
            self.value().total_cmp(&other.value())
        }
    }
}

impl PartialOrd for KeyNumber {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Variant order is IndexedDB key order. Strings compare UTF-16 code units,
/// including unpaired surrogates, and binary keys compare unsigned bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Number(KeyNumber),
    Date(i64),
    String(Vec<u16>),
    Binary(Vec<u8>),
    Array(Vec<Key>),
}

impl Key {
    pub fn number(value: f64) -> Option<Self> {
        KeyNumber::new(value).map(Self::Number)
    }

    /// Reject nesting that would exceed the supported native key depth.
    pub fn validate_depth(&self) -> Result<(), &'static str> {
        self.validate_depth_at(0)
    }

    fn validate_depth_at(&self, depth: usize) -> Result<(), &'static str> {
        if let Self::Array(keys) = self {
            if depth >= MAX_KEY_DEPTH {
                return Err(KEY_DEPTH_ERROR);
            }
            // The depth limit also bounds the native call stack. A wide array
            // must not allocate another Vec containing all its children merely
            // to validate an existing payload.
            for key in keys {
                key.validate_depth_at(depth + 1)?;
            }
        }
        Ok(())
    }
}

impl From<&str> for Key {
    fn from(value: &str) -> Self {
        Self::String(value.encode_utf16().collect())
    }
}

impl From<String> for Key {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<i64> for Key {
    fn from(value: i64) -> Self {
        Self::number(value as f64).expect("integer keys cannot be NaN")
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
    use super::{Key, KeyPath, MAX_KEY_DEPTH};

    #[test]
    fn key_order_preserves_all_key_types() {
        let keys = vec![
            Key::number(f64::NEG_INFINITY).unwrap(),
            Key::number(-1.5).unwrap(),
            Key::from(0),
            Key::number(f64::from_bits(1)).unwrap(),
            Key::number(1.5).unwrap(),
            Key::number(f64::INFINITY).unwrap(),
            Key::Date(-1),
            Key::Date(0),
            Key::from(""),
            Key::from("a"),
            Key::String(vec![0xd800]),
            Key::String(vec![0xd800, 0xdc00]),
            Key::String(vec![0xdc00]),
            Key::from("\u{fffd}"),
            Key::Binary(vec![]),
            Key::Binary(vec![0]),
            Key::Binary(vec![0, 255]),
            Key::Binary(vec![1]),
            Key::Array(vec![]),
            Key::Array(vec![Key::from(0)]),
            Key::Array(vec![Key::Array(vec![])]),
        ];
        for pair in keys.windows(2) {
            assert!(pair[0] < pair[1], "{pair:?}");
        }
        assert!(Key::number(f64::NAN).is_none());
        assert_eq!(Key::number(-0.0), Key::number(0.0));
        assert_eq!(
            Key::number(-0.0).cmp(&Key::number(0.0)),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn maximum_depth_keys_fit_recursive_operations_on_a_bounded_native_stack() {
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(|| {
                let mut key = Key::from(1);
                for _ in 0..MAX_KEY_DEPTH {
                    key = Key::Array(vec![key]);
                }
                let copy = key.clone();
                assert_eq!(key.cmp(&copy), std::cmp::Ordering::Equal);
                let wire = key.encode_value().unwrap();
                assert_eq!(Key::decode_value(&wire).unwrap(), copy);
                drop(key);
                drop(copy);
            })
            .unwrap()
            .join()
            .unwrap();
    }

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
