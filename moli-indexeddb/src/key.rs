use std::cmp::Ordering;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
}

// The original on-disk format used Integer and UTF-8 String variants. Continue
// reading those while storing Number bits (JSON cannot represent infinities)
// and UTF-16 units without loss.
#[derive(Deserialize)]
enum StoredKey {
    Integer(i64),
    String(String),
    Number(u64),
    Date(i64),
    String16(Vec<u16>),
    Binary(Vec<u8>),
    Array(Vec<Key>),
    ArrayLength(usize),
    ArrayParts(Vec<StoredKey>),
}

#[derive(Serialize)]
enum StoredKeyRef<'a> {
    Number(u64),
    Date(i64),
    String16(&'a [u16]),
    Binary(&'a [u8]),
    ArrayLength(usize),
    ArrayParts(Vec<StoredKeyRef<'a>>),
}

impl Serialize for Key {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Number(number) => StoredKeyRef::Number(number.0),
            Self::Date(ms) => StoredKeyRef::Date(*ms),
            Self::String(units) => StoredKeyRef::String16(units),
            Self::Binary(bytes) => StoredKeyRef::Binary(bytes),
            Self::Array(_) => StoredKeyRef::ArrayParts(flat_key_parts(self)),
        }
        .serialize(serializer)
    }
}

// Encode compound keys in prefix order. JSON nesting stays constant even for
// deep array keys, so a successful commit cannot make the origin unreadable
// on the next open merely because serde_json has a recursion limit.
fn flat_key_parts(key: &Key) -> Vec<StoredKeyRef<'_>> {
    let mut pending = vec![key];
    let mut parts = Vec::new();
    while let Some(key) = pending.pop() {
        parts.push(match key {
            Key::Number(value) => StoredKeyRef::Number(value.0),
            Key::Date(value) => StoredKeyRef::Date(*value),
            Key::String(units) => StoredKeyRef::String16(units),
            Key::Binary(bytes) => StoredKeyRef::Binary(bytes),
            Key::Array(keys) => {
                pending.extend(keys.iter().rev());
                StoredKeyRef::ArrayLength(keys.len())
            }
        });
    }
    parts
}

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        decode_stored_key(StoredKey::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

fn decode_stored_key(key: StoredKey) -> Result<Key, &'static str> {
    Ok(match key {
        StoredKey::Integer(value) => Key::from(value),
        StoredKey::String(value) => Key::from(value),
        StoredKey::Number(bits) => {
            Key::number(f64::from_bits(bits)).ok_or("NaN is not an IndexedDB key")?
        }
        StoredKey::Date(ms) => {
            if !(-8_640_000_000_000_000..=8_640_000_000_000_000).contains(&ms) {
                return Err("invalid IndexedDB Date key");
            }
            Key::Date(ms)
        }
        StoredKey::String16(units) => Key::String(units),
        StoredKey::Binary(bytes) => Key::Binary(bytes),
        StoredKey::Array(keys) => Key::Array(keys),
        StoredKey::ArrayParts(parts) => decode_array_parts(parts)?,
        StoredKey::ArrayLength(_) => return Err("array length outside a compound key"),
    })
}

fn decode_array_parts(parts: Vec<StoredKey>) -> Result<Key, &'static str> {
    if !matches!(parts.first(), Some(StoredKey::ArrayLength(_))) {
        return Err("compound key must start with an array");
    }
    let mut frames: Vec<(usize, Vec<Key>)> = Vec::new();
    let mut result = None;
    for part in parts {
        if result.is_some() {
            return Err("trailing compound key parts");
        }
        let mut key = match part {
            StoredKey::ArrayLength(0) => Key::Array(Vec::new()),
            StoredKey::ArrayLength(length) => {
                // Do not allocate from an untrusted declared length.
                frames.push((length, Vec::new()));
                continue;
            }
            StoredKey::Array(_) | StoredKey::ArrayParts(_) => {
                return Err("nested compound key encoding");
            }
            part => decode_stored_key(part)?,
        };
        loop {
            let Some((length, keys)) = frames.last_mut() else {
                result = Some(key);
                break;
            };
            keys.push(key);
            if keys.len() != *length {
                break;
            }
            key = Key::Array(frames.pop().unwrap().1);
        }
    }
    result.ok_or("truncated compound key")
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
    use super::{Key, KeyPath};

    #[test]
    fn key_order_and_json_roundtrip_preserve_all_key_types() {
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
        let wire = serde_json::to_vec(&keys).unwrap();
        assert_eq!(serde_json::from_slice::<Vec<Key>>(&wire).unwrap(), keys);
        assert!(Key::number(f64::NAN).is_none());
        assert_eq!(Key::number(-0.0), Key::number(0.0));
        assert_eq!(
            Key::number(-0.0).cmp(&Key::number(0.0)),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn deep_compound_keys_use_a_flat_validated_storage_encoding() {
        let mut key = Key::from(1);
        for _ in 0..100 {
            key = Key::Array(vec![key]);
        }
        let wire = serde_json::to_vec(&key).unwrap();
        assert_eq!(serde_json::from_slice::<Key>(&wire).unwrap(), key);
        for wire in [
            r#"{"ArrayParts":[]}"#,
            r#"{"ArrayParts":[{"Integer":1}]}"#,
            r#"{"ArrayParts":[{"ArrayLength":1}]}"#,
            r#"{"ArrayParts":[{"ArrayLength":0},{"Integer":1}]}"#,
            r#"{"ArrayParts":[{"ArrayLength":1},{"Array":[]}]}"#,
            r#"{"ArrayLength":0}"#,
        ] {
            assert!(serde_json::from_str::<Key>(wire).is_err(), "{wire}");
        }
    }

    #[test]
    fn legacy_keys_decode_into_the_same_ordered_key_model() {
        let key: Key =
            serde_json::from_str(r#"{"Array":[{"Integer":7},{"String":"a😀"}]}"#).unwrap();
        assert_eq!(key, Key::Array(vec![Key::from(7), Key::from("a😀")]));
        let nan = format!(r#"{{"Number":{}}}"#, f64::NAN.to_bits());
        assert!(serde_json::from_str::<Key>(&nan).is_err());
        assert!(serde_json::from_str::<Key>(r#"{"Date":8640000000000001}"#).is_err());
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
