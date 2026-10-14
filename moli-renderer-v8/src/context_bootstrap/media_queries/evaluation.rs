use super::event::create_media_query_list_event;
use super::{
    dispatch_media_query_list_event, initialize_simple_event_target,
    simple_object_event_set_handler,
};
use crate::context_bootstrap::DEFAULT_WINDOW_SURFACE_PROFILE;
use crate::context_bootstrap::current_window_style_viewport;
use crate::context_bootstrap::window_accessors::{
    prepare_window_viewport_read, window_current_dispatch_scope,
};
use crate::context_bootstrap::{
    MEDIA_QUERY_LIST_MATCHES_SLOT, MEDIA_QUERY_LIST_MEDIA_SLOT, MEDIA_QUERY_LIST_ONCHANGE_SLOT,
    MEDIA_QUERY_LIST_REGISTRY_SLOT,
};
use crate::context_bootstrap::{global_queue_array, push_object_to_global_registry};
use crate::native_bridge::{
    JsContextHost, WindowDocumentTaskTarget, WindowExecutionContextBinding,
};
use crate::style_engine::{
    StyleViewport,
    media_list::{evaluate_media_query_list, normalize_media_query_list},
};
use crate::util::{
    callback_data_index_value, callback_data_item, context_host_ptr_from_global_bridge,
    get_private_value, set_private_value,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

#[derive(WebApiObject)]
#[webapi(interface = web_api_interfaces::MediaQueryList)]
struct MediaQueryListObjectDeclaration {
    #[webapi(slot = MEDIA_QUERY_LIST_MEDIA_SLOT)]
    media: String,
    #[webapi(slot = MEDIA_QUERY_LIST_MATCHES_SLOT)]
    matches: bool,
    #[webapi(slot = MEDIA_QUERY_LIST_ONCHANGE_SLOT, init = "null")]
    onchange: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaQueryList, receiver)]
struct MediaQueryListPrototypeAccessorsDeclaration {
    #[webapi(
        accessor_property,
        getter = media_query_list_attribute_getter_callback,
        data = callback_data_index_value(scope, 0),
        enumerable
    )]
    media: (),
    #[webapi(
        accessor_property,
        getter = media_query_list_attribute_getter_callback,
        data = callback_data_index_value(scope, 1),
        enumerable
    )]
    matches: (),
    #[webapi(
        accessor_property,
        getter = media_query_list_onchange_getter_callback,
        setter = media_query_list_onchange_setter_callback,
        enumerable
    )]
    onchange: (),
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Window.matchMedia")]
struct MatchMediaArgs {
    #[webidl(required)]
    query: String,
}

pub(in crate::context_bootstrap) fn install_media_query_list_template_bindings<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    let prototype = template.prototype_template(scope);
    MediaQueryListPrototypeAccessorsDeclaration::initialize_prototype_template(scope, prototype);
}

fn media_query_list_attribute_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(slot) = callback_data_item(
        scope,
        &args,
        MEDIA_QUERY_LIST_ATTRIBUTE_SLOTS,
        "MediaQueryList attribute slots",
    ) else {
        rv.set_undefined();
        return;
    };
    if slot == MEDIA_QUERY_LIST_MATCHES_SLOT {
        if let Some(matches) = read_media_query_matches(scope, args.this()) {
            rv.set_bool(matches);
        }
        return;
    }
    rv.set(
        media_query_list_slot_value(scope, args.this(), slot)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

const REALTIME_MEDIA_QUERY_ID_SLOT: &str = "__moliRealtimeMediaQueryId";

fn realtime_media_query_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    get_private_value(scope, list, REALTIME_MEDIA_QUERY_ID_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .and_then(|value| {
            let (id, lossless) = value.u64_value();
            lossless.then_some(id)
        })
}

pub(super) fn read_media_query_matches<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<bool> {
    let context = list
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    let media = media_query_list_media_slot(scope, list).unwrap_or_default();
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return Some(evaluate_match_media_query_list(&media, None));
    };
    let host = unsafe { &*host_ptr };
    if window_current_dispatch_scope(scope, global, host).is_none() {
        return Some(false);
    }
    if !prepare_window_viewport_read(scope, global) {
        return None;
    }
    if host.layout_policy().is_realtime()
        && let Some(id) = realtime_media_query_id(scope, list)
    {
        return Some(host.realtime_media_query_matches(id));
    }
    Some(evaluate_match_media_query_list_with_viewport(
        &media,
        Some(host.emulated_media()),
        current_window_style_viewport(scope, host),
    ))
}

pub(in crate::context_bootstrap) fn realtime_media_query_listeners_changed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    slot: &str,
    event_type: &str,
) {
    if slot != MEDIA_QUERY_LIST_LISTENERS_SLOT || event_type != "change" {
        return;
    }
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    if !unsafe { &*host_ptr }.layout_policy().is_realtime() {
        return;
    }
    let Some(id) = realtime_media_query_id(scope, list) else {
        return;
    };
    let active =
        !super::simple_object_event_listeners_snapshot(scope, list, slot, event_type).is_empty();
    unsafe { &*host_ptr }.set_realtime_media_query_listening(id, active);
}

/// These strong roots exist only for a Document's current media phase. The
/// persistent native query holds the list weakly; it never roots a Context.
pub(crate) struct PreparedRealtimeMediaQueryEvent {
    pub(crate) id: u64,
    pub(crate) binding: WindowExecutionContextBinding,
    pub(crate) list: v8::Global<v8::Object>,
    pub(crate) event: v8::Global<v8::Object>,
}

pub(crate) fn take_realtime_media_query_events(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut JsContextHost,
    target: WindowDocumentTaskTarget,
) -> Vec<PreparedRealtimeMediaQueryEvent> {
    // Freeze the whole Document batch once, not separately per Realm. Queries
    // queued or registered during a callback belong to the next media phase.
    let candidates = unsafe { &*host_ptr }.take_realtime_media_query_notifications(target);
    let mut events = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !unsafe { &*host_ptr }.realtime_media_query_is_current(target, candidate.id) {
            continue;
        }
        // No Host/state borrow survives a weak upgrade or any V8 operation.
        let Some(list) = candidate.list.to_local(scope) else {
            unsafe { &*host_ptr }.retire_realtime_media_query(candidate.id);
            continue;
        };
        let Some(context) = list.get_creation_context(scope) else {
            unsafe { &*host_ptr }.retire_realtime_media_query(candidate.id);
            continue;
        };
        let identity = candidate.execution_context;
        let binding = WindowExecutionContextBinding::new(
            identity.owner(),
            identity.dispatch_scope(),
            identity.realm_token(),
            v8::Global::new(scope, context),
        );
        let list = v8::Global::new(scope, list);
        let event = binding.with_current_scope(scope, host_ptr, |scope, _| {
            let list = v8::Local::new(scope, &list);
            let event =
                super::event::create_realtime_media_query_list_event(scope, list, &candidate.media);
            v8::Global::new(scope, event)
        });
        let Some(event) = event else {
            unsafe { &*host_ptr }.retire_realtime_media_query(candidate.id);
            continue;
        };
        // Prepare every event before the first author callback. The existing
        // native event.matches getter deliberately continues to read its list.
        events.push(PreparedRealtimeMediaQueryEvent {
            id: candidate.id,
            binding,
            list,
            event,
        });
    }
    events
}

pub(crate) fn dispatch_realtime_media_query_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    event: v8::Local<'s, v8::Object>,
) {
    let _ = dispatch_media_query_list_event(scope, list, event);
}

const MEDIA_QUERY_LIST_ATTRIBUTE_SLOTS: &[&str] =
    &[MEDIA_QUERY_LIST_MEDIA_SLOT, MEDIA_QUERY_LIST_MATCHES_SLOT];

fn media_query_list_onchange_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let value = media_query_list_slot_value(scope, args.this(), MEDIA_QUERY_LIST_ONCHANGE_SLOT)
        .unwrap_or_else(|| v8::null(scope).into());
    if value.is_null_or_undefined() {
        rv.set_null();
    } else {
        rv.set(value);
    }
}

fn media_query_list_onchange_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = args.this();
    simple_object_event_set_handler(
        scope,
        target,
        "change",
        MEDIA_QUERY_LIST_ONCHANGE_SLOT,
        args.get(0),
    );
    realtime_media_query_listeners_changed(
        scope,
        target,
        MEDIA_QUERY_LIST_LISTENERS_SLOT,
        "change",
    );
    rv.set_undefined();
}

pub(crate) fn window_match_media_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<MatchMediaArgs>(scope, &args) else {
        return;
    };
    let serialized_query = serialize_match_media_query_list(&parsed.query);
    let global = scope.get_current_context().global(scope);
    if !prepare_window_viewport_read(scope, global) {
        return;
    }
    let (emulated_media, viewport) =
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            let host = unsafe { &*host_ptr };
            (
                Some(host.emulated_media().clone()),
                current_window_style_viewport(scope, host),
            )
        } else {
            (None, match_media_style_viewport(None))
        };
    let matches = evaluate_match_media_query_list_with_viewport(
        &serialized_query,
        emulated_media.as_ref(),
        viewport,
    );
    let realtime_registration = if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope)
        && unsafe { &*host_ptr }.layout_policy().is_realtime()
        && let Some(target) = unsafe { &*host_ptr }.current_window_document_task_target(scope)
    {
        Some((host_ptr, target, serialized_query.clone()))
    } else {
        None
    };
    let mql = MediaQueryListObjectDeclaration::new(serialized_query, matches)
        .bind(scope)
        .expect("MediaQueryList declaration should bind");
    if let Some((host_ptr, target, media)) = realtime_registration {
        let id = match unsafe { &*host_ptr }
            .register_realtime_media_query(scope, target, media, matches, mql)
        {
            Ok(id) => id,
            Err(error) => {
                crate::context_bootstrap::throw_error(scope, &error.to_string());
                return;
            }
        };
        let value = v8::BigInt::new_from_u64(scope, id);
        set_private_value(scope, mql, REALTIME_MEDIA_QUERY_ID_SLOT, value.into());
    }
    initialize_simple_event_target(scope, mql);

    if global_queue_array(scope, MEDIA_QUERY_LIST_REGISTRY_SLOT).is_none() {
        let global = scope.get_current_context().global(scope);
        let registry = v8::Array::new(scope, 0);
        set_private_value(
            scope,
            global,
            MEDIA_QUERY_LIST_REGISTRY_SLOT,
            registry.into(),
        );
    }
    push_object_to_global_registry(scope, MEDIA_QUERY_LIST_REGISTRY_SLOT, mql);
    rv.set(mql.into());
}

pub(crate) fn dispatch_media_query_list_change_events<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> bool {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return false;
    };
    let host = unsafe { &*host_ptr };
    let target = host.current_window_document_task_target(scope);
    let current_media = host.emulated_media().clone();
    let current_viewport = current_window_style_viewport(scope, host);
    let Some(registry) = global_queue_array(scope, MEDIA_QUERY_LIST_REGISTRY_SLOT) else {
        return false;
    };
    let mut dispatched = false;
    for index in 0..registry.length() {
        let Some(value) = registry.get_index(scope, index) else {
            continue;
        };
        let Ok(mql) = v8::Local::<v8::Object>::try_from(value) else {
            continue;
        };
        let Some(media) = media_query_list_media_slot(scope, mql) else {
            continue;
        };
        if target.is_none_or(|target| {
            !unsafe { &*host_ptr }.window_document_owner_is_current_for_dispatch_scope(
                target.owner(),
                target.dispatch_scope(),
            )
        }) {
            break;
        }
        let previous_matches = get_private_value(scope, mql, MEDIA_QUERY_LIST_MATCHES_SLOT)
            .is_some_and(|value| value.boolean_value(scope));
        let current_matches = evaluate_match_media_query_list_with_viewport(
            &media,
            Some(&current_media),
            current_viewport,
        );
        set_media_query_list_bool_slot(scope, mql, MEDIA_QUERY_LIST_MATCHES_SLOT, current_matches);
        if previous_matches == current_matches {
            continue;
        }
        let event = create_media_query_list_event(scope, &media, current_matches);
        let _ = dispatch_media_query_list_event(scope, mql, event);
        dispatched = true;
    }
    dispatched
}

fn media_query_list_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    slot: &str,
) -> Option<v8::Local<'s, v8::Value>> {
    get_private_value(scope, list, slot)
}

fn media_query_list_media_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<String> {
    media_query_list_slot_value(scope, list, MEDIA_QUERY_LIST_MEDIA_SLOT)
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
}

fn set_media_query_list_slot_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    slot: &str,
    value: v8::Local<'s, v8::Value>,
) {
    set_private_value(scope, list, slot, value);
}

fn set_media_query_list_bool_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    slot: &str,
    value: bool,
) {
    set_media_query_list_slot_value(scope, list, slot, v8::Boolean::new(scope, value).into());
}

fn serialize_match_media_query_list(query: &str) -> String {
    if query.trim().is_empty() {
        return String::new();
    }
    normalize_media_query_list(query)
}

pub(crate) fn evaluate_match_media_query_list(
    query: &str,
    emulated_media: Option<&crate::protocol_types::EmulatedMediaOverrides>,
) -> bool {
    evaluate_match_media_query_list_with_viewport_width(query, emulated_media, None)
}

pub(crate) fn evaluate_match_media_query_list_with_viewport_width(
    query: &str,
    emulated_media: Option<&crate::protocol_types::EmulatedMediaOverrides>,
    viewport_width: Option<f64>,
) -> bool {
    evaluate_match_media_query_list_with_viewport(
        query,
        emulated_media,
        match_media_style_viewport(viewport_width),
    )
}

pub(crate) fn evaluate_match_media_query_list_with_viewport(
    query: &str,
    emulated_media: Option<&crate::protocol_types::EmulatedMediaOverrides>,
    viewport: StyleViewport,
) -> bool {
    if query.trim().is_empty() {
        return true;
    }
    evaluate_media_query_list(query, emulated_media, viewport)
}

fn match_media_style_viewport(viewport_width: Option<f64>) -> StyleViewport {
    StyleViewport::new(
        Some(viewport_width.unwrap_or(DEFAULT_WINDOW_SURFACE_PROFILE.inner_width)),
        Some(DEFAULT_WINDOW_SURFACE_PROFILE.inner_height),
    )
    .with_screen_size(
        Some(DEFAULT_WINDOW_SURFACE_PROFILE.screen_width),
        Some(DEFAULT_WINDOW_SURFACE_PROFILE.screen_height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_media_stylo_serializes_with_css_token_boundaries() {
        assert_eq!(
            serialize_match_media_query_list(
                "screen and (orientation:landscape), print and (prefers-color-scheme: dark)"
            ),
            "screen and (orientation: landscape), print and (prefers-color-scheme: dark)"
        );
        assert_eq!(
            serialize_match_media_query_list("(min-aspect-ratio:16/9)"),
            "(min-aspect-ratio: 16 / 9)"
        );
        assert_eq!(serialize_match_media_query_list("and"), "not all");
        assert_eq!(serialize_match_media_query_list(" , "), "not all, not all");
        assert_eq!(serialize_match_media_query_list("foo,"), "foo, not all");
    }

    #[test]
    fn match_media_stylo_evaluates_multiple_conditions_and_not_modifier() {
        assert!(evaluate_match_media_query_list("", None));
        assert!(evaluate_match_media_query_list(" \n\t", None));
        assert!(evaluate_match_media_query_list(
            "screen and (orientation: landscape) and (prefers-color-scheme: light)",
            None,
        ));
        assert!(!evaluate_match_media_query_list(
            "screen and (orientation: portrait) and (prefers-color-scheme: light)",
            None,
        ));
        assert!(evaluate_match_media_query_list(
            "not (orientation: portrait)",
            None,
        ));
        assert!(!evaluate_match_media_query_list("not all", None));
    }

    #[test]
    fn match_media_evaluates_default_desktop_viewport_and_input_features() {
        assert!(evaluate_match_media_query_list("(min-width: 768px)", None));
        assert!(evaluate_match_media_query_list("(width: 1920px)", None));
        assert!(evaluate_match_media_query_list("(min-height: 720px)", None));
        assert!(evaluate_match_media_query_list("(height: 1080px)", None));
        assert!(!evaluate_match_media_query_list("(max-width: 768px)", None));
        assert!(evaluate_match_media_query_list(
            "(min-device-width: 1px)",
            None,
        ));
        assert!(evaluate_match_media_query_list(
            "(device-width: 1920px)",
            None,
        ));
        assert!(evaluate_match_media_query_list(
            "(min-device-height: 1px)",
            None,
        ));
        assert!(evaluate_match_media_query_list(
            "(device-height: 1080px)",
            None,
        ));
        assert!(!evaluate_match_media_query_list(
            "(max-device-width: 1px)",
            None,
        ));
        assert!(evaluate_match_media_query_list("(pointer)", None));
        assert!(evaluate_match_media_query_list("(pointer: fine)", None));
        assert!(!evaluate_match_media_query_list("(pointer: coarse)", None));
        assert!(evaluate_match_media_query_list("(hover)", None));
        assert!(evaluate_match_media_query_list("(hover: hover)", None));
        assert!(!evaluate_match_media_query_list("(hover: none)", None));
        assert!(evaluate_match_media_query_list("(any-pointer: fine)", None));
        assert!(evaluate_match_media_query_list("(any-hover: hover)", None));
    }

    #[test]
    fn match_media_viewport_width_override_does_not_override_screen_size() {
        assert!(evaluate_match_media_query_list_with_viewport_width(
            "(width: 800px) and (device-width: 1920px)",
            None,
            Some(800.0),
        ));
        assert!(!evaluate_match_media_query_list_with_viewport_width(
            "(width: 800px) and (device-width: 800px)",
            None,
            Some(800.0),
        ));
    }

    #[test]
    fn match_media_evaluation_uses_stylo_for_standard_media_features() {
        assert!(evaluate_match_media_query_list(
            "(width >= 768px) and (color)",
            None,
        ));
        assert!(!evaluate_match_media_query_list(
            "(width < 768px) or (color-index)",
            None,
        ));
        assert!(evaluate_match_media_query_list(
            "print and (prefers-color-scheme: dark)",
            Some(&crate::protocol_types::EmulatedMediaOverrides {
                media: Some("print".to_owned()),
                color_scheme: Some("dark".to_owned()),
                ..Default::default()
            }),
        ));
    }

    #[test]
    fn match_media_evaluation_defaults_to_chromium_no_preference_profile() {
        assert!(!evaluate_match_media_query_list(
            "(prefers-reduced-motion)",
            None,
        ));
        assert!(!evaluate_match_media_query_list(
            "(prefers-reduced-motion: reduce)",
            None,
        ));
        assert!(evaluate_match_media_query_list(
            "(prefers-reduced-motion: no-preference)",
            None,
        ));
        assert!(!evaluate_match_media_query_list("(prefers-contrast)", None));
        assert!(!evaluate_match_media_query_list(
            "(prefers-contrast: more)",
            None,
        ));
        assert!(evaluate_match_media_query_list(
            "(prefers-contrast: no-preference)",
            None,
        ));
        assert!(evaluate_match_media_query_list(
            "(forced-colors: active)",
            Some(&crate::protocol_types::EmulatedMediaOverrides {
                forced_colors: Some("active".to_owned()),
                ..Default::default()
            }),
        ));
    }
}
