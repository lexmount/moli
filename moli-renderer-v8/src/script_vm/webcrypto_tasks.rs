use anyhow::Result;

use super::ScriptVm;
use crate::{
    context_bootstrap::{WebCryptoRejection, WebCryptoTaskResult},
    page_task_queue::RendererPageWebCryptoTaskOwner,
    runtime::AuthorizedCurrentPageWebCryptoTask,
    util::{v8_string, v8str},
};
use moli_webapi_declare::WebApiObject;

#[derive(WebApiObject)]
#[webapi(record, data_properties, enumerable)]
struct CryptoKeyPairResultDeclaration<'scope> {
    private_key: v8::Local<'scope, v8::Object>,
    public_key: v8::Local<'scope, v8::Object>,
}

impl ScriptVm {
    #[cfg(test)]
    pub(crate) fn register_pending_webcrypto_task_producer_for_executor_test(
        &mut self,
    ) -> Result<crate::page_task_queue::RendererPageWebCryptoTaskProducer> {
        self.with_default_context_scope_and_checkpoint_for_test(|scope, host_ptr| {
            let resolver = v8::PromiseResolver::new(scope)
                .expect("WebCrypto executor test resolver should exist");
            unsafe { &mut *host_ptr }
                .register_pending_webcrypto_task(scope, resolver)
                .ok_or_else(|| {
                    anyhow::anyhow!("WebCrypto executor test must capture the current Window realm")
                })
        })
    }

    pub(crate) fn current_pending_webcrypto_task_execution_context(
        &self,
        task: crate::page_task_queue::RendererPageWebCryptoTaskId,
    ) -> Option<crate::native_bridge::WindowExecutionContextIdentity> {
        self._context_host
            .borrow()
            .current_pending_webcrypto_task_execution_context(task)
    }

    /// Settle one page-side WebCrypto Promise body only after the Page arbiter
    /// has authorized its exact PageVm and Window realm.
    ///
    /// The selected Page-task dispatcher owns the enclosing task's microtask
    /// checkpoint. This method deliberately leaves reactions queued after
    /// resolving or rejecting the Promise.
    pub(crate) fn apply_current_webcrypto_task_body(
        &mut self,
        authorization: AuthorizedCurrentPageWebCryptoTask,
    ) -> Result<()> {
        let task = authorization.into_task();
        let owner = task.owner();
        let pending = self
            ._context_host
            .borrow_mut()
            .take_pending_webcrypto_task_for_exact_owner(owner.execution_context(), owner.task())
            .ok_or_else(|| {
                anyhow::anyhow!("authorized WebCrypto task lost its exact pending Promise")
            })?;

        let (bound_owner, bound_dispatch_scope, _realm_token, context) =
            pending.relevant_context.into_parts();
        debug_assert_eq!(bound_owner, owner.execution_context().owner());
        debug_assert_eq!(
            bound_dispatch_scope,
            owner.execution_context().dispatch_scope()
        );
        let context_ptr: *const v8::Global<v8::Context> = &context;
        let resolver = pending.resolver;
        self.with_context_scope_by_ptr(context_ptr, move |scope, _host_ptr| {
            let previous_dispatch_scope = bound_dispatch_scope.enter(scope);
            let resolver = v8::Local::new(scope, &resolver);
            settle_webcrypto_task_result(scope, resolver, task.into_result());
            bound_dispatch_scope.defer_restore(scope, previous_dispatch_scope);
            tracing::debug!(
                task_id = owner.task().task_id(),
                execution_context = ?owner.execution_context(),
                "settled WebCrypto task body in relevant Window execution context"
            );
            Ok(())
        })
    }

    pub(crate) fn discard_stale_webcrypto_task(&mut self, owner: RendererPageWebCryptoTaskOwner) {
        let _ = self
            ._context_host
            .borrow_mut()
            .take_pending_webcrypto_task_for_exact_owner(owner.execution_context(), owner.task());
    }

    /// Apply one WebCrypto task body in a low-level exact-owner fixture.
    ///
    /// This deliberately does not run the selected Page-task checkpoint.
    /// Tests that observe Promise reactions must use the PageVm dispatcher;
    /// this helper is limited to pending-map ownership and stale cleanup.
    #[cfg(test)]
    pub(crate) fn run_webcrypto_task_body_for_authorization_test(&mut self) -> Result<bool> {
        let source = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("WebCrypto executor fixture must retain its production Page source")
            .task_sources();
        let Some(task) = source.take_webcrypto_task_for_executor_test() else {
            return Ok(false);
        };
        let owner = task.owner();
        if self.current_pending_webcrypto_task_execution_context(owner.task())
            == Some(owner.execution_context())
        {
            self.apply_current_webcrypto_task_body(
                AuthorizedCurrentPageWebCryptoTask::new_for_executor_test(task),
            )?;
        } else {
            self.discard_stale_webcrypto_task(owner);
        }
        Ok(true)
    }
}

/// Settle a WebCrypto promise resolver from a completed blocking-task result.
///
/// Shared by the page runtime owner queue and the worker event loop so both
/// lanes map `WebCryptoTaskResult` / `WebCryptoRejection` to the same
/// renderer-visible promise outcomes. The caller must already be inside the
/// owning context scope with the resolver localized.
pub(crate) fn settle_webcrypto_task_result<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    result: Result<WebCryptoTaskResult, WebCryptoRejection>,
) {
    match result {
        Ok(WebCryptoTaskResult::Bytes(bytes)) => {
            match crate::blob::array_buffer_from_bytes(scope, bytes) {
                Some(buffer) => {
                    let _ = resolver.resolve(scope, buffer.into());
                }
                None => reject_webcrypto_task(scope, resolver, WebCryptoRejection::Type),
            }
        }
        Ok(WebCryptoTaskResult::Bool(value)) => {
            let value = v8::Boolean::new(scope, value);
            let _ = resolver.resolve(scope, value.into());
        }
        Ok(WebCryptoTaskResult::JsonWebKey(value)) => match serde_v8::to_v8(scope, value) {
            Ok(value) => {
                let _ = resolver.resolve(scope, value);
            }
            Err(_) => reject_webcrypto_task(scope, resolver, WebCryptoRejection::Type),
        },
        Ok(WebCryptoTaskResult::CryptoKey(payload)) => {
            match crate::context_bootstrap::crypto_key_object_from_clone_payload(scope, *payload) {
                Some(key) => {
                    let _ = resolver.resolve(scope, key.into());
                }
                None => reject_webcrypto_task(scope, resolver, WebCryptoRejection::Type),
            }
        }
        Ok(WebCryptoTaskResult::CryptoKeyPair {
            private_key,
            public_key,
        }) => {
            let private_key =
                crate::context_bootstrap::crypto_key_object_from_clone_payload(scope, *private_key);
            let public_key =
                crate::context_bootstrap::crypto_key_object_from_clone_payload(scope, *public_key);
            match (private_key, public_key) {
                (Some(private_key), Some(public_key)) => {
                    let pair = CryptoKeyPairResultDeclaration {
                        private_key,
                        public_key,
                    }
                    .bind(scope)
                    .expect("CryptoKeyPair declaration should bind");
                    let _ = resolver.resolve(scope, pair.into());
                }
                _ => reject_webcrypto_task(scope, resolver, WebCryptoRejection::Type),
            }
        }
        Err(rejection) => reject_webcrypto_task(scope, resolver, rejection),
    }
}

pub(crate) fn reject_webcrypto_task<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    resolver: v8::Local<'s, v8::PromiseResolver>,
    rejection: WebCryptoRejection,
) {
    let name = rejection.name();
    let exception = if name == "TypeError" {
        let name = v8_string(scope, name).unwrap_or_else(|| v8str(scope, "TypeError"));
        v8::Exception::type_error(scope, name)
    } else {
        crate::context_bootstrap::new_dom_exception_value(scope, name, name)
    };
    let _ = resolver.reject(scope, exception);
}
