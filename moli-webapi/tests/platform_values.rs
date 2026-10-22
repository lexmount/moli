mod support;

use moli_v8_test_util::ensure_v8;
use moli_v8_util::v8_string;
use moli_webapi::{
    css_values, dom_exception, dom_string_list, geolocation, interfaces, queuing_strategy, rtc,
};
use std::pin::pin;
use support::{eval, finish_template, illegal_constructor, install_errors, publish};

fn install_values(scope: &mut v8::PinScope<'_, '_>) {
    install_errors(scope);
    let receiver = v8::FunctionTemplate::new(scope, illegal_constructor);
    rtc::install_template_bindings(scope, receiver, "RTCRtpReceiver");
    finish_template(scope, interfaces::RTCRtpReceiver::DESCRIPTOR, receiver);
    let description = v8::FunctionTemplate::new(
        scope,
        rtc::session_description::rtc_session_description_constructor_callback,
    );
    let candidate = v8::FunctionTemplate::new(
        scope,
        rtc::ice_candidate::rtc_ice_candidate_constructor_callback,
    );
    let audio = v8::FunctionTemplate::new(scope, rtc::encoded_frames::audio_constructor);
    let video = v8::FunctionTemplate::new(scope, rtc::encoded_frames::video_constructor);
    let style = v8::FunctionTemplate::new(scope, illegal_constructor);
    let numeric = v8::FunctionTemplate::new(scope, illegal_constructor);
    numeric.inherit(style);
    let keyword =
        v8::FunctionTemplate::new(scope, css_values::css_keyword_value_constructor_callback);
    keyword.inherit(style);
    let unit = v8::FunctionTemplate::new(scope, css_values::css_unit_value_constructor_callback);
    unit.inherit(numeric);
    let count = v8::FunctionTemplate::new(
        scope,
        queuing_strategy::count_queuing_strategy_constructor_callback,
    );
    let bytes = v8::FunctionTemplate::new(
        scope,
        queuing_strategy::byte_length_queuing_strategy_constructor_callback,
    );
    let names = v8::FunctionTemplate::new(scope, illegal_constructor);
    let coordinates = v8::FunctionTemplate::new(scope, illegal_constructor);
    let position = v8::FunctionTemplate::new(scope, illegal_constructor);
    let geo_error = v8::FunctionTemplate::new(scope, illegal_constructor);
    for (interface, template) in [
        (interfaces::RTCSessionDescription::DESCRIPTOR, description),
        (interfaces::RTCIceCandidate::DESCRIPTOR, candidate),
        (interfaces::RTCEncodedAudioFrame::DESCRIPTOR, audio),
        (interfaces::RTCEncodedVideoFrame::DESCRIPTOR, video),
        (interfaces::CSSStyleValue::DESCRIPTOR, style),
        (interfaces::CSSNumericValue::DESCRIPTOR, numeric),
        (interfaces::CSSKeywordValue::DESCRIPTOR, keyword),
        (interfaces::CSSUnitValue::DESCRIPTOR, unit),
        (interfaces::CountQueuingStrategy::DESCRIPTOR, count),
        (interfaces::ByteLengthQueuingStrategy::DESCRIPTOR, bytes),
        (interfaces::DOMStringList::DESCRIPTOR, names),
        (interfaces::GeolocationCoordinates::DESCRIPTOR, coordinates),
        (interfaces::GeolocationPosition::DESCRIPTOR, position),
        (interfaces::GeolocationPositionError::DESCRIPTOR, geo_error),
    ] {
        let prototype = template.prototype_template(scope);
        match interface.name() {
            "RTCSessionDescription" => {
                rtc::session_description::install_session_description_template_bindings(
                    scope, template,
                )
            }
            "RTCIceCandidate" => {
                rtc::ice_candidate::install_ice_candidate_template_bindings(scope, template)
            }
            "RTCEncodedAudioFrame" | "RTCEncodedVideoFrame" => {
                rtc::encoded_frames::install(scope, prototype, interface.name())
            }
            _ => {}
        }
        css_values::install_typed_value_template_bindings(scope, template, interface.name());
        queuing_strategy::install_queuing_strategy_template_bindings(
            scope,
            prototype,
            interface.name(),
        );
        dom_string_list::install_dom_string_list_template_bindings(
            scope,
            prototype,
            interface.name(),
        );
        geolocation::install(scope, template, interface.name());
        finish_template(scope, interface, template);
    }
    eval(
        scope,
        "function check(value, message) { if (!value) throw new Error(message || 'check failed'); }",
    );
}

fn with_values(test: impl for<'s, 'i> FnOnce(&mut v8::PinScope<'s, 'i>)) {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    install_values(scope);
    test(scope);
}

#[test]
fn native_error_values_keep_utf16_inheritance_constants_and_dictionary_conversions() {
    with_values(|scope| {
        let socket = dom_exception::new_websocket_error_value(scope, "closed", Some(1001), "away")
            .to_object(scope)
            .unwrap();
        publish(scope, "socketError", socket);
        let error = dom_exception::new_dom_error_value(scope, "ReadError", "failed")
            .to_object(scope)
            .unwrap();
        publish(scope, "legacyError", error);
        eval(
            scope,
            r#"(() => {
            const error = new DOMException('\ud800', 'SyntaxError');
            check(error instanceof Error && error instanceof DOMException);
            check(error.message.charCodeAt(0) === 0xd800 && error.code === 12);
            check(DOMException.SYNTAX_ERR === 12 && DOMException.prototype.SYNTAX_ERR === 12);
            check(!Object.getOwnPropertyDescriptor(DOMException, 'SYNTAX_ERR').writable);
            check(legacyError.name === 'ReadError' && legacyError.message === 'failed');
            check(socketError instanceof DOMException && socketError instanceof WebSocketError);
            check(socketError.name === 'WebSocketError' && socketError.closeCode === 1001 && socketError.reason === 'away');
            const reads = [];
            const quota = new QuotaExceededError('full', {
                get requested() { reads.push('requested'); return 20; },
                get quota() { reads.push('quota'); return 10; }
            });
            check(reads.join(',') === 'quota,requested');
            check(quota instanceof DOMException && quota.quota === 10 && quota.requested === 20 && quota.code === 22);
            const rtc = new RTCError({errorDetail:'sdp-syntax-error', sdpLineNumber:7}, 'bad SDP');
            check(rtc instanceof DOMException && rtc.name === 'OperationError' && rtc.code === 0);
            check(rtc.sdpLineNumber === 7 && rtc.errorDetail === 'sdp-syntax-error' && rtc.sentAlert === null);
            const transport = new WebTransportError('bad transport', {source:'session', streamErrorCode:12.5});
            check(transport instanceof DOMException && transport.source === 'session' && transport.streamErrorCode === 12);
            try { new RTCError({errorDetail:'not-an-error'}); throw new Error('invalid enum accepted'); }
            catch (error) { check(error instanceof TypeError); }
            try { Object.getOwnPropertyDescriptor(DOMException.prototype,'name').get.call(Object.create(DOMException.prototype));
                throw new Error('forged exception accepted'); }
            catch (error) { check(error instanceof TypeError); }
        })()"#,
        );
    });
}

#[test]
fn rtc_descriptions_and_candidates_preserve_dictionary_order_and_utf16() {
    with_values(|scope| {
        eval(
            scope,
            r#"(() => {
            const reads = [];
            const description = new RTCSessionDescription({
                get type() { reads.push('type'); return 'offer'; },
                get sdp() { reads.push('sdp'); return '\ud800'; }
            });
            check(reads.join(',') === 'sdp,type');
            check(description.type === 'offer' && description.sdp.charCodeAt(0) === 0xd800);
            check(description.toJSON().sdp === '\ud800' && new RTCSessionDescription().type === null);
            const candidate = new RTCIceCandidate({
                candidate:'candidate:abc 1 UDP 2122260223 192.0.2.1 5000 typ host',
                sdpMid:'\ud800', sdpMLineIndex:1
            });
            check(candidate.foundation === 'abc' && candidate.component === 'rtp' && candidate.protocol === 'udp');
            check(candidate.address === '192.0.2.1' && candidate.port === 5000 && candidate.type === 'host');
            check(candidate.sdpMid.charCodeAt(0) === 0xd800 && candidate.toJSON().sdpMid === '\ud800');
            let converted = false;
            try { RTCIceCandidate.prototype.toJSON.call({toString(){converted=true;}}); throw new Error('forged candidate accepted'); }
            catch (error) { check(error instanceof TypeError && !converted); }
        })()"#,
        );
    });
}

#[test]
fn rtc_encoded_frame_copies_keep_bytes_metadata_and_native_brands() {
    with_values(|scope| {
        let metadata = eval(scope, "({rtpTimestamp:123, contributingSources:[1,2], payloadType:96, width:2, height:3, spatialIndex:0, temporalIndex:0})").to_object(scope).unwrap();
        let frame_type = Some(moli_v8_util::v8str(scope, "key"));
        let audio = rtc::encoded_frames::build_native_frame(
            scope,
            rtc::encoded_frames::FrameKind::Audio,
            &[1, 2, 3],
            metadata,
            None,
        )
        .unwrap();
        let video = rtc::encoded_frames::build_native_frame(
            scope,
            rtc::encoded_frames::FrameKind::Video,
            &[4, 5],
            metadata,
            frame_type,
        )
        .unwrap();
        publish(scope, "audioFrame", audio);
        publish(scope, "videoFrame", video);
        eval(
            scope,
            r#"(() => {
            const copy = new RTCEncodedAudioFrame(audioFrame);
            check(copy instanceof RTCEncodedAudioFrame && copy.getMetadata().rtpTimestamp === 123);
            check(new Uint8Array(copy.data).join() === '1,2,3' && copy.data !== audioFrame.data);
            const metadata = copy.getMetadata(); metadata.contributingSources.push(7);
            check(copy.getMetadata().contributingSources.join() === '1,2');
            new Uint8Array(copy.data)[0] = 9;
            check(new Uint8Array(audioFrame.data)[0] === 1);
            const video = new RTCEncodedVideoFrame(videoFrame);
            check(video.type === 'key' && video.getMetadata().rtpTimestamp === 123 && video.getMetadata().width === 2);
            video.data = new Uint8Array([8,9,10]).buffer;
            check(new Uint8Array(video.data).join() === '8,9,10' && videoFrame.data.byteLength === 2);
            try { new RTCEncodedAudioFrame(Object.create(RTCEncodedAudioFrame.prototype)); throw new Error('forged frame accepted'); }
            catch (error) { check(error instanceof TypeError); }
        })()"#,
        );
    });
}

#[test]
fn css_values_parse_and_mutate_without_a_document_or_style_host() {
    with_values(|scope| {
        let numeric = css_values::style_value_from_text(scope, "25px", true);
        let keyword = css_values::style_value_from_text(scope, "block", true);
        let fallback = css_values::style_value_from_text(scope, "rgb(1, 2, 3)", false);
        publish(scope, "parsedNumeric", numeric);
        publish(scope, "parsedKeyword", keyword);
        publish(scope, "parsedFallback", fallback);
        eval(
            scope,
            r#"(() => {
            check(parsedNumeric instanceof CSSUnitValue && parsedNumeric instanceof CSSStyleValue);
            check(parsedNumeric.value === 25 && parsedNumeric.unit === 'px' && String(parsedNumeric) === '25px');
            check(parsedKeyword instanceof CSSKeywordValue && parsedKeyword.value === 'block');
            check(parsedFallback instanceof CSSStyleValue && String(parsedFallback) === 'rgb(1, 2, 3)');
            const unit = new CSSUnitValue(2, 'PX'); unit.value = 7;
            check(unit.unit === 'px' && String(unit) === '7px');
            const keyword = new CSSKeywordValue('auto'); keyword.value = 'none';
            check(String(keyword) === 'none');
            try { new CSSKeywordValue(''); throw new Error('empty keyword accepted'); }
            catch (error) { check(error instanceof TypeError); }
            try { new CSSUnitValue(1, 'no-such-unit'); throw new Error('invalid unit accepted'); }
            catch (error) { check(error instanceof TypeError); }
            let converted = false;
            try { Object.getOwnPropertyDescriptor(CSSUnitValue.prototype,'value').set.call({}, {valueOf(){converted=true; return 1;}}); throw new Error('forged unit accepted'); }
            catch (error) { check(error instanceof TypeError); }
            check(!converted, 'receiver must be checked before conversion');
        })()"#,
        );
    });
}

#[test]
fn dom_string_list_keeps_utf16_indexed_descriptors_and_iteration() {
    with_values(|scope| {
        let values =
            v8::Local::<v8::Array>::try_from(eval(scope, "['a', '\\ud800', 'z']")).unwrap();
        let list = dom_string_list::new_dom_string_list(scope, values);
        publish(scope, "names", list);
        eval(
            scope,
            r#"(() => {
            check(names instanceof DOMStringList && names.length === 3);
            check(names.contains('\ud800') && !names.contains('\ufffd'));
            check(names.item(1).charCodeAt(0) === 0xd800 && names.item(9) === null && names[9] === undefined);
            check([...names].join('|') === 'a|\ud800|z');
            const desc = Object.getOwnPropertyDescriptor(names, '0');
            check(!desc.writable && desc.enumerable && desc.value === 'a');
            check(Reflect.set(names, '0', 'changed') === false && names[0] === 'a');
            check(Reflect.deleteProperty(names, '1') === false && names.length === 3);
            let converted = false;
            try { DOMStringList.prototype.contains.call(Object.create(DOMStringList.prototype), {toString(){converted=true; return 'a';}}); throw new Error('forged list accepted'); }
            catch (error) { check(error instanceof TypeError); }
            check(!converted, 'forged list converted an argument');
        })()"#,
        );
    });
}

#[test]
fn geolocation_values_keep_realm_prototypes_nulls_and_json_snapshots() {
    with_values(|scope| {
        let position = geolocation::build_position(
            scope,
            &geolocation::Coordinates {
                latitude: 10.,
                longitude: 20.,
                altitude: None,
                accuracy: 3.,
                altitude_accuracy: None,
                heading: Some(90.),
                speed: Some(2.),
            },
            1234.,
        );
        let error = geolocation::build_error(scope, 2, "unavailable").unwrap();
        publish(scope, "position", position);
        publish(scope, "positionError", error);
        eval(
            scope,
            r#"(() => {
            check(position instanceof GeolocationPosition && position.coords instanceof GeolocationCoordinates);
            check(position.coords === position.coords && position.timestamp === 1234 && position.coords.latitude === 10);
            check(position.coords.altitude === null && position.coords.altitudeAccuracy === null && position.coords.heading === 90);
            const json = position.toJSON();
            check(Object.getPrototypeOf(json) === Object.prototype && json.coords !== position.coords && json.coords.longitude === 20);
            check(Object.keys(json.coords).join() === 'accuracy,latitude,longitude,altitude,altitudeAccuracy,heading,speed');
            check(positionError instanceof GeolocationPositionError && positionError.code === 2 && positionError.message === 'unavailable');
            check(GeolocationPositionError.POSITION_UNAVAILABLE === 2 && positionError.TIMEOUT === 3);
            check(!Object.getOwnPropertyDescriptor(GeolocationPositionError, 'TIMEOUT').writable);
            try { Object.getOwnPropertyDescriptor(GeolocationCoordinates.prototype,'latitude').get.call({}); throw new Error('forged coordinates accepted'); }
            catch (error) { check(error instanceof TypeError); }
        })()"#,
        );
    });
}

#[test]
fn queuing_strategies_cache_size_per_realm_and_preserve_getter_effects() {
    with_values(|scope| {
        eval(
            scope,
            r#"(() => {
            const reads = [];
            const count = new CountQueuingStrategy({get highWaterMark(){ reads.push('hwm'); return 2; }});
            check(reads.join() === 'hwm' && count.highWaterMark === 2 && count.size() === 1);
            check(count.size === new CountQueuingStrategy({highWaterMark:9}).size);
            const bytes = new ByteLengthQueuingStrategy({highWaterMark:3});
            check(bytes.size === new ByteLengthQueuingStrategy({highWaterMark:4}).size && bytes.size !== count.size);
            check(count.size.length === 0 && bytes.size.length === 1);
            check(bytes.size({get byteLength(){ reads.push('bytes'); return 17; }}) === 17 && reads.join() === 'hwm,bytes');
            check(new CountQueuingStrategy({highWaterMark:NaN}).highWaterMark.toString() === 'NaN');
            try { new CountQueuingStrategy({}); throw new Error('missing hwm accepted'); }
            catch (error) { check(error instanceof TypeError); }
            try { Object.getOwnPropertyDescriptor(CountQueuingStrategy.prototype,'size').get.call({}); throw new Error('forged strategy accepted'); }
            catch (error) { check(error instanceof TypeError); }
        })()"#,
        );
    });
}

#[test]
fn shared_new_target_fallback_uses_the_target_realm_without_repeating_proxy_getters() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let source = v8::Context::new(scope, Default::default());
    let destination = v8::Context::new(scope, Default::default());
    let (target, prototype, size) = {
        let scope = &mut v8::ContextScope::new(scope, destination);
        install_values(scope);
        let target = eval(scope, "(function Target() {}).bind(null)")
            .to_object(scope)
            .unwrap();
        // IDL-002 keeps the prototype selected by V8, including its native
        // target-realm Object.prototype fallback.
        let prototype = eval(scope, "Object.prototype").to_object(scope).unwrap();
        let size = eval(scope, "new CountQueuingStrategy({highWaterMark:1}).size")
            .to_object(scope)
            .unwrap();
        (
            v8::Global::new(scope, target),
            v8::Global::new(scope, prototype),
            v8::Global::new(scope, size),
        )
    };
    let scope = &mut v8::ContextScope::new(scope, source);
    install_values(scope);
    let target = v8::Local::new(scope, &target);
    let prototype = v8::Local::new(scope, &prototype);
    let size = v8::Local::new(scope, &size);
    publish(scope, "foreignTarget", target);
    publish(scope, "foreignPrototype", prototype);
    publish(scope, "foreignSize", size);
    eval(
        scope,
        r#"(() => {
        let reads = 0;
        const target = new Proxy(foreignTarget, {get(object, key, receiver) { if(key === 'prototype') { reads++; return null; } return Reflect.get(object,key,receiver); }});
        const description = Reflect.construct(RTCSessionDescription, [{type:'answer',sdp:'ok'}], target);
        check(reads === 1, 'NewTarget.prototype was read more than once');
        const sdp = Object.getOwnPropertyDescriptor(RTCSessionDescription.prototype, 'sdp').get;
        check(Object.getPrototypeOf(description) === foreignPrototype && sdp.call(description) === 'ok');
        let explicitReads = 0;
        const explicitTarget = new Proxy(foreignTarget, {get(object, key, receiver) { if(key === 'prototype') { explicitReads++; return foreignPrototype; } return Reflect.get(object,key,receiver); }});
        const explicit = Reflect.construct(RTCSessionDescription, [{type:'offer',sdp:'explicit'}], explicitTarget);
        check(explicitReads === 1 && Object.getPrototypeOf(explicit) === foreignPrototype && sdp.call(explicit) === 'explicit', 'native constructor replaced an explicit Object.prototype');
        check(new CountQueuingStrategy({highWaterMark:1}).size !== foreignSize, 'size cache leaked across realms');
    })()"#,
    );
}

#[test]
fn native_error_and_transport_clone_builders_use_destination_intrinsics_after_global_replacement() {
    ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let source = v8::Context::new(scope, Default::default());
    let destination = v8::Context::new(scope, Default::default());
    let payload = {
        let scope = &mut v8::ContextScope::new(scope, source);
        install_values(scope);
        let object = eval(
            scope,
            "new WebTransportError('\\ud800', {source:'session',streamErrorCode:42})",
        )
        .to_object(scope)
        .unwrap();
        dom_exception::web_transport_error_clone_payload_from_object(scope, object).unwrap()
    };
    let scope = &mut v8::ContextScope::new(scope, destination);
    install_values(scope);
    eval(
        scope,
        "globalThis.nativeException = DOMException; globalThis.nativeTransport = WebTransportError; globalThis.DOMException = globalThis.WebTransportError = function(){throw new Error('public constructor called');};",
    );
    let object =
        dom_exception::build_web_transport_error_from_clone_payload(scope, &payload).unwrap();
    let native = dom_exception::new_dom_exception_value(scope, "native", "SyntaxError")
        .to_object(scope)
        .unwrap();
    publish(scope, "transportClone", object);
    publish(scope, "nativeError", native);
    eval(
        scope,
        r#"check(transportClone instanceof nativeTransport && transportClone instanceof nativeException);
        check(transportClone.message.charCodeAt(0) === 0xd800 && transportClone.source === 'session' && transportClone.streamErrorCode === 42);
        check(nativeError instanceof nativeException && nativeError.name === 'SyntaxError' && nativeError.code === 12);"#,
    );
    // Shared builders must not instantiate an author-provided constructor.
    assert!(dom_exception::dom_exception_clone_fields(scope, native).is_some());
    let not_an_error = v8_string(scope, "not a native error").unwrap();
    assert!(dom_exception::websocket_error_close_info(scope, not_an_error.into()).is_none());
    assert!(eval(scope, "Object.hasOwn(transportClone, 'stack')").is_true());
}

#[test]
fn rtc_capabilities_are_fresh_without_a_renderer() {
    with_values(|scope| {
        assert!(eval(scope, r#"(() => {
            const caps = RTCRtpReceiver.getCapabilities('audio');
            check(Array.isArray(caps.codecs) && caps.codecs.length > 0 && Array.isArray(caps.headerExtensions));
            caps.codecs.length = 0;
            check(RTCRtpReceiver.getCapabilities('audio').codecs.length > 0);
            check(RTCRtpReceiver.getCapabilities('unknown') === null);
            check(RTCRtpReceiver.getCapabilities('video').codecs.length > 0);
            return true;
        })()"#).is_true());
    });
}
