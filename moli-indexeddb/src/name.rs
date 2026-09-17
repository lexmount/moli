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

    pub(crate) fn usage_bytes(&self) -> u64 {
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
