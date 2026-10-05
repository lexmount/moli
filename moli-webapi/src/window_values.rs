//! Window surface objects; the embedding supplies live viewport and event state.

pub mod options;

use crate::interfaces as web_api_interfaces;
use moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE;
use moli_webapi_declare::{BindError, WebApiObject};

pub const SCREEN_WIDTH_SLOT: &str = "__moliScreenWidth";
pub const SCREEN_HEIGHT_SLOT: &str = "__moliScreenHeight";
pub const SCREEN_AVAIL_WIDTH_SLOT: &str = "__moliScreenAvailWidth";
pub const SCREEN_AVAIL_HEIGHT_SLOT: &str = "__moliScreenAvailHeight";
pub const SCREEN_AVAIL_LEFT_SLOT: &str = "__moliScreenAvailLeft";
pub const SCREEN_AVAIL_TOP_SLOT: &str = "__moliScreenAvailTop";
pub const SCREEN_COLOR_DEPTH_SLOT: &str = "__moliScreenColorDepth";
pub const SCREEN_PIXEL_DEPTH_SLOT: &str = "__moliScreenPixelDepth";
pub const SCREEN_ORIENTATION_TYPE_SLOT: &str = "__moliScreenOrientationType";
pub const SCREEN_ORIENTATION_ANGLE_SLOT: &str = "__moliScreenOrientationAngle";
pub const SCREEN_ORIENTATION_ONCHANGE_SLOT: &str = "__moliScreenOrientationOnchange";
pub const VISUAL_VIEWPORT_OFFSET_LEFT_SLOT: &str = "__moliVisualViewportOffsetLeft";
pub const VISUAL_VIEWPORT_OFFSET_TOP_SLOT: &str = "__moliVisualViewportOffsetTop";
pub const VISUAL_VIEWPORT_PAGE_LEFT_SLOT: &str = "__moliVisualViewportPageLeft";
pub const VISUAL_VIEWPORT_PAGE_TOP_SLOT: &str = "__moliVisualViewportPageTop";
pub const VISUAL_VIEWPORT_WIDTH_SLOT: &str = "__moliVisualViewportWidth";
pub const VISUAL_VIEWPORT_HEIGHT_SLOT: &str = "__moliVisualViewportHeight";
pub const VISUAL_VIEWPORT_SCALE_SLOT: &str = "__moliVisualViewportScale";
pub const BAR_PROP_OWNER_SLOT: &str = "__moliBarPropOwner";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::Screen)]
struct ScreenObjectDeclaration {
    #[webapi(slot = SCREEN_WIDTH_SLOT)]
    width: f64,
    #[webapi(slot = SCREEN_HEIGHT_SLOT)]
    height: f64,
    #[webapi(slot = SCREEN_AVAIL_WIDTH_SLOT)]
    avail_width: f64,
    #[webapi(slot = SCREEN_AVAIL_HEIGHT_SLOT)]
    avail_height: f64,
    #[webapi(slot = SCREEN_AVAIL_LEFT_SLOT)]
    avail_left: f64,
    #[webapi(slot = SCREEN_AVAIL_TOP_SLOT)]
    avail_top: f64,
    #[webapi(slot = SCREEN_COLOR_DEPTH_SLOT)]
    color_depth: f64,
    #[webapi(slot = SCREEN_PIXEL_DEPTH_SLOT)]
    pixel_depth: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::ScreenOrientation)]
struct ScreenOrientationObjectDeclaration {
    #[webapi(slot = SCREEN_ORIENTATION_TYPE_SLOT)]
    orientation_type: &'static str,
    #[webapi(slot = SCREEN_ORIENTATION_ANGLE_SLOT)]
    angle: f64,
    #[webapi(slot = SCREEN_ORIENTATION_ONCHANGE_SLOT, init = "null")]
    onchange: (),
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::VisualViewport)]
struct VisualViewportObjectDeclaration {
    #[webapi(slot = VISUAL_VIEWPORT_OFFSET_LEFT_SLOT)]
    offset_left: f64,
    #[webapi(slot = VISUAL_VIEWPORT_OFFSET_TOP_SLOT)]
    offset_top: f64,
    #[webapi(slot = VISUAL_VIEWPORT_PAGE_LEFT_SLOT)]
    page_left: f64,
    #[webapi(slot = VISUAL_VIEWPORT_PAGE_TOP_SLOT)]
    page_top: f64,
    #[webapi(slot = VISUAL_VIEWPORT_WIDTH_SLOT)]
    width: f64,
    #[webapi(slot = VISUAL_VIEWPORT_HEIGHT_SLOT)]
    height: f64,
    #[webapi(slot = VISUAL_VIEWPORT_SCALE_SLOT)]
    scale: f64,
}

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::BarProp)]
struct BarPropObjectDeclaration<'s> {
    #[webapi(prototype)]
    prototype: v8::Local<'s, v8::Object>,
    #[webapi(slot = BAR_PROP_OWNER_SLOT)]
    owner: v8::Local<'s, v8::Array>,
}

pub fn new_screen<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>, BindError> {
    let profile = &DEFAULT_WINDOW_SURFACE_PROFILE;
    ScreenObjectDeclaration {
        width: profile.screen_width,
        height: profile.screen_height,
        avail_width: profile.screen_avail_width,
        avail_height: profile.screen_avail_height,
        avail_left: 0.0,
        avail_top: 0.0,
        color_depth: profile.color_depth,
        pixel_depth: profile.pixel_depth,
    }
    .bind(scope)
}

pub fn new_screen_orientation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>, BindError> {
    let profile = &DEFAULT_WINDOW_SURFACE_PROFILE;
    ScreenOrientationObjectDeclaration::new(profile.orientation_type, profile.orientation_angle)
        .bind(scope)
}

pub fn new_visual_viewport<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    width: f64,
    height: f64,
) -> Result<v8::Local<'s, v8::Object>, BindError> {
    VisualViewportObjectDeclaration {
        offset_left: 0.0,
        offset_top: 0.0,
        page_left: 0.0,
        page_top: 0.0,
        width,
        height,
        scale: DEFAULT_WINDOW_SURFACE_PROFILE.visual_viewport_scale,
    }
    .bind(scope)
}

pub fn new_bar_prop<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    prototype: v8::Local<'s, v8::Object>,
    owner: v8::Local<'s, v8::Array>,
) -> Result<v8::Local<'s, v8::Object>, BindError> {
    BarPropObjectDeclaration::new(prototype, owner).bind(scope)
}

pub const MEDIA_SESSION_HANDLERS_SLOT: &str = "__moliMediaSessionHandlers";
pub const MEDIA_SESSION_PLAYBACK_STATE_SLOT: &str = "__moliMediaSessionPlaybackState";
pub const MEDIA_SESSION_STATE_SLOT: &str = "__moliMediaSessionState";
pub const MEDIA_SESSION_METADATA_SLOT: &str = "__moliMediaSessionMetadata";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaSession)]
struct MediaSessionObjectDeclaration<'s> {
    #[webapi(slot = MEDIA_SESSION_STATE_SLOT)]
    state: v8::Local<'s, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(plain)]
struct MediaSessionStateDeclaration {
    #[webapi(slot = MEDIA_SESSION_HANDLERS_SLOT, init = "null_object")]
    handlers: (),
    #[webapi(slot = MEDIA_SESSION_PLAYBACK_STATE_SLOT)]
    playback_state: &'static str,
    #[webapi(slot = MEDIA_SESSION_METADATA_SLOT, init = "null")]
    metadata: (),
}

pub const PERMISSION_STATUS_NAME_SLOT: &str = "__moliPermissionStatusName";
pub const PERMISSION_STATUS_STATE_SLOT: &str = "__moliPermissionStatusState";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::PermissionStatus)]
struct PermissionStatusObjectDeclaration {
    #[webapi(slot = PERMISSION_STATUS_NAME_SLOT)]
    name: String,

    #[webapi(slot = PERMISSION_STATUS_STATE_SLOT)]
    state: String,

    #[webapi(data_property, init = "null")]
    onchange: (),
}

pub fn new_media_session<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    state: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>, BindError> {
    MediaSessionObjectDeclaration::new(state).bind(scope)
}

pub fn new_media_session_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Result<v8::Local<'s, v8::Object>, BindError> {
    MediaSessionStateDeclaration::new("none").bind(scope)
}

pub fn new_permission_status<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    name: &str,
    state: &str,
) -> v8::Local<'s, v8::Object> {
    PermissionStatusObjectDeclaration::new(name.to_owned(), state.to_owned())
        .bind(scope)
        .expect("PermissionStatus declaration should bind")
}
