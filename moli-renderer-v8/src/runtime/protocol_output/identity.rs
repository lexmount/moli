use std::{
    num::NonZeroU64,
    sync::atomic::{AtomicU64, Ordering},
};

use moli_page_types::RendererDevToolsAgentToken;

use crate::runtime::{PageId, RendererBrowserContextRuntimeId, RendererOwnerLocalHostId};

static NEXT_RENDERER_OUTPUT_STREAM_EPOCH: AtomicU64 = AtomicU64::new(1);
static NEXT_RENDERER_OUTPUT_FENCE_LEASE_ID: AtomicU64 = AtomicU64::new(1);

/// Distinguishes two lifetimes that reuse the same logical renderer residence.
///
/// A page keeps one epoch across cross-document navigation because its
/// renderer agent and owner residence remain the same. Replacing that
/// residence or agent allocates a new epoch, so a delayed publication cannot
/// be mistaken for output from the replacement.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RendererOutputStreamEpoch(NonZeroU64);

impl RendererOutputStreamEpoch {
    pub(crate) fn allocate() -> Self {
        let raw = NEXT_RENDERER_OUTPUT_STREAM_EPOCH
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("renderer output stream epoch exhausted");
        Self(NonZeroU64::new(raw).expect("renderer output stream epoch allocator returned zero"))
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// The renderer-owned residence whose turns form one ordered output stream.
///
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RendererOutputResidenceIdentity {
    Page {
        owner_local_host_id: RendererOwnerLocalHostId,
        page_id: PageId,
    },
    DedicatedWorker {
        browser_context_runtime_id: RendererBrowserContextRuntimeId,
        instance_id: u64,
    },
    SharedWorker {
        browser_context_runtime_id: RendererBrowserContextRuntimeId,
        instance_id: u64,
    },
    ServiceWorker {
        browser_context_runtime_id: RendererBrowserContextRuntimeId,
        version_id: u64,
    },
}

/// Stable source identity for one renderer output stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererOutputStreamIdentity {
    residence: RendererOutputResidenceIdentity,
    renderer_agent: RendererDevToolsAgentToken,
    epoch: RendererOutputStreamEpoch,
}

impl RendererOutputStreamIdentity {
    pub(crate) fn new_page(
        owner_local_host_id: RendererOwnerLocalHostId,
        page_id: PageId,
        renderer_agent: RendererDevToolsAgentToken,
    ) -> Self {
        Self {
            residence: RendererOutputResidenceIdentity::Page {
                owner_local_host_id,
                page_id,
            },
            renderer_agent,
            epoch: RendererOutputStreamEpoch::allocate(),
        }
    }

    pub(crate) fn new_shared_worker(
        browser_context_runtime_id: RendererBrowserContextRuntimeId,
        instance_id: u64,
    ) -> Self {
        Self {
            residence: RendererOutputResidenceIdentity::SharedWorker {
                browser_context_runtime_id,
                instance_id,
            },
            renderer_agent: RendererDevToolsAgentToken::allocate(),
            epoch: RendererOutputStreamEpoch::allocate(),
        }
    }

    pub(crate) fn new_dedicated_worker(
        browser_context_runtime_id: RendererBrowserContextRuntimeId,
        instance_id: u64,
    ) -> Self {
        Self {
            residence: RendererOutputResidenceIdentity::DedicatedWorker {
                browser_context_runtime_id,
                instance_id,
            },
            renderer_agent: RendererDevToolsAgentToken::allocate(),
            epoch: RendererOutputStreamEpoch::allocate(),
        }
    }

    pub(crate) fn new_service_worker(
        browser_context_runtime_id: RendererBrowserContextRuntimeId,
        version_id: u64,
    ) -> Self {
        Self {
            residence: RendererOutputResidenceIdentity::ServiceWorker {
                browser_context_runtime_id,
                version_id,
            },
            renderer_agent: RendererDevToolsAgentToken::allocate(),
            epoch: RendererOutputStreamEpoch::allocate(),
        }
    }

    #[doc(hidden)]
    pub fn new_page_for_protocol_test(page_id: PageId) -> Self {
        Self::new_page(
            RendererOwnerLocalHostId::new_for_testing(1),
            page_id,
            RendererDevToolsAgentToken::allocate(),
        )
    }

    #[doc(hidden)]
    pub fn new_shared_worker_for_protocol_test(instance_id: u64) -> Self {
        Self::new_shared_worker(
            RendererBrowserContextRuntimeId::new_for_testing(1),
            instance_id,
        )
    }

    pub fn residence(self) -> RendererOutputResidenceIdentity {
        self.residence
    }

    pub fn renderer_agent(self) -> RendererDevToolsAgentToken {
        self.renderer_agent
    }

    pub fn epoch(self) -> RendererOutputStreamEpoch {
        self.epoch
    }
}

/// Position of one non-empty renderer publication in its exact source stream.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererOutputCursor {
    stream: RendererOutputStreamIdentity,
    sequence: NonZeroU64,
}

/// Process-unique lifetime token for one cursor exported outside the concrete
/// renderer transport.
///
/// Ordering is still represented by [`RendererOutputCursor`]. This token only
/// prevents protocol ingress from forgetting a closed stream while another
/// channel still owns a cursor that may query it.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RendererOutputFenceLeaseId(NonZeroU64);

impl RendererOutputFenceLeaseId {
    pub(crate) fn allocate() -> Self {
        let raw = NEXT_RENDERER_OUTPUT_FENCE_LEASE_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("renderer output fence lease ID exhausted");
        Self(NonZeroU64::new(raw).expect("renderer output fence lease allocator returned zero"))
    }

    #[doc(hidden)]
    pub fn new_for_test(raw: u64) -> Self {
        Self(NonZeroU64::new(raw).expect("test renderer output fence lease ID must be non-zero"))
    }
}

impl RendererOutputCursor {
    pub(crate) fn new(stream: RendererOutputStreamIdentity, sequence: NonZeroU64) -> Self {
        Self { stream, sequence }
    }

    pub fn stream(self) -> RendererOutputStreamIdentity {
        self.stream
    }

    pub fn sequence(self) -> u64 {
        self.sequence.get()
    }

    /// Joins two response fences from the same renderer FIFO.
    ///
    /// Waiting for the later cursor already implies that every earlier
    /// publication in that stream has crossed protocol ingress. A command
    /// cannot join unrelated Page/Worker streams; that would mix target
    /// ownership rather than establish FIFO order.
    pub fn latest_in_same_stream(self, other: Self) -> Self {
        assert_eq!(
            self.stream, other.stream,
            "one command response cannot join unrelated renderer output streams"
        );
        if self.sequence >= other.sequence {
            self
        } else {
            other
        }
    }

    #[doc(hidden)]
    pub fn new_for_test(stream: RendererOutputStreamIdentity, sequence: u64) -> Self {
        Self::new(
            stream,
            NonZeroU64::new(sequence).expect("test renderer output sequence must be non-zero"),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererOutputStreamCloseReason {
    ResidenceRetired,
    RendererAgentReplaced,
}

/// Explicit lifetime boundary for an ordered renderer output stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererOutputStreamControl {
    Opened {
        stream: RendererOutputStreamIdentity,
    },
    Closed {
        stream: RendererOutputStreamIdentity,
        last_published_sequence: Option<NonZeroU64>,
        reason: RendererOutputStreamCloseReason,
    },
}
