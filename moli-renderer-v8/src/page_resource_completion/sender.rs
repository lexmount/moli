use crate::page_task_queue::RendererPageNetworkingRoute;

use super::{RendererPageResourceCompletion, RendererResourceCompletionRouteClosed};

#[cfg(test)]
use super::RendererPageResourceCompletionOwner;
#[cfg(test)]
use crate::resource_ready::RendererPageTaskReadyMetadata;

/// Producer-only view of the Page's shared HTML networking source.
///
/// Resource producers retain their narrow typed API, while the underlying
/// residence is shared with other networking tasks. This prevents a resource
/// completion and a text-track networking step from acquiring separate
/// scheduler fairness slots.
#[derive(Debug, Clone)]
pub(crate) struct RendererPageResourceCompletionSender {
    route: RendererPageNetworkingRoute,
}

impl RendererPageResourceCompletionSender {
    pub(crate) fn new(route: RendererPageNetworkingRoute) -> Self {
        Self { route }
    }

    pub(crate) fn send(
        &self,
        completion: RendererPageResourceCompletion,
    ) -> Result<(), RendererResourceCompletionRouteClosed> {
        self.route
            .send(completion.into())
            .map_err(|_| RendererResourceCompletionRouteClosed)
    }

    #[cfg(test)]
    pub(crate) fn same_route_as(&self, other: &Self) -> bool {
        self.route.same_route_as(&other.route)
    }
}

/// Resource-only adapter used by low-level lane tests. Its implementations
/// still consume the production networking source; it does not own a second
/// queue or bypass source ordering.
#[cfg(test)]
pub(crate) trait RendererPageResourceCompletionTestSource {
    fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata>;
    fn next_ready_owner(&mut self) -> Option<RendererPageResourceCompletionOwner>;
    fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageResourceCompletion,
    )>;
}

#[cfg(test)]
impl RendererPageResourceCompletionTestSource
    for crate::page_task_queue::RendererPageNetworkingSource
{
    fn next_ready_metadata(&mut self) -> Option<RendererPageTaskReadyMetadata> {
        crate::page_task_queue::RendererPageNetworkingSource::next_ready_metadata(self)
    }

    fn next_ready_owner(&mut self) -> Option<RendererPageResourceCompletionOwner> {
        crate::page_task_queue::RendererPageNetworkingSource::next_ready_owner(self)
    }

    fn pop_front(
        &mut self,
    ) -> Option<(
        RendererPageTaskReadyMetadata,
        RendererPageResourceCompletion,
    )> {
        crate::page_task_queue::RendererPageNetworkingSource::pop_front(self)
    }
}
