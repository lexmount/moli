use super::highlights::{HighlightRuntimeState, build_highlight_runtime_state};
use super::install::build_css_namespace;
use super::registered_properties::set_css_owner_document_handle;
use super::*;
use crate::{
    context_bootstrap::shared::throw_error,
    document_runtime::DomHandle,
    native_bridge::child_window_handle_from_marker_data,
    util::{get_private_value, set_private_value},
};

const CSS_LAZY_STATE_INSTALLED_SLOT: &str = "__moliCssLazyStateInstalled";
const CSS_NAMESPACE_SLOT: &str = "__moliCssNamespace";
const CSS_NAMESPACE_MATERIALIZING_SLOT: &str = "__moliCssNamespaceMaterializing";
const CSS_OWNER_DOCUMENT_SEED_SLOT: &str = "__moliCssOwnerDocumentSeed";
const HIGHLIGHT_RUNTIME_MATERIALIZING_SLOT: &str = "__moliHighlightRuntimeMaterializing";
const HIGHLIGHT_REGISTRY_SLOT: &str = "__moliHighlightRegistry";
const HIGHLIGHT_CONSTRUCTOR_SLOT: &str = "__moliHighlightConstructor";
const HIGHLIGHT_REGISTRY_CONSTRUCTOR_SLOT: &str = "__moliHighlightRegistryConstructor";

#[derive(Clone, Copy)]
enum CssGlobalLazyProperty {
    Css,
    Highlight,
    HighlightRegistry,
}

impl CssGlobalLazyProperty {
    const ALL: [(Self, &'static str); 3] = [
        (Self::Css, "CSS"),
        (Self::Highlight, "Highlight"),
        (Self::HighlightRegistry, "HighlightRegistry"),
    ];

    const fn callback_data(self) -> u32 {
        match self {
            Self::Css => 0,
            Self::Highlight => 1,
            Self::HighlightRegistry => 2,
        }
    }

    fn from_callback_data(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Css),
            1 => Some(Self::Highlight),
            2 => Some(Self::HighlightRegistry),
            _ => None,
        }
    }
}

pub(super) fn install_css_lazy_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    owner_document: Option<DomHandle>,
) -> Result<()> {
    set_owner_document_seed(scope, global, owner_document);
    if let Some(css) = cached_object(scope, global, CSS_NAMESPACE_SLOT)
        && let Some(owner_document) = owner_document
    {
        set_css_owner_document_handle(scope, css, owner_document);
    }

    if get_private_value(scope, global, CSS_LAZY_STATE_INSTALLED_SLOT).is_some() {
        return Ok(());
    }
    for (property, name) in CssGlobalLazyProperty::ALL {
        let data = v8::Integer::new_from_unsigned(scope, property.callback_data());
        global
            .set_lazy_data_property_with_configuration(
                scope,
                v8str(scope, name).into(),
                v8::LazyDataPropertyConfiguration::new(css_global_lazy_getter)
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
        CSS_LAZY_STATE_INSTALLED_SLOT,
        v8::Boolean::new(scope, true).into(),
    );
    Ok(())
}

fn css_global_lazy_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(property) = args
        .data()
        .uint32_value(scope)
        .and_then(CssGlobalLazyProperty::from_callback_data)
    else {
        throw_error(scope, "CSS lazy property has invalid callback data.");
        return;
    };
    let Some(relevant_context) = args.holder().get_creation_context(scope) else {
        throw_error(scope, "CSS lazy property holder has no creation context.");
        return;
    };
    let owner = v8::Global::new(scope, args.holder());
    let result = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let owner = v8::Local::new(target_scope, &owner);
        let value: Result<v8::Local<'_, v8::Value>> = match property {
            CssGlobalLazyProperty::Css => ensure_css_namespace(target_scope, owner).map(Into::into),
            CssGlobalLazyProperty::Highlight => {
                ensure_highlight_runtime_for_owner(target_scope, owner)
                    .map(|state| state.highlight_constructor)
            }
            CssGlobalLazyProperty::HighlightRegistry => {
                ensure_highlight_runtime_for_owner(target_scope, owner)
                    .map(|state| state.registry_constructor)
            }
        };
        value.map(|value| v8::Global::new(target_scope, value))
    };
    match result {
        Ok(value) => rv.set(v8::Local::new(scope, &value)),
        Err(error) => throw_error(
            scope,
            &format!("Failed to materialize CSS runtime state: {error}"),
        ),
    }
}

fn css_highlights_lazy_getter<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    _name: v8::Local<'s, v8::Name>,
    args: v8::PropertyCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(relevant_context) = args.holder().get_creation_context(scope) else {
        throw_error(scope, "CSS.highlights holder has no creation context.");
        return;
    };
    let css = v8::Global::new(scope, args.holder());
    let result = {
        let target_scope = &mut v8::ContextScope::new(scope, relevant_context);
        let css = v8::Local::new(target_scope, &css);
        ensure_highlight_runtime_for_css(target_scope, css)
            .map(|state| v8::Global::new(target_scope, state.registry))
    };
    match result {
        Ok(registry) => rv.set(v8::Local::new(scope, &registry).into()),
        Err(error) => throw_error(
            scope,
            &format!("Failed to materialize CSS Highlights: {error}"),
        ),
    }
}

// Keep the CSS namespace cache on the concrete Window that owns the lazy
// global property. Related auxiliary Pages own distinct V8 contexts, while
// child and remote Window projections can still cross realm boundaries;
// keying the cache to the owning Window preserves the observable namespace.
fn ensure_css_namespace<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    if let Some(css) = cached_object(scope, owner, CSS_NAMESPACE_SLOT) {
        return Ok(css);
    }
    begin_materialization(scope, owner, CSS_NAMESPACE_MATERIALIZING_SLOT, "CSS")?;
    let result = build_css_namespace_with_state(scope, owner);
    clear_private_slot(scope, owner, CSS_NAMESPACE_MATERIALIZING_SLOT);
    result
}

fn build_css_namespace_with_state<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Result<v8::Local<'s, v8::Object>> {
    let css = build_css_namespace(scope);
    if let Some(owner_document) = owner_document_seed(scope, owner) {
        set_css_owner_document_handle(scope, css, owner_document);
    }
    css.set_lazy_data_property_with_configuration(
        scope,
        v8str(scope, "highlights").into(),
        v8::LazyDataPropertyConfiguration::new(css_highlights_lazy_getter).property_attribute(
            v8::PropertyAttribute::DONT_ENUM | v8::PropertyAttribute::READ_ONLY,
        ),
    )
    .unwrap_or(false)
    .then_some(())
    .ok_or_else(|| anyhow!("failed to install lazy CSS.highlights"))?;
    set_private_value(scope, owner, CSS_NAMESPACE_SLOT, css.into());
    Ok(css)
}

fn ensure_highlight_runtime_for_owner<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    owner: v8::Local<'s, v8::Object>,
) -> Result<HighlightRuntimeState<'s>> {
    let css = ensure_css_namespace(scope, owner)?;
    ensure_highlight_runtime_for_css(scope, css)
}

fn ensure_highlight_runtime_for_css<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    css: v8::Local<'s, v8::Object>,
) -> Result<HighlightRuntimeState<'s>> {
    if let Some(state) = cached_highlight_runtime(scope, css) {
        return Ok(state);
    }
    begin_materialization(
        scope,
        css,
        HIGHLIGHT_RUNTIME_MATERIALIZING_SLOT,
        "CSS Highlights",
    )?;
    let result = build_and_cache_highlight_runtime(scope, css);
    clear_private_slot(scope, css, HIGHLIGHT_RUNTIME_MATERIALIZING_SLOT);
    result
}

fn build_and_cache_highlight_runtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    css: v8::Local<'s, v8::Object>,
) -> Result<HighlightRuntimeState<'s>> {
    let state = build_highlight_runtime_state(scope)?;
    set_private_value(scope, css, HIGHLIGHT_REGISTRY_SLOT, state.registry.into());
    set_private_value(
        scope,
        css,
        HIGHLIGHT_CONSTRUCTOR_SLOT,
        state.highlight_constructor,
    );
    set_private_value(
        scope,
        css,
        HIGHLIGHT_REGISTRY_CONSTRUCTOR_SLOT,
        state.registry_constructor,
    );
    Ok(state)
}

fn cached_highlight_runtime<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    css: v8::Local<'s, v8::Object>,
) -> Option<HighlightRuntimeState<'s>> {
    Some(HighlightRuntimeState {
        registry: cached_object(scope, css, HIGHLIGHT_REGISTRY_SLOT)?,
        highlight_constructor: get_private_value(scope, css, HIGHLIGHT_CONSTRUCTOR_SLOT)?,
        registry_constructor: get_private_value(scope, css, HIGHLIGHT_REGISTRY_CONSTRUCTOR_SLOT)?,
    })
}

fn begin_materialization<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    slot: &'static str,
    label: &str,
) -> Result<()> {
    if get_private_value(scope, global, slot).is_some() {
        return Err(anyhow!("reentrant {label} materialization"));
    }
    set_private_value(scope, global, slot, v8::Boolean::new(scope, true).into());
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

fn set_owner_document_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
    owner_document: Option<DomHandle>,
) {
    let value: v8::Local<'s, v8::Value> = owner_document.map_or_else(
        || v8::undefined(scope).into(),
        |owner_document| v8::BigInt::new_from_u64(scope, owner_document.index() as u64).into(),
    );
    set_private_value(scope, global, CSS_OWNER_DOCUMENT_SEED_SLOT, value);
}

fn owner_document_seed<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    global: v8::Local<'s, v8::Object>,
) -> Option<DomHandle> {
    get_private_value(scope, global, CSS_OWNER_DOCUMENT_SEED_SLOT)
        .and_then(|value| child_window_handle_from_marker_data(scope, value))
}

fn clear_private_slot<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    slot: &'static str,
) {
    set_private_value(scope, object, slot, v8::undefined(scope).into());
}

#[cfg(test)]
pub(crate) fn css_lazy_state_diagnostics(
    scope: &mut v8::PinScope<'_, '_>,
) -> CssLazyStateDiagnostics {
    let global = scope.get_current_context().global(scope);
    let css = cached_object(scope, global, CSS_NAMESPACE_SLOT);
    CssLazyStateDiagnostics {
        css_materialized: css.is_some(),
        highlights_materialized: css
            .and_then(|css| cached_object(scope, css, HIGHLIGHT_REGISTRY_SLOT))
            .is_some(),
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CssLazyStateDiagnostics {
    pub(crate) css_materialized: bool,
    pub(crate) highlights_materialized: bool,
}
