use anyhow::Result;

use crate::automation::{
    DevToolsBrowserContextId, DevToolsTargetId, DevToolsTargetInfo, DevToolsTargetKind,
};

use crate::{DEFAULT_CDP_PAGE_TARGET_ID, DEFAULT_CDP_TAB_TARGET_ID};

use crate::conn::{
    CdpConnection, CommandWorkflowContext, PreparedCommandWorkflow, PreparedTargetHostDelta,
    ProtocolStateHandle, ProtocolWorkflowDriver, TargetEventPlan,
};
use crate::domains::command_output::CommandOutputPlan;

/// The placeholder has no Page to retire. Its tab sessions can still own a
/// tracing cancellation, which must finish outside protocol state access.
pub(crate) struct PreparedDefaultTargetClose {
    events: TargetEventPlan,
    cleanup: Option<crate::domains::session::PreparedClosedTargetSessions>,
    destroyed: Vec<PreparedTargetHostDelta>,
}

impl PreparedDefaultTargetClose {
    pub(crate) fn prepare(conn: &mut CdpConnection, target_id: &str) -> Option<Self> {
        if !conn
            .default_target_lifecycle
            .is_placeholder_target(target_id)
        {
            return None;
        }
        let (detached, destroyed) = conn
            .prepare_target_host_closure(DEFAULT_CDP_PAGE_TARGET_ID)
            .into_parts();
        let events = conn.prepared_target_host_deltas_event_plan(detached);
        let cleanup = conn
            .take_closed_top_level_target_sessions_cleanup_plan(
                DEFAULT_CDP_PAGE_TARGET_ID,
                Some("Render process gone."),
            )
            .map(|plan| {
                crate::conn::ProtocolWorkflowState::new(conn)
                    .prepare_closed_target_sessions(plan, None)
            });
        // Removing its catalog entry and revoking creation are one admission
        // operation. Native cleanup retains its original closing bindings.
        assert!(conn.default_target_lifecycle.close_placeholder(target_id));
        Some(Self {
            events,
            cleanup,
            destroyed,
        })
    }

    pub(crate) fn into_workflow(self, access: ProtocolStateHandle) -> PreparedCommandWorkflow {
        PreparedCommandWorkflow::new(move |mut context| async move {
            let response = CommandOutputPlan::from_devtools_result(
                crate::automation::AutomationResult::CloseTarget(
                    crate::automation::DevToolsCloseTargetResult { success: true },
                ),
            );
            match self
                .run(
                    &mut ProtocolWorkflowDriver::new(access),
                    &mut context,
                    Some(response),
                )
                .await
            {
                Ok(()) => context.complete(CommandOutputPlan::default()),
                Err(error) => context.fail(CommandOutputPlan::error(-32000, error)),
            }
        })
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
        response: Option<CommandOutputPlan>,
    ) -> Result<()> {
        let completed = match self.cleanup {
            Some(cleanup) => Some(cleanup.wait().await),
            None => None,
        };
        context.with_state(driver, move |state, context| {
            let events = state.finish_default_target_close(completed, self.events, self.destroyed);
            let terminal = response.is_some();
            let mut output = response.unwrap_or_default();
            output.extend_background_events(events.into_background_events());
            if terminal {
                context.retain_terminal(output);
            } else {
                context.retain_output(output);
            }
        })
    }
}

impl CdpConnection {}

pub(crate) const DEFAULT_BROWSER_CONTEXT_ID: &str = "BID-default";

/// Lifecycle of the server-published default target.
///
/// A placeholder is a real target-catalog entry, but it deliberately owns no
/// `BrowserContext` or navigation runtime until an operation needs a live page.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DefaultTargetLifecycle {
    #[default]
    Unpublished,
    Placeholder,
    Live,
    Closed,
}

impl DefaultTargetLifecycle {
    pub(crate) fn publish(&mut self) -> bool {
        if *self != Self::Unpublished {
            return false;
        }
        *self = Self::Placeholder;
        true
    }

    pub(crate) fn is_placeholder(&self) -> bool {
        *self == Self::Placeholder
    }

    pub(crate) fn is_live(&self) -> bool {
        *self == Self::Live
    }

    pub(crate) fn is_closed(&self) -> bool {
        *self == Self::Closed
    }

    pub(crate) fn owns(target_id: &str) -> bool {
        matches!(
            target_id,
            DEFAULT_CDP_PAGE_TARGET_ID | DEFAULT_CDP_TAB_TARGET_ID
        )
    }

    pub(crate) fn is_placeholder_target(&self, target_id: &str) -> bool {
        self.is_placeholder() && Self::owns(target_id)
    }

    pub(crate) fn placeholder_page_info(&self) -> Option<DevToolsTargetInfo> {
        self.is_placeholder().then(|| DevToolsTargetInfo {
            target_id: Some(DevToolsTargetId::from(DEFAULT_CDP_PAGE_TARGET_ID)),
            kind: DevToolsTargetKind::Page,
            title: String::new(),
            url: "about:blank".to_owned(),
            attached: false,
            opener_id: None,
            opener_frame_id: None,
            can_access_opener: false,
            browser_context_id: Some(DevToolsBrowserContextId::from(DEFAULT_BROWSER_CONTEXT_ID)),
            moli_popup_id: None,
        })
    }

    pub(crate) fn mark_live(&mut self) {
        if *self != Self::Closed {
            *self = Self::Live;
        }
    }

    pub(crate) fn close_placeholder(&mut self, target_id: &str) -> bool {
        if !self.is_placeholder_target(target_id) {
            return false;
        }
        *self = Self::Closed;
        true
    }

    pub(crate) fn mark_closed(&mut self) {
        *self = Self::Closed;
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_BROWSER_CONTEXT_ID, DefaultTargetLifecycle};
    use crate::{DEFAULT_CDP_PAGE_TARGET_ID, DEFAULT_CDP_TAB_TARGET_ID};

    #[test]
    fn lifecycle_only_exposes_placeholder_metadata_while_published() {
        let mut lifecycle = DefaultTargetLifecycle::default();
        assert!(lifecycle.placeholder_page_info().is_none());
        assert!(lifecycle.publish());
        assert!(!lifecycle.publish());

        let info = lifecycle
            .placeholder_page_info()
            .expect("published placeholder info");
        assert_eq!(
            info.target_id.as_ref().map(|id| id.as_str()),
            Some(DEFAULT_CDP_PAGE_TARGET_ID)
        );
        assert_eq!(
            info.browser_context_id.as_ref().map(|id| id.as_str()),
            Some(DEFAULT_BROWSER_CONTEXT_ID)
        );
        assert!(lifecycle.is_placeholder_target(DEFAULT_CDP_TAB_TARGET_ID));

        lifecycle.mark_live();
        assert!(lifecycle.is_live());
        assert!(lifecycle.placeholder_page_info().is_none());
    }

    #[test]
    fn closed_default_target_cannot_be_republished_or_revived() {
        let mut lifecycle = DefaultTargetLifecycle::default();
        assert!(lifecycle.publish());
        assert!(lifecycle.close_placeholder(DEFAULT_CDP_PAGE_TARGET_ID));
        assert!(!lifecycle.publish());
        lifecycle.mark_live();
        assert!(!lifecycle.is_live());
        assert!(lifecycle.placeholder_page_info().is_none());
    }
}
