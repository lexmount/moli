use moli_webidl::{WebIdlArgs, WebIdlDictionary};

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.enforceRangeUnsignedShort")]
struct Args {
    #[webidl(required, converter = "enforce_range_unsigned_short")]
    value: u16,
}

#[derive(WebIdlDictionary)]
#[webidl(prefix = "Options")]
struct Options {
    #[webidl(default = 7, converter = "enforce_range_unsigned_short")]
    value: u16,
}

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.dictionary")]
struct DictionaryArgs {
    #[webidl(dictionary)]
    options: Options,
}

fn convert<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(Args { value }) = moli_webidl::parse_args(scope, &args) {
        rv.set_uint32(u32::from(value));
    }
}

fn dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(DictionaryArgs { options }) = moli_webidl::parse_args(scope, &args) {
        rv.set_uint32(u32::from(options.value));
    }
}

#[test]
fn enforce_range_unsigned_short_truncates_checks_bounds_and_preserves_callee_exceptions() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let callee = v8::Context::new(scope, Default::default());
    let values = {
        let scope = &mut v8::ContextScope::new(scope, callee);
        let convert = v8::Function::new(scope, convert).unwrap();
        let dictionary = v8::Function::new(scope, dictionary).unwrap();
        let key = v8::String::new(scope, "TypeError").unwrap();
        let error = callee.global(scope).get(scope, key.into()).unwrap();
        [
            ("convert", convert.into()),
            ("dictionary", dictionary.into()),
            ("CalleeTypeError", error),
        ]
    };
    let caller = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, caller);
    for (name, value) in values {
        let key = v8::String::new(scope, name).unwrap();
        assert_eq!(
            caller
                .global(scope)
                .create_data_property(scope, key.into(), value),
            Some(true)
        );
    }
    let source = v8::String::new(scope, r#"
      (()=>{
        const assert=(ok,message)=>{if(!ok)throw Error(message)};
        for(const [value,expected] of [[0,0],[-0,0],[-0.9,0],[3.9,3],[65535,65535],[65535.9,65535],['100',100],[null,0],[true,1]]) {
          assert(Object.is(convert(value),expected),'argument truncation');
          assert(Object.is(dictionary({value}),expected),'dictionary truncation');
        }
        assert(dictionary()===7&&dictionary(null)===7&&dictionary({value:undefined})===7,'dictionary defaults');
        for(const value of [-1,-1.1,65536,65536.1,NaN,Infinity,-Infinity,'65536',Symbol(),1n]) {
          for(const body of [()=>convert(value),()=>dictionary({value})]) {
            let error;try{body()}catch(e){error=e}
            assert(error instanceof CalleeTypeError&&!(error instanceof TypeError),'callee range TypeError');
          }
        }
        let reads=0,conversions=0;
        assert(dictionary({get value(){reads++;return {[Symbol.toPrimitive](hint){assert(hint==='number','hint');conversions++;return 17.8}}}})===17,'converted value');
        assert(reads===1&&conversions===1,'observable conversion once');
        const sentinel={};
        for(const body of [()=>convert({valueOf(){throw sentinel}}),()=>dictionary({get value(){throw sentinel}})]) {
          let error;try{body()}catch(e){error=e}assert(error===sentinel,'exception identity');
        }
        return true;
      })()
    "#).unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    assert!(script.run(scope).unwrap().is_true());
}
