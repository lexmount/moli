use super::{
    canvas::window_create_image_bitmap_callback,
    indexed_db::window_indexed_db_getter,
    location_runtime::{window_location_setter, window_navigation_setter},
    media_queries::window_match_media_callback,
    navigation_callbacks::{
        window_history_getter, window_location_getter, window_navigation_getter,
    },
    selection_surface::window_get_selection_callback,
    web_storage::{window_local_storage_getter, window_session_storage_getter},
    window_accessors::*,
    window_events::*,
    window_runtime::*,
};
use crate::{
    network_host,
    queue_microtask::window_queue_microtask_callback,
    util::{global_constructor_prototype, v8str},
    window_host,
};
use anyhow::{Result, anyhow};
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Window", enumerable)]
struct WindowEarlyTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = window_host::window_set_timeout_callback)]
    set_timeout: (),

    #[webapi(method, length = 1, callback = window_host::window_set_interval_callback)]
    set_interval: (),

    #[webapi(method, length = 0, callback = window_host::window_clear_timer_callback)]
    clear_timeout: (),

    #[webapi(method, length = 0, callback = window_host::window_clear_timer_callback)]
    clear_interval: (),

    #[webapi(method, length = 1, callback = window_host::window_post_message_callback)]
    post_message: (),

    #[webapi(
        method,
        length = 1,
        callback = window_queue_microtask_callback
    )]
    queue_microtask: (),

    #[webapi(
        method,
        length = 1,
        callback = window_host::window_get_computed_style_callback
    )]
    get_computed_style: (),

    #[webapi(method, length = 1, callback = window_structured_clone_callback)]
    structured_clone: (),

    #[webapi(method, length = 0, callback = window_noop_callback)]
    clear_immediate: (),

    #[webapi(method, length = 0, callback = window_stop_callback)]
    stop: (),

    #[webapi(method, length = 0, callback = window_noop_callback)]
    print: (),

    #[webapi(method, length = 0, callback = window_open_callback)]
    open: (),

    #[webapi(method, length = 0, callback = window_noop_callback)]
    close: (),

    #[webapi(method, length = 0, callback = window_noop_callback)]
    focus: (),

    #[webapi(method, length = 0, callback = window_noop_callback)]
    blur: (),

    #[webapi(method, length = 0, callback = window_const_false_callback)]
    find: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Window", enumerable)]
struct WindowPostNetworkTemplateMethodsDeclaration {
    #[webapi(
        method = "createImageBitmap",
        length = 1,
        callback = window_create_image_bitmap_callback
    )]
    create_image_bitmap: (),

    #[webapi(method, length = 0, callback = window_alert_callback)]
    alert: (),

    #[webapi(method, length = 0, callback = window_confirm_callback)]
    confirm: (),

    #[webapi(method, length = 0, callback = window_prompt_callback)]
    prompt: (),

    #[webapi(method, length = 1, callback = window_report_error_callback)]
    report_error: (),

    #[webapi(method, length = 0, callback = window_host::window_scroll_to_callback)]
    scroll_to: (),

    #[webapi(method, length = 0, callback = window_host::window_scroll_to_callback)]
    scroll: (),

    #[webapi(method, length = 0, callback = window_host::window_scroll_by_callback)]
    scroll_by: (),

    #[webapi(
        method,
        length = 1,
        callback = window_host::window_request_animation_frame_callback
    )]
    request_animation_frame: (),

    #[webapi(
        method,
        length = 1,
        callback = window_host::window_clear_timer_callback
    )]
    cancel_animation_frame: (),

    #[webapi(
        method,
        length = 1,
        callback = window_host::window_request_idle_callback
    )]
    request_idle_callback: (),

    #[webapi(
        method,
        length = 1,
        callback = window_host::window_clear_timer_callback
    )]
    cancel_idle_callback: (),

    #[webapi(method, length = 1, callback = window_btoa_callback)]
    btoa: (),

    #[webapi(method, length = 1, callback = window_atob_callback)]
    atob: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Window", enumerable)]
struct WindowSelectionTemplateMethodsDeclaration {
    #[webapi(method, length = 0, callback = window_get_selection_callback)]
    get_selection: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Window", enumerable)]
struct WindowMediaTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = window_match_media_callback)]
    match_media: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Window", enumerable)]
struct WindowIdentityAccessorsDeclaration {
    #[webapi(accessor_property, getter = window_window_getter)]
    window: (),

    #[webapi(accessor_property = "self", getter = window_self_getter)]
    self_: (),

    #[webapi(accessor_property, getter = window_top_getter)]
    top: (),

    #[webapi(accessor_property, getter = window_parent_getter)]
    parent: (),

    #[webapi(accessor_property, getter = window_frames_getter)]
    frames: (),

    #[webapi(accessor_property, getter = window_frame_element_getter)]
    frame_element: (),

    #[webapi(accessor_property, getter = window_document_getter)]
    document: (),

    #[webapi(
        accessor_property,
        dont_delete,
        getter = window_location_getter,
        setter = window_location_setter
    )]
    location: (),

    #[webapi(accessor_property, getter = window_console_getter)]
    console: (),

    #[webapi(
        accessor_property,
        getter = window_event_getter,
        setter = window_event_setter
    )]
    event: (),

    #[webapi(
        accessor_property,
        getter = window_onerror_getter_function,
        setter = window_onerror_setter_function
    )]
    onerror: (),

    #[webapi(
        accessor_property,
        getter = window_onunhandledrejection_getter_function,
        setter = window_onunhandledrejection_setter_function
    )]
    onunhandledrejection: (),

    #[webapi(
        accessor_property,
        getter = window_onrejectionhandled_getter_function,
        setter = window_onrejectionhandled_setter_function
    )]
    onrejectionhandled: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Window", enumerable)]
struct WindowPostRuntimeAccessorsDeclaration {
    #[webapi(accessor_property, getter = window_opener_getter)]
    opener: (),

    #[webapi(accessor_property, getter = window_inner_width_getter)]
    inner_width: (),

    #[webapi(accessor_property, getter = window_inner_height_getter)]
    inner_height: (),

    #[webapi(accessor_property, getter = window_outer_width_getter)]
    outer_width: (),

    #[webapi(accessor_property, getter = window_outer_height_getter)]
    outer_height: (),

    #[webapi(accessor_property, getter = window_device_pixel_ratio_getter)]
    device_pixel_ratio: (),

    #[webapi(accessor_property, getter = window_credentialless_getter)]
    credentialless: (),

    #[webapi(
        accessor_property = "crossOriginIsolated",
        getter = window_cross_origin_isolated_getter
    )]
    cross_origin_isolated: (),

    #[webapi(accessor_property, getter = window_navigator_getter)]
    navigator: (),

    #[webapi(accessor_property, getter = window_history_getter)]
    history: (),

    #[webapi(
        accessor_property,
        getter = window_navigation_getter,
        setter = window_navigation_setter
    )]
    navigation: (),

    #[webapi(accessor_property, getter = window_screen_getter)]
    screen: (),

    #[webapi(
        accessor_property = "speechSynthesis",
        getter = window_speech_synthesis_getter
    )]
    speech_synthesis: (),

    #[webapi(accessor_property, getter = window_performance_getter)]
    performance: (),

    #[webapi(accessor_property, getter = window_visual_viewport_getter)]
    visual_viewport: (),

    #[webapi(accessor_property, getter = window_scroll_x_getter)]
    scroll_x: (),

    #[webapi(accessor_property, getter = window_scroll_y_getter)]
    scroll_y: (),

    #[webapi(accessor_property, getter = window_scroll_x_getter)]
    page_x_offset: (),

    #[webapi(accessor_property, getter = window_scroll_y_getter)]
    page_y_offset: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "Window", enumerable)]
struct WindowStorageAccessorsDeclaration {
    #[webapi(accessor_property, getter = window_local_storage_getter)]
    local_storage: (),

    #[webapi(accessor_property, getter = window_session_storage_getter)]
    session_storage: (),

    #[webapi(
        accessor_property = "indexedDB",
        getter = window_indexed_db_getter
    )]
    indexed_db: (),

    #[webapi(accessor_property, getter = window_custom_elements_getter)]
    custom_elements: (),

    #[webapi(accessor_property, getter = window_length_getter)]
    length: (),
}

pub(crate) fn install_window_own_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    window_template: v8::Local<'s, v8::ObjectTemplate>,
) {
    window_template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(window_indexed_property_getter)
            .query(window_indexed_property_query)
            .enumerator(window_indexed_property_enumerator)
            .descriptor(window_indexed_property_descriptor),
    );
    // Window is a [Global] WebIDL interface. Blink installs its members on the
    // concrete global template; Window.prototype only carries constructor
    // metadata. Installing on this aggregate template also gives accessor
    // functions the actual WindowProxy as `this`.
    WindowIdentityAccessorsDeclaration::initialize_prototype_template(scope, window_template);
    WindowEarlyTemplateMethodsDeclaration::initialize_prototype_template(scope, window_template);
    network_host::install_window_network_bindings(scope, window_template);
    WindowPostNetworkTemplateMethodsDeclaration::initialize_prototype_template(
        scope,
        window_template,
    );
    WindowPostRuntimeAccessorsDeclaration::initialize_prototype_template(scope, window_template);
    WindowSelectionTemplateMethodsDeclaration::initialize_prototype_template(
        scope,
        window_template,
    );
    WindowStorageAccessorsDeclaration::initialize_prototype_template(scope, window_template);
    WindowMediaTemplateMethodsDeclaration::initialize_prototype_template(scope, window_template);
}

pub(super) fn install_window_named_properties_object(
    scope: &mut v8::PinScope<'_, '_>,
) -> Result<()> {
    let window_prototype = global_constructor_prototype(scope, "Window")
        .ok_or_else(|| anyhow!("missing Window.prototype for named properties object"))?;
    let event_target_prototype = global_constructor_prototype(scope, "EventTarget")
        .ok_or_else(|| anyhow!("missing EventTarget.prototype for named properties object"))?;

    // Blink installs Window's named getter on an otherwise anonymous
    // WindowProperties object between Window.prototype and
    // EventTarget.prototype. Keeping the interceptor off the global instance
    // means ordinary Window properties (especially the hot `document` getter)
    // resolve before named DOM access is considered.
    let named_properties_template = v8::ObjectTemplate::new(scope);
    named_properties_template.set_with_attr(
        v8::Symbol::get_to_string_tag(scope).into(),
        v8str(scope, "WindowProperties").into(),
        v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY,
    );
    named_properties_template.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(window_named_property_getter)
            .query(window_named_property_query)
            .flags(
                // Window named properties must not mask real own/prototype
                // properties. Let V8 skip the interceptor for non-string keys and
                // perform the ordinary lookup before calling us for id/name misses.
                v8::PropertyHandlerFlags::NON_MASKING
                    | v8::PropertyHandlerFlags::ONLY_INTERCEPT_STRINGS,
            ),
    );
    let named_properties = named_properties_template
        .new_instance(scope)
        .ok_or_else(|| anyhow!("failed to create Window named properties object"))?;
    if !named_properties
        .set_prototype(scope, event_target_prototype.into())
        .unwrap_or(false)
    {
        return Err(anyhow!(
            "failed to link Window named properties object to EventTarget.prototype"
        ));
    }
    if !window_prototype
        .set_prototype(scope, named_properties.into())
        .unwrap_or(false)
    {
        return Err(anyhow!(
            "failed to link Window.prototype to named properties object"
        ));
    }
    Ok(())
}
