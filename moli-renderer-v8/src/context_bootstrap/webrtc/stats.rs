//! Native statistics snapshots. The peer connection is monitored immediately;
//! unnegotiated RTP endpoints have no SSRC or transport statistics to report.

use crate::{
    native_bridge::throw_dom_exception,
    page_task_queue::RendererPageWebRtcTaskKind,
    util::{get_private_object, get_private_value, v8str},
    web_api_interfaces, webidl,
    webidl_iterator::{
        MaplikeWebIdlIteratorMethod, call_live_maplike_webidl_for_each,
        new_live_maplike_webidl_iterator,
    },
};
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

pub(super) const ID: &str = "__moliRtcPeerStatsId";
const BACKING: &str = "__moliRtcStatsBacking";
const REPORT: &str = "__moliRtcStatsResult";
const RESOLVER: &str = "__moliRtcStatsResolver";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::RTCStatsReport, require_prototype)]
struct Report<'s> {
    #[webapi(slot = BACKING)]
    backing: v8::Local<'s, v8::Map>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct PeerStats<'s> {
    #[webapi(data_property, enumerable)]
    id: v8::Local<'s, v8::String>,
    #[webapi(data_property, enumerable)]
    timestamp: f64,
    #[webapi(data_property, enumerable)]
    r#type: v8::Local<'s, v8::String>,
    #[webapi(data_property, enumerable)]
    data_channels_opened: u32,
    #[webapi(data_property, enumerable)]
    data_channels_closed: u32,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct Task<'s> {
    #[webapi(slot = REPORT)]
    report: v8::Local<'s, v8::Object>,
    #[webapi(slot = RESOLVER)]
    resolver: v8::Local<'s, v8::PromiseResolver>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnection.getStats")]
struct PeerArgs<'s> {
    #[webidl(nullable, interface = web_api_interfaces::MediaStreamTrack)]
    selector: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCStatsReport lookup")]
struct KeyArgs {
    // WebIDL maplike lookups convert a missing key to "undefined".
    #[webidl(converter = "raw", default = webidl::DomString16("undefined".encode_utf16().collect()))]
    key: webidl::DomString16,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCStatsReport.forEach")]
struct ForEachArgs<'s> {
    #[webidl(required)]
    callback: v8::Local<'s, v8::Value>,
    #[webidl(default = v8::undefined(scope).into())]
    this_arg: v8::Local<'s, v8::Value>,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::RTCStatsReport, enumerable, receiver)]
struct Prototype {
    #[webapi(accessor_property, getter = size)]
    size: (),
    #[webapi(method, length = 1, callback = get)]
    get: (),
    #[webapi(method, length = 1, callback = has)]
    has: (),
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

pub(super) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    prototype: v8::Local<'s, v8::ObjectTemplate>,
) {
    Prototype::initialize_prototype_template(scope, prototype);
}

fn snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Object>> {
    let map = v8::Map::new(scope);
    if let Some(pc) = pc {
        let id = v8::Local::<v8::String>::try_from(
            get_private_value(scope, pc, ID).expect("native PC stats identity"),
        )
        .expect("stats identity string");
        // Use the host's monotonic epoch clock, never an author-replaceable
        // performance/Date property or a new wall-clock reading per report.
        let timestamp =
            moli_time::coarsened_dom_time_millis(moli_time::monotonic_timestamp_seconds() * 1000.0);
        // No channel can reach "open" before transport negotiation is implemented.
        let value = PeerStats::new(id, timestamp, v8str(scope, "peer-connection"), 0, 0)
            .bind(scope)
            .ok()?;
        map.set(scope, id.into(), value.into())?;
    }
    Report::new(map).bind(scope).ok()
}

fn gather<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    pc: Option<v8::Local<'s, v8::Object>>,
) -> Option<v8::Local<'s, v8::Promise>> {
    let report = snapshot(scope, pc)?;
    let resolver = v8::PromiseResolver::new(scope)?;
    let task = Task::new(report, resolver).bind(scope).ok()?;
    // Unlike operations-chain tasks, getStats belongs to the callee's global
    // and is valid after close. The request captures that exact Document owner.
    super::operations::queue(scope, task, task, RendererPageWebRtcTaskKind::GetStats);
    Some(resolver.get_promise(scope))
}

pub(super) fn peer_get_stats<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<PeerArgs>(scope, &args) else {
        return;
    };
    let pc = super::rtp_transceivers::target(scope, args.this());
    if let Some(track) = parsed.selector
        && super::rtp_transceivers::stats_selector_matches(scope, pc, track) != 1
    {
        throw_dom_exception(
            scope,
            "InvalidAccessError",
            15,
            "The track must identify exactly one sender or receiver on this connection.",
        );
        return;
    }
    if let Some(promise) = gather(scope, parsed.selector.is_none().then_some(pc)) {
        rv.set(promise.into());
    }
}

pub(super) fn rtp_get_stats<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(promise) = gather(scope, None) {
        rv.set(promise.into());
    }
}

pub(super) fn apply<'s>(scope: &mut v8::PinScope<'s, '_>, task: v8::Local<'s, v8::Object>) -> bool {
    let report = get_private_object(scope, task, REPORT).expect("stats report");
    let resolver = get_private_object(scope, task, RESOLVER).expect("stats resolver");
    // The native Task declaration is the only writer of this private slot.
    let resolver = unsafe { v8::Local::<v8::PromiseResolver>::cast_unchecked(resolver) };
    resolver.resolve(scope, report.into()) == Some(true)
}

fn backing<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Map> {
    let object = super::rtp_transceivers::target(scope, receiver);
    v8::Local::<v8::Map>::try_from(
        get_private_object(scope, object, BACKING).expect("native stats backing"),
    )
    .expect("stats map")
}

fn size<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let size = backing(scope, args.this()).size();
    rv.set(v8::Number::new(scope, size as f64).into());
}

fn key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<v8::Local<'s, v8::String>> {
    let parsed = webidl::parse_args::<KeyArgs>(scope, args)?;
    v8::String::new_from_two_byte(scope, &parsed.key.0, v8::NewStringType::Normal)
}

fn get<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(key) = key(scope, &args) else {
        return;
    };
    if let Some(value) = backing(scope, args.this()).get(scope, key.into()) {
        rv.set(value);
    }
}

fn has<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(key) = key(scope, &args) else {
        return;
    };
    if let Some(value) = backing(scope, args.this()).has(scope, key.into()) {
        rv.set_bool(value);
    }
}

fn for_each<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
) {
    let Some(parsed) = webidl::parse_args::<ForEachArgs>(scope, &args) else {
        return;
    };
    let map = backing(scope, args.this());
    if let Some(result) = call_live_maplike_webidl_for_each(
        scope,
        map,
        args.this(),
        parsed.callback,
        parsed.this_arg,
        "RTCStatsReport.forEach",
    ) {
        rv.set(result);
    }
}

fn iterator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s>,
    method: MaplikeWebIdlIteratorMethod,
) {
    let map = backing(scope, args.this());
    if let Some(value) = new_live_maplike_webidl_iterator(scope, map, method) {
        rv.set(value.into());
    }
}

fn entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    iterator(scope, args, rv, MaplikeWebIdlIteratorMethod::Entries);
}
fn keys<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    iterator(scope, args, rv, MaplikeWebIdlIteratorMethod::Keys);
}
fn values<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue<'s>,
) {
    iterator(scope, args, rv, MaplikeWebIdlIteratorMethod::Values);
}
