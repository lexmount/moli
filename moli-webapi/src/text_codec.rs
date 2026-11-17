use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use encoding_rs::{CoderResult, Decoder, DecoderResult, Encoding};
use parking_lot::Mutex;

use moli_v8_util::{V8FinalizerRegistry, get_private_value, set_private_value};
use std::{cell::RefCell, rc::Rc};

const TEXT_DECODER_ID_SLOT: &str = "__lmTextDecoderId";

#[derive(Default)]
pub struct TextEncoderState {
    leading_surrogate: Option<u16>,
}

impl TextEncoderState {
    pub fn new(leading_surrogate: Option<u16>) -> Self {
        debug_assert!(leading_surrogate.is_none_or(|unit| (0xd800..=0xdbff).contains(&unit)));
        Self { leading_surrogate }
    }

    pub fn leading_surrogate(&self) -> Option<u16> {
        self.leading_surrogate
    }

    pub fn encode(&mut self, input: &[u16]) -> Vec<u8> {
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

    pub fn flush(&mut self) -> Vec<u8> {
        if self.leading_surrogate.take().is_some() {
            vec![0xef, 0xbf, 0xbd]
        } else {
            Vec::new()
        }
    }
}

pub struct TextCodecStore {
    state: Arc<Mutex<Option<TextCodecStoreState>>>,
    finalizers: RefCell<V8FinalizerRegistry>,
}

impl Default for TextCodecStore {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(Some(TextCodecStoreState::default()))),
            finalizers: RefCell::default(),
        }
    }
}

#[derive(Default)]
struct TextCodecStoreState {
    next_decoder_id: u32,
    decoders: HashMap<u32, TextDecoderState>,
}

#[derive(Default)]
pub struct TextDecoderState {
    encoding: Option<&'static Encoding>,
    decoder: Option<Decoder>,
    fatal: bool,
    ignore_bom: bool,
}

#[derive(Debug)]
pub enum TextDecodeError {
    MalformedInput,
}

impl TextDecodeError {
    pub fn message(&self) -> &'static str {
        match self {
            Self::MalformedInput => "The encoded data was not valid.",
        }
    }
}

struct TextCodecBinding(std::rc::Weak<TextCodecStore>);

impl TextCodecStore {
    /// The realm retains only a weak component binding; the Page/Worker owner
    /// controls teardown without exposing its native host to these value APIs.
    pub fn install(context: v8::Local<'_, v8::Context>, store: &Rc<Self>) {
        if let Some(binding) = context.get_slot::<TextCodecBinding>() {
            assert!(
                std::rc::Weak::ptr_eq(&binding.0, &Rc::downgrade(store)),
                "a realm's codec owner must not be rebound"
            );
            return;
        }
        context.set_slot(Rc::new(TextCodecBinding(Rc::downgrade(store))));
    }

    pub fn from_scope(scope: &mut v8::PinScope<'_, '_>) -> Option<Rc<Self>> {
        let store = scope
            .get_current_context()
            .get_slot::<TextCodecBinding>()?
            .0
            .upgrade()?;
        let active = store.state.lock().is_some();
        active.then_some(store)
    }

    pub fn clear_for_context_teardown(&self) {
        // Retire before running cleanup, which may reenter the component.
        drop(self.state.lock().take());
        let mut finalizers = std::mem::take(&mut *self.finalizers.borrow_mut());
        finalizers.clear_for_context_teardown();
    }

    pub fn init_decoder(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        decoder: v8::Local<'_, v8::Object>,
        encoding: &'static Encoding,
        fatal: bool,
        ignore_bom: bool,
    ) -> Option<u32> {
        let decoder_id = {
            let mut state = self.state.lock();
            let state = state.as_mut()?;
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
        V8FinalizerRegistry::track(&self.finalizers, scope, decoder, move || {
            remove_decoder_state(&state, decoder_id);
        });
        Some(decoder_id)
    }

    pub fn decode(
        &self,
        decoder_id: u32,
        bytes: &[u8],
        stream: bool,
    ) -> Result<String, TextDecodeError> {
        self.state
            .lock()
            .as_mut()
            .and_then(|state| state.decoders.get_mut(&decoder_id))
            .map(|state| state.decode(bytes, stream))
            .unwrap_or_else(|| Ok(String::new()))
    }

    pub fn decoder_id_from_object<'s>(
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

fn remove_decoder_state(state: &Weak<Mutex<Option<TextCodecStoreState>>>, decoder_id: u32) {
    if let Some(state) = state.upgrade()
        && let Some(state) = state.lock().as_mut()
    {
        state.decoders.remove(&decoder_id);
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

    pub fn decode(&mut self, bytes: &[u8], stream: bool) -> Result<String, TextDecodeError> {
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
        moli_v8_test_util::ensure_v8();
        let codecs = Rc::new(TextCodecStore::default());
        let mut isolate = v8::Isolate::new(Default::default());
        let (context, retained_decoder, retained_id) = {
            let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let scope = &mut scope.init();
            let context = v8::Context::new(scope, Default::default());
            TextCodecStore::install(context, &codecs);
            let scope = &mut v8::ContextScope::new(scope, context);
            let collected = v8::Object::new(scope);
            let _ = codecs.init_decoder(scope, collected, encoding_rs::UTF_8, false, false);
            let retained = v8::Object::new(scope);
            let id = codecs
                .init_decoder(scope, retained, encoding_rs::UTF_8, false, false)
                .unwrap();
            assert_eq!(codecs.state.lock().as_ref().unwrap().decoders.len(), 2);
            (
                v8::Global::new(scope, context),
                v8::Global::new(scope, retained),
                id,
            )
        };
        for _ in 0..2 {
            isolate.low_memory_notification();
        }
        assert_eq!(codecs.state.lock().as_ref().unwrap().decoders.len(), 1);
        assert_eq!(codecs.decode(retained_id, &[0xE2, 0x82], true).unwrap(), "");
        assert_eq!(codecs.decode(retained_id, &[0xAC], false).unwrap(), "€");
        codecs.clear_for_context_teardown();
        assert!(codecs.state.lock().is_none());
        assert!(codecs.finalizers.borrow().is_empty());
        codecs.clear_for_context_teardown();
        drop(retained_decoder);
        drop(context);
        drop(isolate);
    }

    #[test]
    fn text_decoder_state_streams_split_utf8_sequences() {
        let mut state = TextDecoderState::new(encoding_rs::UTF_8, false, false);

        assert_eq!(state.decode(&[0xE2, 0x82], true).unwrap(), "");
        assert_eq!(state.decode(&[0xAC], false).unwrap(), "\u{20AC}");
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
