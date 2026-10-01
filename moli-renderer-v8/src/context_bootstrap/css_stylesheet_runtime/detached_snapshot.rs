use super::*;
use crate::live_stylesheet::{NativeCssDeclarationSource, RetainedStylesheetRuleSnapshot};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::atomic::{AtomicU64, Ordering},
};

const DECLARATION_SOURCE_ID_SLOT: &str = "__moliDetachedDeclarationSourceId";
const RULE_DECLARATION_SOURCE_SLOT: &str = "__moliCssRuleDetachedDeclarationSource";
static NEXT_DECLARATION_SOURCE_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static DECLARATION_SOURCES: RefCell<HashMap<u64, NativeCssDeclarationSource>> =
        RefCell::new(HashMap::new());
}

#[cfg(test)]
pub(crate) fn detached_declaration_source_count_for_test() -> usize {
    DECLARATION_SOURCES.with(|sources| sources.borrow().len())
}

const SNAPSHOT_RULE_TYPE_INDEX: u32 = 0;
const SNAPSHOT_CSS_TEXT_INDEX: u32 = 1;
const SNAPSHOT_PRELUDE_TEXT_INDEX: u32 = 2;
const SNAPSHOT_SELECTOR_TEXT_INDEX: u32 = 3;
const SNAPSHOT_DECLARATION_TEXT_INDEX: u32 = 4;
const SNAPSHOT_CHILDREN_INDEX: u32 = 5;
const SNAPSHOT_FIELD_COUNT: i32 = 6;
const CSS_RULE_SNAPSHOT_TYPE_SLOT: &str = "__moliCssRuleSnapshotType";

pub(crate) fn set_css_rule_snapshot_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    rule_type: Option<CssRuleType>,
) {
    set_private_u32(
        scope,
        rule,
        CSS_RULE_SNAPSHOT_TYPE_SLOT,
        rule_type.map(css_rule_type_snapshot_code).unwrap_or(0),
    );
}

pub(crate) fn css_rule_snapshot_type<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<CssRuleType> {
    private_u32(scope, rule, CSS_RULE_SNAPSHOT_TYPE_SLOT).and_then(css_rule_type_from_snapshot_code)
}

pub(crate) struct DetachedCssRuleSnapshotEntry<'s> {
    pub(crate) snapshot: CssRuleSnapshot,
    pub(crate) child_snapshots: v8::Local<'s, v8::Array>,
    pub(crate) declaration_source: Option<v8::Local<'s, v8::Object>>,
}

impl<'s> DetachedCssRuleSnapshotEntry<'s> {
    pub(crate) fn complete_snapshot(mut self, scope: &mut v8::PinScope<'s, '_>) -> CssRuleSnapshot {
        self.snapshot.child_rules = decode_rule_snapshots(scope, self.child_snapshots);
        self.snapshot
    }
}

/// Text is only the CSSOM presentation. Its empty pending-substitution
/// longhands cannot be parsed back into the original declarations.
pub(crate) fn encode_retained_css_rule_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    retained: RetainedStylesheetRuleSnapshot,
) -> DetachedCssRuleSnapshotEntry<'s> {
    let encoded = encode_rule_snapshot(scope, &retained.snapshot);
    let mut sources = retained.declaration_sources.into_iter();
    bind_snapshot_declaration_sources(scope, encoded, &mut sources);
    assert!(
        sources.next().is_none(),
        "native declaration sources must match the snapshot tree"
    );
    let entry = decode_rule_snapshot_entry(scope, encoded.into())
        .expect("a freshly encoded native snapshot is valid");
    DetachedCssRuleSnapshotEntry {
        snapshot: retained.snapshot,
        ..entry
    }
}

fn bind_snapshot_declaration_sources<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    encoded: v8::Local<'s, v8::Array>,
    sources: &mut std::vec::IntoIter<Option<NativeCssDeclarationSource>>,
) {
    if let Some(source) = sources
        .next()
        .expect("native declaration sources must cover every snapshot")
    {
        let id = NEXT_DECLARATION_SOURCE_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("detached declaration source id space exhausted");
        DECLARATION_SOURCES.with(|sources| sources.borrow_mut().insert(id, source));
        set_private_u64(scope, encoded.into(), DECLARATION_SOURCE_ID_SLOT, id);
        // The encoded snapshot itself is the lifetime token. A materialized
        // rule retains it too, so neither owner can invalidate the other.
        crate::v8_finalizer::track_context_owned_v8_finalizer(scope, encoded.into(), move || {
            DECLARATION_SOURCES.with(|sources| sources.borrow_mut().remove(&id));
        });
    }
    let children = encoded
        .get_index(scope, SNAPSHOT_CHILDREN_INDEX)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .expect("every encoded snapshot has a children array");
    for index in 0..children.length() {
        let child = children
            .get_index(scope, index)
            .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
            .expect("every encoded child is a snapshot array");
        bind_snapshot_declaration_sources(scope, child, sources);
    }
}

pub(crate) fn set_css_rule_detached_declaration_source<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    source: Option<v8::Local<'s, v8::Object>>,
) {
    let value = source
        .map(|source| source.into())
        .unwrap_or_else(|| v8::null(scope).into());
    set_private_value(scope, rule, RULE_DECLARATION_SOURCE_SLOT, value);
}

fn with_css_rule_detached_declaration_source<'s, T>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    read: impl FnOnce(&NativeCssDeclarationSource) -> T,
) -> Option<T> {
    let source = get_private_object(scope, rule, RULE_DECLARATION_SOURCE_SLOT)?;
    let id = private_u64(scope, source, DECLARATION_SOURCE_ID_SLOT)?;
    DECLARATION_SOURCES.with(|sources| sources.borrow().get(&id).map(read))
}

pub(crate) fn css_rule_detached_native_declaration_block<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<moli_css_parse::CssDeclarationBlock> {
    with_css_rule_detached_declaration_source(
        scope,
        rule,
        NativeCssDeclarationSource::declaration_block,
    )
}

pub(crate) fn css_rule_detached_native_declaration_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<String> {
    with_css_rule_detached_declaration_source(scope, rule, NativeCssDeclarationSource::css_text)
}

#[cfg(test)]
thread_local! {
    static DETACHED_CSS_RULE_SNAPSHOT_WRITE_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

#[cfg(test)]
pub(crate) fn reset_detached_css_rule_snapshot_write_count_for_test() {
    DETACHED_CSS_RULE_SNAPSHOT_WRITE_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(crate) fn detached_css_rule_snapshot_write_count_for_test() -> usize {
    DETACHED_CSS_RULE_SNAPSHOT_WRITE_COUNT.with(std::cell::Cell::get)
}

pub(crate) fn css_rule_detached_snapshot_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> String {
    private_string(scope, rule, CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT)
}

pub(crate) fn clear_css_rule_detached_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) {
    set_css_rule_snapshot_type(scope, rule, None);
    set_css_rule_detached_declaration_source(scope, rule, None);
    set_private_string(scope, rule, CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT, "");
    set_private_value(
        scope,
        rule,
        CSS_RULE_DETACHED_CHILD_SNAPSHOTS_SLOT,
        v8::null(scope).into(),
    );
}

pub(crate) fn set_detached_css_rule_snapshot_text<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    css_text: &str,
) -> bool {
    if css_rule_has_attached_native_binding(scope, rule) {
        return false;
    }
    #[cfg(test)]
    DETACHED_CSS_RULE_SNAPSHOT_WRITE_COUNT.with(|count| count.set(count.get() + 1));
    set_private_string(scope, rule, CSS_RULE_DETACHED_SNAPSHOT_TEXT_SLOT, css_text);
    true
}

pub(crate) fn set_detached_css_rule_child_snapshot_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
    snapshots: v8::Local<'s, v8::Array>,
) {
    set_private_value(
        scope,
        rule,
        CSS_RULE_DETACHED_CHILD_SNAPSHOTS_SLOT,
        snapshots.into(),
    );
}

pub(crate) fn detached_css_rule_child_snapshot_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    rule: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, rule, CSS_RULE_DETACHED_CHILD_SNAPSHOTS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(crate) fn set_css_rule_list_detached_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    snapshots: &[CssRuleSnapshot],
) {
    let snapshots = encode_rule_snapshots(scope, snapshots);
    set_css_rule_list_detached_snapshot_array(scope, list, snapshots);
}

pub(crate) fn set_css_rule_list_detached_snapshot_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    snapshots: v8::Local<'s, v8::Array>,
) {
    set_private_value(
        scope,
        list,
        CSS_RULE_LIST_DETACHED_SNAPSHOTS_SLOT,
        snapshots.into(),
    );
}

pub(crate) fn clear_css_rule_list_detached_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) {
    set_private_value(
        scope,
        list,
        CSS_RULE_LIST_DETACHED_SNAPSHOTS_SLOT,
        v8::null(scope).into(),
    );
}

pub(crate) fn css_rule_list_detached_snapshot_array<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Array>> {
    get_private_value(scope, list, CSS_RULE_LIST_DETACHED_SNAPSHOTS_SLOT)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
}

pub(crate) fn css_rule_list_detached_snapshot_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<DetachedCssRuleSnapshotEntry<'s>> {
    let snapshots = css_rule_list_detached_snapshot_array(scope, list)?;
    let encoded = snapshots.get_index(scope, index)?;
    decode_rule_snapshot_entry(scope, encoded)
}

pub(crate) fn css_rule_list_detached_snapshot_text_at<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    index: u32,
) -> Option<String> {
    css_rule_list_detached_snapshot_at(scope, list, index).map(|entry| entry.snapshot.css_text)
}

pub(crate) fn detached_css_rule_snapshot_array_texts<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshots: v8::Local<'s, v8::Array>,
) -> Vec<String> {
    let mut texts = Vec::with_capacity(snapshots.length() as usize);
    for index in 0..snapshots.length() {
        let Some(value) = snapshots.get_index(scope, index) else {
            continue;
        };
        if let Some(entry) = decode_rule_snapshot_entry(scope, value) {
            texts.push(entry.snapshot.css_text);
        }
    }
    texts
}

pub(crate) fn detached_css_rule_snapshot_array_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshots: v8::Local<'s, v8::Array>,
) -> Vec<CssRuleSnapshot> {
    decode_rule_snapshots(scope, snapshots)
}

fn encode_rule_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshots: &[CssRuleSnapshot],
) -> v8::Local<'s, v8::Array> {
    let array = v8::Array::new(scope, snapshots.len() as i32);
    for (index, snapshot) in snapshots.iter().enumerate() {
        let encoded = encode_rule_snapshot(scope, snapshot);
        let _ = array.set_index(scope, index as u32, encoded.into());
    }
    array
}

#[derive(Clone, Copy)]
pub(crate) enum DetachedCssRuleListChange {
    Insert(u32),
    Delete(u32),
}

pub(crate) fn encode_css_rule_list_snapshot_change<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    list: v8::Local<'s, v8::Object>,
    snapshots: &[CssRuleSnapshot],
    change: DetachedCssRuleListChange,
) -> v8::Local<'s, v8::Array> {
    let Some(previous) = css_rule_list_detached_snapshot_array(scope, list) else {
        return encode_rule_snapshots(scope, snapshots);
    };
    let expected_length = match change {
        DetachedCssRuleListChange::Insert(_) => previous.length() as usize + 1,
        DetachedCssRuleListChange::Delete(_) => previous.length() as usize - 1,
    };
    assert_eq!(snapshots.len(), expected_length);
    let array = v8::Array::new(scope, snapshots.len() as i32);
    for (index, snapshot) in snapshots.iter().enumerate() {
        let index = index as u32;
        let previous_index = match change {
            DetachedCssRuleListChange::Insert(at) if index == at => None,
            DetachedCssRuleListChange::Insert(at) => Some(index - u32::from(index > at)),
            DetachedCssRuleListChange::Delete(at) => Some(index + u32::from(index >= at)),
        };
        // Reuse the encoded object, including its native declaration source
        // and unread descendants. CSSOM text is not a reversible serialization.
        let encoded = if let Some(previous_index) = previous_index {
            previous
                .get_index(scope, previous_index)
                .expect("an unchanged detached rule must have a snapshot")
        } else {
            encode_rule_snapshot(scope, snapshot).into()
        };
        let _ = array.set_index(scope, index, encoded);
    }
    array
}

fn encode_rule_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshot: &CssRuleSnapshot,
) -> v8::Local<'s, v8::Array> {
    let encoded = v8::Array::new(scope, SNAPSHOT_FIELD_COUNT);
    let rule_type =
        v8::Integer::new_from_unsigned(scope, css_rule_type_snapshot_code(snapshot.rule_type));
    let css_text = v8_dynamic_string_value(scope, &snapshot.css_text);
    let prelude_text = optional_snapshot_string(scope, snapshot.prelude_text.as_deref());
    let selector_text = optional_snapshot_string(scope, snapshot.selector_text.as_deref());
    let declaration_text = optional_snapshot_string(scope, snapshot.declaration_text.as_deref());
    let children = encode_rule_snapshots(scope, &snapshot.child_rules);
    let _ = encoded.set_index(scope, SNAPSHOT_RULE_TYPE_INDEX, rule_type.into());
    let _ = encoded.set_index(scope, SNAPSHOT_CSS_TEXT_INDEX, css_text);
    let _ = encoded.set_index(scope, SNAPSHOT_PRELUDE_TEXT_INDEX, prelude_text);
    let _ = encoded.set_index(scope, SNAPSHOT_SELECTOR_TEXT_INDEX, selector_text);
    let _ = encoded.set_index(scope, SNAPSHOT_DECLARATION_TEXT_INDEX, declaration_text);
    let _ = encoded.set_index(scope, SNAPSHOT_CHILDREN_INDEX, children.into());
    encoded
}

fn optional_snapshot_string<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: Option<&str>,
) -> v8::Local<'s, v8::Value> {
    value
        .map(|value| v8_dynamic_string_value(scope, value))
        .unwrap_or_else(|| v8::null(scope).into())
}

fn decode_rule_snapshots<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    snapshots: v8::Local<'s, v8::Array>,
) -> Vec<CssRuleSnapshot> {
    let mut decoded = Vec::with_capacity(snapshots.length() as usize);
    for index in 0..snapshots.length() {
        let Some(value) = snapshots.get_index(scope, index) else {
            continue;
        };
        if let Some(snapshot) = decode_rule_snapshot(scope, value) {
            decoded.push(snapshot);
        }
    }
    decoded
}

fn decode_rule_snapshot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<CssRuleSnapshot> {
    let entry = decode_rule_snapshot_entry(scope, value)?;
    let mut snapshot = entry.snapshot;
    snapshot.child_rules = decode_rule_snapshots(scope, entry.child_snapshots);
    Some(snapshot)
}

fn decode_rule_snapshot_entry<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<DetachedCssRuleSnapshotEntry<'s>> {
    let encoded = v8::Local::<v8::Array>::try_from(value).ok()?;
    let rule_type = encoded
        .get_index(scope, SNAPSHOT_RULE_TYPE_INDEX)?
        .uint32_value(scope)
        .and_then(css_rule_type_from_snapshot_code)?;
    let css_text = snapshot_string(scope, encoded, SNAPSHOT_CSS_TEXT_INDEX)?;
    let prelude_text = optional_snapshot_string_value(scope, encoded, SNAPSHOT_PRELUDE_TEXT_INDEX);
    let selector_text =
        optional_snapshot_string_value(scope, encoded, SNAPSHOT_SELECTOR_TEXT_INDEX);
    let declaration_text =
        optional_snapshot_string_value(scope, encoded, SNAPSHOT_DECLARATION_TEXT_INDEX);
    let child_snapshots = encoded
        .get_index(scope, SNAPSHOT_CHILDREN_INDEX)
        .and_then(|value| v8::Local::<v8::Array>::try_from(value).ok())
        .unwrap_or_else(|| v8::Array::new(scope, 0));
    Some(DetachedCssRuleSnapshotEntry {
        snapshot: CssRuleSnapshot {
            rule_type,
            css_text,
            prelude_text,
            selector_text,
            declaration_text,
            child_rules: Vec::new(),
        },
        child_snapshots,
        declaration_source: private_u64(scope, encoded.into(), DECLARATION_SOURCE_ID_SLOT)
            .map(|_| encoded.into()),
    })
}

fn snapshot_string(
    scope: &mut v8::PinScope<'_, '_>,
    encoded: v8::Local<v8::Array>,
    index: u32,
) -> Option<String> {
    let value = encoded.get_index(scope, index)?;
    (!value.is_null_or_undefined()).then(|| value.to_rust_string_lossy(scope))
}

fn optional_snapshot_string_value(
    scope: &mut v8::PinScope<'_, '_>,
    encoded: v8::Local<v8::Array>,
    index: u32,
) -> Option<String> {
    snapshot_string(scope, encoded, index)
}

fn css_rule_type_from_snapshot_code(code: u32) -> Option<CssRuleType> {
    Some(match code {
        1 => CssRuleType::Style,
        3 => CssRuleType::Import,
        4 => CssRuleType::Media,
        5 => CssRuleType::FontFace,
        6 => CssRuleType::Page,
        7 => CssRuleType::Keyframes,
        8 => CssRuleType::Keyframe,
        9 => CssRuleType::Margin,
        10 => CssRuleType::Namespace,
        11 => CssRuleType::CounterStyle,
        12 => CssRuleType::Supports,
        13 => CssRuleType::Document,
        14 => CssRuleType::FontFeatureValues,
        16 => CssRuleType::LayerBlock,
        17 => CssRuleType::LayerStatement,
        18 => CssRuleType::Container,
        19 => CssRuleType::FontPaletteValues,
        20 => CssRuleType::Property,
        21 => CssRuleType::Scope,
        22 => CssRuleType::StartingStyle,
        23 => CssRuleType::PositionTry,
        24 => CssRuleType::NestedDeclarations,
        25 => CssRuleType::CustomMedia,
        26 => CssRuleType::AppearanceBase,
        27 => CssRuleType::ViewTransition,
        _ => return None,
    })
}

fn css_rule_type_snapshot_code(rule_type: CssRuleType) -> u32 {
    match rule_type {
        CssRuleType::Style => 1,
        CssRuleType::Import => 3,
        CssRuleType::Media => 4,
        CssRuleType::FontFace => 5,
        CssRuleType::Page => 6,
        CssRuleType::Keyframes => 7,
        CssRuleType::Keyframe => 8,
        CssRuleType::Margin => 9,
        CssRuleType::Namespace => 10,
        CssRuleType::CounterStyle => 11,
        CssRuleType::Supports => 12,
        CssRuleType::Document => 13,
        CssRuleType::FontFeatureValues => 14,
        CssRuleType::LayerBlock => 16,
        CssRuleType::LayerStatement => 17,
        CssRuleType::Container => 18,
        CssRuleType::FontPaletteValues => 19,
        CssRuleType::Property => 20,
        CssRuleType::Scope => 21,
        CssRuleType::StartingStyle => 22,
        CssRuleType::PositionTry => 23,
        CssRuleType::NestedDeclarations => 24,
        CssRuleType::CustomMedia => 25,
        CssRuleType::AppearanceBase => 26,
        CssRuleType::ViewTransition => 27,
    }
}
