use super::*;
use crate::{
    context_bootstrap::shared::throw_error,
    util::{get_private_value, set_private_value},
};

const TRUSTED_TYPES_LAZY_STATE_INSTALLED_SLOT: &str = "__moliTrustedTypesLazyStateInstalled";
const TRUSTED_TYPES_FACTORY_SLOT: &str = "__moliTrustedTypesFactory";
const TRUSTED_TYPES_MATERIALIZING_SLOT: &str = "__moliTrustedTypesMaterializing";

#[derive(WebApiObject)]
#[webapi(fragment)]
struct TrustedTypesGlobalAccessorDeclaration {
    window_receiver: bool,

    #[webapi(
        accessor_property = "trustedTypes",
        getter = trusted_types_global_getter,
        data = self.window_receiver,
        enumerable
    )]
    trusted_types: (),
}

pub(super) fn install_lazy_trusted_types_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    if get_private_value(scope, global, TRUSTED_TYPES_LAZY_STATE_INSTALLED_SLOT).is_some() {
        return Ok(());
    }
    install_trusted_script_code_like_constructor(scope, global)?;
    install_function_constructor_brand_guards(scope, global)?;
    for (index, kind) in TRUSTED_TYPE_KINDS.into_iter().enumerate() {
        let name = kind.constructor_name();
        let data = crate::util::callback_data_index_value(scope, index);
        global
            .set_lazy_data_property_with_configuration(
                scope,
                v8str(scope, name).into(),
                v8::LazyDataPropertyConfiguration::new(trusted_types_global_lazy_getter)
                    .data(data)
                    .property_attribute(v8::PropertyAttribute::DONT_ENUM),
            )
            .unwrap_or(false)
            .then_some(())
            .ok_or_else(|| anyhow!("failed to install lazy `{name}` global"))?;
    }
    // Window owns its mixin attribute; workers inherit it from WorkerGlobalScope.
    let is_worker = get_private_value(scope, global, crate::worker::WORKER_STATE_SLOT).is_some();
    let target = if is_worker {
        crate::util::global_constructor_prototype(scope, "WorkerGlobalScope").ok_or_else(|| {
            anyhow!("WorkerGlobalScope.prototype missing during Trusted Types bootstrap")
        })?
    } else {
        global
    };
    TrustedTypesGlobalAccessorDeclaration::new(!is_worker)
        .initialize(scope, target)
        .map_err(|error| anyhow!("failed to install trustedTypes accessor: {error}"))?;
    set_private_value(
        scope,
        global,
        TRUSTED_TYPES_LAZY_STATE_INSTALLED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    Ok(())
}

fn trusted_types_global_lazy_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(kind) = args
        .data()
        .uint32_value(scope)
        .and_then(|index| TrustedTypeKind::from_callback_index(index as usize))
    else {
        throw_error(
            scope,
            "Trusted Types lazy property has invalid callback data.",
        );
        return;
    };
    let Some(relevant_context) = args.holder().get_creation_context(scope) else {
        throw_error(
            scope,
            "Trusted Types lazy property holder has no creation context.",
        );
        return;
    };
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    if let Err(error) = ensure_trusted_types_state(target_scope) {
        throw_error(
            target_scope,
            &format!("Failed to materialize Trusted Types: {error}"),
        );
        return;
    }
    let global = relevant_context.global(target_scope);
    if let Some(value) = get_private_value(target_scope, global, kind.constructor_slot()) {
        rv.set(value);
    } else {
        throw_error(
            target_scope,
            "Materialized Trusted Types constructor is missing.",
        );
    }
}

fn trusted_types_global_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let receiver = args.this();
    // Reject in the getter's realm before entering a valid receiver's realm.
    // Borrowing a getter changes neither its TypeError realm nor its brand check.
    if args.data().is_true() {
        if !super::super::window_receiver::require_same_origin_window_receiver(
            scope, receiver, false,
        ) {
            return;
        }
    } else if !receiver.strict_equals(scope.get_current_context().global(scope).into()) {
        throw_type_error(
            scope,
            "trustedTypes getter called on incompatible receiver.",
        );
        return;
    }
    let relevant_context = receiver
        .get_creation_context(scope)
        .unwrap_or_else(|| scope.get_current_context());
    let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
    if let Err(error) = ensure_trusted_types_state(target_scope) {
        throw_error(
            target_scope,
            &format!("Failed to materialize Trusted Types: {error}"),
        );
        return;
    }
    let global = relevant_context.global(target_scope);
    if let Some(factory) = cached_object(target_scope, global, TRUSTED_TYPES_FACTORY_SLOT) {
        rv.set(factory.into());
    } else {
        throw_error(
            target_scope,
            "Materialized Trusted Types factory is missing.",
        );
    }
}

fn ensure_trusted_types_state(scope: &mut v8::PinScope<'_, '_>) -> Result<()> {
    let global = scope.get_current_context().global(scope);
    if cached_object(scope, global, TRUSTED_TYPES_FACTORY_SLOT).is_some() {
        return Ok(());
    }
    if get_private_value(scope, global, TRUSTED_TYPES_MATERIALIZING_SLOT).is_some() {
        return Err(anyhow!("reentrant Trusted Types materialization"));
    }
    set_private_value(
        scope,
        global,
        TRUSTED_TYPES_MATERIALIZING_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    let result = build_and_cache_trusted_types_state(scope, global);
    set_private_value(
        scope,
        global,
        TRUSTED_TYPES_MATERIALIZING_SLOT,
        v8::undefined(scope).into(),
    );
    result
}

fn build_and_cache_trusted_types_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    let constructors = TRUSTED_TYPE_KINDS
        .into_iter()
        .map(|kind| build_trusted_type_constructor(scope, kind))
        .collect::<Result<Vec<_>>>()?;

    for binding in constructors {
        set_private_value(
            scope,
            global,
            binding.kind.constructor_slot(),
            binding.constructor.into(),
        );
        set_private_value(
            scope,
            global,
            binding.kind.prototype_slot(),
            binding.prototype.into(),
        );
    }
    let empty_html =
        build_trusted_type_object(scope, TrustedTypeKind::Html, DomStringValue::default())
            .ok_or_else(|| anyhow!("failed to create trustedTypes.emptyHTML"))?;
    let empty_script =
        build_trusted_type_object(scope, TrustedTypeKind::Script, DomStringValue::default())
            .ok_or_else(|| anyhow!("failed to create trustedTypes.emptyScript"))?;
    let factory = TrustedTypesFactoryObjectDeclaration {
        empty_html,
        empty_script,
        created_policy_names: v8::Array::new(scope, 0),
    }
    .bind(scope)
    .map_err(|error| anyhow!("failed to bind TrustedTypePolicyFactory object: {error}"))?;
    set_private_value(scope, global, TRUSTED_TYPES_FACTORY_SLOT, factory.into());
    Ok(())
}

fn cached_object<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) -> Option<v8::Local<'s, v8::Object>> {
    get_private_value(scope, object, slot)
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
}

#[cfg(test)]
pub(crate) fn trusted_types_lazy_state_materialized(scope: &mut v8::PinScope<'_, '_>) -> bool {
    let global = scope.get_current_context().global(scope);
    cached_object(scope, global, TRUSTED_TYPES_FACTORY_SLOT).is_some()
}
