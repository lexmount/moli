use std::time::Instant;

use anyhow::Result;
use moli_protocol::ParsedCdpCommand;
use parking_lot::Mutex;
use serde_json::Value;

use crate::{cdp_scheduler::ProtocolOutputSequence, cdp_writer::CdpSocketSink};

use self::routing::CdpFrontendRoutingState;

mod routing;

pub(crate) enum CdpPreparedFrontendCommand {
    Command(ParsedCdpCommand),
    ImmediateResponse { frontend_id: u64, message: Value },
}

pub(crate) struct CdpFrontendRouter {
    routing: Mutex<CdpFrontendRoutingState>,
}

impl CdpFrontendRouter {
    pub(crate) fn new() -> Self {
        Self {
            routing: Mutex::new(CdpFrontendRoutingState::default()),
        }
    }

    pub(crate) fn prepare_command_str(
        &self,
        frontend_id: u64,
        raw: String,
    ) -> Option<CdpPreparedFrontendCommand> {
        self.routing.lock().prepare_command_str(frontend_id, raw)
    }

    pub(crate) fn enqueue_immediate_response(&self, frontend_id: u64, message: Value) {
        let frontend = self.routing.lock().frontend_by_id(frontend_id);
        if let Some(frontend) = frontend {
            frontend.enqueue_message(message);
        }
    }

    pub(crate) fn register_browser_frontend(
        &self,
        frontend_id: u64,
        session_id: String,
        sink: CdpSocketSink,
    ) -> Result<()> {
        self.routing
            .lock()
            .register_browser_frontend(frontend_id, session_id, sink)
    }

    pub(crate) fn register_target_frontend(
        &self,
        frontend_id: u64,
        target_id: String,
        session_id: String,
        sink: CdpSocketSink,
    ) -> Result<()> {
        self.routing
            .lock()
            .register_target_frontend(frontend_id, target_id, session_id, sink)
    }

    pub(crate) fn unregister_browser_frontend(&self, frontend_id: u64) -> Option<String> {
        self.routing.lock().unregister_browser_frontend(frontend_id)
    }

    pub(crate) fn unregister_target_frontend(&self, frontend_id: u64) -> Option<String> {
        self.routing.lock().unregister_target_frontend(frontend_id)
    }

    pub(crate) fn unregister_frontends_for_target(&self, target_id: &str) {
        self.routing
            .lock()
            .unregister_frontends_for_target(target_id);
    }

    pub(crate) fn register_private_session(&self, session_id: String) -> Result<()> {
        self.routing.lock().register_private_session(session_id)
    }

    pub(crate) fn enqueue_protocol_output_sequence(&self, output: ProtocolOutputSequence) -> bool {
        if output.is_empty() {
            return true;
        }
        let message_count = output.len();
        let mut all_enqueued = true;
        let trace_started = moli_trace::cdp_runtime_trace_enabled().then(Instant::now);
        if trace_started.is_some() {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "writer_enqueue_start",
                messages = message_count,
            );
        }
        for delivery in output.into_deliveries() {
            if !delivery.has_protocol_wire_message() {
                continue;
            }
            let wire_session = delivery.protocol_session_id().map(str::to_owned);
            let message = delivery.into_protocol_message();
            let routed = self
                .routing
                .lock()
                .route_message(message, wire_session.as_deref());
            let Some((frontend, message)) = routed else {
                continue;
            };
            let frontend_id = frontend.frontend_id();
            let enqueued = frontend.enqueue_message(message);
            all_enqueued &= enqueued;
            if !enqueued && let Some(started) = trace_started {
                tracing::info!(
                    target: "moli_cdp_runtime",
                    stage = "writer_enqueue_failed",
                    messages = message_count,
                    elapsed_us = %started.elapsed().as_micros(),
                    reason = "frontend_queue_or_byte_budget",
                    frontend_id,
                );
            }
        }
        if let Some(started) = trace_started {
            tracing::info!(
                target: "moli_cdp_runtime",
                stage = "writer_enqueue_done",
                messages = message_count,
                enqueued = all_enqueued,
                elapsed_us = %started.elapsed().as_micros(),
            );
        }
        all_enqueued
    }
}
