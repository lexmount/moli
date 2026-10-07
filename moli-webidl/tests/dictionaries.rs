use moli_webidl::{Context, WebIdlArgs, WebIdlDictionary};

#[derive(WebIdlDictionary)]
#[webidl(prefix = "ChildOptions")]
struct ChildOptions<'scope> {
    #[webidl(default = 7)]
    limit: u32,
    #[webidl(default = "")]
    label: String,
    #[webidl(converter = "raw", default = v8::String::new(scope, "scoped").unwrap().into())]
    marker: v8::Local<'scope, v8::Value>,
}

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.defaults")]
struct DefaultArgs<'scope> {
    #[webidl(default = "")]
    before: String,
    #[webidl(dictionary)]
    options: ChildOptions<'scope>,
    #[webidl(default = "")]
    after: String,
}

#[derive(WebIdlArgs, WebIdlDictionary)]
#[webidl(prefix = "Test.nested")]
struct Nested<'scope> {
    #[webidl(dictionary)]
    child: Option<ChildOptions<'scope>>,
    #[webidl(dictionary, nullable)]
    nullable: Option<ChildOptions<'scope>>,
    #[webidl(default = "")]
    after: String,
}

#[derive(WebIdlDictionary)]
#[webidl(prefix = "Test.implicit")]
struct ImplicitNested<'scope> {
    #[webidl(dictionary)]
    child: ChildOptions<'scope>,
}

#[derive(WebIdlDictionary)]
#[webidl(prefix = "RequiredOptions")]
struct RequiredOptions {
    #[webidl(required)]
    name: String,
}

#[derive(WebIdlArgs)]
#[webidl(prefix = "Test.required")]
struct RequiredArgs {
    #[webidl(default = "")]
    before: String,
    #[webidl(required, dictionary)]
    options: RequiredOptions,
}

#[derive(WebIdlDictionary)]
#[webidl(prefix = "Test.requiredNested")]
struct RequiredNested {
    #[webidl(dictionary)]
    child: Option<RequiredOptions>,
    #[webidl(default = "")]
    after: String,
}

#[derive(WebIdlArgs, WebIdlDictionary)]
#[webidl(prefix = "Test.explicit")]
struct ExplicitDefault<'scope> {
    #[webidl(dictionary, default = explicit_default(scope))]
    options: ChildOptions<'scope>,
}

fn explicit_default<'s>(scope: &mut v8::PinScope<'s, '_>) -> ChildOptions<'s> {
    ChildOptions {
        limit: 9,
        label: "explicit".into(),
        marker: v8::String::new(scope, "explicit scoped").unwrap().into(),
    }
}

fn child_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    child: ChildOptions<'s>,
) -> v8::Local<'s, v8::Array> {
    let values = [
        v8::Integer::new_from_unsigned(scope, child.limit).into(),
        v8::String::new(scope, &child.label).unwrap().into(),
        child.marker,
    ];
    v8::Array::new_with_elements(scope, &values)
}

fn nested_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parsed: Nested<'s>,
) -> v8::Local<'s, v8::Array> {
    let child = parsed
        .child
        .map(|child| child_array(scope, child).into())
        .unwrap_or_else(|| v8::null(scope).into());
    let nullable = parsed
        .nullable
        .map(|child| child_array(scope, child).into())
        .unwrap_or_else(|| v8::null(scope).into());
    let after = v8::String::new(scope, &parsed.after).unwrap();
    v8::Array::new_with_elements(scope, &[child, nullable, after.into()])
}

fn defaults<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<DefaultArgs>(scope, &args) {
        let before = v8::String::new(scope, &parsed.before).unwrap();
        let options = child_array(scope, parsed.options);
        let after = v8::String::new(scope, &parsed.after).unwrap();
        rv.set(
            v8::Array::new_with_elements(scope, &[before.into(), options.into(), after.into()])
                .into(),
        );
    }
}

fn nested_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<Nested>(scope, &args) {
        rv.set(nested_array(scope, parsed).into());
    }
}

fn nested_dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::parse_dictionary_object::<Nested>(scope, args.get(0).try_into().unwrap()) {
        Ok(parsed) => rv.set(nested_array(scope, parsed).into()),
        Err(error) => moli_webidl::throw_error(scope, &error),
    }
}

fn implicit_dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::parse_dictionary_object::<ImplicitNested>(
        scope,
        args.get(0).try_into().unwrap(),
    ) {
        Ok(parsed) => rv.set(child_array(scope, parsed.child).into()),
        Err(error) => moli_webidl::throw_error(scope, &error),
    }
}

fn required_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<RequiredArgs>(scope, &args) {
        rv.set(
            v8::String::new(scope, &format!("{}|{}", parsed.before, parsed.options.name))
                .unwrap()
                .into(),
        );
    }
}

fn required_nested<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::parse_dictionary_object::<RequiredNested>(
        scope,
        args.get(0).try_into().unwrap(),
    ) {
        Ok(parsed) => rv.set(
            v8::String::new(
                scope,
                &format!(
                    "{:?}|{}",
                    parsed.child.map(|child| child.name),
                    parsed.after
                ),
            )
            .unwrap()
            .into(),
        ),
        Err(error) => moli_webidl::throw_error(scope, &error),
    }
}

fn explicit_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(parsed) = moli_webidl::parse_args::<ExplicitDefault>(scope, &args) {
        rv.set(child_array(scope, parsed.options).into());
    }
}

fn explicit_dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::parse_dictionary_object::<ExplicitDefault>(
        scope,
        args.get(0).try_into().unwrap(),
    ) {
        Ok(parsed) => rv.set(child_array(scope, parsed.options).into()),
        Err(error) => moli_webidl::throw_error(scope, &error),
    }
}

fn runtime_dictionary<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::convert::<moli_webidl::Dictionary<RequiredOptions>>(
        scope,
        args.get(0),
        Context::argument("Test.runtime", 1),
    ) {
        Ok(parsed) => rv.set(v8::String::new(scope, &parsed.0.name).unwrap().into()),
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
        let key = v8::String::new(scope, "TypeError").unwrap();
        let type_error = callee.global(scope).get(scope, key.into()).unwrap();
        [
            (
                "defaults",
                v8::Function::new(scope, defaults).unwrap().into(),
            ),
            (
                "nestedArgs",
                v8::Function::new(scope, nested_args).unwrap().into(),
            ),
            (
                "nested",
                v8::Function::new(scope, nested_dictionary).unwrap().into(),
            ),
            (
                "implicit",
                v8::Function::new(scope, implicit_dictionary)
                    .unwrap()
                    .into(),
            ),
            (
                "requiredArgs",
                v8::Function::new(scope, required_args).unwrap().into(),
            ),
            (
                "requiredNested",
                v8::Function::new(scope, required_nested).unwrap().into(),
            ),
            (
                "explicitArgs",
                v8::Function::new(scope, explicit_args).unwrap().into(),
            ),
            (
                "explicit",
                v8::Function::new(scope, explicit_dictionary)
                    .unwrap()
                    .into(),
            ),
            (
                "runtime",
                v8::Function::new(scope, runtime_dictionary).unwrap().into(),
            ),
            ("CalleeTypeError", type_error),
            (
                "calleePrototype",
                v8::Object::new(scope).get_prototype(scope).unwrap(),
            ),
        ]
    };
    let caller = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, caller);
    for (name, value) in exports {
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
            "dictionary assertions failed: {}",
            scope.exception().unwrap().to_rust_string_lossy(scope)
        );
    }
    assert!(result.unwrap().is_true());
}

#[test]
fn empty_optional_nullable_and_scope_dependent_dictionary_defaults() {
    run_checks(
        r#"
        const assert = (ok, message) => { if (!ok) throw new Error(message); };
        const empty = value => JSON.stringify(value) === '[7,"","scoped"]';
        for (const args of [[], ['before'], ['before', undefined], ['before', null], ['before', {}]]) {
            assert(empty(defaults(...args)[1]), 'empty/default dictionary: ' + args);
        }
        for (const value of [{}, {child: undefined}, {child: null}, {child: {}}]) {
            assert(empty(implicit(value)), 'implicit nested dictionary');
        }
        assert(nested({})[0] === null && nested({child: undefined})[0] === null, 'optional absence');
        assert(empty(nested({child: null})[0]), 'optional null is a present empty dictionary');
        assert(nested({nullable: null})[1] === null, 'nullable null is absent');
        assert(empty(nested({nullable: {}})[1]), 'nullable object parses');
        assert(nestedArgs()[0] === null && empty(nestedArgs(null)[0]), 'positional optional');
        assert(nestedArgs(undefined, null)[1] === null, 'positional nullable');
        assert(explicitArgs()[0] === 9 && explicitArgs(undefined)[0] === 9, 'explicit argument default');
        assert(explicit({})[0] === 9 && explicit({options: undefined})[0] === 9, 'explicit member default');
        assert(empty(explicitArgs(null)) && empty(explicit({options: null})), 'null still parses member defaults');
        for (const prototype of [Object.prototype, calleePrototype]) {
            Object.defineProperty(prototype, 'limit', {configurable: true, get() { throw 'prototype getter'; }});
        }
        try {
            assert(empty(defaults('before', null)[1]) && empty(implicit({child: null})), 'null has no inherited properties');
        } finally {
            delete Object.prototype.limit;
            delete calleePrototype.limit;
        }
        true;
    "#,
    );
}

#[test]
fn required_dictionary_members_and_argument_arity_are_not_bypassed_by_nullish_values() {
    run_checks(
        r#"
        const assert = (ok, message) => { if (!ok) throw new Error(message); };
        const fails = (call, message) => {
            let error;
            try { call(); } catch (caught) { error = caught; }
            assert(error instanceof CalleeTypeError && !(error instanceof TypeError), 'callee TypeError');
            assert(error.message.includes(message), 'error context: ' + error.message);
        };
        let reads = 0;
        const poison = {toString() { reads++; throw 'poison'; }};
        fails(() => requiredArgs(poison), 'required');
        assert(reads === 0, 'arity before coercion');
        for (const value of [undefined, null, {}]) {
            fails(() => requiredArgs('before', value), 'RequiredOptions: name is required');
            fails(() => runtime(value), 'RequiredOptions: name is required');
        }
        assert(requiredNested({}) === 'None|', 'missing optional required dictionary');
        fails(() => requiredNested({child: null}), 'RequiredOptions: name is required');
        assert(requiredArgs('before', {name: 'name'}) === 'before|name', 'required success');
        assert(runtime({name: 'name'}) === 'name', 'runtime conversion');
        for (const value of [false, 3, 'text', Symbol('symbol')]) {
            fails(() => defaults('before', value), 'Test.defaults: Argument 2');
            fails(() => nested({child: value}), 'Test.nested: child');
        }
        true;
    "#,
    );
}

#[test]
fn dictionary_getters_coercions_and_exceptions_follow_member_order() {
    run_checks(
        r#"
        const assert = (ok, message) => { if (!ok) throw new Error(message); };
        const order = [];
        const text = name => ({toString() { order.push(name); return name; }});
        const child = Object.create({
            get limit() { order.push('limit'); return {valueOf() {order.push('number'); return 4;}}; },
            get label() { order.push('label'); return text('string'); },
            get marker() { order.push('marker'); return 'identity'; }
        });
        const parsed = nested({
            get child() { order.push('child'); return child; },
            get nullable() { order.push('nullable'); return undefined; },
            get after() { order.push('after'); return text('last'); }
        });
        assert(order.join() === 'after,last,child,label,string,limit,number,marker,nullable', 'dictionary order: ' + order);
        assert(parsed[0][0] === 4 && parsed[0][2] === 'identity', 'inherited members');
        order.length = 0;
        defaults(text('before'), child, text('after'));
        assert(order.join() === 'before,label,string,limit,number,marker,after', 'argument order: ' + order);
        const sentinel = {};
        let after = 0;
        for (const child of [
            {get limit() { throw sentinel; }},
            {limit: {valueOf() { throw sentinel; }}},
            {get label() { throw sentinel; }},
            {label: {toString() { throw sentinel; }}},
        ]) {
            let error;
            try { nested({child, get after() {after++; return '';}}); } catch (caught) {error = caught;}
            assert(error === sentinel, 'preserve child exception');
        }
        let error;
        try { requiredNested({child: null, get after() {after++; return '';}}); } catch (caught) {error = caught;}
        assert(error instanceof CalleeTypeError && after === 5, 'earlier member precedes child conversion failures');
        const marker = {};
        assert(nested({child: {marker}})[0][2] === marker, 'retain raw member identity');
        true;
    "#,
    );
}
