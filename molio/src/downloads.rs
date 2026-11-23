use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    fs::{self, File},
    io,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use moli_core::{
    download::{
        DownloadBody, content_length_from_headers, download_request, filename_from_headers,
        filename_from_url, sanitize_filename,
    },
    network::ResourceRequestClient,
    page::RendererPendingDownloadActivation,
};
use moli_fetch::FetchCancelHandle;
use serde::{Deserialize, Serialize};
use tempfile::TempPath;
use tokio::sync::{mpsc, watch};
use url::Url;

#[derive(Clone, Serialize, Deserialize)]
pub struct Download {
    #[serde(skip)]
    pub id: u64,
    pub name: String,
    pub source: String,
    pub received: u64,
    pub total: Option<u64>,
    pub state: DownloadState,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum DownloadState {
    Downloading,
    Complete(PathBuf),
    Canceled,
    Failed(String),
}

impl Download {
    pub fn is_active(&self) -> bool {
        matches!(self.state, DownloadState::Downloading)
    }
}

#[derive(Clone, Copy)]
pub enum DownloadCommand {
    Cancel(u64),
    Remove(u64),
    Clear,
}

#[derive(Clone)]
pub struct DownloadClient {
    pub commands: mpsc::UnboundedSender<DownloadCommand>,
    pub entries: watch::Receiver<Vec<Download>>,
    pub directory: PathBuf,
}

impl DownloadClient {
    pub fn channel(
        directory: PathBuf,
    ) -> (
        Self,
        mpsc::UnboundedReceiver<DownloadCommand>,
        watch::Sender<Vec<Download>>,
    ) {
        let (commands, receiver) = mpsc::unbounded_channel();
        let (entries, updates) = watch::channel(Vec::new());
        (
            Self {
                commands,
                entries: updates,
                directory,
            },
            receiver,
            entries,
        )
    }

    pub fn send(&self, command: DownloadCommand) {
        let _ = self.commands.send(command);
    }
}

pub struct Downloads {
    entries: watch::Sender<Vec<Download>>,
    directory: PathBuf,
    history: Option<PathBuf>,
    next_id: Cell<u64>,
    active: RefCell<HashMap<u64, FetchCancelHandle>>,
    loader: ResourceRequestClient,
}

impl Downloads {
    pub fn new(
        loader: ResourceRequestClient,
        entries: watch::Sender<Vec<Download>>,
        directory: PathBuf,
        profile: Option<&Path>,
    ) -> Rc<Self> {
        let history = profile.map(|profile| profile.join("download-history.json"));
        let mut restored: Vec<Download> = history.as_deref().map(load_history).unwrap_or_default();
        restored.retain(|entry| !entry.is_active());
        for (id, entry) in restored.iter_mut().enumerate() {
            entry.id = id as u64;
        }
        let next_id = restored.len() as u64;
        entries.send_replace(restored);
        Rc::new(Self {
            entries,
            directory,
            history,
            next_id: Cell::new(next_id),
            active: RefCell::new(HashMap::new()),
            loader,
        })
    }

    pub fn start(self: &Rc<Self>, activation: RendererPendingDownloadActivation) {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let url = Url::parse(&activation.url).ok();
        let name = activation
            .suggested_filename
            .as_deref()
            .filter(|name| !name.trim().is_empty())
            .map(sanitize_filename)
            .or_else(|| url.as_ref().and_then(filename_from_url))
            .unwrap_or_else(|| "download".to_owned());
        let source = url.as_ref().map(download_source).unwrap_or_default();
        let cancel = FetchCancelHandle::new();
        self.active.borrow_mut().insert(id, cancel.clone());
        self.entries.send_modify(|entries| {
            entries.insert(
                0,
                Download {
                    id,
                    name,
                    source,
                    received: 0,
                    total: None,
                    state: DownloadState::Downloading,
                },
            )
        });
        let downloads = Rc::clone(self);
        tokio::task::spawn_local(async move {
            let result = downloads.transfer(id, activation, &cancel).await;
            let canceled = cancel.is_cancelled();
            downloads.active.borrow_mut().remove(&id);
            downloads.update(id, |entry| {
                entry.state = match result {
                    Ok((path, bytes)) => {
                        entry.name = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        entry.received = bytes;
                        entry.total = Some(bytes);
                        DownloadState::Complete(path)
                    }
                    Err(_) if canceled => DownloadState::Canceled,
                    Err(error) => DownloadState::Failed(format!("{error:#}")),
                };
            });
            downloads.persist();
        });
    }

    pub fn command(&self, command: DownloadCommand) {
        if let DownloadCommand::Cancel(id) = command {
            if let Some(cancel) = self.active.borrow().get(&id) {
                cancel.cancel();
            }
            return;
        }
        self.entries.send_modify(|entries| {
            entries.retain(|entry| {
                entry.is_active()
                    || match command {
                        DownloadCommand::Remove(id) => entry.id != id,
                        DownloadCommand::Clear => false,
                        DownloadCommand::Cancel(_) => unreachable!(),
                    }
            });
        });
        self.persist();
    }

    async fn transfer(
        &self,
        id: u64,
        activation: RendererPendingDownloadActivation,
        cancel: &FetchCancelHandle,
    ) -> Result<(PathBuf, u64)> {
        let (url, status, headers, body) = if let Some(response) = activation.response {
            (
                Url::parse(&response.final_url)?,
                response.status,
                response.headers,
                response.body.into(),
            )
        } else {
            let request = download_request(&activation.url, activation.initiator_url.as_deref())?;
            ensure!(
                matches!(request.url.scheme(), "http" | "https"),
                "Download source is unavailable"
            );
            let transport_cancel = FetchCancelHandle::new();
            let stream = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    transport_cancel.cancel();
                    anyhow::bail!("Download canceled");
                }
                response = self.loader.fetch_raw_stream_with_cancel(request, transport_cancel.clone()) => response?,
            };
            (
                stream.final_url.clone(),
                stream.status,
                stream.headers.clone(),
                DownloadBody::Streaming(Box::new(stream)),
            )
        };
        ensure!(
            (200..300).contains(&status),
            "Server returned HTTP {status}"
        );
        let name = filename_from_headers(&headers)
            .or_else(|| {
                activation
                    .suggested_filename
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
                    .map(sanitize_filename)
            })
            .or_else(|| filename_from_url(&url))
            .unwrap_or_else(|| "download".to_owned());
        let total = body.known_size().or_else(|| {
            (!headers
                .iter()
                .any(|(name, _)| name.eq_ignore_ascii_case("content-encoding")))
            .then(|| content_length_from_headers(&headers))
            .flatten()
        });
        self.update(id, |entry| {
            entry.name.clone_from(&name);
            entry.source = download_source(&url);
            entry.total = total;
        });
        let directory = self.directory.clone();
        let temporary = tokio::task::spawn_blocking(move || {
            fs::create_dir_all(&directory).context("Create the download folder")?;
            tempfile::Builder::new()
                .prefix(".molio-")
                .suffix(".crdownload")
                .tempfile_in(directory)
                .context("Create the temporary download file")
        })
        .await??;
        let (file, temp_path) = temporary.into_parts();
        let mut writer = tokio::fs::File::from_std(file);
        let transport_cancel = body.cancellation_handle();
        let received = Cell::new(0);
        let result = {
            let transfer = body.write_to(&mut writer, |bytes| received.set(bytes));
            tokio::pin!(transfer);
            let mut progress = tokio::time::interval(Duration::from_millis(125));
            progress.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut reported = 0;
            loop {
                tokio::select! {
                    biased;
                    () = cancel.cancelled() => break Err(anyhow::anyhow!("Download canceled")),
                    result = &mut transfer => break result,
                    _ = progress.tick() => {
                        let bytes = received.get();
                        if bytes != reported {
                            self.update(id, |entry| entry.received = bytes);
                            reported = bytes;
                        }
                    }
                }
            }
        };
        if result.is_err() {
            transport_cancel.cancel();
            use tokio::io::AsyncWriteExt as _;
            let _ = writer.flush().await;
            self.update(id, |entry| entry.received = received.get());
        }
        let bytes = result?;
        drop(writer);
        ensure!(!cancel.is_cancelled(), "Download canceled");
        let directory = self.directory.clone();
        let path = tokio::task::spawn_blocking(move || persist_file(temp_path, &directory, &name))
            .await??;
        Ok((path, bytes))
    }

    fn update(&self, id: u64, change: impl FnOnce(&mut Download)) {
        self.entries.send_if_modified(|entries| {
            let Some(entry) = entries.iter_mut().find(|entry| entry.id == id) else {
                return false;
            };
            change(entry);
            true
        });
    }

    fn persist(&self) {
        let Some(path) = &self.history else { return };
        let entries = self.entries.borrow();
        let history: Vec<_> = entries.iter().filter(|entry| !entry.is_active()).collect();
        if let Err(error) = write_history(path, &history) {
            eprintln!("Could not save download history: {error:#}");
        }
    }
}

fn download_source(url: &Url) -> String {
    match url.origin().ascii_serialization().as_str() {
        "null" => match url.scheme() {
            "blob" | "data" => "Generated by the page".to_owned(),
            _ => String::new(),
        },
        origin => origin.to_owned(),
    }
}

fn persist_file(mut temp: TempPath, directory: &Path, name: &str) -> Result<PathBuf> {
    let file = Path::new(name);
    let stem = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name);
    let extension = file
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{ext}"))
        .unwrap_or_default();
    for number in 0.. {
        let path = if number == 0 {
            directory.join(name)
        } else {
            let suffix = format!(" ({number})");
            let limit = 255usize.saturating_sub(suffix.len() + extension.len());
            let stem = &stem[..stem.floor_char_boundary(limit)];
            directory.join(format!("{stem}{suffix}{extension}"))
        };
        match temp.persist_noclobber(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => temp = error.path,
            Err(error) => return Err(error.error).context("Save the finished download"),
        }
    }
    unreachable!()
}

fn load_history(path: &Path) -> Vec<Download> {
    let result = File::open(path)
        .map_err(anyhow::Error::from)
        .and_then(|file| Ok(serde_json::from_reader(file)?));
    match result {
        Ok(entries) => entries,
        Err(error) => {
            if !error
                .downcast_ref::<io::Error>()
                .is_some_and(|error| error.kind() == io::ErrorKind::NotFound)
            {
                eprintln!("Could not load download history: {error:#}");
            }
            Vec::new()
        }
    }
}

fn write_history(path: &Path, entries: &[&Download]) -> Result<()> {
    let directory = path
        .parent()
        .context("Download history has no parent folder")?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(temporary.as_file_mut(), entries)?;
    temporary.persist(path)?;
    Ok(())
}
