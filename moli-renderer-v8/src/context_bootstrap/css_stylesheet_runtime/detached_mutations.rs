use super::*;

#[cfg(test)]
thread_local! {
    static DETACHED_RULE_MUTATION_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) fn reset_detached_rule_mutation_count_for_test() {
    DETACHED_RULE_MUTATION_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn detached_rule_mutation_count_for_test() -> usize {
    DETACHED_RULE_MUTATION_COUNT.with(std::cell::Cell::get)
}

fn record_detached_rule_mutation() {
    #[cfg(test)]
    DETACHED_RULE_MUTATION_COUNT.with(|count| count.set(count.get().saturating_add(1)));
}

fn apply_detached_rule_list_snapshot_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    snapshots: &[CssRuleSnapshot],
    change: DetachedCssRuleListChange,
) {
    let snapshots = encode_css_rule_list_snapshot_change(scope, rules, snapshots, change);
    match change {
        DetachedCssRuleListChange::Insert(index) => {
            insert_css_rule_list_unmaterialized_rule(scope, rules, index);
        }
        DetachedCssRuleListChange::Delete(index) => {
            delete_css_rule_list_rule(scope, rules, index);
        }
    }
    bind_css_rule_list_to_detached_snapshot_array(
        scope,
        rules,
        parent_style_sheet,
        Some(parent_rule),
        snapshots,
    );
}

pub(crate) fn apply_detached_nested_rule_insert_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
    rule_text: &str,
    index: u32,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
    containing_rule_type_bits: u32,
) -> Result<(), CssRuleInsertError> {
    record_detached_rule_mutation();
    let namespace_rule_texts = selector_context.stylo_parent_rule_texts();
    let existing_rule_texts = css_rule_list_css_texts(scope, rules);
    let mutation = insert_detached_nested_rule_with_stylo(
        &namespace_rule_texts,
        &existing_rule_texts,
        rule_text,
        index as usize,
        containing_rule_type_bits,
        stylo_parse_relative_rule_type(style_rule_context),
    )?;
    apply_detached_rule_list_snapshot_change(
        scope,
        parent_rule,
        rules,
        parent_style_sheet,
        &mutation.rules,
        DetachedCssRuleListChange::Insert(index),
    );
    Ok(())
}

pub(crate) fn apply_detached_nested_rule_delete_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
    index: u32,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
    selector_context: &CssomSelectorNamespaceContext,
    style_rule_context: StyleRuleSelectorContext,
    containing_rule_type_bits: u32,
) -> Result<(), CssRuleInsertError> {
    record_detached_rule_mutation();
    let namespace_rule_texts = selector_context.stylo_parent_rule_texts();
    let existing_rule_texts = css_rule_list_css_texts(scope, rules);
    let mutation = delete_detached_nested_rule_with_stylo(
        &namespace_rule_texts,
        &existing_rule_texts,
        index as usize,
        containing_rule_type_bits,
        stylo_parse_relative_rule_type(style_rule_context),
    )?;
    apply_detached_rule_list_snapshot_change(
        scope,
        parent_rule,
        rules,
        parent_style_sheet,
        &mutation.rules,
        DetachedCssRuleListChange::Delete(index),
    );
    Ok(())
}

pub(crate) fn apply_detached_keyframe_rule_insert_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
    rule_text: &str,
    index: u32,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
) -> Result<(), CssRuleInsertError> {
    record_detached_rule_mutation();
    let existing_rule_texts = css_rule_list_css_texts(scope, rules);
    let mutation =
        insert_detached_keyframe_rule_with_stylo(&existing_rule_texts, rule_text, index as usize)?;
    apply_detached_rule_list_snapshot_change(
        scope,
        parent_rule,
        rules,
        parent_style_sheet,
        &mutation.rules,
        DetachedCssRuleListChange::Insert(index),
    );
    Ok(())
}

pub(crate) fn apply_detached_keyframe_rule_delete_mutation<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    parent_rule: v8::Local<'s, v8::Object>,
    rules: v8::Local<'s, v8::Object>,
    index: u32,
    parent_style_sheet: Option<v8::Local<'s, v8::Object>>,
) -> Result<(), CssRuleInsertError> {
    record_detached_rule_mutation();
    let existing_rule_texts = css_rule_list_css_texts(scope, rules);
    let mutation = delete_detached_keyframe_rule_with_stylo(&existing_rule_texts, index as usize)?;
    apply_detached_rule_list_snapshot_change(
        scope,
        parent_rule,
        rules,
        parent_style_sheet,
        &mutation.rules,
        DetachedCssRuleListChange::Delete(index),
    );
    Ok(())
}
