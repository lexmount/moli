mod support;

use moli_v8_test_util::ensure_v8;
use moli_webapi::{interfaces, text_codec::TextCodecStore, text_decoder};
use std::{pin::pin, rc::Rc};
use support::{eval, finish_template};

fn with_codec(test: impl FnOnce(&mut v8::PinScope<'_, '_>, &Rc<TextCodecStore>)) {
    ensure_v8();
    let codecs = Rc::new(TextCodecStore::default());
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    TextCodecStore::install(context, &codecs);
    let scope = &mut v8::ContextScope::new(scope, context);
    let template = v8::FunctionTemplate::new(
        scope,
        moli_webapi_declare::web_api_constructor!(
            interfaces::TextDecoder,
            text_decoder::text_decoder_constructor_callback
        ),
    );
    text_decoder::install_text_codec_template_bindings(scope, template, "TextDecoder");
    finish_template(scope, interfaces::TextDecoder::DESCRIPTOR, template);
    test(scope, &codecs);
    codecs.clear_for_context_teardown();
}

#[test]
fn standalone_text_decoder_keeps_stream_state_and_validation_without_a_host() {
    with_codec(|scope, _| {
        assert!(eval(scope, r#"(() => {
          const check = (ok) => { if (!ok) throw new Error('codec'); };
          const decoder = new TextDecoder('utf-8');
          check(decoder.encoding === 'utf-8' && !decoder.fatal && !decoder.ignoreBOM);
          check(decoder.decode(new Uint8Array([0xef, 0xbb]), {stream:true}) === '');
          check(decoder.decode(new Uint8Array([0xbf, 0xe2, 0x82]), {stream:true}) === '');
          check(decoder.decode(new Uint8Array([0xac])) === '€');
          check(decoder.decode(new Uint8Array([0x61])) === 'a');
          check(new TextDecoder('windows-1252').decode(new Uint8Array([0x80])) === '€');
          for (const action of [() => new TextDecoder('invalid-label'), () => new TextDecoder('utf-8', {fatal:true}).decode(new Uint8Array([0xff]))]) {
            try { action(); throw new Error('accepted'); }
            catch (error) { check(error instanceof TypeError || error instanceof RangeError); }
          }
          try { TextDecoder.prototype.decode.call({}); throw new Error('unbranded'); }
          catch (error) { check(error instanceof TypeError); }
          return true;
        })()"#).is_true());
    });
}

#[test]
fn retired_codec_component_cannot_be_reopened_by_a_retained_realm() {
    with_codec(|scope, codecs| {
        eval(scope, "globalThis.retainedDecoder = new TextDecoder();");
        codecs.clear_for_context_teardown();
        assert!(TextCodecStore::from_scope(scope).is_none());
        let object = v8::Object::new(scope);
        assert!(
            codecs
                .init_decoder(scope, object, encoding_rs::UTF_8, false, false)
                .is_none()
        );
        assert!(eval(scope, "retainedDecoder.decode(new Uint8Array([65])) === '' && new TextDecoder().decode(new Uint8Array([65])) === ''").is_true());
    });
}
