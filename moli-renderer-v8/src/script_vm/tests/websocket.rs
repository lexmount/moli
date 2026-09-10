use super::*;
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(plain)]
struct LoadingWorkerWrapperProbeDeclaration {}

fn loading_worker_wrapper_probe<'s>(scope: &mut v8::PinScope<'s, '_>) -> v8::Local<'s, v8::Object> {
    LoadingWorkerWrapperProbeDeclaration::new()
        .bind(scope)
        .expect("loading worker wrapper probe declaration should bind")
}

#[test]
fn loading_worker_terminate_blocks_late_script_loaded_transition() {
    let mut vm = new_storage_test_vm("https://example.com/");

    let (finished, post_after_finish) = vm
        .with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let wrapper = loading_worker_wrapper_probe(scope);
            let creator_storage_key = host
                .active_storage_context(scope, None)
                .storage_key()
                .clone();
            let top_level_site = creator_storage_key.top_level_site().to_owned();
            let owner = host
                .current_runtime_window_execution_context_binding(scope)
                .expect("test Worker should capture the main execution context");
            let outside_settings_load = host
                .register_dedicated_worker_outside_settings_load(owner.dispatch_scope())
                .expect("test Worker should capture its Document script-load authority");
            let worker_id = host.register_loading_worker(
                scope,
                wrapper,
                top_level_site,
                creator_storage_key,
                host.prepare_dedicated_worker_host(
                    "data:text/html,fixture".parse().unwrap(),
                    "data:text/javascript,fixture".parse().unwrap(),
                    String::new(),
                ),
                moli_fetch::RequestCredentialsMode::SameOrigin,
                None,
                outside_settings_load,
                owner,
            );
            let queued_before_terminate = crate::context_bootstrap::structured_serialize_value(
                scope,
                v8::String::new(scope, "queued-before-terminate")
                    .expect("v8 string allocation")
                    .into(),
            )
            .expect("test worker message should serialize");
            assert!(host.post_worker_message(worker_id, queued_before_terminate));
            assert!(host.terminate_worker(worker_id));
            let finished = host.finish_loading_worker(
                worker_id,
                "https://example.com/worker.js".to_owned(),
                crate::worker::WorkerScriptSource::text(
                    "postMessage('late'); self.close();".to_owned(),
                ),
                Box::new(crate::types::NavigationResponse::from_text_body(
                    "https://example.com/worker.js".parse().unwrap(),
                    200,
                    Vec::new(),
                    "postMessage('late'); self.close();".to_owned(),
                )),
                crate::worker::WorkerScriptKind::Classic,
                true,
                None,
                None,
                Default::default(),
                Vec::new(),
                Vec::new(),
                crate::content_security_policy::ContentSecurityPolicyReportingEndpoints::default(),
            );
            let queued_after_finish = crate::context_bootstrap::structured_serialize_value(
                scope,
                v8::String::new(scope, "queued-after-finish")
                    .expect("v8 string allocation")
                    .into(),
            )
            .expect("test worker message should serialize");
            let post_after_finish = host.post_worker_message(worker_id, queued_after_finish);
            Ok::<_, anyhow::Error>((finished, post_after_finish))
        })
        .expect("loading worker termination probe should succeed");

    assert!(
        !finished,
        "late ScriptLoaded should not transition a terminated loading worker to running"
    );
    assert!(
        !post_after_finish,
        "terminated loading worker should be removed after the late ScriptLoaded path"
    );
}

#[test]
fn websocket_minimal_surface_is_feature_detectable() {
    let mut vm = new_storage_test_vm("https://websocket-surface.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const socket = new WebSocket('wss://example.test/socket');
                return [
                    typeof WebSocket,
                    socket instanceof WebSocket,
                    socket instanceof EventTarget,
                    WebSocket.CONNECTING,
                    WebSocket.OPEN,
                    WebSocket.CLOSING,
                    WebSocket.CLOSED,
                    socket.readyState,
                    socket.url,
                    socket.protocol,
                    socket.extensions,
                    socket.binaryType,
                    typeof socket.addEventListener,
                    typeof socket.send,
                    typeof socket.close
                ].join('|');
            })()
            "#,
        )
        .expect("websocket surface should evaluate");

    assert_eq!(
        result,
        "function|true|true|0|1|2|3|0|wss://example.test/socket|||blob|function|function|function"
    );
}

#[test]
fn websocket_document_csp_blocks_connect_src_and_dispatches_event() {
    let mut vm = new_storage_test_vm("https://websocket-connect-csp.test/");
    vm.set_response_content_security_policies(&[String::from("connect-src 'none'")]);
    vm.set_response_content_security_report_only_policies(&[String::from("connect-src 'none'")]);

    let result = vm
        .eval(
            r#"
(() => {
  const events = [];
  self.addEventListener("securitypolicyviolation", event => {
    events.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition,
      instance: event instanceof SecurityPolicyViolationEvent
    });
  });
  const socket = new WebSocket("wss://websocket-connect-csp.test/socket");
  globalThis.__websocketCspResult = { readyState: socket.readyState, events };
  return socket.readyState;
})()
"#,
        )
        .expect("WebSocket CSP block probe should evaluate");

    assert_eq!(result, "0");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        2
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__websocketCspResult)")
            .expect("queued WebSocket CSP violations should be observable"),
        r#"{"readyState":0,"events":[{"blockedURI":"wss://websocket-connect-csp.test/socket","effectiveDirective":"connect-src","disposition":"report","instance":true},{"blockedURI":"wss://websocket-connect-csp.test/socket","effectiveDirective":"connect-src","disposition":"enforce","instance":true}]}"#
    );
}

#[test]
fn websocket_document_csp_block_precedes_mixed_content_rejection() {
    let mut vm = new_storage_test_vm("http://localhost:8000/trusted-types/reporting.html");
    vm.set_response_content_security_policies(&[String::from("connect-src 'none'")]);

    let result = vm
        .eval(
            r#"
(() => {
  const events = [];
  self.addEventListener("securitypolicyviolation", event => {
    events.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition
    });
  });
  let outcome;
  try {
    const socket = new WebSocket("ws:/common/blank.html");
    outcome = `socket:${socket.readyState}:${socket.url}`;
  } catch (error) {
    outcome = `throw:${error.name}`;
  }
  globalThis.__websocketMixedContentCspResult = { outcome, events };
  return outcome;
})()
"#,
        )
        .expect("WebSocket CSP and mixed-content ordering probe should evaluate");

    assert_eq!(result, "socket:0:ws://common/blank.html");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__websocketMixedContentCspResult)")
            .expect("queued mixed-content WebSocket CSP violation should be observable"),
        r#"{"outcome":"socket:0:ws://common/blank.html","events":[{"blockedURI":"ws://common/blank.html","effectiveDirective":"connect-src","disposition":"enforce"}]}"#
    );
}

#[test]
fn websocket_document_csp_report_only_dispatches_without_blocking() {
    let mut vm = new_storage_test_vm("https://websocket-connect-report-only.test/");
    vm.set_response_content_security_report_only_policies(&[String::from("connect-src 'none'")]);

    let result = vm
        .eval(
            r#"
(() => {
  const events = [];
  self.addEventListener("securitypolicyviolation", event => {
    events.push({
      blockedURI: event.blockedURI,
      effectiveDirective: event.effectiveDirective,
      disposition: event.disposition,
      instance: event instanceof SecurityPolicyViolationEvent
    });
  });
  const socket = new WebSocket("wss://websocket-connect-report-only.test/socket");
  globalThis.__websocketReportOnlyCspResult = { readyState: socket.readyState, events };
  return socket.readyState;
})()
"#,
        )
        .expect("WebSocket CSP report-only probe should evaluate");

    assert_eq!(result, "0");
    assert_eq!(
        drain_pre_domcontentloaded_non_script_page_tasks_for_test(&mut vm),
        1
    );
    assert_eq!(
        vm.eval("JSON.stringify(globalThis.__websocketReportOnlyCspResult)")
            .expect("queued report-only WebSocket CSP violation should be observable"),
        r#"{"readyState":0,"events":[{"blockedURI":"wss://websocket-connect-report-only.test/socket","effectiveDirective":"connect-src","disposition":"report","instance":true}]}"#
    );
}

#[test]
fn websocket_constructor_rejects_invalid_url_and_protocols() {
    let mut vm = new_storage_test_vm("http://127.0.0.1:65535/base/");

    let result = vm
        .eval(
            r#"
            (() => {
                function outcome(callback) {
                    try {
                        callback();
                        return 'ok';
                    } catch (error) {
                        return `${error && error.name}:${String(error && error.message).includes('WebSocket')}`;
                    }
                }
                return [
                    outcome(() => new WebSocket('ftp://example.test/socket')),
                    outcome(() => new WebSocket('ws://example.test/socket#frag')),
                    outcome(() => new WebSocket('ws://example.test/socket', ['chat', 'chat'])),
                    outcome(() => new WebSocket('ws://example.test/socket', ['chat', 'CHAT'])),
                    outcome(() => new WebSocket('ws://example.test/socket', 'bad protocol')),
                    new WebSocket('/relative').url
                ].join('|');
            })()
            "#,
        )
        .expect("websocket validation should evaluate");

    assert_eq!(
        result,
        "SyntaxError:true|SyntaxError:true|SyntaxError:true|SyntaxError:true|SyntaxError:true|ws://127.0.0.1:65535/relative"
    );
}

#[test]
fn websocket_constructor_normalizes_http_urls_and_ignores_extra_arguments() {
    let mut vm = new_storage_test_vm("http://websocket-constructor.test/base/page.html");

    let result = vm
        .eval(
            r#"
            (() => {
                return [
                    new WebSocket('http://example.test/socket', 'chat', 'ignored').url,
                    new WebSocket('https://example.test/socket').url,
                    new WebSocket('/relative', undefined, 'ignored').url
                ].join('|');
            })()
            "#,
        )
        .expect("websocket http/https URL normalization should evaluate");

    assert_eq!(
        result,
        "ws://example.test/socket|wss://example.test/socket|ws://websocket-constructor.test/relative"
    );
}

#[test]
fn websocket_constructor_requires_new_and_url_argument() {
    let mut vm = new_storage_test_vm("https://websocket-constructor.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                function outcome(callback) {
                    try {
                        callback();
                        return 'ok';
                    } catch (error) {
                        return `${error && error.name}:${String(error && error.message).includes('WebSocket')}`;
                    }
                }
                return [
                    outcome(() => WebSocket('wss://example.test/socket')),
                    outcome(() => new WebSocket())
                ].join('|');
            })()
            "#,
        )
        .expect("websocket constructor argument validation should evaluate");

    assert_eq!(result, "TypeError:true|TypeError:true");
}

// Chromium/WPT target tests below are intentionally ignored. They encode the
// WebSocket compatibility surface we still need to converge on, using local
// snippets derived from Chromium's `external/wpt/websockets` coverage.

#[test]
fn websocket_wpt_target_idl_attributes_are_prototype_accessors() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const socket = new WebSocket('wss://example.test/socket');
            const ownInternalSlots = Object.getOwnPropertyNames(socket)
                .filter(name => name.startsWith('__moliWebSocket'));
            if (ownInternalSlots.length !== 0) {
                throw new Error(`WebSocket internal slots should not be reflected: ${ownInternalSlots.join(',')}`);
            }
            Object.assign(WebSocket.prototype, {
                __moliWebSocketUrl: 'wss://poison.example/socket',
                __moliWebSocketReadyState: 99,
                __moliWebSocketBufferedAmount: 99,
                __moliWebSocketExtensions: 'poison-ext',
                __moliWebSocketProtocol: 'poison-proto',
                __moliWebSocketBinaryType: 'arraybuffer'
            });
            Object.assign(socket, {
                __moliWebSocketUrl: 'wss://own-poison.example/socket',
                __moliWebSocketReadyState: 88,
                __moliWebSocketBufferedAmount: 88,
                __moliWebSocketExtensions: 'own-poison-ext',
                __moliWebSocketProtocol: 'own-poison-proto',
                __moliWebSocketBinaryType: 'arraybuffer'
            });
            if (
                socket.url !== 'wss://example.test/socket' ||
                socket.readyState !== 0 ||
                socket.bufferedAmount !== 0 ||
                socket.extensions !== '' ||
                socket.protocol !== '' ||
                socket.binaryType !== 'blob'
            ) {
                throw new Error('WebSocket accessors should ignore string-named internal slot spoofing');
            }
            const readonly = ['url', 'readyState', 'bufferedAmount', 'extensions', 'protocol'];
            for (const name of readonly) {
                if (Object.prototype.hasOwnProperty.call(socket, name)) {
                    throw new Error(`${name} should not be an own data property`);
                }
                const descriptor = Object.getOwnPropertyDescriptor(WebSocket.prototype, name);
                if (!descriptor || typeof descriptor.get !== 'function' || descriptor.set !== undefined) {
                    throw new Error(`${name} should be a readonly prototype accessor`);
                }
                if (descriptor.get.name !== `get ${name}` || descriptor.get.length !== 0) {
                    throw new Error(`${name} getter metadata should match Web IDL`);
                }
                if (descriptor.enumerable !== true || descriptor.configurable !== true) {
                    throw new Error(`${name} descriptor shape should match Web IDL`);
                }
            }
            const binaryType = Object.getOwnPropertyDescriptor(WebSocket.prototype, 'binaryType');
            if (!binaryType || typeof binaryType.get !== 'function' || typeof binaryType.set !== 'function') {
                throw new Error('binaryType should be a prototype accessor pair');
            }
            if (binaryType.get.name !== 'get binaryType' || binaryType.get.length !== 0 ||
                binaryType.set.name !== 'set binaryType' || binaryType.set.length !== 1 ||
                binaryType.enumerable !== true || binaryType.configurable !== true) {
                throw new Error('binaryType descriptor metadata should match Web IDL');
            }
            socket.bufferedAmount = 5;
            if (socket.bufferedAmount !== 0) {
                throw new Error('bufferedAmount should be readonly');
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket IDL descriptor test should pass once implemented");
}

#[test]
fn websocket_wpt_target_event_handler_attributes_treat_non_callable_as_null() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const socket = new WebSocket('wss://example.test/socket');
            for (const key of ['onopen', 'onmessage', 'onerror', 'onclose']) {
                const descriptor = Object.getOwnPropertyDescriptor(WebSocket.prototype, key);
                if (!descriptor || typeof descriptor.get !== 'function' || typeof descriptor.set !== 'function') {
                    throw new Error(`${key} should be a prototype accessor pair`);
                }
                if (descriptor.get.name !== `get ${key}` || descriptor.get.length !== 0 ||
                    descriptor.set.name !== `set ${key}` || descriptor.set.length !== 1) {
                    throw new Error(`${key} getter/setter metadata should match Web IDL`);
                }
                if (descriptor.enumerable !== true || descriptor.configurable !== true) {
                    throw new Error(`${key} descriptor shape should match Web IDL`);
                }
                if (Object.prototype.hasOwnProperty.call(socket, key)) {
                    throw new Error(`${key} should not be installed per instance`);
                }
                if (socket[key] !== null) {
                    throw new Error(`${key} should start as null`);
                }
                socket[key] = function () {};
                if (typeof socket[key] !== 'function') {
                    throw new Error(`${key} should accept callable values`);
                }
                if (Object.prototype.hasOwnProperty.call(socket, key)) {
                    throw new Error(`${key} should continue to resolve through the prototype accessor`);
                }
                socket[key] = 2;
                if (socket[key] !== null) {
                    throw new Error(`${key} should coerce non-callable values to null`);
                }
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket event handler attribute test should pass once implemented");
}

#[test]
fn websocket_wpt_target_constants_have_webidl_descriptors() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            for (const [name, value] of [['CONNECTING', 0], ['OPEN', 1], ['CLOSING', 2], ['CLOSED', 3]]) {
                for (const owner of [WebSocket, WebSocket.prototype]) {
                    const descriptor = Object.getOwnPropertyDescriptor(owner, name);
                    if (!descriptor) {
                        throw new Error(`${name} missing on ${owner === WebSocket ? 'constructor' : 'prototype'}`);
                    }
                    if (descriptor.value !== value || descriptor.writable !== false ||
                        descriptor.enumerable !== true || descriptor.configurable !== false) {
                        throw new Error(`${name} descriptor does not match Web IDL constants`);
                    }
                }
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket constant descriptor test should pass once implemented");
}

#[test]
fn websocket_wpt_target_event_target_listener_options_and_objects() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const socket = new WebSocket('wss://example.test/socket');
            const simpleOwn = Object.getOwnPropertyNames(socket)
                .filter(name =>
                    name === '__moliEventTargetSlot' ||
                    name === '__moliSimpleEventTargetOrderedHandlers'
                );
            if (simpleOwn.length !== 0) {
                throw new Error(`WebSocket simple EventTarget internals should not be reflected: ${simpleOwn.join(',')}`);
            }
            const expected = {
                addEventListener: 'true:true:true:true:function:0:addEventListener',
                removeEventListener: 'true:true:true:true:function:0:removeEventListener',
                dispatchEvent: 'true:true:true:true:function:0:dispatchEvent'
            };
            for (const [name, shape] of Object.entries(expected)) {
                const descriptor = Object.getOwnPropertyDescriptor(socket, name);
                const actual = [
                    !!descriptor,
                    descriptor && descriptor.enumerable,
                    descriptor && descriptor.configurable,
                    descriptor && descriptor.writable,
                    descriptor && typeof descriptor.value,
                    descriptor && descriptor.value.length,
                    descriptor && descriptor.value.name
                ].join(':');
                if (actual !== shape) {
                    throw new Error(`${name} descriptor mismatch: ${actual}`);
                }
            }
            socket.__moliEventTargetSlot = '__wrongSlot';
            socket.__moliSimpleEventTargetOrderedHandlers = false;
            const events = [];
            socket.addEventListener('open', { handleEvent(event) { events.push(`object:${event.type}:${this === socket}`); } });
            socket.addEventListener('open', function once(event) {
                events.push(`once:${event.target === socket}:${event.currentTarget === socket}`);
            }, { once: true });
            socket.dispatchEvent(new Event('open'));
            socket.dispatchEvent(new Event('open'));
            if (events.join('|') !== 'object:open:false|once:true:true|object:open:false') {
                throw new Error(events.join('|'));
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket EventTarget listener options test should pass once implemented");
}

#[test]
fn websocket_wpt_target_event_handler_synthetic_ui_event_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const socket = new WebSocket('wss://example.test/socket');
                const events = [];
                for (const type of ['open', 'message', 'error', 'close']) {
                    const key = `on${type}`;
                    if (!(key in socket)) {
                        throw new Error(`${key} should be present on WebSocket`);
                    }
                    socket[key] = event => events.push(`${type}:${event.detail}:${event.target === socket}:${event.currentTarget === socket}`);
                    const event = document.createEvent('UIEvents');
                    event.initUIEvent(type, false, false, window, 5);
                    socket.dispatchEvent(event);
                }
                return events.join('|');
            })()
            "#,
        )
        .expect("target WebSocket synthetic UIEvent surface test should evaluate");

    assert_eq!(
        result,
        "open:5:true:true|message:5:true:true|error:5:true:true|close:5:true:true"
    );
}

#[test]
fn websocket_wpt_target_event_handler_assignment_matrix() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const socket = new WebSocket('wss://example.test/socket');
            for (const key of ['onopen', 'onmessage', 'onerror', 'onclose']) {
                const callback = function () {};
                socket[key] = callback;
                if (socket[key] !== callback) {
                    throw new Error(`${key} should retain callable values`);
                }
                for (const value of [1, ';', null, undefined]) {
                    socket[key] = callback;
                    socket[key] = value;
                    if (socket[key] !== null) {
                        throw new Error(`${key} should clear ${String(value)} to null`);
                    }
                }
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket event handler assignment matrix should pass once implemented");
}

#[test]
fn websocket_wpt_target_remove_event_listener_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const socket = new WebSocket('wss://example.test/socket');
            for (const type of ['open', 'message', 'error', 'close']) {
                let count = 0;
                function listener() { count++; }
                socket.addEventListener(type, listener);
                socket.removeEventListener(type, listener);
                socket.dispatchEvent(new Event(type));
                if (count !== 0) {
                    throw new Error(`${type} listener should have been removed`);
                }
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket removeEventListener test should pass once implemented");
}

#[test]
fn websocket_wpt_target_domexception_names_for_send_and_close_validation() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            function expectName(expected, callback) {
                try {
                    callback();
                } catch (error) {
                    if (error && error.name === expected) {
                        return;
                    }
                    throw new Error(`expected ${expected}, got ${error && error.name}`);
                }
                throw new Error(`expected ${expected}, got no exception`);
            }
            function outcome(callback) {
                try {
                    return String(callback());
                } catch (error) {
                    return `throw:${error && error.name}`;
                }
            }
            expectName('InvalidStateError', () => new WebSocket('wss://example.test/a').send('x'));
            for (const value of [0, 500, 1004, 1005, 1006, 5000, NaN, 'string', null, 0x10000 + 1000]) {
                expectName('InvalidAccessError', () => new WebSocket('wss://example.test/b').close(value));
            }
            expectName('InvalidAccessError', () => new WebSocket('wss://example.test/c').close('reason only'));
            if (outcome(() => new WebSocket('wss://example.test/d').close(1000.5, undefined)) !== 'undefined') {
                throw new Error('[Clamp] half-even rounding should keep 1000.5 valid as 1000');
            }
            if (outcome(() => new WebSocket('wss://example.test/e').close(2999.5, 'rounded')) !== 'undefined') {
                throw new Error('[Clamp] half-even rounding should make 2999.5 valid as 3000');
            }
            expectName('InvalidAccessError', () => new WebSocket('wss://example.test/f').close(1001.5));
            expectName('TypeError', () => new WebSocket('wss://example.test/g').close(1000, Symbol()));
            expectName('SyntaxError', () => new WebSocket('wss://example.test/h').close(1000, 'x'.repeat(124)));
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket DOMException validation test should pass once implemented");
}

#[test]
fn websocket_wpt_target_send_connecting_invalid_state_precedes_text_conversion() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            for (const value of ['a', 'a\uDC00x', 'a\uD800x', 'a\uDC00\uD800x']) {
                const socket = new WebSocket('wss://example.test/socket');
                try {
                    socket.send(value);
                } catch (error) {
                    if (error && error.name === 'InvalidStateError') {
                        continue;
                    }
                    throw new Error(`expected InvalidStateError, got ${error && error.name}`);
                }
                throw new Error('expected send while CONNECTING to throw');
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket CONNECTING send validation test should pass once implemented");
}

#[test]
fn websocket_wpt_target_constructor_syntaxerror_names_for_url_and_protocol_validation() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            function expectSyntaxError(callback) {
                try {
                    callback();
                } catch (error) {
                    if (error && error.name === 'SyntaxError') {
                        return;
                    }
                    throw new Error(`expected SyntaxError, got ${error && error.name}`);
                }
                throw new Error('expected SyntaxError, got no exception');
            }
            expectSyntaxError(() => new WebSocket('ws://web platform.test/socket'));
            expectSyntaxError(() => new WebSocket('wss://example.test/socket#fragment'));
            expectSyntaxError(() => new WebSocket('wss://example.test/socket', 'bad protocol'));
            expectSyntaxError(() => new WebSocket('wss://example.test/socket', ['chat', 'CHAT']));
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket constructor SyntaxError test should pass once implemented");
}

#[test]
fn websocket_wpt_target_constructor_ignores_extra_arguments() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const socket = new WebSocket('wss://example.test/socket', 'echo', 'stray');
                return [
                    socket instanceof WebSocket,
                    socket.protocol,
                    socket.url
                ].join('|');
            })()
            "#,
        )
        .expect("target WebSocket extra constructor argument test should evaluate");

    assert_eq!(result, "true||wss://example.test/socket");
}

#[test]
fn websocket_wpt_target_protocol_token_validation_matches_wpt() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            function expectSyntaxError(protocols) {
                try {
                    new WebSocket('wss://example.test/socket', protocols);
                } catch (error) {
                    if (error && error.name === 'SyntaxError') {
                        return;
                    }
                    throw new Error(`expected SyntaxError, got ${error && error.name}`);
                }
                throw new Error(`expected SyntaxError for ${JSON.stringify(protocols)}`);
            }
            for (const protocol of ['', 'bad protocol', 'bad,protocol', 'bad/protocol',
                                    'bad;protocol', 'bad=protocol', '\u007F', '\u0080echo']) {
                expectSyntaxError(protocol);
            }
            expectSyntaxError(['echo', 'ECHO']);
            const socket = new WebSocket('wss://example.test/socket', [
                'echo',
                "!#$%&'*+-.^_`|~0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
            ]);
            if (socket.protocol !== '') {
                throw new Error('protocol should be empty before the handshake selects one');
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket protocol validation test should pass once implemented");
}

#[test]
fn websocket_wpt_target_protocol_iterable_conversion_order() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const steps = [];
                const protocols = {
                    [Symbol.iterator]() {
                        steps.push('iterator');
                        let index = 0;
                        return {
                            next() {
                                index++;
                                if (index === 1) {
                                    return {
                                        done: false,
                                        value: { toString() { steps.push('first'); return 'chat'; } }
                                    };
                                }
                                if (index === 2) {
                                    return {
                                        done: false,
                                        value: { toString() { steps.push('second'); return 'superchat'; } }
                                    };
                                }
                                return { done: true };
                            }
                        };
                    }
                };
                const socket = new WebSocket('wss://example.test/socket', protocols);
                return `${steps.join('|')}|${socket.protocol}`;
            })()
            "#,
        )
        .expect("target WebSocket protocol iterable conversion test should evaluate");

    assert_eq!(result, "iterator|first|second|");
}

#[test]
fn websocket_wpt_target_insecure_websocket_is_blocked_from_secure_context() {
    let mut vm = new_storage_test_vm("https://secure-websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            try {
                new WebSocket('ws://insecure.example.test/socket');
            } catch (error) {
                if (error && error.name === 'SecurityError') {
                    return 'ok';
                }
                throw new Error(`expected SecurityError, got ${error && error.name}`);
            }
            throw new Error('expected insecure WebSocket construction to throw');
        })()
        "#,
    )
    .expect("target WebSocket mixed-content construction test should pass once implemented");
}

#[test]
fn websocket_wpt_target_loopback_websocket_is_allowed_from_secure_context() {
    let mut vm = new_storage_test_vm("https://secure-websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const socket = new WebSocket('ws://localhost/socket');
                return socket.url;
            })()
            "#,
        )
        .expect("target WebSocket loopback mixed-content exemption test should evaluate");

    assert_eq!(result, "ws://localhost/socket");
}

#[test]
fn websocket_wpt_target_insecure_websocket_is_blocked_from_loopback_secure_context() {
    let mut vm = new_storage_test_vm("http://localhost/");

    vm.eval(
        r#"
        (() => {
            try {
                new WebSocket('ws://insecure.example.test/socket');
            } catch (error) {
                if (error && error.name === 'SecurityError') {
                    return 'ok';
                }
                throw new Error(`expected SecurityError, got ${error && error.name}`);
            }
            throw new Error('expected insecure WebSocket construction to throw');
        })()
        "#,
    )
    .expect("target WebSocket loopback secure-context construction test should pass");
}

#[test]
fn websocket_wpt_target_url_serialization_percent_encodes_spaces() {
    let mut vm = new_storage_test_vm("http://127.0.0.1:65535/base/");

    let result = vm
        .eval(
            r#"
            (() => {
                const socket = new WebSocket('/echo?foo%20bar baz');
                return socket.url;
            })()
            "#,
        )
        .expect("target WebSocket URL serialization test should evaluate");

    assert_eq!(result, "ws://127.0.0.1:65535/echo?foo%20bar%20baz");
}

#[test]
fn websocket_wpt_target_url_bare_authority_and_query_serialization() {
    let mut vm = new_storage_test_vm("http://websocket-wpt-target.test/base/");

    let result = vm
        .eval(
            r#"
            (() => {
                return [
                    new WebSocket('wss://example.test').url,
                    new WebSocket('wss://example.test?foo%20bar baz').url,
                    new WebSocket('wss://example.test:443').url,
                    new WebSocket('ws://example.test:80').url
                ].join('|');
            })()
            "#,
        )
        .expect("target WebSocket bare-authority URL serialization test should evaluate");

    assert_eq!(
        result,
        "wss://example.test/|wss://example.test/?foo%20bar%20baz|wss://example.test/|ws://example.test/"
    );
}

#[test]
fn websocket_wpt_target_constructor_rejects_url_with_space() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            try {
                new WebSocket('wss://web platform.test/socket');
            } catch (error) {
                if (error && error.name === 'SyntaxError') {
                    return 'ok';
                }
                throw new Error(`expected SyntaxError, got ${error && error.name}`);
            }
            throw new Error('expected URL with space to throw');
        })()
        "#,
    )
    .expect("target WebSocket URL space test should pass once implemented");
}

#[test]
fn websocket_wpt_target_relative_url_conversion_for_primitive_inputs() {
    let mut vm = new_storage_test_vm("http://127.0.0.1:65535/base/path?query");

    let result = vm
        .eval(
            r#"
            (() => {
                return [
                    new WebSocket('test').url,
                    new WebSocket('?').url,
                    new WebSocket(null).url,
                    new WebSocket(123).url
                ].join('|');
            })()
            "#,
        )
        .expect("target WebSocket primitive URL conversion test should evaluate");

    assert_eq!(
        result,
        "ws://127.0.0.1:65535/base/test|ws://127.0.0.1:65535/base/path?|ws://127.0.0.1:65535/base/null|ws://127.0.0.1:65535/base/123"
    );
}

#[test]
fn websocket_wpt_target_close_event_constructor_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const first = new CloseEvent('foo');
                const second = new CloseEvent('bar', {
                    bubbles: true,
                    cancelable: true,
                    wasClean: true,
                    code: 7,
                    reason: 'x'
                });
                return [
                    first instanceof CloseEvent,
                    first.type,
                    first.bubbles,
                    first.cancelable,
                    first.wasClean,
                    first.code,
                    first.reason,
                    second instanceof CloseEvent,
                    second.type,
                    second.bubbles,
                    second.cancelable,
                    second.wasClean,
                    second.code,
                    second.reason,
                    'initCloseEvent' in CloseEvent.prototype,
                    'initCloseEvent' in second
                ].join('|');
            })()
            "#,
        )
        .expect("target CloseEvent constructor test should evaluate");

    assert_eq!(
        result,
        "true|foo|false|false|false|0||true|bar|true|true|true|7|x|false|false"
    );
}

#[test]
fn websocket_wpt_target_message_event_constructor_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const first = new MessageEvent('message');
                const second = new MessageEvent('message', {
                    bubbles: true,
                    cancelable: true,
                    data: 'payload',
                    origin: 'ws://example.test',
                    lastEventId: 'event-id'
                });
                return [
                    first instanceof MessageEvent,
                    first.type,
                    first.bubbles,
                    first.cancelable,
                    first.data === null,
                    first.origin,
                    first.lastEventId,
                    second instanceof MessageEvent,
                    second.bubbles,
                    second.cancelable,
                    second.data,
                    second.origin,
                    second.lastEventId,
                    'ports' in second,
                    'source' in second,
                    new MessageEvent('message', { data: undefined }).data === undefined
                ].join('|');
            })()
            "#,
        )
        .expect("target MessageEvent constructor test should evaluate");

    assert_eq!(
        result,
        "true|message|false|false|true|||true|true|true|payload|ws://example.test|event-id|true|true|true"
    );
}

#[test]
fn websocket_wpt_target_constructor_identity_and_string_tags() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const socket = new WebSocket('wss://example.test/socket');
                return [
                    WebSocket.length,
                    WebSocket.name,
                    WebSocket.prototype.constructor === WebSocket,
                    Object.prototype.toString.call(socket),
                    String(socket),
                    Object.prototype.toString.call(WebSocket.prototype)
                ].join('|');
            })()
            "#,
        )
        .expect("target WebSocket constructor identity test should evaluate");

    assert_eq!(
        result,
        "1|WebSocket|true|[object WebSocket]|[object WebSocket]|[object WebSocketPrototype]"
    );
}

#[test]
fn websocket_wpt_target_global_constructor_and_prototype_descriptors() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, 'WebSocket');
            if (!globalDescriptor || globalDescriptor.value !== WebSocket ||
                globalDescriptor.writable !== true ||
                globalDescriptor.enumerable !== false ||
                globalDescriptor.configurable !== true) {
                throw new Error('global WebSocket descriptor should match Web IDL');
            }
            const prototypeDescriptor = Object.getOwnPropertyDescriptor(WebSocket, 'prototype');
            if (!prototypeDescriptor || prototypeDescriptor.value !== WebSocket.prototype ||
                prototypeDescriptor.writable !== false ||
                prototypeDescriptor.enumerable !== false ||
                prototypeDescriptor.configurable !== false) {
                throw new Error('WebSocket.prototype descriptor should match Web IDL');
            }
            const constructorDescriptor = Object.getOwnPropertyDescriptor(WebSocket.prototype, 'constructor');
            if (!constructorDescriptor || constructorDescriptor.value !== WebSocket ||
                constructorDescriptor.writable !== true ||
                constructorDescriptor.enumerable !== false ||
                constructorDescriptor.configurable !== true) {
                throw new Error('WebSocket.prototype.constructor descriptor should match Web IDL');
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket constructor descriptor test should pass once implemented");
}

#[test]
fn websocket_wpt_target_close_event_reason_usvstring_conversion() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const event = new CloseEvent('close', { reason: '\uD800' });
                return [
                    event.reason.length,
                    event.reason.charCodeAt(0).toString(16)
                ].join('|');
            })()
            "#,
        )
        .expect("target CloseEvent USVString conversion test should evaluate");

    assert_eq!(result, "1|fffd");
}

#[test]
fn websocket_wpt_target_close_event_attribute_descriptors() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            for (const name of ['wasClean', 'code', 'reason']) {
                const descriptor = Object.getOwnPropertyDescriptor(CloseEvent.prototype, name);
                if (!descriptor || typeof descriptor.get !== 'function' || descriptor.set !== undefined) {
                    throw new Error(`${name} should be a readonly prototype accessor`);
                }
                if (descriptor.enumerable !== true || descriptor.configurable !== true) {
                    throw new Error(`${name} descriptor should match Web IDL`);
                }
            }
            const event = new CloseEvent('close', { wasClean: true, code: 1000, reason: 'ok' });
            event.code = 3000;
            event.reason = 'changed';
            if (event.code !== 1000 || event.reason !== 'ok') {
                throw new Error('CloseEvent readonly attributes should ignore assignment');
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target CloseEvent attribute descriptor test should pass once implemented");
}

#[test]
fn websocket_wpt_target_interface_property_mutation_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const socket = new WebSocket('wss://example.test/socket');
            if (delete socket.url !== true || socket.url !== 'wss://example.test/socket') {
                throw new Error('readonly url accessor should survive instance delete');
            }
            if (delete socket.bufferedAmount !== true || socket.bufferedAmount !== 0) {
                throw new Error('readonly bufferedAmount accessor should survive instance delete');
            }
            Object.defineProperty(socket, 'readyState', { value: 99 });
            if (socket.readyState !== 99 || !Object.prototype.hasOwnProperty.call(socket, 'readyState')) {
                throw new Error('readyState own data property should shadow the prototype accessor like Chromium');
            }
            Object.defineProperty(socket, 'binaryType', { value: 'arraybuffer' });
            if (socket.binaryType !== 'arraybuffer' || !Object.prototype.hasOwnProperty.call(socket, 'binaryType')) {
                throw new Error('binaryType own data property should shadow the prototype accessor like Chromium');
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket interface mutation test should pass once implemented");
}

#[test]
fn websocket_wpt_target_buffered_amount_prototype_mutation_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const socket = new WebSocket('wss://example.test/socket');
            if (socket.bufferedAmount !== 0) {
                throw new Error('bufferedAmount should start at 0');
            }
            socket.bufferedAmount = 5;
            if (socket.bufferedAmount !== 0) {
                throw new Error('bufferedAmount should be readonly through the default accessor');
            }
            delete socket.bufferedAmount;
            if (socket.bufferedAmount !== 0) {
                throw new Error('deleting instance bufferedAmount should preserve prototype getter');
            }
            Object.defineProperty(WebSocket.prototype, 'bufferedAmount', {
                configurable: true,
                get() { return 'getter-ran'; }
            });
            if (socket.bufferedAmount !== 'getter-ran') {
                throw new Error('bufferedAmount getter override should run');
            }
            Object.defineProperty(WebSocket.prototype, 'bufferedAmount', {
                configurable: true,
                set(value) { globalThis[value] = true; }
            });
            socket.bufferedAmount = 'setter_ran';
            if (globalThis.setter_ran !== true) {
                throw new Error('bufferedAmount setter override should run');
            }
            delete WebSocket.prototype.bufferedAmount;
            if (new WebSocket('wss://example.test/socket').bufferedAmount !== undefined) {
                throw new Error('deleted prototype bufferedAmount should expose undefined');
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket bufferedAmount prototype mutation test should pass once implemented");
}

#[test]
fn websocket_wpt_target_url_accessor_mutation_and_uppercase_absence() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/base/page.html");

    vm.eval(
        r#"
        (() => {
            const socket = new WebSocket('/echo');
            if (socket.url !== 'wss://websocket-wpt-target.test/echo') {
                throw new Error(`unexpected url ${socket.url}`);
            }
            socket.url = 'wss://example.test/other';
            if (socket.url !== 'wss://websocket-wpt-target.test/echo') {
                throw new Error('url should be readonly through the default accessor');
            }
            if (socket.URL !== undefined || ('URL' in socket) ||
                WebSocket.prototype.URL !== undefined || ('URL' in WebSocket.prototype)) {
                throw new Error('legacy uppercase URL should not be present');
            }
            delete socket.url;
            if (socket.url !== 'wss://websocket-wpt-target.test/echo') {
                throw new Error('deleting instance url should preserve prototype getter');
            }
            Object.defineProperty(WebSocket.prototype, 'url', {
                configurable: true,
                get() { return 'getter-ran'; }
            });
            if (socket.url !== 'getter-ran') {
                throw new Error('url getter override should run');
            }
            Object.defineProperty(WebSocket.prototype, 'url', {
                configurable: true,
                set(value) { globalThis[value] = true; }
            });
            socket.url = 'setter_ran';
            if (globalThis.setter_ran !== true) {
                throw new Error('url setter override should run');
            }
            delete WebSocket.prototype.url;
            if (new WebSocket('/echo').url !== undefined) {
                throw new Error('deleted prototype url should expose undefined');
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket url accessor mutation test should pass once implemented");
}

#[test]
fn websocket_wpt_target_interface_prototype_accessor_override_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            Object.defineProperty(WebSocket.prototype, 'readyState', {
                configurable: true,
                get() { return 'ready-override'; },
                set(value) { globalThis.__readySetterValue = value; }
            });
            const first = new WebSocket('wss://example.test/socket');
            if (first.readyState !== 'ready-override') {
                throw new Error('readyState getter override should be observed');
            }
            first.readyState = 'setter-ran';
            if (globalThis.__readySetterValue !== 'setter-ran') {
                throw new Error('readyState setter override should run');
            }
            delete WebSocket.prototype.readyState;
            const second = new WebSocket('wss://example.test/socket');
            if (second.readyState !== undefined) {
                throw new Error('deleted readyState prototype accessor should expose undefined');
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket prototype accessor override test should pass once implemented");
}

#[test]
fn websocket_wpt_target_readonly_accessor_override_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            for (const [name, value] of [
                ['url', 'url-override'],
                ['bufferedAmount', 'buffered-override'],
                ['extensions', 'extensions-override'],
                ['protocol', 'protocol-override']
            ]) {
                Object.defineProperty(WebSocket.prototype, name, {
                    configurable: true,
                    get() { return value; }
                });
                const socket = new WebSocket('wss://example.test/socket');
                if (socket[name] !== value) {
                    throw new Error(`${name} prototype getter override should be observed`);
                }
                delete WebSocket.prototype[name];
                if (new WebSocket('wss://example.test/socket')[name] !== undefined) {
                    throw new Error(`${name} delete should remove prototype accessor`);
                }
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket readonly accessor override test should pass once implemented");
}

#[test]
fn websocket_wpt_target_protocol_extensions_accessor_mutation_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            for (const [name, value] of [
                ['protocol', 'protocol-override'],
                ['extensions', 'extensions-override']
            ]) {
                const socket = new WebSocket('wss://example.test/socket');
                if (socket[name] !== '') {
                    throw new Error(`${name} should initially be empty`);
                }
                socket[name] = 'attempted-write';
                if (socket[name] !== '') {
                    throw new Error(`${name} should be readonly through the default accessor`);
                }
                delete socket[name];
                if (socket[name] !== '') {
                    throw new Error(`deleting instance ${name} should preserve prototype getter`);
                }
                Object.defineProperty(WebSocket.prototype, name, {
                    configurable: true,
                    get() { return value; }
                });
                if (socket[name] !== value) {
                    throw new Error(`${name} getter override should run`);
                }
                Object.defineProperty(WebSocket.prototype, name, {
                    configurable: true,
                    set(input) { globalThis[`${name}_${input}`] = true; }
                });
                socket[name] = 'setter';
                if (globalThis[`${name}_setter`] !== true) {
                    throw new Error(`${name} setter override should run`);
                }
                delete WebSocket.prototype[name];
                if (new WebSocket('wss://example.test/socket')[name] !== undefined) {
                    throw new Error(`deleted prototype ${name} should expose undefined`);
                }
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket protocol/extensions mutation test should pass once implemented");
}

#[test]
fn websocket_wpt_target_url_and_protocol_initial_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/base/page.html");

    let result = vm
        .eval(
            r#"
            (() => {
                const absolute = new WebSocket('wss://example.test/socket');
                const relative = new WebSocket('/relative');
                const requestedProtocol = new WebSocket('wss://example.test/socket', 'chat');
                return [
                    absolute.url,
                    relative.url,
                    absolute.protocol,
                    requestedProtocol.protocol
                ].join('|');
            })()
            "#,
        )
        .expect("target WebSocket url/protocol initial surface test should evaluate");

    assert_eq!(
        result,
        "wss://example.test/socket|wss://websocket-wpt-target.test/relative||"
    );
}

#[test]
fn websocket_wpt_target_binary_type_accessor_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const descriptor = Object.getOwnPropertyDescriptor(WebSocket.prototype, 'binaryType');
            if (!descriptor || typeof descriptor.get !== 'function' || typeof descriptor.set !== 'function') {
                throw new Error('binaryType should be a prototype accessor pair');
            }
            if (descriptor.enumerable !== true || descriptor.configurable !== true) {
                throw new Error('binaryType descriptor shape should match Web IDL');
            }
            const socket = new WebSocket('wss://example.test/socket');
            if (socket.binaryType !== 'blob') {
                throw new Error('binaryType should initially be blob');
            }
            socket.binaryType = 'arraybuffer';
            if (socket.binaryType !== 'arraybuffer') {
                throw new Error('binaryType should accept arraybuffer');
            }
            let invalidDidThrow = (() => {
                try {
                    socket.binaryType = 'notBlobOrArrayBuffer';
                    return false;
                } catch (error) {
                    if (error?.name !== 'SyntaxError') {
                        throw error;
                    }
                    return true;
                }
            })();
            if (!invalidDidThrow) {
                throw new Error('invalid binaryType assignments should throw SyntaxError');
            }
            if (socket.binaryType !== 'arraybuffer') {
                throw new Error('invalid binaryType assignments should preserve the previous value');
            }
            Object.defineProperty(WebSocket.prototype, 'binaryType', {
                configurable: true,
                get() { return 'binary-override'; },
                set(value) { globalThis.__binaryTypeSetter = value; }
            });
            const overridden = new WebSocket('wss://example.test/socket');
            if (overridden.binaryType !== 'binary-override') {
                throw new Error('binaryType prototype getter override should be observed');
            }
            overridden.binaryType = 'setter-ran';
            if (globalThis.__binaryTypeSetter !== 'setter-ran') {
                throw new Error('binaryType prototype setter override should run');
            }
            delete WebSocket.prototype.binaryType;
            if (new WebSocket('wss://example.test/socket').binaryType !== undefined) {
                throw new Error('deleted binaryType prototype accessor should expose undefined');
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket binaryType accessor surface test should pass once implemented");
}

#[test]
fn websocket_wpt_target_constants_readonly_nonconfigurable_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const socket = new WebSocket('wss://example.test/socket');
            const constants = ['CONNECTING', 'OPEN', 'CLOSING', 'CLOSED'];
            for (let i = 0; i < constants.length; i++) {
                const name = constants[i];
                if (WebSocket[name] !== i) {
                    throw new Error(`WebSocket.${name} should be ${i}`);
                }
                if (WebSocket.prototype[name] !== i) {
                    throw new Error(`WebSocket.prototype.${name} should be ${i}`);
                }
                if (socket[name] !== i) {
                    throw new Error(`socket.${name} should inherit ${i}`);
                }
                const ctorDescriptor = Object.getOwnPropertyDescriptor(WebSocket, name);
                const protoDescriptor = Object.getOwnPropertyDescriptor(WebSocket.prototype, name);
                for (const descriptor of [ctorDescriptor, protoDescriptor]) {
                    if (!descriptor || descriptor.value !== i || descriptor.writable !== false ||
                        descriptor.enumerable !== true || descriptor.configurable !== false) {
                        throw new Error(`${name} descriptor should be readonly and non-configurable`);
                    }
                }
                WebSocket[name] = 5;
                WebSocket.prototype[name] = 5;
                socket[name] = 5;
                delete WebSocket[name];
                delete WebSocket.prototype[name];
                delete socket[name];
                if (WebSocket[name] !== i || WebSocket.prototype[name] !== i || socket[name] !== i) {
                    throw new Error(`${name} should survive assignment and delete`);
                }
                try {
                    Object.defineProperty(WebSocket.prototype, name, { get() { return 'override'; } });
                    throw new Error(`${name} defineProperty getter should throw`);
                } catch (error) {
                    if (!(error instanceof TypeError)) throw error;
                }
                try {
                    Object.defineProperty(WebSocket.prototype, name, { set() {} });
                    throw new Error(`${name} defineProperty setter should throw`);
                } catch (error) {
                    if (!(error instanceof TypeError)) throw error;
                }
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket constants surface test should pass once implemented");
}

#[test]
fn websocket_wpt_target_method_replacement_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const socket = new WebSocket('wss://example.test/socket');
            for (const name of ['send', 'close']) {
                const descriptor = Object.getOwnPropertyDescriptor(WebSocket.prototype, name);
                if (!descriptor || typeof descriptor.value !== 'function') {
                    throw new Error(`${name} should be a prototype function`);
                }
                if (descriptor.writable !== true || descriptor.enumerable !== true ||
                    descriptor.configurable !== true) {
                    throw new Error(`${name} descriptor should match Web IDL operations`);
                }
                socket[name] = 5;
                if (socket[name] !== 5) {
                    throw new Error(`${name} should be replaceable on the instance`);
                }
                delete socket[name];
                if (typeof socket[name] !== 'function') {
                    throw new Error(`${name} should fall back to prototype after delete`);
                }
            }
            const closeResult = socket.close();
            if (closeResult !== undefined) {
                throw new Error('close() should return undefined');
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket method replacement test should pass once implemented");
}

#[test]
fn websocket_wpt_target_event_handler_object_assignment_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    vm.eval(
        r#"
        (() => {
            const socket = new WebSocket('wss://example.test/socket');
            for (const name of ['onopen', 'onmessage', 'onerror', 'onclose']) {
                let called = false;
                const handler = { handleEvent() { called = true; } };
                socket[name] = handler;
                if (socket[name] !== handler) {
                    throw new Error(`${name} should retain object handler value`);
                }
                socket.dispatchEvent(new Event(name.slice(2)));
                if (called) {
                    throw new Error(`${name} object handler should not call handleEvent`);
                }
                socket[name] = undefined;
                if (socket[name] !== null) {
                    throw new Error(`${name} should clear to null when set to undefined`);
                }
            }
            return 'ok';
        })()
        "#,
    )
    .expect("target WebSocket event handler object assignment test should pass once implemented");
}

#[test]
fn websocket_wpt_target_websocket_stream_surface_is_accounted_for() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval("String(typeof WebSocketStream === 'function')")
        .expect("target WebSocketStream inventory test should evaluate");

    assert_eq!(result, "true");
}

#[test]
fn websocket_wpt_target_websocket_stream_constructor_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const stream = new WebSocketStream('wss://example.test/socket');
                return [
                    stream instanceof WebSocketStream,
                    typeof stream.url,
                    stream.opened instanceof Promise,
                    stream.closed instanceof Promise,
                    typeof stream.close,
                    String(stream.close())
                ].join('|');
            })()
            "#,
        )
        .expect("target WebSocketStream constructor surface test should evaluate");

    assert_eq!(result, "true|string|true|true|function|undefined");
}

#[test]
fn websocket_stream_backing_slots_ignore_reflection_and_spoofing() {
    let mut vm = new_storage_test_vm("https://websocket-private-slots.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const stream = new WebSocketStream('wss://example.test/socket');
                const opened = stream.opened;
                const closed = stream.closed;
                const internalNamesBefore = Object.getOwnPropertyNames(stream)
                    .filter(name => name.startsWith('__moliWebSocketStream'))
                    .sort();
                stream.__moliWebSocketStreamUrl = 'wss://spoofed.test/';
                stream.__moliWebSocketStreamOpened = Promise.resolve('fake-opened');
                stream.__moliWebSocketStreamClosed = Promise.resolve('fake-closed');
                stream.__moliWebSocketStreamOpenedResolve = () => {};
                stream.__moliWebSocketStreamOpenedReject = () => {};
                stream.__moliWebSocketStreamClosedResolve = () => {};
                stream.__moliWebSocketStreamClosedReject = () => {};
                const fake = { __moliWebSocketStreamUrl: 'wss://fake.test/' };
                const descriptorReport = name => {
                    const descriptor = Object.getOwnPropertyDescriptor(WebSocketStream.prototype, name);
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
                const urlDescriptor = Object.getOwnPropertyDescriptor(WebSocketStream.prototype, 'url');
                const openedDescriptor = Object.getOwnPropertyDescriptor(WebSocketStream.prototype, 'opened');
                const closedDescriptor = Object.getOwnPropertyDescriptor(WebSocketStream.prototype, 'closed');
                return JSON.stringify({
                    internalNamesBefore,
                    descriptors: [
                        descriptorReport('url'),
                        descriptorReport('opened'),
                        descriptorReport('closed')
                    ],
                    url: stream.url,
                    openedSame: stream.opened === opened,
                    closedSame: stream.closed === closed,
                    fakeUrl: urlDescriptor.get.call(fake),
                    fakeOpened: String(openedDescriptor.get.call(fake)),
                    fakeClosed: String(closedDescriptor.get.call(fake))
                });
            })()
            "#,
        )
        .expect("WebSocketStream private slot spoofing test should evaluate");

    assert_eq!(
        result,
        r#"{"internalNamesBefore":[],"descriptors":["url:function:get url:0:undefined:true:true","opened:function:get opened:0:undefined:true:true","closed:function:get closed:0:undefined:true:true"],"url":"wss://example.test/socket","openedSame":true,"closedSame":true,"fakeUrl":"","fakeOpened":"undefined","fakeClosed":"undefined"}"#
    );
}

#[test]
fn websocket_wpt_target_websocket_stream_loopback_websocket_is_allowed_from_secure_context() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const stream = new WebSocketStream('ws://127.0.0.1/socket');
                return stream.url;
            })()
            "#,
        )
        .expect("target WebSocketStream loopback mixed-content exemption test should evaluate");

    assert_eq!(result, "ws://127.0.0.1/socket");
}

#[test]
fn websocket_wpt_target_websocket_error_constructor_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                function outcome(callback) {
                    try {
                        callback();
                        return 'ok';
                    } catch (error) {
                        return `${error && error.name}:${error instanceof DOMException}`;
                    }
                }
                const defaults = new WebSocketError();
                const full = new WebSocketError('message', { closeCode: 3456, reason: 'reason' });
                const codeOnly = new WebSocketError('', { closeCode: 3333 });
                const reasonOnly = new WebSocketError('', { reason: 'specified' });
                const closeCodeDescriptor =
                    Object.getOwnPropertyDescriptor(WebSocketError.prototype, 'closeCode');
                const reasonDescriptor =
                    Object.getOwnPropertyDescriptor(WebSocketError.prototype, 'reason');
                return [
                    typeof WebSocketError,
                    defaults instanceof DOMException,
                    defaults instanceof WebSocketError,
                    Object.prototype.toString.call(defaults),
                    defaults.name,
                    defaults.message,
                    String(defaults.code),
                    String(defaults.closeCode),
                    defaults.reason,
                    full.message,
                    String(full.closeCode),
                    full.reason,
                    String(codeOnly.closeCode),
                    codeOnly.reason,
                    String(reasonOnly.closeCode),
                    reasonOnly.reason,
                    typeof closeCodeDescriptor.get,
                    closeCodeDescriptor.get.name,
                    String(closeCodeDescriptor.get.length),
                    String(closeCodeDescriptor.set === undefined),
                    String(closeCodeDescriptor.enumerable),
                    String(closeCodeDescriptor.configurable),
                    typeof reasonDescriptor.get,
                    reasonDescriptor.get.name,
                    String(reasonDescriptor.get.length),
                    String(reasonDescriptor.set === undefined),
                    String(reasonDescriptor.enumerable),
                    String(reasonDescriptor.configurable),
                    String(Object.getPrototypeOf(WebSocketError.prototype) === DOMException.prototype),
                    String(Object.prototype.hasOwnProperty.call(defaults, 'message')),
                    String(Object.prototype.hasOwnProperty.call(defaults, 'closeCode')),
                    String(new DOMException('message', 'WebSocketError') instanceof WebSocketError),
                    outcome(() => Reflect.get(WebSocketError.prototype, 'closeCode')),
                    outcome(() => Reflect.get(WebSocketError.prototype, 'reason')),
                    outcome(() => new WebSocketError('', { closeCode: 1005 })),
                    outcome(() => new WebSocketError('', { closeCode: 1000, reason: 'x'.repeat(124) })),
                    outcome(() => new WebSocketError('', { reason: '\u{1f50c}'.repeat(32) })),
                ].join('|');
            })()
            "#,
        )
        .expect("target WebSocketError constructor surface test should evaluate");

    assert_eq!(
        result,
        "function|true|true|[object WebSocketError]|WebSocketError||0|null||message|3456|reason|3333||1000|specified|function|get closeCode|0|true|true|true|function|get reason|0|true|true|true|true|false|false|false|TypeError:false|TypeError:false|InvalidAccessError:true|SyntaxError:true|SyntaxError:true"
    );
}

#[test]
fn websocket_wpt_target_websocket_stream_options_validation_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/base/page.html");

    let result = vm
        .eval(
            r#"
            (() => {
                function outcome(callback) {
                    try {
                        callback();
                        return 'ok';
                    } catch (error) {
                        return `${error && error.name}:${String(error && error.message).includes('WebSocketStream')}`;
                    }
                }
                const iterable = {
                    *[Symbol.iterator]() {
                        yield 'alpha';
                        yield 'beta';
                    }
                };
                const validArray = new WebSocketStream('/array', { protocols: ['alpha', 'beta'] });
                const validIterable = new WebSocketStream('/iterable', { protocols: iterable });
                const noProtocols = new WebSocketStream('/none', {});
                return [
                    outcome(() => new WebSocketStream()),
                    outcome(() => new WebSocketStream('/ok', true)),
                    outcome(() => new WebSocketStream('/ok', { protocols: 'chat' })),
                    outcome(() => new WebSocketStream('/ok', { protocols: ['bad protocol'] })),
                    outcome(() => new WebSocketStream('/ok', { protocols: ['chat', 'CHAT'] })),
                    validArray.url,
                    validArray.opened instanceof Promise,
                    validIterable.url,
                    noProtocols.url,
                ].join('|');
            })()
            "#,
        )
        .expect("target WebSocketStream options validation surface test should evaluate");

    assert_eq!(
        result,
        "TypeError:true|TypeError:true|TypeError:true|SyntaxError:true|SyntaxError:true|wss://websocket-wpt-target.test/array|true|wss://websocket-wpt-target.test/iterable|wss://websocket-wpt-target.test/none"
    );
}

#[test]
fn websocket_wpt_target_websocket_stream_close_info_validation_surface() {
    let mut vm = new_storage_test_vm("https://websocket-wpt-target.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const stream = new WebSocketStream('wss://example.test/socket');
                function outcome(callback) {
                    try {
                        callback();
                        return 'ok';
                    } catch (error) {
                        return `${error && error.name}:${error instanceof DOMException}`;
                    }
                }
                return [
                    String(stream.close()),
                    outcome(() => stream.close({})),
                    outcome(() => stream.close({ closeCode: 3456, reason: 'pizza' })),
                    outcome(() => stream.close({ reason: 'non-empty' })),
                    outcome(() => stream.close(true)),
                    outcome(() => stream.close({ reason: '.'.repeat(124) })),
                    outcome(() => stream.close({ closeCode: 999 })),
                    outcome(() => stream.close({ closeCode: 1001 })),
                    outcome(() => stream.close({ closeCode: 2999 })),
                    outcome(() => stream.close({ closeCode: 5000 })),
                ].join('|');
            })()
            "#,
        )
        .expect("target WebSocketStream close info validation surface test should evaluate");

    assert_eq!(
        result,
        "undefined|ok|ok|ok|TypeError:false|SyntaxError:true|InvalidAccessError:true|InvalidAccessError:true|InvalidAccessError:true|InvalidAccessError:true"
    );
}

#[test]
fn readable_stream_pending_read_resolves_on_future_enqueue() {
    let mut vm = new_storage_test_vm("https://stream-runtime.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__streamEvents = [];
                const stream = new ReadableStream({
                    start(controller) {
                        globalThis.__streamController = controller;
                    }
                });
                const reader = stream.getReader();
                reader.read().then(({ value, done }) => {
                    globalThis.__streamEvents.push(`${value}:${done}`);
                });
                return JSON.stringify(globalThis.__streamEvents);
            })()
            "#,
        )
        .expect("pending stream read setup should evaluate");
    assert_eq!(initial, "[]");

    vm.eval(
        r#"
            (() => {
                globalThis.__streamController.enqueue('future');
                return JSON.stringify(globalThis.__streamEvents);
            })()
            "#,
    )
    .expect("future stream enqueue should evaluate");
    let after_enqueue = vm
        .eval("JSON.stringify(globalThis.__streamEvents)")
        .expect("future stream enqueue microtask should settle");
    assert_eq!(after_enqueue, r#"["future:false"]"#);
}

#[test]
fn readable_stream_pending_read_resolves_done_on_future_close() {
    let mut vm = new_storage_test_vm("https://stream-runtime.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__streamEvents = [];
                const stream = new ReadableStream({
                    start(controller) {
                        globalThis.__streamController = controller;
                    }
                });
                const reader = stream.getReader();
                reader.read().then(({ value, done }) => {
                    globalThis.__streamEvents.push(`${String(value)}:${done}`);
                });
                return JSON.stringify(globalThis.__streamEvents);
            })()
            "#,
        )
        .expect("pending stream read setup should evaluate");
    assert_eq!(initial, "[]");

    vm.eval(
        r#"
            (() => {
                globalThis.__streamController.close();
                return JSON.stringify(globalThis.__streamEvents);
            })()
            "#,
    )
    .expect("future stream close should evaluate");
    let after_close = vm
        .eval("JSON.stringify(globalThis.__streamEvents)")
        .expect("future stream close microtask should settle");
    assert_eq!(after_close, r#"["undefined:true"]"#);
}

#[test]
fn readable_stream_async_iterator_return_honors_prevent_cancel() {
    let mut vm = new_storage_test_vm("https://stream-runtime.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__streamEvents = [];
                const defaultStream = new ReadableStream({
                    start(controller) {
                        controller.enqueue('default-queued');
                    },
                    cancel(reason) {
                        globalThis.__streamEvents.push(`default-cancel:${reason}`);
                    }
                });
                const defaultIterator = defaultStream.values();
                defaultIterator.return('stop').then(({ value, done }) => {
                    globalThis.__streamEvents.push(`default-return:${String(value)}:${done}:${defaultStream.locked}`);
                });
                defaultIterator.next().then(({ value, done }) => {
                    globalThis.__streamEvents.push(`default-next-after-return:${String(value)}:${done}`);
                });

                const keptStream = new ReadableStream({
                    start(controller) {
                        controller.enqueue('kept-queued');
                    },
                    cancel(reason) {
                        globalThis.__streamEvents.push(`kept-cancel:${reason}`);
                    }
                });
                const keptIterator = keptStream.values({
                    get preventCancel() {
                        globalThis.__streamEvents.push('prevent-get');
                        return true;
                    }
                });
                keptIterator.return('keep').then(({ value, done }) => {
                    globalThis.__streamEvents.push(`kept-return:${String(value)}:${done}:${keptStream.locked}`);
                });
                keptIterator.next().then(({ value, done }) => {
                    globalThis.__streamEvents.push(`kept-next-after-return:${String(value)}:${done}`);
                });
                globalThis.__keptStream = keptStream;
                return JSON.stringify(globalThis.__streamEvents);
            })()
            "#,
        )
        .expect("readable stream async iterator return setup should evaluate");
    assert_eq!(initial, r#"["default-cancel:stop","prevent-get"]"#);

    let after_return = vm
        .eval("JSON.stringify(globalThis.__streamEvents)")
        .expect("readable stream async iterator return promises should settle");
    assert_eq!(
        after_return,
        r#"["default-cancel:stop","prevent-get","kept-return:keep:true:false","kept-next-after-return:undefined:true","default-return:stop:true:false","default-next-after-return:undefined:true"]"#
    );

    vm.eval(
        r#"
            (() => {
                globalThis.__keptStream.getReader().read().then(({ value, done }) => {
                    globalThis.__streamEvents.push(`kept-read-after-return:${value}:${done}`);
                });
                return JSON.stringify(globalThis.__streamEvents);
            })()
            "#,
    )
    .expect("preventCancel stream read should evaluate");
    let after_read = vm
        .eval("JSON.stringify(globalThis.__streamEvents)")
        .expect("preventCancel stream read promise should settle");
    assert_eq!(
        after_read,
        r#"["default-cancel:stop","prevent-get","kept-return:keep:true:false","kept-next-after-return:undefined:true","default-return:stop:true:false","default-next-after-return:undefined:true","kept-read-after-return:kept-queued:false"]"#
    );

    let non_object = vm
        .eval(
            r#"
            (() => {
                try {
                    new ReadableStream().values(1);
                    return 'ok';
                } catch (error) {
                    return `${error.name}:${error instanceof TypeError}`;
                }
            })()
            "#,
        )
        .expect("readable stream values non-object options should evaluate");
    assert_eq!(non_object, "TypeError:true");
}

#[test]
fn readable_stream_async_iterator_serializes_operations_and_waits_for_cancel() {
    let mut vm = new_storage_test_vm("https://stream-runtime.test/");

    let initial = vm
        .eval(
            r#"
            (() => {
                globalThis.__iteratorOwnerEvents = [];
                globalThis.__iteratorOwnerResult = "pending";
                let resolveCancel;
                const delayed = new ReadableStream({
                    cancel(reason) {
                        globalThis.__iteratorOwnerEvents.push(`cancel:${reason}`);
                        return new Promise(resolve => { resolveCancel = resolve; });
                    }
                });
                const delayedIterator = delayed.values();
                const returned = delayedIterator.return('stop').then(result => {
                    globalThis.__iteratorOwnerEvents.push(`return:${result.value}:${result.done}`);
                    return result;
                });
                const next = delayedIterator.next().then(result => {
                    globalThis.__iteratorOwnerEvents.push(`next:${String(result.value)}:${result.done}`);
                    return result;
                });
                globalThis.__iteratorOwnerEvents.push(`locked:${delayed.locked}`);

                let pulls = 0;
                const error = { name: 'iterator-error' };
                const errored = new ReadableStream({
                    pull(controller) {
                        if (pulls++ === 0) controller.enqueue(0);
                        else controller.error(error);
                    }
                });
                const erroredIterator = errored.values();
                const ordered = Promise.allSettled([
                    erroredIterator.next(),
                    erroredIterator.next(),
                    erroredIterator.next(),
                    erroredIterator.return('after-error')
                ]);

                Promise.all([returned, next, ordered]).then(([, , results]) => {
                    globalThis.__iteratorOwnerResult = JSON.stringify({
                        events: globalThis.__iteratorOwnerEvents,
                        results: results.map(result => result.status === 'rejected'
                            ? `rejected:${result.reason === error}`
                            : `fulfilled:${String(result.value.value)}:${result.value.done}`)
                    });
                });
                globalThis.__resolveIteratorCancel = resolveCancel;
                return JSON.stringify(globalThis.__iteratorOwnerEvents);
            })()
            "#,
        )
        .expect("async iterator owner setup should evaluate");
    assert_eq!(initial, r#"["cancel:stop","locked:false"]"#);

    let pending = vm
        .eval("globalThis.__iteratorOwnerResult")
        .expect("pending async iterator owner result should evaluate");
    assert_eq!(pending, "pending");

    vm.eval("globalThis.__resolveIteratorCancel()")
        .expect("delayed iterator cancel should resolve");
    for _ in 0..8 {
        let result = vm
            .eval("globalThis.__iteratorOwnerResult")
            .expect("async iterator owner promises should drain");
        if result != "pending" {
            break;
        }
    }
    let result = vm
        .eval("globalThis.__iteratorOwnerResult")
        .expect("async iterator owner result should evaluate");
    assert_eq!(
        result,
        r#"{"events":["cancel:stop","locked:false","return:stop:true","next:undefined:true"],"results":["fulfilled:0:false","rejected:true","fulfilled:undefined:true","fulfilled:after-error:true"]}"#
    );
}

#[test]
fn readable_stream_async_iterator_declared_shape_keeps_symbol_and_tag() {
    let mut vm = new_storage_test_vm("https://stream-runtime.test/");

    let result = vm
        .eval(
            r#"
            (() => {
                const internalNames = () => Object.getOwnPropertyNames(globalThis)
                    .filter(name => name === "__moliReadableStreamAsyncIteratorPrototype")
                    .join(",");
                const globalNamesBefore = internalNames();
                const stream = new ReadableStream({
                    start(controller) {
                        controller.enqueue("queued");
                    }
                });
                const iterator = stream.values({ preventCancel: true });
                const prototype = Object.getPrototypeOf(iterator);
                const intrinsicAsyncIteratorPrototype =
                    Object.getPrototypeOf(Object.getPrototypeOf(async function*() {}).prototype);
                const asyncDescriptor = Object.getOwnPropertyDescriptor(
                    intrinsicAsyncIteratorPrototype,
                    Symbol.asyncIterator
                );
                const tagDescriptor = Object.getOwnPropertyDescriptor(prototype, Symbol.toStringTag);
                const ownSymbols = Object.getOwnPropertySymbols(iterator).map(symbol => {
                    if (symbol === Symbol.asyncIterator) return "Symbol.asyncIterator";
                    return String(symbol);
                });
                const globalNamesAfterCache = internalNames();
                Object.defineProperty(globalThis, "__moliReadableStreamAsyncIteratorPrototype", {
                    configurable: true,
                    value: { spoofed: true }
                });
                const globalNamesAfterSpoof = internalNames();
                const iteratorAfterSpoof = new ReadableStream().values();
                return JSON.stringify({
                    toString: Object.prototype.toString.call(iterator),
                    asyncIteratorReturnsSelf: iterator[Symbol.asyncIterator]() === iterator,
                    globalNamesBefore,
                    globalNamesAfterCache,
                    globalNamesAfterSpoof,
                    ownNames: Object.getOwnPropertyNames(iterator),
                    ownSymbols,
                    leaksReader: "__moliReadableStreamIteratorReader" in iterator,
                    leaksClosed: "__moliReadableStreamIteratorClosed" in iterator,
                    leaksPreventCancel: "__moliReadableStreamIteratorPreventCancel" in iterator,
                    asyncDescriptor: [
                        asyncDescriptor.enumerable,
                        asyncDescriptor.writable,
                        asyncDescriptor.configurable,
                        asyncDescriptor.value.length
                    ].join(","),
                    prototypeParentIsIntrinsic:
                        Object.getPrototypeOf(prototype) === intrinsicAsyncIteratorPrototype,
                    prototypeStableAfterSpoof:
                        Object.getPrototypeOf(iteratorAfterSpoof) === prototype,
                    prototypeNames: Object.getOwnPropertyNames(prototype).sort(),
                    prototypeHasConstructor: Object.hasOwn(prototype, "constructor"),
                    tagDescriptor: [
                        tagDescriptor.enumerable,
                        tagDescriptor.writable,
                        tagDescriptor.configurable,
                        tagDescriptor.value
                    ].join(",")
                });
            })()
            "#,
        )
        .expect("readable stream async iterator declared shape should evaluate");

    assert_eq!(
        result,
        r#"{"toString":"[object ReadableStream AsyncIterator]","asyncIteratorReturnsSelf":true,"globalNamesBefore":"","globalNamesAfterCache":"","globalNamesAfterSpoof":"__moliReadableStreamAsyncIteratorPrototype","ownNames":[],"ownSymbols":[],"leaksReader":false,"leaksClosed":false,"leaksPreventCancel":false,"asyncDescriptor":"false,true,true,0","prototypeParentIsIntrinsic":true,"prototypeStableAfterSpoof":true,"prototypeNames":["next","return"],"prototypeHasConstructor":false,"tagDescriptor":"false,false,true,ReadableStream AsyncIterator"}"#
    );
}
