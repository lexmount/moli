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

pub mod abort_signal_values;
pub mod base64;
pub mod callbacks;
pub mod canvas_gradient;
pub mod constructors;
pub mod credentials;
pub mod crypto_key;
pub mod css_escape;
pub mod css_values;
pub mod dom_exception;
pub mod dom_quad;
pub mod dom_rect;
pub mod dom_rect_list;
pub mod dom_string_list;
pub mod encoded_video_chunk;
pub mod event_target;
pub mod events;
pub mod external;
pub mod file_list;
pub mod gamepad;
pub mod geolocation;
pub mod geometry;
pub mod geometry_clone;
pub mod headers;
pub mod highlights;
pub mod idle_deadline;
pub mod image_bitmap;
pub mod image_data;
pub mod indexed_db_values;
pub mod interfaces;
pub mod intersection_observer_entry;
pub mod media_capabilities;
pub mod media_device_info;
pub mod media_error;
pub mod media_source;
pub mod mutation_record;
pub mod navigator_collections;
pub mod navigator_ua_data;
pub mod performance;
pub mod picture_in_picture;
pub mod push_values;
pub mod queuing_strategy;
pub mod resize_observer_entry;
pub mod rtc;
pub mod speech_synthesis;
pub mod text_encoder;
pub mod text_metrics;
pub mod time_ranges;
pub mod touch;
pub mod trusted_values;
pub mod url_api;
pub mod url_search_params;
pub mod video_codecs;
pub mod web_audio;
pub mod webgl;
pub mod webgl_values;
pub mod webidl_iterator;
pub mod worker_location;

pub use embedding::{RealmCallbacks, install_realm_callbacks};
