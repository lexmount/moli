use std::{
    cell::RefCell,
    collections::VecDeque,
    ffi::{CStr, CString, c_char, c_void},
    future::pending,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    thread,
};

use anyhow::Result;
use moli_core::{
    LayoutPolicy, OptionalResourceFetchMask, RendererDocumentLifecycleIdentity, RendererOutputItem,
    RendererOutputTransportMessage, RendererOwnerAction, RendererPageNavigationState,
    RendererViewportEdit, RendererViewportEditAction, RendererViewportEditor,
    RendererViewportPointerInput, RendererViewportWheelInput, RendererViewportWheelPhase,
    page::{
        BufferedViewportFrames, Page, PageInputExt, RendererAuxiliaryWindow,
        RendererPopupActivationParts, ViewportBuffers, ViewportOutput, ViewportUpdate,
    },
    protocol_types::{DocumentActivity, ViewportSurface},
    runtime::{Browser, BrowserConfig},
};
use parking_lot::Mutex;
use tokio::{
    sync::{mpsc, watch},
    task::LocalSet,
};

use super::downloads::{DownloadClient, DownloadCommand, DownloadState, Downloads};
use super::viewport_edit_sequence::ViewportEditSequence;

enum Command {
    Navigate(Option<String>),
    History(i64),
    Stop,
    Activity(bool),
    Edit(RendererViewportEdit),
    EditorKey {
        document: RendererDocumentLifecycleIdentity,
        target: u32,
        key: EditorKey,
    },
    Resize {
        width: u32,
        height: u32,
        scale: f64,
        screen: (u32, u32),
        revision: u64,
    },
    Tap(InputPosition),
    Scroll(InputPosition, (f64, f64)),
}

enum EditorKey {
    Backspace,
    Enter,
    SelectAll,
}

struct InputPosition {
    document: RendererDocumentLifecycleIdentity,
    revision: u64,
    position: (f64, f64),
}

// The UIKit main thread owns the handle. Only the channels and viewport output
// are shared with the browser worker.
type PopupQueue = Arc<Mutex<VecDeque<Box<RendererPopupActivationParts>>>>;
type EditorState = Option<(RendererDocumentLifecycleIdentity, RendererViewportEditor)>;

#[derive(Default)]
struct EditorUpdate {
    state: EditorState,
    ack: Option<u64>,
}

struct OpenTab {
    popup: Option<Box<RendererPopupActivationParts>>,
    editor: watch::Sender<EditorUpdate>,
    commands: mpsc::UnboundedReceiver<Command>,
    output: Arc<ViewportOutput>,
    status: watch::Sender<String>,
    navigation: watch::Sender<Option<RendererPageNavigationState>>,
}

pub struct Molio {
    tabs: mpsc::UnboundedSender<OpenTab>,
    popups: PopupQueue,
    downloads: DownloadClient,
    auxiliary: Option<RendererAuxiliaryWindow>,
    editor: watch::Receiver<EditorUpdate>,
    displayed_editor: EditorState,
    edits: RefCell<ViewportEditSequence>,
    commands: mpsc::UnboundedSender<Command>,
    viewport: Arc<ViewportOutput>,
    frames: watch::Receiver<()>,
    status: watch::Receiver<String>,
    navigation: watch::Receiver<Option<RendererPageNavigationState>>,
    revision: u64,
    presented: Option<(RendererDocumentLifecycleIdentity, u64)>,
}

async fn run_host(
    mut tabs: mpsc::UnboundedReceiver<OpenTab>,
    profile: PathBuf,
    popups: PopupQueue,
    mut download_commands: mpsc::UnboundedReceiver<DownloadCommand>,
    download_entries: watch::Sender<Vec<super::downloads::Download>>,
    download_directory: PathBuf,
) {
    let mut config = BrowserConfig::default()
        .with_layout_policy(LayoutPolicy::Realtime)
        .with_optional_resource_fetch_mask(
            OptionalResourceFetchMask::IMAGE | OptionalResourceFetchMask::FONT,
        );
    config.set_realtime_max_fps(30);
    config.set_profile_dir(Some(profile.clone()));

    let browser = match Browser::new(config) {
        Ok(browser) => browser,
        Err(error) => {
            while let Some(tab) = tabs.recv().await {
                tab.status.send_replace(format!("{error:#}"));
            }
            return;
        }
    };
    let downloads = Downloads::new(
        browser.resource_request_client(),
        download_entries,
        download_directory,
        Some(&profile),
    );
    let mut actions = browser.subscribe_viewport_actions();
    loop {
        tokio::select! {
            tab = tabs.recv() => {
                let Some(tab) = tab else { break };
                let browser = browser.clone();
                let downloads = Rc::clone(&downloads);
                tokio::task::spawn_local(async move {
                    if let Err(error) = run_tab(browser, &tab.status, tab.commands,
                        tab.output, tab.navigation, tab.editor, tab.popup, downloads).await {
                        tab.status.send_replace(format!("{error:#}"));
                    }
                });
            }
            Some(command) = download_commands.recv() => downloads.command(command),
            message = actions.recv(), if !actions.is_closed() => {
                let Some(RendererOutputTransportMessage::Publication(output)) = message else { continue };
                for record in output.into_records() {
                    match record.into_parts().1 {
                        RendererOutputItem::OwnerAction(RendererOwnerAction::Download(activation)) =>
                            downloads.start(activation),
                        RendererOutputItem::OwnerAction(RendererOwnerAction::Popup(activation)) => {
                            let popup = activation.into_parts();
                            if popup.pending_auxiliary_page.is_some() {
                                popups.lock().push_back(Box::new(popup));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    downloads.shutdown().await;
}

async fn run_tab(
    browser: Browser,
    status: &watch::Sender<String>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    output: Arc<ViewportOutput>,
    navigation_state: watch::Sender<Option<RendererPageNavigationState>>,
    editor_state: watch::Sender<EditorUpdate>,
    popup: Option<Box<RendererPopupActivationParts>>,
    downloads: Rc<Downloads>,
) -> Result<()> {
    let mut page = if let Some(popup) = popup {
        browser.adopt_viewport_page(&popup).await?
    } else {
        browser.create_viewport_page(Default::default()).await?
    };
    let mut page_state = page.subscribe_navigation_state().await?;
    page_state.mark_changed();
    status.send_replace(String::new());
    let mut active = page.document_activity().visible;
    let mut viewport = None;
    let mut capture: Option<BufferedViewportFrames> = None;
    let mut last_edit_frame_sequence = 0;
    let mut navigation: Option<
        tokio::task::JoinHandle<Result<moli_core::page::CompletedPageCommand>>,
    > = None;
    loop {
        tokio::select! {
            Ok(()) = page_state.changed() => {
                navigation_state.send_replace(page_state.borrow_and_update().clone());
            }
            result = async {
                match &mut navigation {
                    Some(task) => task.await,
                    None => pending().await,
                }
            } => {
                navigation = None;
                let result = result
                    .map_err(anyhow::Error::from)
                    .and_then(|result| result)
                    .and_then(|completion| page.finish_viewport_navigation(completion));
                status.send_replace(match result {
                    Ok(true) => String::new(),
                    Ok(false) => "Navigation did not commit".into(),
                    Err(error) => format!("{error:#}"),
                });
            }
            update = async {
                match &mut capture {
                    Some(capture) => capture.changed().await,
                    None => pending().await,
                }
            } => match update {
                Ok(ViewportUpdate::ResourceChanged) => page.wake_viewport_frames().await?,
                Ok(ViewportUpdate::StateChanged) => {
                    let frame = capture.as_mut().and_then(BufferedViewportFrames::latest_state);
                    if frame.as_ref().is_some_and(|frame| frame.sequence <= last_edit_frame_sequence) {
                        continue;
                    }
                    let editor = frame.and_then(|frame| frame.editor.map(|editor| (frame.document, editor)));
                    editor_state.send_if_modified(|current| {
                        if current.state == editor { return false; }
                        current.state = editor;
                        true
                    });
                }
                Err(_) => capture = None,
            },
            command = commands.recv() => match command {
                Some(Command::Navigate(url)) => {
                    if let Some(task) = navigation.take() {
                        task.abort();
                    }
                    match page.start_viewport_navigation(url) {
                        Ok(command) => {
                            status.send_replace("Loading…".into());
                            navigation = Some(tokio::task::spawn_local(command.wait()));
                        }
                        Err(error) => { status.send_replace(format!("{error:#}")); }
                    }
                }
                Some(Command::History(delta)) => {
                    if let Some(task) = navigation.take() {
                        task.abort();
                    }
                    if let Err(error) = page.traverse_viewport_history(delta).await {
                        status.send_replace(format!("{error:#}"));
                    } else {
                        status.send_replace(String::new());
                    }
                }
                Some(Command::Stop) => {
                    if let Some(task) = navigation.take() {
                        task.abort();
                    }
                    page.stop_document_lifecycle_async().await?;
                    status.send_replace(String::new());
                }
                Some(Command::Activity(next)) => {
                    active = next;
                    if !active {
                        capture = None;
                        editor_state.send_if_modified(|current| current.state.take().is_some());
                    }
                    page.set_document_activity_async(DocumentActivity::new(active, active)).await?;
                    if active && capture.is_none()
                        && let Some((surface, revision)) = viewport
                    {
                        capture = Some(subscribe_viewport(&mut page, surface, revision, &output).await?);
                        last_edit_frame_sequence = 0;
                    }
                }
                Some(Command::Edit(edit)) => {
                    let document = edit.document;
                    let sequence = edit.sequence;
                    if active && page_state.borrow().as_ref().is_some_and(|state| state.document == document) {
                        let reply = page.edit_viewport(edit).await?;
                        last_edit_frame_sequence = reply.last_frame_sequence;
                        editor_state.send_replace(EditorUpdate {
                            state: reply.editor.map(|editor| (document, editor)),
                            ack: Some(sequence),
                        });
                    } else {
                        editor_state.send_modify(|current| current.ack = Some(sequence));
                    }
                }
                Some(Command::EditorKey { document, target, key }) if active => {
                    if !page_state.borrow().as_ref().is_some_and(|state| state.document == document) {
                        continue;
                    }
                    if !page.viewport_editor().await?.is_some_and(|editor| editor.target == target) {
                        continue;
                    }
                    let (key, code, text, modifiers) = match key {
                        EditorKey::Backspace => ("Backspace", "Backspace", "", 0),
                        EditorKey::Enter => ("Enter", "Enter", "\r", 0),
                        EditorKey::SelectAll => ("a", "KeyA", "", 2),
                    };
                    for (event, text) in [("keydown", text), ("keyup", "")] {
                        let outcome = page.dispatch_key_event_with_outcome_async(
                            event, key, code, text, modifiers, false, !text.is_empty(),
                        ).await?;
                        if let Some(download) = outcome.pending_download {
                            downloads.start(download);
                        }
                    }
                    let editor = page.viewport_editor().await?;
                    editor_state.send_modify(|current| current.state = editor.map(|editor| (document, editor)));
                }
                Some(Command::Resize { width, height, scale, screen, revision }) => {
                    let surface = ViewportSurface {
                        inner_width: width,
                        inner_height: height,
                        outer_width: width,
                        outer_height: height,
                        device_pixel_ratio: scale,
                        screen_width: screen.0,
                        screen_height: screen.1,
                        screen_avail_width: screen.0,
                        screen_avail_height: screen.1,
                        screen_orientation: moli_core::protocol_types::ScreenOrientation {
                            kind: "portrait-primary",
                            angle: 0,
                        },
                        ..ViewportSurface::default()
                    };
                    viewport = Some((surface, revision));
                    if active {
                        capture = Some(subscribe_viewport(&mut page, surface, revision, &output).await?);
                        last_edit_frame_sequence = 0;
                    }
                }
                Some(Command::Tap(input)) if active => {
                    for (event, buttons) in [("mousedown", 1), ("mouseup", 0)] {
                        let outcome = page.dispatch_viewport_pointer(RendererViewportPointerInput {
                            document: input.document,
                            viewport_revision: input.revision,
                            position: input.position,
                            event,
                            button: 0,
                            buttons,
                            click_count: 1,
                            modifiers: 0,
                        }).await?;
                        if let Some(download) = outcome.and_then(|value| value.pending_download) {
                            downloads.start(download);
                        }
                    }
                    let editor = page.viewport_editor().await?;
                    editor_state.send_modify(|current| current.state = editor.map(|editor| (input.document, editor)));
                }
                Some(Command::Scroll(input, delta)) if active => {
                    page.enqueue_viewport_wheel(RendererViewportWheelInput {
                        document: input.document,
                        viewport_revision: input.revision,
                        position: input.position,
                        buttons: 0,
                        delta,
                        modifiers: 0,
                        phase: RendererViewportWheelPhase::Unspecified,
                        precise: true,
                    })?;
                }
                Some(_) => {}
                None => break,
            }
        }
    }
    if let Some(task) = navigation {
        task.abort();
    }
    Ok(())
}

async fn subscribe_viewport(
    page: &mut Page,
    surface: ViewportSurface,
    revision: u64,
    output: &Arc<ViewportOutput>,
) -> Result<BufferedViewportFrames> {
    let layout = ViewportBuffers::layout_for(surface)?;
    let storage = std::array::from_fn(|_| Vec::with_capacity(layout.byte_len));
    let buffers = ViewportBuffers::new(layout, storage)?;
    page.subscribe_viewport_buffer(surface, revision, buffers, output.clone())
        .await
}

#[unsafe(no_mangle)]
unsafe extern "C" fn molio_create(
    profile: *const c_char,
    download_directory: *const c_char,
) -> *mut Molio {
    if profile.is_null() || download_directory.is_null() {
        return std::ptr::null_mut();
    }
    let (Ok(profile), Ok(download_directory)) = (
        (unsafe { CStr::from_ptr(profile) }).to_str(),
        (unsafe { CStr::from_ptr(download_directory) }).to_str(),
    ) else {
        return std::ptr::null_mut();
    };
    let profile = PathBuf::from(profile);
    let (tabs, receiver) = mpsc::unbounded_channel();
    let popups = PopupQueue::default();
    let host_popups = popups.clone();
    let (downloads, download_commands, download_entries) =
        DownloadClient::channel(PathBuf::from(download_directory));
    let download_directory = downloads.directory.clone();
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return std::ptr::null_mut();
    };
    let worker = thread::Builder::new()
        .name("molio-ios".into())
        .spawn(move || {
            LocalSet::new().block_on(
                &runtime,
                run_host(
                    receiver,
                    profile,
                    host_popups,
                    download_commands,
                    download_entries,
                    download_directory,
                ),
            )
        });
    if worker.is_err() {
        return std::ptr::null_mut();
    }
    open_tab(tabs, popups, downloads, None)
}

fn open_tab(
    tabs: mpsc::UnboundedSender<OpenTab>,
    popups: PopupQueue,
    downloads: DownloadClient,
    popup: Option<Box<RendererPopupActivationParts>>,
) -> *mut Molio {
    let (commands, receiver) = mpsc::unbounded_channel();
    let (status, status_rx) = watch::channel("Starting…".into());
    let (navigation, navigation_rx) = watch::channel(None);
    let (editor, editor_rx) = watch::channel(EditorUpdate::default());
    let auxiliary = popup
        .as_ref()
        .and_then(|popup| popup.auxiliary_window.clone());
    let viewport = Arc::new(ViewportOutput::default());
    if tabs
        .send(OpenTab {
            popup,
            editor,
            commands: receiver,
            output: viewport.clone(),
            status,
            navigation,
        })
        .is_err()
    {
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(Molio {
        tabs,
        popups,
        downloads,
        auxiliary,
        editor: editor_rx,
        displayed_editor: None,
        edits: RefCell::default(),
        commands,
        frames: viewport.subscribe(),
        viewport,
        status: status_rx,
        navigation: navigation_rx,
        revision: 0,
        presented: None,
    }))
}

#[unsafe(no_mangle)]
extern "C" fn molio_new_tab(handle: &Molio) -> *mut Molio {
    open_tab(
        handle.tabs.clone(),
        handle.popups.clone(),
        handle.downloads.clone(),
        None,
    )
}

#[unsafe(no_mangle)]
extern "C" fn molio_pop_tab(handle: &Molio) -> *mut Molio {
    let popup = handle.popups.lock().pop_front();
    let Some(popup) = popup else {
        return std::ptr::null_mut();
    };
    open_tab(
        handle.tabs.clone(),
        handle.popups.clone(),
        handle.downloads.clone(),
        Some(popup),
    )
}

#[unsafe(no_mangle)]
extern "C" fn molio_tab_closed(handle: &Molio) -> bool {
    handle
        .auxiliary
        .as_ref()
        .is_some_and(RendererAuxiliaryWindow::is_closed)
}

#[unsafe(no_mangle)]
extern "C" fn molio_download_command(handle: &Molio, id: u64, action: u32) {
    let command = match action {
        0 => DownloadCommand::Cancel(id),
        1 => DownloadCommand::Remove(id),
        2 => DownloadCommand::Clear,
        _ => return,
    };
    handle.downloads.send(command);
}

type ShowDownload = extern "C" fn(*mut c_void, u64, *const c_char, *const c_char, u32, u64, u64);

#[unsafe(no_mangle)]
extern "C" fn molio_list_downloads(handle: &Molio, context: *mut c_void, show: ShowDownload) {
    for item in handle.downloads.entries.borrow().iter().rev() {
        let Ok(name) = CString::new(item.name.replace('\0', "�")) else {
            continue;
        };
        let (state, path) = match &item.state {
            DownloadState::Downloading => (0, ""),
            DownloadState::Complete(path) => (1, path.to_str().unwrap_or("")),
            DownloadState::Canceled => (2, ""),
            DownloadState::Failed(error) => (3, error.as_str()),
        };
        let Ok(path) = CString::new(path.replace('\0', "�")) else {
            continue;
        };
        show(
            context,
            item.id,
            name.as_ptr(),
            path.as_ptr(),
            state,
            item.received,
            item.total.unwrap_or(0),
        );
    }
}

#[unsafe(no_mangle)]
extern "C" fn molio_set_active(handle: &Molio, active: bool) {
    if !active {
        handle.edits.borrow_mut().clear();
    }
    let _ = handle.commands.send(Command::Activity(active));
}

#[unsafe(no_mangle)]
extern "C" fn molio_history(handle: &Molio, delta: i32) {
    let _ = handle.commands.send(Command::History(i64::from(delta)));
}

#[unsafe(no_mangle)]
extern "C" fn molio_reload(handle: &Molio, stop: bool) {
    let command = if stop {
        Command::Stop
    } else {
        Command::Navigate(None)
    };
    let _ = handle.commands.send(command);
}

#[unsafe(no_mangle)]
unsafe extern "C" fn molio_destroy(handle: *mut Molio) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn molio_navigate(handle: &Molio, url: *const c_char) {
    if !url.is_null()
        && let Ok(url) = (unsafe { CStr::from_ptr(url) }).to_str()
    {
        let _ = handle
            .commands
            .send(Command::Navigate(Some(url.to_owned())));
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn molio_open_address(handle: &Molio, input: *const c_char) {
    if !input.is_null()
        && let Ok(input) = (unsafe { CStr::from_ptr(input) }).to_str()
        && let Some(url) = super::address_target::address_target(input)
    {
        let _ = handle
            .commands
            .send(Command::Navigate(Some(url.to_string())));
    }
}

#[unsafe(no_mangle)]
extern "C" fn molio_resize(
    handle: &mut Molio,
    width: u32,
    height: u32,
    scale: f64,
    screen_width: u32,
    screen_height: u32,
) {
    if width > 0 && height > 0 && scale.is_finite() && scale > 0. {
        handle.revision += 1;
        handle.presented = None;
        let _ = handle.commands.send(Command::Resize {
            width,
            height,
            scale,
            screen: (screen_width, screen_height),
            revision: handle.revision,
        });
    }
}

impl Molio {
    fn input(&self, x: f64, y: f64, command: impl FnOnce(InputPosition) -> Command) {
        if let Some((document, revision)) = self.presented {
            let _ = self.commands.send(command(InputPosition {
                document,
                revision,
                position: (x, y),
            }));
        }
    }
}

#[unsafe(no_mangle)]
extern "C" fn molio_tap(handle: &Molio, x: f64, y: f64) {
    handle.input(x, y, Command::Tap);
}

#[unsafe(no_mangle)]
extern "C" fn molio_scroll(handle: &Molio, x: f64, y: f64, delta_x: f64, delta_y: f64) {
    handle.input(x, y, |input| Command::Scroll(input, (delta_x, delta_y)));
}

impl Molio {
    fn edit(&self, action: RendererViewportEditAction) {
        if let Some((document, editor)) = &self.displayed_editor {
            let edit = self.edits.borrow_mut().submit(*document, editor, action);
            let _ = self.commands.send(Command::Edit(edit));
        }
    }

    fn refresh_displayed_editor(&mut self) -> bool {
        if !self.editor.has_changed().unwrap_or(false) {
            return false;
        }
        let update = self.editor.borrow_and_update();
        let target = |state: &EditorState| {
            state
                .as_ref()
                .map(|(document, editor)| (*document, editor.target))
        };
        let same_target = target(&self.displayed_editor) == target(&update.state);
        if !self.edits.get_mut().accept_editor(same_target, update.ack) {
            return false;
        }
        self.displayed_editor.clone_from(&update.state);
        true
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn molio_insert_text(
    handle: &Molio,
    text: *const c_char,
    len: usize,
    marked: bool,
    selection_start: usize,
    selection_end: usize,
) {
    let bytes = if len == 0 {
        &[]
    } else if text.is_null() || len > isize::MAX as usize {
        return;
    } else {
        unsafe { std::slice::from_raw_parts(text.cast::<u8>(), len) }
    };
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    let text = text.to_owned();
    let action = if marked {
        RendererViewportEditAction::Compose {
            range: None,
            text,
            selection: Some(selection_start..selection_end),
        }
    } else {
        RendererViewportEditAction::Commit { range: None, text }
    };
    handle.edit(action);
}

#[unsafe(no_mangle)]
extern "C" fn molio_finish_composition(handle: &Molio) {
    handle.edit(RendererViewportEditAction::Finish);
}

#[unsafe(no_mangle)]
extern "C" fn molio_editor_key(handle: &Molio, key: u8) {
    let key = match key {
        0 => EditorKey::Backspace,
        1 => EditorKey::Enter,
        2 => EditorKey::SelectAll,
        _ => return,
    };
    if let Some((document, editor)) = &handle.displayed_editor {
        let _ = handle.commands.send(Command::EditorKey {
            document: *document,
            target: editor.target,
            key,
        });
    }
}

type Present = extern "C" fn(*mut c_void, *const u8, usize, u32, u32, usize);
type ShowStatus = extern "C" fn(*mut c_void, *const c_char);
type ShowEditor = extern "C" fn(*mut c_void, u32, *const c_char, bool, usize, usize, *const f32);
type ShowNavigation = extern "C" fn(*mut c_void, *const c_char, *const c_char, bool, bool);

#[unsafe(no_mangle)]
extern "C" fn molio_poll(
    handle: &mut Molio,
    context: *mut c_void,
    present: Present,
    status: ShowStatus,
    navigation: ShowNavigation,
    editor: ShowEditor,
) {
    if handle.status.has_changed().unwrap_or(true) {
        let message = handle.status.borrow_and_update().replace('\0', "�");
        if let Ok(message) = CString::new(message) {
            status(context, message.as_ptr());
        }
    }
    if handle.navigation.has_changed().unwrap_or(false)
        && let Some(state) = handle.navigation.borrow_and_update().as_ref()
    {
        let metadata = &state.navigation;
        let address = CString::new(metadata.url.as_str()).expect("URL contains no NUL");
        let title = CString::new(metadata.title.replace('\0', "�")).expect("NULs replaced");
        navigation(
            context,
            address.as_ptr(),
            title.as_ptr(),
            metadata.can_go_back,
            metadata.can_go_forward,
        );
    }
    if handle.refresh_displayed_editor() {
        if let Some((_, current)) = &handle.displayed_editor {
            let text = CString::new(current.text.replace('\0', "�")).expect("NULs replaced");
            editor(
                context,
                current.target,
                text.as_ptr(),
                current.password,
                current.selection.start.saturating_sub(current.text_start),
                current.selection.end.saturating_sub(current.text_start),
                current
                    .caret_rect
                    .as_ref()
                    .map_or(std::ptr::null(), |rect| rect.as_ptr()),
            );
        } else {
            editor(context, 0, std::ptr::null(), false, 0, 0, std::ptr::null());
        }
    }
    if handle.frames.has_changed().unwrap_or(false) {
        handle.frames.borrow_and_update();
        let revision = handle.revision;
        handle.presented = handle
            .viewport
            .read(|frame| {
                if frame.viewport_revision == revision
                    && let Some(pixels) = frame.pixels
                {
                    present(
                        context,
                        pixels.as_ptr(),
                        pixels.len(),
                        frame.layout.width,
                        frame.layout.height,
                        frame.layout.width as usize * 4,
                    );
                    return Some((frame.document, revision));
                }
                None
            })
            .flatten();
        if handle.presented.is_none() {
            present(context, std::ptr::null(), 0, 0, 0, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use anyhow::Context as _;

    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(8);

    fn open_test_tab(root: &tempfile::TempDir, html: &str) -> Result<Box<Molio>> {
        let profile = CString::new(root.path().join("profile").to_str().unwrap())?;
        let downloads = CString::new(root.path().join("downloads").to_str().unwrap())?;
        let handle = unsafe { molio_create(profile.as_ptr(), downloads.as_ptr()) };
        anyhow::ensure!(!handle.is_null(), "could not start the iOS browser worker");
        let mut tab = unsafe { Box::from_raw(handle) };
        let url = CString::new(format!("data:text/html,{html}"))?;
        unsafe { molio_navigate(&tab, url.as_ptr()) };
        molio_resize(&mut tab, 40, 30, 1., 40, 30);
        Ok(tab)
    }

    async fn wait_for_frame(
        tab: &mut Molio,
        color: [u8; 4],
    ) -> Result<RendererDocumentLifecycleIdentity> {
        tokio::time::timeout(TIMEOUT, async {
            loop {
                let document = tab.viewport.read(|frame| {
                    let pixels = frame.pixels?;
                    (frame.viewport_revision == tab.revision && pixels.starts_with(&color))
                        .then_some(frame.document)
                });
                if let Some(document) = document.flatten() {
                    return Ok(document);
                }
                tab.frames.changed().await?;
            }
        })
        .await
        .with_context(|| format!("waiting for a frame with BGRA {color:?}"))?
    }

    #[tokio::test]
    async fn reactivated_tab_presents_a_new_frame_without_resizing() -> Result<()> {
        let root = tempfile::tempdir()?;
        let mut tab = open_test_tab(
            &root,
            concat!(
                "<style>html{background:red}body{margin:0;min-height:100vh}</style>",
                "<script>document.addEventListener('mousedown', () => {",
                "  document.documentElement.style.background = 'blue';",
                "});</script>",
            ),
        )?;
        let initial_document = wait_for_frame(&mut tab, [0, 0, 255, 255]).await?;
        let initial_revision = tab.revision;

        molio_set_active(&tab, false);
        tokio::time::timeout(TIMEOUT, async {
            while tab.viewport.read(|_| ()).is_some() {
                tab.frames.changed().await?;
            }
            Ok::<_, watch::error::RecvError>(())
        })
        .await
        .context("waiting for the hidden tab to release its viewport")??;

        molio_set_active(&tab, true);
        let resumed_document = wait_for_frame(&mut tab, [0, 0, 255, 255]).await?;
        assert_eq!(resumed_document, initial_document);
        assert_eq!(tab.revision, initial_revision);

        tab.presented = Some((resumed_document, initial_revision));
        molio_tap(&tab, 10., 10.);
        assert_eq!(
            wait_for_frame(&mut tab, [255, 0, 0, 255]).await?,
            initial_document
        );
        Ok(())
    }

    fn insert_text(tab: &Molio, text: &str, marked: bool) {
        let caret = text.encode_utf16().count();
        unsafe { molio_insert_text(tab, text.as_ptr().cast(), text.len(), marked, caret, caret) };
    }

    async fn wait_for_edit_ack(
        updates: &mut watch::Receiver<EditorUpdate>,
        sequence: u64,
    ) -> Result<RendererViewportEditor> {
        let update = tokio::time::timeout(
            TIMEOUT,
            updates.wait_for(|update| update.ack == Some(sequence)),
        )
        .await
        .with_context(|| format!("waiting for edit {sequence}"))??;
        update
            .state
            .as_ref()
            .map(|(_, editor)| editor.clone())
            .context("the input should still be focused")
    }

    #[tokio::test]
    async fn ime_chains_queued_edits_and_rejects_a_stale_editor_revision() -> Result<()> {
        let root = tempfile::tempdir()?;
        let mut tab = open_test_tab(
            &root,
            concat!(
                "<input id='field' value='abc'><script>",
                "const field = document.getElementById('field');",
                "field.focus(); field.setSelectionRange(3, 3);",
                "field.addEventListener('keydown', event => {",
                "  if (event.key !== 'a') return;",
                "  event.preventDefault();",
                "  field.value = 'script'; field.setSelectionRange(0, 0);",
                "});</script>",
            ),
        )?;
        let mut updates = tab.editor.clone();
        tokio::time::timeout(TIMEOUT, updates.wait_for(|update| update.state.is_some()))
            .await
            .context("waiting for the focused input")??;
        assert!(tab.refresh_displayed_editor());

        insert_text(&tab, "n", true);
        insert_text(&tab, "ni", true);
        insert_text(&tab, "你", false);
        let editor = wait_for_edit_ack(&mut updates, 3).await?;
        assert_eq!(editor.text, "abc你");
        assert_eq!(editor.selection, 4..4);
        assert!(editor.marked.is_none());
        assert!(tab.refresh_displayed_editor());

        molio_editor_key(&tab, 2);
        tokio::time::timeout(
            TIMEOUT,
            updates.wait_for(|update| {
                update.state.as_ref().is_some_and(|(_, editor)| {
                    editor.text == "script" && editor.selection == (0..0)
                })
            }),
        )
        .await
        .context("waiting for the script to change the input and caret")??;

        // UIKit still has the snapshot from before the key event.
        insert_text(&tab, "wrong", true);
        let editor = wait_for_edit_ack(&mut updates, 4).await?;
        assert_eq!(editor.text, "script");
        assert_eq!(editor.selection, 0..0);
        assert!(editor.marked.is_none());
        assert!(tab.refresh_displayed_editor());

        insert_text(&tab, "new ", false);
        assert_eq!(wait_for_edit_ack(&mut updates, 5).await?.text, "new script");
        Ok(())
    }
}
