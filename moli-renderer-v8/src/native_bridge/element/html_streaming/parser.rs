use std::{collections::VecDeque, mem};

use moli_parser::{HtmlFragmentStream, HtmlFragmentStreamAbortHandle, ParserPumpStep, ParserYield};
use moli_webapi::{
    dom_exception::new_dom_exception_value, streams::adapter::error_writable_stream_with_value,
};

use crate::{
    document_runtime::DomHandle,
    native_bridge::{PageScriptHost, WindowDocumentTaskTarget},
};

use super::{FragmentStream, scripts::FragmentScriptJobs};

pub(super) struct FragmentStreamOwner {
    pub(super) host: *mut PageScriptHost,
    pub(super) target: DomHandle,
    pub(super) document: DomHandle,
    reference: Option<DomHandle>,
    pub(super) script_target: Option<WindowDocumentTaskTarget>,
}

impl FragmentStreamOwner {
    pub(super) fn new(
        runtime: &PageScriptHost,
        host: *mut PageScriptHost,
        target: DomHandle,
        reference: Option<DomHandle>,
    ) -> Option<Self> {
        Some(Self {
            host,
            target,
            document: runtime.inner_html_document_handle(target)?,
            reference,
            script_target: runtime
                .owner_dispatch_scope_for_node(target)
                .and_then(|dispatch| {
                    runtime.current_window_document_task_target_for_dispatch_scope(dispatch)
                }),
        })
    }
}

enum FragmentParserPhase {
    Open,
    Closing(v8::Global<v8::PromiseResolver>),
    Closed,
    Aborted,
}

pub(super) struct FragmentStreamProgress {
    parser: Option<HtmlFragmentStream>,
    parser_abort: HtmlFragmentStreamAbortHandle,
    phase: FragmentParserPhase,
    queued_chunks: VecDeque<String>,
    leading_surrogate: Option<u16>,
    running: bool,
    pub(super) scripts: FragmentScriptJobs,
    pub(super) scripts_retired: bool,
}

impl FragmentStreamProgress {
    pub(super) fn new(parser: HtmlFragmentStream) -> Self {
        Self {
            parser_abort: parser.abort_handle(),
            parser: Some(parser),
            phase: FragmentParserPhase::Open,
            queued_chunks: VecDeque::new(),
            leading_surrogate: None,
            running: false,
            scripts: FragmentScriptJobs::default(),
            scripts_retired: false,
        }
    }

    pub(super) fn has_tasks(&self) -> bool {
        self.running || self.scripts.has_tasks()
    }

    pub(super) fn is_closed(&self) -> bool {
        matches!(self.phase, FragmentParserPhase::Closed)
    }

    pub(super) fn is_aborted(&self) -> bool {
        matches!(self.phase, FragmentParserPhase::Aborted)
    }

    pub(super) fn append_chunk(&mut self, mut chunk: Vec<u16>) {
        if !matches!(self.phase, FragmentParserPhase::Open) {
            return;
        }
        if let Some(leading) = self.leading_surrogate.take() {
            chunk.insert(0, leading);
        }
        // A DOMString chunk can end between the code units of one character.
        if chunk
            .last()
            .is_some_and(|unit| (0xd800..=0xdbff).contains(unit))
        {
            self.leading_surrogate = chunk.pop();
        }
        self.queued_chunks
            .push_back(String::from_utf16_lossy(&chunk));
    }

    pub(super) fn start_close(&mut self, resolver: v8::Global<v8::PromiseResolver>) {
        debug_assert!(matches!(self.phase, FragmentParserPhase::Open));
        if self.leading_surrogate.take().is_some() {
            self.queued_chunks.push_back("\u{fffd}".to_owned());
        }
        self.phase = FragmentParserPhase::Closing(resolver);
    }
}

enum FragmentParserAction {
    Append(String),
    Pump,
    Finish,
    Pause,
    Abort,
}

impl FragmentStream {
    pub(super) fn reference_is_valid(&self) -> bool {
        self.owner.reference.is_none_or(|reference| {
            unsafe { &*self.owner.host }.parent_node(reference) == Some(self.owner.target)
        })
    }

    pub(super) fn invalid_reference<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> v8::Local<'s, v8::Value> {
        let error = new_dom_exception_value(
            scope,
            "The reference child is no longer a child of the HTML stream's target.",
            "HierarchyRequestError",
        );
        if let Some(writable) = self.writable.to_local(scope) {
            error_writable_stream_with_value(scope, writable, error);
        }
        self.abort_parser(scope, Some(error));
        error
    }

    pub(super) fn abort_parser<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        reason: Option<v8::Local<'s, v8::Value>>,
    ) {
        let (parser, phase) = {
            let mut progress = self.progress.borrow_mut();
            progress.parser_abort.request();
            progress.queued_chunks.clear();
            progress.leading_surrogate = None;
            progress.scripts.abort_parser();
            (
                progress.parser.take(),
                mem::replace(&mut progress.phase, FragmentParserPhase::Aborted),
            )
        };
        if let FragmentParserPhase::Closing(close) = phase
            && let Some(reason) = reason
        {
            let close = v8::Local::new(scope, &close);
            let _ = close.reject(scope, reason);
        }
        if let Some(parser) = parser {
            self.abort_checked_out_parser(scope, parser);
        }
    }

    fn abort_checked_out_parser(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        parser: HtmlFragmentStream,
    ) {
        let host_ptr = self.owner.host;
        // A sink can outlive the Window that created it. Its stored pointer is
        // only an identity until the entered realm supplies a live host.
        if crate::util::context_host_ptr_from_context_slot(scope.get_current_context())
            == Some(host_ptr)
        {
            unsafe { &mut *host_ptr }.abort_html_fragment_stream(scope, host_ptr, parser);
        }
    }

    fn next_parser_action(&self, parser: &HtmlFragmentStream) -> FragmentParserAction {
        let mut progress = self.progress.borrow_mut();
        if progress.is_aborted() {
            FragmentParserAction::Abort
        } else if progress.scripts.is_blocked() {
            FragmentParserAction::Pause
        } else if let Some(chunk) = progress.queued_chunks.pop_front() {
            FragmentParserAction::Append(chunk)
        } else if parser.has_pending_input() {
            FragmentParserAction::Pump
        } else if matches!(progress.phase, FragmentParserPhase::Closing(_)) {
            FragmentParserAction::Finish
        } else {
            FragmentParserAction::Pause
        }
    }

    pub(super) fn run_fragment_parser<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
    ) -> Result<(), v8::Local<'s, v8::Value>> {
        let parser = {
            let mut progress = self.progress.borrow_mut();
            if progress.running || progress.scripts.is_blocked() {
                return Ok(());
            }
            let Some(parser) = progress.parser.take() else {
                return Ok(());
            };
            progress.running = true;
            parser
        };
        let result = self.pump_fragment_parser(scope, parser);
        self.progress.borrow_mut().running = false;
        result
    }

    fn pump_fragment_parser<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        parser: HtmlFragmentStream,
    ) -> Result<(), v8::Local<'s, v8::Value>> {
        let host_ptr = self.owner.host;
        loop {
            let next = self.next_parser_action(&parser);
            let outcome = match next {
                FragmentParserAction::Append(chunk) => unsafe { &mut *host_ptr }
                    .enqueue_html_fragment_stream(scope, host_ptr, &parser, chunk),
                FragmentParserAction::Pump => {
                    self.pump_fragment_parser_step(scope, host_ptr, &parser)
                }
                FragmentParserAction::Finish => {
                    return self.finish_fragment_parser(scope, parser);
                }
                FragmentParserAction::Pause => {
                    self.progress.borrow_mut().parser = Some(parser);
                    return Ok(());
                }
                FragmentParserAction::Abort => {
                    self.abort_checked_out_parser(scope, parser);
                    return Ok(());
                }
            };
            if outcome.is_err() {
                let error = self.invalid_reference(scope);
                self.abort_checked_out_parser(scope, parser);
                return Err(error);
            }
        }
    }

    fn pump_fragment_parser_step(
        &self,
        scope: &mut v8::PinScope<'_, '_>,
        host_ptr: *mut PageScriptHost,
        parser: &HtmlFragmentStream,
    ) -> Result<(), moli_parser::InvalidFragmentStreamReference> {
        let result =
            unsafe { &mut *host_ptr }.pump_html_fragment_stream(scope, host_ptr, parser)?;
        match result {
            ParserPumpStep::Yield(ParserYield::ScriptPreparation(request)) => {
                if request.needs_microtask_checkpoint()
                    && let Err(error) =
                        crate::script_cleanup::perform_parser_script_preparation_checkpoint(scope)
                {
                    tracing::debug!(%error, "streamed script preparation checkpoint did not finish");
                }
                if self.progress.borrow().is_aborted() || scope.is_execution_terminating() {
                    self.abort_parser(scope, None);
                    return Ok(());
                }
                let handoff = unsafe { &mut *host_ptr }
                    .prepare_html_fragment_script(scope, host_ptr, parser, *request);
                self.handle_script_handoff(scope, handoff);
            }
            ParserPumpStep::Yield(ParserYield::Script(handoff)) => {
                self.handle_script_handoff(scope, *handoff);
            }
            // Live fragment construction already handles synchronous custom element
            // reactions. The fragment never shares the live Document's blockers.
            ParserPumpStep::Yield(ParserYield::CustomElementConstruction(_))
            | ParserPumpStep::InputDrained => {}
            ParserPumpStep::Yield(ParserYield::OwnerInterrupted) => {
                debug_assert!(self.progress.borrow().is_aborted());
            }
            ParserPumpStep::Yield(ParserYield::BlockingStylesheet(_)) => {
                unreachable!("a live fragment parser has no document parser blockers");
            }
        }
        Ok(())
    }

    fn finish_fragment_parser<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        parser: HtmlFragmentStream,
    ) -> Result<(), v8::Local<'s, v8::Value>> {
        let host_ptr = self.owner.host;
        if unsafe { &mut *host_ptr }
            .close_html_fragment_stream(scope, host_ptr, parser)
            .is_err()
        {
            return Err(self.invalid_reference(scope));
        }
        let close = {
            let mut progress = self.progress.borrow_mut();
            if !matches!(progress.phase, FragmentParserPhase::Closing(_)) {
                return Ok(());
            }
            match mem::replace(&mut progress.phase, FragmentParserPhase::Closed) {
                FragmentParserPhase::Closing(close) => close,
                _ => unreachable!(),
            }
        };
        let close = v8::Local::new(scope, &close);
        let value = v8::undefined(scope);
        let _ = close.resolve(scope, value.into());
        self.queue_next_deferred_script();
        Ok(())
    }
}
