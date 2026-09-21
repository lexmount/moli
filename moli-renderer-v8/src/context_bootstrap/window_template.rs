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
use crate::web_api_interfaces;
use crate::{
    network_host,
    queue_microtask::window_queue_microtask_callback,
    util::{
        call_script_visible_function, get_private_value, global_constructor_object,
        global_constructor_prototype, set_private_value, v8str,
    },
    window_host,
};
use anyhow::{Result, anyhow};
use moli_webapi_declare::WebApiFunctionTemplate;

const WINDOW_NAMED_PROPERTIES_READY_SLOT: &str = "__moliWindowNamedPropertiesReady";
const WINDOW_NAMED_PROPERTIES_REFLECT_SET_SLOT: &str = "__moliWindowNamedPropertiesReflectSet";

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Window, enumerable)]
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

    #[webapi(method, length = 0, callback = window_obsolete_noop_callback)]
    clear_immediate: (),

    #[webapi(method, length = 0, callback = window_stop_callback)]
    stop: (),

    #[webapi(method, length = 0, callback = window_obsolete_noop_callback)]
    print: (),

    #[webapi(method, length = 0, callback = window_open_callback)]
    open: (),

    #[webapi(method, length = 0, callback = window_noop_callback)]
    close: (),

    #[webapi(method, length = 0, callback = window_focus_callback)]
    focus: (),

    #[webapi(method, length = 0, callback = window_blur_callback)]
    blur: (),

    #[webapi(method, length = 0, callback = window_const_false_callback)]
    find: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Window, enumerable)]
struct WindowObsoleteTemplateMethodsDeclaration {
    #[webapi(method, length = 0, callback = window_obsolete_noop_callback)]
    capture_events: (),

    #[webapi(method, length = 0, callback = window_obsolete_noop_callback)]
    release_events: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Window, enumerable)]
struct WindowPostNetworkTemplateMethodsDeclaration {
    #[webapi(
        method = "createImageBitmap",
        length = 1,
        callback = window_create_image_bitmap_callback,
        returns_promise
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
        callback = window_host::window_cancel_animation_frame_callback
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
        callback = window_host::window_cancel_idle_callback
    )]
    cancel_idle_callback: (),

    #[webapi(method, length = 1, callback = window_btoa_callback)]
    btoa: (),

    #[webapi(method, length = 1, callback = window_atob_callback)]
    atob: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Window, enumerable)]
struct WindowSelectionTemplateMethodsDeclaration {
    #[webapi(method, length = 0, callback = window_get_selection_callback)]
    get_selection: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Window, enumerable)]
struct WindowMediaTemplateMethodsDeclaration {
    #[webapi(method, length = 1, callback = window_match_media_callback)]
    match_media: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::Window, enumerable)]
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

    #[webapi(accessor_property, getter = window_closed_getter)]
    closed: (),

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
#[webapi(interface = web_api_interfaces::Window, enumerable)]
struct WindowPostRuntimeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = window_opener_getter,
        setter = window_opener_setter
    )]
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
#[webapi(interface = web_api_interfaces::Window, enumerable)]
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
            .setter(window_indexed_property_setter)
            .query(window_indexed_property_query)
            .deleter(window_indexed_property_deleter)
            .enumerator(window_indexed_property_enumerator)
            .definer(window_indexed_property_definer)
            .descriptor(window_indexed_property_descriptor),
    );
    // Window is a [Global] WebIDL interface. Blink installs its members on the
    // concrete global template; Window.prototype only carries constructor
    // metadata. Installing on this aggregate template also gives accessor
    // functions the actual WindowProxy as `this`.
    WindowIdentityAccessorsDeclaration::initialize_prototype_template(scope, window_template);
    WindowEarlyTemplateMethodsDeclaration::initialize_prototype_template(scope, window_template);
    WindowObsoleteTemplateMethodsDeclaration::initialize_prototype_template(scope, window_template);
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

fn reject_window_named_properties_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    // V8 installs a constructor property while instantiating the intermediate
    // FunctionTemplate. Bootstrap removes it before exposing this exotic object.
    if !get_private_value(scope, args.holder(), WINDOW_NAMED_PROPERTIES_READY_SLOT)
        .is_some_and(|value| value.is_true())
    {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn set_window_named_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    key: v8::Local<'s, v8::Name>,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let holder = args.holder();
    let Some(reflect_set) =
        get_private_value(scope, holder, WINDOW_NAMED_PROPERTIES_REFLECT_SET_SLOT)
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return v8::Intercepted::kNo;
    };
    if key.strict_equals(v8::Symbol::get_to_string_tag(scope).into()) {
        rv.set_bool(false);
        return v8::Intercepted::kYes;
    }
    // V8 calls this setter only when the interceptor holder is the receiver.
    // Its ordinary Set fast paths can add properties without calling a definer,
    // so only forward writes which reach an inherited accessor. Both prototypes
    // above WindowProperties are ordinary objects with immutable prototypes.
    let parent = holder
        .get_prototype(scope)
        .expect("WindowProperties has a parent");
    let mut prototype = Some(parent);
    let mut inherited_accessor = false;
    while let Some(object) =
        prototype.and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        let Some(descriptor) = object.get_own_property_descriptor(scope, key) else {
            return v8::Intercepted::kYes;
        };
        if let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor) {
            inherited_accessor = descriptor
                .has_own_property(scope, v8str(scope, "set").into())
                .unwrap_or(false);
            break;
        }
        prototype = object.get_prototype(scope);
    }
    if !inherited_accessor {
        rv.set_bool(false);
        return v8::Intercepted::kYes;
    }
    let undefined = v8::undefined(scope).into();
    if let Some(result) = call_script_visible_function(
        scope,
        reflect_set,
        undefined,
        &[parent, key.into(), value, holder.into()],
        "WindowProperties [[Set]]",
    ) {
        rv.set_bool(result.boolean_value(scope));
    }
    v8::Intercepted::kYes
}

fn window_named_properties_indexed_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    value: v8::Local<'s, v8::Value>,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    let key = v8::String::new(scope, &index.to_string()).expect("indexed property name");
    set_window_named_property(scope, key.into(), value, args, rv)
}

fn window_named_properties_definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _key: v8::Local<'_, v8::Name>,
    _descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    reject_window_named_properties_mutation(scope, args, rv)
}

fn window_named_properties_indexed_definer<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    reject_window_named_properties_mutation(scope, args, rv)
}

fn window_named_properties_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _key: v8::Local<'_, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    reject_window_named_properties_mutation(scope, args, rv)
}

fn window_named_properties_indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    reject_window_named_properties_mutation(scope, args, rv)
}

pub(in crate::context_bootstrap) fn window_named_properties_template<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    event_target: v8::Local<'s, v8::FunctionTemplate>,
) -> v8::Local<'s, v8::FunctionTemplate> {
    let template = v8::FunctionTemplate::new(scope, window_noop_callback);
    template.inherit(event_target);
    let prototype = template.prototype_template(scope);
    prototype.set_immutable_proto();
    prototype.set_with_attr(
        v8::Symbol::get_to_string_tag(scope).into(),
        v8str(scope, "WindowProperties").into(),
        v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY,
    );
    // The getters implement named-property visibility themselves. The mutation
    // hooks must also see real own properties and symbols, so are not NON_MASKING.
    prototype.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(window_named_property_getter)
            .setter(set_window_named_property)
            .query(window_named_property_query)
            .definer(window_named_properties_definer)
            .deleter(window_named_properties_deleter),
    );
    prototype.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(window_named_properties_indexed_property_getter)
            .setter(window_named_properties_indexed_setter)
            .query(window_named_properties_indexed_property_query)
            .definer(window_named_properties_indexed_definer)
            .deleter(window_named_properties_indexed_deleter),
    );
    template
}

pub(super) fn install_window_named_properties_object(
    scope: &mut v8::PinScope<'_, '_>,
) -> Result<()> {
    let window_prototype = global_constructor_prototype(scope, "Window")
        .ok_or_else(|| anyhow!("missing Window.prototype for named properties object"))?;
    let event_target_prototype = global_constructor_prototype(scope, "EventTarget")
        .ok_or_else(|| anyhow!("missing EventTarget.prototype for named properties object"))?;
    let window_constructor = global_constructor_object(scope, "Window")
        .ok_or_else(|| anyhow!("missing Window constructor"))?;
    let event_target_constructor = global_constructor_object(scope, "EventTarget")
        .ok_or_else(|| anyhow!("missing EventTarget constructor"))?;
    if !window_constructor
        .set_prototype(scope, event_target_constructor.into())
        .unwrap_or(false)
    {
        return Err(anyhow!("failed to link Window constructor to EventTarget"));
    }

    let named_properties = window_prototype
        .get_prototype(scope)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing Window named properties object"))?;
    if !named_properties
        .get_prototype(scope)
        .is_some_and(|value| value.strict_equals(event_target_prototype.into()))
    {
        return Err(anyhow!("invalid Window named properties prototype chain"));
    }
    if !named_properties
        .delete(scope, v8str(scope, "constructor").into())
        .unwrap_or(false)
    {
        return Err(anyhow!(
            "failed to remove Window named properties constructor"
        ));
    }
    let global = scope.get_current_context().global(scope);
    let reflect = global
        .get(scope, v8str(scope, "Reflect").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing Reflect for Window named properties object"))?;
    let reflect_set = reflect
        .get(scope, v8str(scope, "set").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing Reflect.set for Window named properties object"))?;
    set_private_value(
        scope,
        named_properties,
        WINDOW_NAMED_PROPERTIES_REFLECT_SET_SLOT,
        reflect_set.into(),
    );
    let ready = v8::Boolean::new(scope, true);
    set_private_value(
        scope,
        named_properties,
        WINDOW_NAMED_PROPERTIES_READY_SLOT,
        ready.into(),
    );

    Ok(())
}
