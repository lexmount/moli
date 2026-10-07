use super::*;

#[test]
fn webidl_required_nullable_dictionary_rejects_undefined_member() {
    let vm = new_storage_test_vm("https://webidl-dictionary-nullable-required.test/");
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_runtime.context as *const _;

    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let absent = NullableRequiredDictionaryAbsentProbe {}
                .bind(scope)
                .expect("absent dictionary probe declaration should bind");
            let absent_error = crate::webidl::parse_dictionary_object::<
                NullableRequiredDictionaryProbe,
            >(scope, absent)
            .expect_err("absent required nullable member should be rejected");
            assert_eq!(
                absent_error.to_string(),
                "NullableRequiredDictionaryProbe: value is required"
            );

            let explicit_undefined = NullableRequiredDictionaryValueProbe {
                value: v8::undefined(scope).into(),
            }
            .bind(scope)
            .expect("undefined value probe should bind");
            let undefined_error = crate::webidl::parse_dictionary_object::<
                NullableRequiredDictionaryProbe,
            >(scope, explicit_undefined)
            .expect_err("undefined required nullable member should be rejected");
            assert_eq!(
                undefined_error.to_string(),
                "NullableRequiredDictionaryProbe: value is required"
            );

            let explicit_null = NullableRequiredDictionaryValueProbe {
                value: v8::null(scope).into(),
            }
            .bind(scope)
            .expect("null value probe should bind");
            let parsed_null = crate::webidl::parse_dictionary_object::<
                NullableRequiredDictionaryProbe,
            >(scope, explicit_null)
            .expect("null required nullable member should parse as null");
            assert_eq!(parsed_null.value, None);

            let value = v8::String::new(scope, "ok").expect("test value should allocate");
            let explicit_string = NullableRequiredDictionaryValueProbe {
                value: value.into(),
            }
            .bind(scope)
            .expect("string value probe should bind");
            let parsed_string = crate::webidl::parse_dictionary_object::<
                NullableRequiredDictionaryProbe,
            >(scope, explicit_string)
            .expect("present required nullable member should parse");
            assert_eq!(parsed_string.value, Some("ok".to_owned()));
            Ok(())
        })
        .expect("nullable required dictionary probe should parse");
}

#[test]
fn fetch_url_constructors_use_child_frame_base_url() {
    let mut vm = new_storage_html_test_vm("https://fetch-current.test/entry/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const currentFrame = document.createElement('iframe');
  const relevantFrame = document.createElement('iframe');
  const parent = document.body || document.documentElement || document;
  parent.appendChild(currentFrame);
  parent.appendChild(relevantFrame);

  currentFrame.contentDocument.open();
  currentFrame.contentDocument.write('<!doctype html><base href="https://fetch-current.test/current/success/">');
  currentFrame.contentDocument.close();

  relevantFrame.contentDocument.open();
  relevantFrame.contentDocument.write('<!doctype html><base href="https://fetch-current.test/relevant/">');
  relevantFrame.contentDocument.close();

  const current = currentFrame.contentWindow;
  const relevant = relevantFrame.contentWindow;
  return [
    new current.Request('url').url,
    current.Response.redirect.call(relevant.Response, 'url').headers.get('Location')
  ].join('|');
})()
"#,
        )
        .expect("fetch URL constructors should resolve against the current frame base URL");

    assert_eq!(
        result,
        "https://fetch-current.test/current/success/url|https://fetch-current.test/current/success/url"
    );
}

#[test]
fn fetch_child_frame_constructor_lengths_match_top_level() {
    let mut vm = new_storage_test_vm("https://fetch-child-length.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  const win = frame.contentWindow;
  return [
    Request.length,
    win.Request.length,
    Response.redirect.length,
    win.Response.redirect.length
  ].join('|');
})()
"#,
        )
        .expect("child frame fetch constructors should expose matching lengths");

    assert_eq!(result, "1|1|1|1");
}

#[test]
fn fetch_url_constructors_use_detached_nested_iframe_base_url() {
    let mut vm = new_storage_test_vm("https://fetch-current.test/entry/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const currentBlob = URL.createObjectURL(new Blob([
    '<!doctype html><base href="https://fetch-current.test/current/success/">'
  ], { type: 'text/html' }));
  const relevantBlob = URL.createObjectURL(new Blob([
    '<!doctype html><base href="https://fetch-current.test/relevant/">'
  ], { type: 'text/html' }));
  const frame = document.createElement('iframe');
  (document.body || document.documentElement || document).appendChild(frame);
  frame.contentDocument.open();
  frame.contentDocument.write(`<!doctype html>
    <iframe id="c" src="${currentBlob}"></iframe>
    <iframe id="r" src="${relevantBlob}"></iframe>
    <script>
      window.createRequest = (...args) => {
        const current = document.querySelector('#c').contentWindow;
        return new current.Request(...args);
      };
      window.createRedirectResponse = (...args) => {
        const current = document.querySelector('#c').contentWindow;
        const relevant = document.querySelector('#r').contentWindow;
        return current.Response.redirect.call(relevant.Response, ...args);
      };
    </scr` + `ipt>`);
  frame.contentDocument.close();
  return [
    frame.contentWindow.createRequest('url').url,
    frame.contentWindow.createRedirectResponse('url').headers.get('Location'),
    frame.contentDocument.querySelector('#c').contentWindow ===
      frame.contentDocument.querySelector('#c').contentWindow
  ].join('|');
})()
"#,
        )
        .expect("detached nested iframe fetch URL constructors should resolve");

    assert_eq!(
        result,
        "https://fetch-current.test/current/success/url|https://fetch-current.test/current/success/url|true"
    );
}

#[test]
fn css_style_declaration_exposes_webkit_vendor_probe_properties() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const s = document.createElement("html").style;
                let engine;
                window.opera && Object.prototype.toString.call(opera) === "[object Opera]"
                    ? engine = "presto"
                    : "MozAppearance" in s
                        ? engine = "gecko"
                        : "WebkitAppearance" in s
                            ? engine = "webkit"
                            : typeof navigator.cpuClass === "string" && (engine = "trident");
                const cssPrefix = {
                    trident: "-ms-",
                    gecko: "-moz-",
                    webkit: "-webkit-",
                    presto: "-o-",
                }[engine];
                const jsPrefix = {
                    trident: "ms",
                    gecko: "Moz",
                    webkit: "Webkit",
                    presto: "O",
                }[engine];
                const node = document.createElement("div");
                return JSON.stringify({
                    engine,
                    cssPrefix,
                    jsPrefix,
                    perspectiveType: typeof node.style[jsPrefix + "Perspective"],
                    transformType: typeof node.style[jsPrefix + "Transform"],
                    transitionType: typeof node.style[jsPrefix + "Transition"],
                    transitionEnd: jsPrefix.toLowerCase() + "TransitionEnd",
                });
            })()
            "#,
        )
        .expect("webkit style probe should evaluate");

    assert_eq!(
        result,
        r#"{"engine":"webkit","cssPrefix":"-webkit-","jsPrefix":"Webkit","perspectiveType":"string","transformType":"string","transitionType":"string","transitionEnd":"webkitTransitionEnd"}"#
    );
}

#[test]
fn window_named_items_do_not_shadow_builtin_window_aliases() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                if (!document.documentElement) {
                    const html = document.createElement("html");
                    document.appendChild(html);
                }
                if (!document.body) {
                    const body = document.createElement("body");
                    document.documentElement.appendChild(body);
                }
                for (const id of ["window", "self", "top", "parent", "frames"]) {
                    const node = document.createElement("div");
                    node.id = id;
                    document.body.appendChild(node);
                }
                return [
                    window === globalThis,
                    self === globalThis,
                    top === globalThis,
                    parent === globalThis,
                    frames === globalThis,
                    typeof window.addEventListener,
                    typeof window.removeEventListener,
                    typeof window.dispatchEvent
                ].join("|");
            })()
            "#,
        )
        .expect("window builtins should not be shadowed by named items");

    assert_eq!(
        result,
        "true|true|true|true|true|function|function|function"
    );
}

#[test]
fn text_codec_constructors_preserve_declared_private_metadata() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const methodDescriptor = (prototype, name) => {
                    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                    return [
                        name,
                        typeof descriptor?.value,
                        descriptor?.value?.name,
                        descriptor?.value?.length,
                        descriptor?.enumerable,
                        descriptor?.writable,
                        descriptor?.configurable,
                    ].join(":");
                };
                const accessorDescriptor = (prototype, name) => {
                    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                    return [
                        name,
                        typeof descriptor?.get,
                        descriptor?.get?.name,
                        typeof descriptor?.set,
                        descriptor?.enumerable,
                        descriptor?.configurable,
                    ].join(":");
                };
                const encoder = new TextEncoder();
                const decoder = new TextDecoder("utf-16le", {
                    fatal: true,
                    ignoreBOM: true,
                });
                return JSON.stringify({
                    encoderEncoding: encoder.encoding,
                    encoderOwnNames: Object.getOwnPropertyNames(encoder),
                    encoderAccessors: [
                        accessorDescriptor(TextEncoder.prototype, "encoding"),
                    ],
                    encoderMethods: [
                        methodDescriptor(TextEncoder.prototype, "encode"),
                        methodDescriptor(TextEncoder.prototype, "encodeInto"),
                    ],
                    encoderInstance: encoder instanceof TextEncoder,
                    decoderEncoding: decoder.encoding,
                    decoderFatal: decoder.fatal,
                    decoderIgnoreBOM: decoder.ignoreBOM,
                    decoderOwnNames: Object.getOwnPropertyNames(decoder),
                    decoderAccessors: [
                        accessorDescriptor(TextDecoder.prototype, "encoding"),
                        accessorDescriptor(TextDecoder.prototype, "fatal"),
                        accessorDescriptor(TextDecoder.prototype, "ignoreBOM"),
                    ],
                    decoderMethods: [
                        methodDescriptor(TextDecoder.prototype, "decode"),
                    ],
                    decoderInstance: decoder instanceof TextDecoder,
                });
            })()
            "#,
        )
        .expect("Text codec constructor metadata should evaluate");

    assert_eq!(
        result,
        "{\"encoderEncoding\":\"utf-8\",\"encoderOwnNames\":[],\"encoderAccessors\":[\"encoding:function:get encoding:undefined:true:true\"],\"encoderMethods\":[\"encode:function:encode:0:true:true:true\",\"encodeInto:function:encodeInto:2:true:true:true\"],\"encoderInstance\":true,\"decoderEncoding\":\"utf-16le\",\"decoderFatal\":true,\"decoderIgnoreBOM\":true,\"decoderOwnNames\":[],\"decoderAccessors\":[\"encoding:function:get encoding:undefined:true:true\",\"fatal:function:get fatal:undefined:true:true\",\"ignoreBOM:function:get ignoreBOM:undefined:true:true\"],\"decoderMethods\":[\"decode:function:decode:0:true:true:true\"],\"decoderInstance\":true}"
    );
}

#[test]
fn buffer_source_extended_attributes_reject_resizable_backing_stores() {
    let mut vm = new_storage_test_vm("https://buffer-source-attributes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const encoder = new TextEncoder();
  const fixedShared = new SharedArrayBuffer(16);
  const growableShared = new SharedArrayBuffer(16, { maxByteLength: 32 });
  const fixedResult = encoder.encodeInto('abc', new Uint8Array(fixedShared));
  return JSON.stringify({
    fixedResponse: probe(() => new Response(new Uint8Array(4)).constructor.name),
    resizableResponse: probe(() => new Response(new Uint8Array(
      new ArrayBuffer(16, { maxByteLength: 32 })
    ))),
    fixedShared: `${fixedResult.read}:${fixedResult.written}`,
    growableShared: probe(() => encoder.encodeInto('abc', new Uint8Array(growableShared)))
  });
})()
"#,
        )
        .expect("BufferSource backing store attribute probe should evaluate");

    assert_eq!(
        result,
        r#"{"fixedResponse":"Response","resizableResponse":"throw:TypeError","fixedShared":"3:3","growableShared":"throw:TypeError"}"#
    );
}

#[test]
fn text_decoder_uses_encoding_rs_labels_and_streaming_decode() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const latin1 = new TextDecoder("latin1");
                const utf8 = new TextDecoder("utf-8");
                const first = utf8.decode(new Uint8Array([0xE2, 0x82]), { stream: true });
                const second = utf8.decode(new Uint8Array([0xAC]));
                const probe = (callback) => {
                    try {
                        return callback();
                    } catch (error) {
                        return `throw:${error.constructor.name}`;
                    }
                };
                const buffer = new Uint8Array([0x41, 0x42, 0x43]).buffer;
                const view = new Uint8Array(buffer, 1, 1);
                const dataView = new DataView(new Uint8Array([0x44, 0x45]).buffer, 1, 1);
                let invalidLabelIsRangeError = false;
                try {
                    new TextDecoder("not-a-real-encoding");
                } catch (error) {
                    invalidLabelIsRangeError = error instanceof RangeError;
                }
                return JSON.stringify({
                    encoding: latin1.encoding,
                    decoded: latin1.decode(new Uint8Array([0x80])),
                    first,
                    second,
                    empty: utf8.decode(),
                    explicitUndefined: utf8.decode(undefined),
                    arrayBuffer: utf8.decode(buffer),
                    view: utf8.decode(view),
                    dataView: utf8.decode(dataView),
                    fatalMalformed: (() => {
                        try {
                            new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array([0xFF]));
                            return "no-throw";
                        } catch (error) {
                            return `${error.constructor.name}:${error.message}`;
                        }
                    })(),
                    nullInput: probe(() => utf8.decode(null)),
                    objectInput: probe(() => utf8.decode({})),
                    invalidLabelIsRangeError,
                });
            })()
            "#,
        )
        .expect("TextDecoder probe should evaluate");

    assert_eq!(
        result,
        "{\"encoding\":\"windows-1252\",\"decoded\":\"\u{20AC}\",\"first\":\"\",\"second\":\"\u{20AC}\",\"empty\":\"\",\"explicitUndefined\":\"\",\"arrayBuffer\":\"ABC\",\"view\":\"B\",\"dataView\":\"E\",\"fatalMalformed\":\"TypeError:The encoded data was not valid.\",\"nullInput\":\"throw:TypeError\",\"objectInput\":\"throw:TypeError\",\"invalidLabelIsRangeError\":true}"
    );
}

#[test]
fn text_codec_internal_slots_are_not_page_visible_or_forgeable() {
    let mut vm = new_storage_test_vm("https://text-codec-private-brand.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const encoder = new TextEncoder();
                const decoder = new TextDecoder("utf-8");
                const first = decoder.decode(new Uint8Array([0xE2, 0x82]), { stream: true });
                TextEncoder.prototype.__lmTextEncoderBrand = true;
                TextEncoder.prototype.__lmTextEncoderEncoding = "utf-8";
                TextDecoder.prototype.__lmTextDecoderBrand = true;
                TextDecoder.prototype.__lmTextDecoderEncoding = "utf-8";
                TextDecoder.prototype.__lmTextDecoderFatal = false;
                TextDecoder.prototype.__lmTextDecoderIgnoreBOM = false;
                const fakeEncoder = Object.assign(Object.create(TextEncoder.prototype), {
                    __lmTextEncoderBrand: true,
                    __lmTextEncoderEncoding: "utf-8",
                });
                const fakeDecoder = Object.assign(Object.create(TextDecoder.prototype), {
                    __lmTextDecoderBrand: true,
                    __lmTextDecoderId: decoder.__lmTextDecoderId ?? 1,
                    __lmTextDecoderEncoding: "utf-8",
                    __lmTextDecoderFatal: false,
                    __lmTextDecoderIgnoreBOM: false,
                });
                const probe = (callback) => {
                    try {
                        return callback();
                    } catch (error) {
                        return `throw:${error.constructor.name}`;
                    }
                };
                const getter = (prototype, name) =>
                    Object.getOwnPropertyDescriptor(prototype, name).get;
                return JSON.stringify({
                    encoderHasVisibleSlots: Object.hasOwn(encoder, "__lmTextEncoderBrand") || Object.hasOwn(encoder, "__lmTextEncoderEncoding"),
                    decoderHasVisibleSlots: Object.hasOwn(decoder, "__lmTextDecoderBrand") || Object.hasOwn(decoder, "__lmTextDecoderId"),
                    encoderOwnNames: Object.getOwnPropertyNames(encoder),
                    decoderOwnNames: Object.getOwnPropertyNames(decoder),
                    first,
                    fakeEncoderAccessors: [
                        probe(() => getter(TextEncoder.prototype, "encoding").call(fakeEncoder)),
                    ],
                    fakeEncoderMethods: [
                        probe(() => TextEncoder.prototype.encode.call(fakeEncoder, "x")),
                        probe(() => TextEncoder.prototype.encodeInto.call(fakeEncoder, "x", new Uint8Array(1))),
                    ],
                    fakeDecoderAccessors: [
                        probe(() => getter(TextDecoder.prototype, "encoding").call(fakeDecoder)),
                        probe(() => getter(TextDecoder.prototype, "fatal").call(fakeDecoder)),
                        probe(() => getter(TextDecoder.prototype, "ignoreBOM").call(fakeDecoder)),
                    ],
                    forgedDecode: probe(() => TextDecoder.prototype.decode.call(fakeDecoder, new Uint8Array([0xAC]))),
                    second: decoder.decode(new Uint8Array([0xAC])),
                });
            })()
            "#,
        )
        .expect("Text codec private brand probe should evaluate");

    assert_eq!(
        result,
        "{\"encoderHasVisibleSlots\":false,\"decoderHasVisibleSlots\":false,\"encoderOwnNames\":[],\"decoderOwnNames\":[],\"first\":\"\",\"fakeEncoderAccessors\":[\"throw:TypeError\"],\"fakeEncoderMethods\":[\"throw:TypeError\",\"throw:TypeError\"],\"fakeDecoderAccessors\":[\"throw:TypeError\",\"throw:TypeError\",\"throw:TypeError\"],\"forgedDecode\":\"throw:TypeError\",\"second\":\"\u{20AC}\"}"
    );
}

#[test]
fn text_stream_constructors_preserve_declared_metadata() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const encoder = new TextEncoderStream();
                const decoder = new TextDecoderStream("utf-16le", {
                    fatal: true,
                    ignoreBOM: true,
                });
                const descriptor = (object, name) => {
                    const desc = Object.getOwnPropertyDescriptor(object, name);
                    return `${desc && desc.value}:${desc && desc.enumerable}`;
                };
                return JSON.stringify({
                    encoderEncoding: encoder.encoding,
                    encoderEncodingOwn: descriptor(encoder, "encoding"),
                    decoderEncoding: decoder.encoding,
                    decoderFatal: decoder.fatal,
                    decoderIgnoreBOM: decoder.ignoreBOM,
                    decoderEncodingOwn: descriptor(decoder, "encoding"),
                    decoderFatalOwn: descriptor(decoder, "fatal"),
                    decoderIgnoreBOMOwn: descriptor(decoder, "ignoreBOM"),
                    encoderReadableOwn: encoder.hasOwnProperty("readable"),
                    decoderWritableOwn: decoder.hasOwnProperty("writable"),
                });
            })()
            "#,
        )
        .expect("Text stream constructor metadata should evaluate");

    assert_eq!(
        result,
        "{\"encoderEncoding\":\"utf-8\",\"encoderEncodingOwn\":\"utf-8:false\",\"decoderEncoding\":\"utf-16le\",\"decoderFatal\":true,\"decoderIgnoreBOM\":true,\"decoderEncodingOwn\":\"utf-16le:false\",\"decoderFatalOwn\":\"true:false\",\"decoderIgnoreBOMOwn\":\"true:false\",\"encoderReadableOwn\":false,\"decoderWritableOwn\":false}"
    );
}

#[test]
fn text_decoder_stream_uses_encoding_rs_labels_and_streaming_decode() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__textDecoderStreamEvents = [];
                const stream = new TextDecoderStream("gbk");
                const writer = stream.writable.getWriter();
                const reader = stream.readable.getReader();
                reader.read().then(({ value, done }) => {
                    globalThis.__textDecoderStreamEvents.push(`${value}:${done}`);
                });
                writer.write(new Uint8Array([0xCC]));
                writer.write(new Uint8Array([0xAB, 0xC6, 0xBD, 0xD1, 0xF3]));
                writer.close();
                return JSON.stringify({
                    encoding: stream.encoding,
                    fatal: stream.fatal,
                    ignoreBOM: stream.ignoreBOM,
                    events: globalThis.__textDecoderStreamEvents,
                });
            })()
            "#,
        )
        .expect("TextDecoderStream setup should evaluate");
    assert_eq!(
        initial,
        "{\"encoding\":\"gbk\",\"fatal\":false,\"ignoreBOM\":false,\"events\":[]}"
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__textDecoderStreamEvents)")
        .expect("TextDecoderStream read should settle");
    assert_eq!(settled, r#"["太平洋:false"]"#);
}

#[test]
fn text_decoder_stream_fatal_errors_reject_write_and_read() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__textDecoderStreamFatalEvents = [];
                const stream = new TextDecoderStream("utf-8", { fatal: true });
                const writer = stream.writable.getWriter();
                const reader = stream.readable.getReader();
                reader.read().then(
                    () => globalThis.__textDecoderStreamFatalEvents.push("read:resolved"),
                    error => globalThis.__textDecoderStreamFatalEvents.push(`read:${error.name}:${error.message}`)
                );
                writer.write(new Uint8Array([0xFF])).then(
                    () => globalThis.__textDecoderStreamFatalEvents.push("write:resolved"),
                    error => globalThis.__textDecoderStreamFatalEvents.push(`write:${error.name}:${error.message}`)
                );
                return JSON.stringify(globalThis.__textDecoderStreamFatalEvents);
            })()
            "#,
        )
        .expect("fatal TextDecoderStream setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__textDecoderStreamFatalEvents.sort())")
        .expect("fatal TextDecoderStream promises should settle");
    assert_eq!(
        settled,
        r#"["read:TypeError:The encoded data was not valid.","write:TypeError:The encoded data was not valid."]"#
    );
}

#[test]
fn text_decoder_stream_rejects_non_buffer_source_chunks() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__textDecoderStreamTypeEvents = [];
                const stream = new TextDecoderStream("utf-8");
                const writer = stream.writable.getWriter();
                const reader = stream.readable.getReader();
                reader.read().then(
                    () => globalThis.__textDecoderStreamTypeEvents.push("read:resolved"),
                    error => globalThis.__textDecoderStreamTypeEvents.push(`read:${error.constructor.name}`)
                );
                writer.write("not bytes").then(
                    () => globalThis.__textDecoderStreamTypeEvents.push("write:resolved"),
                    error => globalThis.__textDecoderStreamTypeEvents.push(`write:${error.constructor.name}`)
                );
                return JSON.stringify(globalThis.__textDecoderStreamTypeEvents);
            })()
            "#,
        )
        .expect("TextDecoderStream type setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__textDecoderStreamTypeEvents.sort())")
        .expect("TextDecoderStream type promises should settle");
    assert_eq!(settled, r#"["read:TypeError","write:TypeError"]"#);
}

#[test]
fn text_transform_stream_write_waits_for_readable_demand() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__textTransformBackpressureEvents = [];
                globalThis.__textTransformEncoder = new TextEncoderStream();
                globalThis.__textTransformEncoderWriter =
                    globalThis.__textTransformEncoder.writable.getWriter();
                globalThis.__textTransformEncoderReader =
                    globalThis.__textTransformEncoder.readable.getReader();
                globalThis.__textTransformEncoderWriter.write("A").then(() => {
                    globalThis.__textTransformBackpressureEvents.push("encoder:write");
                });
                globalThis.__textTransformDecoder = new TextDecoderStream();
                globalThis.__textTransformDecoderWriter =
                    globalThis.__textTransformDecoder.writable.getWriter();
                globalThis.__textTransformDecoderReader =
                    globalThis.__textTransformDecoder.readable.getReader();
                globalThis.__textTransformDecoderWriter.write(new Uint8Array([66])).then(() => {
                    globalThis.__textTransformBackpressureEvents.push("decoder:write");
                });
                return JSON.stringify(globalThis.__textTransformBackpressureEvents);
            })()
            "#,
        )
        .expect("Text transform stream backpressure setup should evaluate");
    assert_eq!(initial, "[]");

    let before_read = vm
        .eval("JSON.stringify(globalThis.__textTransformBackpressureEvents)")
        .expect("Text transform write promises should wait for readable demand");
    assert_eq!(before_read, "[]");

    let read_started = vm
        .eval(
            r#"
            (() => {
                globalThis.__textTransformEncoderReader.read().then(({ value, done }) => {
                    globalThis.__textTransformBackpressureEvents.push(
                        `encoder:read:${value[0]}:${done}`
                    );
                });
                globalThis.__textTransformDecoderReader.read().then(({ value, done }) => {
                    globalThis.__textTransformBackpressureEvents.push(
                        `decoder:read:${value}:${done}`
                    );
                });
                return JSON.stringify(globalThis.__textTransformBackpressureEvents);
            })()
            "#,
        )
        .expect("Text transform readable demand should evaluate");
    assert_eq!(read_started, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__textTransformBackpressureEvents.sort())")
        .expect("Text transform writes should settle after readable demand");
    assert_eq!(
        settled,
        r#"["decoder:read:B:false","decoder:write","encoder:read:65:false","encoder:write"]"#
    );
}

#[test]
fn writable_stream_close_method_closes_transform_readable_side() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__writableStreamCloseEvents = [];
                const stream = new TextEncoderStream();
                const reader = stream.readable.getReader();
                reader.read().then(({ value, done }) => {
                    globalThis.__writableStreamCloseEvents.push(`${String(value)}:${done}`);
                });
                stream.writable.close().then(() => {
                    globalThis.__writableStreamCloseEvents.push("close:resolved");
                });
                return JSON.stringify(globalThis.__writableStreamCloseEvents);
            })()
            "#,
        )
        .expect("WritableStream close setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__writableStreamCloseEvents.sort())")
        .expect("WritableStream close promises should settle");
    assert_eq!(settled, r#"["close:resolved","undefined:true"]"#);
}

#[test]
fn transform_stream_controller_error_rejects_readable_and_writable_sides() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformControllerErrorEvents = [];
                const stream = new TransformStream({
                    transform(_chunk, controller) {
                        controller.error(new Error("boom"));
                    }
                });
                const writer = stream.writable.getWriter();
                const reader = stream.readable.getReader();
                reader.read().then(
                    () => globalThis.__transformControllerErrorEvents.push("read:resolved"),
                    error => globalThis.__transformControllerErrorEvents.push(`read:${error.message}`)
                );
                writer.write("first").then(
                    () => globalThis.__transformControllerErrorEvents.push("first:resolved"),
                    error => globalThis.__transformControllerErrorEvents.push(`first:${error.message}`)
                );
                writer.write("second").then(
                    () => globalThis.__transformControllerErrorEvents.push("second:resolved"),
                    error => globalThis.__transformControllerErrorEvents.push(`second:${error.message}`)
                );
                writer.ready.then(
                    () => globalThis.__transformControllerErrorEvents.push("ready:resolved"),
                    error => globalThis.__transformControllerErrorEvents.push(`ready:${error.message}`)
                );
                writer.closed.then(
                    () => globalThis.__transformControllerErrorEvents.push("closed:resolved"),
                    error => globalThis.__transformControllerErrorEvents.push(`closed:${error.message}`)
                );
                return JSON.stringify(globalThis.__transformControllerErrorEvents);
            })()
            "#,
        )
        .expect("TransformStream controller.error setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformControllerErrorEvents.sort())")
        .expect("TransformStream controller.error promises should settle");
    assert_eq!(
        settled,
        r#"["closed:boom","first:resolved","read:boom","ready:boom","second:boom"]"#
    );
}

#[test]
fn transform_stream_thrown_transform_error_settles_every_owned_promise() {
    let mut vm = new_storage_test_vm("https://example.com/");

    vm.eval(
        r#"
        (() => {
            const error = new Error("transform-boom");
            const state = globalThis.__transformThrownErrorState = {
                write: "pending",
                read: "pending",
                readerClosed: "pending",
                writerClosed: "pending"
            };
            const stream = new TransformStream({
                transform() { throw error; }
            });
            const reader = stream.readable.getReader();
            const writer = stream.writable.getWriter();
            writer.write("a").then(
                () => { state.write = "resolved"; },
                reason => { state.write = reason === error ? "same-error" : "wrong-error"; }
            );
            reader.read().then(
                () => { state.read = "resolved"; },
                reason => { state.read = reason === error ? "same-error" : "wrong-error"; }
            );
            reader.closed.then(
                () => { state.readerClosed = "resolved"; },
                reason => { state.readerClosed = reason === error ? "same-error" : "wrong-error"; }
            );
            writer.closed.then(
                () => { state.writerClosed = "resolved"; },
                reason => { state.writerClosed = reason === error ? "same-error" : "wrong-error"; }
            );
        })()
        "#,
    )
    .expect("thrown TransformStream error setup should evaluate");

    for _ in 0..16 {
        let state = vm
            .eval("JSON.stringify(globalThis.__transformThrownErrorState)")
            .expect("thrown TransformStream error should drain microtasks");
        if !state.contains("pending") {
            break;
        }
    }
    let state = vm
        .eval("JSON.stringify(globalThis.__transformThrownErrorState)")
        .expect("thrown TransformStream error state should evaluate");
    assert_eq!(
        state,
        r#"{"write":"same-error","read":"same-error","readerClosed":"same-error","writerClosed":"same-error"}"#
    );
}

#[test]
fn stream_constructors_preserve_declared_state_defaults() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__streamConstructorEvents = [];
                const internalNames = object => Object.getOwnPropertyNames(object)
                    .filter(name => name.startsWith("__moliReadableStream") ||
                                    name.startsWith("__moliWritableStream") ||
                                    name.startsWith("__moliTransformStream") ||
                                    name.startsWith("__moliStreamController"))
                    .sort();
                let readableController;
                const readable = new ReadableStream({
                    start(controller) {
                        readableController = controller;
                        controller.enqueue("seed");
                    }
                }, { highWaterMark: 4 });
                let writableController;
                const writable = new WritableStream({
                    start(controller) {
                        writableController = controller;
                        globalThis.__streamConstructorEvents.push(`writable-start:${!!controller}`);
                    }
                });
                const transform = new TransformStream();
                const spoofReadable = new ReadableStream({
                    start(controller) {
                        controller.enqueue("real");
                    }
                });
                const spoofReader = spoofReadable.getReader();
                const spoofReaderInternalNamesBefore = internalNames(spoofReader);
                spoofReadable.__moliReadableStreamQueue = ["fake"];
                spoofReadable.__moliReadableStreamClosed = true;
                spoofReader.__moliReadableStreamReaderStream = new ReadableStream();
                spoofReader.read().then(({ value, done }) => {
                    globalThis.__streamConstructorEvents.push(`spoof-read:${value}:${done}`);
                });
                const spoofWritable = new WritableStream({
                    write(chunk) {
                        globalThis.__streamConstructorEvents.push(`spoof-write:${chunk}`);
                    }
                });
                const spoofWriter = spoofWritable.getWriter();
                const spoofWriterInternalNamesBefore = internalNames(spoofWriter);
                spoofWritable.__moliWritableStreamSink = {
                    write() {
                        globalThis.__streamConstructorEvents.push("fake-stream-write");
                    }
                };
                spoofWriter.__moliWritableStreamWriterStream = new WritableStream({
                    write() {
                        globalThis.__streamConstructorEvents.push("fake-writer-write");
                    }
                });
                spoofWriter.write("real").then(() => {
                    globalThis.__streamConstructorEvents.push("spoof-write-done");
                });
                const transformReadableBefore = transform.readable;
                const transformWritableBefore = transform.writable;
                const transformInternalNamesBefore = internalNames(transform);
                transform.__moliTransformStreamReadable = new ReadableStream();
                transform.__moliTransformStreamWritable = new WritableStream();
                const descriptor = (object, name) => {
                    const desc = Object.getOwnPropertyDescriptor(object, name);
                    return `${desc && desc.value}:${desc && desc.enumerable}`;
                };
                const snapshot = {
                    readableLocked: readable.locked,
                    readableLockedOwn: descriptor(readable, "locked"),
                    writableLocked: writable.locked,
                    writableLockedOwn: descriptor(writable, "locked"),
                    transformReadable: transform.readable instanceof ReadableStream,
                    transformWritable: transform.writable instanceof WritableStream,
                    transformReadableLocked: transform.readable.locked,
                    transformWritableLocked: transform.writable.locked,
                    transformReadableOwn: transform.hasOwnProperty("readable"),
                    transformWritableOwn: transform.hasOwnProperty("writable"),
                    readableInternalNames: internalNames(readable),
                    writableInternalNames: internalNames(writable),
                    transformInternalNames: transformInternalNamesBefore,
                    readableControllerInternalNames: internalNames(readableController),
                    writableControllerInternalNames: internalNames(writableController),
                    spoofReaderInternalNames: spoofReaderInternalNamesBefore,
                    spoofWriterInternalNames: spoofWriterInternalNamesBefore,
                    transformReadableSpoofIgnored: transform.readable === transformReadableBefore,
                    transformWritableSpoofIgnored: transform.writable === transformWritableBefore,
                    events: globalThis.__streamConstructorEvents
                };
                readable.getReader().read().then(({ value, done }) => {
                    globalThis.__streamConstructorEvents.push(`read:${value}:${done}`);
                });
                return JSON.stringify(snapshot);
            })()
            "#,
        )
        .expect("stream constructor defaults should evaluate");
    assert_eq!(
        initial,
        "{\"readableLocked\":false,\"readableLockedOwn\":\"undefined:undefined\",\"writableLocked\":false,\"writableLockedOwn\":\"undefined:undefined\",\"transformReadable\":true,\"transformWritable\":true,\"transformReadableLocked\":false,\"transformWritableLocked\":false,\"transformReadableOwn\":false,\"transformWritableOwn\":false,\"readableInternalNames\":[],\"writableInternalNames\":[],\"transformInternalNames\":[],\"readableControllerInternalNames\":[],\"writableControllerInternalNames\":[],\"spoofReaderInternalNames\":[],\"spoofWriterInternalNames\":[],\"transformReadableSpoofIgnored\":true,\"transformWritableSpoofIgnored\":true,\"events\":[\"writable-start:true\"]}"
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__streamConstructorEvents.sort())")
        .expect("stream constructor queued read should settle");
    assert_eq!(
        settled,
        r#"["read:seed:false","spoof-read:real:false","spoof-write-done","spoof-write:real","writable-start:true"]"#
    );
}

#[test]
fn readable_stream_pending_read_state_ignores_public_spoofing() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__readableStreamPendingReadEvents = [];
                const internalNames = object => Object.getOwnPropertyNames(object)
                    .filter(name => name.startsWith("__moliReadableStreamPending"))
                    .sort();
                Object.defineProperty(Object.prototype, "__moliReadableStreamPendingResolve", {
                    configurable: true,
                    value() { globalThis.__readableStreamPendingReadEvents.push("prototype-resolve"); }
                });
                Object.defineProperty(Object.prototype, "__moliReadableStreamPendingReject", {
                    configurable: true,
                    value() { globalThis.__readableStreamPendingReadEvents.push("prototype-reject"); }
                });
                let controller;
                const stream = new ReadableStream({
                    start(value) {
                        controller = value;
                    }
                });
                const reader = stream.getReader();
                const streamNames = internalNames(stream);
                const readerNames = internalNames(reader);
                stream.__moliReadableStreamPendingResolve = () => {
                    globalThis.__readableStreamPendingReadEvents.push("stream-resolve");
                };
                reader.__moliReadableStreamPendingResolve = () => {
                    globalThis.__readableStreamPendingReadEvents.push("reader-resolve");
                };
                reader.read().then(
                    ({ value, done }) => {
                        globalThis.__readableStreamPendingReadEvents.push(`read:${value}:${done}`);
                    },
                    error => {
                        globalThis.__readableStreamPendingReadEvents.push(`read-error:${error && error.name}`);
                    }
                );
                controller.enqueue("real");
                return JSON.stringify({
                    streamNames,
                    readerNames,
                    events: globalThis.__readableStreamPendingReadEvents
                });
            })()
            "#,
        )
        .expect("ReadableStream pending read spoofing setup should evaluate");
    assert_eq!(
        initial,
        r#"{"streamNames":[],"readerNames":[],"events":[]}"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__readableStreamPendingReadEvents)")
        .expect("ReadableStream pending read promise should settle");
    assert_eq!(settled, r#"["read:real:false"]"#);
}

#[test]
fn readable_stream_constructor_converts_strategy_before_reading_underlying_source_members() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const events = [];
                const sourceError = new Error("source");
                sourceError.name = "source";
                const strategyError = new Error("strategy");
                strategyError.name = "strategy";
                const underlyingSource = {
                    get start() {
                        events.push("source-start");
                        throw sourceError;
                    }
                };
                const strategy = {
                    highWaterMark: 0,
                    get size() {
                        events.push("strategy-size");
                        throw strategyError;
                    }
                };

                try {
                    new ReadableStream(underlyingSource, strategy);
                    events.push("constructed");
                } catch (error) {
                    events.push(`caught:${error === strategyError}:${error.name}`);
                }
                return events.join("|");
            })()
            "#,
        )
        .expect("ReadableStream constructor conversion order should evaluate");

    assert_eq!(result, "strategy-size|caught:true:strategy");
}

#[test]
fn readable_stream_pull_runs_for_pending_read_with_zero_high_water_mark() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__zeroHwmPullEvents = [];
                let pulls = 0;
                const stream = new ReadableStream({
                    pull(controller) {
                        pulls += 1;
                        controller.enqueue("chunk-" + pulls);
                    }
                }, { highWaterMark: 0 });
                const reader = stream.getReader();
                reader.read().then(({ value, done }) => {
                    globalThis.__zeroHwmPullEvents.push(`${value}:${done}:${pulls}`);
                });
                return JSON.stringify(globalThis.__zeroHwmPullEvents);
            })()
            "#,
        )
        .expect("ReadableStream zero HWM pending read setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__zeroHwmPullEvents)")
        .expect("ReadableStream zero HWM pending read should settle");
    assert_eq!(settled, r#"["chunk-1:false:1"]"#);
}

#[test]
fn readable_stream_start_reaction_state_ignores_public_spoofing() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__startReactionEvents = [];
                const internalNames = object => Object.getOwnPropertyNames(object)
                    .filter(name => name.startsWith("__moliReadableStreamStart"))
                    .sort();
                Object.defineProperty(Object.prototype, "__moliReadableStreamStartRejectedStream", {
                    configurable: true,
                    writable: true,
                    value: null
                });
                Object.defineProperty(Object.prototype, "__moliReadableStreamStartPullAfterStart", {
                    configurable: true,
                    writable: true,
                    value: false
                });

                let resolveStart;
                const fulfilledGate = new Promise(resolve => { resolveStart = resolve; });
                const fulfilledStream = new ReadableStream({
                    start() {
                        return fulfilledGate;
                    },
                    pull(controller) {
                        globalThis.__startReactionEvents.push("pull");
                        controller.enqueue("after-start");
                    }
                }, { highWaterMark: 0 });
                const fulfilledReader = fulfilledStream.getReader();
                const fulfilledNames = internalNames(fulfilledStream);
                fulfilledStream.__moliReadableStreamStartRejectedStream = null;
                fulfilledStream.__moliReadableStreamStartPullAfterStart = false;
                fulfilledReader.read().then(
                    ({ value, done }) => {
                        globalThis.__startReactionEvents.push(`fulfilled-read:${value}:${done}`);
                    },
                    error => {
                        globalThis.__startReactionEvents.push(`fulfilled-error:${error && error.name}`);
                    }
                );

                let rejectStart;
                const reason = new Error("start-boom");
                reason.name = "StartBoom";
                const rejectedGate = new Promise((_, reject) => { rejectStart = reject; });
                const rejectedStream = new ReadableStream({
                    start() {
                        return rejectedGate;
                    }
                });
                const rejectedReader = rejectedStream.getReader();
                const rejectedNames = internalNames(rejectedStream);
                rejectedStream.__moliReadableStreamStartRejectedStream = fulfilledStream;
                rejectedReader.read().then(
                    () => globalThis.__startReactionEvents.push("rejected-read:resolved"),
                    error => {
                        globalThis.__startReactionEvents.push(`rejected-read:${error === reason}:${error.name}`);
                    }
                );

                globalThis.__resolveReadableStreamStart = () => resolveStart();
                globalThis.__rejectReadableStreamStart = () => rejectStart(reason);
                return JSON.stringify({
                    fulfilledNames,
                    rejectedNames,
                    events: globalThis.__startReactionEvents
                });
            })()
            "#,
        )
        .expect("ReadableStream start reaction spoofing setup should evaluate");
    assert_eq!(
        initial,
        r#"{"fulfilledNames":[],"rejectedNames":[],"events":[]}"#
    );

    vm.eval("globalThis.__resolveReadableStreamStart()")
        .expect("ReadableStream start fulfillment should be scheduled");
    let fulfilled = vm
        .eval("JSON.stringify(globalThis.__startReactionEvents)")
        .expect("ReadableStream start fulfillment should settle pending read");
    assert_eq!(fulfilled, r#"["pull","fulfilled-read:after-start:false"]"#);

    vm.eval("globalThis.__rejectReadableStreamStart()")
        .expect("ReadableStream start rejection should be scheduled");
    let rejected = vm
        .eval("JSON.stringify(globalThis.__startReactionEvents)")
        .expect("ReadableStream start rejection should settle pending read");
    assert_eq!(
        rejected,
        r#"["pull","fulfilled-read:after-start:false","rejected-read:true:StartBoom"]"#
    );
}

#[test]
fn readable_stream_strategy_size_controls_desired_size_and_pull() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__strategySizeEvents = [];
                let controller;
                let pulls = 0;
                const stream = new ReadableStream({
                    start(c) {
                        controller = c;
                    },
                    pull(c) {
                        pulls += 1;
                        globalThis.__strategySizeEvents.push(`pull:${pulls}:${c.desiredSize}`);
                    }
                }, {
                    highWaterMark: 5,
                    size(chunk) {
                        globalThis.__strategySizeEvents.push(`size:${chunk.label}:${chunk.units}`);
                        return chunk.units;
                    }
                });
                globalThis.__strategySizeStream = stream;
                controller.enqueue({ label: "a", units: 2 });
                const afterA = controller.desiredSize;
                controller.enqueue({ label: "b", units: 3 });
                const afterB = controller.desiredSize;
                controller.enqueue({ label: "c", units: 4 });
                const afterC = controller.desiredSize;
                const reader = stream.getReader();
                reader.read().then(({ value, done }) => {
                    globalThis.__strategySizeEvents.push(`read:${value.label}:${done}:${controller.desiredSize}`);
                });
                return JSON.stringify({ afterA, afterB, afterC, events: globalThis.__strategySizeEvents });
            })()
            "#,
        )
        .expect("ReadableStream strategy size setup should evaluate");
    assert_eq!(
        initial,
        r#"{"afterA":3,"afterB":0,"afterC":-4,"events":["size:a:2","size:b:3","size:c:4"]}"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__strategySizeEvents)")
        .expect("ReadableStream strategy size read should settle");
    assert_eq!(
        settled,
        r#"["size:a:2","size:b:3","size:c:4","read:a:false:-2"]"#
    );
}

#[test]
fn transform_stream_readable_strategy_size_controls_enqueue_desired_size() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformReadableStrategyEvents = [];
                const stream = new TransformStream({
                    transform(chunk, controller) {
                        globalThis.__transformReadableStrategyEvents.push(
                            `before:${chunk.label}:${controller.desiredSize}`
                        );
                        controller.enqueue(chunk);
                        globalThis.__transformReadableStrategyEvents.push(
                            `after:${chunk.label}:${controller.desiredSize}`
                        );
                    }
                }, undefined, {
                    highWaterMark: 5,
                    size(chunk) {
                        globalThis.__transformReadableStrategyEvents.push(`size:${chunk.label}:${chunk.units}`);
                        return chunk.units;
                    }
                });
                globalThis.__transformReadableStrategyStream = stream;
                const writer = stream.writable.getWriter();
                writer.write({ label: "a", units: 2 });
                writer.write({ label: "b", units: 4 });
                return JSON.stringify(globalThis.__transformReadableStrategyEvents);
            })()
            "#,
        )
        .expect("TransformStream readable strategy size setup should evaluate");
    assert_eq!(initial, "[]");

    let after_reads = vm
        .eval(
            r#"
            (() => {
                const reader = globalThis.__transformReadableStrategyStream.readable.getReader();
                reader.read().then(({ value, done }) => {
                    globalThis.__transformReadableStrategyEvents.push(`read:${value.label}:${done}`);
                });
                reader.read().then(({ value, done }) => {
                    globalThis.__transformReadableStrategyEvents.push(`read:${value.label}:${done}`);
                });
                return JSON.stringify(globalThis.__transformReadableStrategyEvents);
            })()
            "#,
        )
        .expect("TransformStream readable strategy queued reads should evaluate");
    assert_eq!(
        after_reads,
        r#"["before:a:5","size:a:2","after:a:3","before:b:3","size:b:4","after:b:-1"]"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformReadableStrategyEvents)")
        .expect("TransformStream readable strategy queued reads should settle");
    assert_eq!(
        settled,
        r#"["before:a:5","size:a:2","after:a:3","before:b:3","size:b:4","after:b:-1","read:a:false","read:b:false"]"#
    );
}

#[test]
fn transform_stream_readable_high_water_mark_gates_pending_writes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformReadableHwmEvents = [];
                globalThis.__transformReadableHwmStream = new TransformStream({
                    transform(chunk, controller) {
                        globalThis.__transformReadableHwmEvents.push(`transform:${chunk}`);
                        controller.enqueue(chunk);
                    }
                }, undefined, { highWaterMark: 2 });
                const writer = globalThis.__transformReadableHwmStream.writable.getWriter();
                writer.write(0);
                writer.write(1);
                writer.write(2).then(() => {
                    globalThis.__transformReadableHwmEvents.push("write-2:resolved");
                });
                return JSON.stringify(globalThis.__transformReadableHwmEvents);
            })()
            "#,
        )
        .expect("TransformStream readable HWM setup should evaluate");
    assert_eq!(initial, "[]");

    let after_read = vm
        .eval(
            r#"
            (() => {
                const reader = globalThis.__transformReadableHwmStream.readable.getReader();
                reader.read().then(({ value, done }) => {
                    globalThis.__transformReadableHwmEvents.push(`read:${value}:${done}`);
                });
                return JSON.stringify(
                    globalThis.__transformReadableHwmEvents.filter(event => event.startsWith("transform:"))
                );
            })()
            "#,
        )
        .expect("TransformStream readable HWM read should evaluate");
    assert_eq!(after_read, r#"["transform:0","transform:1"]"#);

    let settled = vm
        .eval(
            r#"
            JSON.stringify(
                globalThis.__transformReadableHwmEvents
                    .filter(event => event.startsWith("read:") || event.startsWith("write-"))
                    .sort()
            )
            "#,
        )
        .expect("TransformStream readable HWM promises should settle");
    assert_eq!(settled, r#"["read:0:false","write-2:resolved"]"#);
}

#[test]
fn transform_stream_default_readable_high_water_mark_waits_for_read_demand() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformDefaultReadableHwmEvents = [];
                globalThis.__transformDefaultReadableHwmStream = new TransformStream({
                    transform(chunk, controller) {
                        globalThis.__transformDefaultReadableHwmEvents.push(
                            `before:${controller.desiredSize}`
                        );
                        controller.enqueue(chunk);
                        globalThis.__transformDefaultReadableHwmEvents.push(
                            `after-first:${controller.desiredSize}`
                        );
                        controller.enqueue(`${chunk}-extra`);
                        globalThis.__transformDefaultReadableHwmEvents.push(
                            `after-second:${controller.desiredSize}`
                        );
                    }
                });
                const writer = globalThis.__transformDefaultReadableHwmStream.writable.getWriter();
                writer.write("x").then(() => {
                    globalThis.__transformDefaultReadableHwmEvents.push("write:resolved");
                });
                return JSON.stringify(globalThis.__transformDefaultReadableHwmEvents);
            })()
            "#,
        )
        .expect("TransformStream default readable HWM setup should evaluate");
    assert_eq!(initial, "[]");

    let after_read = vm
        .eval(
            r#"
            (() => {
                const reader = globalThis.__transformDefaultReadableHwmStream.readable.getReader();
                reader.read().then(({ value, done }) => {
                    globalThis.__transformDefaultReadableHwmEvents.push(`read:${value}:${done}`);
                });
                return JSON.stringify(
                    globalThis.__transformDefaultReadableHwmEvents.filter(event => event !== "write:resolved")
                );
            })()
            "#,
        )
        .expect("TransformStream default readable HWM read should evaluate");
    assert_eq!(after_read, "[]");

    let transformed = vm
        .eval(
            r#"
            JSON.stringify(
                globalThis.__transformDefaultReadableHwmEvents
                    .filter(event => event !== "write:resolved" && !event.startsWith("read:"))
            )
            "#,
        )
        .expect("TransformStream default readable HWM transform should run after read demand");
    assert_eq!(
        transformed,
        r#"["before:0","after-first:0","after-second:-1"]"#
    );

    let settled = vm
        .eval(
            r#"
            JSON.stringify(
                globalThis.__transformDefaultReadableHwmEvents
                    .filter(event => event.startsWith("read:") || event.startsWith("write:"))
                    .sort()
            )
            "#,
        )
        .expect("TransformStream default readable HWM promises should settle");
    assert_eq!(settled, r#"["read:x:false","write:resolved"]"#);
}

#[test]
fn transform_stream_writable_strategy_size_runs_before_transform() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const events = [];
                const stream = new TransformStream({
                    transform(chunk, controller) {
                        events.push(`transform:${chunk.label}:${controller.desiredSize}`);
                        controller.enqueue(chunk);
                    }
                }, {
                    highWaterMark: 7,
                    size(chunk) {
                        events.push(`writable-size:${chunk.label}:${chunk.writeUnits}`);
                        return chunk.writeUnits;
                    }
                }, {
                    highWaterMark: 5,
                    size(chunk) {
                        events.push(`readable-size:${chunk.label}:${chunk.readUnits}`);
                        return chunk.readUnits;
                    }
                });
                const writer = stream.writable.getWriter();
                globalThis.__transformWritableStrategyEvents = events;
                const before = writer.desiredSize;
                writer.write({ label: "a", writeUnits: 2, readUnits: 3 });
                const after = writer.desiredSize;
                return JSON.stringify({ before, after, events });
            })()
            "#,
        )
        .expect("TransformStream writable strategy size should evaluate");

    assert_eq!(
        result,
        r#"{"before":7,"after":5,"events":["writable-size:a:2"]}"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformWritableStrategyEvents)")
        .expect("TransformStream strategy algorithms should settle");
    assert_eq!(
        settled,
        r#"["writable-size:a:2","transform:a:5","readable-size:a:3"]"#
    );
}

#[test]
fn transform_stream_bad_readable_strategy_size_rejects_identity_write() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformBadReadableSizeEvents = [];
                const stream = new TransformStream(undefined, undefined, {
                    highWaterMark: 1,
                    size() {
                        return NaN;
                    }
                });
                const writer = stream.writable.getWriter();
                writer.write("x").then(
                    () => globalThis.__transformBadReadableSizeEvents.push("write:resolved"),
                    error => globalThis.__transformBadReadableSizeEvents.push(`write:${error.name}`)
                );
                return JSON.stringify(globalThis.__transformBadReadableSizeEvents);
            })()
            "#,
        )
        .expect("TransformStream bad readable size identity setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformBadReadableSizeEvents)")
        .expect("TransformStream bad readable size identity write should settle");
    assert_eq!(settled, r#"["write:RangeError"]"#);
}

#[test]
fn transform_stream_caught_enqueue_strategy_error_errors_stream_but_resolves_write() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformCaughtEnqueueEvents = [];
                const stream = new TransformStream({
                    transform(chunk, controller) {
                        try {
                            controller.enqueue(chunk);
                        } catch (error) {
                            globalThis.__transformCaughtEnqueueEvents.push(`enqueue:${error.name}`);
                        }
                    }
                }, undefined, {
                    highWaterMark: 1,
                    size() {
                        return -1;
                    }
                });
                const writer = stream.writable.getWriter();
                writer.write("x").then(
                    () => {
                        globalThis.__transformCaughtEnqueueEvents.push("write:resolved");
                        writer.ready.catch(error => {
                            globalThis.__transformCaughtEnqueueEvents.push(`ready:${error.name}`);
                        });
                        writer.closed.catch(error => {
                            globalThis.__transformCaughtEnqueueEvents.push(`closed:${error.name}`);
                        });
                        stream.readable.getReader().closed.catch(error => {
                            globalThis.__transformCaughtEnqueueEvents.push(`readable:${error.name}`);
                        });
                    },
                    error => globalThis.__transformCaughtEnqueueEvents.push(`write:${error.name}`)
                );
                return JSON.stringify(globalThis.__transformCaughtEnqueueEvents);
            })()
            "#,
        )
        .expect("TransformStream caught enqueue strategy error setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformCaughtEnqueueEvents.sort())")
        .expect("TransformStream caught enqueue strategy error promises should settle");
    assert_eq!(
        settled,
        r#"["closed:RangeError","enqueue:RangeError","readable:RangeError","ready:RangeError","write:resolved"]"#
    );
}
