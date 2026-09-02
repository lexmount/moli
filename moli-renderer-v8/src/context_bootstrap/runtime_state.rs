use super::date_locale_runtime::install_date_locale_runtime_state;
use super::file_api::initialize_file_api_runtime_queues;
use super::shared::*;
use super::webassembly_runtime::install_webassembly_runtime_extensions;
use super::window_events::*;
use super::{
    crypto::install_window_crypto_runtime_state,
    css_runtime::install_css_runtime_state,
    exposed_interfaces::is_lazy_exposed_interface,
    indexed_db::ensure_indexed_db_runtime_state,
    location_runtime::{
        ensure_location_constructor_runtime_state, location_href_slot,
        sync_document_location_runtime_state_from_window,
    },
    navigation_bootstrap::install_window_location_history_navigation_runtime_state,
    navigator_runtime::{bind_window_navigator_identity_seed, install_navigator_runtime_state},
    performance_runtime::install_default_window_performance_seed,
    range_surface::reset_range_runtime_state,
    trusted_types::install_trusted_types_runtime_state,
    web_storage::{
        install_storage_runtime_state, window_local_storage_getter, window_session_storage_getter,
    },
    window_runtime::{build_legacy_storage_info_object, window_noop_callback},
    window_template::install_window_named_properties_object,
};
use crate::{
    document_runtime::{DocumentRuntime, DomHandle},
    native_bridge::{
        self, JsContextHost,
        document::{
            document_all_value_for_receiver, document_cookie_for_receiver,
            set_document_cookie_for_receiver,
        },
    },
    network_host,
    util::{
        callback_data_index_value, callback_data_item, context_host_ptr_from_global_bridge,
        context_host_ptr_from_window_object, create_script_origin_with_base_url, get_private_value,
        script_base_url_continuation_data, set_private_value, throw_type_error, v8_string, v8str,
    },
    webidl,
    webidl_iterator::install_webidl_collection_iterator_intrinsics,
    window_host,
};
use anyhow::{Result, anyhow, bail};
use moli_script::html_script_element_supports_type;
use moli_webapi_declare::WebApiObject;

#[cfg(test)]
mod tests;

pub(crate) const ORIGINAL_WEBASSEMBLY_INSTANCE_CONSTRUCTOR_SLOT: &str =
    "__moliOriginalWebAssemblyInstanceConstructor";
pub(crate) const ORIGINAL_WEBASSEMBLY_COMPILE_ERROR_CONSTRUCTOR_SLOT: &str =
    "__moliOriginalWebAssemblyCompileErrorConstructor";
pub(crate) const ORIGINAL_WEBASSEMBLY_LINK_ERROR_CONSTRUCTOR_SLOT: &str =
    "__moliOriginalWebAssemblyLinkErrorConstructor";
pub(crate) const ORIGINAL_WEBASSEMBLY_GLOBAL_VALUE_GETTER_SLOT: &str =
    "__moliOriginalWebAssemblyGlobalValueGetter";
const WINDOW_INDEXED_DB_SURFACE_SLOT: &str = "moli.Window.indexedDB";
const WINDOW_ORIGIN_RUNTIME_SLOT: &str = "__moliWindowOriginRuntime";
const WINDOW_INTRINSIC_EVAL_SLOT: &str = "__moliWindowIntrinsicEval";
pub(in crate::context_bootstrap) const WINDOW_SECURE_CONTEXT_AVAILABLE_SLOT: &str =
    "__moliWindowSecureContextAvailable";

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "HTMLScriptElement.supports")]
struct HtmlScriptElementSupportsArgs {
    #[webidl(required, name = "type")]
    script_type: String,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "HTMLScriptElement", enumerable)]
struct HtmlScriptElementStaticMethodsDeclaration {
    #[webapi(method, length = 1, callback = html_script_element_supports_callback)]
    supports: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Document", enumerable)]
struct DocumentStaticMethodsDeclaration {
    #[webapi(
        method = "parseHTMLUnsafe",
        length = 1,
        callback = document_parse_html_unsafe_callback
    )]
    parse_html_unsafe: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Document", enumerable)]
struct DocumentPrototypeRuntimeDeclaration {
    #[webapi(
        accessor_property = "designMode",
        getter = native_bridge::document::node_document_design_mode_getter_function,
        setter = native_bridge::document::node_document_design_mode_setter_function
    )]
    design_mode: (),
    #[webapi(accessor_property, getter = document_all_runtime_getter)]
    all: (),
    #[webapi(
        accessor_property,
        getter = document_cookie_runtime_getter,
        setter = document_cookie_runtime_setter
    )]
    cookie: (),
    #[webapi(
        accessor_property,
        getter = native_bridge::document::node_document_style_sheets_getter_function
    )]
    style_sheets: (),
    #[webapi(
        accessor_property,
        getter = native_bridge::document::node_document_adopted_style_sheets_getter_function,
        setter = native_bridge::document::node_document_adopted_style_sheets_setter_function
    )]
    adopted_style_sheets: (),
    #[webapi(
        accessor_property = "fullscreenEnabled",
        getter = document_fullscreen_enabled_getter,
        setter = document_fullscreen_enabled_lenient_setter
    )]
    fullscreen_enabled: (),
    #[webapi(
        accessor_property = "pointerLockElement",
        getter = native_bridge::pointer_lock::document_pointer_lock_element_getter
    )]
    pointer_lock_element: (),
    #[webapi(
        method = "exitPointerLock",
        length = 0,
        callback = native_bridge::pointer_lock::document_exit_pointer_lock_callback
    )]
    exit_pointer_lock: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct WindowComputedStyleMethodDeclaration {
    #[webapi(
        method,
        enumerable,
        length = 1,
        callback = window_host::window_get_computed_style_callback
    )]
    get_computed_style: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", scope_lifetime = 'scope)]
struct WindowPublicSurfaceAccessorsDeclaration<'scope> {
    history_slot: v8::Local<'scope, v8::Value>,
    navigation_slot: v8::Local<'scope, v8::Value>,
    navigator_slot: v8::Local<'scope, v8::Value>,
    screen_slot: v8::Local<'scope, v8::Value>,
    speech_synthesis_slot: v8::Local<'scope, v8::Value>,
    custom_elements_slot: v8::Local<'scope, v8::Value>,
    crypto_slot: v8::Local<'scope, v8::Value>,
    performance_slot: v8::Local<'scope, v8::Value>,
    visual_viewport_slot: v8::Local<'scope, v8::Value>,
    indexed_db_slot: v8::Local<'scope, v8::Value>,
    navigation_name: v8::Local<'scope, v8::Value>,
    screen_name: v8::Local<'scope, v8::Value>,
    performance_name: v8::Local<'scope, v8::Value>,
    visual_viewport_name: v8::Local<'scope, v8::Value>,

    #[webapi(
        accessor_property,
        enumerable,
        getter = window_surface_slot_getter,
        data = self.history_slot
    )]
    history: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = window_surface_slot_getter,
        setter = window_surface_replaceable_setter,
        data = self.navigation_slot,
        setter_data = self.navigation_name
    )]
    navigation: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = window_surface_slot_getter,
        data = self.navigator_slot
    )]
    navigator: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = window_surface_slot_getter,
        setter = window_surface_replaceable_setter,
        data = self.screen_slot,
        setter_data = self.screen_name
    )]
    screen: (),
    #[webapi(
        accessor_property = "speechSynthesis",
        enumerable,
        getter = window_surface_slot_getter,
        data = self.speech_synthesis_slot
    )]
    speech_synthesis: (),
    #[webapi(
        accessor_property = "customElements",
        enumerable,
        getter = window_surface_slot_getter,
        data = self.custom_elements_slot
    )]
    custom_elements: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = window_surface_slot_getter,
        data = self.crypto_slot
    )]
    crypto: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = window_surface_slot_getter,
        setter = window_surface_replaceable_setter,
        data = self.performance_slot,
        setter_data = self.performance_name
    )]
    performance: (),
    #[webapi(
        accessor_property = "visualViewport",
        enumerable,
        getter = window_surface_slot_getter,
        setter = window_surface_replaceable_setter,
        data = self.visual_viewport_slot,
        setter_data = self.visual_viewport_name
    )]
    visual_viewport: (),
    #[webapi(
        accessor_property = "localStorage",
        enumerable,
        getter = window_local_storage_getter
    )]
    local_storage: (),
    #[webapi(
        accessor_property = "sessionStorage",
        enumerable,
        getter = window_session_storage_getter
    )]
    session_storage: (),
    #[webapi(
        accessor_property = "indexedDB",
        enumerable,
        getter = window_surface_slot_getter,
        data = self.indexed_db_slot
    )]
    indexed_db: (),
    #[webapi(
        accessor_property,
        enumerable,
        getter = window_name_runtime_getter,
        setter = window_name_runtime_setter
    )]
    name: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object", enumerable)]
struct WindowLegacyAliasAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = legacy_unforgeable_self_getter,
        setter = replaceable_self_setter
    )]
    self_: (),
    #[webapi(
        accessor_property,
        getter = legacy_unforgeable_parent_getter,
        setter = replaceable_parent_setter
    )]
    parent: (),
    #[webapi(accessor_property, dont_delete, getter = legacy_unforgeable_top_getter)]
    top: (),
    #[webapi(
        accessor_property,
        getter = legacy_unforgeable_frames_getter,
        setter = replaceable_frames_setter
    )]
    frames: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Window", enumerable)]
struct WindowAdditionalReplaceableAccessorsDeclaration<'scope> {
    origin_name: v8::Local<'scope, v8::Value>,
    inner_width_name: v8::Local<'scope, v8::Value>,
    length_name: v8::Local<'scope, v8::Value>,
    event_name: v8::Local<'scope, v8::Value>,
    outer_height_name: v8::Local<'scope, v8::Value>,
    scroll_x_name: v8::Local<'scope, v8::Value>,
    screen_left_name: v8::Local<'scope, v8::Value>,
    screen_top_name: v8::Local<'scope, v8::Value>,
    screen_x_name: v8::Local<'scope, v8::Value>,
    screen_y_name: v8::Local<'scope, v8::Value>,
    #[webapi(
        accessor_property,
        getter = window_origin_replaceable_getter,
        setter = window_surface_replaceable_setter,
        setter_data = self.origin_name
    )]
    origin: (),
    #[webapi(
        accessor_property = "innerWidth",
        getter = window_inner_width_replaceable_getter,
        setter = window_surface_replaceable_setter,
        setter_data = self.inner_width_name
    )]
    inner_width: (),
    #[webapi(
        accessor_property,
        getter = window_length_replaceable_getter,
        setter = window_surface_replaceable_setter,
        setter_data = self.length_name
    )]
    length: (),
    #[webapi(
        accessor_property,
        getter = window_event_replaceable_getter,
        setter = window_surface_replaceable_setter,
        setter_data = self.event_name
    )]
    event: (),
    #[webapi(
        accessor_property = "outerHeight",
        getter = window_outer_height_replaceable_getter,
        setter = window_surface_replaceable_setter,
        setter_data = self.outer_height_name
    )]
    outer_height: (),
    #[webapi(
        accessor_property = "scrollX",
        getter = window_scroll_x_replaceable_getter,
        setter = window_surface_replaceable_setter,
        setter_data = self.scroll_x_name
    )]
    scroll_x: (),
    #[webapi(
        accessor_property = "screenLeft",
        getter = window_zero_replaceable_getter,
        setter = window_surface_replaceable_setter,
        setter_data = self.screen_left_name
    )]
    screen_left: (),
    #[webapi(
        accessor_property = "screenTop",
        getter = window_zero_replaceable_getter,
        setter = window_surface_replaceable_setter,
        setter_data = self.screen_top_name
    )]
    screen_top: (),
    #[webapi(
        accessor_property = "screenX",
        getter = window_zero_replaceable_getter,
        setter = window_surface_replaceable_setter,
        setter_data = self.screen_x_name
    )]
    screen_x: (),
    #[webapi(
        accessor_property = "screenY",
        getter = window_zero_replaceable_getter,
        setter = window_surface_replaceable_setter,
        setter_data = self.screen_y_name
    )]
    screen_y: (),
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct ConsoleObjectDeclaration {
    #[webapi(method, callback = console_log_callback)]
    log: (),
    #[webapi(method, callback = console_info_callback)]
    info: (),
    #[webapi(method, callback = console_warn_callback)]
    warn: (),
    #[webapi(method, callback = console_error_callback)]
    error: (),
    #[webapi(method, callback = console_debug_callback)]
    debug: (),
    #[webapi(method, callback = console_trace_callback)]
    trace: (),
    #[webapi(method, callback = console_noop_callback)]
    clear: (),
    #[webapi(method, callback = console_assert_callback)]
    assert: (),
    #[webapi(method, callback = console_error_callback)]
    exception: (),
    #[webapi(method, callback = console_table_callback)]
    table: (),
    #[webapi(method, callback = console_noop_callback)]
    time: (),
    #[webapi(method, callback = console_noop_callback)]
    time_log: (),
    #[webapi(method, callback = console_noop_callback)]
    time_end: (),
    #[webapi(method, callback = console_noop_callback)]
    count: (),
    #[webapi(method, callback = console_noop_callback)]
    count_reset: (),
    #[webapi(method, callback = console_group_callback)]
    group: (),
    #[webapi(method, callback = console_group_collapsed_callback)]
    group_collapsed: (),
    #[webapi(method, callback = console_noop_callback)]
    group_end: (),
    #[webapi(method, callback = console_profile_callback)]
    profile: (),
    #[webapi(method, callback = console_profile_end_callback)]
    profile_end: (),
}

#[derive(WebApiObject)]
#[webapi(interface = "Object", scope_lifetime = 'scope)]
struct WindowBootstrapGlobalSlotsDeclaration<'scope> {
    #[webapi(slot = WINDOW_CONSOLE_SLOT)]
    console: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = WINDOW_EVENT_SLOT, init = "undefined")]
    event: (),
    #[webapi(data_property = WINDOW_ONERROR_SLOT, init = "null")]
    on_error: (),
    #[webapi(data_property = WINDOW_ONUNHANDLEDREJECTION_SLOT, init = "null")]
    on_unhandled_rejection: (),
    #[webapi(data_property = WINDOW_ONREJECTIONHANDLED_SLOT, init = "null")]
    on_rejection_handled: (),
    #[webapi(data_property = WINDOW_SELF_SLOT)]
    self_value: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = WINDOW_PARENT_SLOT)]
    parent: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = WINDOW_TOP_SLOT)]
    top: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = WINDOW_FRAMES_SLOT)]
    frames: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = "isSecureContext", readonly)]
    is_secure_context: bool,
    #[webapi(data_property = "TEMPORARY")]
    temporary: u32,
    #[webapi(data_property = "PERSISTENT")]
    persistent: u32,
    #[webapi(data_property = "webkitStorageInfo")]
    webkit_storage_info: v8::Local<'scope, v8::Object>,
    #[webapi(data_property = WINDOW_NAME_SLOT)]
    window_name: &'static str,
    #[webapi(data_property = "console")]
    public_console: v8::Local<'scope, v8::Object>,
}

#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WindowEvalGlobalDeclaration<'scope> {
    #[webapi(data_property = "eval")]
    intrinsic_eval: v8::Local<'scope, v8::Value>,
}

#[derive(Default, WebApiObject)]
#[webapi(interface = "Object")]
struct WebAssemblyNamespaceDeclaration {
    #[webapi(method, length = 1, callback = webassembly_namespace_instance_callback)]
    namespace_instance: (),
}

#[cfg(feature = "wpt-extensions")]
#[derive(WebApiObject)]
#[webapi(interface = "Object", prototype = "WebDriver", require_prototype)]
struct WebDriverObjectDeclaration {}

#[cfg(feature = "wpt-extensions")]
#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WebDriverPrototypeDeclaration {
    #[webapi(method, length = 0, callback = webdriver_delete_all_cookies_callback)]
    delete_all_cookies: (),
}

#[cfg(feature = "wpt-extensions")]
#[derive(WebApiObject)]
#[webapi(interface = "Object")]
struct WebDriverPrototypeMetadataDeclaration<'scope> {
    #[webapi(data_property = "constructor")]
    constructor: v8::Local<'scope, v8::Function>,

    #[webapi(to_string_tag, readonly, init = string("WebDriver"))]
    to_string_tag: (),
}

fn legacy_unforgeable_window_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(legacy_unforgeable_window_slot_value(
        scope,
        args.this(),
        WINDOW_SELF_SLOT,
    ));
}

fn legacy_unforgeable_window_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> v8::Local<'s, v8::Value> {
    object_hidden_value(scope, receiver, slot)
        .unwrap_or_else(|| scope.get_current_context().global(scope).into())
}

fn document_fullscreen_enabled_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let valid_receiver =
        native_bridge::node_runtime_and_handle_from_object_or_detached(scope, args.this())
            .ok()
            .is_some_and(|(runtime_ptr, handle)| {
                unsafe { &*runtime_ptr }
                    .dom_host()
                    .node(handle)
                    .is_some_and(crate::dom::native::Node::is_document)
            });
    if !valid_receiver {
        throw_type_error(
            scope,
            "Document.fullscreenEnabled getter called on incompatible receiver.",
        );
        return;
    }
    rv.set_bool(false);
}

fn document_fullscreen_enabled_lenient_setter<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
}

fn define_replaceable_window_property(
    scope: &mut v8::PinScope<'_, '_>,
    receiver: v8::Local<'_, v8::Object>,
    name: &'static str,
    value: v8::Local<'_, v8::Value>,
) {
    match receiver.define_own_property(
        scope,
        v8str(scope, name).into(),
        value,
        v8::PropertyAttribute::NONE,
    ) {
        Some(true) => {}
        Some(false) => throw_type_error(
            scope,
            &format!("Failed to replace non-configurable Window.{name} property."),
        ),
        None => {}
    }
}

fn window_origin_replaceable_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), WINDOW_ORIGIN_RUNTIME_SLOT)
        .or_else(|| {
            let global = scope.get_current_context().global(scope);
            get_private_value(scope, global, WINDOW_ORIGIN_RUNTIME_SLOT)
        })
        .unwrap_or_else(|| v8str(scope, "null").into());
    rv.set(value);
}

fn window_inner_width_replaceable_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let width = super::window_accessors::window_inner_surface_width(scope, args.this());
    rv.set(v8::Number::new(scope, width).into());
}

fn window_outer_height_replaceable_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(
        v8::Number::new(
            scope,
            moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_height,
        )
        .into(),
    );
}

fn window_length_replaceable_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(host_ptr) = context_host_ptr_from_window_object(scope, args.this())
        .or_else(|| context_host_ptr_from_global_bridge(scope))
    else {
        rv.set(v8::Integer::new(scope, 0).into());
        return;
    };
    let host = unsafe { &mut *host_ptr };
    let count = child_context_handle_from_owner(scope, args.this())
        .map(|handle| host.child_browsing_context_child_frame_count(handle))
        .unwrap_or_else(|| host.child_browsing_context_count());
    rv.set(v8::Number::new(scope, count as f64).into());
}

fn window_event_replaceable_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(
        global_hidden_value(scope, WINDOW_EVENT_SLOT)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

fn window_scroll_x_replaceable_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let value = get_private_value(scope, args.this(), WINDOW_SCROLL_X_SLOT)
        .or_else(|| {
            let global = scope.get_current_context().global(scope);
            get_private_value(scope, global, WINDOW_SCROLL_X_SLOT)
        })
        .and_then(|value| value.number_value(scope))
        .unwrap_or(0.0);
    rv.set(v8::Number::new(scope, value).into());
}

fn window_zero_replaceable_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(v8::Integer::new(scope, 0).into());
}

fn define_legacy_unforgeable_window_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let getter = v8::Function::builder(legacy_unforgeable_window_getter)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to build window getter"))?;
    define_get_set_property(
        scope,
        object,
        v8str(scope, "window").into(),
        getter.into(),
        v8::undefined(scope).into(),
        v8::PropertyAttribute::DONT_DELETE,
        "window",
    )
}

fn legacy_unforgeable_self_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(legacy_unforgeable_window_slot_value(
        scope,
        args.this(),
        WINDOW_SELF_SLOT,
    ));
}

fn replaceable_window_alias_set<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    name: &'static str,
) {
    define_replaceable_window_property(scope, args.this(), name, args.get(0));
}

fn replaceable_self_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    replaceable_window_alias_set(scope, args, "self");
}

fn legacy_unforgeable_parent_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(legacy_unforgeable_window_slot_value(
        scope,
        args.this(),
        WINDOW_PARENT_SLOT,
    ));
}

fn replaceable_parent_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    replaceable_window_alias_set(scope, args, "parent");
}

fn legacy_unforgeable_top_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(legacy_unforgeable_window_slot_value(
        scope,
        args.this(),
        WINDOW_TOP_SLOT,
    ));
}

fn legacy_unforgeable_frames_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    rv.set(legacy_unforgeable_window_slot_value(
        scope,
        args.this(),
        WINDOW_FRAMES_SLOT,
    ));
}

fn replaceable_frames_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    replaceable_window_alias_set(scope, args, "frames");
}

fn callback_this_object<'s>(
    _scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> v8::Local<'s, v8::Object> {
    args.this()
}

fn hidden_slot_value_for_receiver<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Value>> {
    if let Some(value) =
        get_private_value(scope, receiver, slot).filter(|value| !value.is_undefined())
    {
        return Some(value);
    }
    if window_surface_slot_uses_legacy_hidden_fallback(slot)
        && let Some(value) =
            object_hidden_value(scope, receiver, slot).filter(|value| !value.is_undefined())
    {
        return Some(value);
    }
    let global = scope.get_current_context().global(scope);
    if let Some(value) =
        get_private_value(scope, global, slot).filter(|value| !value.is_undefined())
    {
        return Some(value);
    }
    if window_surface_slot_uses_legacy_hidden_fallback(slot) {
        return global_hidden_value(scope, slot).filter(|value| !value.is_undefined());
    }
    None
}

fn window_surface_slot_uses_legacy_hidden_fallback(slot: &str) -> bool {
    matches!(slot, WINDOW_CUSTOM_ELEMENTS_SLOT | WINDOW_PERFORMANCE_SLOT)
}

fn window_surface_slot_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some((slot, detached_fallback)) = super::window_receiver::bound_callback_data_item(
        scope,
        &args,
        WINDOW_SURFACE_SLOTS,
        "Window surface slots",
    ) else {
        rv.set_undefined();
        return;
    };
    let receiver = callback_this_object(scope, &args);
    if slot == WINDOW_INDEXED_DB_SURFACE_SLOT {
        if let Some(value) = detached_fallback {
            rv.set(value);
            return;
        }
        match ensure_indexed_db_runtime_state(scope).map(Into::into) {
            Some(value) => rv.set(value),
            None => rv.set_undefined(),
        }
        return;
    }
    match super::window_lazy_surface::ensure_window_lazy_surface_value(scope, receiver, slot) {
        Ok(Some(value)) => {
            rv.set(value);
            return;
        }
        Ok(None) => {}
        Err(error) => {
            throw_error(
                scope,
                &format!("Failed to materialize Window surface: {error}"),
            );
            return;
        }
    }
    match hidden_slot_value_for_receiver(scope, receiver, slot).or(detached_fallback) {
        Some(value) => rv.set(value),
        None => rv.set_undefined(),
    }
}

fn window_surface_replaceable_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(name) = callback_data_item(
        scope,
        &args,
        WINDOW_SURFACE_REPLACEABLE_NAMES,
        "Window surface replaceable names",
    ) else {
        return;
    };
    let receiver = callback_this_object(scope, &args);
    define_replaceable_window_property(scope, receiver, name, args.get(0));
}

fn window_name_runtime_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = callback_this_object(scope, &args);
    let value = object_hidden_value(scope, receiver, WINDOW_NAME_SLOT)
        .unwrap_or_else(|| v8::String::empty(scope).into());
    rv.set(value);
}

fn window_name_runtime_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = callback_this_object(scope, &args);
    let next = args
        .get(0)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_default();
    if let Some(handle) = child_context_handle_from_owner(scope, receiver)
        && let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
    {
        unsafe { &mut *host_ptr }.set_child_browsing_context_name(handle, next.clone());
    }
    define_non_enumerable_string_property(scope, receiver, WINDOW_NAME_SLOT, &next);
}

fn install_public_window_surface_accessors<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let history_slot = window_surface_callback_data(scope, global, WINDOW_HISTORY_SLOT);
    let navigation_slot = window_surface_callback_data(scope, global, WINDOW_NAVIGATION_SLOT);
    let navigator_slot = window_surface_callback_data(scope, global, WINDOW_NAVIGATOR_SLOT);
    let screen_slot = window_surface_callback_data(scope, global, WINDOW_SCREEN_SLOT);
    let speech_synthesis_slot =
        window_surface_callback_data(scope, global, WINDOW_SPEECH_SYNTHESIS_SLOT);
    let custom_elements_slot =
        window_surface_callback_data(scope, global, WINDOW_CUSTOM_ELEMENTS_SLOT);
    let crypto_slot = window_surface_callback_data(scope, global, WINDOW_CRYPTO_SLOT);
    let performance_slot = window_surface_callback_data(scope, global, WINDOW_PERFORMANCE_SLOT);
    let visual_viewport_slot =
        window_surface_callback_data(scope, global, WINDOW_VISUAL_VIEWPORT_SLOT);
    let indexed_db_slot =
        window_surface_callback_data(scope, global, WINDOW_INDEXED_DB_SURFACE_SLOT);
    let callback_data_registry = v8::Array::new_with_elements(
        scope,
        &[
            history_slot,
            navigation_slot,
            navigator_slot,
            screen_slot,
            speech_synthesis_slot,
            custom_elements_slot,
            crypto_slot,
            performance_slot,
            visual_viewport_slot,
            indexed_db_slot,
        ],
    );
    set_private_value(
        scope,
        global,
        WINDOW_SURFACE_CALLBACK_DATA_REGISTRY_SLOT,
        callback_data_registry.into(),
    );
    let navigation_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("navigation").unwrap(),
    );
    let screen_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("screen").unwrap(),
    );
    let performance_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("performance").unwrap(),
    );
    let visual_viewport_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("visualViewport").unwrap(),
    );
    let origin_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("origin").unwrap(),
    );
    let inner_width_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("innerWidth").unwrap(),
    );
    let length_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("length").unwrap(),
    );
    let event_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("event").unwrap(),
    );
    let outer_height_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("outerHeight").unwrap(),
    );
    let scroll_x_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("scrollX").unwrap(),
    );
    let screen_left_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("screenLeft").unwrap(),
    );
    let screen_top_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("screenTop").unwrap(),
    );
    let screen_x_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("screenX").unwrap(),
    );
    let screen_y_name = callback_data_index_value(
        scope,
        window_surface_replaceable_name_index("screenY").unwrap(),
    );

    WindowPublicSurfaceAccessorsDeclaration {
        history_slot,
        navigation_slot,
        navigator_slot,
        screen_slot,
        speech_synthesis_slot,
        custom_elements_slot,
        crypto_slot,
        performance_slot,
        visual_viewport_slot,
        indexed_db_slot,
        navigation_name,
        screen_name,
        performance_name,
        visual_viewport_name,
        history: (),
        navigation: (),
        navigator: (),
        screen: (),
        speech_synthesis: (),
        custom_elements: (),
        crypto: (),
        performance: (),
        visual_viewport: (),
        local_storage: (),
        session_storage: (),
        indexed_db: (),
        name: (),
    }
    .initialize(scope, global)?;
    WindowAdditionalReplaceableAccessorsDeclaration {
        origin_name,
        inner_width_name,
        length_name,
        event_name,
        outer_height_name,
        scroll_x_name,
        screen_left_name,
        screen_top_name,
        screen_x_name,
        screen_y_name,
        origin: (),
        inner_width: (),
        length: (),
        event: (),
        outer_height: (),
        scroll_x: (),
        screen_left: (),
        screen_top: (),
        screen_x: (),
        screen_y: (),
    }
    .initialize(scope, global)?;
    Ok(())
}

fn window_surface_callback_data<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> v8::Local<'s, v8::Value> {
    let index = window_surface_slot_index(slot).expect("Window surface slot should be registered");
    let fallback = hidden_slot_value_for_receiver(scope, global, slot)
        .unwrap_or_else(|| v8::undefined(scope).into());
    super::window_receiver::bound_callback_data(scope, index, global, fallback)
}

pub(super) fn update_window_surface_detached_fallback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: v8::Local<'s, v8::Value>,
) {
    let index = window_surface_slot_index(slot)
        .unwrap_or_else(|| panic!("Window surface slot `{slot}` is not registered"));
    let data = get_private_value(scope, receiver, WINDOW_SURFACE_CALLBACK_DATA_REGISTRY_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .and_then(|registry| registry.get_index(scope, index as u32))
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .unwrap_or_else(|| panic!("missing Window callback data for `{slot}`"));
    assert_eq!(
        data.set_index(scope, 2, value),
        Some(true),
        "failed to update detached Window fallback for `{slot}`"
    );
}

fn window_surface_slot_index(slot: &str) -> Option<usize> {
    WINDOW_SURFACE_SLOTS
        .iter()
        .position(|candidate| *candidate == slot)
}

fn window_surface_replaceable_name_index(name: &str) -> Option<usize> {
    WINDOW_SURFACE_REPLACEABLE_NAMES
        .iter()
        .position(|candidate| *candidate == name)
}

const WINDOW_SURFACE_SLOTS: &[&str] = &[
    WINDOW_HISTORY_SLOT,
    WINDOW_NAVIGATION_SLOT,
    WINDOW_NAVIGATOR_SLOT,
    WINDOW_SCREEN_SLOT,
    WINDOW_SPEECH_SYNTHESIS_SLOT,
    WINDOW_CUSTOM_ELEMENTS_SLOT,
    WINDOW_CRYPTO_SLOT,
    WINDOW_PERFORMANCE_SLOT,
    WINDOW_VISUAL_VIEWPORT_SLOT,
    WINDOW_INDEXED_DB_SURFACE_SLOT,
];

const WINDOW_SURFACE_CALLBACK_DATA_REGISTRY_SLOT: &str = "__moliWindowSurfaceCallbackDataRegistry";

const WINDOW_SURFACE_REPLACEABLE_NAMES: &[&str] = &[
    "navigation",
    "screen",
    "performance",
    "visualViewport",
    "origin",
    "innerWidth",
    "length",
    "event",
    "outerHeight",
    "scrollX",
    "screenLeft",
    "screenTop",
    "screenX",
    "screenY",
];

fn legacy_unforgeable_document_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    let Some(host_ptr) = context_host_ptr_from_window_object(scope, receiver)
        .or_else(|| context_host_ptr_from_global_bridge(scope))
    else {
        rv.set_null();
        return;
    };
    if let Some(child_handle) = child_context_handle_from_owner(scope, receiver) {
        match unsafe { &mut *host_ptr }.child_browsing_context_document_wrapper(scope, child_handle)
        {
            Some(document) => rv.set(document.into()),
            None => rv.set_null(),
        }
        return;
    }
    let handle = unsafe { &*host_ptr }.document_handle();
    match unsafe { &mut *host_ptr }
        .native_bridge_mut()
        .wrap_handle(scope, host_ptr, handle)
    {
        Some(document) => rv.set(document.into()),
        None => rv.set_null(),
    }
}

fn child_context_handle_from_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    private_child_context_handle_from_object(scope, owner)
}

fn private_child_context_handle_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    let value = get_private_value(scope, object, CHILD_BROWSING_CONTEXT_HANDLE_SLOT)?;
    dom_handle_from_value(scope, value)
}

fn dom_handle_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<DomHandle> {
    let n = value.number_value(scope)?;
    if n.is_finite() && n >= 0.0 && n.fract() == 0.0 {
        Some(DomHandle::new(n as usize))
    } else {
        None
    }
}

fn child_window_eval_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Option<url::Url> {
    // Child frames do not eagerly get their own native V8 Context in the
    // normal page lifecycle yet. The child Window shell still carries the
    // location state we need for eval/module resolution, including after the
    // iframe has been detached and author script only holds contentWindow.
    let location = object_own_hidden_value(scope, window, WINDOW_LOCATION_SLOT)
        .filter(|value| !value.is_undefined())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
    location_href_slot(scope, location).and_then(|href| url::Url::parse(&href).ok())
}

fn run_child_window_eval_expression<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    handle: DomHandle,
    expression: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    let base_url = child_window_eval_base_url(scope, window);
    // This intentionally runs the saved child Window object directly instead
    // of resolving `handle` through the live child-frame registry. Blink keeps
    // an initialized detached WindowProxy usable by author-held references; in
    // our current single-context model, the closest owner boundary is the
    // retained child Window shell.
    //
    // Passing the expression as an eval argument is also deliberate: V8 parses
    // the author source, so dynamic import goes through the normal V8
    // HostImportModuleDynamically callback. Avoid source-string prefix checks
    // such as "import(" here.
    let source = v8_string(
        scope,
        r#"(function(window, __expr) {
    const document = window.document;
    return (function(window, document, globalThis) {
        with (window) {
            return eval(__expr);
        }
    }).call(window, window, document, window);
})"#,
    )?;
    let origin = base_url.as_ref().map(|base_url| {
        create_script_origin_with_base_url(scope, base_url.as_str(), 0, Some(base_url))
    });
    let function = v8::Script::compile(scope, source, origin.as_ref())
        .and_then(|script| script.run(scope))
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())?;
    let expression = v8_string(scope, expression)?;

    let previous_active_child = native_bridge::enter_active_child_window_scope(scope, Some(handle));
    let previous_active_child = v8::Global::new(scope, previous_active_child);
    let previous_continuation_data = scope.get_continuation_preserved_embedder_data();
    // The active child marker and continuation data are the bridge between the
    // child Window shell and module/network machinery that still runs in the
    // main native context. Restore both immediately after the eval call.
    if let Some(base_url) = base_url.as_ref()
        && let Some(value) = script_base_url_continuation_data(scope, base_url)
    {
        scope.set_continuation_preserved_embedder_data(value);
    }
    let result = function.call(scope, window.into(), &[window.into(), expression.into()]);
    scope.set_continuation_preserved_embedder_data(previous_continuation_data);
    let previous_active_child = v8::Local::new(scope, &previous_active_child);
    native_bridge::restore_active_child_window_scope(scope, previous_active_child);

    result.map(|value| mark_detached_child_eval_promise_handled(scope, handle, value))
}

fn mark_detached_child_eval_promise_handled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    handle: DomHandle,
    value: v8::Local<'s, v8::Value>,
) -> v8::Local<'s, v8::Value> {
    // Detached child eval can still return the promise created by V8 for
    // import(). If the child browsing context is no longer live, treat that
    // promise as owned by the inactive context so a later rejected dynamic
    // import does not become a top-level unhandled rejection in the parent WPT
    // harness. Keep this limited to real V8 promises and detached children.
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return value;
    };
    if unsafe { &*host_ptr }.child_browsing_context_is_live(handle) {
        return value;
    }
    let Ok(promise) = v8::Local::<v8::Promise>::try_from(value) else {
        return value;
    };
    let promise_value: v8::Local<'s, v8::Value> = promise.into();
    let Ok(promise_object) = v8::Local::<v8::Object>::try_from(promise_value) else {
        return value;
    };
    let Some(catch) = promise_object
        .get(scope, v8str(scope, "catch").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return value;
    };
    let Some(noop) = v8::Function::builder(window_noop_callback).build(scope) else {
        return value;
    };
    let _ = catch.call(scope, promise_object.into(), &[noop.into()]);
    value
}

fn evaluate_child_window_expression<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
    handle: DomHandle,
    expression: &str,
) -> Result<v8::Local<'s, v8::Value>> {
    let eval_key = v8str(scope, "eval");
    let own_eval = window
        .get(scope, eval_key.into())
        .ok_or_else(|| anyhow!("failed to capture child Window eval"))?;
    let intrinsic_eval = get_private_value(scope, window, WINDOW_INTRINSIC_EVAL_SLOT)
        .ok_or_else(|| anyhow!("missing child Window intrinsic eval"))?;
    if !window
        .define_own_property(
            scope,
            eval_key.into(),
            intrinsic_eval,
            v8::PropertyAttribute::DONT_ENUM,
        )
        .unwrap_or(false)
    {
        bail!("failed to expose child Window intrinsic eval");
    }

    let result = {
        let try_catch = std::pin::pin!(v8::TryCatch::new(scope));
        let mut scope = try_catch.init();
        run_child_window_eval_expression(&mut scope, window, handle, expression)
            .map(|value| v8::Global::new(&scope, value))
            .ok_or_else(|| {
                let message = scope
                    .message()
                    .map(|message| message.get(&scope).to_rust_string_lossy(&scope))
                    .or_else(|| {
                        scope
                            .exception()
                            .and_then(|value| value.to_string(&scope))
                            .map(|value| value.to_rust_string_lossy(&scope))
                    })
                    .unwrap_or_else(|| "failed to run child window eval source".to_owned());
                anyhow!(message)
            })
    };

    if !window
        .define_own_property(
            scope,
            eval_key.into(),
            own_eval,
            v8::PropertyAttribute::DONT_ENUM,
        )
        .unwrap_or(false)
    {
        bail!("failed to restore child Window eval");
    }

    result.map(|value| v8::Local::new(scope, &value))
}

fn child_window_eval_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if args.length() < 1 {
        rv.set_undefined();
        return;
    }

    let argument = args.get(0);
    if !argument.is_string() {
        rv.set(argument);
        return;
    }

    let receiver = callback_this_object(scope, &args);
    let Some(handle) = child_context_handle_from_owner(scope, receiver) else {
        rv.set(argument);
        return;
    };
    let Some(expression) = argument
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
    else {
        rv.set(argument);
        return;
    };

    match evaluate_child_window_expression(scope, receiver, handle, &expression) {
        Ok(value) => rv.set(value),
        Err(error) => {
            let exception = v8::Exception::error(
                scope,
                v8_string(scope, &error.to_string())
                    .unwrap_or_else(|| v8str(scope, "child window eval failed")),
            );
            scope.throw_exception(exception);
        }
    }
}

pub(crate) fn install_child_window_eval_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    window: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let inherited_or_intrinsic_eval = get_private_value(scope, window, WINDOW_INTRINSIC_EVAL_SLOT)
        .filter(|value| value.is_function())
        .or_else(|| {
            window
                .get(scope, v8str(scope, "eval").into())
                .filter(|value| value.is_function())
        })
        .or_else(|| {
            let global = scope.get_current_context().global(scope);
            get_private_value(scope, global, WINDOW_INTRINSIC_EVAL_SLOT)
        })
        .ok_or_else(|| anyhow!("failed to resolve child Window intrinsic eval"))?;
    set_private_value(
        scope,
        window,
        WINDOW_INTRINSIC_EVAL_SLOT,
        inherited_or_intrinsic_eval,
    );
    let eval = v8::Function::builder(child_window_eval_callback)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to create child window eval callback"))?;
    let _ = window.define_own_property(
        scope,
        v8str(scope, "eval").into(),
        eval.into(),
        v8::PropertyAttribute::DONT_ENUM,
    );
    Ok(())
}

fn define_legacy_unforgeable_document_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let getter = v8::Function::builder(legacy_unforgeable_document_getter)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to build document getter"))?;
    getter.set_name(v8str(scope, "get document"));
    define_get_set_property(
        scope,
        object,
        v8str(scope, "document").into(),
        getter.into(),
        v8::undefined(scope).into(),
        v8::PropertyAttribute::DONT_DELETE,
        "document",
    )
}

fn document_all_runtime_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    match document_all_value_for_receiver(scope, args.this()) {
        Some(value) => rv.set(value),
        None => rv.set(v8::undefined(scope).into()),
    }
}

fn document_cookie_runtime_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    let cookie = document_cookie_for_receiver(scope, receiver).unwrap_or_default();
    let value = v8_string(scope, &cookie).unwrap_or_else(|| v8::String::empty(scope));
    rv.set(value.into());
}

fn document_cookie_runtime_setter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let receiver = args.this();
    let Some(value) = args.get(0).to_string(scope) else {
        return;
    };
    let _ = set_document_cookie_for_receiver(scope, receiver, &value.to_rust_string_lossy(scope));
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Document.parseHTMLUnsafe")]
struct DocumentParseHtmlUnsafeArgs {
    #[webidl(required, with = document_parse_html_unsafe_source_arg)]
    html: DocumentParseHtmlUnsafeSource,
}

enum DocumentParseHtmlUnsafeSource {
    TrustedHtml(String),
    String(String),
}

fn document_parse_html_unsafe_source_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> Result<DocumentParseHtmlUnsafeSource, webidl::WebIdlError> {
    if args.length() <= index {
        return Err(webidl::WebIdlError::custom_message(
            "Failed to execute 'parseHTMLUnsafe' on 'Document': 1 argument required, but only 0 present.",
        ));
    }
    let value = args.get(index);
    if let Some(value) = crate::context_bootstrap::trusted_html_value_string(scope, value) {
        return Ok(DocumentParseHtmlUnsafeSource::TrustedHtml(value));
    }
    webidl::convert::<webidl::DomString>(
        scope,
        value,
        webidl::Context::argument("Document.parseHTMLUnsafe", (index + 1) as usize),
    )
    .map(|value| DocumentParseHtmlUnsafeSource::String(value.0))
}

fn document_parse_html_unsafe_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<DocumentParseHtmlUnsafeArgs>(scope, &args) else {
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let runtime = unsafe { &*host_ptr };
    let html = match parsed.html {
        DocumentParseHtmlUnsafeSource::TrustedHtml(html) => html,
        DocumentParseHtmlUnsafeSource::String(html) => {
            let Some(value) = v8_string(scope, &html) else {
                return;
            };
            let requirements = runtime.trusted_types_for_script_requirements(scope);
            let Some(html) = crate::context_bootstrap::trusted_html_string_or_throw(
                scope,
                value.into(),
                requirements,
                "Document parseHTMLUnsafe",
                "parseHTMLUnsafe",
            ) else {
                return;
            };
            html
        }
    };
    let Some(document) =
        crate::dom_parser::parse_detached_html_document_from_source_with_declarative_shadow_roots(
            scope,
            url::Url::parse("about:blank").expect("static about:blank URL parses"),
            &html,
        )
    else {
        rv.set(v8::null(scope).into());
        return;
    };
    rv.set(document.into());
}

fn install_document_static_methods<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(document_constructor) = global_constructor_object(scope, "Document") else {
        return Ok(());
    };
    DocumentStaticMethodsDeclaration::default()
        .initialize(scope, document_constructor)
        .map_err(|err| anyhow!("failed to install Document static methods: {err}"))?;
    Ok(())
}

fn install_document_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    install_document_static_methods(scope, global)?;
    if let Some(prototype) = global_constructor_prototype(scope, "Document") {
        DocumentPrototypeRuntimeDeclaration::default().initialize(scope, prototype)?;
    }
    if let Some(document) = global
        .get(scope, v8str(scope, "document").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        sync_document_location_runtime_state_from_window(scope, document, global);
    }
    Ok(())
}

#[cfg(feature = "wpt-extensions")]
fn webdriver_constructor_callback(
    scope: &mut v8::PinScope<'_, '_>,
    args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if args.is_construct_call() {
        rv.set(args.this().into());
        return;
    }
    webidl::throw_type_error(scope, "WebDriver constructor requires 'new'");
}

#[cfg(feature = "wpt-extensions")]
fn webdriver_delete_all_cookies_callback(
    scope: &mut v8::PinScope<'_, '_>,
    _args: v8::FunctionCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
        unsafe { &mut *host_ptr }.delete_all_cookies_for_wpt();
    }
    rv.set(v8::undefined(scope).into());
}

#[cfg(feature = "wpt-extensions")]
pub(crate) fn install_wpt_webdriver_runtime_state(
    scope: &mut v8::PinScope<'_, '_>,
    global: v8::Local<'_, v8::Object>,
) -> Result<()> {
    if global
        .get(scope, v8str(scope, "webdriver").into())
        .is_some_and(|value| !value.is_undefined())
    {
        return Ok(());
    }

    let constructor_key = v8str(scope, "WebDriver");
    let constructor = v8::Function::builder(webdriver_constructor_callback)
        .length(0)
        .build(scope)
        .ok_or_else(|| anyhow!("failed to create WebDriver constructor"))?;
    constructor.set_name(constructor_key);

    let prototype = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("failed to get WebDriver.prototype"))?;
    WebDriverPrototypeDeclaration::new()
        .initialize(scope, prototype)
        .map_err(|error| anyhow!("failed to initialize WebDriver.prototype: {error}"))?;
    WebDriverPrototypeMetadataDeclaration::new(constructor)
        .initialize(scope, prototype)
        .map_err(|error| anyhow!("failed to initialize WebDriver prototype metadata: {error}"))?;
    define_global_value(scope, global, "WebDriver", constructor.into())?;
    let webdriver = WebDriverObjectDeclaration::new()
        .bind(scope)
        .map_err(|error| anyhow!("failed to create WebDriver object: {error}"))?;
    define_global_value(scope, global, "webdriver", webdriver.into())
}

pub(crate) fn finish_context_bootstrap(
    scope: &mut v8::PinScope<'_, '_>,
    document_runtime: &mut JsContextHost,
    secure_context_url: &url::Url,
) -> Result<()> {
    super::exposed_interfaces::initialize_realm_interface_registry(
        scope,
        super::exposed_interfaces::RealmKind::Window,
    )?;
    install_window_named_properties_object(scope)?;
    let global = scope.get_current_context().global(scope);
    // V8 creates its inspector-aware console as an own global property after
    // applying the embedder ObjectTemplate. Preserve it for forwarding, then
    // replace it with Moli's observable console object. This property
    // cannot use the Window ObjectTemplate accessor path because V8's own
    // console shadows that descriptor during context creation.
    let console_key = v8str(scope, "console");
    if let Some(original_console) = global.get(scope, console_key.into()) {
        set_private_value(
            scope,
            global,
            WINDOW_ORIGINAL_CONSOLE_SLOT,
            original_console,
        );
    }
    let _ = global.delete(scope, console_key.into());
    install_window_runtime_state(scope, global, document_runtime, secure_context_url)?;
    // WPT harness helper only. Normal builds keep the feature disabled so pages
    // do not observe non-standard `webdriver` / `WebDriver` globals.
    #[cfg(feature = "wpt-extensions")]
    if document_runtime.wpt_extensions_enabled() {
        install_wpt_webdriver_runtime_state(scope, global)?;
    }

    install_node_filter_constants(scope, global);
    for (ctor_name, tag) in [
        ("NodeList", "NodeList"),
        ("HTMLCollection", "HTMLCollection"),
        ("PluginArray", "PluginArray"),
        ("MimeTypeArray", "MimeTypeArray"),
        ("Plugin", "Plugin"),
        ("MimeType", "MimeType"),
        ("Attr", "Attr"),
        ("NamedNodeMap", "NamedNodeMap"),
        ("HTMLAllCollection", "HTMLAllCollection"),
        ("DOMException", "DOMException"),
        ("DOMError", "DOMError"),
        ("DocumentType", "DocumentType"),
        ("DOMImplementation", "DOMImplementation"),
        ("DOMTokenList", "DOMTokenList"),
        ("DOMStringMap", "DOMStringMap"),
        ("CustomElementRegistry", "CustomElementRegistry"),
        ("HTMLFormControlsCollection", "HTMLFormControlsCollection"),
        ("HTMLOptionsCollection", "HTMLOptionsCollection"),
        ("RadioNodeList", "RadioNodeList"),
        ("ValidityState", "ValidityState"),
        ("EventTarget", "EventTarget"),
        ("HTMLLabelElement", "HTMLLabelElement"),
        ("NodeIterator", "NodeIterator"),
        ("TreeWalker", "TreeWalker"),
        ("XPathEvaluator", "XPathEvaluator"),
        ("XPathResult", "XPathResult"),
        ("SVGLength", "SVGLength"),
        ("SVGNumber", "SVGNumber"),
        ("SVGAnimatedLength", "SVGAnimatedLength"),
        ("SVGLengthList", "SVGLengthList"),
        ("SVGAnimatedLengthList", "SVGAnimatedLengthList"),
        ("SVGAnimatedNumber", "SVGAnimatedNumber"),
        ("SVGNumberList", "SVGNumberList"),
        ("SVGAnimatedNumberList", "SVGAnimatedNumberList"),
        ("SVGAnimatedEnumeration", "SVGAnimatedEnumeration"),
        ("SVGAnimatedTransformList", "SVGAnimatedTransformList"),
        ("SVGTransformList", "SVGTransformList"),
        ("SVGTransform", "SVGTransform"),
        ("SVGMatrix", "SVGMatrix"),
        ("StyleSheet", "StyleSheet"),
        ("StyleSheetList", "StyleSheetList"),
        ("MediaList", "MediaList"),
        ("CSSRuleList", "CSSRuleList"),
        ("CSSRule", "CSSRule"),
        ("CSSGroupingRule", "CSSGroupingRule"),
        ("CSSConditionRule", "CSSConditionRule"),
        ("CSSMediaRule", "CSSMediaRule"),
        ("CSSSupportsRule", "CSSSupportsRule"),
        ("CSSContainerRule", "CSSContainerRule"),
        ("CSSLayerBlockRule", "CSSLayerBlockRule"),
        ("CSSLayerStatementRule", "CSSLayerStatementRule"),
        ("CSSScopeRule", "CSSScopeRule"),
        ("CSSImportRule", "CSSImportRule"),
        ("CSSFontFaceRule", "CSSFontFaceRule"),
        ("CSSFontFeatureValuesRule", "CSSFontFeatureValuesRule"),
        ("CSSKeyframesRule", "CSSKeyframesRule"),
        ("CSSKeyframeRule", "CSSKeyframeRule"),
        ("CSSPageRule", "CSSPageRule"),
        ("CSSMarginRule", "CSSMarginRule"),
        ("CSSNamespaceRule", "CSSNamespaceRule"),
        ("CSSCounterStyleRule", "CSSCounterStyleRule"),
        ("CSSStyleRule", "CSSStyleRule"),
        ("CSSNestedDeclarations", "CSSNestedDeclarations"),
        ("CSSStyleDeclaration", "CSSStyleDeclaration"),
        ("CSSStyleProperties", "CSSStyleProperties"),
        ("CSSFontFaceDescriptors", "CSSFontFaceDescriptors"),
        ("CSSPageDescriptors", "CSSPageDescriptors"),
        ("CSSStyleSheet", "CSSStyleSheet"),
        ("FontFace", "FontFace"),
        ("FontFaceSet", "FontFaceSet"),
        ("FontFaceSetLoadEvent", "FontFaceSetLoadEvent"),
        ("BaseAudioContext", "BaseAudioContext"),
        ("OfflineAudioContext", "OfflineAudioContext"),
        ("AudioDestinationNode", "AudioDestinationNode"),
        ("OscillatorNode", "OscillatorNode"),
        ("DynamicsCompressorNode", "DynamicsCompressorNode"),
        ("AnalyserNode", "AnalyserNode"),
        ("AudioParam", "AudioParam"),
        ("AudioBuffer", "AudioBuffer"),
        ("AbortSignal", "AbortSignal"),
        ("AbortController", "AbortController"),
        ("TextEncoder", "TextEncoder"),
        ("TextDecoder", "TextDecoder"),
        ("ReadableStream", "ReadableStream"),
        ("ReadableStreamDefaultReader", "ReadableStreamDefaultReader"),
        (
            "ReadableStreamDefaultController",
            "ReadableStreamDefaultController",
        ),
        ("WritableStream", "WritableStream"),
        ("WritableStreamDefaultWriter", "WritableStreamDefaultWriter"),
        (
            "WritableStreamDefaultController",
            "WritableStreamDefaultController",
        ),
        ("TransformStream", "TransformStream"),
        (
            "TransformStreamDefaultController",
            "TransformStreamDefaultController",
        ),
        ("TextEncoderStream", "TextEncoderStream"),
        ("TextDecoderStream", "TextDecoderStream"),
        ("Performance", "Performance"),
        ("PerformanceEntry", "PerformanceEntry"),
        ("PerformanceMark", "PerformanceMark"),
        ("PerformanceMeasure", "PerformanceMeasure"),
        ("PerformanceResourceTiming", "PerformanceResourceTiming"),
        ("EventCounts", "EventCounts"),
        ("MediaQueryList", "MediaQueryList"),
        ("MediaError", "MediaError"),
        ("TextTrack", "TextTrack"),
        ("TextTrackList", "TextTrackList"),
        ("TextTrackCue", "TextTrackCue"),
        ("TextTrackCueList", "TextTrackCueList"),
        ("TrackEvent", "TrackEvent"),
        ("VTTCue", "VTTCue"),
        ("Crypto", "Crypto"),
        ("SubtleCrypto", "SubtleCrypto"),
        ("CryptoKey", "CryptoKey"),
        ("VisualViewport", "VisualViewport"),
        ("Navigator", "Navigator"),
        ("Permissions", "Permissions"),
        ("PermissionStatus", "PermissionStatus"),
        ("MediaDevices", "MediaDevices"),
        ("MediaCapabilities", "MediaCapabilities"),
        ("Screen", "Screen"),
        ("ScreenOrientation", "ScreenOrientation"),
        ("Touch", "Touch"),
        ("TouchList", "TouchList"),
        ("TouchEvent", "TouchEvent"),
        ("BroadcastChannel", "BroadcastChannel"),
        ("EventSource", "EventSource"),
        ("MessageChannel", "MessageChannel"),
        ("MessagePort", "MessagePort"),
        ("Event", "Event"),
        ("CustomEvent", "CustomEvent"),
        ("DragEvent", "DragEvent"),
        ("InputEvent", "InputEvent"),
        ("MessageEvent", "MessageEvent"),
        ("StorageEvent", "StorageEvent"),
        ("PageTransitionEvent", "PageTransitionEvent"),
        ("PopStateEvent", "PopStateEvent"),
        (
            "NavigationCurrentEntryChangeEvent",
            "NavigationCurrentEntryChangeEvent",
        ),
        ("NavigateEvent", "NavigateEvent"),
        ("ErrorEvent", "ErrorEvent"),
        ("CloseEvent", "CloseEvent"),
        ("SubmitEvent", "SubmitEvent"),
        ("FormDataEvent", "FormDataEvent"),
        ("Worker", "Worker"),
        ("SharedWorker", "SharedWorker"),
        ("WebSocket", "WebSocket"),
        ("WebSocketError", "WebSocketError"),
        ("WebSocketStream", "WebSocketStream"),
        ("RTCPeerConnection", "RTCPeerConnection"),
        ("RTCRtpReceiver", "RTCRtpReceiver"),
        ("RTCDataChannel", "RTCDataChannel"),
        ("Blob", "Blob"),
        ("File", "File"),
        ("DataTransfer", "DataTransfer"),
        ("ImageData", "ImageData"),
        ("ImageBitmap", "ImageBitmap"),
        ("CanvasGradient", "CanvasGradient"),
        ("CanvasPattern", "CanvasPattern"),
        ("TextMetrics", "TextMetrics"),
        ("Path2D", "Path2D"),
        ("OffscreenCanvas", "OffscreenCanvas"),
        ("CanvasRenderingContext2D", "CanvasRenderingContext2D"),
        (
            "OffscreenCanvasRenderingContext2D",
            "OffscreenCanvasRenderingContext2D",
        ),
        ("WebGLRenderingContext", "WebGLRenderingContext"),
        ("WebGL2RenderingContext", "WebGL2RenderingContext"),
        ("WebGLObject", "WebGLObject"),
        ("WebGLBuffer", "WebGLBuffer"),
        ("WebGLFramebuffer", "WebGLFramebuffer"),
        ("WebGLProgram", "WebGLProgram"),
        ("WebGLRenderbuffer", "WebGLRenderbuffer"),
        ("WebGLShader", "WebGLShader"),
        ("WebGLUniformLocation", "WebGLUniformLocation"),
        ("WEBGL_debug_renderer_info", "WEBGL_debug_renderer_info"),
        ("WEBGL_lose_context", "WEBGL_lose_context"),
        ("FileList", "FileList"),
        ("FileReader", "FileReader"),
        ("DOMRect", "DOMRect"),
        ("DOMPoint", "DOMPoint"),
        ("CaretPosition", "CaretPosition"),
        ("DOMMatrixReadOnly", "DOMMatrixReadOnly"),
        ("DOMMatrix", "DOMMatrix"),
        ("DOMParser", "DOMParser"),
        ("XMLSerializer", "XMLSerializer"),
        ("MutationObserver", "MutationObserver"),
        ("MutationRecord", "MutationRecord"),
        ("IntersectionObserver", "IntersectionObserver"),
        ("IntersectionObserverEntry", "IntersectionObserverEntry"),
        ("ResizeObserver", "ResizeObserver"),
        ("PerformanceObserver", "PerformanceObserver"),
        (
            "PerformanceObserverEntryList",
            "PerformanceObserverEntryList",
        ),
        ("PerformanceTiming", "PerformanceTiming"),
        ("PerformanceNavigation", "PerformanceNavigation"),
        ("NavigatorUAData", "NavigatorUAData"),
        ("StorageManager", "StorageManager"),
        ("StorageEstimate", "StorageEstimate"),
        ("StorageAccessHandle", "StorageAccessHandle"),
        ("StorageBucketManager", "StorageBucketManager"),
        ("StorageBucket", "StorageBucket"),
        ("FileSystemHandle", "FileSystemHandle"),
        ("FileSystemFileHandle", "FileSystemFileHandle"),
        ("FileSystemDirectoryHandle", "FileSystemDirectoryHandle"),
        (
            "FileSystemWritableFileStream",
            "FileSystemWritableFileStream",
        ),
        ("IdleDeadline", "IdleDeadline"),
        ("AbstractRange", "AbstractRange"),
        ("Range", "Range"),
        ("Selection", "Selection"),
        ("History", "History"),
        ("Location", "Location"),
        ("Navigation", "Navigation"),
        ("NavigationHistoryEntry", "NavigationHistoryEntry"),
        ("NavigationActivation", "NavigationActivation"),
        ("NavigationTransition", "NavigationTransition"),
        ("URL", "URL"),
        ("URLSearchParams", "URLSearchParams"),
        ("FormData", "FormData"),
        ("Headers", "Headers"),
        ("Request", "Request"),
        ("Response", "Response"),
        ("XMLHttpRequest", "XMLHttpRequest"),
        ("IDBFactory", "IDBFactory"),
        ("IDBRequest", "IDBRequest"),
        ("IDBOpenDBRequest", "IDBOpenDBRequest"),
        ("IDBDatabase", "IDBDatabase"),
        ("IDBTransaction", "IDBTransaction"),
        ("IDBObjectStore", "IDBObjectStore"),
        ("IDBIndex", "IDBIndex"),
        ("IDBCursor", "IDBCursor"),
        ("IDBCursorWithValue", "IDBCursorWithValue"),
        ("IDBKeyRange", "IDBKeyRange"),
        ("IDBVersionChangeEvent", "IDBVersionChangeEvent"),
        ("Window", "Window"),
    ] {
        if is_lazy_exposed_interface(scope, ctor_name) {
            continue;
        }
        install_to_string_tag(scope, global, ctor_name, tag);
    }
    install_window_global_accessors(scope, global);

    native_bridge::install_detached_bridge_methods(scope);
    reset_range_runtime_state(scope);
    initialize_file_api_runtime_queues(scope, global)?;
    super::exposed_interfaces::capture_eager_intrinsic_interfaces(
        scope,
        global,
        super::exposed_interfaces::RealmKind::Window,
    )?;
    Ok(())
}

pub(crate) fn set_interface_prototype_constructor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    constructor: v8::Local<'s, v8::Function>,
    public_constructor: v8::Local<'s, v8::Value>,
) {
    let Some(prototype) = constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let _ = prototype.define_own_property(
        scope,
        v8str(scope, "constructor").into(),
        public_constructor,
        v8::PropertyAttribute::DONT_ENUM,
    );
}

fn install_window_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    runtime: &DocumentRuntime,
    secure_context_url: &url::Url,
) -> Result<()> {
    let secure_context_available = moli_url::is_potentially_trustworthy_url(secure_context_url);
    set_private_value(
        scope,
        global,
        WINDOW_SECURE_CONTEXT_AVAILABLE_SLOT,
        v8::Boolean::new(scope, secure_context_available).into(),
    );
    install_date_locale_runtime_state(scope, global)?;
    ensure_location_constructor_runtime_state(scope, global)?;
    install_webassembly_runtime_state(scope, global)?;
    install_webidl_collection_iterator_intrinsics(scope, global)?;

    install_window_location_history_navigation_runtime_state(
        scope,
        global,
        runtime.document_url().as_str(),
    )?;
    let origin = moli_url::origin_ascii_serialization(runtime.document_url());
    set_window_origin_runtime_state(scope, global, &origin)?;

    let console = ConsoleObjectDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!("failed to create console object: {error}"))?;
    let webkit_storage_info = build_legacy_storage_info_object(scope)?;
    WindowBootstrapGlobalSlotsDeclaration::new(
        console,
        global,
        global,
        global,
        global,
        secure_context_available,
        0,
        1,
        webkit_storage_info,
        "",
        console,
    )
    .initialize(scope, global)
    .map_err(|error| anyhow!("failed to initialize Window bootstrap slots: {error}"))?;
    define_legacy_unforgeable_window_property(scope, global)?;
    define_legacy_unforgeable_document_property(scope, global)?;
    // Blink exposes `self`, `parent`, and `frames` as [Replaceable], while
    // `top` stays [LegacyUnforgeable]. We mirror that split so top-level
    // classic-script declarations like `var parent = ...` work, without letting
    // DOM named items shadow the builtins.
    WindowLegacyAliasAccessorsDeclaration::default().initialize(scope, global)?;
    let intrinsic_eval = v8::Script::compile(scope, v8str(scope, "eval"), None)
        .and_then(|script| script.run(scope))
        .ok_or_else(|| anyhow!("failed to resolve intrinsic eval"))?;
    set_private_value(scope, global, WINDOW_INTRINSIC_EVAL_SLOT, intrinsic_eval);
    WindowEvalGlobalDeclaration::new(intrinsic_eval)
        .initialize(scope, global)
        .map_err(|error| anyhow!("failed to initialize Window intrinsic eval: {error}"))?;
    // Window is a [Global] WebIDL interface. Its members are own properties of
    // the global object, not duplicate properties on Window.prototype.
    WindowComputedStyleMethodDeclaration::default()
        .initialize(scope, global)
        .map_err(|error| anyhow!(error.to_string()))?;
    install_document_runtime_state(scope, global)?;
    install_navigator_runtime_state(scope, global, secure_context_available)?;
    let document_loader = runtime
        .current_document_resource_loader()
        .expect("Window bootstrap requires the committed Document resource authority");
    bind_window_navigator_identity_seed(
        scope,
        global,
        document_loader.request_client().browser_identity(),
    )?;
    super::exposed_interfaces::filter_window_exposed_interfaces(
        scope,
        global,
        secure_context_available,
    )?;
    install_html_script_element_static_methods(scope, global)?;
    install_trusted_types_runtime_state(scope, global)?;
    install_window_crypto_runtime_state(scope, global, secure_context_available)?;
    install_css_runtime_state(scope, global)?;
    install_default_window_performance_seed(scope, global)?;
    install_storage_runtime_state(scope, global)?;
    network_host::initialize_fetch_realm_helpers(scope)?;
    install_public_window_surface_accessors(scope, global)?;

    Ok(())
}

pub(crate) fn set_window_origin_runtime_state(
    scope: &mut v8::PinScope<'_, '_>,
    window: v8::Local<'_, v8::Object>,
    origin: &str,
) -> Result<()> {
    let origin_value =
        v8_string(scope, origin).ok_or_else(|| anyhow!("failed to allocate Window.origin"))?;
    set_private_value(
        scope,
        window,
        WINDOW_ORIGIN_RUNTIME_SLOT,
        origin_value.into(),
    );
    Ok(())
}

pub(crate) fn install_webassembly_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let Some(webassembly) = global
        .get(scope, v8str(scope, "WebAssembly").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Ok(());
    };
    for (name, slot) in [
        ("Instance", ORIGINAL_WEBASSEMBLY_INSTANCE_CONSTRUCTOR_SLOT),
        (
            "CompileError",
            ORIGINAL_WEBASSEMBLY_COMPILE_ERROR_CONSTRUCTOR_SLOT,
        ),
        (
            "LinkError",
            ORIGINAL_WEBASSEMBLY_LINK_ERROR_CONSTRUCTOR_SLOT,
        ),
    ] {
        let Some(constructor) = webassembly
            .get(scope, v8str(scope, name).into())
            .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
        else {
            continue;
        };
        set_private_value(scope, global, slot, constructor.into());
    }
    store_webassembly_global_value_getter(scope, global, webassembly);
    install_webassembly_runtime_extensions(scope)?;
    install_webassembly_namespace_instance(scope, webassembly);
    super::webassembly_runtime::capture_webassembly_default_prototypes(scope, global, webassembly);
    Ok(())
}

fn store_webassembly_global_value_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    webassembly: v8::Local<'s, v8::Object>,
) {
    let Some(global_constructor) = webassembly
        .get(scope, v8str(scope, "Global").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    let Some(global_prototype) = global_constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let Some(descriptor) =
        global_prototype.get_own_property_descriptor(scope, v8str(scope, "value").into())
    else {
        return;
    };
    let Ok(descriptor) = v8::Local::<v8::Object>::try_from(descriptor) else {
        return;
    };
    let Some(getter) = descriptor
        .get(scope, v8str(scope, "get").into())
        .and_then(|value| v8::Local::<v8::Function>::try_from(value).ok())
    else {
        return;
    };
    set_private_value(
        scope,
        global,
        ORIGINAL_WEBASSEMBLY_GLOBAL_VALUE_GETTER_SLOT,
        getter.into(),
    );
}

fn install_webassembly_namespace_instance<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    webassembly: v8::Local<'s, v8::Object>,
) {
    let key = v8str(scope, "namespaceInstance");
    if webassembly
        .get(scope, key.into())
        .is_some_and(|value| value.is_function())
    {
        return;
    }
    let _ = WebAssemblyNamespaceDeclaration::default().initialize(scope, webassembly);
}

fn webassembly_namespace_instance_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let Ok(namespace) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        throw_type_error(scope, "WebAssembly namespace object expected.");
        return;
    };

    if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && let Some(instance) =
            (unsafe { &*host_ptr }).native_wasm_instance_for_namespace(scope, namespace)
    {
        rv.set(instance.into());
        return;
    }

    if let Some(instance) = crate::worker::worker_wasm_instance_for_namespace(scope, namespace) {
        rv.set(instance.into());
        return;
    }

    throw_type_error(scope, "WebAssembly namespace object expected.");
}

fn install_html_script_element_static_methods(
    scope: &mut v8::PinScope<'_, '_>,
    _global: v8::Local<'_, v8::Object>,
) -> Result<()> {
    let Some(ctor) = global_constructor_object(scope, "HTMLScriptElement") else {
        return Ok(());
    };
    HtmlScriptElementStaticMethodsDeclaration::default()
        .initialize(scope, ctor)
        .map_err(|err| anyhow!("failed to install HTMLScriptElement static methods: {err}"))?;
    Ok(())
}

fn html_script_element_supports_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<HtmlScriptElementSupportsArgs>(scope, &args) else {
        return;
    };
    rv.set(
        v8::Boolean::new(
            scope,
            html_script_element_supports_type(&parsed.script_type),
        )
        .into(),
    );
}
