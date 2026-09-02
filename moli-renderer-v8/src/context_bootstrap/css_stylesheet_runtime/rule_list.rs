use super::*;
use moli_webapi_declare::DataPropertyDescriptorDeclaration;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CssRuleListMaterializedTraversalMetrics {
    pub traversals: usize,
    pub entries: usize,
}

#[cfg(test)]
thread_local! {
    static CSS_RULE_LIST_MATERIALIZED_TRAVERSAL_METRICS: std::cell::Cell<CssRuleListMaterializedTraversalMetrics> =
        const { std::cell::Cell::new(CssRuleListMaterializedTraversalMetrics {
            traversals: 0,
            entries: 0,
        }) };
}

#[cfg(test)]
pub(crate) fn reset_css_rule_list_materialized_traversal_metrics_for_test() {
    CSS_RULE_LIST_MATERIALIZED_TRAVERSAL_METRICS
        .with(|metrics| metrics.set(CssRuleListMaterializedTraversalMetrics::default()));
}

#[cfg(test)]
pub(crate) fn css_rule_list_materialized_traversal_metrics_for_test()
-> CssRuleListMaterializedTraversalMetrics {
    CSS_RULE_LIST_MATERIALIZED_TRAVERSAL_METRICS.with(std::cell::Cell::get)
}

#[cfg(test)]
fn note_css_rule_list_materialized_traversal(entry_count: usize) {
    CSS_RULE_LIST_MATERIALIZED_TRAVERSAL_METRICS.with(|metrics| {
        let current = metrics.get();
        metrics.set(CssRuleListMaterializedTraversalMetrics {
            traversals: current.traversals + 1,
            entries: current.entries + entry_count,
        });
    });
}

#[cfg(not(test))]
fn note_css_rule_list_materialized_traversal(_: usize) {}

pub(crate) fn install_css_rule_list_indexed_property_handler<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    configure_css_rule_list_indexed_property_handler(template.instance_template(scope));
}

fn configure_css_rule_list_indexed_property_handler(template: v8::Local<'_, v8::ObjectTemplate>) {
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(css_rule_list_indexed_getter)
            .setter(css_rule_list_indexed_setter)
            .query(css_rule_list_indexed_query)
            .deleter(css_rule_list_indexed_deleter)
            .enumerator(css_rule_list_indexed_enumerator)
            .definer(css_rule_list_indexed_definer)
            .descriptor(css_rule_list_indexed_descriptor),
    );
}

pub(crate) fn install_css_keyframes_rule_indexed_property_handler<'s>(
    scope: &mut v8::PinScope<'s, '_, ()>,
    template: v8::Local<'s, v8::FunctionTemplate>,
) {
    configure_css_keyframes_rule_indexed_property_handler(template.instance_template(scope));
}

fn configure_css_keyframes_rule_indexed_property_handler(
    template: v8::Local<'_, v8::ObjectTemplate>,
) {
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(css_keyframes_rule_indexed_getter)
            .setter(css_keyframes_rule_indexed_setter)
            .query(css_keyframes_rule_indexed_query)
            .deleter(css_keyframes_rule_indexed_deleter)
            .enumerator(css_keyframes_rule_indexed_enumerator)
            .definer(css_keyframes_rule_indexed_definer)
            .descriptor(css_keyframes_rule_indexed_descriptor),
    );
}

pub(crate) fn new_css_keyframes_rule_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let template = v8::ObjectTemplate::new(scope);
    configure_css_keyframes_rule_indexed_property_handler(template);
    template
        .new_instance(scope)
        .expect("CSSKeyframesRule object template should instantiate")
}

pub(crate) fn new_css_rule_list_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
) -> v8::Local<'s, v8::Object> {
    let template = v8::ObjectTemplate::new(scope);
    configure_css_rule_list_indexed_property_handler(template);
    let list = template
        .new_instance(scope)
        .expect("CSSRuleList object template should instantiate");
    CssRuleListDeclaration {
        brand: (),
        length: 0,
    }
    .bind_into(scope, list)
    .expect("CSSRuleList declaration should bind into list");
    list
}

pub(crate) fn css_rule_list_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> u32 {
    private_u32(scope, list, CSS_RULE_LIST_LENGTH_SLOT).unwrap_or(0)
}

pub(crate) fn set_css_rule_list_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    length: u32,
) {
    set_private_u32(scope, list, CSS_RULE_LIST_LENGTH_SLOT, length);
}

pub(crate) fn bind_css_rule_list_to_parent<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
) {
    set_optional_private_object(
        scope,
        list,
        CSS_RULE_LIST_PARENT_STYLE_SHEET_SLOT,
        parent_style_sheet,
    );
    set_optional_private_object(scope, list, CSS_RULE_LIST_PARENT_RULE_SLOT, parent_rule);
}

pub(crate) fn initialize_attached_css_rule_list<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    parent_style_sheet: v8::Local<'s, v8::Object>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
    length: usize,
) {
    bind_css_rule_list_to_parent(scope, list, Some(parent_style_sheet), parent_rule);
    clear_css_rule_list_detached_snapshots(scope, list);
    reset_css_rule_list_materialized_items(scope, list);
    set_css_rule_list_length(scope, list, length.min(u32::MAX as usize) as u32);
}

pub(crate) fn bind_css_rule_list_to_detached_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
    snapshots: &[CssRuleSnapshot],
) {
    set_css_rule_list_detached_snapshots(scope, list, snapshots);
    let snapshots = css_rule_list_detached_snapshot_array(scope, list)
        .expect("detached CSSRuleList snapshot backing should initialize");
    bind_css_rule_list_to_detached_snapshot_array(
        scope,
        list,
        parent_style_sheet,
        parent_rule,
        snapshots,
    );
}

pub(crate) fn bind_css_rule_list_to_detached_snapshot_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: Option<v8::Local<'s, v8::Object>>,
    snapshots: v8::Local<'s, v8::Array>,
) {
    bind_css_rule_list_to_parent(scope, list, parent_style_sheet, parent_rule);
    set_css_rule_list_detached_snapshot_array(scope, list, snapshots);
    if let Some(parent_rule) = parent_rule {
        set_detached_css_rule_child_snapshot_array(scope, parent_rule, snapshots);
    }
    set_css_rule_list_length(scope, list, snapshots.length());
}

pub(crate) fn initialize_detached_css_rule_list_from_parent_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    parent_rule: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(snapshots) = detached_css_rule_child_snapshot_array(scope, parent_rule) else {
        return false;
    };
    bind_css_rule_list_to_detached_snapshot_array(
        scope,
        list,
        parent_style_sheet,
        Some(parent_rule),
        snapshots,
    );
    true
}

fn set_optional_private_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
    value: Option<v8::Local<'s, v8::Object>>,
) {
    let value = value
        .map(v8::Local::<v8::Value>::from)
        .unwrap_or_else(|| v8::null(scope).into());
    set_private_value(scope, object, slot, value);
}

fn css_rule_list_parent_style_sheet<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, list, CSS_RULE_LIST_PARENT_STYLE_SHEET_SLOT)
}

fn css_rule_list_parent_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_object(scope, list, CSS_RULE_LIST_PARENT_RULE_SLOT)
}

fn css_rule_list_materialized_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Map>> {
    get_private_value(scope, list, CSS_RULE_LIST_MATERIALIZED_ITEMS_SLOT)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
}

fn ensure_css_rule_list_materialized_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Map> {
    if let Some(items) = css_rule_list_materialized_items(scope, list) {
        return items;
    }

    let items = v8::Map::new(scope);
    set_private_value(
        scope,
        list,
        CSS_RULE_LIST_MATERIALIZED_ITEMS_SLOT,
        items.into(),
    );
    items
}

pub(crate) fn reset_css_rule_list_materialized_items<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) {
    if css_rule_list_materialized_items(scope, list).is_some() {
        set_private_value(
            scope,
            list,
            CSS_RULE_LIST_MATERIALIZED_ITEMS_SLOT,
            v8::null(scope).into(),
        );
    }
}

pub(crate) fn css_rule_list_materialized_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let items = css_rule_list_materialized_items(scope, list)?;
    let key = v8::Integer::new_from_unsigned(scope, index);
    items
        .get(scope, key.into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

pub(crate) fn css_rule_list_materialized_entries<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Vec<(u32, v8::Local<'s, v8::Object>)> {
    let Some(items) = css_rule_list_materialized_items(scope, list) else {
        return Vec::new();
    };
    let item_count = items.size();
    note_css_rule_list_materialized_traversal(item_count);
    let flattened = items.as_array(scope);
    debug_assert_eq!(flattened.length() as usize, item_count.saturating_mul(2));
    let mut entries = Vec::with_capacity(item_count);
    for entry_index in 0..item_count {
        let key_index = (entry_index * 2) as u32;
        let value_index = key_index + 1;
        let Some(index) = flattened
            .get_index(scope, key_index)
            .and_then(|key| key.uint32_value(scope))
        else {
            continue;
        };
        if let Some(rule) = flattened
            .get_index(scope, value_index)
            .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        {
            entries.push((index, rule));
        }
    }
    entries.sort_unstable_by_key(|(index, _)| *index);
    entries
}

pub(crate) fn set_css_rule_list_materialized_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    index: u32,
    rule: v8::Local<'s, v8::Object>,
) {
    let items = ensure_css_rule_list_materialized_items(scope, list);
    let key = v8::Integer::new_from_unsigned(scope, index);
    let _ = items.set(scope, key.into(), rule.into());
}

pub(crate) fn insert_css_rule_list_unmaterialized_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    index: u32,
) {
    let length = css_rule_list_length(scope, list);
    let mut entries = css_rule_list_materialized_entries(scope, list);
    entries.retain(|(existing_index, _)| *existing_index >= index);
    for (existing_index, rule) in entries.into_iter().rev() {
        set_css_rule_list_materialized_rule(scope, list, existing_index + 1, rule);
        delete_css_rule_list_materialized_rule(scope, list, existing_index);
    }
    set_css_rule_list_length(scope, list, length + 1);
}

pub(crate) fn delete_css_rule_list_materialized_rule<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    index: u32,
) {
    if let Some(items) = css_rule_list_materialized_items(scope, list) {
        let key = v8::Integer::new_from_unsigned(scope, index);
        let _ = items.delete(scope, key.into());
    }
}

pub(crate) fn css_rule_list_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let creation_context = list.get_creation_context(scope)?;
    if creation_context == scope.get_current_context() {
        return css_rule_list_item_in_current_context(scope, list, index);
    }

    let rule = {
        let target_scope = &mut v8::ContextScope::new(scope, creation_context);
        let rule = css_rule_list_item_in_current_context(target_scope, list, index)?;
        v8::Global::new(target_scope, rule)
    };
    Some(v8::Local::new(scope, &rule))
}

fn css_rule_list_item_in_current_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    if index >= css_rule_list_length(scope, list) {
        return None;
    }
    if let Some(rule) = css_rule_list_materialized_rule(scope, list, index) {
        return Some(rule);
    }

    let parent_rule = css_rule_list_parent_rule(scope, list);
    if let Some(entry) = css_rule_list_detached_snapshot_at(scope, list, index) {
        let parent_style_sheet = css_rule_list_parent_style_sheet(scope, list);
        let selector_context = parent_style_sheet
            .map(|sheet| css_style_sheet_selector_namespace_context(scope, sheet))
            .unwrap_or_default();
        let style_rule_context =
            css_style_rule_selector_context_for_parent_rule(scope, parent_rule);
        let rule = build_detached_css_rule_object_from_snapshot(
            scope,
            &entry.snapshot,
            entry.child_snapshots,
            parent_style_sheet,
            parent_rule,
            &selector_context,
            style_rule_context,
        );
        set_css_rule_list_materialized_rule(scope, list, index, rule);
        return Some(rule);
    }

    let parent_style_sheet = css_rule_list_parent_style_sheet(scope, list)?;
    let path = css_rule_list_item_path_in_parent_style_sheet(
        scope,
        Some(parent_style_sheet),
        parent_rule,
        index as usize,
    )?;
    let rule = build_css_rule_object_from_live_stylesheet_rule_path(
        scope,
        parent_style_sheet,
        parent_rule,
        &path,
    )?;
    if !bind_css_rule_object_to_native_stylesheet_rule(scope, rule, parent_style_sheet, path) {
        return None;
    }
    set_css_rule_list_materialized_rule(scope, list, index, rule);
    Some(rule)
}

pub(crate) fn ensure_css_rule_list_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    member: &'static str,
) -> bool {
    if get_private_value(scope, object, CSS_RULE_LIST_BRAND_SLOT).is_some() {
        return true;
    }
    throw_type_error(
        scope,
        &format!("Failed to get '{member}' on 'CSSRuleList': Illegal invocation."),
    );
    false
}

pub(crate) fn css_rule_list_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_list_object(scope, args.this(), "length") {
        return;
    }
    let length = css_rule_list_length(scope, args.this());
    rv.set(v8::Integer::new_from_unsigned(scope, length).into());
}

pub(crate) fn css_rule_list_item_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_list_object(scope, args.this(), "item") {
        return;
    }
    let Some(parsed) = webidl::parse_args::<CssRuleListItemArgs>(scope, &args) else {
        return;
    };
    match css_rule_list_item(scope, args.this(), parsed.index) {
        Some(rule) => rv.set(rule.into()),
        None => rv.set(v8::null(scope).into()),
    }
}

fn css_rule_list_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(rule) = css_rule_list_item(scope, args.holder(), index) else {
        return v8::Intercepted::kNo;
    };
    rv.set(rule.into());
    v8::Intercepted::kYes
}

fn css_rule_list_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if index >= css_rule_list_length(scope, args.holder()) {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
    v8::Intercepted::kYes
}

fn css_rule_list_indexed_setter(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _value: v8::Local<'_, v8::Value>,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn css_rule_list_indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    if index >= css_rule_list_length(scope, args.holder()) {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn css_rule_list_indexed_definer(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    // Web IDL indexed properties ignore defineProperty attempts while V8
    // reports the operation as successful, matching Chromium's binding.
    rv.set_bool(true);
    v8::Intercepted::kYes
}

fn css_rule_list_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let keys = (0..css_rule_list_length(scope, args.holder()))
        .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

fn css_rule_list_indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(rule) = css_rule_list_item(scope, args.holder(), index) else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) =
        DataPropertyDescriptorDeclaration::new(rule.into(), false, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}

fn css_keyframes_rule_item<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<v8::Local<'s, v8::Object>> {
    let rules = css_keyframes_rule_rules_array(scope, rule);
    css_rule_list_item(scope, rules, index)
}

fn css_keyframes_rule_length<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> u32 {
    if let Some((_, count)) = css_rule_live_stylesheet_child_rule_count(scope, rule) {
        return count.min(u32::MAX as usize) as u32;
    }
    let rules = css_keyframes_rule_rules_array(scope, rule);
    css_rule_list_length(scope, rules)
}

pub(crate) fn css_keyframes_rule_length_getter_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    if !ensure_css_rule_object(scope, args.this(), "CSSKeyframesRule", "length") {
        return;
    }
    let length = css_keyframes_rule_length(scope, args.this());
    rv.set(v8::Integer::new_from_unsigned(scope, length).into());
}

fn css_keyframes_rule_indexed_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(rule) = css_keyframes_rule_item(scope, args.holder(), index) else {
        return v8::Intercepted::kNo;
    };
    rv.set(rule.into());
    v8::Intercepted::kYes
}

fn css_keyframes_rule_indexed_query<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Integer>,
) -> v8::Intercepted {
    if index >= css_keyframes_rule_length(scope, args.holder()) {
        return v8::Intercepted::kNo;
    }
    rv.set_int32(v8::PropertyAttribute::READ_ONLY.as_u32() as i32);
    v8::Intercepted::kYes
}

fn css_keyframes_rule_indexed_setter(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _value: v8::Local<'_, v8::Value>,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn css_keyframes_rule_indexed_deleter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    if index >= css_keyframes_rule_length(scope, args.holder()) {
        return v8::Intercepted::kNo;
    }
    rv.set_bool(false);
    v8::Intercepted::kYes
}

fn css_keyframes_rule_indexed_definer(
    _scope: &mut v8::PinScope<'_, '_>,
    _index: u32,
    _descriptor: &v8::PropertyDescriptor,
    _args: v8::PropertyCallbackArguments<'_>,
    mut rv: v8::ReturnValue<'_, v8::Boolean>,
) -> v8::Intercepted {
    rv.set_bool(true);
    v8::Intercepted::kYes
}

fn css_keyframes_rule_indexed_enumerator<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Array>,
) {
    let keys = (0..css_keyframes_rule_length(scope, args.holder()))
        .map(|index| v8::Integer::new_from_unsigned(scope, index).into())
        .collect::<Vec<_>>();
    rv.set(v8::Array::new_with_elements(scope, &keys));
}

fn css_keyframes_rule_indexed_descriptor<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    index: u32,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) -> v8::Intercepted {
    let Some(rule) = css_keyframes_rule_item(scope, args.holder(), index) else {
        return v8::Intercepted::kNo;
    };
    let Ok(descriptor) =
        DataPropertyDescriptorDeclaration::new(rule.into(), false, true).bind(scope)
    else {
        return v8::Intercepted::kNo;
    };
    rv.set(descriptor.into());
    v8::Intercepted::kYes
}
