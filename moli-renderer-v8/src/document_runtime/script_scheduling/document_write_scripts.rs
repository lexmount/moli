use super::*;
use crate::host::{
    ScriptEventKind, ScriptEventTask, ScriptHandleSource,
    begin_prepared_document_write_script_start,
};
use crate::planning::ScriptSource;
use crate::script_vm::perform_microtask_checkpoint_and_report_pending_promise_rejections;
use crate::types::ScriptKind;
use crate::util::create_script_origin;
use crate::v8_execution_watchdog::{
    SCRIPT_TURN_WATCHDOG_TIMEOUT, V8ExecutionWatchdog, V8ExecutionWatchdogKind,
    V8ExecutionWatchdogOutcome,
};
use crate::{context_bootstrap, native_bridge};
use tracing::debug;

pub(in crate::document_runtime) enum DocumentWriteCurrentScriptEventBehavior {
    Skip,
    DispatchImmediately(ScriptEventKind),
}

fn perform_document_write_microtask_checkpoints(scope: &mut v8::PinScope<'_, '_>) {
    perform_microtask_checkpoint_and_report_pending_promise_rejections(scope);
}

impl DocumentRuntime {
    pub(in crate::document_runtime) fn run_prepared_document_write_connected_script(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        node: DomHandle,
        mut script: PreparedScript,
        parser_insertion_controller: Option<ParserInsertionController>,
    ) -> DocumentWriteScriptRunOutcome {
        if !self
            .dom_host
            .node(node)
            .is_some_and(crate::dom::native::Node::is_script_element)
        {
            return DocumentWriteScriptRunOutcome::Complete;
        }
        if self.script_execution_disabled() {
            let _ = self.dom_host.set_script_already_started(node, true);
            return DocumentWriteScriptRunOutcome::Complete;
        }
        unsafe { &mut *host_ptr }
            .sync_owner_style_sheet_texts_for_document_tree_scopes(self.document_handle());

        let Some(wrapper) = unsafe { &mut *host_ptr }
            .native_bridge_mut()
            .wrap_handle(scope, host_ptr, node)
        else {
            return DocumentWriteScriptRunOutcome::Complete;
        };

        let host_script_handle =
            native_bridge::object_string_property(scope, wrapper, "__moliHandle").unwrap_or_else(
                || {
                    let handle = format!("document-write-script-native-{}", node.index());
                    self.script_lifecycle
                        .scripts_mut()
                        .register_script_handle_with_source(
                            &handle,
                            node,
                            ScriptHandleSource::DocumentWriteOwned,
                        );
                    if let Some(value) = v8::String::new(scope, &handle) {
                        let key = v8str(scope, "__moliHandle");
                        let _ = wrapper.define_own_property(
                            scope,
                            key.into(),
                            value.into(),
                            v8::PropertyAttribute::DONT_ENUM,
                        );
                    }
                    handle
                },
            );
        script.node_id = node;
        script.host_script_handle = Some(host_script_handle.clone());
        let inline_classic_source = match (&script.kind, &script.source) {
            (ScriptKind::Classic, ScriptSource::Inline(source)) => Some(source.clone()),
            _ => None,
        };
        if !begin_prepared_document_write_script_start(
            &mut self.dom_host,
            self.script_lifecycle.scripts_mut(),
            node,
            &host_script_handle,
            inline_classic_source.is_some(),
        ) {
            return DocumentWriteScriptRunOutcome::Complete;
        }

        if let Some(source) = inline_classic_source {
            let request =
                crate::content_security_policy::ContentSecurityPolicyScriptElementRequest {
                    nonce: script.fetch_metadata.nonce.as_deref(),
                    integrity: script.fetch_metadata.integrity.as_deref(),
                    parser_inserted: true,
                };
            let Some(source) = native_bridge::element::inline_script_source_for_execution(
                scope, host_ptr, node, &source, request,
            ) else {
                return DocumentWriteScriptRunOutcome::Complete;
            };
            self.execute_document_write_immediate_script(
                scope,
                host_ptr,
                node,
                &host_script_handle,
                source,
                parser_insertion_controller,
                DocumentWriteCurrentScriptEventBehavior::Skip,
            );
            DocumentWriteScriptRunOutcome::Complete
        } else {
            self.execute_document_write_prepared_script(
                scope,
                host_ptr,
                node,
                &host_script_handle,
                script,
                parser_insertion_controller,
            )
        }
    }

    pub(in crate::document_runtime) fn execute_document_write_immediate_script(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        node: DomHandle,
        host_script_handle: &str,
        source: String,
        parser_insertion_controller: Option<ParserInsertionController>,
        current_script_event_behavior: DocumentWriteCurrentScriptEventBehavior,
    ) {
        self.set_current_script_context(CurrentScriptContextSpec {
            handle: Some(node),
            parser_write_insertion_point_active: true,
            parser_insertion_controller,
        });
        // A parser-created script belongs to the document's main world even
        // when an isolated/utility world called document.write(). Blink's
        // ScriptLoader likewise selects ToScriptStateForMainWorld from the
        // script element's Document instead of inheriting the caller's world.
        let default_context = unsafe { &*host_ptr }.page_default_context(scope).expect(
            "parser-created top-level script execution requires the page main-world context",
        );
        let scope = &mut v8::ContextScope::new(scope, default_context);
        let watchdog = V8ExecutionWatchdog::arm(
            V8ExecutionWatchdogKind::ScriptTurn,
            scope.thread_safe_handle(),
            SCRIPT_TURN_WATCHDOG_TIMEOUT,
        );
        let run_result = {
            let _parser_script_nesting = self.enter_parser_script_nesting();
            (|| {
                let source = v8::String::new(scope, &source)?;
                let origin = create_script_origin(scope, self.document_url().as_str(), 0);
                let script = v8::Script::compile(scope, source, Some(&origin))?;
                script.run(scope)
            })()
        };
        let script_timed_out = watchdog.disarm() == V8ExecutionWatchdogOutcome::TimedOut;
        self.clear_current_script_handle();
        if run_result.is_none() {
            if script_timed_out {
                tracing::warn!(
                    host_script_handle,
                    timeout = ?SCRIPT_TURN_WATCHDOG_TIMEOUT,
                    "document.write script execution exceeded its deadline and was terminated"
                );
            }
            return;
        }
        match current_script_event_behavior {
            DocumentWriteCurrentScriptEventBehavior::Skip => {}
            DocumentWriteCurrentScriptEventBehavior::DispatchImmediately(kind) => {
                let task = ScriptEventTask::new(kind, host_script_handle);
                if let Err(error) = self.host_dispatch_script_event(scope, host_ptr, &task) {
                    debug!(
                        host_script_handle,
                        event_kind = ?kind,
                        error,
                        "document.write immediate script event dispatch failed"
                    );
                }
            }
        }
        let watchdog = V8ExecutionWatchdog::arm(
            V8ExecutionWatchdogKind::ScriptTurn,
            scope.thread_safe_handle(),
            SCRIPT_TURN_WATCHDOG_TIMEOUT,
        );
        perform_document_write_microtask_checkpoints(scope);
        if watchdog.disarm() == V8ExecutionWatchdogOutcome::TimedOut {
            tracing::warn!(
                host_script_handle,
                timeout = ?SCRIPT_TURN_WATCHDOG_TIMEOUT,
                "document.write script microtask checkpoint exceeded its deadline and was terminated"
            );
        }
    }

    fn execute_document_write_prepared_script(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        node: DomHandle,
        host_script_handle: &str,
        script: PreparedScript,
        parser_insertion_controller: Option<ParserInsertionController>,
    ) -> DocumentWriteScriptRunOutcome {
        if script.kind == ScriptKind::ImportMap
            && script.source_kind == crate::ScriptSourceKind::Inline
        {
            let source = match &script.source {
                crate::planning::ScriptSource::Inline(source)
                | crate::planning::ScriptSource::Loaded(source)
                | crate::planning::ScriptSource::LoadedBinary { source, .. } => source.as_str(),
                crate::planning::ScriptSource::External => "",
            };
            match self.register_import_map_source(source) {
                Ok(_) => {}
                Err(message) => {
                    if let Err(error) = context_bootstrap::dispatch_window_report_error_message(
                        scope,
                        host_ptr,
                        &message,
                        Some(script.url.as_str()),
                    ) {
                        debug!(
                            host_script_handle,
                            url = %script.url,
                            error,
                            "document.write inline importmap window error dispatch failed"
                        );
                    }
                }
            }
            return DocumentWriteScriptRunOutcome::Complete;
        }

        if matches!(&script.source, crate::planning::ScriptSource::External) {
            if unsafe { &*host_ptr }
                .current_main_document_resource_loader()
                .is_none()
            {
                let task = ScriptEventTask::new(ScriptEventKind::Error, host_script_handle);
                if let Err(error) = self.host_dispatch_script_event(scope, host_ptr, &task) {
                    debug!(
                        host_script_handle,
                        url = %script.url,
                        error,
                        "document.write prepared script error dispatch failed without loader"
                    );
                }
                return DocumentWriteScriptRunOutcome::Complete;
            }

            return DocumentWriteScriptRunOutcome::Suspend(Box::new(
                DocumentWriteExternalScriptStart {
                    node,
                    host_script_handle: host_script_handle.to_owned(),
                    script,
                },
            ));
        }

        let source = match &script.source {
            crate::planning::ScriptSource::Inline(source)
            | crate::planning::ScriptSource::Loaded(source)
            | crate::planning::ScriptSource::LoadedBinary { source, .. } => source.clone(),
            crate::planning::ScriptSource::External => {
                debug_assert!(
                    false,
                    "external document.write scripts should suspend before inline execution"
                );
                return DocumentWriteScriptRunOutcome::Complete;
            }
        };

        self.execute_document_write_immediate_script(
            scope,
            host_ptr,
            node,
            host_script_handle,
            source,
            parser_insertion_controller,
            DocumentWriteCurrentScriptEventBehavior::DispatchImmediately(ScriptEventKind::Load),
        );
        DocumentWriteScriptRunOutcome::Complete
    }
}
