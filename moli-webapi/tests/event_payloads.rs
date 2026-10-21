mod support;

use moli_v8_test_util::ensure_v8;
use moli_v8_util::{global_constructor_prototype, set_private_value, v8str};
use moli_webapi::{
    events::{
        self, EventRuntime, audio, device, ice_error_event, kind::EventSubclassKind, speech, state,
        subclasses, value, webrtc,
    },
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
    // Minimal value projection. DOM targets and isolated-world wrappers are
    // exercised by the Renderer tests, rather than implemented by this fixture.
    let names = backing.get_own_property_names(
        scope,
        v8::GetPropertyNamesArgs {
            property_filter: v8::PropertyFilter::ALL_PROPERTIES | v8::PropertyFilter::SKIP_SYMBOLS,
            ..Default::default()
        },
    )?;
    for index in 0..names.length() {
        let key = names.get_index(scope, index)?;
        let name = key.to_rust_string_lossy(scope);
        if [
            "type",
            "bubbles",
            "cancelable",
            "composed",
            "timeStamp",
            "isTrusted",
        ]
        .contains(&name.as_str())
            || subclasses::template_declaration(interface)
                .is_some_and(|declaration| declaration.attributes.contains(&name.as_str()))
        {
            continue;
        }
        let getter = v8::Function::builder(state::event_value_attribute_getter)
            .data(key)
            .build(scope)?;
        let attributes = backing.get_property_attributes(scope, key)?;
        let mut descriptor =
            v8::PropertyDescriptor::new_from_get_set(getter.into(), v8::undefined(scope).into());
        descriptor.set_enumerable(!attributes.is_dont_enum());
        descriptor.set_configurable(!attributes.is_dont_delete());
        wrapper.define_property(
            scope,
            v8::Local::<v8::Name>::try_from(key).ok()?,
            &descriptor,
        )?;
    }
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
            is_window_receiver: |scope, receiver| interfaces::Window::is_instance(scope, receiver),
            initialize_wrapper,
        },
    );
    install_errors(scope);
    let event = v8::FunctionTemplate::new(scope, events::event_constructor_callback);
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

    let ui = subclasses::build_template(scope, EventSubclassKind::UiEvent).unwrap();
    ui.inherit(event);
    finish_template(scope, interfaces::UIEvent::DESCRIPTOR, ui);
    for kind in [
        EventSubclassKind::FocusEvent,
        EventSubclassKind::CompositionEvent,
    ] {
        let template = subclasses::build_template(scope, kind).unwrap();
        template.inherit(ui);
        finish_template(
            scope,
            interfaces::descriptor(kind.constructor_name()).unwrap(),
            template,
        );
    }

    let mouse = subclasses::build_template(scope, EventSubclassKind::MouseEvent).unwrap();
    mouse.inherit(ui);
    finish_template(scope, interfaces::MouseEvent::DESCRIPTOR, mouse);
    for kind in [
        EventSubclassKind::WheelEvent,
        EventSubclassKind::PointerEvent,
        EventSubclassKind::DragEvent,
        EventSubclassKind::KeyboardEvent,
        EventSubclassKind::TouchEvent,
    ] {
        let template = subclasses::build_template(scope, kind).unwrap();
        template.inherit(
            if matches!(
                kind,
                EventSubclassKind::KeyboardEvent | EventSubclassKind::TouchEvent
            ) {
                ui
            } else {
                mouse
            },
        );
        if kind == EventSubclassKind::PointerEvent {
            // The embedder controls exposure; these callbacks only copy native payloads.
            let coalesced = v8::FunctionTemplate::new(
                scope,
                subclasses::pointer_event_get_coalesced_events_callback,
            );
            let predicted = v8::FunctionTemplate::new(
                scope,
                subclasses::pointer_event_get_predicted_events_callback,
            );
            let prototype = template.prototype_template(scope);
            prototype.set(v8str(scope, "getCoalescedEvents").into(), coalesced.into());
            prototype.set(v8str(scope, "getPredictedEvents").into(), predicted.into());
        }
        if kind == EventSubclassKind::KeyboardEvent {
            let method = v8::FunctionTemplate::new(scope, subclasses::keyboard_event_init_callback);
            template
                .prototype_template(scope)
                .set(v8str(scope, "initKeyboardEvent").into(), method.into());
        }
        if kind == EventSubclassKind::TouchEvent {
            moli_webapi::touch::install_touch_template_bindings(scope, template, "TouchEvent");
        }
        finish_template(
            scope,
            interfaces::descriptor(kind.constructor_name()).unwrap(),
            template,
        );
    }
    for interface in [
        interfaces::Touch::DESCRIPTOR,
        interfaces::TouchList::DESCRIPTOR,
    ] {
        let template = if interface.name() == "Touch" {
            v8::FunctionTemplate::new(scope, moli_webapi::touch::touch_constructor_callback)
        } else {
            v8::FunctionTemplate::new(scope, illegal_constructor)
        };
        moli_webapi::touch::install_touch_template_bindings(scope, template, interface.name());
        finish_template(scope, interface, template);
    }

    let progress =
        v8::FunctionTemplate::new(scope, events::progress::progress_event_constructor_callback);
    progress.inherit(event);
    events::progress::install_progress_event_template_bindings(scope, progress, "ProgressEvent");
    finish_template(scope, interfaces::ProgressEvent::DESCRIPTOR, progress);

    let extendable = v8::FunctionTemplate::new(
        scope,
        events::service_worker::extendable_event_constructor_callback,
    );
    extendable.inherit(event);
    finish_template(scope, interfaces::ExtendableEvent::DESCRIPTOR, extendable);
    let message = v8::FunctionTemplate::new(
        scope,
        events::service_worker::extendable_message_event_constructor_callback,
    );
    message.inherit(extendable);
    finish_template(
        scope,
        interfaces::ExtendableMessageEvent::DESCRIPTOR,
        message,
    );
    let fetch = v8::FunctionTemplate::new(scope, events::fetch::fetch_event_constructor_callback);
    fetch.inherit(extendable);
    events::fetch::install_value_template_bindings(scope, fetch);
    finish_template(scope, interfaces::FetchEvent::DESCRIPTOR, fetch);

    for kind in [
        EventSubclassKind::CustomEvent,
        EventSubclassKind::NavigateEvent,
        EventSubclassKind::NavigationCurrentEntryChangeEvent,
        EventSubclassKind::MessageEvent,
        EventSubclassKind::StorageEvent,
        EventSubclassKind::CloseEvent,
        EventSubclassKind::ClipboardEvent,
        EventSubclassKind::CommandEvent,
        EventSubclassKind::ToggleEvent,
        EventSubclassKind::InterestEvent,
        EventSubclassKind::PopStateEvent,
        EventSubclassKind::HashChangeEvent,
        EventSubclassKind::PageTransitionEvent,
        EventSubclassKind::PromiseRejectionEvent,
        EventSubclassKind::FormDataEvent,
        EventSubclassKind::SubmitEvent,
        EventSubclassKind::TrackEvent,
        EventSubclassKind::ErrorEvent,
        EventSubclassKind::SecurityPolicyViolationEvent,
        EventSubclassKind::FontFaceSetLoadEvent,
        EventSubclassKind::ClipboardChangeEvent,
        EventSubclassKind::CapturedMouseEvent,
    ] {
        let template = subclasses::build_template(scope, kind).unwrap();
        template.inherit(event);
        if let Some(declaration) = subclasses::template_declaration(kind.constructor_name()) {
            (declaration.install)(scope, template);
        }
        if kind == EventSubclassKind::SubmitEvent {
            subclasses::submit::install_template_bindings(scope, template);
        }
        if kind == EventSubclassKind::FontFaceSetLoadEvent {
            subclasses::font::install_font_face_set_load_event_template_accessors(scope, template);
        }
        finish_template(
            scope,
            interfaces::descriptor(kind.constructor_name()).unwrap(),
            template,
        );
    }
    let query = events::media_query_list_event::build_media_query_list_event_template(scope);
    query.inherit(event);
    finish_template(scope, interfaces::MediaQueryListEvent::DESCRIPTOR, query);
    let version = v8::FunctionTemplate::new(
        scope,
        events::idb_version_change_event::idb_version_change_event_constructor_callback,
    );
    version.inherit(event);
    let version_prototype = version.prototype_template(scope);
    events::idb_version_change_event::install_version_change_event_template_bindings(
        scope,
        version_prototype,
        "IDBVersionChangeEvent",
    );
    finish_template(
        scope,
        interfaces::IDBVersionChangeEvent::DESCRIPTOR,
        version,
    );
    let before_unload = v8::FunctionTemplate::new(scope, illegal_constructor);
    before_unload.inherit(event);
    (subclasses::template_declaration("BeforeUnloadEvent")
        .unwrap()
        .install)(scope, before_unload);
    finish_template(
        scope,
        interfaces::BeforeUnloadEvent::DESCRIPTOR,
        before_unload,
    );

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
    for (interface, template) in [
        (
            interfaces::RTCPeerConnectionIceEvent::DESCRIPTOR,
            v8::FunctionTemplate::new(
                scope,
                webrtc::rtc_peer_connection_ice_event_constructor_callback,
            ),
        ),
        (
            interfaces::RTCDataChannelEvent::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, webrtc::rtc_data_channel_event_constructor_callback),
        ),
        (
            interfaces::RTCErrorEvent::DESCRIPTOR,
            v8::FunctionTemplate::new(scope, webrtc::rtc_error_event_constructor_callback),
        ),
        (
            interfaces::RTCPeerConnectionIceErrorEvent::DESCRIPTOR,
            v8::FunctionTemplate::new(
                scope,
                ice_error_event::rtc_peer_connection_ice_error_event_constructor,
            ),
        ),
    ] {
        template.inherit(event);
        if interface.name() == "RTCPeerConnectionIceErrorEvent" {
            let prototype = template.prototype_template(scope);
            ice_error_event::install(scope, prototype);
        } else {
            webrtc::install_event_template_bindings(scope, template, interface.name());
        }
        finish_template(scope, interface, template);
    }

    // Native payload brands without a DOM, audio graph, payment or sensor host.
    for (interface, name) in [
        (interfaces::Window::DESCRIPTOR, "windowValue"),
        (
            interfaces::InputDeviceCapabilities::DESCRIPTOR,
            "capabilitiesValue",
        ),
        (interfaces::CSSAnimation::DESCRIPTOR, "animation"),
        (interfaces::CSSTransition::DESCRIPTOR, "transition"),
        (interfaces::Blob::DESCRIPTOR, "blob"),
        (interfaces::Gamepad::DESCRIPTOR, "gamepad"),
        (interfaces::PictureInPictureWindow::DESCRIPTOR, "pip"),
        (interfaces::MIDIPort::DESCRIPTOR, "port"),
        (interfaces::RTCIceCandidate::DESCRIPTOR, "candidate"),
        (interfaces::RTCDataChannel::DESCRIPTOR, "channel"),
        (interfaces::AudioBuffer::DESCRIPTOR, "buffer"),
        (interfaces::DataTransfer::DESCRIPTOR, "clipboard"),
        (interfaces::MessagePort::DESCRIPTOR, "endpoint"),
        (interfaces::Storage::DESCRIPTOR, "storage"),
        (interfaces::FormData::DESCRIPTOR, "form"),
        (interfaces::TextTrack::DESCRIPTOR, "track"),
        (interfaces::FontFace::DESCRIPTOR, "font"),
        (interfaces::Element::DESCRIPTOR, "sourceElement"),
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

#[test]
fn rtc_payloads_keep_native_references_and_string_conversion_kinds() {
    check(
        r#"(() => {
      const ice = new RTCPeerConnectionIceEvent('ice\ud800', {candidate, url: 'stun:\ud800', bubbles: true});
      assert(ice instanceof Event && ice.candidate === candidate);
      assert(ice.type === 'ice\ud800' && ice.url === 'stun:\ufffd');
      assert(ice.bubbles && ice.timeStamp === 37.25 && !ice.isTrusted);
      assert(new RTCPeerConnectionIceEvent('empty').candidate === null);
      const data = new RTCDataChannelEvent('data', {channel});
      assert(data.channel === channel);
      const error = new RTCError({errorDetail: 'data-channel-failure'}, 'detail');
      assert(new RTCErrorEvent('error', {error}).error === error);
      const failure = new RTCPeerConnectionIceErrorEvent('failure', {
        address: '\ud800', port: 65537, url: 'stun:\ud800', errorCode: 701, errorText: 'x\udfff'
      });
      assert(failure.address === '\ud800' && failure.port === 1);
      assert(failure.url === 'stun:\ufffd' && failure.errorText === 'x\ufffd' && failure.errorCode === 701);
      Object.defineProperty(failure, 'errorCode', {get() {throw Error('public shadow');}});
      assert(Object.getOwnPropertyDescriptor(RTCPeerConnectionIceErrorEvent.prototype, 'errorCode').get.call(failure) === 701);
      return true;
    })()"#,
    );
}

#[test]
fn rtc_payload_conversion_order_and_native_receiver_checks() {
    check(
        r#"(() => {
      function fails(run) { try {run();} catch (e) {assert(e instanceof TypeError); return;} throw Error('missing TypeError'); }
      fails(() => new RTCDataChannelEvent('x', {channel: Object.create(RTCDataChannel.prototype)}));
      fails(() => new RTCErrorEvent('x', {}));
      fails(() => new RTCPeerConnectionIceErrorEvent('x', {}));
      const getter = Object.getOwnPropertyDescriptor(RTCPeerConnectionIceEvent.prototype, 'candidate').get;
      fails(() => getter.call(Object.create(RTCPeerConnectionIceEvent.prototype)));
      let reads = [];
      const init = new Proxy({errorCode: 701}, {get(target, key) {reads.push(key); return target[key];}});
      new RTCPeerConnectionIceErrorEvent('x', init);
      assert(reads.join(',') === 'bubbles,cancelable,composed,address,errorCode,errorText,port,url');
      return true;
    })()"#,
    );
}

#[test]
fn value_subclass_constructors_keep_payloads_flags_and_own_property_shapes() {
    check(
        r#"(() => {
        const details = {}, promise = Promise.resolve(1);
        const cases = [
            [CustomEvent, {detail: details}, 'detail', details],
            [CloseEvent, {code: -1, reason: 'bye\ud800', wasClean: true}, 'reason', 'bye\ud800'],
            [ClipboardEvent, {clipboardData: clipboard}, 'clipboardData', clipboard],
            [CommandEvent, {command: 'go\ud800', source: sourceElement}, 'source', sourceElement],
            [ToggleEvent, {oldState: 'closed', newState: 'open', source: sourceElement}, 'newState', 'open'],
            [InterestEvent, {source: sourceElement}, 'source', sourceElement],
            [PopStateEvent, {state: details, hasUAVisualTransition: true}, 'state', details],
            [HashChangeEvent, {oldURL: '#a\ud800', newURL: '#b'}, 'oldURL', '#a\ud800'],
            [PageTransitionEvent, {persisted: true}, 'persisted', true],
            [PromiseRejectionEvent, {promise, reason: details}, 'promise', promise],
            [SubmitEvent, {agentInvoked: true}, 'agentInvoked', true],
            [TrackEvent, {track}, 'track', track],
            [FontFaceSetLoadEvent, {fontfaces: [font]}, 'fontfaces', null],
        ];
        for (const [Ctor, init, property, expected] of cases) {
            class Derived extends Ctor {}
            const e = new Derived('type\ud800', {...init, bubbles: true, cancelable: true, composed: true});
            assert(e instanceof Derived && e instanceof Ctor && e instanceof Event, Ctor.name);
            assert(e.type === 'type\ud800' && e.bubbles && e.cancelable && e.composed);
            assert(e.timeStamp === 37.25 && e.isTrusted === false);
            if (Ctor === FontFaceSetLoadEvent) {
                assert(e.fontfaces.length === 1 && e.fontfaces[0] === font && Object.isFrozen(e.fontfaces));
            } else assert(e[property] === expected, Ctor.name + '.' + property);
        }
        const custom = new CustomEvent('c', {detail: details});
        assert(!Object.hasOwn(custom, 'detail') && !Object.hasOwn(custom, 'initCustomEvent'), 'CustomEvent own fields');
        const detail = Object.getOwnPropertyDescriptor(CustomEvent.prototype, 'detail');
        assert(typeof detail.get === 'function' && detail.set === undefined && detail.enumerable && detail.configurable, 'CustomEvent detail descriptor');
        assert(detail.get.call(custom) === details, 'CustomEvent detail value');
        for (const Ctor of [CommandEvent, ToggleEvent]) {
            const event = new Ctor('source', {source: sourceElement});
            const source = Object.getOwnPropertyDescriptor(Ctor.prototype, 'source');
            assert(!Object.hasOwn(event, 'source') && source.get.call(event) === sourceElement, Ctor.name + ' source placement');
            assert(source.enumerable && source.configurable && source.set === undefined, Ctor.name + ' source descriptor');
        }
        const toggle = new ToggleEvent('t', {newState: 'open'});
        const d = Object.getOwnPropertyDescriptor(ToggleEvent.prototype, 'newState');
        assert(!Object.hasOwn(toggle, 'newState') && d.enumerable && d.configurable && typeof d.get === 'function' && d.set === undefined, 'ToggleEvent state descriptor ' + JSON.stringify(d));
        assert(!Object.hasOwn(new CommandEvent('c'), 'command'), 'CommandEvent command placement');
        assert(new CloseEvent('c', {code: -1}).code === 65535);
        assert(!Object.hasOwn(new HashChangeEvent('h'), 'oldURL'));
        return true;
    })()"#,
    );
}

#[test]
fn value_event_conversion_order_preserves_utf16_and_usv_string_distinctions() {
    check(
        r#"(() => {
        const reads = [];
        const dictionary = new Proxy({message: '\ud800', filename: '\ud800', error: 7}, {
            get(target, name) {reads.push(name); return target[name];}
        });
        const e = new ErrorEvent({toString() {reads.push('type'); return 'error';}}, dictionary);
        assert(reads.join(',') === 'type,bubbles,cancelable,composed,colno,error,filename,lineno,message', reads);
        assert(e.message === '\ud800' && e.filename === '\ufffd' && e.error === 7);
        assert(!Object.hasOwn(e, 'message'));
        const cspReads = [];
        const csp = new SecurityPolicyViolationEvent('violation', new Proxy({
            blockedURI: '\ud800', documentURI: '\ud800', sourceFile: '\ud800',
            referrer: '\ud800', sample: '\ud800', statusCode: 65537, disposition: 'report'
        }, {get(target, name) {cspReads.push(name); return target[name];}}));
        assert(cspReads.join(',') === 'bubbles,cancelable,composed,blockedURI,columnNumber,disposition,documentURI,effectiveDirective,lineNumber,originalPolicy,referrer,sample,sourceFile,statusCode,violatedDirective', cspReads);
        assert(csp.blockedURI === '\ufffd' && csp.documentURI === '\ufffd' && csp.sourceFile === '\ufffd');
        assert(csp.referrer === '\ufffd' && csp.sample === '\ud800' && csp.statusCode === 1 && csp.disposition === 'report');
        const toggleReads = [];
        new ToggleEvent('t', new Proxy({}, {get(_, key) {toggleReads.push(key);}}));
        assert(toggleReads.join(',') === 'bubbles,cancelable,composed,newState,oldState,source', toggleReads);
        return true;
    })()"#,
    );
}

#[test]
fn event_subclass_validation_precedes_timestamp_and_preserves_thrown_values() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 12.0);
    assert!(
        eval(
            scope,
            r#"(() => {
        for (const [Ctor, init] of [
            [FormDataEvent, {}], [FormDataEvent, {formData: {}}],
            [TrackEvent, {track: {}}], [ClipboardEvent, {clipboardData: {}}],
            [CommandEvent, {source: {}}], [SecurityPolicyViolationEvent, {disposition: 'invalid'}],
            [FontFaceSetLoadEvent, {fontfaces: [{}]}], [ClipboardChangeEvent, {changeId: 1}]
        ]) {
            let caught;
            try { new Ctor('x', init); } catch (error) {caught = error;}
            assert(caught instanceof TypeError, Ctor.name);
        }
        for (const Ctor of [PromiseRejectionEvent, FormDataEvent]) {
            let converted = false, caught;
            try {new Ctor({toString() {converted = true; return 'x';}});} catch (e) {caught = e;}
            assert(caught instanceof TypeError && !converted);
        }
        const sentinel = {};
        let caught;
        try {new CustomEvent('c', {get detail() {throw sentinel;}});} catch (e) {caught = e;}
        assert(caught === sentinel);
        return true;
    })()"#
        )
        .is_true()
    );
    assert_eq!(
        scope
            .get_current_context()
            .get_slot::<Clock>()
            .unwrap()
            .reads
            .get(),
        0
    );
}

#[test]
fn clipboard_change_freezes_lossless_sequences_and_applies_bigint_conversion() {
    check(
        r#"(() => {
        const order = [];
        const e = new ClipboardChangeEvent('change', {
            get types() {order.push('types'); return new Set(['text/plain\ud800', {toString() {order.push('string'); return 'x';}}]);},
            get changeId() {order.push('changeId'); return {valueOf() {order.push('bigint'); return 23n;}};}
        });
        assert(order.join(',') === 'changeId,bigint,types,string', order);
        assert(e.changeId === 23n && e.types[0] === 'text/plain\ud800' && e.types[1] === 'x');
        assert(Object.isFrozen(e.types) && e.types === e.types);
        assert(new ClipboardChangeEvent('c').changeId === 0n && new ClipboardChangeEvent('c').types.length === 0);
        return true;
    })()"#,
    );
}

#[test]
fn legacy_custom_init_is_blocked_during_dispatch_and_keeps_private_values() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 37.25);
    let event = eval(
        scope,
        "globalThis.custom = new CustomEvent('first', {detail: 'old'}); custom;",
    );
    let event = v8::Local::<v8::Object>::try_from(event).unwrap();
    state::set_event_dispatching(scope, event, true);
    assert!(eval(scope, "custom.initCustomEvent('ignored', true, true, 2); custom.type === 'first' && custom.detail === 'old'").is_true());
    state::set_event_dispatching(scope, event, false);
    assert!(eval(scope, "custom.initCustomEvent('next\\ud800', true, true, 9); custom.type === 'next\\ud800' && custom.detail === 9 && custom.bubbles && custom.cancelable").is_true());
    assert!(subclasses::event_subclass_kind(scope, event) == Some(EventSubclassKind::CustomEvent));
}

#[test]
fn native_error_and_before_unload_factories_ignore_replaced_public_constructors() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 37.25);
    eval(
        scope,
        "globalThis.OriginalErrorEvent = ErrorEvent; globalThis.OriginalBeforeUnloadEvent = BeforeUnloadEvent; ErrorEvent = BeforeUnloadEvent = function() {throw new Error('public constructor');};",
    );
    let error = v8::Integer::new(scope, 7).into();
    let event =
        subclasses::error::construct_original_error_event(scope, "message", "file", 4, 2, error)
            .unwrap();
    publish(scope, "nativeError", event);
    let event = subclasses::new_uninitialized_before_unload_event(scope).unwrap();
    publish(scope, "beforeUnload", event);
    assert!(eval(scope, r#"(() => {
        assert(nativeError instanceof OriginalErrorEvent && nativeError.message === 'message' && nativeError.error === 7);
        assert(beforeUnload instanceof OriginalBeforeUnloadEvent && beforeUnload.returnValue === '');
        beforeUnload.returnValue = {toString() {return 'leave\ud800';}};
        assert(beforeUnload.returnValue === 'leave\ud800');
        return true;
    })()"#).is_true());
    assert_eq!(state::event_initialized(scope, event), Some(false));
}

#[test]
fn message_and_storage_events_preserve_native_references_and_conversion_order() {
    check(
        r#"(() => {
        const payload = {}, order = [];
        const message = new MessageEvent('message', new Proxy({
            data: payload, origin: '\ud800', lastEventId: '\ud800', ports: [endpoint], source: endpoint,
        }, {get(target, name) {order.push(name); return target[name];}}));
        assert(order.join(',') === 'bubbles,cancelable,composed,data,lastEventId,origin,ports,source', order);
        assert(message.data === payload && message.source === endpoint && message.ports[0] === endpoint);
        assert(message.origin === '\ufffd' && message.lastEventId === '\ud800' && Object.isFrozen(message.ports));
        const storageOrder = [];
        const storageEvent = new StorageEvent('storage', new Proxy({
            key: '\ud800', newValue: '\udfff', oldValue: null, storageArea: storage, url: '\ud800',
        }, {get(target, name) {storageOrder.push(name); return target[name];}}));
        assert(storageOrder.join(',') === 'bubbles,cancelable,composed,key,newValue,oldValue,storageArea,url', storageOrder);
        assert(storageEvent.storageArea === storage && storageEvent.key === '\ud800');
        assert(storageEvent.newValue === '\udfff' && storageEvent.oldValue === null && storageEvent.url === '\ufffd');
        for (const [Ctor, init] of [
            [MessageEvent, {source: {}}], [MessageEvent, {ports: [{}]}],
            [StorageEvent, {storageArea: {}}],
        ]) {
            let caught; try {new Ctor('invalid', init);} catch (error) {caught = error;}
            assert(caught instanceof TypeError, Ctor.name);
        }
        return true;
    })()"#,
    );
}

#[test]
fn message_and_storage_legacy_initializers_keep_converted_payloads() {
    check(
        r#"(() => {
        const payload = {}, message = new MessageEvent('before', {composed: true});
        message.initMessageEvent('after\ud800', true, true, payload, '\ud800', '\udfff', endpoint, [endpoint]);
        assert(message.type === 'after\ud800' && message.bubbles && message.cancelable && message.composed);
        assert(message.data === payload && message.origin === '\ufffd' && message.lastEventId === '\udfff');
        assert(message.source === endpoint && message.ports[0] === endpoint && Object.isFrozen(message.ports));
        const e = new StorageEvent('before', {composed: true});
        e.initStorageEvent('after', true, false, '\ud800', null, '\udfff', '\ud800', storage);
        assert(e.type === 'after' && e.bubbles && !e.cancelable && e.composed);
        assert(e.key === '\ud800' && e.oldValue === null && e.newValue === '\udfff' && e.url === '\ufffd');
        assert(e.storageArea === storage);
        return true;
    })()"#,
    );
}

#[test]
fn native_message_and_storage_factories_do_not_read_author_constructors_or_iterators() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 37.25);
    let endpoint = eval(scope, "endpoint");
    let storage = eval(scope, "storage");
    let ports = v8::Array::new_with_elements(scope, &[endpoint]);
    let data = v8::Integer::new(scope, 7).into();
    eval(
        scope,
        r#"
        globalThis.OriginalMessage = MessageEvent; globalThis.OriginalStorage = StorageEvent;
        for (const name of ['MessageEvent', 'StorageEvent']) {
            Object.defineProperty(globalThis, name, {configurable: true, get() {throw new Error('public constructor');}});
        }
        Object.defineProperty(Array.prototype, Symbol.iterator, {configurable: true, get() {throw new Error('author iterator');}});
    "#,
    );
    let message = subclasses::message::construct_original_message_event(
        scope,
        "message",
        data,
        "origin",
        &[0xd800],
        endpoint,
        ports,
    )
    .unwrap();
    publish(scope, "nativeMessage", message);
    let storage = subclasses::storage::construct_original_storage_event_utf16(
        scope,
        "storage",
        Some(&[0xd800]),
        None,
        Some(&[0xdfff]),
        "url",
        Some(storage),
    )
    .unwrap();
    publish(scope, "nativeStorage", storage);
    assert!(eval(scope, r#"(() => {
        assert(nativeMessage instanceof OriginalMessage && nativeMessage.data === 7);
        assert(nativeMessage.source === endpoint && nativeMessage.ports[0] === endpoint && Object.isFrozen(nativeMessage.ports));
        assert(nativeMessage.lastEventId === '\ud800');
        assert(nativeStorage instanceof OriginalStorage && nativeStorage.storageArea === storage);
        assert(nativeStorage.key === '\ud800' && nativeStorage.newValue === '\udfff');
        return true;
    })()"#).is_true());
}

#[test]
fn standalone_media_query_and_version_events_preserve_dictionary_order_and_utf16() {
    check(
        r#"(() => {
      const reads=[];
      const init={};for(const name of ['bubbles','cancelable','composed','matches','media'])Object.defineProperty(init,name,{get(){reads.push(name);return name==='media'?'screen\ud800':true;}});
      const query=new MediaQueryListEvent('change',init);
      assert(reads.join()==='bubbles,cancelable,composed,matches,media');
      assert(query.media==='screen\ud800' && query.matches && query.composed && query instanceof Event);
      const version=new IDBVersionChangeEvent('versionchange',{oldVersion:2,newVersion:3});
      assert(version.oldVersion===2 && version.newVersion===3 && version.isTrusted===false && version instanceof Event);
      assert(new IDBVersionChangeEvent('blocked').newVersion===null);
      const captured=new CapturedMouseEvent('move',{surfaceX:4,surfaceY:5});
      assert(captured.surfaceX===4 && captured.surfaceY===5 && captured instanceof Event);
      try{new CapturedMouseEvent('move',{surfaceX:-1,surfaceY:2});throw Error('invalid coordinates accepted');}catch(error){assert(error instanceof RangeError);}
      return true;
    })()"#,
    );
}
#[test]
fn standalone_native_media_query_event_keeps_trust_and_intrinsic_after_global_replacement() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 37.25);
    eval(
        scope,
        "globalThis.NativeQueryEvent=MediaQueryListEvent;globalThis.MediaQueryListEvent=function(){throw 'author constructor'};",
    );
    let event =
        events::media_query_list_event::create_media_query_list_event(scope, "screen", true);
    publish(scope, "nativeQuery", event);
    assert!(eval(scope,"nativeQuery instanceof NativeQueryEvent && nativeQuery.media==='screen' && nativeQuery.matches && nativeQuery.type==='change' && nativeQuery.isTrusted && nativeQuery.timeStamp===37.25").is_true());
}

#[test]
fn standalone_event_and_progress_constructors_share_realm_clock_and_native_projection() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 81.5);
    eval(
        scope,
        r#"(() => {
      const assert = (ok) => { if (!ok) throw new Error('event construction'); };
      const event = new Event('basic', {bubbles: true, composed: true});
      assert(event.type === 'basic' && event.bubbles && event.composed && !event.isTrusted);
      assert(event.timeStamp === 81.5);
      const order = [], init = {};
      for (const key of ['bubbles', 'cancelable', 'composed', 'lengthComputable', 'loaded', 'total']) {
        Object.defineProperty(init, key, {get() { order.push(key); return key === 'loaded' ? 4 : key === 'total' ? 10 : true; }});
      }
      const progress = new ProgressEvent('progress', init);
      assert(order.join() === 'bubbles,cancelable,composed,lengthComputable,loaded,total');
      assert(progress instanceof Event && progress instanceof ProgressEvent);
      assert(progress.lengthComputable && progress.loaded === 4 && progress.total === 10);
      assert(progress.timeStamp === 81.5 && !progress.isTrusted);
      globalThis.NativeProgress = ProgressEvent;
      globalThis.ProgressEvent = function() { throw new Error('author constructor'); };
      return true;
    })()"#,
    );
    let event =
        events::progress::construct_native_progress_event(scope, "load", true, 3.0, 7.0).unwrap();
    publish(scope, "nativeProgress", event);
    assert!(eval(scope, "nativeProgress instanceof NativeProgress && nativeProgress.isTrusted && nativeProgress.timeStamp === 81.5 && nativeProgress.loaded === 3 && nativeProgress.total === 7").is_true());
}

#[test]
fn standalone_navigation_and_worker_events_preserve_native_references_and_promises() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 64.0);
    // The embedder supplies already-resolved native identities. These value
    // constructors do not allocate a Request, history entry or AbortSignal owner.
    for (name, interface) in [
        ("requestValue", interfaces::Request::DESCRIPTOR),
        ("entryValue", interfaces::NavigationHistoryEntry::DESCRIPTOR),
        (
            "destinationValue",
            interfaces::NavigationDestination::DESCRIPTOR,
        ),
        ("signalValue", interfaces::AbortSignal::DESCRIPTOR),
        ("workerValue", interfaces::ServiceWorker::DESCRIPTOR),
    ] {
        let template = v8::FunctionTemplate::new(scope, illegal_constructor);
        finish_template(scope, interface, template);
        let object = v8::Object::new(scope);
        interface.initialize(scope, object).unwrap();
        publish(scope, name, object);
    }
    assert!(eval(scope, r#"(() => {
      const check = ok => { if (!ok) throw new Error('event values'); };
      const entry = new NavigationCurrentEntryChangeEvent('currententrychange', {from:entryValue, navigationType:'replace'});
      check(entry.from === entryValue && entry.navigationType === 'replace' && entry.timeStamp === 64);
      check(new NavigationCurrentEntryChangeEvent('change', {from:entryValue}).navigationType === null);
      const navigate = new NavigateEvent('navigate', {destination:destinationValue, signal:signalValue, canIntercept:true});
      check(navigate.destination === destinationValue && navigate.signal === signalValue && navigate.canIntercept);
      const extendable = new ExtendableEvent('activate', {bubbles:true, composed:true});
      check(extendable.type === 'activate' && extendable.bubbles && extendable.composed);
      const order = [], messageInit = {};
      Object.defineProperty(workerValue, 'scriptURL', {get() {throw new Error('identity must not read scriptURL');}});
      for (const key of ['bubbles', 'cancelable', 'composed', 'data', 'lastEventId', 'origin', 'ports', 'source']) {
        Object.defineProperty(messageInit, key, {get() {
          order.push(key);
          return ({data:7, lastEventId:'id\ud800', origin:'https://example.test', ports:new Set([endpoint]), source:workerValue})[key];
        }});
      }
      const message = new ExtendableMessageEvent('message', messageInit);
      check(order.join() === 'bubbles,cancelable,composed,data,lastEventId,origin,ports,source');
      check(message.data === 7 && message.source === workerValue && message.ports[0] === endpoint && Object.isFrozen(message.ports));
      check(message.lastEventId === 'id\ud800');
      const fetch = new FetchEvent('fetch', {request:requestValue, clientId:'client', preloadResponse:7});
      check(fetch instanceof ExtendableEvent && fetch.request === requestValue && fetch.clientId === 'client');
      check(fetch.preloadResponse instanceof Promise && fetch.handled instanceof Promise);
      check(fetch.resultingClientId === '' && fetch.replacesClientId === '');
      try { new FetchEvent('fetch', {request:{}}); throw new Error('unbranded'); }
      catch (error) { check(error instanceof TypeError); }
      return true;
    })()"#).is_true());
}

#[test]
fn ui_event_values_convert_base_first_without_a_document_or_dispatcher() {
    check(
        r#"(() => {
      const order = [], values = {
        bubbles:true, cancelable:true, composed:true, detail:4294967297.9,
        sourceCapabilities:capabilitiesValue, view:windowValue, relatedTarget:sourceElement
      };
      const init = new Proxy(values, {get(value, key) {order.push(key); return value[key];}});
      const event = new FocusEvent({toString() {order.push('type'); return 'focus';}}, init);
      assert(order.join() === 'type,bubbles,cancelable,composed,detail,sourceCapabilities,view,relatedTarget');
      assert(event instanceof Event && event instanceof UIEvent && event instanceof FocusEvent);
      assert(event.type === 'focus' && event.bubbles && event.cancelable && event.composed);
      assert(event.detail === 1 && event.sourceCapabilities === capabilitiesValue);
      assert(event.view === windowValue && event.relatedTarget === sourceElement);
      assert(new UIEvent('default').view === null && new FocusEvent('default').relatedTarget === null);
      assert(new CompositionEvent('input', {view:windowValue, data:'before\ud800after'}).data === 'before\ud800after');
      const OriginalUI = UIEvent;
      globalThis.UIEvent = function() {throw new Error('public UI constructor');};
      assert(new CompositionEvent('input') instanceof OriginalUI);
      let traps = 0;
      const proxy = new Proxy(windowValue, {
        get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 42;}
      });
      for (const fake of [{}, Object.create(Window.prototype), proxy]) {
        let error;
        try {new FocusEvent('bad', {view:fake});} catch (value) {error=value;}
        assert(error instanceof TypeError && traps === 0);
      }
      const sentinel = new Error('view conversion');
      try {new CompositionEvent('bad', {get view() {throw sentinel;}});}
      catch (error) {assert(error === sentinel); return true;}
      throw new Error('view getter was not read');
    })()"#,
    );
}

#[test]
fn ui_event_window_checks_use_the_embedder_predicate_before_initializing_state() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install(scope, 23.5);
    events::install_runtime(
        scope,
        EventRuntime {
            timestamp,
            is_window_receiver: |_, _| false,
            initialize_wrapper,
        },
    );
    assert!(
        eval(
            scope,
            r#"(() => {
      for (const Constructor of [UIEvent, FocusEvent, CompositionEvent, MouseEvent, WheelEvent,
        PointerEvent, DragEvent, KeyboardEvent, TouchEvent]) {
        let error;
        try {new Constructor('denied', {view:windowValue});} catch (value) {error=value;}
        if (!(error instanceof TypeError)) return false;
      }
      return true;
    })()"#
        )
        .is_true()
    );
    assert_eq!(
        scope
            .get_current_context()
            .get_slot::<Clock>()
            .unwrap()
            .reads
            .get(),
        0
    );
    events::install_runtime(
        scope,
        EventRuntime {
            timestamp,
            is_window_receiver: |scope, receiver| interfaces::Window::is_instance(scope, receiver),
            initialize_wrapper,
        },
    );
    assert!(
        eval(
            scope,
            "new UIEvent('allowed', {view:windowValue}).timeStamp === 23.5"
        )
        .is_true()
    );
    assert_eq!(
        scope
            .get_current_context()
            .get_slot::<Clock>()
            .unwrap()
            .reads
            .get(),
        1
    );
}

#[test]
fn pointing_event_payloads_and_pointer_snapshots_work_without_a_page() {
    check(
        r#"(() => {
      const mouse = new MouseEvent('move', {view:windowValue, relatedTarget:sourceElement,
        clientX:1.25, clientY:2.5, screenX:7, screenY:8, button:1, buttons:3, altKey:true});
      assert(mouse instanceof UIEvent && mouse.view === windowValue && mouse.relatedTarget === sourceElement);
      assert(mouse.clientX === 1.25 && mouse.x === 1.25 && mouse.pageY === 2.5);
      assert(mouse.screenX === 7 && mouse.button === 1 && mouse.buttons === 3 && mouse.altKey);
      const wheel = new WheelEvent('wheel', {deltaX:1.5, deltaY:-2.25, deltaMode:1});
      assert(wheel instanceof MouseEvent && wheel.deltaMode === 1 && wheel.deltaY === -2.25);
      assert(wheel.wheelDeltaX === 1 && wheel.wheelDeltaY === -2 && wheel.wheelDelta === -2);
      const sample = new PointerEvent('sample', {pointerId:1});
      const pointer = new PointerEvent('pointer', {view:windowValue, pointerId:3, tiltX:30,
        pressure:.5, pointerType:'pen\ud800', coalescedEvents:[sample], predictedEvents:[sample]});
      assert(pointer instanceof MouseEvent && pointer.pointerId === 3 && pointer.pressure === .5);
      assert(pointer.tiltX === 30 && pointer.tiltY === 0 && pointer.azimuthAngle === 0);
      assert(Math.abs(pointer.altitudeAngle - Math.PI/3) < 1e-12 && pointer.pointerType === 'pen\ud800');
      for (const method of ['getCoalescedEvents','getPredictedEvents']) {
        const first = pointer[method](), second = pointer[method]();
        assert(first !== second && first.length === 1 && first[0] === sample && second[0] === sample);
        first.length = 0;
        assert(pointer[method]().length === 1);
      }
      const drag = new DragEvent('drag', {view:windowValue, dataTransfer:clipboard});
      assert(drag instanceof MouseEvent && drag.dataTransfer === clipboard);
      for (const [Constructor, init] of [
        [DragEvent, {dataTransfer:{}}],
        [PointerEvent, {coalescedEvents:[Object.create(PointerEvent.prototype)]}]
      ]) {
        let error;
        try {new Constructor('bad', init);} catch (value) {error=value;}
        assert(error instanceof TypeError);
      }
      return true;
    })()"#,
    );
}

#[test]
fn keyboard_event_values_and_legacy_reinitialization_share_the_native_state() {
    check(
        r#"(() => {
      const order = [], values = {view:windowValue, altKey:true, modifierAltGraph:true,
        key:'key\ud800', code:'code\udfff', which:4294967297.9, repeat:true};
      const event = new KeyboardEvent('keydown', new Proxy(values, {
        get(value, key) {order.push(key); return value[key];}
      }));
      assert(order.slice(0,6).join() === 'bubbles,cancelable,composed,detail,sourceCapabilities,view');
      assert(order.indexOf('shiftKey') < order.indexOf('charCode') && order.filter(key => key === 'view').length === 1);
      assert(event instanceof UIEvent && event.view === windowValue && event.altKey);
      assert(event.key === 'key\ud800' && event.code === 'code\udfff' && event.which === 1 && event.repeat);
      event.initKeyboardEvent('keyup', true, true, windowValue, 'new\ud800', 2, true, false, true, false);
      assert(event.type === 'keyup' && event.bubbles && event.cancelable && event.view === windowValue);
      assert(event.key === 'new\ud800' && event.location === 2 && !event.repeat);
      assert(event.ctrlKey && event.shiftKey && !event.altKey && !event.metaKey);
      return true;
    })()"#,
    );
}

#[test]
fn touch_event_lists_preserve_native_identity_without_a_document_or_public_constructors() {
    check(
        r#"(() => {
      const NativeTouch = Touch, NativeList = TouchList;
      const touch = new NativeTouch({identifier:7, target:sourceElement, clientX:3.5});
      Object.defineProperty(globalThis, 'Touch', {configurable:true, get() {throw new Error('public Touch');}});
      Object.defineProperty(globalThis, 'TouchList', {configurable:true, get() {throw new Error('public TouchList');}});
      const event = new TouchEvent('touchstart', {view:windowValue, altKey:true,
        touches:new Set([touch]), targetTouches:[touch], changedTouches:[touch]});
      assert(event instanceof UIEvent && event.view === windowValue && event.altKey);
      assert(event.touches instanceof NativeList && event.targetTouches instanceof NativeList);
      assert(event.touches !== event.targetTouches && event.touches[0] === touch && event.targetTouches[0] === touch);
      assert(event.changedTouches.item(0) === touch && [...event.touches][0] === touch);
      assert(touch.target === sourceElement && touch.clientX === 3.5 && event.touches.item(1) === null);
      const second = new TouchEvent('next', {touches:[touch]});
      assert(second.touches !== event.touches && second.touches[0] === touch);
      let error;
      try {new TouchEvent('bad', {touches:[Object.create(NativeTouch.prototype)]});} catch (value) {error=value;}
      assert(error instanceof TypeError);
      return true;
    })()"#,
    );
}
