use moli_webidl::{Context, WebIdlArgs, WebIdlDictionary};

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.clamp")]
struct Args {
    #[webidl(required, converter = "clamped_unsigned_long")]
    value: u32,
}

#[derive(WebIdlDictionary)]
#[webidl(prefix = "Test.ClampInit")]
struct Init {
    #[webidl(sequence, converter = "clamped_unsigned_long", default = Vec::new())]
    codes: Vec<u32>,
    #[webidl(converter = "clamped_unsigned_long", default = 2)]
    fallback: u32,
    #[webidl(converter = "clamped_unsigned_long", nullable)]
    value: Option<u32>,
}

fn argument<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<Args>(scope, &args) {
        rv.set(v8::Number::new(scope, f64::from(parsed.value)).into());
    }
}

fn dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let parsed = match moli_webidl::convert::<moli_webidl::Dictionary<Init>>(
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
    let codes: Vec<_> = parsed
        .codes
        .into_iter()
        .map(|value| v8::Number::new(scope, f64::from(value)).into())
        .collect();
    let codes = v8::Array::new_with_elements(scope, &codes);
    let fallback = v8::Number::new(scope, f64::from(parsed.fallback));
    let value = parsed
        .value
        .map(|value| v8::Number::new(scope, f64::from(value)).into())
        .unwrap_or_else(|| v8::null(scope).into());
    rv.set(v8::Array::new_with_elements(scope, &[codes.into(), fallback.into(), value]).into());
}

#[test]
fn clamped_unsigned_long_preserves_numeric_conversion_and_derive_policies() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let callee = v8::Context::new(scope, Default::default());
    let values = {
        let scope = &mut v8::ContextScope::new(scope, callee);
        let convert = v8::Function::new(scope, argument).unwrap();
        let dictionary = v8::Function::new(scope, dictionary).unwrap();
        let key = v8::String::new(scope, "TypeError").unwrap();
        let error = callee.global(scope).get(scope, key.into()).unwrap();
        [
            ("clamp", convert.into()),
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
    let source = v8::String::new(scope, r#"
      (() => {
        const assert=(ok, message)=>{if(!ok)throw Error(message);};
        const matrix=[[undefined,0],[null,0],[NaN,0],[Infinity,4294967295],[-Infinity,0],[-1,0],[-0,0],
          [0,0],[0.5,0],[1.5,2],[2.5,2],[3.5,4],[65536,65536],[4294967294.5,4294967294],
          [4294967295,4294967295],[4294967296,4294967295],[Number.MAX_VALUE,4294967295],['3.5',4],['x',0],[true,1]];
        for(const [value,expected]of matrix)assert(Object.is(clamp(value),expected),'clamp '+value);
        for(const value of [1n,Symbol()]){let error;try{clamp(value);}catch(caught){error=caught;}
          assert(error instanceof CalleeTypeError&&!(error instanceof TypeError),'callee realm');}
        let calls=0;
        assert(clamp({[Symbol.toPrimitive](hint){calls++;assert(hint==='number','hint');return 2.5;}})===2&&calls===1,'one conversion');
        const sentinel={};let error;try{clamp({valueOf(){throw sentinel;}});}catch(caught){error=caught;}assert(error===sentinel,'exception');
        try{clamp();}catch(caught){error=caught;}assert(error instanceof CalleeTypeError,'arity');
        assert(JSON.stringify(dictionary({}))==='[[],2,null]','defaults');
        assert(JSON.stringify(dictionary({codes:new Set([1.5,-1,Infinity]),fallback:3.5,value:null}))==='[[2,0,4294967295],4,null]','sequence and nullable');
        assert(JSON.stringify(dictionary({fallback:null,value:NaN}))==='[[],0,0]','null default distinction');
        const reads=[];const init=new Proxy({}, {get(target,name){reads.push(name);return name==='codes'?[0.5,2.5]:undefined;}});
        assert(JSON.stringify(dictionary(init))==='[[0,2],2,null]'&&JSON.stringify(reads)==='["codes","fallback","value"]','dictionary order');
        return true;
      })()
    "#).unwrap();
    assert!(
        v8::Script::compile(scope, source, None)
            .unwrap()
            .run(scope)
            .expect("[Clamp] unsigned long conversion matrix")
            .is_true()
    );
}
