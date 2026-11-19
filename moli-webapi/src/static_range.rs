//! StaticRange boundary storage for already-resolved node wrappers.

use crate::interfaces as web_api_interfaces;
use moli_webapi_declare::WebApiObject;
use moli_webidl::{self as webidl, WebIdlDictionarySerialize};

pub const STATIC_RANGE_START_CONTAINER_STORAGE_KEY: &str = "__moliStaticRangeStartContainer";
pub const STATIC_RANGE_START_OFFSET_STORAGE_KEY: &str = "__moliStaticRangeStartOffset";
pub const STATIC_RANGE_END_CONTAINER_STORAGE_KEY: &str = "__moliStaticRangeEndContainer";
pub const STATIC_RANGE_END_OFFSET_STORAGE_KEY: &str = "__moliStaticRangeEndOffset";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::AbstractRange)]
struct StaticRangeBoundaryStorageDeclaration {}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "StaticRangeInit", snapshot)]
pub struct StaticRangeInitMembers<'s> {
    #[webidl(required, name = "startContainer", interface = web_api_interfaces::Node)]
    #[webidl(snapshot_slot = STATIC_RANGE_START_CONTAINER_STORAGE_KEY)]
    pub start_container: v8::Local<'s, v8::Object>,
    #[webidl(required, name = "startOffset")]
    #[webidl(snapshot_slot = STATIC_RANGE_START_OFFSET_STORAGE_KEY)]
    pub start_offset: u32,
    #[webidl(required, name = "endContainer", interface = web_api_interfaces::Node)]
    #[webidl(snapshot_slot = STATIC_RANGE_END_CONTAINER_STORAGE_KEY)]
    pub end_container: v8::Local<'s, v8::Object>,
    #[webidl(required, name = "endOffset")]
    #[webidl(snapshot_slot = STATIC_RANGE_END_OFFSET_STORAGE_KEY)]
    pub end_offset: u32,
}

pub fn initialize_static_range_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    range: v8::Local<'s, v8::Object>,
    start_container: v8::Local<'s, v8::Object>,
    start_offset: u32,
    end_container: v8::Local<'s, v8::Object>,
    end_offset: u32,
) {
    StaticRangeInitMembers {
        start_container,
        start_offset,
        end_container,
        end_offset,
    }
    .initialize_snapshot(scope, range)
    .expect("StaticRange boundaries");
    StaticRangeBoundaryStorageDeclaration::new()
        .initialize(scope, range)
        .expect("StaticRange boundary declaration should initialize object");
}
