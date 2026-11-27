//! File transport for already-authorized browser requests.
use std::{fs::File, path::Path};

use anyhow::{Context, Result, bail, ensure};
use moli_fetch::{FetchCancelHandle, FetchCancelled, ResponseHead, StreamingRawResponse};
use tokio::{
    io::AsyncReadExt,
    sync::{mpsc, oneshot},
};
use url::Url;

const READ_CHUNK_BYTES: usize = 64 * 1024;
const QUEUED_CHUNKS: usize = 2;

pub(super) async fn stream_file(
    url: Url,
    max_bytes: Option<usize>,
    cancel: FetchCancelHandle,
) -> Result<StreamingRawResponse> {
    let path = url
        .to_file_path()
        .map_err(|()| anyhow::anyhow!("Invalid local file path"))?;
    let read_cancel = cancel.clone();
    let max_bytes = max_bytes.map(|limit| limit as u64);
    let open = tokio::task::spawn_blocking(move || {
        ensure!(!read_cancel.is_cancelled(), FetchCancelled);
        let (file, size) = open_file(&path, max_bytes)?;
        Ok::<_, anyhow::Error>((file, size, file_head(url, &path, size)))
    });
    let (file, size, head) = tokio::select! {
        biased;
        () = cancel.cancelled() => bail!(FetchCancelled),
        opened = open => opened.context("File loader worker failed")??,
    };
    let (body_tx, body_rx) = mpsc::channel(QUEUED_CHUNKS);
    let (completion_tx, completion_rx) = oneshot::channel();
    let read_cancel = cancel.clone();
    tokio::spawn(async move {
        let result = tokio::select! {
            biased;
            () = read_cancel.cancelled() => Err(FetchCancelled.into()),
            result = async {
                let mut file = tokio::fs::File::from_std(file);
                let mut remaining = size;
                while remaining != 0 {
                    let mut bytes = vec![0; remaining.min(READ_CHUNK_BYTES as u64) as usize];
                    file.read_exact(&mut bytes).await?;
                    remaining -= bytes.len() as u64;
                    if body_tx.send(bytes).await.is_err() {
                        return Err(FetchCancelled.into());
                    }
                }
                Ok(())
            } => result,
        };
        let _ = completion_tx.send(result);
    });
    Ok(StreamingRawResponse::new_with_bounded_body(
        head,
        body_rx,
        cancel,
        completion_rx,
    ))
}

fn open_file(path: &Path, max_bytes: Option<u64>) -> Result<(File, u64)> {
    // Reject devices, directories and pipes before opening. Use metadata from
    // the opened handle for the body length.
    ensure!(
        path.metadata()?.is_file(),
        "File URL requires a regular file"
    );
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "File URL requires a regular file");
    let size = metadata.len();
    if let Some(limit) = max_bytes {
        ensure!(size <= limit, "File exceeds response size limit");
    }
    Ok((file, size))
}

fn file_head(url: Url, path: &Path, size: u64) -> ResponseHead {
    let mut headers = vec![("Content-Length".to_owned(), size.to_string().into_bytes())];
    let mime = moli_web_mime::selected_file_mime_type(&path.to_string_lossy());
    if !mime.is_empty() {
        headers.push(("Content-Type".to_owned(), mime.as_bytes().to_vec()));
    }
    ResponseHead {
        final_url: url,
        status: 200,
        status_text: None,
        headers,
        request_cookie_report: None,
        cookie_set_reports: Vec::new(),
        redirected: false,
        redirect_chain: Vec::new(),
        from_cache: false,
        cache_state: Default::default(),
        negotiated_http_version: None,
    }
}
