use std::{
    collections::{HashMap, VecDeque},
    fs,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use moli_core::{
    LayoutPolicy, OptionalResourceFetchMask, RendererDocumentLifecycleIdentity, RendererOutputItem,
    RendererOutputTransportMessage, RendererOwnerAction,
    page::{
        RendererPendingFileChooserActivation, RendererPendingJavaScriptDialog,
        RendererPopupDisposition,
    },
    runtime::{Browser, BrowserConfig},
};
use tokio::sync::mpsc;

use crate::{
    Shared,
    downloads::{self, DownloadClient, DownloadCommand, Downloads},
    model::{
        self, Command, DownloadState, Geometry, HOME, MAX_TABS, Prompt, SavedTab, Session,
        TabState, Visit,
    },
    tab::{PageStart, TabCommand, TabEvent, TabHandle},
};

pub fn run(
    profile: PathBuf,
    commands: mpsc::UnboundedReceiver<Command>,
    shared: Arc<Shared>,
) -> Result<()> {
    fs::create_dir_all(&profile).context("create the Molio profile")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    tokio::task::LocalSet::new().block_on(&runtime, run_browser(profile, commands, shared))
}

async fn run_browser(
    profile: PathBuf,
    mut commands: mpsc::UnboundedReceiver<Command>,
    shared: Arc<Shared>,
) -> Result<()> {
    let mut config = BrowserConfig::default()
        .with_layout_policy(LayoutPolicy::Realtime)
        .with_optional_resource_fetch_mask(
            OptionalResourceFetchMask::IMAGE | OptionalResourceFetchMask::FONT,
        );
    config.set_realtime_max_fps(60);
    config.set_profile_dir(Some(profile.clone()));
    let browser = Browser::new(config).context("initialize the Moli engine")?;
    let mut renderer_events = browser.subscribe_viewport_actions();
    let (download_client, mut download_commands, download_entries) =
        DownloadClient::channel(profile.join("Downloads"));
    let downloads = Downloads::new(
        browser.resource_request_client(),
        download_entries,
        download_client.directory.clone(),
        Some(&profile),
    );
    let mut download_updates = download_client.entries.clone();
    download_updates.mark_changed();
    let (tab_events, mut tab_updates) = mpsc::unbounded_channel();
    let session_path = profile.join("mobile-session.json");
    let saved = Session::load(&session_path);
    let picker_directory = profile.join("FilePicker");
    fs::create_dir_all(&picker_directory).context("create the file chooser directory")?;
    let mut host = BrowserHost {
        browser,
        shared,
        downloads,
        download_client,
        tab_events,
        picker_directory,
        session_path,
        tabs: HashMap::new(),
        next_tab: 1,
        selected: 0,
        visible: true,
        geometry: Geometry::default(),
        prompts: VecDeque::new(),
        next_prompt: 1,
    };
    host.shared.update(|state| {
        state.history = saved
            .history
            .into_iter()
            .filter(|item| model::web_url(&item.url))
            .take(200)
            .collect();
        state.bookmarks = saved
            .bookmarks
            .into_iter()
            .filter(|item| model::web_url(&item.url))
            .take(200)
            .collect();
    });
    let selected = saved
        .selected
        .min(saved.tabs.len().min(MAX_TABS).saturating_sub(1));
    for (index, saved) in saved.tabs.into_iter().take(MAX_TABS).enumerate() {
        let url = model::web_url(&saved.url).then_some(saved.url);
        host.open_tab(
            PageStart::New,
            url,
            saved.zoom.clamp(model::MIN_ZOOM, model::MAX_ZOOM),
            index == selected,
        );
    }
    if host.tabs.is_empty() {
        host.open_tab(PageStart::New, None, 1.0, true);
    }
    host.shared.update(|state| state.ready = true);

    let mut needs_save = false;
    let mut save_tick = tokio::time::interval(Duration::from_millis(400));
    save_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                None | Some(Command::Shutdown) => break,
                Some(command) => { needs_save |= host.command(command); }
            },
            event = tab_updates.recv() => if let Some(event) = event {
                match event {
                    TabEvent::Visit(visit) => {
                        host.shared.update(|state| {
                            state.history.retain(|item| item.url != visit.url);
                            state.history.insert(0, visit);
                            state.history.truncate(200);
                        });
                        needs_save = true;
                    }
                    TabEvent::ChooseFiles { tab, activation } => host.offer_file_picker(tab, activation),
                }
            },
            event = renderer_events.recv(), if !renderer_events.is_closed() => {
                if let Some(RendererOutputTransportMessage::Publication(publication)) = event {
                    for item in publication.into_records() {
                        if let RendererOutputItem::OwnerAction(action) = item.into_parts().1 {
                            needs_save |= host.renderer_action(action);
                        }
                    }
                }
            },
            _ = download_updates.changed() => {
                let entries = download_updates.borrow_and_update().iter().map(download_state).collect();
                host.shared.update(|state| state.downloads = entries);
            }
            command = download_commands.recv() => if let Some(command) = command {
                host.downloads.command(command);
            },
            _ = save_tick.tick() => {
                if needs_save {
                    host.save();
                    needs_save = false;
                }
            }
        }
    }

    host.save();
    for (_, prompt) in host.prompts.drain(..) {
        if let PendingPrompt::Dialog(dialog) = prompt {
            dialog.finish(false, String::new());
        }
    }
    host.tabs.clear();
    tokio::task::yield_now().await;
    let _ = tokio::time::timeout(Duration::from_secs(2), host.downloads.shutdown()).await;
    Ok(())
}

struct BrowserHost {
    browser: Browser,
    shared: Arc<Shared>,
    downloads: Rc<Downloads>,
    download_client: DownloadClient,
    tab_events: mpsc::UnboundedSender<TabEvent>,
    picker_directory: PathBuf,
    session_path: PathBuf,
    tabs: HashMap<u64, TabHandle>,
    next_tab: u64,
    selected: u64,
    visible: bool,
    geometry: Geometry,
    prompts: VecDeque<(Prompt, PendingPrompt)>,
    next_prompt: u64,
}

enum PendingPrompt {
    Dialog(RendererPendingJavaScriptDialog),
    File {
        tab: u64,
        activation: RendererPendingFileChooserActivation,
    },
}

impl BrowserHost {
    fn open_tab(&mut self, start: PageStart, url: Option<String>, zoom: f64, foreground: bool) {
        if self.tabs.len() >= MAX_TABS {
            self.shared
                .update(|state| state.error = Some("标签页已达上限".into()));
            return;
        }
        let id = self.next_tab;
        self.next_tab += 1;
        let handle = TabHandle::spawn(
            id,
            self.browser.clone(),
            start,
            self.shared.clone(),
            self.tab_events.clone(),
            self.downloads.clone(),
            self.picker_directory.clone(),
        );
        self.tabs.insert(id, handle);
        self.shared.update(|state| {
            state.error = None;
            state.tabs.push(TabState::new(id, zoom));
        });
        if foreground {
            self.select_tab(id);
        }
        if let Some(url) = url {
            self.send(id, TabCommand::Navigate(Some(url)));
        }
    }

    fn select_tab(&mut self, id: u64) {
        if self.selected == id || !self.tabs.contains_key(&id) {
            return;
        }
        self.send(
            self.selected,
            TabCommand::Present {
                geometry: None,
                zoom: 1.0,
            },
        );
        self.selected = id;
        self.shared.update(|state| state.selected_tab = id);
        self.present_selected();
    }

    fn close_tab(&mut self, id: u64) {
        let Some(tab) = self.tabs.remove(&id) else {
            return;
        };
        if let Some(window) = tab.window.auxiliary_window() {
            window.close();
        }
        drop(tab);
        self.shared
            .update(|state| state.tabs.retain(|tab| tab.id != id));
        if self.selected == id {
            let next = self.shared.state.lock().tabs.last().map(|tab| tab.id);
            if let Some(id) = next {
                self.select_tab(id);
            } else {
                self.open_tab(PageStart::New, None, 1.0, true);
            }
        }
    }

    fn send(&self, id: u64, command: TabCommand) {
        if let Some(tab) = self.tabs.get(&id) {
            tab.send(command);
        }
    }

    fn present_selected(&self) {
        let geometry = (self.visible && self.geometry.available()).then_some(self.geometry);
        let zoom = self
            .shared
            .state
            .lock()
            .tabs
            .iter()
            .find(|tab| tab.id == self.selected)
            .map_or(1.0, |tab| tab.zoom);
        let viewport = geometry
            .and_then(|_| self.tabs.get(&self.selected))
            .map(|tab| tab.viewport.clone());
        *self.shared.viewport.lock() = viewport;
        self.send(self.selected, TabCommand::Present { geometry, zoom });
        self.shared.signal_frame();
    }

    fn open_address(&self, tab: u64, input: &str) {
        if let Some(url) = model::address_target(input) {
            self.send(tab, TabCommand::Navigate(Some(url)));
        } else {
            self.shared.update(|state| {
                if let Some(tab) = state.tabs.iter_mut().find(|item| item.id == tab) {
                    tab.error = Some("无法打开此网址".into());
                }
            });
        }
    }

    fn command(&mut self, command: Command) -> bool {
        match command {
            Command::NewTab { url } => {
                self.open_tab(
                    PageStart::New,
                    url.and_then(|url| model::address_target(&url)),
                    1.0,
                    true,
                );
                return true;
            }
            Command::SelectTab { id } => {
                self.select_tab(id);
                return true;
            }
            Command::CloseTab { id } => {
                self.close_tab(id);
                return true;
            }
            Command::Open { tab, input } => self.open_address(tab, &input),
            Command::Home { tab } => self.send(tab, TabCommand::Navigate(Some(HOME.to_owned()))),
            Command::Back { tab } => self.send(tab, TabCommand::History(-1)),
            Command::Forward { tab } => self.send(tab, TabCommand::History(1)),
            Command::Reload { tab } => self.send(tab, TabCommand::Navigate(None)),
            Command::Stop { tab } => self.send(tab, TabCommand::Stop),
            Command::Visible { visible } => {
                self.visible = visible;
                self.present_selected();
                return true;
            }
            Command::Resize { width, height, dpr } => {
                let geometry = Geometry {
                    width: width.min(8192),
                    height: height.min(8192),
                    dpr: dpr.clamp(0.5, 8.0),
                };
                if geometry != self.geometry {
                    self.geometry = geometry;
                    self.present_selected();
                }
            }
            Command::Zoom { tab, zoom } => {
                self.shared.update(|state| {
                    if let Some(tab) = state.tabs.iter_mut().find(|item| item.id == tab) {
                        tab.zoom = zoom.clamp(model::MIN_ZOOM, model::MAX_ZOOM);
                    }
                });
                if tab == self.selected {
                    self.present_selected();
                }
                return true;
            }
            Command::Touch { tab, phase, x, y } if tab == self.selected => {
                self.send(tab, TabCommand::Touch { phase, x, y });
            }
            Command::Scroll {
                tab,
                phase,
                x,
                y,
                dx,
                dy,
            } if tab == self.selected => {
                self.send(
                    tab,
                    TabCommand::Scroll {
                        phase,
                        x,
                        y,
                        dx,
                        dy,
                    },
                );
            }
            Command::Edit { tab, token, edit } if tab == self.selected => {
                self.send(tab, TabCommand::Edit { token, edit });
            }
            Command::Bookmark { tab } => {
                self.shared.update(|state| {
                    if let Some(tab) = state
                        .tabs
                        .iter()
                        .find(|item| item.id == tab && model::web_url(&item.url))
                    {
                        if let Some(index) = state
                            .bookmarks
                            .iter()
                            .position(|visit| visit.url == tab.url)
                        {
                            state.bookmarks.remove(index);
                        } else {
                            state.bookmarks.insert(
                                0,
                                Visit {
                                    url: tab.url.clone(),
                                    title: tab.title.clone(),
                                },
                            );
                            state.bookmarks.truncate(200);
                        }
                    }
                });
                return true;
            }
            Command::ClearDownloads => self.download_client.send(DownloadCommand::Clear),
            Command::ClearHistory => {
                self.shared.update(|state| state.history.clear());
                return true;
            }
            Command::CancelDownload { id } => {
                self.download_client.send(DownloadCommand::Cancel(id))
            }
            Command::RemoveDownload { id } => {
                self.download_client.send(DownloadCommand::Remove(id))
            }
            Command::Respond {
                id,
                accepted,
                value,
                files,
            } => {
                if self
                    .prompts
                    .front()
                    .is_some_and(|(prompt, _)| prompt.id == id)
                    && let Some((_, pending)) = self.prompts.pop_front()
                {
                    match pending {
                        PendingPrompt::Dialog(dialog) => {
                            dialog.finish(accepted, value);
                        }
                        PendingPrompt::File { tab, activation } => {
                            self.send(tab, TabCommand::Files { activation, files });
                        }
                    }
                    self.publish_prompt();
                }
            }
            _ => {}
        }
        false
    }

    fn tab_for_document(&self, document: RendererDocumentLifecycleIdentity) -> Option<u64> {
        self.tabs
            .iter()
            .find(|(_, tab)| *tab.document.lock() == Some(document))
            .map(|(id, _)| *id)
    }

    fn renderer_action(&mut self, action: RendererOwnerAction) -> bool {
        match action {
            RendererOwnerAction::Download(activation) => self.downloads.start(activation),
            RendererOwnerAction::Popup(popup) => {
                let page = popup.into_parts();
                let previous = page.window_identity().and_then(|window| {
                    self.tabs
                        .iter()
                        .find(|(_, tab)| tab.window == window)
                        .map(|(id, _)| *id)
                });
                let foreground = page.disposition != RendererPopupDisposition::Background;
                if let Some(id) = previous {
                    if foreground {
                        self.select_tab(id);
                    }
                    if page.navigation_requested && model::web_url(&page.url) {
                        self.send(id, TabCommand::Navigate(Some(page.url)));
                    }
                } else if page.pending_auxiliary_page.is_some() {
                    self.open_tab(PageStart::Popup(Box::new(page)), None, 1.0, foreground);
                } else if model::web_url(&page.url) {
                    self.open_tab(PageStart::New, Some(page.url), 1.0, foreground);
                }
                return true;
            }
            RendererOwnerAction::CloseAuxiliaryWindow(window) => {
                let id = self
                    .tabs
                    .iter()
                    .find(|(_, tab)| tab.window.auxiliary_window() == Some(&window))
                    .map(|(id, _)| *id);
                if let Some(id) = id {
                    self.close_tab(id);
                    return true;
                }
            }
            RendererOwnerAction::JavaScriptDialog(dialog) => {
                if self.tab_for_document(dialog.source_document()) != Some(self.selected)
                    || self.prompts.len() >= 6
                {
                    dialog.finish(false, String::new());
                } else {
                    self.push_prompt(
                        Prompt {
                            id: 0,
                            kind: dialog.dialog_type().to_owned(),
                            source: dialog.source_url().to_owned(),
                            message: dialog.message().chars().take(4096).collect(),
                            value: dialog.default_prompt().to_owned(),
                            multiple: false,
                        },
                        PendingPrompt::Dialog(dialog),
                    );
                }
            }
            RendererOwnerAction::FileChooser(activation) => {
                if let Some(tab) = self.tab_for_document(activation.source_document()) {
                    self.offer_file_picker(tab, activation);
                }
            }
            _ => {}
        }
        false
    }

    fn offer_file_picker(&mut self, tab: u64, activation: RendererPendingFileChooserActivation) {
        if tab != self.selected || self.prompts.iter().any(|(_, pending)| {
            matches!(pending, PendingPrompt::File { tab: source, activation: existing } if *source == tab && *existing == activation)
        }) || self.prompts.len() >= 6 {
            return;
        }
        self.push_prompt(
            Prompt {
                id: 0,
                kind: "file".into(),
                source: String::new(),
                message: String::new(),
                value: String::new(),
                multiple: activation.allow_multiple(),
            },
            PendingPrompt::File { tab, activation },
        );
    }

    fn push_prompt(&mut self, mut prompt: Prompt, pending: PendingPrompt) {
        prompt.id = self.next_prompt;
        self.next_prompt += 1;
        self.prompts.push_back((prompt, pending));
        self.publish_prompt();
    }

    fn publish_prompt(&self) {
        self.shared
            .update(|state| state.prompt = self.prompts.front().map(|(prompt, _)| prompt.clone()));
    }

    fn save(&self) {
        let session = {
            let state = self.shared.state.lock();
            Session {
                selected: state
                    .tabs
                    .iter()
                    .position(|tab| tab.id == self.selected)
                    .unwrap_or(0),
                tabs: state
                    .tabs
                    .iter()
                    .map(|tab| {
                        let url = tab
                            .pending_url
                            .as_ref()
                            .filter(|url| model::web_url(url))
                            .unwrap_or(&tab.url);
                        SavedTab {
                            url: url.clone(),
                            zoom: tab.zoom,
                        }
                    })
                    .collect(),
                history: state.history.clone(),
                bookmarks: state.bookmarks.clone(),
            }
        };
        if let Err(error) = session.save(&self.session_path) {
            eprintln!("Molio: save browsing session: {error:#}");
        }
    }
}

fn download_state(download: &downloads::Download) -> DownloadState {
    let (status, path, error) = match &download.state {
        downloads::DownloadState::Downloading => ("downloading", None, None),
        downloads::DownloadState::Complete(path) => {
            ("complete", Some(path.to_string_lossy().into_owned()), None)
        }
        downloads::DownloadState::Canceled => ("canceled", None, None),
        downloads::DownloadState::Failed(error) => ("failed", None, Some(error.clone())),
    };
    DownloadState {
        id: download.id,
        name: download.name.clone(),
        source: download.source.clone(),
        received: download.received,
        total: download.total,
        status,
        path,
        error,
    }
}
