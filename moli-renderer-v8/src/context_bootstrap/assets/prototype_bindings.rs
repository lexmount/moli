use super::super::{
    animation_runtime::{
        animation_effect_getter, animation_effect_setter, animation_finished_getter,
        animation_onfinish_getter, animation_onfinish_setter, animation_pending_getter,
        animation_play_state_getter, animation_ready_getter, animation_start_time_getter,
        animation_start_time_setter, animation_timeline_getter, animation_timeline_setter,
        install_animation_template_bindings,
    },
    broadcast_channel::install_broadcast_channel_template_bindings,
    constructors::{
        install_custom_element_registry_template_bindings, install_dom_exception_template_bindings,
        install_dom_implementation_template_bindings, install_text_codec_template_bindings,
    },
    crypto::install_crypto_template_bindings,
    css_runtime::install_css_typed_om_template_bindings,
    css_stylesheet_runtime::install_css_stylesheet_template_bindings,
    dom_quad::install_dom_quad_template_bindings,
    dom_rect::install_dom_rect_template_bindings,
    dom_rect_list::install_dom_rect_list_template_bindings,
    event_template::install_event_template_bindings,
    exposed_interfaces::TemplateBuildProfile,
    file_api::install_file_api_template_bindings,
    geometry_runtime::install_geometry_template_bindings,
    idle_detection::install_idle_detector_template_bindings,
    image_data::install_image_data_template_bindings,
    indexed_db::install_indexed_db_template_bindings,
    media_cues::install_media_cue_template_bindings,
    media_file_template::install_media_file_template_bindings,
    media_queries::install_media_query_list_template_bindings,
    media_source::install_media_source_template_bindings,
    message_ports::install_message_port_template_bindings,
    navigation_callbacks::{document_location_getter, document_location_setter},
    navigation_surface::{install_history_bindings, install_navigation_bindings},
    navigator_runtime::{
        install_navigator_template_bindings, install_screen_template_bindings,
        install_storage_manager_constructor_template_bindings,
        install_visual_viewport_template_bindings,
    },
    notification_runtime::install_notification_template_bindings,
    observer_template::install_observer_template_bindings,
    opfs::install_opfs_constructor_template_bindings,
    performance_runtime::install_performance_template_bindings,
    range_surface::install_range_template_bindings,
    selection_surface::install_selection_template_bindings,
    shared::{
        install_abort_template_bindings, install_attr_template_bindings,
        install_constructor_constant_template_bindings,
        install_css_style_declaration_template_accessors,
    },
    specs::ConstructorSpec,
    speech_synthesis::install_speech_synthesis_template_bindings,
    storage_access::install_storage_access_template_bindings,
    streams::install_stream_template_bindings,
    style_font_template::install_style_font_template_bindings,
    svg_runtime::install_svg_template_bindings,
    touch_runtime::install_touch_template_bindings,
    view_transition_runtime::install_view_transition_template_bindings,
    web_audio_runtime::{
        audio_buffer_get_channel_data_callback, install_offline_audio_context_bindings,
        install_web_audio_template_bindings,
    },
    webrtc::install_webrtc_template_bindings,
    websocket::{install_websocket_bindings, install_websocket_stream_bindings},
    window_runtime::storage_bucket_caches_getter_callback,
    window_runtime::storage_bucket_durability_callback,
    window_runtime::storage_bucket_estimate_callback,
    window_runtime::storage_bucket_expires_callback,
    window_runtime::storage_bucket_get_directory_callback,
    window_runtime::storage_bucket_indexed_db_getter_callback,
    window_runtime::storage_bucket_manager_delete_callback,
    window_runtime::storage_bucket_manager_keys_callback,
    window_runtime::storage_bucket_manager_open_callback,
    window_runtime::storage_bucket_name_getter_callback,
    window_runtime::storage_bucket_persist_callback,
    window_runtime::storage_bucket_persisted_callback,
    window_runtime::storage_bucket_set_expires_callback,
};
use crate::{
    detached_css_style::install_css_style_declaration_template_bindings,
    dom_parser::install_dom_parser_template_bindings, native_bridge::element, network_host,
    util::v8str,
};
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "CSSStyleDeclaration", enumerable)]
struct CssStyleDeclarationTemplateMethodsDeclaration {
    #[webapi(
        method = "getPropertyValue",
        length = 1,
        callback = element::style_get_property_value_callback
    )]
    get_property_value: (),

    #[webapi(
        method = "getPropertyPriority",
        length = 1,
        callback = element::style_get_property_priority_callback
    )]
    get_property_priority: (),

    #[webapi(method = "setProperty", length = 2, callback = element::style_set_property_callback)]
    set_property: (),

    #[webapi(
        method = "removeProperty",
        length = 1,
        callback = element::style_remove_property_callback
    )]
    remove_property: (),

    #[webapi(method = "item", length = 1, callback = element::style_item_callback)]
    item: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLDocument", enumerable)]
struct HtmlDocumentTemplateAccessorsDeclaration {
    #[webapi(
        accessor_property = "location",
        getter = document_location_getter,
        setter = document_location_setter
    )]
    location: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Animation", enumerable)]
struct AnimationTemplateAccessorsDeclaration {
    #[webapi(accessor_property = "playState", getter = animation_play_state_getter)]
    play_state: (),

    #[webapi(accessor_property, getter = animation_pending_getter)]
    pending: (),

    #[webapi(accessor_property, getter = animation_ready_getter)]
    ready: (),

    #[webapi(accessor_property, getter = animation_finished_getter)]
    finished: (),

    #[webapi(
        accessor_property,
        getter = animation_effect_getter,
        setter = animation_effect_setter
    )]
    effect: (),

    #[webapi(
        accessor_property,
        getter = animation_timeline_getter,
        setter = animation_timeline_setter
    )]
    timeline: (),

    #[webapi(
        accessor_property = "startTime",
        getter = animation_start_time_getter,
        setter = animation_start_time_setter
    )]
    start_time: (),

    #[webapi(
        accessor_property,
        getter = animation_onfinish_getter,
        setter = animation_onfinish_setter
    )]
    onfinish: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "HTMLCanvasElement", enumerable)]
struct HtmlCanvasElementTemplateMethodsDeclaration {
    #[webapi(method = "getContext", length = 1, callback = element::canvas_get_context_callback)]
    get_context: (),

    #[webapi(method = "toDataURL", length = 0, callback = element::canvas_to_data_url_callback)]
    to_data_url: (),

    #[webapi(
        method = "transferControlToOffscreen",
        length = 0,
        callback = element::canvas_transfer_control_to_offscreen_callback
    )]
    transfer_control_to_offscreen: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "AudioBuffer", enumerable)]
struct AudioBufferTemplateMethodsDeclaration {
    #[webapi(
        method = "getChannelData",
        length = 1,
        callback = audio_buffer_get_channel_data_callback
    )]
    get_channel_data: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Worker", enumerable)]
struct WorkerTemplateMethodsDeclaration {
    #[webapi(
        method = "postMessage",
        length = 1,
        callback = super::super::worker_host::worker_post_message_callback
    )]
    post_message: (),

    #[webapi(
        method = "terminate",
        length = 0,
        callback = super::super::worker_host::worker_terminate_callback
    )]
    terminate: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DataTransfer", enumerable)]
struct DataTransferTemplateMethodsDeclaration {
    #[webapi(
        method = "getData",
        length = 1,
        callback = super::super::file_api::data_transfer_get_data_callback
    )]
    get_data: (),

    #[webapi(
        method = "setData",
        length = 2,
        callback = super::super::file_api::data_transfer_set_data_callback
    )]
    set_data: (),

    #[webapi(
        method = "clearData",
        length = 1,
        callback = super::super::file_api::data_transfer_clear_data_callback
    )]
    clear_data: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DataTransferItem", enumerable)]
struct DataTransferItemTemplateMethodsDeclaration {
    #[webapi(
        method = "getAsFile",
        length = 0,
        callback = super::super::file_api::data_transfer_item_get_as_file_callback
    )]
    get_as_file: (),

    #[webapi(
        method = "getAsString",
        length = 1,
        callback = super::super::file_api::data_transfer_item_get_as_string_callback
    )]
    get_as_string: (),

    #[webapi(
        method = "webkitGetAsEntry",
        length = 0,
        callback = super::super::file_api::data_transfer_item_webkit_get_as_entry_callback
    )]
    webkit_get_as_entry: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "DataTransferItemList", enumerable)]
struct DataTransferItemListTemplateMethodsDeclaration {
    #[webapi(
        intrinsic_data_property = v8::Intrinsic::ArrayProtoValues,
        symbol = "iterator"
    )]
    iterator: (),

    #[webapi(
        method = "add",
        length = 2,
        callback = super::super::file_api::data_transfer_item_list_add_callback
    )]
    add: (),

    #[webapi(
        method = "item",
        length = 1,
        callback = super::super::file_api::data_transfer_item_list_item_callback
    )]
    item: (),

    #[webapi(
        method = "remove",
        length = 1,
        callback = super::super::file_api::data_transfer_item_list_remove_callback
    )]
    remove: (),

    #[webapi(
        method = "clear",
        length = 0,
        callback = super::super::file_api::data_transfer_item_list_clear_callback
    )]
    clear: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileSystemFileEntry", enumerable)]
struct FileSystemFileEntryTemplateMethodsDeclaration {
    #[webapi(
        method = "file",
        length = 1,
        callback = super::super::file_api::file_system_file_entry_file_callback
    )]
    file: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileSystemDirectoryEntry", enumerable)]
struct FileSystemDirectoryEntryTemplateMethodsDeclaration {
    #[webapi(
        method = "createReader",
        length = 0,
        callback = super::super::file_api::file_system_directory_entry_create_reader_callback
    )]
    create_reader: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "FileSystemDirectoryReader", enumerable)]
struct FileSystemDirectoryReaderTemplateMethodsDeclaration {
    #[webapi(
        method = "readEntries",
        length = 1,
        callback = super::super::file_api::file_system_directory_reader_read_entries_callback
    )]
    read_entries: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "StorageBucketManager", enumerable)]
struct StorageBucketManagerTemplateMethodsDeclaration {
    #[webapi(method = "open", length = 1, callback = storage_bucket_manager_open_callback)]
    open: (),

    #[webapi(method = "keys", length = 0, callback = storage_bucket_manager_keys_callback)]
    keys: (),

    #[webapi(
        method = "delete",
        length = 1,
        callback = storage_bucket_manager_delete_callback
    )]
    delete: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "StorageBucket", enumerable)]
struct StorageBucketTemplateMethodsDeclaration {
    #[webapi(accessor_property, getter = storage_bucket_name_getter_callback)]
    name: (),

    #[webapi(
        accessor_property = "indexedDB",
        getter = storage_bucket_indexed_db_getter_callback
    )]
    indexed_db: (),

    #[webapi(accessor_property, getter = storage_bucket_caches_getter_callback)]
    caches: (),

    #[webapi(method = "persist", length = 0, callback = storage_bucket_persist_callback)]
    persist: (),

    #[webapi(method = "persisted", length = 0, callback = storage_bucket_persisted_callback)]
    persisted: (),

    #[webapi(method = "estimate", length = 0, callback = storage_bucket_estimate_callback)]
    estimate: (),

    #[webapi(
        method = "durability",
        length = 0,
        callback = storage_bucket_durability_callback
    )]
    durability: (),

    #[webapi(
        method = "setExpires",
        length = 1,
        callback = storage_bucket_set_expires_callback
    )]
    set_expires: (),

    #[webapi(method = "expires", length = 0, callback = storage_bucket_expires_callback)]
    expires: (),

    #[webapi(
        method = "getDirectory",
        length = 0,
        callback = storage_bucket_get_directory_callback
    )]
    get_directory: (),
}

pub(super) fn install_constructor_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    spec: ConstructorSpec,
    profile: TemplateBuildProfile,
) {
    install_node_mixin_unscopables(scope, template, spec.name);
    install_constructor_constant_template_bindings(scope, template, spec.name);
    install_css_style_declaration_template_accessors(scope, template, spec.name);
    install_abort_template_bindings(scope, template, spec.name);
    install_attr_template_bindings(scope, template, spec.name);
    install_dom_quad_template_bindings(scope, template, spec.name);
    install_dom_rect_template_bindings(scope, template, spec.name);
    install_dom_rect_list_template_bindings(scope, template, spec.name);
    install_dom_exception_template_bindings(scope, template, spec.name);
    install_dom_implementation_template_bindings(scope, template, spec.name);
    install_custom_element_registry_template_bindings(scope, template, spec.name);
    install_text_codec_template_bindings(scope, template, spec.name);
    install_geometry_template_bindings(scope, template, spec.name, profile);
    if spec.name == "ImageData" {
        install_image_data_template_bindings(scope, template);
    }
    if spec.name == "MediaQueryList" {
        install_media_query_list_template_bindings(scope, template);
    }
    install_media_cue_template_bindings(scope, template, spec.name);
    install_media_source_template_bindings(scope, template, spec.name);
    install_message_port_template_bindings(scope, template, spec.name);
    if spec.name == "BroadcastChannel" {
        install_broadcast_channel_template_bindings(scope, template);
    }
    if spec.name == "EventSource" {
        network_host::install_event_source_bindings(scope, template);
    }
    if spec.name == "IdleDetector" {
        install_idle_detector_template_bindings(scope, template);
    }
    if spec.name == "IdleDeadline" {
        crate::window_host::install_idle_deadline_template_bindings(scope, template);
    }
    if spec.name == "Notification" {
        install_notification_template_bindings(scope, template);
    }
    install_animation_template_bindings(scope, template, spec.name);
    crate::blob::install_blob_template_bindings(scope, template, spec.name);
    install_css_style_declaration_template_bindings(scope, template, spec.name);
    install_event_template_bindings(scope, template, spec);
    install_media_file_template_bindings(scope, template, spec.name);
    install_observer_template_bindings(scope, template, spec.name);
    install_style_font_template_bindings(scope, template, spec);
    install_stream_template_bindings(scope, template, spec.name);
    crate::native_bridge::install_character_data_template_bindings(scope, template, spec.name);
    crate::native_bridge::install_node_template_bindings(scope, template, spec.name);
    crate::native_bridge::install_document_template_bindings(scope, template, spec.name);
    crate::native_bridge::install_collection_template_bindings(scope, template, spec.name);
    crate::native_bridge::install_traversal_template_bindings(scope, template, spec.name);
    crate::native_bridge::element::install_global_event_handler_template_bindings(
        scope, template, spec.name,
    );
    crate::native_bridge::element::install_element_template_bindings(scope, template, spec.name);
    crate::native_bridge::element::install_element_internals_template_bindings(
        scope, template, spec.name,
    );
    crate::native_bridge::element::install_text_track_template_bindings(scope, template, spec.name);
    crate::native_bridge::document::install_caret_position_template_bindings(
        scope, template, spec.name,
    );
    crate::native_bridge::document::install_named_node_map_template_bindings(
        scope, template, spec.name,
    );
    crate::native_bridge::document::install_xpath_template_bindings(scope, template, spec.name);
    crate::observer_runtime::install_intersection_observer_template_accessors(
        scope, template, spec.name,
    );
    crate::network_host::install_progress_event_template_bindings(scope, template, spec.name);
    install_svg_template_bindings(scope, template, spec.name);
    install_opfs_constructor_template_bindings(scope, template, spec.name);
    install_css_typed_om_template_bindings(scope, template, spec.name);
    install_css_stylesheet_template_bindings(scope, template, spec.name);
    install_range_template_bindings(scope, template, spec.name);
    install_indexed_db_template_bindings(scope, template, spec.name);
    install_selection_template_bindings(scope, template, spec.name);
    install_file_api_template_bindings(scope, template, spec.name);
    install_dom_parser_template_bindings(scope, template, spec.name);
    install_performance_template_bindings(scope, template, spec.name);
    install_crypto_template_bindings(scope, template, spec.name);
    install_navigator_template_bindings(scope, template, spec.name);
    install_screen_template_bindings(scope, template, spec.name);
    install_visual_viewport_template_bindings(scope, template, spec.name);
    install_speech_synthesis_template_bindings(scope, template, spec.name);
    install_storage_access_template_bindings(scope, template, spec.name);
    install_touch_template_bindings(scope, template, spec.name);
    install_view_transition_template_bindings(scope, template, spec.name);
    install_web_audio_template_bindings(scope, template, spec.name);
    install_webrtc_template_bindings(scope, template, spec.name);
    crate::context_bootstrap::navigation_activation::install_navigation_activation_template_bindings(
        scope, template, spec.name,
    );

    match spec.name {
        "HTMLDocument" => {
            let proto = template.prototype_template(scope);
            HtmlDocumentTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
        }
        "XMLHttpRequestEventTarget" => {
            network_host::install_xml_http_request_event_target_bindings(scope, template);
        }
        "XMLHttpRequest" => {
            network_host::install_xml_http_request_bindings(scope, template);
        }
        "Headers" => {
            network_host::install_headers_template_bindings(scope, template);
        }
        "WebSocket" => {
            install_websocket_bindings(scope, template);
        }
        "WebSocketStream" => {
            install_websocket_stream_bindings(scope, template);
        }
        "Animation" => {
            let proto = template.prototype_template(scope);
            AnimationTemplateAccessorsDeclaration::initialize_prototype_template(scope, proto);
        }
        "StorageManager" => {
            install_storage_manager_constructor_template_bindings(scope, template, true);
        }
        "StorageBucketManager" => {
            let proto = template.prototype_template(scope);
            StorageBucketManagerTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        "StorageBucket" => {
            let proto = template.prototype_template(scope);
            StorageBucketTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "CSSStyleDeclaration" => {
            let proto = template.prototype_template(scope);
            CssStyleDeclarationTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        "DOMTokenList" => {
            element::install_dom_token_list_prototype_bindings(scope, template);
        }
        "Request" => {
            network_host::install_request_bindings(scope, template);
        }
        "Response" => {
            network_host::install_response_bindings(scope, template);
        }
        "HTMLCanvasElement" => {
            let proto = template.prototype_template(scope);
            HtmlCanvasElementTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        "HTMLFormElement" => {
            element::install_html_form_element_prototype_bindings(scope, template);
        }
        "HTMLSelectElement" => {
            element::install_html_select_element_prototype_bindings(scope, template);
        }
        "History" => {
            install_history_bindings(scope, template);
        }
        "Navigation" => {
            install_navigation_bindings(scope, template);
        }
        "OfflineAudioContext" => {
            install_offline_audio_context_bindings(scope, template);
        }
        "AudioBuffer" => {
            let proto = template.prototype_template(scope);
            AudioBufferTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "Worker" => {
            let proto = template.prototype_template(scope);
            WorkerTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "DataTransfer" => {
            let proto = template.prototype_template(scope);
            DataTransferTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "DataTransferItem" => {
            let proto = template.prototype_template(scope);
            DataTransferItemTemplateMethodsDeclaration::initialize_prototype_template(scope, proto);
        }
        "DataTransferItemList" => {
            let proto = template.prototype_template(scope);
            DataTransferItemListTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        "FileSystemFileEntry" => {
            let proto = template.prototype_template(scope);
            FileSystemFileEntryTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        "FileSystemDirectoryEntry" => {
            let proto = template.prototype_template(scope);
            FileSystemDirectoryEntryTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        "FileSystemDirectoryReader" => {
            let proto = template.prototype_template(scope);
            FileSystemDirectoryReaderTemplateMethodsDeclaration::initialize_prototype_template(
                scope, proto,
            );
        }
        _ => {}
    }
}

const CHARACTER_DATA_UNSCOPABLES: &[&str] = &["after", "before", "remove", "replaceWith"];
const DOCUMENT_UNSCOPABLES: &[&str] = &["append", "fullscreen", "prepend", "replaceChildren"];
const DOCUMENT_FRAGMENT_UNSCOPABLES: &[&str] = &["append", "prepend", "replaceChildren"];
const DOCUMENT_TYPE_UNSCOPABLES: &[&str] = &["after", "before", "remove", "replaceWith"];
const ELEMENT_UNSCOPABLES: &[&str] = &[
    "after",
    "append",
    "before",
    "prepend",
    "remove",
    "replaceChildren",
    "replaceWith",
    "slot",
];

fn install_node_mixin_unscopables<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
    interface_name: &str,
) {
    let names = match interface_name {
        "CharacterData" => CHARACTER_DATA_UNSCOPABLES,
        "Document" => DOCUMENT_UNSCOPABLES,
        "DocumentFragment" => DOCUMENT_FRAGMENT_UNSCOPABLES,
        "DocumentType" => DOCUMENT_TYPE_UNSCOPABLES,
        "Element" => ELEMENT_UNSCOPABLES,
        _ => return,
    };
    let unscopables = v8::ObjectTemplate::new(scope);
    for &name in names {
        unscopables.set(
            v8str(scope, name).into(),
            v8::Boolean::new(scope, true).into(),
        );
    }
    template.prototype_template(scope).set_with_attr(
        v8::Symbol::get_unscopables(scope).into(),
        unscopables.into(),
        v8::PropertyAttribute::READ_ONLY | v8::PropertyAttribute::DONT_ENUM,
    );
}
