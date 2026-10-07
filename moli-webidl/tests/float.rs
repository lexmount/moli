use moli_webidl::WebIdlArgs;

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.float")]
struct InferredFloatArgs {
    #[webidl(required)]
    value: f32,
}

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.explicitFloat")]
struct ExplicitFloatArgs {
    #[webidl(required, converter = "float")]
    value: f32,
}

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.unrestrictedFloat")]
struct UnrestrictedFloatArgs {
    #[webidl(required, converter = "unrestricted_float")]
    value: f32,
}

fn unrestricted_float<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<UnrestrictedFloatArgs>(scope, &args) {
        rv.set(v8::Number::new(scope, f64::from(parsed.value)).into());
    }
}

#[test]
fn unrestricted_float_accepts_nonfinite_values_but_preserves_number_conversion_errors() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let callee = v8::Context::new(scope, Default::default());
    let (function, type_error) = {
        let scope = &mut v8::ContextScope::new(scope, callee);
        let function = v8::Function::new(scope, unrestricted_float).unwrap();
        let key = v8::String::new(scope, "TypeError").unwrap();
        (
            function,
            callee.global(scope).get(scope, key.into()).unwrap(),
        )
    };
    let caller = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, caller);
    for (name, value) in [
        ("convert", function.into()),
        ("CalleeTypeError", type_error),
    ] {
        let key = v8::String::new(scope, name).unwrap();
        assert_eq!(
            caller.global(scope).set(scope, key.into(), value),
            Some(true)
        );
    }
    let source = v8::String::new(scope, r#"(() => {
        const assert=(ok,message)=>{if(!ok)throw Error(message);};
        for(const value of [0,-0,1/3,16777217,2**-149,2**-150,-(2**-150),
            NaN,Infinity,-Infinity,3.5e38,1e300,undefined,'1.25',true,null]) {
            assert(Object.is(convert(value),Math.fround(Number(value))),'binary32 rounding');
        }
        for(const value of [Symbol(),1n]) {
            let error;try{convert(value);}catch(e){error=e;}
            assert(error instanceof CalleeTypeError && !(error instanceof TypeError),'callee TypeError');
        }
        let error;try{convert();}catch(e){error=e;}
        assert(error instanceof CalleeTypeError,'required argument');
        let reads=0;assert(convert({valueOf(){reads++;return 1/3;}})===Math.fround(1/3)&&reads===1,'one conversion');
        const sentinel={};try{convert({valueOf(){throw sentinel;}});}catch(e){error=e;}
        assert(error===sentinel,'original exception');
        return true;
    })()"#).unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    assert!(
        script
            .run(scope)
            .expect("unrestricted float conversion matrix")
            .is_true()
    );
}

fn inferred_float<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<InferredFloatArgs>(scope, &args) {
        rv.set(v8::Number::new(scope, f64::from(parsed.value)).into());
    }
}

fn explicit_float<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<ExplicitFloatArgs>(scope, &args) {
        rv.set(v8::Number::new(scope, f64::from(parsed.value)).into());
    }
}

#[test]
fn restricted_float_rounds_and_preserves_conversion_errors_in_the_callee_realm() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let callee = v8::Context::new(scope, Default::default());
    let values = {
        let scope = &mut v8::ContextScope::new(scope, callee);
        let inferred = v8::Function::new(scope, inferred_float).unwrap();
        let explicit = v8::Function::new(scope, explicit_float).unwrap();
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
          for (const convert of [inferred, explicit]) {
            for (const value of [0, -0, 1 / 3, 16777217, 2 ** -149, 2 ** -150,
                -(2 ** -150), 3.4028234663852886e38, '1.25', true, null]) {
              assert(Object.is(convert(value), Math.fround(Number(value))), 'rounding: ' + value);
            }
            for (const value of [NaN, Infinity, -Infinity, 3.5e38, 1e300, undefined,
                Symbol('number'), 1n]) {
              let error;
              try { convert(value); } catch (caught) { error = caught; }
              assert(error instanceof CalleeTypeError && !(error instanceof TypeError),
                'restricted float rejects nonfinite, overflow and invalid ToNumber in callee realm');
            }
            let error;
            try { convert(); } catch (caught) { error = caught; }
            assert(error instanceof CalleeTypeError, 'required argument');
            let reads = 0;
            const value = { valueOf() { reads++; return 1 / 3; } };
            assert(convert(value) === Math.fround(1 / 3) && reads === 1, 'one numeric conversion');
            const sentinel = {};
            try { convert({ valueOf() { throw sentinel; } }); } catch (caught) { error = caught; }
            assert(error === sentinel, 'original ToNumber exception is preserved');
          }
          return true;
        })()
    "#).unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    assert!(
        script
            .run(scope)
            .expect("restricted float conversion matrix")
            .is_true()
    );
}
