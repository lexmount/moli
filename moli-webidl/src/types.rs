use crate::WebIdlDictionary;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct EnumValue<T>(pub T);

/// WebIDL `DOMString` represented at the Rust boundary as UTF-8 `String`.
///
/// This intentionally follows Deno's practical boundary model rather than
/// Servo's lazy JSString-backed storage model: JavaScript values are converted
/// with WebIDL `ToString`, then stored as Rust text. Raw WTF-16 code units such
/// as lone surrogates are not preserved once the value crosses into Rust.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DomString(pub String);

/// Lossless WebIDL DOMString storage for APIs whose identity depends on the
/// original UTF-16 units, including unpaired surrogates.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DomString16(pub Vec<u16>);

impl From<DomString> for String {
    fn from(value: DomString) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UsvString(pub String);

impl From<UsvString> for String {
    fn from(value: UsvString) -> Self {
        value.0
    }
}

/// WebIDL `ByteString` represented as raw bytes at the Rust boundary.
///
/// This follows the Deno/Servo-style model for byte-oriented WebIDL strings:
/// conversion rejects JS string code points above `0xff`, then stores each
/// accepted code point as one byte. Consumers that still need a Rust `String`
/// can project the bytes back to the corresponding U+0000..U+00FF scalar
/// values with `into_scalar_string`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ByteString(pub Vec<u8>);

impl ByteString {
    pub fn from_scalar_string(value: impl AsRef<str>) -> Self {
        let bytes = value
            .as_ref()
            .chars()
            .map(|ch| {
                let code_point = ch as u32;
                assert!(
                    code_point <= 0xff,
                    "ByteString scalar string contains a code point above 0xff"
                );
                code_point as u8
            })
            .collect();
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn to_scalar_string(&self) -> String {
        self.0.iter().map(|byte| char::from(*byte)).collect()
    }

    pub fn into_scalar_string(self) -> String {
        self.0.into_iter().map(char::from).collect()
    }
}

impl From<ByteString> for String {
    fn from(value: ByteString) -> Self {
        value.into_scalar_string()
    }
}

impl From<ByteString> for Vec<u8> {
    fn from(value: ByteString) -> Self {
        value.0
    }
}

impl From<Vec<u8>> for ByteString {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl From<&[u8]> for ByteString {
    fn from(value: &[u8]) -> Self {
        Self(value.to_vec())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BufferSource(pub Vec<u8>);

impl BufferSource {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

impl From<BufferSource> for Vec<u8> {
    fn from(value: BufferSource) -> Self {
        value.0
    }
}

impl From<Vec<u8>> for BufferSource {
    fn from(value: Vec<u8>) -> Self {
        Self(value)
    }
}

impl From<&[u8]> for BufferSource {
    fn from(value: &[u8]) -> Self {
        Self(value.to_vec())
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Boolean(pub bool);

impl From<Boolean> for bool {
    fn from(value: Boolean) -> Self {
        value.0
    }
}

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq)]
pub struct EventListenerOptions {
    pub capture: bool,
    pub once: bool,
    pub passive: Option<bool>,
}

#[derive(WebIdlDictionary)]
#[webidl(prefix = "EventListenerOptions")]
pub(crate) struct EventListenerOptionsMembers {
    #[webidl(default = false)]
    pub(crate) capture: bool,
}

#[derive(WebIdlDictionary)]
#[webidl(prefix = "AddEventListenerOptions")]
pub(crate) struct AddEventListenerOptionsMembers {
    #[webidl(default = false)]
    pub(crate) capture: bool,
    #[webidl(default = false)]
    pub(crate) once: bool,
    pub(crate) passive: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnrestrictedDouble(pub f64);

impl From<UnrestrictedDouble> for f64 {
    fn from(value: UnrestrictedDouble) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Double(pub f64);

impl From<Double> for f64 {
    fn from(value: Double) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Long(pub i32);

impl From<Long> for i32 {
    fn from(value: Long) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct EnforceRangeLong(pub i32);

impl From<EnforceRangeLong> for i32 {
    fn from(value: EnforceRangeLong) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct UnsignedLong(pub u32);

impl From<UnsignedLong> for u32 {
    fn from(value: UnsignedLong) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct EnforceRangeUnsignedLong(pub u32);

impl From<EnforceRangeUnsignedLong> for u32 {
    fn from(value: EnforceRangeUnsignedLong) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct UnsignedShort(pub u16);

impl From<UnsignedShort> for u16 {
    fn from(value: UnsignedShort) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ClampedUnsignedShort(pub u16);

impl From<ClampedUnsignedShort> for u16 {
    fn from(value: ClampedUnsignedShort) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct UnsignedLongLong(pub u64);

impl From<UnsignedLongLong> for u64 {
    fn from(value: UnsignedLongLong) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct EnforceRangeUnsignedLongLong(pub u64);

impl From<EnforceRangeUnsignedLongLong> for u64 {
    fn from(value: EnforceRangeUnsignedLongLong) -> Self {
        value.0
    }
}

#[derive(Debug, Default)]
pub struct StringOptions {
    pub treat_null_as_empty_string: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sequence<T>(pub Vec<T>);

#[derive(Debug, Clone, PartialEq)]
pub struct Record<K, V>(pub Vec<(K, V)>);

#[cfg(test)]
mod tests {
    use super::ByteString;

    #[derive(Debug, Eq, PartialEq, crate::WebIdlEnum)]
    #[webidl(name = "TestEnum", rename_all = "kebab-case")]
    enum TestEnum {
        SameOrigin,
        #[webidl(token = "")]
        Empty,
    }

    #[derive(Debug, Eq, PartialEq, crate::WebIdlEnum)]
    #[webidl(name = "ParsedEnum", parse_with = parse_parsed_enum)]
    enum ParsedEnum {
        Value,
    }

    fn parse_parsed_enum(value: &str) -> Option<ParsedEnum> {
        (value == "value").then_some(ParsedEnum::Value)
    }

    #[test]
    fn byte_string_projects_bytes_to_scalar_string() {
        let value = ByteString(vec![b'H', 0xe9, 0xff]);

        assert_eq!(value.as_bytes(), &[b'H', 0xe9, 0xff]);
        assert_eq!(value.to_scalar_string(), "H\u{00e9}\u{00ff}");
        assert_eq!(String::from(value), "H\u{00e9}\u{00ff}");
    }

    #[test]
    fn byte_string_from_scalar_string_uses_low_byte_values() {
        let value = ByteString::from_scalar_string("A\u{00a3}");

        assert_eq!(value.into_bytes(), vec![b'A', 0xa3]);
    }

    #[test]
    fn webidl_enum_derive_parses_declared_tokens() {
        assert_eq!(
            <TestEnum as crate::WebIdlEnum>::parse_token("same-origin"),
            Some(TestEnum::SameOrigin)
        );
        assert_eq!(
            <TestEnum as crate::WebIdlEnum>::parse_token(""),
            Some(TestEnum::Empty)
        );
        assert_eq!(
            <TestEnum as crate::WebIdlEnum>::parse_token("SameOrigin"),
            None
        );
    }

    #[test]
    fn webidl_enum_derive_uses_custom_parser() {
        assert_eq!(
            <ParsedEnum as crate::WebIdlEnum>::parse_token("value"),
            Some(ParsedEnum::Value)
        );
        assert_eq!(
            <ParsedEnum as crate::WebIdlEnum>::parse_token("other"),
            None
        );
    }
}
