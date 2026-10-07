use moli_webidl::{WebIdlArgs, WebIdlDictionary};

mod interfaces {
    moli_webapi_declare::declare_web_api_interfaces! {
        pub Base = "SequenceBase";
        pub Derived: Base;
    }
}

#[derive(WebIdlArgs, WebIdlDictionary)]
#[webidl(prefix = "Test.sequence")]
struct InterfaceSequence<'scope> {
    #[webidl(default = "")]
    before: String,
    #[webidl(required, sequence, interface = interfaces::Base)]
    objects: Vec<v8::Local<'scope, v8::Object>>,
    #[webidl(default = "")]
    after: String,
}

#[derive(WebIdlArgs, WebIdlDictionary)]
#[webidl(prefix = "Test.optionalSequence")]
struct OptionalSequence<'scope> {
    #[webidl(sequence, interface = interfaces::Base)]
    objects: Option<Vec<v8::Local<'scope, v8::Object>>>,
    #[webidl(sequence, interface = interfaces::Base, nullable)]
    nullable: Option<Vec<v8::Local<'scope, v8::Object>>>,
    #[webidl(sequence, interface = interfaces::Base, brand_check = is_current_global)]
    special: Option<Vec<v8::Local<'scope, v8::Object>>>,
}

#[derive(WebIdlArgs, WebIdlDictionary)]
#[webidl(prefix = "Test.itemConverters")]
struct ItemConverters {
    #[webidl(sequence, converter = "usv_string", default = Vec::new())]
    strings: Vec<String>,
    #[webidl(sequence, treat_null_as_empty_string)]
    null_strings: Option<Vec<String>>,
    #[webidl(sequence, converter = "raw")]
    utf16: Option<Vec<moli_webidl::DomString16>>,
    #[webidl(sequence, converter = "enforce_range_unsigned_long")]
    numbers: Option<Vec<u32>>,
}

fn is_current_global<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> bool {
    object.strict_equals(scope.get_current_context().global(scope).into())
}

fn objects_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    objects: Vec<v8::Local<'s, v8::Object>>,
) -> v8::Local<'s, v8::Array> {
    let values: Vec<_> = objects.into_iter().map(Into::into).collect();
    v8::Array::new_with_elements(scope, &values)
}

fn sequence_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: InterfaceSequence<'s>,
) -> v8::Local<'s, v8::Array> {
    let before = v8::String::new(scope, &parsed.before).unwrap();
    let objects = objects_array(scope, parsed.objects);
    let after = v8::String::new(scope, &parsed.after).unwrap();
    v8::Array::new_with_elements(scope, &[before.into(), objects.into(), after.into()])
}

fn optional_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: OptionalSequence<'s>,
) -> v8::Local<'s, v8::Array> {
    let values: Vec<_> = [parsed.objects, parsed.nullable, parsed.special]
        .into_iter()
        .map(|objects| {
            objects
                .map(|objects| objects_array(scope, objects).into())
                .unwrap_or_else(|| v8::null(scope).into())
        })
        .collect();
    v8::Array::new_with_elements(scope, &values)
}

fn items_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: ItemConverters,
) -> v8::Local<'s, v8::Array> {
    let strings: Vec<_> = parsed
        .strings
        .into_iter()
        .map(|string| v8::String::new(scope, &string).unwrap().into())
        .collect();
    let strings = v8::Array::new_with_elements(scope, &strings);
    let null_strings: Vec<_> = parsed
        .null_strings
        .unwrap_or_default()
        .into_iter()
        .map(|string| v8::String::new(scope, &string).unwrap().into())
        .collect();
    let null_strings = v8::Array::new_with_elements(scope, &null_strings);
    let utf16: Vec<_> = parsed
        .utf16
        .unwrap_or_default()
        .into_iter()
        .map(|string| {
            v8::String::new_from_two_byte(scope, &string.0, v8::NewStringType::Normal)
                .unwrap()
                .into()
        })
        .collect();
    let utf16 = v8::Array::new_with_elements(scope, &utf16);
    let numbers: Vec<_> = parsed
        .numbers
        .unwrap_or_default()
        .into_iter()
        .map(|number| v8::Integer::new_from_unsigned(scope, number).into())
        .collect();
    let numbers = v8::Array::new_with_elements(scope, &numbers);
    v8::Array::new_with_elements(
        scope,
        &[
            strings.into(),
            null_strings.into(),
            utf16.into(),
            numbers.into(),
        ],
    )
}

fn sequence_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<InterfaceSequence>(scope, &args) {
        rv.set(sequence_array(scope, parsed).into());
    }
}

fn sequence_dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::parse_dictionary_object::<InterfaceSequence>(
        scope,
        args.get(0).try_into().unwrap(),
    ) {
        Ok(parsed) => rv.set(sequence_array(scope, parsed).into()),
        Err(error) => moli_webidl::throw_error(scope, &error),
    }
}

fn optional_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<OptionalSequence>(scope, &args) {
        rv.set(optional_array(scope, parsed).into());
    }
}

fn optional_dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::parse_dictionary_object::<OptionalSequence>(
        scope,
        args.get(0).try_into().unwrap(),
    ) {
        Ok(parsed) => rv.set(optional_array(scope, parsed).into()),
        Err(error) => moli_webidl::throw_error(scope, &error),
    }
}

fn item_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<ItemConverters>(scope, &args) {
        rv.set(items_array(scope, parsed).into());
    }
}

fn item_dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::parse_dictionary_object::<ItemConverters>(
        scope,
        args.get(0).try_into().unwrap(),
    ) {
        Ok(parsed) => rv.set(items_array(scope, parsed).into()),
        Err(error) => moli_webidl::throw_error(scope, &error),
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
                "sequenceArgs",
                v8::Function::new(scope, sequence_args).unwrap().into(),
            ),
            (
                "sequence",
                v8::Function::new(scope, sequence_dictionary)
                    .unwrap()
                    .into(),
            ),
            (
                "optionalArgs",
                v8::Function::new(scope, optional_args).unwrap().into(),
            ),
            (
                "optional",
                v8::Function::new(scope, optional_dictionary)
                    .unwrap()
                    .into(),
            ),
            (
                "itemsArgs",
                v8::Function::new(scope, item_args).unwrap().into(),
            ),
            (
                "items",
                v8::Function::new(scope, item_dictionary).unwrap().into(),
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
    for (name, value) in exports.into_iter().chain([("real", real.into())]) {
        let key = v8::String::new(scope, name).unwrap();
        assert_eq!(
            caller.global(scope).set(scope, key.into(), value),
            Some(true)
        );
    }
    let scope = std::pin::pin!(v8::TryCatch::new(scope));
    let scope = &mut scope.init();
    let source = v8::String::new(scope, source).unwrap();
    let result = v8::Script::compile(scope, source, None).unwrap().run(scope);
    if result.is_none() {
        panic!(
            "sequence assertions failed: {}",
            scope.exception().unwrap().to_rust_string_lossy(scope)
        );
    }
    assert!(result.unwrap().is_true());
}

#[test]
fn interface_sequences_accept_iterables_and_retain_native_proxy_and_cross_realm_identity() {
    run_checks(
        r#"
        const assert = (ok, message) => { if (!ok) throw new Error(message); };
        Object.setPrototypeOf(real, null);
        const output = sequenceArgs('before', new Set([real, foreign, native]), 'after');
        assert(output[1][0] === real && output[1][1] === foreign && output[1][2] === native, 'sequence identity');
        assert(sequence({objects: [native]})[1][0] === native, 'dictionary sequence');
        const fail = value => {
            let error;
            try { sequenceArgs('', value); } catch (caught) {error = caught;}
            assert(error instanceof CalleeTypeError && !(error instanceof TypeError), 'callee TypeError');
            assert(error.message.includes('Argument 2'), 'argument context');
        };
        for (const value of [null, undefined, 1, 'text', {0: real, length: 1}]) fail(value);
        let traps = 0;
        const fake = Object.create(foreign);
        const proxy = new Proxy(real, {get() {traps++; throw 'trap';}});
        const {proxy: revoked, revoke} = Proxy.revocable(real, {});
        revoke();
        for (const value of [fake, proxy, revoked, {}, null]) fail([value]);
        assert(traps === 0, 'brand validation must not invoke author traps');
        assert(optional({})[0] === null && optional({objects: undefined})[0] === null, 'optional absence');
        assert(optionalArgs(undefined, null)[1] === null, 'nullable sequence');
        assert(optional({nullable: null})[1] === null, 'nullable member');
        assert(optional({objects: []})[0].length === 0, 'present empty sequence');
        assert(optionalArgs(undefined, undefined, [special])[2][0] === special, 'custom item predicate');
        let error;
        try { optionalArgs(null); } catch (caught) {error = caught;}
        assert(error instanceof CalleeTypeError, 'optional non-nullable rejects null');
        try { optionalArgs(undefined, undefined, [real]); } catch (caught) {error = caught;}
        assert(error instanceof CalleeTypeError, 'custom predicate overrides default brand');
        true;
    "#,
    );
}

#[test]
fn sequence_item_options_defaults_and_wrapper_outputs_use_runtime_conversion() {
    run_checks(
        r#"
        const assert = (ok, message) => { if (!ok) throw new Error(message); };
        const parsed = itemsArgs(['\uD800'], [null, 'text'], ['\uD800'], [3.9]);
        assert(parsed[0][0] === '\uFFFD', 'USVString item converter');
        assert(parsed[1].join('|') === '|text', 'item string options');
        assert(parsed[2][0].charCodeAt(0) === 0xD800, 'raw DomString16 wrapper');
        assert(parsed[3][0] === 3, 'explicit item numeric converter');
        assert(itemsArgs()[0].length === 0 && items({})[0].length === 0, 'default empty Vec');
        let error;
        try { items({numbers: [-1]}); } catch (caught) {error = caught;}
        assert(error instanceof CalleeTypeError && error.message.includes('numbers'), 'item error context');
        true;
    "#,
    );
}

#[test]
fn sequence_iteration_stays_interleaved_with_conversion_and_stops_at_first_error() {
    run_checks(
        r#"
        const assert = (ok, message) => { if (!ok) throw new Error(message); };
        const order = [];
        const iterable = {
            get [Symbol.iterator]() {
                order.push('iterator');
                return function() {
                    order.push('call'); let index = 0;
                    return {
                        next() {order.push('next'); return index++ < 2
                            ? {value: {toString() {order.push('string'); return 'value';}}, done: false}
                            : {done: true}; }
                    };
                };
            }
        };
        items({get strings() {order.push('strings'); return iterable;}, get nullStrings() {order.push('after');}});
        assert(order.join() === 'after,strings,iterator,call,next,string,next,string,next', 'interleaving: ' + order);
        let beforeCalls = 0;
        const poison = {toString() {beforeCalls++; throw 'before';}};
        let error;
        try { sequenceArgs(poison); } catch (caught) {error = caught;}
        assert(error instanceof CalleeTypeError && beforeCalls === 0, 'required arity preflight');
        let nextCalls = 0, closes = 0, afterCalls = 0;
        const invalid = {
            [Symbol.iterator]() {
                return {next() {nextCalls++; return {value: {}, done: false};},
                    return() {closes++; return {done: true};}};
            }
        };
        try { sequence({objects: invalid, get after() {afterCalls++; return '';}}); } catch (caught) {error = caught;}
        assert(error instanceof CalleeTypeError && nextCalls === 1 && closes === 0 && afterCalls === 1,
            'earlier member precedes brand failure without iterator close');
        const sentinel = {};
        for (const objects of [
            {get [Symbol.iterator]() {throw sentinel;}},
            {[Symbol.iterator]() {throw sentinel;}},
            {[Symbol.iterator]() {return {next() {throw sentinel;}};}},
            {[Symbol.iterator]() {return {next() {return {get done() {throw sentinel;}};}};}},
            {[Symbol.iterator]() {return {next() {return {done: false, get value() {throw sentinel;}};}};}},
        ]) {
            try { sequence({objects, get after() {afterCalls++; return '';}}); } catch (caught) {error = caught;}
            assert(error === sentinel, 'preserve original iterator exception');
        }
        try { itemsArgs([{toString() {throw sentinel;}}]); } catch (caught) {error = caught;}
        assert(error === sentinel && afterCalls === 6, 'item exception and earlier member count');
        true;
    "#,
    );
}
