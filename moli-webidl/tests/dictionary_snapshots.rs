use moli_webapi_declare::WebApiValue;
use moli_webidl::{SnapshotPrototype, WebIdlDictionary, WebIdlDictionarySerialize};

mod interfaces {
    moli_webapi_declare::declare_web_api_interfaces! {
        pub Certificate;
    }
}

#[derive(WebIdlDictionary)]
#[webidl(snapshot)]
struct Server {
    #[webidl(sequence, required)]
    urls: Vec<String>,
    username: Option<String>,
}

#[derive(WebIdlDictionary)]
#[webidl(snapshot)]
struct Options<'s> {
    #[webidl(default = 7, name = "renamedNumber")]
    number: u32,
    #[webidl(sequence, converter = "dictionary", default = Vec::new())]
    servers: Vec<Server>,
    #[webidl(sequence, interface = interfaces::Certificate, default = Vec::new())]
    certificates: Vec<v8::Local<'s, v8::Object>>,
    // Adding a field needs no separate snapshot list or copy implementation.
    #[webidl(default = "new member")]
    added_later: String,
}

fn snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let owner = v8::Local::<v8::Object>::try_from(args.data()).unwrap();
    let stored =
        moli_webidl::DictionarySnapshot::<Options>::from_private_slot(scope, owner, "SnapshotTest")
            .unwrap();
    let snapshot = stored
        .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
        .unwrap();
    rv.set(snapshot.into());
}

fn eval<'s>(scope: &mut v8::PinScope<'s, '_>, source: &str) -> v8::Local<'s, v8::Value> {
    let source = v8::String::new(scope, source).unwrap();
    v8::Script::compile(scope, source, None)
        .unwrap()
        .run(scope)
        .unwrap()
}

#[test]
fn snapshots_follow_dictionary_metadata_and_copy_only_containers() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
    let scope = &mut scope.init();
    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    let certificate = v8::Object::new(scope);
    interfaces::Certificate::DESCRIPTOR
        .initialize(scope, certificate)
        .unwrap();
    let key = v8::String::new(scope, "certificate").unwrap();
    assert_eq!(
        global.create_data_property(scope, key.into(), certificate.into()),
        Some(true)
    );
    let source = eval(
        scope,
        "({servers:[{urls:['stun:host.example']}], certificates:[certificate], unknown:123})",
    );
    let parsed =
        moli_webidl::parse_dictionary_object::<Options>(scope, source.try_into().unwrap()).unwrap();
    let snapshot_state = parsed.to_snapshot(scope).unwrap();
    let stored: v8::Local<v8::Object> = snapshot_state
        .to_v8_value(scope)
        .unwrap()
        .try_into()
        .unwrap();
    let owner = v8::Object::new(scope);
    snapshot_state
        .store_private(scope, owner, "SnapshotTest")
        .unwrap();
    let key = v8::String::new(scope, "stored").unwrap();
    assert_eq!(
        global.create_data_property(scope, key.into(), stored.into()),
        Some(true)
    );
    let snapshot = v8::Function::builder(snapshot)
        .data(owner.into())
        .build(scope)
        .unwrap();
    let key = v8::String::new(scope, "snapshot").unwrap();
    assert_eq!(
        global.create_data_property(scope, key.into(), snapshot.into()),
        Some(true)
    );
    assert!(eval(scope, r#"(() => {
        const assert = (ok, message) => { if (!ok) throw Error(message); };
        assert(Object.getPrototypeOf(stored) === null, 'private root');
        assert(Object.getPrototypeOf(stored.servers[0]) === null, 'private nested dictionary');
        const first = snapshot();
        assert(Object.keys(first).join() === 'addedLater,certificates,renamedNumber,servers', 'renamed and newly added members');
        assert(first.addedLater === 'new member' && first.renamedNumber === 7, 'member defaults');
        assert(Object.getPrototypeOf(first) === Object.prototype, 'public root');
        assert(Object.getPrototypeOf(first.servers[0]) === Object.prototype, 'public nested dictionary');
        assert(first.servers instanceof Array && first.servers[0].urls instanceof Array, 'public arrays');
        assert(first.certificates !== stored.certificates && first.certificates[0] === certificate, 'interface identity');
        first.servers[0].urls[0] = 'stun:changed.example';
        first.servers[0].username = 'changed';
        first.certificates.length = 0;
        first.addedLater = 'changed';
        Object.defineProperty(Object.prototype, 'username', {configurable:true, get(){throw Error('inherited member');}});
        Object.defineProperty(Object.prototype, 'servers', {configurable:true, set(){throw Error('inherited setter');}});
        const iterator = Object.getOwnPropertyDescriptor(Array.prototype, Symbol.iterator);
        const index = Object.getOwnPropertyDescriptor(Array.prototype, '0');
        Object.defineProperty(Array.prototype, Symbol.iterator, {configurable:true, value(){throw Error('iterator');}});
        Object.defineProperty(Array.prototype, '0', {configurable:true, set(){throw Error('indexed setter');}});
        try {
            const next = snapshot();
            assert(next.servers[0].urls[0] === 'stun:host.example', 'deep copy');
            assert(!Object.hasOwn(next.servers[0], 'username'), 'optional member stays absent');
            assert(next.certificates.length === 1 && next.certificates[0] === certificate, 'interface array retained');
            assert(next.addedLater === 'new member', 'scalar copy');
            const descriptor = Object.getOwnPropertyDescriptor(next, 'servers');
            assert(descriptor.writable && descriptor.enumerable && descriptor.configurable, 'data property descriptors');
        } finally {
            delete Object.prototype.username;
            delete Object.prototype.servers;
            Object.defineProperty(Array.prototype, Symbol.iterator, iterator);
            if (index) Object.defineProperty(Array.prototype, '0', index); else delete Array.prototype[0];
        }
        return true;
    })()"#).is_true());

    // Even a corrupted private carrier must not run an accessor.
    eval(
        scope,
        "globalThis.reads = 0; Object.defineProperty(stored, 'addedLater', {configurable:true, get(){ ++reads; throw Error('getter'); }})",
    );
    assert!(
        snapshot_state
            .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
            .is_none()
    );
    assert_eq!(eval(scope, "reads").int32_value(scope), Some(0));

    // An incomplete binding is an invariant violation, not a partial snapshot.
    let key = v8::String::new(scope, "addedLater").unwrap();
    assert_eq!(stored.delete(scope, key.into()), Some(true));
    assert!(
        snapshot_state
            .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
            .is_none()
    );
}

#[test]
fn private_snapshots_trace_interfaces_without_rooting_unreachable_cycles() {
    moli_v8_test_util::ensure_v8();
    let mut isolate = v8::Isolate::new(Default::default());
    let (owner, weak_owner, weak_certificate) = {
        let scope = std::pin::pin!(v8::HandleScope::new(&mut isolate));
        let scope = &mut scope.init();
        let context = v8::Context::new(scope, Default::default());
        let scope = &mut v8::ContextScope::new(scope, context);
        let certificate = v8::Object::new(scope);
        interfaces::Certificate::DESCRIPTOR
            .initialize(scope, certificate)
            .unwrap();
        let parsed = Options {
            number: 7,
            servers: Vec::new(),
            certificates: vec![certificate],
            added_later: "new member".into(),
        };
        let stored = parsed.to_snapshot(scope).unwrap();
        let owner = v8::Object::new(scope);
        stored.store_private(scope, owner, "SnapshotTest").unwrap();
        let key = v8::String::new(scope, "owner").unwrap();
        assert_eq!(
            certificate.create_data_property(scope, key.into(), owner.into()),
            Some(true)
        );
        (
            v8::Global::new(scope, owner),
            v8::Weak::new(scope, owner),
            v8::Weak::new(scope, certificate),
        )
    };
    isolate.low_memory_notification();
    assert!(
        !weak_certificate.is_empty(),
        "the live owner traces its certificate"
    );
    drop(owner);
    isolate.low_memory_notification();
    assert!(
        weak_owner.is_empty(),
        "the owner/certificate cycle is collectible"
    );
    assert!(
        weak_certificate.is_empty(),
        "no native global roots the certificate"
    );
}
