use std::pin::pin;
use std::time::Duration;

use tokio::time::sleep;

use crate::context_bootstrap::worker_host::{
    constructor::worker_constructor_callback,
    dispatch::dispatch_worker_messages,
    methods::{worker_post_message_callback, worker_terminate_callback},
};
use crate::ensure_v8_for_test as ensure_v8;

/// Create an isolate + context with the Worker constructor installed.
/// Returns (isolate, context_global).
fn setup_worker_context() -> (v8::OwnedIsolate, v8::Global<v8::Context>) {
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let request_client_owner =
        crate::network::ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("standalone Worker constructor tests need a fetch runtime");
    assert!(
        isolate.set_slot::<crate::network::ResourceRequestClientOwner>(request_client_owner),
        "standalone Worker constructor tests must install one fetch runtime owner"
    );
    let context = {
        let scope = pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();

        // Build Worker constructor via FunctionTemplate so prototype methods work.
        let worker_tmpl = v8::FunctionTemplate::builder(worker_constructor_callback)
            .length(1)
            .build(scope);

        let class_name = v8::String::new(scope, "Worker").unwrap();
        worker_tmpl.set_class_name(class_name);

        // Install prototype methods.
        let proto = worker_tmpl.prototype_template(scope);
        let pm_key = v8::String::new(scope, "postMessage").unwrap();
        proto.set(
            pm_key.into(),
            v8::FunctionTemplate::builder(worker_post_message_callback)
                .length(1)
                .build(scope)
                .into(),
        );
        let term_key = v8::String::new(scope, "terminate").unwrap();
        proto.set(
            term_key.into(),
            v8::FunctionTemplate::builder(worker_terminate_callback)
                .length(0)
                .build(scope)
                .into(),
        );

        let context = v8::Context::new(scope, Default::default());
        let ctx_scope = &mut v8::ContextScope::new(scope, context);
        let global = context.global(ctx_scope);

        install_dom_exception(ctx_scope, global);
        let form_data_template =
            crate::context_bootstrap::build_named_constructor_template(ctx_scope, "FormData")
                .expect("FormData template");
        let form_data = form_data_template
            .get_function(ctx_scope)
            .expect("FormData constructor");
        let _ = global.define_own_property(
            ctx_scope,
            v8::String::new(ctx_scope, "FormData").unwrap().into(),
            form_data.into(),
            v8::PropertyAttribute::DONT_ENUM,
        );

        // Install Worker constructor on global.
        let worker_fn = worker_tmpl.get_function(ctx_scope).unwrap();
        let worker_key = v8::String::new(ctx_scope, "Worker").unwrap();
        let _ = global.set(ctx_scope, worker_key.into(), worker_fn.into());

        // Install __drainWorkerMessages helper (calls dispatch_worker_messages)
        let drain_fn = v8::Function::new(ctx_scope, drain_worker_messages_callback).unwrap();
        let drain_key = v8::String::new(ctx_scope, "__drainWorkerMessages").unwrap();
        let _ = global.set(ctx_scope, drain_key.into(), drain_fn.into());

        v8::Global::new(ctx_scope, context)
    };
    (isolate, context)
}

fn install_dom_exception<'s>(scope: &mut v8::PinScope<'s, '_>, global: v8::Local<'s, v8::Object>) {
    let template =
        crate::context_bootstrap::build_named_constructor_template(scope, "DOMException")
            .expect("DOMException template");
    let constructor = template
        .get_function(scope)
        .expect("DOMException constructor");
    let prototype_key = v8::String::new(scope, "prototype").unwrap();
    let prototype = constructor
        .get(scope, prototype_key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .expect("DOMException prototype");
    let _ = global.define_own_property(
        scope,
        v8::String::new(scope, "DOMException").unwrap().into(),
        constructor.into(),
        v8::PropertyAttribute::DONT_ENUM,
    );
    crate::context_bootstrap::finalize_dom_exception_realm_bindings(scope, prototype);
}

/// Helper callback: __drainWorkerMessages(workerObj) → bool
fn drain_worker_messages_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.length() < 1 {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    }
    let worker_val = args.get(0);
    let Ok(worker_obj) = v8::Local::<v8::Object>::try_from(worker_val) else {
        rv.set(v8::Boolean::new(scope, false).into());
        return;
    };
    // Convert through Global to align lifetimes with scope.
    let global_ref = v8::Global::new(scope, worker_obj);
    let worker_local = v8::Local::new(scope, &global_ref);
    let dispatched = dispatch_worker_messages(scope, worker_local);
    rv.set(v8::Boolean::new(scope, dispatched).into());
}

/// Run JS in the given context and return the stringified result.
fn eval(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    code: &str,
) -> Option<String> {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);

    let source = v8::String::new(scope, code)?;
    let script = v8::Script::compile(scope, source, None)?;
    let result = script.run(scope)?;
    Some(result.to_rust_string_lossy(scope))
}

/// Run JS, expect it to succeed (no exception).
fn eval_ok(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    code: &str,
) -> String {
    eval(isolate, context, code).unwrap_or_else(|| panic!("JS eval failed for: {code}"))
}

/// Run JS, expect it to throw.
fn eval_throws(
    isolate: &mut v8::OwnedIsolate,
    context: &v8::Global<v8::Context>,
    code: &str,
) -> String {
    let scope = pin!(v8::HandleScope::new(isolate));
    let scope = &mut scope.init();
    let ctx = v8::Local::new(scope, context);
    let scope = &mut v8::ContextScope::new(scope, ctx);

    let try_catch = pin!(v8::TryCatch::new(scope));
    let scope = try_catch.init();

    let source = v8::String::new(&scope, code).unwrap();
    let script = v8::Script::compile(&scope, source, None);
    if let Some(script) = script {
        let _result = script.run(&scope);
    }
    assert!(scope.has_caught(), "expected exception for: {code}");
    scope
        .exception()
        .map(|e| e.to_rust_string_lossy(&scope))
        .unwrap_or_default()
}

// ─── Constructor basics ─────────────────────────────────────────────

#[test]
fn constructor_exists() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let result = eval_ok(&mut isolate, &ctx, "typeof Worker");
    assert_eq!(result, "function");
}

#[test]
fn constructor_requires_new() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let err = eval_throws(&mut isolate, &ctx, "Worker('test')");
    assert!(err.contains("new"), "expected 'new' error, got: {err}");
}

#[test]
fn constructor_requires_argument() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let err = eval_throws(&mut isolate, &ctx, "new Worker()");
    assert!(
        err.contains("1 argument required"),
        "expected argument error, got: {err}"
    );
}

#[test]
fn constructor_script_url_uses_webidl_usvstring_conversion() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    let symbol_error = eval_throws(&mut isolate, &ctx, "new Worker(Symbol('worker-url'))");
    assert!(
        symbol_error.contains("TypeError"),
        "expected Symbol URL conversion TypeError, got: {symbol_error}"
    );

    let throwing_error = eval_throws(
        &mut isolate,
        &ctx,
        r#"
        new Worker({
          toString() {
            throw new RangeError('worker-url');
          }
        })
        "#,
    );
    assert!(
        throwing_error.contains("RangeError"),
        "expected URL stringifier exception propagation, got: {throwing_error}"
    );
}

#[test]
fn constructor_accepts_classic_worker_type_option() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let result = eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var w = new Worker("postMessage('hi')", { type: "classic" });
            typeof w;
            "#,
    );
    assert_eq!(result, "object");
    eval_ok(&mut isolate, &ctx, "w.terminate()");
    std::thread::sleep(Duration::from_millis(50));
}

#[tokio::test]
async fn constructor_accepts_module_worker_type_option() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var source = [
                "export const answer = 42;",
                "postMessage({",
                "  answer: answer,",
                "  importScriptsType: typeof importScripts,",
                "  metaUrl: import.meta.url",
                "});"
            ].join("\n");
            var w = new Worker("data:text/javascript," + encodeURIComponent(source), { type: "module" });
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(
            &mut isolate,
            &ctx,
            r#"
            __drainWorkerMessages(w);
            received === null
                ? "pending"
                : JSON.stringify([
                    received.answer,
                    received.importScriptsType,
                    received.metaUrl.indexOf("data:text/javascript,") === 0
                ]);
            "#,
        );
        if result != "pending" {
            assert_eq!(result, r#"[42,"function",true]"#);
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for module worker message");
}

#[tokio::test]
async fn constructor_module_worker_supports_data_url_static_imports() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            function dataUrl(source) {
                return "data:text/javascript," + encodeURIComponent(source);
            }
            var depUrl = dataUrl([
                "export const answer = 42;",
                "export const suffix = 'dep';",
                "export default 'default-value';",
                "export function double(value) { return value * 2; }",
                "export class Box { constructor(value) { this.value = value; } }"
            ].join("\n"));
            var reexportUrl = dataUrl([
                "export { answer as importedAnswer, default as importedDefault } from " + JSON.stringify(depUrl) + ";",
                "export * as depNamespace from " + JSON.stringify(depUrl) + ";"
            ].join("\n"));
            var starUrl = dataUrl([
                "export * from " + JSON.stringify(depUrl) + ";"
            ].join("\n"));
            var source = [
                "import defaultValue, { answer, suffix as renamed, double, Box } from " + JSON.stringify(depUrl) + ";",
                "import * as re from " + JSON.stringify(reexportUrl) + ";",
                "import { answer as starAnswer } from " + JSON.stringify(starUrl) + ";",
                "var box = new Box(9);",
                "postMessage({",
                "  defaultValue,",
                "  answer,",
                "  renamed,",
                "  doubled: double(21),",
                "  boxValue: box.value,",
                "  importedAnswer: re.importedAnswer,",
                "  importedDefault: re.importedDefault,",
                "  namespaceAnswer: re.depNamespace.answer,",
                "  starAnswer,",
                "  importScriptsType: typeof importScripts",
                "});"
            ].join("\n");
            var w = new Worker(dataUrl(source), { type: "module" });
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(
            &mut isolate,
            &ctx,
            r#"
            __drainWorkerMessages(w);
            received === null
                ? "pending"
                : JSON.stringify([
                    received.defaultValue,
                    received.answer,
                    received.renamed,
                    received.doubled,
                    received.boxValue,
                    received.importedAnswer,
                    received.importedDefault,
                    received.namespaceAnswer,
                    received.starAnswer,
                    received.importScriptsType
                ]);
            "#,
        );
        if result != "pending" {
            assert_eq!(
                result,
                r#"["default-value",42,"dep",42,9,42,"default-value",42,42,"function"]"#
            );
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for module worker import message");
}

#[test]
fn constructor_rejects_invalid_worker_type_option() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let err = eval_throws(
        &mut isolate,
        &ctx,
        r#"new Worker("postMessage('hi')", { type: "potato" })"#,
    );
    assert!(
        err.contains("worker type is invalid"),
        "expected invalid worker type error, got: {err}"
    );
}

#[test]
fn constructor_rejects_null_worker_type_option_member() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let err = eval_throws(
        &mut isolate,
        &ctx,
        r#"new Worker("postMessage('hi')", { type: null })"#,
    );
    assert!(
        err.contains("worker type is invalid"),
        "expected invalid worker type error, got: {err}"
    );
}

#[test]
fn constructor_returns_object() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let result = eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var w = new Worker("postMessage('hi')");
            typeof w;
            "#,
    );
    assert_eq!(result, "object");
    // Cleanup: terminate
    eval_ok(&mut isolate, &ctx, "w.terminate()");
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn constructor_has_onmessage_null() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let result = eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var w = new Worker("1");
            w.onmessage;
            "#,
    );
    assert_eq!(result, "null");
    eval_ok(&mut isolate, &ctx, "w.terminate()");
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn constructor_has_onerror_null() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let result = eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var w = new Worker("1");
            w.onerror;
            "#,
    );
    assert_eq!(result, "null");
    eval_ok(&mut isolate, &ctx, "w.terminate()");
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn constructor_declared_event_target_slots_ignore_reflection_and_spoofing() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var w = new Worker("1");
            const internalNames = [
                "__moliWorkerHandle",
                "__moliWorkerId",
                "__moliWorkerListeners",
                "__moliWorkerOnMessage",
                "__moliWorkerOnMessageError",
                "__moliWorkerOnError",
                "__moliEventTargetSlot",
                "__moliSimpleEventTargetOrderedHandlers"
            ];
            const reflected = Object.getOwnPropertyNames(w)
                .filter(name => internalNames.includes(name));
            if (reflected.length !== 0) {
                throw new Error(`Worker internals should not be reflected: ${reflected.join(",")}`);
            }
            const expectedMethods = {
                addEventListener: "true:true:true:true:function:0:addEventListener",
                removeEventListener: "true:true:true:true:function:0:removeEventListener",
                dispatchEvent: "true:true:true:true:function:0:dispatchEvent"
            };
            for (const [name, shape] of Object.entries(expectedMethods)) {
                const descriptor = Object.getOwnPropertyDescriptor(w, name);
                const actual = [
                    !!descriptor,
                    descriptor && descriptor.enumerable,
                    descriptor && descriptor.configurable,
                    descriptor && descriptor.writable,
                    descriptor && typeof descriptor.value,
                    descriptor && descriptor.value.length,
                    descriptor && descriptor.value.name
                ].join(":");
                if (actual !== shape) {
                    throw new Error(`${name} descriptor mismatch: ${actual}`);
                }
            }
            const expectedAccessors = {
                onmessage: "true:true:true:function:get onmessage:0:function:set onmessage:1:false",
                onmessageerror: "true:true:true:function:get onmessageerror:0:function:set onmessageerror:1:false",
                onerror: "true:true:true:function:get onerror:0:function:set onerror:1:false"
            };
            for (const [name, shape] of Object.entries(expectedAccessors)) {
                const descriptor = Object.getOwnPropertyDescriptor(w, name);
                const actual = [
                    !!descriptor,
                    descriptor && descriptor.enumerable,
                    descriptor && descriptor.configurable,
                    descriptor && typeof descriptor.get,
                    descriptor && descriptor.get.name,
                    descriptor && descriptor.get.length,
                    descriptor && typeof descriptor.set,
                    descriptor && descriptor.set.name,
                    descriptor && descriptor.set.length,
                    descriptor && ("writable" in descriptor)
                ].join(":");
                if (actual !== shape) {
                    throw new Error(`${name} descriptor mismatch: ${actual}`);
                }
            }
            for (const name of internalNames) {
                w[name] = name.includes("Ordered") ? false : null;
            }
            const calls = [];
            w.addEventListener("message", event => calls.push(`listener:${event.type}`));
            w.onmessage = event => calls.push(`handler:${event.type}`);
            if (typeof w.onmessage !== "function") {
                throw new Error("onmessage getter should ignore public slot spoofing");
            }
            w.dispatchEvent({ type: "message" });
            const result = calls.join("|");
            if (result !== "listener:message|handler:message") {
                throw new Error(`Worker ordered dispatch was spoofed: ${result}`);
            }
            w.terminate();
            "ok";
            "#,
    );
    std::thread::sleep(Duration::from_millis(50));
}

// ─── postMessage / terminate methods ────────────────────────────────

#[test]
fn worker_has_post_message_method() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let result = eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var w = new Worker("1");
            typeof w.postMessage;
            "#,
    );
    assert_eq!(result, "function");
    eval_ok(&mut isolate, &ctx, "w.terminate()");
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn worker_post_message_requires_argument() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let err = eval_throws(
        &mut isolate,
        &ctx,
        r#"
            var w = new Worker("1");
            try {
                w.postMessage();
            } finally {
                w.terminate();
            }
            "#,
    );
    assert!(
        err.contains("1 argument required"),
        "expected missing-argument error, got: {err}"
    );
}

#[test]
fn worker_post_message_rejects_non_iterable_second_argument() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let err = eval_throws(
        &mut isolate,
        &ctx,
        r#"
            var w = new Worker("1");
            try {
                w.postMessage("payload", 1);
            } finally {
                w.terminate();
            }
            "#,
    );
    assert!(
        err.contains("TypeError"),
        "expected TypeError for non-iterable second argument, got: {err}"
    );
    assert!(
        err.contains("parameter 2 is not an iterable object or options dictionary"),
        "expected non-iterable second-argument error, got: {err}"
    );
}

#[test]
fn worker_post_message_rejects_non_iterable_transfer_option() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let err = eval_throws(
        &mut isolate,
        &ctx,
        r#"
            var w = new Worker("1");
            try {
                w.postMessage("payload", { transfer: 1 });
            } finally {
                w.terminate();
            }
            "#,
    );
    assert!(
        err.contains("TypeError"),
        "expected TypeError for non-iterable transfer option, got: {err}"
    );
    assert!(
        err.contains("transfer list is not an iterable object"),
        "expected transfer-list iterable error, got: {err}"
    );
}

#[test]
fn worker_has_terminate_method() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    let result = eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var w = new Worker("1");
            typeof w.terminate;
            "#,
    );
    assert_eq!(result, "function");
    eval_ok(&mut isolate, &ctx, "w.terminate()");
    std::thread::sleep(Duration::from_millis(50));
}

#[test]
fn worker_terminate_does_not_throw() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();
    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var w = new Worker("1");
            w.terminate();
            "ok";
            "#,
    );
    std::thread::sleep(Duration::from_millis(50));
}

// ─── Data URL support ───────────────────────────────────────────────

#[tokio::test]
async fn constructor_data_url_plain() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var w = new Worker("data:text/javascript,postMessage('from-data-url')");
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    // Wait for message
    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); received");
        if result != "null" {
            assert_eq!(result, "from-data-url");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for message from data URL worker");
}

#[tokio::test]
async fn constructor_data_url_base64() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    // postMessage('base64') in base64 = cG9zdE1lc3NhZ2UoJ2Jhc2U2NCcp
    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var w = new Worker("data:text/javascript;base64,cG9zdE1lc3NhZ2UoJ2Jhc2U2NCcp");
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); received");
        if result != "null" {
            assert_eq!(result, "base64");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for message from base64 data URL worker");
}

// ─── Message round-trip ─────────────────────────────────────────────

#[tokio::test]
async fn constructor_onmessage_receives_data() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var w = new Worker("postMessage('hello')");
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); received");
        if result != "null" {
            assert_eq!(result, "hello");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for onmessage");
}

#[tokio::test]
async fn constructor_pingpong() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var reply = null;
            var w = new Worker("onmessage = function(e) { postMessage('pong:' + e.data); };");
            w.onmessage = function(e) { reply = e.data; };
            "#,
    );

    // Give the worker time to start its event loop
    sleep(Duration::from_millis(50)).await;

    eval_ok(&mut isolate, &ctx, "w.postMessage('ping')");

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); reply");
        if result != "null" {
            assert_eq!(result, "pong:ping");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for ping-pong reply");
}

#[tokio::test]
async fn constructor_pingpong_arraybuffer() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var reply = null;
            var w = new Worker(`
                onmessage = function(e) {
                    postMessage(Array.from(new Uint8Array(e.data)).join(','));
                };
            `);
            w.onmessage = function(e) { reply = e.data; };
            "#,
    );

    sleep(Duration::from_millis(50)).await;

    eval_ok(
        &mut isolate,
        &ctx,
        "w.postMessage(new Uint8Array([7, 8, 9]).buffer)",
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); reply");
        if result != "null" {
            assert_eq!(result, "7,8,9");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for ArrayBuffer ping-pong reply");
}

#[tokio::test]
async fn constructor_postmessage_arraybuffer_transfer_to_worker_detaches_sender() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var detached = null;
            var w = new Worker(`
                onmessage = function(e) {
                    postMessage(Array.from(new Uint8Array(e.data)).join(','));
                };
            `);
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    sleep(Duration::from_millis(50)).await;
    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            const transferred = new Uint8Array([7, 8, 9]).buffer;
            w.postMessage(transferred, [transferred]);
            detached = transferred.byteLength;
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(
            &mut isolate,
            &ctx,
            "__drainWorkerMessages(w); received !== null ? `${received}|${detached}` : null",
        );
        if result != "null" {
            assert_eq!(result, "7,8,9|0");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for transferred ArrayBuffer ping-pong reply");
}

#[tokio::test]
async fn constructor_postmessage_arraybuffer_transfer_with_options_dict() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var detached = null;
            var w = new Worker(`
                onmessage = function(e) {
                    postMessage(Array.from(new Uint8Array(e.data)).join(','));
                };
            `);
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    sleep(Duration::from_millis(50)).await;
    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            const transferred = new Uint8Array([10, 11]).buffer;
            w.postMessage(transferred, { transfer: [transferred] });
            detached = transferred.byteLength;
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(
            &mut isolate,
            &ctx,
            "__drainWorkerMessages(w); received !== null ? `${received}|${detached}` : null",
        );
        if result != "null" {
            assert_eq!(result, "10,11|0");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for transferred ArrayBuffer via options dict");
}

#[tokio::test]
async fn constructor_postmessage_arraybuffer_transfer_with_iterable_options_transfer() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var detached = null;
            var w = new Worker(`
                onmessage = function(e) {
                    postMessage(Array.from(new Uint8Array(e.data)).join(','));
                };
            `);
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    sleep(Duration::from_millis(50)).await;
    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            const transferred = new Uint8Array([17, 18]).buffer;
            const iterable = {
                [Symbol.iterator]: function* () {
                    yield transferred;
                }
            };
            w.postMessage(transferred, { transfer: iterable });
            detached = transferred.byteLength;
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(
            &mut isolate,
            &ctx,
            "__drainWorkerMessages(w); received !== null ? `${received}|${detached}` : null",
        );
        if result != "null" {
            assert_eq!(result, "17,18|0");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for transferred ArrayBuffer via iterable options.transfer");
}

#[tokio::test]
async fn constructor_postmessage_arraybuffer_transfer_with_raw_iterable_second_argument() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var detached = null;
            var w = new Worker(`
                onmessage = function(e) {
                    postMessage(Array.from(new Uint8Array(e.data)).join(','));
                };
            `);
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    sleep(Duration::from_millis(50)).await;
    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            const transferred = new Uint8Array([21, 22]).buffer;
            const iterable = {
                [Symbol.iterator]: function* () {
                    yield transferred;
                }
            };
            w.postMessage(transferred, iterable);
            detached = transferred.byteLength;
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(
            &mut isolate,
            &ctx,
            "__drainWorkerMessages(w); received !== null ? `${received}|${detached}` : null",
        );
        if result != "null" {
            assert_eq!(result, "21,22|0");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for transferred ArrayBuffer via raw iterable second argument");
}

#[tokio::test]
async fn constructor_postmessage_arraybuffer_with_null_transfer_option_clones_without_detaching() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var detached = null;
            var w = new Worker(`
                onmessage = function(e) {
                    postMessage(Array.from(new Uint8Array(e.data)).join(','));
                };
            `);
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    sleep(Duration::from_millis(50)).await;
    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            const cloned = new Uint8Array([12, 13, 14]).buffer;
            w.postMessage(cloned, { transfer: null });
            detached = cloned.byteLength;
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(
            &mut isolate,
            &ctx,
            "__drainWorkerMessages(w); received !== null ? `${received}|${detached}` : null",
        );
        if result != "null" {
            assert_eq!(result, "12,13,14|3");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for cloned ArrayBuffer via null transfer option");
}

#[tokio::test]
async fn constructor_postmessage_object() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var w = new Worker("postMessage({x: 42, y: 'hello'})");
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(
            &mut isolate,
            &ctx,
            "__drainWorkerMessages(w); received ? received.x + ':' + received.y : null",
        );
        if result != "null" {
            assert_eq!(result, "42:hello");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for object message");
}

#[tokio::test]
async fn constructor_postmessage_arraybuffer_from_worker() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var w = new Worker("postMessage(new Uint8Array([4, 5, 6]).buffer)");
            w.onmessage = function(e) {
                received = e.data instanceof ArrayBuffer
                    ? Array.from(new Uint8Array(e.data)).join(',')
                    : "not-arraybuffer";
            };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); received");
        if result != "null" {
            assert_eq!(result, "4,5,6");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for worker ArrayBuffer message");
}

#[tokio::test]
async fn constructor_postmessage_arraybuffer_transfer_from_worker_detaches_sender() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var detached = null;
            var w = new Worker(`
                const transferred = new Uint8Array([4, 5, 6]).buffer;
                postMessage(transferred, [transferred]);
                postMessage(transferred.byteLength);
            `);
            w.onmessage = function(e) {
                if (e.data instanceof ArrayBuffer) {
                    received = Array.from(new Uint8Array(e.data)).join(',');
                } else {
                    detached = String(e.data);
                }
            };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(
            &mut isolate,
            &ctx,
            "__drainWorkerMessages(w); received !== null && detached !== null ? `${received}|${detached}` : null",
        );
        if result != "null" {
            assert_eq!(result, "4,5,6|0");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for transferred worker ArrayBuffer message");
}

#[tokio::test]
async fn constructor_arraybuffer_round_trip_supports_dataview() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var w = new Worker(`
                onmessage = function(event) {
                    const view = new DataView(event.data);
                    postMessage(new Uint8Array([view.getUint8(0) + 1, view.getUint8(1) + 1]));
                };
            `);
            w.onmessage = function(e) {
                received = [
                    e.data.constructor.name,
                    e.data.length,
                    Array.from(e.data).join(','),
                    String(e.data.buffer instanceof ArrayBuffer)
                ].join('|');
            };
            "#,
    );

    sleep(Duration::from_millis(50)).await;
    eval_ok(
        &mut isolate,
        &ctx,
        "w.postMessage(new Uint8Array([40, 41]).buffer)",
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); received");
        if result != "null" {
            assert_eq!(result, "Uint8Array|2|41,42|true");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for ArrayBuffer round-trip");
}

#[test]
fn constructor_postmessage_rejects_uncloneable_payload() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(&mut isolate, &ctx, r#"var w = new Worker("1");"#);
    let err = eval_throws(&mut isolate, &ctx, "w.postMessage(function nope() {})");
    assert!(
        err.contains("DataCloneError"),
        "expected DataCloneError for uncloneable Worker payload, got: {err}"
    );
    eval_ok(&mut isolate, &ctx, "w.terminate()");
}

#[test]
fn constructor_postmessage_rejects_formdata_payload() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(&mut isolate, &ctx, r#"var w = new Worker("1");"#);
    let err = eval_throws(&mut isolate, &ctx, "w.postMessage(new FormData())");
    assert!(
        err.contains("DataCloneError"),
        "expected DataCloneError for FormData Worker payload, got: {err}"
    );
    eval_ok(&mut isolate, &ctx, "w.terminate()");
}

#[test]
fn constructor_postmessage_rejects_duplicate_transfer_entries() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(&mut isolate, &ctx, r#"var w = new Worker("1");"#);
    let err = eval_throws(
        &mut isolate,
        &ctx,
        "const buffer = new ArrayBuffer(1); w.postMessage(buffer, [buffer, buffer])",
    );
    assert!(
        err.contains("DataCloneError"),
        "expected DataCloneError for duplicate transfer entry, got: {err}"
    );
    eval_ok(&mut isolate, &ctx, "w.terminate()");
}

#[test]
fn constructor_postmessage_rejects_non_arraybuffer_transfer_entry() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(&mut isolate, &ctx, r#"var w = new Worker("1");"#);
    let err = eval_throws(
        &mut isolate,
        &ctx,
        "w.postMessage('nope', [new Uint8Array([1])])",
    );
    assert!(
        err.contains("DataCloneError"),
        "expected DataCloneError for non-ArrayBuffer transfer entry, got: {err}"
    );
    eval_ok(&mut isolate, &ctx, "w.terminate()");
}

#[test]
fn constructor_postmessage_rejects_wasm_memory_buffer_transfer_with_type_error() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(&mut isolate, &ctx, r#"var w = new Worker("1");"#);
    let err = eval_throws(
        &mut isolate,
        &ctx,
        "const buffer = new WebAssembly.Memory({ initial: 1 }).buffer; w.postMessage('nope', [buffer])",
    );
    assert!(
        err.contains("TypeError") && !err.contains("DataCloneError"),
        "expected TypeError for non-transferable wasm memory buffer, got: {err}"
    );
    eval_ok(&mut isolate, &ctx, "w.terminate()");
}

#[test]
fn constructor_postmessage_rejects_duplicate_options_transfer_entries() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(&mut isolate, &ctx, r#"var w = new Worker("1");"#);
    let err = eval_throws(
        &mut isolate,
        &ctx,
        "const buffer = new ArrayBuffer(1); w.postMessage(buffer, { transfer: [buffer, buffer] })",
    );
    assert!(
        err.contains("DataCloneError"),
        "expected DataCloneError for duplicate options.transfer entry, got: {err}"
    );
    eval_ok(&mut isolate, &ctx, "w.terminate()");
}

#[test]
fn constructor_postmessage_rejects_non_transferable_options_transfer_entry() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(&mut isolate, &ctx, r#"var w = new Worker("1");"#);
    let err = eval_throws(
        &mut isolate,
        &ctx,
        "w.postMessage('nope', { transfer: [new Uint8Array([1])] })",
    );
    assert!(
        err.contains("DataCloneError"),
        "expected DataCloneError for non-transferable options.transfer entry, got: {err}"
    );
    eval_ok(&mut isolate, &ctx, "w.terminate()");
}

#[tokio::test]
async fn constructor_message_listener_receives_event() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var w = new Worker("postMessage('listener')");
            w.addEventListener('message', function(e) { received = e.data; });
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); received");
        if result != "null" {
            assert_eq!(result, "listener");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for addEventListener('message')");
}

// ─── onerror ────────────────────────────────────────────────────────

#[tokio::test]
async fn constructor_onerror_uncaught_exception() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var errorMsg = null;
            var w = new Worker("throw new Error('boom')");
            w.onerror = function(e) { errorMsg = e.message; };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); errorMsg");
        if result != "null" {
            assert!(result.contains("boom"), "expected 'boom', got: {result}");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for onerror");
}

#[tokio::test]
async fn constructor_onerror_has_event_fields() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var errEvent = null;
            var w = new Worker("throw new Error('field-test')");
            w.onerror = function(e) { errEvent = e; };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w)");
        let result = eval_ok(
            &mut isolate,
            &ctx,
            "errEvent ? errEvent.type + '|' + typeof errEvent.filename + '|' + typeof errEvent.lineno : null",
        );
        if result != "null" {
            assert_eq!(result, "error|string|number");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for error event fields");
}

#[tokio::test]
async fn constructor_onerror_return_true_does_not_cancel_syntax_error_event() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var errorReturnObservations = null;
            var w = new Worker("function {");
            w.onerror = function(e) {
              errorReturnObservations = [arguments.length, e.defaultPrevented === true];
              return true;
            };
            w.addEventListener("error", function(e) {
              errorReturnObservations.push(e.defaultPrevented === true);
            });
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(
            &mut isolate,
            &ctx,
            "__drainWorkerMessages(w); JSON.stringify(errorReturnObservations)",
        );
        if result != "null" {
            assert_eq!(result, "[1,false,false]");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for worker syntax error event");
}

#[test]
fn worker_event_handler_only_boolean_false_cancels_synthetic_event() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    let result = eval_ok(
        &mut isolate,
        &ctx,
        r#"
        (() => {
          const w = new Worker("");
          const run = returned => {
            w.onerror = () => returned;
            const event = { type: "error", cancelable: true, defaultPrevented: false };
            return [w.dispatchEvent(event), event.defaultPrevented];
          };
          const observations = [run(true), run(false), run(0), run("")];
          w.terminate();
          return JSON.stringify(observations);
        })()
        "#,
    );

    assert_eq!(
        result,
        "[[true,false],[false,true],[true,false],[true,false]]"
    );
}

#[tokio::test]
async fn constructor_error_listener_receives_event() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var errorMsg = null;
            var w = new Worker("throw new Error('listener-boom')");
            w.addEventListener('error', function(e) { errorMsg = e.message; });
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); errorMsg");
        if result != "null" {
            assert!(
                result.contains("listener-boom"),
                "expected listener-boom, got: {result}"
            );
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for addEventListener('error')");
}

// ─── Multiple workers ───────────────────────────────────────────────

#[tokio::test]
async fn constructor_multiple_workers() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var msgs = [];
            var w1 = new Worker("postMessage('from-w1')");
            var w2 = new Worker("postMessage('from-w2')");
            w1.onmessage = function(e) { msgs.push(e.data); };
            w2.onmessage = function(e) { msgs.push(e.data); };
            "#,
    );

    for _ in 0..100 {
        sleep(Duration::from_millis(20)).await;
        eval_ok(
            &mut isolate,
            &ctx,
            "__drainWorkerMessages(w1); __drainWorkerMessages(w2)",
        );
        let count = eval_ok(&mut isolate, &ctx, "msgs.length");
        if count == "2" {
            let result = eval_ok(&mut isolate, &ctx, "msgs.sort().join(',')");
            assert_eq!(result, "from-w1,from-w2");
            eval_ok(&mut isolate, &ctx, "w1.terminate(); w2.terminate()");
            return;
        }
    }
    panic!("timed out waiting for both workers");
}

// ─── Terminate stops worker ─────────────────────────────────────────

#[tokio::test]
async fn constructor_terminate_stops_messages() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var count = 0;
            var w = new Worker("setInterval(function() { postMessage('tick'); }, 10);");
            w.onmessage = function(e) { count++; };
            "#,
    );

    // Wait for at least one tick
    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w)");
        let c = eval_ok(&mut isolate, &ctx, "count");
        if c != "0" {
            break;
        }
    }

    // Terminate, then record count
    eval_ok(&mut isolate, &ctx, "w.terminate()");
    sleep(Duration::from_millis(100)).await;
    let final_count = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); count");
    let c: u32 = final_count.parse().unwrap_or(0);

    // Wait a bit more, check no new messages arrive
    sleep(Duration::from_millis(200)).await;
    eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w)");
    let after_count = eval_ok(&mut isolate, &ctx, "count");
    let ac: u32 = after_count.parse().unwrap_or(0);

    // At most 1-2 more messages could sneak through before terminate
    assert!(
        ac - c <= 2,
        "expected no new ticks after terminate, but count grew from {c} to {ac}"
    );
}

// ─── Self.close() from within worker ────────────────────────────────

#[tokio::test]
async fn constructor_worker_self_close() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var received = null;
            var w = new Worker("postMessage('before-close'); close();");
            w.onmessage = function(e) { received = e.data; };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); received");
        if result != "null" {
            assert_eq!(result, "before-close");
            return;
        }
    }
    panic!("timed out waiting for close message");
}

// ─── MessageEvent shape ─────────────────────────────────────────────

#[tokio::test]
async fn constructor_message_event_has_type() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            var evtType = null;
            var w = new Worker("postMessage('check')");
            w.onmessage = function(e) { evtType = e.type; };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); evtType");
        if result != "null" {
            assert_eq!(result, "message");
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for event type");
}

#[tokio::test]
async fn constructor_message_event_uses_messageevent_constructor_when_available() {
    ensure_v8();
    let (mut isolate, ctx) = setup_worker_context();

    eval_ok(
        &mut isolate,
        &ctx,
        r#"
            globalThis.MessageEvent = function(type, init = {}) {
                this.type = type;
                this.data = init.data;
            };
            MessageEvent.prototype.constructor = MessageEvent;
            Object.defineProperty(MessageEvent.prototype, Symbol.toStringTag, {
                value: "MessageEvent",
                configurable: true
            });

            var evtInfo = null;
            var w = new Worker("postMessage('shape')");
            w.onmessage = function(e) {
                evtInfo = JSON.stringify([
                    e instanceof MessageEvent,
                    Object.prototype.toString.call(e),
                    e.type,
                    e.data
                ]);
            };
            "#,
    );

    for _ in 0..50 {
        sleep(Duration::from_millis(20)).await;
        let result = eval_ok(&mut isolate, &ctx, "__drainWorkerMessages(w); evtInfo");
        if result != "null" {
            assert_eq!(
                result,
                r#"[true,"[object MessageEvent]","message","shape"]"#
            );
            eval_ok(&mut isolate, &ctx, "w.terminate()");
            return;
        }
    }
    panic!("timed out waiting for MessageEvent constructor path");
}
