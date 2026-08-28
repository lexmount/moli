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
        let browser_context_owner = RendererBrowserContextRuntime::new();
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
        let browser_context_owner = RendererBrowserContextRuntime::new();
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

#[cfg(test)]
mod tests {
    use std::{
        pin::pin,
        sync::{Arc, atomic::AtomicU64},
    };

    use moli_parser::HtmlParser;
    use url::Url;

    use super::*;
    use crate::{
        JsRuntime,
        page_task_queue::{
            PageTaskQueueTestHarness, RendererOwnerWakeSender, RendererPageTaskTestResidence,
            RendererResourceCompletionSender,
        },
        runtime::{
            PageId, RendererAuxiliaryPageReservationAllocator, RendererOutputStreamIdentity,
            RendererOwnerLocalHostId, RendererPageToken, RendererTurnOutputJournal,
        },
        script_vm::{RendererDocumentIsolateHandle, RendererPageScriptEnvironment},
    };

    fn evaluate_preinspector_realm(
        bootstrap: &mut super::super::ScriptVmPreinspectorDefaultWorldBootstrap,
        source: &str,
    ) -> anyhow::Result<String> {
        let renderer_document_isolate = bootstrap.inner.renderer_document_isolate.clone();
        let context = &bootstrap.inner.page_default_context;
        renderer_document_isolate.with_entered_renderer_document_isolate(|isolate| {
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(scope, context);
            let scope = &mut v8::ContextScope::new(scope, context);
            let source = crate::util::v8_string(scope, source)
                .ok_or_else(|| anyhow::anyhow!("failed to allocate preinspector test source"))?;
            let script = v8::Script::compile(scope, source, None)
                .ok_or_else(|| anyhow::anyhow!("failed to compile preinspector test source"))?;
            let value = script
                .run(scope)
                .ok_or_else(|| anyhow::anyhow!("failed to run preinspector test source"))?;
            Ok(value
                .to_string(scope)
                .ok_or_else(|| anyhow::anyhow!("preinspector test value was not stringifiable"))?
                .to_rust_string_lossy(scope))
        })
    }

    #[test]
    fn main_default_realm_prebootstrap_preserves_window_and_document_until_inspector_materialization()
     {
        let _js_runtime = JsRuntime::initialize();
        let standalone_runtime = StandaloneTestRuntime::new();
        let _runtime_guard = standalone_runtime.runtime().enter();
        let page_task_queue = PageTaskQueueTestHarness::new();
        let page_task_tx = page_task_queue.owner_attached_runtime_page_task_sender_for_test();
        let parser_boundary_tx = page_task_queue.parser_boundary_sender();
        let document = HtmlParser::SCRIPTING_ENABLED.parse(
            Url::parse("https://prebootstrap.example/").expect("test URL"),
            "<!doctype html><html><head></head><body>initial</body></html>".to_owned(),
        );
        let resource_loader_owner = ResourceRequestClient::new(&moli_fetch::FetchConfig::default())
            .expect("standalone test loader");
        let initial_document_loader_bootstrap = DocumentResourceLoaderBootstrap::new(
            resource_loader_owner.handle(),
            standalone_runtime.resource_task_runner(),
        );
        let browser_context_owner = RendererBrowserContextRuntime::new();
        let page_realm = ScriptVmDefaultWorldBootstrap::standalone_page_realm_from_dom_host_with_resource_completion_sender_and_browser_context_runtime_for_test_with_current_runtime(
            DomHost::from_dom(document),
            page_task_tx,
            parser_boundary_tx,
            RendererResourceCompletionSender::direct_completion_only(),
            initial_document_loader_bootstrap,
            browser_context_owner.handle(),
        )
        .expect("main Page realm scaffold should bootstrap");
        let renderer_document_isolate = page_realm.renderer_document_isolate.clone();
        let mut preinspector = renderer_document_isolate
            .with_renderer_document_isolate_and_bootstrap_mut(|isolate, isolate_bootstrap| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let opener_context = v8::Context::new(scope, Default::default());
                let scope = &mut v8::ContextScope::new(scope, opener_context);
                let global_template = isolate_bootstrap.global_template(scope);
                page_realm.bootstrap_default_world_in_scope(scope, global_template)
            })
            .expect("main default realm should prebootstrap inside the entered opener scope");
        assert_eq!(
            preinspector
                .inner
                .renderer_document_isolate
                .renderer_document_isolate_inspector_default_context_registry_count(),
            0,
            "prebootstrap must not publish an Inspector default context while the opener callback is active"
        );
        assert_eq!(
            evaluate_preinspector_realm(
                &mut preinspector,
                "globalThis.__prebootstrapDocument = document; globalThis.__prebootstrapArray = Array; document.body.textContent = 'written-before-materialization'; String(document.body.textContent)",
            )
            .expect("preinspector realm should already be script-visible"),
            "written-before-materialization"
        );

        let bootstrap = preinspector.materialize_default_inspector_context();
        assert_eq!(
            bootstrap
                .renderer_document_isolate
                .renderer_document_isolate_inspector_default_context_registry_count(),
            1,
            "materialization must publish exactly the prebootstrapped realm"
        );
        let mut vm = bootstrap
            .finish()
            .expect("materialized realm should finish");
        assert_eq!(
            vm.eval("`${__prebootstrapDocument === document}|${__prebootstrapArray === Array}|${document.body.textContent}`")
                .expect("materialized realm state should remain observable"),
            "true|true|written-before-materialization",
            "Inspector materialization must not replace the Window global, intrinsics, or initial Document"
        );
        drop(vm);
        drop(_runtime_guard);
        drop(standalone_runtime);
    }

    #[test]
    fn related_page_isolate_admission_builds_peer_bindings_inside_entered_opener_scope() {
        let _js_runtime = JsRuntime::initialize();
        let source_page_id = PageId::new_for_testing(701);
        let target_page_id = PageId::new_for_testing(702);
        let (source_wake_tx, _source_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let source_residence =
            RendererPageTaskTestResidence::new(Some(RendererOwnerWakeSender::new(
                source_wake_tx,
                RendererPageToken::new_for_testing(source_page_id),
            )));
        let source_runtime_task_source = source_residence.runtime_source();
        let source_v8_sender = source_runtime_task_source
            .v8_foreground_task_sender()
            .expect("source Page test residence should expose its V8 route");
        let source_bootstrap = source_residence
            .with_owner_runtime(|| {
                RendererDocumentIsolateHandle::new_standalone_without_owner_reservation_for_test(
                    source_v8_sender,
                )
            })
            .expect("source Page document isolate should bootstrap");
        let source_isolate =
            source_bootstrap.clone_renderer_document_isolate_handle_for_owner_retention();
        let source_inspector_backend = source_bootstrap.inspector_isolate_backend_handle();
        let source_membership = source_bootstrap
            .script_agent_page_membership()
            .expect("source Page bootstrap should retain its script-agent membership");
        let source_environment = RendererPageScriptEnvironment::new(
            source_page_id.as_u64(),
            false,
            true,
            true,
            RendererAuxiliaryPageReservationAllocator::new_for_test(
                RendererOwnerLocalHostId::new_for_testing(1),
                source_page_id,
                Arc::new(AtomicU64::new(703)),
            ),
            source_isolate.clone(),
            source_inspector_backend,
            source_membership,
            source_runtime_task_source,
            RendererTurnOutputJournal::new(
                RendererOutputStreamIdentity::new_page_for_protocol_test(source_page_id),
            ),
        )
        .expect("source Page script environment should bind its exact membership");

        let (target_wake_tx, _target_wake_rx) = tokio::sync::mpsc::unbounded_channel();
        let target_residence =
            RendererPageTaskTestResidence::new(Some(RendererOwnerWakeSender::new(
                target_wake_tx,
                RendererPageToken::new_for_testing(target_page_id),
            )));
        let target_v8_sender = target_residence
            .runtime_source()
            .v8_foreground_task_sender()
            .expect("target Page test residence should expose its V8 route");

        let target_bootstrap = source_isolate
            .with_renderer_document_isolate_and_bootstrap_mut(|isolate, _| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let opener_context = v8::Context::new(scope, Default::default());
                let opener_global = opener_context.global(scope);
                let scope = &mut v8::ContextScope::new(scope, opener_context);
                let target_bootstrap = source_environment
                    .bootstrap_related_page_document_isolate_in_scope(
                        scope,
                        &source_bootstrap.bridge_bindings,
                        target_v8_sender,
                    )?;
                let target_global_template = target_bootstrap
                    .bridge_bindings
                    .window_global_template(scope);
                let target_context = v8::Context::new(
                    scope,
                    v8::ContextOptions {
                        global_template: Some(target_global_template),
                        ..Default::default()
                    },
                );
                anyhow::ensure!(
                    !target_context
                        .global(scope)
                        .strict_equals(opener_global.into()),
                    "related Page bridge bindings must construct an independent Context global"
                );
                Ok::<_, anyhow::Error>(target_bootstrap)
            })
            .expect("related Page admission must not re-borrow the already-entered isolate holder");

        assert_eq!(
            source_isolate.script_agent_page_count(),
            2,
            "the source membership capability should admit exactly one related Page route"
        );
        assert_eq!(
            target_bootstrap
                .clone_renderer_document_isolate_handle_for_owner_retention()
                .identity_key(),
            source_isolate.identity_key(),
            "related Page admission must retain the opener's exact script-agent isolate"
        );
        assert_eq!(
            target_bootstrap
                .script_agent_page_membership()
                .expect("related bootstrap should retain its target membership")
                .page_id(),
            target_page_id
        );

        drop(target_bootstrap);
        assert_eq!(
            source_isolate.script_agent_page_count(),
            1,
            "dropping an unadopted related bootstrap must roll back its Page route"
        );
    }
}
