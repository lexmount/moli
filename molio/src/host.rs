use std::{fs, future::pending, path::PathBuf, sync::Arc, thread, time::Duration};

use anyhow::{Context as _, Result};
use moli_core::{
    LayoutPolicy, OptionalResourceFetchMask, RendererDocumentLifecycleIdentity,
    RendererViewportEdit, RendererViewportEditor, RendererViewportNavigation,
    page::{
        BufferedViewportFrames, CompletedPageCommand, Page, PageInputExt, RendererDraggedFile,
        RendererPointerEventProperties, ViewportBuffers, ViewportOutput, ViewportUpdate,
    },
    protocol_types::{DocumentActivity, ViewportSurface},
    runtime::{Browser, BrowserConfig, RenderedDomWaitUntil},
};
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::{JoinHandle, LocalSet},
};

pub struct BrowserHost(mpsc::UnboundedSender<OpenTab>);

pub struct TabConnection {
    pub commands: mpsc::UnboundedSender<TabCommand>,
    pub state: watch::Receiver<TabState>,
    pub viewport: Arc<ViewportOutput>,
}

#[derive(Clone, Default)]
pub struct TabState {
    pub navigation: Option<RendererViewportNavigation>,
    pub document: Option<RendererDocumentLifecycleIdentity>,
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
}

impl BrowserHost {
    pub fn start(profile: Option<PathBuf>) -> Result<Self> {
        let (sender, mut requests) = mpsc::unbounded_channel::<OpenTab>();
        thread::Builder::new()
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
                    config.set_profile_dir(profile);
                    let browser = Browser::new(config);
                    while let Some(tab) = requests.recv().await {
                        match &browser {
                            Ok(browser) => {
                                tokio::task::spawn_local(run_tab(browser.clone(), tab));
                            }
                            Err(error) => {
                                tab.state
                                    .send_modify(|state| state.error = Some(error.to_string()));
                            }
                        }
                    }
                });
            })
            .context("start browser host thread")?;
        Ok(Self(sender))
    }

    pub fn open_tab(&self) -> Result<TabConnection> {
        let (commands, request_rx) = mpsc::unbounded_channel();
        let (state_tx, state) = watch::channel(TabState::default());
        let viewport = Arc::new(ViewportOutput::default());
        self.0
            .send(OpenTab {
                commands: request_rx,
                state: state_tx,
                viewport: viewport.clone(),
            })
            .map_err(|_| anyhow::anyhow!("browser host is unavailable"))?;
        Ok(TabConnection {
            commands,
            state,
            viewport,
        })
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

async fn run_tab(browser: Browser, mut tab: OpenTab) {
    let initial = browser.fetch_allow_http_error_with_wait_until(
        "about:blank",
        RenderedDomWaitUntil::DomContentLoaded,
        Duration::from_secs(20),
    );
    tokio::pin!(initial);
    let mut page = match initial.await {
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
    let mut queued = None;
    let mut activity = DocumentActivity::new(false, false);
    let _ = page.set_document_activity_async(activity).await;

    loop {
        tokio::select! {
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
            event = next_viewport_update(&mut capture) => match event {
                Some(ViewportUpdate::StateChanged) => {
                    let frame = capture.as_mut().and_then(BufferedViewportFrames::latest_state);
                    tab.state.send_if_modified(|state| {
                        let (document, navigation, editor) = frame.map_or((None, None, None), |frame| {
                            (Some(frame.document), Some(frame.navigation), frame.editor.filter(|_| activity.focused))
                        });
                        if state.document == document && state.navigation == navigation && state.editor == editor {
                            return false;
                        }
                        state.document = document;
                        state.navigation = navigation;
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
                        match page.start_viewport_navigation(target.clone()) {
                            Ok(pending) => {
                                tab.state.send_modify(|state| {
                                    state.loading = true;
                                    state.error = None;
                                    state.pending_url = target;
                                });
                                navigation = Some(tokio::task::spawn_local(pending.wait()));
                                Ok(())
                            }
                            Err(error) => Err(error),
                        }
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
                            refresh_capture(&mut page, &mut capture, surface, &tab.viewport).await
                        } else if next.focused {
                            refresh_editor(&mut page, &tab.state).await
                        } else {
                            Ok(())
                        }
                    }
                    TabCommand::Pointer(input) if activity.visible => {
                        let refresh = activity.focused && input.event != "mousemove";
                        let result = dispatch_pointer(&mut page, input).await;
                        if refresh && result.is_ok() { refresh_editor(&mut page, &tab.state).await } else { result }
                    }
                    TabCommand::Key(input) if activity.focused => {
                        let enter = input.event == "keydown"
                            && input.key == "Enter"
                            && input.modifiers & 0b111 == 0;
                        let result = page.dispatch_key_event_with_outcome_async(
                            input.event, &input.key, &input.code, if enter { "\r" } else { "" },
                            input.modifiers, input.repeat, enter,
                        ).await.map(drop);
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

async fn dispatch_pointer(page: &mut Page, input: PointerInput) -> Result<()> {
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
    page.finish_dispatch_mouse_event_at_point_with_outcome(pending.wait().await?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let host = BrowserHost::start(None)?;
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
}
