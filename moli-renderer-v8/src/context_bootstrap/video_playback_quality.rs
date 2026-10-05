//! Native playback-quality snapshots; counters are supplied by the media producer.

use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

use crate::{
    util::{get_private_value, v8str},
    web_api_interfaces,
};

const CREATION_TIME: &str = "__moliVideoPlaybackQualityCreationTime";
const TOTAL_FRAMES: &str = "__moliVideoPlaybackQualityTotalFrames";
const DROPPED_FRAMES: &str = "__moliVideoPlaybackQualityDroppedFrames";
const CORRUPTED_FRAMES: &str = "__moliVideoPlaybackQualityCorruptedFrames";

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::VideoPlaybackQuality, require_prototype)]
struct QualitySnapshot {
    #[webapi(slot = CREATION_TIME)]
    creation_time: f64,
    #[webapi(slot = TOTAL_FRAMES)]
    total_frames: u32,
    #[webapi(slot = DROPPED_FRAMES)]
    dropped_frames: u32,
    #[webapi(slot = CORRUPTED_FRAMES)]
    corrupted_frames: u32,
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::VideoPlaybackQuality, enumerable, receiver)]
struct QualityPrototype {
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, CREATION_TIME))]
    creation_time: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, TOTAL_FRAMES))]
    total_video_frames: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, DROPPED_FRAMES))]
    dropped_video_frames: (),
    #[webapi(accessor_property, getter = slot_getter, data = v8str(scope, CORRUPTED_FRAMES))]
    corrupted_video_frames: (),
}

pub(in crate::context_bootstrap) fn install<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface: &str,
) {
    if interface == "VideoPlaybackQuality" {
        QualityPrototype::initialize_prototype_template(scope, template.prototype_template(scope));
    }
}

pub(crate) fn new_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    creation_time: f64,
    total_frames: u32,
    dropped_frames: u32,
    corrupted_frames: u32,
) -> v8::Local<'s, v8::Object> {
    QualitySnapshot::new(
        creation_time,
        total_frames,
        dropped_frames,
        corrupted_frames,
    )
    .bind(scope)
    .expect("native VideoPlaybackQuality snapshot")
}

pub(crate) fn get_video_playback_quality<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated video receiver");
    let creation_time = document_creation_time(scope, receiver);
    let context = receiver
        .get_creation_context(scope)
        .expect("video relevant realm");
    let scope = &mut v8::ContextScope::new(scope, context);
    // Moli does not decode or present video frames yet. No frames have been
    // displayed, dropped or corrupted; a future decoder must supply its counters.
    rv.set(super::new_video_playback_quality(scope, creation_time, 0, 0, 0).into());
}

fn document_creation_time<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> f64 {
    let Ok((runtime_ptr, handle)) =
        crate::native_bridge::node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return 0.0;
    };
    let document = unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .and_then(|node| node.owner_document());
    let Some(document) = document
        .filter(|&document| unsafe { &*runtime_ptr }.document_has_browsing_context(document))
    else {
        return 0.0;
    };
    let Some(window) = crate::native_bridge::document::document_associated_window_for_handle(
        scope,
        runtime_ptr,
        document,
    ) else {
        return 0.0;
    };
    let Some(context) = window.get_creation_context(scope) else {
        return 0.0;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let origin = super::current_performance_time_origin(scope);
    super::dom_time_since_origin_millis(origin)
}

fn slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = moli_webapi_declare::web_api_object_target(scope, args.this())
        .expect("validated playback-quality receiver");
    let slot = args.data().to_rust_string_lossy(scope);
    rv.set(get_private_value(scope, target, &slot).expect("playback-quality snapshot value"));
}
