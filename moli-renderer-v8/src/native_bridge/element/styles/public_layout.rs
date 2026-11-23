use moli_layout::LayoutError;
use style::computed_value_flags::ComputedValueFlags;

use super::declaration::{
    ComputedStyleRead, style_property_value_for_pseudo_with_context,
    style_property_value_with_context,
};
use super::{
    DomHandle, PageScriptHost, StyleMode, style_object_computation_context,
    style_object_pseudo_element,
};
use crate::{css_style::canonical_style_property_name, util::v8_string};

/// Public CSSOM observation boundary. Internal layout style resolution never
/// enters here: it already owns a pass-specific viewport and style context.
pub(super) fn prepare_public_style_property(
    runtime: &PageScriptHost,
    handle: DomHandle,
    mode: StyleMode,
    property: &str,
) -> Result<(), LayoutError> {
    if mode == StyleMode::Computed
        && runtime.layout_policy().is_realtime()
        && style_read_needs_layout(runtime, handle, || {
            property_may_observe_used_geometry(property)
        })
    {
        runtime.ensure_observable_layout()?;
    }
    Ok(())
}

fn style_read_needs_layout(
    runtime: &PageScriptHost,
    handle: DomHandle,
    observes_used_geometry: impl FnOnce() -> bool,
) -> bool {
    runtime.dom_host().is_connected_to_document(handle)
        && runtime.owner_dispatch_scope_for_node(handle).is_some()
        && (observes_used_geometry()
            || runtime
                .layout_document_for_source(handle)
                .is_some_and(|document| {
                    runtime
                        .child_browsing_context_host_for_document_handle(document)
                        .is_some()
                })
            || size_container_style_needs_layout(runtime, handle))
}

fn size_container_style_needs_layout(runtime: &PageScriptHost, handle: DomHandle) -> bool {
    // Stylo records size containers on the flat-tree style path. Refresh the
    // connected element's style as a whole when that geometry can be stale,
    // including queries originating from its pseudos and inherited values.
    !runtime.realtime_screen_layout_is_current()
        && ComputedStyleRead::new(runtime, handle)
            .computed_values()
            .is_none_or(|style| {
                style
                    .flags
                    .contains(ComputedValueFlags::SELF_OR_ANCESTOR_HAS_SIZE_CONTAINER_TYPE)
            })
}

/// Deliberately conservative for used-value families. A pure root-document
/// color read has no layout demand; a child color may depend on media queries
/// evaluated against a changed parent-produced frame viewport.
fn property_may_observe_used_geometry(property: &str) -> bool {
    let property = canonical_style_property_name(property);
    matches!(
        property.as_str(),
        "width"
            | "height"
            | "inline-size"
            | "block-size"
            | "left"
            | "right"
            | "top"
            | "bottom"
            | "transform"
            | "-webkit-transform"
            | "transform-origin"
            | "perspective-origin"
    ) || ["min-", "max-", "margin", "padding", "inset", "grid", "flex"]
        .iter()
        .any(|prefix| property.starts_with(prefix))
}

/// Complete the batch's freshness barrier before any style observation owns
/// prepared inputs. One snapshot batch has at most one canonical root build.
#[cfg(feature = "devtools")]
pub(super) fn prepare_public_style_batch(
    runtime: &PageScriptHost,
    handles: &[DomHandle],
    properties: &[String],
) -> Result<(), LayoutError> {
    if !runtime.layout_policy().is_realtime() || properties.is_empty() {
        return Ok(());
    }
    let used_geometry = properties
        .iter()
        .any(|property| property_may_observe_used_geometry(property));
    let needs_layout = handles
        .iter()
        .copied()
        .any(|handle| style_read_needs_layout(runtime, handle, || used_geometry));
    if needs_layout {
        runtime.ensure_observable_layout()?;
    }
    Ok(())
}

pub(super) fn read_live_style_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    style: v8::Local<'s, v8::Object>,
    runtime: &PageScriptHost,
    handle: DomHandle,
    mode: StyleMode,
    property: &str,
) -> Option<String> {
    if let Err(error) = prepare_public_style_property(runtime, handle, mode, property) {
        if let Some(message) = v8_string(scope, &format!("Layout failed: {error}")) {
            let exception = v8::Exception::error(scope, message);
            scope.throw_exception(exception);
        }
        return None;
    }
    // A screen layout pass can update the viewport of a child document.
    let context = style_object_computation_context(scope, style, runtime, handle, mode);
    Some(match style_object_pseudo_element(scope, style, mode) {
        Some(pseudo) => style_property_value_for_pseudo_with_context(
            runtime, handle, &pseudo, property, context,
        ),
        None => style_property_value_with_context(runtime, handle, mode, property, context),
    })
}
