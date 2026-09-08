use std::sync::atomic::{AtomicU64, Ordering};

use moli_module_script_tree::ModuleExceptionId;

use super::*;
use crate::util::private_key;

const MODULE_EXCEPTIONS_SLOT: &str = "__moliModuleExceptions";
static NEXT_EXCEPTION_ID: AtomicU64 = AtomicU64::new(1);

/// Keep the original exception in the realm's GC-traced heap, not in a Rust
/// context slot containing a Global that would keep a retired realm alive.
/// The module map survives document.open(), as does this private registry.
pub(super) fn retain_module_exception(
    scope: &mut v8::PinScope<'_, '_>,
    exception: v8::Local<'_, v8::Value>,
) -> Result<ModuleExceptionId> {
    let global = scope.get_current_context().global(scope);
    let map = match get_private_value(scope, global, MODULE_EXCEPTIONS_SLOT)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
    {
        Some(map) => map,
        None => {
            let map = v8::Map::new(scope);
            let key = private_key(scope, MODULE_EXCEPTIONS_SLOT).ok_or_else(|| {
                anyhow::anyhow!("failed to allocate module exception registry key")
            })?;
            anyhow::ensure!(
                global.set_private(scope, key, map.into()) == Some(true),
                "failed to retain module exception registry"
            );
            map
        }
    };
    let id = NEXT_EXCEPTION_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| anyhow::anyhow!("module exception identifiers exhausted"))?;
    let key = v8::BigInt::new_from_u64(scope, id);
    map.set(scope, key.into(), exception)
        .ok_or_else(|| anyhow::anyhow!("failed to retain module exception"))?;
    Ok(ModuleExceptionId(id))
}

pub(super) fn module_load_error_value<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    error: &ModuleLoadError,
) -> Result<v8::Local<'s, v8::Value>> {
    if let Some(id) = error.exception_id() {
        return retained_module_exception(scope, id);
    }
    let message = v8_string(scope, error.message())
        .ok_or_else(|| anyhow::anyhow!("failed to allocate module error message"))?;
    script_error_value(
        scope,
        error
            .error_constructor()
            .unwrap_or(ScriptErrorConstructorKind::TypeError),
        message,
    )
    .ok_or_else(|| anyhow::anyhow!("failed to create module error"))
}

pub(in crate::script_vm) fn retained_module_exception<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    id: ModuleExceptionId,
) -> Result<v8::Local<'s, v8::Value>> {
    let global = scope.get_current_context().global(scope);
    let map = get_private_value(scope, global, MODULE_EXCEPTIONS_SLOT)
        .and_then(|value| v8::Local::<v8::Map>::try_from(value).ok())
        .ok_or_else(|| anyhow::anyhow!("module exception registry missing from request realm"))?;
    let key = v8::BigInt::new_from_u64(scope, id.0);
    anyhow::ensure!(
        map.has(scope, key.into()) == Some(true),
        "module exception {id:?} does not belong to request realm"
    );
    map.get(scope, key.into())
        .ok_or_else(|| anyhow::anyhow!("failed to read retained module exception"))
}

impl ScriptVm {
    pub(crate) fn preserve_native_module_load_error(
        &mut self,
        realm_id: Option<FrameRealmId>,
        error: ModuleLoadError,
    ) -> std::result::Result<ModuleLoadError, ModuleLoadError> {
        if error.exception_id().is_some() {
            return Ok(error);
        }
        let context_ptr = if let Some(realm_id) = realm_id {
            self.frame_realm_context_ptr(realm_id).map_err(|error| {
                ModuleLoadError::new(ModuleLoadStage::Resolve, error.to_string())
            })?
        } else {
            self.native_module_default_context_ptr()
        };
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                // SAFETY: the selected realm is owned by this VM during entry.
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let scope = &mut v8::ContextScope::new(scope, context);
                let exception = module_load_error_value(scope, &error)?;
                let id = retain_module_exception(scope, exception)?;
                Ok(error.with_exception_id(id))
            })
            .map_err(|error: anyhow::Error| {
                ModuleLoadError::new(ModuleLoadStage::Resolve, error.to_string())
            })
    }
}
