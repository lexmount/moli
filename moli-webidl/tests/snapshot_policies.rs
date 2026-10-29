use moli_webapi_declare::WebApiValue;
use moli_webidl::{SnapshotPrototype, WebIdlDictionary, WebIdlDictionarySerialize};

#[derive(WebIdlDictionary)]
#[webidl(snapshot)]
struct Base {
    #[webidl(default = true, snapshot_slot = "SnapshotBaseEnabled")]
    enabled: bool,
}

#[derive(WebIdlDictionary)]
#[webidl(snapshot)]
struct Child {
    #[webidl(sequence, required)]
    values: Vec<String>,
}

#[derive(WebIdlDictionary)]
#[webidl(snapshot)]
struct Policies {
    #[webidl(inherit)]
    base: Base,
    #[webidl(required, converter = "buffer_source")]
    buffer: Vec<u8>,
    #[webidl(with = custom_member, snapshot_with = custom_snapshot, snapshot_state_with = custom_state)]
    custom: String,
    #[webidl(sequence, default = Vec::new(), snapshot_frozen)]
    frozen_items: Vec<String>,
    #[webidl(nullable, dictionary)]
    nullable_child: Option<Child>,
    #[webidl(required, converter = "raw")]
    record: moli_webidl::Record<moli_webidl::DomString, moli_webidl::Dictionary<Child>>,
    #[webidl(snapshot_default = v8::Integer::new(scope, 99).into(), snapshot_slot = "SnapshotSeed")]
    seed: Option<u32>,
    #[webidl(default = "state", snapshot_attributes = v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_DELETE)]
    state_label: String,
    #[webidl(with = nullable_member)]
    tri_state: Option<Option<String>>,
    #[webidl(default = moli_webidl::DomString16(Vec::new()), converter = "raw")]
    utf16: moli_webidl::DomString16,
    #[webidl(default = moli_webidl::EnforceRangeUnsignedLong(4), converter = "raw")]
    wrapped: moli_webidl::EnforceRangeUnsignedLong,
}

fn custom_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<String, moli_webidl::WebIdlError> {
    let context = moli_webidl::Context::member("Policies", name);
    moli_webidl::optional_member::<moli_webidl::DomString>(scope, object, name, context).map(
        |value| {
            value
                .map(|value| value.0)
                .unwrap_or_default()
                .to_uppercase()
        },
    )
}

fn custom_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &str,
    _: SnapshotPrototype,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::String::new(scope, &format!("copy:{value}")).map(Into::into)
}

fn custom_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    v8::String::new(scope, &format!("state:{value}")).map(Into::into)
}

fn nullable_member<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> Result<Option<Option<String>>, moli_webidl::WebIdlError> {
    let context = moli_webidl::Context::member("Policies", name);
    match moli_webidl::property_result(scope, object, name, context)? {
        None => Ok(None),
        Some(value) if value.is_undefined() => Ok(None),
        Some(value) if value.is_null() => Ok(Some(None)),
        Some(value) => moli_webidl::convert::<moli_webidl::DomString>(scope, value, context)
            .map(|value| Some(Some(value.0))),
    }
}

fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap()
}

fn expose<'s>(scope: &mut v8::PinScope<'s, '_>, name: &str, value: v8::Local<'s, v8::Value>) {
    let global = scope.get_current_context().global(scope);
    let name = v8::String::new(scope, name).unwrap();
    assert_eq!(
        global.create_data_property(scope, name.into(), value),
        Some(true)
    );
}

#[test]
fn inherited_nullable_record_buffer_and_state_policies_preserve_conversion() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let source = eval(
        scope,
        r#"(() => {
        globalThis.reads = [];
        globalThis.bytes = new Uint8Array([8, 1, 2, 9]);
        const values = {
            enabled: true, buffer: bytes.subarray(1, 3), custom: 'tag',
            frozenItems: ['one'], nullableChild: null,
            record: {['__proto__']: {values:['safe']}, alpha:{values:['original']}},
            triState: null, utf16: '\ud800', wrapped: 8
        };
        globalThis.input = {};
        for (const key of Object.keys(values)) Object.defineProperty(input, key, {
            configurable:true, get(){ reads.push(key); return values[key]; }
        });
        return input;
    })()"#,
    );
    let mut parsed =
        moli_webidl::parse_dictionary_object::<Policies>(scope, source.try_into().unwrap())
            .unwrap();
    assert!(
        eval(
            scope,
            "reads[0] === 'enabled' && new Set(reads).size === reads.length"
        )
        .is_true()
    );
    eval(
        scope,
        "bytes.fill(0); for (const key of Object.getOwnPropertyNames(input)) Object.defineProperty(input, key, {get(){throw Error('re-read')}, configurable:true})",
    );
    let stored = parsed.to_snapshot(scope).unwrap();
    let internal = stored.to_v8_value(scope).unwrap();
    expose(scope, "internal", internal);
    let first = stored
        .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
        .unwrap();
    expose(scope, "first", first.into());
    assert!(eval(scope, r#"(() => {
        const assert = (ok) => { if (!ok) throw Error('snapshot policy'); };
        assert(first.enabled && first.seed === 99 && first.wrapped === 8);
        assert(first.nullableChild === null && first.triState === null);
        assert(first.custom === 'copy:TAG' && first.utf16.charCodeAt(0) === 0xd800);
        assert(Object.getPrototypeOf(internal.record) === null);
        assert(Object.getPrototypeOf(internal.record.alpha) === null);
        assert(Object.getPrototypeOf(first.record) === Object.prototype);
        assert(Object.hasOwn(first.record, '__proto__') && first.record.__proto__.values[0] === 'safe');
        assert(Object.isFrozen(first.frozenItems) && first.frozenItems[0] === 'one');
        assert(new Uint8Array(first.buffer).join() === '1,2');
        new Uint8Array(first.buffer).fill(7);
        first.record.alpha.values[0] = 'changed';
        const descriptor = Object.getOwnPropertyDescriptor(first, 'stateLabel');
        assert(descriptor.writable && descriptor.enumerable && descriptor.configurable);
        return true;
    })()"#).is_true());
    let second = stored
        .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
        .unwrap();
    expose(scope, "second", second.into());
    assert!(eval(scope, "new Uint8Array(second.buffer).join() === '1,2' && second.record.alpha.values[0] === 'original' && second.buffer !== first.buffer && second.record !== first.record").is_true());

    let state = v8::Object::new(scope);
    parsed.initialize_snapshot(scope, state).unwrap();
    expose(scope, "state", state.into());
    assert!(eval(scope, "state.custom === 'state:TAG'").is_true());
    assert!(eval(scope, "!Object.hasOwn(state, 'seed') && !Object.hasOwn(state, 'enabled') && !Object.getOwnPropertyDescriptor(state, 'stateLabel').writable && !Object.getOwnPropertyDescriptor(state, 'stateLabel').configurable").is_true());
    parsed.base.enabled = false;
    parsed.seed = Some(5);
    parsed.tri_state = None;
    parsed.initialize_own_snapshot(scope, state).unwrap();
    let enabled = moli_v8_util::get_private_value(scope, state, "SnapshotBaseEnabled").unwrap();
    assert!(
        enabled.is_true(),
        "own-member initialization preserves inherited state"
    );
    let seed = moli_v8_util::get_private_value(scope, state, "SnapshotSeed").unwrap();
    assert_eq!(seed.uint32_value(scope), Some(5));
    let output = parsed
        .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
        .unwrap();
    expose(scope, "output", output.into());
    assert!(
        eval(
            scope,
            "!Object.hasOwn(output, 'triState') && output.seed === 5 && !output.enabled"
        )
        .is_true()
    );
    parsed.tri_state = Some(Some("present".into()));
    let output = parsed
        .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
        .unwrap();
    expose(scope, "output", output.into());
    assert!(eval(scope, "output.triState === 'present'").is_true());
}

#[derive(WebIdlDictionary)]
#[webidl(snapshot)]
struct Frozen {
    #[webidl(sequence, converter = "dictionary", default = Vec::new(), snapshot_frozen)]
    children: Vec<Child>,
}

#[test]
fn snapshot_containers_and_frozen_arrays_use_the_entered_realm() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let original = v8::Context::new(scope, Default::default());
    let other = v8::Context::new(scope, Default::default());
    let stored = {
        let scope = &mut v8::ContextScope::new(scope, original);
        Frozen {
            children: vec![Child {
                values: vec!["value".into()],
            }],
        }
        .to_snapshot(scope)
        .unwrap()
    };
    let scope = &mut v8::ContextScope::new(scope, other);
    eval(
        scope,
        "Object.defineProperty(Object.prototype, 'children', {set(){throw Error('setter')}, configurable:true}); Object.defineProperty(Array.prototype, '0', {set(){throw Error('index')}, configurable:true}); Array.prototype[Symbol.iterator] = () => {throw Error('iterator')}",
    );
    let output = stored
        .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
        .unwrap();
    assert_eq!(output.get_creation_context(scope).unwrap(), other);
    expose(scope, "snapshot", output.into());
    assert!(eval(scope, "Object.getPrototypeOf(snapshot) === Object.prototype && snapshot.children instanceof Array && Object.isFrozen(snapshot.children) && Object.getPrototypeOf(snapshot.children[0]) === Object.prototype && snapshot.children[0].values instanceof Array && snapshot.children[0].values[0] === 'value'").is_true());
}

#[derive(WebIdlDictionary)]
#[webidl(snapshot)]
struct Callbacks {
    #[webidl(converter = "callback_function")]
    handler: Option<moli_webidl::WebIdlCallbackFunction>,
    #[webidl(converter = "callback_interface")]
    listener: Option<moli_webidl::WebIdlCallbackInterface>,
}

#[test]
fn snapshot_callback_contexts_are_traced_and_owner_cycles_are_collectible() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let (owner, weak_owner, weak_callback, weak_incumbent) = {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let relevant = v8::Context::new(scope, Default::default());
        let incumbent = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, relevant);
        let function: v8::Local<v8::Object> = eval(scope, "(() => {})").try_into().unwrap();
        let listener = v8::Object::new(scope);
        let parsed = Callbacks {
            handler: Some(
                moli_webidl::WebIdlCallbackFunction::try_new(scope, function, relevant, incumbent)
                    .unwrap(),
            ),
            listener: Some(moli_webidl::WebIdlCallbackInterface::new(
                scope, listener, relevant, incumbent,
            )),
        };
        let snapshot = parsed.to_snapshot(scope).unwrap();
        let public = snapshot
            .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
            .unwrap();
        let handler = v8::String::new(scope, "handler").unwrap();
        let listener_key = v8::String::new(scope, "listener").unwrap();
        assert!(
            public
                .get(scope, handler.into())
                .unwrap()
                .strict_equals(function.into())
        );
        assert!(
            public
                .get(scope, listener_key.into())
                .unwrap()
                .strict_equals(listener.into())
        );
        let owner = v8::Object::new(scope);
        snapshot.store_private(scope, owner, "Callbacks").unwrap();
        let key = v8::String::new(scope, "owner").unwrap();
        function
            .create_data_property(scope, key.into(), owner.into())
            .unwrap();
        let incumbent_anchor = incumbent.get_extras_binding_object(scope);
        (
            v8::Global::new(scope, owner),
            v8::Weak::new(scope, owner),
            v8::Weak::new(scope, function),
            v8::Weak::new(scope, incumbent_anchor),
        )
    };
    isolate.low_memory_notification();
    assert!(!weak_callback.is_empty());
    assert!(
        !weak_incumbent.is_empty(),
        "the independent conversion-time incumbent is retained"
    );
    drop(owner);
    isolate.low_memory_notification();
    assert!(weak_owner.is_empty());
    assert!(weak_callback.is_empty());
    assert!(
        weak_incumbent.is_empty(),
        "no native Global permanently roots the callback contexts"
    );
}
