use url::Url;

/// BrowserContext-scoped ServiceWorker control with no frontend attribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceWorkerCommand {
    SetForceUpdateOnPageLoad(bool),
    Unregister {
        scope: Url,
    },
    Start {
        scope: Url,
    },
    StopVersion {
        version_id: u64,
    },
    StopAll,
    SkipWaiting {
        scope: Url,
    },
    UpdateRegistration {
        scope: Url,
    },
    DeliverPushMessage {
        origin: Url,
        registration_id: u64,
        data: Option<Vec<u8>>,
    },
    DispatchSyncEvent {
        origin: Url,
        registration_id: u64,
        tag: String,
        last_chance: bool,
    },
    DispatchPeriodicSyncEvent {
        origin: Url,
        registration_id: u64,
        tag: String,
    },
}
