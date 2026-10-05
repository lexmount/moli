use moli_webidl::{Context, WebIdlArgs, WebIdlDictionary};

mod interfaces {
    moli_webapi_declare::declare_web_api_interfaces! {
        pub Base = "RenamedBase";
        pub Derived: Base;
        pub Unrelated;
    }
}

#[derive(WebIdlArgs, WebIdlDictionary)]
#[webidl(prefix = "Test.interfaces")]
struct InterfaceMembers<'scope> {
    #[webidl(default = "")]
    before: String,
    #[webidl(interface = interfaces::Base)]
    optional: Option<v8::Local<'scope, v8::Object>>,
    #[webidl(nullable, interface = interfaces::Base)]
    nullable: Option<v8::Local<'scope, v8::Object>>,
    #[webidl(interface = interfaces::Base, brand_check = is_current_global)]
    special: Option<v8::Local<'scope, v8::Object>>,
    #[webidl(default = 0)]
    after: i32,
}

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.required")]
struct RequiredArgs<'scope> {
    #[webidl(required)]
    before: String,
    #[webidl(required, interface = interfaces::Base)]
    object: v8::Local<'scope, v8::Object>,
}

#[derive(WebIdlArgs, WebIdlDictionary)]
#[webidl(prefix = "Test.requiredNullable")]
struct RequiredNullable<'scope> {
    #[webidl(required, nullable, interface = interfaces::Base)]
    object: Option<v8::Local<'scope, v8::Object>>,
}

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.variadic")]
struct VariadicArgs<'scope> {
    #[webidl(variadic, interface = interfaces::Base)]
    objects: Vec<v8::Local<'scope, v8::Object>>,
}

fn is_current_global<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    object.strict_equals(scope.get_current_context().global(scope).into())
}

fn object_or_null<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: Option<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Value> {
    object
        .map(Into::into)
        .unwrap_or_else(|| v8::null(scope).into())
}

fn members_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: InterfaceMembers<'s>,
) -> v8::Local<'s, v8::Array> {
    let values = [
        v8::String::new(scope, &parsed.before).unwrap().into(),
        object_or_null(scope, parsed.optional),
        object_or_null(scope, parsed.nullable),
        object_or_null(scope, parsed.special),
        v8::Integer::new(scope, parsed.after).into(),
    ];
    v8::Array::new_with_elements(scope, &values)
}

fn optional_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<InterfaceMembers>(scope, &args) {
        rv.set(members_array(scope, parsed).into());
    }
}

fn dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::parse_dictionary::<InterfaceMembers>(
        scope,
        args.get(0),
        Context::argument("Test.dictionary", 1),
    ) {
        Ok(Some(parsed)) => rv.set(members_array(scope, parsed).into()),
        Ok(None) => {}
        Err(error) => moli_webidl::throw_error(scope, &error),
    }
}

fn required_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<RequiredArgs>(scope, &args) {
        let before = v8::String::new(scope, &parsed.before).unwrap();
        rv.set(v8::Array::new_with_elements(scope, &[before.into(), parsed.object.into()]).into());
    }
}

fn required_nullable_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<RequiredNullable>(scope, &args) {
        rv.set(object_or_null(scope, parsed.object));
    }
}

fn required_nullable_dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::parse_dictionary::<RequiredNullable>(
        scope,
        args.get(0),
        Context::argument("Test.requiredNullableDictionary", 1),
    ) {
        Ok(Some(parsed)) => rv.set(object_or_null(scope, parsed.object)),
        Ok(None) => {}
        Err(error) => moli_webidl::throw_error(scope, &error),
    }
}

fn variadic_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<VariadicArgs>(scope, &args) {
        let values: Vec<_> = parsed.objects.into_iter().map(Into::into).collect();
        rv.set(v8::Array::new_with_elements(scope, &values).into());
    }
}

fn run_checks(source: &str) {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let callee = v8::Context::new(scope, Default::default());
    let exports = {
        let scope = &mut v8::ContextScope::new(scope, callee);
        let foreign = v8::Object::new(scope);
        interfaces::Derived::DESCRIPTOR
            .initialize(scope, foreign)
            .unwrap();
        let handler = v8::Object::new(scope);
        let native = v8::Proxy::new(scope, foreign, handler).unwrap();
        moli_webapi_declare::register_web_api_proxy(scope, native).unwrap();
        let key = v8::String::new(scope, "TypeError").unwrap();
        let type_error = callee.global(scope).get(scope, key.into()).unwrap();
        [
            (
                "optionalArgs",
                v8::Function::new(scope, optional_args).unwrap().into(),
            ),
            (
                "dictionary",
                v8::Function::new(scope, dictionary).unwrap().into(),
            ),
            (
                "requiredArgs",
                v8::Function::new(scope, required_args).unwrap().into(),
            ),
            (
                "requiredNullableArgs",
                v8::Function::new(scope, required_nullable_args)
                    .unwrap()
                    .into(),
            ),
            (
                "requiredNullableDictionary",
                v8::Function::new(scope, required_nullable_dictionary)
                    .unwrap()
                    .into(),
            ),
            (
                "variadicArgs",
                v8::Function::new(scope, variadic_args).unwrap().into(),
            ),
            ("CalleeTypeError", type_error),
            ("foreign", foreign.into()),
            ("native", native.into()),
            ("special", callee.global(scope).into()),
        ]
    };
    let caller = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, caller);
    let real = v8::Object::new(scope);
    interfaces::Derived::DESCRIPTOR
        .initialize(scope, real)
        .unwrap();
    let unrelated = v8::Object::new(scope);
    interfaces::Unrelated::DESCRIPTOR
        .initialize(scope, unrelated)
        .unwrap();
    for (name, value) in exports
        .into_iter()
        .chain([("real", real.into()), ("unrelated", unrelated.into())])
    {
        let key = v8::String::new(scope, name).unwrap();
        assert_eq!(
            caller.global(scope).set(scope, key.into(), value),
            Some(true)
        );
    }
    let scope = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = &mut scope.init();
    let source = v8::String::new(scope, source).unwrap();
    let script = v8::Script::compile(scope, source, None).unwrap();
    let result = script.run(scope);
    if result.is_none() {
        let exception = scope.exception().unwrap();
        panic!(
            "interface conversion assertions failed: {}",
            exception.to_rust_string_lossy(scope)
        );
    }
    assert!(result.unwrap().is_true());
}

#[test]
fn interface_arguments_preserve_native_identity_conversion_order_and_callee_errors() {
    run_checks(
        r#"
        (() => {
          const assert = (ok, message) => { if (!ok) throw Error(message); };
          const caught = fn => { try { fn(); } catch (error) { return error; } };
          const typeError = fn => {
            const error = caught(fn);
            assert(error instanceof CalleeTypeError && !(error instanceof TypeError), 'callee TypeError');
            return error;
          };
          Object.setPrototypeOf(real, null);
          for (const value of [real, foreign, native]) {
            const result = optionalArgs('before', value, value, special, 7);
            assert(result[1] === value && result[2] === value && result[3] === special && result[4] === 7,
              'original objects, native proxies, inheritance and cross-realm brands');
            assert(requiredArgs('before', value)[1] === value, 'required object');
            assert(requiredNullableArgs(value) === value, 'required nullable object');
          }
          for (const value of [null, undefined]) {
            assert(optionalArgs('before', undefined, value)[2] === null, 'nullable optional');
            assert(requiredNullableArgs(value) === null, 'nullable required accepts present nullish');
          }
          assert(optionalArgs().slice(1, 4).every(value => value === null), 'missing optional members');
          typeError(() => requiredNullableArgs());
          let conversions = 0;
          const sentinel = {};
          const poison = {toString() { conversions++; throw sentinel; }};
          typeError(() => requiredArgs(poison));
          assert(conversions === 0, 'arity preflight precedes conversions');
          assert(caught(() => optionalArgs(poison)) === sentinel, 'original conversion exception');
          let traps = 0;
          const handler = {get() { traps++; throw Error('get trap'); },
            getPrototypeOf() { traps++; throw Error('prototype trap'); }};
          const revoked = Proxy.revocable(real, {}); revoked.revoke();
          const invalid = [null, {}, 1, 'object', true, Symbol('object'), 1n, unrelated,
            Object.create(real), new Proxy(real, handler), new Proxy(native, handler), revoked.proxy];
          for (const value of invalid) {
            const order = [];
            const before = {toString() { order.push('before'); return 'before'; }};
            const after = {valueOf() { order.push('after'); throw sentinel; }};
            const error = typeError(() => optionalArgs(before, value, null, undefined, after));
            assert(error.message.includes('RenamedBase'), 'declared interface name');
            assert(order.join() === 'before', 'interface validation at field position');
            if (value !== null) typeError(() => requiredNullableArgs(value));
          }
          typeError(() => requiredArgs('before', undefined));
          typeError(() => optionalArgs('before', real, real, real));
          assert(traps === 0, 'identity checks never invoke author proxy traps');
          const order = [];
          const result = optionalArgs({toString() { order.push('before'); return 'label'; }}, real,
            native, special, {valueOf() { order.push('after'); return 9; }});
          assert(order.join() === 'before,after' && result[4] === 9, 'successful conversion order');
          assert(variadicArgs().length === 0, 'empty variadic');
          const values = variadicArgs(real, foreign, native);
          assert(values[0] === real && values[1] === foreign && values[2] === native, 'variadic identity');
          for (const value of [...invalid, undefined]) typeError(() => variadicArgs(real, value, foreign));
          return true;
        })()
    "#,
    );
}

#[test]
fn interface_dictionary_members_preserve_getters_nullability_and_native_identity() {
    run_checks(
        r#"
        (() => {
          const assert = (ok, message) => { if (!ok) throw Error(message); };
          const caught = fn => { try { fn(); } catch (error) { return error; } };
          const typeError = fn => {
            const error = caught(fn);
            assert(error instanceof CalleeTypeError && !(error instanceof TypeError), 'callee TypeError');
            return error;
          };
          const result = dictionary({optional: real, nullable: native, special, after: 7});
          assert(result[1] === real && result[2] === native && result[3] === special && result[4] === 7,
            'interface members preserve original identity and custom predicate');
          assert(dictionary(Object.create({optional: foreign}))[1] === foreign, 'inherited dictionary member');
          for (const input of [{}, {optional: undefined, nullable: undefined}, {nullable: null}]) {
            assert(dictionary(input).slice(1, 4).every(value => value === null), 'optional and nullable dictionary members');
          }
          typeError(() => dictionary({optional: null}));
          typeError(() => dictionary({special: real}));
          assert(requiredNullableDictionary({object: null}) === null, 'required nullable null');
          assert(requiredNullableDictionary({object: native}) === native, 'required nullable object');
          typeError(() => requiredNullableDictionary({}));
          typeError(() => requiredNullableDictionary({object: undefined}));
          const sentinel = {};
          let traps = 0;
          const proxy = new Proxy(real, {get() { traps++; throw sentinel; }, getPrototypeOf() { traps++; throw sentinel; }});
          for (const value of [{}, proxy, Object.create(real), unrelated, 1, 'object']) {
            const order = [];
            const error = typeError(() => dictionary({
              get before() { order.push('before'); return 'label'; },
              get optional() { order.push('optional'); return value; },
              get nullable() { order.push('nullable'); throw sentinel; }
            }));
            assert(error.message.includes('RenamedBase') && error.message.includes('optional'), 'interface member error context');
            assert(order.join() === 'before,optional', 'validation precedes later getter');
          }
          assert(traps === 0, 'member value identity checks never invoke author traps');
          assert(caught(() => dictionary({get optional() { throw sentinel; }})) === sentinel, 'getter exception identity');
          const order = [];
          dictionary({
            get before() { order.push('before'); return 'label'; },
            get optional() { order.push('optional'); return real; },
            get nullable() { order.push('nullable'); return native; },
            get special() { order.push('special'); return special; },
            get after() { order.push('after'); return 1; }
          });
          assert(order.join() === 'before,optional,nullable,special,after', 'successful getter order');
          return true;
        })()
    "#,
    );
}
