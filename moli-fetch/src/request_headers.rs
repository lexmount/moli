use anyhow::{Context, Result};

/// Encoded request header values. Ordinary Rust strings use UTF-8; WebIDL
/// ByteStrings must enter through `from_byte_strings` before headers are merged.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RequestHeaders(Vec<(String, Vec<u8>)>);

impl RequestHeaders {
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
                    crate::headers::decode_http_header_bytes(value).into_owned(),
                )
            })
            .collect()
    }
}

impl From<Vec<(String, String)>> for RequestHeaders {
    fn from(headers: Vec<(String, String)>) -> Self {
        Self(
            headers
                .into_iter()
                .map(|(name, value)| (name, value.into_bytes()))
                .collect(),
        )
    }
}

impl std::ops::Deref for RequestHeaders {
    type Target = Vec<(String, Vec<u8>)>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for RequestHeaders {
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
        assert_eq!(RequestHeaders::from(headers.clone())[0].1, [0xc3, 0xa9]);
        assert_eq!(
            RequestHeaders::from_byte_strings(&headers).unwrap()[0].1,
            [0xe9]
        );
        assert!(RequestHeaders::from_byte_strings(&[("X-Test".into(), "中".into())]).is_err());
        assert!(RequestHeaders::from_byte_strings(&[("X-Test".into(), "\u{100}".into())]).is_err());
    }
}
