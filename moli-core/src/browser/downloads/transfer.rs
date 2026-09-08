use std::{io, path::PathBuf};

use moli_fetch::{FetchCancelHandle, Request};
use tokio::{io::AsyncWriteExt, sync::watch};
use url::Url;

use crate::network::ResourceRequestClient;

use super::{
    DownloadBehavior, DownloadBody, DownloadMetadata, DownloadSnapshot, DownloadState, naming,
};

pub(super) struct DownloadRequest {
    pub client: ResourceRequestClient,
    pub request: Request,
    pub suggested_filename: Option<String>,
}

pub(super) enum Source {
    Request(Box<DownloadRequest>),
    Response {
        url: Url,
        headers: Vec<(String, String)>,
        body: DownloadBody,
    },
}

impl Source {
    pub(super) fn initial_metadata(&self) -> Option<DownloadMetadata> {
        match self {
            Self::Request(request) => request
                .suggested_filename
                .as_deref()
                .and_then(naming::non_empty_filename)
                .map(|hint| DownloadMetadata::new(request.request.url.as_str(), &[], Some(hint))),
            Self::Response { url, headers, .. } => {
                Some(DownloadMetadata::new(url.as_str(), headers, None))
            }
        }
    }

    pub(super) fn fallback_metadata(&self) -> DownloadMetadata {
        match self {
            Self::Request(request) => DownloadMetadata::new(
                request.request.url.as_str(),
                &[],
                request.suggested_filename.as_deref(),
            ),
            Self::Response { url, headers, .. } => {
                DownloadMetadata::new(url.as_str(), headers, None)
            }
        }
    }
}

pub(super) async fn run(
    source: Source,
    root: PathBuf,
    behavior: DownloadBehavior,
    guid: String,
    state: watch::Sender<DownloadSnapshot>,
    mut cancellation: watch::Receiver<bool>,
) {
    let fallback = source.fallback_metadata();
    let result = write(source, root, behavior, guid, &state, &mut cancellation).await;
    let result = match result {
        Ok(artifact) => {
            // This closure owns the partial file even if the awaiter is dropped.
            tokio::task::spawn_blocking(move || artifact.commit(&cancellation))
                .await
                .unwrap_or_else(|error| Err(io::Error::other(error)))
        }
        Err(error) => Err(error),
    };
    state.send_modify(|snapshot| {
        snapshot.metadata.get_or_insert(fallback);
        snapshot.total_bytes = Some(snapshot.received_bytes);
        snapshot.state = match result {
            Ok(artifact_path) => DownloadState::Completed { artifact_path },
            Err(_) => DownloadState::Canceled,
        };
    });
}

async fn canceled(cancellation: &mut watch::Receiver<bool>) {
    if !*cancellation.borrow() {
        let _ = cancellation.changed().await;
    }
}

async fn write(
    source: Source,
    root: PathBuf,
    behavior: DownloadBehavior,
    guid: String,
    state: &watch::Sender<DownloadSnapshot>,
    cancellation: &mut watch::Receiver<bool>,
) -> io::Result<PartialArtifact> {
    let (metadata, headers, body) = match source {
        Source::Request(request) => {
            let mut cancel = CancelFetchOnDrop(Some(FetchCancelHandle::new()));
            let fetch = request
                .client
                .fetch_raw_stream_with_cancel(request.request, cancel.0.as_ref().unwrap().clone());
            let response = tokio::select! {
                biased;
                _ = canceled(cancellation) => return Err(io::Error::other("download canceled")),
                response = fetch => response.map_err(io::Error::other)?,
            };
            // StreamingRawResponse now owns transport cancellation. Only disarm
            // the fetch guard once the returned stream has that responsibility.
            cancel.0 = None;
            let metadata = DownloadMetadata::new(
                response.final_url.as_str(),
                &response.headers,
                request.suggested_filename.as_deref(),
            );
            (
                metadata,
                response.headers.clone(),
                DownloadBody::Streaming(Box::new(response)),
            )
        }
        Source::Response { url, headers, body } => (
            DownloadMetadata::new(url.as_str(), &headers, None),
            headers,
            body,
        ),
    };
    let name = naming::artifact_file_name(behavior, &guid, &metadata.suggested_filename);
    state.send_modify(|snapshot| {
        snapshot.metadata.get_or_insert(metadata);
        snapshot.total_bytes = naming::content_length_from_headers(&headers);
    });
    let mut artifact =
        tokio::task::spawn_blocking(move || PartialArtifact::create(root, guid, name))
            .await
            .map_err(io::Error::other)??;
    let mut file = tokio::fs::File::from_std(
        artifact
            .file
            .take()
            .expect("new partial artifact has a file"),
    );
    let write_body = async {
        match body {
            DownloadBody::Buffered(body) => {
                file.write_all(&body).await?;
                state.send_modify(|snapshot| snapshot.received_bytes = body.len() as u64);
            }
            DownloadBody::Streaming(mut response) => {
                while let Some(chunk) = response.next_chunk().await {
                    file.write_all(&chunk).await?;
                    state.send_modify(|snapshot| {
                        snapshot.received_bytes =
                            snapshot.received_bytes.saturating_add(chunk.len() as u64);
                    });
                }
                response.finish().await.map_err(io::Error::other)?;
            }
            DownloadBody::Captured(body) => {
                let mut reader = body.chunk_reader(64 * 1024).map_err(io::Error::other)?;
                loop {
                    let (next, chunk) = tokio::task::spawn_blocking(move || {
                        let chunk = reader.next_chunk();
                        (reader, chunk)
                    })
                    .await
                    .map_err(io::Error::other)?;
                    reader = next;
                    let Some(chunk) = chunk.map_err(io::Error::other)? else {
                        break;
                    };
                    file.write_all(&chunk).await?;
                    state.send_modify(|snapshot| {
                        snapshot.received_bytes =
                            snapshot.received_bytes.saturating_add(chunk.len() as u64);
                    });
                }
            }
        }
        file.flush().await
    };
    // File creation is awaited to regain its ownership before cancellation can
    // publish a terminal state. Cleanup therefore precedes terminal observation.
    tokio::select! {
        biased;
        _ = canceled(cancellation) => return Err(io::Error::other("download canceled")),
        result = write_body => result?,
    }
    drop(file);
    Ok(artifact)
}

struct CancelFetchOnDrop(Option<FetchCancelHandle>);
impl Drop for CancelFetchOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = &self.0 {
            cancel.cancel();
        }
    }
}

/// Unique per-download temporary file; never delete another transfer's partial
/// artifact, including when two downloads choose the same final filename.
struct PartialArtifact {
    file: Option<std::fs::File>,
    partial: PathBuf,
    destination: PathBuf,
}

impl PartialArtifact {
    fn create(root: PathBuf, guid: String, name: String) -> io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        let partial = root.join(format!(".{guid}.crdownload"));
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        Ok(Self {
            file: Some(file),
            partial,
            destination: root.join(name),
        })
    }

    fn commit(self, cancellation: &watch::Receiver<bool>) -> io::Result<PathBuf> {
        if *cancellation.borrow() || cancellation.has_changed().is_err() {
            return Err(io::Error::other("download canceled"));
        }
        std::fs::rename(&self.partial, &self.destination)?;
        Ok(self.destination.clone())
    }
}

impl Drop for PartialArtifact {
    fn drop(&mut self) {
        self.file = None;
        let _ = std::fs::remove_file(&self.partial);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn finalize_download_artifact_preserves_existing_artifact_when_rename_fails() {
        let root = std::env::temp_dir().join(format!(
            "moli-download-finalize-{}",
            naming::generate_download_guid().unwrap()
        ));
        let partial = PartialArtifact::create(
            root.clone(),
            naming::generate_download_guid().unwrap(),
            "artifact.bin".into(),
        )
        .unwrap();
        let artifact_path = root.join("artifact.bin");
        std::fs::write(&artifact_path, b"previous artifact").unwrap();
        std::fs::remove_file(&partial.partial).unwrap();
        let (_owner, cancellation) = watch::channel(false);
        assert!(partial.commit(&cancellation).is_err());
        assert_eq!(std::fs::read(&artifact_path).unwrap(), b"previous artifact");
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
