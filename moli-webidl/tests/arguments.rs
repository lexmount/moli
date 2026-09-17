use moli_webidl::{Context, DomString, WebIdlArgs, WebIdlError};

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.arguments")]
struct Arguments {
    #[webidl(required)]
    first: String,
    #[webidl(required)]
    second: String,
    #[webidl(required, nullable)]
    nullable: Option<String>,
    #[webidl(default = "default")]
    optional: String,
    #[webidl(required, index = 5, with = custom_string, missing_message = "sixth argument missing")]
    custom: String,
    #[webidl(variadic, index = 6)]
    rest: Vec<String>,
}

fn custom_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<String, WebIdlError> {
    moli_webidl::argument::<DomString>(
        scope,
        args,
        index,
        Context::argument("Test.arguments", index as usize + 1),
    )
    .map(|value| value.0)
}

fn callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = moli_webidl::parse_args::<Arguments>(scope, &args) else {
        return;
    };
    let result = format!(
        "{}|{}|{:?}|{}|{}|{:?}",
        parsed.first, parsed.second, parsed.nullable, parsed.optional, parsed.custom, parsed.rest
    );
    rv.set(v8::String::new(scope, &result).unwrap().into());
}

#[test]
fn required_arity_precedes_scalar_nullable_and_custom_argument_conversion() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(v8::CreateParams::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let callee_context = v8::Context::new(scope, Default::default());
    let (function, type_error) = {
        let scope = &mut v8::ContextScope::new(scope, callee_context);
        let function = v8::Function::new(scope, callback).unwrap();
        let key = v8::String::new(scope, "TypeError").unwrap();
        let type_error = callee_context.global(scope).get(scope, key.into()).unwrap();
        (function, type_error)
    };
    let caller_context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, caller_context);
    let global = caller_context.global(scope);
    for (name, value) in [("parse", function.into()), ("CalleeTypeError", type_error)] {
        let key = v8::String::new(scope, name).unwrap();
        global.set(scope, key.into(), value).unwrap();
    }
    let source = v8::String::new(
        scope,
        r#"
        (() => {
          const assert = (ok, message) => { if (!ok) throw new Error(message); };
          let conversions = 0;
          const sentinel = {};
          const poison = { toString() { conversions++; throw sentinel; } };
          for (let length = 0; length < 6; length++) {
            let error;
            try { parse(...Array(length).fill(poison)); } catch (caught) { error = caught; }
            assert(error instanceof CalleeTypeError, 'missing argument: ' + length);
            assert(!(error instanceof TypeError), 'callee error realm: ' + length);
            if (length >= 3) assert(error.message === 'sixth argument missing', 'custom message');
          }
          assert(conversions === 0, 'arity must precede all conversions');
          let error;
          try { parse(poison, 'two', null, undefined, 'ignored', 'five'); } catch (caught) { error = caught; }
          assert(error === sentinel && conversions === 1, 'preserve original conversion exception');
          const order = [];
          const value = text => ({ toString() { order.push(text); return text; } });
          const result = parse(value('one'), value('two'), undefined, undefined, 'ignored', value('five'), value('six'));
          assert(order.join() === 'one,two,five,six', 'successful conversion order: ' + order);
          assert(result === 'one|two|None|default|five|["six"]', 'nullable/default/custom/rest: ' + result);
          assert(parse('one', 'two', null, null, 'ignored', undefined) === 'one|two|None|null|undefined|[]',
            'present undefined custom argument must be converted');
          return true;
        })()
        "#,
    )
    .unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    assert!(
        script
            .run(scope)
            .expect("argument conversion checks")
            .is_true()
    );
}
