use super::*;
use crate::host::{
    ModuleFailurePolicy, PreparedRuntimeScriptStartCommit, RuntimeScriptAdmission,
    RuntimeScriptStartPlan, RuntimeScriptStartReservation, RuntimeScriptTextPreparationReservation,
    ScriptEventKind, ScriptEventTask, ScriptHandleSource, cancel_runtime_script_start_admission,
    dispatch_script_event, finish_runtime_script_start_admission, plan_script_start,
    prepare_runtime_script_start_commit,
};
use crate::page_task_queue::PostParseLifecycleWork;
use crate::planning::PreparedScript;
use crate::types::{ScriptErrorConstructorKind, ScriptKind};
use tracing::warn;

impl DocumentRuntime {
    pub(crate) fn host_plan_script_start(
        &mut self,
        node: DomHandle,
        host_script_handle: &str,
    ) -> Option<RuntimeScriptStartPlan> {
        let options = crate::host::ScriptElementLoaderOptions {
            use_prepared_script_text: self.requires_trusted_types_for_script(),
            ..crate::host::ScriptElementLoaderOptions::with_scripting_enabled(
                self.document_scripting_enabled(),
            )
        };
        plan_script_start(
            &mut self.dom_host,
            &self.document,
            node,
            host_script_handle,
            options,
        )
    }

    pub(crate) fn begin_runtime_script_text_preparation(
        &mut self,
        node: DomHandle,
        host_script_handle: &str,
    ) -> Option<RuntimeScriptTextPreparationReservation> {
        RuntimeScriptTextPreparationReservation::begin(
            &self.dom_host,
            self.script_lifecycle.scripts_mut(),
            node,
            host_script_handle,
        )
    }

    pub(crate) fn release_runtime_script_text_preparation(
        &mut self,
        reservation: RuntimeScriptTextPreparationReservation,
    ) {
        reservation.release(self.script_lifecycle.scripts_mut());
    }

    pub(crate) fn prepare_runtime_script_start_commit(
        &mut self,
        plan: RuntimeScriptStartPlan,
    ) -> std::result::Result<PreparedRuntimeScriptStartCommit, String> {
        prepare_runtime_script_start_commit(
            &mut self.dom_host,
            self.script_lifecycle.scripts_mut(),
            plan,
        )
    }

    pub(crate) fn finish_runtime_script_start_admission(
        &mut self,
        reservation: RuntimeScriptStartReservation,
    ) {
        finish_runtime_script_start_admission(
            &mut self.dom_host,
            self.script_lifecycle.scripts_mut(),
            reservation,
        )
    }

    pub(crate) fn cancel_runtime_script_start_admission(
        &mut self,
        reservation: RuntimeScriptStartReservation,
    ) {
        cancel_runtime_script_start_admission(self.script_lifecycle.scripts_mut(), reservation);
    }

    pub(crate) fn publish_runtime_script_admission(
        &self,
        admission: RuntimeScriptAdmission,
    ) -> Result<(), RuntimeScriptAdmission> {
        self.script_lifecycle
            .scripts()
            .publish_runtime_script_admission(admission)
    }

    pub(crate) fn plan_script_event_task_for_script(
        &self,
        event_kind: ScriptEventKind,
        script: &PreparedScript,
        handle: &str,
    ) -> Option<ScriptEventTask> {
        self.script_lifecycle
            .scripts()
            .plan_script_event_task_for_script(event_kind, script.kind, script.source_kind, handle)
    }

    pub(crate) fn script_event_requires_dispatch_for_script(
        &self,
        event_kind: ScriptEventKind,
        script: &PreparedScript,
    ) -> bool {
        self.script_lifecycle
            .scripts()
            .script_event_requires_dispatch_for_script(
                event_kind,
                script.kind,
                script.source_kind,
                script.host_script_handle.as_deref(),
            )
    }

    pub(crate) fn plan_parser_owned_script_event_task(
        &self,
        kind: ScriptEventKind,
        node: DomHandle,
    ) -> Option<ScriptEventTask> {
        let Some(handle) = self
            .script_lifecycle
            .scripts()
            .script_handle_for_node_with_source(node, ScriptHandleSource::ParserOwned)
        else {
            warn!(
                ?node,
                event_kind = ?kind,
                "parser-owned script completion has no registered host handle"
            );
            return None;
        };
        self.script_lifecycle
            .scripts()
            .plan_script_event_task(kind, handle)
    }

    pub(crate) fn enqueue_script_event_lifecycle_work(
        &mut self,
        kind: ScriptEventKind,
        handle: &str,
    ) -> bool {
        let scripts = self.script_lifecycle.scripts_mut();
        let Some(work) = scripts.plan_script_event_lifecycle_work(kind, handle) else {
            return false;
        };
        scripts.enqueue_post_parse_lifecycle_work(work);
        true
    }

    pub(crate) fn plan_script_failure_lifecycle_work(
        &mut self,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<ModuleFailurePolicy>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> Vec<PostParseLifecycleWork> {
        if script.host_script_handle.is_none()
            && matches!(script.kind, ScriptKind::Module | ScriptKind::ImportMap)
            && self
                .dom_host
                .node(script.node_id)
                .is_some_and(|node| node.is_script_element() && node.flags().parser_created())
        {
            warn!(
                node = ?script.node_id,
                kind = ?script.kind,
                source_kind = ?script.source_kind,
                url = %script.url,
                "parser-owned script reached failure planning without a bound host handle"
            );
            assert!(
                script.host_script_handle.is_some(),
                "parser-owned script should bind host handle before failure planning"
            );
        }
        self.script_lifecycle
            .scripts_mut()
            .plan_script_failure_lifecycle_work(
                script.kind,
                script.source_kind,
                script.host_script_handle.as_deref(),
                message,
                Some(script.url.as_str()),
                module_failure_policy,
                error_constructor,
            )
    }

    #[cfg(test)]
    pub(crate) fn plan_script_failure_page_tasks(
        &mut self,
        script: &PreparedScript,
        message: &str,
        module_failure_policy: Option<ModuleFailurePolicy>,
        error_constructor: Option<ScriptErrorConstructorKind>,
    ) -> Vec<PageTask> {
        self.plan_script_failure_lifecycle_work(
            script,
            message,
            module_failure_policy,
            error_constructor,
        )
        .into_iter()
        .map(PostParseLifecycleWork::into_page_task)
        .collect()
    }

    pub(crate) fn enqueue_parser_boundary_lifecycle_work(&mut self, work: PostParseLifecycleWork) {
        self.script_lifecycle
            .enqueue_parser_boundary_lifecycle_work(work);
    }

    pub(crate) fn host_dispatch_script_event(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        task: &ScriptEventTask,
    ) -> std::result::Result<(), String> {
        dispatch_script_event(
            &self.dom_host,
            self.script_lifecycle.scripts(),
            &mut self.events,
            scope,
            host_ptr,
            task,
        )
    }

    pub(crate) fn resolve_host_script_handle(&self, handle: &str) -> Option<DomHandle> {
        self.script_lifecycle.scripts().script_handle_target(handle)
    }

    pub(crate) fn prepared_script_waits_for_blocking_stylesheets(
        &self,
        script: &PreparedScript,
    ) -> bool {
        if !script.waits_for_blocking_stylesheets() {
            return false;
        }
        let Some(handle) = script.host_script_handle.as_deref() else {
            return true;
        };
        self.script_lifecycle
            .scripts()
            .script_handle_waits_for_blocking_stylesheets(handle)
    }

    pub(crate) fn prepared_script_waits_until_dom_content_loaded(
        &self,
        script: &PreparedScript,
    ) -> bool {
        let Some(handle) = script.host_script_handle.as_deref() else {
            return false;
        };
        self.script_lifecycle
            .scripts()
            .script_handle_waits_until_dom_content_loaded(handle)
            && !self.dom_content_loaded_dispatched()
    }

    pub(crate) fn script_handle_source(&self, handle: &str) -> ScriptHandleSource {
        self.script_lifecycle.scripts().script_handle_source(handle)
    }

    pub(crate) fn script_handle_page_task_execution_kind(
        &self,
        handle: &str,
    ) -> Option<crate::host::ScriptPageTaskExecutionKind> {
        self.script_lifecycle
            .scripts()
            .script_handle_page_task_execution_kind(handle)
    }

    pub(crate) fn script_handle_followup_lane(
        &self,
        handle: &str,
    ) -> Option<crate::document_runtime::DeferredPageTaskLane> {
        self.script_lifecycle
            .scripts()
            .script_handle_followup_lane(handle)
    }

    pub(crate) fn set_script_handle_followup_lane(
        &mut self,
        handle: &str,
        lane: crate::document_runtime::DeferredPageTaskLane,
    ) {
        self.script_lifecycle
            .scripts_mut()
            .set_script_handle_followup_lane(handle, lane);
    }

    pub(crate) fn set_script_handle_waits_until_dom_content_loaded(&mut self, handle: &str) {
        self.script_lifecycle
            .scripts_mut()
            .set_script_handle_waits_until_dom_content_loaded(handle);
    }

    pub(crate) fn bind_parser_owned_script_handle_for_node(&mut self, node: DomHandle) -> String {
        let native = self
            .dom_host
            .node(node)
            .unwrap_or_else(|| panic!("missing parser-owned script node {node:?}"));
        assert!(
            native.is_script_element(),
            "parser-owned handle binding requires a <script> node"
        );
        assert!(
            native.flags().parser_created(),
            "parser-owned handle binding requires a parser-created <script>"
        );
        let handle = format!("parser-script-native-{}", node.index());
        self.script_lifecycle
            .scripts_mut()
            .register_script_handle_with_source(&handle, node, ScriptHandleSource::ParserOwned);
        handle
    }

    pub(crate) fn bind_document_write_owned_script_handle_for_node(
        &mut self,
        node: DomHandle,
    ) -> String {
        let native = self
            .dom_host
            .node(node)
            .unwrap_or_else(|| panic!("missing document-write-owned script node {node:?}"));
        assert!(
            native.is_script_element(),
            "document-write-owned handle binding requires a <script> node"
        );
        let handle = format!("document-write-script-native-{}", node.index());
        self.script_lifecycle
            .scripts_mut()
            .register_script_handle_with_source(
                &handle,
                node,
                ScriptHandleSource::DocumentWriteOwned,
            );
        handle
    }

    #[cfg(test)]
    pub(crate) fn bind_runtime_owned_script_handle_for_node(
        &mut self,
        node: DomHandle,
        handle: &str,
    ) -> String {
        self.script_lifecycle
            .scripts_mut()
            .register_script_handle_with_source(handle, node, ScriptHandleSource::RuntimeOwned);
        handle.to_owned()
    }
}
