use std::{
    collections::{HashMap, hash_map::Entry},
    fmt,
    num::NonZeroI32,
};

use moli_core::{
    RendererRuntimeInspectorAsyncCompletion, RendererRuntimeInspectorResponseChannel,
    RendererRuntimeInspectorResponseSender,
};
use moli_page_types::{
    DevToolsSessionKey, FrontendCommandId, RendererAgentAttachmentId, RendererCallId,
    RendererInspectorResponseDelivery,
};
use moli_protocol_cdp::{
    CdpRendererCommandPolicy, CdpRendererCommandReplacement, CdpRendererCommandReplayDispatch,
    ParsedCdpCommand,
};
#[cfg(test)]
use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RendererCommandDescriptor {
    replacement: CdpRendererCommandReplacement,
    replay: RendererCommandReplay,
    response_delivery: RendererInspectorResponseDelivery,
    frontend_payload: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RendererCommandReplay {
    Inspector(CdpRendererCommandReplayDispatch),
    PerformanceGetMetrics,
    SetScriptExecutionDisabled { disabled: bool },
}

impl RendererCommandDescriptor {
    /// Builds a replay descriptor from the policy already derived at typed CDP
    /// ingress. Production frontend commands must use this constructor.
    pub(crate) fn from_frontend_policy(
        frontend_payload: String,
        policy: CdpRendererCommandPolicy,
        response_delivery: RendererInspectorResponseDelivery,
    ) -> Self {
        Self {
            replacement: policy.replacement(),
            replay: RendererCommandReplay::Inspector(policy.replay_dispatch()),
            response_delivery,
            frontend_payload,
        }
    }

    pub(crate) fn performance_get_metrics(
        frontend_payload: String,
        policy: CdpRendererCommandPolicy,
        response_delivery: RendererInspectorResponseDelivery,
    ) -> Self {
        Self {
            replacement: policy.replacement(),
            replay: RendererCommandReplay::PerformanceGetMetrics,
            response_delivery,
            frontend_payload,
        }
    }

    pub(crate) fn set_script_execution_disabled(
        frontend_payload: String,
        policy: CdpRendererCommandPolicy,
        disabled: bool,
        response_delivery: RendererInspectorResponseDelivery,
    ) -> Self {
        Self {
            replacement: policy.replacement(),
            replay: RendererCommandReplay::SetScriptExecutionDisabled { disabled },
            response_delivery,
            frontend_payload,
        }
    }

    /// Validates an internally synthesized Inspector payload that has no
    /// `ParsedCdpCommand` ingress residence of its own.
    ///
    /// This is deliberately named as a fallback: a frontend command reaching
    /// this constructor would reintroduce method-policy reconstruction.
    pub(crate) fn from_synthesized_payload(frontend_payload: String) -> Result<Self, String> {
        let command = ParsedCdpCommand::parse_str(&frontend_payload)
            .map_err(|error| format!("invalid renderer Inspector command JSON: {error}"))?;
        let policy = command.renderer_policy();
        Ok(Self {
            replacement: policy.replacement(),
            replay: RendererCommandReplay::Inspector(policy.replay_dispatch()),
            // Internal Classic/BiDi adapters own their reply channel. A
            // method being eligible for frontend session output must never
            // redirect a synthesized adapter command implicitly.
            response_delivery: RendererInspectorResponseDelivery::CommandReply,
            frontend_payload,
        })
    }

    pub(crate) const fn replacement(&self) -> CdpRendererCommandReplacement {
        self.replacement
    }

    pub(crate) fn replay(&self) -> &RendererCommandReplay {
        &self.replay
    }

    pub(crate) const fn response_delivery(&self) -> RendererInspectorResponseDelivery {
        self.response_delivery
    }

    pub(crate) fn frontend_payload(&self) -> &str {
        &self.frontend_payload
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DuplicatePendingRendererCommand {
    frontend_command_id: FrontendCommandId,
}

impl DuplicatePendingRendererCommand {
    #[cfg(test)]
    pub(crate) const fn frontend_command_id(self) -> FrontendCommandId {
        self.frontend_command_id
    }
}

impl fmt::Display for DuplicatePendingRendererCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Duplicate `id` in protocol request")
    }
}

impl std::error::Error for DuplicatePendingRendererCommand {}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PendingRendererCommandKey {
    session_key: DevToolsSessionKey,
    frontend_command_id: FrontendCommandId,
}

impl PendingRendererCommandKey {
    pub(crate) fn new(session_id: Option<&str>, frontend_command_id: u64) -> Self {
        Self {
            session_key: DevToolsSessionKey::from_wire_session_id(session_id),
            frontend_command_id: FrontendCommandId::new(frontend_command_id),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererCallIdExhausted;

impl fmt::Display for RendererCallIdExhausted {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("renderer Inspector call-id space exhausted")
    }
}

impl std::error::Error for RendererCallIdExhausted {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RegisterRendererCallError {
    Duplicate(DuplicatePendingRendererCommand),
    Exhausted(RendererCallIdExhausted),
}

impl fmt::Display for RegisterRendererCallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate(error) => error.fmt(formatter),
            Self::Exhausted(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RegisterRendererCallError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererCommandCorrelation {
    frontend_command_id: FrontendCommandId,
    renderer_call_id: RendererCallId,
    dispatched_attachment_id: Option<RendererAgentAttachmentId>,
}

impl RendererCommandCorrelation {
    pub(crate) const fn frontend_command_id(self) -> FrontendCommandId {
        self.frontend_command_id
    }

    pub(crate) const fn renderer_call_id(self) -> RendererCallId {
        self.renderer_call_id
    }

    pub(crate) const fn dispatched_attachment_id(self) -> Option<RendererAgentAttachmentId> {
        self.dispatched_attachment_id
    }
}

#[derive(Debug)]
pub(crate) struct PreparedRendererCallDispatch {
    correlation: RendererCommandCorrelation,
    response_sender: RendererRuntimeInspectorResponseSender,
    response_receiver:
        Option<tokio::sync::oneshot::Receiver<RendererRuntimeInspectorAsyncCompletion>>,
}

impl PreparedRendererCallDispatch {
    pub(crate) const fn correlation(&self) -> RendererCommandCorrelation {
        self.correlation
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        RendererCommandCorrelation,
        RendererRuntimeInspectorResponseSender,
        Option<tokio::sync::oneshot::Receiver<RendererRuntimeInspectorAsyncCompletion>>,
    ) {
        (
            self.correlation,
            self.response_sender,
            self.response_receiver,
        )
    }
}

#[derive(Debug)]
pub(crate) struct PreparedRendererCallReplay {
    correlation: RendererCommandCorrelation,
    replay: RendererCommandReplay,
    response_delivery: RendererInspectorResponseDelivery,
    frontend_payload: String,
    response_sender: RendererRuntimeInspectorResponseSender,
}

#[derive(Debug)]
pub(crate) enum PreparedRendererCallTermination {
    /// Internal adapters await a private receiver, so navigation rotates the
    /// lease to a synthetic terminal completion owned by the replacement.
    CommandReply {
        correlation: RendererCommandCorrelation,
        response_sender: RendererRuntimeInspectorResponseSender,
    },
    /// Frontend calls remain owned by their protocol session. Navigation only
    /// revokes the retired renderer lease; the session consumes this original
    /// correlation when it emits the terminal response directly.
    DevToolsSession {
        correlation: RendererCommandCorrelation,
    },
}

impl PreparedRendererCallTermination {
    #[cfg(test)]
    pub(crate) const fn correlation(&self) -> RendererCommandCorrelation {
        match self {
            Self::CommandReply { correlation, .. } | Self::DevToolsSession { correlation } => {
                *correlation
            }
        }
    }
}

impl PreparedRendererCallReplay {
    #[cfg(test)]
    pub(crate) const fn correlation(&self) -> RendererCommandCorrelation {
        self.correlation
    }

    #[cfg(test)]
    pub(crate) fn frontend_payload(&self) -> &str {
        &self.frontend_payload
    }

    #[cfg(test)]
    pub(crate) const fn response_delivery(&self) -> RendererInspectorResponseDelivery {
        self.response_delivery
    }

    #[cfg(test)]
    pub(crate) fn into_response_sender(self) -> RendererRuntimeInspectorResponseSender {
        self.response_sender
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        RendererCommandCorrelation,
        RendererCommandReplay,
        RendererInspectorResponseDelivery,
        String,
        RendererRuntimeInspectorResponseSender,
    ) {
        (
            self.correlation,
            self.replay,
            self.response_delivery,
            self.frontend_payload,
            self.response_sender,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RegisteredRendererCall {
    renderer_call_id: RendererCallId,
    dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    descriptor: RendererCommandDescriptor,
    response_channel: RendererRuntimeInspectorResponseChannel,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingRendererCommandRegistry<T> {
    entries: HashMap<FrontendCommandId, T>,
    renderer_calls_by_frontend: HashMap<FrontendCommandId, RegisteredRendererCall>,
    frontend_commands_by_renderer: HashMap<RendererCallId, FrontendCommandId>,
    next_renderer_call_id: Option<NonZeroI32>,
}

impl<T> Default for PendingRendererCommandRegistry<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            renderer_calls_by_frontend: HashMap::new(),
            frontend_commands_by_renderer: HashMap::new(),
            next_renderer_call_id: NonZeroI32::new(1),
        }
    }
}

impl<T> PendingRendererCommandRegistry<T> {
    pub(crate) fn try_insert(
        &mut self,
        frontend_command_id: FrontendCommandId,
        command: T,
    ) -> Result<(), DuplicatePendingRendererCommand> {
        match self.entries.entry(frontend_command_id) {
            Entry::Vacant(entry) => {
                entry.insert(command);
                Ok(())
            }
            Entry::Occupied(_) => Err(DuplicatePendingRendererCommand {
                frontend_command_id,
            }),
        }
    }

    pub(crate) fn try_register_renderer_call(
        &mut self,
        frontend_command_id: FrontendCommandId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
        descriptor: RendererCommandDescriptor,
    ) -> Result<PreparedRendererCallDispatch, RegisterRendererCallError> {
        if self
            .renderer_calls_by_frontend
            .contains_key(&frontend_command_id)
        {
            return Err(RegisterRendererCallError::Duplicate(
                DuplicatePendingRendererCommand {
                    frontend_command_id,
                },
            ));
        }
        let renderer_call_id = self
            .allocate_renderer_call_id()
            .map_err(RegisterRendererCallError::Exhausted)?;
        let response_delivery = descriptor.response_delivery();
        let (response_channel, response_receiver) =
            RendererRuntimeInspectorResponseChannel::new_for_delivery(response_delivery);
        let response_sender =
            response_channel.activate_sender(renderer_call_id.get(), dispatched_attachment_id);
        let previous_renderer_call = self.renderer_calls_by_frontend.insert(
            frontend_command_id,
            RegisteredRendererCall {
                renderer_call_id,
                dispatched_attachment_id,
                descriptor,
                response_channel,
            },
        );
        let previous_frontend_command = self
            .frontend_commands_by_renderer
            .insert(renderer_call_id, frontend_command_id);
        debug_assert!(previous_renderer_call.is_none());
        debug_assert!(previous_frontend_command.is_none());
        Ok(PreparedRendererCallDispatch {
            correlation: RendererCommandCorrelation {
                frontend_command_id,
                renderer_call_id,
                dispatched_attachment_id,
            },
            response_sender,
            response_receiver,
        })
    }

    pub(crate) fn prepare_replays_from_attachment(
        &mut self,
        old_attachment_id: RendererAgentAttachmentId,
        new_attachment_id: RendererAgentAttachmentId,
    ) -> Result<Vec<PreparedRendererCallReplay>, RendererCallIdExhausted> {
        let mut frontend_command_ids = self
            .renderer_calls_by_frontend
            .iter()
            .filter_map(|(frontend_command_id, call)| {
                (call.dispatched_attachment_id == Some(old_attachment_id)
                    && call.descriptor.replacement() == CdpRendererCommandReplacement::Replay)
                    .then_some(*frontend_command_id)
            })
            .collect::<Vec<_>>();
        frontend_command_ids.sort_by_key(|command_id| command_id.get());

        let mut replays = Vec::with_capacity(frontend_command_ids.len());
        for frontend_command_id in frontend_command_ids {
            let renderer_call_id = self.allocate_renderer_call_id()?;
            let response_sender = {
                let call = self
                    .renderer_calls_by_frontend
                    .get(&frontend_command_id)
                    .expect("selected replay command must remain registered");
                call.response_channel
                    .try_activate_sender(renderer_call_id.get(), Some(new_attachment_id))
            };
            let Some(response_sender) = response_sender else {
                continue;
            };
            let call = self
                .renderer_calls_by_frontend
                .get_mut(&frontend_command_id)
                .expect("selected replay command must remain registered");
            let old_renderer_call_id = call.renderer_call_id;
            let removed_frontend = self
                .frontend_commands_by_renderer
                .remove(&old_renderer_call_id);
            debug_assert_eq!(removed_frontend, Some(frontend_command_id));
            let previous_frontend = self
                .frontend_commands_by_renderer
                .insert(renderer_call_id, frontend_command_id);
            debug_assert!(previous_frontend.is_none());

            call.renderer_call_id = renderer_call_id;
            call.dispatched_attachment_id = Some(new_attachment_id);
            replays.push(PreparedRendererCallReplay {
                correlation: RendererCommandCorrelation {
                    frontend_command_id,
                    renderer_call_id,
                    dispatched_attachment_id: Some(new_attachment_id),
                },
                replay: call.descriptor.replay().clone(),
                response_delivery: call.descriptor.response_delivery(),
                frontend_payload: call.descriptor.frontend_payload().to_owned(),
                response_sender,
            });
        }
        Ok(replays)
    }

    pub(crate) fn prepare_terminations_from_attachment(
        &mut self,
        old_attachment_id: RendererAgentAttachmentId,
        terminal_attachment_id: RendererAgentAttachmentId,
    ) -> Result<Vec<PreparedRendererCallTermination>, RendererCallIdExhausted> {
        let mut frontend_command_ids = self
            .renderer_calls_by_frontend
            .iter()
            .filter_map(|(frontend_command_id, call)| {
                (call.dispatched_attachment_id == Some(old_attachment_id)
                    && call.descriptor.replacement() == CdpRendererCommandReplacement::Terminate)
                    .then_some(*frontend_command_id)
            })
            .collect::<Vec<_>>();
        frontend_command_ids.sort_by_key(|command_id| command_id.get());

        let mut terminations = Vec::with_capacity(frontend_command_ids.len());
        for frontend_command_id in frontend_command_ids {
            let response_delivery = self
                .renderer_calls_by_frontend
                .get(&frontend_command_id)
                .expect("selected terminal command must remain registered")
                .descriptor
                .response_delivery();
            match response_delivery {
                RendererInspectorResponseDelivery::DevToolsSession => {
                    let call = self
                        .renderer_calls_by_frontend
                        .get(&frontend_command_id)
                        .expect("selected terminal command must remain registered");
                    if !call.response_channel.try_revoke_active_lease() {
                        continue;
                    }
                    terminations.push(PreparedRendererCallTermination::DevToolsSession {
                        correlation: RendererCommandCorrelation {
                            frontend_command_id,
                            renderer_call_id: call.renderer_call_id,
                            dispatched_attachment_id: call.dispatched_attachment_id,
                        },
                    });
                    continue;
                }
                RendererInspectorResponseDelivery::CommandReply => {}
            }

            let renderer_call_id = self.allocate_renderer_call_id()?;
            let response_sender = {
                let call = self
                    .renderer_calls_by_frontend
                    .get(&frontend_command_id)
                    .expect("selected terminal command must remain registered");
                call.response_channel
                    .try_activate_sender(renderer_call_id.get(), Some(terminal_attachment_id))
            };
            let Some(response_sender) = response_sender else {
                continue;
            };
            let call = self
                .renderer_calls_by_frontend
                .get_mut(&frontend_command_id)
                .expect("selected terminal command must remain registered");
            let old_renderer_call_id = call.renderer_call_id;
            let removed_frontend = self
                .frontend_commands_by_renderer
                .remove(&old_renderer_call_id);
            debug_assert_eq!(removed_frontend, Some(frontend_command_id));
            let previous_frontend = self
                .frontend_commands_by_renderer
                .insert(renderer_call_id, frontend_command_id);
            debug_assert!(previous_frontend.is_none());

            call.renderer_call_id = renderer_call_id;
            call.dispatched_attachment_id = Some(terminal_attachment_id);
            terminations.push(PreparedRendererCallTermination::CommandReply {
                correlation: RendererCommandCorrelation {
                    frontend_command_id,
                    renderer_call_id,
                    dispatched_attachment_id: Some(terminal_attachment_id),
                },
                response_sender,
            });
        }
        Ok(terminations)
    }

    pub(crate) fn terminate_all_renderer_calls(
        &mut self,
        reason: &str,
    ) -> Vec<RendererCommandCorrelation> {
        let mut frontend_command_ids = self
            .renderer_calls_by_frontend
            .keys()
            .copied()
            .collect::<Vec<_>>();
        frontend_command_ids.sort_by_key(|command_id| command_id.get());

        let mut terminated = Vec::with_capacity(frontend_command_ids.len());
        for frontend_command_id in frontend_command_ids {
            let (correlation, response_sender) = {
                let call = self
                    .renderer_calls_by_frontend
                    .get_mut(&frontend_command_id)
                    .expect("selected terminal command must remain registered");
                call.dispatched_attachment_id = None;
                let correlation = RendererCommandCorrelation {
                    frontend_command_id,
                    renderer_call_id: call.renderer_call_id,
                    dispatched_attachment_id: None,
                };
                let response_sender = call
                    .response_channel
                    .try_activate_sender(call.renderer_call_id.get(), None);
                (correlation, response_sender)
            };
            if let Some(response_sender) = response_sender {
                let _ = response_sender.send(serde_json::json!({
                    "id": correlation.renderer_call_id().get(),
                    "error": {
                        "code": -32000,
                        "message": reason,
                    },
                }));
            }
            let removed = self.take_renderer_call_for_frontend_if_matches(
                frontend_command_id,
                correlation.renderer_call_id(),
                None,
            );
            debug_assert_eq!(removed, Some(correlation));
            terminated.push(correlation);
        }
        terminated
    }

    pub(crate) fn take_renderer_call_for_frontend(
        &mut self,
        frontend_command_id: FrontendCommandId,
    ) -> Option<RendererCommandCorrelation> {
        let registered_call = self
            .renderer_calls_by_frontend
            .remove(&frontend_command_id)?;
        registered_call.response_channel.cancel();
        let removed_frontend = self
            .frontend_commands_by_renderer
            .remove(&registered_call.renderer_call_id);
        debug_assert_eq!(removed_frontend, Some(frontend_command_id));
        Some(RendererCommandCorrelation {
            frontend_command_id,
            renderer_call_id: registered_call.renderer_call_id,
            dispatched_attachment_id: registered_call.dispatched_attachment_id,
        })
    }

    pub(crate) fn renderer_call_for_frontend(
        &self,
        frontend_command_id: FrontendCommandId,
    ) -> Option<RendererCommandCorrelation> {
        let registered_call = self.renderer_calls_by_frontend.get(&frontend_command_id)?;
        Some(RendererCommandCorrelation {
            frontend_command_id,
            renderer_call_id: registered_call.renderer_call_id,
            dispatched_attachment_id: registered_call.dispatched_attachment_id,
        })
    }

    pub(crate) fn renderer_command_descriptor_for_renderer_if_attachment_matches(
        &self,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandDescriptor> {
        let frontend_command_id = self.frontend_commands_by_renderer.get(&renderer_call_id)?;
        let registered_call = self.renderer_calls_by_frontend.get(frontend_command_id)?;
        (registered_call.dispatched_attachment_id == dispatched_attachment_id)
            .then(|| registered_call.descriptor.clone())
    }

    #[cfg(test)]
    pub(crate) fn renderer_command_descriptor_for_frontend(
        &self,
        frontend_command_id: FrontendCommandId,
    ) -> Option<&RendererCommandDescriptor> {
        self.renderer_calls_by_frontend
            .get(&frontend_command_id)
            .map(|call| &call.descriptor)
    }

    pub(crate) fn take_renderer_call_for_frontend_if_matches(
        &mut self,
        frontend_command_id: FrontendCommandId,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        let registered_call = self.renderer_calls_by_frontend.get(&frontend_command_id)?;
        if registered_call.renderer_call_id != renderer_call_id
            || registered_call.dispatched_attachment_id != dispatched_attachment_id
        {
            return None;
        }
        self.take_renderer_call_for_frontend(frontend_command_id)
    }

    pub(crate) fn take_frontend_command_for_renderer(
        &mut self,
        renderer_call_id: RendererCallId,
    ) -> Option<RendererCommandCorrelation> {
        let frontend_command_id = self
            .frontend_commands_by_renderer
            .remove(&renderer_call_id)?;
        let removed_renderer = self.renderer_calls_by_frontend.remove(&frontend_command_id);
        if let Some(call) = &removed_renderer {
            call.response_channel.cancel();
        }
        debug_assert_eq!(
            removed_renderer.as_ref().map(|call| call.renderer_call_id),
            Some(renderer_call_id)
        );
        Some(RendererCommandCorrelation {
            frontend_command_id,
            renderer_call_id,
            dispatched_attachment_id: removed_renderer
                .and_then(|call| call.dispatched_attachment_id),
        })
    }

    pub(crate) fn take_frontend_command_for_renderer_if_attachment_matches(
        &mut self,
        renderer_call_id: RendererCallId,
        dispatched_attachment_id: Option<RendererAgentAttachmentId>,
    ) -> Option<RendererCommandCorrelation> {
        let frontend_command_id = *self.frontend_commands_by_renderer.get(&renderer_call_id)?;
        let registered_call = self.renderer_calls_by_frontend.get(&frontend_command_id)?;
        if registered_call.dispatched_attachment_id != dispatched_attachment_id {
            return None;
        }
        self.take_frontend_command_for_renderer(renderer_call_id)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn get_mut(&mut self, frontend_command_id: FrontendCommandId) -> Option<&mut T> {
        self.entries.get_mut(&frontend_command_id)
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn remove(&mut self, frontend_command_id: FrontendCommandId) -> Option<T> {
        self.entries.remove(&frontend_command_id)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&FrontendCommandId, &T)> {
        self.entries.iter()
    }

    fn allocate_renderer_call_id(&mut self) -> Result<RendererCallId, RendererCallIdExhausted> {
        let renderer_call_id = self.next_renderer_call_id.ok_or(RendererCallIdExhausted)?;
        self.next_renderer_call_id = renderer_call_id
            .get()
            .checked_add(1)
            .and_then(NonZeroI32::new);
        Ok(RendererCallId::new(renderer_call_id.get()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(frontend_command_id: u64, method: &str) -> RendererCommandDescriptor {
        RendererCommandDescriptor::from_synthesized_payload(
            serde_json::json!({
                "id": frontend_command_id,
                "method": method,
                "params": {},
            })
            .to_string(),
        )
        .unwrap()
    }

    #[test]
    fn duplicate_frontend_id_does_not_replace_original_command() {
        let id = FrontendCommandId::new(7);
        let mut registry = PendingRendererCommandRegistry::default();

        registry.try_insert(id, "original").unwrap();
        let duplicate = registry.try_insert(id, "replacement").unwrap_err();

        assert_eq!(duplicate.frontend_command_id(), id);
        assert_eq!(duplicate.to_string(), "Duplicate `id` in protocol request");
        assert_eq!(registry.remove(id), Some("original"));
    }

    #[test]
    fn same_frontend_id_is_independent_across_session_owned_registries() {
        let id = FrontendCommandId::new(9);
        let mut primary = PendingRendererCommandRegistry::default();
        let mut auxiliary = PendingRendererCommandRegistry::default();

        primary.try_insert(id, "primary").unwrap();
        auxiliary.try_insert(id, "auxiliary").unwrap();

        assert_eq!(primary.remove(id), Some("primary"));
        assert_eq!(auxiliary.remove(id), Some("auxiliary"));
    }

    #[test]
    fn renderer_call_ids_are_internal_and_session_scoped() {
        let large_frontend_id = FrontendCommandId::new(i32::MAX as u64 + 41);
        let mut primary = PendingRendererCommandRegistry::<()>::default();
        let mut auxiliary = PendingRendererCommandRegistry::<()>::default();

        let primary_first = primary
            .try_register_renderer_call(
                large_frontend_id,
                None,
                descriptor(large_frontend_id.get(), "Runtime.evaluate"),
            )
            .unwrap()
            .correlation();
        let auxiliary_first = auxiliary
            .try_register_renderer_call(
                large_frontend_id,
                None,
                descriptor(large_frontend_id.get(), "Runtime.evaluate"),
            )
            .unwrap()
            .correlation();
        let primary_second = primary
            .try_register_renderer_call(
                FrontendCommandId::new(2),
                None,
                descriptor(2, "Console.enable"),
            )
            .unwrap()
            .correlation();

        assert_eq!(primary_first.renderer_call_id(), RendererCallId::new(1));
        assert_eq!(auxiliary_first.renderer_call_id(), RendererCallId::new(1));
        assert_eq!(primary_second.renderer_call_id(), RendererCallId::new(2));
        assert_eq!(
            primary.take_frontend_command_for_renderer(RendererCallId::new(1)),
            Some(primary_first)
        );
        assert_eq!(
            auxiliary.take_renderer_call_for_frontend(large_frontend_id),
            Some(auxiliary_first)
        );
    }

    #[test]
    fn duplicate_renderer_call_registration_preserves_original_correlation() {
        let frontend_id = FrontendCommandId::new(17);
        let mut registry = PendingRendererCommandRegistry::<()>::default();
        let original = registry
            .try_register_renderer_call(
                frontend_id,
                None,
                descriptor(frontend_id.get(), "Runtime.getProperties"),
            )
            .unwrap()
            .correlation();

        let error = registry
            .try_register_renderer_call(
                frontend_id,
                None,
                descriptor(frontend_id.get(), "Runtime.evaluate"),
            )
            .unwrap_err();

        assert_eq!(error.to_string(), "Duplicate `id` in protocol request");
        assert_eq!(
            registry.take_renderer_call_for_frontend(frontend_id),
            Some(original)
        );
    }

    #[test]
    fn mismatched_renderer_or_attachment_does_not_consume_correlation() {
        let frontend_id = FrontendCommandId::new(23);
        let dispatched_attachment_id = RendererAgentAttachmentId::allocate();
        let stale_attachment_id = RendererAgentAttachmentId::allocate();
        let mut registry = PendingRendererCommandRegistry::<()>::default();
        let correlation = registry
            .try_register_renderer_call(
                frontend_id,
                Some(dispatched_attachment_id),
                descriptor(frontend_id.get(), "Runtime.evaluate"),
            )
            .unwrap()
            .correlation();

        assert_eq!(
            registry.take_renderer_call_for_frontend_if_matches(
                frontend_id,
                RendererCallId::new(correlation.renderer_call_id().get() + 1),
                Some(dispatched_attachment_id),
            ),
            None
        );
        assert_eq!(
            registry.take_renderer_call_for_frontend_if_matches(
                frontend_id,
                correlation.renderer_call_id(),
                Some(stale_attachment_id),
            ),
            None
        );
        assert_eq!(
            registry.take_frontend_command_for_renderer_if_attachment_matches(
                correlation.renderer_call_id(),
                Some(stale_attachment_id),
            ),
            None
        );
        assert_eq!(
            registry.take_renderer_call_for_frontend_if_matches(
                frontend_id,
                correlation.renderer_call_id(),
                Some(dispatched_attachment_id),
            ),
            Some(correlation)
        );
    }

    #[test]
    fn registered_command_preserves_frontend_payload_and_typed_replacement_traits() {
        let frontend_id = FrontendCommandId::new(31);
        let mut registry = PendingRendererCommandRegistry::<()>::default();
        let registered_descriptor = descriptor(frontend_id.get(), "Profiler.stop");
        let expected_payload = registered_descriptor.frontend_payload().to_owned();

        registry
            .try_register_renderer_call(frontend_id, None, registered_descriptor)
            .unwrap();

        let registered = registry
            .renderer_command_descriptor_for_frontend(frontend_id)
            .expect("registered descriptor");
        assert_eq!(
            registered.replacement(),
            CdpRendererCommandReplacement::Replay
        );
        assert_eq!(
            registered.replay(),
            &RendererCommandReplay::Inspector(CdpRendererCommandReplayDispatch::Direct)
        );
        assert_eq!(registered.frontend_payload(), expected_payload);

        let add_binding = descriptor(32, "Runtime.addBinding");
        assert_eq!(
            add_binding.replay(),
            &RendererCommandReplay::Inspector(
                CdpRendererCommandReplayDispatch::ResolveRuntimeContext
            )
        );
    }

    #[test]
    fn frontend_descriptor_uses_ingress_policy_without_reparsing_payload() {
        let ingress = ParsedCdpCommand::parse_str(
            r#"{"id":32,"method":"Runtime.addBinding","params":{"name":"exposed"}}"#,
        )
        .expect("frontend command must parse at ingress");
        let normalized_payload =
            r#"{"id":91,"method":"Runtime.evaluate","params":{"expression":"1"}}"#.to_owned();

        let descriptor = RendererCommandDescriptor::from_frontend_policy(
            normalized_payload.clone(),
            ingress.renderer_policy(),
            RendererInspectorResponseDelivery::CommandReply,
        );

        assert_eq!(descriptor.frontend_payload(), normalized_payload);
        assert_eq!(
            descriptor.replay(),
            &RendererCommandReplay::Inspector(
                CdpRendererCommandReplayDispatch::ResolveRuntimeContext
            ),
            "descriptor construction must consume the ingress policy instead of deriving it from the normalized payload"
        );
    }

    #[test]
    fn concrete_target_capability_selects_the_response_sink() {
        let frontend = ParsedCdpCommand::parse_str(
            r#"{"id":33,"method":"Debugger.getScriptSource","params":{"scriptId":"1"}}"#,
        )
        .expect("frontend command must parse at ingress");
        let page_descriptor = RendererCommandDescriptor::from_frontend_policy(
            frontend.json().to_owned(),
            frontend.renderer_policy(),
            RendererInspectorResponseDelivery::DevToolsSession,
        );
        let worker_descriptor = RendererCommandDescriptor::from_frontend_policy(
            frontend.json().to_owned(),
            frontend.renderer_policy(),
            RendererInspectorResponseDelivery::CommandReply,
        );
        let adapter_descriptor =
            RendererCommandDescriptor::from_synthesized_payload(frontend.json().to_owned())
                .expect("adapter payload must be valid Inspector JSON");

        assert_eq!(
            page_descriptor.response_delivery(),
            RendererInspectorResponseDelivery::DevToolsSession
        );
        assert_eq!(
            worker_descriptor.response_delivery(),
            RendererInspectorResponseDelivery::CommandReply,
            "the method catalog must not choose a Page-only output capability for Workers"
        );
        assert_eq!(
            adapter_descriptor.response_delivery(),
            RendererInspectorResponseDelivery::CommandReply,
            "method policy must not redirect an internal adapter reply to a frontend session"
        );
    }

    #[test]
    fn non_v8_page_io_descriptors_keep_typed_replay_operations() {
        let performance = ParsedCdpCommand::parse_str(
            r#"{"id":34,"method":"Performance.getMetrics","params":{}}"#,
        )
        .expect("Performance command must parse at ingress");
        let performance_descriptor = RendererCommandDescriptor::performance_get_metrics(
            performance.json().to_owned(),
            performance.renderer_policy(),
            RendererInspectorResponseDelivery::DevToolsSession,
        );
        assert_eq!(
            performance_descriptor.replay(),
            &RendererCommandReplay::PerformanceGetMetrics
        );
        assert_eq!(
            performance_descriptor.response_delivery(),
            RendererInspectorResponseDelivery::DevToolsSession
        );

        let emulation = ParsedCdpCommand::parse_str(
            r#"{"id":35,"method":"Emulation.setScriptExecutionDisabled","params":{"value":true}}"#,
        )
        .expect("Emulation command must parse at ingress");
        let emulation_descriptor = RendererCommandDescriptor::set_script_execution_disabled(
            emulation.json().to_owned(),
            emulation.renderer_policy(),
            true,
            RendererInspectorResponseDelivery::DevToolsSession,
        );
        assert_eq!(
            emulation_descriptor.replay(),
            &RendererCommandReplay::SetScriptExecutionDisabled { disabled: true }
        );
        assert_eq!(
            emulation_descriptor.response_delivery(),
            RendererInspectorResponseDelivery::DevToolsSession
        );
    }

    #[tokio::test]
    async fn session_replay_rotates_renderer_id_and_lease_without_command_reply_receiver() {
        let frontend_id = FrontendCommandId::new(41);
        let old_attachment = RendererAgentAttachmentId::allocate();
        let new_attachment = RendererAgentAttachmentId::allocate();
        let mut registry = PendingRendererCommandRegistry::<()>::default();
        let frontend = ParsedCdpCommand::parse_str(
            serde_json::json!({
                "id": frontend_id.get(),
                "method": "Debugger.getScriptSource",
                "params": { "scriptId": "1" },
            })
            .to_string(),
        )
        .expect("frontend command must parse at ingress");
        let prepared = registry
            .try_register_renderer_call(
                frontend_id,
                Some(old_attachment),
                RendererCommandDescriptor::from_frontend_policy(
                    frontend.json().to_owned(),
                    frontend.renderer_policy(),
                    RendererInspectorResponseDelivery::DevToolsSession,
                ),
            )
            .unwrap();
        let (old_correlation, old_sender, response_receiver) = prepared.into_parts();
        assert!(
            response_receiver.is_none(),
            "DevToolsSession delivery must not allocate a command-reply receiver"
        );

        let mut replays = registry
            .prepare_replays_from_attachment(old_attachment, new_attachment)
            .unwrap();

        assert_eq!(replays.len(), 1);
        let replay = replays.pop().unwrap();
        let new_correlation = replay.correlation();
        assert_eq!(new_correlation.frontend_command_id(), frontend_id);
        assert_ne!(
            new_correlation.renderer_call_id(),
            old_correlation.renderer_call_id()
        );
        assert_eq!(
            new_correlation.dispatched_attachment_id(),
            Some(new_attachment)
        );
        let payload: Value = serde_json::from_str(replay.frontend_payload()).unwrap();
        assert_eq!(payload["id"], frontend_id.get());
        assert_eq!(
            replay.response_delivery(),
            RendererInspectorResponseDelivery::DevToolsSession,
            "attachment replacement must preserve the frontend session output capability"
        );

        assert!(
            old_sender
                .send(serde_json::json!({
                    "id": old_correlation.renderer_call_id().get(),
                    "result": { "stale": true },
                }))
                .is_err(),
            "rotating the registry lease must invalidate the old sender"
        );
        assert!(
            replay
                .into_response_sender()
                .send(serde_json::json!({
                    "id": new_correlation.renderer_call_id().get(),
                    "result": { "current": true },
                }))
                .is_err(),
            "a session response cannot publish until its concrete output host is bound"
        );
        assert_eq!(
            registry.take_frontend_command_for_renderer_if_attachment_matches(
                old_correlation.renderer_call_id(),
                Some(old_attachment),
            ),
            None
        );
        assert_eq!(
            registry.take_frontend_command_for_renderer_if_attachment_matches(
                new_correlation.renderer_call_id(),
                Some(new_attachment),
            ),
            Some(new_correlation)
        );
    }

    #[tokio::test]
    async fn completed_response_is_not_rotated_during_attachment_replacement() {
        let frontend_id = FrontendCommandId::new(42);
        let old_attachment = RendererAgentAttachmentId::allocate();
        let new_attachment = RendererAgentAttachmentId::allocate();
        let mut registry = PendingRendererCommandRegistry::<()>::default();
        let prepared = registry
            .try_register_renderer_call(
                frontend_id,
                Some(old_attachment),
                descriptor(frontend_id.get(), "Console.clearMessages"),
            )
            .unwrap();
        let (correlation, sender, response_receiver) = prepared.into_parts();
        let response_receiver = response_receiver
            .expect("a synthesized CommandReply call must allocate a response receiver");
        sender
            .send(serde_json::json!({
                "id": correlation.renderer_call_id().get(),
                "result": {},
            }))
            .expect("old attachment response should complete before cutover");

        assert!(
            registry
                .prepare_replays_from_attachment(old_attachment, new_attachment)
                .unwrap()
                .is_empty(),
            "a completed response has already won its frontend completion"
        );
        assert_eq!(
            registry.renderer_call_for_frontend(frontend_id),
            Some(correlation),
            "the completed response keeps its old attachment-qualified correlation until routing"
        );
        let completion = response_receiver.await.unwrap();
        assert_eq!(
            registry.take_frontend_command_for_renderer_if_attachment_matches(
                RendererCallId::new(completion.call_id),
                completion.renderer_agent_attachment_id(),
            ),
            Some(correlation)
        );
    }

    #[tokio::test]
    async fn command_reply_termination_rotates_attachment_and_invalidates_old_response_lease() {
        let frontend_id = FrontendCommandId::new(42);
        let old_attachment = RendererAgentAttachmentId::allocate();
        let new_attachment = RendererAgentAttachmentId::allocate();
        let mut registry = PendingRendererCommandRegistry::<()>::default();
        let prepared = registry
            .try_register_renderer_call(
                frontend_id,
                Some(old_attachment),
                descriptor(frontend_id.get(), "Runtime.awaitPromise"),
            )
            .unwrap();
        let (old_correlation, old_sender, response_receiver) = prepared.into_parts();
        let response_receiver = response_receiver
            .expect("a synthesized CommandReply call must allocate a response receiver");

        let mut terminations = registry
            .prepare_terminations_from_attachment(old_attachment, new_attachment)
            .unwrap();

        assert_eq!(terminations.len(), 1);
        let termination = terminations.pop().unwrap();
        let terminal_correlation = termination.correlation();
        assert_eq!(
            terminal_correlation.frontend_command_id(),
            old_correlation.frontend_command_id()
        );
        assert_ne!(
            terminal_correlation.renderer_call_id(),
            old_correlation.renderer_call_id()
        );
        assert_eq!(
            terminal_correlation.dispatched_attachment_id(),
            Some(new_attachment)
        );
        assert!(
            old_sender
                .send(serde_json::json!({
                    "id": old_correlation.renderer_call_id().get(),
                    "result": {},
                }))
                .is_err(),
            "replacement must invalidate an old renderer callback before Page teardown"
        );

        let PreparedRendererCallTermination::CommandReply {
            correlation,
            response_sender,
        } = termination
        else {
            panic!("a synthesized command must use CommandReply termination")
        };
        assert_eq!(correlation, terminal_correlation);
        response_sender
            .send(serde_json::json!({
                "id": terminal_correlation.renderer_call_id().get(),
                "error": {
                    "code": -32000,
                    "message": "Inspected target navigated or closed",
                },
            }))
            .unwrap();
        let completion = response_receiver.await.unwrap();
        assert_eq!(
            completion.call_id,
            terminal_correlation.renderer_call_id().get()
        );
        assert_eq!(
            completion.renderer_agent_attachment_id(),
            Some(new_attachment)
        );
        assert_eq!(
            registry.take_frontend_command_for_renderer_if_attachment_matches(
                old_correlation.renderer_call_id(),
                Some(old_attachment),
            ),
            None
        );
        assert_eq!(
            registry.take_frontend_command_for_renderer_if_attachment_matches(
                terminal_correlation.renderer_call_id(),
                Some(new_attachment),
            ),
            Some(terminal_correlation)
        );
    }

    #[test]
    fn devtools_session_termination_revokes_without_renderer_identity_rotation() {
        let frontend_id = FrontendCommandId::new(43);
        let old_attachment = RendererAgentAttachmentId::allocate();
        let new_attachment = RendererAgentAttachmentId::allocate();
        let frontend = ParsedCdpCommand::parse_str(
            r#"{"id":43,"method":"Runtime.evaluate","params":{"expression":"43"}}"#,
        )
        .expect("frontend command must parse at ingress");
        let mut registry = PendingRendererCommandRegistry::<()>::default();
        let prepared = registry
            .try_register_renderer_call(
                frontend_id,
                Some(old_attachment),
                RendererCommandDescriptor::from_frontend_policy(
                    frontend.json().to_owned(),
                    frontend.renderer_policy(),
                    RendererInspectorResponseDelivery::DevToolsSession,
                ),
            )
            .unwrap();
        let (old_correlation, old_sender, response_receiver) = prepared.into_parts();
        assert!(response_receiver.is_none());
        registry.next_renderer_call_id = None;

        let mut terminations = registry
            .prepare_terminations_from_attachment(old_attachment, new_attachment)
            .expect("session termination must not allocate a renderer call id");

        assert_eq!(terminations.len(), 1);
        let termination = terminations.pop().unwrap();
        assert_eq!(termination.correlation(), old_correlation);
        assert!(matches!(
            termination,
            PreparedRendererCallTermination::DevToolsSession {
                correlation
            } if correlation == old_correlation
        ));
        assert_eq!(
            registry.renderer_call_for_frontend(frontend_id),
            Some(old_correlation),
            "the session owner consumes the original correlation when it emits the error"
        );
        assert!(
            old_sender
                .send(serde_json::json!({
                    "id": old_correlation.renderer_call_id().get(),
                    "result": {},
                }))
                .is_err(),
            "preparing the direct session error must revoke the old renderer lease"
        );
        assert_eq!(
            registry.take_frontend_command_for_renderer_if_attachment_matches(
                old_correlation.renderer_call_id(),
                Some(old_attachment),
            ),
            Some(old_correlation)
        );
    }

    #[tokio::test]
    async fn terminal_drain_completes_receivers_and_removes_all_correlations() {
        let first_id = FrontendCommandId::new(51);
        let second_id = FrontendCommandId::new(52);
        let attachment = RendererAgentAttachmentId::allocate();
        let mut registry = PendingRendererCommandRegistry::<()>::default();
        let first = registry
            .try_register_renderer_call(
                first_id,
                Some(attachment),
                descriptor(first_id.get(), "Runtime.evaluate"),
            )
            .unwrap();
        let second = registry
            .try_register_renderer_call(
                second_id,
                Some(attachment),
                descriptor(second_id.get(), "Console.clearMessages"),
            )
            .unwrap();
        let (_, first_sender, first_receiver) = first.into_parts();
        let (_, second_sender, second_receiver) = second.into_parts();
        let first_receiver = first_receiver
            .expect("a synthesized CommandReply call must allocate a response receiver");
        let second_receiver = second_receiver
            .expect("a synthesized CommandReply call must allocate a response receiver");

        let terminated = registry.terminate_all_renderer_calls("Inspector detached");

        assert_eq!(
            terminated
                .iter()
                .map(|correlation| correlation.frontend_command_id().get())
                .collect::<Vec<_>>(),
            vec![first_id.get(), second_id.get()]
        );
        assert!(first_sender.send(serde_json::json!({"id": 1})).is_err());
        assert!(second_sender.send(serde_json::json!({"id": 2})).is_err());
        for completion in [
            first_receiver.await.unwrap(),
            second_receiver.await.unwrap(),
        ] {
            assert_eq!(completion.renderer_agent_attachment_id(), None);
            let response = completion
                .output
                .protocol_response(completion.call_id)
                .expect("terminal response");
            assert_eq!(response["error"]["code"], -32000);
            assert_eq!(response["error"]["message"], "Inspector detached");
        }
        assert!(registry.renderer_calls_by_frontend.is_empty());
        assert!(registry.frontend_commands_by_renderer.is_empty());
    }

    #[test]
    fn terminate_policy_is_not_prepared_for_replay() {
        let frontend_id = FrontendCommandId::new(43);
        let old_attachment = RendererAgentAttachmentId::allocate();
        let new_attachment = RendererAgentAttachmentId::allocate();
        let mut registry = PendingRendererCommandRegistry::<()>::default();
        let correlation = registry
            .try_register_renderer_call(
                frontend_id,
                Some(old_attachment),
                descriptor(frontend_id.get(), "Runtime.evaluate"),
            )
            .unwrap()
            .correlation();

        assert!(
            registry
                .prepare_replays_from_attachment(old_attachment, new_attachment)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            registry.renderer_call_for_frontend(frontend_id),
            Some(correlation)
        );
    }
}
