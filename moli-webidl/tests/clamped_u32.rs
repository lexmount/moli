use moli_webidl::{WebIdlArgs, WebIdlDictionary};
#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.convert")]
struct RequiredArgs {
    #[webidl(required, converter = "clamped_unsigned_long")]
    value: u32,
}
#[derive(WebIdlDictionary)]
#[webidl(prefix = "TestOptions")]
struct TestOptions {
    #[webidl(default = 17, converter = "clamped_unsigned_long")]
    value: u32,
}
#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.dictionary")]
struct DictionaryArgs {
    #[webidl(dictionary)]
    options: TestOptions,
}
fn required<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<RequiredArgs>(scope, &args) {
        rv.set(v8::Number::new(scope, parsed.value as f64).into());
    }
}
fn dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<DictionaryArgs>(scope, &args) {
        rv.set(v8::Number::new(scope, parsed.options.value as f64).into());
    }
}
#[test]
fn numeric_conversion_preserves_boundaries_defaults_and_callee_realm_errors() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let callee = v8::Context::new(scope, Default::default());
    let values = {
        let scope = &mut v8::ContextScope::new(scope, callee);
        let required = v8::Function::new(scope, required).unwrap();
        let dictionary = v8::Function::new(scope, dictionary).unwrap();
        let key = v8::String::new(scope, "TypeError").unwrap();
        let error = callee.global(scope).get(scope, key.into()).unwrap();
        [
            ("required", required.into()),
            ("dictionary", dictionary.into()),
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
    let source=v8::String::new(scope,r#"(() => {
        const assert=(value,message)=>{if(!value)throw Error(message);};
        for(const convert of [required,value=>dictionary({value})]) {
            for(const [value,expected] of [[NaN,0],[Infinity,4294967295],[-Infinity,0],[-1,0],[0,0],[-0,0],[0.5,0],[1.5,2],[2.5,2],[3.5,4],[4294967294.5,4294967294],[4294967295.5,4294967295],[4294967296,4294967295],['4.5',4],[null,0],[true,1]])assert(Object.is(convert(value),expected),'boundary '+String(value));
            for(const value of [Symbol('number'),1n]) {let error;try{convert(value);}catch(caught){error=caught;}assert(error instanceof CalleeTypeError&&!(error instanceof TypeError),'callee TypeError');}
            let reads=0;assert(convert({[Symbol.toPrimitive](hint){reads++;assert(hint==='number','numeric hint');return 4;}})===4&&reads===1,'one conversion');
            const sentinel={};let error;try{convert({valueOf(){throw sentinel;}});}catch(caught){error=caught;}assert(error===sentinel,'exception identity');
        }
        assert(dictionary()===17&&dictionary(null)===17&&dictionary({})===17,'dictionary defaults');
        const sentinel={};let error;try{dictionary({get value(){throw sentinel;}});}catch(caught){error=caught;}assert(error===sentinel,'getter exception');
        try{required();}catch(caught){error=caught;}assert(error instanceof CalleeTypeError,'required argument');
        return true;
    })()"#).unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    assert!(
        script
            .run(scope)
            .expect("numeric conversion regression")
            .is_true()
    );
}
