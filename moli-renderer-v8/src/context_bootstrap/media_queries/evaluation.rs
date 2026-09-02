use super::dispatch_media_query_list_event;
use crate::context_bootstrap::DEFAULT_WINDOW_SURFACE_PROFILE;
use crate::context_bootstrap::events::initialize_event_object;
use crate::context_bootstrap::{
    MEDIA_QUERY_LIST_MATCHES_SLOT, MEDIA_QUERY_LIST_MEDIA_SLOT, MEDIA_QUERY_LIST_ONCHANGE_SLOT,
    MEDIA_QUERY_LIST_REGISTRY_SLOT,
};
use crate::context_bootstrap::{global_queue_array, push_object_to_global_registry};
use crate::style_engine::{
    StyleViewport,
    media_list::{evaluate_media_query_list, normalize_media_query_list},
};
use crate::util::{
    callback_data_index_value, callback_data_item, context_host_ptr_from_global_bridge,
    get_private_value, set_private_value,
};
use crate::webidl;
use moli_webapi_declare::{WebApiFunctionTemplate, WebApiObject};

fn current_window_style_viewport(
    scope: &mut v8::PinScope<'_, '_>,
    host: &crate::native_bridge::JsContextHost,
) -> StyleViewport {
    let global = scope.get_current_context().global(scope);
    super::super::window_accessors::window_child_context_handle(scope, global)
        .and_then(|frame_handle| {
            crate::native_bridge::element::iframe_handle_viewport(host, frame_handle)
        })
        .unwrap_or_else(|| host.style_viewport())
}

#[derive(WebApiObject)]
#[webapi(interface = "MediaQueryList")]
struct MediaQueryListObjectDeclaration {
    #[webapi(slot = MEDIA_QUERY_LIST_MEDIA_SLOT)]
    media: String,
    #[webapi(slot = MEDIA_QUERY_LIST_MATCHES_SLOT)]
    matches: bool,
    #[webapi(slot = MEDIA_QUERY_LIST_ONCHANGE_SLOT, init = "null")]
    onchange: (),
}

#[derive(WebApiFunctionTemplate)]
#[webapi(name = "MediaQueryList")]
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

#[derive(WebApiObject)]
#[webapi(interface = "Event", allow_empty)]
struct MediaQueryListChangeEventObjectDeclaration {}

#[derive(WebApiObject)]
#[webapi(interface = "Event")]
struct MediaQueryListChangeEventPropertiesDeclaration {
    #[webapi(data_property)]
    media: String,
    #[webapi(data_property)]
    matches: bool,
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
        let media = media_query_list_media_slot(scope, args.this()).unwrap_or_default();
        let matches = if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            let host = unsafe { &*host_ptr };
            evaluate_match_media_query_list_with_viewport(
                &media,
                Some(host.emulated_media()),
                current_window_style_viewport(scope, host),
            )
        } else {
            evaluate_match_media_query_list(&media, None)
        };
        rv.set_bool(matches);
        return;
    }
    rv.set(
        media_query_list_slot_value(scope, args.this(), slot)
            .unwrap_or_else(|| v8::undefined(scope).into()),
    );
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
    let value = if args.get(0).is_null_or_undefined() {
        v8::null(scope).into()
    } else {
        args.get(0)
    };
    set_media_query_list_slot_value(scope, args.this(), MEDIA_QUERY_LIST_ONCHANGE_SLOT, value);
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
    let mql = MediaQueryListObjectDeclaration::new(serialized_query, matches)
        .bind(scope)
        .expect("MediaQueryList declaration should bind");
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
    previous_media: &crate::protocol_types::EmulatedMediaOverrides,
    previous_viewport: StyleViewport,
    current_media: &crate::protocol_types::EmulatedMediaOverrides,
    current_viewport: StyleViewport,
) {
    let Some(registry) = global_queue_array(scope, MEDIA_QUERY_LIST_REGISTRY_SLOT) else {
        return;
    };
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
        let previous_matches = evaluate_match_media_query_list_with_viewport(
            &media,
            Some(previous_media),
            previous_viewport,
        );
        let current_matches = evaluate_match_media_query_list_with_viewport(
            &media,
            Some(current_media),
            current_viewport,
        );
        set_media_query_list_bool_slot(scope, mql, MEDIA_QUERY_LIST_MATCHES_SLOT, current_matches);
        if previous_matches == current_matches {
            continue;
        }
        let event = media_query_list_change_event(scope, &media, current_matches);
        let _ = dispatch_media_query_list_event(scope, mql, event);
    }
}

fn media_query_list_change_event<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    media: &str,
    matches: bool,
) -> v8::Local<'s, v8::Object> {
    let event = MediaQueryListChangeEventObjectDeclaration::new()
        .bind(scope)
        .expect("MediaQueryList change Event declaration should bind");
    initialize_event_object(scope, event, "change", false, false);
    MediaQueryListChangeEventPropertiesDeclaration::new(media.to_owned(), matches)
        .initialize(scope, event)
        .expect("MediaQueryList change Event properties should initialize object");
    event
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
