use super::*;

pub(crate) fn ensure_media_list_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> bool {
    if moli_webapi_declare::implements_interface(scope, object, "MediaList") {
        return true;
    }
    throw_type_error(
        scope,
        &format!("Failed to get '{member}' on 'MediaList': Illegal invocation."),
    );
    false
}

pub(crate) fn build_media_list_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
    media_text: &str,
    owner_kind: &'static str,
) -> v8::Local<'s, v8::Object> {
    let list = MediaListDeclaration { owner, owner_kind }
        .bind(scope)
        .expect("MediaList declaration should bind");
    set_media_list_from_text(scope, list, media_text, false);
    list
}

pub(crate) fn media_list_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> u32 {
    get_private_value(scope, list, CSS_MEDIA_LIST_LENGTH_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0)
}

pub(crate) fn media_list_items_from_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Vec<String> {
    let length = media_list_length(scope, list);
    (0..length)
        .filter_map(|index| {
            list.get_index(scope, index)
                .filter(|value| !value.is_undefined())
                .and_then(|value| value.to_string(scope))
                .map(|value| value.to_rust_string_lossy(scope))
        })
        .collect()
}

pub(crate) fn media_list_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> String {
    media_list_items_from_object(scope, list).join(", ")
}

pub(crate) fn set_media_list_from_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    media_text: &str,
    sync_rule: bool,
) {
    let old_length = media_list_length(scope, list);
    for index in 0..old_length {
        let _ = list.delete_index(scope, index);
    }
    let items = media_query_list_items(media_text);
    set_private_value(
        scope,
        list,
        CSS_MEDIA_LIST_LENGTH_SLOT,
        v8::Integer::new_from_unsigned(scope, items.len() as u32).into(),
    );
    for (index, item) in items.iter().enumerate() {
        if let Some(value) = v8_string(scope, item) {
            let _ = list.set_index(scope, index as u32, value.into());
        }
    }
    if sync_rule {
        sync_media_list_owner_rule(scope, list);
    }
}

pub(crate) fn sync_media_list_owner_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) {
    let Some(owner) = get_private_value(scope, list, CSS_MEDIA_LIST_OWNER_RULE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return;
    };
    let media_text = media_list_text(scope, list);
    match private_string(scope, list, CSS_MEDIA_LIST_OWNER_KIND_SLOT).as_str() {
        CSS_MEDIA_LIST_OWNER_IMPORT_RULE => sync_import_rule_media_text(scope, owner, &media_text),
        CSS_MEDIA_LIST_OWNER_STYLE_SHEET => sync_style_sheet_media_text(scope, owner, &media_text),
        _ => sync_media_rule_media_text(scope, owner, &media_text),
    }
}

pub(crate) fn sync_media_rule_media_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    media_text: &str,
) {
    let media_text = normalize_media_query_list(media_text);
    if apply_live_stylesheet_media_rule_media_mutation(scope, rule, &media_text) {
        sync_media_rule_media_list_slot(scope, rule, &media_text);
        return;
    }
    let Some(parts) = css_at_rule_text_parts_from_object(scope, rule) else {
        return;
    };
    let block = parts.block.unwrap_or_default();
    let css_text = serialized_grouping_rule_text(CssAtRuleKind::Media, &media_text, &block);
    if !commit_detached_css_rule_snapshot_text(scope, rule, &css_text, true) {
        let current_media_text = css_condition_rule_read_from_object(scope, rule)
            .filter(|view| view.rule_type == CssRuleType::Media)
            .map(|view| view.condition_text)
            .unwrap_or_default();
        sync_media_rule_media_list_slot(scope, rule, &current_media_text);
        return;
    }
    sync_media_rule_media_list_slot(scope, rule, &media_text);
}

pub(crate) fn sync_media_rule_media_list_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    media_text: &str,
) {
    if let Some(list) = get_private_value(scope, rule, CSS_MEDIA_RULE_MEDIA_LIST_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_media_list_from_text(scope, list, media_text, false);
    }
}

pub(crate) fn sync_import_rule_media_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    media_text: &str,
) {
    let media_text = normalize_media_query_list(media_text);
    if apply_live_stylesheet_import_rule_media_mutation(scope, rule, &media_text) {
        sync_import_rule_media_list_slot(scope, rule, &media_text);
        return;
    }
    let Some(import) = css_import_rule_view_from_object(scope, rule) else {
        return;
    };
    let css_text =
        serialized_import_rule_text_with_media(&import.href, &import.condition_prefix, &media_text);
    if commit_detached_css_rule_snapshot_text(scope, rule, &css_text, true) {
        sync_import_rule_media_list_slot(scope, rule, &media_text);
    } else {
        sync_import_rule_media_list_slot_from_current_rule(scope, rule);
    }
}

fn sync_import_rule_media_list_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    media_text: &str,
) {
    if let Some(list) = get_private_value(scope, rule, CSS_IMPORT_RULE_MEDIA_LIST_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_media_list_from_text(scope, list, media_text, false);
    }
}

pub(crate) fn sync_import_rule_media_list_slot_from_current_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) {
    let current_media_text = css_import_rule_view_from_object(scope, rule)
        .map(|view| view.media_text)
        .unwrap_or_default();
    if let Some(list) = get_private_value(scope, rule, CSS_IMPORT_RULE_MEDIA_LIST_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_media_list_from_text(scope, list, &current_media_text, false);
    }
}

pub(crate) fn css_media_rule_media_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSMediaRule", "media") {
        return;
    }
    if let Some(list) = get_private_value(scope, args.this(), CSS_MEDIA_RULE_MEDIA_LIST_SLOT) {
        rv.set(list);
        return;
    }
    let media_text = css_condition_rule_read_from_object(scope, args.this())
        .filter(|view| view.rule_type == CssRuleType::Media)
        .map(|view| view.condition_text)
        .unwrap_or_default();
    let list = build_media_list_object(
        scope,
        args.this(),
        &media_text,
        CSS_MEDIA_LIST_OWNER_MEDIA_RULE,
    );
    set_private_value(
        scope,
        args.this(),
        CSS_MEDIA_RULE_MEDIA_LIST_SLOT,
        list.into(),
    );
    rv.set(list.into());
}

pub(crate) fn css_media_rule_media_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSMediaRule", "media") {
        return;
    }
    let Some(media_text) =
        cssom_dom_string_property_value(scope, args.get(0), "CSSMediaRule", "media")
    else {
        return;
    };
    sync_media_rule_media_text(scope, args.this(), &media_text);
}

pub(crate) fn css_media_rule_matches_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSMediaRule", "matches") {
        return;
    }
    let media_text = css_condition_rule_read_from_object(scope, args.this())
        .filter(|view| view.rule_type == CssRuleType::Media)
        .map(|view| view.condition_text)
        .unwrap_or_default();
    let (emulated_media, viewport) =
        if let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) {
            let host = unsafe { &*host_ptr };
            (Some(host.emulated_media().clone()), host.style_viewport())
        } else {
            (None, crate::style_engine::StyleViewport::default())
        };
    let matches = super::media_queries::evaluate_match_media_query_list_with_viewport(
        &media_text,
        emulated_media.as_ref(),
        viewport,
    );
    rv.set_bool(matches);
}

pub(crate) fn media_list_media_text_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_media_list_object(scope, args.this(), "mediaText") {
        return;
    }
    let list = args.this();
    let media_text = media_list_text(scope, list);
    rv.set(v8_dynamic_string_value(scope, &media_text));
}

pub(crate) fn media_list_media_text_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_media_list_object(scope, args.this(), "mediaText") {
        return;
    }
    let media_text = if args.get(0).is_null_or_undefined() {
        String::new()
    } else {
        let Some(value) =
            cssom_dom_string_property_value(scope, args.get(0), "MediaList", "mediaText")
        else {
            return;
        };
        value
    };
    set_media_list_from_text(scope, args.this(), &media_text, true);
}

pub(crate) fn media_list_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_media_list_object(scope, args.this(), "length") {
        return;
    }
    let length = media_list_length(scope, args.this());
    rv.set(v8::Integer::new_from_unsigned(scope, length).into());
}

pub(crate) fn media_list_to_string_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_media_list_object(scope, args.this(), "toString") {
        return;
    }
    let list = args.this();
    let media_text = media_list_text(scope, list);
    rv.set(v8_dynamic_string_value(scope, &media_text));
}

pub(crate) fn media_list_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_media_list_object(scope, args.this(), "item") {
        return;
    }
    let Some(parsed) = webidl::parse_args::<MediaListItemArgs>(scope, &args) else {
        return;
    };
    let list = args.this();
    if let Some(value) = list.get_index(scope, parsed.index)
        && !value.is_undefined()
    {
        rv.set(value);
    } else {
        rv.set(v8::null(scope).into());
    }
}

pub(crate) fn media_list_delete_medium_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_media_list_object(scope, args.this(), "deleteMedium") {
        return;
    }
    let Some(parsed) = webidl::parse_args::<MediaListDeleteMediumArgs>(scope, &args) else {
        return;
    };
    let list = args.this();
    let Some(media_text) =
        delete_media_query_list_medium(&media_list_text(scope, list), &parsed.medium)
    else {
        webidl::throw_dom_exception(scope, "NotFoundError", "Medium was not found.");
        return;
    };
    set_media_list_from_text(scope, list, &media_text, true);
}

pub(crate) fn media_list_append_medium_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_media_list_object(scope, args.this(), "appendMedium") {
        return;
    }
    let Some(parsed) = webidl::parse_args::<MediaListAppendMediumArgs>(scope, &args) else {
        return;
    };
    let list = args.this();
    let Some(media_text) =
        append_media_query_list_medium(&media_list_text(scope, list), &parsed.medium)
    else {
        return;
    };
    set_media_list_from_text(scope, list, &media_text, true);
}
