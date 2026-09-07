use std::{
    sync::Arc,
    task::{Context, Wake, Waker},
};

use super::RendererBrowserContextRuntime;
use crate::{
    service_worker_runtime::service_worker_owner_wake_channel,
    shared_worker_runtime::shared_worker_owner_wake_channel,
};

enum WorkerServices {
    Deferred,
    AlreadyLive,
    InitializedAfterRegistration,
}

struct ReceiverWake;

// A static Waker::noop() cannot witness whether the channel retains its owner.
#[allow(clippy::manual_noop_waker)]
impl Wake for ReceiverWake {
    fn wake(self: Arc<Self>) {}
}

fn assert_retired_receiver_releases_waker(services: WorkerServices) {
    let context = RendererBrowserContextRuntime::new();
    let initialize_workers = || {
        context.inner.shared_worker_runtime.get_or_init();
        context.inner.service_worker_runtime.get_or_init();
    };
    if matches!(services, WorkerServices::AlreadyLive) {
        initialize_workers();
    }

    let (shared_tx, mut shared_rx) = shared_worker_owner_wake_channel();
    let (service_tx, mut service_rx) = service_worker_owner_wake_channel();
    context.add_shared_worker_owner_wake_sender(shared_tx);
    context.add_service_worker_owner_wake_sender(service_tx);
    let (peer_shared_tx, mut peer_shared_rx) = shared_worker_owner_wake_channel();
    let (peer_service_tx, mut peer_service_rx) = service_worker_owner_wake_channel();
    context.add_shared_worker_owner_wake_sender(peer_shared_tx);
    context.add_service_worker_owner_wake_sender(peer_service_tx);

    if matches!(services, WorkerServices::InitializedAfterRegistration) {
        initialize_workers();
    }

    let shared_wake = Arc::new(ReceiverWake);
    let shared_weak = Arc::downgrade(&shared_wake);
    let service_wake = Arc::new(ReceiverWake);
    let service_weak = Arc::downgrade(&service_wake);
    {
        let shared_waker = Waker::from(shared_wake);
        let service_waker = Waker::from(service_wake);
        assert!(
            shared_rx
                .poll_recv(&mut Context::from_waker(&shared_waker))
                .is_pending()
        );
        assert!(
            service_rx
                .poll_recv(&mut Context::from_waker(&service_waker))
                .is_pending()
        );
    }
    drop(shared_rx);
    drop(service_rx);

    // The context (including its senders) stays live. Before Tokio 1.53,
    // receiver drop retained the last block_on waker and its I/O driver.
    // Assert before any new registration/wake can happen to prune old routes.
    assert!(
        shared_weak.upgrade().is_none(),
        "retired SharedWorker receiver retained its waker"
    );
    assert!(
        service_weak.upgrade().is_none(),
        "retired ServiceWorker receiver retained its waker"
    );
    assert!(matches!(
        peer_shared_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
    assert!(matches!(
        peer_service_rx.try_recv(),
        Err(tokio::sync::mpsc::error::TryRecvError::Empty)
    ));
}

#[test]
fn retired_deferred_worker_wake_receivers_release_wakers_while_context_lives() {
    assert_retired_receiver_releases_waker(WorkerServices::Deferred);
}

#[test]
fn retired_live_worker_wake_receivers_release_wakers_without_affecting_peer() {
    assert_retired_receiver_releases_waker(WorkerServices::AlreadyLive);
}

#[test]
fn retired_worker_wake_receivers_release_wakers_after_service_initialization() {
    assert_retired_receiver_releases_waker(WorkerServices::InitializedAfterRegistration);
}
