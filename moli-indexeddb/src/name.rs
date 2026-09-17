use std::fmt;

/// An IndexedDB DOMString name, ordered by UTF-16 code units. Unlike Rust
/// text, this preserves unpaired surrogates and keeps them distinct from U+FFFD.
#[derive(Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct IndexedDbName(Vec<u16>);

impl IndexedDbName {
    pub fn from_utf16(units: Vec<u16>) -> Self {
        Self(units)
    }

    pub fn as_utf16(&self) -> &[u16] {
        &self.0
    }

    pub fn usage_bytes(&self) -> u64 {
        // Retain UTF-8 accounting for existing names, using three bytes for
        // each unpaired surrogate as in WTF-8.
        char::decode_utf16(self.0.iter().copied())
            .map(|value| value.map_or(3, char::len_utf8) as u64)
            .sum()
    }
}

impl From<&str> for IndexedDbName {
    fn from(value: &str) -> Self {
        Self(value.encode_utf16().collect())
    }
}

impl From<String> for IndexedDbName {
    fn from(value: String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<&String> for IndexedDbName {
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<&IndexedDbName> for IndexedDbName {
    fn from(value: &IndexedDbName) -> Self {
        value.clone()
    }
}

impl fmt::Display for IndexedDbName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // This projection is only for diagnostics, never identity or storage.
        formatter.write_str(&String::from_utf16_lossy(&self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::{BTreeSet, HashSet},
        hash::{Hash, Hasher},
    };

    #[test]
    fn native_names_keep_utf16_identity_and_diagnostic_projection_separate() {
        let names = [
            IndexedDbName::from_utf16(vec![0xd800]),
            IndexedDbName::from_utf16(vec![0xd801]),
            IndexedDbName::from("\u{fffd}"),
        ];
        assert_eq!(
            names.iter().map(ToString::to_string).collect::<Vec<_>>(),
            vec!["\u{fffd}"; 3]
        );
        assert_eq!(names.iter().cloned().collect::<HashSet<_>>().len(), 3);
        assert_eq!(names.iter().cloned().collect::<BTreeSet<_>>().len(), 3);
        for name in names {
            let copied = IndexedDbName::from(&name);
            assert_eq!(copied, name);
            assert_eq!(copied.as_utf16(), name.as_utf16());
            let hash = |value: &IndexedDbName| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                value.hash(&mut h);
                h.finish()
            };
            assert_eq!(hash(&copied), hash(&name));
        }
    }

    #[test]
    fn native_names_use_code_unit_order_and_wtf8_usage_accounting() {
        let astral = IndexedDbName::from("\u{10000}");
        let bmp = IndexedDbName::from("\u{e000}");
        assert!(astral < bmp);
        for (units, bytes) in [
            (vec![], 0),
            (vec![0], 1),
            (vec![0x7f], 1),
            (vec![0x80], 2),
            (vec![0xe000], 3),
            (vec![0xd800, 0xdc00], 4),
            (vec![0xd800, 0x61, 0xdc00], 7),
        ] {
            assert_eq!(IndexedDbName::from_utf16(units).usage_bytes(), bytes);
        }
    }
}
