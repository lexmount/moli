use anyhow::{Context, Result};

/// Ordered HTTP field names and encoded values, including duplicate fields.
///
/// Ordinary Rust strings use UTF-8. WebIDL ByteStrings must enter through
/// `from_byte_strings` before headers are merged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct HeaderFields(Vec<(String, Vec<u8>)>);

impl HeaderFields {
    /// Encode ordinary Rust or protocol Unicode strings as UTF-8.
    pub fn from_utf8(headers: Vec<(String, String)>) -> Self {
        Self(
            headers
                .into_iter()
                .map(|(name, value)| (name, value.into_bytes()))
                .collect(),
        )
    }

    /// Replace fields with matching names while preserving replacement order and duplicates.
    pub fn overlay(&mut self, headers: Self) {
        self.0.retain(|(name, _)| {
            !headers
                .iter()
                .any(|(other, _)| name.eq_ignore_ascii_case(other))
        });
        self.0.extend(headers);
    }

    pub fn from_bytes(headers: Vec<(String, Vec<u8>)>) -> Self {
        Self(headers)
    }

    pub fn from_byte_strings(headers: &[(String, String)]) -> Result<Self> {
        headers
            .iter()
            .map(|(name, value)| {
                let bytes = value
                    .chars()
                    .map(|ch| u8::try_from(u32::from(ch)))
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .context("HTTP header ByteString contains a non-byte character")?;
                Ok((name.clone(), bytes))
            })
            .collect::<Result<Vec<_>>>()
            .map(Self)
    }

    /// A lossless string projection for Fetch and network observation surfaces.
    pub fn to_byte_strings(&self) -> Vec<(String, String)> {
        self.0
            .iter()
            .map(|(name, value)| {
                (
                    name.clone(),
                    value.iter().copied().map(char::from).collect(),
                )
            })
            .collect()
    }
}

impl From<Vec<(String, String)>> for HeaderFields {
    fn from(headers: Vec<(String, String)>) -> Self {
        Self::from_utf8(headers)
    }
}

impl IntoIterator for HeaderFields {
    type Item = (String, Vec<u8>);
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a HeaderFields {
    type Item = &'a (String, Vec<u8>);
    type IntoIter = std::slice::Iter<'a, (String, Vec<u8>)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl std::ops::Deref for HeaderFields {
    type Target = Vec<(String, Vec<u8>)>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for HeaderFields {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_encodings_are_explicit() {
        let headers = vec![("X-Test".into(), "é".into())];
        assert_eq!(HeaderFields::from(headers.clone())[0].1, [0xc3, 0xa9]);
        assert_eq!(
            HeaderFields::from_byte_strings(&headers).unwrap()[0].1,
            [0xe9]
        );
        assert!(HeaderFields::from_byte_strings(&[("X-Test".into(), "中".into())]).is_err());
        assert!(HeaderFields::from_byte_strings(&[("X-Test".into(), "\u{100}".into())]).is_err());
    }
    #[test]
    fn overlay_keeps_opaque_values_duplicates_and_order() {
        let mut headers = HeaderFields::from_utf8(vec![
            ("X-Replaced".into(), "old".into()),
            ("X-Kept".into(), "é".into()),
        ]);
        headers.overlay(HeaderFields::from_bytes(vec![
            ("x-replaced".into(), vec![0xe9]),
            ("X-Other".into(), vec![0xff]),
            ("X-Replaced".into(), vec![0xc3, 0xa9]),
        ]));
        let expected = HeaderFields::from_bytes(vec![
            ("X-Kept".into(), vec![0xc3, 0xa9]),
            ("x-replaced".into(), vec![0xe9]),
            ("X-Other".into(), vec![0xff]),
            ("X-Replaced".into(), vec![0xc3, 0xa9]),
        ]);
        assert_eq!(headers, expected);
        assert_eq!(
            HeaderFields::from_byte_strings(&headers.to_byte_strings()).unwrap(),
            expected
        );
    }
}
