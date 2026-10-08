use std::borrow::Cow;

use thin_vec::ThinVec;

/// A DOMString with a UTF-8 view for parser, layout and serialization consumers.
/// Only strings containing unpaired surrogates retain a second representation.
/// Both representations are updated together, including when concatenation
/// joins two previously unpaired surrogates into a scalar value.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct DomStringValue {
    text: String,
    unpaired_units: ThinVec<u16>,
}

impl DomStringValue {
    pub fn from_utf16(units: &[u16]) -> Self {
        match String::from_utf16(units) {
            Ok(text) => text.into(),
            Err(_) => Self {
                text: String::from_utf16_lossy(units),
                unpaired_units: units.iter().copied().collect(),
            },
        }
    }

    pub fn as_str_lossy(&self) -> &str {
        &self.text
    }

    /// Returns the scalar string only when doing so preserves every UTF-16 unit.
    pub fn as_str(&self) -> Option<&str> {
        self.unpaired_units.is_empty().then_some(&self.text)
    }

    pub fn utf16_units(&self) -> Cow<'_, [u16]> {
        if self.unpaired_units.is_empty() {
            Cow::Owned(self.text.encode_utf16().collect())
        } else {
            Cow::Borrowed(&self.unpaired_units)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn append_utf16_units_to(&self, units: &mut Vec<u16>) {
        if self.unpaired_units.is_empty() {
            units.extend(self.text.encode_utf16());
        } else {
            units.extend_from_slice(&self.unpaired_units);
        }
    }

    pub fn append(&mut self, other: &Self) {
        if self.unpaired_units.is_empty() && other.unpaired_units.is_empty() {
            self.text.push_str(&other.text);
        } else {
            let mut units = self.utf16_units().into_owned();
            units.extend_from_slice(&other.utf16_units());
            *self = Self::from_utf16(&units);
        }
    }

    /// Append scalar parser text without discarding capacity or original units.
    /// A scalar string cannot begin with an unpaired low surrogate, so no
    /// surrogate pair can form across this boundary.
    pub fn append_str(&mut self, other: &str) {
        self.text.push_str(other);
        if !self.unpaired_units.is_empty() {
            self.unpaired_units.extend(other.encode_utf16());
        }
    }

    pub fn replace_utf16_range(&self, start: usize, count: usize, replacement: &[u16]) -> Self {
        let mut units = self.utf16_units().into_owned();
        let start = start.min(units.len());
        let end = start.saturating_add(count).min(units.len());
        units.splice(start..end, replacement.iter().copied());
        Self::from_utf16(&units)
    }
}

impl From<String> for DomStringValue {
    fn from(text: String) -> Self {
        Self {
            text,
            unpaired_units: ThinVec::new(),
        }
    }
}

impl From<&str> for DomStringValue {
    fn from(text: &str) -> Self {
        text.to_owned().into()
    }
}

impl From<&String> for DomStringValue {
    fn from(text: &String) -> Self {
        text.as_str().into()
    }
}

impl From<&DomStringValue> for DomStringValue {
    fn from(value: &DomStringValue) -> Self {
        value.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::DomStringValue;

    #[test]
    fn scalar_append_retains_reserved_capacity() {
        let mut text = String::with_capacity(256);
        text.push_str("start");
        let pointer = text.as_ptr();
        let mut value = DomStringValue::from(text);
        for _ in 0..16 {
            value.append_str("中文🙂");
            assert_eq!(value.as_str_lossy().as_ptr(), pointer);
        }
        assert_eq!(
            value.as_str(),
            Some(format!("start{}", "中文🙂".repeat(16)).as_str())
        );
    }

    #[test]
    fn scalar_append_preserves_unpaired_utf16_units() {
        for original in [&[0xd800][..], &[0xdc00], &[0x61, 0xd800, 0x62]] {
            let mut value = DomStringValue::from_utf16(original);
            value.append_str("\0😀tail");
            let mut expected = original.to_vec();
            expected.extend("\0😀tail".encode_utf16());
            assert_eq!(&*value.utf16_units(), expected);
            assert_eq!(value, DomStringValue::from_utf16(&expected));
        }
    }

    #[test]
    fn utf16_values_preserve_unpaired_surrogates_and_compare_original_units() {
        for units in [&[0xd800][..], &[0xdc00], &[0x61, 0xd801, 0x62]] {
            let value = DomStringValue::from_utf16(units);
            assert_eq!(&*value.utf16_units(), units);
            assert_eq!(value.clone(), value);
            assert_ne!(value, DomStringValue::from(value.as_str_lossy()));
        }
        assert_ne!(
            DomStringValue::from_utf16(&[0xd800]),
            DomStringValue::from_utf16(&[0xd801])
        );
    }

    #[test]
    fn scalar_values_have_the_same_identity_from_utf8_and_utf16() {
        for text in ["", "hello", "\0", "中文", "😀", "\u{fffd}"] {
            let units = text.encode_utf16().collect::<Vec<_>>();
            assert_eq!(
                DomStringValue::from(text),
                DomStringValue::from_utf16(&units)
            );
        }
    }

    #[test]
    fn concatenation_preserves_units_and_joins_split_surrogate_pairs() {
        let mut value = DomStringValue::from_utf16(&[0xd83d]);
        value.append(&DomStringValue::from_utf16(&[0xde00]));
        assert_eq!(value, DomStringValue::from("😀"));
        value.append(&DomStringValue::from_utf16(&[0xd800]));
        value.append(&DomStringValue::from("text"));
        assert_eq!(
            &*value.utf16_units(),
            &[0xd83d, 0xde00, 0xd800, 0x74, 0x65, 0x78, 0x74]
        );
    }

    #[test]
    fn replacing_code_units_can_split_and_rejoin_surrogate_pairs() {
        let original = DomStringValue::from("😀");
        let split = original.replace_utf16_range(1, 1, &[0x78]);
        assert_eq!(&*split.utf16_units(), &[0xd83d, 0x78]);
        assert_eq!(split.replace_utf16_range(1, 1, &[0xde00]), original);
        assert_eq!(
            DomStringValue::from_utf16(&[0xd800]).replace_utf16_range(0, 1, &[0xdc00]),
            DomStringValue::from_utf16(&[0xdc00])
        );
    }
}
