use super::super::navigation_window::{
    navigation_document_has_opaque_origin, runtime_window_owner,
};
use super::super::window_origin_runtime_state;
use super::helpers::{
    location_host_string, navigate_modified_location_url, parsed_location_url,
    require_location_href_slot, set_return_string, v8_value_to_string,
};
use super::methods::{
    location_assign_callback, location_reload_callback, location_replace_callback,
    location_to_string_callback,
};
use super::slots::{location_href_slot, sync_location_object_fields};
use super::*;
use crate::util::{callback_data_index_value, callback_data_item};
use anyhow::{Result, anyhow};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

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

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Location", constructor = "illegal", constructor_length = 0)]
struct LocationConstructorTemplateDeclaration {}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct LocationConstructorGlobalDeclaration<'scope> {
    #[webapi(data_property = "Location")]
    constructor: v8::Local<'scope, v8::Function>,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Location")]
struct LocationPrototypeMetadataDeclaration {
    #[webapi(to_string_tag, init = string("Location"))]
    to_string_tag: (),
}

pub(in crate::context_bootstrap) fn ensure_location_constructor_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    if global_constructor_prototype(scope, "Location").is_some() {
        return Ok(());
    }

    let template = LocationConstructorTemplateDeclaration::build(scope);
    let constructor = template
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to build Location constructor"))?;
    LocationConstructorGlobalDeclaration::new(constructor)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize Location constructor global: {error}"))?;

    let prototype = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("failed to extract Location.prototype"))?;
    LocationPrototypeMetadataDeclaration::default()
        .initialize(scope, prototype)
        .map_err(|error| anyhow!("failed to initialize Location prototype metadata: {error}"))?;
    Ok(())
}

pub(in crate::context_bootstrap) fn build_location_runtime_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>> {
    let template = v8::ObjectTemplate::new(scope);
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
    template
        .new_instance(scope)
        .ok_or_else(|| anyhow!("failed to instantiate Location object template"))
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
    if let Some(prototype) = global_constructor_prototype(scope, "Location") {
        let _ = location.set_prototype(scope, prototype.into());
    }
    sync_location_object_fields(scope, location, href);
    // Location's legacy-unforgeable own properties are non-configurable.
    // Window resets refresh the backing slots on the existing object without
    // redefining the fixed shape.
    if !location_own_surface_installed(scope, location) {
        LocationOwnSurfaceDeclaration::default()
            .initialize(scope, location)
            .map_err(|error| anyhow!("failed to initialize Location own surface: {error}"))?;
    }
    Ok(())
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
        LocationAttribute::AncestorOrigins => rv.set(v8::Array::new(scope, 0).into()),
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
                    .map(|url| {
                        if moli_url::is_about_blank(&url) {
                            window_origin_runtime_state(scope, owner)
                                .unwrap_or_else(|| moli_url::origin_ascii_serialization(&url))
                        } else {
                            moli_url::origin_ascii_serialization(&url)
                        }
                    })
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
            let scheme = value.strip_suffix(':').unwrap_or(&value).to_owned();
            navigate_modified_location_url(scope, holder, |current| {
                current.set_scheme(&scheme).is_ok()
            });
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
