#[cfg(test)]
use serde_json::Value;
use serde_json::json;

use crate::domains::command_output::CommandOutputPlan;

use super::*;

pub struct PendingCdpCommandDispatch {
    inner: PendingCdpCommandDispatchKind,
    owner_scope: Option<CommandOwnerScope>,
    scheduler_events: Vec<CdpSchedulerEvent>,
}

pub struct CompletedCdpCommandDispatch {
    inner: CompletedCdpCommandDispatchKind,
    owner_scope: Option<CommandOwnerScope>,
}

pub enum CdpCommandTaskStep {
    Pending(Box<PendingCdpCommandDispatch>),
    Complete(CdpRendererOwnerTurnOutcome),
}

impl CdpCommandTaskStep {
    #[cfg(test)]
    pub fn into_parts(self) -> (Vec<Value>, Vec<CdpSchedulerEvent>) {
        match self {
            Self::Complete(outcome) => outcome.into_parts(),
            Self::Pending(_) => {
                panic!("pending CDP command task step cannot be converted into final output")
            }
        }
    }
}

enum PendingCdpCommandDispatchKind {
    Runtime(Box<crate::domains::runtime::PendingRuntimeCommandDispatch>),
    Autofill(crate::domains::autofill::PendingAutofillCommandDispatch),
    Accessibility(crate::domains::accessibility::PendingAccessibilityCommandDispatch),
    Input(crate::domains::input::PendingInputCommandDispatch),
    Css(crate::domains::css::PendingCssCommandDispatch),
    Dom(crate::domains::dom::PendingDomCommandDispatch),
    DomDebugger(crate::domains::dom_debugger::PendingDomDebuggerCommandDispatch),
    DomStorage(Box<crate::domains::dom_storage::PendingDomStorageCommandDispatch>),
    DomSnapshot(crate::domains::dom_snapshot::PendingDomSnapshotCommandDispatch),
    Page(crate::domains::page::PendingPageCommandDispatch),
    Performance(crate::domains::performance::PendingPerformanceCommandDispatch),
    Emulation(crate::domains::emulation::PendingEmulationCommandDispatch),
    Storage(crate::domains::storage::PendingStorageCommandDispatch),
    Network(crate::domains::network::PendingNetworkCommandDispatch),
    Fetch(crate::domains::fetch::PendingFetchCommandDispatch),
    Io(Box<crate::domains::io::PendingIoCommandDispatch>),
    Security(crate::domains::security::PendingSecurityCommandDispatch),
    Browser(crate::domains::browser::PendingBrowserCommandDispatch),
    Target(crate::domains::target::PendingTargetCommandDispatch),
    Tracing(crate::domains::tracing::PendingTracingCommandDispatch),
}

enum CompletedCdpCommandDispatchKind {
    Runtime(Box<crate::domains::runtime::CompletedRuntimeCommandDispatch>),
    Autofill(crate::domains::autofill::CompletedAutofillCommandDispatch),
    Accessibility(crate::domains::accessibility::CompletedAccessibilityCommandDispatch),
    Input(crate::domains::input::CompletedInputCommandDispatch),
    Css(crate::domains::css::CompletedCssCommandDispatch),
    Dom(crate::domains::dom::CompletedDomCommandDispatch),
    DomDebugger(crate::domains::dom_debugger::CompletedDomDebuggerCommandDispatch),
    DomStorage(Box<crate::domains::dom_storage::CompletedDomStorageCommandDispatch>),
    DomSnapshot(crate::domains::dom_snapshot::CompletedDomSnapshotCommandDispatch),
    Page(crate::domains::page::CompletedPageCommandDispatch),
    Performance(crate::domains::performance::CompletedPerformanceCommandDispatch),
    Emulation(crate::domains::emulation::CompletedEmulationCommandDispatch),
    Storage(crate::domains::storage::CompletedStorageCommandDispatch),
    Network(crate::domains::network::CompletedNetworkCommandDispatch),
    Fetch(crate::domains::fetch::CompletedFetchCommandDispatch),
    Io(Box<crate::domains::io::CompletedIoCommandDispatch>),
    Security(crate::domains::security::CompletedSecurityCommandDispatch),
    Browser(crate::domains::browser::CompletedBrowserCommandDispatch),
    Target(crate::domains::target::CompletedTargetCommandDispatch),
    Tracing(crate::domains::tracing::CompletedTracingCommandDispatch),
}

impl PendingCdpCommandDispatchKind {
    fn name(&self) -> &'static str {
        match self {
            Self::Runtime(_) => "Runtime",
            Self::Autofill(_) => "Autofill",
            Self::Accessibility(_) => "Accessibility",
            Self::Input(_) => "Input",
            Self::Css(_) => "CSS",
            Self::Dom(_) => "DOM",
            Self::DomDebugger(_) => "DOMDebugger",
            Self::DomStorage(_) => "DOMStorage",
            Self::DomSnapshot(_) => "DOMSnapshot",
            Self::Page(_) => "Page",
            Self::Performance(_) => "Performance",
            Self::Emulation(_) => "Emulation",
            Self::Storage(_) => "Storage",
            Self::Network(_) => "Network",
            Self::Fetch(_) => "Fetch",
            Self::Io(_) => "IO",
            Self::Security(_) => "Security",
            Self::Browser(_) => "Browser",
            Self::Target(_) => "Target",
            Self::Tracing(_) => "Tracing",
        }
    }

    fn owner_scope_capture_session_id(&self) -> Option<Option<&str>> {
        match self {
            Self::Autofill(pending) => Some(pending.session_id()),
            Self::Accessibility(pending) => Some(pending.session_id()),
            Self::Input(pending) => Some(pending.session_id()),
            Self::Css(pending) => Some(pending.session_id()),
            Self::Dom(pending) => Some(pending.session_id()),
            Self::DomDebugger(pending) => Some(pending.session_id()),
            Self::DomSnapshot(pending) => Some(pending.session_id()),
            Self::Performance(pending) => Some(pending.session_id()),
            Self::Tracing(pending) => Some(pending.session_id()),
            Self::Runtime(_)
            | Self::DomStorage(_)
            | Self::Page(_)
            | Self::Emulation(_)
            | Self::Storage(_)
            | Self::Fetch(_)
            | Self::Io(_)
            | Self::Security(_)
            | Self::Browser(_)
            | Self::Target(_) => None,
            Self::Network(pending) => Some(pending.session_id()),
        }
    }
}

impl CompletedCdpCommandDispatchKind {
    fn name(&self) -> &'static str {
        match self {
            Self::Runtime(_) => "Runtime",
            Self::Autofill(_) => "Autofill",
            Self::Accessibility(_) => "Accessibility",
            Self::Input(_) => "Input",
            Self::Css(_) => "CSS",
            Self::Dom(_) => "DOM",
            Self::DomDebugger(_) => "DOMDebugger",
            Self::DomStorage(_) => "DOMStorage",
            Self::DomSnapshot(_) => "DOMSnapshot",
            Self::Page(_) => "Page",
            Self::Performance(_) => "Performance",
            Self::Emulation(_) => "Emulation",
            Self::Storage(_) => "Storage",
            Self::Network(_) => "Network",
            Self::Fetch(_) => "Fetch",
            Self::Io(_) => "IO",
            Self::Security(_) => "Security",
            Self::Browser(_) => "Browser",
            Self::Target(_) => "Target",
            Self::Tracing(_) => "Tracing",
        }
    }
}

impl PendingCdpCommandDispatch {
    fn new(
        conn: &CdpConnection,
        inner: PendingCdpCommandDispatchKind,
        scheduler_events: Vec<CdpSchedulerEvent>,
    ) -> Self {
        let owner_scope = inner
            // These domain-local session ids are still used for CDP response
            // routing and session-local lookups. Target ownership for their
            // pending completion is captured here by the wrapper.
            .owner_scope_capture_session_id()
            .map(|session_id| CommandOwnerScope::capture(conn, session_id));
        Self {
            inner,
            owner_scope,
            scheduler_events,
        }
    }

    pub fn take_scheduler_events(&mut self) -> Vec<CdpSchedulerEvent> {
        std::mem::take(&mut self.scheduler_events)
    }

    pub fn kind_name(&self) -> &'static str {
        self.inner.name()
    }

    #[cfg(test)]
    pub(crate) fn hold_input_renderer_ack_for_test(&mut self) -> bool {
        match &mut self.inner {
            PendingCdpCommandDispatchKind::Input(pending) => pending.hold_renderer_ack_for_test(),
            _ => false,
        }
    }

    pub fn command_id(&self) -> Option<u64> {
        match &self.inner {
            PendingCdpCommandDispatchKind::Runtime(pending) => pending.command_id(),
            _ => None,
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        match &self.inner {
            PendingCdpCommandDispatchKind::Runtime(pending) => pending.session_id(),
            _ => None,
        }
    }

    pub fn runtime_command_executes_page_javascript(&self) -> bool {
        match &self.inner {
            PendingCdpCommandDispatchKind::Runtime(pending) => pending.executes_page_javascript(),
            _ => false,
        }
    }

    pub fn runtime_deferred_reply_page_owner_access_allowed(&self) -> bool {
        match &self.inner {
            PendingCdpCommandDispatchKind::Runtime(pending) => {
                pending.deferred_reply_page_owner_access_allowed()
            }
            _ => false,
        }
    }

    pub fn waits_for_scheduler_deferred_inspector_reply(&self) -> bool {
        match &self.inner {
            PendingCdpCommandDispatchKind::Runtime(pending) => {
                pending.waits_for_scheduler_deferred_inspector_reply()
            }
            _ => false,
        }
    }

    pub fn take_scheduler_deferred_inspector_reply_events(
        &mut self,
    ) -> Vec<BackgroundProtocolEvent> {
        match &mut self.inner {
            PendingCdpCommandDispatchKind::Runtime(pending) => {
                pending.take_scheduler_deferred_inspector_reply_events()
            }
            _ => Vec::new(),
        }
    }

    pub fn take_scheduler_deferred_inspector_reply_receiver(
        &mut self,
    ) -> Option<crate::conn::RuntimeInspectorAsyncCompletionReceiver> {
        match &mut self.inner {
            PendingCdpCommandDispatchKind::Runtime(pending) => {
                pending.take_scheduler_deferred_inspector_reply_receiver()
            }
            _ => None,
        }
    }

    pub async fn route_scheduler_deferred_inspector_response(
        &mut self,
        conn: &mut CdpConnection,
        response: RuntimeInspectorResponseReady,
    ) -> bool {
        match &mut self.inner {
            PendingCdpCommandDispatchKind::Runtime(pending) => {
                pending
                    .route_scheduler_deferred_inspector_response(conn, response)
                    .await
            }
            _ => false,
        }
    }

    pub fn complete_scheduler_deferred_inspector_reply(
        self,
        conn: &mut CdpConnection,
    ) -> CompletedCdpCommandDispatch {
        match self.inner {
            PendingCdpCommandDispatchKind::Runtime(pending) => CompletedCdpCommandDispatch {
                inner: CompletedCdpCommandDispatchKind::Runtime(Box::new(
                    pending.complete_scheduler_deferred_inspector_reply(conn),
                )),
                owner_scope: None,
            },
            _ => {
                unreachable!(
                    "only Runtime commands can wait for scheduler deferred inspector reply"
                )
            }
        }
    }

    pub fn forget_scheduler_deferred_inspector_reply(self, conn: &mut CdpConnection) {
        if let PendingCdpCommandDispatchKind::Runtime(pending) = self.inner {
            pending.forget_scheduler_deferred_inspector_reply(conn);
        }
    }

    pub async fn wait(self) -> CompletedCdpCommandDispatch {
        let kind = self.inner.name();
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(std::time::Instant::now);
        if trace_started.is_some() {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "command_pending_domain_wait_start",
                pending_kind = kind,
            );
        }
        let inner = match self.inner {
            PendingCdpCommandDispatchKind::Runtime(pending) => {
                CompletedCdpCommandDispatchKind::Runtime(Box::new(Box::pin(pending.wait()).await))
            }
            PendingCdpCommandDispatchKind::Autofill(pending) => {
                CompletedCdpCommandDispatchKind::Autofill(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Accessibility(pending) => {
                CompletedCdpCommandDispatchKind::Accessibility(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Input(pending) => {
                CompletedCdpCommandDispatchKind::Input(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Css(pending) => {
                CompletedCdpCommandDispatchKind::Css(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Dom(pending) => {
                CompletedCdpCommandDispatchKind::Dom(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::DomDebugger(pending) => {
                CompletedCdpCommandDispatchKind::DomDebugger(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::DomStorage(pending) => {
                CompletedCdpCommandDispatchKind::DomStorage(Box::new(
                    Box::pin(pending.wait()).await,
                ))
            }
            PendingCdpCommandDispatchKind::DomSnapshot(pending) => {
                CompletedCdpCommandDispatchKind::DomSnapshot(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Page(pending) => {
                CompletedCdpCommandDispatchKind::Page(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Performance(pending) => {
                CompletedCdpCommandDispatchKind::Performance(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Emulation(pending) => {
                CompletedCdpCommandDispatchKind::Emulation(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Storage(pending) => {
                CompletedCdpCommandDispatchKind::Storage(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Network(pending) => {
                CompletedCdpCommandDispatchKind::Network(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Fetch(pending) => {
                CompletedCdpCommandDispatchKind::Fetch(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Io(pending) => {
                CompletedCdpCommandDispatchKind::Io(Box::new(Box::pin(pending.wait()).await))
            }
            PendingCdpCommandDispatchKind::Security(pending) => {
                CompletedCdpCommandDispatchKind::Security(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Browser(pending) => {
                CompletedCdpCommandDispatchKind::Browser(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Target(pending) => {
                CompletedCdpCommandDispatchKind::Target(Box::pin(pending.wait()).await)
            }
            PendingCdpCommandDispatchKind::Tracing(pending) => {
                CompletedCdpCommandDispatchKind::Tracing(Box::pin(pending.wait()).await)
            }
        };
        if let Some(started) = trace_started {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "command_pending_domain_wait_done",
                pending_kind = kind,
                elapsed_us = %started.elapsed().as_micros(),
            );
        }
        CompletedCdpCommandDispatch {
            inner,
            owner_scope: self.owner_scope,
        }
    }
}

impl CompletedCdpCommandDispatch {
    pub fn kind_name(&self) -> &'static str {
        self.inner.name()
    }

    pub fn runtime_command_page_owner_access_allowed(&self) -> Option<bool> {
        match &self.inner {
            CompletedCdpCommandDispatchKind::Runtime(completed) => {
                Some(completed.page_owner_access_allowed())
            }
            _ => None,
        }
    }
}

fn append_command_output_plan(
    out: &mut Vec<BackgroundProtocolEvent>,
    command_context: &mut CommandDispatchContext,
    mut plan: CommandOutputPlan,
    command_id: Option<u64>,
    session_id: Option<&str>,
) {
    if let Some(predecessor) = plan.take_renderer_output_predecessor() {
        command_context.set_renderer_output_predecessor(predecessor);
    }
    let (events, renderer_output_boundary, post_renderer_output_events, post_response_events) =
        plan.into_renderer_fenced_background_and_post_response_events(command_id, session_id);
    command_context
        .protocol_events_mut()
        .extend(std::mem::take(out));
    command_context.append_renderer_fenced_protocol_events(
        events,
        renderer_output_boundary,
        post_renderer_output_events,
    );
    command_context.extend_post_response_events(post_response_events);
}

impl CdpConnection {
    fn complete_with_protocol_events(
        &mut self,
        command_context: &mut CommandDispatchContext,
        protocol_events: Vec<BackgroundProtocolEvent>,
    ) -> CdpCommandTaskStep {
        // `CommandDispatchContext::protocol_events` contains concrete output
        // explicitly classified as preceding this command's response. Keep
        // that ordering at the one common completion boundary instead of
        // asking each domain handler to splice its response manually.
        command_context
            .protocol_events_mut()
            .extend(protocol_events);
        let (protocol_events, renderer_output_boundary, post_renderer_output_events) =
            command_context.take_renderer_fenced_protocol_events();
        let post_response_events = command_context.take_post_response_events();
        self.record_tracing_protocol_events(&protocol_events);
        self.record_tracing_protocol_events(&post_renderer_output_events);
        self.record_tracing_protocol_events(&post_response_events);
        CdpCommandTaskStep::Complete(
            CdpTurnOutcome::new_with_protocol_and_post_response_events(
                protocol_events,
                post_response_events,
                self.take_scheduler_events(),
            )
            .with_renderer_output_boundary(renderer_output_boundary, post_renderer_output_events)
            .with_renderer_output_predecessor(command_context.take_renderer_output_predecessor()),
        )
    }

    fn complete_with_output_plan(
        &mut self,
        command_context: &mut CommandDispatchContext,
        plan: CommandOutputPlan,
        command_id: Option<u64>,
        session_id: Option<&str>,
    ) -> CdpCommandTaskStep {
        let mut protocol_events = Vec::new();
        append_command_output_plan(
            &mut protocol_events,
            command_context,
            plan,
            command_id,
            session_id,
        );
        self.complete_with_protocol_events(command_context, protocol_events)
    }

    fn pending_step(&mut self, inner: PendingCdpCommandDispatchKind) -> CdpCommandTaskStep {
        let scheduler_events = self.take_scheduler_events();
        CdpCommandTaskStep::Pending(Box::new(PendingCdpCommandDispatch::new(
            self,
            inner,
            scheduler_events,
        )))
    }

    #[cfg(test)]
    pub(crate) fn try_start_pending_command_dispatch(
        &mut self,
        raw: &str,
    ) -> Option<PendingCdpCommandDispatch> {
        match self.start_command_dispatch(raw) {
            CdpCommandTaskStep::Pending(pending) => Some(*pending),
            CdpCommandTaskStep::Complete(_) => None,
        }
    }

    /// Compatibility/test entry point for direct command dispatch.
    ///
    /// The WebSocket scheduler must use `start_parsed_command_dispatch_with_context`
    /// so pending command continuations keep the same command-local output
    /// buffers and response-flush gate across await points.
    #[cfg(test)]
    pub(crate) fn start_command_dispatch(&mut self, raw: &str) -> CdpCommandTaskStep {
        let command = match ParsedCdpCommand::parse_str(raw.to_owned()) {
            Ok(command) => command,
            Err(error) => {
                let mut command_context = CommandDispatchContext::default();
                return self.complete_with_output_plan(
                    &mut command_context,
                    CommandOutputPlan::error_without_session(
                        error.response_code(),
                        error.response_message(),
                    ),
                    error.command_id(),
                    None,
                );
            }
        };
        self.start_parsed_command_dispatch(&command)
    }

    #[cfg(test)]
    pub(crate) fn start_parsed_command_dispatch(
        &mut self,
        command: &ParsedCdpCommand,
    ) -> CdpCommandTaskStep {
        let mut command_context = CommandDispatchContext::default();
        self.start_parsed_command_dispatch_with_context(command, &mut command_context)
    }

    pub fn start_parsed_command_dispatch_with_context(
        &mut self,
        command: &ParsedCdpCommand,
        command_context: &mut CommandDispatchContext,
    ) -> CdpCommandTaskStep {
        let req = command.request();
        let Some(dot) = req.method().find('.') else {
            return self.complete_with_output_plan(
                command_context,
                CommandOutputPlan::error(-32600, "Invalid method"),
                Some(req.id()),
                req.session_id(),
            );
        };
        let domain = &req.method()[..dot];
        if req.session_id() == Some("STARTUP") {
            return self.complete_with_output_plan(
                command_context,
                startup_output_plan(req),
                Some(req.id()),
                req.session_id(),
            );
        }
        if let Some(session_id) = req.session_id()
            && self.session_route(Some(session_id)).is_none()
        {
            return self.complete_with_output_plan(
                command_context,
                CommandOutputPlan::error(-32001, "Unknown sessionId"),
                Some(req.id()),
                Some(session_id),
            );
        }
        let cmd = Cmd::from_parsed(command)
            .expect("validated domain-qualified command must produce a command view");
        self.record_tracing_command(cmd.method, cmd.session_id);
        let step = match domain {
            "Browser" => Some(
                match crate::domains::browser::try_start_browser_command_dispatch(self, &cmd) {
                    crate::domains::browser::BrowserCommandTaskStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Browser(pending))
                    }
                    crate::domains::browser::BrowserCommandTaskStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                },
            ),
            "Runtime" => crate::domains::runtime::try_start_runtime_command_dispatch(self, &cmd)
            .map(|step| match step {
                    crate::domains::runtime::RuntimeCommandTaskStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Runtime(pending))
                    }
                    crate::domains::runtime::RuntimeCommandTaskStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                }),
            "HeapProfiler" => {
                crate::domains::heap_profiler::try_start_heap_profiler_command_dispatch(
                    self,
                    &cmd,
                )
                    .map(|step| match step {
                        crate::domains::runtime::RuntimeCommandTaskStep::Pending(pending) => {
                            self.pending_step(PendingCdpCommandDispatchKind::Runtime(pending))
                        }
                        crate::domains::runtime::RuntimeCommandTaskStep::Complete(plan) => {
                            self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                        }
                    })
            }
            "Profiler" => crate::domains::profiler::try_start_profiler_command_dispatch(self, &cmd)
            .map(|step| match step {
                    crate::domains::runtime::RuntimeCommandTaskStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Runtime(pending))
                    }
                    crate::domains::runtime::RuntimeCommandTaskStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                }),
            "Debugger" => {
                crate::domains::debugger::try_start_debugger_command_dispatch(
                    self,
                    &cmd,
                )
                .map(|step| match step {
                        crate::domains::runtime::RuntimeCommandTaskStep::Pending(pending) => {
                            self.pending_step(PendingCdpCommandDispatchKind::Runtime(pending))
                        }
                        crate::domains::runtime::RuntimeCommandTaskStep::Complete(plan) => {
                            self.complete_with_output_plan(
                                command_context,
                                plan,
                                cmd.id,
                                cmd.session_id,
                            )
                        }
                    })
            }
            "Accessibility" => {
                crate::domains::accessibility::try_start_accessibility_command_dispatch(self, &cmd)
                    .map(|step| {
                        match step {
                        crate::domains::accessibility::AccessibilityCommandDispatchStep::Pending(
                            pending,
                        ) => self.pending_step(PendingCdpCommandDispatchKind::Accessibility(pending)),
                        crate::domains::accessibility::AccessibilityCommandDispatchStep::Complete(
                            plan,
                        ) => self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id),
                    }
                    })
            }
            "Input" => Some(
                match crate::domains::input::try_start_input_command_dispatch(self, &cmd) {
                    crate::domains::input::InputCommandDispatchStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Input(pending))
                    }
                    crate::domains::input::InputCommandDispatchStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                },
            ),
            "CSS" => crate::domains::css::try_start_css_command_dispatch(self, &cmd).map(|step| {
                match step {
                    crate::domains::css::CssCommandDispatchStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Css(pending))
                    }
                    crate::domains::css::CssCommandDispatchStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                }
            }),
            "DOM" => crate::domains::dom::try_start_dom_command_dispatch(self, &cmd).map(|step| {
                match step {
                    crate::domains::dom::DomCommandDispatchStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Dom(*pending))
                    }
                    crate::domains::dom::DomCommandDispatchStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                }
            }),
            "DOMStorage" => Some(
                match crate::domains::dom_storage::try_start_dom_storage_command_dispatch(
                    self, &cmd,
                ) {
                    crate::domains::dom_storage::DomStorageCommandTaskStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::DomStorage(pending))
                    }
                    crate::domains::dom_storage::DomStorageCommandTaskStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                },
            ),
            "Console" => {
                crate::domains::console::try_start_console_command_dispatch(
                    self,
                    &cmd,
                )
                .map(|step| match step {
                        crate::domains::runtime::RuntimeCommandTaskStep::Pending(pending) => {
                            self.pending_step(PendingCdpCommandDispatchKind::Runtime(pending))
                        }
                        crate::domains::runtime::RuntimeCommandTaskStep::Complete(plan) => {
                            self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                        }
                    },
                )
            }
            "Network" => Some(
                match crate::domains::network::start_network_domain_command_dispatch(self, &cmd) {
                    crate::domains::network::NetworkDomainCommandTaskStep::Network(
                        crate::domains::network::NetworkCommandTaskStep::Pending(pending),
                    ) => self.pending_step(PendingCdpCommandDispatchKind::Network(pending)),
                    crate::domains::network::NetworkDomainCommandTaskStep::Network(
                        crate::domains::network::NetworkCommandTaskStep::Complete(plan),
                    ) => self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id),
                    crate::domains::network::NetworkDomainCommandTaskStep::Storage(
                        crate::domains::storage::StorageCommandTaskStep::Pending(pending),
                    ) => self.pending_step(PendingCdpCommandDispatchKind::Storage(pending)),
                    crate::domains::network::NetworkDomainCommandTaskStep::Storage(
                        crate::domains::storage::StorageCommandTaskStep::Complete(plan),
                    ) => self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id),
                    crate::domains::network::NetworkDomainCommandTaskStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                },
            ),
            "Target" => {
                crate::domains::target::try_start_target_command_dispatch(self, &cmd).map(|step| {
                    match step {
                        crate::domains::target::TargetCommandTaskStep::Pending(pending) => {
                            self.pending_step(PendingCdpCommandDispatchKind::Target(pending))
                        }
                        crate::domains::target::TargetCommandTaskStep::Complete(plan) => {
                            self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                        }
                    }
                })
            }
            "Tracing" => Some(
                match crate::domains::tracing::try_start_tracing_command_dispatch(self, &cmd) {
                    crate::domains::tracing::TracingCommandTaskStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Tracing(*pending))
                    }
                    crate::domains::tracing::TracingCommandTaskStep::Complete(plan) => self
                        .complete_with_output_plan(
                            command_context,
                            plan,
                            cmd.id,
                            cmd.session_id,
                        ),
                },
            ),
            "Fetch" => {
                crate::domains::fetch::try_start_fetch_command_dispatch(self, &cmd).map(|step| {
                    match step {
                        crate::domains::fetch::FetchCommandTaskStep::Pending(pending) => {
                            self.pending_step(PendingCdpCommandDispatchKind::Fetch(pending))
                        }
                        crate::domains::fetch::FetchCommandTaskStep::Complete(plan) => {
                            self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                        }
                    }
                })
            }
            "Page" => {
                crate::domains::page::try_start_page_command_dispatch(self, &cmd).map(|step| {
                    match step {
                        crate::domains::page::PageCommandTaskStep::Pending(pending) => {
                            self.pending_step(PendingCdpCommandDispatchKind::Page(pending))
                        }
                        crate::domains::page::PageCommandTaskStep::Complete(plan) => {
                            self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                        }
                    }
                })
            }
            "Inspector" => {
                let plan = crate::domains::inspector::command_output_plan(self, &cmd);
                Some(self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id))
            }
            "Log" => {
                let plan = crate::domains::log::command_output_plan(self, &cmd);
                Some(self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id))
            }
            "Storage" => Some(
                match crate::domains::storage::try_start_storage_command_dispatch(self, &cmd) {
                    crate::domains::storage::StorageCommandTaskStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Storage(pending))
                    }
                    crate::domains::storage::StorageCommandTaskStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                },
            ),
            "DOMSnapshot" => {
                Some(
                    match crate::domains::dom_snapshot::try_start_dom_snapshot_command_dispatch(
                        self, &cmd,
                    ) {
                        crate::domains::dom_snapshot::DomSnapshotCommandDispatchStep::Pending(
                            pending,
                        ) => self.pending_step(PendingCdpCommandDispatchKind::DomSnapshot(pending)),
                        crate::domains::dom_snapshot::DomSnapshotCommandDispatchStep::Complete(
                            plan,
                        ) => self.complete_with_output_plan(
                            command_context,
                            plan,
                            cmd.id,
                            cmd.session_id,
                        ),
                    },
                )
            }
            "Security" => Some(
                match crate::domains::security::try_start_security_command_dispatch(self, &cmd) {
                    crate::domains::security::SecurityCommandTaskStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Security(pending))
                    }
                    crate::domains::security::SecurityCommandTaskStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                },
            ),
            "ServiceWorker" => {
                let plan = crate::domains::service_worker::command_output_plan(self, &cmd);
                Some(self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id))
            }
            "IO" => Some(
                match crate::domains::io::try_start_io_command_dispatch(self, &cmd) {
                    crate::domains::io::IoCommandTaskStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Io(pending))
                    }
                    crate::domains::io::IoCommandTaskStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                },
            ),
            "Autofill" => Some(
                match crate::domains::autofill::try_start_autofill_command_dispatch(self, &cmd) {
                    crate::domains::autofill::AutofillCommandTaskStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Autofill(pending))
                    }
                    crate::domains::autofill::AutofillCommandTaskStep::Complete(plan) => {
                        self.complete_with_output_plan(
                            command_context,
                            plan,
                            cmd.id,
                            cmd.session_id,
                        )
                    }
                },
            ),
            "Audits" => {
                let plan = crate::domains::audits::command_output_plan(self, &cmd);
                Some(self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id))
            }
            "SystemInfo" => {
                let plan = crate::domains::system_info::command_output_plan(&cmd);
                Some(self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id))
            }
            "WebAuthn" => {
                let plan = crate::domains::webauthn::command_output_plan(&cmd);
                Some(self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id))
            }
            "WebMCP" => {
                let plan = crate::domains::web_mcp::command_output_plan(&cmd);
                Some(self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id))
            }
            "Performance" => Some(
                match crate::domains::performance::try_start_performance_command_dispatch(
                    self,
                    &cmd,
                    command.renderer_access(),
                ) {
                    crate::domains::performance::PerformanceCommandTaskStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::Performance(pending))
                    }
                    crate::domains::performance::PerformanceCommandTaskStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                },
            ),
            "DOMDebugger" => Some(
                match crate::domains::dom_debugger::try_start_dom_debugger_command_dispatch(
                    self, &cmd,
                ) {
                    crate::domains::dom_debugger::DomDebuggerCommandTaskStep::Pending(pending) => {
                        self.pending_step(PendingCdpCommandDispatchKind::DomDebugger(pending))
                    }
                    crate::domains::dom_debugger::DomDebuggerCommandTaskStep::Complete(plan) => {
                        self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                    }
                },
            ),
            "Emulation" => crate::domains::emulation::try_start_emulation_command_dispatch(
                self, &cmd,
            )
            .map(|step| match step {
                crate::domains::emulation::EmulationCommandTaskStep::Pending(pending) => {
                    self.pending_step(PendingCdpCommandDispatchKind::Emulation(pending))
                }
                crate::domains::emulation::EmulationCommandTaskStep::Complete(plan) => {
                    self.complete_with_output_plan(command_context, plan, cmd.id, cmd.session_id)
                }
            }),
            _ => {
                Some(self.complete_with_output_plan(
                    command_context,
                    CommandOutputPlan::error(-32601, "Unknown domain"),
                    cmd.id,
                    cmd.session_id,
                ))
            }
        };
        let Some(step) = step else {
            return self.complete_with_output_plan(
                command_context,
                CommandOutputPlan::error(-32601, "UnknownMethod"),
                cmd.id,
                cmd.session_id,
            );
        };
        step
    }

    #[cfg(test)]
    pub(crate) async fn complete_pending_command_dispatch(
        &mut self,
        completed: CompletedCdpCommandDispatch,
    ) -> CdpCommandTaskStep {
        let mut command_context = CommandDispatchContext::default();
        self.complete_pending_command_dispatch_with_context(completed, &mut command_context)
            .await
    }

    pub async fn complete_pending_command_dispatch_with_context(
        &mut self,
        mut completed: CompletedCdpCommandDispatch,
        command_context: &mut CommandDispatchContext,
    ) -> CdpCommandTaskStep {
        if let Some(owner_scope) = completed.owner_scope.take() {
            let mut scope = owner_scope.enter(self);
            return scope
                .conn_mut()
                .complete_pending_command_dispatch_in_current_owner(completed, command_context)
                .await;
        }
        self.complete_pending_command_dispatch_in_current_owner(completed, command_context)
            .await
    }

    async fn complete_pending_command_dispatch_in_current_owner(
        &mut self,
        completed: CompletedCdpCommandDispatch,
        command_context: &mut CommandDispatchContext,
    ) -> CdpCommandTaskStep {
        let mut out = Vec::new();
        match completed.inner {
            CompletedCdpCommandDispatchKind::Runtime(completed) => {
                let completed = *completed;
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let response_flush = command_context.response_flush().clone();
                match crate::domains::runtime::complete_pending_runtime_command_at_response_boundary(
                    self,
                    completed,
                    &response_flush,
                )
                .await
                {
                    crate::domains::runtime::RuntimeCommandTaskStep::Pending(pending) => {
                        return self.pending_step(PendingCdpCommandDispatchKind::Runtime(pending));
                    }
                    crate::domains::runtime::RuntimeCommandTaskStep::Complete(plan) => {
                        append_command_output_plan(
                            &mut out,
                            command_context,
                            plan,
                            command_id,
                            session_id.as_deref(),
                        );
                    }
                }
            }
            CompletedCdpCommandDispatchKind::Autofill(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let plan =
                    crate::domains::autofill::complete_pending_autofill_command(self, completed);
                out.extend(plan.into_background_events(command_id, session_id.as_deref()));
            }
            CompletedCdpCommandDispatchKind::Accessibility(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                match crate::domains::accessibility::complete_pending_accessibility_command(
                    self, completed,
                )
                .await
                {
                    crate::domains::accessibility::AccessibilityCommandDispatchStep::Pending(
                        pending,
                    ) => {
                        return self
                            .pending_step(PendingCdpCommandDispatchKind::Accessibility(pending));
                    }
                    crate::domains::accessibility::AccessibilityCommandDispatchStep::Complete(
                        plan,
                    ) => {
                        append_command_output_plan(
                            &mut out,
                            command_context,
                            plan,
                            command_id,
                            session_id.as_deref(),
                        );
                    }
                }
            }
            CompletedCdpCommandDispatchKind::Input(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let (step, plan) =
                    crate::domains::input::complete_pending_input_command_output_plan(
                        self,
                        completed,
                        command_context,
                    )
                    .await;
                match step {
                    crate::domains::input::InputCommandTaskStep::Complete => {
                        append_command_output_plan(
                            &mut out,
                            command_context,
                            plan,
                            command_id,
                            session_id.as_deref(),
                        );
                    }
                }
            }
            CompletedCdpCommandDispatchKind::Css(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                match crate::domains::css::complete_pending_css_command(self, completed) {
                    crate::domains::css::CssCommandDispatchStep::Pending(pending) => {
                        return self.pending_step(PendingCdpCommandDispatchKind::Css(pending));
                    }
                    crate::domains::css::CssCommandDispatchStep::Complete(plan) => {
                        append_command_output_plan(
                            &mut out,
                            command_context,
                            plan,
                            command_id,
                            session_id.as_deref(),
                        );
                    }
                }
            }
            CompletedCdpCommandDispatchKind::Dom(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let (step, plan) = Box::pin(
                    crate::domains::dom::complete_pending_dom_command_output_plan(self, completed),
                )
                .await;
                append_command_output_plan(
                    &mut out,
                    command_context,
                    plan,
                    command_id,
                    session_id.as_deref(),
                );
                match step {
                    crate::domains::dom::DomCommandTaskStep::Pending(pending) => {
                        return self.pending_step(PendingCdpCommandDispatchKind::Dom(*pending));
                    }
                    crate::domains::dom::DomCommandTaskStep::Complete => {}
                }
            }
            CompletedCdpCommandDispatchKind::DomDebugger(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let plan = crate::domains::dom_debugger::complete_pending_dom_debugger_command(
                    self, completed,
                );
                out.extend(plan.into_background_events(command_id, session_id.as_deref()));
            }
            CompletedCdpCommandDispatchKind::DomStorage(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                match crate::domains::dom_storage::complete_pending_dom_storage_command(
                    self, *completed,
                ) {
                    crate::domains::dom_storage::DomStorageCommandTaskStep::Pending(pending) => {
                        return self
                            .pending_step(PendingCdpCommandDispatchKind::DomStorage(pending));
                    }
                    crate::domains::dom_storage::DomStorageCommandTaskStep::Complete(plan) => {
                        append_command_output_plan(
                            &mut out,
                            command_context,
                            plan,
                            command_id,
                            session_id.as_deref(),
                        );
                    }
                }
            }
            CompletedCdpCommandDispatchKind::DomSnapshot(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                match crate::domains::dom_snapshot::complete_pending_dom_snapshot_command(
                    self, completed,
                ) {
                    crate::domains::dom_snapshot::DomSnapshotCommandDispatchStep::Pending(
                        pending,
                    ) => {
                        return self
                            .pending_step(PendingCdpCommandDispatchKind::DomSnapshot(pending));
                    }
                    crate::domains::dom_snapshot::DomSnapshotCommandDispatchStep::Complete(
                        plan,
                    ) => {
                        append_command_output_plan(
                            &mut out,
                            command_context,
                            plan,
                            command_id,
                            session_id.as_deref(),
                        );
                    }
                }
            }
            CompletedCdpCommandDispatchKind::Page(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                match crate::domains::page::complete_pending_page_command(
                    self,
                    completed,
                    command_context,
                )
                .await
                {
                    crate::domains::page::PageCommandTaskStep::Pending(pending) => {
                        return self.pending_step(PendingCdpCommandDispatchKind::Page(pending));
                    }
                    crate::domains::page::PageCommandTaskStep::Complete(plan) => {
                        append_command_output_plan(
                            &mut out,
                            command_context,
                            plan,
                            command_id,
                            session_id.as_deref(),
                        );
                    }
                }
            }
            CompletedCdpCommandDispatchKind::Performance(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let plan = crate::domains::performance::complete_pending_performance_command(
                    self, completed,
                )
                .await;
                append_command_output_plan(
                    &mut out,
                    command_context,
                    plan,
                    command_id,
                    session_id.as_deref(),
                );
            }
            CompletedCdpCommandDispatchKind::Emulation(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let plan =
                    crate::domains::emulation::complete_pending_emulation_command(self, completed);
                append_command_output_plan(
                    &mut out,
                    command_context,
                    plan,
                    command_id,
                    session_id.as_deref(),
                );
            }
            CompletedCdpCommandDispatchKind::Storage(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let plan =
                    crate::domains::storage::complete_pending_storage_command(self, completed);
                append_command_output_plan(
                    &mut out,
                    command_context,
                    plan,
                    command_id,
                    session_id.as_deref(),
                );
            }
            CompletedCdpCommandDispatchKind::Network(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                match crate::domains::network::complete_pending_network_command(self, completed) {
                    crate::domains::network::NetworkCommandTaskStep::Pending(pending) => {
                        return self.pending_step(PendingCdpCommandDispatchKind::Network(pending));
                    }
                    crate::domains::network::NetworkCommandTaskStep::Complete(plan) => {
                        append_command_output_plan(
                            &mut out,
                            command_context,
                            plan,
                            command_id,
                            session_id.as_deref(),
                        );
                    }
                }
            }
            CompletedCdpCommandDispatchKind::Fetch(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let plan = Box::pin(crate::domains::fetch::complete_pending_fetch_command(
                    self, completed,
                ))
                .await;
                append_command_output_plan(
                    &mut out,
                    command_context,
                    plan,
                    command_id,
                    session_id.as_deref(),
                );
            }
            CompletedCdpCommandDispatchKind::Io(completed) => {
                let completed = *completed;
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let plan = crate::domains::io::complete_pending_io_command(self, completed);
                append_command_output_plan(
                    &mut out,
                    command_context,
                    plan,
                    command_id,
                    session_id.as_deref(),
                );
            }
            CompletedCdpCommandDispatchKind::Security(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let plan =
                    crate::domains::security::complete_pending_security_command(self, completed);
                append_command_output_plan(
                    &mut out,
                    command_context,
                    plan,
                    command_id,
                    session_id.as_deref(),
                );
            }
            CompletedCdpCommandDispatchKind::Browser(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let plan =
                    crate::domains::browser::complete_pending_browser_command(self, completed);
                append_command_output_plan(
                    &mut out,
                    command_context,
                    plan,
                    command_id,
                    session_id.as_deref(),
                );
            }
            CompletedCdpCommandDispatchKind::Target(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                match crate::domains::target::complete_pending_target_command(
                    self,
                    completed,
                    command_context,
                )
                .await
                {
                    crate::domains::target::TargetCommandTaskStep::Pending(pending) => {
                        return self.pending_step(PendingCdpCommandDispatchKind::Target(pending));
                    }
                    crate::domains::target::TargetCommandTaskStep::Complete(plan) => {
                        append_command_output_plan(
                            &mut out,
                            command_context,
                            plan,
                            command_id,
                            session_id.as_deref(),
                        );
                    }
                }
            }
            CompletedCdpCommandDispatchKind::Tracing(completed) => {
                let command_id = completed.command_id();
                let session_id = completed.session_id().map(str::to_owned);
                let plan =
                    crate::domains::tracing::complete_pending_tracing_command(self, completed);
                out.extend(plan.into_background_events(command_id, session_id.as_deref()));
            }
        }
        command_context.protocol_events_mut().extend(out);
        let (out, renderer_output_boundary, post_renderer_output_events) =
            command_context.take_renderer_fenced_protocol_events();
        let post_response_events = command_context.take_post_response_events();
        let scheduler_events = self.take_scheduler_events();
        CdpCommandTaskStep::Complete(
            CdpTurnOutcome::new_with_protocol_and_post_response_events(
                out,
                post_response_events,
                scheduler_events,
            )
            .with_renderer_output_boundary(renderer_output_boundary, post_renderer_output_events)
            .with_renderer_output_predecessor(command_context.take_renderer_output_predecessor()),
        )
    }

    pub async fn process_message_with_turn_outcome_async(
        &mut self,
        raw: &str,
    ) -> CdpRendererOwnerTurnOutcome {
        let outcome = self
            .process_message_through_command_dispatch_async(raw)
            .await;
        let (
            protocol_events,
            post_renderer_output_events,
            renderer_output_boundary,
            post_response_events,
            mut scheduler_events,
            renderer_output_predecessor,
        ) = outcome.into_renderer_owner_turn_parts();
        scheduler_events.extend(self.take_scheduler_events());
        CdpTurnOutcome::new_with_protocol_and_post_response_events(
            protocol_events,
            post_response_events,
            scheduler_events,
        )
        .with_renderer_output_boundary(renderer_output_boundary, post_renderer_output_events)
        .with_renderer_output_predecessor(renderer_output_predecessor)
    }

    /// Test-only compatibility CDP helper that returns protocol messages only.
    ///
    /// The WebSocket scheduler uses `start_parsed_command_dispatch_with_context`
    /// directly so that each pending command keeps its command-local output
    /// buffers and response-flush gate across await points. New direct helper
    /// code should use `process_message_with_turn_outcome_async` so scheduler
    /// events cannot be dropped accidentally.
    #[cfg(test)]
    pub async fn process_message_messages_only_for_test(&mut self, raw: &str) -> Vec<Value> {
        let outcome = self
            .process_message_through_command_dispatch_async(raw)
            .await;
        outcome.into_parts().0
    }

    async fn process_message_through_command_dispatch_async(
        &mut self,
        raw: &str,
    ) -> CdpRendererOwnerTurnOutcome {
        let command = ParsedCdpCommand::parse_str(raw.to_owned());
        let output_session_id = command
            .as_ref()
            .ok()
            .and_then(ParsedCdpCommand::command_output_session_id)
            .map(str::to_owned);
        let mut protocol_events = Vec::new();
        let mut post_renderer_output_events = Vec::new();
        let mut renderer_output_boundary = None;
        let mut scheduler_events = Vec::new();
        let mut renderer_output_predecessor: Option<moli_core::RendererOutputFence> = None;
        let mut command_context = CommandDispatchContext::default();
        let mut step = match command.as_ref() {
            Ok(command) => {
                self.start_parsed_command_dispatch_with_context(command, &mut command_context)
            }
            Err(error) => self.complete_with_output_plan(
                &mut command_context,
                CommandOutputPlan::error_without_session(
                    error.response_code(),
                    error.response_message(),
                ),
                error.command_id(),
                None,
            ),
        };
        loop {
            match step {
                CdpCommandTaskStep::Complete(outcome) => {
                    let (
                        mut complete_protocol_events,
                        mut complete_post_renderer_output_events,
                        complete_renderer_output_boundary,
                        mut complete_post_response_events,
                        mut complete_events,
                        complete_renderer_output_predecessor,
                    ) = outcome.into_renderer_owner_turn_parts();
                    assert!(
                        renderer_output_boundary.is_none(),
                        "one direct command cannot complete multiple renderer insertion boundaries"
                    );
                    renderer_output_boundary = complete_renderer_output_boundary;
                    protocol_events.append(&mut complete_protocol_events);
                    post_renderer_output_events.append(&mut complete_post_renderer_output_events);
                    post_renderer_output_events.append(&mut complete_post_response_events);
                    scheduler_events.append(&mut complete_events);
                    if let Some(predecessor) = complete_renderer_output_predecessor {
                        predecessor.merge_into_same_stream_tail(&mut renderer_output_predecessor);
                    }
                    break;
                }
                CdpCommandTaskStep::Pending(pending) => {
                    let completed = Box::pin(pending.wait()).await;
                    step = Box::pin(self.complete_pending_command_dispatch_with_context(
                        completed,
                        &mut command_context,
                    ))
                    .await;
                }
            }
        }
        Box::pin(
            crate::domains::activity::project_protocol_local_command_outputs(
                self,
                output_session_id.as_deref(),
                &mut command_context,
            ),
        )
        .await;
        command_context
            .protocol_events_mut()
            .extend(protocol_events);
        if renderer_output_boundary.is_some() {
            command_context.append_renderer_fenced_protocol_events(
                Vec::new(),
                renderer_output_boundary,
                post_renderer_output_events,
            );
        } else {
            command_context
                .protocol_events_mut()
                .extend(post_renderer_output_events);
        }
        let post_response_events = command_context.take_post_response_events();
        command_context
            .protocol_events_mut()
            .extend(post_response_events);
        let (protocol_events, renderer_output_boundary, post_renderer_output_events) =
            command_context.take_renderer_fenced_protocol_events();
        scheduler_events.extend(self.take_scheduler_events());
        CdpTurnOutcome::new_with_protocol_events(protocol_events, scheduler_events)
            .with_renderer_output_boundary(renderer_output_boundary, post_renderer_output_events)
            .with_renderer_output_predecessor(renderer_output_predecessor)
    }
}

/// A minimal "startup" context - used before a real browser context exists.
fn startup_output_plan(req: &CdpRequest) -> CommandOutputPlan {
    let dot = req.method().find('.').unwrap_or(0);
    let action = &req.method()[dot + 1..];
    if action == "getFrameTree" {
        CommandOutputPlan::result(json!({
                "frameTree": {
                    "frame": {
                        "id": "TID-STARTUP",
                        "loaderId": "LOADERID24DD2FD56CF1EF33C965C79C",
                        "securityOrigin": URL_BASE,
                        "url": "about:blank",
                        "secureContextType": "Secure",
                    }
                }
        }))
    } else {
        CommandOutputPlan::success()
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
