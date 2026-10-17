use crate::{text_codec::TextDecodeError, util::context_host_ptr_from_global_bridge};

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
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }
            .text_codecs_mut()
            .init_decoder(scope, decoder, encoding, fatal, ignore_bom);
        return;
    }
    if let Some(worker_state) = crate::worker::get_worker_state(scope) {
        worker_state
            .borrow_mut()
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
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        return Some(
            unsafe { &mut *host_ptr }
                .text_codecs_mut()
                .decode(decoder_id, bytes, stream),
        );
    }
    crate::worker::get_worker_state(scope).map(|worker_state| {
        worker_state
            .borrow_mut()
            .text_codecs
            .decode(decoder_id, bytes, stream)
    })
}
