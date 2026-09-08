use std::{collections::HashMap, path::PathBuf};

use moli_fetch::{Request, StreamingRawResponse};
use tokio::sync::watch;
use url::Url;

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

/// Read-only observation. Dropping it never cancels Browser work. Slow observers
/// retain only the latest progress, not an unbounded queue of network chunks.
#[derive(Clone)]
pub struct DownloadObservation {
    guid: String,
    updates: watch::Receiver<DownloadSnapshot>,
}

impl DownloadObservation {
    pub fn guid(&self) -> &str {
        &self.guid
    }

    pub fn snapshot(&mut self) -> DownloadSnapshot {
        self.updates.borrow_and_update().clone()
    }

    pub async fn next_update(&mut self) -> Option<DownloadSnapshot> {
        self.updates.changed().await.ok()?;
        Some(self.snapshot())
    }

    /// A denied activation has an observable lifetime but no transfer or artifact.
    pub fn denied(
        url: &str,
        headers: &[(String, String)],
        hint: Option<&str>,
    ) -> Result<Self, String> {
        let (_, updates) = watch::channel(DownloadSnapshot {
            metadata: Some(DownloadMetadata::new(url, headers, hint)),
            received_bytes: 0,
            total_bytes: Some(0),
            state: DownloadState::Canceled,
        });
        Ok(Self {
            guid: naming::generate_download_guid()?,
            updates,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadAccessError {
    AlreadyTerminal,
    InProgress,
    NoArtifact,
}

/// One BrowserContext's download authority. Only admission mutates this collection;
/// a transfer can publish to its own record, never insert or resurrect records.
/// Dropping the manager closes all cancellation leases, independent of observers.
#[derive(Default)]
pub struct DownloadManager {
    records: HashMap<String, DownloadRecord>,
}

struct DownloadRecord {
    cancel: watch::Sender<bool>,
    updates: watch::Receiver<DownloadSnapshot>,
}

impl DownloadManager {
    pub fn start_request(
        &mut self,
        policy: &DownloadPolicy,
        client: ResourceRequestClient,
        request: Request,
        suggested_filename: Option<String>,
    ) -> Result<Option<DownloadObservation>, String> {
        self.start(
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
        policy: &DownloadPolicy,
        url: Url,
        headers: Vec<(String, String)>,
        body: DownloadBody,
    ) -> Result<Option<DownloadObservation>, String> {
        self.start(policy, transfer::Source::Response { url, headers, body })
    }

    fn start(
        &mut self,
        policy: &DownloadPolicy,
        source: transfer::Source,
    ) -> Result<Option<DownloadObservation>, String> {
        if !policy.behavior.allows_download() {
            return Ok(None);
        }
        let Some(root) = policy.download_path.as_ref() else {
            return Ok(None);
        };
        let guid = naming::generate_download_guid()?;
        let initial = DownloadSnapshot {
            metadata: source.initial_metadata(),
            received_bytes: 0,
            total_bytes: None,
            state: DownloadState::Active,
        };
        let (state, updates) = watch::channel(initial);
        let (cancel, cancellation) = watch::channel(false);
        self.records.insert(
            guid.clone(),
            DownloadRecord {
                cancel,
                updates: updates.clone(),
            },
        );
        tokio::spawn(transfer::run(
            source,
            PathBuf::from(root),
            policy.behavior,
            guid.clone(),
            state,
            cancellation,
        ));
        Ok(Some(DownloadObservation { guid, updates }))
    }

    pub fn cancel(&self, guid: &str) -> Option<Result<(), DownloadAccessError>> {
        let record = self.records.get(guid)?;
        Some(if record.updates.borrow().state == DownloadState::Active {
            record.cancel.send_replace(true);
            Ok(())
        } else {
            Err(DownloadAccessError::AlreadyTerminal)
        })
    }

    pub fn read_artifact(
        &self,
        guid: &str,
    ) -> Option<Result<tokio::task::JoinHandle<Result<Vec<u8>, String>>, DownloadAccessError>> {
        let record = self.records.get(guid)?;
        Some(match &record.updates.borrow().state {
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
