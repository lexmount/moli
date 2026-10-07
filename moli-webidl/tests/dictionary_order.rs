use moli_webidl::{Context, WebIdlArgs, WebIdlDictionary};

#[derive(WebIdlArgs, WebIdlDictionary)]
#[webidl(rename_all = "none")]
struct Renamed {
    #[webidl(name = "zulu", default = 0)]
    first: u32,
    #[webidl(name = "Alpha", default = 0)]
    second: u32,
    #[webidl(default = 0)]
    r#type: u32,
    #[webidl(default = 0)]
    middle: u32,
}

#[derive(WebIdlDictionary)]
#[webidl(rename_all = "kebab-case")]
struct Kebab {
    #[webidl(default = 0)]
    z_value: u32,
    #[webidl(default = 0)]
    a_value: u32,
}

#[derive(WebIdlDictionary)]
struct Base<'s> {
    #[webidl(default = 7)]
    zulu: u32,
    #[webidl(default = "base")]
    bravo: String,
    #[webidl(converter = "raw")]
    marker: Option<v8::Local<'s, v8::Value>>,
}

#[derive(WebIdlDictionary)]
struct Derived<'s> {
    #[webidl(default = "derived")]
    delta: String,
    #[webidl(inherit)]
    base: Base<'s>,
    #[webidl(default = 9)]
    alpha: u32,
}

#[derive(WebIdlDictionary)]
struct MostDerived<'s> {
    #[webidl(required)]
    aardvark: String,
    #[webidl(inherit)]
    parent: Derived<'s>,
}

fn renamed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::convert::<moli_webidl::Dictionary<Renamed>>(
        scope,
        args.get(0),
        Context::argument("Test.renamed", 1),
    ) {
        Ok(value) => rv.set(
            v8::Integer::new_from_unsigned(
                scope,
                value.0.first + value.0.second + value.0.r#type + value.0.middle,
            )
            .into(),
        ),
        Err(error) => moli_webidl::throw_error(scope, &error),
    }
}

fn positional<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    if let Some(value) = moli_webidl::parse_args::<Renamed>(scope, &args) {
        rv.set(
            v8::Integer::new_from_unsigned(
                scope,
                value.first + value.second + value.r#type + value.middle,
            )
            .into(),
        );
    }
}

fn kebab<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::convert::<moli_webidl::Dictionary<Kebab>>(
        scope,
        args.get(0),
        Context::argument("Test.kebab", 1),
    ) {
        Ok(value) => {
            rv.set(v8::Integer::new_from_unsigned(scope, value.0.a_value + value.0.z_value).into())
        }
        Err(error) => moli_webidl::throw_error(scope, &error),
    }
}

fn inherited<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    match moli_webidl::convert::<moli_webidl::Dictionary<MostDerived>>(
        scope,
        args.get(0),
        Context::argument("Test.inherited", 1),
    ) {
        Ok(value) => {
            let value = value.0;
            let base = value.parent.base;
            let text = v8::String::new(
                scope,
                &format!(
                    "{}|{}|{}|{}|{}",
                    base.bravo, base.zulu, value.parent.alpha, value.parent.delta, value.aardvark,
                ),
            )
            .unwrap();
            let marker = base.marker.unwrap_or_else(|| v8::undefined(scope).into());
            rv.set(v8::Array::new_with_elements(scope, &[text.into(), marker]).into());
        }
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
        [
            ("renamed", v8::Function::new(scope, renamed).unwrap().into()),
            (
                "positional",
                v8::Function::new(scope, positional).unwrap().into(),
            ),
            ("kebab", v8::Function::new(scope, kebab).unwrap().into()),
            (
                "inherited",
                v8::Function::new(scope, inherited).unwrap().into(),
            ),
            (
                "CalleeTypeError",
                callee.global(scope).get(scope, key.into()).unwrap(),
            ),
        ]
    };
    let caller = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, caller);
    for (name, value) in exports {
        let name = v8::String::new(scope, name).unwrap();
        assert_eq!(
            caller.global(scope).set(scope, name.into(), value),
            Some(true)
        );
    }
    v8::tc_scope!(let scope, scope);
    let source = v8::String::new(scope, source).unwrap();
    let result = v8::Script::compile(scope, source, None).unwrap().run(scope);
    let error = scope
        .exception()
        .map(|value| value.to_rust_string_lossy(scope));
    assert!(result.is_some_and(|value| value.is_true()), "{error:?}");
}

#[test]
fn final_member_names_determine_dictionary_order_but_not_argument_order() {
    run_checks(
        r#"
        const assert = (ok, message) => {if (!ok) throw Error(message);};
        const names = ['Alpha', 'middle', 'type', 'zulu'];
        for (const kind of ['own', 'inherited', 'proxy']) {
            const order = [], target = {};
            for (const key of names) Object.defineProperty(target, key, {get() {
                order.push(key); return {valueOf() {order.push(key + ':number'); return 1;}};
            }});
            const value = kind === 'inherited' ? Object.create(target) : kind === 'proxy' ? new Proxy(target, {}) : target;
            assert(renamed(value) === 4, 'parsed values');
            assert(order.join() === names.flatMap(key => [key, key + ':number']).join(), 'final name order: ' + order);
        }
        const order = [];
        assert(positional(...[1,2,3,4].map(n => ({valueOf() {order.push(n); return n;}}))) === 10, 'positional values');
        assert(order.join() === '1,2,3,4', 'positional declaration order');
        order.length = 0;
        assert(kebab(new Proxy({}, {get(_, key) {order.push(key); return 1;}})) === 2, 'renamed values');
        assert(order.join() === 'a-value,z-value', 'rename_all before sorting');
        true;
    "#,
    );
}

#[test]
fn each_member_converts_before_the_next_getter_and_rethrows_original_exceptions() {
    run_checks(
        r#"
        const assert = (ok, message) => {if (!ok) throw Error(message);};
        const names = ['Alpha', 'middle', 'type', 'zulu'];
        for (const key of names) for (const conversion of [false, true]) {
            const sentinel = {}, order = [], value = {};
            for (const name of names) Object.defineProperty(value, name, {get() {
                order.push(name);
                if (name === key) {
                    if (!conversion) throw sentinel;
                    return {valueOf() {order.push(name + ':number'); throw sentinel;}};
                }
                return 1;
            }});
            let error;
            try {renamed(value);} catch (caught) {error = caught;}
            const expected = names.slice(0, names.indexOf(key) + 1);
            if (conversion) expected.push(key + ':number');
            assert(error === sentinel && order.join() === expected.join(), 'short circuit ' + key);
        }
        true;
    "#,
    );
}

#[test]
fn inherited_dictionaries_read_one_object_in_ancestor_order_with_scoped_values() {
    run_checks(
        r#"
        const assert = (ok, message) => {if (!ok) throw Error(message);};
        const names = ['bravo', 'marker', 'zulu', 'alpha', 'delta', 'aardvark'];
        const marker = {};
        for (const kind of ['own', 'inherited', 'proxy']) {
            const order = [], target = {};
            for (const key of names) Object.defineProperty(target, key, {get() {
                order.push(key);
                if (key === 'marker') return marker;
                return key === 'zulu' || key === 'alpha'
                    ? {valueOf() {order.push(key + ':number'); return 1;}}
                    : {toString() {order.push(key + ':string'); return key;}};
            }});
            Object.defineProperty(target, 'base', {get() {throw Error('nested base property');}});
            Object.defineProperty(target, 'parent', {get() {throw Error('nested parent property');}});
            const value = kind === 'inherited' ? Object.create(target) : kind === 'proxy' ? new Proxy(target, {}) : target;
            const result = inherited(value);
            assert(result[0] === 'bravo|1|1|delta|aardvark' && result[1] === marker, 'values and scoped identity');
            assert(order.join() === 'bravo,bravo:string,marker,zulu,zulu:number,alpha,alpha:number,delta,delta:string,aardvark,aardvark:string', 'ancestor before own: ' + order);
        }
        assert(inherited({aardvark: 'required'})[0] === 'base|7|9|derived|required', 'ancestor defaults');
        true;
    "#,
    );
}

#[test]
fn inherited_conversion_errors_stop_before_later_members_and_required_checks() {
    run_checks(
        r#"
        const assert = (ok, message) => {if (!ok) throw Error(message);};
        const names = ['bravo', 'marker', 'zulu', 'alpha', 'delta', 'aardvark'];
        for (const key of names) for (const conversion of [false, true]) {
            if (conversion && key === 'marker') continue;
            const sentinel = {}, order = [];
            const value = new Proxy({}, {get(_, name) {
                order.push(name);
                if (name === key) {
                    if (!conversion) throw sentinel;
                    const fail = () => {order.push(name + ':convert'); throw sentinel;};
                    return name === 'zulu' || name === 'alpha' ? {valueOf: fail} : {toString: fail};
                }
                return name === 'marker' ? undefined : name === 'zulu' || name === 'alpha' ? 1 : name;
            }});
            let error;
            try {inherited(value);} catch (caught) {error = caught;}
            const expected = names.slice(0, names.indexOf(key) + 1);
            if (conversion) expected.push(key + ':convert');
            assert(error === sentinel && order.join() === expected.join(), 'inherited exception ' + key);
        }
        const order = [];
        let error;
        try {inherited(new Proxy({}, {get(_, key) {order.push(key); return undefined;}}));} catch (caught) {error = caught;}
        assert(error instanceof CalleeTypeError && !(error instanceof TypeError), 'callee required-member TypeError');
        assert(order.join() === names.join(), 'required check after all preceding members');
        for (const value of [undefined, null]) {
            error = undefined;
            try {inherited(value);} catch (caught) {error = caught;}
            assert(error instanceof CalleeTypeError, 'required descendant of empty dictionary');
        }
        true;
    "#,
    );
}
