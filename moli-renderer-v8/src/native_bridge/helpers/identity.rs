use super::super::{BridgeHandle, JsContextHost, ReflectorId};
use crate::util::{context_host_ptr_from_context_slot, context_host_ptr_from_global_bridge};
use anyhow::{Context, bail, ensure};

fn object_reflector_id(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> anyhow::Result<ReflectorId> {
    let field_index = match object.internal_field_count() {
        1 => 0,
        // Window wrappers retain their host pointer in field 0 because Window
        // brand checks can cross realm boundaries.
        2 => 1,
        count => bail!("wrapper has {count} internal fields, expected 1 or 2"),
    };
    let value = object
        .get_internal_field(scope, field_index)
        .with_context(|| format!("wrapper is missing reflector field {field_index}"))?;
    let value = v8::Local::<v8::Value>::try_from(value)
        .context("wrapper reflector field is not a JS value")?;
    let number = value
        .number_value(scope)
        .context("wrapper reflector field is not numeric")?;
    ensure!(
        number.is_finite() && number.fract() == 0.0 && number > 0.0,
        "wrapper reflector field {number} is not a positive integer"
    );
    Ok(ReflectorId::from_raw(number as u64))
}

pub(in crate::native_bridge) fn bridge_handle_from_object(
    scope: &mut v8::PinScope<'_, '_>,
    object: v8::Local<'_, v8::Object>,
) -> anyhow::Result<(*mut JsContextHost, BridgeHandle)> {
    // Related Pages share an isolate, but reflector IDs belong to the
    // wrapper's original DOM bridge rather than the calling Window's bridge.
    let runtime_ptr = object
        .get_creation_context(scope)
        .and_then(context_host_ptr_from_context_slot)
        .or_else(|| context_host_ptr_from_global_bridge(scope))
        .context("current context has no JsContextHost")?;
    let reflector_id = object_reflector_id(scope, object)?;
    let handle = unsafe { &*runtime_ptr }
        .native_bridge()
        .bridge_handle(reflector_id)
        .with_context(|| format!("no bridge identity for reflector id {}", reflector_id.raw()))?;
    Ok((runtime_ptr, handle))
}
