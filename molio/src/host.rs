use std::{
    cell::RefCell, fs, future::pending, path::PathBuf, rc::Rc, sync::Arc, thread, time::Duration,
};

use anyhow::{Context as _, Result};
use moli_core::{
    LayoutPolicy, OptionalResourceFetchMask, RendererDocumentLifecycleIdentity, RendererOutputItem,
    RendererOutputTransportMessage, RendererOwnerAction, RendererViewportEdit,
    RendererViewportEditor, RendererViewportNavigation, RendererViewportWheelInput,
    page::{
        BufferedViewportFrames, CompletedPageCommand, Page, PageInputExt, RendererAuxiliaryWindow,
        RendererDraggedFile, RendererInputDispatchOutcome, RendererPageWindowIdentity,
        RendererPointerEventProperties, RendererPopupActivationParts, ViewportBuffers,
        ViewportOutput, ViewportUpdate,
    },
    protocol_types::{DocumentActivity, ViewportSurface},
    runtime::{Browser, BrowserConfig},
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::{JoinHandle, LocalSet},
};

use crate::downloads::{DownloadClient, Downloads};

pub struct BrowserHost {
    tabs: mpsc::UnboundedSender<OpenTab>,
    pub downloads: DownloadClient,
    worker: RefCell<Option<(oneshot::Sender<()>, thread::JoinHandle<()>)>>,
}

pub enum BrowserEvent {
    Popup(Box<RendererPopupActivationParts>),
    ClosePopup(RendererAuxiliaryWindow),
}

pub struct TabConnection {
    pub commands: mpsc::UnboundedSender<TabCommand>,
    pub state: watch::Receiver<TabState>,
    pub viewport: Arc<ViewportOutput>,
    pub window_identity: RendererPageWindowIdentity,
}

#[derive(Clone, Default)]
pub struct TabState {
    pub navigation: Option<RendererViewportNavigation>,
    pub document: Option<RendererDocumentLifecycleIdentity>,
    pub favicon: Option<Arc<gpui::RenderImage>>,
    pub pending_url: Option<String>,
    pub loading: bool,
    pub error: Option<String>,
    pub editor: Option<RendererViewportEditor>,
    pub editor_ack: Option<u64>,
}

pub enum TabCommand {
    Navigate(Option<String>),
    History(i64),
    Stop,
    Viewport(ViewportSurface, u64),
    Activity(DocumentActivity),
    Pointer(PointerInput),
    Wheel(RendererViewportWheelInput),
    Key(KeyInput),
    InsertText(String),
    Edit(RendererViewportEdit),
    Copy {
        document: RendererDocumentLifecycleIdentity,
        reply: oneshot::Sender<Option<String>>,
    },
    DropFiles {
        document: RendererDocumentLifecycleIdentity,
        position: (f64, f64),
        paths: Vec<PathBuf>,
        modifiers: u8,
    },
}

impl TabCommand {
    fn superseded_by(&self, next: &Self) -> bool {
        match (self, next) {
            (Self::Viewport(..), Self::Viewport(..)) => true,
            (Self::Pointer(previous), Self::Pointer(next)) => {
                previous.event == "mousemove"
                    && next.event == "mousemove"
                    && previous.buttons == 0
                    && next.buttons == 0
                    && previous.modifiers == next.modifiers
            }
            _ => false,
        }
    }
}

pub struct PointerInput {
    pub event: &'static str,
    pub position: (f64, f64),
    pub button: i32,
    pub buttons: i32,
    pub click_count: i32,
    pub delta: (f64, f64),
    pub modifiers: u8,
}

pub struct KeyInput {
    pub event: &'static str,
    pub key: String,
    pub code: String,
    pub modifiers: u8,
    pub repeat: bool,
}

struct OpenTab {
    commands: mpsc::UnboundedReceiver<TabCommand>,
    state: watch::Sender<TabState>,
    viewport: Arc<ViewportOutput>,
    window_identity: RendererPageWindowIdentity,
    popup: Option<Box<RendererPopupActivationParts>>,
}

impl BrowserHost {
    pub fn start(
        profile: Option<PathBuf>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<BrowserEvent>)> {
        let directory = dirs::download_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join("Downloads")))
            .unwrap_or_else(|| std::env::temp_dir().join("molio-downloads"));
        Self::start_with_download_directory(profile, directory)
    }

    fn start_with_download_directory(
        profile: Option<PathBuf>,
        directory: PathBuf,
    ) -> Result<(Self, mpsc::UnboundedReceiver<BrowserEvent>)> {
        let (sender, mut requests) = mpsc::unbounded_channel::<OpenTab>();
        let (events, event_rx) = mpsc::unbounded_channel();
        let (client, mut download_commands, download_entries) =
            DownloadClient::channel(directory.clone());
        let (shutdown, mut stopping) = oneshot::channel();
        let worker = thread::Builder::new()
            .name("molio-host".into())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build();
                let runtime = match result {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        eprintln!("Could not start browser host: {error}");
                        return;
                    }
                };
                let tabs = LocalSet::new();
                tabs.block_on(&runtime, async move {
                    let mut config = BrowserConfig::default()
                        .with_layout_policy(LayoutPolicy::Realtime)
                        .with_optional_resource_fetch_mask(
                            OptionalResourceFetchMask::IMAGE | OptionalResourceFetchMask::FONT,
                        );
                    config.set_profile_dir(profile.clone());
                    let browser = match Browser::new(config) {
                        Ok(browser) => browser,
                        Err(error) => {
                            loop {
                                tokio::select! {
                                    _ = &mut stopping => break,
                                    tab = requests.recv() => {
                                        let Some(tab) = tab else { break };
                                        tab.state.send_modify(|state| state.error = Some(error.to_string()));
                                    }
                                }
                            }
                            return;
                        }
                    };
                    let mut renderer_output = browser.subscribe_viewport_actions();
                    let downloads = Downloads::new(
                        browser.resource_request_client(), download_entries, directory, profile.as_deref(),
                    );
                    loop {
                        tokio::select! {
                            _ = &mut stopping => break,
                            tab = requests.recv() => {
                                let Some(tab) = tab else { break };
                                tokio::task::spawn_local(run_tab(browser.clone(), Rc::clone(&downloads), tab));
                            }
                            action = download_commands.recv(), if !download_commands.is_closed() => {
                                if let Some(action) = action { downloads.command(action); }
                            }
                            message = renderer_output.recv(), if !renderer_output.is_closed() => {
                                if let Some(RendererOutputTransportMessage::Publication(output)) = message {
                                    for record in output.into_records() {
                                        match record.into_parts().1 {
                                            RendererOutputItem::OwnerAction(RendererOwnerAction::Download(activation)) => {
                                                downloads.start(activation);
                                            }
                                            RendererOutputItem::OwnerAction(RendererOwnerAction::Popup(activation)) => {
                                                let _ = events.send(BrowserEvent::Popup(Box::new(activation.into_parts())));
                                            }
                                            RendererOutputItem::OwnerAction(RendererOwnerAction::CloseAuxiliaryWindow(window)) => {
                                                let _ = events.send(BrowserEvent::ClosePopup(window));
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                    drop(renderer_output);
                    download_commands.close();
                    while let Ok(command) = download_commands.try_recv() {
                        downloads.command(command);
                    }
                    downloads.shutdown().await;
                });
            })
            .context("start browser host thread")?;
        Ok((Self {
            tabs: sender,
            downloads: client,
            worker: RefCell::new(Some((shutdown, worker))),
        }, event_rx))
    }

    pub fn shutdown(&self) {
        let worker = self.worker.borrow_mut().take();
        if let Some((shutdown, worker)) = worker {
            let _ = shutdown.send(());
            if worker.join().is_err() {
                eprintln!("Browser host failed while shutting down");
            }
        }
    }

    pub fn open_tab(&self) -> Result<TabConnection> {
        self.open_viewport(RendererPageWindowIdentity::default(), None)
    }

    pub fn open_popup(&self, popup: Box<RendererPopupActivationParts>) -> Result<TabConnection> {
        let name = popup
            .browsing_context_name
            .clone()
            .context("popup has no Window identity")?;
        let identity = RendererPageWindowIdentity::new(name, popup.auxiliary_window.clone());
        self.open_viewport(identity, Some(popup))
    }

    fn open_viewport(
        &self,
        window_identity: RendererPageWindowIdentity,
        popup: Option<Box<RendererPopupActivationParts>>,
    ) -> Result<TabConnection> {
        let (commands, request_rx) = mpsc::unbounded_channel();
        let (state_tx, state) = watch::channel(TabState::default());
        let viewport = Arc::new(ViewportOutput::default());
        self.tabs
            .send(OpenTab {
                commands: request_rx,
                state: state_tx,
                viewport: viewport.clone(),
                window_identity: window_identity.clone(),
                popup,
            })
            .map_err(|_| anyhow::anyhow!("browser host is unavailable"))?;
        Ok(TabConnection {
            commands,
            state,
            viewport,
            window_identity,
        })
    }
}

impl Drop for BrowserHost {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn next_viewport_update(
    stream: &mut Option<BufferedViewportFrames>,
) -> Option<ViewportUpdate> {
    match stream {
        Some(stream) => stream.changed().await.ok(),
        None => pending().await,
    }
}

type PendingNavigation = JoinHandle<Result<CompletedPageCommand>>;

async fn navigation_finished(task: &mut Option<PendingNavigation>) -> Result<CompletedPageCommand> {
    match task {
        Some(task) => task.await?,
        None => pending().await,
    }
}

fn start_favicon_load(
    browser: &Browser,
    navigation: &RendererViewportNavigation,
) -> Option<JoinHandle<Option<Arc<gpui::RenderImage>>>> {
    if navigation.favicon_urls.is_empty() {
        return None;
    }
    let browser = browser.clone();
    let navigation = navigation.clone();
    Some(tokio::task::spawn_local(async move {
        crate::favicon::load(&browser, &navigation).await
    }))
}

async fn run_tab(browser: Browser, downloads: Rc<Downloads>, mut tab: OpenTab) {
    let initial = tokio::time::timeout(
        Duration::from_secs(20),
        browser.open_viewport_page(tab.window_identity, tab.popup.as_deref()),
    )
    .await
    .context("opening the tab timed out")
    .and_then(|result| result);
    let mut page = match initial {
        Ok(page) => page,
        Err(error) => {
            tab.state
                .send_modify(|state| state.error = Some(format!("{error:#}")));
            return;
        }
    };
    let mut surface = None;
    let mut capture = None;
    let mut navigation = None;
    let mut favicon_task: Option<JoinHandle<Option<Arc<gpui::RenderImage>>>> = None;
    let mut queued = None;
    let mut activity = DocumentActivity::new(false, false);
    let _ = page.set_document_activity_async(activity).await;
    let mut navigation_updates = match page.subscribe_navigation_state().await {
        Ok(updates) => updates,
        Err(error) => {
            tab.state
                .send_modify(|state| state.error = Some(format!("{error:#}")));
            return;
        }
    };
    navigation_updates.mark_changed();
    if let Some(popup) = tab.popup.take()
        && popup.navigation_requested
        && popup.url != page.final_url().as_str()
        && !tab.commands.is_closed()
    {
        match start_navigation(&page, &tab.state, Some(popup.url)) {
            Ok(task) => navigation = Some(task),
            Err(error) => tab
                .state
                .send_modify(|state| state.error = Some(format!("{error:#}"))),
        }
    }

    loop {
        tokio::select! {
            Ok(()) = navigation_updates.changed() => {
                let Some(next) = navigation_updates.borrow_and_update().clone() else { continue };
                tab.state.send_modify(|state| {
                    let document_changed = state.document != Some(next.document);
                    if document_changed
                        || state.navigation.as_ref().map(|nav| &nav.favicon_urls)
                            != Some(&next.navigation.favicon_urls)
                    {
                        if let Some(task) = favicon_task.take() {
                            task.abort();
                        }
                        state.favicon = None;
                        if activity.visible {
                            favicon_task = start_favicon_load(&browser, &next.navigation);
                        }
                    }
                    if document_changed {
                        state.editor = capture
                            .as_mut()
                            .and_then(BufferedViewportFrames::latest_state)
                            .filter(|frame| frame.document == next.document && activity.focused)
                            .and_then(|frame| frame.editor);
                    }
                    state.document = Some(next.document);
                    state.navigation = Some(next.navigation);
                });
            }
            result = navigation_finished(&mut navigation) => {
                navigation = None;
                let result = result.and_then(|completion| page.finish_viewport_navigation(completion));
                tab.state.send_modify(|state| {
                    state.loading = false;
                    if let Err(error) = result {
                        state.error = Some(format!("{error:#}"));
                    } else {
                        state.pending_url = None;
                    }
                });
            }
            icon = async {
                match favicon_task.as_mut() {
                    Some(task) => task.await.ok().flatten(),
                    None => pending().await,
                }
            } => {
                favicon_task = None;
                if let Some(icon) = icon {
                    tab.state.send_modify(|state| state.favicon = Some(icon));
                }
            }
            event = next_viewport_update(&mut capture) => match event {
                Some(ViewportUpdate::StateChanged) => {
                    let Some(frame) = capture.as_mut().and_then(BufferedViewportFrames::latest_state) else { continue };
                    let editor = frame.editor.filter(|_| activity.focused);
                    tab.state.send_if_modified(|state| {
                        if state.document != Some(frame.document) || state.editor == editor {
                            return false;
                        }
                        state.editor = editor;
                        true
                    });
                }
                Some(ViewportUpdate::ResourceChanged) => { let _ = page.wake_viewport_frames().await; }
                None => { capture = None; }
            },
            command = async {
                match queued.take() {
                    Some(command) => Some(command),
                    None => tab.commands.recv().await,
                }
            } => {
                let Some(mut command) = command else { break };
                while let Ok(next) = tab.commands.try_recv() {
                    if command.superseded_by(&next) {
                        command = next;
                    } else {
                        queued = Some(next);
                        break;
                    }
                }
                let result = match command {
                    TabCommand::Navigate(target) => {
                        if let Some(previous) = navigation.take() {
                            previous.abort();
                        }
                        start_navigation(&page, &tab.state, target)
                            .map(|task| navigation = Some(task))
                    }
                    TabCommand::History(delta) => {
                        tab.state.send_modify(|state| state.error = None);
                        page.traverse_viewport_history(delta).await.map(drop)
                    }
                    TabCommand::Stop => {
                        if let Some(navigation) = navigation.take() {
                            navigation.abort();
                        }
                        tab.state.send_modify(|state| {
                            state.loading = false;
                            state.pending_url = None;
                        });
                        page.stop_document_lifecycle_async().await
                    }
                    TabCommand::Viewport(viewport, revision) => {
                        surface = Some((viewport, revision));
                        if activity.visible {
                            refresh_capture(&mut page, &mut capture, surface, &tab.viewport).await
                        } else {
                            Ok(())
                        }
                    }
                    TabCommand::Activity(next) => {
                        let visibility_changed = activity.visible != next.visible;
                        activity = next;
                        if !next.visible {
                            capture = None;
                        }
                        if !next.focused {
                            tab.state.send_if_modified(|state| state.editor.take().is_some());
                        }
                        if let Err(error) = page.set_document_activity_async(next).await {
                            Err(error)
                        } else if visibility_changed && next.visible {
                            {
                                let state = tab.state.borrow();
                                if state.favicon.is_none() && favicon_task.is_none() {
                                    favicon_task = state.navigation.as_ref().and_then(|navigation| {
                                        start_favicon_load(&browser, navigation)
                                    });
                                }
                            }
                            refresh_capture(&mut page, &mut capture, surface, &tab.viewport).await
                        } else if next.focused {
                            refresh_editor(&mut page, &tab.state).await
                        } else {
                            Ok(())
                        }
                    }
                    TabCommand::Pointer(input) if activity.visible => {
                        let refresh = activity.focused && input.event != "mousemove";
                        let result = dispatch_pointer(&mut page, input).await.map(|outcome| {
                            if let Some(activation) = outcome.pending_download {
                                downloads.start(activation);
                            }
                        });
                        if refresh && result.is_ok() { refresh_editor(&mut page, &tab.state).await } else { result }
                    }
                    TabCommand::Wheel(input) if activity.visible => {
                        page.enqueue_viewport_wheel(input)
                    }
                    TabCommand::Key(input) if activity.focused => {
                        let enter = input.event == "keydown"
                            && input.key == "Enter"
                            && input.modifiers & 0b111 == 0;
                        let result = page.dispatch_key_event_with_outcome_async(
                            input.event, &input.key, &input.code, if enter { "\r" } else { "" },
                            input.modifiers, input.repeat, enter,
                        ).await.map(|outcome| {
                            if let Some(activation) = outcome.pending_download {
                                downloads.start(activation);
                            }
                        });
                        if result.is_ok() && input.event == "keydown" { refresh_editor(&mut page, &tab.state).await } else { result }
                    }
                    TabCommand::InsertText(text) if activity.focused => {
                        match page.insert_text_into_active_control_async(&text).await {
                            Ok(_) => refresh_editor(&mut page, &tab.state).await,
                            Err(error) => Err(error),
                        }
                    }
                    TabCommand::Edit(edit) => {
                        let sequence = edit.sequence;
                        match page.edit_viewport(edit).await {
                            Ok(reply) => {
                                tab.state.send_modify(|state| {
                                    state.editor_ack = Some(sequence);
                                    state.editor = reply.editor;
                                });
                                Ok(())
                            }
                            Err(error) => {
                                tab.state.send_modify(|state| {
                                    state.editor_ack = Some(sequence);
                                    state.editor = None;
                                });
                                Err(error)
                            }
                        }
                    }
                    TabCommand::Copy { document, reply } => {
                        match page.viewport_selected_text(document).await {
                            Ok(text) => { let _ = reply.send(text); Ok(()) }
                            Err(error) => Err(error),
                        }
                    }
                    TabCommand::DropFiles { document, position, paths, modifiers } if activity.visible => {
                        drop_files(&mut page, document, position, paths, modifiers).await
                    }
                    _ => Ok(()),
                };
                if let Err(error) = result {
                    eprintln!("Browser command failed: {error:#}");
                }
            }
        }
    }
    if let Some(task) = navigation {
        task.abort();
    }
    if let Some(task) = favicon_task {
        task.abort();
    }
}

fn start_navigation(
    page: &Page,
    state: &watch::Sender<TabState>,
    target: Option<String>,
) -> Result<PendingNavigation> {
    let pending = page.start_viewport_navigation(target.clone())?;
    state.send_modify(|state| {
        state.loading = true;
        state.error = None;
        state.pending_url = target;
    });
    Ok(tokio::task::spawn_local(pending.wait()))
}

async fn drop_files(
    page: &mut Page,
    document: RendererDocumentLifecycleIdentity,
    position: (f64, f64),
    paths: Vec<PathBuf>,
    modifiers: u8,
) -> Result<()> {
    let files = tokio::task::spawn_blocking(move || {
        paths
            .into_iter()
            .map(|path| {
                let name = path.file_name().context("drop has no file name")?;
                let name = name.to_string_lossy().into_owned();
                let metadata = fs::metadata(&path).context("read dropped file metadata")?;
                anyhow::ensure!(metadata.is_file(), "the dropped item is not a regular file");
                let mime_type = moli_web_mime::selected_file_mime_type(&name).to_owned();
                let last_modified = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0., |duration| duration.as_millis() as f64);
                Ok(RendererDraggedFile {
                    name,
                    mime_type,
                    bytes: fs::read(path).context("read dropped file")?,
                    last_modified,
                })
            })
            .collect::<Result<Vec<_>>>()
    })
    .await??;
    page.drop_viewport_files(document, position, files, modifiers)
        .await?;
    Ok(())
}

async fn refresh_editor(page: &mut Page, state: &watch::Sender<TabState>) -> Result<()> {
    let editor = page.viewport_editor().await?;
    state.send_if_modified(|state| {
        if state.editor == editor {
            return false;
        }
        state.editor = editor;
        true
    });
    Ok(())
}

async fn refresh_capture(
    page: &mut Page,
    capture: &mut Option<BufferedViewportFrames>,
    surface: Option<(ViewportSurface, u64)>,
    output: &Arc<ViewportOutput>,
) -> Result<()> {
    if let Some((surface, revision)) = surface {
        let layout = ViewportBuffers::layout_for(surface)?;
        let size = gpui::size(layout.width.into(), layout.height.into());
        let storage = [
            gpui::BgraImage::reserve_pixels(size)?,
            gpui::BgraImage::reserve_pixels(size)?,
            gpui::BgraImage::reserve_pixels(size)?,
        ];
        let buffers = ViewportBuffers::new(layout, storage)?;
        *capture = Some(
            page.subscribe_viewport_buffer(surface, revision, buffers, output.clone())
                .await?,
        );
    }
    Ok(())
}

async fn dispatch_pointer(
    page: &mut Page,
    input: PointerInput,
) -> Result<RendererInputDispatchOutcome> {
    let pending = page.start_dispatch_mouse_event_at_point_with_pointer_outcome(
        input.position.0,
        input.position.1,
        input.event,
        input.button,
        Some(input.buttons),
        input.click_count,
        input.delta.0,
        input.delta.1,
        RendererPointerEventProperties::default(),
        input.modifiers,
        false,
    )?;
    page.finish_dispatch_mouse_event_at_point_with_outcome(pending.wait().await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloads::{Download, DownloadCommand, DownloadState};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    async fn read_http_request(
        socket: &mut BufReader<tokio::net::TcpStream>,
    ) -> Result<(String, Option<String>)> {
        let mut line = String::new();
        socket.read_line(&mut line).await?;
        let path = line
            .split_whitespace()
            .nth(1)
            .context("HTTP request target")?
            .to_owned();
        let mut referer = None;
        loop {
            line.clear();
            if socket.read_line(&mut line).await? == 0 || line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("referer")
            {
                referer = Some(value.trim().to_owned());
            }
        }
        Ok((path, referer))
    }

    async fn download_matching(
        entries: &mut watch::Receiver<Vec<Download>>,
        matches: impl Fn(&Download) -> bool,
    ) -> Result<Download> {
        let entries = tokio::time::timeout(
            Duration::from_secs(10),
            entries.wait_for(|entries| entries.iter().any(&matches)),
        )
        .await??;
        entries
            .iter()
            .find(|entry| matches(entry))
            .cloned()
            .context("download entry")
    }

    fn navigate(tab: &TabConnection, url: impl Into<String>) -> Result<()> {
        tab.commands
            .send(TabCommand::Navigate(Some(url.into())))
            .map_err(|_| anyhow::anyhow!("navigate to download fixture"))
    }

    #[tokio::test]
    async fn large_data_download_keeps_cross_tab_download_delivery_available() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (host, _events) =
            BrowserHost::start_with_download_directory(None, directory.path().to_owned())?;
        let mut entries = host.downloads.entries.clone();
        let data_tab = host.open_tab()?;
        navigate(
            &data_tab,
            "data:text/html,<body><script>const link=document.createElement('a');link.download='large.bin';link.href='data:application/octet-stream;base64,'+'A'.repeat(16*1024*1024);document.body.append(link);link.click()</script>",
        )?;
        let large = download_matching(&mut entries, |entry| {
            entry.name == "large.bin" && !entry.is_active()
        })
        .await
        .context("large data download")?;
        let DownloadState::Complete(path) = large.state else {
            anyhow::bail!("large data download did not complete");
        };
        let contents = fs::read(path)?;
        assert_eq!(contents.len(), 12 * 1024 * 1024);
        assert!(contents.iter().all(|byte| *byte == 0));
        drop(data_tab);

        let blob_tab = host.open_tab()?;
        navigate(
            &blob_tab,
            "data:text/html,<body><script>const link=document.createElement('a');link.download='small.txt';link.href=URL.createObjectURL(new Blob(['small']));document.body.append(link);link.click();URL.revokeObjectURL(link.href)</script>",
        )?;
        let small = download_matching(&mut entries, |entry| {
            entry.name == "small.txt" && !entry.is_active()
        })
        .await
        .context("Blob download in a different tab")?;
        let DownloadState::Complete(path) = small.state else {
            anyhow::bail!("Blob download after large data download did not complete");
        };
        assert_eq!(fs::read(path)?, b"small");
        Ok(())
    }

    #[tokio::test]
    async fn http_downloads_use_the_effective_link_referrer_policy() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (host, _events) =
            BrowserHost::start_with_download_directory(None, directory.path().to_owned())?;
        let mut entries = host.downloads.entries.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let (requests, mut captures) = mpsc::unbounded_channel();
        let server = tokio::spawn(async move {
            for _ in 0..6 {
                let (socket, _) = listener.accept().await?;
                let mut socket = BufReader::new(socket);
                let (path, referer) = read_http_request(&mut socket).await?;
                let body = match path.as_str() {
                    "/document?token=secret" => {
                        r#"<meta name="referrer" content="no-referrer"><a id="d" download="document.txt" href="/export-document"></a><script>d.click()</script>"#
                    }
                    "/link?token=secret" => {
                        r#"<meta name="referrer" content="unsafe-url"><a id="d" referrerpolicy="origin" download="link.txt" href="/export-link"></a><script>d.click()</script>"#
                    }
                    "/relation?token=secret" => {
                        r#"<meta name="referrer" content="unsafe-url"><a id="d" rel="noreferrer" referrerpolicy="unsafe-url" download="relation.txt" href="/export-relation"></a><script>d.click()</script>"#
                    }
                    _ => {
                        assert!(path.starts_with("/export-"), "{path}");
                        requests.send((path, referer))?;
                        "export"
                    }
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await?;
            }
            Ok::<_, anyhow::Error>(())
        });
        for (name, expected) in [
            ("document", None),
            ("link", Some(format!("{origin}/"))),
            ("relation", None),
        ] {
            let tab = host.open_tab()?;
            navigate(&tab, format!("{origin}/{name}?token=secret"))?;
            let (path, referer) = tokio::time::timeout(Duration::from_secs(10), captures.recv())
                .await?
                .context("download request")?;
            assert_eq!(path, format!("/export-{name}"));
            assert_eq!(referer, expected, "{name} download");
            let entry = download_matching(&mut entries, |entry| {
                entry.name == format!("{name}.txt") && !entry.is_active()
            })
            .await?;
            assert!(matches!(entry.state, DownloadState::Complete(_)));
        }
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn background_navigation_download_works_without_a_viewport_subscription() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (host, _events) =
            BrowserHost::start_with_download_directory(None, directory.path().to_owned())?;
        let mut tab = host.open_tab()?;
        let mut entries = host.downloads.entries.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/source", listener.local_addr()?);
        let (started, gate_requested) = oneshot::channel();
        let (release, gate) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await?;
            let mut socket = BufReader::new(socket);
            assert_eq!(read_http_request(&mut socket).await?.0, "/source");
            let body = b"<script>fetch('/release').then(r=>r.text()).then(()=>location.assign('/attachment'))</script>";
            socket.write_all(format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            ).as_bytes()).await?;
            socket.write_all(body).await?;
            drop(socket);

            let (socket, _) = listener.accept().await?;
            let mut socket = BufReader::new(socket);
            assert_eq!(read_http_request(&mut socket).await?.0, "/release");
            started.send(()).expect("notify test of pending fetch");
            gate.await?;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await?;
            drop(socket);

            let (socket, _) = listener.accept().await?;
            let mut socket = BufReader::new(socket);
            assert_eq!(read_http_request(&mut socket).await?.0, "/attachment");
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Disposition: attachment; filename=background.txt\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbody").await?;
            Ok::<_, anyhow::Error>(())
        });
        navigate(&tab, url)?;
        tokio::time::timeout(Duration::from_secs(5), gate_requested).await??;
        tokio::time::timeout(
            Duration::from_secs(5),
            tab.state.wait_for(|state| !state.loading),
        )
        .await??;
        release.send(()).expect("release the background navigation");
        let entry = download_matching(&mut entries, |entry| {
            entry.name == "background.txt" && !entry.is_active()
        })
        .await?;
        let DownloadState::Complete(path) = entry.state else {
            anyhow::bail!("background attachment did not complete");
        };
        assert_eq!(fs::read(path)?, b"body");
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn attachment_download_survives_tab_close_and_blob_does_not_overwrite_it() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (host, _events) =
            BrowserHost::start_with_download_directory(None, directory.path().to_owned())?;
        let tab = host.open_tab()?;
        let mut entries = host.downloads.entries.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let target = format!("http://{}/attachment", listener.local_addr()?);
        let (finish, hold) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut client, _) = listener.accept().await?;
            let mut request = [0; 4096];
            let size = client.read(&mut request).await?;
            assert!(request[..size].starts_with(b"GET /attachment "));
            client.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=report.txt\r\nContent-Length: 7\r\nConnection: close\r\n\r\nnet").await?;
            hold.await?;
            client.write_all(b"work").await?;
            Ok::<_, anyhow::Error>(())
        });
        tab.commands
            .send(TabCommand::Navigate(Some(target)))
            .map_err(|_| anyhow::anyhow!("navigate to attachment"))?;
        let entry = download_matching(&mut entries, |entry| {
            entry.name == "report.txt" && entry.total == Some(7) && entry.received == 3
        })
        .await?;
        assert!(entry.is_active());
        drop(tab);
        finish
            .send(())
            .map_err(|_| anyhow::anyhow!("release response body"))?;
        server.await??;
        let entry = download_matching(&mut entries, |current| {
            current.id == entry.id && !current.is_active()
        })
        .await?;
        assert!(matches!(entry.state, DownloadState::Complete(_)));
        assert_eq!(fs::read(directory.path().join("report.txt"))?, b"network");

        let tab = host.open_tab()?;
        let script = "data:text/html,<body><script>const link=document.createElement('a');link.download='report.txt';link.href=URL.createObjectURL(new Blob(['blob']));document.body.append(link);link.click();URL.revokeObjectURL(link.href)</script>";
        tab.commands
            .send(TabCommand::Navigate(Some(script.to_owned())))
            .map_err(|_| anyhow::anyhow!("navigate to Blob fixture"))?;
        let blob = download_matching(&mut entries, |current| {
            current.id != entry.id && !current.is_active()
        })
        .await?;
        assert!(matches!(blob.state, DownloadState::Complete(_)));
        assert_eq!(blob.name, "report (1).txt");
        assert_eq!(fs::read(directory.path().join("report.txt"))?, b"network");
        assert_eq!(fs::read(directory.path().join("report (1).txt"))?, b"blob");
        Ok(())
    }

    async fn start_stalled_download(
        host: &BrowserHost,
    ) -> Result<(TabConnection, Download, JoinHandle<std::io::Result<usize>>)> {
        let tab = host.open_tab()?;
        let mut entries = host.downloads.entries.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let target = format!("http://{}/slow", listener.local_addr()?);
        let server = tokio::spawn(async move {
            let (mut client, _) = listener.accept().await?;
            let mut request = [0; 4096];
            let _ = client.read(&mut request).await?;
            client.write_all(b"HTTP/1.1 200 OK\r\nContent-Disposition: attachment; filename=slow.dat\r\nContent-Length: 8192\r\nConnection: close\r\n\r\npartial").await?;
            client.read(&mut request).await
        });
        navigate(&tab, target)?;
        let entry = download_matching(&mut entries, |entry| {
            entry.name == "slow.dat" && entry.is_active() && entry.received == 7
        })
        .await?;
        Ok((tab, entry, server))
    }

    #[tokio::test]
    async fn canceling_a_download_removes_the_partial_file() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let (host, _events) =
            BrowserHost::start_with_download_directory(None, directory.path().to_owned())?;
        let (_tab, entry, server) = start_stalled_download(&host).await?;
        let mut entries = host.downloads.entries.clone();
        host.downloads.send(DownloadCommand::Cancel(entry.id));
        let entry = download_matching(&mut entries, |current| {
            current.id == entry.id && !current.is_active()
        })
        .await?;
        assert!(matches!(entry.state, DownloadState::Canceled));
        assert!(fs::read_dir(directory.path())?.next().is_none());
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), server).await???,
            0
        );
        Ok(())
    }

    #[tokio::test]
    async fn quitting_cancels_partial_downloads_and_flushes_the_latest_history() -> Result<()> {
        let root = tempfile::tempdir()?;
        let profile = root.path().join("profile");
        let directory = root.path().join("downloads");
        let (host, _events) =
            BrowserHost::start_with_download_directory(Some(profile.clone()), directory.clone())?;
        let finished_tab = host.open_tab()?;
        let mut entries = host.downloads.entries.clone();
        navigate(
            &finished_tab,
            "data:text/html,<body><script>const link=document.createElement('a');link.download='done.txt';link.href='data:,done';document.body.append(link);link.click()</script>",
        )?;
        let finished = download_matching(&mut entries, |entry| {
            entry.name == "done.txt" && !entry.is_active()
        })
        .await?;
        assert!(matches!(finished.state, DownloadState::Complete(_)));

        let (_tab, _, server) = start_stalled_download(&host).await?;
        host.downloads.send(DownloadCommand::Clear);
        host.shutdown();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(5), server).await???,
            0
        );
        assert_eq!(fs::read(directory.join("done.txt"))?, b"done");
        assert_eq!(fs::read_dir(&directory)?.count(), 1);
        let (restored, _restored_events) =
            BrowserHost::start_with_download_directory(Some(profile), directory)?;
        let mut entries = restored.downloads.entries.clone();
        let canceled = download_matching(&mut entries, |entry| entry.name == "slow.dat").await?;
        assert!(matches!(canceled.state, DownloadState::Canceled));
        assert_eq!(entries.borrow().len(), 1);
        Ok(())
    }

    async fn show_tab(
        tab: &mut TabConnection,
        title: &str,
        revision: u64,
        color: [u8; 4],
    ) -> Result<RendererDocumentLifecycleIdentity> {
        let mut frames = tab.viewport.subscribe();
        tab.commands
            .send(TabCommand::Activity(DocumentActivity::new(true, true)))
            .map_err(|_| anyhow::anyhow!("show tab"))?;
        tab.commands
            .send(TabCommand::Viewport(
                ViewportSurface {
                    inner_width: 40,
                    inner_height: 30,
                    device_pixel_ratio: 2.,
                    ..ViewportSurface::default()
                },
                revision,
            ))
            .map_err(|_| anyhow::anyhow!("resize tab"))?;
        let state = tokio::time::timeout(
            Duration::from_secs(5),
            tab.state.wait_for(|state| {
                state
                    .navigation
                    .as_ref()
                    .is_some_and(|nav| nav.title == title)
            }),
        )
        .await??;
        let document = state.document.context("tab has a committed document")?;
        drop(state);
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(Some(())) = tab.viewport.read(|frame| {
                    if frame.document != document || frame.viewport_revision != revision {
                        return None;
                    }
                    let pixels = frame.pixels?;
                    assert_eq!((frame.layout.width, frame.layout.height), (80, 60));
                    assert_eq!(pixels.len(), 80 * 60 * 4);
                    assert_eq!(pixels[..4], color);
                    Some(())
                }) {
                    break Ok::<_, watch::error::RecvError>(());
                }
                frames.changed().await?;
            }
        })
        .await??;
        Ok(document)
    }

    #[tokio::test]
    async fn tabs_keep_their_documents_and_present_bgra_at_the_requested_scale() -> Result<()> {
        let (host, _events) = BrowserHost::start(None)?;
        let (mut red, mut blue) = (host.open_tab()?, host.open_tab()?);
        red.commands
            .send(TabCommand::Navigate(Some(
                "data:text/html,<title>Red</title><style>html{background:red}</style>".into(),
            )))
            .map_err(|_| anyhow::anyhow!("load red tab"))?;
        blue.commands
            .send(TabCommand::Navigate(Some(
                "data:text/html,<title>Blue</title><style>html{background:blue}</style>".into(),
            )))
            .map_err(|_| anyhow::anyhow!("load blue tab"))?;
        let red_document = show_tab(&mut red, "Red", 1, [0, 0, 255, 255]).await?;
        let mut red_frames = red.viewport.subscribe();
        red.commands
            .send(TabCommand::Activity(DocumentActivity::new(false, false)))
            .map_err(|_| anyhow::anyhow!("hide red tab"))?;
        tokio::time::timeout(Duration::from_secs(5), async {
            while red.viewport.read(|_| ()).is_some() {
                red_frames.changed().await?;
            }
            Ok::<_, watch::error::RecvError>(())
        })
        .await??;
        let blue_document = show_tab(&mut blue, "Blue", 2, [255, 0, 0, 255]).await?;
        assert_ne!(red_document, blue_document);
        drop(blue);
        assert_eq!(
            show_tab(&mut red, "Red", 3, [0, 0, 255, 255]).await?,
            red_document
        );
        Ok(())
    }

    #[tokio::test]
    async fn tabs_update_navigation_and_favicons_without_a_viewport() -> Result<()> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let mut ico = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(16, 16, image::Rgba([225, 40, 19, 255]))
            .write_to(&mut ico, image::ImageFormat::Ico)?;
        let ico = ico.into_inner();
        tokio::spawn(async move {
            while let Ok((connection, _)) = listener.accept().await {
                let mut connection = BufReader::new(connection);
                let mut request_line = String::new();
                if connection.read_line(&mut request_line).await.unwrap_or(0) == 0 {
                    continue;
                }
                let mut header = String::new();
                while connection.read_line(&mut header).await.unwrap_or(0) > 0 && header != "\r\n" {
                    header.clear();
                }
                let (mime, body): (&str, &[u8]) = match request_line.split_whitespace().nth(1) {
                    Some("/page") => (
                        "text/html",
                        br#"<title>Declared</title><base href="/assets/">
                        <link rel="shortcut ICON" href="logo.svg">
                        <link rel="icon" href="broken.svg">"#,
                    ),
                    Some("/assets/logo.svg") => (
                        "image/svg+xml",
                        br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16">
                        <rect width="16" height="16" fill="#12a0e0"/></svg>"##,
                    ),
                    Some("/next") => ("text/html", b"<title>Default</title>"),
                    Some("/favicon.ico") => ("image/x-icon", &ico),
                    _ => ("text/html", b"<p>Invalid icon</p>"),
                };
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len(),
                );
                let connection = connection.get_mut();
                if connection.write_all(head.as_bytes()).await.is_ok() {
                    let _ = connection.write_all(body).await;
                }
            }
        });

        let (host, _events) = BrowserHost::start(None)?;
        let mut tab = host.open_tab()?;

        for (path, title, pixel) in [
            ("/page", "Declared", [224, 160, 18, 255]),
            ("/next", "Default", [19, 40, 225, 255]),
        ] {
            tab.commands
                .send(TabCommand::Navigate(Some(format!(
                    "http://{address}{path}"
                ))))
                .map_err(|_| anyhow::anyhow!("navigate to {path}"))?;
            if path == "/page" {
                let hidden = tokio::time::timeout(
                    Duration::from_secs(5),
                    tab.state.wait_for(|state| {
                        state
                            .navigation
                            .as_ref()
                            .is_some_and(|nav| nav.title == title)
                    }),
                )
                .await??;
                assert!(hidden.favicon.is_none());
                tab.commands.send(TabCommand::Activity(DocumentActivity {
                    focused: false,
                    visible: true,
                }))?;
            }
            let state = tokio::time::timeout(
                Duration::from_secs(10),
                tab.state.wait_for(|state| {
                    state
                        .navigation
                        .as_ref()
                        .is_some_and(|nav| nav.title == title)
                        && state.favicon.is_some()
                }),
            )
            .await
            .with_context(|| format!("wait for {title} favicon"))??;
            let navigation = state.navigation.as_ref().context("tab navigation state")?;
            assert_eq!(navigation.url.path(), path);
            if path == "/next" {
                assert!(navigation.can_go_back);
            }
            assert!(!navigation.can_go_forward);
            assert_eq!(
                &state.favicon.as_ref().unwrap().as_bytes(0).unwrap()[..4],
                &pixel
            );
        }

        tab.commands
            .send(TabCommand::Activity(DocumentActivity::new(false, false)))?;
        tab.commands
            .send(TabCommand::History(-1))
            .map_err(|_| anyhow::anyhow!("go back to the declared icon page"))?;
        let back = tokio::time::timeout(
            Duration::from_secs(5),
            tab.state.wait_for(|state| {
                state.navigation.as_ref().is_some_and(|navigation| {
                    navigation.url.path() == "/page" && navigation.can_go_forward
                })
            }),
        )
        .await??;
        assert!(back.favicon.is_none());
        assert!(tab.viewport.read(|_| ()).is_none());
        drop(back);

        tab.commands
            .send(TabCommand::Navigate(Some(
                "data:text/html,<title>Iconless</title>".to_owned(),
            )))
            .map_err(|_| anyhow::anyhow!("navigate to iconless page"))?;
        let state = tokio::time::timeout(
            Duration::from_secs(5),
            tab.state.wait_for(|state| {
                state
                    .navigation
                    .as_ref()
                    .is_some_and(|nav| nav.title == "Iconless")
            }),
        )
        .await??;
        assert!(state.favicon.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn blank_target_click_adopts_a_separate_tab() -> Result<()> {
        let (host, mut events) = BrowserHost::start(None)?;
        let mut opener = host.open_tab()?;
        opener.commands.send(TabCommand::Navigate(Some(
            "data:text/html,<title>Videos</title><style>html{background:red}body{margin:0}a{display:block;width:36px;height:24px}</style><a target=_blank href='about:blank%23video'>Video</a>".into(),
        ))).map_err(|_| anyhow::anyhow!("load opener"))?;
        let original = show_tab(&mut opener, "Videos", 1, [0, 0, 255, 255]).await?;

        for (event, buttons) in [("mousedown", 1), ("mouseup", 0)] {
            opener
                .commands
                .send(TabCommand::Pointer(PointerInput {
                    event,
                    position: (12., 10.),
                    button: 0,
                    buttons,
                    click_count: 1,
                    delta: (0., 0.),
                    modifiers: 0,
                }))
                .map_err(|_| anyhow::anyhow!("click video"))?;
        }
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await?
            .context("browser host closed before the click opened a tab")?;
        let BrowserEvent::Popup(popup) = event else {
            anyhow::bail!("expected a popup action from the click")
        };
        assert_eq!(popup.url, "about:blank#video");
        assert!(matches!(
            popup.source,
            moli_core::page::RendererPopupActivationSource::Window {
                exposes_opener: false,
                ..
            }
        ));
        let staged_page = popup
            .pending_auxiliary_page
            .as_ref()
            .context("renderer staged the auxiliary Page")?
            .page_reservation()
            .page_id();
        let mut child = host.open_popup(popup)?;
        let child_document = show_tab(&mut child, "", 2, [255, 255, 255, 255]).await?;
        assert_eq!(child_document.document.page_id, staged_page);
        assert_eq!(
            child
                .state
                .borrow()
                .navigation
                .as_ref()
                .unwrap()
                .url
                .as_str(),
            "about:blank#video"
        );
        assert_eq!(opener.state.borrow().document, Some(original));
        Ok(())
    }
}
