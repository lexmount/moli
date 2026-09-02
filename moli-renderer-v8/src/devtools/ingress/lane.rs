use std::collections::{BTreeMap, VecDeque};

use moli_page_types::{DevToolsSessionKey, RendererDevToolsAgentToken};

pub(crate) trait RendererDevToolsIngressCommand {
    fn ingress_command_id(&self) -> u64;
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct RendererDevToolsSessionLaneKey {
    agent_token: RendererDevToolsAgentToken,
    session: DevToolsSessionKey,
}

impl RendererDevToolsSessionLaneKey {
    pub(crate) fn new(
        agent_token: RendererDevToolsAgentToken,
        session: DevToolsSessionKey,
    ) -> Self {
        Self {
            agent_token,
            session,
        }
    }
}

struct RendererDevToolsSessionLane<C> {
    active_command_id: Option<u64>,
    queued: VecDeque<C>,
    ready: bool,
    // Distinct frontend owners may share the Primary lane and detach while an
    // earlier owner cleanup is still queued. Each one-shot guard contributes
    // one arm, so a count is sufficient regardless of completion order.
    pending_detaches: usize,
}

impl<C> Default for RendererDevToolsSessionLane<C> {
    fn default() -> Self {
        Self {
            active_command_id: None,
            queued: VecDeque::new(),
            ready: false,
            pending_detaches: 0,
        }
    }
}

/// Main-receiver admission state for all frontend sessions on one target.
///
/// Chromium gives each session its own ordered Main receiver. The active slot
/// ends when a command first reaches the target agent; asynchronous response
/// completion is deliberately outside that lifetime so a later resume command
/// can enter a nested pause loop.
pub(crate) struct RendererDevToolsSessionLanes<C> {
    sessions: BTreeMap<RendererDevToolsSessionLaneKey, RendererDevToolsSessionLane<C>>,
    ready_sessions: VecDeque<RendererDevToolsSessionLaneKey>,
    closed: bool,
}

impl<C> Default for RendererDevToolsSessionLanes<C> {
    fn default() -> Self {
        Self {
            sessions: BTreeMap::new(),
            ready_sessions: VecDeque::new(),
            closed: false,
        }
    }
}

impl<C: RendererDevToolsIngressCommand> RendererDevToolsSessionLanes<C> {
    pub(crate) fn enqueue(
        &mut self,
        lane_key: RendererDevToolsSessionLaneKey,
        command: C,
    ) -> Result<(), C> {
        if self.closed {
            return Err(command);
        }
        let lane = self.sessions.entry(lane_key.clone()).or_default();
        lane.queued.push_back(command);
        if lane.pending_detaches == 0 && lane.active_command_id.is_none() && !lane.ready {
            lane.ready = true;
            self.ready_sessions.push_back(lane_key);
        }
        Ok(())
    }

    pub(crate) fn claim_next(
        &mut self,
        mut eligible: impl FnMut(&C) -> bool,
    ) -> Option<(RendererDevToolsSessionLaneKey, C)> {
        if self.closed {
            return None;
        }
        let ready_session_count = self.ready_sessions.len();
        for _ in 0..ready_session_count {
            let lane_key = self
                .ready_sessions
                .pop_front()
                .expect("the snapshotted DevTools ready-session count must remain available");
            let eligible_front = self
                .sessions
                .get(&lane_key)
                .and_then(|lane| lane.queued.front())
                .is_some_and(&mut eligible);
            if !eligible_front {
                self.ready_sessions.push_back(lane_key);
                continue;
            }
            let Some(lane) = self.sessions.get_mut(&lane_key) else {
                continue;
            };
            lane.ready = false;
            if lane.active_command_id.is_some() {
                continue;
            }
            let Some(command) = lane.queued.pop_front() else {
                continue;
            };
            lane.active_command_id = Some(command.ingress_command_id());
            return Some((lane_key, command));
        }
        None
    }

    pub(crate) fn assert_active(
        &self,
        lane_key: &RendererDevToolsSessionLaneKey,
        command_id: u64,
        message: &str,
    ) {
        assert_eq!(
            self.sessions
                .get(lane_key)
                .and_then(|lane| lane.active_command_id),
            Some(command_id),
            "{message}"
        );
    }

    pub(crate) fn finish_first_dispatch(
        &mut self,
        lane_key: RendererDevToolsSessionLaneKey,
        command_id: u64,
        message: &str,
    ) -> bool {
        let (make_ready, remove_lane) = {
            let lane = self
                .sessions
                .get_mut(&lane_key)
                .expect("an active DevTools session lane must still exist");
            assert_eq!(lane.active_command_id.take(), Some(command_id), "{message}");
            let make_ready = lane.pending_detaches == 0 && !lane.queued.is_empty() && !lane.ready;
            if make_ready {
                lane.ready = true;
            }
            (
                make_ready,
                lane.pending_detaches == 0
                    && lane.queued.is_empty()
                    && lane.active_command_id.is_none(),
            )
        };
        if make_ready {
            self.ready_sessions.push_back(lane_key.clone());
        }
        if remove_lane {
            self.sessions.remove(&lane_key);
        }
        self.has_ready()
    }

    pub(crate) fn cancel_queued(&mut self, command_id: u64) -> Option<C> {
        let lane_key = self.sessions.iter().find_map(|(key, lane)| {
            lane.queued
                .iter()
                .any(|command| command.ingress_command_id() == command_id)
                .then(|| key.clone())
        })?;
        let lane = self
            .sessions
            .get_mut(&lane_key)
            .expect("a located DevTools session lane must remain present");
        let position = lane
            .queued
            .iter()
            .position(|command| command.ingress_command_id() == command_id)
            .expect("a located DevTools command must remain queued");
        let command = lane.queued.remove(position);
        if lane.pending_detaches == 0 && lane.queued.is_empty() && lane.active_command_id.is_none()
        {
            lane.ready = false;
            self.ready_sessions.retain(|ready| ready != &lane_key);
            self.sessions.remove(&lane_key);
        }
        command
    }

    pub(crate) fn begin_session_detach(
        &mut self,
        lane_key: &RendererDevToolsSessionLaneKey,
    ) -> Vec<C> {
        if self.closed {
            return Vec::new();
        }
        self.ready_sessions.retain(|ready| ready != lane_key);
        let lane = self.sessions.entry(lane_key.clone()).or_default();
        lane.ready = false;
        lane.pending_detaches = lane
            .pending_detaches
            .checked_add(1)
            .expect("renderer DevTools session pending-detach count overflow");
        lane.queued.drain(..).collect()
    }

    /// Releases replacement frontend commands only after every detach queued
    /// ahead of them has destroyed its renderer-side session on the owner
    /// thread.
    pub(crate) fn finish_session_detach(
        &mut self,
        lane_key: &RendererDevToolsSessionLaneKey,
    ) -> bool {
        let Some(lane) = self.sessions.get_mut(lane_key) else {
            return self.has_ready();
        };
        if lane.pending_detaches == 0 {
            debug_assert!(
                self.closed,
                "only a closed target may finish an unarmed detach"
            );
            return self.has_ready();
        }
        lane.pending_detaches -= 1;
        if lane.pending_detaches != 0 {
            return self.has_ready();
        }
        let make_ready = lane.active_command_id.is_none() && !lane.queued.is_empty() && !lane.ready;
        if make_ready {
            lane.ready = true;
            self.ready_sessions.push_back(lane_key.clone());
        }
        if lane.active_command_id.is_none() && lane.queued.is_empty() {
            self.sessions.remove(lane_key);
        }
        self.has_ready()
    }

    pub(crate) fn close_and_drain(&mut self) -> Vec<C> {
        self.closed = true;
        self.drain_queued()
    }

    pub(crate) fn drain_queued(&mut self) -> Vec<C> {
        self.ready_sessions.clear();
        let commands = self
            .sessions
            .values_mut()
            .flat_map(|lane| lane.queued.drain(..))
            .collect();
        self.sessions
            .retain(|_, lane| lane.active_command_id.is_some());
        commands
    }

    pub(crate) fn has_ready(&self) -> bool {
        !self.ready_sessions.is_empty()
    }

    pub(crate) fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub(crate) fn ready_count(&self) -> usize {
        self.ready_sessions.len()
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }
}
