mod ordered_stream;
mod prepared_outputs;
mod projection;
mod renderer;
pub(crate) use projection::RendererOutputProjection;

pub(crate) use ordered_stream::{
    AdmittedRendererOutputPublication, OrderedRendererOutputIngress, RendererOutputIngressAdmission,
};
pub(crate) use prepared_outputs::{PreparedProtocolOutputs, ProtocolOutputProjectionMode};
pub(crate) use renderer::ingest_renderer_output_transport_async;

use crate::conn::{CdpConnection, CommandDispatchContext, CommandOwnerScope};

/// Projects protocol-owned output accumulated at a command boundary.
///
/// DOMStorage subscriptions are owned by the protocol BrowserContext rather
/// than by a renderer output stream. They therefore keep an explicit
/// command-local flush, while renderer output must arrive exclusively through
/// concrete publications and exact cursor fences.
pub(crate) async fn project_protocol_local_command_outputs(
    conn: &mut CdpConnection,
    owner: &CommandOwnerScope,
    command: &mut CommandDispatchContext,
) {
    PreparedProtocolOutputs::from_protocol_local_command_boundary(conn, owner)
        .await
        .project_async(conn, owner, command)
        .await;
}
