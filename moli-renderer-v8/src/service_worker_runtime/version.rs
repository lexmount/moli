use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};

use url::Url;

use crate::{
    runtime::RendererServiceWorkerRunIdentity,
    worker::{WorkerBootstrapFailure, WorkerScriptKind},
};

use super::{
    events::{
        ServiceWorkerFetchEvent, ServiceWorkerLifecycleEvent, ServiceWorkerMessageEvent,
        ServiceWorkerNotificationEvent, ServiceWorkerPeriodicSyncEvent, ServiceWorkerPushEvent,
        ServiceWorkerSyncEvent,
    },
    host::SharedRendererServiceWorkerHost,
    ids::{ServiceWorkerRegistrationId, ServiceWorkerVersionId},
    jobs::ServiceWorkerVersionLaunchConfig,
    script_loading::ServiceWorkerScriptResource,
};

#[derive(Clone, Debug)]
pub(super) struct ServiceWorkerIdleTimeout {
    pub(super) version_id: ServiceWorkerVersionId,
    pub(super) run: RendererServiceWorkerRunIdentity,
    pub(super) token: ServiceWorkerIdleTimeoutToken,
}

#[derive(Clone, Debug)]
pub(super) struct ServiceWorkerIdleTimeoutToken(Arc<()>);

impl ServiceWorkerIdleTimeoutToken {
    pub(super) fn fresh() -> Self {
        Self(Arc::new(()))
    }
}

impl PartialEq for ServiceWorkerIdleTimeoutToken {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for ServiceWorkerIdleTimeoutToken {}

#[derive(Debug)]
pub(super) struct ServiceWorkerVersion {
    pub(super) id: ServiceWorkerVersionId,
    pub(super) registration_id: ServiceWorkerRegistrationId,
    pub(super) script_url: Url,
    pub(super) final_script_url: Option<Url>,
    pub(super) main_script_resource: Option<ServiceWorkerScriptResource>,
    pub(super) imported_script_resources: BTreeMap<String, ServiceWorkerScriptResource>,
    pub(super) allow_identical_script_update: bool,
    pub(super) should_pause_on_start_for_devtools: bool,
    pub(super) script_kind: WorkerScriptKind,
    pub(super) fetch_handler_existence: ServiceWorkerFetchHandlerExistence,
    pub(super) fetch_handler_type: ServiceWorkerFetchHandlerType,
    pub(super) launch_config: ServiceWorkerVersionLaunchConfig,
    pub(super) lifecycle_state: ServiceWorkerVersionLifecycleState,
    pub(super) running_state: ServiceWorkerVersionRunningState,
    pub(super) pending_start_events: VecDeque<ServiceWorkerPendingStartEvent>,
    pub(super) pending_activation_fetch_events: VecDeque<ServiceWorkerFetchEvent>,
    pub(super) in_flight_event_count: usize,
    pub(super) run: RendererServiceWorkerRunIdentity,
    pub(super) idle_timeout_token: Option<ServiceWorkerIdleTimeoutToken>,
    pub(super) skip_waiting_requested: bool,
    pub(super) clients_claim_requested: bool,
    pub(super) last_start_error: Option<String>,
}

#[derive(Clone, Debug)]
pub(super) enum ServiceWorkerPendingStartEvent {
    Fetch(ServiceWorkerFetchEvent),
    Lifecycle(ServiceWorkerLifecycleEvent),
    Message(ServiceWorkerMessageEvent),
    Notification(ServiceWorkerNotificationEvent),
    Push(ServiceWorkerPushEvent),
    Sync(ServiceWorkerSyncEvent),
    PeriodicSync(ServiceWorkerPeriodicSyncEvent),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ServiceWorkerVersionLifecycleState {
    Installing,
    Installed,
    Activating,
    Activated,
    Redundant,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ServiceWorkerFetchHandlerExistence {
    Unknown,
    Exists,
    DoesNotExist,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ServiceWorkerFetchHandlerType {
    NoHandler,
    NotSkippable,
    EmptyFetchHandler,
}

impl ServiceWorkerFetchHandlerType {
    pub(super) fn allows_fetch_event_skip(self) -> bool {
        matches!(
            self,
            ServiceWorkerFetchHandlerType::NoHandler
                | ServiceWorkerFetchHandlerType::EmptyFetchHandler
        )
    }
}

pub(super) enum ServiceWorkerVersionRunningState {
    Stopped,
    Starting {
        host: SharedRendererServiceWorkerHost,
    },
    Running {
        host: SharedRendererServiceWorkerHost,
    },
}

pub(super) enum ServiceWorkerVersionStartFailure {
    HostThreadSpawn { message: String },
    ScriptLoad { message: String },
    Bootstrap { failure: WorkerBootstrapFailure },
    BootstrapChannelClosed,
}

impl std::fmt::Debug for ServiceWorkerVersionRunningState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Stopped => "Stopped",
            Self::Starting { .. } => "Starting",
            Self::Running { .. } => "Running",
        })
    }
}

impl ServiceWorkerVersionRunningState {
    pub(super) fn diagnostics(&self) -> (&'static str, bool) {
        match self {
            Self::Stopped => ("stopped", false),
            Self::Starting { host } => ("starting", host.has_running_worker()),
            Self::Running { host } => ("running", host.has_running_worker()),
        }
    }

    pub(super) fn take_host_for_shutdown(&mut self) -> Option<SharedRendererServiceWorkerHost> {
        match std::mem::replace(self, Self::Stopped) {
            Self::Stopped => None,
            Self::Starting { host } => Some(host),
            Self::Running { host } => Some(host),
        }
    }

    pub(super) fn into_host(self) -> Option<SharedRendererServiceWorkerHost> {
        match self {
            Self::Stopped => None,
            Self::Starting { host } | Self::Running { host } => Some(host),
        }
    }
}

impl ServiceWorkerVersionStartFailure {
    pub(super) fn to_diagnostic_message(&self) -> String {
        match self {
            Self::HostThreadSpawn { message } => {
                format!("failed to spawn service worker host thread: {message}")
            }
            Self::ScriptLoad { message } => message.clone(),
            Self::Bootstrap { failure } => {
                format!(
                    "service worker bootstrap failed: {} at {}:{}:{} event={:?} phase={:?} source={:?}",
                    failure.message,
                    failure.filename,
                    failure.lineno,
                    failure.colno,
                    failure.event_kind,
                    failure.phase,
                    failure.source
                )
            }
            Self::BootstrapChannelClosed => {
                "service worker bootstrap completion channel closed".to_owned()
            }
        }
    }
}

impl ServiceWorkerVersionLifecycleState {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Installing => "installing",
            Self::Installed => "installed",
            Self::Activating => "activating",
            Self::Activated => "activated",
            Self::Redundant => "redundant",
        }
    }
}
