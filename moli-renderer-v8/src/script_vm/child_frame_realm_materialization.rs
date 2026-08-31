use anyhow::{Context, Result, anyhow};
use std::pin::pin;

use super::{ScriptVm, child_document_script_scheduler::ChildDocumentScriptSchedulerOwner};
use crate::{
    document_runtime::DomHandle,
    frame_owner_model::{FrameDocumentOwner, FrameDocumentTaskOwner, FrameRealmId},
    page_task_queue::{
        RendererPageChildFrameTaskOwner, RendererPageChildFrameTaskTarget,
        RendererPageChildRealmMaterializationTarget,
    },
    runtime::AuthorizedCurrentPageChildRealmMaterialization,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildRealmMaterializationBodyActivity {
    /// No stored document-start script body was entered. Realm construction
    /// and binding replay may still have produced state or a reported warning.
    StateOnly,
    /// At least one stored document-start script entered the child realm.
    /// Its Promise reactions remain pending for the selected task completion.
    DocumentStartScript,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChildRealmMaterializationApplication {
    /// The exact pending request was consumed. The nested activity is produced
    /// by execution and selects the outer task-end reconciliation; it is not
    /// scheduler metadata.
    Materialized(ChildRealmMaterializationBodyActivity),
    /// The exact owner was current, but reentrant work had already consumed
    /// its request. The selected task still owns its ordinary checkpoint.
    NoPendingRequest,
}

impl ScriptVm {
    pub(crate) fn has_pending_child_frame_realm_materialization(&self) -> bool {
        self._context_host
            .borrow()
            .has_pending_child_frame_realm_materialization()
    }

    pub(super) fn register_child_window_execution_context(
        &mut self,
        execution_context_id: i64,
    ) -> Result<()> {
        let context = self
            .child_frame_realm_store
            .get(&execution_context_id)
            .ok_or_else(|| anyhow!("unknown child default context `{execution_context_id}`"))?;
        let child_handle = context.child_handle;
        let realm_token = context.runtime_observable_context_token;
        let context_ptr = &context.context as *const v8::Global<v8::Context>;
        let owner = self
            ._context_host
            .borrow()
            .current_window_execution_context_owner(
                crate::native_bridge::OwnerDispatchScope::Child(child_handle),
            )
            .ok_or_else(|| anyhow!("child LocalWindow execution context is unavailable"))?;
        let context_host = self._context_host.clone();
        self.renderer_document_isolate
            .with_entered_renderer_document_isolate(|isolate| {
                let scope = pin!(v8::HandleScope::new(isolate));
                let scope = &mut scope.init();
                let context = unsafe { v8::Local::new(scope, &*context_ptr) };
                let context_global = v8::Global::new(scope, context);
                let host = &mut *context_host.borrow_mut();
                host.register_window_execution_context(
                    crate::native_bridge::WindowExecutionContextBinding::new(
                        owner,
                        crate::native_bridge::OwnerDispatchScope::Child(child_handle),
                        realm_token,
                        context_global,
                    ),
                );
                let child_scope = &mut v8::ContextScope::new(scope, context);
                host.bind_child_window_indexed_db_factory_after_context_registration(
                    child_scope,
                    child_handle,
                );
                crate::window_host::signal_pending_window_message_reconsideration(host);
                Ok(())
            })
            .with_context(|| {
                format!(
                    "failed to register child LocalWindow execution context `{execution_context_id}`"
                )
            })?;
        Ok(())
    }

    pub(crate) fn current_child_realm_materialization_target(
        &self,
        expected: RendererPageChildRealmMaterializationTarget,
    ) -> Option<RendererPageChildRealmMaterializationTarget> {
        let document_owner = self
            ._context_host
            .borrow()
            .current_child_document_task_owner(expected.child_handle())?;
        Some(RendererPageChildRealmMaterializationTarget::new(
            expected.child_handle(),
            document_owner,
        ))
    }

    pub(crate) fn apply_current_child_realm_materialization(
        &mut self,
        authorization: AuthorizedCurrentPageChildRealmMaterialization,
        runtime_isolated_worlds: &[crate::protocol_types::RuntimeIsolatedWorldDefinition],
    ) -> Result<ChildRealmMaterializationApplication> {
        let task = authorization.into_task();
        let RendererPageChildFrameTaskTarget::RealmMaterialization(target) = task.owner().target()
        else {
            anyhow::bail!("child realm executor received another child-frame task kind");
        };
        let handle = target.child_handle();
        let owner = target.document_owner();
        if !self
            ._context_host
            .borrow()
            .has_child_frame_realm_materialization_request(handle, owner)
        {
            return Ok(ChildRealmMaterializationApplication::NoPendingRequest);
        }
        // Keep the exact request registered while materialization enters the
        // child Window. V8 callbacks are re-entrant, and a repeated exposure
        // during this boundary must observe the existing reservation rather
        // than enqueue a second durable Page task.
        self.prune_stale_child_default_execution_contexts();
        match self.materialize_child_frame_realm_context_id_for_owner(
            handle,
            owner,
            runtime_isolated_worlds,
        ) {
            Ok((_execution_context_id, activity)) => {
                self._context_host
                    .borrow_mut()
                    .admit_child_realm_dependent_work_after_materialization(handle, owner);
                let promoted = self
                    ._context_host
                    .borrow_mut()
                    .promote_child_modulepreload_work_after_realm_materialization(handle, owner);
                if promoted != 0 {
                    tracing::debug!(
                        child_handle = handle.index(),
                        ?owner,
                        promoted,
                        "promoted Document-owned modulepreload work after exact-realm materialization"
                    );
                }
                Ok(ChildRealmMaterializationApplication::Materialized(activity))
            }
            Err(error) => {
                self._context_host
                    .borrow_mut()
                    .fail_child_frame_realm_materialization(handle, owner);
                let discarded_document_scripts = self
                    ._context_host
                    .borrow_mut()
                    .retire_child_document_script_ready_tasks_for_owner(owner);
                let discarded = self
                    ._context_host
                    .borrow_mut()
                    .discard_child_modulepreload_work_awaiting_realm(handle, owner);
                tracing::warn!(
                    error = %error,
                    handle = handle.index(),
                    ?owner,
                    discarded_document_script_work = discarded_document_scripts,
                    discarded_modulepreload_work = discarded,
                    "failed to materialize requested child FrameRealm"
                );
                Err(error)
            }
        }
    }

    pub(crate) fn discard_stale_child_realm_materialization(
        &mut self,
        owner: RendererPageChildFrameTaskOwner,
    ) -> bool {
        // Realm prebootstrap can precede the durable owner turn. A same-Page
        // stale task therefore owns both the exact request and any detached
        // prebootstrapped V8 context left behind by that child Document.
        self.prune_stale_child_default_execution_contexts();
        let RendererPageChildFrameTaskTarget::RealmMaterialization(target) = owner.target() else {
            return false;
        };
        let mut host = self._context_host.borrow_mut();
        let discarded_script_work =
            host.retire_child_document_script_ready_tasks_for_owner(target.document_owner());
        discarded_script_work != 0
    }

    /// Run one real typed materialization body in low-level ScriptVm semantic
    /// fixtures. This deliberately omits task-end completion; Page-root
    /// admission, checkpoint authority, liveness and fairness remain covered
    /// by selected-task and owner-scheduler integration tests.
    #[cfg(test)]
    pub(crate) fn run_child_realm_materialization_body_for_test(&mut self) -> Result<bool> {
        let source = self
            ._page_task_residence_for_executor_test
            .as_ref()
            .expect("child-realm body fixture must retain its production Page source")
            .task_sources();
        let Some(task) = source.take_scheduler_task_for_executor_test(|descriptor| {
            matches!(
                descriptor,
                crate::page_task_queue::RendererPageReadyDescriptor::ChildFrameTask {
                    owner,
                    ..
                } if matches!(
                    owner.target(),
                    RendererPageChildFrameTaskTarget::RealmMaterialization(_)
                )
            )
        }) else {
            return Ok(false);
        };
        let crate::page_task_queue::RendererPageSchedulerTask::ChildFrameTask(task) = task else {
            unreachable!("child-frame descriptor must dequeue its own family source")
        };
        let owner = task.owner();
        let RendererPageChildFrameTaskTarget::RealmMaterialization(target) = owner.target() else {
            unreachable!("realm selector must only dequeue realm materialization tasks")
        };
        if self.current_child_realm_materialization_target(target) == Some(target) {
            let _ = self.apply_current_child_realm_materialization(
                AuthorizedCurrentPageChildRealmMaterialization::new_for_executor_test(task),
                &[],
            )?;
        } else {
            self.discard_stale_child_realm_materialization(owner);
        }
        Ok(true)
    }

    /// Resolve an already-materialized realm for one exact child Document.
    ///
    /// This is deliberately observation-only. Missing realms are created only
    /// by `apply_current_child_realm_materialization()` after the stable Page
    /// arbiter authorizes the corresponding typed task.
    pub(super) fn current_child_frame_realm_id_for_owner(
        &mut self,
        handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> Result<FrameRealmId> {
        self.prune_stale_child_default_execution_contexts();
        let snapshot = self
            ._context_host
            .borrow()
            .frame_owner_current_child_snapshot(handle)
            .ok_or_else(|| anyhow!("child frame realm owner is unavailable"))?;
        if snapshot.scheduler_lane_id != owner.scheduler_lane_id
            || snapshot.local_window_id != owner.local_window_id
            || snapshot.document_id != owner.document_id
        {
            anyhow::bail!("child frame realm belongs to a retired document owner");
        }
        let realm_id = snapshot
            .realm_id
            .ok_or_else(|| anyhow!("child frame realm has not been materialized"))?;
        let context = self
            .child_frame_realm_store
            .context_for_owner_realm_id(realm_id)
            .ok_or_else(|| anyhow!("child frame realm has no registered V8 context"))?;
        if context.child_handle != handle || context.local_window_id != owner.local_window_id {
            anyhow::bail!("child frame realm context belongs to another child owner");
        }
        Ok(realm_id)
    }

    pub(super) fn current_child_frame_realm_id_for_document_owner(
        &mut self,
        handle: DomHandle,
        owner: FrameDocumentOwner,
    ) -> Result<FrameRealmId> {
        let task_owner = self
            ._context_host
            .borrow()
            .current_child_document_task_owner(handle)
            .filter(|current| current.document_owner() == owner)
            .ok_or_else(|| anyhow!("child frame realm belongs to a retired document owner"))?;
        self.current_child_frame_realm_id_for_owner(handle, task_owner)
    }

    /// Retire one synthetic child realm without creating its replacement.
    ///
    /// PageVm executor tests use this boundary before exercising the real
    /// Window-exposure producer and typed Page turn. It does not create a
    /// replacement realm or provide a second materialization executor.
    #[cfg(test)]
    pub(crate) fn retire_child_frame_realm_for_test(&mut self, handle: DomHandle) {
        self._context_host
            .borrow_mut()
            .clear_child_default_execution_context_id(handle);
        self.prune_stale_child_default_execution_contexts();
    }

    /// Refresh bindings as part of an already-selected materialization body.
    ///
    /// Binding replay is realm initialization, not a nested HTML task. Its V8
    /// scope must therefore leave the enclosing task's checkpoint pending for
    /// the selected Page dispatcher.
    fn refresh_default_runtime_bindings_for_child_window_body(&mut self, handle: DomHandle) {
        let context_host = self._context_host.clone();
        let _ = self.with_default_context_scope(move |scope, _host_ptr| {
            context_host
                .borrow_mut()
                .refresh_default_runtime_bindings_for_child_window(scope, handle);
            Ok(())
        });
    }

    /// Replay all stored default-world state as one selected Page-task body.
    ///
    /// Multiple document-start scripts are synchronous siblings in this
    /// boundary. A Promise reaction queued by an earlier script must not run
    /// before a later script body. The returned activity tells the outer
    /// completion coordinator whether child-record reconciliation is needed.
    fn replay_default_world_state_into_child_default_context_body(
        &mut self,
        execution_context_id: i64,
    ) -> (ChildRealmMaterializationBodyActivity, Result<()>) {
        let mut activity = ChildRealmMaterializationBodyActivity::StateOnly;
        let binding_names = self
            ._context_host
            .borrow()
            .stored_default_runtime_binding_names();
        for name in binding_names {
            if let Err(error) =
                self.install_runtime_binding_in_child_default_context(execution_context_id, &name)
            {
                return (activity, Err(error));
            }
        }

        let scripts = self
            ._context_host
            .borrow()
            .stored_default_document_start_scripts();
        for script in scripts {
            if script.source.trim().is_empty() {
                continue;
            }
            let job = match self.child_frame_source_script_job_for_execution_context_id(
                execution_context_id,
                crate::frame_owner_model::FrameScriptJobKind::Eval,
                script.source,
            ) {
                Ok(job) => job,
                Err(error) => return (activity, Err(error)),
            };
            activity = ChildRealmMaterializationBodyActivity::DocumentStartScript;
            if let Err(error) = self.execute_frame_script_job_selected_task_body(job) {
                return (activity, Err(error));
            }
        }
        (activity, Ok(()))
    }

    fn child_named_runtime_world_definitions(
        &self,
        runtime_isolated_worlds: &[crate::protocol_types::RuntimeIsolatedWorldDefinition],
    ) -> Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition> {
        let (scripts, bindings) = {
            let host = self._context_host.borrow();
            (
                host.stored_document_start_scripts(),
                host.stored_runtime_bindings(),
            )
        };
        let mut worlds = Vec::<crate::protocol_types::RuntimeIsolatedWorldDefinition>::new();
        let remember = |worlds: &mut Vec<crate::protocol_types::RuntimeIsolatedWorldDefinition>,
                        name: &str,
                        grant_universal_access: bool| {
            if let Some(existing) = worlds.iter_mut().find(|world| world.name == name) {
                existing.grant_universal_access |= grant_universal_access;
            } else {
                worlds.push(crate::protocol_types::RuntimeIsolatedWorldDefinition {
                    name: name.to_owned(),
                    grant_universal_access,
                });
            }
        };

        for world in runtime_isolated_worlds {
            remember(&mut worlds, &world.name, world.grant_universal_access);
        }
        for script in scripts {
            if let Some(world_name) = script.world_name.as_deref() {
                remember(&mut worlds, world_name, false);
            }
        }
        for binding in bindings {
            if let Some(world_name) = binding.execution_context_name.as_deref() {
                remember(&mut worlds, world_name, false);
            }
        }
        worlds
    }

    fn child_materialization_owner_is_current(
        &self,
        handle: DomHandle,
        owner: FrameDocumentTaskOwner,
    ) -> bool {
        self._context_host
            .borrow()
            .current_child_document_task_owner(handle)
            == Some(owner)
    }

    fn prepare_child_named_runtime_world(
        &mut self,
        frame_id: &str,
        handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        devtools_session: Option<&moli_page_types::DevToolsSessionKey>,
        world: &crate::protocol_types::RuntimeIsolatedWorldDefinition,
        bindings: &[crate::protocol_types::RuntimeBindingRegistration],
        prepared_worlds: &mut Vec<(Option<moli_page_types::DevToolsSessionKey>, String, i64)>,
    ) -> Result<i64> {
        if let Some((_, _, execution_context_id)) =
            prepared_worlds.iter().find(|(prepared_session, name, _)| {
                prepared_session.as_ref() == devtools_session && name == &world.name
            })
        {
            return Ok(*execution_context_id);
        }
        if !self.child_materialization_owner_is_current(handle, owner) {
            anyhow::bail!("child Document owner changed during named-world materialization");
        }

        let execution_context_id = self.ensure_isolated_world_for_frame_owner(
            devtools_session,
            frame_id,
            &world.name,
            world.grant_universal_access,
        )?;
        if !self.child_materialization_owner_is_current(handle, owner) {
            anyhow::bail!("child Document owner changed after named-world creation");
        }

        // V8 Inspector persists Runtime.addBinding registrations per session
        // and applies them before reporting a newly-created context, matching
        // Chromium's V8RuntimeAgentImpl::addBindings() boundary. Moli
        // additionally installs its native callback-backed binding so calls
        // enter the exact Document-owned runtime queue.
        for binding in bindings.iter().filter(|binding| {
            binding.devtools_session.as_ref() == devtools_session
                && binding.execution_context_name.as_deref() == Some(&world.name)
        }) {
            self.install_runtime_binding_in_execution_context(execution_context_id, &binding.name)?;
        }
        prepared_worlds.push((
            devtools_session.cloned(),
            world.name.clone(),
            execution_context_id,
        ));
        Ok(execution_context_id)
    }

    /// Complete the named-world half of child realm initialization inside the
    /// same selected Page task that created the main world.
    ///
    /// Chromium performs preload-world creation from
    /// `InspectorPageAgent::DidCreateMainWorldContext`; it does not wait for a
    /// later `Page.frameNavigated` projection and send a command back into the
    /// renderer. Keeping this work here makes V8 context lifecycle the sole
    /// realtime producer and prevents a protocol turn from binding a world to
    /// a replacement Document that reused the same frame id.
    fn replay_named_world_state_into_child_contexts_body(
        &mut self,
        frame_id: &str,
        handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        runtime_isolated_worlds: &[crate::protocol_types::RuntimeIsolatedWorldDefinition],
    ) -> ChildRealmMaterializationBodyActivity {
        let (scripts, bindings) = {
            let host = self._context_host.borrow();
            (
                host.stored_document_start_scripts(),
                host.stored_runtime_bindings(),
            )
        };
        let worlds = self.child_named_runtime_world_definitions(runtime_isolated_worlds);
        let mut prepared_worlds =
            Vec::<(Option<moli_page_types::DevToolsSessionKey>, String, i64)>::new();
        let mut activity = ChildRealmMaterializationBodyActivity::StateOnly;

        // Explicit Page.createIsolatedWorld state corresponds to Chromium's
        // pending isolated-world requests and is restored before preload
        // script evaluation.
        for world in runtime_isolated_worlds {
            if let Err(error) = self.prepare_child_named_runtime_world(
                frame_id,
                handle,
                owner,
                None,
                world,
                &bindings,
                &mut prepared_worlds,
            ) {
                self.record_runtime_warning(format_args!(
                    "child named-world restore for `{}` failed: {error}",
                    world.name
                ));
            }
        }

        // Preserve script registry order. A named world is created immediately
        // before the first script that needs it, just like Blink's
        // EvaluateScriptOnNewDocument path.
        for script in scripts
            .into_iter()
            .filter(|script| script.world_name.is_some())
        {
            if script.source.trim().is_empty() {
                continue;
            }
            if !self.child_materialization_owner_is_current(handle, owner) {
                break;
            }
            let world_name = script
                .world_name
                .as_deref()
                .expect("named child preload script must carry its world");
            let Some(world) = worlds.iter().find(|world| world.name == world_name) else {
                self.record_runtime_warning(format_args!(
                    "child preload world `{world_name}` has no retained definition"
                ));
                continue;
            };
            let execution_context_id = match self.prepare_child_named_runtime_world(
                frame_id,
                handle,
                owner,
                script.devtools_session.as_ref(),
                world,
                &bindings,
                &mut prepared_worlds,
            ) {
                Ok(execution_context_id) => execution_context_id,
                Err(error) => {
                    self.record_runtime_warning(format_args!(
                        "child preload world `{world_name}` creation failed: {error}"
                    ));
                    continue;
                }
            };
            activity = ChildRealmMaterializationBodyActivity::DocumentStartScript;
            if let Err(error) = self.exec_in_execution_context(execution_context_id, &script.source)
            {
                self.record_runtime_warning(format_args!(
                    "child preload script in world `{world_name}` failed: {error}"
                ));
            }
        }

        // A world retained only by a named Runtime binding has no script to
        // trigger lazy creation. Materialize it for that binding's owning
        // session before this child-realm turn settles, so equal world names
        // from peer sessions never share a context or native callback.
        for binding in &bindings {
            if !self.child_materialization_owner_is_current(handle, owner) {
                break;
            }
            let Some(world_name) = binding.execution_context_name.as_deref() else {
                continue;
            };
            let Some(world) = worlds.iter().find(|world| world.name == world_name) else {
                self.record_runtime_warning(format_args!(
                    "child Runtime binding world `{world_name}` has no retained definition"
                ));
                continue;
            };
            if let Err(error) = self.prepare_child_named_runtime_world(
                frame_id,
                handle,
                owner,
                binding.devtools_session.as_ref(),
                world,
                &bindings,
                &mut prepared_worlds,
            ) {
                self.record_runtime_warning(format_args!(
                    "child named world `{world_name}` initialization failed: {error}"
                ));
            }
        }

        activity
    }

    fn materialize_child_frame_realm_context_id_for_owner(
        &mut self,
        handle: DomHandle,
        owner: FrameDocumentTaskOwner,
        runtime_isolated_worlds: &[crate::protocol_types::RuntimeIsolatedWorldDefinition],
    ) -> Result<(i64, ChildRealmMaterializationBodyActivity)> {
        if self
            ._context_host
            .borrow()
            .current_child_document_task_owner(handle)
            != Some(owner)
        {
            anyhow::bail!("child frame realm request belongs to a retired document owner");
        }
        let ready_work = self.with_default_context_scope(|scope, host_ptr| {
            let host = unsafe { &mut *host_ptr };
            let (_, ready_work) =
                host.child_browsing_context_window_wrapper_with_ready_work(scope, handle);
            Ok(ready_work)
        })?;
        {
            let mut ready_inputs = ChildDocumentScriptSchedulerOwner::new(self);
            for work in ready_work {
                ready_inputs.notify_parser_classic_next_owner_action(work);
            }
        }
        if self
            ._context_host
            .borrow()
            .current_child_document_task_owner(handle)
            != Some(owner)
        {
            anyhow::bail!("child frame realm owner changed during materialization");
        }
        if let Some(owner_realm_id) = self
            ._context_host
            .borrow()
            .frame_owner_current_child_snapshot(handle)
            .and_then(|snapshot| snapshot.realm_id)
            && let Some(context) = self
                .child_frame_realm_store
                .context_for_owner_realm_id(owner_realm_id)
        {
            if !self
                ._context_host
                .borrow_mut()
                .complete_child_default_realm_materialization(handle, owner, owner_realm_id)
            {
                anyhow::bail!(
                    "child frame realm state changed before existing context registration completed"
                );
            }
            return Ok((
                context.inspector_execution_context_id,
                ChildRealmMaterializationBodyActivity::StateOnly,
            ));
        }
        let frame_id = {
            let host = self._context_host.borrow();
            let (frame_id, _) = host
                .child_browsing_context_request_scope(handle)
                .ok_or_else(|| anyhow::anyhow!("child frame realm scope is unavailable"))?;
            frame_id
        };
        let context = self.create_new_child_default_world(&frame_id, handle)?;
        let execution_context_id = context.inspector_execution_context_id;
        let owner_realm_id = context.owner_realm_id;
        self.child_frame_realm_store
            .insert(execution_context_id, context);
        self.register_child_window_execution_context(execution_context_id)?;
        if !self
            ._context_host
            .borrow_mut()
            .complete_child_default_realm_materialization(handle, owner, owner_realm_id)
        {
            anyhow::bail!("child frame realm state changed before registration completed");
        }
        // Registration establishes execution authority. Runtime binding and
        // document-start replay happen inside that newly materialized realm;
        // they must not require the pre-registration state to pretend that the
        // realm was already executable.
        self.refresh_default_runtime_bindings_for_child_window_body(handle);
        let (activity, replay_result) =
            self.replay_default_world_state_into_child_default_context_body(execution_context_id);
        if let Err(error) = replay_result {
            // A document-start script failure does not un-create its V8
            // context. Keep the authoritative realm materialized and report
            // the injection failure independently.
            self.record_runtime_warning(format_args!(
                "child FrameRealm document-start replay failed: {error}"
            ));
        }
        let named_world_activity = self.replay_named_world_state_into_child_contexts_body(
            &frame_id,
            handle,
            owner,
            runtime_isolated_worlds,
        );
        Ok((
            execution_context_id,
            if activity == ChildRealmMaterializationBodyActivity::DocumentStartScript
                || named_world_activity
                    == ChildRealmMaterializationBodyActivity::DocumentStartScript
            {
                ChildRealmMaterializationBodyActivity::DocumentStartScript
            } else {
                ChildRealmMaterializationBodyActivity::StateOnly
            },
        ))
    }
}
