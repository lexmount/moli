use super::*;
use crate::util::v8str;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "Object", data_properties)]
struct NullableRequiredDictionaryValueProbe<'scope> {
    value: v8::Local<'scope, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", allow_empty)]
struct NullableRequiredDictionaryAbsentProbe {}

#[test]
fn webidl_required_nullable_dictionary_rejects_undefined_member() {
    let vm = new_storage_test_vm("https://webidl-dictionary-nullable-required.test/");
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;

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
    let mut vm = new_storage_test_vm("https://fetch-current.test/entry/page.html");

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

#[test]
fn transform_stream_flush_waits_for_pending_transform_promise() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformFlushWaitEvents = [];
                let resolveTransform;
                globalThis.__resolveTransformFlushWait = () => resolveTransform();
                const stream = new TransformStream({
                    transform() {
                        globalThis.__transformFlushWaitEvents.push("transform");
                        return new Promise(resolve => {
                            resolveTransform = resolve;
                        });
                    },
                    flush() {
                        globalThis.__transformFlushWaitEvents.push("flush");
                        return new Promise(() => {});
                    }
                }, undefined, { highWaterMark: 1 });
                const writer = stream.writable.getWriter();
                writer.write("a").then(() => {
                    globalThis.__transformFlushWaitEvents.push("write:resolved");
                });
                writer.close().then(() => {
                    globalThis.__transformFlushWaitEvents.push("close:resolved");
                });
                stream.readable.getReader().closed.then(() => {
                    globalThis.__transformFlushWaitEvents.push("readable:closed");
                });
                return JSON.stringify(globalThis.__transformFlushWaitEvents);
            })()
            "#,
        )
        .expect("TransformStream pending transform flush setup should evaluate");
    assert_eq!(initial, "[]");

    let before_resolve = vm
        .eval("JSON.stringify(globalThis.__transformFlushWaitEvents)")
        .expect("TransformStream pending transform flush should not run early");
    assert_eq!(before_resolve, r#"["transform"]"#);

    let after_resolve = vm
        .eval(
            r#"
            (() => {
                globalThis.__resolveTransformFlushWait();
                return JSON.stringify(globalThis.__transformFlushWaitEvents);
            })()
            "#,
        )
        .expect("TransformStream pending transform resolver should evaluate");
    assert_eq!(after_resolve, r#"["transform"]"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformFlushWaitEvents)")
        .expect("TransformStream flush should run after transform settles");
    assert_eq!(settled, r#"["transform","flush","write:resolved"]"#);
}

#[test]
fn transform_stream_flush_can_enqueue_before_readable_closes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformFlushEnqueueEvents = [];
                let savedController;
                const stream = new TransformStream({
                    start(controller) {
                        savedController = controller;
                    },
                    transform() {},
                    flush() {
                        savedController.enqueue("x");
                        savedController.enqueue("y");
                        globalThis.__transformFlushEnqueueEvents.push("flush");
                    }
                });
                const reader = stream.readable.getReader();
                const writer = stream.writable.getWriter();
                writer.write("a");
                writer.close().then(() => {
                    globalThis.__transformFlushEnqueueEvents.push("close:resolved");
                });
                reader.read().then(({ value, done }) => {
                    globalThis.__transformFlushEnqueueEvents.push(`read1:${value}:${done}`);
                    return reader.read();
                }).then(({ value, done }) => {
                    globalThis.__transformFlushEnqueueEvents.push(`read2:${value}:${done}`);
                });
                return JSON.stringify(globalThis.__transformFlushEnqueueEvents);
            })()
            "#,
        )
        .expect("TransformStream flush enqueue setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformFlushEnqueueEvents)")
        .expect("TransformStream flush enqueue promises should settle");
    assert_eq!(
        settled,
        r#"["flush","read1:x:false","read2:y:false","close:resolved"]"#
    );
}

#[test]
fn transform_stream_flush_error_rejects_writer_close() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformFlushErrorEvents = [];
                const error = new Error("flush-boom");
                const stream = new TransformStream({
                    flush(controller) {
                        controller.error(error);
                    }
                });
                stream.writable.getWriter().close().then(
                    () => globalThis.__transformFlushErrorEvents.push("close:resolved"),
                    error => globalThis.__transformFlushErrorEvents.push(`close:${error.message}`)
                );
                return JSON.stringify(globalThis.__transformFlushErrorEvents);
            })()
            "#,
        )
        .expect("TransformStream flush error setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformFlushErrorEvents)")
        .expect("TransformStream flush error close should settle");
    assert_eq!(settled, r#"["close:flush-boom"]"#);
}

#[test]
fn transform_stream_start_promise_gates_transform_and_flush() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformStartGateEvents = [];
                let resolveStart;
                globalThis.__resolveTransformStartGate = () => {
                    globalThis.__transformStartGateEvents.push("start:resolve");
                    resolveStart();
                };
                const stream = new TransformStream({
                    start() {
                        globalThis.__transformStartGateEvents.push("start");
                        return new Promise(resolve => {
                            resolveStart = resolve;
                        });
                    },
                    transform(chunk, controller) {
                        globalThis.__transformStartGateEvents.push(`transform:${chunk}`);
                        controller.enqueue(chunk);
                    },
                    flush() {
                        globalThis.__transformStartGateEvents.push("flush");
                    }
                }, undefined, { highWaterMark: 1 });
                const writer = stream.writable.getWriter();
                writer.write("a").then(() => {
                    globalThis.__transformStartGateEvents.push("write:resolved");
                });
                writer.close().then(() => {
                    globalThis.__transformStartGateEvents.push("close:resolved");
                });
                return JSON.stringify(globalThis.__transformStartGateEvents);
            })()
            "#,
        )
        .expect("TransformStream start gating setup should evaluate");
    assert_eq!(initial, r#"["start"]"#);

    let before_start = vm
        .eval("JSON.stringify(globalThis.__transformStartGateEvents)")
        .expect("TransformStream start promise should gate transform");
    assert_eq!(before_start, r#"["start"]"#);

    let after_resolve = vm
        .eval(
            r#"
            (() => {
                globalThis.__resolveTransformStartGate();
                return JSON.stringify(globalThis.__transformStartGateEvents);
            })()
            "#,
        )
        .expect("TransformStream start resolver should evaluate");
    assert_eq!(after_resolve, r#"["start","start:resolve"]"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformStartGateEvents)")
        .expect("TransformStream write and flush should run after start resolves");
    assert_eq!(
        settled,
        r#"["start","start:resolve","transform:a","flush","write:resolved","close:resolved"]"#
    );
}

#[test]
fn transform_stream_defined_readable_or_writable_type_throws_range_error() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const events = [];
                try {
                    new TransformStream({ readableType: "bytes" });
                    events.push("readable:constructed");
                } catch (error) {
                    events.push(`readable:${error.name}`);
                }
                try {
                    new TransformStream({ writableType: "bytes" });
                    events.push("writable:constructed");
                } catch (error) {
                    events.push(`writable:${error.name}`);
                }
                return JSON.stringify(events);
            })()
            "#,
        )
        .expect("TransformStream defined readable/writable type validation should evaluate");
    assert_eq!(result, r#"["readable:RangeError","writable:RangeError"]"#);
}

#[test]
fn transform_stream_start_rejection_errors_pending_write_without_transform() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformStartRejectWriteEvents = [];
                const error = new Error("start-boom");
                const stream = new TransformStream({
                    start() {
                        return Promise.reject(error);
                    },
                    transform() {
                        globalThis.__transformStartRejectWriteEvents.push("transform");
                    }
                }, undefined, { highWaterMark: 1 });
                const writer = stream.writable.getWriter();
                writer.write("a").then(
                    () => globalThis.__transformStartRejectWriteEvents.push("write:resolved"),
                    error => globalThis.__transformStartRejectWriteEvents.push(`write:${error.message}`)
                );
                stream.readable.getReader().read().catch(error => {
                    globalThis.__transformStartRejectWriteEvents.push(`read:${error.message}`);
                });
                return JSON.stringify(globalThis.__transformStartRejectWriteEvents);
            })()
            "#,
        )
        .expect("TransformStream start rejection write setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformStartRejectWriteEvents.sort())")
        .expect("TransformStream start rejection should error queued write");
    assert_eq!(settled, r#"["read:start-boom","write:start-boom"]"#);
}

#[test]
fn transform_stream_start_rejection_rejects_pending_close_without_flush() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformStartRejectCloseEvents = [];
                const error = new Error("start-close-boom");
                const stream = new TransformStream({
                    start() {
                        return Promise.reject(error);
                    },
                    flush() {
                        globalThis.__transformStartRejectCloseEvents.push("flush");
                    }
                });
                const writer = stream.writable.getWriter();
                writer.close().then(
                    () => globalThis.__transformStartRejectCloseEvents.push("close:resolved"),
                    error => globalThis.__transformStartRejectCloseEvents.push(`close:${error.message}`)
                );
                stream.readable.getReader().closed.catch(error => {
                    globalThis.__transformStartRejectCloseEvents.push(`readable:${error.message}`);
                });
                return JSON.stringify(globalThis.__transformStartRejectCloseEvents);
            })()
            "#,
        )
        .expect("TransformStream start rejection close setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformStartRejectCloseEvents.sort())")
        .expect("TransformStream start rejection should reject queued close");
    assert_eq!(
        settled,
        r#"["close:start-close-boom","readable:start-close-boom"]"#
    );
}

#[test]
fn transform_stream_start_controller_error_beats_later_rejection() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformStartFirstErrorEvents = [];
                const controllerError = new Error("controller-start-error");
                const ignoredError = new Error("ignored-start-error");
                const stream = new TransformStream({
                    start(controller) {
                        return Promise.resolve().then(() => {
                            controller.error(controllerError);
                            throw ignoredError;
                        });
                    },
                    transform() {
                        globalThis.__transformStartFirstErrorEvents.push("transform");
                    },
                    flush() {
                        globalThis.__transformStartFirstErrorEvents.push("flush");
                    }
                }, undefined, { highWaterMark: 1 });
                const writer = stream.writable.getWriter();
                writer.write("a").then(
                    () => globalThis.__transformStartFirstErrorEvents.push("write:resolved"),
                    error => globalThis.__transformStartFirstErrorEvents.push(`write:${error.message}`)
                );
                writer.close().then(
                    () => globalThis.__transformStartFirstErrorEvents.push("close:resolved"),
                    error => globalThis.__transformStartFirstErrorEvents.push(`close:${error.message}`)
                );
                stream.readable.getReader().read().catch(error => {
                    globalThis.__transformStartFirstErrorEvents.push(`read:${error.message}`);
                });
                return JSON.stringify(globalThis.__transformStartFirstErrorEvents);
            })()
            "#,
        )
        .expect("TransformStream start first-error setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformStartFirstErrorEvents.sort())")
        .expect("TransformStream controller.error should beat later start rejection");
    assert_eq!(
        settled,
        r#"["close:controller-start-error","read:controller-start-error","write:controller-start-error"]"#
    );
}

#[test]
fn writable_stream_writer_closed_stays_pending_until_transform_closes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__writerClosedPendingEvents = [];
                const stream = new TransformStream();
                globalThis.__writerClosedPendingWriter = stream.writable.getWriter();
                globalThis.__writerClosedPendingClosed =
                    globalThis.__writerClosedPendingWriter.closed;
                globalThis.__writerClosedPendingClosed.then(() => {
                    globalThis.__writerClosedPendingEvents.push("closed");
                });
                return JSON.stringify({
                    same: globalThis.__writerClosedPendingWriter.closed ===
                        globalThis.__writerClosedPendingClosed,
                    events: globalThis.__writerClosedPendingEvents
                });
            })()
            "#,
        )
        .expect("Writable writer closed pending setup should evaluate");
    assert_eq!(initial, r#"{"same":true,"events":[]}"#);

    let before_close = vm
        .eval("JSON.stringify(globalThis.__writerClosedPendingEvents)")
        .expect("Writable writer closed should stay pending before close");
    assert_eq!(before_close, "[]");

    let close_started = vm
        .eval(
            r#"
            (() => {
                globalThis.__writerClosedPendingWriter.close().then(() => {
                    globalThis.__writerClosedPendingEvents.push("close");
                });
                return JSON.stringify({
                    same: globalThis.__writerClosedPendingWriter.closed ===
                        globalThis.__writerClosedPendingClosed,
                    events: globalThis.__writerClosedPendingEvents
                });
            })()
            "#,
        )
        .expect("Writable writer close should evaluate");
    assert_eq!(close_started, r#"{"same":true,"events":[]}"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__writerClosedPendingEvents.sort())")
        .expect("Writable writer closed should resolve after close");
    assert_eq!(settled, r#"["close","closed"]"#);
}

#[test]
fn writable_stream_writer_closed_rejects_after_transform_controller_error() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__writerClosedErrorEvents = [];
                let savedController;
                const stream = new TransformStream({
                    start(controller) {
                        savedController = controller;
                    }
                });
                const writer = stream.writable.getWriter();
                const closed = writer.closed;
                globalThis.__writerClosedErrorWriter = writer;
                globalThis.__writerClosedErrorClosed = closed;
                closed.then(
                    () => globalThis.__writerClosedErrorEvents.push("closed:resolved"),
                    error => globalThis.__writerClosedErrorEvents.push(`closed:${error.message}`)
                );
                globalThis.__errorWriterClosedTransform = () => {
                    savedController.error(new Error("writer-closed-boom"));
                };
                return JSON.stringify(globalThis.__writerClosedErrorEvents);
            })()
            "#,
        )
        .expect("Writable writer closed error setup should evaluate");
    assert_eq!(initial, "[]");

    let before_error = vm
        .eval("JSON.stringify(globalThis.__writerClosedErrorEvents)")
        .expect("Writable writer closed should stay pending before controller error");
    assert_eq!(before_error, "[]");

    let error_started = vm
        .eval(
            r#"
            (() => {
                globalThis.__errorWriterClosedTransform();
                return JSON.stringify({
                    same: globalThis.__writerClosedErrorWriter.closed ===
                        globalThis.__writerClosedErrorClosed,
                    events: globalThis.__writerClosedErrorEvents
                });
            })()
            "#,
        )
        .expect("Writable writer closed controller error should evaluate");
    assert_eq!(error_started, r#"{"same":true,"events":[]}"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__writerClosedErrorEvents)")
        .expect("Writable writer closed should reject after controller error");
    assert_eq!(settled, r#"["closed:writer-closed-boom"]"#);
}

#[test]
fn transform_stream_controller_terminate_closes_readable_and_errors_writable() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformTerminateEvents = [];
                let controller;
                const stream = new TransformStream({
                    start(c) {
                        controller = c;
                    }
                });
                const writer = stream.writable.getWriter();
                const reader = stream.readable.getReader();
                writer.closed.then(
                    () => globalThis.__transformTerminateEvents.push("writer:resolved"),
                    error => globalThis.__transformTerminateEvents.push(
                        `writer:${error.name}:${error instanceof TypeError}:${error.message}`
                    )
                );
                reader.closed.then(
                    () => globalThis.__transformTerminateEvents.push("reader:closed"),
                    error => globalThis.__transformTerminateEvents.push(`reader:${error.name}`)
                );
                controller.terminate();
                controller.terminate();
                let enqueueResult = "no-throw";
                try {
                    controller.enqueue("after");
                } catch (error) {
                    enqueueResult = `${error.name}:${error instanceof TypeError}`;
                }
                return JSON.stringify({
                    enqueueResult,
                    events: globalThis.__transformTerminateEvents
                });
            })()
            "#,
        )
        .expect("TransformStream controller terminate setup should evaluate");
    assert_eq!(initial, r#"{"enqueueResult":"TypeError:true","events":[]}"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformTerminateEvents.sort())")
        .expect("TransformStream controller terminate promises should settle");
    assert_eq!(
        settled,
        r#"["reader:closed","writer:TypeError:true:The transform stream has been terminated"]"#
    );
}

#[test]
fn transform_stream_controller_terminate_after_readable_cancel_errors_writable_with_type_error() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformTerminateCancelEvents = [];
                let controller;
                const stream = new TransformStream({
                    start(c) {
                        controller = c;
                    }
                });
                const cancelReason = { name: "cancelReason" };
                stream.readable.cancel(cancelReason).then(
                    () => globalThis.__transformTerminateCancelEvents.push("cancel:resolved"),
                    error => globalThis.__transformTerminateCancelEvents.push(
                        `cancel:${error && error.name}`
                    )
                );
                stream.writable.getWriter().closed.then(
                    () => globalThis.__transformTerminateCancelEvents.push("writer:resolved"),
                    error => globalThis.__transformTerminateCancelEvents.push(
                        `writer:${error.name}:${error instanceof TypeError}:${error.message}`
                    )
                );
                controller.terminate();
                return JSON.stringify(globalThis.__transformTerminateCancelEvents);
            })()
            "#,
        )
        .expect("TransformStream terminate after cancel setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformTerminateCancelEvents.sort())")
        .expect("TransformStream terminate after cancel promises should settle");
    assert_eq!(
        settled,
        r#"["cancel:TypeError","writer:TypeError:true:The transform stream has been terminated"]"#
    );
}

#[test]
fn transform_stream_identity_write_restores_writer_desired_size_after_read() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__identityDesiredSizeEvents = [];
                const stream = new TransformStream();
                const writer = stream.writable.getWriter();
                globalThis.__identityDesiredSizeEvents.push(`initial:${writer.desiredSize}`);
                writer.write("a").then(() => {
                    globalThis.__identityDesiredSizeEvents.push(`write:${writer.desiredSize}`);
                });
                globalThis.__identityDesiredSizeEvents.push(`after-write:${writer.desiredSize}`);
                stream.readable.getReader().read().then(result => {
                    globalThis.__identityDesiredSizeEvents.push(
                        `read:${result.value}:${result.done}:${writer.desiredSize}`
                    );
                });
                return JSON.stringify(globalThis.__identityDesiredSizeEvents);
            })()
            "#,
        )
        .expect("TransformStream identity desiredSize setup should evaluate");
    assert_eq!(initial, r#"["initial:1","after-write:0"]"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__identityDesiredSizeEvents)")
        .expect("TransformStream identity desiredSize should settle");
    assert_eq!(
        settled,
        r#"["initial:1","after-write:0","read:a:false:0","write:1"]"#
    );
}

#[test]
fn transform_stream_backpressured_write_and_close_wait_for_readable_demand() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformBackpressureOrderEvents = [];
                const stream = new TransformStream({}, undefined, { highWaterMark: 1 });
                globalThis.__transformBackpressureOrderWriter = stream.writable.getWriter();
                globalThis.__transformBackpressureOrderReader = stream.readable.getReader();
                globalThis.__transformBackpressureOrderWriter.write("a").then(() => {
                    globalThis.__transformBackpressureOrderEvents.push("write:a");
                });
                globalThis.__transformBackpressureOrderWriter.write("b").then(() => {
                    globalThis.__transformBackpressureOrderEvents.push("write:b");
                });
                globalThis.__transformBackpressureOrderWriter.close().then(() => {
                    globalThis.__transformBackpressureOrderEvents.push("close");
                });
                return JSON.stringify(globalThis.__transformBackpressureOrderEvents);
            })()
            "#,
        )
        .expect("TransformStream backpressured write setup should evaluate");
    assert_eq!(initial, "[]");

    let before_read = vm
        .eval("JSON.stringify(globalThis.__transformBackpressureOrderEvents)")
        .expect("TransformStream first write should settle before readable drain");
    assert_eq!(before_read, r#"["write:a"]"#);

    let read_started = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformBackpressureOrderReader.read().then(result => {
                    globalThis.__transformBackpressureOrderEvents.push(
                        `read:${result.value}:${result.done}`
                    );
                });
                return JSON.stringify(globalThis.__transformBackpressureOrderEvents);
            })()
            "#,
        )
        .expect("TransformStream backpressured read should evaluate");
    assert_eq!(read_started, r#"["write:a"]"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformBackpressureOrderEvents)")
        .expect("TransformStream backpressured writes should settle after read");
    assert_eq!(settled, r#"["write:a","read:a:false","write:b","close"]"#);
}

#[test]
fn transform_stream_readable_cancel_rejects_pipe_to_destination() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformPipeAbortEvents = [];
                globalThis.__transformPipeAbortError = new Error("pipe-abort");
                globalThis.__transformPipeAbortStream = new TransformStream();
                new ReadableStream().pipeTo(globalThis.__transformPipeAbortStream.writable).then(
                    () => globalThis.__transformPipeAbortEvents.push("pipe:resolved"),
                    error => globalThis.__transformPipeAbortEvents.push(
                        `pipe:${error === globalThis.__transformPipeAbortError}:${error.message}`
                    )
                );
                return JSON.stringify(globalThis.__transformPipeAbortEvents);
            })()
            "#,
        )
        .expect("TransformStream pipeTo abort setup should evaluate");
    assert_eq!(initial, "[]");

    let cancel_started = vm
        .eval(
            r#"
            (() => {
                globalThis.__transformPipeAbortStream.readable.cancel(
                    globalThis.__transformPipeAbortError
                );
                return JSON.stringify(globalThis.__transformPipeAbortEvents);
            })()
            "#,
        )
        .expect("TransformStream readable cancel should evaluate");
    assert_eq!(cancel_started, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__transformPipeAbortEvents)")
        .expect("TransformStream pipeTo promise should reject after readable cancel");
    assert_eq!(settled, r#"["pipe:true:pipe-abort"]"#);
}

#[test]
fn readable_stream_bad_strategy_size_errors_stream() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__badStrategySizeEvents = [];
                const getterError = new Error("getter");
                const methodError = new Error("method");
                const construction = [];
                try {
                    new ReadableStream({}, { get size() { throw getterError; }, highWaterMark: 1 });
                } catch (error) {
                    construction.push(error === getterError ? "getter" : error.name);
                }
                try {
                    new ReadableStream({}, { size: 1, highWaterMark: 1 });
                } catch (error) {
                    construction.push(error.name);
                }
                try {
                    new ReadableStream({}, {});
                    construction.push("empty-ok");
                } catch (error) {
                    construction.push(`empty:${error.name}`);
                }
                try {
                    new ReadableStream({}, { highWaterMark: NaN });
                } catch (error) {
                    construction.push(`nan:${error.name}`);
                }
                try {
                    new ReadableStream({}, { highWaterMark: -1 });
                } catch (error) {
                    construction.push(`negative:${error.name}`);
                }

                let throwController;
                const throwing = new ReadableStream({
                    start(c) {
                        throwController = c;
                    }
                }, {
                    highWaterMark: 1,
                    size() {
                        throw methodError;
                    }
                });
                throwing.getReader().closed.catch(error => {
                    globalThis.__badStrategySizeEvents.push(`throw-closed:${error === methodError}`);
                });
                try {
                    throwController.enqueue("x");
                } catch (error) {
                    globalThis.__badStrategySizeEvents.push(`throw-enqueue:${error === methodError}`);
                }

                let rangeController;
                const range = new ReadableStream({
                    start(c) {
                        rangeController = c;
                    }
                }, {
                    highWaterMark: 1,
                    size() {
                        return Infinity;
                    }
                });
                range.getReader().closed.catch(error => {
                    globalThis.__badStrategySizeEvents.push(`range-closed:${error.name}`);
                });
                try {
                    rangeController.enqueue("y");
                } catch (error) {
                    globalThis.__badStrategySizeEvents.push(`range-enqueue:${error.name}`);
                }

                const controllerError = { name: "controller error" };
                const thrownError = { name: "thrown error" };
                let priorityController;
                const priority = new ReadableStream({
                    start(c) {
                        priorityController = c;
                    }
                }, {
                    highWaterMark: 1,
                    size() {
                        priorityController.error(controllerError);
                        throw thrownError;
                    }
                });
                priority.getReader().closed.catch(error => {
                    globalThis.__badStrategySizeEvents.push(`priority-closed:${error === controllerError}`);
                });
                try {
                    priorityController.enqueue("z");
                } catch (error) {
                    globalThis.__badStrategySizeEvents.push(`priority-enqueue:${error === thrownError}`);
                }

                const rangeControllerError = { name: "range controller error" };
                let priorityRangeController;
                const priorityRange = new ReadableStream({
                    start(c) {
                        priorityRangeController = c;
                    }
                }, {
                    highWaterMark: 1,
                    size() {
                        priorityRangeController.error(rangeControllerError);
                        return Infinity;
                    }
                });
                priorityRange.getReader().closed.catch(error => {
                    globalThis.__badStrategySizeEvents.push(
                        `priority-range-closed:${error === rangeControllerError}`
                    );
                });
                try {
                    priorityRangeController.enqueue("q");
                } catch (error) {
                    globalThis.__badStrategySizeEvents.push(`priority-range-enqueue:${error.name}`);
                }

                let closeController;
                const closeInsideSize = new ReadableStream({
                    start(c) {
                        closeController = c;
                    }
                }, {
                    highWaterMark: 1,
                    size() {
                        closeController.close();
                        return 1;
                    }
                });
                closeController.enqueue("closed-chunk");
                closeInsideSize.getReader().read().then(({ value, done }) => {
                    globalThis.__badStrategySizeEvents.push(`close-read:${done}:${String(value)}`);
                });

                return JSON.stringify({ construction, events: globalThis.__badStrategySizeEvents });
            })()
            "#,
        )
        .expect("ReadableStream bad strategy size setup should evaluate");
    assert_eq!(
        initial,
        r#"{"construction":["getter","TypeError","empty-ok","nan:RangeError","negative:RangeError"],"events":["throw-enqueue:true","range-enqueue:RangeError","priority-enqueue:true","priority-range-enqueue:RangeError"]}"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__badStrategySizeEvents.sort())")
        .expect("ReadableStream bad strategy size closed promises should settle");
    assert_eq!(
        settled,
        r#"["close-read:true:undefined","priority-closed:true","priority-enqueue:true","priority-range-closed:true","priority-range-enqueue:RangeError","range-closed:RangeError","range-enqueue:RangeError","throw-closed:true","throw-enqueue:true"]"#
    );
}

#[test]
fn queuing_strategy_constructors_expose_high_water_mark_and_size() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const internalNames = () => Object.getOwnPropertyNames(globalThis)
                    .filter(name => name === "__moliCountQueuingStrategySizeFunction" ||
                        name === "__moliByteLengthQueuingStrategySizeFunction")
                    .sort()
                    .join(",");
                const internalNamesBefore = internalNames();
                const count = new CountQueuingStrategy({ highWaterMark: 5 });
                const count2 = new CountQueuingStrategy({ highWaterMark: 10 });
                const byteLength = new ByteLengthQueuingStrategy({ highWaterMark: 7 });
                const byteLength2 = new ByteLengthQueuingStrategy({ highWaterMark: 11 });
                const countSize = count.size;
                const countSize2 = count2.size;
                const byteLengthSize = byteLength.size;
                const byteLengthSize2 = byteLength2.size;
                const internalNamesAfterCache = internalNames();
                Object.defineProperties(globalThis, {
                    __moliCountQueuingStrategySizeFunction: {
                        configurable: true,
                        value() { return 99; }
                    },
                    __moliByteLengthQueuingStrategySizeFunction: {
                        configurable: true,
                        value() { return 99; }
                    }
                });
                const internalNamesAfterSpoof = internalNames();
                const countSizeAfterSpoof = count.size;
                const byteLengthSizeAfterSpoof = byteLength.size;
                const getterError = new Error("byteLength");
                const throws = callback => {
                    try {
                        callback();
                        return "no-throw";
                    } catch (error) {
                        return error === getterError ? "getter-error" : error.name;
                    }
                };
                return [
                    internalNamesBefore,
                    internalNamesAfterCache,
                    internalNamesAfterSpoof,
                    count.highWaterMark,
                    byteLength.highWaterMark,
                    countSize.name,
                    countSize.length,
                    countSize("ignored"),
                    countSize === countSize2,
                    countSize === countSizeAfterSpoof,
                    "prototype" in countSize,
                    throws(() => new countSize()),
                    byteLengthSize.name,
                    byteLengthSize.length,
                    byteLengthSize({ byteLength: 9 }),
                    byteLengthSize === byteLengthSize2,
                    byteLengthSize === byteLengthSizeAfterSpoof,
                    "prototype" in byteLengthSize,
                    throws(() => new byteLengthSize({ byteLength: 1 })),
                    throws(() => byteLengthSize()),
                    throws(() => byteLengthSize(null)),
                    byteLengthSize("potato"),
                    byteLengthSize({}),
                    byteLengthSize({ get byteLength() { return 13; } }),
                    throws(() => byteLengthSize({ get byteLength() { throw getterError; } })),
                    new ReadableStream({}, count) instanceof ReadableStream,
                ].join("|");
            })()
            "#,
        )
        .expect("QueuingStrategy constructors should evaluate");

    assert_eq!(
        result,
        "||__moliByteLengthQueuingStrategySizeFunction,__moliCountQueuingStrategySizeFunction|5|7|size|0|1|true|true|false|TypeError|size|1|9|true|true|false|TypeError|TypeError|TypeError|||13|getter-error|true"
    );
}

#[test]
fn stream_prototype_methods_preserve_declared_descriptors() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const methodDescriptor = (prototype, key, label = key) => {
                    const descriptor = Object.getOwnPropertyDescriptor(prototype, key);
                    return [
                        label,
                        typeof descriptor?.value,
                        descriptor?.value?.name,
                        descriptor?.value?.length,
                        descriptor?.enumerable,
                        descriptor?.writable,
                        descriptor?.configurable,
                    ].join(":");
                };
                const accessorDescriptor = (prototype, key, label = key) => {
                    const descriptor = Object.getOwnPropertyDescriptor(prototype, key);
                    return [
                        label,
                        typeof descriptor?.get,
                        descriptor?.get?.name,
                        descriptor?.get?.length,
                        descriptor?.enumerable,
                        Boolean(descriptor?.set),
                        descriptor?.configurable,
                    ].join(":");
                };
                const readable = new ReadableStream({
                    start(controller) {
                        controller.enqueue("queued");
                    }
                });
                const writable = new WritableStream({
                    write() {}
                });
                const reader = readable.getReader();
                const readableLockedAfterGet = readable.locked;
                reader.releaseLock();
                const readableLockedAfterRelease = readable.locked;
                const writer = writable.getWriter();
                const writableLockedAfterGet = writable.locked;
                writer.releaseLock();
                const writableLockedAfterRelease = writable.locked;
                return JSON.stringify({
                    readableMethods: [
                        methodDescriptor(ReadableStream.prototype, "getReader"),
                        methodDescriptor(ReadableStream.prototype, "cancel"),
                        methodDescriptor(ReadableStream.prototype, "pipeThrough"),
                        methodDescriptor(ReadableStream.prototype, "pipeTo"),
                        methodDescriptor(ReadableStream.prototype, "tee"),
                        methodDescriptor(ReadableStream.prototype, "values"),
                        methodDescriptor(
                            ReadableStream.prototype,
                            Symbol.asyncIterator,
                            "Symbol.asyncIterator"
                        ),
                    ],
                    readableLocked: accessorDescriptor(ReadableStream.prototype, "locked"),
                    readerMethods: [
                        methodDescriptor(ReadableStreamDefaultReader.prototype, "read"),
                        methodDescriptor(ReadableStreamDefaultReader.prototype, "releaseLock"),
                        methodDescriptor(ReadableStreamDefaultReader.prototype, "cancel"),
                    ],
                    readerAccessors: [
                        accessorDescriptor(ReadableStreamDefaultReader.prototype, "closed"),
                    ],
                    readableIteratorAlias:
                        ReadableStream.prototype.values ===
                        ReadableStream.prototype[Symbol.asyncIterator],
                    readableOwnNames: Object.getOwnPropertyNames(readable),
                    writableMethods: [
                        methodDescriptor(WritableStream.prototype, "getWriter"),
                        methodDescriptor(WritableStream.prototype, "abort"),
                        methodDescriptor(WritableStream.prototype, "close"),
                    ],
                    writableLocked: accessorDescriptor(WritableStream.prototype, "locked"),
                    writerMethods: [
                        methodDescriptor(WritableStreamDefaultWriter.prototype, "write"),
                        methodDescriptor(WritableStreamDefaultWriter.prototype, "close"),
                        methodDescriptor(WritableStreamDefaultWriter.prototype, "abort"),
                        methodDescriptor(WritableStreamDefaultWriter.prototype, "releaseLock"),
                    ],
                    writableOwnNames: Object.getOwnPropertyNames(writable),
                    lockStates: [
                        readableLockedAfterGet,
                        readableLockedAfterRelease,
                        writableLockedAfterGet,
                        writableLockedAfterRelease,
                    ],
                });
            })()
            "#,
        )
        .expect("stream prototype descriptors should evaluate");

    assert_eq!(
        result,
        "{\"readableMethods\":[\"getReader:function:getReader:0:true:true:true\",\"cancel:function:cancel:0:true:true:true\",\"pipeThrough:function:pipeThrough:1:true:true:true\",\"pipeTo:function:pipeTo:1:true:true:true\",\"tee:function:tee:0:true:true:true\",\"values:function:values:0:true:true:true\",\"Symbol.asyncIterator:function:values:0:false:true:true\"],\"readableLocked\":\"locked:function:get locked:0:true:false:true\",\"readerMethods\":[\"read:function:read:0:true:true:true\",\"releaseLock:function:releaseLock:0:true:true:true\",\"cancel:function:cancel:0:true:true:true\"],\"readerAccessors\":[\"closed:function:get closed:0:true:false:true\"],\"readableIteratorAlias\":true,\"readableOwnNames\":[],\"writableMethods\":[\"getWriter:function:getWriter:0:true:true:true\",\"abort:function:abort:0:true:true:true\",\"close:function:close:0:true:true:true\"],\"writableLocked\":\"locked:function:get locked:0:true:false:true\",\"writerMethods\":[\"write:function:write:0:true:true:true\",\"close:function:close:0:true:true:true\",\"abort:function:abort:0:true:true:true\",\"releaseLock:function:releaseLock:0:true:true:true\"],\"writableOwnNames\":[],\"lockStates\":[true,false,true,false]}"
    );
}

#[test]
fn readable_stream_default_reader_closed_tracks_close_and_error() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__readerClosedEvents = [];

                const closing = new ReadableStream({
                    start(controller) {
                        globalThis.__readerClosedCloseController = controller;
                    }
                });
                const closingReader = closing.getReader();
                const closingClosed = closingReader.closed;
                globalThis.__readerClosedEvents.push(
                    "close-same:" + (closingReader.closed === closingClosed) + ":" +
                    (closingClosed instanceof Promise)
                );
                closingClosed.then(
                    () => globalThis.__readerClosedEvents.push("close:resolved"),
                    error => globalThis.__readerClosedEvents.push("close:" + error.message)
                );
                globalThis.__readerClosedCloseController.close();

                const erroring = new ReadableStream({
                    start(controller) {
                        globalThis.__readerClosedErrorController = controller;
                    }
                });
                const erroringReader = erroring.getReader();
                const erroringClosed = erroringReader.closed;
                globalThis.__readerClosedEvents.push(
                    "error-same:" + (erroringReader.closed === erroringClosed)
                );
                erroringClosed.then(
                    () => globalThis.__readerClosedEvents.push("error:resolved"),
                    error => globalThis.__readerClosedEvents.push(
                        "error:" + error.name + ":" + error.message
                    )
                );
                globalThis.__readerClosedErrorController.error(new Error("boom"));

                const lateError = new ReadableStream({
                    start(controller) {
                        controller.error(new Error("late"));
                    }
                });
                lateError.getReader().closed.then(
                    () => globalThis.__readerClosedEvents.push("late:resolved"),
                    error => globalThis.__readerClosedEvents.push(
                        "late:" + error.name + ":" + error.message
                    )
                );

                return JSON.stringify(globalThis.__readerClosedEvents);
            })()
            "#,
        )
        .expect("ReadableStreamDefaultReader.closed setup should evaluate");
    assert_eq!(initial, r#"["close-same:true:true","error-same:true"]"#);

    let settled = vm
        .eval("JSON.stringify(globalThis.__readerClosedEvents.sort())")
        .expect("ReadableStreamDefaultReader.closed promises should settle");
    assert_eq!(
        settled,
        r#"["close-same:true:true","close:resolved","error-same:true","error:Error:boom","late:Error:late"]"#
    );
}

#[test]
fn readable_stream_default_reader_release_lock_rejects_closed_and_reads() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__readerReleaseEvents = [];
                const releasedPending = new ReadableStream({
                    start(controller) {
                        globalThis.__releasedPendingController = controller;
                    }
                });
                const pendingReader = releasedPending.getReader();
                const pendingRead = pendingReader.read();
                const pendingClosedBefore = pendingReader.closed;
                pendingClosedBefore.then(
                    () => globalThis.__readerReleaseEvents.push("pending-closed:resolved"),
                    error => globalThis.__readerReleaseEvents.push(
                        "pending-closed:" + error.name + ":" + (error instanceof TypeError)
                    )
                );
                pendingRead.then(
                    () => globalThis.__readerReleaseEvents.push("pending-read:resolved"),
                    error => globalThis.__readerReleaseEvents.push(
                        "pending-read:" + error.name + ":" + (error instanceof TypeError)
                    )
                );
                pendingReader.releaseLock();
                const pendingClosedAfter = pendingReader.closed;
                pendingClosedAfter.catch(() => {});
                pendingReader.read().then(
                    () => globalThis.__readerReleaseEvents.push("future-read:resolved"),
                    error => globalThis.__readerReleaseEvents.push(
                        "future-read:" + error.name + ":" + (error instanceof TypeError)
                    )
                );
                pendingReader.cancel("released").then(
                    () => globalThis.__readerReleaseEvents.push("future-cancel:resolved"),
                    error => globalThis.__readerReleaseEvents.push(
                        "future-cancel:" + error.name + ":" + (error instanceof TypeError)
                    )
                );

                const closedStream = new ReadableStream({
                    start(controller) {
                        controller.close();
                    }
                });
                const closedReader = closedStream.getReader();
                const closedBefore = closedReader.closed;
                closedBefore.then(
                    () => globalThis.__readerReleaseEvents.push("closed-before:resolved"),
                    error => globalThis.__readerReleaseEvents.push("closed-before:" + error.name)
                );
                closedReader.releaseLock();
                const closedAfter = closedReader.closed;
                closedAfter.then(
                    () => globalThis.__readerReleaseEvents.push("closed-after:resolved"),
                    error => globalThis.__readerReleaseEvents.push(
                        "closed-after:" + error.name + ":" + (error instanceof TypeError)
                    )
                );

                return JSON.stringify({
                    locked: releasedPending.locked,
                    pendingClosedSame: pendingClosedBefore === pendingClosedAfter,
                    closedReplaced: closedBefore !== closedAfter,
                    events: globalThis.__readerReleaseEvents
                });
            })()
            "#,
        )
        .expect("ReadableStreamDefaultReader.releaseLock setup should evaluate");

    assert_eq!(
        initial,
        r#"{"locked":false,"pendingClosedSame":true,"closedReplaced":true,"events":[]}"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__readerReleaseEvents.sort())")
        .expect("ReadableStreamDefaultReader.releaseLock promises should settle");
    assert_eq!(
        settled,
        r#"["closed-after:TypeError:true","closed-before:resolved","future-cancel:TypeError:true","future-read:TypeError:true","pending-closed:TypeError:true","pending-read:TypeError:true"]"#
    );
}

#[test]
fn readable_stream_default_reader_release_lock_suppresses_internal_closed_rejection() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__readerReleaseUnhandled = [];
                window.addEventListener("unhandledrejection", event => {
                    globalThis.__readerReleaseUnhandled.push(
                        "unhandled:" + event.reason.name
                    );
                });
                const stream = new ReadableStream({
                    start(controller) {
                        globalThis.__readerReleaseSuppressController = controller;
                    }
                });
                const reader = stream.getReader();
                const savedClosed = reader.closed;
                reader.releaseLock();
                return JSON.stringify({
                    locked: stream.locked,
                    closedSame: reader.closed === savedClosed
                });
            })()
            "#,
        )
        .expect("ReadableStreamDefaultReader.releaseLock suppress setup should evaluate");
    assert_eq!(initial, r#"{"locked":false,"closedSame":true}"#);

    for _ in 0..4 {
        vm.eval("JSON.stringify(globalThis.__readerReleaseUnhandled)")
            .expect("ReadableStreamDefaultReader.releaseLock suppress promises should drain");
    }

    let unhandled = vm
        .eval("JSON.stringify(globalThis.__readerReleaseUnhandled)")
        .expect("ReadableStreamDefaultReader.releaseLock suppress events should evaluate");
    assert_eq!(unhandled, "[]");
}

#[test]
fn readable_stream_default_reader_constructor_and_get_reader_mode_match_wpt() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const events = [];
                const nameOf = callback => {
                    try {
                        callback();
                        return "ok";
                    } catch (error) {
                        return error && error.name;
                    }
                };

                events.push(
                    "missing:" + nameOf(() => new ReadableStreamDefaultReader())
                );
                events.push(
                    "non-stream:" + nameOf(() => new ReadableStreamDefaultReader({}))
                );

                const directStream = new ReadableStream();
                const directReader = new ReadableStreamDefaultReader(directStream);
                const directClosed = directReader.closed;
                events.push(
                    "direct:" +
                    (directReader instanceof ReadableStreamDefaultReader) + ":" +
                    directStream.locked + ":" +
                    (directReader.closed === directClosed)
                );
                events.push(
                    "direct-locked:" +
                    nameOf(() => new ReadableStreamDefaultReader(directStream))
                );
                events.push(
                    "get-reader-locked:" + nameOf(() => directStream.getReader())
                );
                directReader.releaseLock();

                const closedStream = new ReadableStream({
                    start(controller) {
                        controller.close();
                    }
                });
                const closedReader = new ReadableStreamDefaultReader(closedStream);
                events.push("closed-direct:" + closedStream.locked);
                closedReader.releaseLock();

                const erroredStream = new ReadableStream({
                    start(controller) {
                        controller.error(new Error("stream-error"));
                    }
                });
                const erroredReader = new ReadableStreamDefaultReader(erroredStream);
                erroredReader.closed.catch(() => {});
                events.push("errored-direct:" + erroredStream.locked);
                erroredReader.releaseLock();

                let toStringCalled = false;
                const modeStream = new ReadableStream();
                const modeError = nameOf(() => modeStream.getReader({
                    mode: {
                        toString() {
                            toStringCalled = true;
                            return "";
                        }
                    }
                }));
                events.push("mode:" + modeError + ":" + toStringCalled);

                return events.join("|");
            })()
            "#,
        )
        .expect("ReadableStreamDefaultReader constructor probe should evaluate");

    assert_eq!(
        result,
        "missing:TypeError|non-stream:TypeError|direct:true:true:true|direct-locked:TypeError|get-reader-locked:TypeError|closed-direct:true|errored-direct:true|mode:TypeError:true"
    );
}

#[test]
fn readable_stream_start_rejected_promise_errors_stream_with_reason() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__startRejectEvents = [];
                const stream = new ReadableStream({
                    start() {
                        return Promise.reject();
                    }
                });
                const reader = stream.getReader();
                reader.read().then(
                    () => globalThis.__startRejectEvents.push("read:resolved"),
                    error => globalThis.__startRejectEvents.push(
                        "read:" + String(error) + ":" + (error === undefined)
                    )
                );
                reader.closed.then(
                    () => globalThis.__startRejectEvents.push("closed:resolved"),
                    error => globalThis.__startRejectEvents.push(
                        "closed:" + String(error) + ":" + (error === undefined)
                    )
                );
                return JSON.stringify(globalThis.__startRejectEvents);
            })()
            "#,
        )
        .expect("ReadableStream start reject setup should evaluate");
    assert_eq!(initial, "[]");

    vm.eval("undefined")
        .expect("ReadableStream start reject should advance one microtask turn");
    let settled = vm
        .eval("JSON.stringify(globalThis.__startRejectEvents.sort())")
        .expect("ReadableStream start reject promises should settle");
    assert_eq!(
        settled,
        r#"["closed:undefined:true","read:undefined:true"]"#
    );
}

#[test]
fn readable_stream_cancel_rejects_locked_stream_without_calling_source_cancel() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__lockedCancelEvents = [];
                const stream = new ReadableStream({
                    start(controller) {
                        controller.enqueue("a");
                        controller.close();
                    },
                    cancel() {
                        globalThis.__lockedCancelEvents.push("source-cancel");
                    }
                });
                const reader = stream.getReader();
                stream.cancel().then(
                    () => globalThis.__lockedCancelEvents.push("cancel:resolved"),
                    error => globalThis.__lockedCancelEvents.push("cancel:" + error.name)
                );
                reader.read().then(({ value, done }) => {
                    globalThis.__lockedCancelEvents.push(`read:${value}:${done}`);
                });
                reader.closed.then(() => {
                    globalThis.__lockedCancelEvents.push("closed");
                });
                return JSON.stringify(globalThis.__lockedCancelEvents);
            })()
            "#,
        )
        .expect("ReadableStream locked cancel setup should evaluate");
    assert_eq!(initial, "[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__lockedCancelEvents.sort())")
        .expect("ReadableStream locked cancel promises should settle");
    assert_eq!(settled, r#"["cancel:TypeError","closed","read:a:false"]"#);
}

#[test]
fn readable_stream_cancel_follows_underlying_source_result() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__cancelEvents = [];
                const thrown = new Error("thrown");
                const rejectReason = new Error("reject");

                new ReadableStream({
                    cancel(reason) {
                        globalThis.__cancelEvents.push("throw-called:" + reason);
                        throw thrown;
                    }
                }).cancel("sync").then(
                    () => globalThis.__cancelEvents.push("throw:resolved"),
                    error => globalThis.__cancelEvents.push("throw:" + (error === thrown))
                );

                let resolveSource;
                let sourceFulfilled = false;
                const resolved = new ReadableStream({
                    cancel() {
                        const promise = new Promise(resolve => { resolveSource = resolve; });
                        promise.then(() => { sourceFulfilled = true; });
                        return promise;
                    }
                });
                resolved.cancel().then(value => {
                    globalThis.__cancelEvents.push(
                        "resolve:" + sourceFulfilled + ":" + (value === undefined)
                    );
                });
                globalThis.__resolveCancel = () => resolveSource("ignored");

                let rejectSource;
                let sourceRejected = false;
                const rejected = new ReadableStream({
                    cancel() {
                        const promise = new Promise((_, reject) => { rejectSource = reject; });
                        promise.catch(() => { sourceRejected = true; });
                        return promise;
                    }
                });
                rejected.cancel().then(
                    () => globalThis.__cancelEvents.push("reject:resolved"),
                    error => globalThis.__cancelEvents.push(
                        "reject:" + sourceRejected + ":" + (error === rejectReason)
                    )
                );
                globalThis.__rejectCancel = () => rejectSource(rejectReason);
                return JSON.stringify(globalThis.__cancelEvents);
            })()
            "#,
        )
        .expect("ReadableStream cancel source result setup should evaluate");
    assert_eq!(initial, r#"["throw-called:sync"]"#);

    let after_throw = vm
        .eval("JSON.stringify(globalThis.__cancelEvents)")
        .expect("ReadableStream cancel sync throw should settle");
    assert_eq!(after_throw, r#"["throw-called:sync","throw:true"]"#);

    let after_resolve = vm
        .eval("globalThis.__resolveCancel(); JSON.stringify(globalThis.__cancelEvents)")
        .expect("ReadableStream cancel resolve should be scheduled");
    assert_eq!(after_resolve, r#"["throw-called:sync","throw:true"]"#);
    let after_resolve_settled = vm
        .eval("JSON.stringify(globalThis.__cancelEvents)")
        .expect("ReadableStream cancel resolve should settle");
    assert_eq!(
        after_resolve_settled,
        r#"["throw-called:sync","throw:true","resolve:true:true"]"#
    );

    let after_reject = vm
        .eval("globalThis.__rejectCancel(); JSON.stringify(globalThis.__cancelEvents)")
        .expect("ReadableStream cancel reject should be scheduled");
    assert_eq!(
        after_reject,
        r#"["throw-called:sync","throw:true","resolve:true:true"]"#
    );
    let after_reject_settled = vm
        .eval("JSON.stringify(globalThis.__cancelEvents)")
        .expect("ReadableStream cancel reject should settle");
    assert_eq!(
        after_reject_settled,
        r#"["throw-called:sync","throw:true","resolve:true:true","reject:true:true"]"#
    );
}

#[test]
fn readable_stream_underlying_source_algorithms_match_bad_sources_wpt() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__badSourceEvents = [];
                const events = globalThis.__badSourceEvents;
                const startGetterError = new Error("start-getter");
                const startMethodError = new Error("start-method");
                const pullGetterError = new Error("pull-getter");
                const pullMethodError = new Error("pull-method");
                const cancelGetterError = new Error("cancel-getter");
                const cancelMethodError = new Error("cancel-method");

                const nameOf = callback => {
                    try {
                        callback();
                        return "ok";
                    } catch (error) {
                        return error && error.message;
                    }
                };

                events.push("start-getter:" + nameOf(() => new ReadableStream({
                    get start() {
                        throw startGetterError;
                    }
                })));
                events.push("start-method:" + nameOf(() => new ReadableStream({
                    start() {
                        throw startMethodError;
                    }
                })));
                events.push("pull-getter:" + nameOf(() => new ReadableStream({
                    get pull() {
                        throw pullGetterError;
                    }
                })));
                events.push("cancel-getter:" + nameOf(() => new ReadableStream({
                    get cancel() {
                        throw cancelGetterError;
                    }
                })));

                const pullStream = new ReadableStream({
                    pull() {
                        throw pullMethodError;
                    }
                });
                const pullReader = pullStream.getReader();
                pullReader.closed.then(
                    () => events.push("pull-method:resolved"),
                    error => events.push("pull-method:" + (error === pullMethodError))
                );

                new ReadableStream({
                    cancel() {
                        throw cancelMethodError;
                    }
                }).cancel().then(
                    () => events.push("cancel-method:resolved"),
                    error => events.push("cancel-method:" + (error === cancelMethodError))
                );

                let counter = 0;
                const singleGetStream = new ReadableStream({
                    get pull() {
                        ++counter;
                        if (counter === 1) {
                            return controller => controller.enqueue("a");
                        }
                        throw new Error("second-get");
                    }
                });
                const singleGetReader = singleGetStream.getReader();
                Promise.all([
                    singleGetReader.read(),
                    singleGetReader.read(),
                ]).then(([first, second]) => {
                    events.push(
                        `single-get:${counter}:${first.value}:${first.done}:${second.value}:${second.done}`
                    );
                });

                let methodCounter = 0;
                const secondPullStream = new ReadableStream({
                    pull(controller) {
                        ++methodCounter;
                        if (methodCounter === 1) {
                            controller.enqueue("first");
                            return;
                        }
                        throw new Error("second-method");
                    }
                });
                const secondPullReader = secondPullStream.getReader();
                secondPullReader.read().then(({ value, done }) => {
                    events.push(`second-method-read:${value}:${done}`);
                });
                secondPullReader.closed.then(
                    () => events.push("second-method-closed:resolved"),
                    error => events.push(
                        `second-method-closed:${methodCounter}:${error.message}`
                    )
                );

                return JSON.stringify(events);
            })()
            "#,
        )
        .expect("ReadableStream bad underlying source setup should evaluate");
    assert_eq!(
        initial,
        r#"["start-getter:start-getter","start-method:start-method","pull-getter:pull-getter","cancel-getter:cancel-getter"]"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__badSourceEvents.sort())")
        .expect("ReadableStream bad underlying source promises should settle");
    assert_eq!(
        settled,
        r#"["cancel-getter:cancel-getter","cancel-method:true","pull-getter:pull-getter","pull-method:true","second-method-closed:2:second-method","second-method-read:first:false","single-get:1:a:false:a:false","start-getter:start-getter","start-method:start-method"]"#
    );
}

#[test]
fn readable_stream_controller_state_checks_match_bad_sources_wpt() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__controllerStateEvents = [];
                const events = globalThis.__controllerStateEvents;
                const nameOf = callback => {
                    try {
                        callback();
                        return "ok";
                    } catch (error) {
                        return error && error.name;
                    }
                };

                let canceledEmptyController;
                const canceledEmpty = new ReadableStream({
                    start(controller) {
                        canceledEmptyController = controller;
                    }
                });
                canceledEmpty.cancel();
                events.push("enqueue-canceled-empty:" + nameOf(() => {
                    canceledEmptyController.enqueue("a");
                }));
                events.push("close-canceled-empty:" + nameOf(() => {
                    canceledEmptyController.close();
                }));
                canceledEmpty.getReader().closed.then(() => {
                    events.push("canceled-empty-closed");
                });

                let canceledQueuedController;
                const canceledQueued = new ReadableStream({
                    start(controller) {
                        canceledQueuedController = controller;
                        controller.enqueue("a");
                    }
                });
                canceledQueued.cancel();
                events.push("enqueue-canceled-queued:" + nameOf(() => {
                    canceledQueuedController.enqueue("b");
                }));
                events.push("close-canceled-queued:" + nameOf(() => {
                    canceledQueuedController.close();
                }));
                canceledQueued.getReader().closed.then(() => {
                    events.push("canceled-queued-closed");
                });

                new ReadableStream({
                    start(controller) {
                        controller.close();
                        events.push("enqueue-closed:" + nameOf(() => controller.enqueue("a")));
                        events.push("close-closed:" + nameOf(() => controller.close()));
                        events.push("error-after-close:" + nameOf(() => controller.error()));
                    }
                }).getReader().closed.then(() => events.push("closed-stream-closed"));

                const error = new Error("boom");
                new ReadableStream({
                    start(controller) {
                        controller.error(error);
                        events.push("enqueue-errored:" + nameOf(() => controller.enqueue("a")));
                        events.push("close-errored:" + nameOf(() => controller.close()));
                        events.push("error-twice:" + nameOf(() => controller.error()));
                    }
                }).getReader().closed.then(
                    () => events.push("errored-stream:resolved"),
                    reason => events.push("errored-stream:" + (reason === error))
                );

                return JSON.stringify(events);
            })()
            "#,
        )
        .expect("ReadableStream controller state setup should evaluate");
    assert_eq!(
        initial,
        r#"["enqueue-canceled-empty:TypeError","close-canceled-empty:TypeError","enqueue-canceled-queued:TypeError","close-canceled-queued:TypeError","enqueue-closed:TypeError","close-closed:TypeError","error-after-close:ok","enqueue-errored:TypeError","close-errored:TypeError","error-twice:ok"]"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__controllerStateEvents.sort())")
        .expect("ReadableStream controller state promises should settle");
    assert_eq!(
        settled,
        r#"["canceled-empty-closed","canceled-queued-closed","close-canceled-empty:TypeError","close-canceled-queued:TypeError","close-closed:TypeError","close-errored:TypeError","closed-stream-closed","enqueue-canceled-empty:TypeError","enqueue-canceled-queued:TypeError","enqueue-closed:TypeError","enqueue-errored:TypeError","error-after-close:ok","error-twice:ok","errored-stream:true"]"#
    );
}

#[test]
fn transform_stream_readable_writable_getters_are_on_prototypes() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const encoder = new TextEncoderStream();
                const decoder = new TextDecoderStream();
                const transform = new TransformStream();
                const getterDescriptor = (prototype, name) => {
                    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
                    return [
                        name,
                        typeof descriptor?.get,
                        descriptor?.get?.name,
                        descriptor?.get?.length,
                        typeof descriptor?.set,
                        descriptor?.enumerable,
                        descriptor?.configurable
                    ].join(':');
                };
                const encoderReadableDescriptor = Object.getOwnPropertyDescriptor(TextEncoderStream.prototype, 'readable');
                const encoderWritableDescriptor = Object.getOwnPropertyDescriptor(TextEncoderStream.prototype, 'writable');
                const decoderReadableDescriptor = Object.getOwnPropertyDescriptor(TextDecoderStream.prototype, 'readable');
                const decoderWritableDescriptor = Object.getOwnPropertyDescriptor(TextDecoderStream.prototype, 'writable');
                const transformReadableDescriptor = Object.getOwnPropertyDescriptor(TransformStream.prototype, 'readable');
                const transformWritableDescriptor = Object.getOwnPropertyDescriptor(TransformStream.prototype, 'writable');
                const encoderReadable = encoderReadableDescriptor.get;
                const encoderWritable = encoderWritableDescriptor.get;
                const decoderReadable = decoderReadableDescriptor.get;
                const decoderWritable = decoderWritableDescriptor.get;
                const transformReadable = transformReadableDescriptor.get;
                const transformWritable = transformWritableDescriptor.get;
                return [
                    getterDescriptor(TextEncoderStream.prototype, 'readable'),
                    getterDescriptor(TextEncoderStream.prototype, 'writable'),
                    getterDescriptor(TextDecoderStream.prototype, 'readable'),
                    getterDescriptor(TextDecoderStream.prototype, 'writable'),
                    getterDescriptor(TransformStream.prototype, 'readable'),
                    getterDescriptor(TransformStream.prototype, 'writable'),
                    typeof encoderReadable,
                    typeof encoderWritable,
                    encoderReadable.call(encoder) === encoder.readable,
                    encoderWritable.call(encoder) === encoder.writable,
                    decoderReadable.call(decoder) === decoder.readable,
                    decoderWritable.call(decoder) === decoder.writable,
                    transformReadable.call(transform) === transform.readable,
                    transformWritable.call(transform) === transform.writable,
                    encoder.hasOwnProperty('readable'),
                    encoder.hasOwnProperty('writable'),
                    decoder.hasOwnProperty('readable'),
                    decoder.hasOwnProperty('writable'),
                    transform.hasOwnProperty('readable'),
                    transform.hasOwnProperty('writable'),
                ].join('|');
            })()
            "#,
        )
        .expect("TransformStream prototype getters should evaluate");

    assert_eq!(
        result,
        "readable:function:get readable:0:undefined:true:true|writable:function:get writable:0:undefined:true:true|readable:function:get readable:0:undefined:true:true|writable:function:get writable:0:undefined:true:true|readable:function:get readable:0:undefined:true:true|writable:function:get writable:0:undefined:true:true|function|function|true|true|true|true|true|true|false|false|false|false|false|false"
    );
}

#[test]
fn child_window_stream_constructors_keep_lengths_and_result_realms() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__childStreamRealmEvents = [];
                const frame = document.createElement('iframe');
                (document.body || document.documentElement || document).appendChild(frame);
                const win = frame.contentWindow;
                globalThis.__childStreamRealmWindow = win;

                const readable = new win.ReadableStream({
                    start(controller) {
                        controller.enqueue('ok');
                    }
                });
                readable.getReader().read().then(result => {
                    globalThis.__childStreamRealmEvents.push([
                        'readable',
                        Object.getPrototypeOf(result) === win.Object.prototype,
                        result.constructor === win.Object
                    ].join(':'));
                });

                const encoder = new win.TextEncoderStream();
                encoder.readable.getReader().read().then(result => {
                    globalThis.__childStreamRealmEvents.push([
                        'encoder',
                        Object.getPrototypeOf(result) === win.Object.prototype,
                        result.constructor === win.Object,
                        Object.getPrototypeOf(result.value) === win.Uint8Array.prototype,
                        encoder.hasOwnProperty('readable'),
                        encoder.hasOwnProperty('writable')
                    ].join(':'));
                });
                encoder.writable.getWriter().write('A');

                return [
                    win.ReadableStream.length,
                    win.WritableStream.length,
                    win.TransformStream.length,
                    win.TextEncoderStream.length,
                    win.TextDecoderStream.length,
                    JSON.stringify(globalThis.__childStreamRealmEvents)
                ].join('|');
            })()
            "#,
        )
        .expect("child stream realm setup should evaluate");
    assert_eq!(initial, "0|0|0|0|0|[]");

    let settled = vm
        .eval("JSON.stringify(globalThis.__childStreamRealmEvents.sort())")
        .expect("child stream realm promises should settle");
    assert_eq!(
        settled,
        r#"["encoder:true:true:true:false:false","readable:true:true"]"#
    );
}

#[test]
fn borrowed_stream_methods_create_readers_iterators_and_tee_branches_in_stream_realm() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const frame = document.createElement('iframe');
                (document.body || document.documentElement || document).appendChild(frame);
                const other = frame.contentWindow;

                const stream = new ReadableStream();
                const borrowedReader = other.ReadableStream.prototype.getReader.call(stream);
                const constructedReader = new other.ReadableStreamDefaultReader(new ReadableStream());
                const readerRealms = [
                    borrowedReader instanceof ReadableStreamDefaultReader,
                    borrowedReader instanceof other.ReadableStreamDefaultReader,
                    constructedReader instanceof ReadableStreamDefaultReader,
                    constructedReader instanceof other.ReadableStreamDefaultReader
                ];
                borrowedReader.releaseLock();
                constructedReader.releaseLock();

                const iterator = other.ReadableStream.prototype.values.call(new ReadableStream(), {
                    preventCancel: true
                });
                const mainIteratorPrototype = Object.getPrototypeOf(
                    new ReadableStream().values({ preventCancel: true })
                );
                const iteratorRealm = Object.getPrototypeOf(iterator) === mainIteratorPrototype;
                iterator.return();

                const branches = other.ReadableStream.prototype.tee.call(new ReadableStream());

                let pendingController;
                const pendingStream = new ReadableStream({
                    start(controller) {
                        pendingController = controller;
                    }
                });
                const pendingReader = other.ReadableStream.prototype.getReader.call(pendingStream);
                const pendingRead = pendingReader.read();
                globalThis.__borrowedStreamReadResultRealm = [];
                pendingRead.then(result => {
                    globalThis.__borrowedStreamReadResultRealm.push(
                        result instanceof other.Object,
                        result instanceof Object,
                        result.done
                    );
                });
                other.ReadableStreamDefaultController.prototype.close.call(pendingController);

                return JSON.stringify({
                    readerRealms,
                    iteratorRealm,
                    branchRealms: branches.map(branch => [
                        branch instanceof ReadableStream,
                        branch instanceof other.ReadableStream
                    ])
                });
            })()
            "#,
        )
        .expect("borrowed stream method realms should evaluate");

    assert_eq!(
        result,
        r#"{"readerRealms":[true,false,false,true],"iteratorRealm":true,"branchRealms":[[true,false],[true,false]]}"#
    );

    let settled = vm
        .eval("JSON.stringify(globalThis.__borrowedStreamReadResultRealm)")
        .expect("borrowed stream read result realm should settle");
    assert_eq!(settled, "[true,false,true]");
}

#[test]
fn readable_stream_pipe_through_forwards_future_text_encoder_chunks() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__pipeThroughEvents = [];
                const source = new ReadableStream({
                    start(controller) {
                        globalThis.__pipeThroughController = controller;
                    }
                });
                const encoded = source.pipeThrough(new TextEncoderStream());
                globalThis.__pipeThroughReader = encoded.getReader();
                globalThis.__pipeThroughReader.read().then(({ value, done }) => {
                    globalThis.__pipeThroughEvents.push(
                        `${Array.from(value || []).join(",")}:${done}`
                    );
                });
                return JSON.stringify(globalThis.__pipeThroughEvents);
            })()
            "#,
        )
        .expect("pipeThrough setup should evaluate");
    assert_eq!(initial, "[]");

    vm.eval(
        r#"
        (() => {
            globalThis.__pipeThroughController.enqueue("abc");
            return JSON.stringify(globalThis.__pipeThroughEvents);
        })()
        "#,
    )
    .expect("future pipeThrough enqueue should evaluate");
    let after_enqueue = vm
        .eval("JSON.stringify(globalThis.__pipeThroughEvents)")
        .expect("future pipeThrough enqueue should settle");
    assert_eq!(after_enqueue, r#"["97,98,99:false"]"#);

    vm.eval(
        r#"
        (() => {
            globalThis.__pipeThroughReader.read().then(({ value, done }) => {
                globalThis.__pipeThroughEvents.push(`${String(value)}:${done}`);
            });
            globalThis.__pipeThroughController.close();
            return JSON.stringify(globalThis.__pipeThroughEvents);
        })()
        "#,
    )
    .expect("future pipeThrough close should evaluate");
    let after_close = vm
        .eval("JSON.stringify(globalThis.__pipeThroughEvents)")
        .expect("future pipeThrough close should settle");
    assert_eq!(after_close, r#"["97,98,99:false","undefined:true"]"#);
}

#[test]
fn tee_cancel_settles_when_source_was_closed_with_queued_chunks() {
    let mut vm = new_storage_test_vm("https://tee-closed-source.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
              globalThis.__teeClosedSourceEvents = [];
              const source = new ReadableStream({
                start(controller) {
                  controller.enqueue("one");
                  controller.enqueue("two");
                  controller.close();
                },
                cancel(reason) {
                  globalThis.__teeClosedSourceEvents.push(`source-cancel:${String(reason)}`);
                }
              });
              const [left, right] = source.tee();
              const leftReader = left.getReader();
              const rightReader = right.getReader();
              Promise.all([
                leftReader.read().then(({ value }) => {
                  globalThis.__teeClosedSourceEvents.push(`left:${value}`);
                  return leftReader.cancel("left-stop").then(() => {
                    globalThis.__teeClosedSourceEvents.push("left-canceled");
                  });
                }),
                (async () => {
                  const values = [];
                  while (true) {
                    const { value, done } = await rightReader.read();
                    if (done) break;
                    values.push(value);
                  }
                  globalThis.__teeClosedSourceEvents.push(`right:${values.join("|")}`);
                })()
              ]).then(() => globalThis.__teeClosedSourceEvents.push("settled"));
              return JSON.stringify(globalThis.__teeClosedSourceEvents);
            })()
            "#,
        )
        .expect("closed source tee setup should evaluate");
    assert_eq!(initial, "[]");

    for _ in 0..8 {
        vm.eval("0")
            .expect("closed source tee promise chain should drain");
    }
    let settled = vm
        .eval("JSON.stringify(globalThis.__teeClosedSourceEvents)")
        .expect("closed source tee events should evaluate");
    // Chromium drains the surviving branch before the shared tee cancel
    // promise reaction runs once the already-closed source is observed.
    assert_eq!(
        settled,
        r#"["left:one","right:one|two","left-canceled","settled"]"#
    );
}

#[test]
fn writable_stream_controller_signal_aborts_before_underlying_sink_abort() {
    let mut vm = new_storage_test_vm("https://writable-controller-signal.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
              globalThis.__writableControllerSignalEvents = [];
              let capturedSignal;
              const stream = new WritableStream({
                start(controller) {
                  capturedSignal = controller.signal;
                  __writableControllerSignalEvents.push(`start:${controller.signal.aborted}`);
                  controller.signal.addEventListener("abort", () => {
                    __writableControllerSignalEvents.push(`signal:${String(controller.signal.reason)}`);
                  });
                },
                write(chunk, controller) {
                  __writableControllerSignalEvents.push(
                    `write:${chunk}:${controller.signal === capturedSignal}:${controller.signal.aborted}`
                  );
                },
                abort(reason) {
                  __writableControllerSignalEvents.push(
                    `sink:${String(reason)}:${capturedSignal.aborted}:${String(capturedSignal.reason)}`
                  );
                }
              });
              const writer = stream.getWriter();
              writer.write("one").then(() => writer.abort("stop")).then(() => {
                __writableControllerSignalEvents.push("settled");
              });
              return JSON.stringify({
                events: __writableControllerSignalEvents,
                signalTag: Object.prototype.toString.call(capturedSignal),
                descriptor: (() => {
                  const descriptor = Object.getOwnPropertyDescriptor(
                    WritableStreamDefaultController.prototype,
                    "signal"
                  );
                  return [typeof descriptor.get, descriptor.enumerable, descriptor.configurable];
                })()
              });
            })()
            "#,
        )
        .expect("WritableStream controller signal setup should evaluate");
    assert_eq!(
        initial,
        r#"{"events":["start:false"],"signalTag":"[object AbortSignal]","descriptor":["function",true,true]}"#
    );

    for _ in 0..8 {
        vm.eval("0")
            .expect("WritableStream controller signal promise chain should drain");
    }
    let settled = vm
        .eval("JSON.stringify(globalThis.__writableControllerSignalEvents)")
        .expect("WritableStream controller signal events should evaluate");
    assert_eq!(
        settled,
        r#"["start:false","write:one:true:false","signal:stop","sink:stop:true:stop","settled"]"#
    );
}

#[test]
fn pipe_through_resumes_after_transform_backpressure_and_closes_once() {
    let mut vm = new_storage_test_vm("https://transform-backpressure.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__transformBackpressureResult = null;
          const input = ["alpha", "beta", "gamma"];
          let index = 0;
          const source = new ReadableStream({
            pull(controller) {
              if (index === input.length) {
                controller.close();
                return;
              }
              controller.enqueue(input[index++]);
            }
          }, { highWaterMark: 1 });
          globalThis.__transformBackpressureSource = source;
          const output = source.pipeThrough(new TransformStream({
            transform(chunk, controller) {
              controller.enqueue(chunk.toUpperCase());
            },
            flush(controller) {
              controller.enqueue("FLUSH");
            }
          }, { highWaterMark: 1 }, { highWaterMark: 1 }));
          (async () => {
            const values = [];
            for await (const value of output) values.push(value);
            globalThis.__transformBackpressureResult = JSON.stringify({
              values,
              sourceLocked: source.locked,
              outputLocked: output.locked
            });
          })();
        })()
        "#,
    )
    .expect("transform backpressure pipeline should initialize");

    for _ in 0..32 {
        vm.eval("0")
            .expect("transform backpressure pipeline should drain");
    }
    let result = vm
        .eval("globalThis.__transformBackpressureResult")
        .expect("transform backpressure result should evaluate");
    assert_eq!(
        result,
        r#"{"values":["ALPHA","BETA","GAMMA","FLUSH"],"sourceLocked":true,"outputLocked":false}"#
    );
    let source_unlocked = vm
        .eval("String(!globalThis.__transformBackpressureSource.locked)")
        .expect("transform source lock should evaluate after pipe shutdown finalization");
    assert_eq!(source_unlocked, "true");
}

#[test]
fn transform_pipe_observes_source_refill_before_write_and_releases_after_shutdown() {
    let mut vm = new_storage_test_vm("https://transform-pipe-order.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__transformPipeOrderResult = null;
          const lifecycle = [];
          const input = ["alpha", "beta", "gamma"];
          let index = 0;
          const source = new ReadableStream({
            pull(controller) {
              const chunk = input[index];
              if (chunk === undefined) {
                lifecycle.push("source-close");
                controller.close();
                return;
              }
              index += 1;
              lifecycle.push(`source:${chunk}:${String(controller.desiredSize)}`);
              controller.enqueue(chunk);
            }
          }, { highWaterMark: 1 });
          globalThis.__transformPipeOrderSource = source;
          const output = source.pipeThrough(new TransformStream({
            transform(chunk, controller) {
              lifecycle.push(`transform:${chunk}:${String(controller.desiredSize)}`);
              controller.enqueue(`${index}:${chunk.toUpperCase()}`);
            },
            flush(controller) {
              lifecycle.push(`flush:${String(controller.desiredSize)}`);
              controller.enqueue("FLUSH");
            }
          }, { highWaterMark: 1 }, { highWaterMark: 1 }));
          (async () => {
            const reader = output.getReader();
            const values = [];
            while (true) {
              const { value, done } = await reader.read();
              if (done) break;
              values.push(value);
            }
            await reader.closed;
            reader.releaseLock();
            globalThis.__transformPipeOrderResult = JSON.stringify({
              values,
              lifecycle,
              sourceLockedAtReaderClose: source.locked,
              outputLocked: output.locked
            });
          })();
        })()
        "#,
    )
    .expect("transform pipe order should initialize");

    for _ in 0..32 {
        vm.eval("0").expect("transform pipe order should drain");
    }
    let result = vm
        .eval("globalThis.__transformPipeOrderResult")
        .expect("transform pipe order result should evaluate");
    assert_eq!(
        result,
        r#"{"values":["2:ALPHA","3:BETA","3:GAMMA","FLUSH"],"lifecycle":["source:alpha:1","source:beta:1","transform:alpha:1","source:gamma:1","transform:beta:1","source-close","transform:gamma:1","flush:1"],"sourceLockedAtReaderClose":true,"outputLocked":false}"#
    );
    let source_unlocked = vm
        .eval("String(!globalThis.__transformPipeOrderSource.locked)")
        .expect("transform pipe source lock should evaluate after shutdown");
    assert_eq!(source_unlocked, "true");
}

#[test]
fn text_encoder_decoder_pipe_chain_releases_each_pipe_after_one_close() {
    let mut vm = new_storage_test_vm("https://encoding-pipe-close.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__encodingPipeCloseResult = null;
          const input = ["react:", "café:", "東京"];
          let index = 0;
          const source = new ReadableStream({
            pull(controller) {
              if (index === input.length) {
                controller.close();
                return;
              }
              controller.enqueue(input[index++]);
            }
          });
          const pipeline = source
            .pipeThrough(new TextEncoderStream())
            .pipeThrough(new TextDecoderStream("utf-8", { fatal: true }));
          (async () => {
            const values = [];
            for await (const value of pipeline) values.push(value);
            globalThis.__encodingPipeCloseResult = JSON.stringify({
              values,
              combined: values.join(""),
              sourceLocked: source.locked,
              pipelineLocked: pipeline.locked
            });
          })();
        })()
        "#,
    )
    .expect("encoding pipe chain should initialize");

    for _ in 0..48 {
        vm.eval("0").expect("encoding pipe chain should drain");
    }
    let result = vm
        .eval("globalThis.__encodingPipeCloseResult")
        .expect("encoding pipe close result should evaluate");
    assert_eq!(
        result,
        r#"{"values":["react:","café:","東京"],"combined":"react:café:東京","sourceLocked":false,"pipelineLocked":false}"#
    );
}

#[test]
fn readable_byte_stream_byob_reads_transfer_buffers_and_preserve_remainders() {
    let mut vm = new_storage_test_vm("https://byte-stream-byob.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamByobResult = null;
          const sourceBytes = new Uint8Array([1, 2, 3, 127, 128, 254, 255]);
          const lifecycle = [];
          const stream = new ReadableStream({
            type: "bytes",
            start(controller) {
              lifecycle.push(`start:${String(controller.desiredSize)}`);
              lifecycle.push(Object.prototype.toString.call(controller));
              controller.enqueue(sourceBytes);
              lifecycle.push(`enqueue:${sourceBytes.byteLength}`);
              controller.close();
            }
          });
          const reader = stream.getReader({ mode: "byob" });
          (async () => {
            const firstBuffer = new ArrayBuffer(3);
            const first = await reader.read(new Uint8Array(firstBuffer));
            const secondBuffer = new ArrayBuffer(8);
            const second = await reader.read(new Uint8Array(secondBuffer));
            const terminalBuffer = new ArrayBuffer(2);
            const terminal = await reader.read(new Uint8Array(terminalBuffer));
            reader.releaseLock();
            globalThis.__byteStreamByobResult = JSON.stringify({
              first: Array.from(first.value),
              firstInput: firstBuffer.byteLength,
              second: Array.from(second.value),
              secondInput: secondBuffer.byteLength,
              terminal: [terminal.done, terminal.value.byteLength, terminalBuffer.byteLength],
              lifecycle,
              byobReader: reader instanceof ReadableStreamBYOBReader,
              unlocked: !stream.locked
            });
          })();
        })()
        "#,
    )
    .expect("byte stream BYOB reads should initialize");

    for _ in 0..12 {
        vm.eval("0").expect("byte stream BYOB reads should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamByobResult")
        .expect("byte stream BYOB result should evaluate");
    assert_eq!(
        result,
        r#"{"first":[1,2,3],"firstInput":0,"second":[127,128,254,255],"secondInput":0,"terminal":[true,0,0],"lifecycle":["start:0","[object ReadableByteStreamController]","enqueue:0"],"byobReader":true,"unlocked":true}"#
    );
}

#[test]
fn readable_byte_stream_pending_byob_read_is_fulfilled_by_pull() {
    let mut vm = new_storage_test_vm("https://byte-stream-pending-byob.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
              globalThis.__pendingByobResult = null;
              let pulls = 0;
              const stream = new ReadableStream({
                type: "bytes",
                pull(controller) {
                  pulls += 1;
                  controller.enqueue(new Uint8Array([9, 8]));
                  controller.close();
                }
              });
              const reader = new ReadableStreamBYOBReader(stream);
              const input = new ArrayBuffer(4);
              reader.read(new Uint8Array(input)).then(({ value, done }) => {
                reader.releaseLock();
                globalThis.__pendingByobResult = JSON.stringify({
                  value: Array.from(value),
                  done,
                  input: input.byteLength,
                  pulls,
                  unlocked: !stream.locked
                });
              });
              return `${input.byteLength}:${pulls}`;
            })()
            "#,
        )
        .expect("pending BYOB read should initialize");
    assert_eq!(initial, "0:0");

    for _ in 0..8 {
        vm.eval("0").expect("pending BYOB read should drain");
    }
    let result = vm
        .eval("globalThis.__pendingByobResult")
        .expect("pending BYOB result should evaluate");
    assert_eq!(
        result,
        r#"{"value":[9,8],"done":false,"input":0,"pulls":1,"unlocked":true}"#
    );
}

#[test]
fn readable_byte_stream_byob_request_auto_allocation_and_typed_partial_respond_match_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-byob-request.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamByobRequestResult = null;
          const output = {};
          let autoPull = 0;
          const autoStream = new ReadableStream({
            type: "bytes",
            autoAllocateChunkSize: 4,
            pull(controller) {
              autoPull += 1;
              const request = controller.byobRequest;
              output.autoRequest = [
                request === controller.byobRequest,
                request instanceof ReadableStreamBYOBRequest,
                request.view.constructor.name,
                request.view.byteOffset,
                request.view.byteLength
              ];
              request.view.set([7, 8]);
              request.respond(2);
              output.autoStale = [request.view, controller.byobRequest];
              controller.close();
            }
          });

          let typedPull = 0;
          const typedStream = new ReadableStream({
            type: "bytes",
            pull(controller) {
              typedPull += 1;
              const request = controller.byobRequest;
              request.view[0] = typedPull === 1 ? 0x11 : 0x22;
              request.respond(1);
              if (typedPull === 2) controller.close();
            }
          });

          (async () => {
            const auto = await autoStream.getReader().read();
            output.auto = [Array.from(auto.value), auto.done, autoPull];

            const input = new ArrayBuffer(8);
            const typed = await typedStream
              .getReader({ mode: "byob" })
              .read(new Uint16Array(input, 2, 2));
            output.typed = [
              typed.value.constructor.name,
              typed.value.byteOffset,
              typed.value.byteLength,
              typed.value[0],
              input.byteLength,
              typedPull
            ];
            output.surface = [
              ReadableStreamBYOBReader.prototype.read.length,
              ReadableStreamBYOBRequest.prototype.respond.length,
              ReadableStreamBYOBRequest.prototype.respondWithNewView.length,
              Object.prototype.toString.call(Object.getPrototypeOf(controllerForTag()))
            ];
            globalThis.__byteStreamByobRequestResult = JSON.stringify(output);
          })();

          function controllerForTag() {
            let captured;
            new ReadableStream({ type: "bytes", start(controller) { captured = controller; } });
            return captured;
          }
        })()
        "#,
    )
    .expect("BYOB request oracle should initialize");

    for _ in 0..24 {
        vm.eval("0").expect("BYOB request oracle should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamByobRequestResult")
        .expect("BYOB request oracle result should evaluate");
    assert_eq!(
        result,
        r#"{"autoRequest":[true,true,"Uint8Array",0,4],"autoStale":[null,null],"auto":[[7,8],false,1],"typed":["Uint16Array",2,2,8721,0,2],"surface":[1,1,1,"[object ReadableByteStreamController]"]}"#
    );
}

#[test]
fn readable_stream_start_promise_boundary_matches_chromium_wpt() {
    let mut vm = new_storage_test_vm("https://readable-start-boundary.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__readableStartBoundaryResult = "pending";

          (async () => {
            let implicitPulls = 0;
            const implicit = new ReadableStream({
              pull(controller) {
                implicitPulls += 1;
                controller.enqueue("first");
                controller.enqueue("second");
                controller.close();
              }
            }, { highWaterMark: 0 });
            const implicitReader = implicit.getReader();
            const implicitReads = Promise.all([
              implicitReader.read(),
              implicitReader.read()
            ]);
            const implicitSyncPulls = implicitPulls;
            const [implicitFirst, implicitSecond] = await implicitReads;

            let explicitPulls = 0;
            const explicit = new ReadableStream({
              start(controller) {
                controller.enqueue("queued-in-start");
              },
              pull() {
                explicitPulls += 1;
              }
            });
            const explicitRead = explicit.getReader().read();
            const explicitSyncPulls = explicitPulls;
            const explicitResult = await explicitRead;
            await Promise.resolve();

            let closedPulls = 0;
            const closed = new ReadableStream({
              start(controller) {
                controller.enqueue("terminal");
                controller.close();
                return Promise.resolve();
              },
              pull() {
                closedPulls += 1;
              }
            });
            const closedReader = closed.getReader();
            const closedFirst = await closedReader.read();
            const closedSecond = await closedReader.read();
            await closedReader.closed;

            globalThis.__readableStartBoundaryResult = JSON.stringify({
              implicit: {
                syncPulls: implicitSyncPulls,
                pulls: implicitPulls,
                values: [
                  [implicitFirst.value, implicitFirst.done],
                  [implicitSecond.value, implicitSecond.done]
                ]
              },
              explicit: {
                syncPulls: explicitSyncPulls,
                pulls: explicitPulls,
                value: [explicitResult.value, explicitResult.done]
              },
              closed: {
                pulls: closedPulls,
                values: [
                  [closedFirst.value, closedFirst.done],
                  [closedSecond.value === undefined, closedSecond.done]
                ]
              }
            });
          })().catch(error => {
            globalThis.__readableStartBoundaryResult =
              `error:${error && error.name}:${error && error.message}`;
          });
        })()
        "#,
    )
    .expect("ReadableStream start promise boundary setup should evaluate");

    for _ in 0..24 {
        let result = vm
            .eval("globalThis.__readableStartBoundaryResult")
            .expect("ReadableStream start promise boundary should drain");
        if result != "pending" {
            break;
        }
    }

    let result = vm
        .eval("globalThis.__readableStartBoundaryResult")
        .expect("ReadableStream start promise boundary result should evaluate");
    assert_eq!(
        result,
        r#"{"implicit":{"syncPulls":0,"pulls":1,"values":[["first",false],["second",false]]},"explicit":{"syncPulls":0,"pulls":1,"value":["queued-in-start",false]},"closed":{"pulls":0,"values":[["terminal",false],[true,true]]}}"#
    );
}

#[test]
fn readable_byte_stream_respond_after_enqueue_matches_chromium_wpt() {
    let mut vm = new_storage_test_vm("https://byte-stream-respond-after-enqueue.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamRespondAfterEnqueueResult = "pending";
          (async () => {
            const freshStream = new ReadableStream({
              type: "bytes",
              autoAllocateChunkSize: 10,
              pull(controller) {
                controller.enqueue(new Uint8Array([1, 2, 3]));
                controller.byobRequest.respond(10);
              }
            });
            const fresh = await freshStream.getReader().read();

            const cachedStream = new ReadableStream({
              type: "bytes",
              autoAllocateChunkSize: 10,
              pull(controller) {
                const request = controller.byobRequest;
                controller.enqueue(new Uint8Array([1, 2, 3]));
                request.respond(10);
              }
            });
            const cached = await cachedStream.getReader().read();

            const doubleStream = new ReadableStream({
              type: "bytes",
              autoAllocateChunkSize: 10,
              pull(controller) {
                controller.enqueue(new Uint8Array([1, 2, 3]));
                controller.byobRequest.respond(2);
              }
            });
            const doubleReader = doubleStream.getReader();
            const [first, second] = await Promise.all([
              doubleReader.read(),
              doubleReader.read()
            ]);

            globalThis.__byteStreamRespondAfterEnqueueResult = JSON.stringify({
              fresh: [Array.from(fresh.value), fresh.done],
              cached: [Array.from(cached.value), cached.done],
              double: [
                Array.from(first.value),
                first.done,
                Array.from(second.value),
                second.done
              ]
            });
          })().catch(error => {
            globalThis.__byteStreamRespondAfterEnqueueResult =
              `error:${error && error.name}:${error && error.message}`;
          });
        })()
        "#,
    )
    .expect("byte stream respond-after-enqueue WPT should initialize");

    for _ in 0..48 {
        let result = vm
            .eval("globalThis.__byteStreamRespondAfterEnqueueResult")
            .expect("byte stream respond-after-enqueue WPT should drain");
        if result != "pending" {
            break;
        }
    }
    let result = vm
        .eval("globalThis.__byteStreamRespondAfterEnqueueResult")
        .expect("byte stream respond-after-enqueue result should evaluate");
    assert_eq!(
        result,
        r#"{"fresh":[[1,2,3],false],"cached":[[1,2,3],false],"double":[[1,2,3],false,[0,0],false]}"#
    );
}

#[test]
fn readable_byte_stream_commits_all_fillable_descriptors_before_resolving_promises() {
    let mut vm = new_storage_test_vm("https://byte-stream-reentrant-resolution.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamReentrantResolutionResult = null;
          (async () => {
            let controller;
            const stream = new ReadableStream({
              type: "bytes",
              start(value) { controller = value; }
            });
            const reader = stream.getReader({ mode: "byob" });
            const length = 0x4000;
            const read1 = reader.read(new Uint8Array(0x100));
            const read2 = reader.read(
              new BigUint64Array(new ArrayBuffer(length), length - 8, 1)
            );

            let thenObserved = false;
            let requestWasNull = false;
            Object.defineProperty(Object.prototype, "then", {
              configurable: true,
              get() {
                if (!thenObserved) {
                  thenObserved = true;
                  requestWasNull = controller.byobRequest === null;
                }
                return undefined;
              }
            });

            try {
              controller.enqueue(new Uint8Array(0x110).fill(0x42));
              const result1 = await read1;
              const result2 = await read2;
              globalThis.__byteStreamReentrantResolutionResult = JSON.stringify({
                thenObserved,
                requestWasNull,
                first: [
                  result1.done,
                  result1.value.byteLength,
                  result1.value.every(value => value === 0x42)
                ],
                second: [
                  result2.done,
                  result2.value.constructor.name,
                  result2.value.byteOffset,
                  result2.value.length,
                  result2.value[0].toString(16)
                ]
              });
            } finally {
              delete Object.prototype.then;
            }
          })();
        })()
        "#,
    )
    .expect("reentrant BYOB descriptor resolution should initialize");

    for _ in 0..32 {
        vm.eval("0")
            .expect("reentrant BYOB descriptor resolution should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamReentrantResolutionResult")
        .expect("reentrant BYOB descriptor resolution result should evaluate");
    assert_eq!(
        result,
        r#"{"thenObserved":true,"requestWasNull":true,"first":[false,256,true],"second":[false,"BigUint64Array",16376,1,"4242424242424242"]}"#
    );
}

#[test]
fn readable_byte_stream_auto_allocation_close_and_byob_request_surface_match_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-auto-close.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamAutoCloseResult = null;
          const probe = callback => {
            try { return String(callback()); } catch (error) { return error.name; }
          };
          let controller;
          let firstRequest;
          let firstAlias;
          const stream = new ReadableStream({
            type: "bytes",
            autoAllocateChunkSize: 4,
            pull(value) {
              controller = value;
              firstRequest = value.byobRequest;
              firstAlias = firstRequest.view;
              value.close();
            }
          });

          (async () => {
            const terminal = await stream.getReader().read();
            const before = [
              terminal.value === undefined,
              terminal.done,
              firstRequest.view.constructor.name,
              firstRequest.view.byteLength,
              firstAlias.byteLength,
              controller.byobRequest === firstRequest
            ];

            firstRequest.respond(0);
            const secondRequest = controller.byobRequest;
            const secondAlias = secondRequest.view;
            const middle = [
              firstRequest.view,
              firstAlias.byteLength,
              secondRequest === firstRequest,
              secondAlias.constructor.name,
              secondAlias.byteLength
            ];

            secondRequest.respondWithNewView(new Uint8Array(
              secondAlias.buffer,
              secondAlias.byteOffset,
              0
            ));
            const after = [
              secondRequest.view,
              secondAlias.byteLength,
              controller.byobRequest === secondRequest,
              controller.byobRequest.view.byteLength
            ];

            const viewDescriptor = Object.getOwnPropertyDescriptor(
              ReadableStreamBYOBRequest.prototype,
              "view"
            );
            const surface = [
              Object.prototype.toString.call(firstRequest),
              probe(() => new ReadableStreamBYOBRequest()),
              probe(() => viewDescriptor.get.call({})),
              probe(() => ReadableStreamBYOBRequest.prototype.respond.call({}, 0)),
              viewDescriptor.enumerable,
              viewDescriptor.configurable
            ];
            globalThis.__byteStreamAutoCloseResult = JSON.stringify({
              before,
              middle,
              after,
              surface
            });
          })();
        })()
        "#,
    )
    .expect("closed auto-allocation state machine should initialize");

    for _ in 0..32 {
        vm.eval("0")
            .expect("closed auto-allocation state machine should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamAutoCloseResult")
        .expect("closed auto-allocation result should evaluate");
    assert_eq!(
        result,
        r#"{"before":[true,true,"Uint8Array",4,4,true],"middle":[null,0,false,"Uint8Array",4],"after":[null,0,false,4],"surface":["[object ReadableStreamBYOBRequest]","TypeError","TypeError","TypeError",true,true]}"#
    );
}

#[test]
fn readable_byte_stream_min_new_view_release_and_byte_tee_match_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-state-machine.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamStateMachineResult = null;
          (async () => {
            const output = {};

            let pulls = 0;
            const minimumStream = new ReadableStream({
              type: "bytes",
              pull(controller) {
                pulls += 1;
                const request = controller.byobRequest;
                request.view[0] = pulls;
                request.respond(1);
                if (pulls === 4) controller.close();
              }
            });
            const minimumInput = new ArrayBuffer(12);
            const minimum = await minimumStream
              .getReader({ mode: "byob" })
              .read(new Uint16Array(minimumInput, 2, 4), { min: 2 });
            output.minimum = [
              minimum.value.constructor.name,
              minimum.value.byteOffset,
              minimum.value.byteLength,
              Array.from(new Uint8Array(
                minimum.value.buffer,
                minimum.value.byteOffset,
                minimum.value.byteLength
              )),
              minimumInput.byteLength,
              pulls
            ];

            let replacementController;
            const replacementStream = new ReadableStream({
              type: "bytes",
              start(controller) { replacementController = controller; }
            });
            const replacementReader = replacementStream.getReader({ mode: "byob" });
            const replacementPromise = replacementReader.read(
              new Uint8Array(new ArrayBuffer(8), 2, 4)
            );
            const replacementRequest = replacementController.byobRequest;
            const replacement = new Uint8Array(
              replacementRequest.view.buffer,
              replacementRequest.view.byteOffset,
              2
            );
            replacement.set([5, 6]);
            replacementRequest.respondWithNewView(replacement);
            const replacementResult = await replacementPromise;
            output.replacement = [
              Array.from(replacementResult.value),
              replacementResult.value.byteOffset,
              replacementResult.value.buffer.byteLength,
              replacementRequest.view,
              replacementController.byobRequest
            ];

            let releaseController;
            const releaseStream = new ReadableStream({
              type: "bytes",
              start(controller) { releaseController = controller; }
            });
            const firstReader = releaseStream.getReader({ mode: "byob" });
            const releasedRead = firstReader.read(new Uint8Array(4)).catch(error => error.name);
            const releasedRequest = releaseController.byobRequest;
            releasedRequest.view[0] = 9;
            firstReader.releaseLock();
            const secondReader = releaseStream.getReader({ mode: "byob" });
            const replacementRead = secondReader.read(new Uint8Array(4));
            releaseController.enqueue(new Uint8Array([8]));
            releaseController.close();
            output.release = [
              await releasedRead,
              Array.from((await replacementRead).value),
              releasedRequest.view,
              releaseController.byobRequest
            ];

            let sourceRequest;
            const teeSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                sourceRequest = controller.byobRequest;
                controller.enqueue(new Uint8Array([3, 4]));
                controller.close();
              }
            });
            const [left, right] = teeSource.tee();
            const leftRead = left.getReader({ mode: "byob" }).read(new Uint8Array(4));
            const rightRead = right.getReader().read();
            output.tee = [
              Array.from((await leftRead).value),
              Array.from((await rightRead).value),
              sourceRequest !== null,
              sourceRequest.view
            ];

            globalThis.__byteStreamStateMachineResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream state machine oracle should initialize");

    for _ in 0..48 {
        vm.eval("0")
            .expect("byte stream state machine oracle should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamStateMachineResult")
        .expect("byte stream state machine result should evaluate");
    assert_eq!(
        result,
        r#"{"minimum":["Uint16Array",2,4,[1,2,3,4],0,4],"replacement":[[5,6],2,8,null,null],"release":["TypeError",[8],null,null],"tee":[[3,4],[3,4],true,null]}"#
    );
}

#[test]
fn readable_byte_stream_release_hands_retained_descriptors_to_replacement_readers() {
    let mut vm = new_storage_test_vm("https://byte-stream-reader-handoff.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamReaderHandoffResult = null;
          (async () => {
            const output = {};

            let remainderController;
            const remainderStream = new ReadableStream({
              type: "bytes",
              start(controller) { remainderController = controller; }
            });
            const remainderReader1 = remainderStream.getReader({ mode: "byob" });
            const remainderRead1 = remainderReader1
              .read(new Uint8Array([1, 2, 3]))
              .catch(error => error.name);
            await Promise.resolve();
            const remainderRequest = remainderController.byobRequest;
            remainderReader1.releaseLock();
            const remainderReader2 = remainderStream.getReader({ mode: "byob" });
            const remainderRead2 = remainderReader2.read(new Uint8Array([4, 5]));
            const retainedRemainderRequest = remainderController.byobRequest === remainderRequest;
            const retainedRemainderView = Array.from(remainderRequest.view);
            remainderRequest.view.set([6, 7, 8]);
            remainderRequest.respond(3);
            const remainderResult2 = await remainderRead2;
            const remainderResult3 = await remainderReader2.read(new Uint8Array(3));
            output.remainder = [
              await remainderRead1,
              retainedRemainderRequest,
              retainedRemainderView,
              Array.from(remainderResult2.value),
              Array.from(remainderResult3.value),
              remainderRequest.view,
              remainderController.byobRequest
            ];

            let partialController;
            const partialStream = new ReadableStream({
              type: "bytes",
              start(controller) { partialController = controller; }
            });
            const partialReader1 = partialStream.getReader({ mode: "byob" });
            const partialRead1 = partialReader1
              .read(new Uint16Array(1))
              .catch(error => error.name);
            await Promise.resolve();
            const partialRequest1 = partialController.byobRequest;
            partialRequest1.view[0] = 0x11;
            partialRequest1.respond(1);
            const partialRequest2 = partialController.byobRequest;
            partialReader1.releaseLock();
            const partialReader2 = partialStream.getReader({ mode: "byob" });
            const partialRead2 = partialReader2.read(new Uint16Array(1));
            const retainedPartialRequest = partialController.byobRequest === partialRequest2;
            partialRequest2.view[0] = 0x22;
            partialRequest2.respond(1);
            const partialResult = await partialRead2;
            output.partial = [
              await partialRead1,
              retainedPartialRequest,
              Array.from(new Uint8Array(
                partialResult.value.buffer,
                partialResult.value.byteOffset,
                partialResult.value.byteLength
              )),
              partialResult.value.constructor.name,
              partialRequest2.view
            ];

            let closeController;
            const closeStream = new ReadableStream({
              type: "bytes",
              start(controller) { closeController = controller; }
            });
            const closeReader1 = closeStream.getReader({ mode: "byob" });
            const closeRead1 = closeReader1.read(new Uint8Array(3)).catch(error => error.name);
            await Promise.resolve();
            const closeRequest = closeController.byobRequest;
            closeReader1.releaseLock();
            const closeReader2 = closeStream.getReader({ mode: "byob" });
            const closeRead2 = closeReader2.read(new Uint8Array([4, 5, 6]));
            closeController.close();
            const retainedCloseRequest = closeController.byobRequest === closeRequest;
            closeRequest.respond(0);
            const closeResult = await closeRead2;
            output.close = [
              await closeRead1,
              retainedCloseRequest,
              closeResult.done,
              closeResult.value.constructor.name,
              closeResult.value.byteLength,
              Array.from(closeResult.value),
              closeRequest.view,
              closeController.byobRequest
            ];

            let autoController;
            const autoStream = new ReadableStream({
              type: "bytes",
              autoAllocateChunkSize: 4,
              start(controller) { autoController = controller; }
            });
            const autoReader1 = autoStream.getReader();
            const autoRead1 = autoReader1.read().catch(error => error.name);
            await Promise.resolve();
            const autoRequest = autoController.byobRequest;
            autoReader1.releaseLock();
            const autoReader2 = autoStream.getReader();
            const autoRead2 = autoReader2.read();
            const retainedAutoRequest = autoController.byobRequest === autoRequest;
            autoRequest.view[0] = 11;
            autoRequest.respond(1);
            const autoResult = await autoRead2;
            output.auto = [
              await autoRead1,
              retainedAutoRequest,
              Array.from(autoResult.value),
              autoResult.value.buffer.byteLength,
              autoRequest.view,
              autoController.byobRequest
            ];

            let defaultController;
            const defaultStream = new ReadableStream({
              type: "bytes",
              start(controller) { defaultController = controller; }
            });
            const defaultReader1 = defaultStream.getReader({ mode: "byob" });
            const defaultRead1 = defaultReader1
              .read(new Uint16Array(1))
              .catch(error => error.name);
            await Promise.resolve();
            const defaultRequest1 = defaultController.byobRequest;
            defaultRequest1.view[0] = 0x11;
            defaultRequest1.respond(1);
            const defaultRequest2 = defaultController.byobRequest;
            defaultReader1.releaseLock();
            const defaultReader2 = defaultStream.getReader();
            const defaultRead2 = defaultReader2.read();
            defaultController.enqueue(new Uint8Array([0x22]));
            const defaultResult2 = await defaultRead2;
            const defaultResult3 = await defaultReader2.read();
            output.defaultAfterPartial = [
              await defaultRead1,
              Array.from(defaultResult2.value),
              Array.from(defaultResult3.value),
              defaultRequest2.view,
              defaultController.byobRequest
            ];

            let respondDefaultController;
            const respondDefaultStream = new ReadableStream({
              type: "bytes",
              start(controller) { respondDefaultController = controller; }
            });
            const respondDefaultReader1 = respondDefaultStream.getReader({ mode: "byob" });
            const respondDefaultRead1 = respondDefaultReader1
              .read(new Uint8Array(3))
              .catch(error => error.name);
            await Promise.resolve();
            const respondDefaultRequest = respondDefaultController.byobRequest;
            respondDefaultReader1.releaseLock();
            const respondDefaultRead2 = respondDefaultStream.getReader().read();
            respondDefaultRequest.view.set([31, 32]);
            respondDefaultRequest.respond(2);
            output.defaultAfterRespond = [
              await respondDefaultRead1,
              Array.from((await respondDefaultRead2).value),
              respondDefaultRequest.view,
              respondDefaultController.byobRequest
            ];

            globalThis.__byteStreamReaderHandoffResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream reader handoff matrix should initialize");

    for _ in 0..96 {
        vm.eval("0")
            .expect("byte stream reader handoff matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamReaderHandoffResult")
        .expect("byte stream reader handoff result should evaluate");
    assert_eq!(
        result,
        r#"{"remainder":["TypeError",true,[1,2,3],[6,7],[8],null,null],"partial":["TypeError",true,[17,34],"Uint16Array",null],"close":["TypeError",true,true,"Uint8Array",0,[],null,null],"auto":["TypeError",true,[11],4,null,null],"defaultAfterPartial":["TypeError",[17],[34],null,null],"defaultAfterRespond":["TypeError",[31,32],null,null]}"#
    );
}

#[test]
fn readable_byte_stream_tee_preserves_byob_views_terminal_types_and_cancel_reasons() {
    let mut vm = new_storage_test_vm("https://byte-stream-tee-matrix.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamTeeMatrixResult = null;
          (async () => {
            const output = {};

            let sourceView;
            const offsetSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                const request = controller.byobRequest;
                sourceView = [
                  request !== null,
                  request.view.buffer.byteLength,
                  request.view.byteOffset,
                  request.view.byteLength
                ];
                request.view.set([1, 2, 3]);
                request.respond(3);
                controller.close();
              }
            });
            const [offsetLeft, offsetRight] = offsetSource.tee();
            const offsetInput = new ArrayBuffer(16);
            const offsetLeftRead = offsetLeft
              .getReader({ mode: "byob" })
              .read(new Uint8Array(offsetInput, 4, 6));
            const offsetRightRead = offsetRight.getReader().read();
            const offsetLeftResult = await offsetLeftRead;
            const offsetRightResult = await offsetRightRead;
            offsetLeftResult.value[0] = 9;
            output.offset = [
              sourceView,
              [
                Array.from(offsetLeftResult.value),
                offsetLeftResult.value.buffer.byteLength,
                offsetLeftResult.value.byteOffset,
                offsetInput.byteLength
              ],
              [
                Array.from(offsetRightResult.value),
                offsetRightResult.value.buffer.byteLength,
                offsetRightResult.value.byteOffset
              ]
            ];

            let defaultSourceSawNull;
            const defaultFirstSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                defaultSourceSawNull = controller.byobRequest === null;
                controller.enqueue(new Uint8Array([4, 5]));
                controller.close();
              }
            });
            const [defaultFirstLeft, defaultFirstRight] = defaultFirstSource.tee();
            const defaultFirstRightRead = defaultFirstRight.getReader().read();
            await Promise.resolve();
            const defaultFirstLeftRead = defaultFirstLeft
              .getReader({ mode: "byob" })
              .read(new Uint8Array(4));
            output.defaultFirst = [
              defaultSourceSawNull,
              Array.from((await defaultFirstLeftRead).value),
              Array.from((await defaultFirstRightRead).value)
            ];

            let terminalSourceSawRequest;
            const terminalSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                const request = controller.byobRequest;
                terminalSourceSawRequest = request !== null;
                controller.close();
                request.respond(0);
              }
            });
            const [terminalLeft, terminalRight] = terminalSource.tee();
            const terminalLeftRead = terminalLeft
              .getReader({ mode: "byob" })
              .read(new Uint16Array(new ArrayBuffer(8), 2, 2));
            const terminalRightRead = terminalRight
              .getReader({ mode: "byob" })
              .read(new Uint32Array(2));
            const terminalLeftResult = await terminalLeftRead;
            const terminalRightResult = await terminalRightRead;
            output.terminal = [
              terminalSourceSawRequest,
              [
                terminalLeftResult.done,
                terminalLeftResult.value.constructor.name,
                terminalLeftResult.value.byteOffset,
                terminalLeftResult.value.byteLength,
                terminalLeftResult.value.buffer.byteLength
              ],
              [
                terminalRightResult.done,
                terminalRightResult.value.constructor.name,
                terminalRightResult.value.byteOffset,
                terminalRightResult.value.byteLength,
                terminalRightResult.value.buffer.byteLength
              ]
            ];

            let cancelReason;
            let cancelCalls = 0;
            const cancelSource = new ReadableStream({
              type: "bytes",
              cancel(reason) {
                cancelCalls += 1;
                cancelReason = reason;
                return 7;
              }
            });
            const [cancelLeft, cancelRight] = cancelSource.tee();
            let firstCancelSettled = false;
            const firstCancel = cancelLeft.cancel("left").then(value => {
              firstCancelSettled = true;
              return value;
            });
            await Promise.resolve();
            const settledBeforeSecondCancel = firstCancelSettled;
            const secondCancel = cancelRight.cancel("right");
            const cancelValues = await Promise.all([firstCancel, secondCancel]);
            output.cancel = [
              settledBeforeSecondCancel,
              cancelCalls,
              cancelReason,
              cancelValues
            ];

            globalThis.__byteStreamTeeMatrixResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream tee matrix should initialize");

    for _ in 0..128 {
        vm.eval("0").expect("byte stream tee matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamTeeMatrixResult")
        .expect("byte stream tee matrix result should evaluate");
    assert_eq!(
        result,
        r#"{"offset":[[true,16,4,6],[[9,2,3],16,4,0],[[1,2,3],3,0]],"defaultFirst":[true,[4,5],[4,5]],"terminal":[true,[true,"Uint16Array",2,0,8],[true,"Uint32Array",0,0,8]],"cancel":[false,1,["left","right"],[null,null]]}"#
    );
}

#[test]
fn readable_byte_stream_tee_propagates_demand_error_and_single_branch_cancel() {
    let mut vm = new_storage_test_vm("https://byte-stream-tee-lifecycle.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamTeeLifecycleResult = null;
          (async () => {
            const output = {};

            let idlePulls = 0;
            const idleSource = new ReadableStream({
              type: "bytes",
              pull() { idlePulls += 1; }
            });
            idleSource.tee();
            await Promise.resolve();
            await Promise.resolve();
            output.noDemandPulls = idlePulls;

            const sourceError = new Error("tee-source-error");
            const errorSource = new ReadableStream({
              type: "bytes",
              pull(controller) { controller.error(sourceError); }
            });
            const [errorLeft, errorRight] = errorSource.tee();
            const errorLeftReader = errorLeft.getReader();
            const errorRightReader = errorRight.getReader({ mode: "byob" });
            const errorResults = await Promise.all([
              errorLeftReader.read().then(
                () => false,
                error => error === sourceError
              ),
              errorRightReader.read(new Uint8Array(4)).then(
                () => false,
                error => error === sourceError
              ),
              errorLeftReader.closed.then(
                () => false,
                error => error === sourceError
              ),
              errorRightReader.closed.then(
                () => false,
                error => error === sourceError
              )
            ]);
            output.error = errorResults;

            let cancelCalls = 0;
            let produced = 0;
            const cancelSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                produced += 1;
                controller.enqueue(new Uint8Array([produced]));
                if (produced === 3) controller.close();
              },
              cancel() { cancelCalls += 1; }
            });
            const [cancelLeft, cancelRight] = cancelSource.tee();
            let firstCancelSettled = false;
            const firstCancel = cancelLeft.cancel("unused").then(value => {
              firstCancelSettled = true;
              return value;
            });
            await Promise.resolve();
            const settledBeforeDrain = firstCancelSettled;
            const cancelRightReader = cancelRight.getReader({ mode: "byob" });
            const drained = [];
            for (;;) {
              const result = await cancelRightReader.read(new Uint8Array(2));
              if (result.done) break;
              drained.push(...result.value);
            }
            output.singleCancel = [
              settledBeforeDrain,
              await firstCancel,
              drained,
              produced,
              cancelCalls
            ];

            let bufferedProduced = 0;
            const bufferedSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                bufferedProduced += 1;
                controller.enqueue(new Uint8Array([10 + bufferedProduced]));
                if (bufferedProduced === 3) controller.close();
              }
            });
            const [fast, slow] = bufferedSource.tee();
            const fastReader = fast.getReader();
            const fastValues = [];
            for (;;) {
              const result = await fastReader.read();
              if (result.done) break;
              fastValues.push(...result.value);
            }
            const slowReader = slow.getReader({ mode: "byob" });
            const slowValues = [];
            for (;;) {
              const result = await slowReader.read(new Uint8Array(1));
              if (result.done) break;
              slowValues.push(...result.value);
            }
            output.fastSlow = [fastValues, slowValues, bufferedProduced];

            globalThis.__byteStreamTeeLifecycleResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream tee lifecycle matrix should initialize");

    for _ in 0..160 {
        vm.eval("0")
            .expect("byte stream tee lifecycle matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamTeeLifecycleResult")
        .expect("byte stream tee lifecycle result should evaluate");
    assert_eq!(
        result,
        r#"{"noDemandPulls":0,"error":[true,true,true,true],"singleCancel":[false,null,[1,2,3],3,0],"fastSlow":[[11,12,13],[11,12,13],3]}"#
    );
}

#[test]
fn readable_byte_stream_tee_preserves_original_view_and_serializes_byob_close() {
    let mut vm = new_storage_test_vm("https://byte-stream-tee-read-owner.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteTeeReadOwnerResult = null;
          (async () => {
            const output = {};

            let offsetPulls = 0;
            const offsetSource = new ReadableStream({
              type: "bytes",
              pull(controller) {
                offsetPulls += 1;
                if (offsetPulls === 1) {
                  const buffer = new Uint8Array([1, 2, 3]).buffer;
                  controller.enqueue(new Uint8Array(buffer, 2));
                }
              }
            });
            const [offsetLeft, offsetRight] = offsetSource.tee();
            const [leftResult, rightResult] = await Promise.all([
              offsetLeft.getReader().read(),
              offsetRight.getReader().read()
            ]);
            output.offset = [
              [
                leftResult.value.byteOffset,
                leftResult.value.byteLength,
                leftResult.value.buffer.byteLength,
                Array.from(leftResult.value)
              ],
              [
                rightResult.value.byteOffset,
                rightResult.value.byteLength,
                rightResult.value.buffer.byteLength,
                Array.from(rightResult.value)
              ],
              leftResult.value.buffer !== rightResult.value.buffer
            ];

            let closeController;
            const closeSource = new ReadableStream({
              type: "bytes",
              start(controller) { closeController = controller; }
            });
            const [closeLeft, closeRight] = closeSource.tee();
            const closeLeftReader = closeLeft.getReader({ mode: "byob" });
            const closeRightReader = closeRight.getReader({ mode: "byob" });
            const reads = [
              closeLeftReader.read(new Uint8Array(1)),
              closeLeftReader.read(new Uint8Array(1)),
              closeRightReader.read(new Uint8Array(1)),
              closeRightReader.read(new Uint8Array(1))
            ];
            while (closeController.byobRequest === null) {
              await Promise.resolve();
            }
            closeController.byobRequest.view[0] = 0x11;
            closeController.byobRequest.respond(1);
            closeController.close();
            const closeResults = await Promise.all(reads);
            output.close = closeResults.map(result => [
              Array.from(result.value),
              result.value.byteOffset,
              result.value.byteLength,
              result.value.buffer.byteLength,
              result.done
            ]);

            globalThis.__byteTeeReadOwnerResult = JSON.stringify(output);
          })().catch(error => {
            globalThis.__byteTeeReadOwnerResult = `error:${error.name}:${error.message}`;
          });
        })()
        "#,
    )
    .expect("byte tee read owner matrix should initialize");

    for _ in 0..96 {
        vm.eval("0")
            .expect("byte tee read owner matrix should drain");
    }
    assert_eq!(
        vm.eval("globalThis.__byteTeeReadOwnerResult")
            .expect("byte tee read owner matrix result should evaluate"),
        r#"{"offset":[[2,1,3,[3]],[0,1,1,[3]],true],"close":[[[17],0,1,1,false],[[],0,0,1,true],[[17],0,1,1,false],[[],0,0,1,true]]}"#
    );
}

#[test]
fn readable_byte_stream_close_and_invalidated_request_paths_match_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-terminal-state.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamTerminalResult = null;
          (async () => {
            const output = {};
            let closeController;
            const closeStream = new ReadableStream({
              type: "bytes",
              start(controller) { closeController = controller; }
            });
            const closeReader = closeStream.getReader({ mode: "byob" });
            const closeRead = closeReader.read(new Uint16Array(4), { min: 2 });
            const partialRequest = closeController.byobRequest;
            partialRequest.view.set([0x11, 0x22]);
            partialRequest.respond(2);
            const terminalRequest = closeController.byobRequest;
            closeController.close();
            terminalRequest.respond(0);
            const partial = await closeRead;
            const terminal = await closeReader.read(new Uint16Array(2));
            output.close = [
              Array.from(new Uint8Array(
                partial.value.buffer,
                partial.value.byteOffset,
                partial.value.byteLength
              )),
              partial.done,
              terminal.value.constructor.name,
              terminal.value.byteLength,
              terminal.done,
              terminalRequest.view
            ];

            let enqueueController;
            const enqueueStream = new ReadableStream({
              type: "bytes",
              start(controller) { enqueueController = controller; }
            });
            const enqueueReader = enqueueStream.getReader({ mode: "byob" });
            const enqueueRead = enqueueReader.read(new Uint16Array(4));
            const stale = enqueueController.byobRequest;
            const staleView = stale.view;
            enqueueController.enqueue(new Uint8Array([1, 2, 3]));
            const enqueueResult = await enqueueRead;
            let staleError;
            try { stale.respond(1); } catch (error) { staleError = error.name; }
            output.enqueue = [
              Array.from(new Uint8Array(
                enqueueResult.value.buffer,
                enqueueResult.value.byteOffset,
                enqueueResult.value.byteLength
              )),
              stale.view,
              staleView.byteLength,
              staleError
            ];

            let alignmentController;
            const alignmentStream = new ReadableStream({
              type: "bytes",
              start(controller) { alignmentController = controller; }
            });
            const alignmentReader = alignmentStream.getReader({ mode: "byob" });
            const alignmentRead = alignmentReader
              .read(new Uint16Array(4), { min: 2 })
              .catch(error => error.name);
            const alignmentRequest = alignmentController.byobRequest;
            alignmentRequest.view[0] = 1;
            alignmentRequest.respond(1);
            let closeError;
            try { alignmentController.close(); } catch (error) { closeError = error.name; }
            output.alignment = [closeError, await alignmentRead];

            const queuedCloseStream = new ReadableStream({
              type: "bytes",
              start(controller) {
                controller.enqueue(new Uint8Array([1, 2, 3]));
                controller.close();
              }
            });
            const queuedCloseReader = queuedCloseStream.getReader({ mode: "byob" });
            const queuedCloseFirst = await queuedCloseReader.read(new Uint16Array(2));
            const queuedCloseSecond = await queuedCloseReader
              .read(new Uint16Array(2))
              .then(value => value, error => error.name);
            output.queuedClose = [
              Array.from(new Uint8Array(
                queuedCloseFirst.value.buffer,
                queuedCloseFirst.value.byteOffset,
                queuedCloseFirst.value.byteLength
              )),
              queuedCloseFirst.done,
              queuedCloseSecond,
              await queuedCloseReader.closed.then(() => "closed", error => error.name)
            ];

            globalThis.__byteStreamTerminalResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream terminal oracle should initialize");

    for _ in 0..48 {
        vm.eval("0")
            .expect("byte stream terminal oracle should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamTerminalResult")
        .expect("byte stream terminal result should evaluate");
    assert_eq!(
        result,
        r#"{"close":[[17,34],true,"Uint16Array",0,true,null],"enqueue":[[1,2],null,0,"TypeError"],"alignment":["TypeError","TypeError"],"queuedClose":[[1,2],false,"TypeError","TypeError"]}"#
    );
}

#[test]
fn readable_byte_stream_cancel_and_error_settle_pending_byob_reads() {
    let mut vm = new_storage_test_vm("https://byte-stream-cancel-error.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamCancelErrorResult = null;
          (async () => {
            const output = {};

            let cancelController;
            let cancelReason;
            const cancelStream = new ReadableStream({
              type: "bytes",
              start(controller) { cancelController = controller; },
              cancel(reason) { cancelReason = reason; }
            });
            const cancelReader = cancelStream.getReader({ mode: "byob" });
            const cancelInput = new ArrayBuffer(4);
            const cancelRead = cancelReader.read(new Uint16Array(cancelInput));
            await Promise.resolve();
            const partialRequest = cancelController.byobRequest;
            partialRequest.view[0] = 7;
            partialRequest.respond(1);
            const terminalRequest = cancelController.byobRequest;
            const reason = { kind: "stop" };
            const cancelResult = cancelReader.cancel(reason);
            const canceledRead = await cancelRead;
            output.cancel = [
              canceledRead.value,
              canceledRead.done,
              await cancelResult,
              cancelReason === reason,
              cancelInput.byteLength,
              terminalRequest.view,
              cancelController.byobRequest,
              await cancelReader.closed.then(() => "closed", error => error.name)
            ];
            output.staleAfterCancel = (() => {
              try { terminalRequest.respond(0); return "resolved"; }
              catch (error) { return error.name; }
            })();

            let errorController;
            const streamError = { kind: "source-error" };
            const errorStream = new ReadableStream({
              type: "bytes",
              start(controller) { errorController = controller; }
            });
            const errorReader = errorStream.getReader({ mode: "byob" });
            const errorInput = new ArrayBuffer(4);
            const errorRead = errorReader
              .read(new Uint8Array(errorInput))
              .then(() => false, error => error === streamError);
            const errorClosed = errorReader.closed
              .then(() => false, error => error === streamError);
            await Promise.resolve();
            const errorRequest = errorController.byobRequest;
            errorController.error(streamError);
            output.error = [
              await errorRead,
              await errorClosed,
              errorInput.byteLength,
              errorRequest.view,
              errorController.byobRequest
            ];

            const alreadyErroredReason = { kind: "already" };
            const alreadyErrored = new ReadableStream({
              type: "bytes",
              start(controller) { controller.error(alreadyErroredReason); }
            });
            const untouchedInput = new ArrayBuffer(8);
            const alreadyResult = await alreadyErrored
              .getReader({ mode: "byob" })
              .read(new Uint8Array(untouchedInput))
              .then(() => false, error => error === alreadyErroredReason);
            output.alreadyErrored = [alreadyResult, untouchedInput.byteLength];

            globalThis.__byteStreamCancelErrorResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream cancel/error matrix should initialize");

    for _ in 0..64 {
        vm.eval("0")
            .expect("byte stream cancel/error matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamCancelErrorResult")
        .expect("byte stream cancel/error result should evaluate");
    assert_eq!(
        result,
        r#"{"cancel":[null,true,null,true,0,null,null,"closed"],"staleAfterCancel":"TypeError","error":[true,true,0,null,null],"alreadyErrored":[true,8]}"#
    );
}

#[test]
fn readable_byte_stream_preserves_every_array_buffer_view_brand_and_offset() {
    let mut vm = new_storage_test_vm("https://byte-stream-view-brands.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamViewBrandsResult = null;
          (async () => {
            const cases = [
              ["DataView", buffer => new DataView(buffer, 8, 16), 1],
              ["Int8Array", buffer => new Int8Array(buffer, 8, 16), 1],
              ["Uint8Array", buffer => new Uint8Array(buffer, 8, 16), 1],
              ["Uint8ClampedArray", buffer => new Uint8ClampedArray(buffer, 8, 16), 1],
              ["Int16Array", buffer => new Int16Array(buffer, 8, 8), 2],
              ["Uint16Array", buffer => new Uint16Array(buffer, 8, 8), 2],
              ["Int32Array", buffer => new Int32Array(buffer, 8, 4), 4],
              ["Uint32Array", buffer => new Uint32Array(buffer, 8, 4), 4],
              ["Float16Array", buffer => new Float16Array(buffer, 8, 8), 2],
              ["Float32Array", buffer => new Float32Array(buffer, 8, 4), 4],
              ["Float64Array", buffer => new Float64Array(buffer, 8, 2), 8],
              ["BigInt64Array", buffer => new BigInt64Array(buffer, 8, 2), 8],
              ["BigUint64Array", buffer => new BigUint64Array(buffer, 8, 2), 8]
            ];
            const output = [];
            for (const [name, makeView, elementSize] of cases) {
              let controller;
              const stream = new ReadableStream({
                type: "bytes",
                start(value) { controller = value; }
              });
              const reader = stream.getReader({ mode: "byob" });
              const input = new ArrayBuffer(48);
              const read = reader.read(makeView(input));
              const request = controller.byobRequest;
              for (let index = 0; index < elementSize; index += 1) {
                request.view[index] = index + 1;
              }
              request.respond(elementSize);
              controller.close();
              const result = await read;
              output.push([
                name,
                result.value.constructor.name,
                result.value.byteOffset,
                result.value.byteLength,
                result.value.buffer.byteLength,
                input.byteLength,
                result.done
              ]);
            }
            globalThis.__byteStreamViewBrandsResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream view-brand matrix should initialize");

    for _ in 0..96 {
        vm.eval("0")
            .expect("byte stream view-brand matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamViewBrandsResult")
        .expect("byte stream view-brand matrix result should evaluate");
    assert_eq!(
        result,
        r#"[["DataView","DataView",8,1,48,0,false],["Int8Array","Int8Array",8,1,48,0,false],["Uint8Array","Uint8Array",8,1,48,0,false],["Uint8ClampedArray","Uint8ClampedArray",8,1,48,0,false],["Int16Array","Int16Array",8,2,48,0,false],["Uint16Array","Uint16Array",8,2,48,0,false],["Int32Array","Int32Array",8,4,48,0,false],["Uint32Array","Uint32Array",8,4,48,0,false],["Float16Array","Float16Array",8,2,48,0,false],["Float32Array","Float32Array",8,4,48,0,false],["Float64Array","Float64Array",8,8,48,0,false],["BigInt64Array","BigInt64Array",8,8,48,0,false],["BigUint64Array","BigUint64Array",8,8,48,0,false]]"#
    );
}

#[test]
fn readable_byte_stream_rejects_non_transferable_buffers_and_read_option_failures() {
    let mut vm = new_storage_test_vm("https://byte-stream-invalid-buffers.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamInvalidBuffersResult = null;
          (async () => {
            const output = {};
            let controller;
            const stream = new ReadableStream({
              type: "bytes",
              start(value) { controller = value; }
            });
            const reader = stream.getReader({ mode: "byob" });

            const resizable = new ArrayBuffer(16, { maxByteLength: 32 });
            output.resizableRead = await reader
              .read(new Uint8Array(resizable, 4, 8))
              .then(() => "resolved", error => error.name);
            let enqueueResizable;
            try {
              controller.enqueue(new Uint8Array(resizable, 4, 8));
              enqueueResizable = "resolved";
            } catch (error) {
              enqueueResizable = error.name;
            }
            output.resizable = [
              output.resizableRead,
              enqueueResizable,
              resizable.byteLength,
              controller.byobRequest
            ];

            const memory = new WebAssembly.Memory({ initial: 1 });
            output.wasm = await reader
              .read(new Uint8Array(memory.buffer, 0, 8))
              .then(() => "resolved", error => error.name);

            let getterCount = 0;
            const getterError = new Error("minimum getter");
            output.getter = await reader
              .read(new Uint8Array(8), {
                get min() {
                  getterCount += 1;
                  throw getterError;
                }
              })
              .then(
                () => "resolved",
                error => [error === getterError, error.message, getterCount]
              );
            output.minimums = await Promise.all([
              reader.read(new Uint8Array(8), { min: 0 })
                .then(() => "resolved", error => error.name),
              reader.read(new Uint16Array(2), { min: 3 })
                .then(() => "resolved", error => error.name),
              reader.read({}, {
                get min() { getterCount += 100; return 1; }
              }).then(() => "resolved", error => [error.name, getterCount])
            ]);
            globalThis.__byteStreamInvalidBuffersResult = JSON.stringify(output);
          })();
        })()
        "#,
    )
    .expect("byte stream invalid-buffer matrix should initialize");

    for _ in 0..48 {
        vm.eval("0")
            .expect("byte stream invalid-buffer matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamInvalidBuffersResult")
        .expect("byte stream invalid-buffer matrix result should evaluate");
    assert_eq!(
        result,
        r#"{"resizableRead":"TypeError","resizable":["TypeError","TypeError",16,null],"wasm":"TypeError","getter":[true,"minimum getter",1],"minimums":["TypeError","RangeError",["TypeError",1]]}"#
    );
}

#[test]
fn readable_byte_stream_respond_with_new_view_validation_order_matches_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-new-view-validation.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamNewViewValidationResult = null;
          (async () => {
            const exceptionName = callback => {
              try { callback(); return "resolved"; }
              catch (error) { return error.name; }
            };
            const requestFor = async closed => {
              let controller;
              const stream = new ReadableStream({
                type: "bytes",
                start(value) { controller = value; }
              });
              const reader = stream.getReader({ mode: "byob" });
              reader.read(new Uint8Array([4, 5, 6])).catch(() => {});
              await Promise.resolve();
              if (closed) controller.close();
              return [controller, controller.byobRequest];
            };

            const [, readableDetachedRequest] = await requestFor(false);
            const readableDetached = new Uint8Array([1, 2, 3]);
            readableDetached.buffer.transfer();

            const [, readableZeroRequest] = await requestFor(false);
            const [, readableSubviewRequest] = await requestFor(false);
            const [closedZeroController, closedZeroRequest] = await requestFor(true);
            const [, closedDetachedRequest] = await requestFor(true);
            const closedDetached = new Uint8Array([1, 2, 3]);
            closedDetached.buffer.transfer();

            let movedController;
            const movedStream = new ReadableStream({
              type: "bytes",
              start(value) { movedController = value; }
            });
            const movedReader = movedStream.getReader({ mode: "byob" });
            const movedRead = movedReader.read(new Uint8Array([4, 5, 6]));
            await Promise.resolve();
            const movedRequest = movedController.byobRequest;
            const movedBuffer = movedRequest.view.buffer.transfer();
            const movedView = new Uint8Array(movedBuffer, 0, 1);
            movedView[0] = 42;
            const movedCall = exceptionName(() => movedRequest.respondWithNewView(movedView));
            const movedResult = await movedRead;

            globalThis.__byteStreamNewViewValidationResult = JSON.stringify({
              readableDetached: exceptionName(() =>
                readableDetachedRequest.respondWithNewView(readableDetached)),
              readableZeroBuffer: exceptionName(() =>
                readableZeroRequest.respondWithNewView(new Uint8Array())),
              readableZeroSubview: exceptionName(() =>
                readableSubviewRequest.respondWithNewView(
                  new Uint8Array(readableSubviewRequest.view.buffer, 0, 0)
                )),
              closedZeroBuffer: exceptionName(() =>
                closedZeroRequest.respondWithNewView(new Uint8Array())),
              closedDetached: exceptionName(() =>
                closedDetachedRequest.respondWithNewView(closedDetached)),
              movedOriginal: [
                movedCall,
                Array.from(movedResult.value),
                movedResult.value.buffer.byteLength,
                movedRequest.view
              ],
              closedStillHasRequest: closedZeroController.byobRequest === closedZeroRequest
            });
          })();
        })()
        "#,
    )
    .expect("respondWithNewView validation matrix should initialize");

    for _ in 0..48 {
        vm.eval("0")
            .expect("respondWithNewView validation matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamNewViewValidationResult")
        .expect("respondWithNewView validation result should evaluate");
    assert_eq!(
        result,
        r#"{"readableDetached":"TypeError","readableZeroBuffer":"TypeError","readableZeroSubview":"TypeError","closedZeroBuffer":"RangeError","closedDetached":"TypeError","movedOriginal":["resolved",[42],3,null],"closedStillHasRequest":true}"#
    );
}

#[test]
fn readable_byte_stream_webidl_dictionary_conversion_matches_chromium() {
    let mut vm = new_storage_test_vm("https://byte-stream-webidl-conversion.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__byteStreamWebIdlResult = null;
          (async () => {
            const primitiveResults = [];
            for (const options of [1, "x", true, Symbol("options")]) {
              const stream = new ReadableStream({
                type: "bytes",
                start(controller) {
                  controller.enqueue(new Uint8Array([9]));
                  controller.close();
                }
              });
              const result = await stream
                .getReader({ mode: "byob" })
                .read(new Uint8Array(2), options);
              primitiveResults.push([Array.from(result.value), result.done]);
            }

            const inheritedOptions = Object.create({ min: 2 });
            const inheritedStream = new ReadableStream({
              type: "bytes",
              start(controller) {
                controller.enqueue(new Uint8Array([1, 2]));
                controller.close();
              }
            });
            const inherited = await inheritedStream
              .getReader({ mode: "byob" })
              .read(new Uint8Array(4), inheritedOptions);

            const constructorResult = callback => {
              try { callback(); return "constructed"; }
              catch (error) { return error.name; }
            };
            globalThis.__byteStreamWebIdlResult = JSON.stringify({
              primitiveResults,
              inherited: [Array.from(inherited.value), inherited.done],
              autoAllocateZero: [
                constructorResult(() => new ReadableStream({ autoAllocateChunkSize: 0 })),
                constructorResult(() => new ReadableStream({
                  type: "bytes",
                  autoAllocateChunkSize: 0
                }))
              ]
            });
          })();
        })()
        "#,
    )
    .expect("byte stream Web IDL conversion matrix should initialize");

    for _ in 0..48 {
        vm.eval("0")
            .expect("byte stream Web IDL conversion matrix should drain");
    }
    let result = vm
        .eval("globalThis.__byteStreamWebIdlResult")
        .expect("byte stream Web IDL conversion result should evaluate");
    assert_eq!(
        result,
        r#"{"primitiveResults":[[[9],false],[[9],false],[[9],false],[[9],false]],"inherited":[[1,2],false],"autoAllocateZero":["constructed","TypeError"]}"#
    );
}

#[test]
fn blob_and_fetch_body_streams_expose_the_readable_byte_stream_byob_contract() {
    let mut vm = new_storage_test_vm("https://body-byte-streams.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__bodyByteStreamsResult = null;
          (async () => {
            const blobReader = new Blob([new Uint8Array([1, 2, 3, 4])])
              .stream()
              .getReader({ mode: "byob" });
            const blob = await blobReader.read(new Uint8Array(8));
            const blobDone = await blobReader.read(new Uint8Array(2));

            const response = new Response(new Uint8Array([5, 6, 7]));
            const responseReader = response.body.getReader({ mode: "byob" });
            const body = await responseReader.read(new Uint8Array(8));
            const bodyDone = await responseReader.read(new Uint8Array(2));

            globalThis.__bodyByteStreamsResult = JSON.stringify({
              blob: [Array.from(blob.value), blob.done, blobDone.value.byteLength, blobDone.done],
              response: [Array.from(body.value), body.done, bodyDone.value.byteLength, bodyDone.done],
              bodyUsed: response.bodyUsed
            });
          })();
        })()
        "#,
    )
    .expect("body byte streams should initialize");

    for _ in 0..48 {
        vm.eval("0").expect("body byte streams should drain");
    }
    let result = vm
        .eval("globalThis.__bodyByteStreamsResult")
        .expect("body byte stream result should evaluate");
    assert_eq!(
        result,
        r#"{"blob":[[1,2,3,4],false,0,true],"response":[[5,6,7],false,0,true],"bodyUsed":true}"#
    );
}

#[test]
fn plain_data_surfaces_preserve_expected_shapes() {
    let mut vm = new_storage_test_vm("https://plain-data-shapes.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const target = new Uint8Array(8);
  const encoded = new TextEncoder().encodeInto('hé', target);
  const params = new URLSearchParams('a=1&a=2&b=3');
  const headers = new Headers([['x-a', '1'], ['x-b', '2']]);
  return JSON.stringify({
    encodeInto: [Object.keys(encoded).join(','), encoded.read, encoded.written, Array.from(target.slice(0, 3)).join(',')].join('|'),
    params: [params.getAll('a').join(','), Array.from(params.entries()).map((pair) => pair.join('=')).join(',')].join('|'),
    headers: [
      Array.from(headers.keys()).join(','),
      Array.from(headers.values()).join(','),
      Array.from(headers.entries()).map((pair) => pair.join('=')).join(',')
    ].join('|'),
    perfTiming: Object.keys(performance.timing).join(','),
    perfNavigation: [Object.keys(performance.navigation).join(','), performance.navigation.type, performance.navigation.redirectCount].join('|'),
    perfSupported: PerformanceObserver.supportedEntryTypes.join(',')
  });
})()
"#,
        )
        .expect("serde_v8 plain data shape probe should evaluate");

    assert_eq!(
        result,
        r#"{"encodeInto":"read,written|2|3|104,195,169","params":"1,2|a=1,a=2,b=3","headers":"x-a,x-b|1,2|x-a=1,x-b=2","perfTiming":"navigationStart,unloadEventStart,unloadEventEnd,redirectStart,redirectEnd,fetchStart,domainLookupStart,domainLookupEnd,connectStart,connectEnd,secureConnectionStart,requestStart,responseStart,responseEnd,domLoading,domInteractive,domContentLoadedEventStart,domContentLoadedEventEnd,domComplete,loadEventStart,loadEventEnd","perfNavigation":"|0|0","perfSupported":"mark,measure,navigation,resource"}"#
    );
}

#[test]
fn performance_observer_callbacks_apply_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://performance-observer-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const values = [];
  values.push(probe(() => new PerformanceObserver()));
  values.push(probe(() => new PerformanceObserver({})));

  const observer = new PerformanceObserver(() => {});
  values.push(observer instanceof PerformanceObserver);
  values.push(probe(() => observer.observe()));
  values.push(probe(() => observer.observe(null)));
  values.push(probe(() => observer.observe({})));
  values.push(probe(() => observer.observe({ type: Symbol('entry-type') })));
  values.push(probe(() => observer.observe({ entryTypes: Symbol('entry-types') })));
  values.push(probe(() => observer.observe({ entryTypes: [Symbol('entry-type')] })));

  let typeCalls = 0;
  let bufferedValueOfCalls = 0;
  values.push(probe(() => {
    observer.observe({
      type: { toString() { typeCalls += 1; return 'mark'; } },
      buffered: { valueOf() { bufferedValueOfCalls += 1; return false; } },
    });
    return `${typeCalls}:${bufferedValueOfCalls}`;
  }));

  values.push(probe(() => observer.observe({ entryTypes: ['measure'] })));

  const entryTypesObserver = new PerformanceObserver(() => {});
  let entryTypeCalls = 0;
  values.push(probe(() => {
    entryTypesObserver.observe({
      entryTypes: ['mark', { toString() { entryTypeCalls += 1; return 'mark'; } }, 'measure', 'mark'],
    });
    return entryTypeCalls;
  }));
  performance.mark('po-webidl');
  values.push(entryTypesObserver.takeRecords().map((entry) => `${entry.entryType}:${entry.name}`).join(','));
  return values.join('|');
})()
"#,
        )
        .expect("PerformanceObserver WebIDL conversion probe should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|true|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|1:0|throw:InvalidModificationError|1|mark:po-webidl"
    );
}

#[test]
fn webidl_sequence_conversion_uses_iterator_without_mutable_array_from() {
    let mut vm = new_storage_test_vm("https://webidl-sequence-iterator.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}:${error && error.message}`;
    }
  };
  const results = [];

  const originalArrayFrom = Array.from;
  let arrayFromCalls = 0;
  Array.from = () => {
    arrayFromCalls += 1;
    throw new Error('array-from-polluted');
  };
  try {
    const observer = new IntersectionObserver(() => {}, {
      threshold: [0.25, 0.75],
    });
    results.push(`arrayFrom:${arrayFromCalls}:${observer.thresholds.join(',')}`);
  } finally {
    Array.from = originalArrayFrom;
  }

  const originalArrayIterator = Array.prototype[Symbol.iterator];
  let arrayIteratorCalls = 0;
  Array.prototype[Symbol.iterator] = function() {
    arrayIteratorCalls += 1;
    return {
      next() {
        throw new RangeError('array-iterator-used');
      },
    };
  };
  try {
    results.push(`arrayIterator:${arrayIteratorCalls}:${
      probe(() => new IntersectionObserver(() => {}, { threshold: [0.5] }))
    }:${arrayIteratorCalls}`);
  } finally {
    Array.prototype[Symbol.iterator] = originalArrayIterator;
  }

  const order = [];
  const entryTypes = {
    [Symbol.iterator]() {
      order.push('iterator');
      let index = 0;
      return {
        next() {
          order.push(`next:${index}`);
          if (index === 0) {
            index += 1;
            return {
              get done() {
                order.push('done:0');
                return false;
              },
              get value() {
                order.push('value:0');
                return {
                  toString() {
                    order.push('toString:0');
                    return 'mark';
                  },
                };
              },
            };
          }
          if (index === 1) {
            index += 1;
            return {
              get done() {
                order.push('done:1');
                return false;
              },
              get value() {
                order.push('value:1');
                return {
                  toString() {
                    order.push('toString:1');
                    return 'measure';
                  },
                };
              },
            };
          }
          return {
            get done() {
              order.push('done:2');
              return true;
            },
          };
        },
      };
    },
  };
  const orderedObserver = new PerformanceObserver(() => {});
  orderedObserver.observe({ entryTypes });
  results.push(`order:${order.join(',')}`);

  const throwingNext = {
    [Symbol.iterator]() {
      return {
        next() {
          throw new RangeError('next-boom');
        },
      };
    },
  };
  results.push(`nextThrow:${
    probe(() => new PerformanceObserver(() => {}).observe({ entryTypes: throwingNext }))
  }`);

  const throwingDone = {
    [Symbol.iterator]() {
      return {
        next() {
          return {
            get done() {
              throw new TypeError('done-boom');
            },
          };
        },
      };
    },
  };
  results.push(`doneThrow:${
    probe(() => new PerformanceObserver(() => {}).observe({ entryTypes: throwingDone }))
  }`);

  const throwingValue = {
    [Symbol.iterator]() {
      return {
        next() {
          return {
            done: false,
            get value() {
              throw new SyntaxError('value-boom');
            },
          };
        },
      };
    },
  };
  results.push(`valueThrow:${
    probe(() => new PerformanceObserver(() => {}).observe({ entryTypes: throwingValue }))
  }`);

  let throwingElementIteratorClosed = false;
  const throwingElementConversion = {
    [Symbol.iterator]() {
      let finished = false;
      return {
        next() {
          if (finished) {
            return { done: true };
          }
          finished = true;
          return {
            done: false,
            value: {
              toString() {
                throw new URIError('string-boom');
              },
            },
          };
        },
        return() {
          throwingElementIteratorClosed = true;
          throw new EvalError('close-must-not-replace-element-error');
        },
      };
    },
  };
  results.push(`elementThrow:${
    probe(() => new PerformanceObserver(() => {}).observe({
      entryTypes: throwingElementConversion,
    }))
  }:${throwingElementIteratorClosed}`);

  return results.join('|');
})()
"#,
        )
        .expect("WebIDL sequence iterator conversion probe should evaluate");

    assert_eq!(
        result,
        "arrayFrom:0:0.25,0.75|arrayIterator:0:throw:RangeError:array-iterator-used:1|order:iterator,next:0,done:0,value:0,toString:0,next:1,done:1,value:1,toString:1,next:2,done:2|nextThrow:throw:RangeError:next-boom|doneThrow:throw:TypeError:done-boom|valueThrow:throw:SyntaxError:value-boom|elementThrow:throw:URIError:string-boom:true"
    );
}

#[test]
fn url_search_params_sequence_discrimination_reads_iterator_once() {
    let mut vm = new_storage_test_vm("https://url-search-params-sequence.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function* pairs() {
    yield ['first', 'one'];
    yield ['second', 'two'];
  }
  let iteratorGets = 0;
  const init = {};
  Object.defineProperty(init, Symbol.iterator, {
    get() {
      iteratorGets += 1;
      return pairs;
    }
  });
  const originalArrayFrom = Array.from;
  Array.from = () => { throw new Error('polluted Array.from'); };
  let params;
  try {
    params = new URLSearchParams(init);
  } finally {
    Array.from = originalArrayFrom;
  }
  let stringArrayError = 'none';
  try {
    new URLSearchParams(['key', 'value']);
  } catch (error) {
    stringArrayError = error && error.name;
  }

  let outerClosed = false;
  let invalidPairError = 'none';
  try {
    new URLSearchParams({
      [Symbol.iterator]() {
        return {
          next() { return { done: false, value: ['short'] }; },
          return() {
            outerClosed = true;
            throw new SyntaxError('close error');
          }
        };
      }
    });
  } catch (error) {
    invalidPairError = error && error.name;
  }

  let innerClosed = false;
  let innerError = 'none';
  const throwingPair = {
    [Symbol.iterator]() {
      let index = 0;
      return {
        next() {
          index += 1;
          if (index === 1) return { done: false, value: 'key' };
          return {
            done: false,
            value: { toString() { throw new RangeError('value error'); } }
          };
        },
        return() {
          innerClosed = true;
          throw new SyntaxError('close error');
        }
      };
    }
  };
  try {
    new URLSearchParams([throwingPair]);
  } catch (error) {
    innerError = error && error.name;
  }
  return [
    iteratorGets,
    params.get('first'),
    params.get('second'),
    stringArrayError,
    `${invalidPairError}:${outerClosed}`,
    `${innerError}:${innerClosed}`
  ].join('|');
})()
"#,
        )
        .expect("URLSearchParams sequence discrimination probe should evaluate");

    assert_eq!(result, "1|one|two|TypeError|TypeError:true|RangeError:true");
}

#[test]
fn performance_observer_declared_slots_ignore_prototype_spoofing() {
    let mut vm = new_storage_test_vm("https://performance-observer-declared-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };

  const observer = new PerformanceObserver(() => {});
  observer.observe({ entryTypes: ['mark'] });
  performance.mark('declared-observer');
  const records = observer.takeRecords();
  const ownSlots = Object.getOwnPropertyNames(observer)
    .filter(name => name.startsWith('__moliPerformanceObserver'))
    .sort();

  PerformanceObserver.prototype.__moliPerformanceObserverCallbackId = 1;
  PerformanceObserver.prototype.__moliPerformanceObserverPending = ['spoofed'];
  PerformanceObserver.prototype.__moliPerformanceObserverType = 'mark';
  PerformanceObserver.prototype.__moliPerformanceObserverEntryTypes = ['mark'];
  PerformanceObserver.prototype.__moliPerformanceObserverActive = true;
  PerformanceObserver.prototype.__moliPerformanceObserverScheduled = true;

  const fake = Object.create(PerformanceObserver.prototype);
  const fakeRecords = PerformanceObserver.prototype.takeRecords.call(fake);
  const fakeObserve = probe(() => PerformanceObserver.prototype.observe.call(fake, {
    entryTypes: ['measure'],
  }));
  return JSON.stringify({
    real: [
      records.length,
      records[0] && records[0].entryType,
      records[0] && records[0].name,
      observer.takeRecords().length
    ].join('|'),
    fake: [
      fakeRecords.length,
      fakeRecords[0],
      fakeObserve,
      fake.takeRecords().length
    ].map(value => value === undefined ? 'undefined' : String(value)).join('|'),
    ownSlots
  });
})()
"#,
        )
        .expect("PerformanceObserver declared slots should ignore prototype spoofing");

    assert_eq!(
        result,
        r#"{"real":"1|mark|declared-observer|0","fake":"0|undefined|undefined|0","ownSlots":[]}"#
    );
}

#[test]
fn global_runtime_queues_hide_slots_and_ignore_spoofing() {
    let mut vm = new_storage_test_vm("https://global-runtime-queues.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const queueNames = [
    '__moliFileReaderQueue',
    '__moliResizeObserverQueue',
    '__moliPerformanceObserverQueue'
  ];
  const reflectedQueues = () => Object.getOwnPropertyNames(globalThis)
    .filter(name => queueNames.includes(name))
    .sort();
  const before = reflectedQueues();
  for (const name of queueNames) {
    globalThis[name] = false;
  }

  const performanceObserver = new PerformanceObserver(() => {});
  performanceObserver.observe({ entryTypes: ['mark'] });
  performance.mark('global-queue-mark');
  const performanceRecords = performanceObserver.takeRecords()
    .map(entry => `${entry.entryType}:${entry.name}`)
    .join(',');

  const target = document.createElement('div');
  const root = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || root.appendChild(document.createElement('body'));
  body.appendChild(target);
  const resizeObserver = new ResizeObserver(() => {});
  resizeObserver.observe(target);
  const resizeRecords = resizeObserver.takeRecords();

  const reader = new FileReader();
  reader.readAsText(new Blob(['abc']));

  return JSON.stringify({
    before,
    afterSpoof: reflectedQueues(),
    performanceRecords,
    resizeRecords: [
      resizeRecords.length,
      resizeRecords[0] && resizeRecords[0].target === target
    ].join('|'),
    fileReaderStarted: [
      reader.readyState === FileReader.LOADING,
      reader.result === null,
      Object.getOwnPropertyNames(reader)
        .filter(name => name.startsWith('__moliFileReader'))
        .join(',')
    ].join('|')
  });
})()
"#,
        )
        .expect("global runtime queue slots should ignore public spoofing");

    assert_eq!(
        result,
        r#"{"before":[],"afterSpoof":["__moliFileReaderQueue","__moliPerformanceObserverQueue","__moliResizeObserverQueue"],"performanceRecords":"mark:global-queue-mark","resizeRecords":"1|true","fileReaderStarted":"true|true|"}"#
    );
}

#[test]
fn intersection_observer_servo_aligned_options_surface() {
    let mut vm = new_storage_test_vm("https://intersection-observer-options-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const defaultObserver = new IntersectionObserver(() => {});
  const observer = new IntersectionObserver(() => {}, {
    rootMargin: '1px 2% 3px',
    scrollMargin: '4px 5% 6px 7%',
    threshold: new Set([0.75, 0.25, 0.25]),
    delay: 1,
    trackVisibility: true,
  });
  const emptyThresholdObserver = new IntersectionObserver(() => {}, { threshold: [] });
  let rootMarginCalls = 0;
  let delayCalls = 0;
  const converted = new IntersectionObserver(() => {}, {
    rootMargin: { toString() { rootMarginCalls += 1; return '8px'; } },
    delay: { valueOf() { delayCalls += 1; return 5; } },
    trackVisibility: true,
  });
  const text = document.createTextNode('text');
  const element = document.createElement('div');
  const targetObserver = new IntersectionObserver(() => {});
  const scrollMarginDescriptor =
    Object.getOwnPropertyDescriptor(IntersectionObserver.prototype, 'scrollMargin');
  const delayDescriptor =
    Object.getOwnPropertyDescriptor(IntersectionObserver.prototype, 'delay');
  const trackVisibilityDescriptor =
    Object.getOwnPropertyDescriptor(IntersectionObserver.prototype, 'trackVisibility');
  return [
    defaultObserver.scrollMargin,
    defaultObserver.delay,
    defaultObserver.trackVisibility,
    observer.rootMargin,
    observer.scrollMargin,
    JSON.stringify(observer.thresholds),
    observer.delay,
    observer.trackVisibility,
    JSON.stringify(emptyThresholdObserver.thresholds),
    converted.rootMargin,
    converted.delay,
    `${rootMarginCalls}:${delayCalls}`,
    typeof scrollMarginDescriptor.get,
    String(scrollMarginDescriptor.enumerable),
    typeof delayDescriptor.get,
    typeof trackVisibilityDescriptor.get,
    probe(() => new IntersectionObserver(() => {}, 1)),
    probe(() => new IntersectionObserver(() => {}, { rootMargin: null })),
    probe(() => new IntersectionObserver(() => {}, { scrollMargin: '1em' })),
    probe(() => new IntersectionObserver(() => {}, { root: text })),
    probe(() => new IntersectionObserver(() => {}, { root: document }) instanceof IntersectionObserver),
    probe(() => targetObserver.observe(text)),
    probe(() => targetObserver.observe(document)),
    probe(() => targetObserver.observe(element)),
    probe(() => targetObserver.unobserve(text)),
    probe(() => targetObserver.unobserve()),
  ].join('|');
})()
"#,
        )
        .expect("IntersectionObserver Servo-aligned option surface should evaluate");

    assert_eq!(
        result,
        "0px 0px 0px 0px|0|false|1px 2% 3px 2%|4px 5% 6px 7%|[0.25,0.75]|100|true|[0]|8px 8px 8px 8px|100|1:1|function|true|function|function|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|true|throw:TypeError|throw:TypeError|undefined|throw:TypeError|throw:TypeError"
    );
}

#[test]
fn resize_observer_callbacks_apply_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://resize-observer-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const values = [];
  values.push(probe(() => new ResizeObserver()));
  values.push(probe(() => new ResizeObserver({})));

  const observer = new ResizeObserver(() => {});
  const element = document.createElement('div');
  const text = document.createTextNode('text');
  body.appendChild(element);
  values.push(observer instanceof ResizeObserver);
  values.push(probe(() => observer.observe()));
  values.push(probe(() => observer.observe(text)));
  values.push(probe(() => observer.observe(element, 1)));
  values.push(probe(() => observer.observe(element, { box: 'invalid-box' })));

  let boxCalls = 0;
  values.push(probe(() => {
    observer.observe(element, {
      box: { toString() { boxCalls += 1; return 'border-box'; } },
    });
    return `${boxCalls}:${observer.takeRecords().length}`;
  }));
  values.push(probe(() => observer.unobserve()));
  values.push(probe(() => observer.unobserve(text)));
  values.push(probe(() => {
    observer.observe(element, { box: 'content-box' });
    observer.unobserve(element);
    return observer.takeRecords().length;
  }));
  return values.join('|');
})()
"#,
        )
        .expect("ResizeObserver WebIDL conversion probe should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|true|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|1:1|throw:TypeError|throw:TypeError|0"
    );
}

#[test]
fn resize_observer_entries_expose_box_size_arrays() {
    let mut vm = new_storage_test_vm("https://resize-observer-box-size.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  target.style.cssText = 'width: 41px; height: 23px';
  body.appendChild(target);
  const observer = new ResizeObserver(() => {});
  observer.observe(target, { box: 'border-box' });
  const first = observer.takeRecords()[0];
  observer.observe(target, { box: 'device-pixel-content-box' });
  const second = observer.takeRecords()[0];
  return [
    first.target === target,
    Array.isArray(first.contentBoxSize),
    Array.isArray(first.borderBoxSize),
    Array.isArray(first.devicePixelContentBoxSize),
    first.contentBoxSize.length,
    first.contentBoxSize[0].inlineSize,
    first.contentBoxSize[0].blockSize,
    first.borderBoxSize[0].inlineSize,
    first.borderBoxSize[0].blockSize,
    second.devicePixelContentBoxSize[0].inlineSize,
    second.devicePixelContentBoxSize[0].blockSize,
    Object.keys(first.contentBoxSize[0]).join(',')
  ].join('|');
})()
"#,
        )
        .expect("ResizeObserver box-size entries should evaluate");

    assert_eq!(
        result,
        "true|true|true|true|1|41|23|41|23|41|23|inlineSize,blockSize"
    );
}

#[test]
fn resize_observer_callback_runs_after_microtask_checkpoint() {
    let mut vm = new_storage_test_vm("https://resize-observer-delivery.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  globalThis.__resizeObserverLog = [];
  const target = document.createElement('div');
  target.style.cssText = 'width: 10px; height: 20px';
  body.appendChild(target);
  const observer = new ResizeObserver((entries, instance) => {
    globalThis.__resizeObserverLog.push(`${entries.length}:${instance === observer}:${entries[0].target === target}`);
  });
  observer.observe(target);
  return 'scheduled';
})()
"#,
        )
        .expect("ResizeObserver delivery setup should evaluate");

    assert_eq!(result, "scheduled");
    let delivered = vm
        .eval("globalThis.__resizeObserverLog.join('|')")
        .expect("ResizeObserver delivery log should evaluate");
    assert_eq!(delivered, "1:true:true");
}

#[test]
fn resize_observer_declared_slots_ignore_prototype_spoofing() {
    let mut vm = new_storage_test_vm("https://resize-observer-declared-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const stringify = value => value === undefined ? 'undefined' : String(value);
  const target = document.createElement('div');
  target.style.cssText = 'width: 10px; height: 20px';
  body.appendChild(target);
  const observer = new ResizeObserver(() => {});
  observer.observe(target, { box: 'border-box' });
  const records = observer.takeRecords();
  const ownSlots = Object.getOwnPropertyNames(observer)
    .filter(name => name.startsWith('__moliResizeObserver'))
    .sort();

  ResizeObserver.prototype.__moliResizeObserverCallbackId = 1;
  ResizeObserver.prototype.__moliResizeObserverTargets = [{ target }];
  ResizeObserver.prototype.__moliResizeObserverPendingTargets = [{ target }];
  ResizeObserver.prototype.__moliResizeObserverScheduled = true;

  const fake = Object.create(ResizeObserver.prototype);
  const fakeRecords = ResizeObserver.prototype.takeRecords.call(fake);
  const fakeObserve = ResizeObserver.prototype.observe.call(fake, target);
  const fakeAfterObserve = fake.takeRecords();
  return JSON.stringify({
    real: [
      records.length,
      records[0] && records[0].target === target,
      records[0] && records[0].borderBoxSize.length,
      observer.takeRecords().length
    ].join('|'),
    fake: [
      fakeRecords.length,
      fakeRecords[0],
      fakeObserve,
      fakeAfterObserve.length,
      fakeAfterObserve[0]
    ].map(stringify).join('|'),
    ownSlots
  });
})()
"#,
        )
        .expect("ResizeObserver declared slots should ignore prototype spoofing");

    assert_eq!(
        result,
        r#"{"real":"1|true|1|0","fake":"0|undefined|undefined|0|undefined","ownSlots":[]}"#
    );
}

#[test]
fn resize_observer_observed_records_ignore_public_spoofing() {
    let mut vm = new_storage_test_vm("https://resize-observer-record-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const html = document.documentElement || document.appendChild(document.createElement('html'));
  const body = document.body || html.appendChild(document.createElement('body'));
  const target = document.createElement('div');
  const other = document.createElement('section');
  target.style.cssText = 'width: 10px; height: 20px';
  other.style.cssText = 'width: 30px; height: 40px';
  body.append(target, other);

  Object.prototype.__moliResizeObserverRecordTarget = other;
  Object.prototype.__moliResizeObserverRecordBox = 'device-pixel-content-box';
  target.__moliResizeObserverRecordTarget = other;
  target.__moliResizeObserverRecordBox = 'border-box';

  const observer = new ResizeObserver(() => {});
  observer.observe(target, { box: 'content-box' });
  observer.observe(target, { box: 'border-box' });
  const first = observer.takeRecords();

  observer.observe(target);
  observer.observe(other);
  observer.unobserve(target);
  const second = observer.takeRecords();

  return JSON.stringify({
    first: [
      first.length,
      first[0] && first[0].target === target,
      first[0] && first[0].contentRect.width,
      first[0] && Object.getOwnPropertyNames(first[0]).some(name => name.startsWith('__moliResizeObserverRecord'))
    ].join('|'),
    second: [
      second.length,
      second[0] && second[0].target === other,
      second[0] && second[0].contentRect.width
    ].join('|'),
    targetSpoofVisible: target.__moliResizeObserverRecordTarget === other
  });
})()
"#,
        )
        .expect("ResizeObserver observed records should ignore public spoofing");

    assert_eq!(
        result,
        r#"{"first":"1|true|10|false","second":"1|true|30","targetSpoofVisible":true}"#
    );
}

#[test]
fn vtt_cue_constructor_applies_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://vtt-cue-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const values = [];
  values.push(probe(() => new VTTCue()));
  values.push(probe(() => new VTTCue(0)));
  values.push(probe(() => new VTTCue(0, 1)));
  values.push(probe(() => new VTTCue(Symbol('start'), 1, 'text')));
  values.push(probe(() => new VTTCue(0, Infinity, 'text')));
  values.push(probe(() => new VTTCue(0, 1, Symbol('text'))));

  let startCalls = 0;
  let textCalls = 0;
  const cue = new VTTCue(
    { valueOf() { startCalls += 1; return 1.25; } },
    '2.5',
    { toString() { textCalls += 1; return 'caption'; } },
  );
  values.push([cue.startTime, cue.endTime, cue.text, cue instanceof VTTCue, cue instanceof TextTrackCue, startCalls, textCalls].join(','));
  return values.join('|');
})()
"#,
        )
        .expect("VTTCue WebIDL conversion probe should evaluate");

    assert_eq!(
        result,
        "throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|1.25,2.5,caption,true,true,1,1"
    );
}

#[test]
fn headers_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://headers-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const headers = new Headers();
  headers.append('X-A', '1');
  headers.append('X-A', '2');
  headers.set('X-B', '3');
  headers.set('X-Byte', '\u00ff');
  const seen = [];
  const thisArg = { marker: 'ctx' };
  headers.forEach(function(value, name, owner) {
    const displayValue = name === 'x-byte' ? value.charCodeAt(0) : value;
    seen.push([this.marker, name, displayValue, owner === headers].join(':'));
  }, thisArg);
  let invalidValue = 'missing';
  try {
    headers.set('X-Bad', '\u0100');
  } catch (error) {
    invalidValue = error && error.name;
  }
  const beforeDelete = [
    headers.get('x-a'),
    headers.has('x-b'),
    headers.get('x-byte').charCodeAt(0),
    invalidValue,
    seen.join(',')
  ].join('|');
  headers.delete('x-b');
  return beforeDelete + '|' + headers.has('x-b');
})()
"#,
        )
        .expect("Headers methods should parse WebIDL arguments");

    assert_eq!(
        result,
        "1, 2|true|255|TypeError|ctx:x-a:1, 2:true,ctx:x-b:3:true,ctx:x-byte:255:true|false"
    );
}

#[test]
fn headers_for_each_uses_webidl_callback_function_semantics() {
    let mut vm = new_storage_test_vm("https://headers-callback-function.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const headers = new Headers([
    ['X-A', '1'],
    ['X-B', '2'],
    ['X-C', '3'],
  ]);
  const marker = { kind: 'abrupt' };
  const seen = [];
  const thisArg = { label: 'receiver' };
  const callback = new Proxy(function(value, name, owner) {
    seen.push(`${this.label}:${name}:${value}:${owner === headers}`);
    if (name === 'x-b') {
      throw marker;
    }
  }, {
    apply(target, receiver, arguments) {
      seen.push('proxy-apply');
      return Reflect.apply(target, receiver, arguments);
    }
  });
  let thrownIdentity = false;
  try {
    headers.forEach(callback, thisArg);
  } catch (error) {
    thrownIdentity = error === marker;
  }
  let nonCallable = 'accepted';
  try {
    headers.forEach({});
  } catch (error) {
    nonCallable = error?.name;
  }
  return JSON.stringify({ seen, thrownIdentity, nonCallable });
})()
"#,
        )
        .expect("Headers.forEach callback-function probe should evaluate");

    assert_eq!(
        result,
        r#"{"seen":["proxy-apply","receiver:x-a:1:true","proxy-apply","receiver:x-b:2:true"],"thrownIdentity":true,"nonCallable":"TypeError"}"#
    );
}

#[tokio::test]
async fn headers_for_each_uses_callback_relevant_realm() {
    let mut vm = new_storage_test_vm("https://headers-callback-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__headersCallbackFrame = frame;
})()
"#,
    )
    .expect("Headers.forEach callback realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "Headers.forEach callback realm setup",
    )
    .await;
    let _ = materialize_single_child_default_realm_for_test(
        &mut vm,
        "Headers.forEach callback realm setup",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const child = __headersCallbackFrame.contentWindow;
  const headers = new Headers([['X-Realm', 'ok']]);
  child.__headersOwner = headers;
  child.__headersSeen = [];
  child.__headersRealmMarker = 'child';
  const callback = child.Function(
    'value',
    'name',
    'owner',
    `globalThis.__headersSeen.push([
      globalThis.__headersRealmMarker,
      this.receiverMarker,
      name,
      value,
      owner === globalThis.__headersOwner
    ].join(':'))`
  );
  headers.forEach(callback, { receiverMarker: 'parent-this' });
  return JSON.stringify({
    callbackRealm: Object.getPrototypeOf(callback) === child.Function.prototype,
    seen: child.__headersSeen,
  });
})()
"#,
        )
        .expect("cross-Realm Headers.forEach callback should evaluate");

    assert_eq!(
        result,
        r#"{"callbackRealm":true,"seen":["child:parent-this:x-realm:ok:true"]}"#
    );
}

#[test]
fn form_data_and_url_search_params_for_each_use_live_webidl_callback_semantics() {
    let mut vm = new_storage_test_vm("https://pair-iterable-callback-function.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const makeFormData = () => {
    const collection = new FormData();
    collection.append('a', '1');
    collection.append('b', '2');
    collection.append('c', '3');
    return collection;
  };
  const makeSearchParams = () => new URLSearchParams('a=1&b=2&c=3');

  const probe = makeCollection => {
    const collection = makeCollection();
    const receiver = { label: 'receiver' };
    const seen = [];
    let applyCount = 0;
    const callback = new Proxy(function(value, key, owner) {
      seen.push([
        this.label,
        key,
        value,
        owner === collection,
        arguments.length
      ].join(':'));
      if (key === 'a') {
        owner.delete('b');
        owner.append('d', '4');
      }
    }, {
      apply(target, thisArg, args) {
        applyCount += 1;
        return Reflect.apply(target, thisArg, args);
      }
    });
    collection.forEach(callback, receiver);

    let omittedThisIsUndefined = false;
    makeCollection().forEach(function() {
      'use strict';
      omittedThisIsUndefined = this === undefined;
    });

    const marker = {};
    let abruptCount = 0;
    let abruptIdentity = false;
    try {
      makeCollection().forEach(() => {
        abruptCount += 1;
        throw marker;
      });
    } catch (error) {
      abruptIdentity = error === marker;
    }

    const revoked = Proxy.revocable(function() {}, {});
    revoked.revoke();
    let revokedError = '';
    try {
      makeCollection().forEach(revoked.proxy);
    } catch (error) {
      revokedError = error && error.name;
    }

    return {
      seen,
      applyCount,
      omittedThisIsUndefined,
      abruptCount,
      abruptIdentity,
      revokedError
    };
  };

  return JSON.stringify({
    formData: probe(makeFormData),
    searchParams: probe(makeSearchParams)
  });
})()
"#,
        )
        .expect("pair-iterable callback-function probe should evaluate");

    assert_eq!(
        result,
        r#"{"formData":{"seen":["receiver:a:1:true:3","receiver:c:3:true:3","receiver:d:4:true:3"],"applyCount":3,"omittedThisIsUndefined":true,"abruptCount":1,"abruptIdentity":true,"revokedError":"TypeError"},"searchParams":{"seen":["receiver:a:1:true:3","receiver:c:3:true:3","receiver:d:4:true:3"],"applyCount":3,"omittedThisIsUndefined":true,"abruptCount":1,"abruptIdentity":true,"revokedError":"TypeError"}}"#
    );
}

#[tokio::test]
async fn form_data_and_url_search_params_for_each_use_callback_relevant_realm() {
    let mut vm = new_storage_test_vm("https://pair-iterable-callback-realm.test/");

    vm.eval(
        r#"
(() => {
  const root = document.documentElement || document.appendChild(document.createElement("html"));
  const body = document.body || root.appendChild(document.createElement("body"));
  const frame = document.createElement("iframe");
  body.appendChild(frame);
  globalThis.__pairIterableCallbackFrame = frame;
})()
"#,
    )
    .expect("pair-iterable callback realm setup should evaluate");
    assert_initial_about_blank_child_completed_synchronously_for_test(
        &mut vm,
        "pair-iterable callback realm setup",
    )
    .await;
    let _ = materialize_single_child_default_realm_for_test(
        &mut vm,
        "pair-iterable callback realm setup",
    );

    let result = vm
        .eval(
            r#"
(() => {
  const child = __pairIterableCallbackFrame.contentWindow;
  const formData = new FormData();
  formData.append('form', 'value');
  const searchParams = new URLSearchParams('search=value');
  child.__pairIterableOwners = { formData, searchParams };
  child.__pairIterableSeen = [];
  child.__pairIterableRealmMarker = 'child';

  const makeCallback = child.Function(
    'kind',
    `return function(value, key, owner) {
      globalThis.__pairIterableSeen.push([
        globalThis.__pairIterableRealmMarker,
        this.receiverMarker,
        kind,
        key,
        value,
        owner === globalThis.__pairIterableOwners[kind]
      ].join(':'));
    }`
  );
  const formCallback = makeCallback('formData');
  const searchCallback = makeCallback('searchParams');
  formData.forEach(formCallback, { receiverMarker: 'parent-this' });
  searchParams.forEach(searchCallback, { receiverMarker: 'parent-this' });

  return JSON.stringify({
    formCallbackRealm:
      Object.getPrototypeOf(formCallback) === child.Function.prototype,
    searchCallbackRealm:
      Object.getPrototypeOf(searchCallback) === child.Function.prototype,
    seen: child.__pairIterableSeen
  });
})()
"#,
        )
        .expect("cross-Realm pair-iterable callbacks should evaluate");

    assert_eq!(
        result,
        r#"{"formCallbackRealm":true,"searchCallbackRealm":true,"seen":["child:parent-this:formData:form:value:true","child:parent-this:searchParams:search:value:true"]}"#
    );
}

#[test]
fn headers_prototype_methods_are_declared_operations() {
    let mut vm = new_storage_test_vm("https://headers-prototype-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const headers = new Headers([['X-A', '1']]);
  headers.append('X-A', '2');
  headers.set('X-B', '3');
  const methods = [
    ['get', 1],
    ['has', 1],
    ['getSetCookie', 0],
    ['set', 2],
    ['delete', 1],
    ['append', 2],
    ['keys', 0],
    ['values', 0],
    ['entries', 0],
    ['forEach', 1],
  ];
  const descriptors = methods.map(([name, expectedLength]) => {
    const descriptor = Object.getOwnPropertyDescriptor(Headers.prototype, name);
    return [
      name,
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      expectedLength,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable,
    ].join(':');
  });
  const iteratorDescriptor = Object.getOwnPropertyDescriptor(Headers.prototype, Symbol.iterator);
  const instanceOwn = methods
    .map(([name]) => name)
    .filter((name) => Object.hasOwn(headers, name))
    .join(',');
  const prototypeEnumerable = Object.keys(Headers.prototype)
    .filter((name) => methods.some(([methodName]) => methodName === name))
    .join(',');
  const iterated = Array.from(headers).map(([name, value]) => `${name}=${value}`).join(',');
  const forEachSeen = [];
  headers.forEach(function(value, name, owner) {
    forEachSeen.push([this.label, name, value, owner === headers].join(':'));
  }, { label: 'ctx' });
  return JSON.stringify({
    descriptors,
    iterator: [
      typeof iteratorDescriptor?.value,
      iteratorDescriptor?.value === Headers.prototype.entries,
      iteratorDescriptor?.value?.name,
      iteratorDescriptor?.value?.length,
      iteratorDescriptor?.enumerable,
      iteratorDescriptor?.writable,
      iteratorDescriptor?.configurable,
    ].join(':'),
    instanceOwn,
    prototypeEnumerable,
    behavior: [
      headers.get('x-a'),
      headers.has('x-b'),
      iterated,
      forEachSeen.join(','),
    ].join('|'),
  });
})()
"#,
        )
        .expect("Headers prototype methods descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["get:function:get:1:1:true:true:true","has:function:has:1:1:true:true:true","getSetCookie:function:getSetCookie:0:0:true:true:true","set:function:set:2:2:true:true:true","delete:function:delete:1:1:true:true:true","append:function:append:2:2:true:true:true","keys:function:keys:0:0:true:true:true","values:function:values:0:0:true:true:true","entries:function:entries:0:0:true:true:true","forEach:function:forEach:1:1:true:true:true"],"iterator":"function:true:entries:0:false:true:true","instanceOwn":"","prototypeEnumerable":"get,has,getSetCookie,set,delete,append,keys,values,entries,forEach","behavior":"1, 2|true|x-a=1, 2,x-b=3|ctx:x-a:1, 2:true,ctx:x-b:3:true"}"#
    );
}

#[test]
fn headers_methods_reject_incompatible_receivers() {
    let mut vm = new_storage_test_vm("https://headers-receiver.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  const fake = Object.create(Headers.prototype);
  const bare = {};
  const methods = [
    ['get', () => Headers.prototype.get.call(fake, 'x-test')],
    ['has', () => Headers.prototype.has.call(fake, 'x-test')],
    ['getSetCookie', () => Headers.prototype.getSetCookie.call(fake)],
    ['set', () => Headers.prototype.set.call(fake, 'x-test', '1')],
    ['delete', () => Headers.prototype.delete.call(fake, 'x-test')],
    ['append', () => Headers.prototype.append.call(fake, 'x-test', '1')],
    ['keys', () => Headers.prototype.keys.call(fake)],
    ['values', () => Headers.prototype.values.call(fake)],
    ['entries', () => Headers.prototype.entries.call(fake)],
    ['forEach', () => Headers.prototype.forEach.call(fake, () => {})],
    ['iterator', () => Headers.prototype[Symbol.iterator].call(fake)],
    ['bare', () => Headers.prototype.get.call(bare, 'x-test')],
  ];
  const failures = methods
    .map(([name, callback]) => `${name}:${probe(callback)}`)
    .join(',');
  const fakeInit = probe(() => new Headers(fake));
  const real = new Headers([['X-Test', '1']]);
  return [
    failures,
    fakeInit,
    real.get('x-test'),
    Array.from(real.keys()).join(','),
  ].join('|');
})()
"#,
        )
        .expect("Headers receiver brand checks should evaluate");

    assert_eq!(
        result,
        "get:TypeError,has:TypeError,getSetCookie:TypeError,set:TypeError,delete:TypeError,append:TypeError,keys:TypeError,values:TypeError,entries:TypeError,forEach:TypeError,iterator:TypeError,bare:TypeError|TypeError|1|x-test"
    );
}

#[test]
fn headers_declared_methods_preserve_descriptors_and_iterator_alias() {
    let mut vm = new_storage_test_vm("https://headers-declared-methods.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const headers = new Response(null, { headers: [['X-A', '1']] }).headers;
  const descriptors = [
    ['get', 1],
    ['has', 1],
    ['getSetCookie', 0],
    ['set', 2],
    ['delete', 1],
    ['append', 2],
    ['keys', 0],
    ['values', 0],
    ['entries', 0],
    ['forEach', 1],
  ].map(([name, expectedLength]) => {
    const descriptor = Object.getOwnPropertyDescriptor(headers, name);
    return [
      name,
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      expectedLength,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable,
    ].join(':');
  });
  const iteratorDescriptor = Object.getOwnPropertyDescriptor(headers, Symbol.iterator);
  const iterated = Array.from(headers).map(([name, value]) => `${name}=${value}`).join(',');
  return JSON.stringify({
    descriptors,
    iterator: [
      typeof iteratorDescriptor?.value,
      iteratorDescriptor?.value === headers.entries,
      iteratorDescriptor?.value?.name,
      iteratorDescriptor?.value?.length,
      iteratorDescriptor?.enumerable,
      iteratorDescriptor?.writable,
      iteratorDescriptor?.configurable,
      iterated,
    ].join(':'),
  });
})()
"#,
        )
        .expect("Headers declared methods descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":["get:function:get:1:1:true:true:true","has:function:has:1:1:true:true:true","getSetCookie:function:getSetCookie:0:0:true:true:true","set:function:set:2:2:true:true:true","delete:function:delete:1:1:true:true:true","append:function:append:2:2:true:true:true","keys:function:keys:0:0:true:true:true","values:function:values:0:0:true:true:true","entries:function:entries:0:0:true:true:true","forEach:function:forEach:1:1:true:true:true"],"iterator":"function:true:entries:0:true:true:true:x-a=1"}"#
    );
}

#[test]
fn headers_backing_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://headers-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const internalNames = (object) => Object.getOwnPropertyNames(object)
    .filter((name) => name.startsWith('__lmHeaders'))
    .sort()
    .join(',');
  const nullable = (value) => value === null ? 'null' : String(value);

  const headers = new Headers([
    ['X-Test', 'one'],
    ['Set-Cookie', 'a=1'],
  ]);
  const response = new Response('body', { headers: [['X-Safe', '1']] });
  const prototypeOwnBefore = internalNames(Headers.prototype);
  const initialOwn = [internalNames(headers), internalNames(response.headers)].join('|');

  Headers.prototype.__lmHeadersEntriesJson = '[["x-prototype","bad"]]';
  Headers.prototype.__lmHeadersImmutable = true;
  Headers.prototype.__lmHeadersGuard = 'request-no-cors';
  headers.__lmHeadersEntriesJson = '[["x-own","bad"]]';
  headers.__lmHeadersImmutable = true;
  headers.__lmHeadersGuard = 'request-no-cors';
  headers.append('X-Test', 'two');
  headers.append('X-Unsafe', 'bad');

  const fakeHeadersLike = Object.create(Headers.prototype);
  Object.defineProperty(fakeHeadersLike, '__lmHeadersEntriesJson', {
    value: '[["x-fake","bad"]]',
    enumerable: true,
  });
  const fakeInit = probe(() => {
    new Headers(fakeHeadersLike);
    return 'ok';
  });

  const record = { 'x-record': 'ok' };
  Object.defineProperty(record, '__lmHeadersEntriesJson', {
    value: '[["x-spoof","bad"]]',
    enumerable: false,
  });
  const recordCopy = new Headers(record);

  Headers.prototype.__lmHeadersGuard = 'none';
  response.headers.__lmHeadersGuard = 'none';
  response.headers.append('Set-Cookie', 'b=2');
  response.headers.append('X-Safe', '2');

  Headers.prototype.__lmHeadersImmutable = false;
  const errorHeaders = Response.error().headers;
  errorHeaders.__lmHeadersImmutable = false;
  const immutable = probe(() => {
    errorHeaders.append('X-Error', 'bad');
    return errorHeaders.get('X-Error');
  });

  return JSON.stringify({
    prototypeOwnBefore,
    initialOwn,
    real: [headers.get('X-Test'), headers.get('X-Unsafe'), headers.getSetCookie().join('|')].join('|'),
    fakeInit,
    record: [nullable(recordCopy.get('X-Record')), nullable(recordCopy.get('X-Spoof'))].join('|'),
    guarded: [response.headers.get('X-Safe'), response.headers.getSetCookie().join('|')].join('|'),
    immutable,
  });
})()
"#,
        )
        .expect("Headers private slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"prototypeOwnBefore":"","initialOwn":"|","real":"one, two|bad|a=1","fakeInit":"throw:TypeError","record":"ok|null","guarded":"1, 2|","immutable":"throw:TypeError"}"#
    );
}

#[test]
fn response_backing_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://response-slots.test/");

    vm.eval(
        r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const internalNames = (object) => Object.getOwnPropertyNames(object)
    .filter((name) => name.startsWith('__lmResponse'))
    .sort()
    .join(',');
  const nullable = (value) => value === null ? 'null' : String(value);

  const response = new Response('payload', {
    status: 201,
    statusText: 'Created',
    headers: [['Content-Type', 'text/plain'], ['X-Original', '1']],
  });
  const initial = [
    internalNames(Response.prototype),
    internalNames(response),
    response.status,
    response.statusText,
    response.ok,
    response.type,
    response.url,
    response.redirected,
    response.headers.get('X-Original'),
    response.bodyUsed,
  ].join('|');

  const spoofEntries = {
    __lmResponseStatus: 599,
    __lmResponseStatusText: 'Spoofed',
    __lmResponseOk: false,
    __lmResponseUrl: 'https://spoofed.invalid/',
    __lmResponseRedirected: true,
    __lmResponseType: 'opaque',
    __lmResponseHeadersObject: new Headers([['X-Spoof', 'bad']]),
    __lmResponseBody: null,
    __lmResponseBodyUsed: true,
  };
  for (const [name, value] of Object.entries(spoofEntries)) {
    Object.defineProperty(Response.prototype, name, {
      value,
      configurable: true,
    });
    Object.defineProperty(response, name, {
      value,
      configurable: true,
    });
  }

  const clone = response.clone();
  const cloneInitial = [
    clone.status,
    clone.statusText,
    clone.ok,
    clone.type,
    clone.url,
    clone.redirected,
    clone.headers.get('X-Original'),
    nullable(clone.headers.get('X-Spoof')),
    clone.bodyUsed,
  ].join('|');

  const errorResponse = Response.error();
  errorResponse.__lmResponseStatus = 200;
  errorResponse.__lmResponseType = 'basic';
  errorResponse.__lmResponseBodyUsed = true;
  errorResponse.__lmResponseHeadersObject = new Headers([['X-Error-Spoof', 'bad']]);
  const errorClone = errorResponse.clone();
  const errorSurface = [
    errorResponse.status,
    errorResponse.ok,
    errorResponse.type,
    errorResponse.url,
    errorResponse.redirected,
    errorResponse.body === null,
    errorResponse.bodyUsed,
    nullable(errorResponse.headers.get('X-Error-Spoof')),
    errorClone.status,
    errorClone.type,
    errorClone.body === null,
    errorClone.bodyUsed,
  ].join('|');

  const fake = Object.create(Response.prototype);
  const bodyOutcome = async (prototype, method, receiver) => {
    let value;
    try {
      value = prototype[method].call(receiver);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
    const isPromise = value instanceof Promise;
    try {
      await value;
      return `resolved:${isPromise}`;
    } catch (error) {
      return `rejected:${isPromise}:${error && error.name}`;
    }
  };
  const fakeSurface = [
    probe(() => fake.status),
    probe(() => fake.headers),
    probe(() => fake.bodyUsed),
    probe(() => Response.prototype.clone.call(fake)),
  ].join('|');

  globalThis.__responseSlotProbe = {
    initial,
    cloneInitial,
    errorSurface,
    fakeSurface,
    consumed: null,
    fakeBodyMethods: null,
  };
  Promise.all([response.text(), clone.text()]).then(
    ([text, cloneText]) => {
      globalThis.__responseSlotProbe.consumed = [
        text,
        cloneText,
        response.bodyUsed,
        clone.bodyUsed,
        probe(() => response.clone()),
      ].join('|');
    },
    (error) => {
      globalThis.__responseSlotProbe.consumed = `reject:${error && error.name}`;
    }
  );
  Promise.all(["arrayBuffer", "blob", "bytes", "formData", "json", "text"]
    .map((method) => bodyOutcome(Response.prototype, method, fake))).then((outcomes) => {
      globalThis.__responseSlotProbe.fakeBodyMethods = outcomes.join("|");
    });
})()
"#,
    )
    .expect("Response private slot spoofing setup should evaluate");

    for _ in 0..8 {
        vm.eval("0")
            .expect("Response private slot spoofing promise chain should drain");
    }
    let result = vm
        .eval("JSON.stringify(globalThis.__responseSlotProbe)")
        .expect("Response private slot spoofing probe result should evaluate");

    assert_eq!(
        result,
        r#"{"initial":"||201|Created|true|default||false|1|false","cloneInitial":"201|Created|true|default||false|1|null|false","errorSurface":"0|false|error||false|true|false|null|0|error|true|false","fakeSurface":"throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError","consumed":"payload|payload|true|true|throw:TypeError","fakeBodyMethods":"rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError"}"#
    );
}

#[test]
fn request_backing_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://request-slots.test/");

    vm.eval(
        r#"
(() => {
  const probe = (callback) => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const internalNames = (object) => Object.getOwnPropertyNames(object)
    .filter((name) => name.startsWith('__lmRequest') || name.startsWith('__lmNetworkBodySource'))
    .sort()
    .join(',');
  const nullable = (value) => value === null ? 'null' : String(value);

  const request = new Request('/source', {
    method: 'POST',
    body: 'payload',
    headers: [['X-Original', '1']],
    cache: 'reload',
    credentials: 'include',
    redirect: 'manual',
    duplex: 'half'
  });
  const initial = [
    internalNames(Request.prototype),
    internalNames(request),
    request.method,
    request.url,
    request.headers.get('X-Original'),
    request.cache,
    request.credentials,
    request.redirect,
    request.bodyUsed
  ].join('|');

  const spoofSource = {
    __lmNetworkBodySourceKind: 'bytes',
    __lmBody: 'spoof-body'
  };
  const spoofEntries = {
    __lmRequestMethod: 'GET',
    __lmRequestUrl: 'data:text/plain,spoof-url',
    __lmRequestHeaders: new Headers([['X-Spoof', 'bad']]),
    __lmRequestCache: 'only-if-cached',
    __lmRequestCredentials: 'omit',
    __lmRequestRedirect: 'error',
    __lmRequestBody: null,
    __lmRequestBodyUsed: true,
    __lmNetworkBodySource: spoofSource
  };
  for (const [name, value] of Object.entries(spoofEntries)) {
    Object.defineProperty(Request.prototype, name, {
      value,
      configurable: true
    });
    Object.defineProperty(request, name, {
      value,
      configurable: true
    });
  }

  const inherited = new Request(request);
  const clone = request.clone();
  const inheritedSurface = [
    inherited.method,
    inherited.url,
    inherited.headers.get('X-Original'),
    nullable(inherited.headers.get('X-Spoof')),
    inherited.cache,
    inherited.credentials,
    inherited.redirect,
    inherited.bodyUsed
  ].join('|');
  const cloneSurface = [
    clone.method,
    clone.url,
    clone.headers.get('X-Original'),
    nullable(clone.headers.get('X-Spoof')),
    clone.cache,
    clone.credentials,
    clone.redirect,
    clone.bodyUsed
  ].join('|');

  const fetchRequest = new Request('data:text/plain,fetch-ok');
  fetchRequest.__lmRequestUrl = 'data:text/plain,fetch-bad';
  fetchRequest.__lmNetworkBodySource = spoofSource;

  const fake = Object.create(Request.prototype);
  const bodyOutcome = async (prototype, method, receiver) => {
    let value;
    try {
      value = prototype[method].call(receiver);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
    const isPromise = value instanceof Promise;
    try {
      await value;
      return `resolved:${isPromise}`;
    } catch (error) {
      return `rejected:${isPromise}:${error && error.name}`;
    }
  };
  const fakeSurface = [
    probe(() => fake.method),
    probe(() => fake.headers),
    probe(() => fake.bodyUsed),
    probe(() => Request.prototype.clone.call(fake)),
  ].join('|');

  globalThis.__requestSlotProbe = {
    initial,
    inheritedSurface,
    cloneSurface,
    fakeSurface,
    consumed: null,
    fakeBodyMethods: null
  };
  Promise.all([
    request.text(),
    clone.text(),
    inherited.text(),
    fetch(fetchRequest).then((response) => response.text())
  ]).then(
    ([text, cloneText, inheritedText, fetchText]) => {
      globalThis.__requestSlotProbe.consumed = [
        text,
        cloneText,
        inheritedText,
        fetchText,
        request.bodyUsed,
        clone.bodyUsed,
        inherited.bodyUsed,
        probe(() => request.clone())
      ].join('|');
    },
    (error) => {
      globalThis.__requestSlotProbe.consumed = `reject:${error && error.name}`;
    }
  );
  Promise.all(["arrayBuffer", "blob", "bytes", "formData", "json", "text"]
    .map((method) => bodyOutcome(Request.prototype, method, fake))).then((outcomes) => {
      globalThis.__requestSlotProbe.fakeBodyMethods = outcomes.join("|");
    });
})()
"#,
    )
    .expect("Request private slot spoofing setup should evaluate");

    for _ in 0..12 {
        vm.eval("0")
            .expect("Request private slot spoofing promise chain should drain");
    }
    let result = vm
        .eval("JSON.stringify(globalThis.__requestSlotProbe)")
        .expect("Request private slot spoofing probe result should evaluate");

    assert_eq!(
        result,
        r#"{"initial":"||POST|https://request-slots.test/source|1|reload|include|manual|false","inheritedSurface":"POST|https://request-slots.test/source|1|null|reload|include|manual|false","cloneSurface":"POST|https://request-slots.test/source|1|null|reload|include|manual|false","fakeSurface":"throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError","consumed":"payload|payload|payload|fetch-ok|true|true|true|throw:TypeError","fakeBodyMethods":"rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError|rejected:true:TypeError"}"#
    );
}

#[test]
fn request_constructor_rejects_used_or_locked_inherited_body() {
    let mut vm = new_storage_test_vm("https://request-body.test/");

    vm.eval(
        r#"
(() => {
  const probe = callback => {
    try {
      return String(callback());
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const request = new Request("/used", {
    method: "POST",
    body: "payload",
    duplex: "half"
  });
  const cloneBefore = request.clone();
  const inheritedBefore = new Request(request);
  const locked = request.clone();
  locked.body.getReader();

  globalThis.__requestBodyUsedProbe = {
    bodyIsStream: request.body instanceof ReadableStream,
    bodyUsedBefore: request.bodyUsed,
    lockedNew: probe(() => new Request(locked)),
    lockedClone: probe(() => locked.clone()),
    done: null
  };

  Promise.all([
    request.text(),
    cloneBefore.text(),
    inheritedBefore.text(),
    request.text().then(
      value => `resolve:${value}`,
      error => `reject:${error && error.name}`
    )
  ]).then(
    ([requestText, cloneText, inheritedText, secondRead]) => {
      Object.assign(globalThis.__requestBodyUsedProbe, {
        requestText,
        cloneText,
        inheritedText,
        secondRead,
        bodyUsedAfter: request.bodyUsed,
        afterReplacement: probe(() => new Request(request, {
          body: "replacement",
          duplex: "half"
        }).bodyUsed),
        afterNew: probe(() => new Request(request)),
        afterClone: probe(() => request.clone()),
        done: "ok"
      });
    },
    error => {
      globalThis.__requestBodyUsedProbe.done = `reject:${error && error.name}`;
    }
  );
})()
"#,
    )
    .expect("Request bodyUsed probe setup should evaluate");

    for _ in 0..12 {
        vm.eval("0")
            .expect("Request bodyUsed promise chain should drain");
    }
    let result = vm
        .eval("JSON.stringify(globalThis.__requestBodyUsedProbe)")
        .expect("Request bodyUsed probe result should evaluate");

    assert_eq!(
        result,
        r#"{"bodyIsStream":true,"bodyUsedBefore":false,"lockedNew":"throw:TypeError","lockedClone":"throw:TypeError","done":"ok","requestText":"payload","cloneText":"payload","inheritedText":"payload","secondRead":"reject:TypeError","bodyUsedAfter":true,"afterReplacement":"false","afterNew":"throw:TypeError","afterClone":"throw:TypeError"}"#
    );
}

#[test]
fn fetch_disturbs_the_input_request_body_before_network_completion() {
    let mut vm = new_storage_test_vm("https://request-fetch-body-used.test/");

    vm.eval(
        r#"
(() => {
  const request = new Request("data:text/plain,done", {
    method: "POST",
    body: "payload"
  });
  const outcome = globalThis.__fetchRequestBodyUsed = {
    before: request.bodyUsed,
    immediate: null,
    clone: null,
    text: null
  };
  const promise = fetch(request);
  outcome.immediate = request.bodyUsed;
  try {
    request.clone();
    outcome.clone = "resolved";
  } catch (error) {
    outcome.clone = error && error.name;
  }
  promise.then(response => response.text()).then(text => {
    outcome.text = text;
  });
})()
"#,
    )
    .expect("fetch Request body disturbance probe should run");
    for _ in 0..4 {
        vm.eval("0")
            .expect("fetch Request body disturbance promise should drain");
    }

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__fetchRequestBodyUsed)")
            .expect("fetch Request body disturbance result should evaluate"),
        r#"{"before":false,"immediate":true,"clone":"TypeError","text":"done"}"#
    );
}

#[test]
fn request_signal_init_inherited_and_clone_are_dependent() {
    let mut vm = new_storage_test_vm("https://request-signal.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const controller = new AbortController();
  const request = new Request("/signal", { signal: controller.signal });
  const inherited = new Request(request);
  const clone = request.clone();
  const nullOverride = new Request(request, { signal: null });
  const preAborted = new Request("/pre-aborted", {
    signal: AbortSignal.abort("pre-aborted")
  });
  const events = [];
  request.signal.addEventListener("abort", () => events.push("request"));
  inherited.signal.addEventListener("abort", () => events.push("inherited"));
  clone.signal.addEventListener("abort", () => events.push("clone"));

  const before = {
    tag: Object.prototype.toString.call(request.signal),
    requestSignalDifferent: request.signal !== controller.signal,
    inheritedSignalDifferent: inherited.signal !== request.signal,
    cloneSignalDifferent: clone.signal !== request.signal,
    nullOverrideDifferent: nullOverride.signal !== request.signal,
    preAborted: preAborted.signal.aborted,
    preAbortedReason: String(preAborted.signal.reason),
    nullOverrideAborted: nullOverride.signal.aborted
  };
  controller.abort("request-abort");
  return JSON.stringify({
    before,
    after: {
      requestAborted: request.signal.aborted,
      requestReason: String(request.signal.reason),
      inheritedAborted: inherited.signal.aborted,
      inheritedReason: String(inherited.signal.reason),
      cloneAborted: clone.signal.aborted,
      cloneReason: String(clone.signal.reason),
      nullOverrideAborted: nullOverride.signal.aborted,
      events: events.sort()
    }
  });
})()
"#,
        )
        .expect("Request signal dependency probe should evaluate");

    assert_eq!(
        result,
        r#"{"before":{"tag":"[object AbortSignal]","requestSignalDifferent":true,"inheritedSignalDifferent":true,"cloneSignalDifferent":true,"nullOverrideDifferent":true,"preAborted":true,"preAbortedReason":"pre-aborted","nullOverrideAborted":false},"after":{"requestAborted":true,"requestReason":"request-abort","inheritedAborted":true,"inheritedReason":"request-abort","cloneAborted":true,"cloneReason":"request-abort","nullOverrideAborted":false,"events":["clone","inherited","request"]}}"#
    );
}

#[test]
fn window_fetch_missing_input_rejects_type_error() {
    let mut vm = new_storage_test_vm("https://fetch-missing-input.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__fetchMissingInputResult = "pending";
  fetch().then(
    () => { globalThis.__fetchMissingInputResult = "resolved"; },
    (error) => { globalThis.__fetchMissingInputResult = error && error.name; }
  );
  return "scheduled";
})()
"#,
    )
    .expect("fetch missing input probe should evaluate");

    let result = vm
        .eval("String(globalThis.__fetchMissingInputResult)")
        .expect("fetch missing input rejection should settle");

    assert_eq!(result, "TypeError");
}

#[test]
fn window_fetch_document_csp_blocks_connect_src_and_dispatches_event() {
    let mut vm = new_storage_test_vm("https://fetch-connect-csp.test/");
    vm.set_response_content_security_policies(&[String::from("connect-src 'none'")]);

    vm.eval(
        r#"
(() => {
  globalThis.__fetchCspEvents = [];
  globalThis.__fetchCspResult = "pending";
  self.addEventListener("securitypolicyviolation", event => {
    __fetchCspEvents.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition,
      instance: event instanceof SecurityPolicyViolationEvent
    });
  });
  fetch("data:text/plain,ok").then(
    () => { __fetchCspResult = "resolved"; },
    (error) => {
      __fetchCspResult = {
        name: error && error.name,
        csp: String(error && error.message).includes("Content Security Policy")
      };
    }
  );
  return "scheduled";
})()
"#,
    )
    .expect("fetch CSP block setup should evaluate");

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );

    let result = vm
        .eval(
            "JSON.stringify({ ...globalThis.__fetchCspResult, events: globalThis.__fetchCspEvents })",
        )
        .expect("fetch CSP rejection should settle");

    assert_eq!(
        result,
        r#"{"name":"TypeError","csp":true,"events":[{"blockedURI":"data","effectiveDirective":"connect-src","disposition":"enforce","instance":true}]}"#
    );
}

#[test]
fn window_fetch_document_csp_report_only_dispatches_without_blocking() {
    let mut vm = new_storage_test_vm("https://fetch-connect-report-only.test/");
    vm.set_response_content_security_report_only_policies(&[String::from("connect-src 'none'")]);

    vm.eval(
        r#"
(() => {
  globalThis.__fetchReportOnlyEvents = [];
  globalThis.__fetchReportOnlyResult = "pending";
  self.addEventListener("securitypolicyviolation", event => {
    __fetchReportOnlyEvents.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition,
      instance: event instanceof SecurityPolicyViolationEvent
    });
  });
  fetch("data:text/plain,ok").then(
    (response) => {
      __fetchReportOnlyResult = { status: response.status };
    },
    (error) => { __fetchReportOnlyResult = "rejected:" + (error && error.name); }
  );
  return "scheduled";
})()
"#,
    )
    .expect("fetch CSP report-only setup should evaluate");

    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );

    let result = vm
        .eval(
            "JSON.stringify({ ...globalThis.__fetchReportOnlyResult, events: globalThis.__fetchReportOnlyEvents })",
        )
        .expect("fetch CSP report-only request should settle");

    assert_eq!(
        result,
        r#"{"status":200,"events":[{"blockedURI":"data","effectiveDirective":"connect-src","disposition":"report","instance":true}]}"#
    );
}

#[test]
fn request_redirect_init_uses_webidl_enum() {
    let mut vm = new_storage_test_vm("https://request-redirect-init.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const inherited = new Request("/source", { redirect: "manual" });
  let invalidName = "";
  try {
    new Request("/bad", { redirect: "invalid" });
  } catch (error) {
    invalidName = error && error.name;
  }
  return [
    new Request("/default").redirect,
    inherited.redirect,
    new Request(inherited, { redirect: "error" }).redirect,
    invalidName
  ].join("|");
})()
"#,
        )
        .expect("Request redirect init should evaluate");

    assert_eq!(result, "follow|manual|error|TypeError");
}

#[test]
fn request_mode_init_uses_webidl_enum_and_no_cors_method_gate() {
    let mut vm = new_storage_test_vm("https://request-mode-init.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const inherited = new Request("/source", { mode: "no-cors" });
  const probe = (callback) => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  return [
    new Request("/default").mode,
    inherited.mode,
    new Request(inherited, { mode: "same-origin" }).mode,
    probe(() => new Request("/bad", { mode: "invalid" })),
    probe(() => new Request("/nav", { mode: "navigate" })),
    probe(() => new Request("/put", { mode: "no-cors", method: "PUT" }))
  ].join("|");
})()
"#,
        )
        .expect("Request mode init should evaluate");

    assert_eq!(
        result,
        "cors|no-cors|same-origin|TypeError|TypeError|TypeError"
    );
}

#[test]
fn body_init_string_fallback_uses_webidl_usvstring_errors() {
    let mut vm = new_storage_test_vm("https://body-init-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = (callback) => {
    try {
      callback();
      return "ok";
    } catch (error) {
      return error && error.name;
    }
  };
  const throwing = { toString() { throw new RangeError("body stringifier"); } };
  globalThis.__fetchBodySymbolResult = "pending";
  fetch("/body-symbol", { method: "POST", body: Symbol("body") }).then(
    () => { globalThis.__fetchBodySymbolResult = "resolved"; },
    (error) => { globalThis.__fetchBodySymbolResult = error && error.name; }
  );
  return JSON.stringify({
    requestSymbol: probe(() => new Request("/body-symbol", { method: "POST", body: Symbol("body") })),
    requestThrowing: probe(() => new Request("/body-throwing", { method: "POST", body: throwing })),
    responseSymbol: probe(() => new Response(Symbol("body"))),
    responseThrowing: probe(() => new Response(throwing)),
  });
})()
"#,
        )
        .expect("BodyInit conversion probe should evaluate");

    let fetch_result = vm
        .eval("String(globalThis.__fetchBodySymbolResult)")
        .expect("fetch body conversion rejection should settle");

    assert_eq!(
        result,
        r#"{"requestSymbol":"TypeError","requestThrowing":"RangeError","responseSymbol":"TypeError","responseThrowing":"RangeError"}"#
    );
    assert_eq!(fetch_result, "TypeError");
}

#[test]
fn url_search_params_delete_preserves_opaque_path_trailing_space() {
    let mut vm = new_storage_test_vm("https://url-opaque-path-query-removal.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const first = new URL('data:space    ?test');
  first.searchParams.delete('test');
  const second = new URL('data:space    ?test#test');
  second.searchParams.delete('test');
  return JSON.stringify({
    firstPathname: first.pathname,
    firstHref: first.href,
    secondPathname: second.pathname,
    secondHref: second.href,
  });
})()
"#,
        )
        .expect("URL opaque path query removal probe should evaluate");

    assert_eq!(
        result,
        r#"{"firstPathname":"space   %20","firstHref":"data:space   %20","secondPathname":"space   %20","secondHref":"data:space   %20#test"}"#
    );
}

#[test]
fn url_search_params_methods_parse_webidl_arguments() {
    let mut vm = new_storage_test_vm("https://url-search-params-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const params = new URLSearchParams('a=1&a=2&b=null');
  const seen = [];
  const thisArg = { marker: 'ctx' };
  params.append(null, undefined);
  params.set('c', 3);
  params.delete('a', 1);
  params.forEach(function(value, name, owner) {
    seen.push(`${this.marker}:${name}:${value}:${owner === params}`);
  }, thisArg);
  return JSON.stringify({
    allA: params.getAll('a').join(','),
    hasANull: params.has('a', null),
    hasBNull: params.has('b', null),
    nullValue: params.get(null),
    serialized: params.toString(),
    seen,
    getSymbol: probe(() => params.get(Symbol())),
    appendMissingValue: probe(() => params.append('x')),
    deleteSymbolValue: probe(() => params.delete('a', Symbol())),
    hasSymbolName: probe(() => params.has(Symbol())),
    forEachMissing: probe(() => params.forEach())
  });
})()
"#,
        )
        .expect("URLSearchParams WebIDL argument conversion probe should run");

    assert_eq!(
        result,
        r#"{"allA":"2","hasANull":false,"hasBNull":true,"nullValue":"undefined","serialized":"a=2&b=null&null=undefined&c=3","seen":["ctx:a:2:true","ctx:b:null:true","ctx:null:undefined:true","ctx:c:3:true"],"getSymbol":"throw:TypeError","appendMissingValue":"throw:TypeError","deleteSymbolValue":"throw:TypeError","hasSymbolName":"throw:TypeError","forEachMissing":"throw:TypeError"}"#
    );
}

#[test]
fn form_data_and_url_search_params_template_methods_have_webidl_descriptors() {
    let mut vm = new_storage_test_vm("https://template-method-descriptors.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const methodDescriptor = (Constructor, key) => {
    const descriptor = Object.getOwnPropertyDescriptor(Constructor.prototype, key);
    return [
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(':');
  };
  const iteratorAliasShape = Constructor => {
    const entries = Object.getOwnPropertyDescriptor(Constructor.prototype, 'entries');
    const iterator = Object.getOwnPropertyDescriptor(Constructor.prototype, Symbol.iterator);
    return [
      entries.value === iterator.value,
      iterator.enumerable,
      iterator.writable,
      iterator.configurable,
      iterator.value.name,
      iterator.value.length
    ].join(':');
  };
  return JSON.stringify({
    uspAppend: methodDescriptor(URLSearchParams, 'append'),
    uspForEach: methodDescriptor(URLSearchParams, 'forEach'),
    uspIterator: iteratorAliasShape(URLSearchParams),
    fdAppend: methodDescriptor(FormData, 'append'),
    fdForEach: methodDescriptor(FormData, 'forEach'),
    fdIterator: iteratorAliasShape(FormData)
  });
})()
"#,
        )
        .expect("template method descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"uspAppend":"function:append:2:true:true:true","uspForEach":"function:forEach:1:true:true:true","uspIterator":"true:false:true:true:entries:0","fdAppend":"function:append:2:true:true:true","fdForEach":"function:forEach:1:true:true:true","fdIterator":"true:false:true:true:entries:0"}"#
    );
}

#[test]
fn url_search_params_iterators_share_a_webidl_iterator_prototype() {
    let mut vm = new_storage_test_vm("https://url-search-params-iterator-prototype.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const params = new URLSearchParams("a=1");
  const entries = params.entries();
  const keys = params.keys();
  const prototype = Object.getPrototypeOf(entries);
  const intrinsicIteratorPrototype = Object.getPrototypeOf(
    Object.getPrototypeOf([][Symbol.iterator]())
  );
  const next = Object.getOwnPropertyDescriptor(prototype, "next");
  const outcome = callback => {
    try { callback(); return "return"; }
    catch (error) { return error && error.name; }
  };
  return JSON.stringify({
    shared: prototype === Object.getPrototypeOf(keys),
    base: Object.getPrototypeOf(prototype) === intrinsicIteratorPrototype,
    tag: Object.prototype.toString.call(entries),
    ownTag: Object.hasOwn(entries, Symbol.toStringTag),
    next: [typeof next.value, next.enumerable, next.writable, next.configurable].join(":"),
    invalid: [
      outcome(() => prototype.next()),
      outcome(() => prototype.next.call(new Headers().entries()))
    ].join(",")
  });
})()
"#,
        )
        .expect("URLSearchParams iterator prototype probe should evaluate");

    assert_eq!(
        result,
        r#"{"shared":true,"base":true,"tag":"[object URLSearchParams Iterator]","ownTag":false,"next":"function:true:true:true","invalid":"TypeError,TypeError"}"#
    );
}

#[test]
fn webidl_iterator_prototypes_use_v8_intrinsics_after_public_tampering() {
    let mut vm = new_storage_test_vm("https://webidl-iterator-intrinsics.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const ArrayConstructor = Array;
  const originalArrayIterator = ArrayConstructor.prototype[Symbol.iterator];
  const originalObjectGetPrototypeOf = Object.getPrototypeOf;
  const getPrototypeOf = Reflect.getPrototypeOf;
  const iteratorPrototype = getPrototypeOf(
    getPrototypeOf([][Symbol.iterator]())
  );
  const asyncIteratorPrototype = getPrototypeOf(
    getPrototypeOf(async function*() {}).prototype
  );
  const specs = [
    ["URLSearchParams", () => new URLSearchParams("a=1").entries()],
    ["Headers", () => new Headers({ "x-a": "1" }).entries()],
    ["FormData", () => new FormData().entries()]
  ];
  const failures = [];
  const poisoned = function poisonedIterator() {
    throw new Error("public Array iterator was observed");
  };
  ArrayConstructor.prototype[Symbol.iterator] = poisoned;
  Object.getPrototypeOf = function poisonedGetPrototypeOf() {
    throw new Error("public Object.getPrototypeOf was observed");
  };
  globalThis.Array = undefined;
  try {
    for (let index = 0; index < specs.length; index += 1) {
      const name = specs[index][0];
      const iterator = specs[index][1]();
      const prototype = getPrototypeOf(iterator);
      const next = Object.getOwnPropertyDescriptor(prototype, "next");
      const tag = Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag);
      if (getPrototypeOf(prototype) !== iteratorPrototype) {
        failures.push(`${name}:parent`);
      }
      if (iterator[Symbol.iterator]() !== iterator) {
        failures.push(`${name}:iterator`);
      }
      if (Object.hasOwn(iterator, Symbol.iterator)) {
        failures.push(`${name}:own-iterator`);
      }
      if (Object.hasOwn(prototype, "constructor")) {
        failures.push(`${name}:constructor`);
      }
      if (
        !next ||
        next.enumerable !== true ||
        next.writable !== true ||
        next.configurable !== true
      ) {
        failures.push(`${name}:next`);
      }
      if (
        !tag ||
        tag.value !== `${name} Iterator` ||
        tag.enumerable !== false ||
        tag.writable !== false ||
        tag.configurable !== true
      ) {
        failures.push(`${name}:tag`);
      }
    }
    const asyncIterator = new ReadableStream({
      start(controller) {
        controller.close();
      }
    }).values();
    const asyncPrototype = getPrototypeOf(asyncIterator);
    const asyncTag = Object.getOwnPropertyDescriptor(
      asyncPrototype,
      Symbol.toStringTag
    );
    if (getPrototypeOf(asyncPrototype) !== asyncIteratorPrototype) {
      failures.push("ReadableStream:parent");
    }
    if (asyncIterator[Symbol.asyncIterator]() !== asyncIterator) {
      failures.push("ReadableStream:iterator");
    }
    if (Object.hasOwn(asyncIterator, Symbol.asyncIterator)) {
      failures.push("ReadableStream:own-iterator");
    }
    if (Object.hasOwn(asyncPrototype, "constructor")) {
      failures.push("ReadableStream:constructor");
    }
    if (
      !asyncTag ||
      asyncTag.value !== "ReadableStream AsyncIterator" ||
      asyncTag.enumerable !== false ||
      asyncTag.writable !== false ||
      asyncTag.configurable !== true
    ) {
      failures.push("ReadableStream:tag");
    }
  } finally {
    globalThis.Array = ArrayConstructor;
    ArrayConstructor.prototype[Symbol.iterator] = originalArrayIterator;
    Object.getPrototypeOf = originalObjectGetPrototypeOf;
  }
  return failures.join("|");
})()
"#,
        )
        .expect("WebIDL iterator prototypes should use V8 intrinsics");

    assert_eq!(result, "");
}

#[test]
fn url_search_params_subclasses_retain_their_webidl_brand() {
    let mut vm = new_storage_test_vm("https://url-search-params-subclass.test/");

    let result = vm
        .eval(
            r#"
(() => {
  class ReadonlyURLSearchParams extends URLSearchParams {
    append() { throw new Error('readonly'); }
    delete() { throw new Error('readonly'); }
    set() { throw new Error('readonly'); }
    sort() { throw new Error('readonly'); }
  }

  const params = new ReadonlyURLSearchParams('loc=fr&loc=de&empty=');
  const seen = [];
  params.forEach((value, name, owner) => {
    seen.push(`${name}:${value}:${owner === params}`);
  });

  return JSON.stringify({
    directPrototypeIsSubclass:
      Object.getPrototypeOf(params) === ReadonlyURLSearchParams.prototype,
    get: params.get('loc'),
    getAll: params.getAll('loc').join(','),
    has: params.has('empty'),
    size: params.size,
    entries: Array.from(params.entries()).map(pair => pair.join(':')).join(','),
    seen,
    serialized: params.toString()
  });
})()
"#,
        )
        .expect("URLSearchParams subclass WebIDL operations should evaluate");

    assert_eq!(
        result,
        r#"{"directPrototypeIsSubclass":true,"get":"fr","getAll":"fr,de","has":true,"size":3,"entries":"loc:fr,loc:de,empty:","seen":["loc:fr:true","loc:de:true","empty::true"],"serialized":"loc=fr&loc=de&empty="}"#
    );
}

#[test]
fn url_and_search_params_declared_slots_ignore_prototype_spoofing() {
    let mut vm = new_storage_test_vm("https://url-declared-slots.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      const value = callback();
      return value === undefined ? 'undefined' : String(value);
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const hasOwn = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
  const stringify = value => value === undefined ? 'undefined' : String(value);
  const descriptorShape = (prototype, receiver, name) => {
    const descriptor = Object.getOwnPropertyDescriptor(prototype, name);
    return [
      name,
      typeof descriptor.get,
      descriptor.get && descriptor.get.name,
      descriptor.get && descriptor.get.length,
      typeof descriptor.set,
      descriptor.set && descriptor.set.name,
      descriptor.set && descriptor.set.length,
      descriptor.enumerable,
      descriptor.configurable,
      hasOwn(receiver, name)
    ].map(stringify).join(':');
  };

  const url = new URL('https://example.test/path?a=1');
  url.searchParams.append('b', '2');
  const params = new URLSearchParams('x=1&x=2');
  params.append('y', '3');
  const urlAttributeNames = [
    'href',
    'protocol',
    'username',
    'password',
    'host',
    'hostname',
    'port',
    'pathname',
    'search',
    'hash',
    'origin',
    'searchParams'
  ];
  const urlSlots = Object.getOwnPropertyNames(url)
    .filter(name => name.startsWith('__moliUrl'))
    .sort()
    .join(',');
  const paramsSlots = Object.getOwnPropertyNames(params)
    .filter(name => name.startsWith('__moliUrlSearchParams'))
    .sort()
    .join(',');

  URL.prototype.__moliUrlHref = 'https://spoof.test/?p=1';
  URL.prototype.__moliUrlSearchParams = new URLSearchParams('p=1');
  Object.defineProperties(url, {
    __moliUrlHref: {
      value: 'https://own-spoof.test/?p=1',
      configurable: true
    },
    __moliUrlSearchParams: {
      value: new URLSearchParams('own=1'),
      configurable: true
    }
  });
  URLSearchParams.prototype.__moliUrlSearchParamsOwner = url;
  URLSearchParams.prototype.__moliUrlSearchParamsPairs = [['p', '1']];
  Object.assign(params, {
    __moliUrlSearchParamsOwner: url,
    __moliUrlSearchParamsPairs: [['poison', '1']]
  });

  const fakeUrl = Object.create(URL.prototype);
  const fakeParams = Object.create(URLSearchParams.prototype);
  Object.assign(fakeParams, {
    __moliUrlSearchParamsOwner: url,
    __moliUrlSearchParamsPairs: [['p', '1']]
  });
  const sizeDescriptor = Object.getOwnPropertyDescriptor(URLSearchParams.prototype, 'size');
  const sizeGetter = sizeDescriptor.get;
  params.size = 99;

  return JSON.stringify({
    urlDescriptors: urlAttributeNames.map(name => descriptorShape(URL.prototype, url, name)),
    realUrl: [
      url.href,
      url.search,
      url.searchParams.get('a'),
      url.searchParams.get('b'),
      url.toString()
    ].join('|'),
    ownUrlSpoof: [
      hasOwn(url, '__moliUrlHref'),
      hasOwn(url, '__moliUrlSearchParams'),
      url.href,
      url.searchParams.get('a'),
      url.searchParams.get('b')
    ].join('|'),
    realParams: [
      params.toString(),
      params.getAll('x').join(','),
      params.size
    ].join('|'),
    sizeDescriptor: [
      typeof sizeDescriptor.get,
      sizeDescriptor.get.name,
      sizeDescriptor.get.length,
      stringify(sizeDescriptor.set),
      sizeDescriptor.enumerable,
      sizeDescriptor.configurable
    ].join('|'),
    sizeAssign: [
      hasOwn(params, 'size'),
      params.size
    ].join('|'),
    fakeUrl: [
      probe(() => fakeUrl.href),
      probe(() => fakeUrl.searchParams),
      probe(() => URL.prototype.toString.call(fakeUrl)),
      probe(() => URL.prototype.toJSON.call(fakeUrl)),
      probe(() => { fakeUrl.href = 'https://fake.test/'; })
    ].join('|'),
    fakeParams: [
      probe(() => URLSearchParams.prototype.get.call(fakeParams, 'p')),
      probe(() => URLSearchParams.prototype.toString.call(fakeParams)),
      probe(() => sizeGetter.call(fakeParams))
    ].join('|'),
    cloneParams: probe(() => structuredClone(params)),
    urlSlots,
    paramsSlots
  });
})()
"#,
        )
        .expect("URL declared slot spoofing probe should evaluate");

    assert_eq!(
        result,
        r#"{"urlDescriptors":["href:function:get href:0:function:set href:1:true:true:false","protocol:function:get protocol:0:function:set protocol:1:true:true:false","username:function:get username:0:function:set username:1:true:true:false","password:function:get password:0:function:set password:1:true:true:false","host:function:get host:0:function:set host:1:true:true:false","hostname:function:get hostname:0:function:set hostname:1:true:true:false","port:function:get port:0:function:set port:1:true:true:false","pathname:function:get pathname:0:function:set pathname:1:true:true:false","search:function:get search:0:function:set search:1:true:true:false","hash:function:get hash:0:function:set hash:1:true:true:false","origin:function:get origin:0:undefined:undefined:undefined:true:true:false","searchParams:function:get searchParams:0:undefined:undefined:undefined:true:true:false"],"realUrl":"https://example.test/path?a=1&b=2|?a=1&b=2|1|2|https://example.test/path?a=1&b=2","ownUrlSpoof":"true|true|https://example.test/path?a=1&b=2|1|2","realParams":"x=1&x=2&y=3|1,2|3","sizeDescriptor":"function|get size|0|undefined|true|true","sizeAssign":"false|3","fakeUrl":"throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError","fakeParams":"throw:TypeError|throw:TypeError|throw:TypeError","cloneParams":"throw:DataCloneError","urlSlots":"","paramsSlots":""}"#
    );
}

#[test]
fn url_static_parse_and_can_parse_stringify_undefined_input() {
    let mut vm = new_storage_test_vm("https://url-static-stringification.test/");

    let result = vm
        .eval(
            r#"
(() => {
  function probe(callback) {
    try {
      const value = callback();
      return value && value.href ? value.href : String(value);
    } catch (error) {
      return 'throw:' + error.name;
    }
  }
  const base = new URL('https://example.test/root/base/');
  const blobUrl = URL.createObjectURL(new Blob(['ok'], { type: 'text/plain' }));
  const staticMethodNames = ['parse', 'canParse', 'createObjectURL', 'revokeObjectURL'];
  const summarize = name => {
    const descriptor = Object.getOwnPropertyDescriptor(URL, name);
    return [
      !!descriptor,
      typeof descriptor?.value,
      descriptor?.value?.name,
      descriptor?.value?.length,
      descriptor?.enumerable,
      descriptor?.writable,
      descriptor?.configurable
    ].join(':');
  };
  return JSON.stringify({
    descriptors: staticMethodNames.map(summarize).join('|'),
    keys: Object.keys(URL).filter(name => staticMethodNames.includes(name)).join(','),
    parseWithoutBase: URL.parse(undefined),
    parseWithOpaqueBase: URL.parse(undefined, 'aaa:/b').href,
    canParseWithoutBase: URL.canParse(undefined),
    canParseWithOpaqueBase: URL.canParse(undefined, 'aaa:/b'),
    parseUrlObjectBase: URL.parse('child', base).href,
    canParseUrlObjectBase: URL.canParse('child', base),
    parseMissing: probe(() => URL.parse()),
    canParseMissing: probe(() => URL.canParse()),
    parseSymbolInput: probe(() => URL.parse(Symbol())),
    canParseSymbolBase: probe(() => URL.canParse('child', Symbol())),
    parseThrowingBase: probe(() => URL.parse('child', { toString() { throw new Error('base failed'); } })),
    revokeBlob: probe(() => URL.revokeObjectURL(blobUrl)),
    revokeMissing: probe(() => URL.revokeObjectURL()),
    revokeSymbol: probe(() => URL.revokeObjectURL(Symbol())),
    revokeNull: probe(() => URL.revokeObjectURL(null)),
  });
})()
"#,
        )
        .expect("URL static parse/canParse probe should evaluate");

    assert_eq!(
        result,
        r#"{"descriptors":"true:function:parse:1:true:true:true|true:function:canParse:1:true:true:true|true:function:createObjectURL:1:true:true:true|true:function:revokeObjectURL:1:true:true:true","keys":"parse,canParse,createObjectURL,revokeObjectURL","parseWithoutBase":null,"parseWithOpaqueBase":"aaa:/undefined","canParseWithoutBase":false,"canParseWithOpaqueBase":true,"parseUrlObjectBase":"https://example.test/root/base/child","canParseUrlObjectBase":true,"parseMissing":"throw:TypeError","canParseMissing":"throw:TypeError","parseSymbolInput":"throw:TypeError","canParseSymbolBase":"throw:TypeError","parseThrowingBase":"throw:Error","revokeBlob":"undefined","revokeMissing":"throw:TypeError","revokeSymbol":"throw:TypeError","revokeNull":"undefined"}"#
    );
}

#[test]
fn url_attribute_setters_use_usv_string_conversion() {
    let mut vm = new_storage_test_vm("https://url-attribute-setters-webidl.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return `throw:${error && error.name}`;
    }
  };
  const throwing = { toString() { throw new RangeError('stringify'); } };

  const href = new URL('https://example.test/root');
  const hrefBefore = href.href;
  const hrefSymbol = probe(() => { href.href = Symbol(); });

  const protocol = new URL('https://example.test/root');
  const protocolThrow = probe(() => { protocol.protocol = throwing; });

  const username = new URL('https://example.test/root');
  username.username = undefined;

  const password = new URL('https://user@example.test/root');
  password.password = null;

  const host = new URL('https://example.test/root');
  host.host = { toString() { return 'other.test:8443'; } };

  const search = new URL('https://example.test/root');
  search.search = undefined;

  const hash = new URL('https://example.test/root');
  hash.hash = undefined;

  const pathname = new URL('https://example.test/root');
  pathname.pathname = '\uD800';

  return JSON.stringify({
    hrefSymbol,
    hrefUnchanged: href.href === hrefBefore,
    protocolThrow,
    username: `${username.username}|${username.href}`,
    password: `${password.password}|${password.href}`,
    host: `${host.host}|${host.href}`,
    search: `${search.search}|${search.href}`,
    hash: `${hash.hash}|${hash.href}`,
    pathname: `${pathname.pathname}|${pathname.href}`,
  });
})()
"#,
        )
        .expect("URL writable attribute setters should parse WebIDL USVString values");

    assert_eq!(
        result,
        r##"{"hrefSymbol":"throw:TypeError","hrefUnchanged":true,"protocolThrow":"throw:RangeError","username":"undefined|https://undefined@example.test/root","password":"null|https://user:null@example.test/root","host":"other.test:8443|https://other.test:8443/root","search":"?undefined|https://example.test/root?undefined","hash":"#undefined|https://example.test/root#undefined","pathname":"/%EF%BF%BD|https://example.test/%EF%BF%BD"}"##
    );
}

#[test]
fn response_init_status_and_headers_getters_propagate_exceptions() {
    let mut vm = new_storage_test_vm("https://response-init-getters.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const statusMarker = { marker: 'status' };
  const headersMarker = { marker: 'headers' };
  let statusCaught = false;
  let headersCaught = false;
  try {
    new Response(null, {
      get status() {
        throw statusMarker;
      }
    });
  } catch (error) {
    statusCaught = error === statusMarker;
  }
  try {
    new Response(null, {
      get headers() {
        throw headersMarker;
      }
    });
  } catch (error) {
    headersCaught = error === headersMarker;
  }
  return JSON.stringify({ statusCaught, headersCaught });
})()
"#,
        )
        .expect("ResponseInit getter exception probe should evaluate");

    assert_eq!(result, r#"{"statusCaught":true,"headersCaught":true}"#);
}

#[test]
fn response_init_status_uses_unsigned_short_conversion() {
    let mut vm = new_storage_test_vm("https://response-init-status.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const wrapped = new Response(null, { status: 65737 });
  let negativeRange = false;
  try {
    new Response(null, { status: -1 });
  } catch (error) {
    negativeRange = error instanceof RangeError;
  }
  return JSON.stringify({ status: wrapped.status, ok: wrapped.ok, negativeRange });
})()
"#,
        )
        .expect("ResponseInit status conversion probe should evaluate");

    assert_eq!(result, r#"{"status":201,"ok":true,"negativeRange":true}"#);
}

#[test]
fn web_api_mime_surfaces_use_parser_normalization() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let result = vm
        .eval(
            r#"
            (() => {
                const blob = new Blob(["body"], { type: "Text/Plain; Charset=UTF-8" });
                const slice = blob.slice(0, 2, "Application/JSON");
                const detached = new DOMParser().parseFromString("<html><body><p id='x'></p></body></html>", "text/html");
                let domParserParameterized = "no-throw";
                try {
                    new DOMParser().parseFromString("<html></html>", "Text/HTML; Charset=UTF-8");
                } catch (error) {
                    domParserParameterized = error.name;
                }
                const video = document.createElement("video");
                const probe = callback => {
                    try {
                        return callback();
                    } catch (error) {
                        return error && error.name;
                    }
                };
                return JSON.stringify({
                    blobType: blob.type,
                    sliceType: slice.type,
                    domParserParagraph: detached?.getElementById("x")?.tagName ?? null,
                    domParserParameterized,
                    canPlayType: video.canPlayType("Video/MP4; codecs=\"avc1.42E01E\""),
                    canPlayTypeObject: video.canPlayType({ toString() { return "audio/flac"; } }),
                    canPlayTypeParameterizedAudio: video.canPlayType("Audio/MPEG; charset=utf-8"),
                    canPlayTypeMissing: probe(() => video.canPlayType()),
                    canPlayTypeSymbol: probe(() => video.canPlayType(Symbol("type"))),
                });
            })()
            "#,
        )
        .expect("MIME parser-backed Web API probe should evaluate");

    assert_eq!(
        result,
        r#"{"blobType":"text/plain; charset=utf-8","sliceType":"application/json","domParserParagraph":"P","domParserParameterized":"TypeError","canPlayType":"probably","canPlayTypeObject":"maybe","canPlayTypeParameterizedAudio":"probably","canPlayTypeMissing":"TypeError","canPlayTypeSymbol":"TypeError"}"#
    );
}

#[test]
fn response_body_consumers_use_shared_content_type_helpers() {
    let mut vm = new_storage_test_vm("https://response-body-mime.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__responseBodyMimeProbe = null;
          Promise.all([
            new Response("a=1&b=two", {
              headers: [["Content-Type", " application/x-www-form-urlencoded;charset=UTF-8 "]]
            }).formData().then((formData) => Array.from(formData).join("|")),
            (() => {
              const formData = new FormData();
              formData.append("field", "value");
              formData.append("json", new Blob(['{"ok":true}'], { type: "Application/JSON" }));
              return new Response(formData).formData().then(async parsed => {
                const file = parsed.get("json");
                return [
                  parsed.get("field"),
                  file instanceof File,
                  file.name,
                  file.type,
                  await file.text()
                ].join("|");
              });
            })(),
            (() => {
              const formData = new FormData();
              formData.append("foo", new Blob(['{"bar":"baz"}'], { type: "application/json" }));
              return new Response(formData).blob().then(async blob => {
                const body = (await blob.text()).toLowerCase();
                const parsed = await new Response(body, {
                  headers: [["Content-Type", blob.type.toLowerCase()]]
                }).formData();
                return parsed.get("foo").type;
              });
            })(),
            new Response("body", {
              headers: [["Content-Type", "Text/Plain; Charset=UTF-8"]]
            }).blob().then((blob) => blob.type),
            new Response("body", {
              headers: [["Content-Type", "text/plain"], ["Content-Type", "application/json"]]
            }).blob().then((blob) => blob.type)
          ]).then((values) => {
            globalThis.__responseBodyMimeProbe = values;
          });
        })()
        "#,
    )
    .expect("Response body MIME probe should evaluate");

    let result = vm
        .eval("JSON.stringify(globalThis.__responseBodyMimeProbe)")
        .expect("Response body MIME promise chain should settle");

    assert_eq!(
        result,
        r#"["a,1|b,two","value|true|blob|application/json|{\"ok\":true}","application/json","text/plain; charset=utf-8","text/plain, application/json"]"#
    );
}

#[test]
fn response_body_consumers_materialize_readable_stream_chunks() {
    let mut vm = new_storage_test_vm("https://response-body-stream.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__responseBodyStreamProbe = [];
          const stream = new ReadableStream({
            start(controller) {
              Promise.resolve()
                .then(() => controller.enqueue(new Uint8Array([1, 2])))
                .then(() => controller.enqueue(new Uint8Array([3])))
                .then(() => controller.close());
            }
          });
          const response = new Response(stream);
          __responseBodyStreamProbe.push(`body:${response.body === stream}:${response.bodyUsed}`);
          const bodyPromise = response.arrayBuffer().then(
            (buffer) => __responseBodyStreamProbe.push(
              `bytes:${Array.from(new Uint8Array(buffer)).join(",")}:${response.bodyUsed}`
            ),
            (error) => __responseBodyStreamProbe.push(`bytes-error:${error.constructor.name}`)
          );
          __responseBodyStreamProbe.push(`after-call:${response.bodyUsed}`);

          const invalidChunk = new ReadableStream({
            start(controller) {
              controller.enqueue(new Uint8Array([9]).buffer);
              controller.close();
            }
          });
          const invalidPromise = new Response(invalidChunk).arrayBuffer().then(
            () => __responseBodyStreamProbe.push("invalid:resolved"),
            (error) => __responseBodyStreamProbe.push(`invalid:${error.constructor.name}`)
          );
          Promise.allSettled([bodyPromise, invalidPromise]).then(() => {
            __responseBodyStreamProbe.push("settled");
          });
        })()
        "#,
    )
    .expect("Response stream body probe should evaluate");

    for _ in 0..8 {
        vm.eval("0")
            .expect("Response stream body promise chain should drain");
    }
    let result = vm
        .eval("JSON.stringify(globalThis.__responseBodyStreamProbe.sort())")
        .expect("Response stream body probe result should evaluate");

    assert_eq!(
        result,
        r#"["after-call:true","body:true:false","bytes:1,2,3:true","invalid:TypeError","settled"]"#
    );
}

#[test]
fn response_clone_tees_user_readable_stream_body() {
    let mut vm = new_storage_test_vm("https://response-clone-stream.test/");

    vm.eval(
        r#"
        (() => {
          globalThis.__responseCloneStreamProbe = [];
          const stream = new ReadableStream({
            start(controller) {
              controller.enqueue(new Uint8Array([65, 66]));
              controller.close();
            }
          });
          const response = new Response(stream);
          const clone = response.clone();
          __responseCloneStreamProbe.push([
            "body",
            response.body !== stream,
            clone.body instanceof ReadableStream,
            response.body !== clone.body,
            response.bodyUsed,
            clone.bodyUsed
          ].join(":"));
          Promise.all([response.text(), clone.text()]).then(
            ([originalText, cloneText]) => {
              __responseCloneStreamProbe.push([
                "text",
                originalText,
                cloneText,
                response.bodyUsed,
                clone.bodyUsed
              ].join(":"));
            },
            (error) => __responseCloneStreamProbe.push(`error:${error.constructor.name}`)
          );

          const asyncStream = new ReadableStream({
            start(controller) {
              Promise.resolve()
                .then(() => controller.enqueue(new Uint8Array([67])))
                .then(() => controller.enqueue(new Uint8Array([68])))
                .then(() => controller.close());
            }
          });
          const asyncResponse = new Response(asyncStream);
          const asyncClone = asyncResponse.clone();
          Promise.all([asyncResponse.text(), asyncClone.text()]).then(
            ([originalText, cloneText]) => {
              __responseCloneStreamProbe.push(`async:${originalText}:${cloneText}`);
            },
            (error) => __responseCloneStreamProbe.push(`async-error:${error.constructor.name}`)
          );
        })()
        "#,
    )
    .expect("Response clone stream setup should evaluate");

    for _ in 0..12 {
        vm.eval("0")
            .expect("Response clone stream promise chain should drain");
    }
    let result = vm
        .eval("JSON.stringify(globalThis.__responseCloneStreamProbe.sort())")
        .expect("Response clone stream probe result should evaluate");

    assert_eq!(
        result,
        r#"["async:CD:CD","body:true:true:true:false:false","text:AB:AB:true:true"]"#
    );
}

#[test]
fn response_clone_tees_pending_network_body_after_parent_consumption() {
    let mut vm = new_storage_test_vm("https://response-clone-pending-stream.test/");
    let body_source_id = crate::network_host::new_network_body_source_id();
    let document_url = Url::parse("https://response-clone-pending-stream.test/")
        .expect("document URL should parse");
    let response_url = Url::parse("https://response-clone-pending-stream.test/data.json")
        .expect("response URL should parse");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let response =
                crate::network_host::build_fetch_response_object_from_stream_for_request_mode(
                    scope,
                    &document_url,
                    moli_fetch::RequestMode::Cors,
                    moli_fetch::ResponseHead {
                        final_url: response_url,
                        status: 200,
                        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
                        request_cookie_report: None,
                        cookie_set_reports: Vec::new(),
                        redirected: false,
                        redirect_chain: Vec::new(),
                        from_cache: false,
                        negotiated_http_version: None,
                    },
                    body_source_id,
                );
            let global = context.global(scope);
            let _ = global.set(
                scope,
                v8str(scope, "__pendingFetchResponse").into(),
                response.into(),
            );
            Ok(())
        })
        .expect("pending fetch response should be installed");

    vm.eval(
        r#"
(() => {
  globalThis.__pendingFetchCloneProbe = [];
  const original = globalThis.__pendingFetchResponse;
  const firstClone = original.clone();
  const secondClone = firstClone.clone();
  const record = label => value => {
    globalThis.__pendingFetchCloneProbe.push(`${label}:${value}`);
  };
  const recordError = label => error => {
    globalThis.__pendingFetchCloneProbe.push(
      `${label}-error:${error && error.constructor && error.constructor.name}:${error && error.message}`
    );
  };
  Promise.allSettled([
    original.text().then(record("original"), recordError("original")),
    firstClone.text().then(record("first"), recordError("first"))
  ]).then(() => {
    secondClone.text().then(record("second"), recordError("second"));
  });
})()
"#,
    )
    .expect("pending fetch clone fan-out should evaluate");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            crate::network_host::enqueue_pending_network_body_chunk(
                scope,
                body_source_id,
                br#"{"ok":true}"#.to_vec(),
            );
            crate::network_host::close_pending_network_body_stream(scope, body_source_id);
            Ok(())
        })
        .expect("pending fetch response body should close");

    for _ in 0..12 {
        vm.eval("0")
            .expect("pending fetch clone promise chain should drain");
    }
    let result = vm
        .eval("JSON.stringify(globalThis.__pendingFetchCloneProbe.sort())")
        .expect("pending fetch clone probe result should evaluate");

    assert_eq!(
        result,
        r#"["first:{\"ok\":true}","original:{\"ok\":true}","second:{\"ok\":true}"]"#
    );
}

#[test]
fn pending_fetch_body_pipe_through_text_decoder_stream_pulls_future_chunks() {
    let mut vm = new_storage_test_vm("https://pending-fetch-pipe-through.test/");
    let body_source_id = crate::network_host::new_network_body_source_id();
    let document_url =
        Url::parse("https://pending-fetch-pipe-through.test/").expect("document URL should parse");
    let response_url = Url::parse("https://pending-fetch-pipe-through.test/stream.txt")
        .expect("response URL should parse");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let response =
                crate::network_host::build_fetch_response_object_from_stream_for_request_mode(
                    scope,
                    &document_url,
                    moli_fetch::RequestMode::Cors,
                    moli_fetch::ResponseHead {
                        final_url: response_url,
                        status: 200,
                        headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                        request_cookie_report: None,
                        cookie_set_reports: Vec::new(),
                        redirected: false,
                        redirect_chain: Vec::new(),
                        from_cache: false,
                        negotiated_http_version: None,
                    },
                    body_source_id,
                );
            let global = context.global(scope);
            let _ = global.set(
                scope,
                v8str(scope, "__pendingPipeResponse").into(),
                response.into(),
            );
            Ok(())
        })
        .expect("pending fetch response should be installed");

    vm.eval(
        r#"
(() => {
  globalThis.__pendingPipeEvents = [];
  const reader = globalThis.__pendingPipeResponse.body
    .pipeThrough(new TextDecoderStream())
    .getReader();
  (async () => {
    for (;;) {
      const { value, done } = await reader.read();
      globalThis.__pendingPipeEvents.push(done ? "done" : `chunk:${value}`);
      if (done) break;
    }
  })().then(
    () => globalThis.__pendingPipeEvents.push("settled"),
    error => globalThis.__pendingPipeEvents.push(`error:${error && error.constructor && error.constructor.name}`)
  );
})()
"#,
    )
    .expect("pending fetch pipeThrough setup should evaluate");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            crate::network_host::enqueue_pending_network_body_chunk(
                scope,
                body_source_id,
                b"O".to_vec(),
            );
            Ok(())
        })
        .expect("first pending fetch response body chunk should enqueue");

    for _ in 0..4 {
        vm.eval("0")
            .expect("first pending pipeThrough chunk should drain");
    }
    let first = vm
        .eval("JSON.stringify(globalThis.__pendingPipeEvents)")
        .expect("first pending pipeThrough result should evaluate");
    assert_eq!(first, r#"["chunk:O"]"#);

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            crate::network_host::enqueue_pending_network_body_chunk(
                scope,
                body_source_id,
                b"K".to_vec(),
            );
            crate::network_host::close_pending_network_body_stream(scope, body_source_id);
            Ok(())
        })
        .expect("second pending fetch response body chunk should enqueue and close");

    for _ in 0..8 {
        vm.eval("0")
            .expect("pending pipeThrough close should drain");
    }
    let result = vm
        .eval("JSON.stringify(globalThis.__pendingPipeEvents)")
        .expect("pending pipeThrough result should evaluate");

    assert_eq!(result, r#"["chunk:O","chunk:K","done","settled"]"#);
}

#[test]
fn materialize_response_object_preserves_redirected_slot() {
    let vm = new_storage_test_vm("https://response-materialize-redirected.test/");
    let document_url = Url::parse("https://response-materialize-redirected.test/")
        .expect("document URL should parse");
    let final_url = Url::parse("https://response-materialize-redirected.test/final.txt")
        .expect("final URL should parse");
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;

    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let response = crate::network_host::build_fetch_response_object_for_request_mode(
                scope,
                &document_url,
                moli_fetch::RequestMode::Cors,
                moli_fetch::Response::from_head_and_text_body(
                    moli_fetch::ResponseHead {
                        final_url: final_url.clone(),
                        status: 200,
                        headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                        request_cookie_report: None,
                        cookie_set_reports: Vec::new(),
                        redirected: true,
                        redirect_chain: Vec::new(),
                        from_cache: false,
                        negotiated_http_version: None,
                    },
                    "redirected-body".to_owned(),
                ),
            );
            let materialized =
                crate::network_host::materialize_response_object(scope, response.into(), "test")
                    .expect("materialized response should be accepted");
            assert_eq!(materialized.final_url.as_ref(), Some(&final_url));
            assert_eq!(materialized.response_type, "basic");
            assert!(materialized.redirected);
            assert_eq!(materialized.status, 200);
            assert_eq!(materialized.body, b"redirected-body".to_vec());
            Ok(())
        })
        .expect("redirected response should materialize");
}

#[test]
fn filtered_response_materialization_preserves_internal_url_without_exposing_url() {
    let mut vm = new_storage_test_vm("https://response-materialize-filtered.test/");
    let document_url = Url::parse("https://response-materialize-filtered.test/")
        .expect("document URL should parse");
    let final_url = Url::parse("https://cross-response-materialize-filtered.test/redirect-start")
        .expect("final URL should parse");
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;

    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate({
            let final_url = final_url.clone();
            move |isolate| {
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let response = crate::network_host::build_fetch_response_object_from_body_source_for_request_mode_with_filter(
                    scope,
                    &document_url,
                    moli_fetch::RequestMode::Cors,
                    moli_fetch::ResponseHead {
                        final_url: final_url.clone(),
                        status: 302,
                        headers: vec![("location".to_owned(), "target.html".to_owned())],
                        request_cookie_report: None,
                        cookie_set_reports: Vec::new(),
                        redirected: false,
                        redirect_chain: Vec::new(),
                        from_cache: false,
                        negotiated_http_version: None,
                    },
                    moli_fetch::ResponseBody::materialized_bytes(Vec::new()),
                    Some(crate::types::AsyncSubresourceFetchResponseFilter::OpaqueRedirect),
                );
                let global = context.global(scope);
                let _ = global.set(
                    scope,
                    v8str(scope, "__filteredResponse").into(),
                    response.into(),
                );
                let (head, _) =
                    crate::network_host::materialize_response_object_head(
                        scope,
                        response.into(),
                        "test",
                    )
                    .expect("filtered response head should materialize with internal URL");
                assert_eq!(head.final_url.as_ref(), Some(&final_url));
                assert_eq!(head.response_type, "opaqueredirect");
                assert_eq!(head.status, 0);
                Ok(())
            }
        })
        .expect("filtered response should install");

    let visible_url = vm
        .eval("globalThis.__filteredResponse.url")
        .expect("filtered response visible URL should evaluate");
    assert_eq!(visible_url, "");

    vm.exec(
        r#"
        globalThis.__filteredResponseClone = globalThis.__filteredResponse.clone();
        globalThis.__filteredResponseCacheClone = globalThis.__filteredResponse.clone();
        globalThis.__filteredResponseCacheProbe = "pending";
        (async () => {
          const bucket = await navigator.storageBuckets.open("filtered-response-url");
          const cache = await bucket.caches.open("responses");
          await cache.put("redirect", globalThis.__filteredResponseCacheClone);
          globalThis.__filteredResponseCached = await cache.match("redirect");
          await navigator.storageBuckets.delete("filtered-response-url");
          globalThis.__filteredResponseCacheProbe = [
            globalThis.__filteredResponseCached.type,
            globalThis.__filteredResponseCached.status,
            globalThis.__filteredResponseCached.url === "",
            globalThis.__filteredResponseCached.body === null
          ].join("|");
        })().catch(error => {
          globalThis.__filteredResponseCacheProbe =
            "error:" + String(error && error.name) + ":" + String(error && error.message);
        });
        "#,
        None,
    )
    .expect("filtered response cache roundtrip should schedule");

    let cache_probe = vm
        .eval("String(globalThis.__filteredResponseCacheProbe)")
        .expect("filtered response cache roundtrip should settle");
    assert_eq!(cache_probe, "opaqueredirect|0|true|true");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let global = context.global(scope);
            let clone = global
                .get(scope, v8str(scope, "__filteredResponseClone").into())
                .expect("filtered response clone should exist");
            let materialized_clone =
                crate::network_host::materialize_response_object(scope, clone, "clone")
                    .expect("filtered response clone should preserve internal URL");
            assert_eq!(materialized_clone.final_url.as_ref(), Some(&final_url));
            assert_eq!(materialized_clone.response_type, "opaqueredirect");

            let cached = global
                .get(scope, v8str(scope, "__filteredResponseCached").into())
                .expect("cached filtered response should exist");
            let materialized_cached =
                crate::network_host::materialize_response_object(scope, cached, "cache")
                    .expect("cached filtered response should preserve internal URL");
            assert_eq!(materialized_cached.final_url.as_ref(), Some(&final_url));
            assert_eq!(materialized_cached.response_type, "opaqueredirect");
            Ok(())
        })
        .expect("filtered response clone/cache should materialize");
}

#[test]
fn materialize_response_object_rejects_locked_response_body() {
    let mut vm = new_storage_test_vm("https://response-materialize-locked.test/");
    let state = vm
        .eval(
            r#"
(() => {
  const response = new Response(new ReadableStream({
    start(controller) {
      controller.enqueue(new Uint8Array([1]));
    }
  }));
  globalThis.__lockedMaterializeResponse = response;
  globalThis.__lockedMaterializeReader = response.body.getReader();
  return String(response.body.locked) + "|" + String(response.bodyUsed);
})()
"#,
        )
        .expect("locked response setup should evaluate");
    assert_eq!(state, "true|false");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let global = context.global(scope);
            let response = global
                .get(scope, v8str(scope, "__lockedMaterializeResponse").into())
                .expect("locked response should be installed");
            let error = crate::network_host::materialize_response_object(scope, response, "test")
                .expect_err("locked response body should be rejected");
            assert_eq!(error, "test rejected a Response whose body is locked.");
            Ok(())
        })
        .expect("locked response should reject materialization");
}
