mod activation;
mod auto_attach_owner;
pub(crate) use auto_attach_owner::AutoAttachPolicyIdentity;
mod control;
mod default_target;
mod graph;
mod observer;
mod projection;
mod route;
mod session;
mod session_binding;
mod transaction;
mod worker_auto_attach;
mod worker_session;

pub(crate) use activation::{TargetActivationTransition, TargetSurfaceUpdate};
pub(crate) use control::TargetControlPlane;
pub(crate) use default_target::{
    DEFAULT_BROWSER_CONTEXT_ID, DefaultTargetLifecycle, PreparedDefaultTargetClose,
};
pub(crate) use graph::{TargetClosurePlan, TargetHostDelta};
pub(crate) use observer::{TargetHandlerStore, target_destroyed_automation_events};
pub(crate) use route::{CdpSessionRoute, TargetHandlerAccessMode};
pub(crate) use session::{
    CommittedAttachSession, DetachedTargetSession, PreparedAttachSession, SessionDisposalWait,
    TargetSessionRegistry,
};
pub(crate) use transaction::{
    PreparedTargetAttach, PreparedTargetHostClosure, PreparedTargetHostDelta, SessionDisposalPlan,
    SessionDisposalTarget, TargetAttachRollbackPlan, TargetAttachSessionCommit,
    TargetClosureCleanupPlan, TargetEventPlan, TargetSessionDetachCleanupPlan,
};
pub(crate) use worker_session::TargetWorkerProtocolAttachmentIdentity;
