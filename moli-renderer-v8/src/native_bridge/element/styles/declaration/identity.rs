use crate::document_runtime::DomHandle;
use anyhow::{Context, bail};

use super::super::super::super::{BridgeHandle, JsContextHost, bridge_handle_from_object};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum StyleMode {
    Inline,
    Computed,
}

pub(in crate::native_bridge::element::styles) fn style_runtime_and_handle_from_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> anyhow::Result<(*mut JsContextHost, DomHandle, StyleMode)> {
    let (runtime_ptr, handle) = bridge_handle_from_object(scope, object)
        .context("failed to resolve CSSStyleDeclaration wrapper")?;
    match handle {
        BridgeHandle::Style(handle) => {
            let runtime = unsafe { &mut *runtime_ptr };
            if runtime
                .element_inline_style_declaration_state(handle)
                .is_none()
            {
                let base = runtime.existing_element_inline_style_base_url(handle);
                let state = super::parse::inline_style_declaration_state_for_handle(
                    runtime,
                    handle,
                    base.as_ref(),
                );
                // Background longhands and prefixed grammars retain their native
                // shorthand nodes, priority and deferred substitution.
                if state.block.property_is_declared("background")
                    || state.block.style_resolution_text() != state.block.css_text()
                {
                    runtime.set_element_inline_style_declaration_state(handle, state);
                }
            }
            Ok((runtime_ptr, handle, StyleMode::Inline))
        }
        BridgeHandle::ComputedStyle(handle, _) => Ok((runtime_ptr, handle, StyleMode::Computed)),
        BridgeHandle::Window
        | BridgeHandle::Node(_)
        | BridgeHandle::ClassList(_, _)
        | BridgeHandle::Dataset(_) => {
            bail!("wrapper did not contain a CSSStyleDeclaration identity")
        }
    }
}
