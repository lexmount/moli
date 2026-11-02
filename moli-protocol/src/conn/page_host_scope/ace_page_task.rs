use super::*;

impl ProtocolPageHostScope {
    pub(crate) fn capture_for_ace_page_task(
        conn: &CdpConnection,
        owner: &CommandOwnerScope,
    ) -> Result<Self> {
        let mut scope = Self::capture(conn, owner)?;
        // Embedder work belongs to the target, independently of any CDP attachment.
        scope.binding = None;
        scope.session_id = None;
        anyhow::ensure!(scope.can_start_operation(conn), NoDocumentLoaded);
        Ok(scope)
    }
}
