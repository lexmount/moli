mod attributes;
mod methods;
mod state;
mod text_tracks;
mod video;

pub(crate) use attributes::{
    MediaLoadEventPhase, dispatch_media_load_event_phase, queue_media_canplay_after_text_tracks,
    queue_media_load_if_needed, queue_media_load_if_source_or_loading_change,
    queue_media_load_network_terminal_followup, queue_revealed_lazy_media_loads,
};
pub(in crate::native_bridge) use attributes::{
    media_autoplay_getter_function, media_autoplay_setter_function, media_controls_getter_function,
    media_controls_setter_function, media_cross_origin_getter_function,
    media_cross_origin_setter_function, media_default_muted_getter_function,
    media_default_muted_setter_function, media_loading_getter_function,
    media_loading_setter_function, media_loop_getter_function, media_loop_setter_function,
    media_plays_inline_getter_function, media_plays_inline_setter_function,
    media_preload_getter_function, media_preload_setter_function, media_src_getter_function,
    media_src_setter_function,
};
pub(in crate::native_bridge) use methods::{
    media_can_play_type_callback, media_load_callback, media_pause_callback, media_play_callback,
};
pub(crate) use state::{dispatch_media_seek_completion, dispatch_media_seeking_event};
pub(in crate::native_bridge) use state::{
    media_current_time_getter_function, media_current_time_setter_function,
    media_duration_getter_function, media_ended_getter_function, media_error_getter_function,
    media_muted_getter_function, media_muted_setter_function, media_network_state_getter_function,
    media_paused_getter_function, media_playback_rate_getter_function,
    media_playback_rate_setter_function, media_ready_state_getter_function,
    media_seeking_getter_function, media_volume_getter_function, media_volume_setter_function,
};
pub(crate) use text_tracks::{
    apply_default_text_track_mode_for_track, apply_text_track_load_task,
    dispatch_text_track_list_event, install_text_track_template_bindings,
    queue_default_text_track_mode_if_needed, queue_media_selection_text_track_loads,
    queue_text_track_load_if_needed, queue_text_track_terminal_followup,
    resort_text_track_cues_for_cue,
};
pub(in crate::native_bridge) use text_tracks::{
    apply_default_text_track_modes_for_media, media_add_text_track_callback,
    media_text_tracks_getter_function, queue_text_track_load_if_source,
    refresh_media_active_text_track_cues, track_ready_state_for_handle,
    track_text_track_getter_function,
};
pub(in crate::native_bridge) use video::{
    media_height_getter_function, media_height_setter_function, media_poster_getter_function,
    media_poster_setter_function, media_video_height_getter_function,
    media_video_width_getter_function, media_width_getter_function, media_width_setter_function,
};
