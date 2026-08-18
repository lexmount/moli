use crate::util::{get_private_value, set_private_value, throw_type_error, v8_string, v8str};

use crate::document_runtime::DomHandle;
use crate::native_bridge::{
    BridgeHandle, ComputedStyleTargetKey, JsContextHost, bridge_handle_from_object,
};
use crate::style_engine::StyleViewport;
use crate::webidl;

use super::super::node::{
    node_is_element, node_runtime_and_handle_from_object,
    node_runtime_and_handle_from_object_or_detached,
};
mod accessors;
mod computed_names;
mod declaration;
mod methods;
mod named;

pub(crate) use accessors::{
    style_css_text_getter_callback, style_css_text_setter_callback, style_length_getter_callback,
};
pub(in crate::native_bridge::element) use declaration::style_base_url;
pub(crate) use declaration::{ComputedStyleRead, ComputedStyleReadScope};
use declaration::{
    StyleComputationContext, shorthand_longhands, style_property_count_with_context,
    style_property_value_with_context, style_runtime_and_handle_from_object,
};
pub(crate) use declaration::{
    StyleMode, active_css_animation_transform_value, css_animation_start_applies,
    cssom_style_entry_is_pdb_supplemental_side_entry, cssom_style_entry_requires_structured_parser,
    cssom_style_property_affected_names_with_pdb,
    cssom_style_property_mutation_affected_names_with_pdb,
    cssom_style_property_mutation_cleanup_names_with_pdb,
    cssom_style_property_uses_preferred_pdb_supplemental_entries,
    cssom_style_property_write_can_use_pdb_storage, cssom_text_decoration_line_value_is_compat,
    parse_cssom_style_property_entries_for_write, parse_cssom_style_property_entries_with_base,
    parse_inline_css_text_with_base, pdb_property_priority_for_cssom_query_with_side_entries,
    pdb_property_value_for_cssom_query_with_side_entries, raw_inline_style_property_value,
    serialize_animation_range_shorthand, serialize_animation_shorthand_from_longhands,
    serialize_transition_shorthand_from_longhands, set_pdb_block_property_collecting_entries,
    style_entries_css_text_with_pdb, style_entries_property_priority_with_pdb,
    style_entries_property_value_with_pdb, style_property_value,
};
pub(crate) use methods::{
    computed_style_property_names_from_object, computed_style_property_value_from_object,
    style_get_property_priority_callback, style_get_property_value_callback, style_item_callback,
    style_remove_property_callback, style_set_property_callback,
};
pub(crate) use named::{live_style_named_property_value, set_live_style_named_property_value};
use named::{
    style_indexed_enumerator, style_indexed_getter, style_indexed_query, style_named_enumerator,
    style_named_getter, style_named_query, style_named_setter,
};

pub(super) const STYLE_DECLARATION_BASE_URL_SLOT: &str = "__moliStyleDeclarationBaseUrl";
pub(crate) const STYLE_DECLARATION_VIEWPORT_WIDTH_SLOT: &str =
    "__moliStyleDeclarationViewportWidth";
pub(crate) const STYLE_DECLARATION_VIEWPORT_HEIGHT_SLOT: &str =
    "__moliStyleDeclarationViewportHeight";
pub(crate) const STYLE_DECLARATION_SCREEN_WIDTH_SLOT: &str = "__moliStyleDeclarationScreenWidth";
pub(crate) const STYLE_DECLARATION_SCREEN_HEIGHT_SLOT: &str = "__moliStyleDeclarationScreenHeight";
pub(crate) const STYLE_DECLARATION_FORCED_EMPTY_COMPUTED_SLOT: &str =
    "__moliStyleDeclarationForcedEmptyComputed";
pub(crate) const STYLE_DECLARATION_TARGET_EMPTY_COMPUTED_SLOT: &str =
    "__moliStyleDeclarationTargetEmptyComputed";
pub(crate) const STYLE_DECLARATION_TARGET_CONTEXT_EPOCH_SLOT: &str =
    "__moliStyleDeclarationTargetContextEpoch";
pub(crate) const STYLE_DECLARATION_PSEUDO_ELEMENT_SLOT: &str =
    "__moliStyleDeclarationPseudoElement";
pub(crate) const STYLE_DECLARATION_READ_DOCUMENT_SLOT: &str = "__moliStyleDeclarationReadDocument";
const STYLE_DECLARATION_COMPUTED_PROPERTY_COUNT_SLOT: &str =
    "__moliStyleDeclarationComputedPropertyCount";
const STYLE_DECLARATION_COMPUTED_PROPERTY_COUNT_GENERATION_SLOT: &str =
    "__moliStyleDeclarationComputedPropertyCountGeneration";
const STYLE_DECLARATION_COMPUTED_PROPERTY_COUNT_TARGET_EPOCH_SLOT: &str =
    "__moliStyleDeclarationComputedPropertyCountTargetEpoch";
const DETACHED_STYLE_SLOT: &str = "__lmDetachedStyle";

pub(crate) fn is_live_style_declaration_object(
    scope: &mut v8::PinScope<'_, '_>,
    style: v8::Local<'_, v8::Object>,
) -> bool {
    style_runtime_and_handle_from_object(scope, style).is_ok()
}

pub(crate) fn computed_style_property_is_shorthand(property: &str) -> bool {
    shorthand_longhands(property).is_some()
}

fn style_object_forces_empty_computed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    mode: StyleMode,
) -> bool {
    if mode != StyleMode::Computed {
        return false;
    }
    let Ok((runtime_ptr, bridge_handle)) = bridge_handle_from_object(scope, style) else {
        return true;
    };
    unsafe { &*runtime_ptr }.drain_pending_style_invalidations_for_computed_style_read();
    if get_private_value(scope, style, STYLE_DECLARATION_FORCED_EMPTY_COMPUTED_SLOT)
        .is_some_and(|value| value.boolean_value(scope))
    {
        return true;
    }
    if let Some(value) =
        get_private_value(scope, style, STYLE_DECLARATION_TARGET_EMPTY_COMPUTED_SLOT)
        && value.is_boolean()
        && target_context_epoch_is_current(scope, unsafe { &*runtime_ptr }, style)
    {
        return value.boolean_value(scope);
    }
    match bridge_handle {
        BridgeHandle::ComputedStyle(handle, descriptor) => match descriptor.target {
            ComputedStyleTargetKey::ChildFrame(frame_handle) => {
                let runtime = unsafe { &*runtime_ptr };
                let empty = !child_frame_target_document_is_current(runtime, handle, frame_handle)
                    || style_property_value(runtime, frame_handle, StyleMode::Computed, "display")
                        == "none";
                cache_target_empty_context(scope, runtime, style, empty);
                cache_frame_viewport(scope, runtime, style, frame_handle);
                empty
            }
            ComputedStyleTargetKey::DetachedIframe(frame_handle) => {
                !detached_iframe_target_document_is_current(
                    scope,
                    runtime_ptr,
                    style,
                    handle,
                    frame_handle,
                )
            }
            ComputedStyleTargetKey::PopupDocument(document_handle) => {
                !popup_document_target_is_current(unsafe { &*runtime_ptr }, handle, document_handle)
            }
            ComputedStyleTargetKey::Dynamic => {
                computed_style_target_context(scope, unsafe { &*runtime_ptr }, handle, None)
                    .returns_empty_style()
            }
        },
        _ => true,
    }
}

fn child_frame_target_document_is_current(
    runtime: &JsContextHost,
    target: DomHandle,
    frame_handle: DomHandle,
) -> bool {
    let target_document = runtime
        .dom_host()
        .node(target)
        .and_then(crate::dom::native::Node::owner_document);
    runtime
        .child_browsing_context_document_handle(frame_handle)
        .is_some_and(|document| Some(document) == target_document)
}

fn popup_document_target_is_current(
    runtime: &JsContextHost,
    target: DomHandle,
    popup_document: DomHandle,
) -> bool {
    let target_document = runtime
        .dom_host()
        .node(target)
        .and_then(crate::dom::native::Node::owner_document);
    target_document == Some(popup_document)
        && runtime
            .lightweight_popup_id_for_document_handle(popup_document)
            .is_some_and(|popup_id| runtime.lightweight_popup_is_open(popup_id))
}

fn detached_iframe_target_document_is_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut JsContextHost,
    style: v8::Local<'s, v8::Object>,
    target: DomHandle,
    frame_handle: DomHandle,
) -> bool {
    let runtime = unsafe { &*runtime_ptr };
    let read_document = style_object_read_document(scope, style);
    let owner_document = runtime
        .dom_host()
        .node(target)
        .and_then(crate::dom::native::Node::owner_document);
    let target_document = read_document.or(owner_document);
    crate::native_bridge::document::detached_iframe_current_content_document_handle(
        scope,
        runtime_ptr,
        frame_handle,
    )
    .is_some_and(|document| Some(document) == target_document)
}

fn target_context_epoch_is_current<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &JsContextHost,
    style: v8::Local<'s, v8::Object>,
) -> bool {
    let Some(value) = get_private_value(scope, style, STYLE_DECLARATION_TARGET_CONTEXT_EPOCH_SLOT)
    else {
        return false;
    };
    let Ok(value) = v8::Local::<v8::BigInt>::try_from(value) else {
        return false;
    };
    let (epoch, lossless) = value.u64_value();
    lossless && epoch == runtime.style_target_context_epoch()
}

fn cache_target_empty_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &JsContextHost,
    style: v8::Local<'s, v8::Object>,
    empty: bool,
) {
    let empty = v8::Boolean::new(scope, empty);
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_TARGET_EMPTY_COMPUTED_SLOT,
        empty.into(),
    );
    let epoch = v8::BigInt::new_from_u64(scope, runtime.style_target_context_epoch());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_TARGET_CONTEXT_EPOCH_SLOT,
        epoch.into(),
    );
}

fn cache_frame_viewport<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime: &JsContextHost,
    style: v8::Local<'s, v8::Object>,
    frame_handle: DomHandle,
) {
    let viewport = iframe_handle_viewport(runtime, frame_handle);
    let width = viewport
        .and_then(|viewport| viewport.width)
        .map(|width| v8::Number::new(scope, width).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(scope, style, STYLE_DECLARATION_VIEWPORT_WIDTH_SLOT, width);
    let height = viewport
        .and_then(|viewport| viewport.height)
        .map(|height| v8::Number::new(scope, height).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(scope, style, STYLE_DECLARATION_VIEWPORT_HEIGHT_SLOT, height);
    let screen_width = viewport
        .and_then(|viewport| viewport.screen_width)
        .map(|screen_width| v8::Number::new(scope, screen_width).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_SCREEN_WIDTH_SLOT,
        screen_width,
    );
    let screen_height = viewport
        .and_then(|viewport| viewport.screen_height)
        .map(|screen_height| v8::Number::new(scope, screen_height).into())
        .unwrap_or_else(|| v8::undefined(scope).into());
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_SCREEN_HEIGHT_SLOT,
        screen_height,
    );
}

fn style_object_pseudo_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    mode: StyleMode,
) -> Option<String> {
    if mode != StyleMode::Computed {
        return None;
    }
    get_private_value(scope, style, STYLE_DECLARATION_PSEUDO_ELEMENT_SLOT)
        .and_then(|value| v8::Local::<v8::String>::try_from(value).ok())
        .map(|value| value.to_rust_string_lossy(scope))
}

fn style_object_viewport_width<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    get_private_value(scope, style, STYLE_DECLARATION_VIEWPORT_WIDTH_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn style_object_viewport_height<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    get_private_value(scope, style, STYLE_DECLARATION_VIEWPORT_HEIGHT_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn style_object_screen_width<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    get_private_value(scope, style, STYLE_DECLARATION_SCREEN_WIDTH_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn style_object_screen_height<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<f64> {
    get_private_value(scope, style, STYLE_DECLARATION_SCREEN_HEIGHT_SLOT)
        .and_then(|value| value.number_value(scope))
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn style_object_viewport<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> StyleViewport {
    StyleViewport::new(
        style_object_viewport_width(scope, style),
        style_object_viewport_height(scope, style),
    )
    .with_screen_size(
        style_object_screen_width(scope, style),
        style_object_screen_height(scope, style),
    )
}

fn style_object_computation_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> StyleComputationContext {
    let read_document = style_object_read_document(scope, style);
    StyleComputationContext::new(style_object_viewport(scope, style))
        .with_read_document(read_document)
}

fn style_object_property_count_with_context<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    context: StyleComputationContext,
) -> usize {
    if mode != StyleMode::Computed {
        return style_property_count_with_context(runtime, handle, mode, context);
    }

    let generation = runtime.computed_style_cache_generation_for_handle(handle);
    let target_epoch = runtime.style_target_context_epoch();
    if private_u64_equals(
        scope,
        style,
        STYLE_DECLARATION_COMPUTED_PROPERTY_COUNT_GENERATION_SLOT,
        generation,
    ) && private_u64_equals(
        scope,
        style,
        STYLE_DECLARATION_COMPUTED_PROPERTY_COUNT_TARGET_EPOCH_SLOT,
        target_epoch,
    ) && let Some(count) =
        get_private_value(scope, style, STYLE_DECLARATION_COMPUTED_PROPERTY_COUNT_SLOT)
            .and_then(|value| value.uint32_value(scope))
    {
        return count as usize;
    }

    let count = style_property_count_with_context(runtime, handle, mode, context);
    let count_value = v8::Integer::new_from_unsigned(scope, count as u32);
    set_private_value(
        scope,
        style,
        STYLE_DECLARATION_COMPUTED_PROPERTY_COUNT_SLOT,
        count_value.into(),
    );
    cache_private_u64(
        scope,
        style,
        STYLE_DECLARATION_COMPUTED_PROPERTY_COUNT_GENERATION_SLOT,
        generation,
    );
    cache_private_u64(
        scope,
        style,
        STYLE_DECLARATION_COMPUTED_PROPERTY_COUNT_TARGET_EPOCH_SLOT,
        target_epoch,
    );
    count
}

fn private_u64_equals<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    expected: u64,
) -> bool {
    let Some(value) = get_private_value(scope, object, slot) else {
        return false;
    };
    let Ok(value) = v8::Local::<v8::BigInt>::try_from(value) else {
        return false;
    };
    let (value, lossless) = value.u64_value();
    lossless && value == expected
}

fn cache_private_u64<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &str,
    value: u64,
) {
    let value = v8::BigInt::new_from_u64(scope, value);
    set_private_value(scope, object, slot, value.into());
}

fn style_object_read_document<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    get_private_value(scope, style, STYLE_DECLARATION_READ_DOCUMENT_SLOT)
        .and_then(|value| value.uint32_value(scope))
        .map(|index| DomHandle::new(index as usize))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ComputedStyleTargetContext {
    ActiveDocument,
    ChildFrameDocument {
        frame_handle: DomHandle,
        rendered: bool,
    },
    EmptyForDetached,
}

impl ComputedStyleTargetContext {
    pub(crate) fn returns_empty_style(self) -> bool {
        match self {
            Self::ActiveDocument => false,
            Self::ChildFrameDocument { rendered, .. } => !rendered,
            Self::EmptyForDetached => true,
        }
    }

    pub(crate) fn viewport(self, host: &JsContextHost) -> StyleViewport {
        match self {
            Self::ChildFrameDocument { frame_handle, .. } => {
                iframe_handle_viewport(host, frame_handle).unwrap_or_default()
            }
            Self::ActiveDocument => host.style_viewport(),
            Self::EmptyForDetached => StyleViewport::default(),
        }
    }
}

pub(crate) fn computed_style_target_context(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    handle: DomHandle,
    requesting_child_window: Option<DomHandle>,
) -> ComputedStyleTargetContext {
    if let Some(frame_handle) = computed_style_target_child_frame_handle(scope, host, handle)
        .or_else(|| {
            computed_style_target_requesting_child_frame_handle(
                host,
                handle,
                requesting_child_window,
            )
        })
    {
        let rendered =
            style_property_value(host, frame_handle, StyleMode::Computed, "display") != "none";
        return ComputedStyleTargetContext::ChildFrameDocument {
            frame_handle,
            rendered,
        };
    }
    if computed_style_target_is_in_flat_tree(host, handle) {
        ComputedStyleTargetContext::ActiveDocument
    } else {
        ComputedStyleTargetContext::EmptyForDetached
    }
}

fn computed_style_target_child_frame_handle(
    scope: &mut v8::PinScope<'_, '_>,
    host: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    let document_handle = host
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::owner_document)?;
    host.child_browsing_context_handle_by_document_handle(scope, document_handle)
}

fn computed_style_target_requesting_child_frame_handle(
    host: &JsContextHost,
    handle: DomHandle,
    requesting_child_window: Option<DomHandle>,
) -> Option<DomHandle> {
    let frame_handle = requesting_child_window?;
    let target_document = host
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::owner_document)?;
    host.child_browsing_context_document_handle(frame_handle)
        .filter(|document| *document == target_document)
        .map(|_| frame_handle)
}

pub(crate) fn iframe_handle_viewport(
    host: &JsContextHost,
    frame_handle: DomHandle,
) -> Option<StyleViewport> {
    iframe_handle_viewport_with_depth(host, frame_handle, 0)
}

pub(crate) fn computed_style_property_values_for_document_snapshot(
    host: &JsContextHost,
    handle: DomHandle,
    properties: &[String],
) -> Option<Vec<String>> {
    let DocumentSnapshotStyleComputation::Available(context) =
        style_computation_context_for_document_snapshot(host, handle)?
    else {
        return Some(vec![String::new(); properties.len()]);
    };
    let style = ComputedStyleRead::new_with_context(host, handle, context);
    Some(
        properties
            .iter()
            .map(|property| style.property(property))
            .collect(),
    )
}

pub(crate) fn computed_style_properties_for_inspector_handle(
    host: &JsContextHost,
    handle: DomHandle,
) -> Option<Vec<(String, String)>> {
    let node = host.dom_host().node(handle)?;
    if !node.is_element() {
        return Some(Vec::new());
    }
    let Some(DocumentSnapshotStyleComputation::Available(context)) =
        style_computation_context_for_document_snapshot(host, handle)
    else {
        // Chromium resolves the node identity independently from whether the
        // node currently participates in a rendered document. Preserve that
        // distinction: an existing detached or hidden-frame element has an
        // empty computed declaration, while a missing handle returns None.
        return Some(Vec::new());
    };
    Some(ComputedStyleRead::new_with_context(host, handle, context).properties())
}

pub(crate) fn marker_pseudo_element_is_generated_for_document_snapshot(
    host: &JsContextHost,
    handle: DomHandle,
) -> Option<bool> {
    let DocumentSnapshotStyleComputation::Available(context) =
        style_computation_context_for_document_snapshot(host, handle)?
    else {
        return Some(false);
    };
    let style = ComputedStyleRead::new_with_context(host, handle, context);
    if !style
        .property("display")
        .split_ascii_whitespace()
        .any(|component| component.eq_ignore_ascii_case("list-item"))
    {
        return Some(false);
    }
    if document_snapshot_has_display_none_ancestor(host, handle, context) {
        return Some(false);
    }

    let content = style.raw_pseudo_property("marker", "content");
    if content.eq_ignore_ascii_case("none") {
        return Some(false);
    }
    if !content.is_empty() && !content.eq_ignore_ascii_case("normal") {
        return Some(true);
    }

    Some(
        !style
            .property("list-style-type")
            .eq_ignore_ascii_case("none")
            || !style
                .property("list-style-image")
                .eq_ignore_ascii_case("none"),
    )
}

fn document_snapshot_has_display_none_ancestor(
    host: &JsContextHost,
    handle: DomHandle,
    context: StyleComputationContext,
) -> bool {
    let mut current = document_snapshot_flat_tree_parent(host, handle);
    while let Some(candidate) = current {
        if host
            .dom_host()
            .node(candidate)
            .is_some_and(crate::dom::native::Node::is_element)
            && ComputedStyleRead::new_with_context(host, candidate, context).property("display")
                == "none"
        {
            return true;
        }
        current = document_snapshot_flat_tree_parent(host, candidate);
    }
    false
}

fn document_snapshot_flat_tree_parent(
    host: &JsContextHost,
    handle: DomHandle,
) -> Option<DomHandle> {
    if let Some(slot) = host.dom_host().assigned_slot_for_node(handle) {
        return Some(slot);
    }
    let parent = host
        .dom_host()
        .node(handle)
        .and_then(crate::dom::native::Node::parent_node)?;
    if host.dom_host().is_shadow_root(parent) {
        return host.dom_host().shadow_root_host(parent);
    }
    Some(parent)
}

enum DocumentSnapshotStyleComputation {
    Available(StyleComputationContext),
    HiddenChildFrame,
}

fn style_computation_context_for_document_snapshot(
    host: &JsContextHost,
    handle: DomHandle,
) -> Option<DocumentSnapshotStyleComputation> {
    let document = host
        .dom_host()
        .node(handle)
        .filter(|node| node.is_element())?
        .owner_document()?;
    if document == host.document_handle() {
        if !computed_style_target_is_in_flat_tree(host, handle) {
            return None;
        }
        Some(DocumentSnapshotStyleComputation::Available(
            StyleComputationContext::new(host.style_viewport()).with_read_document(Some(document)),
        ))
    } else {
        let frame_handle = host.child_browsing_context_host_for_document_handle(document)?;
        if style_property_value(host, frame_handle, StyleMode::Computed, "display") == "none" {
            return Some(DocumentSnapshotStyleComputation::HiddenChildFrame);
        }
        Some(DocumentSnapshotStyleComputation::Available(
            StyleComputationContext::new(
                iframe_handle_viewport(host, frame_handle).unwrap_or_default(),
            )
            .with_read_document(Some(document)),
        ))
    }
}

fn iframe_handle_viewport_with_depth(
    host: &JsContextHost,
    frame_handle: DomHandle,
    depth: usize,
) -> Option<StyleViewport> {
    if depth > 32 {
        return None;
    }
    let parent_viewport = host
        .dom_host()
        .owner_document_handle(frame_handle)
        .and_then(|document| host.child_browsing_context_host_for_document_handle(document))
        .filter(|parent_frame| *parent_frame != frame_handle)
        .and_then(|parent_frame| iframe_handle_viewport_with_depth(host, parent_frame, depth + 1));
    let parent_viewport = parent_viewport.unwrap_or_else(|| host.style_viewport());
    let computed_width = style_property_value_with_context(
        host,
        frame_handle,
        StyleMode::Computed,
        "width",
        StyleComputationContext::new(parent_viewport),
    );
    let width = css_px_value(&computed_width)
        .or_else(|| {
            parent_viewport
                .width
                .zip(css_percent_value(&computed_width))
                .map(|(parent_width, percent)| parent_width * percent / 100.0)
        })
        .or_else(|| {
            host.dom_host()
                .get_attribute(frame_handle, "width")
                .and_then(|value| iframe_width_attribute_viewport_width(&value))
        });
    let computed_height = style_property_value_with_context(
        host,
        frame_handle,
        StyleMode::Computed,
        "height",
        StyleComputationContext::new(parent_viewport),
    );
    let height = css_px_value(&computed_height)
        .or_else(|| {
            parent_viewport
                .height
                .zip(css_percent_value(&computed_height))
                .map(|(parent_height, percent)| parent_height * percent / 100.0)
        })
        .or_else(|| {
            host.dom_host()
                .get_attribute(frame_handle, "height")
                .and_then(|value| iframe_dimension_attribute_viewport_size(&value))
        })
        .or(Some(
            moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_height,
        ));
    Some(
        StyleViewport::new(width, height)
            .with_screen_size(parent_viewport.screen_width, parent_viewport.screen_height),
    )
}

fn computed_style_target_is_in_flat_tree(host: &JsContextHost, handle: DomHandle) -> bool {
    let mut current = handle;
    loop {
        let Some(parent) = host
            .dom_host()
            .node(current)
            .and_then(crate::dom::native::Node::parent_node)
        else {
            return current == host.dom_host().document_handle()
                && host
                    .dom_host()
                    .node(current)
                    .is_some_and(crate::dom::native::Node::is_document);
        };
        if host.dom_host().is_shadow_root(parent) {
            let Some(shadow_host) = host.dom_host().shadow_root_host(parent) else {
                return false;
            };
            current = shadow_host;
            continue;
        }
        if host.dom_host().shadow_root_handle(parent).is_some()
            && computed_style_node_is_slotable_for_flat_tree(host, current)
            && host.dom_host().assigned_slot_for_node(current).is_none()
        {
            return false;
        }
        current = parent;
    }
}

fn computed_style_node_is_slotable_for_flat_tree(host: &JsContextHost, handle: DomHandle) -> bool {
    host.dom_host()
        .node(handle)
        .is_some_and(|node| node.is_element() || node.is_text())
}

pub(crate) fn iframe_width_attribute_viewport_width(value: &str) -> Option<f64> {
    let value = value.trim();
    if let Some(percent) = value
        .strip_suffix('%')
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
    {
        return Some(
            moli_browser_profile::DEFAULT_WINDOW_SURFACE_PROFILE.inner_width * percent / 100.0,
        );
    }
    html_non_negative_integer_prefix(value)
        .filter(|value| *value > 0)
        .map(f64::from)
}

fn iframe_dimension_attribute_viewport_size(value: &str) -> Option<f64> {
    html_non_negative_integer_prefix(value)
        .filter(|value| *value > 0)
        .map(f64::from)
}

fn css_px_value(value: &str) -> Option<f64> {
    value
        .trim()
        .strip_suffix("px")
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
}

fn css_percent_value(value: &str) -> Option<f64> {
    value
        .trim()
        .strip_suffix('%')
        .and_then(|value| value.trim().parse::<f64>().ok())
        .filter(|value| value.is_finite())
}

fn html_non_negative_integer_prefix(value: &str) -> Option<u32> {
    let mut chars = value.chars().skip_while(|ch| ch.is_ascii_whitespace());
    if chars.clone().next() == Some('+') {
        let _ = chars.next();
    }
    let mut acc: u64 = 0;
    let mut had_digit = false;
    for ch in chars {
        if !ch.is_ascii_digit() {
            break;
        }
        had_digit = true;
        acc = acc
            .saturating_mul(10)
            .saturating_add((ch as u8 - b'0') as u64);
        if acc > u32::MAX as u64 {
            return Some(u32::MAX);
        }
    }
    had_digit.then_some(acc as u32)
}

pub(in crate::native_bridge) fn build_style_wrapper_template<'s, 'i>(
    scope: &mut v8::PinScope<'s, 'i, ()>,
) -> v8::Local<'s, v8::ObjectTemplate> {
    let template = v8::ObjectTemplate::new(scope);
    let _ = template.set_internal_field_count(2);
    template.set_indexed_property_handler(
        v8::IndexedPropertyHandlerConfiguration::new()
            .getter(style_indexed_getter)
            .query(style_indexed_query)
            .enumerator(style_indexed_enumerator),
    );
    template.set_named_property_handler(
        v8::NamedPropertyHandlerConfiguration::new()
            .getter(style_named_getter)
            .query(style_named_query)
            .setter(style_named_setter)
            .enumerator(style_named_enumerator),
    );
    template
}

pub(in crate::native_bridge) fn node_style_getter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'s, v8::Value>,
) {
    match style_for_element(scope, args.this()) {
        Some(style) => rv.set(style.into()),
        None => throw_type_error(
            scope,
            "Failed to get 'style' on 'Element': Illegal invocation.",
        ),
    }
}

pub(in crate::native_bridge) fn node_style_setter_function<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'s, v8::Value>,
) {
    let target = match webidl::property_result(
        scope,
        args.this(),
        "style",
        webidl::Context::member("Element", "style"),
    ) {
        Ok(Some(target)) => target,
        Ok(None) => {
            throw_type_error(scope, "Element.style getter did not return an object.");
            return;
        }
        Err(_) => return,
    };
    let Ok(target) = v8::Local::<v8::Object>::try_from(target) else {
        throw_type_error(scope, "Element.style getter did not return an object.");
        return;
    };
    // [PutForwards=cssText] performs ordinary [[Get]] and [[Set]]. This keeps
    // author-defined accessors observable and preserves exceptions from either
    // side. A false [[Set]] result is intentionally ignored by WebIDL.
    let _ = target.set(scope, v8str(scope, "cssText").into(), args.get(0));
}

fn style_for_element<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> Option<v8::Local<'s, v8::Object>> {
    if let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object(scope, receiver) {
        return style_for_element_handle(scope, runtime_ptr, handle);
    }
    let Ok((runtime_ptr, handle)) =
        node_runtime_and_handle_from_object_or_detached(scope, receiver)
    else {
        return None;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        return None;
    }
    if unsafe { &*runtime_ptr }
        .dom_host()
        .node(handle)
        .is_some_and(crate::dom::native::Node::is_connected)
    {
        return style_for_element_handle(scope, runtime_ptr, handle);
    }
    if let Some(style) = get_private_value(scope, receiver, DETACHED_STYLE_SLOT)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
    {
        return Some(style);
    }
    let style = crate::detached_css_style::build_lightweight_detached_css_style_declaration(scope);
    if let Some(callback) = v8::Function::builder(detached_style_changed_callback)
        .data(receiver.into())
        .build(scope)
    {
        crate::detached_css_style::set_lightweight_css_style_change_callback(
            scope, style, callback,
        );
    }
    set_private_value(scope, receiver, DETACHED_STYLE_SLOT, style.into());
    Some(style)
}

fn detached_style_changed_callback<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Ok(element) = v8::Local::<v8::Object>::try_from(args.data()) else {
        return;
    };
    let Ok((runtime_ptr, handle)) = node_runtime_and_handle_from_object_or_detached(scope, element)
    else {
        return;
    };
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        return;
    }
    let Ok(style) = v8::Local::<v8::Object>::try_from(args.get(0)) else {
        return;
    };
    let Some(css_text) = crate::detached_css_style::lightweight_css_style_css_text(scope, style)
    else {
        return;
    };
    unsafe { &mut *runtime_ptr }.set_attribute(scope, runtime_ptr, handle, "style", &css_text);
}

pub(crate) fn style_for_element_handle<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    runtime_ptr: *mut crate::native_bridge::JsContextHost,
    handle: crate::document_runtime::DomHandle,
) -> Option<v8::Local<'s, v8::Object>> {
    if !node_is_element(unsafe { &*runtime_ptr }, handle) {
        return None;
    }
    let runtime = unsafe { &mut *runtime_ptr };
    let style = runtime
        .native_bridge_mut()
        .wrap_style(scope, runtime_ptr, handle)?;
    if get_private_value(scope, style, STYLE_DECLARATION_BASE_URL_SLOT).is_none() {
        let base_url = style_base_url(runtime, handle);
        if runtime.dom_host().get_attribute(handle, "style").is_some() {
            runtime.ensure_element_inline_style_base_url(handle, base_url.clone());
        }
        if let Some(value) = v8_string(scope, base_url.as_str()) {
            set_private_value(scope, style, STYLE_DECLARATION_BASE_URL_SLOT, value.into());
        }
    }
    Some(style)
}
