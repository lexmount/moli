use base64::{
    Engine as _, alphabet,
    engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
};

use super::value::Text;

// WebAuthn base64url has no padding. Unused bits in the last digit need not be
// zero, but whitespace and the ordinary base64 alphabet are invalid. Callers
// supply the API-specific exception: JSON uses EncodingError, signals TypeError.
const BASE64URL: GeneralPurpose = GeneralPurpose::new(
    &alphabet::URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::RequireNone)
        .with_decode_allow_trailing_bits(true),
);

pub(super) fn decode(text: &Text) -> Option<Vec<u8>> {
    let ascii = text
        .0
        .0
        .iter()
        .map(|&unit| (unit <= 0x7f).then_some(unit as u8))
        .collect::<Option<Vec<_>>>()?;
    BASE64URL.decode(ascii).ok()
}
