//! EME's readonly binary-key collection. CDM/session production is separate.
//! Its pair iterator reads the current sorted list on every next() call, as
//! required by Web IDL. BufferSource keys retain their existing object identity;
//! only iterator result objects and entry arrays are newly allocated.

use crate::{
    util::{
        get_private_object, get_private_value, materialize_hidden_function_template_prototype,
        set_private_value,
    },
    web_api_interfaces, webidl,
    webidl_iterator::{
        invoke_webidl_collection_for_each_callback, prepare_webidl_collection_for_each_callback,
    },
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

const BACKING: &str = "__moliMediaKeyStatusMapBacking";
const TARGET: &str = "__moliMediaKeyStatusMapIteratorTarget";
const INDEX: &str = "__moliMediaKeyStatusMapIteratorIndex";
const KIND: &str = "__moliMediaKeyStatusMapIteratorKind";
const ITERATOR_PROTOTYPE: &str = "__moliMediaKeyStatusMapIteratorPrototype";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaKeyStatusMap, receiver, enumerable)]
struct Prototype {
    #[webapi(accessor_property, getter = size)]
    size: (),
    #[webapi(method, length = 1, callback = has)]
    has: (),
    #[webapi(method, length = 1, callback = get)]
    get: (),
    #[webapi(method, length = 0, callback = entries)]
    entries: (),
    #[webapi(method, length = 0, callback = keys)]
    keys: (),
    #[webapi(method, length = 0, callback = values)]
    values: (),
    #[webapi(method, length = 1, callback = for_each)]
    for_each: (),
    #[webapi(alias = "entries", symbol = "iterator")]
    iterator: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaKeyStatusMap lookup")]
struct KeyArgs<'s> {
    #[webidl(required, converter = "raw")]
    key_id: webidl::NonSharedBufferSource<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaKeyStatusMap.forEach")]
struct ForEachArgs<'s> {
    #[webidl(required)]
    callback: v8::Local<'s, v8::Value>,
    #[webidl(default = v8::undefined(scope).into())]
    this_arg: v8::Local<'s, v8::Value>,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaKeyStatusMapIterator, prototype = "Object")]
struct IteratorObject<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
    #[webapi(slot = TARGET)]
    target: v8::Local<'s, v8::Object>,
    #[webapi(slot = INDEX, init = 0)]
    index: (),
    #[webapi(slot = KIND)]
    kind: i32,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaKeyStatusMapIterator, receiver,
    intrinsic_prototype_parent = v8::Intrinsic::IteratorPrototype,
    prototype_to_string_tag = "MediaKeyStatusMap Iterator", readonly_prototype, enumerable)]
struct IteratorPrototype {
    #[webapi(method, length = 0, callback = next)]
    next: (),
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct IteratorResult<'s> {
    value: v8::Local<'s, v8::Value>,
    done: bool,
}

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    name: &str,
) {
    if name == "MediaKeyStatusMap" {
        Prototype::initialize_prototype_template(scope, template.prototype_template(scope));
    }
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, receiver)
        .expect("EME collection receiver was validated")
}

fn backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Map> {
    let object = target(scope, receiver);
    get_private_object(scope, object, BACKING)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        .expect("native key status map retains its backing")
}

fn size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    rv.set_uint32(backing(scope, args.this()).size() as u32);
}

fn lookup<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    bytes: &[u8],
) -> Option<v8::Local<'s, v8::Value>> {
    let map = backing(scope, receiver);
    let entries = map.as_array(scope);
    for index in 0..map.size() as u32 {
        let key =
            v8::Local::<v8::ArrayBuffer>::try_from(entries.get_index(scope, index * 2)?).ok()?;
        if key.byte_length() == bytes.len()
            && webidl::AllowSharedBufferSource::Buffer(key).to_vec(scope) == bytes
        {
            return entries.get_index(scope, index * 2 + 1);
        }
    }
    None
}

fn has<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<KeyArgs>(scope, &args) else {
        return;
    };
    let bytes = parsed.key_id.to_vec(scope);
    rv.set_bool(lookup(scope, args.this(), &bytes).is_some());
}

fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<KeyArgs>(scope, &args) else {
        return;
    };
    let bytes = parsed.key_id.to_vec(scope);
    rv.set(lookup(scope, args.this(), &bytes).unwrap_or_else(|| v8::undefined(scope).into()));
}

struct KeyStatusPair<'s> {
    bytes: Vec<u8>,
    key: v8::Local<'s, v8::ArrayBuffer>,
    status: v8::Local<'s, v8::Value>,
}

fn sorted_pairs<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Vec<KeyStatusPair<'s>> {
    let map = backing(scope, receiver);
    let array = map.as_array(scope);
    let mut pairs = Vec::with_capacity(map.size());
    for index in 0..map.size() as u32 {
        let key = v8::Local::<v8::ArrayBuffer>::try_from(
            array.get_index(scope, index * 2).expect("native key"),
        )
        .expect("native key id is an ArrayBuffer");
        let source = webidl::AllowSharedBufferSource::Buffer(key);
        let status = array
            .get_index(scope, index * 2 + 1)
            .expect("native key status");
        pairs.push(KeyStatusPair {
            bytes: source.to_vec(scope),
            key,
            status,
        });
    }
    pairs.sort_by(|a, b| a.bytes.cmp(&b.bytes));
    pairs
}

fn iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
    kind: i32,
) {
    let global = scope.get_current_context().global(scope);
    let prototype = get_private_object(scope, global, ITERATOR_PROTOTYPE).unwrap_or_else(|| {
        let template = IteratorPrototype::build(scope);
        let prototype = materialize_hidden_function_template_prototype(scope, template)
            .expect("EME iterator prototype");
        set_private_value(scope, global, ITERATOR_PROTOTYPE, prototype.into());
        prototype
    });
    let object = IteratorObject::new(prototype, target(scope, args.this()), kind)
        .bind(scope)
        .expect("EME iterator should bind");
    rv.set(object.into());
}

fn entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    iterator(scope, args, rv, 0);
}
fn keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    iterator(scope, args, rv, 1);
}
fn values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    iterator(scope, args, rv, 2);
}

fn next<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let iterator = target(scope, args.this());
    let owner = get_private_object(scope, iterator, TARGET).expect("EME iterator target");
    let index = get_private_value(scope, iterator, INDEX)
        .and_then(|value| value.uint32_value(scope))
        .expect("EME iterator index");
    let kind = get_private_value(scope, iterator, KIND)
        .and_then(|value| value.int32_value(scope))
        .expect("EME iterator kind");
    let pairs = sorted_pairs(scope, owner);
    let (value, done) = if let Some(pair) = pairs.into_iter().nth(index as usize) {
        set_private_value(
            scope,
            iterator,
            INDEX,
            v8::Integer::new_from_unsigned(scope, index.saturating_add(1)).into(),
        );
        let value = match kind {
            0 => v8::Array::new_with_elements(scope, &[pair.key.into(), pair.status]).into(),
            1 => pair.key.into(),
            2 => pair.status,
            _ => unreachable!("native EME iterator kind"),
        };
        (value, false)
    } else {
        (v8::undefined(scope).into(), true)
    };
    rv.set(
        IteratorResult::new(value, done)
            .bind(scope)
            .expect("EME iterator result")
            .into(),
    );
}

fn for_each<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ForEachArgs>(scope, &args) else {
        return;
    };
    let Some(callback) = prepare_webidl_collection_for_each_callback(
        scope,
        parsed.callback,
        "MediaKeyStatusMap.forEach",
    ) else {
        return;
    };
    let owner = args.this();
    let mut index = 0;
    loop {
        let pairs = sorted_pairs(scope, owner);
        let Some(pair) = pairs.into_iter().nth(index) else {
            break;
        };
        if invoke_webidl_collection_for_each_callback(
            scope,
            &callback,
            parsed.this_arg,
            pair.status,
            pair.key.into(),
            owner,
        )
        .is_none()
        {
            return;
        }
        index += 1;
    }
    rv.set_undefined();
}

#[cfg(test)]
pub(crate) fn map_for_test<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Map>,
) -> v8::Local<'s, v8::Object> {
    #[derive(WebApiObject)]
    #[webapi(interface = web_api_interfaces::MediaKeyStatusMap, require_prototype)]
    struct NativeStatusMap<'s> {
        #[webapi(slot = BACKING)]
        backing: v8::Local<'s, v8::Map>,
    }
    NativeStatusMap::new(map)
        .bind(scope)
        .expect("native EME test collection")
}
