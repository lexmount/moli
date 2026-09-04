use std::collections::HashMap;

use anyhow::{Result, anyhow};

use crate::context_bootstrap::specs::{
    ConstructorKind, ConstructorPrototypeProperty, ConstructorSpec,
};

pub(in crate::context_bootstrap) const STORAGE_INTERFACE_NAMES: &[&str] = &[
    "StorageManager",
    "StorageEstimate",
    "StorageBucketManager",
    "StorageBucket",
    "FileSystemHandle",
    "FileSystemFileHandle",
    "FileSystemDirectoryHandle",
    "FileSystemWritableFileStream",
    "FileSystemSyncAccessHandle",
];

pub(in crate::context_bootstrap) const WORKER_SHARED_INTERFACE_NAMES: &[&str] = &[
    "EventTarget",
    "Event",
    "CustomEvent",
    "MessageEvent",
    "ErrorEvent",
    "PromiseRejectionEvent",
    "SecurityPolicyViolationEvent",
    "CloseEvent",
    "Worker",
    "WebSocket",
    "AbortSignal",
    "AbortController",
    "Headers",
    "Request",
    "Response",
    "ProgressEvent",
    "XMLHttpRequestEventTarget",
    "XMLHttpRequestUpload",
    "XMLHttpRequest",
    "EventSource",
    "TextEncoder",
    "TextDecoder",
    "Crypto",
    "SubtleCrypto",
    "CryptoKey",
    "MediaCapabilities",
    "NavigatorUAData",
    "WorkerNavigator",
    "MessageChannel",
    "MessagePort",
    "BroadcastChannel",
    "Notification",
    "ImageData",
    "ImageBitmap",
    "CanvasGradient",
    "CanvasPattern",
    "TextMetrics",
    "OffscreenCanvas",
    "OffscreenCanvasRenderingContext2D",
    "WebGLRenderingContext",
    "WebGL2RenderingContext",
    "WebGLObject",
    "WebGLBuffer",
    "WebGLFramebuffer",
    "WebGLProgram",
    "WebGLRenderbuffer",
    "WebGLShader",
    "WebGLUniformLocation",
    "Path2D",
    "Blob",
    "File",
    "FileList",
    "FileReader",
    "FileReaderSync",
    "DOMException",
    "QuotaExceededError",
    "URL",
    "URLSearchParams",
    "FormData",
];

const SECURE_CONTEXT_ONLY_INTERFACE_NAMES: &[&str] = &["SubtleCrypto", "CryptoKey", "IdleDetector"];
const WORKER_ONLY_INTERFACE_NAMES: &[&str] = &["WorkerNavigator", "WorkerLocation"];
const WINDOW_DEDICATED_AND_SHARED_WORKER_INTERFACE_NAMES: &[&str] = &[
    "Worker",
    "ProgressEvent",
    "XMLHttpRequestEventTarget",
    "XMLHttpRequestUpload",
    "XMLHttpRequest",
];
const DEDICATED_AND_SHARED_WORKER_INTERFACE_NAMES: &[&str] = &["FileReaderSync"];

pub(in crate::context_bootstrap) const INDEXED_DB_INTERFACE_NAMES: &[&str] = &[
    "DOMStringList",
    "IDBFactory",
    "IDBRequest",
    "IDBOpenDBRequest",
    "IDBDatabase",
    "IDBTransaction",
    "IDBObjectStore",
    "IDBIndex",
    "IDBCursor",
    "IDBCursorWithValue",
    "IDBKeyRange",
    "IDBVersionChangeEvent",
];

#[cfg(test)]
pub(crate) fn dedicated_worker_lazy_interface_names_for_test() -> Vec<&'static str> {
    STORAGE_INTERFACE_NAMES
        .iter()
        .copied()
        .chain(WORKER_SHARED_INTERFACE_NAMES.iter().copied())
        .chain(crate::context_bootstrap::streams::stream_interface_names())
        .chain(INDEXED_DB_INTERFACE_NAMES.iter().copied())
        .collect()
}

/// Interfaces which still require eager realm construction.
///
/// `Window` owns the concrete global template and its prototype is needed
/// while the realm's named-properties chain is assembled.
const EAGER_INTERFACE_NAMES: &[&str] = &["Window"];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct InterfaceId(u32);

impl InterfaceId {
    pub(super) fn from_index(index: usize) -> Result<Self> {
        u32::try_from(index)
            .map(Self)
            .map_err(|_| anyhow!("exposed interface table exceeds u32 callback-data capacity"))
    }

    pub(super) const fn index(self) -> usize {
        self.0 as usize
    }

    pub(super) const fn callback_data(self) -> u32 {
        self.0
    }

    pub(super) const fn from_callback_data(value: u32) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RealmKind {
    Window,
    DedicatedWorker,
    SharedWorker,
    ServiceWorker,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::context_bootstrap) enum TemplateBuildProfile {
    Window,
    DedicatedWorker,
    SharedWorker,
    ServiceWorker,
}

impl TemplateBuildProfile {
    pub(super) const fn for_realm(realm: RealmKind) -> Self {
        match realm {
            RealmKind::Window => Self::Window,
            RealmKind::DedicatedWorker => Self::DedicatedWorker,
            RealmKind::SharedWorker => Self::SharedWorker,
            RealmKind::ServiceWorker => Self::ServiceWorker,
        }
    }

    pub(super) const fn realm_kind(self) -> RealmKind {
        match self {
            Self::Window => RealmKind::Window,
            Self::DedicatedWorker => RealmKind::DedicatedWorker,
            Self::SharedWorker => RealmKind::SharedWorker,
            Self::ServiceWorker => RealmKind::ServiceWorker,
        }
    }

    fn supports_name(self, name: &str) -> bool {
        match self {
            Self::Window => true,
            Self::DedicatedWorker | Self::SharedWorker | Self::ServiceWorker => {
                STORAGE_INTERFACE_NAMES.contains(&name)
                    || WORKER_SHARED_INTERFACE_NAMES.contains(&name)
                    || crate::context_bootstrap::streams::is_worker_exposed_stream_interface(name)
                    || INDEXED_DB_INTERFACE_NAMES.contains(&name)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ExposureSet(u8);

impl ExposureSet {
    const WINDOW: u8 = 1 << 0;
    const DEDICATED_WORKER: u8 = 1 << 1;
    const SHARED_WORKER: u8 = 1 << 2;
    const SERVICE_WORKER: u8 = 1 << 3;

    const WINDOW_ONLY: Self = Self(Self::WINDOW);
    const ALL_REALMS: Self =
        Self(Self::WINDOW | Self::DEDICATED_WORKER | Self::SHARED_WORKER | Self::SERVICE_WORKER);
    const WORKERS: Self = Self(Self::DEDICATED_WORKER | Self::SHARED_WORKER | Self::SERVICE_WORKER);
    const WINDOW_DEDICATED_AND_SHARED_WORKER: Self =
        Self(Self::WINDOW | Self::DEDICATED_WORKER | Self::SHARED_WORKER);
    const DEDICATED_AND_SHARED_WORKER: Self = Self(Self::DEDICATED_WORKER | Self::SHARED_WORKER);
    const DEDICATED_WORKER_ONLY: Self = Self(Self::DEDICATED_WORKER);

    pub(super) const fn contains(self, realm: RealmKind) -> bool {
        let bit = match realm {
            RealmKind::Window => Self::WINDOW,
            RealmKind::DedicatedWorker => Self::DEDICATED_WORKER,
            RealmKind::SharedWorker => Self::SHARED_WORKER,
            RealmKind::ServiceWorker => Self::SERVICE_WORKER,
        };
        self.0 & bit != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GlobalInstallation {
    Lazy,
    Eager,
    NotExposed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResolvedPrototypeProperty {
    TemplateReadOnly,
    RuntimeInstalled { prototype: InterfaceId },
}

#[derive(Clone, Copy)]
pub(super) struct ExposedInterfaceMetadata {
    pub(super) id: InterfaceId,
    pub(super) name: &'static str,
    pub(super) kind: ConstructorKind,
    pub(super) parent: Option<InterfaceId>,
    pub(super) exposure: ExposureSet,
    pub(super) secure_context_only: bool,
    pub(super) installation: GlobalInstallation,
    pub(super) prototype_property: ResolvedPrototypeProperty,
}

impl ExposedInterfaceMetadata {
    pub(super) fn is_exposed(self, realm: RealmKind, secure_context: bool) -> bool {
        self.installation != GlobalInstallation::NotExposed
            && self.exposure.contains(realm)
            && (!self.secure_context_only || secure_context)
    }

    pub(super) fn is_supported_by(self, profile: TemplateBuildProfile) -> bool {
        self.installation != GlobalInstallation::NotExposed
            && self.exposure.contains(profile.realm_kind())
            && profile.supports_name(self.name)
    }
}

pub(super) struct ExposedInterfaceMetadataTable {
    entries: Vec<ExposedInterfaceMetadata>,
    by_name: HashMap<&'static str, InterfaceId>,
}

impl ExposedInterfaceMetadataTable {
    pub(super) fn from_constructor_specs(specs: &[ConstructorSpec]) -> Result<Self> {
        let mut by_name = HashMap::with_capacity(specs.len());
        for (index, spec) in specs.iter().enumerate() {
            let id = InterfaceId::from_index(index)?;
            if by_name.insert(spec.name, id).is_some() {
                return Err(anyhow!(
                    "duplicate exposed interface metadata name `{}`",
                    spec.name
                ));
            }
        }

        let mut entries = Vec::with_capacity(specs.len());
        for (index, spec) in specs.iter().enumerate() {
            let parent = spec
                .parent
                .map(|name| {
                    by_name.get(name).copied().ok_or_else(|| {
                        anyhow!(
                            "exposed interface `{}` references unknown parent `{name}`",
                            spec.name
                        )
                    })
                })
                .transpose()?;
            let is_storage = STORAGE_INTERFACE_NAMES.contains(&spec.name);
            let exposure = exposure_for_name(spec.name);
            let prototype_property = match spec.prototype_property() {
                ConstructorPrototypeProperty::TemplateReadOnly => {
                    ResolvedPrototypeProperty::TemplateReadOnly
                }
                ConstructorPrototypeProperty::RuntimeInstalled {
                    prototype_interface,
                } => {
                    let prototype = by_name.get(prototype_interface).copied().ok_or_else(|| {
                        anyhow!(
                            "exposed interface `{}` references unknown runtime prototype interface `{prototype_interface}`",
                            spec.name
                        )
                    })?;
                    ResolvedPrototypeProperty::RuntimeInstalled { prototype }
                }
            };
            entries.push(ExposedInterfaceMetadata {
                id: InterfaceId::from_index(index)?,
                name: spec.name,
                kind: spec.kind,
                parent,
                exposure,
                secure_context_only: is_storage
                    || SECURE_CONTEXT_ONLY_INTERFACE_NAMES.contains(&spec.name),
                installation: installation_for_spec(*spec),
                prototype_property,
            });
        }

        validate_materialization_cycles(&entries)?;
        Ok(Self { entries, by_name })
    }

    pub(super) fn entries(&self) -> &[ExposedInterfaceMetadata] {
        &self.entries
    }

    pub(super) fn get(&self, id: InterfaceId) -> Option<ExposedInterfaceMetadata> {
        self.entries.get(id.index()).copied()
    }

    pub(super) fn id_by_name(&self, name: &str) -> Option<InterfaceId> {
        self.by_name.get(name).copied()
    }

    #[cfg(test)]
    pub(super) fn metadata_by_name(&self, name: &str) -> Option<ExposedInterfaceMetadata> {
        self.id_by_name(name).and_then(|id| self.get(id))
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }
}

pub(super) fn installation_for_spec(spec: ConstructorSpec) -> GlobalInstallation {
    if EAGER_INTERFACE_NAMES.contains(&spec.name) {
        GlobalInstallation::Eager
    } else {
        GlobalInstallation::Lazy
    }
}

fn exposure_for_name(name: &str) -> ExposureSet {
    match name {
        "FileSystemSyncAccessHandle" => ExposureSet::DEDICATED_WORKER_ONLY,
        _ if WINDOW_DEDICATED_AND_SHARED_WORKER_INTERFACE_NAMES.contains(&name) => {
            ExposureSet::WINDOW_DEDICATED_AND_SHARED_WORKER
        }
        _ if DEDICATED_AND_SHARED_WORKER_INTERFACE_NAMES.contains(&name) => {
            ExposureSet::DEDICATED_AND_SHARED_WORKER
        }
        _ if WORKER_ONLY_INTERFACE_NAMES.contains(&name) => ExposureSet::WORKERS,
        _ if STORAGE_INTERFACE_NAMES.contains(&name)
            || WORKER_SHARED_INTERFACE_NAMES.contains(&name)
            || crate::context_bootstrap::streams::is_worker_exposed_stream_interface(name)
            || INDEXED_DB_INTERFACE_NAMES.contains(&name) =>
        {
            ExposureSet::ALL_REALMS
        }
        _ => ExposureSet::WINDOW_ONLY,
    }
}

pub(in crate::context_bootstrap) fn constructor_spec_is_lazy(spec: ConstructorSpec) -> bool {
    installation_for_spec(spec) == GlobalInstallation::Lazy
}

fn validate_materialization_cycles(entries: &[ExposedInterfaceMetadata]) -> Result<()> {
    fn visit(
        id: InterfaceId,
        entries: &[ExposedInterfaceMetadata],
        states: &mut [u8],
    ) -> Result<()> {
        match states[id.index()] {
            2 => return Ok(()),
            1 => {
                return Err(anyhow!(
                    "exposed interface parent cycle reaches `{}`",
                    entries[id.index()].name
                ));
            }
            _ => {}
        }
        states[id.index()] = 1;
        if let Some(parent) = entries[id.index()].parent {
            visit(parent, entries, states)?;
        }
        if let ResolvedPrototypeProperty::RuntimeInstalled { prototype } =
            entries[id.index()].prototype_property
        {
            visit(prototype, entries, states)?;
        }
        states[id.index()] = 2;
        Ok(())
    }

    let mut states = vec![0; entries.len()];
    for metadata in entries {
        visit(metadata.id, entries, &mut states)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_bootstrap::specs::ConstructorKind;

    fn spec(name: &'static str, parent: Option<&'static str>) -> ConstructorSpec {
        ConstructorSpec {
            name,
            parent,
            kind: ConstructorKind::Illegal,
        }
    }

    #[test]
    fn metadata_assigns_stable_ids_and_resolves_parents() {
        let table = ExposedInterfaceMetadataTable::from_constructor_specs(&[
            spec("Parent", None),
            spec("Child", Some("Parent")),
        ])
        .expect("valid metadata");
        let parent = table.metadata_by_name("Parent").expect("parent metadata");
        let child = table.metadata_by_name("Child").expect("child metadata");

        assert_eq!(parent.id.index(), 0);
        assert_eq!(child.id.index(), 1);
        assert_eq!(child.parent, Some(parent.id));
        assert_eq!(child.installation, GlobalInstallation::Lazy);
    }

    #[test]
    fn storage_policy_matches_window_and_worker_exposure() {
        let table = ExposedInterfaceMetadataTable::from_constructor_specs(&[
            spec("StorageManager", None),
            spec("FileSystemSyncAccessHandle", None),
        ])
        .expect("storage metadata");
        let manager = table
            .metadata_by_name("StorageManager")
            .expect("StorageManager");
        assert_eq!(manager.installation, GlobalInstallation::Lazy);
        assert!(!manager.is_exposed(RealmKind::Window, false));
        assert!(manager.is_exposed(RealmKind::Window, true));
        assert!(manager.is_exposed(RealmKind::SharedWorker, true));

        let sync = table
            .metadata_by_name("FileSystemSyncAccessHandle")
            .expect("sync access handle");
        assert!(!sync.is_exposed(RealmKind::Window, true));
        assert!(sync.is_exposed(RealmKind::DedicatedWorker, true));
        assert!(!sync.is_exposed(RealmKind::SharedWorker, true));
    }

    #[test]
    fn shared_and_window_only_interface_exposure_is_explicit() {
        let table = ExposedInterfaceMetadataTable::from_constructor_specs(&[
            spec("URL", None),
            spec("Range", None),
            spec("CSSStyleRule", None),
            spec("IDBFactory", None),
            spec("WorkerLocation", None),
        ])
        .expect("shared interface metadata");

        for name in ["URL", "IDBFactory"] {
            let metadata = table.metadata_by_name(name).expect("shared metadata");
            for realm in [
                RealmKind::Window,
                RealmKind::DedicatedWorker,
                RealmKind::SharedWorker,
                RealmKind::ServiceWorker,
            ] {
                assert!(metadata.is_exposed(realm, true), "{name} in {realm:?}");
            }
        }

        let range = table.metadata_by_name("Range").expect("Range metadata");
        assert!(range.is_exposed(RealmKind::Window, true));
        assert!(!range.is_exposed(RealmKind::DedicatedWorker, true));
        assert!(!range.is_exposed(RealmKind::SharedWorker, true));
        assert!(!range.is_exposed(RealmKind::ServiceWorker, true));

        let css_style_rule = table
            .metadata_by_name("CSSStyleRule")
            .expect("CSSStyleRule metadata");
        assert!(css_style_rule.is_exposed(RealmKind::Window, true));
        assert!(!css_style_rule.is_exposed(RealmKind::DedicatedWorker, true));
        assert!(!css_style_rule.is_exposed(RealmKind::SharedWorker, true));
        assert!(!css_style_rule.is_exposed(RealmKind::ServiceWorker, true));

        let worker_location = table
            .metadata_by_name("WorkerLocation")
            .expect("WorkerLocation metadata");
        assert!(!worker_location.is_exposed(RealmKind::Window, true));
        assert!(worker_location.is_exposed(RealmKind::DedicatedWorker, true));
        assert!(worker_location.is_exposed(RealmKind::SharedWorker, true));
        assert!(worker_location.is_exposed(RealmKind::ServiceWorker, true));
    }

    #[test]
    fn chromium_worker_exposure_exceptions_are_preserved() {
        let table = ExposedInterfaceMetadataTable::from_constructor_specs(&[
            spec("Worker", Some("EventTarget")),
            spec("EventTarget", None),
            spec("ProgressEvent", Some("Event")),
            spec("Event", None),
            spec("XMLHttpRequestEventTarget", Some("EventTarget")),
            spec("XMLHttpRequestUpload", Some("XMLHttpRequestEventTarget")),
            spec("XMLHttpRequest", Some("XMLHttpRequestEventTarget")),
            spec("FileReaderSync", None),
        ])
        .expect("worker exposure metadata");

        let worker = table.metadata_by_name("Worker").expect("Worker metadata");
        assert!(worker.is_exposed(RealmKind::Window, true));
        assert!(worker.is_exposed(RealmKind::DedicatedWorker, true));
        assert!(worker.is_exposed(RealmKind::SharedWorker, true));
        assert!(!worker.is_exposed(RealmKind::ServiceWorker, true));

        for name in [
            "ProgressEvent",
            "XMLHttpRequestEventTarget",
            "XMLHttpRequestUpload",
            "XMLHttpRequest",
        ] {
            let metadata = table.metadata_by_name(name).expect("XHR metadata");
            assert!(metadata.is_exposed(RealmKind::Window, true), "{name}");
            assert!(
                metadata.is_exposed(RealmKind::DedicatedWorker, true),
                "{name}"
            );
            assert!(metadata.is_exposed(RealmKind::SharedWorker, true), "{name}");
            assert!(
                !metadata.is_exposed(RealmKind::ServiceWorker, true),
                "{name}"
            );
        }

        let file_reader_sync = table
            .metadata_by_name("FileReaderSync")
            .expect("FileReaderSync metadata");
        assert!(!file_reader_sync.is_exposed(RealmKind::Window, true));
        assert!(file_reader_sync.is_exposed(RealmKind::DedicatedWorker, true));
        assert!(file_reader_sync.is_exposed(RealmKind::SharedWorker, true));
        assert!(!file_reader_sync.is_exposed(RealmKind::ServiceWorker, true));
    }

    #[test]
    fn exposed_interfaces_default_to_lazy_with_window_as_the_eager_exception() {
        let table = ExposedInterfaceMetadataTable::from_constructor_specs(&[
            spec("Event", None),
            spec("Window", None),
            ConstructorSpec {
                name: "Audio",
                parent: None,
                kind: ConstructorKind::Audio,
            },
            spec("HTMLAudioElement", None),
        ])
        .expect("default installation metadata");

        assert_eq!(
            table
                .metadata_by_name("Event")
                .expect("Event metadata")
                .installation,
            GlobalInstallation::Lazy
        );
        assert_eq!(
            table
                .metadata_by_name("Window")
                .expect("Window metadata")
                .installation,
            GlobalInstallation::Eager
        );
        assert_eq!(
            table
                .metadata_by_name("Audio")
                .expect("Audio metadata")
                .installation,
            GlobalInstallation::Lazy
        );
    }

    #[test]
    fn metadata_rejects_duplicate_unknown_parent_and_cycle() {
        assert!(
            ExposedInterfaceMetadataTable::from_constructor_specs(&[
                spec("Duplicate", None),
                spec("Duplicate", None),
            ])
            .is_err()
        );
        assert!(
            ExposedInterfaceMetadataTable::from_constructor_specs(&[spec(
                "Child",
                Some("Missing"),
            )])
            .is_err()
        );
        assert!(
            ExposedInterfaceMetadataTable::from_constructor_specs(&[
                spec("A", Some("B")),
                spec("B", Some("A")),
            ])
            .is_err()
        );
    }

    #[test]
    fn metadata_resolves_and_validates_runtime_prototype_dependencies() {
        let table = ExposedInterfaceMetadataTable::from_constructor_specs(&[
            ConstructorSpec {
                name: "Audio",
                parent: None,
                kind: ConstructorKind::Audio,
            },
            spec("HTMLAudioElement", None),
        ])
        .expect("legacy factory metadata");
        let audio = table.metadata_by_name("Audio").expect("Audio metadata");
        let element = table
            .metadata_by_name("HTMLAudioElement")
            .expect("HTMLAudioElement metadata");
        assert_eq!(
            audio.prototype_property,
            ResolvedPrototypeProperty::RuntimeInstalled {
                prototype: element.id
            }
        );

        assert!(
            ExposedInterfaceMetadataTable::from_constructor_specs(&[ConstructorSpec {
                name: "Audio",
                parent: None,
                kind: ConstructorKind::Audio,
            }])
            .is_err(),
            "a missing runtime prototype dependency must fail bootstrap"
        );
        assert!(
            ExposedInterfaceMetadataTable::from_constructor_specs(&[
                ConstructorSpec {
                    name: "Audio",
                    parent: None,
                    kind: ConstructorKind::Audio,
                },
                spec("HTMLAudioElement", Some("Audio")),
            ])
            .is_err(),
            "runtime prototype dependencies must participate in cycle validation"
        );
    }
}
