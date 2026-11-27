//! MediaQueryList wrappers. The host evaluates their live matches getter.

use crate::interfaces as web_api_interfaces;
use moli_webapi_declare::WebApiObject;

pub const MEDIA_QUERY_LIST_MEDIA_SLOT: &str = "__moliMediaQueryListMedia";
pub const MEDIA_QUERY_LIST_ONCHANGE_SLOT: &str = "__moliMediaQueryListOnchange";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaQueryList)]
struct MediaQueryListObjectDeclaration {
    #[webapi(slot = MEDIA_QUERY_LIST_MEDIA_SLOT)]
    media: String,
    #[webapi(slot = MEDIA_QUERY_LIST_ONCHANGE_SLOT, init = "null")]
    onchange: (),
}

pub fn new_media_query_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    media: String,
) -> v8::Local<'s, v8::Object> {
    MediaQueryListObjectDeclaration::new(media)
        .bind(scope)
        .expect("MediaQueryList declaration should bind")
}
