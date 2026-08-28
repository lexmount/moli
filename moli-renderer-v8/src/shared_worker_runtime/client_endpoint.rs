use std::collections::HashMap;

use moli_shared_worker::{SharedWorkerClientId, SharedWorkerClientOwnerId};

use crate::{document_runtime::DomHandle, runtime::RendererBrowserContextRuntime};

use super::client::SharedWorkerClientEndpointDisposition;

/// Exact error target produced after Page authorization and endpoint lifetime
/// settlement.
///
/// `Closed` has a separate state-only API and therefore cannot be represented
/// as a missing error target. Error callers receive the wrapper and its
/// post-event lifetime as one indivisible value.
pub(crate) struct AppliedSharedWorkerClientErrorTarget<'s> {
    owner_scope: crate::native_bridge::OwnerDispatchScope,
    wrapper: v8::Local<'s, v8::Object>,
    endpoint_disposition: SharedWorkerClientEndpointDisposition,
}

impl<'s> AppliedSharedWorkerClientErrorTarget<'s> {
    pub(crate) fn into_parts(
        self,
    ) -> (
        crate::native_bridge::OwnerDispatchScope,
        v8::Local<'s, v8::Object>,
        SharedWorkerClientEndpointDisposition,
    ) {
        (self.owner_scope, self.wrapper, self.endpoint_disposition)
    }
}

/// Renderer-context local owner for SharedWorker client endpoints.
///
/// Chromium keeps a connected `SharedWorker` wrapper alive through
/// `SharedWorkerClient`'s `Persistent<SharedWorker>` while the per-window
/// client receiver is connected. Moli mirrors that at the page context
/// boundary with strong V8 handles, but this owner must stay renderer local:
/// browser-context state/host state should only see client ids and endpoint
/// events, never V8 objects.
#[derive(Default)]
pub(crate) struct SharedWorkerClientEndpointOwner {
    endpoints: HashMap<SharedWorkerClientId, SharedWorkerClientEndpoint>,
}

/// Renderer-local identity for the frame/context that owns a SharedWorker
/// client endpoint.
///
/// Chromium uses `GlobalRenderFrameHostId` when aggregating SharedWorker client
/// observer state. Moli does not have RenderFrameHost, so this stores the
/// matching page or child browsing-context identity next to the neutral owner
/// id used by the shared registry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SharedWorkerClientFrameIdentity {
    owner_id: SharedWorkerClientOwnerId,
    execution_context: crate::native_bridge::WindowExecutionContextIdentity,
}

/// Renderer-local equivalent of Chromium's per-window SharedWorker client
/// receiver. Dropping or explicitly disconnecting it removes the runtime
/// client; runtime-originated terminal events disarm it first because the
/// runtime state transition has already consumed that client.
pub(crate) struct SharedWorkerClientEndpointReceiver {
    client_id: SharedWorkerClientId,
    frame_identity: SharedWorkerClientFrameIdentity,
    browser_context_runtime: Option<RendererBrowserContextRuntime>,
}

struct SharedWorkerClientEndpoint {
    receiver: SharedWorkerClientEndpointReceiver,
    wrapper: v8::Global<v8::Object>,
}

impl SharedWorkerClientEndpointReceiver {
    pub(crate) fn new(
        client_id: SharedWorkerClientId,
        frame_identity: SharedWorkerClientFrameIdentity,
        browser_context_runtime: RendererBrowserContextRuntime,
    ) -> Self {
        Self {
            client_id,
            frame_identity,
            browser_context_runtime: Some(browser_context_runtime),
        }
    }

    fn client_id(&self) -> SharedWorkerClientId {
        self.client_id
    }

    fn frame_identity(&self) -> SharedWorkerClientFrameIdentity {
        self.frame_identity
    }

    fn disconnect(mut self) {
        if let Some(browser_context_runtime) = self.browser_context_runtime.take() {
            browser_context_runtime.remove_shared_worker_client(self.client_id);
        }
    }

    fn forget_after_runtime_terminal(mut self) {
        self.browser_context_runtime.take();
    }
}

impl SharedWorkerClientFrameIdentity {
    pub(crate) fn new(
        owner_id: SharedWorkerClientOwnerId,
        execution_context: crate::native_bridge::WindowExecutionContextIdentity,
    ) -> Self {
        Self {
            owner_id,
            execution_context,
        }
    }

    pub(crate) fn owner_id(self) -> SharedWorkerClientOwnerId {
        self.owner_id
    }

    pub(crate) fn owner_dispatch_scope(self) -> crate::native_bridge::OwnerDispatchScope {
        self.execution_context.dispatch_scope()
    }

    pub(crate) fn execution_context(self) -> crate::native_bridge::WindowExecutionContextIdentity {
        self.execution_context
    }

    pub(crate) fn is_child_context(self, handle: DomHandle) -> bool {
        matches!(
            self.execution_context.dispatch_scope(),
            crate::native_bridge::OwnerDispatchScope::Child(child_handle)
                if child_handle == handle
        )
    }

    #[cfg(test)]
    fn child_handle(self) -> Option<DomHandle> {
        match self.execution_context.dispatch_scope() {
            crate::native_bridge::OwnerDispatchScope::Child(handle) => Some(handle),
            crate::native_bridge::OwnerDispatchScope::Top => None,
        }
    }
}

impl Drop for SharedWorkerClientEndpointReceiver {
    fn drop(&mut self) {
        if let Some(browser_context_runtime) = self.browser_context_runtime.take() {
            browser_context_runtime.remove_shared_worker_client(self.client_id);
        }
    }
}

impl SharedWorkerClientEndpointOwner {
    pub(crate) fn insert(
        &mut self,
        scope: &mut v8::PinScope<'_, '_>,
        receiver: SharedWorkerClientEndpointReceiver,
        wrapper: v8::Local<'_, v8::Object>,
    ) {
        self.endpoints.insert(
            receiver.client_id(),
            SharedWorkerClientEndpoint {
                receiver,
                wrapper: v8::Global::new(scope, wrapper),
            },
        );
    }

    fn target<'s>(
        &self,
        scope: &mut v8::PinScope<'s, '_>,
        client_id: SharedWorkerClientId,
    ) -> Option<(
        crate::native_bridge::OwnerDispatchScope,
        v8::Local<'s, v8::Object>,
    )> {
        self.endpoints.get(&client_id).map(|endpoint| {
            (
                endpoint.receiver.frame_identity().owner_dispatch_scope(),
                v8::Local::new(scope, &endpoint.wrapper),
            )
        })
    }

    pub(crate) fn execution_context_identity(
        &self,
        client_id: SharedWorkerClientId,
    ) -> Option<crate::native_bridge::WindowExecutionContextIdentity> {
        self.endpoints
            .get(&client_id)
            .map(|endpoint| endpoint.receiver.frame_identity().execution_context())
    }

    fn assert_authorized_identity(
        &self,
        client_id: SharedWorkerClientId,
        expected: crate::native_bridge::WindowExecutionContextIdentity,
    ) {
        let actual = self.execution_context_identity(client_id);
        assert_eq!(
            actual,
            Some(expected),
            "authorized SharedWorker client target changed inside one owner turn"
        );
    }

    /// Apply one already-authorized runtime close without manufacturing a V8
    /// dispatch target. Runtime terminal cleanup only forgets the local
    /// wrapper; it must not call back into remove-client and race the runtime
    /// transition that already produced this close.
    pub(crate) fn apply_authorized_close(
        &mut self,
        client_id: SharedWorkerClientId,
        expected: crate::native_bridge::WindowExecutionContextIdentity,
    ) {
        self.assert_authorized_identity(client_id, expected);
        assert!(
            self.forget_runtime_endpoint(client_id),
            "authorized SharedWorker close must retire its exact endpoint"
        );
    }

    /// Resolve an already-authorized error target and settle its endpoint
    /// lifetime before callers run arbitrary JS handlers.
    pub(crate) fn apply_authorized_error<'s>(
        &mut self,
        scope: &mut v8::PinScope<'s, '_>,
        client_id: SharedWorkerClientId,
        expected: crate::native_bridge::WindowExecutionContextIdentity,
        endpoint_disposition: SharedWorkerClientEndpointDisposition,
    ) -> AppliedSharedWorkerClientErrorTarget<'s> {
        self.assert_authorized_identity(client_id, expected);
        let (owner_scope, wrapper) = self
            .target(scope, client_id)
            .expect("authorized SharedWorker error must retain its exact V8 wrapper");
        if endpoint_disposition == SharedWorkerClientEndpointDisposition::Retire {
            assert!(
                self.forget_runtime_endpoint(client_id),
                "terminal SharedWorker error must retire its exact endpoint"
            );
        }
        AppliedSharedWorkerClientErrorTarget {
            owner_scope,
            wrapper,
            endpoint_disposition,
        }
    }

    fn forget_runtime_endpoint(&mut self, client_id: SharedWorkerClientId) -> bool {
        let Some(endpoint) = self.endpoints.remove(&client_id) else {
            return false;
        };
        endpoint.receiver.forget_after_runtime_terminal();
        true
    }

    fn disconnect_endpoint(&mut self, client_id: SharedWorkerClientId) -> bool {
        let Some(endpoint) = self.endpoints.remove(&client_id) else {
            return false;
        };
        endpoint.receiver.disconnect();
        true
    }

    pub(crate) fn disconnect_all_for_context_teardown(&mut self) {
        let client_ids = self.endpoints.keys().copied().collect::<Vec<_>>();
        for client_id in client_ids {
            self.disconnect_endpoint(client_id);
        }
    }

    pub(crate) fn disconnect_all_for_child_context(&mut self, handle: DomHandle) -> usize {
        let client_ids = self
            .endpoints
            .iter()
            .filter_map(|(client_id, endpoint)| {
                endpoint
                    .receiver
                    .frame_identity()
                    .is_child_context(handle)
                    .then_some(*client_id)
            })
            .collect::<Vec<_>>();
        let mut disconnected = 0;
        for client_id in client_ids {
            if self.disconnect_endpoint(client_id) {
                disconnected += 1;
            }
        }
        disconnected
    }

    pub(crate) fn disconnect_all_for_execution_context_owner(
        &mut self,
        owner: crate::native_bridge::WindowExecutionContextOwner,
    ) -> usize {
        let client_ids = self
            .endpoints
            .iter()
            .filter_map(|(client_id, endpoint)| {
                (endpoint
                    .receiver
                    .frame_identity()
                    .execution_context()
                    .owner()
                    == owner)
                    .then_some(*client_id)
            })
            .collect::<Vec<_>>();
        let disconnected = client_ids.len();
        for client_id in client_ids {
            self.disconnect_endpoint(client_id);
        }
        disconnected
    }

    pub(crate) fn disconnect_all_for_context_token(
        &mut self,
        context_token: crate::native_bridge::RuntimeObservableContextToken,
    ) -> usize {
        let client_ids = self
            .endpoints
            .iter()
            .filter_map(|(client_id, endpoint)| {
                (endpoint
                    .receiver
                    .frame_identity()
                    .execution_context()
                    .realm_token()
                    == context_token)
                    .then_some(*client_id)
            })
            .collect::<Vec<_>>();
        let disconnected = client_ids.len();
        for client_id in client_ids {
            self.disconnect_endpoint(client_id);
        }
        disconnected
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.endpoints.len()
    }

    #[cfg(test)]
    pub(crate) fn identities_for_test(
        &self,
    ) -> Vec<(
        SharedWorkerClientId,
        crate::native_bridge::WindowExecutionContextIdentity,
    )> {
        self.endpoints
            .iter()
            .map(|(client_id, endpoint)| {
                (
                    *client_id,
                    endpoint.receiver.frame_identity().execution_context(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use moli_shared_worker::{
        SharedWorkerClientId, SharedWorkerClientOwnerId, SharedWorkerConnectAction,
        SharedWorkerDescriptor,
    };

    use crate::{
        broadcast_channel_runtime::new_broadcast_channel_registry,
        document_runtime::DomHandle,
        message_port_runtime::new_message_port_registry,
        native_bridge::{
            OwnerDispatchScope, RuntimeObservableContextToken, WindowExecutionContextAccessPolicy,
            WindowExecutionContextIdentity, WindowExecutionContextOwner,
        },
        runtime::{RendererBrowserContextRuntime, RendererBrowserContextRuntimeOwner},
    };

    use super::{SharedWorkerClientEndpointReceiver, SharedWorkerClientFrameIdentity};
    use crate::shared_worker_runtime::{service::SharedWorkerRuntimeService, test_support};

    fn runtime_for_service(
        service: SharedWorkerRuntimeService,
    ) -> RendererBrowserContextRuntimeOwner {
        RendererBrowserContextRuntime::new_with_parts_for_test(
            new_message_port_registry(),
            new_broadcast_channel_registry(),
            service,
        )
    }

    fn start_loading_client(service: &SharedWorkerRuntimeService) -> SharedWorkerClientId {
        let action = test_support::connect_matching(
            service,
            test_support::shared_worker_key(),
            SharedWorkerDescriptor::default(),
        );
        match action {
            SharedWorkerConnectAction::StartLoading { client_id, .. } => client_id,
            _ => panic!("expected StartLoading"),
        }
    }

    fn frame_identity(
        owner_id: SharedWorkerClientOwnerId,
        dispatch_scope: OwnerDispatchScope,
        realm_token: u64,
    ) -> SharedWorkerClientFrameIdentity {
        SharedWorkerClientFrameIdentity::new(
            owner_id,
            WindowExecutionContextIdentity::new(
                WindowExecutionContextOwner::Frame(crate::frame_owner_model::LocalWindowId(
                    realm_token,
                )),
                dispatch_scope,
                RuntimeObservableContextToken::from_raw(realm_token),
                WindowExecutionContextAccessPolicy::EnforceWebOrigin,
            ),
        )
    }

    #[test]
    fn endpoint_receiver_drop_removes_runtime_client() {
        let service = test_support::runtime_service();
        let runtime = runtime_for_service(service.clone());
        let client_id = start_loading_client(&service);

        {
            let _receiver = SharedWorkerClientEndpointReceiver::new(
                client_id,
                frame_identity(
                    SharedWorkerClientOwnerId::from_u64(100),
                    OwnerDispatchScope::Top,
                    1,
                ),
                runtime.clone(),
            );
        }

        assert!(test_support::matching_is_empty(&service));
        drop(runtime);
    }

    #[test]
    fn endpoint_receiver_disarm_keeps_runtime_client_for_terminal_cleanup() {
        let service = test_support::runtime_service();
        let runtime = runtime_for_service(service.clone());
        let client_id = start_loading_client(&service);
        let receiver = SharedWorkerClientEndpointReceiver::new(
            client_id,
            frame_identity(
                SharedWorkerClientOwnerId::from_u64(100),
                OwnerDispatchScope::Top,
                1,
            ),
            runtime.clone(),
        );

        receiver.forget_after_runtime_terminal();

        assert!(!test_support::matching_is_empty(&service));
        drop(runtime);
    }

    #[test]
    fn frame_identity_tracks_child_context_handle_with_owner() {
        let owner_id = SharedWorkerClientOwnerId::from_u64(100);
        let handle = DomHandle::new(7);
        let identity = frame_identity(owner_id, OwnerDispatchScope::Child(handle), 7);

        assert_eq!(identity.owner_id(), owner_id);
        assert_eq!(identity.child_handle(), Some(handle));
        assert!(identity.is_child_context(handle));
        assert_ne!(
            identity,
            frame_identity(owner_id, OwnerDispatchScope::Top, 8)
        );
    }
}
