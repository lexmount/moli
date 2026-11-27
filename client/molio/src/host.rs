use std::{
    cell::RefCell,
    fs,
    future::pending,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result};
use moli_core::{
    LayoutPolicy, OptionalResourceFetchMask, RendererDocumentLifecycleIdentity, RendererOutputItem,
    RendererOutputTransportMessage, RendererOwnerAction, RendererViewportEdit,
    RendererViewportEditor, RendererViewportNavigation, RendererViewportPointerInput,
    RendererViewportWheelInput,
    page::{
        BufferedViewportFrames, CompletedPageCommand, Page, PageInputExt, RendererAuxiliaryWindow,
        RendererDraggedFile, RendererPageWindowIdentity, RendererPopupActivationParts,
        ViewportBuffers, ViewportOutput, ViewportUpdate,
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
    follow_display: bool,
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
    pub display_ticks: Option<watch::Sender<(u64, Instant)>>,
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
    Viewport {
        surface: ViewportSurface,
        revision: u64,
        pointer_position: Option<(f64, f64)>,
    },
    Activity(DocumentActivity),
    Pointer(RendererViewportPointerInput),
    PointerPosition {
        viewport_revision: u64,
        position: Option<(f64, f64)>,
    },
    CancelPointer {
        document: RendererDocumentLifecycleIdentity,
        viewport_revision: u64,
    },
    Wheel(RendererViewportWheelInput),
    Key(KeyInput),
    InsertText(String),
    Edit(RendererViewportEdit),
    Clipboard {
        document: RendererDocumentLifecycleIdentity,
        key: KeyInput,
        action: ClipboardAction,
    },
    DropFiles {
        document: RendererDocumentLifecycleIdentity,
        position: (f64, f64),
        paths: Vec<PathBuf>,
        modifiers: u8,
    },
}

pub enum ClipboardAction {
    Copy(oneshot::Sender<Option<String>>),
    Cut(oneshot::Sender<Option<String>>),
    Paste {
        text: Option<String>,
        sequence: Option<u64>,
    },
}

impl TabCommand {
    fn superseded_by(&self, next: &Self) -> bool {
        match (self, next) {
            (Self::Viewport { .. }, Self::Viewport { .. }) => true,
            (Self::Pointer(previous), Self::Pointer(next)) => {
                previous.event == "mousemove"
                    && next.event == "mousemove"
                    && previous.document == next.document
                    && previous.viewport_revision == next.viewport_revision
                    && previous.buttons == 0
                    && next.buttons == 0
                    && previous.modifiers == next.modifiers
            }
            _ => false,
        }
    }
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
    display_ticks: Option<watch::Receiver<(u64, Instant)>>,
    page: TabPage,
}

enum TabPage {
    New(RendererPageWindowIdentity),
    Popup(Box<RendererPopupActivationParts>),
}

impl BrowserHost {
    pub fn start(
        profile: Option<PathBuf>,
        max_fps: Option<u32>,
    ) -> Result<(Self, mpsc::UnboundedReceiver<BrowserEvent>)> {
        let directory = dirs::download_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join("Downloads")))
            .unwrap_or_else(|| std::env::temp_dir().join("molio-downloads"));
        Self::start_with_download_directory(profile, directory, max_fps)
    }

    fn start_with_download_directory(
        profile: Option<PathBuf>,
        directory: PathBuf,
        max_fps: Option<u32>,
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
                    let mut resources =
                        OptionalResourceFetchMask::IMAGE | OptionalResourceFetchMask::FONT;
                    if cfg!(all(any(target_os = "linux", target_os = "macos"), target_pointer_width = "64")) {
                        resources |= OptionalResourceFetchMask::VIDEO;
                    }
                    let mut config = BrowserConfig::default()
                        .with_layout_policy(LayoutPolicy::Realtime)
                        .with_optional_resource_fetch_mask(resources);
                    if let Some(max_fps) = max_fps {
                        config.set_realtime_max_fps(max_fps);
                    }
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
        Ok((
            Self {
                tabs: sender,
                follow_display: max_fps.is_none(),
                downloads: client,
                worker: RefCell::new(Some((shutdown, worker))),
            },
            event_rx,
        ))
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
        self.open_viewport(TabPage::New(RendererPageWindowIdentity::default()))
    }

    pub fn open_popup(&self, popup: Box<RendererPopupActivationParts>) -> Result<TabConnection> {
        self.open_viewport(TabPage::Popup(popup))
    }

    fn open_viewport(&self, page: TabPage) -> Result<TabConnection> {
        let window_identity = match &page {
            TabPage::New(identity) => identity.clone(),
            TabPage::Popup(popup) => popup
                .window_identity()
                .context("popup has no Window identity")?,
        };
        let (commands, request_rx) = mpsc::unbounded_channel();
        let (state_tx, state) = watch::channel(TabState::default());
        let viewport = Arc::new(ViewportOutput::default());
        let (display_ticks, display_rx) = if self.follow_display {
            let (tx, rx) = watch::channel((0, Instant::now()));
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        self.tabs
            .send(OpenTab {
                commands: request_rx,
                state: state_tx,
                viewport: viewport.clone(),
                display_ticks: display_rx,
                page,
            })
            .map_err(|_| anyhow::anyhow!("browser host is unavailable"))?;
        Ok(TabConnection {
            commands,
            state,
            viewport,
            display_ticks,
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
    let initial = tokio::time::timeout(Duration::from_secs(20), async {
        match tab.page {
            TabPage::New(identity) => browser.create_viewport_page(identity).await,
            TabPage::Popup(popup) => browser.adopt_viewport_page(&popup).await,
        }
    })
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
            tick = async {
                if let Some(clock) = &mut tab.display_ticks
                    && clock.changed().await.is_ok()
                {
                    return *clock.borrow_and_update();
                }
                pending().await
            }, if activity.visible && capture.is_some() => {
                let (revision, time) = tick;
                let _ = page.viewport_display_tick(revision, time).await;
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
                    TabCommand::Viewport { surface: viewport, revision, pointer_position } => {
                        surface = Some((viewport, revision));
                        if activity.visible {
                            refresh_capture(&mut page, &mut capture, surface, pointer_position, &tab.viewport).await
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
                            refresh_capture(&mut page, &mut capture, surface, None, &tab.viewport).await
                        } else if next.focused {
                            refresh_editor(&mut page, &tab.state).await
                        } else {
                            Ok(())
                        }
                    }
                    TabCommand::Pointer(input) if activity.visible => {
                        let refresh = activity.focused && input.event != "mousemove";
                        match page.dispatch_viewport_pointer(input).await {
                            Ok(Some(outcome)) => {
                                if let Some(activation) = outcome.pending_download {
                                    downloads.start(activation);
                                }
                                if refresh { refresh_editor(&mut page, &tab.state).await } else { Ok(()) }
                            }
                            Ok(None) => Ok(()),
                            Err(error) => Err(error),
                        }
                    }
                    TabCommand::PointerPosition { viewport_revision, position } if activity.visible => {
                        page.set_viewport_pointer_position(viewport_revision, position).await
                    }
                    TabCommand::CancelPointer { document, viewport_revision } => {
                        page.cancel_viewport_pointer(document, viewport_revision).await
                    }
                    TabCommand::Wheel(input) if activity.visible || input.phase.is_terminal() => {
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
                        let editor = page.edit_viewport(edit).await.map(|reply| reply.editor);
                        publish_editor_ack(&tab.state, sequence, editor)
                    }
                    TabCommand::Clipboard { document, key, action } => {
                        let allowed = if activity.focused && tab.state.borrow().document == Some(document) {
                            page.dispatch_key_event_with_outcome_async(
                                key.event, &key.key, &key.code, "", key.modifiers, key.repeat, false,
                            ).await.map(|outcome| {
                                if let Some(activation) = outcome.pending_download {
                                    downloads.start(activation);
                                }
                                outcome.handled
                            })
                        } else {
                            Ok(false)
                        };
                        let sequence = match &action {
                            ClipboardAction::Paste { sequence, .. } => *sequence,
                            _ => None,
                        };
                        let result = match (allowed, action) {
                            (Ok(true), ClipboardAction::Copy(reply)) => {
                                page.copy_viewport_selection(document).await.map(|text| {
                                    let _ = reply.send(text);
                                })
                            }
                            (Ok(true), ClipboardAction::Cut(reply)) => {
                                page.cut_viewport_selection(document).await.map(|text| {
                                    let _ = reply.send(text);
                                })
                            }
                            (Ok(true), ClipboardAction::Paste { text, sequence }) => {
                                page.paste_viewport_text(document, text, sequence).await.map(|_| ())
                            }
                            (Ok(false), _) => Ok(()),
                            (Err(error), _) => Err(error),
                        };
                        if let Some(sequence) = sequence {
                            let editor = match result {
                                Ok(()) => page.viewport_editor().await,
                                Err(error) => Err(error),
                            };
                            publish_editor_ack(&tab.state, sequence, editor)
                        } else if result.is_ok() {
                            refresh_editor(&mut page, &tab.state).await
                        } else {
                            result
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

fn publish_editor_ack(
    state: &watch::Sender<TabState>,
    sequence: u64,
    editor: Result<Option<RendererViewportEditor>>,
) -> Result<()> {
    match editor {
        Ok(editor) => {
            state.send_modify(|state| {
                state.editor_ack = Some(sequence);
                state.editor = editor;
            });
            Ok(())
        }
        Err(error) => {
            state.send_modify(|state| {
                state.editor_ack = Some(sequence);
                state.editor = None;
            });
            Err(error)
        }
    }
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
    pointer_position: Option<(f64, f64)>,
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
            page.subscribe_viewport_buffer_with_pointer(
                surface,
                revision,
                pointer_position,
                buffers,
                output.clone(),
            )
            .await?,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloads::{Download, DownloadCommand, DownloadState};
    use moli_core::RendererViewportCursorKind;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

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
            BrowserHost::start_with_download_directory(None, directory.path().to_owned(), None)?;
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
            BrowserHost::start_with_download_directory(None, directory.path().to_owned(), None)?;
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
            BrowserHost::start_with_download_directory(None, directory.path().to_owned(), None)?;
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
            BrowserHost::start_with_download_directory(None, directory.path().to_owned(), None)?;
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
            BrowserHost::start_with_download_directory(None, directory.path().to_owned(), None)?;
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
        let (host, _events) = BrowserHost::start_with_download_directory(
            Some(profile.clone()),
            directory.clone(),
            None,
        )?;
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
            BrowserHost::start_with_download_directory(Some(profile), directory, None)?;
        let mut entries = restored.downloads.entries.clone();
        let canceled = download_matching(&mut entries, |entry| entry.name == "slow.dat").await?;
        assert!(matches!(canceled.state, DownloadState::Canceled));
        assert_eq!(entries.borrow().len(), 1);
        Ok(())
    }

    fn click(tab: &TabConnection, modifiers: u8) -> Result<()> {
        let (document, viewport_revision) = tab
            .viewport
            .read(|frame| {
                frame
                    .pixels
                    .map(|_| (frame.document, frame.viewport_revision))
            })
            .flatten()
            .context("no displayed viewport for the popup link")?;
        for (event, buttons) in [("mousedown", 1), ("mouseup", 0)] {
            tab.commands
                .send(TabCommand::Pointer(RendererViewportPointerInput {
                    document,
                    viewport_revision,
                    event,
                    position: (12., 10.),
                    button: 0,
                    buttons,
                    click_count: 1,
                    modifiers,
                }))
                .map_err(|_| anyhow::anyhow!("click popup link"))?;
        }
        Ok(())
    }

    async fn next_popup(
        events: &mut mpsc::UnboundedReceiver<BrowserEvent>,
    ) -> Result<Box<RendererPopupActivationParts>> {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .context("waiting for popup activation")?
            .context("browser host closed before the click opened a tab")?;
        let BrowserEvent::Popup(popup) = event else {
            anyhow::bail!("expected a popup action from the click");
        };
        Ok(popup)
    }

    async fn wait_for_navigation(
        tab: &TabConnection,
        url: &str,
        title: &str,
    ) -> Result<RendererDocumentLifecycleIdentity> {
        let mut updates = tab.state.clone();
        let result = tokio::time::timeout(
            Duration::from_secs(8),
            updates.wait_for(|state| {
                state.error.is_some()
                    || state
                        .navigation
                        .as_ref()
                        .is_some_and(|nav| nav.url.as_str() == url && nav.title == title)
            }),
        )
        .await;
        let state = result.with_context(|| {
            format!(
                "waiting for {url} with title {title:?}, saw {:?}",
                tab.state.borrow().navigation
            )
        })??;
        anyhow::ensure!(
            state.error.is_none(),
            "tab failed to navigate: {:?}",
            state.error
        );
        state.document.context("tab has a committed document")
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
            .send(TabCommand::Viewport {
                surface: ViewportSurface {
                    inner_width: 40,
                    inner_height: 30,
                    device_pixel_ratio: 2.,
                    ..ViewportSurface::default()
                },
                revision,
                pointer_position: None,
            })
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
        let (host, _events) = BrowserHost::start(None, None)?;
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
    async fn clipboard_shortcut_respects_canceled_keydown() -> Result<()> {
        let (host, _events) = BrowserHost::start(None, None)?;
        let mut tab = host.open_tab()?;
        navigate(
            &tab,
            concat!(
                "data:text/html,<title>Clipboard</title><script>",
                "document.addEventListener('keydown', event => event.preventDefault(), {once:true});",
                "document.addEventListener('copy', event => {",
                "event.clipboardData.setData('text/plain','from copy');event.preventDefault()",
                "})</script>",
            ),
        )?;
        let document = show_tab(&mut tab, "Clipboard", 1, [255; 4]).await?;
        for allowed in [false, true] {
            let (reply, result) = oneshot::channel();
            tab.commands
                .send(TabCommand::Clipboard {
                    document,
                    key: KeyInput {
                        event: "keydown",
                        key: "c".into(),
                        code: "KeyC".into(),
                        modifiers: if cfg!(target_os = "macos") { 4 } else { 2 },
                        repeat: false,
                    },
                    action: ClipboardAction::Copy(reply),
                })
                .map_err(|_| anyhow::anyhow!("dispatch copy shortcut"))?;
            let text = tokio::time::timeout(Duration::from_secs(5), result).await?;
            if allowed {
                assert_eq!(text?.as_deref(), Some("from copy"));
            } else {
                assert!(text.is_err());
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn viewport_resize_uses_the_pointer_position_for_its_revision() -> Result<()> {
        let (host, _events) = BrowserHost::start(None, None)?;
        let mut tab = host.open_tab()?;
        let html = concat!(
            "data:text/html,<title>Cursor</title><style>",
            "body{margin:0}div{display:inline-block;width:40px;height:40px;cursor:crosshair}",
            "div+div{cursor:text}</style><div></div><div></div>"
        );
        tab.commands
            .send(TabCommand::Navigate(Some(html.into())))
            .map_err(|_| anyhow::anyhow!("load cursor tab"))?;
        let document = show_tab(&mut tab, "Cursor", 1, [255; 4]).await?;
        let mut updates = tab.viewport.subscribe();

        for (revision, width, x) in [(2, 84, 16.), (3, 88, 60.)] {
            tab.commands
                .send(TabCommand::Viewport {
                    surface: ViewportSurface {
                        inner_width: width,
                        inner_height: 60,
                        ..ViewportSurface::default()
                    },
                    revision,
                    pointer_position: Some((x, 16.)),
                })
                .map_err(|_| anyhow::anyhow!("resize cursor tab"))?;
        }

        let cursor = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(cursor) = tab
                    .viewport
                    .cursor()
                    .filter(|cursor| cursor.viewport_revision == 3)
                {
                    break Ok::<_, watch::error::RecvError>(cursor);
                }
                updates.changed().await?;
            }
        })
        .await??;
        assert_eq!(cursor.document, document);
        assert_eq!(cursor.kind, RendererViewportCursorKind::Text);
        Ok(())
    }

    #[cfg(all(
        any(target_os = "linux", target_os = "macos"),
        target_pointer_width = "64"
    ))]
    #[tokio::test]
    async fn default_host_loads_h264_video_from_http() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let server = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let mut request = BufReader::new(socket);
                let Ok((path, _)) = read_http_request(&mut request).await else {
                    continue;
                };
                let (mime, body): (&str, &[u8]) = if path == "/clip.mp4" {
                    (
                        "video/mp4",
                        include_bytes!("../../../moli-video/tests/fixtures/primary-colors.mp4"),
                    )
                } else {
                    (
                        "text/html",
                        br#"<video src="/clip.mp4" onloadeddata="document.title = String(this.videoWidth)"></video>"#,
                    )
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let mut socket = request.into_inner();
                if socket.write_all(response.as_bytes()).await.is_ok() {
                    let _ = socket.write_all(body).await;
                }
            }
        });
        let (host, _events) = BrowserHost::start(None, None)?;
        let tab = host.open_tab()?;
        navigate(&tab, &url)?;
        let result = wait_for_navigation(&tab, &url, "48").await;
        server.abort();
        result?;
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

        let (host, _events) = BrowserHost::start(None, None)?;
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
        let (host, mut events) = BrowserHost::start(None, None)?;
        let mut opener = host.open_tab()?;
        opener.commands.send(TabCommand::Navigate(Some(
            "data:text/html,<title>Videos</title><style>html{background:red}body{margin:0}a{display:block;width:36px;height:24px}</style><a target=_blank href='about:blank%23video'>Video</a>".into(),
        ))).map_err(|_| anyhow::anyhow!("load opener"))?;
        let original = show_tab(&mut opener, "Videos", 1, [0, 0, 255, 255]).await?;

        click(&opener, 0)?;
        let popup = next_popup(&mut events).await?;
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
        let name = popup
            .browsing_context_name
            .clone()
            .context("popup name cell")?;
        let window = popup.auxiliary_window.clone().context("auxiliary window")?;
        let mut child = host.open_popup(popup)?;
        assert_eq!(child.window_identity.name(), &name);
        assert_eq!(child.window_identity.auxiliary_window(), Some(&window));
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

    #[tokio::test]
    async fn popup_adoption_keeps_the_fragment_set_by_the_opening_script() -> Result<()> {
        let (base, _requests) = popup_fixture().await?;
        let (host, mut events) = BrowserHost::start(None, None)?;
        let mut opener = host.open_tab()?;
        opener
            .commands
            .send(TabCommand::Navigate(Some(format!(
                "{base}/fragment-source"
            ))))
            .map_err(|_| anyhow::anyhow!("load opener"))?;
        show_tab(&mut opener, "Source", 1, [0, 0, 255, 255]).await?;
        click(&opener, 0)?;
        let popup = next_popup(&mut events).await?;
        assert_eq!(popup.url, "about:blank#first");
        let mut child = host.open_popup(popup)?;
        show_tab(&mut child, "", 2, [255, 255, 255, 255]).await?;
        assert_eq!(
            child
                .state
                .borrow()
                .navigation
                .as_ref()
                .unwrap()
                .url
                .as_str(),
            "about:blank#second"
        );
        Ok(())
    }

    const SOURCE_PAGE: &str = r#"<!doctype html><title>Source</title><base href="/base/">
        <style>html{background:red}body{margin:0}a{display:block;width:36px;height:24px}</style>
        <a id="open" href="/redirect" target="_blank" referrerpolicy="origin">Open</a>
        <script>
        if (location.pathname === "/fragment-source") {
            document.getElementById("open").onclick = event => {
                event.preventDefault();
                const child = window.open("about:blank#first", "child");
                child.location.hash = "second";
            };
        } else if (location.pathname === "/event-source") {
            let step = 0;
            let report;
            const link = document.getElementById("open");
            link.target = "report";
            link.rel = "opener";
            link.onclick = event => {
                switch (step++) {
                    case 0:
                        event.preventDefault();
                        report = window.open("/navigation-events", "report");
                        break;
                    case 1:
                        event.preventDefault();
                        window.open("/cancel", "report");
                        break;
                    case 2:
                        link.href = "/navigation-events#details";
                        break;
                    default:
                        link.href = "/intercept";
                        if (report.location.hash) {
                            event.preventDefault();
                            document.title = "Fragment unexpectedly committed";
                        }
                }
            };
        } else if (location.pathname === "/source") {
            let step = 0;
            let report;
            document.getElementById("open").onclick = event => {
                event.preventDefault();
                if (step++ === 0) {
                    report = window.open("/obsolete", "report");
                    report.bootstrapToken = "retained";
                    report.location.href = "/report";
                } else if (step === 2) {
                    const url = new URL("/reused", location.href);
                    url.hostname = "localhost";
                    window.open(url.href, "report");
                } else {
                    window.open("about:blank", "report");
                    const timer = setInterval(() => {
                        try {
                            if (report.location.href === "about:blank") {
                                document.title = report.document.baseURI;
                                clearInterval(timer);
                            }
                        } catch (error) {
                            document.title = String(error);
                        }
                    }, 10);
                }
            };
        }
        </script>"#;

    const POPUP_NAVIGATION_EVENTS: &str = r##"<!doctype html><title>Navigation events</title>
        <script>
        navigation.addEventListener("navigate", event => {
            const { pathname: path, hash } = new URL(event.destination.url);
            if (path === "/cancel") {
                event.preventDefault();
                opener.document.title = event.defaultPrevented && event.sourceElement === null
                    ? "Popup navigation canceled" : "Unexpected window.open event";
            } else if (hash === "#details") {
                if (event.userInitiated && event.destination.sameDocument && event.hashChange
                    && event.sourceElement === opener.document.getElementById("open")) {
                    event.preventDefault();
                }
                opener.document.title = event.defaultPrevented
                    ? "Popup fragment navigation canceled" : "Unexpected fragment event";
            } else if (path === "/intercept") {
                event.intercept({handler() {
                    document.title = event.sourceElement === opener.document.getElementById("open")
                        && event.userInitiated ? "Popup navigation intercepted" : "Unexpected link event";
                    opener.document.title = document.title;
                }});
            }
        });
        </script>"##;

    async fn popup_fixture() -> Result<(String, mpsc::UnboundedReceiver<(String, Option<String>)>)>
    {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let (requests_tx, requests_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let requests = requests_tx.clone();
                tokio::spawn(async move {
                    let mut reader = BufReader::new(socket);
                    let mut line = String::new();
                    reader.read_line(&mut line).await?;
                    let path = line
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .to_owned();
                    let mut referrer = None;
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).await? == 0 || line.trim().is_empty() {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':')
                            && name.eq_ignore_ascii_case("referer")
                        {
                            referrer = Some(value.trim().to_owned());
                        }
                    }
                    let (status, headers, body) = match path.as_str() {
                        "/source" | "/fragment-source" | "/event-source" | "/background-source" => {
                            ("200 OK", "", SOURCE_PAGE)
                        }
                        "/navigation-events" => ("200 OK", "", POPUP_NAVIGATION_EVENTS),
                        "/report" => (
                            "200 OK",
                            "",
                            "<script>document.title = (window.bootstrapToken ?? 'missing') + '|' + document.referrer;</script>",
                        ),
                        "/reused" => (
                            "200 OK",
                            "",
                            "<script>document.title = 'Reused report|' + document.referrer; navigation.addEventListener('navigate', event => event.preventDefault());</script>",
                        ),
                        "/redirect" => (
                            "302 Found",
                            "Location: /background-report\r\nReferrer-Policy: no-referrer\r\n",
                            "",
                        ),
                        "/background-report" => (
                            "200 OK",
                            "",
                            "<title>Background report</title><script>setTimeout(() => {location.hash = 'done'; document.title = 'Background ready|' + document.referrer;}, 0);</script>",
                        ),
                        _ => ("404 Not Found", "", ""),
                    };
                    let _ = requests.send((path, referrer));
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
                        body.len()
                    );
                    reader.into_inner().write_all(response.as_bytes()).await
                });
            }
        });
        Ok((base, requests_rx))
    }

    #[tokio::test]
    async fn popup_navigations_preserve_source_and_initial_window() -> Result<()> {
        let (base, mut requests) = popup_fixture().await?;
        let (host, mut events) = BrowserHost::start(None, None)?;
        let source = format!("{base}/source");
        let mut opener = host.open_tab()?;
        opener
            .commands
            .send(TabCommand::Navigate(Some(source.clone())))
            .map_err(|_| anyhow::anyhow!("load popup source"))?;
        show_tab(&mut opener, "Source", 1, [0, 0, 255, 255]).await?;
        click(&opener, 0)?;
        let popup = next_popup(&mut events).await?;
        assert_eq!(popup.url, format!("{base}/obsolete"));
        let report = host.open_popup(popup)?;
        let first_document = wait_for_navigation(
            &report,
            &format!("{base}/report"),
            &format!("retained|{source}"),
        )
        .await?;
        click(&opener, 0)?;
        let reuse = next_popup(&mut events).await?;
        assert!(reuse.pending_auxiliary_page.is_none());
        assert_eq!(
            reuse.browsing_context_name.as_ref(),
            Some(report.window_identity.name())
        );
        let reused_url = format!("{}/reused", base.replace("127.0.0.1", "localhost"));
        let reused_document =
            wait_for_navigation(&report, &reused_url, &format!("Reused report|{base}/")).await?;
        assert_ne!(first_document, reused_document);
        click(&opener, 0)?;
        let blank = next_popup(&mut events).await?;
        assert!(blank.pending_auxiliary_page.is_none());
        assert_eq!(blank.url, "about:blank");
        let blank_document = wait_for_navigation(&report, "about:blank", "").await?;
        assert_ne!(blank_document, reused_document);
        wait_for_navigation(&opener, &source, &format!("{base}/base/")).await?;
        let mut visited = Vec::new();
        while let Ok((path, referrer)) = requests.try_recv() {
            assert_ne!(
                path, "/obsolete",
                "a newer Location assignment superseded this URL"
            );
            let expected_referrer = match path.as_str() {
                "/report" => source.clone(),
                "/reused" => format!("{base}/"),
                _ => continue,
            };
            assert_eq!(referrer.as_deref(), Some(expected_referrer.as_str()));
            visited.push(path);
        }
        assert_eq!(visited, ["/report", "/reused"]);
        Ok(())
    }

    #[tokio::test]
    async fn reused_popup_applies_navigation_event_cancellation_and_interception() -> Result<()> {
        let (base, mut requests) = popup_fixture().await?;
        let (host, mut events) = BrowserHost::start(None, None)?;
        let source = format!("{base}/event-source");
        let mut opener = host.open_tab()?;
        opener
            .commands
            .send(TabCommand::Navigate(Some(source.clone())))
            .map_err(|_| anyhow::anyhow!("load popup navigation event source"))?;
        show_tab(&mut opener, "Source", 1, [0, 0, 255, 255]).await?;
        click(&opener, 0)?;
        let report = host.open_popup(next_popup(&mut events).await?)?;
        let report_url = format!("{base}/navigation-events");
        let document = wait_for_navigation(&report, &report_url, "Navigation events").await?;

        click(&opener, 0)?;
        let canceled = next_popup(&mut events).await?;
        assert_eq!(canceled.url, format!("{base}/cancel"));
        assert!(canceled.pending_auxiliary_page.is_none());
        wait_for_navigation(&opener, &source, "Popup navigation canceled").await?;
        assert_eq!(
            wait_for_navigation(&report, &report_url, "Navigation events").await?,
            document,
        );

        click(&opener, 0)?;
        let fragment = next_popup(&mut events).await?;
        assert_eq!(fragment.url, format!("{report_url}#details"));
        assert!(fragment.pending_auxiliary_page.is_none());
        wait_for_navigation(&opener, &source, "Popup fragment navigation canceled").await?;
        assert_eq!(
            wait_for_navigation(&report, &report_url, "Navigation events").await?,
            document,
        );

        click(&opener, 0)?;
        let intercepted = next_popup(&mut events).await?;
        assert_eq!(intercepted.url, format!("{base}/intercept"));
        assert!(intercepted.pending_auxiliary_page.is_none());
        assert_eq!(
            intercepted.disposition,
            moli_core::page::RendererPopupDisposition::Foreground,
        );
        wait_for_navigation(&opener, &source, "Popup navigation intercepted").await?;
        // The browser selects the reused tab after a foreground activation.
        report
            .commands
            .send(TabCommand::Activity(DocumentActivity::new(true, true)))
            .map_err(|_| anyhow::anyhow!("activate the reused popup"))?;
        assert_eq!(
            wait_for_navigation(&report, &intercepted.url, "Popup navigation intercepted").await?,
            document,
        );
        while let Ok((path, _)) = requests.try_recv() {
            assert!(!matches!(path.as_str(), "/cancel" | "/intercept"), "{path}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn background_popup_updates_metadata_through_redirect_and_script() -> Result<()> {
        let (base, mut requests) = popup_fixture().await?;
        let (host, mut events) = BrowserHost::start(None, None)?;
        let mut opener = host.open_tab()?;
        opener
            .commands
            .send(TabCommand::Navigate(Some(format!(
                "{base}/background-source"
            ))))
            .map_err(|_| anyhow::anyhow!("load background popup source"))?;
        show_tab(&mut opener, "Source", 1, [0, 0, 255, 255]).await?;
        let new_tab_modifier = if cfg!(target_os = "macos") {
            0b100
        } else {
            0b010
        };
        click(&opener, new_tab_modifier)?;
        let popup = next_popup(&mut events).await?;
        assert_eq!(
            popup.disposition,
            moli_core::page::RendererPopupDisposition::Background
        );
        let report = host.open_popup(popup)?;
        wait_for_navigation(
            &report,
            &format!("{base}/background-report#done"),
            "Background ready|",
        )
        .await?;
        assert!(report.viewport.read(|_| ()).is_none());
        let mut visited = Vec::new();
        let expected_referrer = format!("{base}/");
        while let Ok((path, referrer)) = requests.try_recv() {
            let expected = match path.as_str() {
                "/redirect" => Some(expected_referrer.as_str()),
                "/background-report" => None,
                _ => continue,
            };
            assert_eq!(referrer.as_deref(), expected, "{path}");
            visited.push(path);
        }
        assert_eq!(visited, ["/redirect", "/background-report"]);
        Ok(())
    }
}
