//! Concrete V8 Web API definitions and bindings.
//!
//! This crate owns the native interface catalog and self-contained Web API
//! implementations, including their declaration derives, WebIDL conversions,
//! callbacks, and template bindings. Generic declaration machinery lives in
//! `moli-webapi-declare`, and generic conversions live in `moli-webidl`.
//!
//! Realm exposure, prototype registration, and page-owned host capabilities
//! are supplied by the embedding runtime. This crate does not depend on the
//! renderer or its document and task scheduling state.

mod embedding;

pub mod abort_signal;
pub mod abort_signal_values;
pub mod animation;
pub mod base64;
pub mod blob;
pub mod callbacks;
pub mod canvas_context_values;
pub mod canvas_gradient;
pub mod caret_position;
pub mod clipboard_item;
pub mod constructors;
pub mod credentials;
pub mod crypto_key;
pub mod css_declarations;
pub mod css_escape;
pub mod css_stylesheet;
pub mod css_values;
pub mod dom_exception;
pub mod dom_options;
pub mod dom_parser_values;
pub mod dom_quad;
pub mod dom_rect;
pub mod dom_rect_list;
pub mod dom_string_list;
pub mod drag_data_values;
pub mod element_visibility;
pub mod encoded_video_chunk;
pub mod event_target;
pub mod events;
pub mod external;
pub mod fetch_request;
pub mod fetch_response;
pub mod file;
pub mod file_list;
pub mod file_reader_sync;
pub mod file_system_options;
pub mod font_face;
pub mod form_data;
pub mod gamepad;
pub mod geolocation;
pub mod geometry;
pub mod geometry_clone;
pub mod headers;
pub mod highlights;
pub mod idle_deadline;
pub mod idle_detector_values;
pub mod image_bitmap;
pub mod image_data;
pub mod indexed_db_values;
pub mod interface_constants;
pub mod interfaces;
pub mod intersection_observer_entry;
pub mod media_capabilities;
pub mod media_cue_values;
pub mod media_device_info;
pub mod media_error;
pub mod media_query_list;
pub mod media_source;
pub mod message_channel;
pub mod mutation_record;
pub mod navigation_values;
pub mod navigator_collections;
pub mod navigator_options;
pub mod navigator_ua_data;
pub mod network_options;
pub mod notification;
pub mod performance;
pub mod picture_in_picture;
pub mod push_values;
pub mod queuing_strategy;
pub mod resize_observer_entry;
pub mod rtc;
pub mod scroll_options;
pub mod speech_synthesis;
pub mod static_range;
pub mod storage_values;
pub mod streams;
pub mod svg_values;
pub mod text_codec;
pub mod text_decoder;
pub mod text_encoder;
pub mod text_metrics;
pub mod time_ranges;
pub mod touch;
pub mod trusted_values;
pub mod url_api;
pub mod url_search_params;
pub mod validity_state;
pub mod video_codecs;
pub mod web_audio;
pub mod web_mcp_values;
pub mod webassembly;
pub mod webgl;
pub mod webgl_values;
pub mod webidl_iterator;
pub mod websocket_values;
pub mod window_values;
pub mod worker_location;
pub mod worker_options;
pub mod xpath_values;

pub use embedding::{
    RealmCallbacks, RealmRuntime, WebIdlCallbackInvocation, WebIdlCallbackResult,
    install_realm_callbacks,
};
