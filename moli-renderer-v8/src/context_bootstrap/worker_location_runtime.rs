use super::*;
use crate::util::{callback_data_index_value, callback_data_item, get_private_object};
use anyhow::{Result, anyhow};
use moli_webapi_declare::WebApiObject;

const WORKER_LOCATION_DATA_SLOT: &str = "__moliWorkerLocationData";
const WORKER_LOCATION_PROPERTIES: &[&str] = &[
    "href", "origin", "protocol", "host", "hostname", "port", "pathname", "search", "hash",
];

#[derive(WebApiObject)]
#[webapi(interface = "WorkerLocation")]
struct WorkerLocationObjectDeclaration<'scope> {
    #[webapi(slot = WORKER_LOCATION_DATA_SLOT)]
    data: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerLocationGlobalDeclaration<'scope> {
    #[webapi(data_property = "location")]
    location: v8::Local<'scope, v8::Object>,
}

#[derive(Default, WebApiObject)]
#[webapi(unbranded, interface = "WorkerLocation", enumerable)]
struct WorkerLocationPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = worker_location_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    href: (),
    #[webapi(
        accessor_property,
        getter = worker_location_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    origin: (),
    #[webapi(
        accessor_property,
        getter = worker_location_getter_callback,
        data = callback_data_index_value(scope, 2),
        enumerable
    )]
    protocol: (),
    #[webapi(
        accessor_property,
        getter = worker_location_getter_callback,
        data = callback_data_index_value(scope, 3),
        enumerable
    )]
    host: (),
    #[webapi(
        accessor_property,
        getter = worker_location_getter_callback,
        data = callback_data_index_value(scope, 4),
        enumerable
    )]
    hostname: (),
    #[webapi(
        accessor_property,
        getter = worker_location_getter_callback,
        data = callback_data_index_value(scope, 5),
        enumerable
    )]
    port: (),
    #[webapi(
        accessor_property,
        getter = worker_location_getter_callback,
        data = callback_data_index_value(scope, 6),
        enumerable
    )]
    pathname: (),
    #[webapi(
        accessor_property,
        getter = worker_location_getter_callback,
        data = callback_data_index_value(scope, 7),
        enumerable
    )]
    search: (),
    #[webapi(
        accessor_property,
        getter = worker_location_getter_callback,
        data = callback_data_index_value(scope, 8),
        enumerable
    )]
    hash: (),
    #[webapi(method, length = 0, callback = worker_location_to_string_callback)]
    to_string: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WorkerLocationBackingDeclaration {
    #[webapi(data_property, enumerable)]
    href: String,
    #[webapi(data_property, enumerable)]
    origin: String,
    #[webapi(data_property, enumerable)]
    protocol: String,
    #[webapi(data_property, enumerable)]
    host: String,
    #[webapi(data_property, enumerable)]
    hostname: String,
    #[webapi(data_property, enumerable)]
    port: String,
    #[webapi(data_property, enumerable)]
    pathname: String,
    #[webapi(data_property, enumerable)]
    search: String,
    #[webapi(data_property, enumerable)]
    hash: String,
}

fn worker_location_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(backing) = worker_location_backing(scope, args.this()) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    let Some(key) = callback_data_item(
        scope,
        &args,
        WORKER_LOCATION_PROPERTIES,
        "WorkerLocation properties",
    ) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    match backing.get(scope, v8str(scope, key).into()) {
        Some(value) => rv.set(value),
        None => rv.set(v8::undefined(scope).into()),
    }
}

fn worker_location_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(backing) = worker_location_backing(scope, args.this()) else {
        rv.set(v8::undefined(scope).into());
        return;
    };
    match backing.get(scope, v8str(scope, "href").into()) {
        Some(value) => rv.set(value),
        None => rv.set(v8::undefined(scope).into()),
    }
}

fn worker_location_backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, object, WORKER_LOCATION_DATA_SLOT)
}

fn install_worker_location_prototype_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
) -> Result<()> {
    WorkerLocationPrototypeDeclaration::default().initialize(scope, prototype)?;
    Ok(())
}

fn worker_location_search(url: &url::Url) -> String {
    match url.query() {
        Some("") | None => String::new(),
        Some(query) => format!("?{query}"),
    }
}

fn worker_location_hash(url: &url::Url) -> String {
    match url.fragment() {
        Some(fragment) if !fragment.is_empty() => format!("#{fragment}"),
        Some(_) | None => String::new(),
    }
}

pub(crate) fn install_worker_location_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    script_url: &url::Url,
) -> Result<()> {
    install_worker_constructor(scope, global, "WorkerLocation")?;

    if let Some(prototype) = global_constructor_prototype(scope, "WorkerLocation") {
        install_worker_location_prototype_bindings(scope, prototype)?;
    }

    let backing =
        WorkerLocationBackingDeclaration::from(WorkerLocationBacking::from_url(script_url))
            .bind(scope)
            .expect("WorkerLocation backing declaration should bind");

    let location = WorkerLocationObjectDeclaration::new(backing)
        .bind(scope)
        .expect("WorkerLocation declaration should bind");
    WorkerLocationGlobalDeclaration::new(location)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize WorkerLocation global: {error}"))
}

struct WorkerLocationBacking<'a> {
    href: &'a str,
    origin: String,
    protocol: String,
    host: &'a str,
    hostname: &'a str,
    port: String,
    pathname: &'a str,
    search: String,
    hash: String,
}

impl<'a> WorkerLocationBacking<'a> {
    fn from_url(script_url: &'a url::Url) -> Self {
        Self {
            href: script_url.as_str(),
            origin: moli_url::origin_ascii_serialization(script_url),
            protocol: format!("{}:", script_url.scheme()),
            host: &script_url[url::Position::BeforeHost..url::Position::AfterPort],
            hostname: &script_url[url::Position::BeforeHost..url::Position::AfterHost],
            port: script_url
                .port()
                .map(|port| port.to_string())
                .unwrap_or_default(),
            pathname: &script_url[url::Position::BeforePath..url::Position::AfterPath],
            search: worker_location_search(script_url),
            hash: worker_location_hash(script_url),
        }
    }
}

impl From<WorkerLocationBacking<'_>> for WorkerLocationBackingDeclaration {
    fn from(backing: WorkerLocationBacking<'_>) -> Self {
        Self::new(
            backing.href.to_owned(),
            backing.origin,
            backing.protocol,
            backing.host.to_owned(),
            backing.hostname.to_owned(),
            backing.port,
            backing.pathname.to_owned(),
            backing.search,
            backing.hash,
        )
    }
}
