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
                .and_then(|url| url.query().map(|query| format!("?{query}")))
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
                .and_then(|url| url.fragment().map(|fragment| format!("#{fragment}")))
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
        rv.set_undefined();
        return;
    };
    let Some(this) = require_url_receiver(scope, args.this()) else {
        return;
    };
    match attribute {
        UrlAttribute::Href => {
            let Some(href) = url_attribute_usv_string(scope, args.get(0), attribute) else {
                return;
            };
            match url::Url::parse(&href) {
                Ok(url) => apply_url_update(scope, this, &url),
                Err(_) => throw_type_error(
                    scope,
                    "Failed to set the 'href' property on 'URL': Invalid URL.",
                ),
            }
        }
        UrlAttribute::Protocol => {
            let Some(mut url) = url_object_value(scope, this) else {
                rv.set_undefined();
                return;
            };
            let Some(protocol) = url_attribute_usv_string(scope, args.get(0), attribute) else {
                rv.set_undefined();
                return;
            };
            let scheme = protocol.trim_end_matches(':');
            if !scheme.is_empty() && url.set_scheme(scheme).is_ok() {
                apply_url_update(scope, this, &url);
            }
        }
        UrlAttribute::Username => {
            if let Some(mut url) = url_object_value(scope, this)
                && let Some(username) = url_attribute_usv_string(scope, args.get(0), attribute)
                && url.set_username(&username).is_ok()
            {
                apply_url_update(scope, this, &url);
            }
        }
        UrlAttribute::Password => {
            if let Some(mut url) = url_object_value(scope, this)
                && let Some(password) = url_attribute_usv_string(scope, args.get(0), attribute)
                && url.set_password(Some(&password)).is_ok()
            {
                apply_url_update(scope, this, &url);
            }
        }
        UrlAttribute::Host => {
            if let Some(mut url) = url_object_value(scope, this)
                && let Some(host) = url_attribute_usv_string(scope, args.get(0), attribute)
            {
                if host.is_empty() {
                    rv.set_undefined();
                    return;
                }
                let parsed = if host.starts_with('[') {
                    let Some(end_bracket) = host.find(']') else {
                        rv.set_undefined();
                        return;
                    };
                    let hostname_part = &host[..=end_bracket];
                    let suffix = &host[end_bracket + 1..];
                    if suffix.is_empty() {
                        Some((hostname_part, None))
                    } else {
                        suffix
                            .strip_prefix(':')
                            .map(|port_part| (hostname_part, Some(port_part)))
                    }
                } else if let Some(colon_idx) = host.rfind(':') {
                    let hostname_part = &host[..colon_idx];
                    let port_part = &host[colon_idx + 1..];
                    if hostname_part.contains(':') {
                        Some((host.as_str(), None))
                    } else if hostname_part.is_empty() {
                        None
                    } else {
                        Some((hostname_part, Some(port_part)))
                    }
                } else {
                    Some((host.as_str(), None))
                };
                let Some((hostname_part, explicit_port)) = parsed else {
                    rv.set_undefined();
                    return;
                };
                if url.set_host(Some(hostname_part)).is_ok() {
                    let port_result = match explicit_port {
                        Some("") => url.set_port(None),
                        Some(port_part) => match port_part.parse::<u16>() {
                            Ok(port) => url.set_port(Some(port)),
                            Err(_) => {
                                rv.set_undefined();
                                return;
                            }
                        },
                        None => url.set_port(None),
                    };
                    if port_result.is_ok() {
                        apply_url_update(scope, this, &url);
                    }
                }
            }
        }
        UrlAttribute::Hostname => {
            if let Some(mut url) = url_object_value(scope, this)
                && let Some(hostname) = url_attribute_usv_string(scope, args.get(0), attribute)
                && !hostname.is_empty()
            {
                let port = url.port();
                if url.set_host(Some(&hostname)).is_ok() {
                    let _ = url.set_port(port);
                    apply_url_update(scope, this, &url);
                } else {
                    let candidate = format!(
                        "{}{}{}",
                        &url[..url::Position::BeforeHost],
                        hostname,
                        &url[url::Position::AfterHost..]
                    );
                    if let Ok(next_url) = url::Url::parse(&candidate) {
                        apply_url_update(scope, this, &next_url);
                    }
                }
            }
        }
        UrlAttribute::Port => {
            if let Some(mut url) = url_object_value(scope, this)
                && let Some(port) = url_attribute_usv_string(scope, args.get(0), attribute)
            {
                let updated = if port.is_empty() {
                    url.set_port(None)
                } else if let Ok(parsed) = port.parse::<u16>() {
                    url.set_port(Some(parsed))
                } else {
                    rv.set_undefined();
                    return;
                };
                if updated.is_ok() {
                    apply_url_update(scope, this, &url);
                }
            }
        }
        UrlAttribute::Pathname => {
            if let Some(mut url) = url_object_value(scope, this)
                && let Some(mut pathname) = url_attribute_usv_string(scope, args.get(0), attribute)
            {
                if !pathname.starts_with('/') {
                    pathname.insert(0, '/');
                }
                url.set_path(&pathname);
                apply_url_update(scope, this, &url);
            }
        }
        UrlAttribute::Search => {
            if let Some(mut url) = url_object_value(scope, this)
                && let Some(search) = url_attribute_usv_string(scope, args.get(0), attribute)
            {
                if search.is_empty() {
                    url.set_query(None);
                } else {
                    url.set_query(Some(search.trim_start_matches('?')));
                }
                apply_url_update(scope, this, &url);
            }
        }
        UrlAttribute::Hash => {
            if let Some(mut url) = url_object_value(scope, this)
                && let Some(hash) = url_attribute_usv_string(scope, args.get(0), attribute)
            {
                if hash.is_empty() {
                    url.set_fragment(None);
                } else {
                    url.set_fragment(Some(hash.trim_start_matches('#')));
                }
                apply_url_update(scope, this, &url);
            }
        }
        UrlAttribute::Origin | UrlAttribute::SearchParams => {}
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
