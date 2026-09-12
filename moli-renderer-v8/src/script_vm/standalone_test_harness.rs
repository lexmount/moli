use std::ops::{Deref, DerefMut};

use super::{ScriptVm, ScriptVmBootstrapError, ScriptVmDefaultWorldBootstrap};
use crate::{
    dom::native::DomHost,
    network::{
        RendererResourceTaskRunner, ResourceRequestClient, ResourceRequestClientOwner,
        context::DocumentResourceLoaderBootstrap,
    },
    page_task_queue::{PageTask, RendererResourceCompletionSender, RuntimePageTaskSender},
    runtime::{RendererBrowserContextRuntime, RendererBrowserContextRuntimeOwner},
};

/// Construction residence for a standalone [`ScriptVm`] test.
///
/// A few low-level tests construct a V8 realm without the renderer owner loop.
/// That fixture still needs an executor while V8 is being initialized, but the
/// executor is test infrastructure rather than page state. Keeping it here
/// prevents `ScriptVm` and its production bootstrap from acquiring test-only
/// runtime fields.
pub(crate) struct StandaloneScriptVmBootstrapHarness {
    bootstrap: ScriptVmDefaultWorldBootstrap,
    browser_context_owner: RendererBrowserContextRuntimeOwner,
    resource_loader_owner: Option<ResourceRequestClientOwner>,
    standalone_runtime: Option<StandaloneTestRuntime>,
}

/// Fully initialized standalone test VM and the infrastructure that owns it.
///
/// Field order is intentional: the VM and its resource authority must be
/// destroyed before the private runtime used to initialize their V8 platform
/// state.
pub(crate) struct StandaloneScriptVmHarness {
    vm: ScriptVm,
    _browser_context_owner: RendererBrowserContextRuntimeOwner,
    _resource_loader_owner: Option<ResourceRequestClientOwner>,
    _standalone_runtime: Option<StandaloneTestRuntime>,
}

struct StandaloneTestRuntime {
    runtime: Option<tokio::runtime::Runtime>,
}

impl StandaloneTestRuntime {
    fn new() -> Self {
        Self {
            runtime: Some(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("standalone ScriptVm test runtime should build"),
            ),
        }
    }

    fn runtime(&self) -> &tokio::runtime::Runtime {
        self.runtime
            .as_ref()
            .expect("standalone test runtime must remain live")
    }

    fn resource_task_runner(&self) -> RendererResourceTaskRunner {
        RendererResourceTaskRunner::from_tokio_handle(self.runtime().handle().clone())
    }
}

impl Drop for StandaloneTestRuntime {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            // Some standalone fixtures are themselves used from async tests.
            // Non-blocking shutdown is the only Tokio-supported way to release
            // this private runtime from inside another runtime.
            runtime.shutdown_background();
        }
    }
}

#[derive(Clone, Copy)]
enum StandaloneResourceExecution {
    /// Resource work is attached to a private current-thread runtime.
    PrivateRuntime,
    /// Resource work is spawned onto the Tokio runtime that owns the test.
    Networked,
}

impl ScriptVmDefaultWorldBootstrap {
    pub(crate) fn standalone_from_dom_host_for_test(
        bootstrap_dom_host: DomHost,
        page_task_tx: RuntimePageTaskSender,
        page_task_parser_boundary_injection_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
    ) -> Result<StandaloneScriptVmBootstrapHarness, ScriptVmBootstrapError> {
        Self::standalone_from_dom_host_with_resource_completion_sender_for_test(
            bootstrap_dom_host,
            page_task_tx,
            page_task_parser_boundary_injection_tx,
            RendererResourceCompletionSender::direct_completion_only(),
        )
    }

    pub(crate) fn standalone_from_dom_host_with_resource_completion_sender_for_test(
        bootstrap_dom_host: DomHost,
        page_task_tx: RuntimePageTaskSender,
        page_task_parser_boundary_injection_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
        resource_completion_tx: RendererResourceCompletionSender,
    ) -> Result<StandaloneScriptVmBootstrapHarness, ScriptVmBootstrapError> {
        let resource_loader_owner = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("standalone test loader");
        let browser_context_owner = RendererBrowserContextRuntime::new_for_test();
        Self::standalone_from_dom_host_with_resource_environment_for_test(
            bootstrap_dom_host,
            page_task_tx,
            page_task_parser_boundary_injection_tx,
            resource_completion_tx,
            browser_context_owner.handle(),
            resource_loader_owner.handle(),
            StandaloneResourceExecution::PrivateRuntime,
            browser_context_owner,
            Some(resource_loader_owner),
        )
    }

    /// Builds a standalone realm whose resource work runs on the owning test's
    /// Tokio runtime.
    ///
    /// Unlike rebinding a finished VM, this installs the caller's complete
    /// ResourceRequestClient—including its Page policy—when the first Document authority is
    /// created.
    pub(crate) fn standalone_networked_from_dom_host_with_resource_completion_sender_for_test(
        bootstrap_dom_host: DomHost,
        page_task_tx: RuntimePageTaskSender,
        page_task_parser_boundary_injection_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
        resource_completion_tx: RendererResourceCompletionSender,
        resource_loader: ResourceRequestClient,
    ) -> Result<StandaloneScriptVmBootstrapHarness, ScriptVmBootstrapError> {
        let browser_context_owner = RendererBrowserContextRuntime::new_for_test();
        Self::standalone_from_dom_host_with_resource_environment_for_test(
            bootstrap_dom_host,
            page_task_tx,
            page_task_parser_boundary_injection_tx,
            resource_completion_tx,
            browser_context_owner.handle(),
            resource_loader,
            StandaloneResourceExecution::Networked,
            browser_context_owner,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn standalone_from_dom_host_with_resource_environment_for_test(
        bootstrap_dom_host: DomHost,
        page_task_tx: RuntimePageTaskSender,
        page_task_parser_boundary_injection_tx: tokio::sync::mpsc::UnboundedSender<PageTask>,
        resource_completion_tx: RendererResourceCompletionSender,
        browser_context_runtime: RendererBrowserContextRuntime,
        resource_loader: ResourceRequestClient,
        resource_execution: StandaloneResourceExecution,
        browser_context_owner: RendererBrowserContextRuntimeOwner,
        resource_loader_owner: Option<ResourceRequestClientOwner>,
    ) -> Result<StandaloneScriptVmBootstrapHarness, ScriptVmBootstrapError> {
        let (standalone_runtime, resource_task_runner) = match resource_execution {
            StandaloneResourceExecution::PrivateRuntime => {
                let standalone_runtime = StandaloneTestRuntime::new();
                let resource_task_runner = standalone_runtime.resource_task_runner();
                (Some(standalone_runtime), resource_task_runner)
            }
            StandaloneResourceExecution::Networked => {
                let task_runner = match RendererResourceTaskRunner::from_current_tokio() {
                    Ok(task_runner) => task_runner,
                    Err(error) => return Err(Box::new((error, bootstrap_dom_host))),
                };
                (None, task_runner)
            }
        };
        let build_bootstrap = || {
            let initial_document_loader_bootstrap = DocumentResourceLoaderBootstrap::new(
                resource_loader.clone(),
                resource_task_runner.clone(),
            );
            Self::standalone_from_dom_host_with_resource_completion_sender_and_browser_context_runtime_for_test_with_current_runtime(
                bootstrap_dom_host,
                page_task_tx,
                page_task_parser_boundary_injection_tx,
                resource_completion_tx,
                initial_document_loader_bootstrap,
                browser_context_runtime,
            )
        };
        let bootstrap = if let Some(runtime) = standalone_runtime.as_ref() {
            let _runtime_guard = runtime.runtime().enter();
            build_bootstrap()?
        } else {
            build_bootstrap()?
        };

        Ok(StandaloneScriptVmBootstrapHarness {
            bootstrap,
            browser_context_owner,
            resource_loader_owner,
            standalone_runtime,
        })
    }
}

impl StandaloneScriptVmBootstrapHarness {
    pub(crate) fn finish(self) -> Result<StandaloneScriptVmHarness, ScriptVmBootstrapError> {
        let Self {
            bootstrap,
            browser_context_owner,
            resource_loader_owner,
            standalone_runtime,
        } = self;
        let mut vm = if let Some(runtime) = standalone_runtime.as_ref() {
            let _runtime_guard = runtime.runtime().enter();
            bootstrap.finish()?
        } else {
            bootstrap.finish()?
        };
        vm.set_layout_policy(crate::real_layout_test_policy());
        Ok(StandaloneScriptVmHarness {
            vm,
            _browser_context_owner: browser_context_owner,
            _resource_loader_owner: resource_loader_owner,
            _standalone_runtime: standalone_runtime,
        })
    }
}

impl Deref for StandaloneScriptVmHarness {
    type Target = ScriptVm;

    fn deref(&self) -> &Self::Target {
        &self.vm
    }
}

impl DerefMut for StandaloneScriptVmHarness {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.vm
    }
}
