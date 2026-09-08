use super::helpers::{require_url_receiver, url_href_slot};
use super::*;
use crate::util::{callback_data_index_value, callback_data_item, get_private_value};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(Clone, Copy)]
enum UrlAttribute {
    Href,
    Origin,
    Protocol,
    Username,
    Password,
    Host,
    Hostname,
    Port,
    Pathname,
    Search,
    SearchParams,
    Hash,
}

impl UrlAttribute {
    fn idl_name(self) -> &'static str {
        match self {
            UrlAttribute::Href => "href",
            UrlAttribute::Origin => "origin",
            UrlAttribute::Protocol => "protocol",
            UrlAttribute::Username => "username",
            UrlAttribute::Password => "password",
            UrlAttribute::Host => "host",
            UrlAttribute::Hostname => "hostname",
            UrlAttribute::Port => "port",
            UrlAttribute::Pathname => "pathname",
            UrlAttribute::Search => "search",
            UrlAttribute::SearchParams => "searchParams",
            UrlAttribute::Hash => "hash",
        }
    }
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::URL, enumerable)]
struct UrlPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = url_writable_attribute_getter_callback,
        setter = url_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    href: (),
    #[webapi(
        accessor_property,
        getter = url_writable_attribute_getter_callback,
        setter = url_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    protocol: (),
    #[webapi(
        accessor_property,
        getter = url_writable_attribute_getter_callback,
        setter = url_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    username: (),
    #[webapi(
        accessor_property,
        getter = url_writable_attribute_getter_callback,
        setter = url_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    password: (),
    #[webapi(
        accessor_property,
        getter = url_writable_attribute_getter_callback,
        setter = url_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 4),
        enumerable
    )]
    host: (),
    #[webapi(
        accessor_property,
        getter = url_writable_attribute_getter_callback,
        setter = url_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 5),
        enumerable
    )]
    hostname: (),
    #[webapi(
        accessor_property,
        getter = url_writable_attribute_getter_callback,
        setter = url_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 6),
        enumerable
    )]
    port: (),
    #[webapi(
        accessor_property,
        getter = url_writable_attribute_getter_callback,
        setter = url_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 7),
        enumerable
    )]
    pathname: (),
    #[webapi(
        accessor_property,
        getter = url_writable_attribute_getter_callback,
        setter = url_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 8),
        enumerable
    )]
    search: (),
    #[webapi(
        accessor_property,
        getter = url_writable_attribute_getter_callback,
        setter = url_writable_attribute_setter_callback,
        data = callback_data_index_value(scope, 9),
        enumerable
    )]
    hash: (),
    #[webapi(
        accessor_property,
        getter = url_readonly_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    origin: (),
    #[webapi(
        accessor_property,
        getter = url_readonly_attribute_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    search_params: (),
}

pub(in crate::context_bootstrap::url_form) fn initialize_url_prototype_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    UrlPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
}

fn url_readonly_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        URL_READONLY_ATTRIBUTES,
        "URL readonly attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    let Some(this) = require_url_receiver(scope, args.this()) else {
        return;
    };
    url_attribute_getter(scope, this, attribute, &mut rv);
}

fn url_writable_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        URL_WRITABLE_ATTRIBUTES,
        "URL writable attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    let Some(this) = require_url_receiver(scope, args.this()) else {
        return;
    };
    url_attribute_getter(scope, this, attribute, &mut rv);
}

fn url_attribute_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    this: v8::Local<'s, v8::Object>,
    attribute: UrlAttribute,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
) {
    match attribute {
        UrlAttribute::Href => {
            let href = url_href_slot(scope, this).unwrap_or_default();
            set_return_string(scope, rv, &href);
        }
        UrlAttribute::Origin => {
            let origin = url_object_value(scope, this)
                .map(|url| moli_url::origin_ascii_serialization(&url))
                .unwrap_or_default();
            set_return_string(scope, rv, &origin);
        }
        UrlAttribute::Protocol => {
            let protocol = url_object_value(scope, this)
                .map(|url| format!("{}:", url.scheme()))
                .unwrap_or_default();
            set_return_string(scope, rv, &protocol);
        }
        UrlAttribute::Username => {
            let username = url_object_value(scope, this)
                .map(|url| url.username().to_owned())
                .unwrap_or_default();
            set_return_string(scope, rv, &username);
        }
        UrlAttribute::Password => {
            let password = url_object_value(scope, this)
                .map(|url| url.password().unwrap_or_default().to_owned())
                .unwrap_or_default();
            set_return_string(scope, rv, &password);
        }
        UrlAttribute::Host => {
            let host = url_object_value(scope, this)
                .map(|url| {
                    url.host_str()
                        .map(|host| {
                            url.port()
                                .map(|port| format!("{host}:{port}"))
                                .unwrap_or_else(|| host.to_owned())
                        })
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            set_return_string(scope, rv, &host);
        }
        UrlAttribute::Hostname => {
            let hostname = url_object_value(scope, this)
                .and_then(|url| url.host_str().map(ToOwned::to_owned))
                .unwrap_or_default();
            set_return_string(scope, rv, &hostname);
        }
        UrlAttribute::Port => {
            let port = url_object_value(scope, this)
                .and_then(|url| url.port().map(|port| port.to_string()))
                .unwrap_or_default();
            set_return_string(scope, rv, &port);
        }
        UrlAttribute::Pathname => {
            let pathname = url_object_value(scope, this)
                .map(|url| url.path().to_owned())
                .unwrap_or_default();
            set_return_string(scope, rv, &pathname);
        }
        UrlAttribute::Search => {
            let search = url_object_value(scope, this)
                .map(|url| url::quirks::search(&url).to_owned())
                .unwrap_or_default();
            set_return_string(scope, rv, &search);
        }
        UrlAttribute::SearchParams => {
            let value = get_private_value(scope, this, URL_SEARCH_PARAMS_SLOT)
                .unwrap_or_else(|| v8::undefined(scope).into());
            rv.set(value);
        }
        UrlAttribute::Hash => {
            let hash = url_object_value(scope, this)
                .map(|url| url::quirks::hash(&url).to_owned())
                .unwrap_or_default();
            set_return_string(scope, rv, &hash);
        }
    }
}

fn url_writable_attribute_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        URL_WRITABLE_ATTRIBUTES,
        "URL writable attributes",
    ) else {
        return;
    };
    let Some(this) = require_url_receiver(scope, args.this()) else {
        return;
    };
    // WebIDL conversion can run script that changes this URL. Read its current
    // record only after conversion, rather than overwriting those side effects.
    let Some(value) = url_attribute_usv_string(scope, args.get(0), attribute) else {
        return;
    };
    if matches!(attribute, UrlAttribute::Href) {
        match url::Url::parse(&value) {
            Ok(url) => apply_url_update(scope, this, &url),
            Err(_) => throw_type_error(
                scope,
                "Failed to set the 'href' property on 'URL': Invalid URL.",
            ),
        }
        return;
    }
    let Some(mut url) = url_object_value(scope, this) else {
        return;
    };
    let applied = match attribute {
        UrlAttribute::Protocol => url::quirks::set_protocol(&mut url, &value).is_ok(),
        UrlAttribute::Username => url::quirks::set_username(&mut url, &value).is_ok(),
        UrlAttribute::Password => url::quirks::set_password(&mut url, &value).is_ok(),
        UrlAttribute::Host => {
            if url.cannot_be_a_base() {
                return;
            }
            moli_url::components::set_host(&mut url, &value);
            true
        }
        UrlAttribute::Hostname => {
            moli_url::components::set_hostname(&mut url, &value);
            true
        }
        UrlAttribute::Port => {
            moli_url::components::set_port(&mut url, &value);
            true
        }
        UrlAttribute::Pathname => {
            if url.cannot_be_a_base() {
                return;
            }
            moli_url::components::set_pathname(&mut url, &value);
            true
        }
        UrlAttribute::Search => {
            url::quirks::set_search(&mut url, &value);
            true
        }
        UrlAttribute::Hash => {
            url::quirks::set_hash(&mut url, &value);
            true
        }
        UrlAttribute::Href | UrlAttribute::Origin | UrlAttribute::SearchParams => false,
    };
    if applied {
        apply_url_update(scope, this, &url);
    }
    rv.set_undefined();
}

fn url_attribute_usv_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
    attribute: UrlAttribute,
) -> Option<String> {
    match webidl::convert::<webidl::UsvString>(
        scope,
        value,
        webidl::Context::member("URL", attribute.idl_name()),
    ) {
        Ok(value) => Some(value.0),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}

fn set_return_string(
    scope: &mut v8::PinScope<'_, '_>,
    rv: &mut v8::ReturnValue<'_, v8::Value>,
    value: &str,
) {
    if let Some(value) = v8_string(scope, value) {
        rv.set(value.into());
    } else {
        rv.set_empty_string();
    }
}

const URL_WRITABLE_ATTRIBUTES: &[UrlAttribute] = &[
    UrlAttribute::Href,
    UrlAttribute::Protocol,
    UrlAttribute::Username,
    UrlAttribute::Password,
    UrlAttribute::Host,
    UrlAttribute::Hostname,
    UrlAttribute::Port,
    UrlAttribute::Pathname,
    UrlAttribute::Search,
    UrlAttribute::Hash,
];

const URL_READONLY_ATTRIBUTES: &[UrlAttribute] =
    &[UrlAttribute::Origin, UrlAttribute::SearchParams];
