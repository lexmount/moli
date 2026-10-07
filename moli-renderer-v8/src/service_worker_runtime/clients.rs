use std::sync::atomic::{AtomicU64, Ordering};

use url::Url;

use super::ids::{ServiceWorkerClientId, ServiceWorkerRegistrationId, ServiceWorkerVersionId};

static NEXT_SERVICE_WORKER_EXPOSED_CLIENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerClientSnapshot {
    pub(crate) id: ServiceWorkerClientId,
    pub(crate) exposed_id: String,
    pub(crate) url: Url,
    pub(crate) client_type: ServiceWorkerClientType,
    pub(crate) frame_type: ServiceWorkerClientFrameType,
    pub(crate) visibility_state: ServiceWorkerClientVisibilityState,
    pub(crate) controlled: bool,
    pub(crate) focused: bool,
}

impl ServiceWorkerClientSnapshot {
    #[cfg(test)]
    pub(crate) fn window_for_test(id: ServiceWorkerClientId, url: Url, controlled: bool) -> Self {
        Self {
            id,
            exposed_id: service_worker_exposed_client_id(id),
            url,
            client_type: ServiceWorkerClientType::Window,
            frame_type: ServiceWorkerClientFrameType::TopLevel,
            visibility_state: ServiceWorkerClientVisibilityState::Visible,
            controlled,
            focused: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn focused_window_for_test(
        id: ServiceWorkerClientId,
        url: Url,
        controlled: bool,
    ) -> Self {
        Self {
            id,
            exposed_id: service_worker_exposed_client_id(id),
            url,
            client_type: ServiceWorkerClientType::Window,
            frame_type: ServiceWorkerClientFrameType::TopLevel,
            visibility_state: ServiceWorkerClientVisibilityState::Visible,
            controlled,
            focused: true,
        }
    }

    #[cfg(test)]
    pub(crate) fn dedicated_worker_for_test(
        id: ServiceWorkerClientId,
        url: Url,
        controlled: bool,
    ) -> Self {
        Self {
            id,
            exposed_id: service_worker_exposed_client_id(id),
            url,
            client_type: ServiceWorkerClientType::DedicatedWorker,
            frame_type: ServiceWorkerClientFrameType::None,
            visibility_state: ServiceWorkerClientVisibilityState::Hidden,
            controlled,
            focused: false,
        }
    }
}

pub(crate) fn service_worker_exposed_client_id(id: ServiceWorkerClientId) -> String {
    format!("client-{:016x}", id.as_u64())
}

pub(crate) fn allocate_service_worker_exposed_client_id() -> String {
    allocate_service_worker_exposed_client_id_from(&NEXT_SERVICE_WORKER_EXPOSED_CLIENT_ID)
}

fn allocate_service_worker_exposed_client_id_from(counter: &AtomicU64) -> String {
    let raw = counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .expect("ServiceWorker exposed client id allocator exhausted");
    format!("client-document-{raw:016x}")
}

pub(crate) fn service_worker_current_url_for_creation_url(creation_url: &Url) -> Url {
    let mut current_url = creation_url.clone();
    current_url.set_fragment(None);
    current_url
}

#[cfg(test)]
mod exposed_client_identity_tests {
    use super::*;

    #[test]
    fn replacement_ids_are_fresh_and_not_derived_from_internal_client_id() {
        let client_id = ServiceWorkerClientId::from_u64_for_test(7);
        let first = allocate_service_worker_exposed_client_id();
        let second = allocate_service_worker_exposed_client_id();

        assert_ne!(first, second);
        assert_ne!(first, service_worker_exposed_client_id(client_id));
    }

    #[test]
    fn replacement_id_allocator_rejects_exhaustion_without_wrapping() {
        let counter = AtomicU64::new(u64::MAX);
        let exhausted =
            std::panic::catch_unwind(|| allocate_service_worker_exposed_client_id_from(&counter));

        assert!(exhausted.is_err());
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientType {
    Window,
    DedicatedWorker,
    SharedWorker,
}

impl ServiceWorkerClientType {
    pub(crate) fn as_webidl_str(self) -> &'static str {
        match self {
            Self::Window => "window",
            Self::DedicatedWorker => "worker",
            Self::SharedWorker => "sharedworker",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientFrameType {
    Nested,
    None,
    TopLevel,
}

impl ServiceWorkerClientFrameType {
    pub(crate) fn as_webidl_str(self) -> &'static str {
        match self {
            Self::Nested => "nested",
            Self::None => "none",
            Self::TopLevel => "top-level",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientVisibilityState {
    Hidden,
    Visible,
}

impl ServiceWorkerClientVisibilityState {
    pub(crate) fn as_webidl_str(self) -> &'static str {
        match self {
            Self::Hidden => "hidden",
            Self::Visible => "visible",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServiceWorkerClientQueryType {
    All,
    Window,
    Worker,
    SharedWorker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ServiceWorkerClientQueryOptions {
    pub(crate) include_uncontrolled: bool,
    pub(crate) client_type: ServiceWorkerClientQueryType,
}

#[derive(Clone, Debug)]
pub(crate) enum ServiceWorkerClientQueryKind {
    Get {
        exposed_client_id: String,
    },
    MatchAll {
        options: ServiceWorkerClientQueryOptions,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerClientQuery {
    pub(crate) request_id: u64,
    pub(crate) registration_id: ServiceWorkerRegistrationId,
    pub(crate) version_id: ServiceWorkerVersionId,
    pub(crate) kind: ServiceWorkerClientQueryKind,
}

#[derive(Clone, Debug)]
pub(crate) struct ServiceWorkerClientQueryResult {
    pub(crate) request_id: u64,
    pub(crate) clients: Vec<ServiceWorkerClientSnapshot>,
}
