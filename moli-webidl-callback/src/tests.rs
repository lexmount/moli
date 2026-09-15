use super::*;

fn attempt_checkpoint(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    scope.perform_microtask_checkpoint();
}

#[test]
fn callback_scope_defers_checkpoints_through_lookup_call_and_return_conversion() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let checkpoint = v8::Function::new(scope, attempt_checkpoint).expect("checkpoint function");
    let key = v8::String::new(scope, "checkpoint").expect("checkpoint key");
    assert_eq!(
        context
            .global(scope)
            .set(scope, key.into(), checkpoint.into()),
        Some(true)
    );
    let callback = v8::Local::<v8::Object>::try_from(eval(
        scope,
        r#"
        globalThis.log = [];
        ({ get handleEvent() {
          log.push("lookup");
          Promise.resolve().then(() => log.push("microtask"));
          checkpoint();
          return function () {
            log.push("call");
            checkpoint();
            return { valueOf() { log.push("convert"); checkpoint(); return 7; } };
          };
        } })
    "#,
    ))
    .expect("callback interface");
    let receiver = v8::undefined(scope).into();
    let result = with_webidl_callback_contexts(scope, context, context, |scope| {
        invoke_webidl_callback(
            scope,
            WebIdlCallbackInvocation::new(callback, receiver, false, "handleEvent", &[]),
            |scope, function, receiver, arguments| {
                function
                    .call(scope, receiver, arguments)
                    .and_then(|result| result.integer_value(scope))
                    .ok_or("callback or conversion failed")
            },
            |_, _| "callback resolution failed",
        )
    })
    .expect("callback should return");
    assert_eq!(result, 7);
    assert_eq!(
        eval(scope, "log.join(',')").to_rust_string_lossy(scope),
        "lookup,call,convert"
    );
    scope.perform_microtask_checkpoint();
    assert_eq!(
        eval(scope, "log.join(',')").to_rust_string_lossy(scope),
        "lookup,call,convert,microtask"
    );
}

#[test]
fn callback_scope_preserves_auto_policy_checkpoint_timing() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    assert_eq!(isolate.get_microtasks_policy(), v8::MicrotasksPolicy::Auto);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let callback = v8::Local::<v8::Object>::try_from(eval(
        scope,
        r#"
        globalThis.log = [];
        (function () {
          log.push("call");
          Promise.resolve().then(() => log.push("microtask"));
          return { valueOf() { log.push("convert"); return 7; } };
        })
    "#,
    ))
    .expect("callback function");
    let callback = WebIdlCallbackFunction::try_new(scope, callback, context, context)
        .expect("callable")
        .prepare(scope);
    let receiver = v8::undefined(scope).into();
    invoke_webidl_callback_function(
        scope,
        &callback,
        receiver,
        &[],
        |scope, function, receiver, args| {
            function
                .call(scope, receiver, args)
                .and_then(|result| result.integer_value(scope))
                .ok_or("callback or conversion failed")
        },
    )
    .expect("callback should return");
    assert_eq!(
        eval(scope, "log.join(',')").to_rust_string_lossy(scope),
        "call,microtask,convert"
    );
}

#[test]
fn callback_scope_cleans_up_after_throwing_operation_lookup() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Explicit);
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let caller = v8::Context::new(scope, Default::default());
    let relevant = v8::Context::new(scope, Default::default());
    let incumbent = v8::Context::new(scope, Default::default());
    let callback = {
        let scope = &mut v8::ContextScope::new(scope, relevant);
        v8::Local::<v8::Object>::try_from(eval(
            scope,
            "({ get handleEvent() { throw Error('lookup'); } })",
        ))
        .expect("callback interface")
    };
    let scope = &mut v8::ContextScope::new(scope, caller);
    let result = with_webidl_callback_contexts(scope, relevant, incumbent, |scope| {
        let receiver = v8::undefined(scope).into();
        invoke_for_test(
            scope,
            WebIdlCallbackInvocation::new(callback, receiver, false, "handleEvent", &[]),
        )
    });
    assert!(result.expect_err("lookup must throw").contains("lookup"));
    assert_eq!(scope.get_current_context(), caller);
    assert_eq!(
        relevant
            .get_microtask_queue()
            .unwrap()
            .get_microtasks_scope_depth(),
        0
    );
    assert_ne!(scope.get_incumbent_context(), Some(incumbent));
}

#[test]
fn callback_scope_exits_incumbent_before_scoped_policy_checkpoint() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    isolate.set_microtasks_policy(v8::MicrotasksPolicy::Scoped);
    isolate.set_slot(std::cell::Cell::new(None::<bool>));
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let caller = v8::Context::new(scope, Default::default());
    let relevant = v8::Context::new(scope, Default::default());
    let incumbent = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, caller);
    let caller_global = caller.global(scope);
    let microtask = v8::Function::builder(
        |scope: &mut v8::PinScope<'_, '_>,
         args: v8::FunctionCallbackArguments<'_>,
         _rv: v8::ReturnValue<'_, v8::Value>| {
            let restored = scope
                .get_incumbent_context()
                .is_some_and(|context| context.global(scope).strict_equals(args.data()));
            scope
                .get_slot::<std::cell::Cell<Option<bool>>>()
                .unwrap()
                .set(Some(restored));
        },
    )
    .data(caller_global.into())
    .build(scope)
    .expect("microtask");
    let outer_incumbent = std::pin::pin!(v8::BackupIncumbentScope::new(caller));
    let _outer_incumbent = outer_incumbent.init();
    with_webidl_callback_contexts(scope, relevant, incumbent, |scope| {
        assert_eq!(scope.get_incumbent_context(), Some(incumbent));
        scope.enqueue_microtask(microtask);
    });
    assert_eq!(
        scope
            .get_slot::<std::cell::Cell<Option<bool>>>()
            .unwrap()
            .get(),
        Some(true)
    );
    assert_eq!(scope.get_current_context(), caller);
}

fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).expect("test source");
    let script = v8::Script::compile(scope, source, None).expect("compile test source");
    script.run(scope).expect("run test source")
}

fn invoke_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    invocation: WebIdlCallbackInvocation<'s, '_>,
) -> Result<v8::Global<v8::Value>, String> {
    invoke_webidl_callback(
        scope,
        invocation,
        |scope, callback, receiver, arguments| {
            callback
                .call(scope, receiver, arguments)
                .map(|value| v8::Global::new(scope, value))
                .ok_or_else(|| "callback call failed".to_owned())
        },
        |scope, failure| {
            failure
                .exception()
                .and_then(|exception| exception.to_string(scope))
                .map(|message| message.to_rust_string_lossy(scope))
                .unwrap_or_else(|| "callback resolution failed".to_owned())
        },
    )
}

fn invoke_function_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: &PreparedWebIdlCallbackFunction,
    receiver: v8::Local<'s, v8::Value>,
    arguments: &[v8::Local<'s, v8::Value>],
) -> Result<v8::Global<v8::Value>, String> {
    invoke_webidl_callback_function(
        scope,
        callback,
        receiver,
        arguments,
        |scope, callback, receiver, arguments| {
            callback
                .call(scope, receiver, arguments)
                .map(|value| v8::Global::new(scope, value))
                .ok_or_else(|| "callback call failed".to_owned())
        },
    )
}

#[test]
fn rooted_callback_preparation_preserves_callback_and_contexts() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let relevant_context = v8::Context::new(scope, Default::default());
    let incumbent_context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, relevant_context);
    let callback =
        v8::Local::<v8::Object>::try_from(eval(scope, "(function () {})")).expect("callback");

    let rooted = WebIdlCallbackInterface::new(scope, callback, relevant_context, incumbent_context);
    let prepared = rooted.prepare(scope);

    assert!(rooted.matches(scope, callback));
    assert!(prepared.callback(scope).strict_equals(callback.into()));
    assert_eq!(prepared.relevant_context(scope), relevant_context);
    assert_eq!(prepared.incumbent_context(scope), incumbent_context);
    assert!(prepared.callable_at_conversion());
}

#[test]
fn callback_interface_reads_replacement_operation_and_uses_original_receiver() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let callback = v8::Local::<v8::Object>::try_from(eval(
        scope,
        "({ handleEvent() { throw new Error('old operation'); } })",
    ))
    .expect("callback-interface object");
    let rooted = WebIdlCallbackInterface::new(scope, callback, context, context);
    let replacement = eval(
        scope,
        "(function (value) { this.seen = value; return 42; })",
    );
    let handle_event = v8::String::new(scope, "handleEvent").expect("handleEvent key");
    assert_eq!(
        callback.set(scope, handle_event.into(), replacement),
        Some(true)
    );
    let prepared = rooted.prepare(scope);
    let argument: v8::Local<'_, v8::Value> = v8::Integer::new(scope, 7).into();
    let arguments = [argument];
    let relevant_context = prepared.relevant_context(scope);
    let incumbent_context = prepared.incumbent_context(scope);

    let result =
        with_webidl_callback_contexts(scope, relevant_context, incumbent_context, |scope| {
            let callback = prepared.callback(scope);
            let receiver = v8::undefined(scope).into();
            invoke_for_test(
                scope,
                WebIdlCallbackInvocation::new(
                    callback,
                    receiver,
                    prepared.callable_at_conversion(),
                    "handleEvent",
                    &arguments,
                ),
            )
        })
        .expect("replacement handleEvent should run");

    assert_eq!(
        v8::Local::new(scope, &result).integer_value(scope),
        Some(42)
    );
    let seen = callback
        .get(
            scope,
            v8::String::new(scope, "seen").expect("seen key").into(),
        )
        .expect("seen value");
    assert_eq!(seen.integer_value(scope), Some(7));
}

#[test]
fn callback_interface_invocation_enters_relevant_and_incumbent_contexts() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let caller_context = v8::Context::new(scope, Default::default());
    let relevant_context = v8::Context::new(scope, Default::default());
    let incumbent_context = v8::Context::new(scope, Default::default());
    let callback = {
        let scope = &mut v8::ContextScope::new(scope, relevant_context);
        let callback =
            v8::Local::<v8::Object>::try_from(eval(scope, "({ acceptNode() { return 1; } })"))
                .expect("callback-interface object");
        v8::Global::new(scope, callback)
    };
    let scope = &mut v8::ContextScope::new(scope, caller_context);
    let callback = v8::Local::new(scope, &callback);
    let rooted = WebIdlCallbackInterface::new(scope, callback, relevant_context, incumbent_context);
    let prepared = rooted.prepare(scope);
    let relevant_context = prepared.relevant_context(scope);
    let incumbent_context = prepared.incumbent_context(scope);

    let result =
        with_webidl_callback_contexts(scope, relevant_context, incumbent_context, |scope| {
            assert_eq!(scope.get_current_context(), relevant_context);
            assert_eq!(scope.get_incumbent_context(), Some(incumbent_context));
            let callback = prepared.callback(scope);
            let receiver = v8::undefined(scope).into();
            invoke_for_test(
                scope,
                WebIdlCallbackInvocation::new(
                    callback,
                    receiver,
                    prepared.callable_at_conversion(),
                    "acceptNode",
                    &[],
                ),
            )
        })
        .expect("callback-interface invocation");

    assert_eq!(v8::Local::new(scope, &result).integer_value(scope), Some(1));
}

#[test]
fn callback_function_uses_supplied_receiver() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let callback =
        v8::Local::<v8::Object>::try_from(eval(scope, "(function () { this.called = true; })"))
            .expect("function callback");
    let target = v8::Object::new(scope);
    let arguments = [];

    invoke_for_test(
        scope,
        WebIdlCallbackInvocation::new(callback, target.into(), true, "handleEvent", &arguments),
    )
    .expect("function callback should run");

    let called = target
        .get(
            scope,
            v8::String::new(scope, "called").expect("called key").into(),
        )
        .expect("called value");
    assert!(called.boolean_value(scope));
}

#[test]
fn callback_interface_without_callable_operation_reports_type_error() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let callback = v8::Object::new(scope);
    let arguments = [];
    let receiver = v8::undefined(scope).into();

    let error = invoke_for_test(
        scope,
        WebIdlCallbackInvocation::new(callback, receiver, false, "handleEvent", &arguments),
    )
    .expect_err("missing handleEvent must fail");

    assert!(
        error.contains("no callable handleEvent property"),
        "{error}"
    );
}

#[test]
fn callback_function_preparation_enters_captured_contexts_and_uses_receiver() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let caller_context = v8::Context::new(scope, Default::default());
    let relevant_context = v8::Context::new(scope, Default::default());
    let incumbent_context = v8::Context::new(scope, Default::default());
    let callback = {
        let scope = &mut v8::ContextScope::new(scope, relevant_context);
        let _ = eval(scope, "globalThis.realmValue = 40");
        let callback = v8::Local::<v8::Object>::try_from(eval(
            scope,
            "(function (argument) { return this.receiverValue + argument + realmValue; })",
        ))
        .expect("callback function");
        v8::Global::new(scope, callback)
    };
    let scope = &mut v8::ContextScope::new(scope, caller_context);
    let callback = v8::Local::new(scope, &callback);
    let rooted =
        WebIdlCallbackFunction::try_new(scope, callback, relevant_context, incumbent_context)
            .expect("callable callback");
    let prepared = rooted.prepare(scope);
    assert_eq!(prepared.relevant_context(scope), relevant_context);
    assert_eq!(prepared.incumbent_context(scope), incumbent_context);
    let receiver = v8::Object::new(scope);
    let receiver_value = v8::String::new(scope, "receiverValue").expect("receiverValue");
    assert_eq!(
        receiver.set(
            scope,
            receiver_value.into(),
            v8::Integer::new(scope, 1).into()
        ),
        Some(true)
    );
    let arguments = [v8::Integer::new(scope, 1).into()];

    let result = invoke_webidl_callback_function(
        scope,
        &prepared,
        receiver.into(),
        &arguments,
        |scope, callback, receiver, arguments| {
            assert_eq!(scope.get_current_context(), relevant_context);
            callback
                .call(scope, receiver, arguments)
                .map(|value| v8::Global::new(scope, value))
                .ok_or("callback should return")
        },
    )
    .expect("callback function invocation");

    assert_eq!(
        v8::Local::new(scope, &result).integer_value(scope),
        Some(42)
    );
    assert!(rooted.matches(scope, callback));
    assert!(rooted.value(scope).strict_equals(callback.into()));
}

#[test]
fn callback_function_accepts_callable_proxy() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let callback = v8::Local::<v8::Object>::try_from(eval(
        scope,
        "new Proxy(function (value) { return this.base + value; }, {
            apply(target, receiver, arguments) {
                return Reflect.apply(target, receiver, arguments) + 1;
            }
        })",
    ))
    .expect("callable proxy");
    let rooted = WebIdlCallbackFunction::try_new(scope, callback, context, context)
        .expect("callable proxy must convert");
    let prepared = rooted.prepare(scope);
    let receiver = v8::Object::new(scope);
    let base = v8::String::new(scope, "base").expect("base");
    assert_eq!(
        receiver.set(scope, base.into(), v8::Integer::new(scope, 40).into()),
        Some(true)
    );
    let arguments = [v8::Integer::new(scope, 1).into()];

    let result = invoke_function_for_test(scope, &prepared, receiver.into(), &arguments)
        .expect("callable proxy invocation");

    assert_eq!(
        v8::Local::new(scope, &result).integer_value(scope),
        Some(42)
    );
}

#[test]
fn callback_function_rejects_non_callable_object() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let object = v8::Object::new(scope);

    assert!(WebIdlCallbackFunction::try_new(scope, object, context, context).is_none());
}

#[test]
fn revoked_callable_proxy_reaches_host_exception_policy() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let callback = v8::Local::<v8::Object>::try_from(eval(
        scope,
        "globalThis.revocableCallback = Proxy.revocable(function () {}, {});
         revocableCallback.proxy",
    ))
    .expect("revocable callback proxy");
    let rooted = WebIdlCallbackFunction::try_new(scope, callback, context, context)
        .expect("proxy is callable at conversion");
    let prepared = rooted.prepare(scope);
    let _ = eval(scope, "revocableCallback.revoke()");
    let receiver = v8::undefined(scope).into();

    let error = invoke_webidl_callback_function(
        scope,
        &prepared,
        receiver,
        &[],
        |scope, callback, receiver, arguments| {
            let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
            let scope = try_catch.init();
            if callback.call(&scope, receiver, arguments).is_some() {
                return Ok(());
            }
            Err(scope
                .exception()
                .and_then(|exception| exception.to_string(&scope))
                .map(|message| message.to_rust_string_lossy(&scope))
                .unwrap_or_else(|| "host captured callback exception".to_owned()))
        },
    )
    .expect_err("revoked proxy invocation must throw");

    assert!(
        error.contains("revoked") || error.contains("proxy"),
        "unexpected callback exception: {error}"
    );
}
