pub(in crate::context_bootstrap) use moli_webapi::speech_synthesis::{
    install_speech_synthesis_template_bindings, speech_synthesis_utterance_constructor_callback,
};
pub(in crate::context_bootstrap) fn build_window_speech_synthesis<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> anyhow::Result<v8::Local<'s, v8::Object>> {
    Ok(moli_webapi::speech_synthesis::build_window_speech_synthesis(scope)?)
}
