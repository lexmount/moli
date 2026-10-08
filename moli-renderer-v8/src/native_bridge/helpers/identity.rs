use super::super::{BridgeHandle, JsContextHost, ReflectorId};
use crate::util::context_host_ptr_from_context_slot;
use anyhow::{Context, bail, ensure};

pub(in crate::native_bridge) const NATIVE_BRIDGE_OWNER_SLOT: &str = "__moliNativeBridgeOwner";

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
    // Reflector ids belong to the wrapper's native host, not the realm that
    // happens to read an interceptor or borrow an interface method.
    // Fresh platform objects can belong to a receiver realm with a different
    // native host. The private owner retains the producer context and resolves
    // its lifecycle-checked host without storing an unowned raw pointer.
    let object = v8::Local::new(scope, object);
    let owner =
        crate::util::get_private_object(scope, object, NATIVE_BRIDGE_OWNER_SLOT).unwrap_or(object);
    let context = owner
        .get_creation_context(scope)
        .context("native wrapper has no creation context")?;
    let runtime_ptr = context_host_ptr_from_context_slot(context)
        .context("native wrapper's creation context has no JsContextHost")?;
    let reflector_id = object_reflector_id(scope, object)?;
    let handle = unsafe { &*runtime_ptr }
        .native_bridge()
        .bridge_handle(reflector_id)
        .with_context(|| format!("no bridge identity for reflector id {}", reflector_id.raw()))?;
    if let Some(node) = handle.node_handle() {
        let (owner, node) = unsafe { &*runtime_ptr }
            .native_bridge()
            .identity
            .resolve_node_ownership(scope, runtime_ptr, node)
            .context("native node's owning realm has retired")?;
        return Ok((owner, handle.with_node_handle(node)));
    }
    Ok((runtime_ptr, handle))
}
