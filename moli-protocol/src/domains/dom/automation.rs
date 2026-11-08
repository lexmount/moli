use crate::automation::{AutomationCommand, AutomationResult, DevToolsError, DevToolsErrorKind};
use crate::conn::{
    CdpConnection, CommandWorkflowContext, PreparedRuntimeDomQuery, ProtocolWorkflowDriver,
};

use super::child_frame::PreparedChildFrameDomCommand;

pub(crate) struct PreparedAutomationDomCommand(AutomationDomCommandKind);

enum AutomationDomCommandKind {
    Page(Box<PreparedRuntimeDomQuery>),
    Child(Box<PreparedChildFrameDomCommand>),
}

impl PreparedAutomationDomCommand {
    pub(crate) fn prepare(
        conn: &mut CdpConnection,
        command: AutomationCommand,
    ) -> Result<Self, DevToolsError> {
        let owner = conn
            .command_owner_scope_for_devtools_context(command.context())
            .ok_or_else(|| {
                DevToolsError::new(DevToolsErrorKind::NoSuchTarget, DomAutomationTargetMissing)
            })?;
        if let Some(target) = command.context().target_id.as_ref()
            && conn
                .target_session_route_for_target_id(target.as_str())
                .is_none()
        {
            return PreparedChildFrameDomCommand::prepare(
                conn,
                owner,
                target.as_str().to_owned(),
                command,
            )
            .map(|command| Self(AutomationDomCommandKind::Child(Box::new(command))));
        }
        PreparedRuntimeDomQuery::prepare(conn, owner, command)
            .map(|query| Self(AutomationDomCommandKind::Page(Box::new(query))))
    }

    pub(crate) async fn run(
        self,
        driver: &mut ProtocolWorkflowDriver,
        context: &mut CommandWorkflowContext,
    ) -> Result<AutomationResult, DevToolsError> {
        match self.0 {
            AutomationDomCommandKind::Page(query) => {
                query
                    .run(
                        driver,
                        context.context_mut().renderer_output_predecessor_mut(),
                    )
                    .await
            }
            AutomationDomCommandKind::Child(command) => command.run(driver, context).await,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("NoSuchTarget")]
struct DomAutomationTargetMissing;
