use std::borrow::Cow;

use thin_vec::ThinVec;

/// A DOMString with a UTF-8 view for parser, layout and serialization consumers.
/// Only strings containing unpaired surrogates retain a second representation.
/// Both representations are updated together, including when concatenation
/// joins two previously unpaired surrogates into a scalar value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomStringValue {
    text: Box<str>,
    unpaired_units: ThinVec<u16>,
}

impl DomStringValue {
    pub fn from_utf16(units: &[u16]) -> Self {
        match String::from_utf16(units) {
            Ok(text) => text.into(),
            Err(_) => Self {
                text: String::from_utf16_lossy(units).into_boxed_str(),
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
            let mut text = self.text.to_string();
            text.push_str(&other.text);
            self.text = text.into_boxed_str();
        } else {
            let mut units = self.utf16_units().into_owned();
            units.extend_from_slice(&other.utf16_units());
            *self = Self::from_utf16(&units);
        }
    }
}

impl From<String> for DomStringValue {
    fn from(text: String) -> Self {
        Self {
            text: text.into_boxed_str(),
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

#[cfg(test)]
mod tests {
    use super::DomStringValue;

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
}
