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

pub(crate) enum RendererDevToolsLaneEnqueueError<C> {
    TargetClosed(C),
    SessionDetached(C),
}

struct RendererDevToolsSessionLane<C> {
    active_command_id: Option<u64>,
    queued: VecDeque<C>,
    ready: bool,
    detached: bool,
}

impl<C> Default for RendererDevToolsSessionLane<C> {
    fn default() -> Self {
        Self {
            active_command_id: None,
            queued: VecDeque::new(),
            ready: false,
            detached: false,
        }
    }
}

/// Route-local per-session admission state.
///
/// Main and IO each own a distinct instance under a distinct mutex. Sharing
/// this implementation keeps their FIFO, cancellation and first-dispatch
/// invariants aligned without creating a shared cross-route queue.
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
    ) -> Result<(), RendererDevToolsLaneEnqueueError<C>> {
        if self.closed {
            return Err(RendererDevToolsLaneEnqueueError::TargetClosed(command));
        }
        let lane = self.sessions.entry(lane_key.clone()).or_default();
        if lane.detached {
            return Err(RendererDevToolsLaneEnqueueError::SessionDetached(command));
        }
        lane.queued.push_back(command);
        if lane.active_command_id.is_none() && !lane.ready {
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
            let make_ready = !lane.detached && !lane.queued.is_empty() && !lane.ready;
            if make_ready {
                lane.ready = true;
            }
            (make_ready, lane.queued.is_empty())
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
        if lane.queued.is_empty() && lane.active_command_id.is_none() {
            lane.ready = false;
            self.ready_sessions.retain(|ready| ready != &lane_key);
            self.sessions.remove(&lane_key);
        }
        command
    }

    pub(crate) fn detach_session(&mut self, lane_key: &RendererDevToolsSessionLaneKey) -> Vec<C> {
        self.ready_sessions.retain(|ready| ready != lane_key);
        let Some(lane) = self.sessions.get_mut(lane_key) else {
            return Vec::new();
        };
        lane.ready = false;
        lane.detached = true;
        let commands = lane.queued.drain(..).collect();
        if lane.active_command_id.is_none() {
            self.sessions.remove(lane_key);
        }
        commands
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
