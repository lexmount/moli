use moli_webidl::WebIdlArgs;

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.short")]
struct InferredShortArgs {
    #[webidl(required)]
    value: i16,
}

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.explicitShort")]
struct ExplicitShortArgs {
    #[webidl(required, converter = "short")]
    value: i16,
}

fn inferred_short<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<InferredShortArgs>(scope, &args) {
        rv.set(v8::Number::new(scope, f64::from(parsed.value)).into());
    }
}

fn explicit_short<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<ExplicitShortArgs>(scope, &args) {
        rv.set(v8::Number::new(scope, f64::from(parsed.value)).into());
    }
}

#[test]
fn signed_short_wraps_and_preserves_conversion_errors_in_the_callee_realm() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let callee = v8::Context::new(scope, Default::default());
    let values = {
        let scope = &mut v8::ContextScope::new(scope, callee);
        let inferred = v8::Function::new(scope, inferred_short).unwrap();
        let explicit = v8::Function::new(scope, explicit_short).unwrap();
        let key = v8::String::new(scope, "TypeError").unwrap();
        let error = callee.global(scope).get(scope, key.into()).unwrap();
        [
            ("inferred", inferred.into()),
            ("explicit", explicit.into()),
            ("CalleeTypeError", error),
        ]
    };
    let caller = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, caller);
    for (name, value) in values {
        let key = v8::String::new(scope, name).unwrap();
        assert_eq!(
            caller.global(scope).set(scope, key.into(), value),
            Some(true)
        );
    }
    let source = v8::String::new(scope, r#"
        (() => {
          const assert = (ok, message) => { if (!ok) throw Error(message); };
          const values = [[0,0],[-0,0],[3.9,3],[-3.9,-3],[32767,32767],
            [32768,-32768],[65535,-1],[65536,0],[65537,1],[-65537,-1],
            [4294967295,-1],[4294967297,1],[NaN,0],[Infinity,0],[-Infinity,0],
            [undefined,0],[null,0],[true,1],['65535',-1]];
          for (const convert of [inferred, explicit]) {
            for (const [value, expected] of values) assert(Object.is(convert(value),expected),'modulo conversion '+value);
            for (const value of [Symbol('number'), 1n]) {
              let error;try { convert(value); } catch(caught) { error=caught; }
              assert(error instanceof CalleeTypeError && !(error instanceof TypeError),'callee realm TypeError');
            }
            let reads=0;const value={[Symbol.toPrimitive](hint){reads++;assert(hint==='number','numeric hint');return 65535;}};
            assert(convert(value)===-1&&reads===1,'one observable conversion');
            const sentinel={};let error;try{convert({valueOf(){throw sentinel;}});}catch(caught){error=caught;}assert(error===sentinel,'exception identity');
            try { convert(); } catch(caught) { error=caught; }assert(error instanceof CalleeTypeError,'required argument');
          }
          return true;
        })()
    "#).unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    assert!(
        script
            .run(scope)
            .expect("signed short conversion matrix")
            .is_true()
    );
}
