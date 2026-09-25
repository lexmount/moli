use super::*;
use crate::{
    custom_elements,
    dom::native::DocumentReadyState,
    live_document_parser::{
        DocumentParserCloseDisposition, DocumentParserLifetime, DocumentParserRunState,
        DocumentParserSession, LiveDocumentParserStepOutcome, ParserInsertionHandle,
        ParserStopReason, ParserSuspensionCause,
    },
    native_bridge::context_host::document_parser_owner::ContextDocumentParserOwner,
    parser::{ParserScriptHandoff, PreparedScript},
    types::ScriptKind,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum PopupStreamScriptMode {
    Blocking,
    Async,
    Deferred,
}

pub(super) struct PopupDocumentStream {
    task: LightweightPopupNavigationTaskToken,
    control: crate::live_document_parser::DocumentParserSessionControlHandle,
    parser: Option<DocumentParserSession>,
    deferred: std::collections::VecDeque<PreparedScript>,
    deferred_pending: bool,
    async_pending: usize,
}

impl PopupDocumentStream {
    pub(super) fn async_loads_finished(&self) -> bool {
        self.async_pending == 0
    }

    fn parser_finished(&self) -> bool {
        self.parser.is_none() && self.deferred.is_empty() && !self.deferred_pending
    }
}

impl Drop for PopupDocumentStream {
    fn drop(&mut self) {
        // EOF can synchronously construct an element whose callback opens a
        // replacement stream. The old finishing owner detects that replacement;
        // let its already-admitted finish transition unwind normally.
        if self.control.run_state() != DocumentParserRunState::Finishing {
            self.control.stop(ParserStopReason::DocumentReplacement);
        }
    }
}

impl JsContextHost {
    pub(super) fn start_lightweight_popup_document_parser(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
        document_handle: DomHandle,
        markup: &str,
    ) -> LightweightPopupClassicScriptAdvance {
        let mut parser = DocumentParserSession::start_finite_live_document(
            self.document_url_for_handle(document_handle),
            document_handle,
            self.lightweight_popup_scripting_enabled(task.popup_id()),
        );
        let markup = crate::dom_parser::preserve_decoded_bom_only_browsing_context_body(
            markup,
            Some("text/html"),
        );
        parser.queue_arrived_chunk(markup.into_owned());
        self.lightweight_popup_document_record_mut(task.popup_id())
            .expect("installed popup Document")
            .stream = Some(PopupDocumentStream {
            task,
            control: parser.control_handle(),
            parser: Some(parser),
            deferred: Default::default(),
            deferred_pending: false,
            async_pending: 0,
        });
        let activity = self.pump_popup_document_stream(scope, task, None, false);
        if self.popup_document_stream_is_current(task)
            && self
                .lightweight_popup_document_record(task.popup_id())
                .and_then(|record| record.stream.as_ref())
                .is_some_and(|stream| !stream.parser_finished())
        {
            LightweightPopupClassicScriptAdvance::Pending(activity)
        } else {
            LightweightPopupClassicScriptAdvance::Completed(activity)
        }
    }

    pub(super) fn complete_lightweight_popup_document_parser(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
    ) -> Option<PopupDocumentParserCompletion> {
        if !self.popup_document_stream_is_current(task) {
            return None;
        }
        let record = self.lightweight_popup_document_record(task.popup_id())?;
        let stream = record.stream.as_ref()?;
        if stream.control.lifetime() != DocumentParserLifetime::Finite || !stream.parser_finished()
        {
            return None;
        }
        let document_handle = record.handle?;
        self.sync_child_browsing_context_subtree(scope, document_handle);
        self.prepare_lightweight_popup_parser_completion(task)
    }

    pub(in crate::native_bridge::context_host) fn popup_document_parser_is_current(
        &self,
        owner: LightweightPopupDocumentOwner,
        session: crate::live_document_parser::ParserSessionId,
    ) -> bool {
        self.lightweight_popup_document_record(owner.popup_id())
            .and_then(|record| record.stream.as_ref())
            .is_some_and(|stream| {
                stream.control.session_id() == session
                    && self.popup_document_stream_is_current(stream.task)
            })
    }

    fn popup_document_stream_is_current(&self, task: LightweightPopupNavigationTaskToken) -> bool {
        self.lightweight_popup_document_owner_is_current(task.document_owner())
            && self.lightweight_popup_navigation_id(task.popup_id()) == Some(task.navigation_id)
            && self
                .lightweight_popup_document_record(task.popup_id())
                .and_then(|record| record.stream.as_ref())
                .is_some_and(|stream| stream.task == task)
    }

    pub(super) fn popup_document_stream_insertion(
        &self,
        popup_id: u64,
    ) -> Option<ParserInsertionHandle> {
        let stream = self
            .lightweight_popup_document_record(popup_id)?
            .stream
            .as_ref()?;
        self.popup_document_stream_is_current(stream.task)
            .then(|| stream.parser.as_ref()?.insertion_handle())
            .flatten()
    }

    pub(in crate::native_bridge) fn open_lightweight_popup_document_stream(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
    ) {
        let Some(popup_id) = self.lightweight_popup_id_for_document_handle(document_handle) else {
            return;
        };
        if self
            .popup_document_stream_insertion(popup_id)
            .is_some_and(|insertion| insertion.control_handle().parser_script_nesting_level() != 0)
        {
            return;
        }
        let Some(owner) = self.current_lightweight_popup_document_owner(popup_id) else {
            return;
        };
        let entry_document = self.document_open_entry_document(scope);
        Self::dispatch_document_open_descendant_frame_unload_lifecycle(
            scope,
            host_ptr,
            document_handle,
        );
        if !self.lightweight_popup_document_owner_is_current(owner) {
            return;
        }
        let url = entry_document
            .map(|entry| self.document_open_replacement_url(document_handle, entry))
            .unwrap_or_else(|| self.document_url_for_handle(document_handle));
        let Some(task) = self.start_lightweight_popup_navigation_attempt(
            popup_id,
            self.document_url_for_handle(document_handle),
            LightweightPopupNavigationDocumentTarget::CurrentDocument,
        ) else {
            return;
        };
        self.clear_event_callbacks_for_document_replacement(document_handle, false);
        if let Some(window) = self.lightweight_popup_window(scope, popup_id) {
            clear_lightweight_popup_window_document_event_state(scope, window);
        }
        let scripting_enabled = self.lightweight_popup_scripting_enabled(popup_id);
        custom_elements::with_custom_element_reaction_scope(scope, host_ptr, |scope| {
            let host = unsafe { &mut *host_ptr };
            // Drop the old session before removal callbacks can write again.
            host.lightweight_popup_document_record_mut(popup_id)
                .expect("current popup")
                .stream = None;
            host.remove_all_children_for_document_replacement(scope, host_ptr, document_handle);
            // The input stream changes, but the same Document retains its
            // autofocus processed flag. Retire only its old candidates.
            host.clear_autofocus_candidates(document_handle);
            host.set_lightweight_popup_same_document_url(scope, popup_id, url.clone());
            host.dom_host_mut()
                .set_html_quirks_mode_for_parser_document(
                    document_handle,
                    html5ever::tree_builder::QuirksMode::NoQuirks,
                );
            let parser = DocumentParserSession::start_open_live_document(
                url.clone(),
                document_handle,
                scripting_enabled,
            );
            let record = host
                .lightweight_popup_document_record_mut(popup_id)
                .expect("current popup");
            record.stream = Some(PopupDocumentStream {
                task,
                control: parser.control_handle(),
                parser: Some(parser),
                deferred: Default::default(),
                deferred_pending: false,
                async_pending: 0,
            });
            record.dom_content_loaded = PopupDomContentLoadedState::Parsing;
            record.pending_load_event = None;
            record.queued_load_event = None;
            record.load_event_dispatched = false;
            record.incomplete_child_frame_loads.clear();
            host.set_dom_document_ready_state_for_handle(
                document_handle,
                DocumentReadyState::Loading,
            );
            if let Some(window) = host.lightweight_popup_window(scope, popup_id) {
                crate::context_bootstrap::update_history_for_document_open(scope, window, &url);
            }
            if host.popup_document_stream_is_current(task) {
                host.lightweight_popup_document_record_mut(popup_id)
                    .expect("current popup")
                    .is_initial_empty_document = false;
            }
        });
    }

    pub(in crate::native_bridge) fn write_lightweight_popup_document_stream<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        host_ptr: *mut JsContextHost,
        document_handle: DomHandle,
        document: v8::Local<'s, v8::Object>,
        html: &str,
    ) {
        let Some(popup_id) = self.lightweight_popup_id_for_document_handle(document_handle) else {
            return;
        };
        if self.popup_document_stream_insertion(popup_id).is_none() {
            if self.has_document_unload_counter(document_handle)
                || self.has_ignore_destructive_writes_counter(document_handle)
                || !self.check_document_open_origin(scope, document)
            {
                return;
            }
            self.open_lightweight_popup_document_stream(scope, host_ptr, document_handle);
        }
        let Some(task) = self
            .lightweight_popup_document_record(popup_id)
            .and_then(|record| record.stream.as_ref())
            .map(|stream| stream.task)
        else {
            return;
        };
        let _ = self.pump_popup_document_stream(scope, task, Some(html), false);
    }

    pub(in crate::native_bridge) fn close_lightweight_popup_document_stream(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        document_handle: DomHandle,
    ) {
        let Some(task) = self
            .lightweight_popup_id_for_document_handle(document_handle)
            .and_then(|id| self.lightweight_popup_document_record(id))
            .and_then(|record| record.stream.as_ref())
            .map(|stream| stream.task)
        else {
            return;
        };
        // Only document.open() creates a stream that document.close() may end.
        if self
            .popup_document_stream_insertion(task.popup_id())
            .is_some_and(|insertion| insertion.lifetime() == DocumentParserLifetime::Finite)
        {
            return;
        }
        let _ = self.pump_popup_document_stream(scope, task, None, true);
    }

    fn pump_popup_document_stream(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
        html: Option<&str>,
        close: bool,
    ) -> PopupDocumentLoadBodyActivity {
        let mut activity = PopupDocumentLoadBodyActivity::NoPageCodeOrEventDispatch;
        self.pump_popup_document_stream_with_activity(scope, task, html, close, &mut activity);
        activity
    }

    fn pump_popup_document_stream_with_activity(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
        html: Option<&str>,
        close: bool,
        activity: &mut PopupDocumentLoadBodyActivity,
    ) {
        if !self.popup_document_stream_is_current(task) {
            return;
        }
        let popup_id = task.popup_id();
        let Some(insertion) = self.popup_document_stream_insertion(popup_id) else {
            return;
        };
        let document_handle = self
            .lightweight_popup_document_handle(popup_id)
            .expect("parser Document");
        let control = insertion.control_handle();
        let in_script = control.parser_script_nesting_level() != 0;
        let nested_pump = matches!(
            insertion.run_state(),
            DocumentParserRunState::Pumping { .. }
        );
        let ready = if close {
            insertion.request_close() == DocumentParserCloseDisposition::DrainNow && !in_script
        } else {
            matches!(
                insertion.run_state(),
                DocumentParserRunState::Ready | DocumentParserRunState::Pumping { .. }
            )
        };
        if let Some(html) = html {
            if nested_pump && !in_script {
                insertion.append_at_current_insertion_point(html);
            } else if ready {
                // A runnable nested write needs a fresh input frame ahead of
                // the suspended outer tail, just like a child parser write.
                insertion.enqueue_script_input_html(html.to_owned());
            } else if in_script {
                if !insertion.append_to_current_script_input(html) {
                    insertion.enqueue_script_input_html(html.to_owned());
                }
            } else {
                insertion.queue_arrived_chunk(html.to_owned());
            }
        }
        if !ready {
            return;
        }
        loop {
            if !self.popup_document_stream_is_current(task)
                || matches!(control.run_state(), DocumentParserRunState::Stopped(_))
            {
                insertion.stop(ParserStopReason::DocumentReplacement);
                return;
            }
            if insertion.is_suspended() {
                return;
            }
            let outcome = {
                let mut owner = ContextDocumentParserOwner::new_popup(
                    self,
                    scope,
                    document_handle,
                    task.document_owner(),
                    control.clone(),
                );
                insertion.advance_queued_or_resume_step(&mut owner)
            };
            // A parser mutation can invoke a custom element callback which
            // replaces the stream and drops the old insertion controller.
            if !self.popup_document_stream_is_current(task)
                || matches!(control.run_state(), DocumentParserRunState::Stopped(_))
            {
                return;
            }
            // Native parser mutations deliver meta policies and maintain frame
            // ownership before a script preparation checkpoint can run.
            let _ = insertion.take_discovery_signals();
            let outcome = if let LiveDocumentParserStepOutcome::ScriptPreparation(request) = outcome
            {
                if request.needs_microtask_checkpoint() {
                    let _ =
                        crate::script_cleanup::perform_parser_script_preparation_checkpoint(scope);
                }
                if !self.popup_document_stream_is_current(task) {
                    return;
                }
                let mut owner = ContextDocumentParserOwner::new_popup(
                    self,
                    scope,
                    document_handle,
                    task.document_owner(),
                    control.clone(),
                );
                LiveDocumentParserStepOutcome::ScriptHandoff(Box::new(
                    insertion.prepare_script(*request, &mut owner),
                ))
            } else {
                outcome
            };
            match outcome {
                LiveDocumentParserStepOutcome::InputBoundary => {
                    if in_script || nested_pump {
                        return;
                    }
                    if !insertion.input_is_empty() {
                        continue;
                    }
                    if matches!(
                        insertion.lifetime(),
                        DocumentParserLifetime::Finite | DocumentParserLifetime::Closing
                    ) {
                        let Some(mut parser) = self
                            .lightweight_popup_document_record_mut(popup_id)
                            .and_then(|record| record.stream.as_mut())
                            .and_then(|stream| stream.parser.take())
                        else {
                            return;
                        };
                        parser.request_finish();
                        let _ = parser.admit_delayed_finish_at_local_owner_boundary();
                        let mut owner = ContextDocumentParserOwner::new_popup(
                            self,
                            scope,
                            document_handle,
                            task.document_owner(),
                            control.clone(),
                        );
                        let signals = parser.finish(&mut owner);
                        if !self.popup_document_stream_is_current(task) {
                            return;
                        }
                        custom_elements::apply_parser_created_null_registry_associations(
                            self,
                            &signals.parser_created_null_registry_elements,
                        );
                        self.finish_popup_document_stream(scope, task, activity);
                    }
                    return;
                }
                LiveDocumentParserStepOutcome::CustomElementConstructionHandoff(handoff) => {
                    *activity = PopupDocumentLoadBodyActivity::PageCodeOrEventDispatchAttempted;
                    let host_ptr = self as *mut JsContextHost;
                    let _ =
                        custom_elements::construct_parser_created_autonomous_element_from_handoff(
                            scope, host_ptr, &handoff,
                        );
                    custom_elements::flush_parser_custom_element_handoff_replacements(
                        scope, host_ptr,
                    );
                }
                LiveDocumentParserStepOutcome::ScriptHandoff(handoff) => {
                    let (mode, script) = match *handoff {
                        ParserScriptHandoff::BlockingClassic { script, .. } => {
                            (PopupStreamScriptMode::Blocking, script)
                        }
                        ParserScriptHandoff::AsyncPostParse { script, .. } => {
                            (PopupStreamScriptMode::Async, script)
                        }
                        ParserScriptHandoff::NonAsyncPostParse { script, .. } => {
                            (PopupStreamScriptMode::Deferred, script)
                        }
                        other => {
                            crate::host::apply_parser_script_element_state_without_execution(
                                self.dom_host_mut(),
                                &other,
                            );
                            continue;
                        }
                    };
                    // Preparation starts the element even when CSP prevents
                    // execution or the script waits in the deferred queue.
                    self.dom_host_mut()
                        .set_script_already_started(script.node_id, true);
                    if script.kind != ScriptKind::Classic
                        || !self.lightweight_popup_scripting_enabled(popup_id)
                    {
                        continue;
                    }
                    if mode == PopupStreamScriptMode::Deferred {
                        self.lightweight_popup_document_record_mut(popup_id)
                            .expect("current stream")
                            .stream
                            .as_mut()
                            .expect("current stream")
                            .deferred
                            .push_back(script);
                    } else if self
                        .run_popup_document_stream_script(scope, task, mode, script, activity)
                        && mode == PopupStreamScriptMode::Blocking
                    {
                        return;
                    }
                }
                LiveDocumentParserStepOutcome::BlockingStylesheetPause(_) => {}
                LiveDocumentParserStepOutcome::ScriptPreparation(_) => unreachable!(),
            }
        }
    }

    fn run_popup_document_stream_script(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
        mode: PopupStreamScriptMode,
        script: PreparedScript,
        activity: &mut PopupDocumentLoadBodyActivity,
    ) -> bool {
        if !self.popup_document_stream_is_current(task) {
            return false;
        }
        let record = self
            .lightweight_popup_document_record(task.popup_id())
            .expect("current stream");
        let handle = record.handle.expect("parser Document");
        let script_handle = script.node_id;
        let continuation = LightweightPopupClassicScriptContinuation {
            task,
            document_handle: handle,
            document_url: record.url.clone(),
            parser_steps: vec![LightweightPopupParserStep::Script(script.node_id)],
            next_step_index: 0,
            stream_script: Some((mode, script)),
            response_content_security_policies: record
                .state
                .policy_container
                .response_content_security_policies
                .clone(),
            response_content_security_report_only_policies: record
                .state
                .policy_container
                .response_content_security_report_only_policies
                .clone(),
            response_content_security_reporting_endpoints: record
                .state
                .policy_container
                .content_security_reporting_endpoints
                .clone(),
        };
        let advance =
            self.advance_lightweight_popup_classic_scripts(scope, continuation, *activity);
        let pending = match advance {
            LightweightPopupClassicScriptAdvance::Completed(executed) => {
                *activity = executed;
                false
            }
            LightweightPopupClassicScriptAdvance::Pending(executed) => {
                *activity = executed;
                true
            }
        };
        if pending && self.popup_document_stream_is_current(task) {
            let stream = self
                .lightweight_popup_document_record_mut(task.popup_id())
                .expect("current stream")
                .stream
                .as_mut()
                .expect("current stream");
            match mode {
                PopupStreamScriptMode::Blocking => {
                    stream.parser.as_mut().expect("blocking parser").suspend(
                        ParserSuspensionCause::ParserClassicSource {
                            script: script_handle,
                        },
                    );
                }
                PopupStreamScriptMode::Async => stream.async_pending += 1,
                PopupStreamScriptMode::Deferred => stream.deferred_pending = true,
            }
        }
        pending
    }

    fn finish_popup_document_stream(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
        activity: &mut PopupDocumentLoadBodyActivity,
    ) {
        if !self.popup_document_stream_is_current(task) {
            return;
        }
        let stream = self
            .lightweight_popup_document_record(task.popup_id())
            .and_then(|record| record.stream.as_ref())
            .expect("current stream");
        let finite = stream.control.lifetime() == DocumentParserLifetime::Finite;
        if finite && stream.parser_finished() {
            // The enclosing resource task must complete its script checkpoint
            // before publishing interactive and DOMContentLoaded.
            return;
        }
        if finite {
            // Deferred scripts need interactive readiness while this resource
            // task still owns the parser. Settle the last blocking script's
            // reactions before entering that lifecycle boundary.
            let _ = crate::script_cleanup::perform_parser_script_preparation_checkpoint(scope);
            if !self.popup_document_stream_is_current(task) {
                return;
            }
        }
        if let Some(window) = self.current_popup_window_event_target(task.popup_id()) {
            *activity = PopupDocumentLoadBodyActivity::PageCodeOrEventDispatchAttempted;
            self.dispatch_lightweight_popup_document_readiness(
                scope,
                window,
                DocumentReadyState::Interactive,
            );
        }
        if !self.popup_document_stream_is_current(task) {
            return;
        }
        loop {
            let stream = self
                .lightweight_popup_document_record_mut(task.popup_id())
                .expect("current stream")
                .stream
                .as_mut()
                .expect("current stream");
            if stream.parser.is_some() || stream.deferred_pending {
                return;
            }
            let Some(script) = stream.deferred.pop_front() else {
                break;
            };
            if self.run_popup_document_stream_script(
                scope,
                task,
                PopupStreamScriptMode::Deferred,
                script,
                activity,
            ) {
                return;
            }
            if !self.popup_document_stream_is_current(task) {
                return;
            }
        }
        if finite {
            return;
        }
        if let Some(completion) = self.prepare_lightweight_popup_parser_completion(task)
            && self.begin_lightweight_popup_interactive(scope, completion)
        {
            self.finish_lightweight_popup_interactive(completion);
        }
        if self.popup_document_stream_is_current(task)
            && let Some(window) = self.current_popup_window_event_target(task.popup_id())
        {
            self.dispatch_lightweight_popup_document_readiness(
                scope,
                window,
                DocumentReadyState::Complete,
            );
        }
    }

    pub(super) fn resume_popup_document_stream_script(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        task: LightweightPopupNavigationTaskToken,
        mode: PopupStreamScriptMode,
    ) -> PopupDocumentLoadBodyActivity {
        let mut activity = PopupDocumentLoadBodyActivity::NoPageCodeOrEventDispatch;
        if !self.popup_document_stream_is_current(task) {
            return activity;
        }
        let stream = self
            .lightweight_popup_document_record_mut(task.popup_id())
            .expect("current stream")
            .stream
            .as_mut()
            .expect("current stream");
        match mode {
            PopupStreamScriptMode::Blocking => {
                activity = self.pump_popup_document_stream(scope, task, None, false);
            }
            PopupStreamScriptMode::Deferred => {
                stream.deferred_pending = false;
                self.finish_popup_document_stream(scope, task, &mut activity);
            }
            PopupStreamScriptMode::Async => {
                stream.async_pending -= 1;
                self.publish_lightweight_popup_load_event_if_ready(task.popup_id());
            }
        }
        activity
    }

    pub(super) fn prepare_popup_document_stream_script_resume(
        &mut self,
        task: LightweightPopupNavigationTaskToken,
        mode: PopupStreamScriptMode,
    ) -> bool {
        if !self.popup_document_stream_is_current(task) {
            return false;
        }
        if mode == PopupStreamScriptMode::Blocking {
            let parser = self
                .lightweight_popup_document_record_mut(task.popup_id())
                .expect("current stream")
                .stream
                .as_mut()
                .expect("current stream")
                .parser
                .as_mut()
                .expect("blocking parser");
            let Some(permit) = parser.current_resume_permit() else {
                return false;
            };
            return parser.resume(permit);
        }
        true
    }
}
