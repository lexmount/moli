use std::{
    collections::BTreeMap,
    fmt, fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result, anyhow, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::worker::{WorkerScriptKind, WorkerScriptResourceKind};

use super::{
    jobs::ServiceWorkerRegistrationKey,
    registration::{ServiceWorkerNavigationPreloadState, ServiceWorkerUpdateViaCache},
    script_loading::ServiceWorkerScriptResource,
    version::{
        ServiceWorkerFetchHandlerExistence, ServiceWorkerFetchHandlerType,
        ServiceWorkerVersionLifecycleState,
    },
};

const SERVICE_WORKER_RESOURCE_STORE_JSON_VERSION: u32 = 2;
const SERVICE_WORKER_RESOURCE_STORE_MUTATION_RETRY_COUNT: usize = 1;

pub struct ServiceWorkerResourceStore {
    backend: ServiceWorkerResourceStoreBackend,
    revision: u64,
}

pub type SharedServiceWorkerResourceStore = Arc<Mutex<ServiceWorkerResourceStore>>;

enum ServiceWorkerResourceStoreBackend {
    Memory(BTreeMap<String, ServiceWorkerStoredRegistration>),
    Json(JsonServiceWorkerResourceStoreBackend),
}

#[derive(Debug, Clone)]
struct JsonServiceWorkerResourceStoreBackend {
    path: PathBuf,
    registrations: BTreeMap<String, ServiceWorkerStoredRegistration>,
    #[cfg(test)]
    fail_next_persist_attempt_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ServiceWorkerStoredRegistration {
    pub(super) storage_key: String,
    pub(super) scope_url: Url,
    pub(super) script_url: Url,
    pub(super) script_kind: WorkerScriptKind,
    pub(super) update_via_cache: ServiceWorkerUpdateViaCache,
    pub(super) navigation_preload_state: ServiceWorkerNavigationPreloadState,
    pub(super) lifecycle_state: ServiceWorkerVersionLifecycleState,
    pub(super) fetch_handler_existence: ServiceWorkerFetchHandlerExistence,
    pub(super) fetch_handler_type: ServiceWorkerFetchHandlerType,
    pub(super) last_update_check_time_ms: Option<u64>,
    pub(super) main_script_resource: ServiceWorkerScriptResource,
    pub(super) imported_script_resources: BTreeMap<String, ServiceWorkerScriptResource>,
}

pub fn new_shared_service_worker_resource_store() -> SharedServiceWorkerResourceStore {
    Arc::new(Mutex::new(ServiceWorkerResourceStore::default()))
}

pub fn new_shared_json_service_worker_resource_store(
    path: impl AsRef<Path>,
) -> Result<SharedServiceWorkerResourceStore> {
    let backend = JsonServiceWorkerResourceStoreBackend::open(path.as_ref())?;
    Ok(Arc::new(Mutex::new(ServiceWorkerResourceStore {
        backend: ServiceWorkerResourceStoreBackend::Json(backend),
        revision: 0,
    })))
}

impl Default for ServiceWorkerResourceStore {
    fn default() -> Self {
        Self {
            backend: ServiceWorkerResourceStoreBackend::Memory(BTreeMap::new()),
            revision: 0,
        }
    }
}

impl fmt::Debug for ServiceWorkerResourceStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.backend {
            ServiceWorkerResourceStoreBackend::Memory(registrations) => f
                .debug_struct("ServiceWorkerResourceStore")
                .field("backend", &"memory")
                .field("registrations", &registrations.keys().collect::<Vec<_>>())
                .finish(),
            ServiceWorkerResourceStoreBackend::Json(json) => f
                .debug_struct("ServiceWorkerResourceStore")
                .field("backend", &"json")
                .field("path", &json.path)
                .field(
                    "registrations",
                    &json.registrations.keys().collect::<Vec<_>>(),
                )
                .finish(),
        }
    }
}

impl ServiceWorkerResourceStore {
    pub(super) fn store_registration(
        &mut self,
        registration: ServiceWorkerStoredRegistration,
    ) -> Result<()> {
        let mut last_error = None;
        for _ in 0..=SERVICE_WORKER_RESOURCE_STORE_MUTATION_RETRY_COUNT {
            match self.store_registration_once(registration.clone()) {
                Ok(()) => return Ok(()),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.expect("store registration should attempt at least once"))
    }

    fn store_registration_once(
        &mut self,
        registration: ServiceWorkerStoredRegistration,
    ) -> Result<()> {
        match &mut self.backend {
            ServiceWorkerResourceStoreBackend::Memory(registrations) => {
                registrations.insert(
                    resource_store_key_for_registration(&registration),
                    registration,
                );
            }
            ServiceWorkerResourceStoreBackend::Json(json) => {
                let mut next_registrations = json.registrations.clone();
                next_registrations.insert(
                    resource_store_key_for_registration(&registration),
                    registration,
                );
                json.persist_registrations(&next_registrations)?;
                json.registrations = next_registrations;
            }
        }
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn registration_for_key(
        &self,
        key: &ServiceWorkerRegistrationKey,
    ) -> Option<ServiceWorkerStoredRegistration> {
        match &self.backend {
            ServiceWorkerResourceStoreBackend::Memory(registrations) => {
                registrations.get(&resource_store_key(key)).cloned()
            }
            ServiceWorkerResourceStoreBackend::Json(json) => {
                json.registrations.get(&resource_store_key(key)).cloned()
            }
        }
    }

    pub(super) fn registrations(&self) -> Vec<ServiceWorkerStoredRegistration> {
        match &self.backend {
            ServiceWorkerResourceStoreBackend::Memory(registrations) => registrations,
            ServiceWorkerResourceStoreBackend::Json(json) => &json.registrations,
        }
        .values()
        .cloned()
        .collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        match &self.backend {
            ServiceWorkerResourceStoreBackend::Memory(registrations) => registrations.is_empty(),
            ServiceWorkerResourceStoreBackend::Json(json) => json.registrations.is_empty(),
        }
    }

    pub(super) fn delete_registration(&mut self, key: &ServiceWorkerRegistrationKey) -> Result<()> {
        let mut last_error = None;
        for _ in 0..=SERVICE_WORKER_RESOURCE_STORE_MUTATION_RETRY_COUNT {
            match self.delete_registration_once(key) {
                Ok(()) => return Ok(()),
                Err(error) => last_error = Some(error),
            }
        }
        Err(last_error.expect("delete registration should attempt at least once"))
    }

    fn delete_registration_once(&mut self, key: &ServiceWorkerRegistrationKey) -> Result<()> {
        match &mut self.backend {
            ServiceWorkerResourceStoreBackend::Memory(registrations) => {
                registrations.remove(&resource_store_key(key));
            }
            ServiceWorkerResourceStoreBackend::Json(json) => {
                let mut next_registrations = json.registrations.clone();
                next_registrations.remove(&resource_store_key(key));
                json.persist_registrations(&next_registrations)?;
                json.registrations = next_registrations;
            }
        }
        self.revision = self.revision.wrapping_add(1);
        Ok(())
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision
    }

    #[cfg(test)]
    pub(super) fn registration_count(&self) -> usize {
        match &self.backend {
            ServiceWorkerResourceStoreBackend::Memory(registrations) => registrations.len(),
            ServiceWorkerResourceStoreBackend::Json(json) => json.registrations.len(),
        }
    }

    #[cfg(test)]
    pub(super) fn fail_next_persist_attempts_for_test(&mut self, count: usize) {
        match &mut self.backend {
            ServiceWorkerResourceStoreBackend::Memory(_) => {
                panic!("memory service worker resource store cannot fail persist attempts")
            }
            ServiceWorkerResourceStoreBackend::Json(json) => {
                json.fail_next_persist_attempt_count =
                    json.fail_next_persist_attempt_count.saturating_add(count);
            }
        }
    }
}

impl JsonServiceWorkerResourceStoreBackend {
    fn open(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self {
                path: path.to_path_buf(),
                registrations: BTreeMap::new(),
                #[cfg(test)]
                fail_next_persist_attempt_count: 0,
            });
        }

        let bytes = fs::read(path).with_context(|| {
            format!(
                "failed to read Service Worker resource store `{}`",
                path.display()
            )
        })?;
        let file: JsonServiceWorkerResourceStoreFile = serde_json::from_slice(&bytes)
            .with_context(|| {
                format!(
                    "failed to parse Service Worker resource store `{}`",
                    path.display()
                )
            })?;
        if !matches!(file.version, 1 | SERVICE_WORKER_RESOURCE_STORE_JSON_VERSION) {
            bail!(
                "unsupported Service Worker resource store version {} in `{}`; this Moli supports version {}",
                file.version,
                path.display(),
                SERVICE_WORKER_RESOURCE_STORE_JSON_VERSION
            );
        }
        let registrations = file
            .registrations
            .into_values()
            .map(|registration| {
                let registration = registration.into_stored_registration()?;
                Ok((
                    resource_store_key_for_registration(&registration),
                    registration,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        Ok(Self {
            path: path.to_path_buf(),
            registrations,
            #[cfg(test)]
            fail_next_persist_attempt_count: 0,
        })
    }

    fn persist_registrations(
        &mut self,
        registrations: &BTreeMap<String, ServiceWorkerStoredRegistration>,
    ) -> Result<()> {
        #[cfg(test)]
        if self.fail_next_persist_attempt_count > 0 {
            self.fail_next_persist_attempt_count -= 1;
            bail!("transient Service Worker resource store persist failure");
        }
        persist_json_service_worker_resource_store(&self.path, registrations)
    }
}

fn resource_store_key(key: &ServiceWorkerRegistrationKey) -> String {
    format!("{} {}", key.storage_key, key.scope_url.as_str())
}

fn resource_store_key_for_registration(registration: &ServiceWorkerStoredRegistration) -> String {
    resource_store_key(&ServiceWorkerRegistrationKey {
        scope_url: registration.scope_url.clone(),
        storage_key: registration.storage_key.clone(),
    })
}

fn persist_json_service_worker_resource_store(
    path: &Path,
    registrations: &BTreeMap<String, ServiceWorkerStoredRegistration>,
) -> Result<()> {
    let file = JsonServiceWorkerResourceStoreFile {
        version: SERVICE_WORKER_RESOURCE_STORE_JSON_VERSION,
        registrations: registrations
            .iter()
            .map(|(key, registration)| (key.clone(), JsonStoredRegistration::from(registration)))
            .collect(),
    };
    let bytes = serde_json::to_vec_pretty(&file)
        .context("failed to serialize Service Worker resource store")?;
    moli_browser_profile::write_file_atomically(path, &bytes, "Service Worker resource store")
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonServiceWorkerResourceStoreFile {
    version: u32,
    registrations: BTreeMap<String, JsonStoredRegistration>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonStoredRegistration {
    storage_key: Option<String>,
    scope_url: String,
    script_url: String,
    script_kind: String,
    update_via_cache: String,
    lifecycle_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fetch_handler_existence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fetch_handler_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_update_check_time_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    navigation_preload_state: Option<JsonNavigationPreloadState>,
    main_script_resource: JsonScriptResource,
    imported_script_resources: BTreeMap<String, JsonScriptResource>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonNavigationPreloadState {
    enabled: bool,
    header_value: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsonScriptResource {
    request_url: String,
    final_url: String,
    kind: String,
    status: u16,
    headers: Vec<(String, String)>,
    body_len: usize,
    body_sha256: String,
    response_time_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    mime_type: Option<String>,
}

impl JsonStoredRegistration {
    fn into_stored_registration(self) -> Result<ServiceWorkerStoredRegistration> {
        let scope_url = parse_url_field("scopeUrl", &self.scope_url)?;
        let storage_key = self
            .storage_key
            .map(|storage_key| {
                normalize_json_stored_registration_storage_key(&scope_url, storage_key)
            })
            .unwrap_or_else(|| ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url));
        Ok(ServiceWorkerStoredRegistration {
            storage_key,
            scope_url,
            script_url: parse_url_field("scriptUrl", &self.script_url)?,
            script_kind: worker_script_kind_from_store_str(&self.script_kind)?,
            update_via_cache: ServiceWorkerUpdateViaCache::parse_webidl_token(
                &self.update_via_cache,
            )
            .ok_or_else(|| {
                anyhow!(
                    "unsupported Service Worker updateViaCache `{}` in resource store",
                    self.update_via_cache
                )
            })?,
            lifecycle_state: lifecycle_state_from_store_str(&self.lifecycle_state)?,
            fetch_handler_existence: fetch_handler_existence_from_optional_store_str(
                self.fetch_handler_existence.as_deref(),
            )?,
            fetch_handler_type: fetch_handler_type_from_optional_store_str(
                self.fetch_handler_type.as_deref(),
                self.fetch_handler_existence.as_deref(),
            )?,
            last_update_check_time_ms: self.last_update_check_time_ms,
            navigation_preload_state: self
                .navigation_preload_state
                .map(ServiceWorkerNavigationPreloadState::from)
                .unwrap_or_default(),
            main_script_resource: self.main_script_resource.into_script_resource()?,
            imported_script_resources: self
                .imported_script_resources
                .into_iter()
                .map(|(key, resource)| Ok((key, resource.into_script_resource()?)))
                .collect::<Result<BTreeMap<_, _>>>()?,
        })
    }
}

fn normalize_json_stored_registration_storage_key(scope_url: &Url, storage_key: String) -> String {
    if storage_key == moli_url::origin_ascii_serialization(scope_url) {
        return ServiceWorkerRegistrationKey::storage_key_for_scope_url(scope_url);
    }
    storage_key
}

impl From<&ServiceWorkerStoredRegistration> for JsonStoredRegistration {
    fn from(registration: &ServiceWorkerStoredRegistration) -> Self {
        Self {
            storage_key: Some(registration.storage_key.clone()),
            scope_url: registration.scope_url.as_str().to_owned(),
            script_url: registration.script_url.as_str().to_owned(),
            script_kind: worker_script_kind_to_store_str(registration.script_kind).to_owned(),
            update_via_cache: registration.update_via_cache.as_str().to_owned(),
            lifecycle_state: registration.lifecycle_state.as_str().to_owned(),
            fetch_handler_existence: Some(
                fetch_handler_existence_to_store_str(registration.fetch_handler_existence)
                    .to_owned(),
            ),
            fetch_handler_type: Some(
                fetch_handler_type_to_store_str(registration.fetch_handler_type).to_owned(),
            ),
            last_update_check_time_ms: registration.last_update_check_time_ms,
            navigation_preload_state: Some(JsonNavigationPreloadState::from(
                &registration.navigation_preload_state,
            )),
            main_script_resource: JsonScriptResource::from(&registration.main_script_resource),
            imported_script_resources: registration
                .imported_script_resources
                .iter()
                .map(|(key, resource)| (key.clone(), JsonScriptResource::from(resource)))
                .collect(),
        }
    }
}

impl From<JsonNavigationPreloadState> for ServiceWorkerNavigationPreloadState {
    fn from(value: JsonNavigationPreloadState) -> Self {
        Self {
            enabled: value.enabled,
            header_value: value.header_value,
        }
    }
}

impl From<&ServiceWorkerNavigationPreloadState> for JsonNavigationPreloadState {
    fn from(value: &ServiceWorkerNavigationPreloadState) -> Self {
        Self {
            enabled: value.enabled,
            header_value: value.header_value.clone(),
        }
    }
}

impl JsonScriptResource {
    fn into_script_resource(self) -> Result<ServiceWorkerScriptResource> {
        Ok(ServiceWorkerScriptResource {
            request_url: parse_url_field("requestUrl", &self.request_url)?,
            final_url: parse_url_field("finalUrl", &self.final_url)?,
            kind: resource_kind_from_store_str(&self.kind)?,
            status: self.status,
            headers: self.headers,
            body_len: self.body_len,
            body_sha256: self.body_sha256,
            response_time_ms: self.response_time_ms,
            mime_type: self.mime_type,
        })
    }
}

impl From<&ServiceWorkerScriptResource> for JsonScriptResource {
    fn from(resource: &ServiceWorkerScriptResource) -> Self {
        Self {
            request_url: resource.request_url.as_str().to_owned(),
            final_url: resource.final_url.as_str().to_owned(),
            kind: resource.kind.as_str().to_owned(),
            status: resource.status,
            headers: resource.headers.clone(),
            body_len: resource.body_len,
            body_sha256: resource.body_sha256.clone(),
            response_time_ms: resource.response_time_ms,
            mime_type: resource.mime_type.clone(),
        }
    }
}

fn parse_url_field(field: &str, value: &str) -> Result<Url> {
    Url::parse(value).with_context(|| {
        format!("failed to parse Service Worker resource store URL field `{field}`: `{value}`")
    })
}

fn worker_script_kind_to_store_str(kind: WorkerScriptKind) -> &'static str {
    match kind {
        WorkerScriptKind::Classic => "classic",
        WorkerScriptKind::Module => "module",
    }
}

fn worker_script_kind_from_store_str(value: &str) -> Result<WorkerScriptKind> {
    match value {
        "classic" => Ok(WorkerScriptKind::Classic),
        "module" => Ok(WorkerScriptKind::Module),
        _ => bail!("unsupported Service Worker script kind `{value}` in resource store"),
    }
}

fn lifecycle_state_from_store_str(value: &str) -> Result<ServiceWorkerVersionLifecycleState> {
    match value {
        "installed" => Ok(ServiceWorkerVersionLifecycleState::Installed),
        "activated" => Ok(ServiceWorkerVersionLifecycleState::Activated),
        _ => bail!("unsupported Service Worker lifecycle state `{value}` in resource store"),
    }
}

fn fetch_handler_existence_to_store_str(
    existence: ServiceWorkerFetchHandlerExistence,
) -> &'static str {
    match existence {
        ServiceWorkerFetchHandlerExistence::Unknown => "unknown",
        ServiceWorkerFetchHandlerExistence::Exists => "exists",
        ServiceWorkerFetchHandlerExistence::DoesNotExist => "does-not-exist",
    }
}

fn fetch_handler_existence_from_optional_store_str(
    value: Option<&str>,
) -> Result<ServiceWorkerFetchHandlerExistence> {
    match value {
        None | Some("unknown") => Ok(ServiceWorkerFetchHandlerExistence::Unknown),
        Some("exists") => Ok(ServiceWorkerFetchHandlerExistence::Exists),
        Some("does-not-exist") => Ok(ServiceWorkerFetchHandlerExistence::DoesNotExist),
        Some(value) => {
            bail!("unsupported Service Worker fetch handler existence `{value}` in resource store")
        }
    }
}

fn fetch_handler_type_to_store_str(
    fetch_handler_type: ServiceWorkerFetchHandlerType,
) -> &'static str {
    match fetch_handler_type {
        ServiceWorkerFetchHandlerType::NoHandler => "no-handler",
        ServiceWorkerFetchHandlerType::NotSkippable => "not-skippable",
        ServiceWorkerFetchHandlerType::EmptyFetchHandler => "empty-fetch-handler",
    }
}

fn fetch_handler_type_from_optional_store_str(
    value: Option<&str>,
    existence_value: Option<&str>,
) -> Result<ServiceWorkerFetchHandlerType> {
    match value {
        Some("no-handler") => Ok(ServiceWorkerFetchHandlerType::NoHandler),
        Some("not-skippable") => Ok(ServiceWorkerFetchHandlerType::NotSkippable),
        Some("empty-fetch-handler") => Ok(ServiceWorkerFetchHandlerType::EmptyFetchHandler),
        Some(value) => {
            bail!("unsupported Service Worker fetch handler type `{value}` in resource store")
        }
        None => match fetch_handler_existence_from_optional_store_str(existence_value)? {
            ServiceWorkerFetchHandlerExistence::Exists => {
                Ok(ServiceWorkerFetchHandlerType::NotSkippable)
            }
            ServiceWorkerFetchHandlerExistence::Unknown
            | ServiceWorkerFetchHandlerExistence::DoesNotExist => {
                Ok(ServiceWorkerFetchHandlerType::NoHandler)
            }
        },
    }
}

fn resource_kind_from_store_str(value: &str) -> Result<WorkerScriptResourceKind> {
    match value {
        "javascript" => Ok(WorkerScriptResourceKind::JavaScript),
        "css-module" => Ok(WorkerScriptResourceKind::CssModule),
        "json-module" => Ok(WorkerScriptResourceKind::JsonModule),
        "text-module" => Ok(WorkerScriptResourceKind::TextModule),
        "webassembly-module" => Ok(WorkerScriptResourceKind::WebAssemblyModule),
        _ => bail!("unsupported Service Worker script resource kind `{value}` in resource store"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use url::Url;

    use super::{
        ServiceWorkerResourceStore, ServiceWorkerScriptResource, ServiceWorkerStoredRegistration,
        ServiceWorkerUpdateViaCache, ServiceWorkerVersionLifecycleState, WorkerScriptKind,
        WorkerScriptResourceKind, new_shared_json_service_worker_resource_store,
    };
    use crate::service_worker_runtime::{
        ServiceWorkerNavigationPreloadState,
        jobs::ServiceWorkerRegistrationKey,
        version::{ServiceWorkerFetchHandlerExistence, ServiceWorkerFetchHandlerType},
    };

    struct TempStorePath {
        path: PathBuf,
    }

    impl TempStorePath {
        fn new(name: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "moli-service-worker-resource-store-{name}-{}-{nonce}.json",
                std::process::id()
            ));
            Self { path }
        }
    }

    impl Drop for TempStorePath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn json_service_worker_resource_store_persists_and_reopens_registration() {
        let temp = TempStorePath::new("persist");
        let registration = stored_registration();
        let registration_key = ServiceWorkerRegistrationKey {
            scope_url: registration.scope_url.clone(),
            storage_key: registration.storage_key.clone(),
        };

        {
            let store = new_shared_json_service_worker_resource_store(&temp.path)
                .expect("json service worker resource store should open");
            store
                .lock()
                .store_registration(registration.clone())
                .expect("stored registration should persist");
        }

        let json = fs::read_to_string(&temp.path).expect("resource store should be written");
        assert!(json.contains(r#""version": 2"#));
        assert!(json.contains(
            r#""storageKey": "storage-key:v1;origin=https://service-worker-resources.test;top-level-site=https://service-worker-resources.test""#
        ));
        assert!(json.contains(r#""https://service-worker-resources.test/app/worker.js""#));
        assert!(json.contains(r#""css-module""#));
        assert!(json.contains(r#""fetchHandlerExistence": "does-not-exist""#));
        assert!(json.contains(r#""fetchHandlerType": "no-handler""#));
        assert!(json.contains(r#""navigationPreloadState""#));
        assert!(json.contains(r#""enabled": true"#));
        assert!(json.contains(r#""headerValue": "custom-preload""#));

        let reopened = new_shared_json_service_worker_resource_store(&temp.path)
            .expect("json service worker resource store should reopen");
        assert_eq!(
            reopened.lock().registration_for_key(&registration_key),
            Some(registration)
        );
    }

    #[test]
    fn json_service_worker_resource_store_round_trips_empty_fetch_handler_type() {
        let temp = TempStorePath::new("empty-fetch-handler");
        let mut registration = stored_registration();
        registration.fetch_handler_existence = ServiceWorkerFetchHandlerExistence::Exists;
        registration.fetch_handler_type = ServiceWorkerFetchHandlerType::EmptyFetchHandler;
        let registration_key = registration_key_for(&registration);

        {
            let store = new_shared_json_service_worker_resource_store(&temp.path)
                .expect("json service worker resource store should open");
            store
                .lock()
                .store_registration(registration.clone())
                .expect("stored registration should persist");
        }

        let json = fs::read_to_string(&temp.path).expect("resource store should be written");
        assert!(json.contains(r#""fetchHandlerExistence": "exists""#));
        assert!(json.contains(r#""fetchHandlerType": "empty-fetch-handler""#));

        let reopened = new_shared_json_service_worker_resource_store(&temp.path)
            .expect("json service worker resource store should reopen");
        assert_eq!(
            reopened.lock().registration_for_key(&registration_key),
            Some(registration)
        );
    }

    #[test]
    fn json_service_worker_resource_store_persists_registration_deletion() {
        let temp = TempStorePath::new("delete");
        let registration = stored_registration();
        let registration_key = registration_key_for(&registration);

        {
            let store = new_shared_json_service_worker_resource_store(&temp.path)
                .expect("json service worker resource store should open");
            store
                .lock()
                .store_registration(registration)
                .expect("stored registration should persist");
            store
                .lock()
                .delete_registration(&registration_key)
                .expect("registration deletion should persist");
        }

        let reopened = new_shared_json_service_worker_resource_store(&temp.path)
            .expect("json service worker resource store should reopen");
        assert_eq!(reopened.lock().registration_count(), 0);
    }

    #[test]
    fn resource_store_revision_advances_after_successful_mutations() {
        let mut store = ServiceWorkerResourceStore::default();
        let registration = stored_registration();
        let registration_key = registration_key_for(&registration);

        assert_eq!(store.revision(), 0);
        store
            .store_registration(registration)
            .expect("stored registration should persist");
        assert_eq!(store.revision(), 1);
        store
            .delete_registration(&registration_key)
            .expect("stored registration should delete");
        assert_eq!(store.revision(), 2);
    }

    #[test]
    fn json_service_worker_resource_store_retries_transient_persist_failure() {
        let temp = TempStorePath::new("transient-retry");
        let registration = stored_registration();
        let registration_key = registration_key_for(&registration);
        let store = new_shared_json_service_worker_resource_store(&temp.path)
            .expect("json service worker resource store should open");
        {
            let mut store = store.lock();
            store.fail_next_persist_attempts_for_test(1);
            store
                .store_registration(registration.clone())
                .expect("transient store failure should retry");
            assert_eq!(store.revision(), 1);
            assert_eq!(
                store.registration_for_key(&registration_key),
                Some(registration)
            );

            store.fail_next_persist_attempts_for_test(1);
            store
                .delete_registration(&registration_key)
                .expect("transient delete failure should retry");
            assert_eq!(store.revision(), 2);
            assert_eq!(store.registration_count(), 0);
        }

        let reopened = new_shared_json_service_worker_resource_store(&temp.path)
            .expect("json service worker resource store should reopen");
        assert_eq!(reopened.lock().registration_count(), 0);
    }

    #[test]
    fn json_service_worker_resource_store_keys_same_scope_by_storage_key() {
        let temp = TempStorePath::new("storage-key");
        let first = stored_registration();
        let mut second = stored_registration();
        second.storage_key = "https://embedder.test".to_owned();
        second.last_update_check_time_ms = Some(84);
        let first_key = registration_key_for(&first);
        let second_key = registration_key_for(&second);

        {
            let store = new_shared_json_service_worker_resource_store(&temp.path)
                .expect("json service worker resource store should open");
            store
                .lock()
                .store_registration(first.clone())
                .expect("first stored registration should persist");
            store
                .lock()
                .store_registration(second.clone())
                .expect("second stored registration should persist");
        }

        let reopened = new_shared_json_service_worker_resource_store(&temp.path)
            .expect("json service worker resource store should reopen");
        let store = reopened.lock();
        assert_eq!(store.registration_for_key(&first_key), Some(first));
        assert_eq!(store.registration_for_key(&second_key), Some(second));
    }

    #[test]
    fn json_service_worker_resource_store_normalizes_legacy_bare_origin_storage_key() {
        let temp = TempStorePath::new("legacy-storage-key");
        let expected = stored_registration();
        let expected_key = registration_key_for(&expected);
        let mut legacy = expected.clone();
        legacy.storage_key = "https://service-worker-resources.test".to_owned();

        {
            let store = new_shared_json_service_worker_resource_store(&temp.path)
                .expect("json service worker resource store should open");
            store
                .lock()
                .store_registration(legacy)
                .expect("legacy stored registration should persist");
        }

        let reopened = new_shared_json_service_worker_resource_store(&temp.path)
            .expect("json service worker resource store should reopen");
        assert_eq!(
            reopened.lock().registration_for_key(&expected_key),
            Some(expected)
        );
    }

    fn registration_key_for(
        registration: &ServiceWorkerStoredRegistration,
    ) -> ServiceWorkerRegistrationKey {
        ServiceWorkerRegistrationKey {
            scope_url: registration.scope_url.clone(),
            storage_key: registration.storage_key.clone(),
        }
    }

    fn stored_registration() -> ServiceWorkerStoredRegistration {
        let scope_url: Url = "https://service-worker-resources.test/app/"
            .parse()
            .expect("scope URL should parse");
        let main_script_resource = script_resource(
            "https://service-worker-resources.test/app/worker.js",
            WorkerScriptResourceKind::JavaScript,
        );
        let imported_script_resource = script_resource(
            "https://service-worker-resources.test/app/styles.css",
            WorkerScriptResourceKind::CssModule,
        );
        ServiceWorkerStoredRegistration {
            storage_key: ServiceWorkerRegistrationKey::storage_key_for_scope_url(&scope_url),
            scope_url,
            script_url: "https://service-worker-resources.test/app/worker.js"
                .parse()
                .expect("script URL should parse"),
            script_kind: WorkerScriptKind::Classic,
            update_via_cache: ServiceWorkerUpdateViaCache::None,
            lifecycle_state: ServiceWorkerVersionLifecycleState::Activated,
            fetch_handler_existence: ServiceWorkerFetchHandlerExistence::DoesNotExist,
            fetch_handler_type: ServiceWorkerFetchHandlerType::NoHandler,
            last_update_check_time_ms: Some(42),
            navigation_preload_state: ServiceWorkerNavigationPreloadState {
                enabled: true,
                header_value: "custom-preload".to_owned(),
            },
            main_script_resource,
            imported_script_resources: [(
                "https://service-worker-resources.test/app/styles.css".to_owned(),
                imported_script_resource,
            )]
            .into_iter()
            .collect(),
        }
    }

    fn script_resource(
        raw_url: &str,
        kind: WorkerScriptResourceKind,
    ) -> ServiceWorkerScriptResource {
        ServiceWorkerScriptResource {
            request_url: raw_url.parse().expect("request URL should parse"),
            final_url: raw_url.parse().expect("final URL should parse"),
            kind,
            status: 200,
            headers: vec![("content-type".to_owned(), "text/javascript".to_owned())],
            body_len: 11,
            body_sha256: "hash".to_owned(),
            response_time_ms: 7,
            mime_type: Some("text/javascript".to_owned()),
        }
    }
}
