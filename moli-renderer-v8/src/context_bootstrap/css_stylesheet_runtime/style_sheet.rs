use super::*;

#[cfg(test)]
thread_local! {
    static CSS_STYLE_SHEET_RULE_SYNC_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) fn reset_css_style_sheet_rule_sync_count_for_test() {
    CSS_STYLE_SHEET_RULE_SYNC_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn css_style_sheet_rule_sync_count_for_test() -> usize {
    CSS_STYLE_SHEET_RULE_SYNC_COUNT.with(std::cell::Cell::get)
}

pub(crate) fn css_style_sheet_rules_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    get_private_value(scope, object, CSS_STYLE_SHEET_RULES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .inspect(|&existing| {
            install_css_rule_list_surface(scope, existing);
        })
        .unwrap_or_else(|| {
            let array = new_css_rule_list_object(scope);
            set_private_value(scope, object, CSS_STYLE_SHEET_RULES_SLOT, array.into());
            array
        })
}

pub(crate) fn initialize_css_style_sheet_rules_from_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    css_text: &str,
) {
    #[cfg(test)]
    CSS_STYLE_SHEET_RULE_SYNC_COUNT.with(|count| count.set(count.get() + 1));

    replace_css_style_sheet_live_stylesheet_from_text(scope, object, css_text)
        .expect("a CSSStyleSheet source sync requires a page context");
}

pub(crate) fn sync_constructed_css_style_sheet_rules_from_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    css_text: &str,
) {
    crate::native_bridge::document::clear_adopted_stylesheet_font_face_wrappers(scope, object);
    replace_css_style_sheet_live_stylesheet_from_text(scope, object, css_text)
        .expect("a constructed CSSStyleSheet source sync requires a page context");
}

pub(crate) fn initialize_css_module_style_sheet_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    module_url: &str,
) {
    set_private_value(
        scope,
        object,
        CSS_STYLE_SHEET_CONSTRUCTED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    if let Some(handle) = current_css_style_sheet_constructor_document_handle_for_context(scope)
        .filter(|handle| {
            context_host_ptr_from_global_bridge(scope).is_none_or(|host| {
                unsafe { &*host }
                    .dom_host()
                    .node(*handle)
                    .is_some_and(crate::dom::native::Node::is_document)
            })
        })
    {
        set_css_style_sheet_constructor_document_handle(scope, object, handle);
    }
    if let Ok(base_url) = module_url.parse::<url::Url>() {
        set_private_string(
            scope,
            object,
            CSS_STYLE_SHEET_BASE_URL_SLOT,
            base_url.as_str(),
        );
    }
    sync_constructed_css_style_sheet_rules_from_text(scope, object, "");
}

pub(crate) fn new_css_style_sheet_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let rules = new_css_rule_list_object(scope).into();
    CssStyleSheetDeclaration::new(rules)
        .bind(scope)
        .expect("CSSStyleSheet declaration should bind")
}

pub(crate) fn set_css_style_sheet_owner_node(
    scope: &mut v8::PinScope<'_, '_>,
    sheet: v8::Local<'_, v8::Object>,
    owner_node: v8::Local<'_, v8::Object>,
) {
    set_private_value(
        scope,
        sheet,
        CSS_STYLE_SHEET_OWNER_NODE_SLOT,
        owner_node.into(),
    );
}

pub(crate) fn set_css_style_sheet_href(
    scope: &mut v8::PinScope<'_, '_>,
    sheet: v8::Local<'_, v8::Object>,
    href: &url::Url,
) {
    set_private_string(scope, sheet, CSS_STYLE_SHEET_HREF_SLOT, href.as_str());
}

pub(crate) fn set_css_style_sheet_origin_clean<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    origin_clean: bool,
) {
    require_css_style_sheet_live_stylesheet(scope, sheet).set_origin_clean(origin_clean);
}

pub(crate) fn css_style_sheet_origin_clean<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> bool {
    require_css_style_sheet_live_stylesheet(scope, sheet).origin_clean()
}

fn ensure_css_style_sheet_origin_clean<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    message: &'static str,
) -> bool {
    if css_style_sheet_origin_clean(scope, sheet) {
        return true;
    }
    webidl::throw_dom_exception(scope, "SecurityError", message);
    false
}

pub(crate) fn clear_css_style_sheet_owner_node(
    scope: &mut v8::PinScope<'_, '_>,
    sheet: v8::Local<'_, v8::Object>,
) {
    set_private_value(
        scope,
        sheet,
        CSS_STYLE_SHEET_OWNER_NODE_SLOT,
        v8::null(scope).into(),
    );
}

pub(crate) fn adopted_style_sheet_installations_from_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Vec<crate::style_engine::AdoptedStyleSheetInstallation> {
    let mut installations = Vec::new();
    if let Ok(sheets) = v8::Local::<v8::Array>::try_from(value) {
        for index in 0..sheets.length() {
            let Some(sheet_value) = sheets.get_index(scope, index) else {
                continue;
            };
            let Ok(sheet) = v8::Local::<v8::Object>::try_from(sheet_value) else {
                continue;
            };
            let stylesheet = require_css_style_sheet_live_stylesheet(scope, sheet);
            installations.push(crate::style_engine::AdoptedStyleSheetInstallation::new(
                stylesheet,
            ));
        }
    }
    installations
}

pub(crate) fn sync_css_style_sheet_document_adopted_owner_tracking<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
    document: DomHandle,
) {
    sync_css_style_sheet_adopted_owner_tracking(
        scope,
        array,
        CssStyleSheetAdoptedOwnerKey::Document(document),
    );
}

pub(crate) fn clear_css_style_sheet_document_adopted_owner_tracking<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
    document: DomHandle,
) {
    clear_css_style_sheet_adopted_owner_tracking(
        scope,
        array,
        CssStyleSheetAdoptedOwnerKey::Document(document),
    );
}

pub(crate) fn sync_css_style_sheet_shadow_root_adopted_owner_tracking<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
    root: DomHandle,
) {
    sync_css_style_sheet_adopted_owner_tracking(
        scope,
        array,
        CssStyleSheetAdoptedOwnerKey::ShadowRoot(root),
    );
}

pub(crate) fn clear_css_style_sheet_shadow_root_adopted_owner_tracking<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
    root: DomHandle,
) {
    clear_css_style_sheet_adopted_owner_tracking(
        scope,
        array,
        CssStyleSheetAdoptedOwnerKey::ShadowRoot(root),
    );
}

pub(crate) fn sync_css_style_sheet_adopted_owner_tracking<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
    owner: CssStyleSheetAdoptedOwnerKey,
) {
    let previous_sheets = tracked_adopted_style_sheet_objects_for_array(scope, array);
    let current_sheets = adopted_style_sheet_objects_from_array_object(scope, array);
    for previous in previous_sheets {
        if !object_list_contains(&current_sheets, previous) {
            remove_css_style_sheet_adopted_owner_key(scope, previous, owner);
        }
    }
    for &current in &current_sheets {
        add_css_style_sheet_adopted_owner(scope, current, owner, array);
    }
    set_tracked_adopted_style_sheet_objects_for_array(scope, array, &current_sheets);
}

pub(crate) fn clear_css_style_sheet_adopted_owner_tracking<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
    owner: CssStyleSheetAdoptedOwnerKey,
) {
    let mut sheets = tracked_adopted_style_sheet_objects_for_array(scope, array);
    for current in adopted_style_sheet_objects_from_array_object(scope, array) {
        if !object_list_contains(&sheets, current) {
            sheets.push(current);
        }
    }
    for sheet in sheets {
        remove_css_style_sheet_adopted_owner_key(scope, sheet, owner);
    }
}

pub(crate) fn adopted_style_sheet_objects_from_array_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    let length = array
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    (0..length)
        .filter_map(|index| {
            let object = array.get_index(scope, index)?.try_into().ok()?;
            moli_webapi_declare::implements_interface(scope, object, "CSSStyleSheet")
                .then_some(object)
        })
        .collect()
}

pub(crate) fn tracked_adopted_style_sheet_objects_for_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
) -> Vec<v8::Local<'s, v8::Object>> {
    get_private_value(scope, array, ADOPTED_STYLE_SHEETS_ARRAY_TRACKED_SHEETS_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|snapshot| adopted_style_sheet_objects_from_array_object(scope, snapshot))
        .unwrap_or_default()
}

pub(crate) fn set_tracked_adopted_style_sheet_objects_for_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    array: v8::Local<'s, v8::Object>,
    sheets: &[v8::Local<'s, v8::Object>],
) {
    let snapshot = serialize_v8_array(scope, sheets).unwrap_or_else(|| v8::Array::new(scope, 0));
    set_private_value(
        scope,
        array,
        ADOPTED_STYLE_SHEETS_ARRAY_TRACKED_SHEETS_SLOT,
        snapshot.into(),
    );
}

pub(crate) fn add_css_style_sheet_adopted_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    owner: CssStyleSheetAdoptedOwnerKey,
    owner_array: v8::Local<'s, v8::Object>,
) {
    let mut owners = css_style_sheet_adopted_owners(scope, sheet);
    if let Some(existing) = owners.iter_mut().find(|candidate| candidate.key == owner) {
        if !existing.array.strict_equals(owner_array.into()) {
            existing.array = owner_array;
            set_css_style_sheet_adopted_owners(scope, sheet, &owners);
        }
        return;
    }
    owners.push(CssStyleSheetAdoptedOwner {
        key: owner,
        array: owner_array,
    });
    set_css_style_sheet_adopted_owners(scope, sheet, &owners);
}

pub(crate) fn remove_css_style_sheet_adopted_owner_key<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    owner: CssStyleSheetAdoptedOwnerKey,
) {
    let mut owners = css_style_sheet_adopted_owners(scope, sheet);
    let before = owners.len();
    owners.retain(|candidate| candidate.key != owner);
    if owners.len() != before {
        set_css_style_sheet_adopted_owners(scope, sheet, &owners);
    }
}

pub(crate) fn css_style_sheet_adopted_owners<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> Vec<CssStyleSheetAdoptedOwner<'s>> {
    let Some(keys) = get_private_value(scope, sheet, CSS_STYLE_SHEET_ADOPTED_OWNER_KEYS_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Vec::new();
    };
    let Some(arrays) = get_private_value(scope, sheet, CSS_STYLE_SHEET_ADOPTED_OWNER_ARRAYS_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        return Vec::new();
    };
    let length = keys
        .get(scope, v8str(scope, "length").into())
        .and_then(|value| value.uint32_value(scope))
        .unwrap_or(0);
    (0..length)
        .filter_map(|index| {
            let key_value = keys.get_index(scope, index)?;
            let text = key_value.to_string(scope)?.to_rust_string_lossy(scope);
            let key = css_style_sheet_adopted_owner_key_from_text(&text)?;
            let array = arrays
                .get_index(scope, index)
                .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())?;
            Some(CssStyleSheetAdoptedOwner { key, array })
        })
        .collect()
}

pub(crate) fn set_css_style_sheet_adopted_owners<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    owners: &[CssStyleSheetAdoptedOwner<'s>],
) {
    let keys = owners
        .iter()
        .map(|owner| css_style_sheet_adopted_owner_key_text(owner.key))
        .collect::<Vec<_>>();
    let arrays = owners.iter().map(|owner| owner.array).collect::<Vec<_>>();
    let keys =
        serialize_v8_array(scope, keys.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0));
    let arrays =
        serialize_v8_array(scope, arrays.as_slice()).unwrap_or_else(|| v8::Array::new(scope, 0));
    set_private_value(
        scope,
        sheet,
        CSS_STYLE_SHEET_ADOPTED_OWNER_KEYS_SLOT,
        keys.into(),
    );
    set_private_value(
        scope,
        sheet,
        CSS_STYLE_SHEET_ADOPTED_OWNER_ARRAYS_SLOT,
        arrays.into(),
    );
}

pub(crate) fn css_style_sheet_adopted_owner_key_text(
    owner: CssStyleSheetAdoptedOwnerKey,
) -> String {
    match owner {
        CssStyleSheetAdoptedOwnerKey::Document(document) => format!("d:{}", document.index()),
        CssStyleSheetAdoptedOwnerKey::ShadowRoot(root) => format!("s:{}", root.index()),
    }
}

pub(crate) fn css_style_sheet_adopted_owner_key_from_text(
    text: &str,
) -> Option<CssStyleSheetAdoptedOwnerKey> {
    let (kind, index) = text.split_once(':')?;
    let handle = index.parse::<usize>().ok().map(DomHandle::new)?;
    match kind {
        "d" => Some(CssStyleSheetAdoptedOwnerKey::Document(handle)),
        "s" => Some(CssStyleSheetAdoptedOwnerKey::ShadowRoot(handle)),
        _ => None,
    }
}

pub(crate) fn set_css_style_sheet_constructor_document_handle(
    scope: &mut v8::PinScope<'_, '_>,
    sheet: v8::Local<'_, v8::Object>,
    handle: DomHandle,
) {
    set_private_value(
        scope,
        sheet,
        CSS_STYLE_SHEET_CONSTRUCTOR_DOCUMENT_HANDLE_SLOT,
        v8::BigInt::new_from_u64(scope, handle.index() as u64).into(),
    );
}

pub(crate) fn css_style_sheet_css_rules_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "CSSStyleSheet", "cssRules") {
        return;
    }
    if !ensure_css_style_sheet_origin_clean(scope, args.this(), "Cannot access rules") {
        return;
    }
    let rules = css_style_sheet_rules_array(scope, args.this());
    rv.set(rules.into());
}

pub(crate) fn css_style_sheet_owner_rule_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "CSSStyleSheet", "ownerRule") {
        return;
    }
    if let Some(owner_rule) = get_private_value(scope, args.this(), CSS_STYLE_SHEET_OWNER_RULE_SLOT)
    {
        rv.set(owner_rule);
    } else {
        rv.set(v8::null(scope).into());
    }
}

pub(crate) fn css_style_sheet_type_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "StyleSheet", "type") {
        return;
    }
    rv.set(v8str(scope, "text/css").into());
}

pub(crate) fn css_style_sheet_disabled_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "StyleSheet", "disabled") {
        return;
    }
    let disabled = css_style_sheet_disabled(scope, args.this());
    rv.set(v8::Boolean::new(scope, disabled).into());
}

pub(crate) fn css_style_sheet_disabled_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "StyleSheet", "disabled") {
        return;
    }
    let disabled = args.get(0).boolean_value(scope);
    let change = require_css_style_sheet_live_stylesheet(scope, args.this()).set_disabled(disabled);
    notify_css_style_sheet_runtime_state_change(scope, args.this(), change);
}

pub(crate) fn css_style_sheet_disabled<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> bool {
    require_css_style_sheet_live_stylesheet(scope, sheet).disabled()
}

pub(crate) fn css_style_sheet_owner_node_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "StyleSheet", "ownerNode") {
        return;
    }
    if let Some(owner) = css_style_sheet_owner_node(scope, args.this()) {
        rv.set(owner.into());
    } else {
        rv.set(v8::null(scope).into());
    }
}

pub(crate) fn css_style_sheet_parent_style_sheet_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "StyleSheet", "parentStyleSheet") {
        return;
    }
    let parent = get_private_value(scope, args.this(), CSS_STYLE_SHEET_OWNER_RULE_SLOT)
        .and_then(|owner_rule| v8::Local::<v8::Object>::try_from(owner_rule).ok())
        .and_then(|owner_rule| owner_rule.get(scope, v8str(scope, "parentStyleSheet").into()));
    match parent {
        Some(parent) => rv.set(parent),
        None => rv.set(v8::null(scope).into()),
    }
}

pub(crate) fn css_style_sheet_href_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "StyleSheet", "href") {
        return;
    }
    if let Some(owner_rule) = get_private_value(scope, args.this(), CSS_STYLE_SHEET_OWNER_RULE_SLOT)
        .and_then(|owner_rule| v8::Local::<v8::Object>::try_from(owner_rule).ok())
        && let Some(href) = owner_rule.get(scope, v8str(scope, "href").into())
    {
        rv.set(href);
        return;
    }
    let href = private_string(scope, args.this(), CSS_STYLE_SHEET_HREF_SLOT);
    if !href.is_empty() {
        rv.set(v8_dynamic_string_value(scope, &href));
        return;
    }
    let Some(owner) = css_style_sheet_owner_node(scope, args.this()) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(name) = v8_string(scope, "href") else {
        rv.set(v8::null(scope).into());
        return;
    };
    let href = call_object_method(scope, owner, "getAttribute", &[name.into()])
        .filter(|value| !value.is_null_or_undefined());
    match href {
        Some(href) => rv.set(href),
        None => rv.set(v8::null(scope).into()),
    }
}

pub(crate) fn css_style_sheet_title_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "StyleSheet", "title") {
        return;
    }
    let Some(owner) = css_style_sheet_owner_node(scope, args.this()) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(name) = v8_string(scope, "title") else {
        rv.set(v8::null(scope).into());
        return;
    };
    let title = call_object_method(scope, owner, "getAttribute", &[name.into()])
        .filter(|value| !value.is_null_or_undefined())
        .and_then(|value| value.to_string(scope))
        .map(|value| value.to_rust_string_lossy(scope))
        .filter(|value| !value.is_empty());
    match title {
        Some(title) => rv.set(v8_dynamic_string_value(scope, &title)),
        None => rv.set(v8::null(scope).into()),
    }
}

pub(crate) fn css_style_sheet_media_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "StyleSheet", "media") {
        return;
    }
    if let Some(list) = get_private_value(scope, args.this(), CSS_STYLE_SHEET_MEDIA_LIST_SLOT) {
        rv.set(list);
        return;
    }
    let media_text = require_css_style_sheet_live_stylesheet(scope, args.this()).media_text();
    let list = build_media_list_object(
        scope,
        args.this(),
        &media_text,
        CSS_MEDIA_LIST_OWNER_STYLE_SHEET,
    );
    set_private_value(
        scope,
        args.this(),
        CSS_STYLE_SHEET_MEDIA_LIST_SLOT,
        list.into(),
    );
    rv.set(list.into());
}

pub(crate) fn css_style_sheet_media_setter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "StyleSheet", "media") {
        return;
    }
    let Some(media_text) =
        cssom_dom_string_property_value(scope, args.get(0), "CSSStyleSheet", "media")
    else {
        return;
    };
    sync_style_sheet_media_text(scope, args.this(), &media_text);
}

pub(crate) fn css_style_sheet_insert_rule_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    let this = args.this();
    if !ensure_css_style_sheet_object(scope, this, "CSSStyleSheet", "insertRule") {
        return;
    }
    if !ensure_css_style_sheet_origin_clean(scope, this, "Cannot access StyleSheet to insertRule") {
        return;
    }
    let rules = css_style_sheet_rules_array(scope, this);
    let rules_len = css_rule_list_length(scope, rules);
    let Some(parsed) = parse_insert_rule_args(scope, &args, rules_len) else {
        return;
    };
    let index = parsed.index;
    match apply_stylo_top_level_insert_rule_mutation(scope, this, rules, &parsed.rule, index) {
        Ok(()) => rv.set(v8::Integer::new(scope, index as i32).into()),
        Err(error) => throw_insert_rule_error(scope, error),
    }
}

pub(crate) fn css_rule_list_item_path_in_parent_style_sheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
    index: usize,
) -> Option<Vec<usize>> {
    let parent_style_sheet = parent_style_sheet?;
    if let Some(parent_rule) = parent_rule {
        let mut path = css_rule_path_in_parent_style_sheet(scope, parent_rule, parent_style_sheet)?;
        path.push(index);
        return Some(path);
    }
    Some(vec![index])
}

pub(crate) fn css_style_sheet_delete_rule_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "CSSStyleSheet", "deleteRule") {
        return;
    }
    if !ensure_css_style_sheet_origin_clean(
        scope,
        args.this(),
        "Cannot access StyleSheet to deleteRule",
    ) {
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssStyleSheetDeleteRuleArgs>(scope, &args) else {
        return;
    };
    let index = parsed.index;
    let this = args.this();
    let rules = get_private_value(scope, this, CSS_STYLE_SHEET_RULES_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok());
    let rules_len = rules
        .map(|rules| css_rule_list_length(scope, rules))
        .unwrap_or(0);
    if index >= rules_len {
        webidl::throw_index_size_error(scope);
        return;
    };
    let Some(rules) = rules else {
        webidl::throw_index_size_error(scope);
        return;
    };
    if let Err(error) = apply_stylo_top_level_delete_rule_mutation(scope, this, rules, index) {
        throw_delete_rule_error(scope, error);
        return;
    }
    sync_css_style_sheet_change(scope, this);
    rv.set_undefined();
}

pub(crate) fn css_style_sheet_remove_rule_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "CSSStyleSheet", "removeRule") {
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssStyleSheetRemoveRuleArgs>(scope, &args) else {
        return;
    };
    let this = args.this();
    let rules = css_style_sheet_rules_array(scope, this);
    if parsed.index >= css_rule_list_length(scope, rules) {
        webidl::throw_index_size_error(scope);
        return;
    }
    if let Err(error) = apply_stylo_top_level_delete_rule_mutation(scope, this, rules, parsed.index)
    {
        throw_delete_rule_error(scope, error);
        return;
    }
    sync_css_style_sheet_change(scope, this);
    rv.set_undefined();
}

pub(crate) fn parent_style_sheet_current_rule_texts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
) -> Vec<String> {
    // Local detached-state refresh happens after wrapper slots were just updated;
    // do not consult a still-valid live stylesheet here.
    parent_style_sheet
        .and_then(|sheet| get_private_value(scope, sheet, CSS_STYLE_SHEET_RULES_SLOT))
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .map(|rules| css_rule_list_current_css_texts(scope, rules))
        .unwrap_or_default()
}

pub(crate) fn css_rule_path_in_parent_style_sheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    parent_style_sheet: v8::Local<'s, v8::Object>,
) -> Option<Vec<usize>> {
    // parentStyleSheet remains observable on wrappers retired by replaceSync().
    // The native lease is the authority for whether this rule still belongs to
    // the current Stylo tree and for its path within that tree.
    css_rule_attached_native_path(scope, rule, parent_style_sheet)
}

pub(crate) fn css_style_sheet_add_rule_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "CSSStyleSheet", "addRule") {
        return;
    }
    let selector = css_style_sheet_add_rule_string_arg(scope, &args, 0);
    let style = if args.length() > 1 {
        css_style_sheet_add_rule_string_arg(scope, &args, 1)
    } else {
        String::new()
    };
    let this = args.this();
    let rules = css_style_sheet_rules_array(scope, this);
    let index = if args.length() > 2 && !args.get(2).is_undefined() {
        args.get(2).uint32_value(scope).unwrap_or(u32::MAX)
    } else {
        css_rule_list_length(scope, rules)
    };
    if index > css_rule_list_length(scope, rules) {
        webidl::throw_index_size_error(scope);
        return;
    }
    let rule = if style.is_empty() {
        format!("{selector} {{ }}")
    } else {
        format!("{selector} {{ {style} }}")
    };
    match apply_stylo_top_level_insert_rule_mutation(scope, this, rules, &rule, index) {
        Ok(()) => rv.set(v8::Integer::new(scope, -1).into()),
        Err(error) => throw_insert_rule_error(scope, error),
    }
}

pub(crate) fn css_style_sheet_add_rule_string_arg<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
    index: i32,
) -> String {
    if args.length() <= index {
        return "undefined".to_owned();
    }
    args.get(index)
        .to_string(scope)
        .map(|value| value.to_rust_string_lossy(scope))
        .unwrap_or_else(|| "undefined".to_owned())
}

pub(crate) fn css_style_sheet_selector_namespace_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> CssomSelectorNamespaceContext {
    let Some(stylesheet) = css_style_sheet_live_stylesheet(scope, sheet) else {
        return CssomSelectorNamespaceContext::default();
    };
    let namespaces = stylesheet.selector_namespaces();
    CssomSelectorNamespaceContext {
        default_namespace_uri: namespaces.default_namespace_uri,
        namespace_prefixes: namespaces.namespace_prefixes,
    }
}

pub(crate) fn sync_document_adopted_style_sheet_installations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    style_document: crate::document_runtime::DomHandle,
    sheets: v8::Local<'s, v8::Object>,
) {
    if unsafe { &*host_ptr }
        .dom_host()
        .node(style_document)
        .is_none()
    {
        return;
    }
    let installations = adopted_style_sheet_installations_from_value(scope, sheets.into());
    unsafe { &mut *host_ptr }
        .set_document_adopted_style_sheet_installations(style_document, installations);
    crate::native_bridge::document::sync_document_fonts_for_handle(
        scope,
        unsafe { &*host_ptr },
        style_document,
    );
}

pub(crate) fn sync_shadow_root_adopted_style_sheet_installations<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    host_ptr: *mut crate::native_bridge::JsContextHost,
    root: crate::document_runtime::DomHandle,
    sheets: v8::Local<'s, v8::Object>,
) {
    if unsafe { &*host_ptr }.dom_host().node(root).is_none() {
        return;
    }
    let installations = adopted_style_sheet_installations_from_value(scope, sheets.into());
    unsafe { &mut *host_ptr }
        .set_shadow_root_adopted_style_sheet_installations(root, installations);
}

pub(crate) fn sync_css_style_sheet_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) {
    notify_live_css_style_sheet_change(scope, sheet);
}

fn notify_live_css_style_sheet_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) {
    let stylesheet = require_css_style_sheet_live_stylesheet(scope, sheet);
    if css_style_sheet_is_constructed(scope, sheet) {
        // Constructed sheets have no owner input text. Their adopters consume
        // the same parsed stylesheet and only need a generation refresh.
        sync_adopted_style_sheet_installations_for_sheet(scope, sheet);
        return;
    }
    if let Some(parent_sheet) = get_private_value(scope, sheet, CSS_STYLE_SHEET_OWNER_RULE_SLOT)
        .and_then(|owner_rule| v8::Local::<v8::Object>::try_from(owner_rule).ok())
        .and_then(|owner_rule| {
            get_private_value(scope, owner_rule, CSS_RULE_PARENT_STYLE_SHEET_SLOT)
        })
        .and_then(|parent_sheet| v8::Local::<v8::Object>::try_from(parent_sheet).ok())
    {
        notify_live_css_style_sheet_change(scope, parent_sheet);
        return;
    }
    let Some(owner) = css_style_sheet_owner_node(scope, sheet) else {
        return;
    };
    let Ok((host_ptr, owner)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    unsafe { &mut *host_ptr }.note_owner_live_stylesheet_mutation(owner, stylesheet.id());
}

pub(crate) fn sync_adopted_style_sheet_installations_for_sheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) {
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        return;
    };
    for owner in css_style_sheet_adopted_owners(scope, sheet) {
        match owner.key {
            CssStyleSheetAdoptedOwnerKey::Document(document) => {
                sync_document_adopted_style_sheet_installations(
                    scope,
                    host_ptr,
                    document,
                    owner.array,
                );
            }
            CssStyleSheetAdoptedOwnerKey::ShadowRoot(root) => {
                sync_shadow_root_adopted_style_sheet_installations(
                    scope,
                    host_ptr,
                    root,
                    owner.array,
                );
            }
        }
    }
}

pub(crate) fn css_style_sheet_replace_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !moli_webapi_declare::implements_interface(scope, args.this(), "CSSStyleSheet") {
        let promise = rejected_type_error_promise(
            scope,
            "Failed to execute 'replace' on 'CSSStyleSheet': Illegal invocation.",
        );
        rv.set(promise.unwrap_or_else(|| v8::undefined(scope).into()));
        return;
    }
    if args.length() == 0 {
        let promise = rejected_type_error_promise(scope, "CSSStyleSheet.replace requires a text");
        rv.set(promise.unwrap_or_else(|| v8::undefined(scope).into()));
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssStyleSheetReplaceArgs>(scope, &args) else {
        return;
    };
    if !css_style_sheet_is_constructed(scope, args.this()) {
        let promise = rejected_css_style_sheet_not_allowed_promise(scope);
        rv.set(promise.unwrap_or_else(|| v8::undefined(scope).into()));
        return;
    }
    sync_constructed_css_style_sheet_rules_from_text(scope, args.this(), &parsed.text);
    sync_css_style_sheet_change(scope, args.this());
    match resolved_promise(scope, args.this().into()) {
        Some(promise) => rv.set(v8::Local::<v8::Value>::from(promise)),
        None => rv.set(v8::undefined(scope).into()),
    }
}

pub(crate) fn css_style_sheet_replace_sync_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_style_sheet_object(scope, args.this(), "CSSStyleSheet", "replaceSync") {
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssStyleSheetReplaceArgs>(scope, &args) else {
        return;
    };
    if !css_style_sheet_is_constructed(scope, args.this()) {
        webidl::throw_dom_exception(
            scope,
            "NotAllowedError",
            "Cannot replace rules on a non-constructed CSSStyleSheet.",
        );
        return;
    }
    sync_constructed_css_style_sheet_rules_from_text(scope, args.this(), &parsed.text);
    sync_css_style_sheet_change(scope, args.this());
    rv.set_undefined();
}

pub(crate) fn css_style_sheet_is_constructed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> bool {
    get_private_value(scope, sheet, CSS_STYLE_SHEET_CONSTRUCTED_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
}

pub(crate) fn css_style_sheet_constructor_document_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    get_private_value(
        scope,
        sheet,
        CSS_STYLE_SHEET_CONSTRUCTOR_DOCUMENT_HANDLE_SLOT,
    )
    .and_then(|value| dom_handle_from_marker_value(scope, value))
}

pub(crate) fn css_style_sheet_base_url<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> Option<url::Url> {
    get_private_value(scope, sheet, CSS_STYLE_SHEET_BASE_URL_SLOT)
        .and_then(|value| value.to_rust_string_lossy(scope).parse().ok())
}

pub(crate) fn ensure_css_style_sheet_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    interface: &'static str,
    member: &'static str,
) -> bool {
    if moli_webapi_declare::implements_interface(scope, object, "CSSStyleSheet") {
        return true;
    }
    throw_type_error(
        scope,
        &format!("Failed to get '{member}' on '{interface}': Illegal invocation."),
    );
    false
}

pub(crate) fn rejected_css_style_sheet_not_allowed_promise<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> Option<v8::Local<'s, v8::Value>> {
    let resolver = v8::PromiseResolver::new(scope)?;
    let promise = resolver.get_promise(scope);
    let reason = moli_v8_util::dom_exception_value(
        scope,
        "NotAllowedError",
        "Cannot replace rules on a non-constructed CSSStyleSheet.",
    );
    let _ = resolver.reject(scope, reason);
    Some(promise.into())
}

pub(crate) fn style_sheet_list_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_style_sheet_list_object(scope, args.this(), "item") {
        return;
    }
    let Some(parsed) = webidl::parse_args::<StyleSheetListItemArgs>(scope, &args) else {
        return;
    };
    if let Some(value) = args.this().get_index(scope, parsed.index)
        && !value.is_undefined()
    {
        rv.set(value);
    } else {
        rv.set(v8::null(scope).into());
    }
}

pub(crate) fn style_sheet_list_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_style_sheet_list_object(scope, args.this(), "length") {
        return;
    }
    let length = style_sheet_list_length(scope, args.this());
    rv.set(v8::Integer::new_from_unsigned(scope, length).into());
}

pub(crate) fn ensure_style_sheet_list_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> bool {
    if moli_webapi_declare::implements_interface(scope, object, "StyleSheetList") {
        return true;
    }
    throw_type_error(
        scope,
        &format!("Failed to get '{member}' on 'StyleSheetList': Illegal invocation."),
    );
    false
}

pub(crate) fn style_sheet_list_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> u32 {
    private_u32(scope, list, STYLE_SHEET_LIST_LENGTH_SLOT).unwrap_or(0)
}

pub(crate) fn new_style_sheet_list_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    StyleSheetListDeclaration { length: () }
        .bind(scope)
        .expect("StyleSheetList declaration should bind")
}

pub(crate) fn set_style_sheet_list_contents<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    values: &[v8::Local<'s, v8::Value>],
) {
    let old_length = style_sheet_list_length(scope, list);
    for (index, value) in values.iter().enumerate() {
        let _ = list.set_index(scope, index as u32, *value);
    }
    for index in values.len() as u32..old_length {
        let _ = list.delete_index(scope, index);
    }
    set_private_u32(
        scope,
        list,
        STYLE_SHEET_LIST_LENGTH_SLOT,
        values.len() as u32,
    );
}

pub(crate) fn initialize_css_style_sheet_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
) {
    let rules = get_private_value(scope, object, CSS_STYLE_SHEET_RULES_SLOT)
        .unwrap_or_else(|| new_css_rule_list_object(scope).into());
    CssStyleSheetDeclaration::new(rules)
        .bind_into(scope, object)
        .expect("CSSStyleSheet declaration should bind into object");
}

pub(crate) fn css_style_sheet_owner_node<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, sheet, CSS_STYLE_SHEET_OWNER_NODE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(crate) fn css_rule_parent_style_sheet_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSRule", "parentStyleSheet") {
        return;
    }
    rv.set(
        get_private_value(scope, args.this(), CSS_RULE_PARENT_STYLE_SHEET_SLOT)
            .unwrap_or_else(|| v8::null(scope).into()),
    );
}

pub(crate) fn sync_style_sheet_media_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    media_text: &str,
) {
    let media_text = normalize_media_query_list(media_text);
    if let Some(list) = get_private_value(scope, sheet, CSS_STYLE_SHEET_MEDIA_LIST_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_media_list_from_text(scope, list, &media_text, false);
    }
    let change = require_css_style_sheet_live_stylesheet(scope, sheet).set_media_text(&media_text);
    notify_css_style_sheet_runtime_state_change(scope, sheet, change);
}

pub(crate) fn sync_css_style_sheet_media_list_from_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    media_text: &str,
) {
    let media_text = normalize_media_query_list(media_text);
    if let Some(list) = get_private_value(scope, sheet, CSS_STYLE_SHEET_MEDIA_LIST_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        set_media_list_from_text(scope, list, &media_text, false);
    }
}

fn notify_css_style_sheet_runtime_state_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    sheet: v8::Local<'s, v8::Object>,
    change: crate::live_stylesheet::LiveStylesheetRuntimeStateChange,
) {
    if !change.affects_cascade() {
        return;
    }
    if css_style_sheet_is_constructed(scope, sheet) {
        sync_adopted_style_sheet_installations_for_sheet(scope, sheet);
        return;
    }
    let Some(owner) = css_style_sheet_owner_node(scope, sheet) else {
        return;
    };
    let Ok((host_ptr, owner)) =
        crate::native_bridge::node_runtime_and_handle_from_object(scope, owner)
    else {
        return;
    };
    let stylesheet_id = require_css_style_sheet_live_stylesheet(scope, sheet).id();
    unsafe { &mut *host_ptr }.note_owner_live_stylesheet_runtime_state_change(owner, stylesheet_id);
}

pub(crate) fn css_import_rule_style_sheet_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSImportRule", "styleSheet") {
        return;
    }
    if let Some(sheet) = get_private_value(scope, args.this(), CSS_IMPORT_RULE_STYLE_SHEET_SLOT) {
        rv.set(sheet);
        return;
    }

    let Some(parent_sheet) =
        get_private_value(scope, args.this(), CSS_RULE_PARENT_STYLE_SHEET_SLOT)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(parent_stylesheet_id) = css_style_sheet_id(scope, parent_sheet) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(rule_lease_id) =
        private_u64(scope, args.this(), CSS_RULE_NATIVE_WRAPPER_LEASE_ID_SLOT)
            .and_then(crate::live_stylesheet::StylesheetRuleWrapperLeaseId::from_raw)
    else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(host_ptr) = context_host_ptr_from_global_bridge(scope) else {
        rv.set(v8::null(scope).into());
        return;
    };
    let Some(stylesheet) = (unsafe { &*host_ptr })
        .imported_live_stylesheet_for_rule_wrapper(rule_lease_id, parent_stylesheet_id)
    else {
        rv.set(v8::null(scope).into());
        return;
    };

    let sheet = new_css_style_sheet_object(scope);
    bind_css_style_sheet_to_live_stylesheet(scope, sheet, stylesheet.clone());
    set_css_style_sheet_href(scope, sheet, stylesheet.base_url());
    set_css_style_sheet_origin_clean(scope, sheet, stylesheet.origin_clean());
    sync_css_style_sheet_media_list_from_owner(scope, sheet, &stylesheet.media_text());
    set_private_value(
        scope,
        sheet,
        CSS_STYLE_SHEET_OWNER_RULE_SLOT,
        args.this().into(),
    );
    set_private_value(
        scope,
        args.this(),
        CSS_IMPORT_RULE_STYLE_SHEET_SLOT,
        sheet.into(),
    );
    rv.set(sheet.into());
}
