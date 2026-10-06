use moli_webidl::{Context, Uint8ArrayOptions, WebIdlArgs, WebIdlDictionary};

#[test]
fn uint8_array_conversion_classifies_native_buffers_and_preserves_identity() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let other = v8::Context::new(scope, Default::default());
    for owner in [context, other] {
        for shared in [false, true] {
            for resizable in [false, true] {
                for len in [0, 2] {
                    for detached in [false, true] {
                        if shared && detached {
                            continue;
                        }
                        let view = {
                            let scope = &mut v8::ContextScope::new(scope, owner);
                            let constructor = if shared {
                                "SharedArrayBuffer"
                            } else {
                                "ArrayBuffer"
                            };
                            let options = if resizable {
                                ", {maxByteLength: 4}"
                            } else {
                                ""
                            };
                            let detach = if detached { "buffer.transfer();" } else { "" };
                            let source = format!(
                                "(() => {{ const buffer = new {constructor}({len}{options}); const view = new Uint8Array(buffer); {detach} Object.defineProperty(view, 'buffer', {{get() {{throw 42;}}}}); Object.defineProperty(buffer, 'resizable', {{get() {{throw 43;}}}}); return view; }})()"
                            );
                            let source = v8::String::new(scope, &source).unwrap();
                            let value = v8::Script::compile(scope, source, None)
                                .unwrap()
                                .run(scope)
                                .unwrap();
                            v8::Local::<v8::Uint8Array>::try_from(value).unwrap()
                        };
                        let scope = &mut v8::ContextScope::new(scope, context);
                        for allow_shared in [false, true] {
                            let converted =
                                moli_webidl::convert_with_options::<v8::Local<v8::Uint8Array>>(
                                    scope,
                                    view.into(),
                                    Context::argument("Test.view", 1),
                                    &Uint8ArrayOptions { allow_shared },
                                );
                            let accepted = !resizable && (!shared || allow_shared);
                            assert_eq!(
                                converted.is_ok(),
                                accepted,
                                "shared={shared}, resizable={resizable}, len={len}, detached={detached}, allow_shared={allow_shared}"
                            );
                            if accepted {
                                assert!(converted.unwrap().strict_equals(view.into()));
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.views")]
struct Views<'s> {
    #[webidl(required)]
    ordinary: v8::Local<'s, v8::Uint8Array>,
    #[webidl(required, allow_shared)]
    shared: v8::Local<'s, v8::Uint8Array>,
}

#[derive(WebIdlDictionary)]
#[webidl(prefix = "Test.ViewInit")]
struct ViewInit<'s> {
    ordinary: Option<v8::Local<'s, v8::Uint8Array>>,
    #[webidl(allow_shared)]
    shared: Option<v8::Local<'s, v8::Uint8Array>>,
    #[webidl(sequence, allow_shared)]
    views: Option<Vec<v8::Local<'s, v8::Uint8Array>>>,
}

fn callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = moli_webidl::parse_args::<Views>(scope, &args) else {
        return;
    };
    let result = v8::Array::new(scope, 2);
    result.set_index(scope, 0, parsed.ordinary.into()).unwrap();
    result.set_index(scope, 1, parsed.shared.into()).unwrap();
    rv.set(result.into());
}

fn dictionary_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let parsed = match moli_webidl::convert::<moli_webidl::Dictionary<ViewInit>>(
        scope,
        args.get(0),
        Context::argument("Test.dictionary", 1),
    ) {
        Ok(parsed) => parsed.0,
        Err(error) => {
            moli_webidl::throw_error(scope, &error);
            return;
        }
    };
    let result = v8::Array::new(scope, 3);
    result
        .set_index(
            scope,
            0,
            parsed
                .ordinary
                .map(Into::into)
                .unwrap_or_else(|| v8::null(scope).into()),
        )
        .unwrap();
    result
        .set_index(
            scope,
            1,
            parsed
                .shared
                .map(Into::into)
                .unwrap_or_else(|| v8::null(scope).into()),
        )
        .unwrap();
    let views = v8::Array::new(scope, 0);
    for (index, value) in parsed.views.unwrap_or_default().into_iter().enumerate() {
        views.set_index(scope, index as u32, value.into()).unwrap();
    }
    result.set_index(scope, 2, views.into()).unwrap();
    rv.set(result.into());
}

#[test]
fn uint8_array_derives_keep_allow_shared_explicit_and_reject_author_proxies() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let (parse, dictionary, error) = {
        let scope = &mut v8::ContextScope::new(scope, context);
        let parse = v8::Function::new(scope, callback).unwrap();
        let dictionary = v8::Function::new(scope, dictionary_callback).unwrap();
        let name = v8::String::new(scope, "TypeError").unwrap();
        let error = context.global(scope).get(scope, name.into()).unwrap();
        (parse, dictionary, error)
    };
    let caller = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, caller);
    for (name, value) in [
        ("parse", parse.into()),
        ("dictionary", dictionary.into()),
        ("CalleeTypeError", error),
    ] {
        let name = v8::String::new(scope, name).unwrap();
        caller.global(scope).set(scope, name.into(), value).unwrap();
    }
    let source = v8::String::new(scope, r#"(() => {
        const assert = ok => { if (!ok) throw Error('Uint8Array conversion'); };
        const throws = run => { let error; try {run();} catch (caught) {error = caught;} assert(error instanceof CalleeTypeError && !(error instanceof TypeError)); };
        const ordinary = new Uint8Array(2), shared = new Uint8Array(new SharedArrayBuffer(2));
        const result = parse(ordinary, shared); assert(result[0] === ordinary && result[1] === shared);
        throws(() => parse(shared, ordinary));
        const init = dictionary({ordinary, shared, views: [ordinary, shared]});
        assert(init[0] === ordinary && init[1] === shared && init[2][0] === ordinary && init[2][1] === shared);
        throws(() => dictionary({ordinary: shared}));
        assert(dictionary({ordinary: undefined, shared: undefined})[0] === null);
        throws(() => dictionary({ordinary: null}));
        let traps = 0;
        const proxy = new Proxy(ordinary, {get() {traps++; throw 42;}, getPrototypeOf() {traps++; throw 43;}});
        const revoked = Proxy.revocable(ordinary, {}); revoked.revoke();
        for (const bad of [{}, [], new ArrayBuffer(2), new DataView(new ArrayBuffer(2)), new Int8Array(2), new Uint8ClampedArray(2), Object.create(ordinary), Object.create(Uint8Array.prototype), proxy, revoked.proxy]) {
            throws(() => parse(bad, ordinary)); throws(() => parse(ordinary, bad));
        }
        assert(traps === 0);
        return true;
    })()"#).unwrap();
    assert!(
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .unwrap()
            .is_true()
    );
}
