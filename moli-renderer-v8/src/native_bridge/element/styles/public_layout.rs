use moli_layout::LayoutError;

use super::declaration::property_uses_container_geometry;
use super::{DomHandle, JsContextHost, StyleMode};
use crate::{css_style::canonical_style_property_name, util::v8_string};

/// Public CSSOM observation boundary. Internal layout style resolution never
/// enters here: it already owns a pass-specific viewport and style context.
pub(super) fn prepare_public_style_property(
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    property: &str,
) -> Result<(), LayoutError> {
    if mode != StyleMode::Computed
        || !runtime.layout_policy().is_realtime()
        || !runtime.dom_host().is_connected_to_document(handle)
        || runtime.owner_dispatch_scope_for_node(handle).is_none()
    {
        return Ok(());
    }
    let child_viewport_dependency =
        runtime
            .layout_document_for_source(handle)
            .is_some_and(|document| {
                runtime
                    .child_browsing_context_host_for_document_handle(document)
                    .is_some()
            });
    if child_viewport_dependency
        || property_may_observe_used_geometry(property)
        || property_uses_container_geometry(runtime, handle, property)
    {
        runtime.ensure_observable_layout()?;
    }
    Ok(())
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
pub(super) fn prepare_public_style_batch(
    runtime: &JsContextHost,
    handles: &[DomHandle],
    properties: &[String],
) -> Result<(), LayoutError> {
    if !runtime.layout_policy().is_realtime() || properties.is_empty() {
        return Ok(());
    }
    let used_geometry = properties
        .iter()
        .any(|property| property_may_observe_used_geometry(property));
    let needs_layout = handles.iter().copied().any(|handle| {
        runtime.dom_host().is_connected_to_document(handle)
            && runtime.owner_dispatch_scope_for_node(handle).is_some()
            && (used_geometry
                || properties
                    .iter()
                    .any(|property| property_uses_container_geometry(runtime, handle, property))
                || runtime
                    .layout_document_for_source(handle)
                    .is_some_and(|document| {
                        runtime
                            .child_browsing_context_host_for_document_handle(document)
                            .is_some()
                    }))
    });
    if needs_layout {
        runtime.ensure_observable_layout()?;
    }
    Ok(())
}

pub(super) fn prepare_live_style_property(
    scope: &mut v8::PinScope<'_, '_>,
    runtime: &JsContextHost,
    handle: DomHandle,
    mode: StyleMode,
    property: &str,
) -> bool {
    match prepare_public_style_property(runtime, handle, mode, property) {
        Ok(()) => true,
        Err(error) => {
            if let Some(message) = v8_string(scope, &format!("Layout failed: {error}")) {
                let exception = v8::Exception::error(scope, message);
                scope.throw_exception(exception);
            }
            false
        }
    }
}
