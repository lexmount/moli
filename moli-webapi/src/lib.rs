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

pub mod css_values;
pub mod dom_exception;
pub mod dom_quad;
pub mod dom_rect;
pub mod dom_rect_list;
pub mod dom_string_list;
mod embedding;
pub mod encoded_video_chunk;
pub mod geolocation;
pub mod geometry;
pub mod geometry_clone;
pub mod image_data;
pub mod interfaces;
pub mod queuing_strategy;
pub mod rtc;
pub mod time_ranges;

pub use embedding::{RealmCallbacks, install_realm_callbacks};
