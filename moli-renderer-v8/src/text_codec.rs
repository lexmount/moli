use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use encoding_rs::{CoderResult, Decoder, DecoderResult, Encoding};
use parking_lot::Mutex;

use super::util::{get_private_value, set_private_value};

const TEXT_DECODER_ID_SLOT: &str = "__lmTextDecoderId";

#[derive(Default)]
pub(super) struct TextEncoderState {
    leading_surrogate: Option<u16>,
}

impl TextEncoderState {
    pub(super) fn new(leading_surrogate: Option<u16>) -> Self {
        debug_assert!(leading_surrogate.is_none_or(|unit| (0xd800..=0xdbff).contains(&unit)));
        Self { leading_surrogate }
    }

    pub(super) fn leading_surrogate(&self) -> Option<u16> {
        self.leading_surrogate
    }

    pub(super) fn encode(&mut self, input: &[u16]) -> Vec<u8> {
        if input.is_empty() {
            return Vec::new();
        }
        let previous = self.leading_surrogate.take();
        let body = match input.split_last() {
            Some((last, rest)) if (0xd800..=0xdbff).contains(last) => {
                self.leading_surrogate = Some(*last);
                rest
            }
            _ => input,
        };
        char::decode_utf16(previous.into_iter().chain(body.iter().copied()))
            .map(|result| result.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect::<String>()
            .into_bytes()
    }

    pub(super) fn flush(&mut self) -> Vec<u8> {
        if self.leading_surrogate.take().is_some() {
            vec![0xef, 0xbf, 0xbd]
        } else {
            Vec::new()
        }
    }
}

#[derive(Default)]
pub(super) struct TextCodecStore {
    state: Arc<Mutex<TextCodecStoreState>>,
}

#[derive(Default)]
struct TextCodecStoreState {
    next_decoder_id: u32,
    decoders: HashMap<u32, TextDecoderState>,
}

#[derive(Default)]
pub(super) struct TextDecoderState {
    encoding: Option<&'static Encoding>,
    decoder: Option<Decoder>,
    fatal: bool,
    ignore_bom: bool,
}

#[derive(Debug)]
pub(super) enum TextDecodeError {
    MalformedInput,
}

impl TextDecodeError {
    pub(super) fn message(&self) -> &'static str {
        match self {
            Self::MalformedInput => "The encoded data was not valid.",
        }
    }
}

impl TextCodecStore {
    pub(super) fn init_decoder(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        decoder: v8::Local<'_, v8::Object>,
        encoding: &'static Encoding,
        fatal: bool,
        ignore_bom: bool,
    ) -> u32 {
        let decoder_id = {
            let mut state = self.state.lock();
            let decoder_id = state.alloc_decoder_id();
            state.decoders.insert(
                decoder_id,
                TextDecoderState::new(encoding, fatal, ignore_bom),
            );
            decoder_id
        };
        set_private_value(
            scope,
            decoder,
            TEXT_DECODER_ID_SLOT,
            v8::Number::new(scope, decoder_id as f64).into(),
        );
        let state = Arc::downgrade(&self.state);
        crate::v8_finalizer::track_context_owned_v8_finalizer(scope, decoder, move || {
            remove_decoder_state(&state, decoder_id);
        });
        decoder_id
    }

    pub(super) fn decode(
        &self,
        decoder_id: u32,
        bytes: &[u8],
        stream: bool,
    ) -> Result<String, TextDecodeError> {
        self.state
            .lock()
            .decoders
            .get_mut(&decoder_id)
            .map(|state| state.decode(bytes, stream))
            .unwrap_or_else(|| Ok(String::new()))
    }

    pub(super) fn decoder_id_from_object<'s>(
        scope: &mut v8::PinScope<'s, '_>,
        object: v8::Local<'s, v8::Object>,
    ) -> Option<u32> {
        get_private_value(scope, object, TEXT_DECODER_ID_SLOT)
            .and_then(|value| value.number_value(scope))
            .filter(|value| value.is_finite() && *value >= 1.0)
            .map(|value| value as u32)
    }
}

impl TextCodecStoreState {
    fn alloc_decoder_id(&mut self) -> u32 {
        self.next_decoder_id = self
            .next_decoder_id
            .checked_add(1)
            .expect("TextDecoder id space exhausted");
        self.next_decoder_id
    }
}

fn remove_decoder_state(state: &Weak<Mutex<TextCodecStoreState>>, decoder_id: u32) {
    if let Some(state) = state.upgrade() {
        state.lock().decoders.remove(&decoder_id);
    }
}

impl TextDecoderState {
    fn new(encoding: &'static Encoding, fatal: bool, ignore_bom: bool) -> Self {
        let decoder = Some(new_encoding_decoder(encoding, ignore_bom));
        Self {
            encoding: Some(encoding),
            decoder,
            fatal,
            ignore_bom,
        }
    }

    pub(super) fn decode(&mut self, bytes: &[u8], stream: bool) -> Result<String, TextDecodeError> {
        let last = !stream;
        let mut output = String::new();

        if self.fatal {
            if self.decode_without_replacement(bytes, &mut output, last)? && last {
                self.reset_decoder();
            }
        } else if self.decode_with_replacement(bytes, &mut output, last) && last {
            self.reset_decoder();
        }

        Ok(output)
    }

    fn decode_with_replacement(&mut self, bytes: &[u8], output: &mut String, last: bool) -> bool {
        let mut total_read = 0usize;
        loop {
            let input = &bytes[total_read..];
            let reserve = self
                .decoder
                .as_ref()
                .and_then(|decoder| decoder.max_utf8_buffer_length(input.len()))
                .unwrap_or_else(|| input.len().saturating_mul(3).saturating_add(16));
            output.reserve(reserve);
            let (result, read, _) = self
                .decoder
                .as_mut()
                .expect("TextDecoderState should always own a decoder")
                .decode_to_string(input, output, last);
            total_read += read;
            match result {
                CoderResult::InputEmpty => return true,
                CoderResult::OutputFull => continue,
            }
        }
    }

    fn decode_without_replacement(
        &mut self,
        bytes: &[u8],
        output: &mut String,
        last: bool,
    ) -> Result<bool, TextDecodeError> {
        let mut total_read = 0usize;
        loop {
            let input = &bytes[total_read..];
            let reserve = self
                .decoder
                .as_ref()
                .and_then(|decoder| decoder.max_utf8_buffer_length_without_replacement(input.len()))
                .unwrap_or_else(|| input.len().saturating_mul(3).saturating_add(16));
            output.reserve(reserve);
            let (result, read) = self
                .decoder
                .as_mut()
                .expect("TextDecoderState should always own a decoder")
                .decode_to_string_without_replacement(input, output, last);
            total_read += read;
            match result {
                DecoderResult::InputEmpty => return Ok(true),
                DecoderResult::OutputFull => continue,
                DecoderResult::Malformed(_, _) => {
                    self.reset_decoder();
                    return Err(TextDecodeError::MalformedInput);
                }
            }
        }
    }

    fn reset_decoder(&mut self) {
        if let Some(encoding) = self.encoding {
            self.decoder = Some(new_encoding_decoder(encoding, self.ignore_bom));
        }
    }
}

fn new_encoding_decoder(encoding: &'static Encoding, ignore_bom: bool) -> Decoder {
    if ignore_bom {
        encoding.new_decoder_without_bom_handling()
    } else {
        encoding.new_decoder_with_bom_removal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_text_codec_state_releases_collected_and_teardown_decoders() {
        use crate::{
            common_runtime::{CommonRuntimeBinding, CommonRuntimeState},
            native_bridge::JsContextHost,
            util::ContextHostLifecycle,
        };
        use std::{cell::Cell, rc::Rc};

        crate::ensure_v8_for_test();
        let common = Rc::new(CommonRuntimeState::default());
        let lifecycle = Rc::new(Cell::new(ContextHostLifecycle::Active));
        let mut isolate = v8::Isolate::new(Default::default());
        let (context, retained_decoder, retained_id) = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            // Component lookup needs an owner identity, but never dereferences
            // the opaque host address in this native backing lifetime test.
            CommonRuntimeBinding::install_window(
                context,
                &common,
                std::ptr::NonNull::<JsContextHost>::dangling().as_ptr(),
                &lifecycle,
            );
            let scope = &mut v8::ContextScope::new(scope, context);
            let collected = v8::Object::new(scope);
            common
                .text_codecs
                .init_decoder(scope, collected, encoding_rs::UTF_8, false, false);
            let retained = v8::Object::new(scope);
            let id =
                common
                    .text_codecs
                    .init_decoder(scope, retained, encoding_rs::UTF_8, false, false);
            assert_eq!(common.text_codecs.state.lock().decoders.len(), 2);
            (
                v8::Global::new(scope, context),
                v8::Global::new(scope, retained),
                id,
            )
        };
        for _ in 0..2 {
            isolate.low_memory_notification();
        }
        assert_eq!(common.text_codecs.state.lock().decoders.len(), 1);
        assert_eq!(
            common
                .text_codecs
                .decode(retained_id, &[0xE2, 0x82], true)
                .unwrap(),
            ""
        );
        assert_eq!(
            common
                .text_codecs
                .decode(retained_id, &[0xAC], false)
                .unwrap(),
            "€"
        );
        common.clear_for_context_teardown();
        assert_eq!(common.text_codecs.state.lock().decoders.len(), 0);
        assert_eq!(common.finalizer_count(), 0);
        common.clear_for_context_teardown();
        drop(retained_decoder);
        drop(context);
        drop(isolate);
    }

    #[test]
    fn text_encoder_state_preserves_utf16_across_all_three_chunk_partitions() {
        let cases: &[&[u16]] = &[
            &[],
            &[0, 0x41, 0x20ac],
            &[0xd800],
            &[0xdfff],
            &[0xd800, 0xdc00],
            &[0xdbff, 0xdfff],
            &[0x41, 0xd83d, 0xdc99, 0x42],
            &[0xd800, 0xd800, 0xdc00, 0xdfff],
            &[0xd800, 0x41, 0xdc00, 0xdbff],
        ];
        for input in cases {
            let expected = String::from_utf16_lossy(input).into_bytes();
            for first in 0..=input.len() {
                for second in first..=input.len() {
                    let mut state = TextEncoderState::default();
                    let mut actual = Vec::new();
                    for chunk in [&input[..first], &input[first..second], &input[second..]] {
                        actual.extend(state.encode(chunk));
                        assert!(state.encode(&[]).is_empty());
                    }
                    actual.extend(state.flush());
                    assert_eq!(actual, expected, "input={input:?}, cuts={first}/{second}");
                    assert!(state.flush().is_empty());
                }
            }
        }
    }

    #[test]
    fn text_decoder_state_streams_split_utf8_sequences() {
        let mut state = TextDecoderState::new(encoding_rs::UTF_8, false, false);

        assert_eq!(state.decode(&[0xE2, 0x82], true).unwrap(), "");
        assert_eq!(state.decode(&[0xAC], false).unwrap(), "\u{20AC}");
    }

    #[test]
    fn text_decoder_state_preserves_multibyte_sequences_across_empty_chunks() {
        let cases: &[(&'static Encoding, &[u8], &str)] = &[
            (encoding_rs::BIG5, &[0xfe, 0x40], "\u{9442}"),
            (encoding_rs::SHIFT_JIS, &[0x81, 0x87], "\u{221e}"),
            (encoding_rs::EUC_KR, &[0x81, 0x41], "\u{ac02}"),
        ];
        for &(encoding, input, expected) in cases {
            for fatal in [false, true] {
                for split in 0..=input.len() {
                    let mut state = TextDecoderState::new(encoding, fatal, false);
                    let mut output = state.decode(&input[..split], true).unwrap();
                    for _ in 0..4 {
                        assert_eq!(state.decode(&[], true).unwrap(), "");
                    }
                    output.push_str(&state.decode(&input[split..], true).unwrap());
                    for _ in 0..4 {
                        assert_eq!(state.decode(&[], true).unwrap(), "");
                    }
                    output.push_str(&state.decode(&[], false).unwrap());
                    assert_eq!(
                        output,
                        expected,
                        "{} fatal={fatal} split={split}",
                        encoding.name()
                    );
                }
            }
        }
    }

    #[test]
    fn legacy_decoder_preserves_pending_bytes_when_output_is_full() {
        for (encoding, input, expected) in [
            (encoding_rs::BIG5, [0xfe, 0x40], "\u{9442}"),
            (encoding_rs::SHIFT_JIS, [0x81, 0x87], "\u{221e}"),
            (encoding_rs::EUC_KR, [0x81, 0x41], "\u{ac02}"),
        ] {
            let mut decoder = encoding.new_decoder_without_bom_handling();
            let mut output = [0; 8];
            assert_eq!(
                decoder.decode_to_utf8_without_replacement(&input[..1], &mut output, false),
                (DecoderResult::InputEmpty, 1, 0)
            );
            assert_eq!(
                decoder.decode_to_utf8_without_replacement(&input[1..], &mut [], false),
                (DecoderResult::OutputFull, 0, 0)
            );
            let (result, read, written) =
                decoder.decode_to_utf8_without_replacement(&input[1..], &mut output, true);
            assert_eq!((result, read), (DecoderResult::InputEmpty, 1));
            assert_eq!(
                &output[..written],
                expected.as_bytes(),
                "{}",
                encoding.name()
            );
        }
    }

    #[test]
    fn text_decoder_state_replaces_invalid_utf8_when_not_fatal() {
        let mut state = TextDecoderState::new(encoding_rs::UTF_8, false, false);

        assert_eq!(
            state.decode(&[0x61, 0xFF, 0x62], false).unwrap(),
            "a\u{FFFD}b"
        );
    }

    #[test]
    fn text_decoder_state_errors_on_invalid_utf8_when_fatal() {
        let mut state = TextDecoderState::new(encoding_rs::UTF_8, true, false);

        assert!(matches!(
            state.decode(&[0x61, 0xFF, 0x62], false),
            Err(TextDecodeError::MalformedInput)
        ));
    }

    #[test]
    fn text_decoder_state_removes_utf8_bom_across_stream_chunks() {
        let mut state = TextDecoderState::new(encoding_rs::UTF_8, false, false);

        assert_eq!(state.decode(&[0xEF], true).unwrap(), "");
        assert_eq!(state.decode(&[0xBB, 0xBF, b'a'], false).unwrap(), "a");
    }

    #[test]
    fn text_decoder_state_supports_legacy_encoding_labels() {
        let mut state = TextDecoderState::new(encoding_rs::WINDOWS_1252, false, false);

        assert_eq!(state.decode(&[0x80], false).unwrap(), "\u{20AC}");
    }
    #[test]
    fn text_encoder_state_preserves_utf16_across_all_three_chunk_partitions() {
        let cases: &[&[u16]] = &[
            &[],
            &[0, 0x41, 0x20ac],
            &[0xd800],
            &[0xdfff],
            &[0xd800, 0xdc00],
            &[0xdbff, 0xdfff],
            &[0x41, 0xd83d, 0xdc99, 0x42],
            &[0xd800, 0xd800, 0xdc00, 0xdfff],
            &[0xd800, 0x41, 0xdc00, 0xdbff],
        ];
        for input in cases {
            let expected = String::from_utf16_lossy(input).into_bytes();
            for first in 0..=input.len() {
                for second in first..=input.len() {
                    let mut state = TextEncoderState::default();
                    let mut actual = Vec::new();
                    for chunk in [&input[..first], &input[first..second], &input[second..]] {
                        actual.extend(state.encode(chunk));
                        assert!(state.encode(&[]).is_empty());
                    }
                    actual.extend(state.flush());
                    assert_eq!(actual, expected, "input={input:?}, cuts={first}/{second}");
                    assert!(state.flush().is_empty());
                }
            }
        }
    }
}
