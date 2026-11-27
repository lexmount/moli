use std::{future::pending, ops::Range, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use moli_core::{
    RendererDocumentLifecycleIdentity, RendererViewportEdit, RendererViewportEditAction,
    RendererViewportEditor, RendererViewportWheelInput, RendererViewportWheelPhase,
    page::{
        BufferedViewportFrames, CompletedPageCommand, Page, PageInputExt,
        RendererInputDispatchOutcome, RendererPageWindowIdentity,
        RendererPendingFileChooserActivation, RendererPopupActivationParts, SelectedFile,
        ViewportBuffers, ViewportOutput, ViewportUpdate,
    },
    protocol_types::{DocumentActivity, ViewportSurface},
    runtime::Browser,
};
use parking_lot::Mutex;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    Shared,
    downloads::Downloads,
    model::{Edit, EditorState, Geometry, PickedFile, ScrollPhase, TouchPhase, Visit},
};

pub enum PageStart {
    New,
    Popup(Box<RendererPopupActivationParts>),
}

pub enum TabCommand {
    Navigate(Option<String>),
    History(i64),
    Stop,
    Present {
        geometry: Option<Geometry>,
        zoom: f64,
    },
    Touch {
        phase: TouchPhase,
        x: f64,
        y: f64,
    },
    Scroll {
        phase: ScrollPhase,
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
    },
    Edit {
        token: u64,
        edit: Edit,
    },
    Files {
        activation: RendererPendingFileChooserActivation,
        files: Vec<PickedFile>,
    },
}

pub enum TabEvent {
    Visit(Visit),
    ChooseFiles {
        tab: u64,
        activation: RendererPendingFileChooserActivation,
    },
}

pub struct TabHandle {
    pub commands: mpsc::UnboundedSender<TabCommand>,
    pub viewport: Arc<ViewportOutput>,
    pub document: Arc<Mutex<Option<RendererDocumentLifecycleIdentity>>>,
    pub window: RendererPageWindowIdentity,
    task: JoinHandle<()>,
}

impl TabHandle {
    pub fn spawn(
        id: u64,
        browser: Browser,
        start: PageStart,
        shared: Arc<Shared>,
        events: mpsc::UnboundedSender<TabEvent>,
        downloads: Rc<Downloads>,
        picker_directory: PathBuf,
    ) -> Self {
        let window = match &start {
            PageStart::New => RendererPageWindowIdentity::default(),
            PageStart::Popup(parts) => parts.window_identity().unwrap_or_default(),
        };
        let (commands, receiver) = mpsc::unbounded_channel();
        let viewport = Arc::new(ViewportOutput::default());
        let document = Arc::new(Mutex::new(None));
        let worker = TabWorker {
            id,
            shared,
            events,
            downloads,
            picker_directory,
            viewport: viewport.clone(),
            document: document.clone(),
        };
        let page_identity = window.clone();
        let task = tokio::task::spawn_local(async move {
            if let Err(error) = worker.run(browser, start, page_identity, receiver).await {
                worker.update(|state| {
                    state.loading = false;
                    state.error = Some(format!("{error:#}"));
                });
            }
        });
        Self {
            commands,
            viewport,
            document,
            window,
            task,
        }
    }

    pub fn send(&self, command: TabCommand) {
        let _ = self.commands.send(command);
    }
}

impl Drop for TabHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct TabWorker {
    id: u64,
    shared: Arc<Shared>,
    events: mpsc::UnboundedSender<TabEvent>,
    downloads: Rc<Downloads>,
    picker_directory: PathBuf,
    viewport: Arc<ViewportOutput>,
    document: Arc<Mutex<Option<RendererDocumentLifecycleIdentity>>>,
}

struct Tab {
    page: Page,
    capture: Option<BufferedViewportFrames>,
    navigation: Option<JoinHandle<Result<CompletedPageCommand>>>,
    geometry: Option<Geometry>,
    viewport_revision: u64,
    zoom: f64,
    editor: Option<RendererViewportEditor>,
    editor_token: u64,
    edit_sequence: u64,
}

impl Drop for Tab {
    fn drop(&mut self) {
        if let Some(navigation) = self.navigation.take() {
            navigation.abort();
        }
    }
}

impl TabWorker {
    async fn run(
        &self,
        browser: Browser,
        start: PageStart,
        window: RendererPageWindowIdentity,
        mut commands: mpsc::UnboundedReceiver<TabCommand>,
    ) -> Result<()> {
        let mut page = tokio::time::timeout(Duration::from_secs(20), async {
            match start {
                PageStart::New => browser.create_viewport_page(window).await,
                PageStart::Popup(parts) => browser.adopt_viewport_page(&parts).await,
            }
        })
        .await
        .context("opening tab timed out")??;
        page.set_document_activity_async(DocumentActivity::new(false, false))
            .await?;
        let mut navigation_updates = page.subscribe_navigation_state().await?;
        navigation_updates.mark_changed();
        let mut frame_updates = self.viewport.subscribe();
        let mut tab = Tab {
            page,
            capture: None,
            navigation: None,
            geometry: None,
            viewport_revision: 0,
            zoom: 1.0,
            editor: None,
            editor_token: 0,
            edit_sequence: 0,
        };

        loop {
            tokio::select! {
                change = navigation_updates.changed() => {
                    change?;
                    let next = navigation_updates.borrow_and_update().clone();
                    if let Some(next) = next {
                        let navigation = next.navigation;
                        let document_changed = {
                            let mut document = self.document.lock();
                            document.replace(next.document) != Some(next.document)
                        };
                        if document_changed {
                            tab.editor = None;
                            tab.editor_token += 1;
                        }
                        let url = navigation.url.to_string();
                        self.update(|state| {
                            state.url.clone_from(&url);
                            state.title.clone_from(&navigation.title);
                            state.can_go_back = navigation.can_go_back;
                            state.can_go_forward = navigation.can_go_forward;
                            if document_changed {
                                state.editor = None;
                                state.error = None;
                            }
                        });
                        if crate::model::web_url(&url) {
                            let _ = self.events.send(TabEvent::Visit(Visit { url, title: navigation.title }));
                        }
                    }
                }
                result = finish_navigation(&mut tab.navigation) => {
                    tab.navigation = None;
                    let result = result.and_then(|completion| tab.page.finish_viewport_navigation(completion));
                    self.update(|state| {
                        state.loading = false;
                        if let Err(error) = result {
                            state.error = Some(format!("{error:#}"));
                        } else {
                            state.pending_url = None;
                        }
                    });
                }
                change = viewport_changed(&mut tab.capture) => {
                    match change {
                        Ok(ViewportUpdate::StateChanged) => {
                            let state = tab.capture.as_mut().and_then(BufferedViewportFrames::latest_state);
                            if let Some(state) = state
                                && self.current_document() == Some(state.document)
                            {
                                self.set_editor(&mut tab, state.editor);
                            }
                        }
                        Ok(ViewportUpdate::ResourceChanged) => { tab.page.wake_viewport_frames().await?; }
                        Err(_) => { tab.capture = None; }
                    }
                }
                result = frame_updates.changed() => {
                    result?;
                    self.shared.signal_frame();
                }
                command = commands.recv() => {
                    let Some(command) = command else { break };
                    if let Err(error) = self.command(&mut tab, command).await {
                        eprintln!("Molio tab {}: {error:#}", self.id);
                        self.update(|state| {
                            state.error = Some(format!("{error:#}"));
                            state.loading = false;
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn current_document(&self) -> Option<RendererDocumentLifecycleIdentity> {
        *self.document.lock()
    }

    fn update(&self, update: impl FnOnce(&mut crate::model::TabState)) {
        self.shared.update(|state| {
            if let Some(tab) = state.tabs.iter_mut().find(|tab| tab.id == self.id) {
                update(tab);
            }
        });
    }

    fn set_editor(&self, tab: &mut Tab, editor: Option<RendererViewportEditor>) {
        let previous = tab.editor.as_ref().map(|editor| editor.target);
        if previous != editor.as_ref().map(|editor| editor.target) {
            tab.editor_token += 1;
        }
        if tab.editor == editor {
            return;
        }
        tab.editor = editor;
        let view = tab.editor.as_ref().map(|editor| EditorState {
            token: tab.editor_token,
            text: if editor.password {
                String::new()
            } else {
                editor.text.clone()
            },
            text_start: editor.text_start,
            start: editor.selection.start,
            end: editor.selection.end,
            password: editor.password,
        });
        self.update(|state| state.editor = view);
    }

    async fn refresh_editor(&self, tab: &mut Tab) -> Result<()> {
        let editor = tab.page.viewport_editor().await?;
        self.set_editor(tab, editor);
        Ok(())
    }

    fn input_outcome(&self, outcome: RendererInputDispatchOutcome) {
        if let Some(download) = outcome.pending_download {
            self.downloads.start(download);
        }
        if let Some(activation) = outcome.pending_file_chooser {
            let _ = self.events.send(TabEvent::ChooseFiles {
                tab: self.id,
                activation,
            });
        }
    }

    async fn command(&self, tab: &mut Tab, command: TabCommand) -> Result<()> {
        match command {
            TabCommand::Navigate(target) => {
                if let Some(navigation) = tab.navigation.take() {
                    navigation.abort();
                }
                let pending = tab.page.start_viewport_navigation(target.clone())?;
                tab.navigation = Some(tokio::task::spawn_local(pending.wait()));
                self.update(|state| {
                    state.loading = true;
                    state.error = None;
                    state.pending_url = target;
                    state.editor = None;
                });
                tab.editor = None;
                tab.editor_token += 1;
            }
            TabCommand::History(delta) => {
                tab.page.traverse_viewport_history(delta).await?;
                self.update(|state| {
                    state.error = None;
                    state.pending_url = None;
                });
            }
            TabCommand::Stop => {
                if let Some(navigation) = tab.navigation.take() {
                    navigation.abort();
                }
                tab.page.stop_document_lifecycle_async().await?;
                self.update(|state| {
                    state.loading = false;
                    state.pending_url = None;
                });
            }
            TabCommand::Present { geometry, zoom } => {
                if (geometry, zoom) != (tab.geometry, tab.zoom) {
                    self.present(tab, geometry, zoom).await?;
                }
            }
            TabCommand::Touch { phase, x, y } if tab.geometry.is_some() => {
                let name = match phase {
                    TouchPhase::Down => "touchstart",
                    TouchPhase::Move => "touchmove",
                    TouchPhase::Up => "touchend",
                    TouchPhase::Cancel => "touchcancel",
                };
                let outcome = tab
                    .page
                    .dispatch_touch_event_at_point_with_outcome_async(
                        x / tab.zoom,
                        y / tab.zoom,
                        name,
                        false,
                    )
                    .await?;
                self.input_outcome(outcome);
                if matches!(phase, TouchPhase::Up) {
                    self.refresh_editor(tab).await?;
                }
            }
            TabCommand::Scroll {
                phase,
                x,
                y,
                dx,
                dy,
            } if tab.geometry.is_some() => {
                if let Some(document) = self.current_document() {
                    let phase = match phase {
                        ScrollPhase::Begin => RendererViewportWheelPhase::Started,
                        ScrollPhase::Move => RendererViewportWheelPhase::Moved,
                        ScrollPhase::End => RendererViewportWheelPhase::Ended,
                        ScrollPhase::Cancel => RendererViewportWheelPhase::Cancelled,
                    };
                    tab.page
                        .enqueue_viewport_wheel(RendererViewportWheelInput {
                            document,
                            viewport_revision: tab.viewport_revision,
                            position: (x / tab.zoom, y / tab.zoom),
                            buttons: 0,
                            delta: (dx / tab.zoom, dy / tab.zoom),
                            modifiers: 0,
                            phase,
                            precise: true,
                        })?;
                }
            }
            TabCommand::Edit { token, edit }
                if token == tab.editor_token && tab.geometry.is_some() =>
            {
                self.edit(tab, edit).await?;
            }
            TabCommand::Files { activation, files } => {
                self.set_files(tab, activation, files).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn present(&self, tab: &mut Tab, geometry: Option<Geometry>, zoom: f64) -> Result<()> {
        let visible = geometry.is_some();
        if visible != tab.geometry.is_some() {
            tab.page
                .set_document_activity_async(DocumentActivity::new(visible, visible))
                .await?;
        }
        tab.geometry = geometry;
        tab.zoom = zoom;
        tab.capture = None;
        let Some(geometry) = geometry else {
            self.set_editor(tab, None);
            return Ok(());
        };
        let width = f64::from(geometry.width) / geometry.dpr;
        let height = f64::from(geometry.height) / geometry.dpr;
        let surface = ViewportSurface {
            inner_width: (width / zoom).round().max(1.0) as u32,
            inner_height: (height / zoom).round().max(1.0) as u32,
            outer_width: width.round() as u32,
            outer_height: height.round() as u32,
            screen_width: width.round() as u32,
            screen_height: height.round() as u32,
            screen_avail_width: width.round() as u32,
            screen_avail_height: height.round() as u32,
            device_pixel_ratio: geometry.dpr * zoom,
            ..ViewportSurface::default()
        };
        let layout = ViewportBuffers::layout_for(surface)?;
        let mut storage = [Vec::new(), Vec::new(), Vec::new()];
        for pixels in &mut storage {
            pixels.try_reserve_exact(layout.byte_len)?;
        }
        let buffers = ViewportBuffers::new(layout, storage)?;
        tab.viewport_revision += 1;
        tab.capture = Some(
            tab.page
                .subscribe_viewport_buffer(
                    surface,
                    tab.viewport_revision,
                    buffers,
                    self.viewport.clone(),
                )
                .await?,
        );
        Ok(())
    }

    async fn edit(&self, tab: &mut Tab, edit: Edit) -> Result<()> {
        let (Some(document), Some(editor)) = (self.current_document(), tab.editor.as_ref()) else {
            return Ok(());
        };
        let action = match edit {
            Edit::Insert { text } => RendererViewportEditAction::Commit { range: None, text },
            Edit::Compose { text, start, end } => RendererViewportEditAction::Compose {
                range: composition_range(start, end),
                text,
                selection: None,
            },
            Edit::Finish => RendererViewportEditAction::Finish,
            Edit::Delete { count, left } => {
                let mut range = editor.selection.clone();
                if range.is_empty() {
                    if left {
                        range.start = range.start.saturating_sub(count);
                    } else {
                        range.end = range
                            .end
                            .saturating_add(count)
                            .min(editor.text_start + editor.text.encode_utf16().count());
                    }
                }
                RendererViewportEditAction::Commit {
                    range: Some(range),
                    text: String::new(),
                }
            }
            Edit::Submit => {
                let outcome = tab
                    .page
                    .dispatch_key_event_with_outcome_async(
                        "keydown", "Enter", "Enter", "\r", 0, false, true,
                    )
                    .await?;
                self.input_outcome(outcome);
                tab.page
                    .dispatch_key_event_with_outcome_async(
                        "keyup", "Enter", "Enter", "", 0, false, false,
                    )
                    .await?;
                return self.refresh_editor(tab).await;
            }
        };
        tab.edit_sequence += 1;
        let reply = tab
            .page
            .edit_viewport(RendererViewportEdit {
                document,
                target: editor.target,
                revision: editor.revision,
                sequence: tab.edit_sequence,
                previous: None,
                action,
            })
            .await?;
        self.set_editor(tab, reply.editor);
        Ok(())
    }

    async fn set_files(
        &self,
        tab: &mut Tab,
        activation: RendererPendingFileChooserActivation,
        files: Vec<PickedFile>,
    ) -> Result<()> {
        let directory = self.picker_directory.clone();
        let limit = if activation.allow_multiple() { 32 } else { 1 };
        let files = tokio::task::spawn_blocking(move || {
            files
                .into_iter()
                .take(limit)
                .filter_map(|file| {
                    let path = PathBuf::from(file.path);
                    if path.parent() != Some(directory.as_path()) {
                        return None;
                    }
                    use std::io::Read;
                    let data = std::fs::File::open(&path)
                        .and_then(|file| {
                            let mut contents = Vec::new();
                            file.take(32 * 1024 * 1024 + 1).read_to_end(&mut contents)?;
                            (contents.len() <= 32 * 1024 * 1024)
                                .then_some(contents)
                                .ok_or_else(|| std::io::Error::other("selected file exceeds 32 MB"))
                        })
                        .ok();
                    let _ = std::fs::remove_file(path);
                    Some(SelectedFile {
                        data: data?.into(),
                        mime_type: moli_web_mime::selected_file_mime_type(&file.name).to_owned(),
                        name: file.name,
                        relative_path: String::new(),
                        last_modified: 0.0,
                    })
                })
                .collect::<Vec<_>>()
        })
        .await?;
        if self.current_document() != Some(activation.source_document()) || files.is_empty() {
            return Ok(());
        }
        let pending = tab.page.start_set_file_input_files_for_backend_node_id(
            activation.backend_node_id(),
            files,
            false,
        )?;
        tab.page
            .finish_set_file_input_files(pending.wait().await?)?;
        Ok(())
    }
}

fn composition_range(start: i32, end: i32) -> Option<Range<usize>> {
    (start >= 0 && end >= start).then_some(start as usize..end as usize)
}

async fn finish_navigation(
    navigation: &mut Option<JoinHandle<Result<CompletedPageCommand>>>,
) -> Result<CompletedPageCommand> {
    match navigation {
        Some(navigation) => navigation.await?,
        None => pending().await,
    }
}

async fn viewport_changed(capture: &mut Option<BufferedViewportFrames>) -> Result<ViewportUpdate> {
    match capture {
        Some(capture) => Ok(capture.changed().await?),
        None => pending().await,
    }
}
