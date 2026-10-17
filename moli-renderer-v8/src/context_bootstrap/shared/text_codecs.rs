use crate::text_codec::TextDecodeError;

pub(in crate::context_bootstrap) fn text_decoder_encoding_for_label(
    label: &str,
) -> Option<&'static encoding_rs::Encoding> {
    encoding_rs::Encoding::for_label_no_replacement(label.as_bytes())
}

pub(in crate::context_bootstrap) fn canonical_text_decoder_encoding_name(
    encoding: &'static encoding_rs::Encoding,
) -> String {
    encoding.name().to_ascii_lowercase()
}

pub(in crate::context_bootstrap) fn init_text_decoder_state(
    scope: &mut v8::PinScope<'_, '_>,
    decoder: v8::Local<'_, v8::Object>,
    encoding: &'static encoding_rs::Encoding,
    fatal: bool,
    ignore_bom: bool,
) {
    if let Some(common) = crate::common_runtime::CommonRuntimeState::from_scope(scope) {
        common
            .text_codecs
            .init_decoder(scope, decoder, encoding, fatal, ignore_bom);
    }
}

pub(in crate::context_bootstrap) fn decode_text_decoder_state(
    scope: &mut v8::PinScope<'_, '_>,
    decoder_id: u32,
    bytes: &[u8],
    stream: bool,
) -> Option<Result<String, TextDecodeError>> {
    let common = crate::common_runtime::CommonRuntimeState::from_scope(scope)?;
    Some(common.text_codecs.decode(decoder_id, bytes, stream))
}
