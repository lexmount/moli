//! Network decisions are not Page turns and cannot refresh a frozen Page view.

use anyhow::{Result, bail};
use moli_renderer_v8::network::{ResourceInterceptionPause, ResourceInterceptionReply};

use super::{CompletedPageCommand, PendingPageCommand};

pub enum PendingResourceCommand {
    Page(PendingPageCommand),
    Network(Result<ResourceInterceptionReply>),
}

impl From<PendingPageCommand> for PendingResourceCommand {
    fn from(command: PendingPageCommand) -> Self {
        Self::Page(command)
    }
}

impl PendingResourceCommand {
    pub async fn wait(self) -> Result<CompletedResourceCommand> {
        match self {
            Self::Page(command) => command
                .wait()
                .await
                .map(|command| CompletedResourceCommand::Page(Box::new(command))),
            Self::Network(reply) => reply.map(CompletedResourceCommand::Network),
        }
    }
}

pub enum CompletedResourceCommand {
    Page(Box<CompletedPageCommand>),
    Network(ResourceInterceptionReply),
}

impl CompletedResourceCommand {
    pub fn renderer_output_predecessor(&self) -> Option<crate::RendererOutputFence> {
        match self {
            Self::Page(command) => command.renderer_output_predecessor(),
            Self::Network(_) => None,
        }
    }

    pub fn into_unit_reply(self) -> Result<()> {
        match self {
            Self::Page(command) => command.into_unit_reply(),
            Self::Network(ResourceInterceptionReply::Unit) => Ok(()),
            Self::Network(ResourceInterceptionReply::Started) => {
                bail!("expected a unit network reply")
            }
        }
    }

    pub fn into_subresource_continue_outcome(
        self,
    ) -> Result<super::PendingSubresourceContinueOutcome> {
        match self {
            Self::Page(command) => command.into_subresource_continue_outcome(),
            Self::Network(ResourceInterceptionReply::Started) => {
                Ok(super::PendingSubresourceContinueOutcome::Started)
            }
            Self::Network(ResourceInterceptionReply::Unit) => {
                bail!("expected a network start reply")
            }
        }
    }

    pub fn into_page_command(self) -> Result<CompletedPageCommand> {
        match self {
            Self::Page(command) => Ok(*command),
            Self::Network(_) => bail!("a network decision has no Page state completion"),
        }
    }
}

impl super::PageDocumentCommandTarget {
    pub fn native_response_body(&self) -> Option<&crate::network::ResourceInterceptionBody> {
        self.network_pause.as_ref()?.response_body()
    }
    pub fn update_native_interception_policy(
        &self,
        classes: crate::page::SubresourceInterceptionClasses,
    ) -> bool {
        if let Some(pause) = &self.network_pause {
            pause.set_target_interception(classes);
            true
        } else {
            false
        }
    }

    pub fn has_native_network_pause(&self) -> bool {
        self.network_pause.is_some()
    }

    pub fn with_network_pause(mut self, pause: ResourceInterceptionPause) -> Self {
        self.network_pause = Some(pause);
        self
    }

    pub fn start_resource_command(
        &self,
        command: crate::RendererPageCommand,
    ) -> Result<PendingResourceCommand> {
        match &self.network_pause {
            Some(pause) => Ok(PendingResourceCommand::Network(pause.dispatch(command))),
            None => self.start(command).map(PendingResourceCommand::Page),
        }
    }
}
