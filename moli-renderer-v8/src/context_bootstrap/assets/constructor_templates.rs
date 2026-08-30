use super::super::constructors::*;
use super::super::{
    animation_runtime::{animation_constructor_callback, keyframe_effect_constructor_callback},
    broadcast_channel::broadcast_channel_constructor_callback,
    canvas::{
        canvas_rendering_context_2d_constructor_callback, offscreen_canvas_constructor_callback,
        offscreen_canvas_rendering_context_2d_constructor_callback,
        webgl_debug_renderer_info_constructor_callback, webgl_lose_context_constructor_callback,
        webgl_rendering_context_constructor_callback,
    },
    css_fontface_runtime::{font_face_constructor_callback, font_face_set_constructor_callback},
    css_runtime::{css_keyword_value_constructor_callback, css_unit_value_constructor_callback},
    css_stylesheet_runtime::css_style_sheet_constructor_callback,
    events::{EventSubclassKind, build_event_subclass_template, event_constructor_callback},
    exposed_interfaces::install_interface_template_metadata,
    file_api::{
        data_transfer_constructor_callback, file_constructor_callback,
        file_list_constructor_callback, file_reader_constructor_callback,
        file_reader_sync_constructor_callback,
    },
    form_data_runtime::build_form_data_constructor_template,
    geometry_runtime::{
        dom_matrix_constructor_callback, dom_point_constructor_callback,
        dom_point_readonly_constructor_callback,
    },
    idle_detection::idle_detector_constructor_callback,
    image_data::image_data_constructor_callback,
    location_runtime::build_location_constructor_template,
    media_cues::{
        media_error_constructor_callback, text_track_cue_constructor_callback,
        vtt_cue_constructor_callback,
    },
    media_source::media_source_constructor_callback,
    message_ports::{message_channel_constructor_callback, message_port_constructor_callback},
    navigator_runtime::clipboard_item_constructor_callback,
    notification_runtime::notification_constructor_callback,
    performance_runtime::{
        performance_mark_constructor_callback, performance_observer_constructor_callback,
    },
    range_surface::{
        build_abstract_range_template, build_range_constructor_template,
        build_static_range_constructor_template,
    },
    resize_observer_runtime::resize_observer_constructor_callback,
    shared_worker_host::shared_worker_constructor_callback,
    specs::{ConstructorKind, ConstructorPrototypeProperty, ConstructorSpec},
    speech_synthesis::speech_synthesis_utterance_constructor_callback,
    streams::{
        byte_length_queuing_strategy_constructor_callback, compression_stream_constructor_callback,
        count_queuing_strategy_constructor_callback,
        readable_stream_byob_reader_constructor_callback, readable_stream_constructor_callback,
        readable_stream_default_reader_constructor_callback,
        text_decoder_stream_constructor_callback, text_encoder_stream_constructor_callback,
        transform_stream_constructor_callback, writable_stream_constructor_callback,
        writable_stream_default_writer_constructor_callback,
    },
    touch_runtime::touch_constructor_callback,
    url_form::build_url_constructor_template,
    url_search_params_runtime::build_url_search_params_constructor_template,
    web_audio_runtime::{
        build_audio_context_constructor_template, build_audio_worklet_node_constructor_template,
        offline_audio_context_constructor_callback,
    },
    webrtc::{
        rtc_ice_candidate_constructor_callback, rtc_peer_connection_constructor_callback,
        rtc_session_description_constructor_callback,
    },
    websocket::{
        websocket_constructor_callback, websocket_error_constructor_callback,
        websocket_stream_constructor_callback,
    },
    worker_host::worker_constructor_callback,
};
use super::prototype_bindings::install_constructor_template_bindings;
use crate::web_api_interfaces;
use crate::{
    blob, dom_parser, native_bridge::abort, network_host, observer_runtime, util::v8_string,
    xml_serializer,
};
use anyhow::{Result, anyhow};

pub(in crate::context_bootstrap) fn build_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    spec: ConstructorSpec,
) -> Result<v8::Local<'s, v8::FunctionTemplate>> {
    let template = match spec.kind {
        ConstructorKind::Illegal => v8::FunctionTemplate::builder(illegal_constructor_callback)
            .length(0)
            .build(scope),
        ConstructorKind::Unsupported => {
            v8::FunctionTemplate::builder(unsupported_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::Event => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::Event,
                event_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::UiEvent => {
            build_event_subclass_template(scope, EventSubclassKind::UiEvent)
        }
        ConstructorKind::FocusEvent => {
            build_event_subclass_template(scope, EventSubclassKind::FocusEvent)
        }
        ConstructorKind::CompositionEvent => {
            build_event_subclass_template(scope, EventSubclassKind::CompositionEvent)
        }
        ConstructorKind::CustomEvent => {
            build_event_subclass_template(scope, EventSubclassKind::CustomEvent)
        }
        ConstructorKind::MouseEvent => {
            build_event_subclass_template(scope, EventSubclassKind::MouseEvent)
        }
        ConstructorKind::CapturedMouseEvent => {
            build_event_subclass_template(scope, EventSubclassKind::CapturedMouseEvent)
        }
        ConstructorKind::DragEvent => {
            build_event_subclass_template(scope, EventSubclassKind::DragEvent)
        }
        ConstructorKind::ClipboardEvent => {
            build_event_subclass_template(scope, EventSubclassKind::ClipboardEvent)
        }
        ConstructorKind::ClipboardItem => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::ClipboardItem,
                clipboard_item_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::KeyboardEvent => {
            build_event_subclass_template(scope, EventSubclassKind::KeyboardEvent)
        }
        ConstructorKind::InputEvent => {
            build_event_subclass_template(scope, EventSubclassKind::InputEvent)
        }
        ConstructorKind::WheelEvent => {
            build_event_subclass_template(scope, EventSubclassKind::WheelEvent)
        }
        ConstructorKind::PointerEvent => {
            build_event_subclass_template(scope, EventSubclassKind::PointerEvent)
        }
        ConstructorKind::TouchEvent => {
            build_event_subclass_template(scope, EventSubclassKind::TouchEvent)
        }
        ConstructorKind::MessageEvent => {
            build_event_subclass_template(scope, EventSubclassKind::MessageEvent)
        }
        ConstructorKind::StorageEvent => {
            build_event_subclass_template(scope, EventSubclassKind::StorageEvent)
        }
        ConstructorKind::ErrorEvent => {
            build_event_subclass_template(scope, EventSubclassKind::ErrorEvent)
        }
        ConstructorKind::PromiseRejectionEvent => {
            build_event_subclass_template(scope, EventSubclassKind::PromiseRejectionEvent)
        }
        ConstructorKind::SecurityPolicyViolationEvent => {
            build_event_subclass_template(scope, EventSubclassKind::SecurityPolicyViolationEvent)
        }
        ConstructorKind::NavigationCurrentEntryChangeEvent => build_event_subclass_template(
            scope,
            EventSubclassKind::NavigationCurrentEntryChangeEvent,
        ),
        ConstructorKind::NavigateEvent => {
            build_event_subclass_template(scope, EventSubclassKind::NavigateEvent)
        }
        ConstructorKind::CloseEvent => {
            build_event_subclass_template(scope, EventSubclassKind::CloseEvent)
        }
        ConstructorKind::SubmitEvent => {
            build_event_subclass_template(scope, EventSubclassKind::SubmitEvent)
        }
        ConstructorKind::FormDataEvent => {
            build_event_subclass_template(scope, EventSubclassKind::FormDataEvent)
        }
        ConstructorKind::CommandEvent => {
            build_event_subclass_template(scope, EventSubclassKind::CommandEvent)
        }
        ConstructorKind::ToggleEvent => {
            build_event_subclass_template(scope, EventSubclassKind::ToggleEvent)
        }
        ConstructorKind::InterestEvent => {
            build_event_subclass_template(scope, EventSubclassKind::InterestEvent)
        }
        ConstructorKind::PopStateEvent => {
            build_event_subclass_template(scope, EventSubclassKind::PopStateEvent)
        }
        ConstructorKind::PageTransitionEvent => {
            build_event_subclass_template(scope, EventSubclassKind::PageTransitionEvent)
        }
        ConstructorKind::FontFaceSetLoadEvent => {
            build_event_subclass_template(scope, EventSubclassKind::FontFaceSetLoadEvent)
        }
        ConstructorKind::DomException => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::DOMException,
                dom_exception_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::DomError => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::DOMError,
                dom_error_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::QuotaExceededError => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::QuotaExceededError,
                quota_exceeded_error_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::CustomElementRegistry => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::CustomElementRegistry,
                custom_elements_registry_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::Document => v8::FunctionTemplate::builder(document_constructor_callback)
            .length(0)
            .build(scope),
        ConstructorKind::DocumentFragment => {
            v8::FunctionTemplate::builder(document_fragment_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::XmlHttpRequest => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::XMLHttpRequest,
                network_host::xhr_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::Headers => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::Headers,
                network_host::headers_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::Request => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::Request,
                network_host::request_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::Response => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::Response,
                network_host::response_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::ProgressEvent => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::ProgressEvent,
                network_host::progress_event_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::DomParser => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::DOMParser,
                dom_parser::dom_parser_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::TextEncoder => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::TextEncoder,
                text_encoder_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::TextDecoder => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::TextDecoder,
                text_decoder_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::ReadableStream => {
            v8::FunctionTemplate::builder(readable_stream_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::WritableStream => {
            v8::FunctionTemplate::builder(writable_stream_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::ReadableStreamDefaultReader => {
            v8::FunctionTemplate::builder(readable_stream_default_reader_constructor_callback)
                .length(1)
                .build(scope)
        }
        ConstructorKind::ReadableStreamByobReader => {
            v8::FunctionTemplate::builder(readable_stream_byob_reader_constructor_callback)
                .length(1)
                .build(scope)
        }
        ConstructorKind::WritableStreamDefaultWriter => {
            v8::FunctionTemplate::builder(writable_stream_default_writer_constructor_callback)
                .length(1)
                .build(scope)
        }
        ConstructorKind::ReadableStreamDefaultController
        | ConstructorKind::WritableStreamDefaultController
        | ConstructorKind::TransformStreamDefaultController => {
            v8::FunctionTemplate::builder(illegal_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::TransformStream => {
            v8::FunctionTemplate::builder(transform_stream_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::TextEncoderStream => {
            v8::FunctionTemplate::builder(text_encoder_stream_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::TextDecoderStream => {
            v8::FunctionTemplate::builder(text_decoder_stream_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::CompressionStream => {
            v8::FunctionTemplate::builder(compression_stream_constructor_callback::<false>)
                .length(1)
                .build(scope)
        }
        ConstructorKind::DecompressionStream => {
            v8::FunctionTemplate::builder(compression_stream_constructor_callback::<true>)
                .length(1)
                .build(scope)
        }
        ConstructorKind::CountQueuingStrategy => {
            v8::FunctionTemplate::builder(count_queuing_strategy_constructor_callback)
                .length(1)
                .build(scope)
        }
        ConstructorKind::ByteLengthQueuingStrategy => {
            v8::FunctionTemplate::builder(byte_length_queuing_strategy_constructor_callback)
                .length(1)
                .build(scope)
        }
        ConstructorKind::Blob => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::Blob,
                blob::blob_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::DataTransfer => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::DataTransfer,
                data_transfer_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::ImageData => {
            let template =
                v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                    web_api_interfaces::ImageData,
                    image_data_constructor_callback
                ))
                .length(2)
                .build(scope);
            let instance = template.instance_template(scope);
            let _ = instance.set_internal_field_count(1);
            template
        }
        ConstructorKind::OffscreenCanvas => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::OffscreenCanvas,
                offscreen_canvas_constructor_callback
            ))
            .length(2)
            .build(scope)
        }
        ConstructorKind::CanvasRenderingContext2D => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::CanvasRenderingContext2D,
                canvas_rendering_context_2d_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::OffscreenCanvasRenderingContext2D => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::OffscreenCanvasRenderingContext2D,
                offscreen_canvas_rendering_context_2d_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::WebGLRenderingContext => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::WebGLRenderingContext,
                webgl_rendering_context_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::WebGlDebugRendererInfo => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::WEBGLDebugRendererInfo,
                webgl_debug_renderer_info_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::WebGlLoseContext => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::WEBGLLoseContext,
                webgl_lose_context_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::File => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::File,
                file_constructor_callback
            ))
            .length(2)
            .build(scope)
        }
        ConstructorKind::FileList => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::FileList,
                file_list_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::FileReader => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::FileReader,
                file_reader_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::FileReaderSync => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::FileReaderSync,
                file_reader_sync_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::DomRectReadOnly => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::DOMRectReadOnly,
                super::super::dom_rect::dom_rect_readonly_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::DomRect => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::DOMRect,
                super::super::dom_rect::dom_rect_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::DomPointReadOnly => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::DOMPointReadOnly,
                dom_point_readonly_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::DomPoint => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::DOMPoint,
                dom_point_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::DomMatrix => {
            v8::FunctionTemplate::builder(dom_matrix_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::XmlSerializer => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::XMLSerializer,
                xml_serializer::xml_serializer_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::AbortController => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::AbortController,
                abort::abort_controller_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::MessageChannel => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::MessageChannel,
                message_channel_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::MessagePort => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::MessagePort,
                message_port_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::BroadcastChannel => {
            // The callback initializes both live and detached instances through
            // BroadcastChannelObjectDeclaration in their relevant realm.
            v8::FunctionTemplate::builder(broadcast_channel_constructor_callback)
                .length(1)
                .build(scope)
        }
        ConstructorKind::EventSource => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::EventSource,
                network_host::event_source_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::IdleDetector => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::IdleDetector,
                idle_detector_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::Notification => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::Notification,
                notification_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::WebSocket => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::WebSocket,
                websocket_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::RtcPeerConnection => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::RTCPeerConnection,
                rtc_peer_connection_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::RtcIceCandidate => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::RTCIceCandidate,
                rtc_ice_candidate_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::RtcSessionDescription => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::RTCSessionDescription,
                rtc_session_description_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::Navigator
        | ConstructorKind::WorkerNavigator
        | ConstructorKind::Permissions
        | ConstructorKind::PermissionStatus
        | ConstructorKind::WorkerLocation
        | ConstructorKind::Screen => v8::FunctionTemplate::builder(illegal_constructor_callback)
            .length(0)
            .build(scope),
        ConstructorKind::SpeechSynthesisUtterance => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::SpeechSynthesisUtterance,
                speech_synthesis_utterance_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::WebSocketError => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::WebSocketError,
                websocket_error_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::WebSocketStream => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::WebSocketStream,
                websocket_stream_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::ResizeObserver => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::ResizeObserver,
                resize_observer_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::PerformanceObserver => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::PerformanceObserver,
                performance_observer_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::Selection => {
            v8::FunctionTemplate::builder(illegal_constructor_callback).build(scope)
        }
        ConstructorKind::History | ConstructorKind::Navigation => {
            v8::FunctionTemplate::builder(illegal_constructor_callback).build(scope)
        }
        ConstructorKind::Location => build_location_constructor_template(scope),
        ConstructorKind::MediaError
        | ConstructorKind::TextTrack
        | ConstructorKind::TextTrackList
        | ConstructorKind::TextTrackCueList => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::TextTrackCueList,
                media_error_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::TrackEvent => {
            build_event_subclass_template(scope, EventSubclassKind::TrackEvent)
        }
        ConstructorKind::TextTrackCue => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::TextTrackCue,
                text_track_cue_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::VTTCue => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::VTTCue,
                vtt_cue_constructor_callback
            ))
            .length(3)
            .build(scope)
        }
        ConstructorKind::PerformanceObserverEntryList => {
            v8::FunctionTemplate::builder(illegal_constructor_callback).build(scope)
        }
        ConstructorKind::PerformanceMark => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::PerformanceMark,
                performance_mark_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::PerformanceEntry
        | ConstructorKind::PerformanceNavigationTiming
        | ConstructorKind::PerformanceMeasure
        | ConstructorKind::PerformanceResourceTiming
        | ConstructorKind::EventCounts
        | ConstructorKind::PerformanceNavigation
        | ConstructorKind::PerformanceTiming
        | ConstructorKind::NavigatorUAData
        | ConstructorKind::StorageManager
        | ConstructorKind::StorageEstimate
        | ConstructorKind::StorageBucketManager
        | ConstructorKind::StorageBucket
        | ConstructorKind::IdleDeadline
        | ConstructorKind::NavigationHistoryEntry
        | ConstructorKind::NavigationActivation
        | ConstructorKind::NavigationTransition
        | ConstructorKind::MediaQueryList => {
            v8::FunctionTemplate::builder(illegal_constructor_callback).build(scope)
        }
        ConstructorKind::MediaSource => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::MediaSource,
                media_source_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::Animation => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::Animation,
                animation_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::KeyframeEffect => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::KeyframeEffect,
                keyframe_effect_constructor_callback
            ))
            .length(2)
            .build(scope)
        }
        ConstructorKind::HtmlElement => {
            let Some(constructor_name) = v8_string(scope, spec.interface.name()) else {
                return Err(anyhow!("failed to allocate HTML element constructor name"));
            };
            v8::FunctionTemplate::builder(html_element_constructor_callback)
                .data(constructor_name.into())
                .length(0)
                .build(scope)
        }
        ConstructorKind::Option => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::HTMLOptionElement,
                option_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::MutationObserver => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::MutationObserver,
                observer_runtime::mutation_observer_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::IntersectionObserver => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::IntersectionObserver,
                observer_runtime::intersection_observer_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::IntersectionObserverEntry => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::IntersectionObserverEntry,
                observer_runtime::intersection_observer_entry_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::MutationRecord => {
            v8::FunctionTemplate::builder(illegal_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::Image => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::HTMLImageElement,
                image_constructor_callback
            ))
            .length(2)
            .build(scope)
        }
        ConstructorKind::Audio => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::HTMLAudioElement,
                audio_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::StyleSheet
        | ConstructorKind::StyleSheetList
        | ConstructorKind::MediaList
        | ConstructorKind::CssRuleList
        | ConstructorKind::CssRule
        | ConstructorKind::CssStyleRule => {
            v8::FunctionTemplate::builder(illegal_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::CssStyleSheet => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::CSSStyleSheet,
                css_style_sheet_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::CssKeywordValue => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::CSSKeywordValue,
                css_keyword_value_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::CssUnitValue => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::CSSUnitValue,
                css_unit_value_constructor_callback
            ))
            .length(2)
            .build(scope)
        }
        ConstructorKind::FontFace => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::FontFace,
                font_face_constructor_callback
            ))
            .length(2)
            .build(scope)
        }
        ConstructorKind::FontFaceSet => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::FontFaceSet,
                font_face_set_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::AudioContext => build_audio_context_constructor_template(scope),
        ConstructorKind::AudioWorkletNode => build_audio_worklet_node_constructor_template(scope),
        ConstructorKind::OfflineAudioContext => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::OfflineAudioContext,
                offline_audio_context_constructor_callback
            ))
            .length(3)
            .build(scope)
        }
        ConstructorKind::BaseAudioContext
        | ConstructorKind::AudioDestinationNode
        | ConstructorKind::OscillatorNode
        | ConstructorKind::DynamicsCompressorNode
        | ConstructorKind::AnalyserNode
        | ConstructorKind::BiquadFilterNode
        | ConstructorKind::AudioParam
        | ConstructorKind::AudioBuffer => {
            v8::FunctionTemplate::builder(illegal_constructor_callback)
                .length(0)
                .build(scope)
        }
        ConstructorKind::Text => v8::FunctionTemplate::builder(text_constructor_callback)
            .length(0)
            .build(scope),
        ConstructorKind::Comment => v8::FunctionTemplate::builder(comment_constructor_callback)
            .length(0)
            .build(scope),
        ConstructorKind::Touch => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::Touch,
                touch_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::EventTarget => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::EventTarget,
                event_target_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::XPathEvaluator => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::XPathEvaluator,
                xpath_evaluator_constructor_callback
            ))
            .length(0)
            .build(scope)
        }
        ConstructorKind::Worker => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::Worker,
                worker_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::SharedWorker => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::SharedWorker,
                shared_worker_constructor_callback
            ))
            .length(1)
            .build(scope)
        }
        ConstructorKind::AbstractRange => build_abstract_range_template(scope),
        ConstructorKind::Range => build_range_constructor_template(scope),
        ConstructorKind::StaticRange => build_static_range_constructor_template(scope),
        ConstructorKind::Url => build_url_constructor_template(scope),
        ConstructorKind::UrlSearchParams => build_url_search_params_constructor_template(scope),
        ConstructorKind::FormData => build_form_data_constructor_template(scope),
        ConstructorKind::IndexedDb => v8::FunctionTemplate::builder(illegal_constructor_callback)
            .length(0)
            .build(scope),
        ConstructorKind::IndexedDbVersionChangeEvent => {
            v8::FunctionTemplate::builder(moli_webapi_declare::web_api_constructor!(
                web_api_interfaces::IDBVersionChangeEvent,
                crate::context_bootstrap::indexed_db::idb_version_change_event_constructor_callback
            ))
            .length(2)
            .build(scope)
        }
    };
    finalize_constructor_template(scope, spec, template)
}

pub(in crate::context_bootstrap) fn build_constructor_template_with_callback<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    spec: ConstructorSpec,
    callback: impl v8::MapFnTo<v8::FunctionCallback>,
) -> Result<v8::Local<'s, v8::FunctionTemplate>> {
    let template = v8::FunctionTemplate::builder(callback)
        .length(1)
        .build(scope);
    finalize_constructor_template(scope, spec, template)
}

fn finalize_constructor_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    spec: ConstructorSpec,
    template: v8::Local<'s, v8::FunctionTemplate>,
) -> Result<v8::Local<'s, v8::FunctionTemplate>> {
    let class_name = v8_string(scope, spec.interface.name()).ok_or_else(|| {
        anyhow!(
            "failed to allocate context bootstrap class `{}`",
            spec.interface.name()
        )
    })?;
    template.set_class_name(class_name);
    // WebIDL interface objects expose a non-writable `prototype` property.
    // V8 FunctionTemplate defaults to writable, so make the binding-level
    // descriptor explicit unless the interface metadata says a runtime pass
    // must install the final legacy-factory shape.
    if spec.prototype_property() == ConstructorPrototypeProperty::TemplateReadOnly {
        template.read_only_prototype();
    }

    install_constructor_template_bindings(scope, template, spec);
    install_interface_template_metadata(scope, template, spec.interface.name());

    Ok(template)
}
