use anyhow::Result;

use super::{CdpConnection, RuntimeInspectorError};
use crate::conn::{SessionDisposalWait, TargetWorkerProtocolAttachmentIdentity};

/// The native attachment and control-plane binding admitted by one Worker
/// command. Neither identity follows a later reuse of its literal session id.
#[derive(Clone)]
pub(crate) struct WorkerRuntimeScope {
    attachment: TargetWorkerProtocolAttachmentIdentity,
    binding: SessionDisposalWait,
}

impl WorkerRuntimeScope {
    pub(crate) fn capture(conn: &CdpConnection, session: Option<&str>) -> Result<Self> {
        let session = session.ok_or(RuntimeInspectorError::UnknownSession)?;
        Ok(Self {
            attachment: conn
                .committed_worker_protocol_attachment_identity(session)
                .cloned()
                .ok_or(RuntimeInspectorError::UnknownSession)?,
            binding: conn
                .session_binding_lifetime(session)
                .ok_or(RuntimeInspectorError::UnknownSession)?,
        })
    }

    pub(crate) fn session_id(&self) -> &str {
        self.attachment.session_id()
    }

    pub(crate) fn binding_is_current(&self, conn: &CdpConnection) -> bool {
        !self.binding.is_closed()
            && conn.session_binding_lifetime(self.session_id()).as_ref() == Some(&self.binding)
            && conn.committed_worker_protocol_attachment_identity(self.session_id())
                == Some(&self.attachment)
    }

    pub(crate) fn can_configure(&self, conn: &CdpConnection) -> bool {
        self.binding_is_current(conn)
            && self.attachment.is_current()
            && !conn.session_is_closing(self.session_id())
    }
}
