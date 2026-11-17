//! Live iterators over V8-traced Highlight records. No native range or registry owner is retained.
use crate::interfaces as web_api_interfaces;
use moli_v8_util::{
    get_private_object, get_private_value, materialize_hidden_function_template_prototype,
    set_private_value, throw_type_error, v8str,
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};
const HIGHLIGHT_ITERATOR_RECORDS_SLOT: &str = "__moliHighlightIteratorRecords";
const HIGHLIGHT_ITERATOR_MAPPER_SLOT: &str = "__moliHighlightIteratorMapper";
const HIGHLIGHT_ITERATOR_INDEX_SLOT: &str = "__moliHighlightIteratorIndex";
const HIGHLIGHT_ITERATOR_PROTOTYPE_SLOT: &str = "__moliHighlightIteratorPrototype";
const HIGHLIGHT_REGISTRY_ITERATOR_PROTOTYPE_SLOT: &str = "__moliHighlightRegistryIteratorPrototype";

#[derive(WebApiObject)]
#[webapi(plain)]
struct HighlightIteratorObjectDeclaration<'s> {
    #[webapi(slot = HIGHLIGHT_ITERATOR_RECORDS_SLOT)]
    records: v8::Local<'s, v8::Array>,
    #[webapi(slot = HIGHLIGHT_ITERATOR_MAPPER_SLOT)]
    mapper: v8::Local<'s, v8::Function>,
    #[webapi(slot = HIGHLIGHT_ITERATOR_INDEX_SLOT)]
    index: i32,
}

#[derive(WebApiObject)]
#[webapi(plain, data_properties, enumerable)]
struct HighlightIteratorResultDeclaration<'s> {
    value: v8::Local<'s, v8::Value>,
    done: bool,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::HighlightIterator,
    intrinsic_prototype_parent = v8::Intrinsic::SetIteratorPrototype,
    prototype_to_string_tag = "Highlight Iterator",
    readonly_prototype,
    enumerable
)]
struct HighlightIteratorPrototypeDeclaration {
    #[webapi(method, length = 0, callback = highlight_iterator_next_callback)]
    next: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(
    interface = web_api_interfaces::HighlightRegistryIterator,
    intrinsic_prototype_parent = v8::Intrinsic::MapIteratorPrototype,
    prototype_to_string_tag = "HighlightRegistry Iterator",
    readonly_prototype,
    enumerable
)]
struct HighlightRegistryIteratorPrototypeDeclaration {
    #[webapi(method, length = 0, callback = highlight_iterator_next_callback)]
    next: (),
}

#[derive(Clone, Copy)]
enum HighlightIteratorKind {
    Highlight,
    Registry,
}

impl HighlightIteratorKind {
    const fn prototype_slot(self) -> &'static str {
        match self {
            Self::Highlight => HIGHLIGHT_ITERATOR_PROTOTYPE_SLOT,
            Self::Registry => HIGHLIGHT_REGISTRY_ITERATOR_PROTOTYPE_SLOT,
        }
    }

    fn build_template<'s>(
        self,
        scope: &mut v8::PinScope<'s, '_, ()>,
    ) -> v8::Local<'s, v8::FunctionTemplate> {
        match self {
            Self::Highlight => HighlightIteratorPrototypeDeclaration::build(scope),
            Self::Registry => HighlightRegistryIteratorPrototypeDeclaration::build(scope),
        }
    }
}

pub(super) fn highlight_iterator_factory_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let kind = match args.get(0).uint32_value(scope) {
        Some(0) => HighlightIteratorKind::Highlight,
        Some(1) => HighlightIteratorKind::Registry,
        _ => {
            throw_type_error(scope, "Invalid Highlight iterator kind");
            return;
        }
    };
    let Ok(records) = v8::Local::<v8::Array>::try_from(args.get(1)) else {
        throw_type_error(scope, "Invalid Highlight iterator records");
        return;
    };
    let Ok(mapper) = v8::Local::<v8::Function>::try_from(args.get(2)) else {
        throw_type_error(scope, "Invalid Highlight iterator mapper");
        return;
    };
    let Ok(iterator) = HighlightIteratorObjectDeclaration::new(records, mapper, 0).bind(scope)
    else {
        throw_type_error(scope, "Failed to create Highlight iterator");
        return;
    };
    let Some(prototype) = highlight_iterator_prototype(scope, kind) else {
        throw_type_error(scope, "Failed to create Highlight iterator prototype");
        return;
    };
    if iterator.set_prototype(scope, prototype.into()) != Some(true) {
        throw_type_error(scope, "Failed to bind Highlight iterator prototype");
        return;
    }
    let interface = match kind {
        HighlightIteratorKind::Highlight => "Highlight Iterator",
        HighlightIteratorKind::Registry => "HighlightRegistry Iterator",
    };
    if web_api_interfaces::initialize(scope, iterator, interface).is_err() {
        return;
    }
    rv.set(iterator.into());
}

fn highlight_iterator_prototype<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    kind: HighlightIteratorKind,
) -> Option<v8::Local<'s, v8::Object>> {
    let global = scope.get_current_context().global(scope);
    if let Some(prototype) = get_private_object(scope, global, kind.prototype_slot()) {
        return Some(prototype);
    }
    let template = kind.build_template(scope);
    let prototype = materialize_hidden_function_template_prototype(scope, template)?;
    set_private_value(scope, global, kind.prototype_slot(), prototype.into());
    Some(prototype)
}

fn highlight_iterator_next_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let iterator = args.this();
    let Some(records) = get_private_object(scope, iterator, HIGHLIGHT_ITERATOR_RECORDS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
    else {
        throw_type_error(
            scope,
            "Highlight iterator next called on an incompatible receiver.",
        );
        return;
    };
    let Some(mapper) = get_private_value(scope, iterator, HIGHLIGHT_ITERATOR_MAPPER_SLOT)
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        throw_type_error(scope, "Highlight iterator mapper is unavailable.");
        return;
    };
    let Some(mut index) = get_private_value(scope, iterator, HIGHLIGHT_ITERATOR_INDEX_SLOT)
        .and_then(|value| value.integer_value(scope))
        .and_then(|value| u32::try_from(value).ok())
    else {
        throw_type_error(scope, "Highlight iterator state is unavailable.");
        return;
    };
    while index < records.length() {
        let record_index = index;
        index = index.saturating_add(1);
        set_private_value(
            scope,
            iterator,
            HIGHLIGHT_ITERATOR_INDEX_SLOT,
            v8::Integer::new_from_unsigned(scope, index).into(),
        );
        let Some(record) = records
            .get_index(scope, record_index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        else {
            continue;
        };
        let deleted = record
            .get(scope, v8str(scope, "deleted").into())
            .is_some_and(|value| value.boolean_value(scope));
        if deleted {
            continue;
        }
        // This mapper is captured by the Highlight runtime, not an IDL callback registration.
        // Keep the existing native script-entry microtask policy.
        let Some(value) = moli_v8_util::with_microtasks_scope(scope, |scope| {
            mapper.call(scope, v8::undefined(scope).into(), &[record.into()])
        }) else {
            return;
        };
        let result = HighlightIteratorResultDeclaration::new(value, false)
            .bind(scope)
            .expect("Highlight iterator result declaration should bind");
        rv.set(result.into());
        return;
    }
    let result = HighlightIteratorResultDeclaration::new(v8::undefined(scope).into(), true)
        .bind(scope)
        .expect("Highlight iterator result declaration should bind");
    rv.set(result.into());
}
