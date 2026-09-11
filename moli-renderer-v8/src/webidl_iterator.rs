use crate::web_api_interfaces;
use crate::{
    callback_invocation::invoke_synchronous_webidl_callback_function,
    util::{
        get_private_object, get_private_value, materialize_hidden_function_template_prototype,
        set_private_value, throw_type_error, v8str,
    },
    webidl,
};
use anyhow::{Result, anyhow};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject, v8};

const SNAPSHOT_ITERATOR_VALUES_SLOT: &str = "__moliWebIdlIteratorValues";
const SNAPSHOT_ITERATOR_INDEX_SLOT: &str = "__moliWebIdlIteratorIndex";
const NATIVE_COLLECTION_ITERATOR_SLOT: &str = "__moliWebIdlNativeCollectionIterator";
const NATIVE_COLLECTION_ITERATOR_NEXT_SLOT: &str = "__moliWebIdlNativeCollectionIteratorNext";
const NATIVE_SET_ENTRIES_INTRINSIC_SLOT: &str = "__moliSetEntriesIntrinsic";
const NATIVE_SET_KEYS_INTRINSIC_SLOT: &str = "__moliSetKeysIntrinsic";
const NATIVE_SET_VALUES_INTRINSIC_SLOT: &str = "__moliSetValuesIntrinsic";
const NATIVE_SET_ITERATOR_NEXT_INTRINSIC_SLOT: &str = "__moliSetIteratorNextIntrinsic";
const NATIVE_REFLECT_APPLY_INTRINSIC_SLOT: &str = "__moliReflectApplyIntrinsic";
const STYLE_PROPERTY_MAP_ITERATOR_PROTOTYPE_SLOT: &str = "__moliStylePropertyMapIteratorPrototype";
const EVENT_COUNTS_ITERATOR_PROTOTYPE_SLOT: &str = "__moliEventCountsIteratorPrototype";
const FONT_FACE_SET_ITERATOR_PROTOTYPE_SLOT: &str = "__moliFontFaceSetIteratorPrototype";
const VIEW_TRANSITION_TYPE_SET_ITERATOR_PROTOTYPE_SLOT: &str =
    "__moliViewTransitionTypeSetIteratorPrototype";
const CUSTOM_STATE_SET_ITERATOR_PROTOTYPE_SLOT: &str = "__moliCustomStateSetIteratorPrototype";
const CSS_FONT_FEATURE_VALUES_MAP_ITERATOR_PROTOTYPE_SLOT: &str =
    "__moliCssFontFeatureValuesMapIteratorPrototype";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SnapshotWebIdlIteratorKind {
    StylePropertyMapReadOnly,
    EventCounts,
    FontFaceSet,
    CssFontFeatureValuesMap,
}

impl SnapshotWebIdlIteratorKind {
    const fn interface(self) -> &'static str {
        match self {
            Self::StylePropertyMapReadOnly => "StylePropertyMapReadOnly Iterator",
            Self::EventCounts => "EventCounts Iterator",
            Self::FontFaceSet => "FontFaceSet Iterator",
            Self::CssFontFeatureValuesMap => "CSSFontFeatureValuesMap Iterator",
        }
    }

    const fn prototype_slot(self) -> &'static str {
        match self {
            Self::StylePropertyMapReadOnly => STYLE_PROPERTY_MAP_ITERATOR_PROTOTYPE_SLOT,
            Self::EventCounts => EVENT_COUNTS_ITERATOR_PROTOTYPE_SLOT,
            Self::FontFaceSet => FONT_FACE_SET_ITERATOR_PROTOTYPE_SLOT,
            Self::CssFontFeatureValuesMap => CSS_FONT_FEATURE_VALUES_MAP_ITERATOR_PROTOTYPE_SLOT,
        }
    }

    fn build_template<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::FunctionTemplate> {
        match self {
            Self::StylePropertyMapReadOnly => {
                StylePropertyMapReadOnlyIteratorPrototypeDeclaration::build(scope)
            }
            Self::EventCounts => EventCountsIteratorPrototypeDeclaration::build(scope),
            Self::FontFaceSet => FontFaceSetIteratorPrototypeDeclaration::build(scope),
            Self::CssFontFeatureValuesMap => {
                CssFontFeatureValuesMapIteratorPrototypeDeclaration::build(scope)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetlikeWebIdlIteratorKind {
    ViewTransitionTypeSet,
    CustomStateSet,
}

impl SetlikeWebIdlIteratorKind {
    const fn interface(self) -> &'static str {
        match self {
            Self::ViewTransitionTypeSet => "ViewTransitionTypeSet Iterator",
            Self::CustomStateSet => "CustomStateSet Iterator",
        }
    }

    const fn prototype_slot(self) -> &'static str {
        match self {
            Self::ViewTransitionTypeSet => VIEW_TRANSITION_TYPE_SET_ITERATOR_PROTOTYPE_SLOT,
            Self::CustomStateSet => CUSTOM_STATE_SET_ITERATOR_PROTOTYPE_SLOT,
        }
    }

    fn build_template<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::FunctionTemplate> {
        match self {
            Self::ViewTransitionTypeSet => {
                ViewTransitionTypeSetIteratorPrototypeDeclaration::build(scope)
            }
            Self::CustomStateSet => CustomStateSetIteratorPrototypeDeclaration::build(scope),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetlikeWebIdlIteratorMethod {
    Entries,
    Keys,
    Values,
}

impl SetlikeWebIdlIteratorMethod {
    const fn intrinsic_slot(self) -> &'static str {
        match self {
            Self::Entries => NATIVE_SET_ENTRIES_INTRINSIC_SLOT,
            Self::Keys => NATIVE_SET_KEYS_INTRINSIC_SLOT,
            Self::Values => NATIVE_SET_VALUES_INTRINSIC_SLOT,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MaplikeWebIdlIteratorMethod {
    Entries,
    Keys,
    Values,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct SnapshotWebIdlIteratorDeclaration<'s> {
    #[webapi(slot = SNAPSHOT_ITERATOR_VALUES_SLOT)]
    values: v8::Local<'s, v8::Array>,
    #[webapi(slot = SNAPSHOT_ITERATOR_INDEX_SLOT)]
    index: i32,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct NativeCollectionWebIdlIteratorDeclaration<'s> {
    #[webapi(slot = NATIVE_COLLECTION_ITERATOR_SLOT)]
    iterator: v8::Local<'s, v8::Object>,
    #[webapi(slot = NATIVE_COLLECTION_ITERATOR_NEXT_SLOT)]
    next: v8::Local<'s, v8::Function>,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct SnapshotWebIdlIteratorResultDeclaration<'s> {
    value: v8::Local<'s, v8::Value>,
    done: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::StylePropertyMapReadOnlyIterator,
    intrinsic_prototype_parent = v8::Intrinsic::IteratorPrototype,
    prototype_to_string_tag = "StylePropertyMapReadOnly Iterator",
    readonly_prototype,
    enumerable
)]
struct StylePropertyMapReadOnlyIteratorPrototypeDeclaration {
    #[webapi(method, length = 0, callback = snapshot_webidl_iterator_next_callback)]
    next: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::EventCountsIterator,
    intrinsic_prototype_parent = v8::Intrinsic::MapIteratorPrototype,
    prototype_to_string_tag = "EventCounts Iterator",
    readonly_prototype,
    enumerable
)]
struct EventCountsIteratorPrototypeDeclaration {
    #[webapi(method, length = 0, callback = snapshot_webidl_iterator_next_callback)]
    next: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::FontFaceSetIterator,
    intrinsic_prototype_parent = v8::Intrinsic::SetIteratorPrototype,
    prototype_to_string_tag = "FontFaceSet Iterator",
    readonly_prototype,
    enumerable
)]
struct FontFaceSetIteratorPrototypeDeclaration {
    #[webapi(method, length = 0, callback = snapshot_webidl_iterator_next_callback)]
    next: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::ViewTransitionTypeSetIterator,
    intrinsic_prototype_parent = v8::Intrinsic::SetIteratorPrototype,
    prototype_to_string_tag = "ViewTransitionTypeSet Iterator",
    readonly_prototype,
    enumerable
)]
struct ViewTransitionTypeSetIteratorPrototypeDeclaration {
    #[webapi(
        method,
        length = 0,
        callback = native_collection_webidl_iterator_next_callback
    )]
    next: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::CustomStateSetIterator,
    intrinsic_prototype_parent = v8::Intrinsic::SetIteratorPrototype,
    prototype_to_string_tag = "CustomStateSet Iterator",
    readonly_prototype,
    enumerable
)]
struct CustomStateSetIteratorPrototypeDeclaration {
    #[webapi(
        method,
        length = 0,
        callback = native_collection_webidl_iterator_next_callback
    )]
    next: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::CSSFontFeatureValuesMapIterator,
    intrinsic_prototype_parent = v8::Intrinsic::MapIteratorPrototype,
    prototype_to_string_tag = "CSSFontFeatureValuesMap Iterator",
    readonly_prototype,
    enumerable
)]
struct CssFontFeatureValuesMapIteratorPrototypeDeclaration {
    #[webapi(method, length = 0, callback = snapshot_webidl_iterator_next_callback)]
    next: (),
}

pub(crate) fn install_webidl_collection_iterator_intrinsics<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let set = v8::Set::new(scope);
    let prototype = set
        .get_prototype(scope)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing intrinsic Set prototype"))?;

    for (name, slot) in [
        ("entries", NATIVE_SET_ENTRIES_INTRINSIC_SLOT),
        ("keys", NATIVE_SET_KEYS_INTRINSIC_SLOT),
        ("values", NATIVE_SET_VALUES_INTRINSIC_SLOT),
    ] {
        let function = prototype
            .get(scope, v8str(scope, name).into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
            .ok_or_else(|| anyhow!("missing intrinsic Set.prototype.{name}"))?;
        set_private_value(scope, global, slot, function.into());
    }

    let values = native_collection_intrinsic(scope, NATIVE_SET_VALUES_INTRINSIC_SLOT)
        .ok_or_else(|| anyhow!("missing captured intrinsic Set.prototype.values"))?;
    let iterator = values
        .call(scope, set.into(), &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("failed to create intrinsic Set iterator"))?;
    let iterator_prototype = iterator
        .get_prototype(scope)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing intrinsic Set iterator prototype"))?;
    let next = iterator_prototype
        .get(scope, v8str(scope, "next").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing intrinsic Set iterator next"))?;
    set_private_value(
        scope,
        global,
        NATIVE_SET_ITERATOR_NEXT_INTRINSIC_SLOT,
        next.into(),
    );

    let reflect = global
        .get(scope, v8str(scope, "Reflect").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing intrinsic Reflect object"))?;
    let apply = reflect
        .get(scope, v8str(scope, "apply").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing intrinsic Reflect.apply"))?;
    set_private_value(
        scope,
        global,
        NATIVE_REFLECT_APPLY_INTRINSIC_SLOT,
        apply.into(),
    );

    Ok(())
}

pub(crate) fn webidl_collection_reflect_apply_intrinsic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Function>> {
    native_collection_intrinsic(scope, NATIVE_REFLECT_APPLY_INTRINSIC_SLOT)
}

pub(crate) fn new_snapshot_webidl_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    values: v8::Local<'s, v8::Array>,
    kind: SnapshotWebIdlIteratorKind,
) -> Option<v8::Local<'s, v8::Object>> {
    let iterator = SnapshotWebIdlIteratorDeclaration::new(values, 0)
        .bind(scope)
        .ok()?;
    web_api_interfaces::initialize(scope, iterator, kind.interface()).ok()?;
    let prototype = snapshot_webidl_iterator_prototype(scope, kind)?;
    (iterator.set_prototype(scope, prototype.into()) == Some(true)).then_some(iterator)
}

pub(crate) fn new_setlike_webidl_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    set: v8::Local<'s, v8::Set>,
    kind: SetlikeWebIdlIteratorKind,
    method: SetlikeWebIdlIteratorMethod,
) -> Option<v8::Local<'s, v8::Object>> {
    let (native_iterator, next) = new_native_set_iterator(scope, set, method)?;
    let iterator = NativeCollectionWebIdlIteratorDeclaration::new(native_iterator, next)
        .bind(scope)
        .ok()?;
    web_api_interfaces::initialize(scope, iterator, kind.interface()).ok()?;
    let prototype = setlike_webidl_iterator_prototype(scope, kind)?;
    (iterator.set_prototype(scope, prototype.into()) == Some(true)).then_some(iterator)
}

pub(crate) fn call_setlike_webidl_for_each<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    set: v8::Local<'s, v8::Set>,
    owner: v8::Local<'s, v8::Object>,
    callback: v8::Local<'s, v8::Value>,
    this_arg: v8::Local<'s, v8::Value>,
    operation: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    let callback = prepare_webidl_collection_for_each_callback(scope, callback, operation)?;
    // A captured intrinsic Set iterator has the same live backing-list
    // behavior as Set.prototype.forEach without manufacturing a second
    // JavaScript callback to forward into the page callback.
    let (iterator, next) =
        new_native_set_iterator(scope, set, SetlikeWebIdlIteratorMethod::Values)?;
    loop {
        let value = match advance_native_collection_iterator(scope, iterator, next)? {
            NativeCollectionIteratorStep::Done => break,
            NativeCollectionIteratorStep::Value(value) => value,
        };
        invoke_webidl_collection_for_each_callback(
            scope, &callback, this_arg, value, value, owner,
        )?;
    }
    Some(v8::undefined(scope).into())
}

pub(crate) fn new_maplike_webidl_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Map>,
    method: MaplikeWebIdlIteratorMethod,
) -> Option<v8::Local<'s, v8::Object>> {
    let entries = map.as_array(scope);
    let mut values = Vec::with_capacity(map.size());
    for index in 0..map.size().min(u32::MAX as usize) as u32 {
        let key = entries.get_index(scope, index.saturating_mul(2))?;
        let value = entries.get_index(scope, index.saturating_mul(2).saturating_add(1))?;
        values.push(match method {
            MaplikeWebIdlIteratorMethod::Entries => {
                v8::Array::new_with_elements(scope, &[key, value]).into()
            }
            MaplikeWebIdlIteratorMethod::Keys => key,
            MaplikeWebIdlIteratorMethod::Values => value,
        });
    }
    let values = v8::Array::new_with_elements(scope, &values);
    new_snapshot_webidl_iterator(
        scope,
        values,
        SnapshotWebIdlIteratorKind::CssFontFeatureValuesMap,
    )
}

pub(crate) fn call_maplike_webidl_for_each<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    map: v8::Local<'s, v8::Map>,
    owner: v8::Local<'s, v8::Object>,
    callback: v8::Local<'s, v8::Value>,
    this_arg: v8::Local<'s, v8::Value>,
    operation: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    let callback = prepare_webidl_collection_for_each_callback(scope, callback, operation)?;
    // CSSFontFeatureValuesMap currently defines its iteration residence as
    // this surface-owned snapshot. Callback migration must not silently turn
    // that separate collection policy into a live iterator.
    let entries = map.as_array(scope);
    for index in 0..map.size().min(u32::MAX as usize) as u32 {
        let key = entries.get_index(scope, index.saturating_mul(2))?;
        let value = entries.get_index(scope, index.saturating_mul(2).saturating_add(1))?;
        invoke_webidl_collection_for_each_callback(scope, &callback, this_arg, value, key, owner)?;
    }
    Some(v8::undefined(scope).into())
}

pub(crate) fn prepare_webidl_collection_for_each_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: v8::Local<'s, v8::Value>,
    operation: &'static str,
) -> Option<webidl::PreparedWebIdlCallbackFunction> {
    // Generated maplike/setlike forEach is normatively IsCallable + Call,
    // rather than an IDL Function argument. The strong callback-function
    // value is an equivalent V8 ownership representation: it preserves
    // callable proxies and context anchors, but owns no collection policy.
    match webidl::convert::<webidl::WebIdlCallbackFunction>(
        scope,
        callback,
        webidl::Context::argument(operation, 1),
    ) {
        Ok(callback) => Some(callback.prepare(scope)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

pub(crate) fn invoke_webidl_collection_for_each_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    callback: &webidl::PreparedWebIdlCallbackFunction,
    this_arg: v8::Local<'s, v8::Value>,
    value: v8::Local<'s, v8::Value>,
    key: v8::Local<'s, v8::Value>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<()> {
    let _ = invoke_synchronous_webidl_callback_function(
        scope,
        callback,
        this_arg,
        &[value, key, owner.into()],
    )?;
    Some(())
}

fn new_native_set_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    set: v8::Local<'s, v8::Set>,
    method: SetlikeWebIdlIteratorMethod,
) -> Option<(v8::Local<'s, v8::Object>, v8::Local<'s, v8::Function>)> {
    let intrinsic = native_collection_intrinsic(scope, method.intrinsic_slot())?;
    let iterator = intrinsic
        .call(scope, set.into(), &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    let next = native_collection_intrinsic(scope, NATIVE_SET_ITERATOR_NEXT_INTRINSIC_SLOT)?;
    Some((iterator, next))
}

enum NativeCollectionIteratorStep<'s> {
    Value(v8::Local<'s, v8::Value>),
    Done,
}

fn advance_native_collection_iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    iterator: v8::Local<'s, v8::Object>,
    next: v8::Local<'s, v8::Function>,
) -> Option<NativeCollectionIteratorStep<'s>> {
    let result = next
        .call(scope, iterator.into(), &[])
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    if result
        .get(scope, v8str(scope, "done").into())
        .is_some_and(|value| value.boolean_value(scope))
    {
        return Some(NativeCollectionIteratorStep::Done);
    }
    result
        .get(scope, v8str(scope, "value").into())
        .map(NativeCollectionIteratorStep::Value)
}

fn snapshot_webidl_iterator_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: SnapshotWebIdlIteratorKind,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    if let Some(existing) = get_private_value(scope, global, kind.prototype_slot())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(existing);
    }
    let template = kind.build_template(scope);
    let prototype = materialize_hidden_function_template_prototype(scope, template)?;
    set_private_value(scope, global, kind.prototype_slot(), prototype.into());
    Some(prototype)
}

fn setlike_webidl_iterator_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: SetlikeWebIdlIteratorKind,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    if let Some(existing) = get_private_value(scope, global, kind.prototype_slot())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(existing);
    }
    let template = kind.build_template(scope);
    let prototype = materialize_hidden_function_template_prototype(scope, template)?;
    set_private_value(scope, global, kind.prototype_slot(), prototype.into());
    Some(prototype)
}

fn snapshot_webidl_iterator_next_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let iterator = args.this();
    let Some(values) = get_private_object(scope, iterator, SNAPSHOT_ITERATOR_VALUES_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        throw_type_error(
            scope,
            "WebIDL iterator next called on an incompatible receiver.",
        );
        return;
    };
    let Some(index) = get_private_value(scope, iterator, SNAPSHOT_ITERATOR_INDEX_SLOT)
        .and_then(|value| value.integer_value(scope))
        .and_then(|value| u32::try_from(value).ok())
    else {
        throw_type_error(scope, "WebIDL iterator state is unavailable.");
        return;
    };
    if index >= values.length() {
        let result =
            SnapshotWebIdlIteratorResultDeclaration::new(v8::undefined(scope).into(), true)
                .bind(scope)
                .expect("WebIDL iterator result declaration should bind");
        rv.set(result.into());
        return;
    }
    let value = values
        .get_index(scope, index)
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        iterator,
        SNAPSHOT_ITERATOR_INDEX_SLOT,
        v8::Integer::new_from_unsigned(scope, index.saturating_add(1)).into(),
    );
    let result = SnapshotWebIdlIteratorResultDeclaration::new(value, false)
        .bind(scope)
        .expect("WebIDL iterator result declaration should bind");
    rv.set(result.into());
}

fn native_collection_webidl_iterator_next_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(iterator) = get_private_object(scope, args.this(), NATIVE_COLLECTION_ITERATOR_SLOT)
    else {
        throw_type_error(
            scope,
            "WebIDL collection iterator next called on an incompatible receiver.",
        );
        return;
    };
    let Some(next) = get_private_value(scope, args.this(), NATIVE_COLLECTION_ITERATOR_NEXT_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        throw_type_error(scope, "WebIDL collection iterator state is unavailable.");
        return;
    };
    if let Some(result) = next.call(scope, iterator.into(), &[]) {
        rv.set(result);
    }
}

fn native_collection_intrinsic<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Function>> {
    let global = scope.get_current_context().global(scope);
    get_private_value(scope, global, slot)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
}
