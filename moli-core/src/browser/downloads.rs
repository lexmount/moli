use std::{collections::HashMap, path::PathBuf, sync::Arc};

use moli_fetch::{Request, StreamingRawResponse};
use tokio::sync::watch;
use url::Url;

use super::{BrowserSequence, WebContentsHandle};
use crate::network::ResourceRequestClient;

mod naming;
mod transfer;

#[cfg(test)]
mod tests;

/// Browser policy only: no frontend subscriptions or session attribution.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DownloadPolicy {
    pub behavior: DownloadBehavior,
    pub download_path: Option<String>,
}

/// A response body whose ownership has been transferred to the Browser download service.
#[derive(Debug)]
pub enum DownloadBody {
    Buffered(Vec<u8>),
    Streaming(Box<StreamingRawResponse>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadMetadata {
    pub url: String,
    pub suggested_filename: String,
}

impl DownloadMetadata {
    fn new(url: &str, headers: &[(String, String)], hint: Option<&str>) -> Self {
        Self {
            url: url.to_owned(),
            suggested_filename: naming::filename_from_headers(headers)
                .or_else(|| hint.and_then(naming::non_empty_filename).map(str::to_owned))
                .or_else(|| {
                    Url::parse(url)
                        .ok()
                        .and_then(|url| naming::filename_from_url(&url))
                })
                .unwrap_or_else(|| "download".to_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadState {
    Active,
    Completed { artifact_path: PathBuf },
    Canceled,
}

/// A complete observation, including the start metadata even if progress is coalesced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadSnapshot {
    pub metadata: Option<DownloadMetadata>,
    pub received_bytes: u64,
    pub total_bytes: Option<u64>,
    pub state: DownloadState,
}

/// One exact Browser-owned download revision, including its physical source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadEvent {
    pub sequence: BrowserSequence,
    pub web_contents: WebContentsHandle,
    pub guid: String,
    pub snapshot: DownloadSnapshot,
}

/// Read-only observation. Dropping it never cancels Browser work. Slow observers
/// retain only the latest progress, not an unbounded queue of network chunks.
#[derive(Clone, Debug)]
pub struct DownloadObservation {
    guid: String,
    updates: watch::Receiver<Arc<DownloadEvent>>,
}

impl PartialEq for DownloadObservation {
    fn eq(&self, other: &Self) -> bool {
        self.guid == other.guid && self.updates.same_channel(&other.updates)
    }
}
impl Eq for DownloadObservation {}

/// A frozen native revision together with read-only access to that exact
/// record. A retired Context cannot be queried again, but existing observers
/// can still recover its transfer's cleanup result after event-stream lag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DownloadRecordSnapshot {
    pub event: Arc<DownloadEvent>,
    pub observation: DownloadObservation,
}

impl DownloadObservation {
    pub fn guid(&self) -> &str {
        &self.guid
    }

    pub fn snapshot(&mut self) -> DownloadSnapshot {
        self.event().snapshot.clone()
    }

    pub fn event(&mut self) -> Arc<DownloadEvent> {
        self.updates.borrow_and_update().clone()
    }

    pub fn record_snapshot(&mut self) -> DownloadRecordSnapshot {
        DownloadRecordSnapshot {
            event: self.event(),
            observation: self.clone(),
        }
    }

    pub async fn next_update(&mut self) -> Option<DownloadSnapshot> {
        self.updates.changed().await.ok()?;
        Some(self.snapshot())
    }
}

pub(in crate::browser) struct AdmittedDownload {
    pub observation: DownloadObservation,
    pub state: watch::Sender<Arc<DownloadEvent>>,
    pub progress: Option<watch::Receiver<DownloadSnapshot>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadAccessError {
    AlreadyTerminal,
    InProgress,
    NoArtifact,
}

/// One BrowserContext's download authority. Only admission mutates this collection;
/// transfer reports are committed to their original records by the Browser owner.
/// Dropping the manager closes all cancellation leases, independent of observers.
#[derive(Default)]
pub(in crate::browser) struct DownloadManager {
    records: HashMap<String, DownloadRecord>,
}

struct DownloadRecord {
    cancel: Option<watch::Sender<bool>>,
    updates: watch::Receiver<Arc<DownloadEvent>>,
}

impl DownloadManager {
    pub fn start_request(
        &mut self,
        web_contents: WebContentsHandle,
        policy: &DownloadPolicy,
        client: ResourceRequestClient,
        request: Request,
        suggested_filename: Option<String>,
    ) -> Result<Option<AdmittedDownload>, String> {
        self.start(
            web_contents,
            policy,
            transfer::Source::Request(Box::new(transfer::DownloadRequest {
                client,
                request,
                suggested_filename,
            })),
        )
    }

    pub fn start_response(
        &mut self,
        web_contents: WebContentsHandle,
        policy: &DownloadPolicy,
        url: Url,
        headers: Vec<(String, String)>,
        body: DownloadBody,
    ) -> Result<Option<AdmittedDownload>, String> {
        self.start(
            web_contents,
            policy,
            transfer::Source::Response { url, headers, body },
        )
    }

    pub fn deny(
        &mut self,
        web_contents: WebContentsHandle,
        url: &str,
        headers: &[(String, String)],
        hint: Option<&str>,
    ) -> Result<AdmittedDownload, String> {
        self.admit_denied(web_contents, DownloadMetadata::new(url, headers, hint))
    }

    fn admit_denied(
        &mut self,
        web_contents: WebContentsHandle,
        metadata: DownloadMetadata,
    ) -> Result<AdmittedDownload, String> {
        self.admit(
            web_contents,
            DownloadSnapshot {
                metadata: Some(metadata),
                received_bytes: 0,
                total_bytes: Some(0),
                state: DownloadState::Canceled,
            },
            None,
        )
    }

    fn start(
        &mut self,
        web_contents: WebContentsHandle,
        policy: &DownloadPolicy,
        source: transfer::Source,
    ) -> Result<Option<AdmittedDownload>, String> {
        if policy.behavior.is_canceled_without_download() {
            return self
                .admit_denied(web_contents, source.fallback_metadata())
                .map(Some);
        }
        let Some(root) = policy.download_path.as_ref() else {
            return Ok(None);
        };
        let initial = DownloadSnapshot {
            metadata: source.initial_metadata(),
            received_bytes: 0,
            total_bytes: None,
            state: DownloadState::Active,
        };
        let (cancel, cancellation) = watch::channel(false);
        let mut admitted = self.admit(web_contents, initial.clone(), Some(cancel))?;
        let (progress, observed) = watch::channel(initial);
        tokio::spawn(transfer::run(
            source,
            PathBuf::from(root),
            policy.behavior,
            admitted.observation.guid.clone(),
            progress,
            cancellation,
        ));
        admitted.progress = Some(observed);
        Ok(Some(admitted))
    }

    fn admit(
        &mut self,
        web_contents: WebContentsHandle,
        snapshot: DownloadSnapshot,
        cancel: Option<watch::Sender<bool>>,
    ) -> Result<AdmittedDownload, String> {
        let guid = naming::generate_download_guid()?;
        let (state, updates) = watch::channel(Arc::new(DownloadEvent {
            sequence: BrowserSequence::allocate(),
            web_contents,
            guid: guid.clone(),
            snapshot,
        }));
        // A denied activation has an observable terminal occurrence, but no
        // transfer item to cancel or open. Preserve that public access contract.
        self.records.insert(
            guid.clone(),
            DownloadRecord {
                cancel,
                updates: updates.clone(),
            },
        );
        Ok(AdmittedDownload {
            observation: DownloadObservation { guid, updates },
            state,
            progress: None,
        })
    }

    pub fn snapshots(&self) -> impl Iterator<Item = DownloadRecordSnapshot> + '_ {
        self.records.values().map(|record| {
            let event = record.updates.borrow().clone();
            DownloadRecordSnapshot {
                observation: DownloadObservation {
                    guid: event.guid.clone(),
                    updates: record.updates.clone(),
                },
                event,
            }
        })
    }

    pub fn cancel(&self, guid: &str) -> Option<Result<(), DownloadAccessError>> {
        let record = self.records.get(guid)?;
        let cancel = record.cancel.as_ref()?;
        Some(
            if record.updates.borrow().snapshot.state == DownloadState::Active {
                cancel.send_replace(true);
                Ok(())
            } else {
                Err(DownloadAccessError::AlreadyTerminal)
            },
        )
    }

    pub fn read_artifact(
        &self,
        guid: &str,
    ) -> Option<Result<tokio::task::JoinHandle<Result<Vec<u8>, String>>, DownloadAccessError>> {
        let record = self.records.get(guid)?;
        record.cancel.as_ref()?;
        Some(match &record.updates.borrow().snapshot.state {
            DownloadState::Active => Err(DownloadAccessError::InProgress),
            DownloadState::Canceled => Err(DownloadAccessError::NoArtifact),
            DownloadState::Completed { artifact_path } => {
                let path = artifact_path.clone();
                Ok(tokio::task::spawn_blocking(move || {
                    std::fs::read(&path)
                        .map_err(|_| format!("Download artifact not found: {}", path.display()))
                }))
            }
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DownloadBehavior {
    #[default]
    Default,
    Deny,
    Allow,
    AllowAndName,
}

impl DownloadBehavior {
    pub fn allows_download(self) -> bool {
        matches!(self, Self::Allow | Self::AllowAndName)
    }

    pub fn names_artifact_by_guid(self) -> bool {
        self == Self::AllowAndName
    }

    pub fn is_canceled_without_download(self) -> bool {
        matches!(self, Self::Default | Self::Deny)
    }
}
