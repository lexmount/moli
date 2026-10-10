use moli_webidl::{WebIdlArgs, WebIdlDictionary};

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.octets")]
struct OctetArgs {
    #[webidl(required)]
    value: u8,
    #[webidl(sequence, default = Vec::new())]
    bytes: Vec<u8>,
    #[webidl(converter = "octet", default = 7)]
    explicit: u8,
}

// This dictionary is declared in IDL member order; argument order is separate.
#[derive(WebIdlDictionary)]
#[webidl(prefix = "Test.octets")]
struct OctetDictionary {
    #[webidl(sequence, default = Vec::new())]
    bytes: Vec<u8>,
    #[webidl(converter = "octet", default = 7)]
    explicit: u8,
    #[webidl(required)]
    value: u8,
}

fn octets<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<OctetArgs>(scope, &args) {
        let values: Vec<v8::Local<v8::Value>> = std::iter::once(parsed.value)
            .chain(parsed.bytes)
            .chain(std::iter::once(parsed.explicit))
            .map(|value| v8::Integer::new(scope, i32::from(value)).into())
            .collect();
        rv.set(v8::Array::new_with_elements(scope, &values).into());
    }
}

fn dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::convert::<moli_webidl::Dictionary<OctetDictionary>>(
        scope,
        args.get(0),
        moli_webidl::Context::argument("Test.octets", 1),
    ) {
        Ok(parsed) => {
            assert!(parsed.0.bytes.is_empty());
            rv.set_int32(i32::from(parsed.0.value) + i32::from(parsed.0.explicit));
        }
        Err(error) => moli_webidl::throw_error(scope, &error),
    }
}

#[test]
fn octet_conversion_wraps_and_preserves_sequence_order_and_callee_errors() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let callee = v8::Context::new(scope, Default::default());
    let values = {
        let scope = &mut v8::ContextScope::new(scope, callee);
        let scalar = v8::Function::new(scope, octets).unwrap();
        let dictionary = v8::Function::new(scope, dictionary).unwrap();
        let key = v8::String::new(scope, "TypeError").unwrap();
        let error = callee.global(scope).get(scope, key.into()).unwrap();
        [
            ("octets", scalar.into()),
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
    let source = v8::String::new(scope, r#"(() => {
      const assert = (ok, name) => {if (!ok) throw Error(name)};
      for (const [value,expected] of [[0,0],[-0,0],[3.9,3],[-3.9,253],[255,255],[256,0],[257,1],[-257,255],[65535,255],[4294967297,1],[Number.MAX_SAFE_INTEGER,255],[NaN,0],[Infinity,0],[-Infinity,0],[undefined,0],[null,0],[true,1],['511',255]]) {
        const result = octets(value,[value],value);
        assert(result.join() === [expected,expected,value === undefined ? 7 : expected].join(), 'modulo-256 conversion and optional parameter default');
      }
      for (const value of [Symbol(),1n]) {
        let error; try {octets(value)} catch(caught) {error=caught}
        assert(error instanceof CalleeTypeError && !(error instanceof TypeError), 'callee numeric error');
      }
      const log=[], marker={};
      const first={valueOf(){log.push('first'); return -1}}, last={valueOf(){log.push('last'); return 256}};
      const iterable={get [Symbol.iterator](){log.push('iterator'); return function*(){log.push('next'); yield {valueOf(){log.push('item'); return 257}}; log.push('done')}}};
      assert(octets(first,iterable,last).join()==='255,1,0' && log.join()==='first,iterator,next,item,done,last', 'sequence converts each item before next argument');
      log.length=0; let error;
      try {octets(0,[{valueOf(){throw marker}}],last)} catch(caught){error=caught}
      assert(error===marker && log.length===0,'conversion exception stops later arguments');
      log.length=0;
      const input={get bytes(){log.push('bytes'); return []},get explicit(){log.push('explicit'); return -1},get value(){log.push('value'); return 257}};
      assert(dictionary(input)===256 && log.join()==='bytes,explicit,value','dictionary order and octet conversion');
      try {octets()} catch(caught){error=caught}
      assert(error instanceof CalleeTypeError, 'required scalar');
      assert(octets(1).join()==='1,7', 'optional sequence and octet default');
      return true;
    })()"#).unwrap();
    let tc = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = &mut tc.init();
    let result = v8::Script::compile(scope, source, None).unwrap().run(scope);
    if let Some(exception) = scope.exception() {
        panic!(
            "octet conversion matrix: {}",
            exception.to_rust_string_lossy(scope)
        );
    }
    assert!(result.is_some_and(|result| result.is_true()));
}
