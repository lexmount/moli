use std::collections::HashSet;

use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken};

use crate::runtime::{
    RendererInspectorPauseCommandEffect, RendererRuntimeCommandCausalIdentity,
    RendererRuntimeInspectorMessage,
};

pub(super) struct RendererInspectorPauseCommandDispatch {
    pub(super) command_id: u64,
    pub(super) transition: RendererInspectorPauseCommandTransition,
}

pub(super) struct RendererInspectorPauseCommandTransition {
    pub(super) causal_identity: RendererRuntimeCommandCausalIdentity,
    pub(super) effect: RendererInspectorPauseCommandEffect,
    pub(super) response_succeeded: bool,
    pub(super) awaiting_resumed: HashSet<(RendererDevToolsAgentToken, DevToolsSessionKey)>,
    pub(super) awaiting_repause: HashSet<(RendererDevToolsAgentToken, DevToolsSessionKey)>,
}

impl RendererInspectorPauseCommandTransition {
    pub(super) fn is_complete(&self) -> bool {
        match self.effect {
            RendererInspectorPauseCommandEffect::None => true,
            RendererInspectorPauseCommandEffect::Resume => self.awaiting_resumed.is_empty(),
            RendererInspectorPauseCommandEffect::Step => {
                self.awaiting_resumed.is_empty() && self.awaiting_repause.is_empty()
            }
        }
    }

    pub(super) fn observe_notification(
        &mut self,
        session: &(RendererDevToolsAgentToken, DevToolsSessionKey),
        is_resumed_notification: bool,
        is_paused_notification: bool,
    ) -> bool {
        if self.effect == RendererInspectorPauseCommandEffect::None {
            return false;
        }
        if is_resumed_notification && self.awaiting_resumed.remove(session) {
            if self.effect == RendererInspectorPauseCommandEffect::Step {
                self.awaiting_repause.insert(session.clone());
            }
            return true;
        }
        is_paused_notification
            && self.effect == RendererInspectorPauseCommandEffect::Step
            && self.awaiting_repause.remove(session)
    }

    pub(super) fn output_route(&self) -> RendererInspectorPauseCommandOutputRoute {
        RendererInspectorPauseCommandOutputRoute {
            causal_identity: self.causal_identity.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RendererInspectorPauseCommandOutputRoute {
    pub(crate) causal_identity: RendererRuntimeCommandCausalIdentity,
}

pub(super) struct RendererInspectorPausePreface {
    pub(super) id: u64,
    pub(super) agent_token: RendererDevToolsAgentToken,
    pub(super) session: DevToolsSessionKey,
    pub(super) messages: Vec<RendererRuntimeInspectorMessage>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RendererInspectorPauseNotificationRoute {
    OrdinaryTurn,
    PublishImmediately {
        preface: Vec<RendererRuntimeInspectorMessage>,
        command_output: Option<RendererInspectorPauseCommandOutputRoute>,
    },
    Drop,
}
