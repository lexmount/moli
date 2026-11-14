use super::*;
use crate::{
    context_bootstrap::shared::throw_error,
    util::{get_private_value, set_private_value},
};

const TRUSTED_TYPES_LAZY_STATE_INSTALLED_SLOT: &str = "__moliTrustedTypesLazyStateInstalled";
const TRUSTED_TYPES_FACTORY_SLOT: &str = "__moliTrustedTypesFactory";
const TRUSTED_TYPES_MATERIALIZING_SLOT: &str = "__moliTrustedTypesMaterializing";

#[derive(Clone, Copy)]
enum TrustedTypesGlobalProperty {
    Html,
    Script,
    ScriptUrl,
    Factory,
    Policy,
}

impl TrustedTypesGlobalProperty {
    const ALL: [(Self, &'static str); 5] = [
        (Self::Html, "TrustedHTML"),
        (Self::Script, "TrustedScript"),
        (Self::ScriptUrl, "TrustedScriptURL"),
        (Self::Factory, "trustedTypes"),
        (Self::Policy, "TrustedTypePolicy"),
    ];

    const fn callback_data(self) -> u32 {
        match self {
            Self::Html => 0,
            Self::Script => 1,
            Self::ScriptUrl => 2,
            Self::Factory => 3,
            Self::Policy => 4,
        }
    }

    fn from_callback_data(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Html),
            1 => Some(Self::Script),
            2 => Some(Self::ScriptUrl),
            3 => Some(Self::Factory),
            4 => Some(Self::Policy),
            _ => None,
        }
    }

    const fn trusted_type_kind(self) -> Option<TrustedTypeKind> {
        match self {
            Self::Html => Some(TrustedTypeKind::Html),
            Self::Script => Some(TrustedTypeKind::Script),
            Self::ScriptUrl => Some(TrustedTypeKind::ScriptUrl),
            Self::Factory | Self::Policy => None,
        }
    }
}

pub(super) fn install_lazy_trusted_types_runtime_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Result<()> {
    if get_private_value(scope, global, TRUSTED_TYPES_LAZY_STATE_INSTALLED_SLOT).is_some() {
        return Ok(());
    }
    install_trusted_script_code_like_constructor(scope, global)?;
    for (property, name) in TrustedTypesGlobalProperty::ALL {
        let data = v8::Integer::new_from_unsigned(scope, property.callback_data());
        global
            .set_lazy_data_property_with_configuration(
                scope,
                v8str(scope, name).into(),
                v8::LazyDataPropertyConfiguration::new(trusted_types_global_lazy_getter)
                    .data(data.into())
                    .property_attribute(v8::PropertyAttribute::DONT_ENUM),
            )
            .unwrap_or(false)
            .then_some(())
            .ok_or_else(|| anyhow!("failed to install lazy `{name}` global"))?;
    }
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
    let Some(property) = args
        .data()
        .uint32_value(scope)
        .and_then(TrustedTypesGlobalProperty::from_callback_data)
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
    match ensure_trusted_types_state(target_scope)
        .and_then(|()| cached_public_value(target_scope, property))
    {
        Ok(value) => rv.set(value),
        Err(error) => throw_error(
            target_scope,
            &format!("Failed to materialize Trusted Types: {error}"),
        ),
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
    // Register the intrinsic prototype before any policy object is bound.
    // The public global remains independently replaceable by author script.
    let policy_constructor = TrustedTypePolicyInterfaceDeclaration::build(scope)
        .get_function(scope)
        .ok_or_else(|| anyhow!("failed to build TrustedTypePolicy interface"))?;
    set_private_value(
        scope,
        global,
        TRUSTED_TYPE_POLICY_CONSTRUCTOR_SLOT,
        policy_constructor.into(),
    );
    let policy_prototype = policy_constructor
        .get(scope, v8str(scope, "prototype").into())
        .and_then(|value| v8::Local::<v8::Object>::try_from(value).ok())
        .ok_or_else(|| anyhow!("missing intrinsic TrustedTypePolicy prototype"))?;
    set_private_value(
        scope,
        global,
        TRUSTED_TYPE_POLICY_PROTOTYPE_SLOT,
        policy_prototype.into(),
    );
    let factory = TrustedTypesFactoryDeclaration::default()
        .bind(scope)
        .map_err(|error| anyhow!("failed to bind TrustedTypePolicyFactory: {error}"))?;
    let constructors = TRUSTED_TYPE_KINDS
        .into_iter()
        .map(|kind| build_trusted_type_constructor(scope, kind))
        .collect::<Result<Vec<_>>>()?;

    set_private_value(scope, global, TRUSTED_TYPES_FACTORY_SLOT, factory.into());
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
    Ok(())
}

fn cached_public_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    property: TrustedTypesGlobalProperty,
) -> Result<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let slot = match property {
        TrustedTypesGlobalProperty::Policy => TRUSTED_TYPE_POLICY_CONSTRUCTOR_SLOT,
        _ => property.trusted_type_kind().map_or(
            TRUSTED_TYPES_FACTORY_SLOT,
            TrustedTypeKind::constructor_slot,
        ),
    };
    get_private_value(scope, global, slot)
        .ok_or_else(|| anyhow!("materialized Trusted Types value is missing"))
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
