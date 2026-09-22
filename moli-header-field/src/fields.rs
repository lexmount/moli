use anyhow::{Context, Result};

/// Decode HTTP bytes isomorphically, without treating valid UTF-8 sequences specially.
pub fn decode_header_value(value: &[u8]) -> std::borrow::Cow<'_, str> {
    if value.is_ascii() {
        std::borrow::Cow::Borrowed(std::str::from_utf8(value).expect("ASCII is valid UTF-8"))
    } else {
        std::borrow::Cow::Owned(value.iter().copied().map(char::from).collect())
    }
}

/// Project a raw header list onto a WebIDL ByteString or protocol text boundary.
pub fn headers_to_byte_strings(headers: &[(String, Vec<u8>)]) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.clone(), decode_header_value(value).into_owned()))
        .collect()
}

/// Recover raw bytes from a WebIDL ByteString header list.
pub fn headers_from_byte_strings(headers: &[(String, String)]) -> Result<Vec<(String, Vec<u8>)>> {
    headers
        .iter()
        .map(|(name, value)| Ok((name.clone(), header_value_from_byte_string(value)?)))
        .collect()
}

/// Recover HTTP value bytes from an isomorphic string, without UTF-8 encoding.
pub fn header_value_from_byte_string(value: &str) -> Result<Vec<u8>> {
    value
        .chars()
        .map(|ch| u8::try_from(u32::from(ch)))
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("HTTP header ByteString contains a non-byte character")
}

/// Read persisted raw headers, including the legacy isomorphic string representation.
pub fn deserialize_headers<'de, D>(deserializer: D) -> Result<Vec<(String, Vec<u8>)>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Bytes(Vec<u8>),
        ByteString(String),
    }
    Vec::<(String, Value)>::deserialize(deserializer)?
        .into_iter()
        .map(|(name, value)| {
            let bytes = match value {
                Value::Bytes(bytes) => bytes,
                Value::ByteString(value) => {
                    header_value_from_byte_string(&value).map_err(serde::de::Error::custom)?
                }
            };
            Ok((name, bytes))
        })
        .collect()
}

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
        headers_from_byte_strings(headers).map(Self)
    }

    /// A lossless string projection for Fetch and network observation surfaces.
    pub fn to_byte_strings(&self) -> Vec<(String, String)> {
        headers_to_byte_strings(&self.0)
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
    fn persisted_headers_read_legacy_byte_strings_and_write_raw_bytes() {
        #[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
        struct Record {
            #[serde(deserialize_with = "deserialize_headers")]
            headers: Vec<(String, Vec<u8>)>,
        }
        let legacy = r#"{"headers":[["X-Bytes","ÿ"],["X-Bytes","Ã¿"],["X-Empty",""]]}"#;
        let record: Record = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            record.headers,
            vec![
                ("X-Bytes".into(), vec![0xff]),
                ("X-Bytes".into(), vec![0xc3, 0xbf]),
                ("X-Empty".into(), vec![]),
            ]
        );
        let encoded = serde_json::to_value(&record).unwrap();
        assert_eq!(encoded["headers"][0][1], serde_json::json!([255]));
        assert_eq!(serde_json::from_value::<Record>(encoded).unwrap(), record);
        assert!(serde_json::from_str::<Record>(r#"{"headers":[["X-Invalid","中"]]}"#).is_err());
    }

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
