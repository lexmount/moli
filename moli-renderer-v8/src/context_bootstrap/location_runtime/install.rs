use super::super::constructors::illegal_constructor_callback;
use super::super::navigation_window::{
    navigation_document_has_opaque_origin, runtime_window_dispatch_scope, runtime_window_owner,
};
use super::helpers::{
    location_host_string, navigate_modified_location_url, parsed_location_url,
    require_location_href_slot, set_return_string, v8_value_to_string,
};
use super::methods::{
    location_assign_callback, location_reload_callback, location_replace_callback,
    location_to_string_callback,
};
use super::slots::{
    clear_location_ancestor_origins_slot, location_ancestor_origins_slot,
    location_empty_ancestor_origins_slot, location_href_slot, location_relevant_document_id_slot,
    location_relevant_local_window_id_slot, set_location_ancestor_origins_slot,
    set_location_empty_ancestor_origins_slot, set_location_relevant_document_id_slot,
    set_location_relevant_local_window_id_slot, sync_location_object_fields,
};
use super::*;
use crate::context_bootstrap::exposed_interfaces::build_intrinsic_interface_instance;
use crate::context_bootstrap::indexed_db::new_dom_string_list;
use crate::util::{callback_data_index_value, callback_data_item};
use anyhow::{Result, anyhow};
use moli_webapi_declare::WebApiObject;

#[derive(Clone, Copy)]
enum LocationAttribute {
    AncestorOrigins,
    Origin,
    Href,
    Hash,
    Search,
    Pathname,
    Protocol,
    Host,
    Hostname,
    Port,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Location")]
struct LocationOwnSurfaceDeclaration {
    #[webapi(
        accessor_property,
        getter = location_readonly_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable,
        dont_delete
    )]
    ancestor_origins: (),
    #[webapi(
        accessor_property,
        getter = location_readonly_attribute_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable,
        dont_delete
    )]
    origin: (),
    #[webapi(
        accessor_property,
        getter = location_writable_attribute_getter_callback,
        setter = location_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable,
        dont_delete
    )]
    href: (),
    #[webapi(
        accessor_property,
        getter = location_writable_attribute_getter_callback,
        setter = location_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable,
        dont_delete
    )]
    hash: (),
    #[webapi(
        accessor_property,
        getter = location_writable_attribute_getter_callback,
        setter = location_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable,
        dont_delete
    )]
    search: (),
    #[webapi(
        accessor_property,
        getter = location_writable_attribute_getter_callback,
        setter = location_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable,
        dont_delete
    )]
    pathname: (),
    #[webapi(
        accessor_property,
        getter = location_writable_attribute_getter_callback,
        setter = location_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 4),
        enumerable,
        dont_delete
    )]
    protocol: (),
    #[webapi(
        accessor_property,
        getter = location_writable_attribute_getter_callback,
        setter = location_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 5),
        enumerable,
        dont_delete
    )]
    host: (),
    #[webapi(
        accessor_property,
        getter = location_writable_attribute_getter_callback,
        setter = location_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 6),
        enumerable,
        dont_delete
    )]
    hostname: (),
    #[webapi(
        accessor_property,
        getter = location_writable_attribute_getter_callback,
        setter = location_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 7),
        enumerable,
        dont_delete
    )]
    port: (),
    #[webapi(
        method,
        callback = location_assign_callback,
        length = 1,
        enumerable,
        readonly,
        dont_delete
    )]
    assign: (),
    #[webapi(
        method,
        callback = location_replace_callback,
        length = 1,
        enumerable,
        readonly,
        dont_delete
    )]
    replace: (),
    #[webapi(
        method,
        callback = location_reload_callback,
        length = 0,
        enumerable,
        readonly,
        dont_delete
    )]
    reload: (),
    #[webapi(
        method,
        callback = location_to_string_callback,
        length = 0,
        enumerable,
        readonly,
        dont_delete
    )]
    to_string: (),
}

pub(in crate::context_bootstrap) fn build_location_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let template = v8::FunctionTemplate::builder(illegal_constructor_callback)
        .length(0)
        .build(scope);
    let instance = template.instance_template(scope);
    configure_location_instance_template(scope, instance);
    template
}

fn configure_location_instance_template(
    scope: &mut v8::PinScope<'_, '_, ()>,
    template: v8::Local<'_, v8::ObjectTemplate>,
) {
    let locked_property_attributes = || {
        v8::PropertyAttribute::READ_ONLY
            | v8::PropertyAttribute::DONT_ENUM
            | v8::PropertyAttribute::DONT_DELETE
    };
    template.set_intrinsic_data_property(
        v8str(scope, "valueOf").into(),
        v8::Intrinsic::ObjProtoValueOf,
        locked_property_attributes(),
    );
    template.set_with_attr(
        v8::Symbol::get_to_primitive(scope).into(),
        v8::undefined(scope).into(),
        locked_property_attributes(),
    );
    // Location is a named-interceptor exotic object in Chromium as part of
    // its cross-origin surface. Even when the same-origin getter declines to
    // intercept a name, V8 consequently implements Location's specified
    // [[PreventExtensions]] result: Object.preventExtensions throws and
    // Reflect.preventExtensions returns false.
    template.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(location_same_origin_named_property_getter)
            .flags(v8::PropertyHandlerFlags::ONLY_INTERCEPT_STRINGS),
    );
    template.set_immutable_proto();
}

pub(in crate::context_bootstrap) fn build_location_runtime_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    build_intrinsic_interface_instance(scope, "Location")
}

fn location_same_origin_named_property_getter(
    _scope: &mut v8::PinScope<'_, '_>,
    _key: v8::Local<'_, v8::Name>,
    _args: v8::PropertyCallbackArguments<'_>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    v8::Intercepted::kNo
}

pub(in crate::context_bootstrap) fn install_location_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    href: &str,
) -> Result<()> {
    sync_location_object_fields(scope, location, href);
    // Location's legacy-unforgeable own properties are non-configurable.
    // Window resets refresh the backing slots on the existing object without
    // redefining the fixed shape.
    if !location_own_surface_installed(scope, location) {
        LocationOwnSurfaceDeclaration::default()
            .initialize(scope, location)
            .map_err(|error| anyhow!("failed to initialize Location own surface: {error}"))?;
    }
    install_location_ancestor_origins_state(scope, location);
    Ok(())
}

fn install_location_ancestor_origins_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
) {
    let Some((local_window_id, document_id)) = current_location_owner_ids(scope, location) else {
        return;
    };
    if location_relevant_local_window_id_slot(scope, location) == Some(local_window_id)
        && location_relevant_document_id_slot(scope, location) == Some(document_id)
    {
        return;
    }
    // DOMStringList is otherwise a lazy interface. Record the relevant
    // Document now, but materialize its list only when script first reads it.
    clear_location_ancestor_origins_slot(scope, location);
    set_location_relevant_local_window_id_slot(scope, location, local_window_id);
    set_location_relevant_document_id_slot(scope, location, document_id);
}

fn new_location_dom_string_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
    values: &[String],
) -> v8::Local<'s, v8::Object> {
    let Some(relevant_context) = location.get_creation_context(scope) else {
        return new_dom_string_list(scope, values);
    };
    if relevant_context == scope.get_current_context() {
        return new_dom_string_list(scope, values);
    }
    let list = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let list = new_dom_string_list(target_scope, values);
        v8::Global::new(target_scope, list)
    };
    v8::Local::new(scope, &list)
}

fn current_location_document_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
) -> Option<(u64, u64, Vec<String>)> {
    let owner = runtime_window_owner(scope, location);
    let dispatch_scope = runtime_window_dispatch_scope(scope, owner)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    let host = unsafe { &*host_ptr };
    let (local_window_id, document_id) =
        location_owner_ids_for_dispatch_scope(host, dispatch_scope)?;
    let ancestor_origins = match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Child(handle) => {
            host.child_browsing_context_ancestor_origins(handle)?
        }
        crate::native_bridge::OwnerDispatchScope::Top
        | crate::native_bridge::OwnerDispatchScope::LightweightPopup(_) => Vec::new(),
    };
    Some((local_window_id, document_id, ancestor_origins))
}

fn current_location_owner_ids<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
) -> Option<(u64, u64)> {
    let owner = runtime_window_owner(scope, location);
    let dispatch_scope = runtime_window_dispatch_scope(scope, owner)?;
    let host_ptr = context_host_ptr_from_global_bridge(scope)?;
    location_owner_ids_for_dispatch_scope(unsafe { &*host_ptr }, dispatch_scope)
}

fn location_owner_ids_for_dispatch_scope(
    host: &crate::native_bridge::JsContextHost,
    dispatch_scope: crate::native_bridge::OwnerDispatchScope,
) -> Option<(u64, u64)> {
    match dispatch_scope {
        crate::native_bridge::OwnerDispatchScope::Top => host
            .current_main_document_task_owner()
            .map(|owner| (owner.local_window_id.0, owner.document_id.0)),
        crate::native_bridge::OwnerDispatchScope::Child(handle) => host
            .current_child_document_task_owner(handle)
            .map(|owner| (owner.local_window_id.0, owner.document_id.0)),
        crate::native_bridge::OwnerDispatchScope::LightweightPopup(popup_id) => Some((
            host.current_lightweight_popup_local_window_id(popup_id)?
                .as_u64(),
            host.current_lightweight_popup_document_owner(popup_id)?
                .document_id()
                .as_u64(),
        )),
    }
}

pub(in crate::context_bootstrap) fn location_belongs_to_current_local_window<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    location: v8::Local<'s, v8::Object>,
) -> bool {
    location_relevant_local_window_id_slot(scope, location).is_some_and(|local_window_id| {
        current_location_owner_ids(scope, location)
            .is_some_and(|(current_local_window_id, _)| current_local_window_id == local_window_id)
    })
}

pub(in crate::context_bootstrap) fn location_owner_has_current_realm<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(dispatch_scope) = runtime_window_dispatch_scope(scope, owner) else {
        return false;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    unsafe { &*host_ptr }
        .current_runtime_window_execution_context_identity_for_dispatch_scope(scope, dispatch_scope)
        .is_some()
}

fn location_ancestor_origins_for_holder<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    let relevant_owner = location_relevant_local_window_id_slot(scope, holder)
        .zip(location_relevant_document_id_slot(scope, holder));
    if let Some((local_window_id, document_id, ancestor_origins)) =
        current_location_document_state(scope, holder)
        && relevant_owner == Some((local_window_id, document_id))
    {
        if let Some(origins) = location_ancestor_origins_slot(scope, holder) {
            return origins;
        }
        let origins = new_location_dom_string_list(scope, holder, &ancestor_origins);
        set_location_ancestor_origins_slot(scope, holder, origins);
        // The Location can outlive its Document and realm. Cache its required
        // inactive value while that realm can still supply DOMStringList's
        // prototype, but only after script has requested ancestorOrigins.
        if location_empty_ancestor_origins_slot(scope, holder).is_none() {
            let empty = new_location_dom_string_list(scope, holder, &[]);
            set_location_empty_ancestor_origins_slot(scope, holder, empty);
        }
        return origins;
    }
    if let Some(empty) = location_empty_ancestor_origins_slot(scope, holder) {
        return empty;
    }
    let empty = new_location_dom_string_list(scope, holder, &[]);
    set_location_empty_ancestor_origins_slot(scope, holder, empty);
    empty
}

fn location_own_surface_installed(
    scope: &mut v8::PinScope<'_, '_>,
    location: v8::Local<'_, v8::Object>,
) -> bool {
    location
        .has_own_property(scope, v8str(scope, "href").into())
        .unwrap_or(false)
}

fn location_readonly_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        LOCATION_READONLY_ATTRIBUTES,
        "Location readonly attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    location_attribute_getter(scope, args.this(), attribute, &mut rv);
}

fn location_writable_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        LOCATION_WRITABLE_ATTRIBUTES,
        "Location writable attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    location_attribute_getter(scope, args.this(), attribute, &mut rv);
}

fn location_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    holder: v8::Local<'s, v8::Object>,
    attribute: LocationAttribute,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    let Some(current_href) = require_location_href_slot(scope, holder) else {
        return;
    };
    match attribute {
        LocationAttribute::AncestorOrigins => {
            rv.set(location_ancestor_origins_for_holder(scope, holder).into());
        }
        LocationAttribute::Href => {
            set_return_string(scope, rv, &current_href);
        }
        LocationAttribute::Hash => {
            let hash = url::Url::parse(&current_href)
                .ok()
                .map(|url| location_hash_string(&url))
                .unwrap_or_default();
            set_return_string(scope, rv, &hash);
        }
        LocationAttribute::Search => {
            let search = url::Url::parse(&current_href)
                .ok()
                .and_then(|url| url.query().map(|query| format!("?{query}")))
                .unwrap_or_default();
            set_return_string(scope, rv, &search);
        }
        LocationAttribute::Pathname => {
            let pathname = location_href_slot(scope, holder)
                .and_then(|href| url::Url::parse(&href).ok())
                .map(|url| url.path().to_owned())
                .unwrap_or_default();
            set_return_string(scope, rv, &pathname);
        }
        LocationAttribute::Protocol => {
            let protocol = parsed_location_url(scope, holder)
                .map(|url| format!("{}:", url.scheme()))
                .unwrap_or_default();
            set_return_string(scope, rv, &protocol);
        }
        LocationAttribute::Origin => {
            let owner = runtime_window_owner(scope, holder);
            let origin = if navigation_document_has_opaque_origin(scope, owner) {
                "null".to_owned()
            } else {
                parsed_location_url(scope, holder)
                    .map(|url| moli_url::origin_ascii_serialization(&url))
                    .unwrap_or_default()
            };
            set_return_string(scope, rv, &origin);
        }
        LocationAttribute::Host => {
            let host = parsed_location_url(scope, holder)
                .map(|url| location_host_string(&url))
                .unwrap_or_default();
            set_return_string(scope, rv, &host);
        }
        LocationAttribute::Hostname => {
            let hostname = parsed_location_url(scope, holder)
                .and_then(|url| url.host_str().map(ToOwned::to_owned))
                .unwrap_or_default();
            set_return_string(scope, rv, &hostname);
        }
        LocationAttribute::Port => {
            let port = parsed_location_url(scope, holder)
                .and_then(|url| url.port().map(|port| port.to_string()))
                .unwrap_or_default();
            set_return_string(scope, rv, &port);
        }
    }
}

fn location_writable_attribute_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        LOCATION_WRITABLE_ATTRIBUTES,
        "Location writable attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    let Some(value) = v8_value_to_string(scope, args.get(0)) else {
        return;
    };
    let holder = args.this();
    if require_location_href_slot(scope, holder).is_none() {
        return;
    }
    match attribute {
        LocationAttribute::Href => {
            navigate_location_object(scope, holder, LocationNavigationKind::Assign, Some(value));
        }
        LocationAttribute::Hash => {
            let hash = if value.is_empty() {
                String::new()
            } else if value.starts_with('#') {
                value
            } else {
                format!("#{value}")
            };
            let current = location_href_slot(scope, holder).unwrap_or_default();
            let base = current
                .find('#')
                .map(|index| current[..index].to_owned())
                .unwrap_or_else(|| current.clone());
            let target = format!("{base}{hash}");
            if target == current {
                rv.set_undefined();
                return;
            }
            navigate_location_object(scope, holder, LocationNavigationKind::Assign, Some(target));
        }
        LocationAttribute::Search => {
            let query = value.strip_prefix('?').unwrap_or(&value).to_owned();
            navigate_modified_location_url(scope, holder, |current| {
                if value.is_empty() {
                    current.set_query(None);
                } else {
                    current.set_query(Some(&query));
                }
                true
            });
        }
        LocationAttribute::Pathname => {
            navigate_modified_location_url(scope, holder, |current| {
                current.set_path(&value);
                true
            });
        }
        LocationAttribute::Protocol => {
            let Some(scheme) = location_protocol_scheme(&value) else {
                crate::context_bootstrap::throw_dom_exception_value(
                    scope,
                    "The provided value is not a valid URL scheme.",
                    "SyntaxError",
                );
                return;
            };
            let Some(current_href) = location_href_slot(scope, holder) else {
                return;
            };
            let Ok(mut target) = url::Url::parse(&current_href) else {
                return;
            };
            // A syntactically valid scheme can still be incompatible with the
            // current URL (for example, changing an HTTP URL to `data`). URL's
            // scheme-state override treats that as a non-failing no-op.
            if target.set_scheme(&scheme).is_err() || !matches!(target.scheme(), "http" | "https") {
                return;
            }
            // Unlike the other component setters, protocol assignment must
            // navigate even when parsing leaves the serialized URL unchanged.
            navigate_location_object(
                scope,
                holder,
                LocationNavigationKind::Assign,
                Some(target.to_string()),
            );
        }
        LocationAttribute::Host => {
            navigate_modified_location_url(scope, holder, |current| {
                if value.is_empty() {
                    return false;
                }
                let Ok(parsed_host) = url::Url::parse(&format!("{}://{value}/", current.scheme()))
                else {
                    return false;
                };
                let Some(host) = parsed_host.host_str() else {
                    return false;
                };
                current.set_host(Some(host)).is_ok() && current.set_port(parsed_host.port()).is_ok()
            });
        }
        LocationAttribute::Hostname => {
            navigate_modified_location_url(scope, holder, |current| {
                !value.is_empty() && current.set_host(Some(&value)).is_ok()
            });
        }
        LocationAttribute::Port => {
            let port = value.parse::<u16>().ok();
            navigate_modified_location_url(scope, holder, |current| current.set_port(port).is_ok());
        }
        LocationAttribute::AncestorOrigins | LocationAttribute::Origin => {}
    }
    rv.set_undefined();
}

fn location_protocol_scheme(value: &str) -> Option<String> {
    // Basic URL parsing removes ASCII tab and newline code points before the
    // scheme-state override runs. The first colon terminates that state, so
    // values such as `https::::` and `http:gunk` both provide only the scheme
    // before their first colon.
    let normalized = value
        .chars()
        .filter(|character| !matches!(character, '\t' | '\n' | '\r'))
        .collect::<String>();
    let candidate = normalized.split(':').next().unwrap_or_default();
    let mut characters = candidate.chars();
    let first = characters.next()?;
    if !first.is_ascii_alphabetic()
        || !characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '-' | '.')
        })
    {
        return None;
    }
    Some(candidate.to_ascii_lowercase())
}

fn location_hash_string(url: &url::Url) -> String {
    match url.fragment() {
        Some(fragment) if !fragment.is_empty() => format!("#{fragment}"),
        Some(_) | None => String::new(),
    }
}

const LOCATION_READONLY_ATTRIBUTES: &[LocationAttribute] = &[
    LocationAttribute::AncestorOrigins,
    LocationAttribute::Origin,
];

const LOCATION_WRITABLE_ATTRIBUTES: &[LocationAttribute] = &[
    LocationAttribute::Href,
    LocationAttribute::Hash,
    LocationAttribute::Search,
    LocationAttribute::Pathname,
    LocationAttribute::Protocol,
    LocationAttribute::Host,
    LocationAttribute::Hostname,
    LocationAttribute::Port,
];
