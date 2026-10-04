use super::*;
use crate::frame_owner_model::{
    DocumentContextTransition, DocumentInspectorBindingTransition, DocumentIsolatedWorldTransition,
    MainDocumentOwnerTransition,
};

/// Cleanup is complete, but preserved worlds may only bind after the new
/// Document is current. Keep the two phases together at commit entry points.
#[must_use]
pub(super) struct PreparedDocumentContextTransition {
    isolated_worlds: DocumentIsolatedWorldTransition,
    retired_world_count: usize,
}

impl ScriptVm {
    pub(super) fn prepare_document_context_transition(
        &mut self,
        transition: DocumentContextTransition,
    ) -> PreparedDocumentContextTransition {
        let isolated_worlds = transition.isolated_world_transition();
        let retired_world_count =
            if transition.inspector_binding() == DocumentInspectorBindingTransition::Replaced {
                // Every cached registration belongs to this Inspector, including
                // descendant worlds. Release cache entries and registrations
                // together while their original runtime and agent are still live.
                let ids = self
                    .page_isolated_world_contexts
                    .execution_context_ids()
                    .collect::<Vec<_>>();
                let count = ids.len();
                for id in ids {
                    self.destroy_isolated_world_context(id);
                }
                count
            } else if let DocumentIsolatedWorldTransition::Retire { retired } = isolated_worlds {
                self.retire_isolated_worlds_for_document_owner(retired)
            } else {
                0
            };
        PreparedDocumentContextTransition {
            isolated_worlds,
            retired_world_count,
        }
    }

    pub(super) fn finish_document_context_transition(
        &mut self,
        prepared: PreparedDocumentContextTransition,
    ) -> (usize, usize) {
        let rebound_world_count = match prepared.isolated_worlds {
            DocumentIsolatedWorldTransition::Rebind { retired, current } => {
                self.rebind_isolated_worlds_for_document_owner_transition(retired, current)
            }
            DocumentIsolatedWorldTransition::Unchanged
            | DocumentIsolatedWorldTransition::Retire { .. } => 0,
        };
        (prepared.retired_world_count, rebound_world_count)
    }

    /// Own the entire retained-Window commit: old-world cleanup, document
    /// replacement, and Inspector publication. The replacement callback installs
    /// the Document; this entry owns Inspector handoff and context publication.
    pub(super) fn commit_main_document_with_replaced_inspector(
        &mut self,
        transition: MainDocumentOwnerTransition,
        mut bootstrap: RendererDocumentIsolateBootstrap,
        env: &crate::runtime::PageVmEnvConfig,
        document_url: &Url,
        replace_document: impl FnOnce(
            &mut Self,
            Option<ScriptVmInitialDocumentEnvironment>,
        ) -> Result<()>,
    ) -> Result<()> {
        assert_eq!(
            transition.context_transition().inspector_binding(),
            DocumentInspectorBindingTransition::Replaced,
        );
        self.apply_pending_main_document_owner_transitions();
        if self.current_main_document_task_owner() != Some(transition.retired_owner()) {
            return Err(anyhow!("main Document changed before the prepared commit"));
        }
        let retired_document = self.document_runtime.document_handle();
        let _subframe_loading_disabler = self
            ._context_host
            .borrow()
            .disable_subframe_loading_for_document_subtree(retired_document);
        let prepared = self.prepare_document_context_transition(transition.context_transition());
        self.with_default_context_scope(|scope, host_ptr| {
            JsContextHost::drop_child_browsing_context_subtree_with_window_realm(
                scope,
                host_ptr,
                retired_document,
            );
            Ok(())
        })?;
        // Descendant default worlds also own registrations in the outgoing
        // Inspector. Consume their retirements before either backing is replaced.
        self.apply_pending_child_document_owner_retirements();
        self.prune_stale_child_default_execution_contexts();
        if self.current_main_document_task_owner() != Some(transition.retired_owner()) {
            return Err(anyhow!(
                "main Document changed during descendant retirement"
            ));
        }
        replace_document(self, bootstrap.initial_document_environment.take())?;
        assert_eq!(
            self.current_main_document_task_owner(),
            Some(transition.current_owner()),
            "the commit must install its prepared Document owner"
        );
        self.finish_document_context_transition(prepared);
        self.reset_main_document_local_state();

        assert_eq!(
            self.page_isolated_world_contexts.len(),
            0,
            "replacing an Inspector must not retain registrations in the world cache"
        );
        assert_eq!(
            self.child_frame_realm_store.len(),
            0,
            "replacing an Inspector must retire descendant default registrations"
        );
        assert!(
            self.prebootstrapped_child_default_contexts
                .borrow()
                .is_empty(),
            "replacing an Inspector must retire prebootstrapped descendant realms"
        );
        self.detach_default_inspector_context_for_context_teardown();
        std::mem::swap(&mut self.page_inspector, &mut bootstrap.page_inspector);
        // Destroy the old registrations before publishing this same retained
        // V8 Context. A later context_destroyed would erase the new binding.
        drop(bootstrap);
        self._context_host
            .borrow_mut()
            .rebind_initial_window_debugger(self.page_inspector.dom_debugger_pause_scheduler());
        let environment = self
            .renderer_page_script_environment
            .as_ref()
            .ok_or_else(|| anyhow!("a reused Window must retain its Page environment"))?;
        let journal = environment.output_journal();
        let isolate = self.renderer_document_isolate.clone();
        let inspector_isolate = isolate.clone();
        let context = &self.page_default_context;
        let page_inspector = &mut self.page_inspector;
        isolate.with_renderer_document_isolate_and_inspector_mut(|isolate, backend| {
            page_inspector
                .reattach_v8_sessions(backend, &env.runtime_inspector_session_restore_snapshots);
            if let Some(commit) = env.main_document_commit.clone() {
                journal.append(crate::runtime::PendingRendererOutputRecord::observation(
                    None,
                    crate::runtime::RendererProtocolObservation::MainDocumentCommit(commit),
                ));
            }
            let scope = pin!(v8::HandleScope::new(isolate));
            let scope = &mut scope.init();
            let context = v8::Local::new(scope, context);
            let default_context = v8::Global::new(scope, context);
            let registered_context = v8::Global::new(scope, context);
            let _scope = &mut v8::ContextScope::new(scope, context);
            page_inspector.attach_context(
                inspector_isolate,
                backend,
                context,
                default_context,
                registered_context,
                document_url,
                env.root_frame_id.as_deref(),
            );
        });
        Ok(())
    }
}
