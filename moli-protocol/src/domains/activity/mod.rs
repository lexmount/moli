mod contextual_projection;
mod main_document;
mod output_ingress;
mod output_payloads;
mod output_slot;
mod output_work;
mod publication_route;
mod runtime_command_barrier;
mod scheduler_work;
mod subresource;

pub(in crate::domains) use contextual_projection::ProtocolOutputProjectionContext;
pub(crate) use main_document::{
    CompletedDeferredMainDocumentLoadCompletionActivity,
    DeferredMainDocumentLoadCompletionAdmission, MainDocumentDownloadNavigationActivity,
    MainDocumentFailedNavigationActivity, MainDocumentNavigationActivity,
    PendingDeferredMainDocumentLoadCompletionActivity,
};
pub(crate) use output_ingress::{
    OrderedRendererOutputIngress, RendererOutputIngressAdmission,
    ingest_renderer_output_transport_async, project_protocol_local_command_outputs,
};
pub(in crate::domains) use output_payloads::ProtocolOutputPayloads;
pub(in crate::domains) use output_slot::{ProtocolOutputSink, ProtocolOutputSlot};
pub(crate) use output_work::{
    ProtocolOutputWork, RootFrameStoppedLoadingSettlement, RootFrameStoppedLoadingSettlementError,
};
pub(crate) use publication_route::{RendererPublicationOwner, renderer_publication_owners};
pub use runtime_command_barrier::{
    RuntimeCommandOutputBarrierCompletion, RuntimeCommandOutputBarrierPermit,
    RuntimeCommandOutputBarrierTerminal, RuntimeCommandOutputBarriers,
};
pub(crate) use scheduler_work::ReadyProtocolSchedulerWork;
pub use scheduler_work::{
    ProtocolSchedulerWork, ProtocolSchedulerWorkKind, ProtocolWorkPublishSequence,
};
pub(in crate::domains) use subresource::{
    PreparedSubresourceContinueAction,
    flush_prepared_subresource_continue_actions_background_events_async,
    prepare_subresource_continue_action_for_renderer_record,
};
pub(crate) use subresource::{
    flush_post_subresource_auth_activity_background_events_async,
    flush_post_subresource_auth_activity_for_owner_background_events_async,
    flush_post_subresource_fetch_request_activity_background_events_async,
    flush_post_subresource_fetch_request_activity_for_owner_background_events_async,
    flush_post_subresource_response_activity_background_events_async,
    flush_post_subresource_response_activity_for_owner_background_events_async,
};
