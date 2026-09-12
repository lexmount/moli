use super::*;

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
fn url_parsing_apis_reject_invalid_bases_for_absolute_inputs() {
    let mut vm = new_storage_test_vm("https://url-invalid-base.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const invalidBases = [
    '', '/relative', 'relative', 'http://', 'https://example.test:bogus/',
    'http://[::1', 'file://example:1/', null, false,
  ];
  for (const input of ['about:blank', 'https://example.test/', 'data:text/plain,x', 'child']) {
    for (const base of invalidBases) {
      let error;
      try { new URL(input, base); } catch (caught) { error = caught; }
      assert(error instanceof TypeError, `constructor rejects invalid base ${base} for ${input}`);
      assert(URL.parse(input, base) === null, `parse rejects invalid base ${base} for ${input}`);
      assert(URL.canParse(input, base) === false, `canParse rejects invalid base ${base} for ${input}`);
    }
  }
  return 'ok';
})()
"#,
        )
        .expect("all URL parsing APIs must validate a supplied base");
    assert_eq!(result, "ok");
}

#[test]
fn url_parsing_apis_resolve_same_scheme_inputs_against_the_base() {
    let mut vm = new_storage_test_vm("https://url-same-scheme-base.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const cases = [
    ['https:child', 'https://u:p@example.test:8443/dir/base?old#old', 'https://u:p@example.test:8443/dir/child'],
    ['HTTPS:../next', 'https://example.test/dir/base', 'https://example.test/next'],
    ['https:?next', 'https://example.test/dir/base?old#old', 'https://example.test/dir/base?next'],
    ['https:#next', 'https://example.test/dir/base?old#old', 'https://example.test/dir/base?old#next'],
    ['ftp:child', 'ftp://example.test/dir/base', 'ftp://example.test/dir/child'],
    ['file:child', 'file:///dir/base', 'file:///dir/child'],
    ['https:child', 'http://example.test/dir/base', 'https://child/'],
    ['https:child', undefined, 'https://child/'],
    ['about:blank', 'https://example.test/', 'about:blank'],
    ['data:text/plain,x', 'about:blank', 'data:text/plain,x'],
    ['#new', 'about:blank?old#old', 'about:blank?old#new'],
    ['https://other.test/x', 'https://example.test/dir/base', 'https://other.test/x'],
  ];
  for (const [input, base, expected] of cases) {
    assert(new URL(input, base).href === expected, `constructor resolves ${input} against ${base}`);
    const parsed = URL.parse(input, base);
    assert(parsed instanceof URL && parsed.href === expected, `parse resolves ${input} against ${base}`);
    assert(URL.canParse(input, base), `canParse accepts ${input} against ${base}`);
    assert(parsed.searchParams.toString() === new URL(expected).searchParams.toString(), 'resolved query initializes URLSearchParams');
  }
  return 'ok';
})()
"#,
        )
        .expect("URL parsing must use the base even when the input includes a scheme");
    assert_eq!(result, "ok");
}

#[test]
fn url_parsing_apis_convert_arguments_before_parsing() {
    let mut vm = new_storage_test_vm("https://url-base-conversion-order.test/");
    let result = vm
        .eval(
            r#"
(() => {
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  for (const parse of [(...args) => new URL(...args), URL.parse, URL.canParse]) {
    const order = [];
    const value = (name, text) => ({
      [Symbol.toPrimitive](hint) {
        assert(hint === 'string', 'URL arguments use string conversion');
        order.push(name);
        return text;
      },
    });
    parse(value('input', 'https://example.test/'), value('base', 'https://base.test/'));
    assert(order.join(',') === 'input,base', 'input then base are each converted once');
    order.length = 0;
    const marker = new RangeError('conversion marker');
    let error;
    try {
      parse(value('input', 'http://['), {
        toString() { order.push('base'); throw marker; },
      });
    } catch (caught) { error = caught; }
    assert(error === marker && order.join(',') === 'input,base', 'base conversion errors precede URL parsing');
    order.length = 0;
    error = undefined;
    try {
      parse({ toString() { throw marker; } }, value('base', 'https://base.test/'));
    } catch (caught) { error = caught; }
    assert(error === marker && order.length === 0, 'failed input conversion does not touch the base');
    const withoutBase = parse('https://example.test/');
    const undefinedBase = parse('https://example.test/', undefined);
    assert(String(withoutBase) === String(undefinedBase), 'undefined base is equivalent to omission');
  }
  return 'ok';
})()
"#,
        )
        .expect("URL argument conversion order must be preserved");
    assert_eq!(result, "ok");
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
        r#"["a,1|b,two","value|true|blob|application/json|{\"ok\":true}","application/json","text/plain;charset=UTF-8","application/json"]"#
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

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_runtime.context as *const _;
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
                    crate::network_host::FetchResponseRequest {
                        redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                        method: "GET",
                        mode: moli_fetch::RequestMode::Cors,
                    },
                    moli_fetch::ResponseHead {
                        status_text: None,
                        final_url: response_url,
                        status: 200,
                        headers: vec![("content-type".to_owned(), b"application/json".to_vec())],
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

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_runtime.context as *const _;
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

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_runtime.context as *const _;
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
                    crate::network_host::FetchResponseRequest {
                        redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                        method: "GET",
                        mode: moli_fetch::RequestMode::Cors,
                    },
                    moli_fetch::ResponseHead {
                        status_text: None,
                        final_url: response_url,
                        status: 200,
                        headers: vec![("content-type".to_owned(), b"text/plain".to_vec())],
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

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_runtime.context as *const _;
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

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_runtime.context as *const _;
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
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_runtime.context as *const _;

    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let response = crate::network_host::build_fetch_response_object_for_request_mode(
                scope,
                &document_url,
                crate::network_host::FetchResponseRequest {
                    redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                    method: "GET",
                    mode: moli_fetch::RequestMode::Cors,
                },
                moli_fetch::Response::from_head_and_text_body(
                    moli_fetch::ResponseHead {
                        status_text: None,
                        final_url: final_url.clone(),
                        status: 200,
                        headers: vec![("content-type".to_owned(), b"text/plain".to_vec())],
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

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_runtime.context as *const _;
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

#[test]
fn fetch_body_text_removes_one_initial_utf8_bom() {
    assert_body_utf8_bom_probe("text");
}

#[test]
fn fetch_body_json_removes_initial_utf8_bom_and_preserves_syntax_errors() {
    assert_body_utf8_bom_probe("json");
}

#[test]
fn fetch_body_binary_and_form_methods_preserve_bom_bytes() {
    assert_body_utf8_bom_probe("bytes");
}

#[test]
fn webidl_sequences_propagate_abrupt_completion_without_closing_iterators() {
    let mut vm = new_storage_test_vm("https://sequence-abrupt.test/");
    let result = vm.eval(r#"
(() => {
  const check = (condition, label) => { if (!condition) throw new Error(label); };
  const consumers = [
    ['USVString', input => new URLSearchParams([input])],
    ['DOMString', input => new PerformanceObserver(() => {}).observe({entryTypes: input})]
  ];
  if (typeof IntersectionObserver === 'function') consumers.push(
    ['double', input => new IntersectionObserver(() => {}, {threshold: input})]);
  for (const [name, consume] of consumers) {
    for (const stage of ['next', 'done', 'value', 'convert', 'symbol']) {
      const marker = {};
      const log = [];
      const fail = () => { log.push(stage); throw marker; };
      const input = {[Symbol.iterator]() {
        let finished = false;
        return {
          next() {
            if (finished) return {done: true};
            finished = true;
            if (stage === 'next') fail();
            return {
              get done() { if (stage === 'done') fail(); return false; },
              get value() {
                if (stage === 'value') fail();
                return stage === 'symbol' ? Symbol() : {[Symbol.toPrimitive]: fail};
              }
            };
          },
          get return() { log.push('get:return'); throw new Error('return must not be read'); }
        };
      }};
      let caught;
      try { consume(input); } catch (error) { caught = error; }
      check(stage === 'symbol' ? caught instanceof TypeError : caught === marker, name + ' ' + stage + ' exception');
      const expected = stage === 'symbol' ? [] : [stage];
      check(JSON.stringify(log) === JSON.stringify(expected), name + ' ' + stage + ': ' + JSON.stringify(log));
    }
  }
  return 'ok';
})()
"#).unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn webidl_nested_and_interface_sequences_do_not_read_iterator_return_on_errors() {
    let mut vm = new_storage_test_vm("https://nested-sequence-abrupt.test/");
    let result = vm.eval(r#"
(() => {
  const check = (condition, label) => { if (!condition) throw new Error(label); };
  for (const stage of ['iterator', 'next', 'value', 'convert']) {
    const marker = {};
    let returnReads = 0;
    const fail = () => { throw marker; };
    const wrap = value => ({[Symbol.iterator]() {
      let finished = false;
      return {
        next() {
          if (finished) return {done: true};
          finished = true;
          return {done: false, value};
        },
        get return() { returnReads++; throw new Error('outer return'); }
      };
    }});
    const pair = {get [Symbol.iterator]() {
      if (stage === 'iterator') fail();
      return function() {
        let finished = false;
        return {
          next() {
            if (finished) return {done: true};
            finished = true;
            if (stage === 'next') fail();
            return {done: false, get value() {
              if (stage === 'value') fail();
              return {[Symbol.toPrimitive]: fail};
            }};
          },
          get return() { returnReads++; throw new Error('inner return'); }
        };
      };
    }};
    let caught;
    try { new URLSearchParams(wrap(pair)); } catch (error) { caught = error; }
    check(caught === marker && returnReads === 0, stage + ' must propagate without closing either iterator');
  }
  for (const member of ['coalescedEvents', 'predictedEvents']) {
    let returnReads = 0;
    const events = {[Symbol.iterator]() {
      let finished = false;
      return {
        next() {
          if (finished) return {done: true};
          finished = true;
          return {done: false, value: new Event('invalid')};
        },
        get return() { returnReads++; throw new Error('interface sequence return'); }
      };
    }};
    let caught;
    try { new PointerEvent('pointermove', {[member]: events}); } catch (error) { caught = error; }
    check(caught instanceof TypeError && returnReads === 0, member + ' must reject the interface without closing');
  }
  return 'ok';
})()
"#).unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn initializer_sequences_convert_all_entries_before_validating_pairs() {
    let mut vm = new_storage_test_vm("https://initializer-sequence.test/");
    let result = vm.eval(r#"
(() => {
  const check = (condition, label) => { if (!condition) throw new Error(label); };
  const consumers = [
    ['Headers', input => new Headers(input)],
    ['URLSearchParams', input => new URLSearchParams(input)],
    ['Request', input => new Request('https://initializer-sequence.test/', {headers: input})],
    ['Response', input => new Response(null, {headers: input})]
  ];
  for (const [name, consume] of consumers) {
    const invalidPairs = [['short'], ['key', 'value', 'extra']];
    if (name !== 'URLSearchParams') invalidPairs.push(['', 'value'], ['key', 'bad\nvalue']);
    for (const bad of invalidPairs) {
      for (const stage of ['complete', 'next', 'done', 'value', 'convert']) {
        const marker = {};
        const log = [];
        const fail = () => { log.push('fail:' + stage); throw marker; };
        const input = {[Symbol.iterator]() {
          let index = 0;
          return {
            next() {
              log.push('next:' + index);
              if (index++ === 0) return {done: false, value: bad};
              if (index === 2) {
                if (stage === 'next') fail();
                return {
                  get done() { if (stage === 'done') fail(); return false; },
                  get value() {
                    if (stage === 'value') fail();
                    return ['later', {toString() {
                      if (stage === 'convert') fail();
                      log.push('convert:later');
                      return 'value';
                    }}];
                  }
                };
              }
              return {done: true};
            },
            get return() { log.push('return'); throw new Error('return must not be read'); }
          };
        }};
        let caught;
        try { consume(input); } catch (error) { caught = error; }
        const label = name + ' ' + JSON.stringify(bad) + ' ' + stage;
        check(stage === 'complete' ? caught instanceof TypeError : caught === marker, label + ' exception');
        const expected = stage === 'complete' ? ['next:0', 'next:1', 'convert:later', 'next:2'] :
          ['next:0', 'next:1', 'fail:' + stage];
        check(JSON.stringify(log) === JSON.stringify(expected), label + ': ' + JSON.stringify(log));
      }
    }
  }
  return 'ok';
})()
"#).unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn initializer_pairs_use_inner_iterators_and_convert_extra_elements() {
    let mut vm = new_storage_test_vm("https://initializer-inner-sequence.test/");
    let result = vm.eval(r#"
(() => {
  const check = (condition, label) => { if (!condition) throw new Error(label); };
  for (const Constructor of [Headers, URLSearchParams]) {
    let iteratorReads = 0;
    const pair = {
      get length() { throw new Error('length must not be read'); },
      get 0() { throw new Error('indexed properties must not be read'); },
      get [Symbol.iterator]() {
        iteratorReads++;
        return function*() {
          check(this === pair, 'inner iterator receiver');
          yield 'X-Key';
          yield ' one ';
        };
      }
    };
    const expected = Constructor === Headers ? [['x-key', 'one']] : [['X-Key', ' one ']];
    const actual = Array.from(new Constructor([pair]));
    check(JSON.stringify(actual) === JSON.stringify(expected) && iteratorReads === 1, Constructor.name + ' inner iteration');
    for (const invalid of ['ab', {0: 'key', 1: 'value', length: 2}]) {
      let caught;
      try { new Constructor([invalid]); } catch (error) { caught = error; }
      check(caught instanceof TypeError, Constructor.name + ' requires object iterables');
    }
    const marker = {};
    let conversions = 0;
    let caught;
    try {
      new Constructor([['key', 'value', {toString() { conversions++; throw marker; }}]]);
    } catch (error) { caught = error; }
    check(caught === marker && conversions === 1, Constructor.name + ' must convert the extra element');

    const log = [];
    const input = {[Symbol.iterator]() {
      let index = 0;
      return {next() {
        log.push('next:' + index);
        if (index++ > 0) return {done: true};
        return {done: false, value: ['key', 'value', {toString() {
          log.push('extra');
          return '\u0100';
        }}]};
      }};
    }};
    caught = undefined;
    try { new Constructor(input); } catch (error) { caught = error; }
    check(caught instanceof TypeError, Constructor.name + ' rejects the invalid initializer');
    const expectedLog = Constructor === Headers ? ['next:0', 'extra'] : ['next:0', 'extra', 'next:1'];
    check(JSON.stringify(log) === JSON.stringify(expectedLog), Constructor.name + ': ' + JSON.stringify(log));
  }
  return 'ok';
})()
"#).unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn webidl_initializer_unions_convert_platform_objects_without_iterators_as_records() {
    let mut vm = new_storage_test_vm("https://initializer-record.test/");
    let result = vm.eval(r#"
(() => {
  for (const Constructor of [Headers, URLSearchParams]) {
    const url = new URL('https://initializer-record.test/path?query=value');
    Object.defineProperty(url, Symbol.toPrimitive, {value() {
      throw new Error('URL object must use record conversion');
    }});
    if (Array.from(new Constructor(url)).length !== 0) throw new Error('empty URL record');
    url['x-record'] = 'record';
    if (JSON.stringify(Array.from(new Constructor(url))) !== '[["x-record","record"]]') {
      throw new Error(Constructor.name + ' must use own URL properties');
    }
    for (const factory of [() => new Headers(), () => new URLSearchParams(), () => new FormData()]) {
      for (const value of [undefined, null]) {
        const input = factory();
        let iteratorReads = 0;
        Object.defineProperty(input, Symbol.iterator, {enumerable: true, get() {
          iteratorReads++;
          return value;
        }});
        let caught;
        try { new Constructor(input); } catch (error) { caught = error; }
        if (!(caught instanceof TypeError) || iteratorReads !== 1) {
          throw new Error(Constructor.name + ' must reject the enumerable Symbol key before reading its value');
        }
      }
    }
  }
  return 'ok';
})()
"#).unwrap();
    assert_eq!(result, "ok");
}

#[test]
fn fetched_null_bodies_discard_payloads_without_registering_pending_streams() {
    use crate::network_host::{FetchResponseRequest, MaterializedResponseBody};

    let vm = new_storage_test_vm("https://null-response.test/");
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_runtime.context as *const _;
    let host = vm._context_host.clone();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let document_url = Url::parse("https://null-response.test/").unwrap();
            for (method, status) in [
                ("HEAD", 200), ("HEAD", 302), ("CONNECT", 200),
                ("GET", 101), ("GET", 103), ("GET", 204), ("GET", 205), ("GET", 304),
            ] {
                for opaque in [false, true] {
                    for source in ["response", "bytes", "subresource", "stream", "preload"] {
                        let request = FetchResponseRequest {
redirect_mode: moli_fetch::RequestRedirectMode::Follow,
                            method,
                            mode: if opaque { moli_fetch::RequestMode::NoCors } else { moli_fetch::RequestMode::Cors },
                        };
                        let head = moli_fetch::ResponseHead {
                            status_text: None,
                            final_url: Url::parse("https://cross-null-response.test/data").unwrap(),
                            status,
                            headers: vec![("content-type".to_owned(), b"text/plain".to_vec())],
                            request_cookie_report: None,
                            cookie_set_reports: Vec::new(),
                            redirected: false,
                            redirect_chain: Vec::new(),
                            from_cache: false,
                            negotiated_http_version: None,
                        };
                        let id = crate::network_host::new_network_body_source_id();
                        let response = match source {
                            "response" => crate::network_host::build_fetch_response_object_for_request_mode(
                                scope, &document_url, request,
                                moli_fetch::Response::from_head_and_text_body(head, "discard me".to_owned()),
                            ),
                            "bytes" => crate::network_host::build_fetch_response_object_from_body_source_for_request_mode_with_filter(
                                scope, &document_url, request, head,
                                moli_fetch::ResponseBody::materialized_bytes(b"discard me".to_vec()),
                                opaque.then_some(crate::types::AsyncSubresourceFetchResponseFilter::Opaque),
                            ),
                            "subresource" => crate::network_host::build_fetch_response_object_from_subresource_body_for_request_mode(
                                scope, &document_url, request, head,
                                crate::protocol_types::SubresourceResponseBody::from_bytes(b"discard me".to_vec()),
                            ),
                            "stream" => crate::network_host::build_fetch_response_object_from_stream_for_request_mode(
                                scope, &document_url, request, head, id,
                            ),
                            "preload" => crate::network_host::build_navigation_preload_response_object_from_stream_for_request_mode(
                                scope, &document_url, request, head, id,
                            ),
                            _ => unreachable!(),
                        };
                        assert!(response.get(scope, v8str(scope, "body").into()).unwrap().is_null(),
                            "{method}/{status}/{opaque}/{source}");
                        assert!(!host.borrow().pending_network_body_sources.contains_key(&id));
                        crate::network_host::enqueue_pending_network_body_chunk(scope, id, b"late bytes".to_vec());
                        crate::network_host::error_pending_network_body_stream(scope, id, "late error".to_owned());
                        crate::network_host::close_pending_network_body_stream(scope, id);
                        match crate::network_host::materialize_response_object_body(scope, response, "null body") {
                            MaterializedResponseBody::Ready(bytes) => assert!(bytes.is_empty()),
                            _ => panic!("null internal body must materialize immediately: {method}/{status}/{opaque}/{source}"),
                        }
                        assert!(host.borrow().pending_network_body_sources.is_empty());
                        assert!(host.borrow().pending_network_body_clones.is_empty());
                    }
                }
            }
            Ok(())
        })
        .expect("null response body checks should complete");
}

#[test]
fn opaque_window_fetch_keeps_blocked_bytes_out_of_internal_clone_consumers() {
    use crate::network_host::MaterializedResponseBody;
    for (mime, bytes, expected) in [
        (
            "application/json",
            &b"globalThis.value = 1;"[..],
            &b"globalThis.value = 1;"[..],
        ),
        ("application/json", &b"{\"secret\":true}"[..], &b""[..]),
        (
            "text/html",
            &b"\x89PNG\r\n\x1a\nimage data"[..],
            &b"\x89PNG\r\n\x1a\nimage data"[..],
        ),
    ] {
        let mut vm = new_storage_test_vm("https://opaque-stream.test/");
        vm.set_fetch_subresource_interception(
            true,
            Some(crate::types::SubresourceResourceType::Fetch),
        );
        vm.eval(
            r#"
            globalThis.__chunks = [];
            globalThis.__finished = false;
            globalThis.__onChunk = chunk => __chunks.push(...chunk);
            fetch('https://cross-origin.test/body', {mode: 'no-cors'}).then(response => {
                globalThis.__opaque = response;
                globalThis.__clone = response.clone();
            });
        "#,
        )
        .unwrap();
        let pending = vm.take_pending_subresource_fetch_infos();
        assert_eq!(pending.len(), 1);
        let pending = &pending[0];
        let id = crate::network_host::new_network_body_source_id();
        vm.start_streaming_async_subresource_fetch(
            crate::types::AsyncSubresourceStreamingStarted {
                skip_fetch_security_validation: false,
                response_filter: None,
                internal_id: pending.internal_id,
                request_url: pending.url.clone(),
                request_method: "GET".to_owned(),
                request_headers: Vec::new(),
                request_body: None,
                body_source_id: id,
                network_request_headers: None,
                head: moli_fetch::ResponseHead {
                    status_text: None,
                    final_url: pending.url.clone(),
                    status: 200,
                    headers: vec![("Content-Type".to_owned(), mime.to_owned())],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
            },
        )
        .unwrap();
        assert_eq!(
            vm.eval("JSON.stringify([__opaque.type, __opaque.body, __clone.body])")
                .unwrap(),
            r#"["opaque",null,null]"#
        );
        vm.with_default_context_scope_and_checkpoint_for_test(|scope, _| {
            let global = scope.get_current_context().global(scope);
            let response = v8::Local::<v8::Object>::try_from(
                global.get(scope, v8str(scope, "__clone").into()).unwrap(),
            )
            .unwrap();
            let callback = v8::Local::<v8::Function>::try_from(
                global.get(scope, v8str(scope, "__onChunk").into()).unwrap(),
            )
            .unwrap();
            let (body, _) =
                crate::network_host::materialize_response_object_body_with_chunk_callback(
                    scope,
                    response,
                    "opaque clone consumer",
                    callback,
                );
            let MaterializedResponseBody::Pending(promise) = body else {
                panic!("incomplete opaque body must remain pending");
            };
            assert_eq!(
                global.set(scope, v8str(scope, "__bodyDone").into(), promise.into()),
                Some(true)
            );
            Ok(())
        })
        .unwrap();
        vm.eval("__bodyDone.then(() => __finished = true)").unwrap();
        vm.append_streaming_async_subresource_fetch_chunk(id, bytes[..bytes.len() - 1].to_vec());
        if expected.is_empty() {
            assert_eq!(
                vm.eval("JSON.stringify(__chunks)").unwrap(),
                "[]",
                "blocked prefix reached internal consumer"
            );
        } else {
            assert_eq!(
                vm.eval("String(__finished)").unwrap(),
                "false",
                "allowed body ended before its last byte"
            );
        }
        vm.append_streaming_async_subresource_fetch_chunk(id, bytes[bytes.len() - 1..].to_vec());
        vm.finish_streaming_async_subresource_fetch(pending.internal_id, id, Ok(()))
            .unwrap();
        assert_eq!(vm.eval("String(__finished)").unwrap(), "true");
        assert_eq!(
            vm.eval("JSON.stringify(__chunks)").unwrap(),
            serde_json::to_string(expected).unwrap(),
            "{mime}"
        );
    }
}

#[test]
fn filtered_response_materialization_preserves_internal_head_across_clone_and_cache() {
    use crate::types::AsyncSubresourceFetchResponseFilter::{Opaque, OpaqueRedirect};
    for (response_type, filter) in [("opaque", Opaque), ("opaqueredirect", OpaqueRedirect)] {
        let internal_status = if response_type == "opaque" { 206 } else { 302 };
        let internal_headers = vec![
            (
                "cross-origin-resource-policy".to_owned(),
                "cross-origin".to_owned(),
            ),
            ("location".to_owned(), "target.html".to_owned()),
            ("set-cookie".to_owned(), "hidden=secret".to_owned()),
            ("vary".to_owned(), "*".to_owned()),
        ];
        let internal_headers = moli_fetch::headers_from_byte_strings(&internal_headers).unwrap();
        let mut vm = new_storage_test_vm("https://response-materialize-filtered.test/");
        let document_url = Url::parse("https://response-materialize-filtered.test/")
            .expect("document URL should parse");
        let final_url = Url::parse(
            "https://cross-response-materialize-filtered.test/redirect-start?x=%23#hidden",
        )
        .expect("final URL should parse");
        let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;

        vm.renderer_document_isolate
        .with_entered_renderer_document_isolate({
            let final_url = final_url.clone();
            let internal_headers = internal_headers.clone();
            move |isolate| {
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let response = crate::network_host::build_fetch_response_object_from_body_source_for_request_mode_with_filter(
                    scope,
                    &document_url,
                    crate::network_host::FetchResponseRequest {
redirect_mode: moli_fetch::RequestRedirectMode::Follow, method: "GET", mode: moli_fetch::RequestMode::Cors },
                    moli_fetch::ResponseHead {
                        status_text: Some("Internal Status".to_owned()),
                        final_url: final_url.clone(),
                        status: internal_status,
                        headers: internal_headers,
                        request_cookie_report: None,
                        cookie_set_reports: Vec::new(),
                        redirected: false,
                        redirect_chain: Vec::new(),
                        from_cache: false,
                        negotiated_http_version: None,
                    },
                    moli_fetch::ResponseBody::materialized_bytes(Vec::new()),
                    Some(filter),
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
                assert_eq!(head.response_type, response_type);
                assert_eq!(head.status, 0);
                Ok(())
            }
        })
        .expect("filtered response should install");

        let visible_url = vm
            .eval("globalThis.__filteredResponse.url")
            .expect("filtered response visible URL should evaluate");
        let expected_url = if response_type == "opaque" {
            ""
        } else {
            "https://cross-response-materialize-filtered.test/redirect-start?x=%23"
        };
        assert_eq!(visible_url, expected_url);

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
            globalThis.__filteredResponseCached.url,
            globalThis.__filteredResponseCached.body === null,
            [...globalThis.__filteredResponseCached.headers].length,
            globalThis.__filteredResponseCached.statusText
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
        assert_eq!(
            cache_probe,
            format!("{response_type}|0|{expected_url}|true|0|")
        );
        assert_eq!(
            vm.eval("__filteredResponseClone.url").unwrap(),
            expected_url
        );

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
                    crate::network_host::materialize_response_object_internal_head(
                        scope, clone, "clone",
                    )
                    .expect("filtered response clone should preserve the internal head")
                    .0;
                assert_eq!(materialized_clone.final_url.as_ref(), Some(&final_url));
                assert_eq!(materialized_clone.response_type, response_type);
                assert_eq!(materialized_clone.status, internal_status);
                assert_eq!(materialized_clone.status_text, "Internal Status");
                assert_eq!(materialized_clone.headers, internal_headers);

                let cached = global
                    .get(scope, v8str(scope, "__filteredResponseCached").into())
                    .expect("cached filtered response should exist");
                let materialized_cached =
                    crate::network_host::materialize_response_object_internal_head(
                        scope, cached, "cache",
                    )
                    .expect("cached filtered response should preserve the internal head")
                    .0;
                assert_eq!(materialized_cached.final_url.as_ref(), Some(&final_url));
                assert_eq!(materialized_cached.response_type, response_type);
                assert_eq!(materialized_cached.status, internal_status);
                assert_eq!(materialized_cached.status_text, "Internal Status");
                assert_eq!(materialized_cached.headers, internal_headers);
                Ok(())
            })
            .expect("filtered response clone/cache should materialize");
    }
}
