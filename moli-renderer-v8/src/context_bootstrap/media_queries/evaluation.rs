use super::{
    dispatch_media_query_list_event, initialize_simple_event_target,
    simple_object_event_set_handler,
};
use crate::context_bootstrap::DEFAULT_WINDOW_SURFACE_PROFILE;
use crate::context_bootstrap::window_accessors::{
    prepare_window_viewport_read, window_current_dispatch_scope,
};
use crate::context_bootstrap::{MEDIA_QUERY_LIST_MEDIA_SLOT, MEDIA_QUERY_LIST_ONCHANGE_SLOT};
use crate::native_bridge::{
    PageScriptHost, WindowDocumentTaskTarget, WindowExecutionContextBinding,
};
use crate::style_engine::{
    StyleViewport, StyloStyleEnvironment,
    media_list::{evaluate_media_query_list, normalize_media_query_list},
};
use crate::util::{
    callback_data_index_value, callback_data_item, context_host_ptr_from_global_bridge,
    get_private_value, set_private_value,
};
use crate::web_api_interfaces;
use crate::webidl;
use moli_webapi::media_query_list::new_media_query_list;
use moli_webapi_declare::WebApiFunctionTemplate;

#[derive(WebApiFunctionTemplate)]
#[webapi(interface = web_api_interfaces::MediaQueryList)]
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
    let Some(attribute) = callback_data_item(
        scope,
        &args,
        MEDIA_QUERY_LIST_ATTRIBUTES,
        "MediaQueryList attributes",
    ) else {
        rv.set_undefined();
        return;
    };
    if attribute == "matches" {
        if let Some(matches) = read_media_query_matches(scope, args.this()) {
            rv.set_bool(matches);
        }
        return;
    }
    rv.set(
        media_query_list_slot_value(scope, args.this(), MEDIA_QUERY_LIST_MEDIA_SLOT)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
}

const MEDIA_QUERY_ID_SLOT: &str = "__moliMediaQueryId";
const MEDIA_QUERY_ROOTS_SLOT: &str = "__moliMediaQueryRoots";

fn remove_media_query_root<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    id: u64,
) {
    let Some(context) = list.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let global = context.global(scope);
    if let Some(registry) = get_private_value(scope, global, MEDIA_QUERY_ROOTS_SLOT)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
    {
        let key = v8::BigInt::new_from_u64(scope, id);
        registry.delete(scope, key.into());
    }
}

fn media_query_id<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<u64> {
    get_private_value(scope, list, MEDIA_QUERY_ID_SLOT)
        .and_then(|value| v8::Local::<v8::BigInt>::try_from(value).ok())
        .and_then(|value| {
            let (id, lossless) = value.u64_value();
            lossless.then_some(id)
        })
}

pub(in crate::context_bootstrap) fn read_media_query_matches<'s>(
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
    let Some(document) = window_current_dispatch_scope(scope, global, host)
        .and_then(|dispatch| host.window_document_handle_for_dispatch_scope(dispatch))
    else {
        return Some(false);
    };
    if !prepare_window_viewport_read(scope, global) {
        return None;
    }
    Some(host.media_query_matches_for_document(document, &media))
}

pub(in crate::context_bootstrap) fn media_query_listeners_changed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    event_type: &str,
) {
    if event_type != "change" {
        return;
    }
    let Some(id) = media_query_id(scope, list) else {
        return;
    };
    let Some(context) = list.get_creation_context(scope) else {
        return;
    };
    let scope = &mut v8::ContextScope::new(scope, context);
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    let host = unsafe { &*host_ptr };
    let active = !super::simple_object_event_listeners_snapshot(scope, list, event_type).is_empty();
    if active == host.media_query_is_listening(id) {
        return;
    }
    let global = context.global(scope);
    if !active {
        if !host.stop_media_query_listening(id) {
            remove_media_query_root(scope, list, id);
        }
        return;
    }
    let Some(target) = host.current_window_document_task_target(scope) else {
        return;
    };
    let Some(document) = host.window_document_handle_for_dispatch_scope(target.dispatch_scope())
    else {
        return;
    };
    let Some(media) = media_query_list_media_slot(scope, list) else {
        return;
    };
    let matches = host.media_query_matches_for_document(document, &media);
    if let Err(error) = host.register_media_query(scope, id, target, media, matches, list) {
        crate::context_bootstrap::throw_error(scope, &error.to_string());
        return;
    }
    // Keep pending notifications alive even if their last listener is removed.
    // Read-only queries use the current environment in their matches getter.
    let registry = get_private_value(scope, global, MEDIA_QUERY_ROOTS_SLOT)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        .unwrap_or_else(|| {
            let registry = v8::Map::new(scope);
            set_private_value(scope, global, MEDIA_QUERY_ROOTS_SLOT, registry.into());
            registry
        });
    let key = v8::BigInt::new_from_u64(scope, id);
    registry.set(scope, key.into(), list.into());
}

/// These strong roots exist only for a Document's current media phase. The
/// persistent native query holds the list weakly; it never roots a Context.
struct PreparedMediaQueryEvent {
    id: u64,
    binding: WindowExecutionContextBinding,
    list: v8::Global<v8::Object>,
    event: v8::Global<v8::Object>,
}

fn take_media_query_events(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    target: WindowDocumentTaskTarget,
) -> Vec<PreparedMediaQueryEvent> {
    // A deferred sample can find no change after the last listener left.
    // Release those entries and roots without manufacturing an event.
    for (id, list) in unsafe { &*host_ptr }.retire_idle_media_queries(target) {
        if let Some(list) = list.to_local(scope) {
            remove_media_query_root(scope, list, id);
        }
    }
    // Freeze the whole Document batch once, not separately per Realm. Queries
    // queued or registered during a callback belong to the next media phase.
    let candidates = unsafe { &*host_ptr }.take_media_query_notifications(target);
    let mut events = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        if !unsafe { &*host_ptr }.media_query_is_current(target, candidate.id) {
            unsafe { &*host_ptr }.retire_media_query(candidate.id);
            continue;
        }
        // No Host/state borrow survives a weak upgrade or any V8 operation.
        let Some(list) = candidate.list.to_local(scope) else {
            unsafe { &*host_ptr }.retire_media_query(candidate.id);
            continue;
        };
        let Some(context) = list.get_creation_context(scope) else {
            unsafe { &*host_ptr }.retire_media_query(candidate.id);
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
                super::event::create_live_media_query_list_event(scope, list, &candidate.media);
            v8::Global::new(scope, event)
        });
        let Some(event) = event else {
            unsafe { &*host_ptr }.retire_media_query(candidate.id);
            continue;
        };
        // Prepare every event before the first author callback. The existing
        // native event.matches getter deliberately continues to read its list.
        events.push(PreparedMediaQueryEvent {
            id: candidate.id,
            binding,
            list,
            event,
        });
    }
    events
}

fn finish_media_query_events(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    events: &[PreparedMediaQueryEvent],
) {
    for prepared in events {
        if unsafe { &*host_ptr }.finish_media_query_notification(prepared.id) {
            continue;
        }
        let list = v8::Local::new(scope, &prepared.list);
        remove_media_query_root(scope, list, prepared.id);
    }
}

/// Drivers provide their current Document authorization. Every policy shares the same frozen batch, Realm binding, event
/// dispatch, recovery, and root cleanup.
pub(crate) fn dispatch_media_query_events(
    scope: &mut v8::PinScope<'_, '_>,
    host_ptr: *mut PageScriptHost,
    mut current_target: impl FnMut() -> Option<WindowDocumentTaskTarget>,
) -> anyhow::Result<bool> {
    let Some(target) = current_target() else {
        return Ok(false);
    };
    let events = take_media_query_events(scope, host_ptr, target);
    let result = (|| -> anyhow::Result<bool> {
        let mut invoked = false;
        for (index, prepared) in events.iter().enumerate() {
            let Some(target) = current_target() else {
                break;
            };
            // Only an explicit registry rebind can transfer this frozen query
            // to a successor Document. Navigation and Realm retirement cannot.
            if !unsafe { &*host_ptr }.media_query_is_current(target, prepared.id) {
                continue;
            }
            let result: anyhow::Result<bool> = prepared
                .binding
                .with_current_scope(scope, host_ptr, |scope, _| {
                    let list = v8::Local::new(scope, &prepared.list);
                    let event = v8::Local::new(scope, &prepared.event);
                    let _ = dispatch_media_query_list_event(scope, list, event);
                    crate::script_vm::ScriptVm::environment_checkpoint(scope)?;
                    Ok(true)
                })
                .unwrap_or(Ok(false));
            if result.is_err()
                && let Some(target) = current_target()
            {
                let uncalled = events[index + 1..]
                    .iter()
                    .map(|event| event.id)
                    .collect::<Vec<_>>();
                unsafe { &*host_ptr }.restore_media_query_notifications(target, &uncalled);
            }
            invoked |= result?;
        }
        Ok(invoked)
    })();
    finish_media_query_events(scope, host_ptr, &events);
    result
}

const MEDIA_QUERY_LIST_ATTRIBUTES: &[&str] = &["media", "matches"];

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
    media_query_listeners_changed(scope, target, "change");
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
    let host = context_host_ptr_from_global_bridge(scope).map(|host| unsafe { &*host });
    let mql = new_media_query_list(scope, serialized_query);
    initialize_simple_event_target(scope, mql);
    if let Some(host) = host {
        // Media change events follow creation order, even when listeners were
        // added later. Only subscribed lists enter the native map.
        let id = match host.allocate_media_query_id() {
            Ok(id) => id,
            Err(error) => {
                crate::context_bootstrap::throw_error(scope, &error.to_string());
                return;
            }
        };
        let value = v8::BigInt::new_from_u64(scope, id);
        set_private_value(scope, mql, MEDIA_QUERY_ID_SLOT, value.into());
    }
    rv.set(mql.into());
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
    evaluate_media_query_list(
        query,
        emulated_media
            .map(StyloStyleEnvironment::from_emulated_media)
            .unwrap_or_default(),
        match_media_style_viewport(viewport_width),
    )
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
