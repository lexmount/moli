use super::*;

/// A single accepted navigation's initiator environment. Only this capability
/// crosses the browser boundary; its V8 token stays in the renderer owner store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RendererCapturedDocumentEnvironment(Arc<CapturedDocumentEnvironmentHandle>);

#[derive(Debug)]
struct CapturedDocumentEnvironmentHandle {
    id: u64,
    origin: String,
    secure_context_type: &'static str,
    runtime: super::page::RendererProducerShutdownHandle,
}

impl PartialEq for CapturedDocumentEnvironmentHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for CapturedDocumentEnvironmentHandle {}

impl Drop for CapturedDocumentEnvironmentHandle {
    fn drop(&mut self) {
        // Cancellation and supersession release native/V8 state on its owner.
        if let Some(runtime) = self.runtime.upgrade_runtime() {
            runtime.release_captured_document_environment(self.id);
        }
    }
}

impl RendererCapturedDocumentEnvironment {
    pub(super) fn capture(
        owner: &owner_local_store::RendererOwnerLocalContext,
        environment: crate::script_vm::ScriptVmCapturedDocumentEnvironment,
        is_secure_context: bool,
    ) -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .expect("captured document environment IDs exhausted");
        let origin = environment.origin().to_owned();
        let secure_context_type = if is_secure_context {
            "Secure"
        } else if Url::parse(&origin)
            .is_ok_and(|url| moli_url::is_potentially_trustworthy_url(&url))
        {
            "InsecureAncestor"
        } else {
            "InsecureScheme"
        };
        owner_local_store::capture_document_environment_on_bound_owner_local_store(id, environment);
        Self(Arc::new(CapturedDocumentEnvironmentHandle {
            id,
            origin,
            secure_context_type,
            runtime: owner
                .owner_state
                .runtime_handle
                .get()
                .expect("live renderer owner has a runtime lease")
                .clone(),
        }))
    }

    pub fn origin(&self) -> &str {
        &self.0.origin
    }

    pub fn secure_context_type(&self) -> &str {
        self.0.secure_context_type
    }

    pub(super) fn take(
        &self,
        isolate_identity: usize,
    ) -> Result<crate::script_vm::ScriptVmInitialDocumentEnvironment> {
        owner_local_store::take_document_environment_on_bound_owner_local_store(
            self.0.id,
            isolate_identity,
        )
    }
}
