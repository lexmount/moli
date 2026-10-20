mod support;
#[path = "support/targets.rs"]
mod targets;

use moli_v8_test_util::ensure_v8;
use moli_v8_util::{global_constructor_prototype, set_private_value, v8str};
use moli_webapi::{
    gamepad, interfaces, media_device_info, media_error, navigator_collections as collections,
    speech_synthesis as speech,
};
use std::pin::pin;
use support::{eval, finish_template, illegal_constructor, install_errors, publish};

fn install<'s>(scope: &mut v8::PinScope<'s, '_>) {
    install_errors(scope);
    targets::install(scope);
    let target = v8::FunctionTemplate::new(scope, illegal_constructor);
    finish_template(scope, interfaces::EventTarget::DESCRIPTOR, target);
    for interface in [
        interfaces::Plugin::DESCRIPTOR,
        interfaces::PluginArray::DESCRIPTOR,
        interfaces::MimeType::DESCRIPTOR,
        interfaces::MimeTypeArray::DESCRIPTOR,
    ] {
        let template = v8::FunctionTemplate::new(scope, illegal_constructor);
        collections::install_navigator_collection_template_bindings(
            scope,
            template,
            interface.name(),
        );
        finish_template(scope, interface, template);
    }
    for (interface, template) in [
        (
            interfaces::SpeechSynthesis::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, illegal_constructor),
        ),
        (
            interfaces::SpeechSynthesisUtterance::DESCRIPTOR,
            v8::FunctionTemplate::new(
                scope,
                speech::speech_synthesis_utterance_constructor_callback,
            ),
        ),
        (
            interfaces::SpeechSynthesisVoice::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, illegal_constructor),
        ),
    ] {
        if interface.name() != "SpeechSynthesisVoice" {
            template.inherit(target);
        }
        speech::install_speech_synthesis_template_bindings(scope, template, interface.name());
        finish_template(scope, interface, template);
    }
    for interface in [
        interfaces::Gamepad::DESCRIPTOR,
        interfaces::GamepadButton::DESCRIPTOR,
        interfaces::GamepadHapticActuator::DESCRIPTOR,
    ] {
        let template = v8::FunctionTemplate::new(scope, illegal_constructor);
        gamepad::install(scope, template, interface.name());
        finish_template(scope, interface, template);
    }
    let device = v8::FunctionTemplate::new(scope, illegal_constructor);
    media_device_info::install(scope, device, "MediaDeviceInfo");
    finish_template(scope, interfaces::MediaDeviceInfo::DESCRIPTOR, device);
    let input = v8::FunctionTemplate::new(scope, illegal_constructor);
    input.inherit(device);
    media_device_info::install(scope, input, "InputDeviceInfo");
    finish_template(scope, interfaces::InputDeviceInfo::DESCRIPTOR, input);
    let error = v8::FunctionTemplate::new(scope, media_error::media_error_constructor_callback);
    media_error::install(scope, error);
    finish_template(scope, interfaces::MediaError::DESCRIPTOR, error);
    eval(
        scope,
        r#"
      function check(ok, label) { if (!ok) throw Error(label); }
      function fails(name, run) {
        try {run();} catch (e) {check(e.name === name, name + ': ' + e); return;}
        throw Error('missing ' + name);
      }
    "#,
    );
}

fn native<'s>(scope: &mut v8::PinScope<'s, '_>, interface: &str) -> v8::Local<'s, v8::Object> {
    let object = v8::Object::new(scope);
    let prototype = global_constructor_prototype(scope, interface).unwrap();
    object.set_prototype(scope, prototype.into()).unwrap();
    interfaces::initialize(scope, object, interface).unwrap();
    object
}

#[test]
fn pdf_collections_preserve_named_items_iteration_and_plugin_backreferences() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    let values = collections::build_navigator_plugin_collections(scope).unwrap();
    publish(scope, "plugins", values.plugins);
    publish(scope, "mimeTypes", values.mime_types);
    eval(
        scope,
        r#"
      check(plugins instanceof PluginArray && mimeTypes instanceof MimeTypeArray, 'array brands');
      check(plugins.length === 5 && mimeTypes.length === 2, 'PDF collection sizes');
      const plugin = plugins.item(0), mime = mimeTypes.namedItem('application/pdf');
      check(plugins.namedItem(plugin.name) === plugin && plugins[plugin.name] === plugin, 'plugin names');
      check(mime === mimeTypes[0] && mime.enabledPlugin === plugin, 'MIME backreference');
      check(plugin[0].enabledPlugin === plugin && plugin[0] instanceof MimeType, 'plugin MIME');
      check([...plugins].length === 5 && [...plugin].length === 2 && [...mimeTypes].length === 2, 'iteration');
      check(!Object.keys(plugins).includes(plugin.name), 'named item non-enumerable');
      check(plugins.item(100) === null && mimeTypes.namedItem('missing') === null, 'missing values');
      fails('TypeError', () => PluginArray.prototype.item.call({}, 0));
      const receiver = Object.create(plugins), sentinel = {};
      let setterCalls = 0;
      Object.defineProperty(PluginArray.prototype, 'authorSetter', {
        configurable: true,
        set(value) {
          check(this === receiver && value === 7, 'forwarded setter receiver and value');
          setterCalls++;
          throw sentinel;
        }
      });
      try {
        try { Reflect.set(plugins, 'authorSetter', 7, receiver); throw Error('setter did not throw'); }
        catch (error) { check(error === sentinel, 'original setter exception'); }
        check(setterCalls === 1, 'setter called exactly once');
      } finally { delete PluginArray.prototype.authorSetter; }
    "#,
    );
}

#[test]
fn speech_values_preserve_domstrings_ranges_and_host_handlers() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    let synthesis = speech::build_window_speech_synthesis(scope).unwrap();
    publish(scope, "synthesis", synthesis);
    let voice = native(scope, "SpeechSynthesisVoice");
    let voice_name = v8str(scope, "test voice");
    set_private_value(
        scope,
        voice,
        "__lmSpeechSynthesisVoiceName",
        voice_name.into(),
    );
    publish(scope, "voice", voice);
    eval(
        scope,
        r#"
      const utterance = new SpeechSynthesisUtterance('text\ud800');
      check(utterance.text === 'text\ud800' && utterance instanceof EventTarget, 'DOMString and inheritance');
      utterance.lang = 'lang\udfff';
      check(utterance.lang === 'lang\udfff', 'language DOMString');
      utterance.voice = voice;
      check(utterance.voice === voice && voice.name === 'test voice', 'voice identity');
      fails('TypeError', () => {utterance.voice = Object.create(SpeechSynthesisVoice.prototype);});
      utterance.volume = 9; utterance.rate = -1; utterance.pitch = 9;
      check(utterance.volume === 1 && Math.abs(utterance.rate - .1) < 1e-6 && utterance.pitch === 2, 'float clamps');
      fails('TypeError', () => {utterance.volume = Infinity;});
      const handler = () => {};
      utterance.onend = handler; synthesis.onvoiceschanged = handler;
      check(utterance.onend === handler && synthesis.onvoiceschanged === handler, 'host handler handoff');
      utterance.onend = 1;
      check(utterance.onend === null, 'handler conversion');
      check(synthesis.getVoices().length === 0, 'no fabricated voices');
      synthesis.speak(utterance);
      check(!synthesis.pending && !synthesis.speaking && !synthesis.paused, 'no fabricated playback');
      fails('TypeError', () => synthesis.speak(Object.create(SpeechSynthesisUtterance.prototype)));
    "#,
    );
}

#[test]
fn media_info_and_errors_use_private_state_and_canonical_prototypes() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    let device = native(scope, "InputDeviceInfo");
    for (slot, value) in [
        ("__moliMediaDeviceId", "id"),
        ("__moliMediaDeviceKind", "audioinput"),
        ("__moliMediaDeviceLabel", ""),
        ("__moliMediaDeviceGroupId", ""),
    ] {
        let value = v8str(scope, value);
        set_private_value(scope, device, slot, value.into());
    }
    publish(scope, "device", device);
    let prototype = global_constructor_prototype(scope, "MediaError").unwrap();
    publish(scope, "errorPrototype", prototype);
    eval(
        scope,
        "Object.defineProperty(globalThis, 'MediaError', {get() {throw Error('author constructor');}})",
    );
    let error = media_error::new_media_error_value(scope, 3, "decode failed").unwrap();
    publish(scope, "mediaError", error);
    eval(
        scope,
        r#"
      check(device instanceof MediaDeviceInfo && device instanceof InputDeviceInfo, 'device inheritance');
      Object.defineProperty(device, 'deviceId', {get() {throw Error('author shadow');}});
      check(device.toJSON().deviceId === 'id' && device.toJSON().label === '', 'private device projection');
      check(Object.keys(device.getCapabilities()).length === 0, 'privacy-filtered capabilities');
      fails('TypeError', () => MediaDeviceInfo.prototype.toJSON.call({}));
      check(Object.getPrototypeOf(mediaError) === errorPrototype && mediaError.code === 3 && mediaError.message === 'decode failed', 'canonical error factory');
      check(errorPrototype.MEDIA_ERR_DECODE === 3 && errorPrototype.MEDIA_ERR_SRC_NOT_SUPPORTED === 4, 'legacy constants');
      fails('TypeError', () => Object.getOwnPropertyDescriptor(errorPrototype, 'code').get.call({}));
    "#,
    );
}

#[test]
fn gamepad_shells_reject_forged_receivers_and_keep_backend_unavailable() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope);
    for (interface, name) in [
        ("Gamepad", "gamepad"),
        ("GamepadButton", "button"),
        ("GamepadHapticActuator", "actuator"),
    ] {
        let object = native(scope, interface);
        publish(scope, name, object);
    }
    eval(
        scope,
        r#"
      fails('TypeError', () => new Gamepad());
      fails('TypeError', () => Object.getOwnPropertyDescriptor(Gamepad.prototype, 'axes').get.call(Object.create(Gamepad.prototype)));
      fails('NotSupportedError', () => gamepad.axes);
      fails('NotSupportedError', () => button.pressed);
      globalThis.rejected = false;
      actuator.reset().then(() => {throw Error('fabricated device result');}, e => {check(e.name === 'NotSupportedError', 'haptic rejection'); globalThis.rejected = true;});
    "#,
    );
    scope.perform_microtask_checkpoint();
    assert!(eval(scope, "rejected").is_true());
}
