use super::*;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLHtmlElement, enumerable)]
pub(super) struct HtmlHtmlElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = html_version_getter_function,
        setter = dom_string_reflection_setter_function,
        setter_data = DomStringReflection::HtmlVersion
    )]
    version: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLMediaElement, enumerable, receiver)]
pub(super) struct HtmlMediaElementPrototypeDeclaration {
    #[webapi(accessor_property, getter = media_buffered_getter_function)]
    buffered: (),
    #[webapi(accessor_property, getter = media_played_getter_function)]
    played: (),
    #[webapi(accessor_property, getter = media_seekable_getter_function)]
    seekable: (),
    #[webapi(accessor_property, getter = media_error_getter_function)]
    error: (),
    #[webapi(
        accessor_property = "crossOrigin",
        getter = media_cross_origin_getter_function,
        setter = media_cross_origin_setter_function
    )]
    cross_origin: (),
    #[webapi(
        accessor_property,
        getter = media_loading_getter_function,
        setter = media_loading_setter_function
    )]
    loading: (),
    #[webapi(
        accessor_property,
        getter = media_preload_getter_function,
        setter = media_preload_setter_function
    )]
    preload: (),
    #[webapi(accessor_property, getter = media_paused_getter_function)]
    paused: (),
    #[webapi(
        accessor_property,
        getter = media_src_getter_function,
        setter = media_src_setter_function
    )]
    src: (),
    #[webapi(
        accessor_property,
        getter = media_volume_getter_function,
        setter = media_volume_setter_function
    )]
    volume: (),
    #[webapi(
        accessor_property,
        getter = media_muted_getter_function,
        setter = media_muted_setter_function
    )]
    muted: (),
    #[webapi(
        accessor_property = "defaultMuted",
        getter = media_default_muted_getter_function,
        setter = media_default_muted_setter_function
    )]
    default_muted: (),
    #[webapi(
        accessor_property = "playbackRate",
        getter = media_playback_rate_getter_function,
        setter = media_playback_rate_setter_function
    )]
    playback_rate: (),
    #[webapi(
        accessor_property = "currentTime",
        getter = media_current_time_getter_function,
        setter = media_current_time_setter_function
    )]
    current_time: (),
    #[webapi(accessor_property, getter = media_duration_getter_function)]
    duration: (),
    #[webapi(accessor_property, getter = media_ended_getter_function)]
    ended: (),
    #[webapi(accessor_property, getter = media_seeking_getter_function)]
    seeking: (),
    #[webapi(accessor_property = "readyState", getter = media_ready_state_getter_function)]
    ready_state: (),
    #[webapi(accessor_property = "networkState", getter = media_network_state_getter_function)]
    network_state: (),
    #[webapi(accessor_property = "textTracks", getter = media_text_tracks_getter_function)]
    text_tracks: (),
    #[webapi(
        accessor_property,
        getter = media_autoplay_getter_function,
        setter = media_autoplay_setter_function
    )]
    autoplay: (),
    #[webapi(
        accessor_property,
        getter = media_controls_getter_function,
        setter = media_controls_setter_function
    )]
    controls: (),
    #[webapi(
        accessor_property = "loop",
        getter = media_loop_getter_function,
        setter = media_loop_setter_function
    )]
    loop_: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::HTMLVideoElement, enumerable)]
pub(super) struct HtmlVideoElementPrototypeDeclaration {
    #[webapi(
        accessor_property,
        getter = media_poster_getter_function,
        setter = media_poster_setter_function
    )]
    poster: (),
    #[webapi(
        accessor_property,
        getter = media_width_getter_function,
        setter = media_width_setter_function
    )]
    width: (),
    #[webapi(
        accessor_property,
        getter = media_height_getter_function,
        setter = media_height_setter_function
    )]
    height: (),
    #[webapi(
        accessor_property = "playsInline",
        getter = media_plays_inline_getter_function,
        setter = media_plays_inline_setter_function
    )]
    plays_inline: (),
    #[webapi(accessor_property = "videoWidth", getter = media_video_width_getter_function)]
    video_width: (),
    #[webapi(accessor_property = "videoHeight", getter = media_video_height_getter_function)]
    video_height: (),
}
