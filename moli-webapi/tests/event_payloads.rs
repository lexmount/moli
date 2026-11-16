mod support;

use moli_v8_test_util::ensure_v8;
use moli_v8_util::{global_constructor_prototype, set_private_value, v8str};
use moli_webapi::{
    events::{self, EventRuntime, audio, device, speech, state, value},
    interfaces,
};
use std::{cell::Cell, pin::pin, rc::Rc};
use support::{eval, finish_template, illegal_constructor, install_errors, publish};

struct Clock {
    now: f64,
    reads: Cell<u32>,
}

fn timestamp(scope: &mut v8::PinScope<'_, '_>) -> f64 {
    let clock = scope.get_current_context().get_slot::<Clock>().unwrap();
    clock.reads.set(clock.reads.get() + 1);
    clock.now
}

// A minimal embedder has one wrapper per Event. The Renderer integration tests
// exercise its additional world caches, target projection and dispatch paths.
fn initialize_wrapper<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    wrapper: v8::Local<'s, v8::Object>,
    backing: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let interface = moli_webapi_declare::web_api_object_type(scope, backing)?.name();
    interfaces::initialize(scope, wrapper, interface).ok()?;
    set_private_value(scope, wrapper, state::EVENT_BACKING_SLOT, backing.into());
    state::define_event_is_trusted_accessor(scope, wrapper);
    Some(())
}

fn core_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let key = args.data().to_rust_string_lossy(scope);
    if key == "timeStamp" {
        rv.set_double(state::event_time_stamp(scope, args.this()));
    } else if let Some(value) = state::event_attribute(scope, args.this(), &key) {
        rv.set(value);
    }
}

fn install(scope: &mut v8::PinScope<'_, '_>, now: f64) {
    scope.get_current_context().set_slot(Rc::new(Clock {
        now,
        reads: Cell::new(0),
    }));
    events::install_runtime(
        scope,
        EventRuntime {
            timestamp,
            initialize_wrapper,
        },
    );
    install_errors(scope);
    let event = v8::FunctionTemplate::new(scope, illegal_constructor);
    let prototype = event.prototype_template(scope);
    for key in ["type", "bubbles", "cancelable", "composed", "timeStamp"] {
        let getter = v8::FunctionTemplate::builder(core_getter)
            .data(v8str(scope, key).into())
            .build(scope);
        prototype.set_accessor_property(
            v8str(scope, key).into(),
            Some(getter),
            None,
            Default::default(),
        );
    }
    finish_template(scope, interfaces::Event::DESCRIPTOR, event);

    let payment =
        value::build_value_event_template(scope, value::ValueEventKind::PaymentRequestUpdate);
    payment.inherit(event);
    value::install_value_event_template_bindings(scope, payment, "PaymentRequestUpdateEvent");
    finish_template(
        scope,
        interfaces::PaymentRequestUpdateEvent::DESCRIPTOR,
        payment,
    );
    for (interface, kind) in [
        (
            interfaces::AnimationEvent::DESCRIPTOR,
            value::ValueEventKind::Animation,
        ),
        (
            interfaces::TransitionEvent::DESCRIPTOR,
            value::ValueEventKind::Transition,
        ),
        (
            interfaces::BlobEvent::DESCRIPTOR,
            value::ValueEventKind::Blob,
        ),
        (
            interfaces::WebGLContextEvent::DESCRIPTOR,
            value::ValueEventKind::WebGlContext,
        ),
        (
            interfaces::ToolActivatedEvent::DESCRIPTOR,
            value::ValueEventKind::ToolActivated,
        ),
        (
            interfaces::ToolCancelEvent::DESCRIPTOR,
            value::ValueEventKind::ToolCancel,
        ),
        (
            interfaces::GamepadEvent::DESCRIPTOR,
            value::ValueEventKind::Gamepad,
        ),
        (
            interfaces::PictureInPictureEvent::DESCRIPTOR,
            value::ValueEventKind::PictureInPicture,
        ),
        (
            interfaces::MIDIMessageEvent::DESCRIPTOR,
            value::ValueEventKind::MidiMessage,
        ),
        (
            interfaces::MIDIConnectionEvent::DESCRIPTOR,
            value::ValueEventKind::MidiConnection,
        ),
        (
            interfaces::PaymentMethodChangeEvent::DESCRIPTOR,
            value::ValueEventKind::PaymentMethodChange,
        ),
    ] {
        let template = value::build_value_event_template(scope, kind);
        if interface.name() == "PaymentMethodChangeEvent" {
            template.inherit(payment);
        } else {
            template.inherit(event);
        }
        value::install_value_event_template_bindings(scope, template, interface.name());
        finish_template(scope, interface, template);
    }
    let orientation =
        v8::FunctionTemplate::new(scope, device::device_orientation_event_constructor);
    let motion = v8::FunctionTemplate::new(scope, device::device_motion_event_constructor);
    for (interface, template) in [
        (interfaces::DeviceOrientationEvent::DESCRIPTOR, orientation),
        (interfaces::DeviceMotionEvent::DESCRIPTOR, motion),
    ] {
        template.inherit(event);
        device::install_device_event_template_bindings(scope, template, interface.name());
        finish_template(scope, interface, template);
    }
    for interface in [
        interfaces::DeviceMotionEventAcceleration::DESCRIPTOR,
        interfaces::DeviceMotionEventRotationRate::DESCRIPTOR,
    ] {
        let template = v8::FunctionTemplate::new(scope, illegal_constructor);
        device::install_device_event_template_bindings(scope, template, interface.name());
        finish_template(scope, interface, template);
    }
    for (interface, kind) in [
        (
            interfaces::AudioProcessingEvent::DESCRIPTOR,
            audio::AudioEventKind::Processing,
        ),
        (
            interfaces::OfflineAudioCompletionEvent::DESCRIPTOR,
            audio::AudioEventKind::Completion,
        ),
    ] {
        let template = audio::build_audio_event_template(scope, kind);
        template.inherit(event);
        finish_template(scope, interface, template);
    }
    let synthesis = speech::build_template(scope, speech::SpeechEventKind::Synthesis);
    synthesis.inherit(event);
    let error = speech::build_template(scope, speech::SpeechEventKind::Error);
    error.inherit(synthesis);
    for (interface, template) in [
        (interfaces::SpeechSynthesisEvent::DESCRIPTOR, synthesis),
        (interfaces::SpeechSynthesisErrorEvent::DESCRIPTOR, error),
    ] {
        speech::install(scope, template, interface.name());
        finish_template(scope, interface, template);
    }

    // Native payload brands without a DOM, audio graph, payment or sensor host.
    for (interface, name) in [
        (interfaces::CSSAnimation::DESCRIPTOR, "animation"),
        (interfaces::CSSTransition::DESCRIPTOR, "transition"),
        (interfaces::Blob::DESCRIPTOR, "blob"),
        (interfaces::Gamepad::DESCRIPTOR, "gamepad"),
        (interfaces::PictureInPictureWindow::DESCRIPTOR, "pip"),
        (interfaces::MIDIPort::DESCRIPTOR, "port"),
        (interfaces::AudioBuffer::DESCRIPTOR, "buffer"),
        (
            interfaces::SpeechSynthesisUtterance::DESCRIPTOR,
            "utterance",
        ),
    ] {
        let template = v8::FunctionTemplate::new(scope, illegal_constructor);
        finish_template(scope, interface, template);
        let prototype = global_constructor_prototype(scope, interface.name()).unwrap();
        let object = v8::Object::new(scope);
        assert_eq!(object.set_prototype(scope, prototype.into()), Some(true));
        interfaces::initialize(scope, object, interface.name()).unwrap();
        publish(scope, name, object);
    }
    eval(
        scope,
        "globalThis.assert = (ok, message = 'assertion failed') => { if (!ok) throw new Error(message); };",
    );
}

fn check(source: &str) {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 37.25);
    assert!(eval(scope, source).is_true());
}

#[test]
fn payload_families_preserve_values_brands_and_inheritance_without_a_renderer() {
    check(
        r#"(() => {
        const data = new Uint8Array([1, 2, 255]), details = {};
        const cases = [
            [AnimationEvent, {animation, animationName: 'spin\ud800', elapsedTime: .5}, 'animation', animation],
            [TransitionEvent, {animation: transition, propertyName: 'color', elapsedTime: .25}, 'animation', transition],
            [BlobEvent, {data: blob}, 'data', blob],
            [WebGLContextEvent, {statusMessage: 'lost\udfff'}, 'statusMessage', 'lost\udfff'],
            [ToolActivatedEvent, {toolName: 'paint'}, 'toolName', 'paint'],
            [ToolCancelEvent, {toolName: 'cancel'}, 'toolName', 'cancel'],
            [GamepadEvent, {gamepad}, 'gamepad', gamepad],
            [PictureInPictureEvent, {pictureInPictureWindow: pip}, 'pictureInPictureWindow', pip],
            [PaymentRequestUpdateEvent, {}, 'type', 'payload\ud800'],
            [PaymentMethodChangeEvent, {methodDetails: details, methodName: 'card'}, 'methodDetails', details],
            [MIDIMessageEvent, {data}, 'data', data],
            [MIDIConnectionEvent, {port}, 'port', port],
            [DeviceOrientationEvent, {alpha: 1, beta: 2, gamma: 3, absolute: true}, 'alpha', 1],
            [DeviceMotionEvent, {acceleration: {x: 2}}, 'interval', 0],
            [AudioProcessingEvent, {inputBuffer: buffer, outputBuffer: buffer, playbackTime: 1.5}, 'inputBuffer', buffer],
            [OfflineAudioCompletionEvent, {renderedBuffer: buffer}, 'renderedBuffer', buffer],
            [SpeechSynthesisEvent, {utterance, name: 'word\ud800'}, 'utterance', utterance],
            [SpeechSynthesisErrorEvent, {utterance, error: 'network'}, 'error', 'network'],
        ];
        for (const [Ctor, init, property, expected] of cases) {
            class Derived extends Ctor {}
            const event = new Derived('payload\ud800', {...init, bubbles: true, cancelable: true, composed: true});
            assert(event instanceof Ctor && event instanceof Event && event instanceof Derived, Ctor.name);
            assert(event[property] === expected, Ctor.name + '.' + property);
            assert(event.type === 'payload\ud800' && event.bubbles && event.cancelable && event.composed);
            assert(event.isTrusted === false && event.timeStamp === 37.25);
            if (property !== 'type') assert(!Object.hasOwn(event, property), Ctor.name + ' payload on prototype');
        }
        const motion = new DeviceMotionEvent('m', {acceleration: {x: 1}, rotationRate: {alpha: 2}});
        assert(motion.acceleration instanceof DeviceMotionEventAcceleration);
        assert(motion.rotationRate instanceof DeviceMotionEventRotationRate);
        assert(motion.acceleration.y === null && motion.rotationRate.gamma === null);
        assert(new SpeechSynthesisErrorEvent('s', {utterance, error: 'network'}) instanceof SpeechSynthesisEvent);
        assert(new PaymentMethodChangeEvent('p') instanceof PaymentRequestUpdateEvent);
        return true;
    })()"#,
    );
}

#[test]
fn inherited_and_nested_dictionaries_convert_in_order_and_preserve_exceptions() {
    check(
        r#"(() => {
        const order = [], init = {};
        const values = {bubbles: true, cancelable: true, composed: true,
            acceleration: {get x() {order.push('x'); return 1;}, get y() {order.push('y'); return 2;}, get z() {order.push('z'); return 3;}},
            accelerationIncludingGravity: null, interval: {valueOf() {order.push('interval.valueOf'); return .5;}}, rotationRate: null};
        for (const name of Object.keys(values).reverse()) Object.defineProperty(init, name, {get() {order.push(name); return values[name];}});
        const event = new DeviceMotionEvent({toString() {order.push('type'); return 'motion';}}, init);
        assert(JSON.stringify(order) === JSON.stringify(['type','bubbles','cancelable','composed','acceleration','x','y','z','accelerationIncludingGravity','interval','interval.valueOf','rotationRate']), JSON.stringify(order));
        assert(event.acceleration.z === 3 && event.interval === .5);
        const marker = {}, reads = [];
        let caught;
        try { new AnimationEvent('a', new Proxy({}, {get(_, name) {reads.push(name); if (name === 'animationName') throw marker;}})); }
        catch (error) {caught = error;}
        assert(caught === marker);
        assert(JSON.stringify(reads) === JSON.stringify(['bubbles','cancelable','composed','animation','animationName']));
        const speechReads = [];
        const speechInit = new Proxy({utterance, error: 'network'}, {get(target, name) {speechReads.push(name); return target[name];}});
        new SpeechSynthesisErrorEvent('s', speechInit);
        assert(JSON.stringify(speechReads) === JSON.stringify(['bubbles','cancelable','composed','charIndex','charLength','elapsedTime','name','utterance','error']));
        return true;
    })()"#,
    );
}

#[test]
fn payload_conversions_validate_native_brands_without_author_proxy_traps() {
    check(
        r#"(() => {
        const cases = [
            [AnimationEvent, 'animation', animation], [TransitionEvent, 'animation', transition],
            [BlobEvent, 'data', blob], [GamepadEvent, 'gamepad', gamepad],
            [PictureInPictureEvent, 'pictureInPictureWindow', pip],
            [MIDIConnectionEvent, 'port', port], [OfflineAudioCompletionEvent, 'renderedBuffer', buffer],
            [SpeechSynthesisEvent, 'utterance', utterance],
        ];
        for (const [Ctor, key, valid] of cases) {
            let traps = 0;
            const proxy = new Proxy(valid, {get() {traps++; throw 1;}, getPrototypeOf() {traps++; throw 2;}});
            const revoked = Proxy.revocable(valid, {}); revoked.revoke();
            for (const invalid of [{}, Object.create(valid), proxy, revoked.proxy]) {
                let caught;
                try {new Ctor('x', {[key]: invalid});} catch (error) {caught = error;}
                assert(caught instanceof TypeError, Ctor.name);
                assert(traps === 0);
            }
            const event = new Ctor('x', {[key]: valid});
            const getter = Object.getOwnPropertyDescriptor(Ctor.prototype, key).get;
            let read = false, caught;
            try {getter.call(new Proxy(event, {get() {read = true; throw 3;}, getPrototypeOf() {read = true; throw 4;}}));}
            catch (error) {caught = error;}
            assert(caught instanceof TypeError && !read, Ctor.name + ' receiver');
        }
        return true;
    })()"#,
    );
}

#[test]
fn numeric_enum_and_required_member_errors_happen_before_event_initialization() {
    check(
        r#"(() => {
        const failures = [
            () => new AnimationEvent('a', {elapsedTime: NaN}),
            () => new TransitionEvent('t', {elapsedTime: Infinity}),
            () => new DeviceOrientationEvent('o', {alpha: Infinity}),
            () => new DeviceMotionEvent('m', {acceleration: {x: NaN}}),
            () => new BlobEvent('b', {}),
            () => new PictureInPictureEvent('p', {}),
            () => new AudioProcessingEvent('a', {inputBuffer: buffer, outputBuffer: buffer}),
            () => new OfflineAudioCompletionEvent('a', {renderedBuffer: {}}),
            () => new SpeechSynthesisEvent('s', {utterance, elapsedTime: 3.5e38}),
            () => new SpeechSynthesisErrorEvent('s', {utterance, error: 'NETWORK'}),
            () => new MIDIMessageEvent('m', {data: new Uint16Array(1)}),
        ];
        for (const run of failures) {
            let caught;
            try {run();} catch (error) {caught = error;}
            assert(caught instanceof TypeError);
        }
        const speech = new SpeechSynthesisEvent('s', {utterance, charIndex: -1, charLength: 4294967297, elapsedTime: 1 / 3, name: '\ud800\udfff'});
        assert(speech.charIndex === 4294967295 && speech.charLength === 1);
        assert(speech.elapsedTime === Math.fround(1 / 3) && speech.name === '\ud800\udfff');
        const blobEvent = new BlobEvent('b', {data: blob});
        assert(Number.isNaN(blobEvent.timecode));
        return true;
    })()"#,
    );
}

#[test]
fn private_event_state_survives_author_shadows_and_reinitialization_during_dispatch() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 18.5);
    let event = eval(
        scope,
        "new AnimationEvent('original', {animationName: 'spin', cancelable: true, composed: true})",
    )
    .to_object(scope)
    .unwrap();
    publish(scope, "event", event);
    eval(
        scope,
        "Object.defineProperty(event, 'animationName', {get() {throw 47;}}); Object.defineProperty(event, 'type', {value: 'forged'});",
    );
    assert_eq!(
        state::event_attribute(scope, event, "type")
            .unwrap()
            .to_rust_string_lossy(scope),
        "original"
    );
    state::mark_event_trusted(scope, event);
    state::set_event_dispatching(scope, event, true);
    assert!(!state::reinitialize_event_object(
        scope,
        event,
        &[0x78],
        false,
        false
    ));
    assert!(state::event_trusted(scope, event));
    state::set_event_dispatching(scope, event, false);
    assert!(state::reinitialize_event_object(
        scope,
        event,
        &[0x78, 0xd800],
        true,
        false
    ));
    assert!(!state::event_trusted(scope, event));
    assert_eq!(state::event_time_stamp(scope, event), 18.5);
    assert!(state::event_bool_attribute(scope, event, "composed"));
    assert!(eval(scope, "Object.getOwnPropertyDescriptor(AnimationEvent.prototype, 'animationName').get.call(event) === 'spin'").is_true());
}

#[test]
fn trusted_audio_completion_uses_intrinsics_and_preserves_buffer_identity() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 12.0);
    let buffer = eval(scope, "buffer").to_object(scope).unwrap();
    eval(
        scope,
        "globalThis.originalCompletion = OfflineAudioCompletionEvent; Object.defineProperty(globalThis, 'OfflineAudioCompletionEvent', {get() {throw 77;}});",
    );
    let event = audio::new_offline_audio_completion_event(scope, buffer).unwrap();
    publish(scope, "completion", event);
    assert!(eval(scope, "completion instanceof originalCompletion && completion.renderedBuffer === buffer && completion.type === 'complete' && completion.timeStamp === 12 && completion.isTrusted && !Object.getOwnPropertyDescriptor(completion, 'isTrusted').configurable").is_true());
}

#[test]
fn payment_update_uses_native_exception_even_when_public_constructor_is_replaced() {
    check(
        r#"(() => {
        const event = new PaymentMethodChangeEvent('p');
        const prototype = DOMException.prototype;
        Object.defineProperty(globalThis, 'DOMException', {get() {throw 79;}});
        let caught;
        try {event.updateWith({then() {}});} catch (error) {caught = error;}
        assert(caught && Object.getPrototypeOf(caught) === prototype);
        assert(caught.name === 'InvalidStateError' && caught.code === 11);
        return true;
    })()"#,
    );
}

#[test]
fn borrowed_event_constructors_keep_callee_clock_nested_prototypes_and_error_realm() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let first = v8::Context::new(scope, Default::default());
    let second = v8::Context::new(scope, Default::default());
    // These realms model two same-origin Windows. Bare V8 Contexts otherwise
    // reject access to one another's globals before any binding is exercised.
    let token = v8str(scope, "same-origin event test");
    first.set_security_token(token.into());
    second.set_security_token(token.into());
    {
        let scope = &mut v8::ContextScope::new(scope, first);
        install(scope, 11.5);
    }
    {
        let scope = &mut v8::ContextScope::new(scope, second);
        install(scope, 29.5);
        let foreign = first.global(scope);
        publish(scope, "foreign", foreign);
        assert!(eval(scope, r#"(() => {
            const foreignProto = foreign.DeviceMotionEventAcceleration.prototype;
            foreign.DeviceMotionEventAcceleration = function Forged() {};
            const event = new foreign.DeviceMotionEvent('motion', {acceleration: {x: 4}});
            assert(event.timeStamp === 11.5);
            assert(Object.getPrototypeOf(event.acceleration) === foreignProto && event.acceleration.x === 4);
            assert(new DeviceMotionEvent('local').timeStamp === 29.5);
            const getter = Object.getOwnPropertyDescriptor(foreign.DeviceMotionEvent.prototype, 'acceleration').get;
            assert(getter.call(new DeviceMotionEvent('local', {acceleration: {x: 7}})).x === 7);
            let caught;
            try {new foreign.DeviceMotionEvent('bad', {interval: Infinity});} catch (error) {caught = error;}
            assert(Object.getPrototypeOf(caught) === foreign.TypeError.prototype);
            return true;
        })()"#).is_true());
    }
}

#[test]
fn realm_clock_is_only_read_after_successful_argument_conversion() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 7.0);
    eval(
        scope,
        "try {new BlobEvent('bad', {});} catch {} try {new DeviceMotionEvent('bad', {interval: Infinity});} catch {} try {SpeechSynthesisEvent('no-new');} catch {}",
    );
    let clock = scope
        .get_current_context()
        .get_slot::<Clock>()
        .unwrap()
        .clone();
    assert_eq!(clock.reads.get(), 0);
    eval(
        scope,
        "new AnimationEvent('ok'); new DeviceMotionEvent('ok'); new AudioProcessingEvent('ok', {inputBuffer: buffer, outputBuffer: buffer, playbackTime: 1}); new SpeechSynthesisEvent('ok', {utterance});",
    );
    assert_eq!(clock.reads.get(), 4);
}
