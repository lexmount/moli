use super::*;
use crate::util::v8str;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(interface = "Object", allow_empty)]
struct PendingBodyOwnerProbeDeclaration {}

#[derive(WebApiObject)]
#[webapi(interface = "Object", allow_empty)]
struct StreamingXhrProbeDeclaration {}

fn pending_body_owner_probe<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    PendingBodyOwnerProbeDeclaration::new()
        .bind(scope)
        .expect("pending body owner probe declaration should bind")
}

fn streaming_xhr_probe<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    StreamingXhrProbeDeclaration::new()
        .bind(scope)
        .expect("streaming xhr probe declaration should bind")
}

#[tokio::test(flavor = "current_thread")]
async fn same_origin_window_fetch_and_xhr_post_send_origin_on_wire() {
    let server = StaticHttpServer::spawn(2).await;
    let base_url = server.base_url();
    let loader = static_http_loader(std::iter::empty::<String>());
    let mut vm = new_page_task_executor_test_vm_with_loader(
        base_url
            .join("page.html")
            .expect("page fixture URL")
            .as_str(),
        &loader,
    );

    vm.eval(
        r#"
(() => {
  globalThis.__sameOriginPostOriginProbe = "pending";
  const xhrRequest = new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("POST", "xhr-origin");
    xhr.onload = () => resolve(xhr.responseText);
    xhr.onerror = () => reject(new Error("XHR failed"));
    xhr.send("xhr-body");
  });
  Promise.all([
    fetch("fetch-origin", {method: "POST", body: "fetch-body"}).then(response => response.text()),
    xhrRequest
  ]).then(
    () => { globalThis.__sameOriginPostOriginProbe = "done"; },
    error => { globalThis.__sameOriginPostOriginProbe = `error:${error && error.message}`; }
  );
  return "started";
})()
"#,
    )
    .expect("same-origin POST probe should evaluate");

    advance_page_task_executor_until_eval_equals(
        &mut vm,
        &loader,
        "String(globalThis.__sameOriginPostOriginProbe)",
        "done",
        "same-origin Fetch/XHR Origin wire probe",
    )
    .await;

    let expected_origin = base_url.origin().ascii_serialization();
    let mut requests = server.finish().await;
    requests.sort_by(|left, right| left.target.cmp(&right.target));
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert_eq!(request.method, "POST", "{}", request.target);
        assert_eq!(
            request.header_value("origin"),
            Some(expected_origin.as_str()),
            "{}",
            request.target
        );
    }
}

fn pending_fetch_continuation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    host: &crate::native_bridge::JsContextHost,
) -> crate::types::PendingSubresourceContinuation {
    let dispatch_scope = crate::native_bridge::OwnerDispatchScope::Top;
    crate::types::PendingSubresourceContinuation::Fetch(
        crate::types::PendingWindowFetchContinuation::new(
            v8::Global::new(scope, resolver),
            false,
            crate::document_runtime::DocumentConnectPolicySnapshot::from_policy_container(
                &crate::document_runtime::DocumentPolicyContainer::default(),
            ),
            crate::network_host::capture_window_csp_report_request_context(
                scope,
                host,
                dispatch_scope,
            )
            .expect("test Fetch should capture its CSP report context"),
        ),
    )
}

#[test]
fn xml_http_request_surface_exposes_chromium_like_upload_and_accessor_shape() {
    let mut vm = new_storage_test_vm("https://xhr-surface-shape.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const xhr = new XMLHttpRequest();
  const xhrProto = XMLHttpRequest.prototype;
  const xhrEventTargetProto = XMLHttpRequestEventTarget.prototype;
  const uploadDesc = Object.getOwnPropertyDescriptor(xhrProto, 'upload');
  const responseURLDesc = Object.getOwnPropertyDescriptor(xhrProto, 'responseURL');
  const onloadDesc = Object.getOwnPropertyDescriptor(xhrEventTargetProto, 'onload');
  const protoKeys = Object.getOwnPropertyNames(xhrProto);
  return [
    typeof XMLHttpRequest,
    typeof XMLHttpRequestEventTarget,
    typeof XMLHttpRequestUpload,
    Object.getPrototypeOf(xhrProto) === xhrEventTargetProto,
    Object.getPrototypeOf(XMLHttpRequestUpload.prototype) === xhrEventTargetProto,
    Object.keys(xhr).length,
    Object.getOwnPropertyNames(xhr).length,
    protoKeys.includes('upload'),
    protoKeys.includes('responseURL'),
    protoKeys.includes('readyState'),
    !protoKeys.includes('onload'),
    xhr.responseURL === '',
    xhr.readyState === 0,
    xhr.status === 0,
    typeof xhr.upload,
    Object.prototype.toString.call(xhr.upload),
    xhr.upload.constructor.name,
    xhr.upload instanceof EventTarget,
    !!uploadDesc,
    uploadDesc.enumerable,
    uploadDesc.configurable,
    typeof uploadDesc.get,
    uploadDesc.set === undefined,
    !!responseURLDesc,
    responseURLDesc.enumerable,
    responseURLDesc.configurable,
    typeof responseURLDesc.get,
    responseURLDesc.set === undefined,
    !!onloadDesc,
    onloadDesc.enumerable,
    onloadDesc.configurable,
    typeof onloadDesc.get,
    typeof onloadDesc.set
  ].join('|');
})()
"#,
        )
        .expect("xhr surface probe should run");

    assert_eq!(
        result,
        "function|function|function|true|true|0|0|true|true|true|true|true|true|true|object|[object XMLHttpRequestUpload]|XMLHttpRequestUpload|true|true|true|true|function|true|true|true|true|function|true|true|true|true|function|function"
    );
}

#[test]
fn xml_http_request_constants_are_declared_on_constructor_and_prototype() {
    let mut vm = new_storage_test_vm("https://xhr-constants-declared.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const constants = [
    ["UNSENT", 0],
    ["OPENED", 1],
    ["HEADERS_RECEIVED", 2],
    ["LOADING", 3],
    ["DONE", 4]
  ];
  const descriptorShape = (owner, name, expected) => {
    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
    return [
      name,
      descriptor && descriptor.value,
      descriptor && descriptor.value === expected,
      descriptor && descriptor.enumerable,
      descriptor && descriptor.writable,
      descriptor && descriptor.configurable
    ].join(":");
  };
  const xhr = new XMLHttpRequest();
  return JSON.stringify({
    constructor: constants.map(([name, value]) =>
      descriptorShape(XMLHttpRequest, name, value)
    ),
    prototype: constants.map(([name, value]) =>
      descriptorShape(XMLHttpRequest.prototype, name, value)
    ),
    instanceOwn: constants
      .map(([name]) => name)
      .filter(name => Object.prototype.hasOwnProperty.call(xhr, name)),
    keysContainConstants: Object.keys(XMLHttpRequest).some(name =>
      constants.some(([constant]) => constant === name)
    ) || Object.keys(XMLHttpRequest.prototype).some(name =>
      constants.some(([constant]) => constant === name)
    ),
    initialStateMatchesUnsent: xhr.readyState === XMLHttpRequest.UNSENT
  });
})()
"#,
        )
        .expect("XMLHttpRequest constants descriptor probe should evaluate");

    assert_eq!(
        result,
        r#"{"constructor":["UNSENT:0:true:true:false:false","OPENED:1:true:true:false:false","HEADERS_RECEIVED:2:true:true:false:false","LOADING:3:true:true:false:false","DONE:4:true:true:false:false"],"prototype":["UNSENT:0:true:true:false:false","OPENED:1:true:true:false:false","HEADERS_RECEIVED:2:true:true:false:false","LOADING:3:true:true:false:false","DONE:4:true:true:false:false"],"instanceOwn":[],"keysContainConstants":true,"initialStateMatchesUnsent":true}"#
    );
}

#[test]
fn xml_http_request_accessors_keep_instance_surface_clean() {
    let mut vm = new_storage_test_vm("https://xhr-accessor-state.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const xhr = new XMLHttpRequest();
  const firstUpload = xhr.upload;
  const secondUpload = xhr.upload;
  xhr.responseType = 'json';
  let invalid = 'missing';
  try {
    xhr.responseType = 'JSON';
  } catch (error) {
    invalid = `${error && error.name}:${xhr.responseType}`;
  }
  xhr.timeout = 250;
  xhr.withCredentials = true;
  xhr.onreadystatechange = () => 'ready';
  xhr.onload = () => 'load';
  return [
    firstUpload === secondUpload,
    xhr.responseType,
    invalid,
    xhr.timeout,
    xhr.withCredentials,
    typeof xhr.onreadystatechange,
    typeof xhr.onload,
    Object.keys(xhr).length,
    Object.getOwnPropertyNames(xhr).length,
    Object.getOwnPropertyNames(XMLHttpRequest.prototype).includes('responseType'),
    Object.getOwnPropertyNames(XMLHttpRequestEventTarget.prototype).includes('onload')
  ].join('|');
})()
"#,
        )
        .expect("xhr accessor state probe should run");

    assert_eq!(
        result,
        "true|json|TypeError:json|250|true|function|function|0|0|true|true"
    );
}

#[test]
fn xml_http_request_rejects_timeouts_for_synchronous_document_requests() {
    let mut vm = new_storage_test_vm("https://xhr-sync-timeout.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const exceptionShape = callback => {
    try {
      callback();
      return "no-throw";
    } catch (error) {
      return {
        name: error.name,
        code: error.code,
        isDomException: error instanceof DOMException,
        message: error.message
      };
    }
  };

  let conversionCalls = 0;
  const afterOpen = new XMLHttpRequest();
  afterOpen.open("GET", "/after-open", false);
  const afterOpenError = exceptionShape(() => {
    afterOpen.timeout = {
      valueOf() {
        conversionCalls += 1;
        return 25;
      }
    };
  });
  const zeroError = exceptionShape(() => { afterOpen.timeout = 0; });

  const beforeOpen = new XMLHttpRequest();
  beforeOpen.timeout = 25;
  const beforeOpenError = exceptionShape(() => {
    beforeOpen.open("GET", "/before-open", false);
  });

  const asynchronous = new XMLHttpRequest();
  asynchronous.open("GET", "/async", true);
  asynchronous.timeout = 30;

  return JSON.stringify({
    afterOpenError,
    zeroError,
    conversionCalls,
    afterOpenTimeout: afterOpen.timeout,
    beforeOpenError,
    beforeOpenReadyState: beforeOpen.readyState,
    beforeOpenTimeout: beforeOpen.timeout,
    asynchronousTimeout: asynchronous.timeout
  });
})()
"#,
        )
        .expect("synchronous document XHR timeout probe should run");

    assert_eq!(
        result,
        r#"{"afterOpenError":{"name":"InvalidAccessError","code":15,"isDomException":true,"message":"Failed to set the 'timeout' property on 'XMLHttpRequest': Timeouts cannot be set for synchronous requests made from a document."},"zeroError":{"name":"InvalidAccessError","code":15,"isDomException":true,"message":"Failed to set the 'timeout' property on 'XMLHttpRequest': Timeouts cannot be set for synchronous requests made from a document."},"conversionCalls":1,"afterOpenTimeout":0,"beforeOpenError":{"name":"InvalidAccessError","code":15,"isDomException":true,"message":"Failed to execute 'open' on 'XMLHttpRequest': Synchronous requests must not set a timeout."},"beforeOpenReadyState":0,"beforeOpenTimeout":25,"asynchronousTimeout":30}"#
    );
}

#[test]
fn xml_http_request_rejects_response_types_for_synchronous_document_requests() {
    let mut vm = new_storage_test_vm("https://xhr-sync-response-type.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const shape = callback => {
    try {
      callback();
      return {name: "missing"};
    } catch (error) {
      return {
        name: error.name,
        code: error.code,
        isDomException: error instanceof DOMException,
        message: error.message
      };
    }
  };

  const beforeOpen = ["arraybuffer", "blob", "json", "text", "document"].map(type => {
    const xhr = new XMLHttpRequest();
    const events = [];
    xhr.onreadystatechange = () => events.push(xhr.readyState);
    xhr.responseType = type;
    const error = shape(() => xhr.open("GET", "/before-open", false));
    return {type, error, events, readyState: xhr.readyState, responseType: xhr.responseType};
  });

  const afterOpen = ["arraybuffer", "blob", "json", "text", "document"].map(type => {
    const xhr = new XMLHttpRequest();
    const events = [];
    xhr.onreadystatechange = () => events.push(xhr.readyState);
    xhr.open("GET", "/after-open", false);
    const error = shape(() => { xhr.responseType = type; });
    return {type, error, events, readyState: xhr.readyState, responseType: xhr.responseType};
  });

  const asynchronous = new XMLHttpRequest();
  asynchronous.open("GET", "/async", true);
  asynchronous.responseType = "json";

  return JSON.stringify({beforeOpen, afterOpen, asynchronous: asynchronous.responseType});
})()
"#,
        )
        .expect("synchronous document XHR responseType probe should run");

    let observed: serde_json::Value =
        serde_json::from_str(&result).expect("responseType probe should return JSON");
    let before = observed["beforeOpen"]
        .as_array()
        .expect("before-open matrix should be an array");
    let after = observed["afterOpen"]
        .as_array()
        .expect("after-open matrix should be an array");
    assert_eq!(before.len(), 5);
    assert_eq!(after.len(), 5);
    for entry in before {
        assert_eq!(entry["error"]["name"], "InvalidAccessError");
        assert_eq!(entry["error"]["code"], 15);
        assert_eq!(entry["error"]["isDomException"], true);
        assert_eq!(
            entry["error"]["message"],
            "Failed to execute 'open' on 'XMLHttpRequest': Synchronous requests from a document must not set a response type."
        );
        assert_eq!(entry["events"], serde_json::json!([]));
        assert_eq!(entry["readyState"], 0);
        assert_eq!(entry["responseType"], entry["type"]);
    }
    for entry in after {
        assert_eq!(entry["error"]["name"], "InvalidAccessError");
        assert_eq!(entry["error"]["code"], 15);
        assert_eq!(entry["error"]["isDomException"], true);
        assert_eq!(
            entry["error"]["message"],
            "Failed to set the 'responseType' property on 'XMLHttpRequest': The response type cannot be changed for synchronous requests made from a document."
        );
        assert_eq!(entry["events"], serde_json::json!([1]));
        assert_eq!(entry["readyState"], 1);
        assert_eq!(entry["responseType"], "");
    }
    assert_eq!(observed["asynchronous"], "json");
}

#[test]
fn xml_http_request_reopen_in_opened_state_does_not_repeat_readystatechange() {
    let mut vm = new_storage_test_vm("https://xhr-reopen.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const xhr = new XMLHttpRequest();
  const states = [];
  xhr.onreadystatechange = () => states.push(xhr.readyState);
  xhr.open("GET", "/first");
  xhr.open("GET", "/second", false);
  return JSON.stringify({states, readyState: xhr.readyState, status: xhr.status});
})()
"#,
        )
        .expect("XHR consecutive-open probe should run");

    assert_eq!(result, r#"{"states":[1],"readyState":1,"status":0}"#);
}

#[test]
fn xml_http_request_declared_state_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://xhr-private-state.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const xhr = new XMLHttpRequest();
  const upload = xhr.upload;
  const internalNames = object => Object.getOwnPropertyNames(object)
    .filter(name =>
      name.startsWith('__lmXhr') ||
      name === '__lmMethod' ||
      name === '__lmUrl' ||
      name === '__lmResponseHeaders' ||
      name.startsWith('__moliXhr') ||
      name === '__moliChildBrowsingContextHandle')
    .sort();
  const ownNamesBefore = {
    xhr: internalNames(xhr),
    upload: internalNames(upload),
    xhrPrototype: internalNames(XMLHttpRequest.prototype),
    eventTargetPrototype: internalNames(XMLHttpRequestEventTarget.prototype)
  };

  XMLHttpRequest.prototype.__lmXhrReadyState = 99;
  XMLHttpRequest.prototype.__lmXhrStatus = 418;
  XMLHttpRequest.prototype.__lmXhrResponseType = 'arraybuffer';
  XMLHttpRequest.prototype.__lmXhrTimeout = 999;
  XMLHttpRequest.prototype.__lmXhrWithCredentials = false;
  XMLHttpRequestEventTarget.prototype.__moliXhrUsesSimpleEventTarget = false;

  xhr.__lmXhrReadyState = 99;
  xhr.__lmXhrStatus = 418;
  xhr.__lmXhrResponseType = 'arraybuffer';
  xhr.__lmXhrTimeout = 999;
  xhr.__lmXhrWithCredentials = false;
  xhr.__moliChildBrowsingContextHandle = 999999;
  upload.__lmXhrUpload = 'spoof';
  upload.__lmXhrUploadInProgress = true;

  xhr.responseType = 'json';
  xhr.timeout = 250;
  xhr.withCredentials = true;
  xhr.onload = () => 'load';
  upload.onload = () => 'upload-load';
  let listenerHits = 0;
  xhr.addEventListener('readystatechange', () => {
    listenerHits += xhr.readyState === 1 ? 1 : 10;
  });

  const fake = Object.create(XMLHttpRequest.prototype);
  fake.__lmXhrReadyState = 77;
  fake.__lmXhrStatus = 599;
  fake.__lmXhrResponseType = 'blob';
  fake.__lmXhrTimeout = 888;
  fake.__lmXhrWithCredentials = true;

  const fakeValues = [
    fake.readyState,
    fake.status,
    fake.responseType,
    fake.timeout,
    fake.withCredentials,
    fake.upload === undefined ? 'undefined' : typeof fake.upload
  ].join('|');

  xhr.open('GET', '/state');
  return JSON.stringify({
    ownNamesBefore,
    realValues: [
      xhr.readyState,
      xhr.status,
      xhr.responseType,
      xhr.timeout,
      xhr.withCredentials,
      typeof xhr.onload,
      typeof upload.onload,
      listenerHits
    ].join('|'),
    fakeValues
  });
})()
"#,
        )
        .expect("XHR private state spoofing probe should run");

    assert_eq!(
        result,
        r#"{"ownNamesBefore":{"xhr":[],"upload":[],"xhrPrototype":[],"eventTargetPrototype":[]},"realValues":"1|0|json|250|true|function|function|1","fakeValues":"0|0||0|false|undefined"}"#
    );
}

#[test]
fn xml_http_request_send_flag_gates_with_credentials_and_resend() {
    let mut vm = new_storage_test_vm("https://xhr-send-flag-state.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const names = [];
  const expectInvalidState = (callback) => {
    try {
      callback();
      names.push('missing');
    } catch (error) {
      names.push(`${error && error.name}:${error instanceof DOMException}`);
    }
  };

  const unopened = new XMLHttpRequest();
  expectInvalidState(() => unopened.send());

  const xhr = new XMLHttpRequest();
  xhr.withCredentials = true;
  xhr.open('GET', '/never-loaded');
  xhr.withCredentials = false;
  xhr.send();
  expectInvalidState(() => { xhr.withCredentials = true; });
  expectInvalidState(() => xhr.send());
  xhr.abort();
  xhr.withCredentials = true;
  names.push(String(xhr.withCredentials));
  return names.join('|');
})()
"#,
        )
        .expect("xhr send flag state probe should run");

    assert_eq!(
        result,
        "InvalidStateError:true|InvalidStateError:true|InvalidStateError:true|true"
    );
}
#[test]
fn xml_http_request_open_and_abort_keep_accessor_backed_state_without_own_props() {
    let mut vm = new_storage_test_vm("https://xhr-open-abort-surface.test/");

    let result = vm
        .eval(
            r#"
(() => {
  let aborted = false;
  let errored = false;
  let ended = false;
  let rsListener = false;
  let abortListener = false;
  let errorListener = false;
  let loadendListener = false;
  const xhr = new XMLHttpRequest();
  const progressShape = (event, type) =>
    event instanceof ProgressEvent &&
    event.type === type &&
    event.lengthComputable === false &&
    event.loaded === 0 &&
    event.total === 0 &&
    event.target === xhr &&
    event.currentTarget === xhr;
  xhr.onabort = event => { aborted = progressShape(event, 'abort'); };
  xhr.onerror = () => { errored = true; };
  xhr.onloadend = event => { ended = progressShape(event, 'loadend'); };
  xhr.addEventListener('readystatechange', () => { rsListener = xhr.readyState === 1; });
  xhr.addEventListener('abort', () => { abortListener = true; });
  xhr.addEventListener('error', () => { errorListener = true; });
  xhr.addEventListener('loadend', () => { loadendListener = true; });
  const invalidState = callback => {
    try {
      callback();
      return false;
    } catch (error) {
      return error && error.name === 'InvalidStateError' && error instanceof DOMException;
    }
  };
  xhr.responseType = 'json';
  xhr.timeout = 250;
  xhr.withCredentials = true;
  xhr.open('POST', '/submit');
  const afterOpen = [
    xhr.readyState === 1,
    xhr.status === 0,
    xhr.statusText === '',
    invalidState(() => xhr.responseText),
    xhr.responseURL === '',
    xhr.responseType === 'json',
    xhr.timeout === 250,
    xhr.withCredentials === true,
    Object.keys(xhr).length === 0,
    Object.getOwnPropertyNames(xhr).length === 0
  ];
  xhr.abort();
  return [
    ...afterOpen,
    xhr.readyState === 0,
    xhr.status === 0,
    xhr.statusText === '',
    xhr.responseURL === '',
    invalidState(() => xhr.responseXML),
    aborted,
    errored,
    ended,
    rsListener,
    abortListener,
    errorListener,
    loadendListener,
    Object.keys(xhr).length === 0,
    Object.getOwnPropertyNames(xhr).length === 0
  ].join('|');
})()
"#,
        )
        .expect("xhr open/abort surface probe should run");

    assert_eq!(
        result,
        "true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|true|false|true|true|true|false|true|true|true"
    );
}
#[test]
fn xml_http_request_methods_apply_webidl_argument_conversion() {
    let mut vm = new_storage_test_vm("https://xhr-webidl-args.test/");

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
  const xhr = new XMLHttpRequest();
  xhr.open('GET', '/headers');
  const mimeXhr = new XMLHttpRequest();
  return [
    probe(() => (new XMLHttpRequest()).open('GET', '/sync', false)),
    probe(() => (new XMLHttpRequest()).open('GET')),
    probe(() => (new XMLHttpRequest()).open('G\u0100T', '/')),
    probe(() => (new XMLHttpRequest()).open('GET', '/', true, Symbol())),
    probe(() => xhr.setRequestHeader('X-Test', 'one')),
    probe(() => xhr.setRequestHeader('X-\u0100', 'value')),
    probe(() => xhr.setRequestHeader('X-Test', Symbol())),
    probe(() => xhr.getResponseHeader(Symbol())),
    probe(() => mimeXhr.overrideMimeType()),
    probe(() => mimeXhr.overrideMimeType(Symbol())),
    probe(() => mimeXhr.overrideMimeType({
      toString() {
        throw new RangeError('mime');
      }
    })),
    probe(() => mimeXhr.overrideMimeType(undefined))
  ].join('|');
})()
"#,
        )
        .expect("xhr WebIDL argument conversion probe should run");

    assert_eq!(
        result,
        "undefined|throw:TypeError|throw:TypeError|throw:TypeError|undefined|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:RangeError|undefined"
    );
}
#[test]
fn xml_http_request_override_mime_type_affects_response_mime() {
    let mut vm = new_storage_test_vm("https://xhr-override-mime.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__xhrMime = new XMLHttpRequest();
  __xhrMime.open('GET', '/mime');
  __xhrMime.responseType = 'blob';
  __xhrMime.overrideMimeType('text/custom');
  return 'ready';
})()
"#,
    )
    .expect("xhr override MIME setup should run");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let global = context.global(scope);
            let key = v8::String::new(scope, "__xhrMime").expect("xhr key should allocate");
            let xhr_value = global
                .get(scope, key.into())
                .expect("stored XHR should be readable");
            let xhr = v8::Local::<v8::Object>::try_from(xhr_value)
                .expect("stored XHR should be an object");
            crate::network_host::apply_xhr_response_body_source(
                scope,
                xhr,
                moli_fetch::ResponseHead {
                    final_url: Url::parse("https://xhr-override-mime.test/mime")
                        .expect("response URL should parse"),
                    status: 200,
                    headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                moli_fetch::ResponseBody::materialized_bytes(b"body".to_vec()),
            );
            Ok(())
        })
        .expect("xhr response should apply");

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
  return [
    __xhrMime.readyState,
    __xhrMime.response.type,
    probe(() => __xhrMime.overrideMimeType('text/plain'))
  ].join('|');
})()
"#,
        )
        .expect("xhr override MIME response probe should run");

    assert_eq!(result, "4|text/custom|throw:InvalidStateError");
}

#[test]
fn xml_http_request_default_response_type_parses_response_xml_for_document_mime() {
    let mut vm = new_storage_test_vm("https://xhr-response-xml.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__xhrXml = new XMLHttpRequest();
  __xhrXml.open('GET', '/xml-doc');
  __xhrXml.overrideMimeType('text/xml');

  globalThis.__xhrPlain = new XMLHttpRequest();
  __xhrPlain.open('GET', '/plain');
  return 'ready';
})()
"#,
    )
    .expect("xhr responseXML setup should run");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let global = context.global(scope);

            let xml_key = v8::String::new(scope, "__xhrXml").expect("xhr xml key should allocate");
            let xml_xhr = global
                .get(scope, xml_key.into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .expect("stored XML XHR should be an object");
            crate::network_host::apply_xhr_response_body_source(
                scope,
                xml_xhr,
                moli_fetch::ResponseHead {
                    final_url: Url::parse("https://xhr-response-xml.test/xml-doc")
                        .expect("XML response URL should parse"),
                    status: 200,
                    headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                moli_fetch::ResponseBody::materialized_bytes(
                    b"<!DOCTYPE html><html><body></body></html>".to_vec(),
                ),
            );

            let plain_key =
                v8::String::new(scope, "__xhrPlain").expect("xhr plain key should allocate");
            let plain_xhr = global
                .get(scope, plain_key.into())
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
                .expect("stored plain XHR should be an object");
            crate::network_host::apply_xhr_response_body_source(
                scope,
                plain_xhr,
                moli_fetch::ResponseHead {
                    final_url: Url::parse("https://xhr-response-xml.test/plain")
                        .expect("plain response URL should parse"),
                    status: 200,
                    headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
                    request_cookie_report: None,
                    cookie_set_reports: Vec::new(),
                    redirected: false,
                    redirect_chain: Vec::new(),
                    from_cache: false,
                    negotiated_http_version: None,
                },
                moli_fetch::ResponseBody::materialized_bytes(b"plain text".to_vec()),
            );
            Ok(())
        })
        .expect("xhr responses should apply");

    let result = vm
        .eval(
            r#"
(() => {
  const created = __xhrXml.responseXML.createElement('wpt-xhr-response-doc');
  return [
    __xhrXml.readyState,
    __xhrXml.response === __xhrXml.responseText,
    __xhrXml.responseXML && __xhrXml.responseXML.documentElement.localName,
    created.ownerDocument === __xhrXml.responseXML,
    __xhrXml.responseXML.createElement('wpt-xhr-response-doc') instanceof HTMLElement,
    __xhrPlain.responseXML === null
  ].join('|');
})()
"#,
        )
        .expect("xhr responseXML probe should run");

    assert_eq!(result, "4|true|html|true|false|true");
}

#[test]
fn xml_http_request_send_body_applies_webidl_conversion() {
    let vm = new_storage_test_vm("https://xhr-send-body-webidl.test/");
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;

    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);

            let string_body =
                v8::String::new(scope, "payload").expect("body string should allocate");
            let prepared = crate::network_host::prepare_xhr_send_body(scope, string_body.into())
                .expect("string body should convert");
            assert_eq!(prepared.body.as_deref(), Some(b"payload".as_slice()));
            assert_eq!(
                prepared.default_content_type.as_deref(),
                Some("text/plain;charset=UTF-8")
            );

            let symbol = v8::Symbol::new(scope, None);
            let symbol_error =
                match crate::network_host::prepare_xhr_send_body(scope, symbol.into()) {
                    Ok(_) => panic!("Symbol body should be rejected by USVString conversion"),
                    Err(error) => error,
                };
            assert_eq!(
                symbol_error.to_string(),
                "XMLHttpRequest.send: Argument 1 can not be converted to USVString"
            );

            let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
            let mut scope = try_catch.init();
            let source =
                v8::String::new(&scope, "({ toString() { throw new RangeError('body'); } })")
                    .expect("throwing body source should allocate");
            let script = v8::Script::compile(&scope, source, None)
                .expect("throwing body script should compile");
            let throwing_body = script
                .run(&scope)
                .expect("throwing body object should create");
            let throwing_error =
                match crate::network_host::prepare_xhr_send_body(&mut scope, throwing_body) {
                    Ok(_) => panic!("throwing stringifier should propagate as a pending exception"),
                    Err(error) => error,
                };
            assert!(throwing_error.is_pending_exception());
            assert!(scope.has_caught());
            let exception = scope.exception().expect("RangeError should be caught");
            let name_key = v8::String::new(&scope, "name").expect("name key should allocate");
            let name = v8::Local::<v8::Object>::try_from(exception)
                .expect("caught exception should be an object")
                .get(&scope, name_key.into())
                .expect("exception name should be readable")
                .to_string(&scope)
                .expect("exception name should stringify")
                .to_rust_string_lossy(&scope);
            assert_eq!(name, "RangeError");
            Ok(())
        })
        .expect("xhr send body WebIDL conversion probe should run");
}
#[test]
fn xml_http_request_send_callback_parses_webidl_body_argument() {
    let mut vm = new_storage_test_vm("https://xhr-send-callback-webidl.test/");

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
  const xhr = new XMLHttpRequest();
  xhr.open('POST', '/submit');
  const symbol = probe(() => xhr.send(Symbol('body')));
  const throwing = probe(() => xhr.send({ toString() { throw new RangeError('body'); } }));

  const get = new XMLHttpRequest();
  get.open('GET', '/ignored-body');
  const ignoredGetBody = probe(() => get.send(Symbol('ignored')));

  const lowercaseGet = new XMLHttpRequest();
  lowercaseGet.open('get', '/ignored-lower-body');
  const ignoredLowercaseGetBody = probe(() => lowercaseGet.send(Symbol('ignored')));

  return [symbol, throwing, ignoredGetBody, ignoredLowercaseGetBody].join('|');
})()
            "#,
        )
        .expect("xhr send callback WebIDL body probe should run");

    assert_eq!(
        result,
        "throw:TypeError|throw:RangeError|undefined|undefined"
    );
}
#[test]
fn xml_http_request_accessors_apply_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://xhr-accessor-webidl.test/");

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
  const messageProbe = callback => {
    try {
      callback();
      return 'no-throw';
    } catch (error) {
      return String(error && error.message).includes('__lmXhrTimeout') ? 'leaked' : 'public';
    }
  };
  const xhr = new XMLHttpRequest();
  let responseTypeToStringCalls = 0;
  let withCredentialsValueOfCalls = 0;
  return [
    probe(() => { xhr.timeout = -1; return xhr.timeout; }),
    probe(() => { xhr.timeout = Symbol(); }),
    messageProbe(() => { xhr.timeout = Symbol(); }),
    probe(() => { xhr.timeout = { valueOf() { throw new RangeError('timeout'); } }; }),
    probe(() => {
      xhr.responseType = {
        toString() {
          responseTypeToStringCalls += 1;
          return 'json';
        }
      };
      return `${xhr.responseType}:${responseTypeToStringCalls}`;
    }),
    probe(() => { xhr.responseType = Symbol(); }),
    probe(() => { xhr.responseType = 'moz-blob'; return xhr.responseType; }),
    probe(() => {
      xhr.withCredentials = {
        valueOf() {
          withCredentialsValueOfCalls += 1;
          return false;
        }
      };
      return `${xhr.withCredentials}:${withCredentialsValueOfCalls}`;
    }),
    probe(() => { xhr.withCredentials = Symbol(); return xhr.withCredentials; })
  ].join('|');
})()
"#,
        )
        .expect("xhr accessor WebIDL conversion probe should run");

    assert_eq!(
        result,
        "4294967295|throw:TypeError|public|throw:RangeError|json:1|throw:TypeError||true:0|true"
    );
}
#[test]
fn progress_event_constructor_applies_webidl_init_conversion() {
    let mut vm = new_storage_test_vm("https://progress-event-webidl-args.test/");

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
  const event = new ProgressEvent('progress', {
    bubbles: 1,
    cancelable: '',
    lengthComputable: 1,
    loaded: 1.9,
    total: NaN
  });
  const empty = new ProgressEvent('empty', null);
  const ownSlots = Object.getOwnPropertyNames(event)
    .filter(name => name.startsWith('__lmProgressEvent'))
    .sort()
    .join(',');
  const loadedGetter = Object.getOwnPropertyDescriptor(ProgressEvent.prototype, 'loaded').get;
  const fake = { __lmProgressEventLoaded: 99 };
  return [
    event.type,
    event.bubbles,
    event.cancelable,
    event.lengthComputable,
    event.loaded,
    event.total,
    empty.loaded,
    empty.total,
    probe(() => new ProgressEvent()),
    probe(() => new ProgressEvent('x', 1)),
    ownSlots,
    probe(() => loadedGetter.call(fake))
  ].join('|');
})()
"#,
        )
        .expect("ProgressEvent WebIDL argument conversion probe should run");

    assert_eq!(
        result,
        "progress|true|false|true|1.9|NaN|0|0|throw:TypeError|throw:TypeError||throw:TypeError"
    );
}
#[test]
fn progress_event_interface_matches_webidl_property_shape() {
    let mut vm = new_storage_test_vm("https://progress-event-interface-shape.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const protoDesc = Object.getOwnPropertyDescriptor(ProgressEvent, 'prototype');
  const attrs = ['lengthComputable', 'loaded', 'total'].map(name => {
    const desc = Object.getOwnPropertyDescriptor(ProgressEvent.prototype, name);
    let throws = false;
    try { ProgressEvent.prototype[name]; } catch (error) { throws = error.name === 'TypeError'; }
    return `${name}:${desc && desc.enumerable}:${desc && desc.configurable}:${throws}`;
  }).join(',');
  let enumerable = false;
  for (const key in window) {
    if (key === 'ProgressEvent') enumerable = true;
  }
  const beforeDelete = [
    typeof ProgressEvent,
    ProgressEvent.length,
    protoDesc && protoDesc.writable,
    protoDesc && protoDesc.enumerable,
    protoDesc && protoDesc.configurable,
    ProgressEvent.prototype.constructor === ProgressEvent,
    Object.getPrototypeOf(ProgressEvent.prototype) === Event.prototype,
    attrs,
    enumerable
  ];
  const deleteResult = delete window.ProgressEvent;
  return beforeDelete.concat([
    deleteResult,
    typeof window.ProgressEvent
  ]).join('|');
})()
"#,
        )
        .expect("ProgressEvent interface shape probe should run");

    assert_eq!(
        result,
        "function|1|false|false|false|true|true|lengthComputable:true:true:true,loaded:true:true:true,total:true:true:true|false|true|undefined"
    );
}
#[test]
fn xml_http_request_data_url_and_loadstart_state_transitions() {
    let mut vm = new_storage_test_vm("https://xhr-loadstart-data.test/");

    vm.eval(
        r#"
(() => {
  const first = new XMLHttpRequest();
  const events = globalThis.__xhrLoadstartEvents = [];
  const probe = callback => {
    try {
      callback();
      return 'ok';
    } catch (error) {
      return error && error.name;
    }
  };
  first.onloadstart = () => {
    events.push(`loadstart:${first.readyState}`);
    events.push(probe(() => first.setRequestHeader('General', 'Organa')));
    events.push(probe(() => { first.withCredentials = true; }));
    events.push(probe(() => first.send()));
    first.onloadstart = null;
    first.open('GET', 'data:,BB-8');
    first.send();
  };
  first.onload = () => events.push(`load:${first.responseText}`);
  first.open('GET', 'data:,R2-D2');
  first.send();

  const second = new XMLHttpRequest();
  second.onloadstart = () => {
    events.push(`abort-loadstart:${second.readyState}`);
    second.abort();
    events.push(`abort-after:${second.readyState}`);
  };
  second.onabort = () => events.push(`abort-event:${second.readyState}`);
  second.open('GET', 'data:,K-2SO');
  second.send();
})()
"#,
    )
    .expect("XHR data URL loadstart probe should run");
    vm.eval("0")
        .expect("follow-up checkpoint should drain queued XHR completion");
    let result = vm
        .eval("globalThis.__xhrLoadstartEvents.join('|')")
        .expect("XHR data URL loadstart events should be readable after microtasks");

    assert_eq!(
        result,
        "loadstart:1|InvalidStateError|InvalidStateError|InvalidStateError|abort-loadstart:1|abort-event:4|abort-after:0|load:BB-8"
    );
}
#[test]
fn xml_http_request_data_url_loads_materialized_body() {
    let mut vm = new_storage_test_vm("https://xhr-data-url.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__xhrDataEvents = [];
  const xhr = new XMLHttpRequest();
  xhr.onload = () => __xhrDataEvents.push(`${xhr.status}:${xhr.responseText}:${xhr.responseURL}`);
  xhr.open('GET', 'data:text/plain;base64,QkItOA==');
  xhr.send();
})()
"#,
    )
    .expect("XHR data URL setup should run");
    vm.eval("0")
        .expect("follow-up checkpoint should drain queued data URL completion");
    let result = vm
        .eval("__xhrDataEvents.join('|')")
        .expect("XHR data URL event list should be readable");

    assert_eq!(result, "200:BB-8:data:text/plain;base64,QkItOA==");
}

#[test]
fn xml_http_request_dispatches_download_progress_before_done() {
    let mut vm = new_storage_test_vm("https://xhr-download-progress.test/");

    vm.eval(
        r#"
(() => {
  const xhr = new XMLHttpRequest();
  const events = globalThis.__xhrDownloadProgressEvents = [];
  xhr.addEventListener('readystatechange', () => events.push(`state:${xhr.readyState}`));
  for (const name of ['loadstart', 'progress', 'load', 'loadend']) {
    xhr.addEventListener(name, event => events.push(`${name}:${event.loaded}:${event.total}`));
  }
  xhr.open('GET', 'data:text/plain,progress');
  xhr.send();
})()
"#,
    )
    .expect("XHR download progress probe should run");
    vm.eval("0")
        .expect("follow-up checkpoint should drain queued XHR completion");

    assert_eq!(
        vm.eval("globalThis.__xhrDownloadProgressEvents.join('|')")
            .expect("XHR download progress events should be readable"),
        "state:1|loadstart:0:0|state:2|state:3|progress:8:8|state:4|load:8:8|loadend:8:8"
    );
}

#[test]
fn xml_http_request_document_csp_blocks_connect_src_and_dispatches_event() {
    let mut vm = new_storage_test_vm("https://xhr-connect-csp.test/");
    vm.set_response_content_security_policies(&[String::from("connect-src 'none'")]);

    vm.eval(
        r#"
(() => {
  globalThis.__xhrCspEvents = [];
  self.addEventListener("securitypolicyviolation", event => {
    __xhrCspEvents.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition,
      instance: event instanceof SecurityPolicyViolationEvent
    });
  });
  globalThis.__xhrCspResult = [];
  const xhr = new XMLHttpRequest();
  xhr.onerror = () => __xhrCspResult.push(`error:${xhr.status}:${xhr.readyState}`);
  xhr.onload = () => __xhrCspResult.push("load");
  xhr.onloadend = () => __xhrCspResult.push("loadend:" + __xhrCspEvents.length);
  xhr.open("GET", "data:text/plain,blocked");
  xhr.send();
  return "scheduled";
})()
"#,
    )
    .expect("XHR CSP block setup should run");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    vm.eval("0")
        .expect("follow-up checkpoint should drain queued XHR CSP failure");
    let result = vm
        .eval("JSON.stringify({events: __xhrCspEvents, result: __xhrCspResult})")
        .expect("XHR CSP result should be readable");

    assert_eq!(
        result,
        r#"{"events":[{"blockedURI":"data","effectiveDirective":"connect-src","disposition":"enforce","instance":true}],"result":["error:0:4","loadend:0"]}"#
    );
}

#[test]
fn xml_http_request_document_csp_report_only_dispatches_without_blocking() {
    let mut vm = new_storage_test_vm("https://xhr-connect-report-only.test/");
    vm.set_response_content_security_report_only_policies(&[String::from("connect-src 'none'")]);

    vm.eval(
        r#"
(() => {
  globalThis.__xhrReportOnlyEvents = [];
  self.addEventListener("securitypolicyviolation", event => {
    __xhrReportOnlyEvents.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition,
      instance: event instanceof SecurityPolicyViolationEvent
    });
  });
  globalThis.__xhrReportOnlyResult = [];
  const xhr = new XMLHttpRequest();
  xhr.onload = () => __xhrReportOnlyResult.push(`${xhr.status}:${xhr.responseText}`);
  xhr.onerror = () => __xhrReportOnlyResult.push("error");
  xhr.onloadend = () => __xhrReportOnlyResult.push("loadend:" + __xhrReportOnlyEvents.length);
  xhr.open("GET", "data:text/plain,allowed");
  xhr.send();
  return "scheduled";
})()
"#,
    )
    .expect("XHR CSP report-only setup should run");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    vm.eval("0")
        .expect("follow-up checkpoint should drain queued XHR report-only response");
    let result = vm
        .eval("JSON.stringify({events: __xhrReportOnlyEvents, result: __xhrReportOnlyResult})")
        .expect("XHR CSP report-only result should be readable");

    assert_eq!(
        result,
        r#"{"events":[{"blockedURI":"data","effectiveDirective":"connect-src","disposition":"report","instance":true}],"result":["200:allowed","loadend:0"]}"#
    );
}

#[test]
fn xml_http_request_sync_data_url_sets_response_url() {
    let mut vm = new_storage_test_vm("https://xhr-sync-data.test/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const xhr = new XMLHttpRequest();
  xhr.open('GET', 'data:,K-2SO', false);
  xhr.send();
  return [
    xhr.readyState,
    xhr.status,
    xhr.responseText,
    xhr.responseURL
  ].join('|');
})()
"#,
        )
        .expect("sync data URL XHR should complete synchronously");

    assert_eq!(result, "4|200|K-2SO|data:,K-2SO");
}

#[test]
fn xml_http_request_reopen_matches_sync_wpt_state_and_response_reset() {
    let mut vm = new_storage_test_vm("https://xhr-sync-reopen.test/page.html");

    let result = vm
        .eval(
            r#"
(() => {
  const replacedBeforeSend = new XMLHttpRequest();
  const replacedStates = [];
  replacedBeforeSend.onreadystatechange = () => replacedStates.push(replacedBeforeSend.readyState);
  replacedBeforeSend.open("GET", "data:,first");
  replacedBeforeSend.open("GET", "data:,second", false);
  replacedBeforeSend.send();

  const reopenedAfterSend = new XMLHttpRequest();
  const reopenedStates = [];
  reopenedAfterSend.onreadystatechange = () => reopenedStates.push(reopenedAfterSend.readyState);
  reopenedAfterSend.open("GET", "data:,complete", false);
  reopenedAfterSend.send();
  const completed = {
    readyState: reopenedAfterSend.readyState,
    status: reopenedAfterSend.status,
    statusText: reopenedAfterSend.statusText,
    responseText: reopenedAfterSend.responseText,
    responseURL: reopenedAfterSend.responseURL,
    hasHeaders: reopenedAfterSend.getAllResponseHeaders() !== ""
  };
  reopenedAfterSend.open("GET", "/replacement");
  const reset = {
    readyState: reopenedAfterSend.readyState,
    status: reopenedAfterSend.status,
    statusText: reopenedAfterSend.statusText,
    responseText: reopenedAfterSend.responseText,
    responseURL: reopenedAfterSend.responseURL,
    responseXML: reopenedAfterSend.responseXML,
    allHeaders: reopenedAfterSend.getAllResponseHeaders()
  };

  const pendingThenReopened = new XMLHttpRequest();
  const pendingThenReopenedStates = [];
  pendingThenReopened.onreadystatechange = () => {
    pendingThenReopenedStates.push(pendingThenReopened.readyState);
  };
  pendingThenReopened.open("GET", "data:,pending");
  pendingThenReopened.send();
  pendingThenReopened.open("GET", "data:,replacement", false);
  globalThis.__pendingThenReopened = pendingThenReopened;
  globalThis.__pendingThenReopenedStates = pendingThenReopenedStates;
  const pendingReset = {
    states: [...pendingThenReopenedStates],
    readyState: pendingThenReopened.readyState,
    status: pendingThenReopened.status,
    statusText: pendingThenReopened.statusText,
    responseText: pendingThenReopened.responseText,
    responseURL: pendingThenReopened.responseURL,
    responseXML: pendingThenReopened.responseXML,
    allHeaders: pendingThenReopened.getAllResponseHeaders()
  };
  return JSON.stringify({
    replacedStates,
    replacedResponse: replacedBeforeSend.responseText,
    reopenedStates,
    completed,
    reset,
    pendingReset
  });
})()
"#,
        )
        .expect("XHR sync reopen WPT probe should run");

    assert_eq!(
        result,
        r#"{"replacedStates":[1,4],"replacedResponse":"second","reopenedStates":[1,4,1],"completed":{"readyState":4,"status":200,"statusText":"OK","responseText":"complete","responseURL":"data:,complete","hasHeaders":true},"reset":{"readyState":1,"status":0,"statusText":"","responseText":"","responseURL":"","responseXML":null,"allHeaders":""},"pendingReset":{"states":[1],"readyState":1,"status":0,"statusText":"","responseText":"","responseURL":"","responseXML":null,"allHeaders":""}}"#
    );

    vm.eval("0")
        .expect("follow-up checkpoint should discard the canceled async XHR completion");
    let after_checkpoint = vm
        .eval(
            r#"JSON.stringify({
  states: __pendingThenReopenedStates,
  readyState: __pendingThenReopened.readyState,
  status: __pendingThenReopened.status,
  statusText: __pendingThenReopened.statusText,
  responseText: __pendingThenReopened.responseText,
  responseURL: __pendingThenReopened.responseURL,
  responseXML: __pendingThenReopened.responseXML,
  allHeaders: __pendingThenReopened.getAllResponseHeaders()
})"#,
        )
        .expect("canceled async XHR state should remain observable");
    assert_eq!(
        after_checkpoint,
        r#"{"states":[1],"readyState":1,"status":0,"statusText":"","responseText":"","responseURL":"","responseXML":null,"allHeaders":""}"#
    );
}
#[test]
fn xml_http_request_response_text_is_empty_before_loading_state() {
    let mut vm = new_storage_test_vm("https://xhr-response-text-state.test/");

    vm.eval(
        r#"
(() => {
  globalThis.__xhrResponseTextStates = [];
  const xhr = new XMLHttpRequest();
  __xhrResponseTextStates.push(`initial:${xhr.responseText}`);
  xhr.onreadystatechange = () => {
    __xhrResponseTextStates.push(`${xhr.readyState}:${xhr.responseText}`);
  };
  xhr.open('GET', 'data:,TEST');
  xhr.send();
})()
"#,
    )
    .expect("XHR responseText state setup should run");
    vm.eval("0")
        .expect("follow-up checkpoint should drain queued XHR completion");
    let result = vm
        .eval("__xhrResponseTextStates.join('|')")
        .expect("XHR responseText state log should be readable");

    assert_eq!(result, "initial:|1:|2:|3:TEST|4:TEST");
}
#[test]
fn legacy_event_initializers_apply_webidl_conversion() {
    let mut vm = new_storage_test_vm("https://legacy-event-webidl.test/");

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
  const base = new Event('before');
  base.initEvent(null, { valueOf() { throw new RangeError('bubbles'); } }, true);
  const created = document.createEvent({
    toString() {
      return 'UIEvents';
    }
  });
  const ui = new UIEvent('before');
  ui.initUIEvent({ toString() { return 'ui'; } }, 1, '', null, 5.9);
  const custom = new CustomEvent('before', { detail: 'old' });
  custom.initCustomEvent(undefined, 1, '', { toString() { return 'detail-object'; } });
  const text = document.createEvent('TextEvent');
  text.initTextEvent(null, true, 0, null, undefined);
  const composition = new CompositionEvent('before');
  composition.initCompositionEvent('compose', { valueOf() { throw new RangeError('bubbles'); } }, false, window, 'data');
  const keyboard = new KeyboardEvent('keydown', {
    ctrlKey: true,
    altKey: false,
    metaKey: false,
    shiftKey: true
  });
  return [
    `${base.type}:${base.bubbles}:${base.cancelable}`,
    `${created.constructor.name}:${created instanceof UIEvent}`,
    `${ui.type}:${ui.bubbles}:${ui.cancelable}:${ui.view === window}:${ui.detail}`,
    `${custom.type}:${custom.bubbles}:${custom.cancelable}:${custom.detail}`,
    `${text.type}:${text.bubbles}:${text.cancelable}:${text.view === window}:${text.data}`,
    `${composition.type}:${composition.bubbles}:${composition.cancelable}:${composition.view === window}:${composition.data}`,
    probe(() => document.createEvent()),
    probe(() => document.createEvent(Symbol())),
    probe(() => document.createEvent({ toString() { throw new RangeError('interface'); } })),
    probe(() => document.createEvent('MissingEvent')),
    probe(() => {
      const event = new Event('x');
      event.initEvent();
    }),
    probe(() => {
      const event = new Event('x');
      event.initEvent(Symbol());
    }),
    probe(() => {
      const event = new Event('x');
      event.initEvent({ toString() { throw new RangeError('type'); } });
    }),
    probe(() => {
      const event = new UIEvent('x');
      event.initUIEvent();
    }),
    probe(() => {
      const event = new UIEvent('x');
      event.initUIEvent(Symbol());
    }),
    probe(() => {
      const event = new UIEvent('x');
      event.initUIEvent('x', false, false, window, Symbol());
    }),
    probe(() => {
      const event = document.createEvent('TextEvent');
      event.initTextEvent('x', false, false, window, Symbol());
    }),
    probe(() => {
      const event = new CompositionEvent('x');
      event.initCompositionEvent({ toString() { throw new RangeError('type'); } });
    }),
    probe(() => keyboard.getModifierState('Control')),
    probe(() => keyboard.getModifierState(null)),
    probe(() => keyboard.getModifierState()),
    probe(() => keyboard.getModifierState(Symbol()))
  ].join('|');
})()
"#,
        )
        .expect("legacy event initializer WebIDL conversion probe should run");

    assert_eq!(
        result,
        "null:true:true|UIEvent:true|ui:true:false:true:5|undefined:true:false:detail-object|null:true:false:false:undefined|compose:true:false:true:data|throw:TypeError|throw:TypeError|throw:RangeError|throw:NotSupportedError|throw:TypeError|throw:TypeError|throw:RangeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:TypeError|throw:RangeError|true|false|throw:TypeError|throw:TypeError"
    );
}

#[test]
fn composition_event_legacy_initializer_requires_type_argument() {
    let mut vm = new_storage_test_vm("https://composition-event-legacy-init.test/");

    let result = vm
        .eval(
            r#"
(() => {
  const event = new CompositionEvent('before', { data: 'before-data' });
  let missingArgumentError = null;
  try {
    event.initCompositionEvent();
  } catch (error) {
    missingArgumentError = error && error.name;
  }
  const afterFailure = [event.type, event.data];
  event.initCompositionEvent('after');
  return JSON.stringify({
    length: CompositionEvent.prototype.initCompositionEvent.length,
    missingArgumentError,
    afterFailure,
    initialized: [
      event.type,
      event.bubbles,
      event.cancelable,
      event.data
    ]
  });
})()
"#,
        )
        .expect("legacy CompositionEvent initializer probe should evaluate");

    assert_eq!(
        result,
        r#"{"length":1,"missingArgumentError":"TypeError","afterFailure":["before","before-data"],"initialized":["after",false,false,""]}"#,
    );
}
#[test]
fn pending_network_body_stream_emits_small_chunks_before_close() {
    let mut vm = new_storage_test_vm("https://pending-stream-small-chunks.test/");
    let body_source_id = crate::network_host::new_network_body_source_id();

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let body_owner = pending_body_owner_probe(scope);
            let stream =
                crate::network_host::pending_network_body_stream(scope, body_owner, body_source_id);
            let global = context.global(scope);
            let _ = global.set(scope, v8str(scope, "__pendingStream").into(), stream.into());
            Ok(())
        })
        .expect("pending stream should be installed");

    let initial = vm
        .eval(
            r#"
(() => {
  globalThis.__pendingStreamEvents = [];
  const reader = globalThis.__pendingStream.getReader();
  reader.read().then(({ value, done }) => {
    globalThis.__pendingStreamEvents.push(
      done ? "done" : new TextDecoder().decode(value)
    );
  });
  return JSON.stringify(globalThis.__pendingStreamEvents);
})()
"#,
        )
        .expect("pending stream read should be registered");
    assert_eq!(initial, "[]");

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
                b"event: small\n\n".to_vec(),
            );
            Ok(())
        })
        .expect("small pending stream chunk should enqueue");

    vm.eval("''")
        .expect("next script turn should drain pending stream read reaction");

    let after_chunk = vm
        .eval("JSON.stringify(globalThis.__pendingStreamEvents)")
        .expect("pending stream read should settle");
    assert_eq!(after_chunk, r#"["event: small\n\n"]"#);
}

#[test]
fn pending_network_body_stream_buffers_future_chunks_until_next_pull() {
    let mut vm = new_storage_test_vm("https://pending-stream-backpressure.test/");
    let body_source_id = crate::network_host::new_network_body_source_id();

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let body_owner = pending_body_owner_probe(scope);
            let stream =
                crate::network_host::pending_network_body_stream(scope, body_owner, body_source_id);
            let global = context.global(scope);
            let _ = global.set(scope, v8str(scope, "__pendingStream").into(), stream.into());
            Ok(())
        })
        .expect("pending stream should be installed");

    vm.eval(
        r#"
(() => {
  globalThis.__pendingStreamEvents = [];
  globalThis.__pendingReader = globalThis.__pendingStream.getReader();
  globalThis.__pendingReader.read().then(({ value, done }) => {
    globalThis.__pendingStreamEvents.push(
      done ? "done" : new TextDecoder().decode(value)
    );
  });
})()
"#,
    )
    .expect("first pending stream read should be registered");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    let buffered_len = vm
        .renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            crate::network_host::enqueue_pending_network_body_chunk(
                scope,
                body_source_id,
                b"first".to_vec(),
            );
            crate::network_host::enqueue_pending_network_body_chunk(
                scope,
                body_source_id,
                b"second".to_vec(),
            );
            Ok(
                crate::network_host::pending_network_body_source_buffered_len_for_test(
                    scope,
                    body_source_id,
                )
                .expect("pending body source should remain after one delivered chunk"),
            )
        })
        .expect("pending stream chunks should enqueue");
    assert_eq!(
        buffered_len,
        b"second".len(),
        "future chunks should stay in the pending source until the next pull"
    );

    vm.eval("0")
        .expect("first pending stream read reaction should drain");
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__pendingStreamEvents)")
            .expect("first pending stream event should be visible"),
        r#"["first"]"#
    );

    vm.eval(
        r#"
globalThis.__pendingReader.read().then(({ value, done }) => {
  globalThis.__pendingStreamEvents.push(
    done ? "done" : new TextDecoder().decode(value)
  );
})
"#,
    )
    .expect("second pending stream read should be registered");
    vm.eval("0")
        .expect("second pending stream read reaction should drain");
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__pendingStreamEvents)")
            .expect("second pending stream event should be visible"),
        r#"["first","second"]"#
    );
}

#[test]
fn streaming_xhr_coalesces_progress_while_publishing_partial_text_before_done() {
    let mut vm = new_storage_test_vm("https://xhr-streaming-visible.test/");
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Xhr));
    vm.eval(
        r#"
globalThis.__streamingXhrEvents = [];
globalThis.__streamingXhr = new XMLHttpRequest();
__streamingXhr.onreadystatechange = () => {
  __streamingXhrEvents.push(
    `readystatechange:${__streamingXhr.readyState}:${__streamingXhr.status}:${__streamingXhr.responseText}`
  );
};
__streamingXhr.onprogress = event => {
  __streamingXhrEvents.push(
    `progress:${event.loaded}:${event.total}:${event.lengthComputable}`
  );
};
__streamingXhr.onload = () => __streamingXhrEvents.push("load");
__streamingXhr.onloadend = () => __streamingXhrEvents.push("loadend");
__streamingXhr.open("GET", "/data");
__streamingXhr.send();
"#,
    )
    .expect("streaming XHR should be intercepted");

    let pending = vm.take_pending_subresource_fetch_infos();
    assert_eq!(pending.len(), 1);
    let pending = &pending[0];
    let internal_id = pending.internal_id;
    let request_url = pending.url.clone();
    let body_source_id = crate::network_host::new_network_body_source_id();
    let response_head = moli_fetch::ResponseHead {
        final_url: request_url.clone(),
        status: 200,
        headers: vec![
            (
                "Content-Type".to_owned(),
                "text/plain; charset=utf-8".to_owned(),
            ),
            ("Content-Length".to_owned(), "7".to_owned()),
        ],
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        redirected: false,
        redirect_chain: Vec::new(),
        from_cache: false,
        negotiated_http_version: None,
    };

    vm.start_streaming_async_subresource_fetch(crate::types::AsyncSubresourceStreamingStarted {
        internal_id,
        request_url: request_url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        body_source_id,
        head: response_head,
        network_request_headers: None,
    })
    .expect("streaming XHR headers should be delivered");
    assert_eq!(
        vm.eval(
            r#"JSON.stringify({
  events: __streamingXhrEvents,
  readyState: __streamingXhr.readyState,
  status: __streamingXhr.status,
  responseText: __streamingXhr.responseText,
  contentType: __streamingXhr.getResponseHeader("content-type")
})"#,
        )
        .expect("XHR response head should be Web-visible"),
        r#"{"events":["readystatechange:1:0:","readystatechange:2:200:"],"readyState":2,"status":200,"responseText":"","contentType":"text/plain; charset=utf-8"}"#
    );

    vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"hi \xe2".to_vec());
    assert_eq!(
        vm.eval("JSON.stringify([__streamingXhr.readyState, __streamingXhr.responseText, __streamingXhrEvents])")
            .expect("first XHR body chunk should be Web-visible"),
        r#"[3,"hi ",["readystatechange:1:0:","readystatechange:2:200:","readystatechange:3:200:hi ","progress:4:7:true"]]"#
    );

    vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"\x82\xac".to_vec());
    assert_eq!(
        vm.eval("JSON.stringify([__streamingXhr.readyState, __streamingXhr.responseText, __streamingXhrEvents])")
            .expect("split UTF-8 XHR body chunk should be Web-visible"),
        r#"[3,"hi €",["readystatechange:1:0:","readystatechange:2:200:","readystatechange:3:200:hi ","progress:4:7:true"]]"#,
        "a chunk inside the 50 ms gate updates responseText without exposing libcurl chunking"
    );

    vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"!".to_vec());
    assert_eq!(
        vm.eval("JSON.stringify([__streamingXhr.responseText, __streamingXhrEvents.length])")
            .expect("latest coalesced XHR body chunk should be Web-visible"),
        r#"["hi €!",4]"#,
        "only the latest pending progress snapshot should remain before completion"
    );

    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("streaming XHR should finish");
    assert_eq!(
        vm.eval("JSON.stringify([__streamingXhr.readyState, __streamingXhr.status, __streamingXhr.responseText, __streamingXhrEvents.slice(-5)])")
            .expect("completed streaming XHR should be Web-visible"),
        r#"[4,200,"hi €!",["readystatechange:3:200:hi €!","progress:7:7:true","readystatechange:4:200:hi €!","load","loadend"]]"#,
        "DONE must flush the latest deferred progress before readystatechange 4"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn streaming_subresource_finish_preserves_response_head_cache_state() {
    let load_owner =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("streaming subresource test loader");
    let load_client = load_owner.handle();
    let mut vm = new_storage_test_vm("https://xhr-streaming-cache-state.test/");
    let internal_id = 77;
    let body_source_id = crate::network_host::new_network_body_source_id();
    let document_url =
        Url::parse("https://xhr-streaming-cache-state.test/").expect("document URL should parse");
    let request_url = Url::parse("https://xhr-streaming-cache-state.test/cached-redirect")
        .expect("request URL should parse");
    let final_url =
        Url::parse("https://xhr-streaming-cache-state.test/data").expect("final URL should parse");
    let mut body_writer = crate::types::SubresourceResponseBodyWriter::default();
    body_writer.append(b"cached-body");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    let context_host = vm._context_host.clone();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate({
            let document_url = document_url.clone();
            let request_url = request_url.clone();
            let final_url = final_url.clone();
            move |isolate| {
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);

                let body_owner = pending_body_owner_probe(scope);
                let _stream = crate::network_host::pending_network_body_stream(
                    scope,
                    body_owner,
                    body_source_id,
                );
                let xhr = streaming_xhr_probe(scope);

                context_host
                    .borrow_mut()
                    .record_streaming_subresource_fetch(super::StreamingSubresourceFetchState {
                        pending: super::PendingSubresourceFetchState {
                            info: crate::types::PendingSubresourceFetchInfo {
                                internal_id,
                                network_request_handle: None,
                                frame_id: None,
                                document_url: document_url.clone(),
                                url: request_url.clone(),
                                websocket_socket_id: None,
                                method: "GET".to_owned(),
                                request_headers: Vec::new(),
                                request_body: None,
                                request_body_bytes: None,
                                resource_type: crate::types::SubresourceResourceType::Xhr,
                                request_cookie_report: None,
                            },
                            execution_context:
                                crate::types::PendingSubresourceExecutionContext::adapter(
                                    crate::native_bridge::OwnerDispatchScope::Top,
                                    v8::Global::new(scope, context),
                                ),
                            credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                            request_mode: moli_fetch::RequestMode::Cors,
                            network_partition_key: None,
                            policy_context: Default::default(),
                            continuation: super::PendingSubresourceContinuation::Xhr(
                                v8::Global::new(scope, xhr),
                            ),
                            load: crate::network::loads::resource_load_lease_for_test(
                                load_client,
                                None,
                            ),
                            deferred_request_started: false,
                        },
                        request_url: request_url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                        body_source_id,
                        head: moli_fetch::ResponseHead {
                            final_url: final_url.clone(),
                            status: 200,
                            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                            request_cookie_report: None,
                            cookie_set_reports: Vec::new(),
                            redirected: true,
                            redirect_chain: vec![moli_fetch::RedirectInfo {
                                from_url: request_url.clone(),
                                to_url: final_url.clone(),
                                status: 301,
                                headers: vec![("location".to_owned(), final_url.to_string())],
                                network_extra_info_available: false,
                                request_extra_info: None,
                                response_extra_info: None,
                                redirect_has_extra_info: false,
                                request_cookie_report: None,
                                cookie_set_reports: Vec::new(),
                                from_cache: true,
                                negotiated_http_version: None,
                            }],
                            from_cache: true,
                            negotiated_http_version: None,
                        },
                        network_request_headers: None,
                        body_writer,
                        event_source_parser: None,
                        xhr_response: None,
                    });
                Ok(())
            }
        })
        .expect("streaming XHR fixture should be recorded");

    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("streaming XHR finish should record success");

    let records: Vec<_> = vm
        ._context_host
        .borrow_mut()
        .take_network_output()
        .into_items()
        .filter_map(|item| match item {
            crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
                Some(*record)
            }
            _ => None,
        })
        .collect();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert!(
        record.from_cache(),
        "streaming completion record must preserve final response cache state"
    );
    match record.outcome() {
        crate::types::SubresourceNetworkOutcome::Success {
            redirect_chain,
            final_url: recorded_final_url,
            ..
        } => {
            assert_eq!(recorded_final_url.as_str(), final_url.as_str());
            assert_eq!(redirect_chain.len(), 1);
            assert!(
                redirect_chain[0].from_cache,
                "streaming completion record must preserve cached redirect provenance"
            );
        }
        other => panic!("expected streaming XHR success, got {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn async_subresource_failure_network_error_override_preserves_fetch_rejection_message() {
    let load_owner =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("async subresource failure test loader");
    let load_client = load_owner.handle();
    let mut vm = new_storage_test_vm("https://sw-non-stream-failure.test/");
    let internal_id = 91;
    let document_url =
        Url::parse("https://sw-non-stream-failure.test/").expect("document URL should parse");
    let request_url =
        Url::parse("https://sw-non-stream-failure.test/data").expect("request URL should parse");
    let rejection_message = "FetchEvent.respondWith rejected an error Response";

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    let context_host = vm._context_host.clone();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate({
            let document_url = document_url.clone();
            let request_url = request_url.clone();
            move |isolate| {
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let resolver = v8::PromiseResolver::new(scope).expect("resolver should be created");
                let promise = resolver.get_promise(scope);
                let global = context.global(scope);
                let _ = global.set(
                    scope,
                    v8str(scope, "__swNonStreamFailurePromise").into(),
                    promise.into(),
                );
                let continuation =
                    pending_fetch_continuation(scope, resolver, &context_host.borrow());

                context_host.borrow_mut().restore_pending_subresource_fetch(
                    super::PendingSubresourceFetchState {
                        info: crate::types::PendingSubresourceFetchInfo {
                            internal_id,
                            network_request_handle: None,
                            frame_id: Some("FRAME-1".to_owned()),
                            document_url,
                            url: request_url.clone(),
                            websocket_socket_id: None,
                            method: "GET".to_owned(),
                            request_headers: Vec::new(),
                            request_body: None,
                            request_body_bytes: None,
                            resource_type: crate::types::SubresourceResourceType::Fetch,
                            request_cookie_report: None,
                        },
                        execution_context:
                            crate::types::PendingSubresourceExecutionContext::adapter(
                                crate::native_bridge::OwnerDispatchScope::Top,
                                v8::Global::new(scope, context),
                            ),
                        credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                        request_mode: moli_fetch::RequestMode::Cors,
                        network_partition_key: None,
                        policy_context: Default::default(),
                        continuation,
                        load: crate::network::loads::resource_load_lease_for_test(
                            load_client,
                            None,
                        ),
                        deferred_request_started: false,
                    },
                );
                Ok(())
            }
        })
        .expect("pending fetch fixture should be recorded");

    vm.eval(
        r#"
globalThis.__swNonStreamFailureProbe = [];
__swNonStreamFailurePromise.then(
  () => __swNonStreamFailureProbe.push("resolved"),
  error => __swNonStreamFailureProbe.push(error.name + ":" + error.message)
);
"#,
    )
    .expect("promise observer should install");

    vm.complete_async_subresource_fetch(crate::types::AsyncSubresourceFetchCompletion {
        internal_id,
        request_url: request_url.clone(),
        request_method: "GET".to_owned(),
        request_headers: Vec::new(),
        request_body: None,
        response_status_text: None,
        skip_fetch_security_validation: false,
        response_filter: None,
        network_error_text: Some(crate::network_host::FAILED_ERROR_TEXT.to_owned()),
        result: Err(rejection_message.to_owned()),
    })
    .expect("async subresource failure should settle pending fetch");

    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__swNonStreamFailureProbe)")
            .expect("fetch rejection probe should evaluate"),
        format!(r#"["TypeError:{rejection_message}"]"#)
    );

    let records: Vec<_> = vm
        ._context_host
        .borrow_mut()
        .take_network_output()
        .into_items()
        .filter_map(|item| match item {
            crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
                Some(*record)
            }
            _ => None,
        })
        .collect();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].url(), &request_url);
    match records[0].outcome() {
        crate::types::SubresourceNetworkOutcome::Failure { error_text } => {
            assert_eq!(error_text, crate::network_host::FAILED_ERROR_TEXT);
        }
        other => panic!("expected network failure record, got {other:?}"),
    }

    assert!(
        vm._context_host
            .borrow_mut()
            .take_pending_subresource_continue_events()
            .is_empty(),
        "plain pending completion should not synthesize a continue event"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn streaming_fetch_body_error_records_response_started_then_body_failed() {
    let load_owner =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("streaming fetch error test loader");
    let load_client = load_owner.handle();
    let mut vm = new_storage_test_vm("https://streaming-fetch-body-error.test/");
    let internal_id = 100;
    let body_source_id = crate::network_host::new_network_body_source_id();
    let request_handle = crate::types::SubresourceNetworkRequestHandle::new(44);
    let request_url = Url::parse("https://streaming-fetch-body-error.test/data")
        .expect("request URL should parse");
    let final_url = Url::parse("https://streaming-fetch-body-error.test/final")
        .expect("final URL should parse");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    let context_host = vm._context_host.clone();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate({
            let request_url = request_url.clone();
            let final_url = final_url.clone();
            move |isolate| {
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let resolver = v8::PromiseResolver::new(scope).expect("resolver should be created");
                let mut body_writer = crate::types::SubresourceResponseBodyWriter::default();
                body_writer.append(b"partial");
                let continuation =
                    pending_fetch_continuation(scope, resolver, &context_host.borrow());

                context_host
                    .borrow_mut()
                    .record_streaming_subresource_fetch(super::StreamingSubresourceFetchState {
                        pending: super::PendingSubresourceFetchState {
                            info: crate::types::PendingSubresourceFetchInfo {
                                internal_id,
                                network_request_handle: Some(request_handle),
                                frame_id: Some("FRAME-1".to_owned()),
                                document_url: Url::parse(
                                    "https://streaming-fetch-body-error.test/",
                                )
                                .expect("document URL should parse"),
                                url: request_url.clone(),
                                websocket_socket_id: None,
                                method: "GET".to_owned(),
                                request_headers: Vec::new(),
                                request_body: None,
                                request_body_bytes: None,
                                resource_type: crate::types::SubresourceResourceType::Fetch,
                                request_cookie_report: None,
                            },
                            execution_context:
                                crate::types::PendingSubresourceExecutionContext::adapter(
                                    crate::native_bridge::OwnerDispatchScope::Top,
                                    v8::Global::new(scope, context),
                                ),
                            credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                            request_mode: moli_fetch::RequestMode::Cors,
                            network_partition_key: None,
                            policy_context: Default::default(),
                            continuation,
                            load: crate::network::loads::resource_load_lease_for_test(
                                load_client,
                                None,
                            ),
                            deferred_request_started: false,
                        },
                        request_url: request_url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                        body_source_id,
                        head: moli_fetch::ResponseHead {
                            final_url: final_url.clone(),
                            status: 206,
                            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                            request_cookie_report: None,
                            cookie_set_reports: Vec::new(),
                            redirected: false,
                            redirect_chain: Vec::new(),
                            from_cache: false,
                            negotiated_http_version: None,
                        },
                        network_request_headers: None,
                        body_writer,
                        event_source_parser: None,
                        xhr_response: None,
                    });
                Ok(())
            }
        })
        .expect("streaming fetch fixture should be recorded");

    vm.finish_streaming_async_subresource_fetch(
        internal_id,
        body_source_id,
        Err("stream aborted after partial body".to_owned()),
    )
    .expect("streaming fetch finish error should record staged network output");

    let items: Vec<_> = vm
        ._context_host
        .borrow_mut()
        .take_network_output()
        .into_items()
        .collect();
    assert_eq!(items.len(), 2);
    match &items[0] {
        crate::types::ScriptNetworkOutputItem::SubresourceResponseStarted(response) => {
            assert_eq!(response.handle(), request_handle);
            assert_eq!(response.final_url(), &final_url);
            assert_eq!(response.status(), 206);
        }
        other => panic!("expected responseStarted item, got {other:?}"),
    }
    match &items[1] {
        crate::types::ScriptNetworkOutputItem::SubresourceBodyFinished(body) => {
            assert_eq!(body.handle(), request_handle);
            match body.result() {
                crate::types::SubresourceBodyFinishedResult::FailedWithPartialBody {
                    error_text,
                    partial_body,
                } => {
                    assert_eq!(error_text, crate::network_host::ABORTED_ERROR_TEXT);
                    assert_eq!(partial_body.diagnostic_bytes().as_ref(), b"partial");
                }
                other => panic!("expected failed body with partial payload, got {other:?}"),
            }
        }
        other => panic!("expected bodyFinished failure item, got {other:?}"),
    }

    let events = vm
        ._context_host
        .borrow_mut()
        .take_pending_subresource_continue_events();
    assert_eq!(
        events,
        vec![crate::types::PendingSubresourceContinueEvent::Completed { internal_id }]
    );
}

fn install_streaming_fetch_response_fixture(
    vm: &mut ScriptVm,
    internal_id: u64,
    body_source_id: crate::types::NetworkBodySourceId,
    request_url: Url,
    cancel_handle: moli_fetch::FetchCancelHandle,
    network_request_handle: Option<crate::types::SubresourceNetworkRequestHandle>,
    global_name: &'static str,
    load_client: crate::network::ResourceRequestClient,
) {
    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    let context_host = vm._context_host.clone();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);
            let document_url =
                Url::parse("https://streaming-fetch-body-cancel.test/").expect("URL should parse");
            let resolver = v8::PromiseResolver::new(scope).expect("resolver should be created");
            let response =
                crate::network_host::build_fetch_response_object_from_stream_for_request_mode(
                    scope,
                    &document_url,
                    moli_fetch::RequestMode::Cors,
                    moli_fetch::ResponseHead {
                        final_url: request_url.clone(),
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
            let _ = global.set(scope, v8str(scope, global_name).into(), response.into());
            let continuation = pending_fetch_continuation(scope, resolver, &context_host.borrow());

            context_host
                .borrow_mut()
                .record_streaming_subresource_fetch(super::StreamingSubresourceFetchState {
                    pending: super::PendingSubresourceFetchState {
                        info: crate::types::PendingSubresourceFetchInfo {
                            internal_id,
                            network_request_handle,
                            frame_id: None,
                            document_url,
                            url: request_url.clone(),
                            websocket_socket_id: None,
                            method: "GET".to_owned(),
                            request_headers: Vec::new(),
                            request_body: None,
                            request_body_bytes: None,
                            resource_type: crate::types::SubresourceResourceType::Fetch,
                            request_cookie_report: None,
                        },
                        execution_context:
                            crate::types::PendingSubresourceExecutionContext::adapter(
                                crate::native_bridge::OwnerDispatchScope::Top,
                                v8::Global::new(scope, context),
                            ),
                        credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                        request_mode: moli_fetch::RequestMode::Cors,
                        network_partition_key: None,
                        policy_context: Default::default(),
                        continuation,
                        load: crate::network::loads::resource_load_lease_for_test(
                            load_client,
                            Some(cancel_handle),
                        ),
                        deferred_request_started: false,
                    },
                    request_url: request_url.clone(),
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    request_body: None,
                    body_source_id,
                    head: moli_fetch::ResponseHead {
                        final_url: request_url,
                        status: 200,
                        headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                        request_cookie_report: None,
                        cookie_set_reports: Vec::new(),
                        redirected: false,
                        redirect_chain: Vec::new(),
                        from_cache: false,
                        negotiated_http_version: None,
                    },
                    network_request_headers: None,
                    body_writer: Default::default(),
                    event_source_parser: None,
                    xhr_response: None,
                });
            Ok(())
        })
        .expect("streaming fetch fixture should be recorded");
}

#[tokio::test(flavor = "current_thread")]
async fn streaming_fetch_body_cancel_aborts_streaming_subresource() {
    let load_owner =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("streaming fetch cancellation test loader");
    let load_client = load_owner.handle();
    let mut vm = new_storage_test_vm("https://streaming-fetch-body-cancel.test/");
    let internal_id = 101;
    let body_source_id = crate::network_host::new_network_body_source_id();
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let request_url = Url::parse("https://streaming-fetch-body-cancel.test/data")
        .expect("request URL should parse");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    let context_host = vm._context_host.clone();
    let cancel_handle_for_state = cancel_handle.clone();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate({
            let request_url = request_url.clone();
            move |isolate| {
                let scope = std::pin::pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let resolver = v8::PromiseResolver::new(scope).expect("resolver should be created");
                let body_owner = pending_body_owner_probe(scope);
                let stream = crate::network_host::pending_network_body_stream(
                    scope,
                    body_owner,
                    body_source_id,
                );
                let global = context.global(scope);
                let _ = global.set(
                    scope,
                    v8str(scope, "__streamingFetchBody").into(),
                    stream.into(),
                );
                let continuation =
                    pending_fetch_continuation(scope, resolver, &context_host.borrow());

                context_host
                    .borrow_mut()
                    .record_streaming_subresource_fetch(super::StreamingSubresourceFetchState {
                        pending: super::PendingSubresourceFetchState {
                            info: crate::types::PendingSubresourceFetchInfo {
                                internal_id,
                                network_request_handle: None,
                                frame_id: None,
                                document_url: Url::parse(
                                    "https://streaming-fetch-body-cancel.test/",
                                )
                                .expect("document URL should parse"),
                                url: request_url.clone(),
                                websocket_socket_id: None,
                                method: "GET".to_owned(),
                                request_headers: Vec::new(),
                                request_body: None,
                                request_body_bytes: None,
                                resource_type: crate::types::SubresourceResourceType::Fetch,
                                request_cookie_report: None,
                            },
                            execution_context:
                                crate::types::PendingSubresourceExecutionContext::adapter(
                                    crate::native_bridge::OwnerDispatchScope::Top,
                                    v8::Global::new(scope, context),
                                ),
                            credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                            request_mode: moli_fetch::RequestMode::Cors,
                            network_partition_key: None,
                            policy_context: Default::default(),
                            continuation,
                            load: crate::network::loads::resource_load_lease_for_test(
                                load_client,
                                Some(cancel_handle_for_state),
                            ),
                            deferred_request_started: false,
                        },
                        request_url: request_url.clone(),
                        request_method: "GET".to_owned(),
                        request_headers: Vec::new(),
                        request_body: None,
                        body_source_id,
                        head: moli_fetch::ResponseHead {
                            final_url: request_url,
                            status: 200,
                            headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                            request_cookie_report: None,
                            cookie_set_reports: Vec::new(),
                            redirected: false,
                            redirect_chain: Vec::new(),
                            from_cache: false,
                            negotiated_http_version: None,
                        },
                        network_request_headers: None,
                        body_writer: Default::default(),
                        event_source_parser: None,
                        xhr_response: None,
                    });
                Ok(())
            }
        })
        .expect("streaming fetch fixture should be recorded");

    vm.eval(
        r#"
(() => {
  globalThis.__streamingFetchBodyCancelProbe = [];
  globalThis.__streamingFetchBody.cancel("consumer done").then(
    () => globalThis.__streamingFetchBodyCancelProbe.push("resolved"),
    error => globalThis.__streamingFetchBodyCancelProbe.push(`rejected:${error}`)
  );
})()
"#,
    )
    .expect("streaming fetch body cancel should evaluate");

    assert!(
        cancel_handle.is_cancelled(),
        "canceling the Web-visible stream should abort the upstream fetch"
    );
    vm.eval("0")
        .expect("streaming fetch body cancel promise reaction should drain");
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__streamingFetchBodyCancelProbe)")
            .expect("streaming fetch body cancel probe should evaluate"),
        r#"["resolved"]"#
    );
    let events = vm
        ._context_host
        .borrow_mut()
        .take_pending_subresource_continue_events();
    assert_eq!(
        events,
        vec![crate::types::PendingSubresourceContinueEvent::Completed { internal_id }]
    );

    let items: Vec<_> = vm
        ._context_host
        .borrow_mut()
        .take_network_output()
        .into_items()
        .collect();
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
            assert_eq!(record.url(), &request_url);
            match record.outcome() {
                crate::types::SubresourceNetworkOutcome::Failure { error_text } => {
                    assert_eq!(error_text, crate::network_host::ABORTED_ERROR_TEXT);
                }
                other => panic!("expected body cancel failure record, got {other:?}"),
            }
        }
        other => panic!("expected body cancel failure record, got {other:?}"),
    }

    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("late streaming finish after body cancel should be ignored");
    assert!(
        vm._context_host
            .borrow_mut()
            .take_network_output()
            .into_items()
            .next()
            .is_none(),
        "late streaming finish must not record a completed network response after cancel"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn streaming_fetch_body_cancel_records_response_started_then_body_failed() {
    let load_owner =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("streaming fetch cancellation test loader");
    let mut vm = new_storage_test_vm("https://streaming-fetch-body-cancel.test/");
    let internal_id = 104;
    let body_source_id = crate::network_host::new_network_body_source_id();
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let request_handle = crate::types::SubresourceNetworkRequestHandle::new(45);
    let request_url = Url::parse("https://streaming-fetch-body-cancel.test/data")
        .expect("request URL should parse");
    install_streaming_fetch_response_fixture(
        &mut vm,
        internal_id,
        body_source_id,
        request_url.clone(),
        cancel_handle.clone(),
        Some(request_handle),
        "__streamingFetchResponse",
        load_owner.handle(),
    );

    vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"partial".to_vec());
    vm.eval(
        r#"
(() => {
  globalThis.__streamingFetchBodyCancelNetworkProbe = [];
  globalThis.__streamingFetchResponse.body.cancel("consumer done").then(
    () => globalThis.__streamingFetchBodyCancelNetworkProbe.push("resolved"),
    error => globalThis.__streamingFetchBodyCancelNetworkProbe.push(`rejected:${error}`)
  );
})()
"#,
    )
    .expect("streaming fetch body cancel should evaluate");

    assert!(
        cancel_handle.is_cancelled(),
        "canceling the Web-visible response body should abort upstream"
    );
    vm.eval("0")
        .expect("streaming fetch body cancel promise reaction should drain");
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__streamingFetchBodyCancelNetworkProbe)")
            .expect("streaming fetch body cancel probe should evaluate"),
        r#"["resolved"]"#
    );

    let items: Vec<_> = vm
        ._context_host
        .borrow_mut()
        .take_network_output()
        .into_items()
        .collect();
    assert_eq!(items.len(), 2);
    match &items[0] {
        crate::types::ScriptNetworkOutputItem::SubresourceResponseStarted(response) => {
            assert_eq!(response.handle(), request_handle);
            assert_eq!(response.final_url(), &request_url);
            assert_eq!(response.status(), 200);
        }
        other => panic!("expected responseStarted item, got {other:?}"),
    }
    match &items[1] {
        crate::types::ScriptNetworkOutputItem::SubresourceBodyFinished(body) => {
            assert_eq!(body.handle(), request_handle);
            match body.result() {
                crate::types::SubresourceBodyFinishedResult::FailedWithPartialBody {
                    error_text,
                    partial_body,
                } => {
                    assert_eq!(error_text, crate::network_host::ABORTED_ERROR_TEXT);
                    assert_eq!(partial_body.diagnostic_bytes().as_ref(), b"partial");
                }
                other => panic!("expected failed body with partial payload, got {other:?}"),
            }
        }
        other => panic!("expected bodyFinished failure item, got {other:?}"),
    }

    let events = vm
        ._context_host
        .borrow_mut()
        .take_pending_subresource_continue_events();
    assert_eq!(
        events,
        vec![crate::types::PendingSubresourceContinueEvent::Completed { internal_id }]
    );

    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("late streaming finish after body cancel should be ignored");
    assert!(
        vm._context_host
            .borrow_mut()
            .take_network_output()
            .into_items()
            .next()
            .is_none(),
        "late streaming finish must not append success after body cancel failure"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn streaming_fetch_body_clone_branches_both_receive_complete_body() {
    let load_owner =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("streaming fetch clone test loader");
    let mut vm = new_storage_test_vm("https://streaming-fetch-body-cancel.test/");
    let internal_id = 105;
    let body_source_id = crate::network_host::new_network_body_source_id();
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let request_url = Url::parse("https://streaming-fetch-body-cancel.test/data")
        .expect("request URL should parse");
    install_streaming_fetch_response_fixture(
        &mut vm,
        internal_id,
        body_source_id,
        request_url,
        cancel_handle.clone(),
        None,
        "__streamingFetchResponse",
        load_owner.handle(),
    );

    vm.eval(
        r#"
(() => {
  globalThis.__streamingFetchTeeProbe = [];
  const response = globalThis.__streamingFetchResponse;
  const clone = response.clone();
  response.text().then(
    text => globalThis.__streamingFetchTeeProbe.push(`original:${text}`),
    error => globalThis.__streamingFetchTeeProbe.push(
      `original-error:${error && error.name}:${error && error.message}`
    )
  );
  clone.text().then(
    text => globalThis.__streamingFetchTeeProbe.push(`clone:${text}`),
    error => globalThis.__streamingFetchTeeProbe.push(
      `clone-error:${error && error.name}:${error && error.message}`
    )
  );
})()
"#,
    )
    .expect("streaming fetch tee probe should evaluate");

    vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"AB".to_vec());
    vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"UX".to_vec());
    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("streaming fetch finish should close both clone branches");
    for _ in 0..12 {
        vm.eval("0")
            .expect("streaming fetch tee promise chain should drain");
    }

    assert!(
        !cancel_handle.is_cancelled(),
        "successful tee consumption must not abort upstream"
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__streamingFetchTeeProbe.sort())")
            .expect("streaming fetch tee probe should evaluate"),
        r#"["clone:ABUX","original:ABUX"]"#
    );
    let events = vm
        ._context_host
        .borrow_mut()
        .take_pending_subresource_continue_events();
    assert_eq!(
        events,
        vec![crate::types::PendingSubresourceContinueEvent::Completed { internal_id }]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn streaming_fetch_body_cancel_does_not_abort_live_clone_branch() {
    let load_owner =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("streaming fetch live-clone test loader");
    let mut vm = new_storage_test_vm("https://streaming-fetch-body-cancel.test/");
    let internal_id = 102;
    let body_source_id = crate::network_host::new_network_body_source_id();
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let request_url = Url::parse("https://streaming-fetch-body-cancel.test/data")
        .expect("request URL should parse");
    install_streaming_fetch_response_fixture(
        &mut vm,
        internal_id,
        body_source_id,
        request_url,
        cancel_handle.clone(),
        None,
        "__streamingFetchResponse",
        load_owner.handle(),
    );

    vm.eval(
        r#"
(() => {
  globalThis.__streamingFetchCloneCancelProbe = [];
  const response = globalThis.__streamingFetchResponse;
  const clone = response.clone();
  response.body.cancel("original done").then(
    () => globalThis.__streamingFetchCloneCancelProbe.push("cancel:resolved"),
    error => globalThis.__streamingFetchCloneCancelProbe.push(`cancel:rejected:${error}`)
  );
  clone.text().then(
    text => globalThis.__streamingFetchCloneCancelProbe.push(`clone:${text}`),
    error => globalThis.__streamingFetchCloneCancelProbe.push(
      `clone-error:${error && error.name}:${error && error.message}`
    )
  );
})()
"#,
    )
    .expect("streaming fetch clone cancel probe should evaluate");

    assert!(
        !cancel_handle.is_cancelled(),
        "canceling one tee branch must not abort the upstream fetch"
    );
    assert!(
        vm._context_host
            .borrow_mut()
            .take_pending_subresource_continue_events()
            .is_empty(),
        "one live clone branch should keep the streaming fetch pending"
    );

    vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"hello ".to_vec());
    vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"clone".to_vec());
    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("streaming fetch finish should close the live clone branch");
    for _ in 0..12 {
        vm.eval("0")
            .expect("streaming fetch clone cancel promise chain should drain");
    }

    assert!(
        !cancel_handle.is_cancelled(),
        "successful completion of the live clone branch must not abort upstream"
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__streamingFetchCloneCancelProbe.sort())")
            .expect("streaming fetch clone cancel probe should evaluate"),
        r#"["cancel:resolved","clone:hello clone"]"#
    );
    let events = vm
        ._context_host
        .borrow_mut()
        .take_pending_subresource_continue_events();
    assert_eq!(
        events,
        vec![crate::types::PendingSubresourceContinueEvent::Completed { internal_id }]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn streaming_fetch_body_cancel_does_not_abort_live_clone_branch_on_source_error() {
    let load_owner =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("streaming fetch clone-error test loader");
    let mut vm = new_storage_test_vm("https://streaming-fetch-body-cancel.test/");
    let internal_id = 106;
    let body_source_id = crate::network_host::new_network_body_source_id();
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let request_url = Url::parse("https://streaming-fetch-body-cancel.test/data")
        .expect("request URL should parse");
    install_streaming_fetch_response_fixture(
        &mut vm,
        internal_id,
        body_source_id,
        request_url.clone(),
        cancel_handle.clone(),
        None,
        "__streamingFetchResponse",
        load_owner.handle(),
    );

    vm.eval(
        r#"
(() => {
  globalThis.__streamingFetchCloneErrorProbe = [];
  const response = globalThis.__streamingFetchResponse;
  const clone = response.clone();
  response.body.cancel("original done").then(
    () => globalThis.__streamingFetchCloneErrorProbe.push("cancel:resolved"),
    error => globalThis.__streamingFetchCloneErrorProbe.push(`cancel:rejected:${error}`)
  );
  clone.text().then(
    text => globalThis.__streamingFetchCloneErrorProbe.push(`clone:${text}`),
    error => globalThis.__streamingFetchCloneErrorProbe.push(
      `clone-error:${error && error.name}:${error && error.message}`
    )
  );
})()
"#,
    )
    .expect("streaming fetch clone error probe should evaluate");

    assert!(
        !cancel_handle.is_cancelled(),
        "canceling one branch must leave upstream alive for the other branch"
    );
    vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"hello".to_vec());
    vm.finish_streaming_async_subresource_fetch(
        internal_id,
        body_source_id,
        Err("stream broke".to_owned()),
    )
    .expect("streaming fetch source error should error the live clone branch");
    for _ in 0..12 {
        vm.eval("0")
            .expect("streaming fetch clone error promise chain should drain");
    }

    assert!(
        !cancel_handle.is_cancelled(),
        "source error after one branch cancel must not report consumer cancellation upstream"
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__streamingFetchCloneErrorProbe.sort())")
            .expect("streaming fetch clone error probe should evaluate"),
        r#"["cancel:resolved","clone-error:TypeError:stream broke"]"#
    );

    let records: Vec<_> = vm
        ._context_host
        .borrow_mut()
        .take_network_output()
        .into_items()
        .filter_map(|item| match item {
            crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
                Some(*record)
            }
            _ => None,
        })
        .collect();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.url(), &request_url);
    match record.outcome() {
        crate::types::SubresourceNetworkOutcome::Failure { error_text } => {
            assert_eq!(error_text, crate::network_host::ABORTED_ERROR_TEXT);
        }
        other => panic!("expected source error failure record, got {other:?}"),
    }

    let events = vm
        ._context_host
        .borrow_mut()
        .take_pending_subresource_continue_events();
    assert_eq!(
        events,
        vec![crate::types::PendingSubresourceContinueEvent::Completed { internal_id }]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn streaming_fetch_body_cancel_aborts_after_all_clone_branches_cancel() {
    let load_owner =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("streaming fetch all-branch cancellation test loader");
    let mut vm = new_storage_test_vm("https://streaming-fetch-body-cancel.test/");
    let internal_id = 103;
    let body_source_id = crate::network_host::new_network_body_source_id();
    let cancel_handle = moli_fetch::FetchCancelHandle::new();
    let request_url = Url::parse("https://streaming-fetch-body-cancel.test/data")
        .expect("request URL should parse");
    install_streaming_fetch_response_fixture(
        &mut vm,
        internal_id,
        body_source_id,
        request_url.clone(),
        cancel_handle.clone(),
        None,
        "__streamingFetchResponse",
        load_owner.handle(),
    );

    vm.eval(
        r#"
(() => {
  globalThis.__streamingFetchAllBranchCancelProbe = [];
  const response = globalThis.__streamingFetchResponse;
  const clone = response.clone();
  response.body.cancel("original done").then(
    () => globalThis.__streamingFetchAllBranchCancelProbe.push("original:resolved"),
    error => globalThis.__streamingFetchAllBranchCancelProbe.push(`original:rejected:${error}`)
  );
  clone.body.cancel("clone done").then(
    () => globalThis.__streamingFetchAllBranchCancelProbe.push("clone:resolved"),
    error => globalThis.__streamingFetchAllBranchCancelProbe.push(`clone:rejected:${error}`)
  );
})()
"#,
    )
    .expect("streaming fetch all branch cancel probe should evaluate");

    assert!(
        cancel_handle.is_cancelled(),
        "canceling all tee branches should abort the upstream fetch"
    );
    for _ in 0..4 {
        vm.eval("0")
            .expect("streaming fetch all branch cancel reactions should drain");
    }
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__streamingFetchAllBranchCancelProbe.sort())")
            .expect("streaming fetch all branch cancel probe should evaluate"),
        r#"["clone:resolved","original:resolved"]"#
    );
    let events = vm
        ._context_host
        .borrow_mut()
        .take_pending_subresource_continue_events();
    assert_eq!(
        events,
        vec![crate::types::PendingSubresourceContinueEvent::Completed { internal_id }]
    );

    let items: Vec<_> = vm
        ._context_host
        .borrow_mut()
        .take_network_output()
        .into_items()
        .collect();
    assert_eq!(items.len(), 1);
    match &items[0] {
        crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
            assert_eq!(record.url(), &request_url);
            match record.outcome() {
                crate::types::SubresourceNetworkOutcome::Failure { error_text } => {
                    assert_eq!(error_text, crate::network_host::ABORTED_ERROR_TEXT);
                }
                other => panic!("expected all-branch cancel failure record, got {other:?}"),
            }
        }
        other => panic!("expected all-branch cancel failure record, got {other:?}"),
    }

    vm.append_streaming_async_subresource_fetch_chunk(body_source_id, b"late".to_vec());
    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("late streaming finish after all branches cancel should be ignored");
    assert!(
        vm._context_host
            .borrow_mut()
            .take_network_output()
            .into_items()
            .next()
            .is_none(),
        "late streaming finish must not record a response after all branches cancel"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn streaming_xhr_materialization_failure_errors_body_source_before_close() {
    let load_owner =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("streaming XHR materialization test loader");
    let load_client = load_owner.handle();
    let mut vm = new_storage_test_vm("https://xhr-streaming-materialize-error.test/");
    let internal_id = 99;
    let body_source_id = crate::network_host::new_network_body_source_id();
    let request_url = Url::parse("https://xhr-streaming-materialize-error.test/data")
        .expect("request URL should parse");

    let mut body_writer = crate::types::SubresourceResponseBodyWriter::new(1);
    body_writer.append(b"hello");
    let missing_path = std::env::temp_dir().join(format!(
        "moli-missing-streaming-xhr-body-{}-{body_source_id}",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&missing_path);
    let original_spool_path = body_writer
        .replace_spool_path_for_test(missing_path)
        .expect("body writer should have spooled to a temp file");

    let context_ptr: *const v8::Global<v8::Context> = &vm.page_default_context as *const _;
    let context_host = vm._context_host.clone();
    vm.renderer_document_isolate
        .with_entered_renderer_document_isolate(move |isolate| {
            let scope = std::pin::pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = unsafe { v8::Local::new(scope, &*context_ptr) };
            let scope = &mut v8::ContextScope::new(scope, context);

            let body_owner = pending_body_owner_probe(scope);
            let _stream =
                crate::network_host::pending_network_body_stream(scope, body_owner, body_source_id);
            let xhr = streaming_xhr_probe(scope);

            context_host
                .borrow_mut()
                .record_streaming_subresource_fetch(super::StreamingSubresourceFetchState {
                    pending: super::PendingSubresourceFetchState {
                        info: crate::types::PendingSubresourceFetchInfo {
                            internal_id,
                            network_request_handle: None,
                            frame_id: None,
                            document_url: Url::parse(
                                "https://xhr-streaming-materialize-error.test/",
                            )
                            .expect("document URL should parse"),
                            url: request_url.clone(),
                            websocket_socket_id: None,
                            method: "GET".to_owned(),
                            request_headers: Vec::new(),
                            request_body: None,
                            request_body_bytes: None,
                            resource_type: crate::types::SubresourceResourceType::Xhr,
                            request_cookie_report: None,
                        },
                        execution_context:
                            crate::types::PendingSubresourceExecutionContext::adapter(
                                crate::native_bridge::OwnerDispatchScope::Top,
                                v8::Global::new(scope, context),
                            ),
                        credentials_mode: moli_fetch::RequestCredentialsMode::SameOrigin,
                        request_mode: moli_fetch::RequestMode::Cors,
                        network_partition_key: None,
                        policy_context: Default::default(),
                        continuation: super::PendingSubresourceContinuation::Xhr(v8::Global::new(
                            scope, xhr,
                        )),
                        load: crate::network::loads::resource_load_lease_for_test(
                            load_client,
                            None,
                        ),
                        deferred_request_started: false,
                    },
                    request_url: request_url.clone(),
                    request_method: "GET".to_owned(),
                    request_headers: Vec::new(),
                    request_body: None,
                    body_source_id,
                    head: moli_fetch::ResponseHead {
                        final_url: request_url,
                        status: 200,
                        headers: Vec::new(),
                        request_cookie_report: None,
                        cookie_set_reports: Vec::new(),
                        redirected: false,
                        redirect_chain: Vec::new(),
                        from_cache: false,
                        negotiated_http_version: None,
                    },
                    network_request_headers: None,
                    body_writer,
                    event_source_parser: None,
                    xhr_response: None,
                });
            Ok(())
        })
        .expect("streaming XHR fixture should be recorded");

    vm.finish_streaming_async_subresource_fetch(internal_id, body_source_id, Ok(()))
        .expect("streaming XHR finish should surface a network failure, not panic");
    let _ = std::fs::remove_file(original_spool_path);

    let records: Vec<_> = vm
        ._context_host
        .borrow_mut()
        .take_network_output()
        .into_items()
        .filter_map(|item| match item {
            crate::types::ScriptNetworkOutputItem::SubresourceNetworkRecord(record) => {
                Some(*record)
            }
            _ => None,
        })
        .collect();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(
        record.resource_type(),
        crate::types::SubresourceResourceType::Xhr
    );
    match record.outcome() {
        crate::types::SubresourceNetworkOutcome::Failure { error_text } => {
            assert!(
                error_text.contains("failed to materialize streaming XHR body"),
                "unexpected error text: {error_text}"
            );
        }
        other => panic!("expected streaming XHR materialization failure, got {other:?}"),
    }

    let events = vm
        ._context_host
        .borrow_mut()
        .take_pending_subresource_continue_events();
    assert_eq!(
        events,
        vec![crate::types::PendingSubresourceContinueEvent::Completed { internal_id }]
    );
}
