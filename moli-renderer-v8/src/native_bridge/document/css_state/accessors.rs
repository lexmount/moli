use super::{
    font_faces::sync_document_fonts,
    shared::{
        ADOPTED_STYLE_SHEETS_SLOT, FONT_FACE_SET_OWNER_DOCUMENT_SLOT, FONTS_SLOT, STYLE_SHEETS_SLOT,
    },
    style_sheets::sync_document_style_sheets,
};
use crate::{
    context_bootstrap::{
        adopted_style_sheet_installations_from_value,
        clear_css_style_sheet_document_adopted_owner_tracking,
        css_style_sheet_constructor_document_handle, css_style_sheet_is_constructed,
        new_style_sheet_list_object, sync_css_style_sheet_document_adopted_owner_tracking,
        sync_css_style_sheet_shadow_root_adopted_owner_tracking,
    },
    document_runtime::DomHandle,
    native_bridge::{
        JsContextHost,
        document::detached_native_handle_for_runtime,
        node::{node_is_document, node_runtime_and_handle_from_object},
    },
    util::{
        callback_data_index_value, callback_data_item, context_host_ptr_from_global_bridge,
        get_private_object, get_private_value, new_null_prototype_object, node_wrapper_from_handle,
        set_private_value, throw_type_error, v8str,
    },
    webidl,
};
use moli_webapi_declare::WebApiObject;
use std::convert::TryFrom;

const ADOPTED_STYLE_SHEETS_ARRAY_MUTATION_METHODS: &[(&str, i32)] = &[
    ("pop", 0),
    ("push", 1),
    ("reverse", 0),
    ("shift", 0),
    ("splice", 2),
    ("unshift", 1),
];
const ADOPTED_STYLE_SHEETS_ARRAY_OWNER_KIND_SLOT: &str = "__moliAdoptedStyleSheetsArrayOwnerKind";
const ADOPTED_STYLE_SHEETS_ARRAY_OWNER_HANDLE_SLOT: &str =
    "__moliAdoptedStyleSheetsArrayOwnerHandle";
const ADOPTED_STYLE_SHEETS_ARRAY_STYLE_DOCUMENT_HANDLE_SLOT: &str =
    "__moliAdoptedStyleSheetsArrayStyleDocumentHandle";
const ADOPTED_STYLE_SHEETS_ARRAY_PRIMORDIALS_SLOT: &str =
    "__moliAdoptedStyleSheetsArrayPrimordials";
const ADOPTED_STYLE_SHEETS_ARRAY_OWNER_DOCUMENT: i32 = 1;
const ADOPTED_STYLE_SHEETS_ARRAY_OWNER_SHADOW_ROOT: i32 = 2;

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct AdoptedStyleSheetsArrayMutationMethodsDeclaration {
    #[webapi(
        method,
        callback = adopted_style_sheets_array_mutation_callback,
        data = callback_data_index_value(scope, 0),
        length = 0
    )]
    pop: (),
    #[webapi(
        method,
        callback = adopted_style_sheets_array_mutation_callback,
        data = callback_data_index_value(scope, 1),
        length = 1
    )]
    push: (),
    #[webapi(
        method,
        callback = adopted_style_sheets_array_mutation_callback,
        data = callback_data_index_value(scope, 2),
        length = 0
    )]
    reverse: (),
    #[webapi(
        method,
        callback = adopted_style_sheets_array_mutation_callback,
        data = callback_data_index_value(scope, 3),
        length = 0
    )]
    shift: (),
    #[webapi(
        method,
        callback = adopted_style_sheets_array_mutation_callback,
        data = callback_data_index_value(scope, 4),
        length = 2
    )]
    splice: (),
    #[webapi(
        method,
        callback = adopted_style_sheets_array_mutation_callback,
        data = callback_data_index_value(scope, 5),
        length = 1
    )]
    unshift: (),
}

#[derive(Clone, Copy)]
pub(in crate::native_bridge) enum AdoptedStyleSheetsArrayOwner {
    Document {
        validation_document: DomHandle,
        style_document: DomHandle,
    },
    ShadowRoot(DomHandle),
}

pub(crate) fn node_document_adopted_style_sheets_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    get_document_adopted_style_sheets_for_receiver(scope, args.this(), &mut rv);
}

pub(crate) fn node_document_adopted_style_sheets_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    set_document_adopted_style_sheets_for_receiver(scope, args.this(), args.get(0));
    rv.set_undefined();
}

fn get_document_adopted_style_sheets_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some((_runtime_ptr, handle)) = document_runtime_and_handle_from_receiver(scope, receiver)
    else {
        throw_document_style_sheets_illegal_invocation(scope, "adoptedStyleSheets");
        rv.set_undefined();
        return;
    };
    let holder = receiver;
    if let Some(existing) = get_private_value(scope, holder, ADOPTED_STYLE_SHEETS_SLOT)
        && !existing.is_undefined()
    {
        rv.set(existing);
        return;
    }
    let array = v8::Array::new(scope, 0);
    install_adopted_style_sheets_array_mutation_methods(
        scope,
        array,
        AdoptedStyleSheetsArrayOwner::Document {
            validation_document: handle,
            style_document: handle,
        },
    );
    set_private_value(scope, holder, ADOPTED_STYLE_SHEETS_SLOT, array.into());
    rv.set(array.into());
}

fn set_document_adopted_style_sheets_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    value: v8::Local<'s, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = document_runtime_and_handle_from_receiver(scope, receiver)
    else {
        throw_document_style_sheets_illegal_invocation(scope, "adoptedStyleSheets");
        return;
    };
    let holder = receiver;
    let Some(next_array) = normalize_adopted_style_sheets_assignment(
        scope,
        value,
        AdoptedStyleSheetsArrayOwner::Document {
            validation_document: handle,
            style_document: handle,
        },
        Some(handle),
    ) else {
        return;
    };
    let next_value = next_array.into();
    let installations = adopted_style_sheet_installations_from_value(scope, next_value);
    if let Some(previous) = get_private_value(scope, holder, ADOPTED_STYLE_SHEETS_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        clear_css_style_sheet_document_adopted_owner_tracking(scope, previous, handle);
    }
    set_private_value(scope, holder, ADOPTED_STYLE_SHEETS_SLOT, next_value);
    unsafe { &mut *runtime_ptr }
        .set_document_adopted_style_sheet_installations(handle, installations);
    sync_css_style_sheet_document_adopted_owner_tracking(scope, next_array.into(), handle);
    super::sync_document_fonts_for_handle(scope, unsafe { &*runtime_ptr }, handle);
}

pub(in crate::native_bridge) fn normalize_adopted_style_sheets_assignment<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    owner: AdoptedStyleSheetsArrayOwner,
    target_document: Option<DomHandle>,
) -> Option<v8::Local<'s, v8::Array>> {
    let array = if value.is_null_or_undefined() {
        v8::Array::new(scope, 0)
    } else {
        let Ok(array) = v8::Local::<v8::Array>::try_from(value) else {
            webidl::throw_type_error(
                scope,
                "DocumentOrShadowRoot.adoptedStyleSheets must be an array.",
            );
            return None;
        };
        array
    };
    if !validate_adopted_style_sheet_array(scope, array, target_document) {
        return None;
    }
    install_adopted_style_sheets_array_mutation_methods(scope, array, owner);
    Some(array)
}

pub(in crate::native_bridge) fn validate_adopted_style_sheet_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    target_document: Option<DomHandle>,
) -> bool {
    (0..array.length()).all(|index| {
        array
            .get_index(scope, index)
            .is_some_and(|value| validate_adopted_style_sheet_value(scope, value, target_document))
    })
}

pub(in crate::native_bridge) fn install_adopted_style_sheets_array_mutation_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    owner: AdoptedStyleSheetsArrayOwner,
) {
    set_adopted_style_sheets_array_owner(scope, array, owner);
    let _ = AdoptedStyleSheetsArrayMutationMethodsDeclaration::default()
        .initialize(scope, array.into());
}

pub(crate) fn install_adopted_style_sheets_array_primordials<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> bool {
    if get_private_object(scope, global, ADOPTED_STYLE_SHEETS_ARRAY_PRIMORDIALS_SLOT).is_some() {
        return true;
    }
    let context_global = scope.get_current_context().global(scope);
    if !global.strict_equals(context_global.into())
        && let Some(primordials) = get_private_object(
            scope,
            context_global,
            ADOPTED_STYLE_SHEETS_ARRAY_PRIMORDIALS_SLOT,
        )
    {
        set_private_value(
            scope,
            global,
            ADOPTED_STYLE_SHEETS_ARRAY_PRIMORDIALS_SLOT,
            primordials.into(),
        );
        return true;
    }
    let array = v8::Array::new(scope, 0);
    let Some(prototype) = array
        .get_prototype(scope)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return false;
    };
    let primordials = new_null_prototype_object(scope);
    for (method, _) in ADOPTED_STYLE_SHEETS_ARRAY_MUTATION_METHODS {
        let Some(function) = prototype
            .get(scope, v8str(scope, method).into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        else {
            return false;
        };
        if primordials.set(scope, v8str(scope, method).into(), function.into()) != Some(true) {
            return false;
        }
    }
    set_private_value(
        scope,
        global,
        ADOPTED_STYLE_SHEETS_ARRAY_PRIMORDIALS_SLOT,
        primordials.into(),
    );
    true
}

fn set_adopted_style_sheets_array_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Array>,
    owner: AdoptedStyleSheetsArrayOwner,
) {
    let (kind, handle, style_document) = match owner {
        AdoptedStyleSheetsArrayOwner::Document {
            validation_document,
            style_document,
        } => (
            ADOPTED_STYLE_SHEETS_ARRAY_OWNER_DOCUMENT,
            validation_document.index_u32(),
            style_document.index_u32(),
        ),
        AdoptedStyleSheetsArrayOwner::ShadowRoot(handle) => (
            ADOPTED_STYLE_SHEETS_ARRAY_OWNER_SHADOW_ROOT,
            handle.index_u32(),
            handle.index_u32(),
        ),
    };
    set_private_value(
        scope,
        array.into(),
        ADOPTED_STYLE_SHEETS_ARRAY_OWNER_KIND_SLOT,
        v8::Integer::new(scope, kind).into(),
    );
    set_private_value(
        scope,
        array.into(),
        ADOPTED_STYLE_SHEETS_ARRAY_OWNER_HANDLE_SLOT,
        v8::Integer::new_from_unsigned(scope, handle).into(),
    );
    set_private_value(
        scope,
        array.into(),
        ADOPTED_STYLE_SHEETS_ARRAY_STYLE_DOCUMENT_HANDLE_SLOT,
        v8::Integer::new_from_unsigned(scope, style_document).into(),
    );
}

fn adopted_style_sheets_array_mutation_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((method, _)) = callback_data_item(
        scope,
        &args,
        ADOPTED_STYLE_SHEETS_ARRAY_MUTATION_METHODS,
        "Document.adoptedStyleSheets array mutation methods",
    ) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let owner = adopted_style_sheets_array_owner(scope, args.this());
    let target_document = owner.and_then(|owner| {
        context_host_ptr_from_global_bridge(scope)
            .and_then(|host| adopted_style_sheets_owner_document(unsafe { &*host }, owner))
    });
    if !validate_adopted_style_sheets_mutation_args(scope, method, &args, target_document) {
        return;
    }
    let result = call_array_prototype_method(scope, args.this(), method, &args)
        .unwrap_or_else(|| v8::undefined(scope).into());
    sync_adopted_style_sheets_array_owner(scope, args.this());
    rv.set(result);
}

fn validate_adopted_style_sheets_mutation_args<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    method: &str,
    args: &v8::FunctionCallbackArguments<'s>,
    target_document: Option<DomHandle>,
) -> bool {
    let inserted_start = match method {
        "push" | "unshift" => 0,
        "splice" => 2,
        _ => return true,
    };
    (inserted_start..args.length())
        .all(|index| validate_adopted_style_sheet_value(scope, args.get(index), target_document))
}

fn validate_adopted_style_sheet_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    target_document: Option<DomHandle>,
) -> bool {
    let Ok(sheet) = v8::Local::<v8::Object>::try_from(value) else {
        webidl::throw_type_error(
            scope,
            "Document.adoptedStyleSheets entries must be CSSStyleSheet objects.",
        );
        return false;
    };
    if !css_style_sheet_is_constructed(scope, sheet) {
        webidl::throw_dom_exception(
            scope,
            "NotAllowedError",
            "Document.adoptedStyleSheets entries must be constructed CSSStyleSheet objects.",
        );
        return false;
    }
    if let (Some(sheet_document), Some(target_document)) = (
        css_style_sheet_constructor_document_handle(scope, sheet),
        target_document,
    ) && sheet_document != target_document
    {
        webidl::throw_dom_exception(
            scope,
            "NotAllowedError",
            "Document.adoptedStyleSheets entries must be constructed in this document.",
        );
        return false;
    }
    true
}

fn adopted_style_sheets_array_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
) -> Option<AdoptedStyleSheetsArrayOwner> {
    let kind = get_private_value(scope, array, ADOPTED_STYLE_SHEETS_ARRAY_OWNER_KIND_SLOT)
        .and_then(|value| value.int32_value(scope))?;
    let handle = get_private_value(scope, array, ADOPTED_STYLE_SHEETS_ARRAY_OWNER_HANDLE_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .map(|value| DomHandle::new(value as usize))?;
    match kind {
        ADOPTED_STYLE_SHEETS_ARRAY_OWNER_DOCUMENT => {
            let style_document = get_private_value(
                scope,
                array,
                ADOPTED_STYLE_SHEETS_ARRAY_STYLE_DOCUMENT_HANDLE_SLOT,
            )
            .and_then(|value| value.uint32_value(scope))
            .map(|value| DomHandle::new(value as usize))
            .unwrap_or(handle);
            Some(AdoptedStyleSheetsArrayOwner::Document {
                validation_document: handle,
                style_document,
            })
        }
        ADOPTED_STYLE_SHEETS_ARRAY_OWNER_SHADOW_ROOT => {
            Some(AdoptedStyleSheetsArrayOwner::ShadowRoot(handle))
        }
        _ => None,
    }
}

fn adopted_style_sheets_owner_document(
    runtime: &JsContextHost,
    owner: AdoptedStyleSheetsArrayOwner,
) -> Option<DomHandle> {
    match owner {
        AdoptedStyleSheetsArrayOwner::Document {
            validation_document,
            ..
        } => Some(validation_document),
        AdoptedStyleSheetsArrayOwner::ShadowRoot(handle) => runtime
            .dom_host()
            .node(handle)
            .and_then(crate::dom::native::Node::owner_document),
    }
}

pub(in crate::native_bridge) fn sync_adopted_style_sheets_array_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &mut *host_ptr };
    match adopted_style_sheets_array_owner(scope, array) {
        Some(AdoptedStyleSheetsArrayOwner::ShadowRoot(handle)) => {
            if !adopted_style_sheets_array_is_current_owner(
                scope,
                array,
                AdoptedStyleSheetsArrayOwner::ShadowRoot(handle),
            ) {
                return;
            }
            let installations = adopted_style_sheet_installations_from_value(scope, array.into());
            host.set_shadow_root_adopted_style_sheet_installations(handle, installations);
            sync_css_style_sheet_shadow_root_adopted_owner_tracking(scope, array, handle);
        }
        Some(AdoptedStyleSheetsArrayOwner::Document { style_document, .. }) => {
            if !adopted_style_sheets_array_is_current_owner(
                scope,
                array,
                AdoptedStyleSheetsArrayOwner::Document {
                    validation_document: style_document,
                    style_document,
                },
            ) {
                return;
            }
            let installations = adopted_style_sheet_installations_from_value(scope, array.into());
            host.set_document_adopted_style_sheet_installations(style_document, installations);
            sync_css_style_sheet_document_adopted_owner_tracking(scope, array, style_document);
            super::sync_document_fonts_for_handle(scope, host, style_document);
        }
        None => {}
    }
}

fn adopted_style_sheets_array_is_current_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
    owner: AdoptedStyleSheetsArrayOwner,
) -> bool {
    let owner_handle = match owner {
        AdoptedStyleSheetsArrayOwner::Document { style_document, .. } => style_document,
        AdoptedStyleSheetsArrayOwner::ShadowRoot(handle) => handle,
    };
    node_wrapper_from_handle(scope, owner_handle)
        .and_then(|holder| {
            get_private_value(scope, holder, ADOPTED_STYLE_SHEETS_SLOT)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        })
        .is_some_and(|current| current.strict_equals(array.into()))
}

fn call_array_prototype_method<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    method: &'static str,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let primordials =
        get_private_object(scope, global, ADOPTED_STYLE_SHEETS_ARRAY_PRIMORDIALS_SLOT)?;
    let function = primordials
        .get(scope, v8str(scope, method).into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let argv = (0..args.length())
        .map(|index| args.get(index))
        .collect::<Vec<_>>();
    function.call(scope, receiver.into(), &argv)
}

pub(crate) fn node_document_style_sheets_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    get_document_style_sheets_for_receiver(scope, args.this(), &mut rv);
}

fn get_document_style_sheets_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some((runtime_ptr, handle)) = document_runtime_and_handle_from_receiver(scope, receiver)
    else {
        throw_document_style_sheets_illegal_invocation(scope, "styleSheets");
        rv.set_undefined();
        return;
    };
    let runtime = unsafe { &*runtime_ptr };
    let holder = receiver;
    let slot = v8str(scope, STYLE_SHEETS_SLOT);
    if let Some(existing) = holder.get(scope, slot.into())
        && !existing.is_undefined()
    {
        rv.set(existing);
        return;
    }
    let list = new_style_sheet_list_object(scope);
    let _ = holder.set(scope, slot.into(), list.into());
    let _ = sync_document_style_sheets(scope, holder, runtime.dom_host(), handle);
    rv.set(list.into());
}

fn document_runtime_and_handle_from_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<(*mut JsContextHost, DomHandle)> {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, receiver)
        && node_is_document(unsafe { &*runtime_ptr }, handle)
    {
        return Some((runtime_ptr, handle));
    }
    let runtime_ptr = context_host_ptr_from_global_bridge(scope)?;
    let handle = detached_native_handle_for_runtime(scope, runtime_ptr, receiver)?;
    node_is_document(unsafe { &*runtime_ptr }, handle).then_some((runtime_ptr, handle))
}

fn throw_document_style_sheets_illegal_invocation(scope: &mut v8::PinScope<'_, '_>, member: &str) {
    throw_type_error(
        scope,
        &format!("Failed to get '{member}' on 'Document': Illegal invocation."),
    );
}

pub(in crate::native_bridge) fn document_fonts_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let Some((runtime_ptr, handle)) = document_runtime_and_handle_from_receiver(scope, holder)
    else {
        let mut rv = rv;
        rv.set_undefined();
        return;
    };
    document_fonts_for_receiver(scope, holder, runtime_ptr, handle, rv);
}

fn document_fonts_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    runtime_ptr: *mut JsContextHost,
    handle: DomHandle,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let is_detached = detached_native_handle_for_runtime(scope, runtime_ptr, holder).is_some();
    unsafe { &mut *runtime_ptr }
        .apply_pending_stylesheet_source_css_projections(scope, runtime_ptr);
    let runtime = unsafe { &*runtime_ptr };
    let slot = v8str(scope, FONTS_SLOT);
    if let Some(existing) = holder.get(scope, slot.into())
        && !existing.is_undefined()
    {
        rv.set(existing);
        return;
    }
    let global = scope.get_current_context().global(scope);
    let Some(ctor_value) = global.get(scope, v8str(scope, "FontFaceSet").into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(ctor) = v8::Local::<v8::Function>::try_from(ctor_value) else {
        rv.set_undefined();
        return;
    };
    let Some(fonts) = ctor.new_instance(scope, &[]) else {
        rv.set_undefined();
        return;
    };
    set_private_value(
        scope,
        fonts,
        FONT_FACE_SET_OWNER_DOCUMENT_SLOT,
        holder.into(),
    );
    let _ = holder.set(scope, slot.into(), fonts.into());
    if !is_detached {
        let _ = sync_document_fonts(scope, holder, runtime, handle);
        if let Some(cycle) = runtime.active_document_web_font_load_cycle() {
            super::font_faces::begin_document_font_face_set_load_cycle_for_document(
                scope, handle, cycle,
            );
        }
    }
    rv.set(fonts.into());
}

pub(in crate::native_bridge::document) fn detached_document_fonts_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let holder = args.this();
    let slot = v8str(scope, FONTS_SLOT);
    if let Some(existing) = holder.get(scope, slot.into())
        && !existing.is_undefined()
    {
        rv.set(existing);
        return;
    }
    let global = scope.get_current_context().global(scope);
    let Some(ctor_value) = global.get(scope, v8str(scope, "FontFaceSet").into()) else {
        rv.set_undefined();
        return;
    };
    let Ok(ctor) = v8::Local::<v8::Function>::try_from(ctor_value) else {
        rv.set_undefined();
        return;
    };
    let Some(fonts) = ctor.new_instance(scope, &[]) else {
        rv.set_undefined();
        return;
    };
    set_private_value(
        scope,
        fonts,
        FONT_FACE_SET_OWNER_DOCUMENT_SLOT,
        holder.into(),
    );
    let _ = holder.set(scope, slot.into(), fonts.into());
    rv.set(fonts.into());
}
