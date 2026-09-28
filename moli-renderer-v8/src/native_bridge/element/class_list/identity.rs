use super::*;
use anyhow::{Context, bail};

pub(super) fn class_list_runtime_handle_and_kind_from_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> anyhow::Result<(*mut JsContextHost, DomHandle, DomTokenListKind)> {
    let (runtime_ptr, handle) = bridge_handle_from_object(scope, object)
        .context("failed to resolve DOMTokenList wrapper")?;
    match handle {
        BridgeHandle::ClassList(handle, kind) => Ok((runtime_ptr, handle, kind)),
        BridgeHandle::Window
        | BridgeHandle::Node(_)
        | BridgeHandle::Dataset(_)
        | BridgeHandle::Style(_)
        | BridgeHandle::ComputedStyle(_, _) => {
            bail!("wrapper did not contain a DOMTokenList identity")
        }
    }
}
