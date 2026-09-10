mod keepalive;
mod registry;

pub(crate) use keepalive::{DetachedKeepaliveLoadDiagnostics, DetachedKeepaliveLoadRegistry};
pub(crate) use registry::{
    ResourceLoadCancellationObserver, ResourceLoadDisposition, ResourceLoadId, ResourceLoadKind,
    ResourceLoadLease, ResourceLoadRegistry, ResourceLoadRegistryDiagnostics,
};

#[cfg(test)]
pub(crate) fn resource_load_lease_for_test(
    request_client: crate::network::ResourceRequestClient,
    cancel_handle: Option<moli_fetch::FetchCancelHandle>,
) -> ResourceLoadLease {
    ResourceLoadRegistry::new(
        crate::network::RendererResourceTaskRunner::from_current_tokio()
            .expect("resource-load lease test must own a Tokio runtime"),
    )
    .register(
        ResourceLoadKind::Fetch,
        ResourceLoadDisposition::Ordinary,
        request_client,
        cancel_handle,
    )
    .expect("test registry should accept resource load")
}
