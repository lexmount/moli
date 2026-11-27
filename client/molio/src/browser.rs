use std::rc::Rc;

use gpui::{
    App, Context, Entity, FontWeight, MouseButton, Render, ScrollHandle, SharedString,
    Subscription, Window, actions, div, prelude::*, px,
};
use moli_core::page::RendererPopupDisposition;
use tokio::sync::mpsc;

use crate::{
    address::{Address, AddressEvent},
    address_target::address_target,
    download_page::{self, DownloadsPage},
    host::{BrowserEvent, BrowserHost, TabCommand, TabConnection, TabState},
    icons::{Icon, glyph},
    palette::Palette,
    surface::MoliSurface,
};

actions!(
    browser,
    [
        NewTab,
        CloseTab,
        ReopenTab,
        FocusAddress,
        ShowDownloads,
        Reload,
        GoBack,
        GoForward,
        NextTab,
        PreviousTab,
        ZoomIn,
        ZoomOut,
        ResetZoom,
        Quit
    ]
);

type TabId = u64;

struct Tab {
    id: TabId,
    connection: Option<TabConnection>,
    state: TabState,
    zoom: f64,
    show_new_tab_page: bool,
    page_focus: bool,
}

impl Tab {
    fn title(&self) -> String {
        if self.connection.is_none() {
            return "Downloads".to_owned();
        }
        match self.state.navigation.as_ref() {
            Some(nav) if !nav.title.is_empty() => nav.title.clone(),
            Some(nav) if nav.url.as_str() != "about:blank" => {
                nav.url.host_str().unwrap_or(nav.url.as_str()).to_owned()
            }
            _ => "New tab".to_owned(),
        }
    }

    fn url(&self) -> &str {
        if self.connection.is_none() {
            return download_page::URL;
        }
        self.state
            .navigation
            .as_ref()
            .map_or("about:blank", |nav| nav.url.as_str())
    }
}

struct DraggedTab {
    id: TabId,
    title: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let colors = Palette::for_window(window.appearance());
        div()
            .px_3()
            .py_2()
            .rounded_md()
            .bg(colors.toolbar)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .shadow_md()
            .text_sm()
            .child(self.title.clone())
    }
}

#[derive(PartialEq, Eq)]
enum Popup {
    Menu,
    Site,
}

pub struct BrowserWindow {
    host: Rc<BrowserHost>,
    surface: Entity<MoliSurface>,
    address: Entity<Address>,
    downloads: Entity<DownloadsPage>,
    tabs: Vec<Tab>,
    active: TabId,
    next_id: TabId,
    closed: Vec<Option<String>>,
    popup: Option<Popup>,
    displayed_address: String,
    tab_scroll: ScrollHandle,
    _address_events: Subscription,
    _address_updates: Subscription,
}

impl BrowserWindow {
    pub fn new(
        host: Rc<BrowserHost>,
        mut host_events: mpsc::UnboundedReceiver<BrowserEvent>,
        initial_url: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let address = cx.new(Address::new);
        let address_updates = cx.observe(&address, |_, _, cx| cx.notify());
        let surface = cx.new(|cx| MoliSurface::new(window, cx));
        let downloads = cx.new(|cx| DownloadsPage::new(host.downloads.clone(), cx));
        cx.spawn_in(window, async move |view, cx| {
            while let Some(event) = host_events.recv().await {
                if cx
                    .update(|window, cx| {
                        view.update(cx, |browser, cx| {
                            browser.receive_host_event(event, window, cx)
                        })
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        let address_events =
            cx.subscribe_in(
                &address,
                window,
                |browser, _, event, window, cx| match event {
                    AddressEvent::Submit(input) => browser.open_address(input, window, cx),
                    AddressEvent::Cancel => {
                        browser.update_address(true, window, cx);
                        browser.focus_page(window, cx);
                    }
                },
            );
        let mut view = Self {
            host,
            surface,
            address,
            downloads,
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            closed: Vec::new(),
            popup: None,
            displayed_address: String::new(),
            tab_scroll: ScrollHandle::new(),
            _address_events: address_events,
            _address_updates: address_updates,
        };
        view.add_tab(initial_url, window, cx);
        view
    }

    fn current(&self) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == self.active)
    }

    fn send(&self, command: TabCommand) {
        if let Some(connection) = self.current().and_then(|tab| tab.connection.as_ref()) {
            let _ = connection.commands.send(command);
        }
    }

    fn add_tab(&mut self, url: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if url.as_deref().is_some_and(download_page::is_downloads_url) {
            self.show_downloads(window, cx);
            return;
        }
        let connection = match self.host.open_tab() {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("Could not open tab: {error}");
                return;
            }
        };
        self.attach_tab(
            connection,
            RendererPopupDisposition::Foreground,
            url.is_none(),
            window,
            cx,
        );
        if let Some(url) = url {
            self.open_address(&url, window, cx);
        }
    }

    fn receive_host_event(
        &mut self,
        event: BrowserEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            BrowserEvent::Popup(popup) => {
                if popup
                    .auxiliary_window
                    .as_ref()
                    .is_some_and(|window| window.is_closed())
                {
                    return;
                }
                let existing = popup.browsing_context_name.as_ref().and_then(|name| {
                    self.tabs.iter().find_map(|tab| {
                        let connection = tab.connection.as_ref()?;
                        (connection.window_identity.name() == name).then_some(tab.id)
                    })
                });
                if let Some(id) = existing {
                    if popup.navigation_requested
                        && popup.disposition != RendererPopupDisposition::Background
                    {
                        self.select_tab(id, window, cx);
                    }
                } else if popup.pending_auxiliary_page.is_some() {
                    let disposition = popup.disposition;
                    match self.host.open_popup(popup) {
                        Ok(connection) => {
                            self.attach_tab(connection, disposition, false, window, cx)
                        }
                        Err(error) => eprintln!("Could not open auxiliary tab: {error:#}"),
                    }
                }
            }
            BrowserEvent::ClosePopup(popup) => {
                if let Some(tab) = self.tabs.iter().find(|tab| {
                    tab.connection
                        .as_ref()
                        .and_then(|connection| connection.window_identity.auxiliary_window())
                        == Some(&popup)
                }) {
                    self.close_tab(tab.id, window, cx);
                }
            }
        }
    }

    fn attach_tab(
        &mut self,
        connection: TabConnection,
        disposition: RendererPopupDisposition,
        show_new_tab_page: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        let mut state = connection.state.clone();
        let mut frames = connection.viewport.subscribe();
        cx.spawn_in(window, async move |view, cx| {
            loop {
                let update = state.borrow_and_update().clone();
                if cx
                    .update(|window, cx| {
                        view.update(cx, |browser, cx| {
                            browser.receive_state(id, update, window, cx)
                        })
                    })
                    .is_err()
                {
                    break;
                }
                if state.changed().await.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |view, cx| {
            while frames.changed().await.is_ok() {
                let _ = frames.borrow_and_update();
                if cx
                    .update(|window, cx| {
                        view.update(cx, |browser, cx| {
                            if browser.active == id {
                                browser.surface.update(cx, |surface, cx| {
                                    surface.request_display_tick(window, cx);
                                    cx.notify();
                                });
                            }
                        })
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        self.tabs.push(Tab {
            id,
            connection: Some(connection),
            state: TabState::default(),
            zoom: 1.,
            show_new_tab_page,
            page_focus: !show_new_tab_page,
        });
        if disposition != RendererPopupDisposition::Background {
            self.select_tab(id, window, cx);
        } else {
            cx.notify();
        }
    }

    fn show_downloads(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = if let Some(tab) = self.tabs.iter().find(|tab| tab.connection.is_none()) {
            tab.id
        } else {
            let id = self.next_id;
            self.next_id += 1;
            self.tabs.push(Tab {
                id,
                connection: None,
                state: TabState::default(),
                zoom: 1.,
                show_new_tab_page: false,
                page_focus: true,
            });
            id
        };
        self.select_tab(id, window, cx);
        self.popup = None;
        self.focus_page(window, cx);
        cx.notify();
    }

    fn select_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if self.active == id {
            return;
        }
        let page_has_focus = self.surface.read(cx).focus.is_focused(window);
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == self.active) {
            tab.page_focus = page_has_focus;
        }
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let tab = &self.tabs[index];
        self.tab_scroll.scroll_to_item(index);
        let zoom = tab.zoom;
        let page_focus = tab.page_focus;
        let document = tab.state.document;
        self.active = id;
        self.popup = None;
        self.surface.update(cx, |surface, cx| {
            surface.select_tab(tab.connection.as_ref(), zoom, document, window, cx)
        });
        window.set_window_title(&format!("{} · molio", tab.title()));
        self.update_address(true, window, cx);
        if page_focus {
            self.focus_page(window, cx);
        } else {
            self.focus_address(window, cx);
        }
        cx.notify();
    }

    fn close_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return;
        };
        let tab = self.tabs.remove(index);
        if let Some(auxiliary) = tab
            .connection
            .as_ref()
            .and_then(|connection| connection.window_identity.auxiliary_window())
        {
            auxiliary.close();
        }
        let url = (!tab.show_new_tab_page).then(|| {
            tab.connection
                .as_ref()
                .and_then(|connection| {
                    connection
                        .state
                        .borrow()
                        .navigation
                        .as_ref()
                        .map(|navigation| navigation.url.to_string())
                })
                .unwrap_or_else(|| tab.url().to_owned())
        });
        self.closed.push(url);
        if self.closed.len() > 20 {
            self.closed.remove(0);
        }
        if self.tabs.is_empty() {
            window.remove_window();
        } else if id == self.active {
            self.select_tab(self.tabs[index.min(self.tabs.len() - 1)].id, window, cx);
        }
        cx.notify();
    }

    fn reopen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(url) = self.closed.pop() {
            self.add_tab(url, window, cx);
        }
    }

    fn move_tab(&mut self, source: TabId, target: TabId, cx: &mut Context<Self>) {
        let Some(from) = self.tabs.iter().position(|tab| tab.id == source) else {
            return;
        };
        let Some(to) = self.tabs.iter().position(|tab| tab.id == target) else {
            return;
        };
        if from != to {
            let tab = self.tabs.remove(from);
            self.tabs.insert(to, tab);
            cx.notify();
        }
    }

    fn cycle_tabs(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(index) = self.tabs.iter().position(|tab| tab.id == self.active) {
            let next = (index as isize + step).rem_euclid(self.tabs.len() as isize) as usize;
            self.select_tab(self.tabs[next].id, window, cx);
        }
    }

    fn receive_state(
        &mut self,
        id: TabId,
        state: TabState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == id) else {
            return;
        };
        let document_changed = tab.state.document != state.document;
        if state.navigation != tab.state.navigation
            && let Some(navigation) = state.navigation.as_ref()
        {
            self.address.update(cx, |address, cx| {
                address.remember(&navigation.url, &navigation.title, cx);
            });
        }
        tab.state = state;
        if id == self.active {
            let document = tab.state.document;
            let editor = tab.state.editor.clone();
            let ack = tab.state.editor_ack;
            self.surface.update(cx, |surface, cx| {
                surface.update_editor(document, editor, ack, window, cx)
            });
            if document_changed {
                self.surface
                    .update(cx, |surface, cx| surface.clear_frame(window, cx));
            }
            self.update_address(false, window, cx);
            let title = self.current().map_or_else(
                || "molio".to_owned(),
                |tab| format!("{} · molio", tab.title()),
            );
            window.set_window_title(&title);
        }
        cx.notify();
    }

    fn update_address(&mut self, force: bool, window: &Window, cx: &mut Context<Self>) {
        let url = self.current().map_or("", Tab::url);
        let url = if url == "about:blank" { "" } else { url };
        if force
            || (!self.address.read(cx).focus.is_focused(window) && self.displayed_address != url)
        {
            let url = url.to_owned();
            self.displayed_address.clone_from(&url);
            self.address.update(cx, |address, cx| address.show(url, cx));
        }
    }

    fn open_address(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if download_page::is_downloads_url(text) {
            self.show_downloads(window, cx);
            return;
        }
        if let Some(url) = address_target(text)
            && let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == self.active)
        {
            let url = url.to_string();
            if tab.connection.is_none() {
                self.add_tab(Some(url), window, cx);
                return;
            }
            tab.show_new_tab_page = false;
            self.displayed_address.clone_from(&url);
            self.address
                .update(cx, |address, cx| address.show(url.clone(), cx));
            self.send(TabCommand::Navigate(Some(url)));
            self.focus_page(window, cx);
            self.popup = None;
            cx.notify();
        }
    }

    fn focus_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.popup = None;
        self.address
            .update(cx, |address, cx| address.select_all(window, cx));
        cx.notify();
    }

    fn focus_page(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.current().is_some_and(|tab| tab.connection.is_none()) {
            self.downloads.read(cx).focus.clone().focus(window, cx);
            return;
        }
        self.surface
            .update(cx, |surface, cx| surface.focus_page(window, cx));
    }

    fn reload_or_stop(&self) {
        let Some(tab) = self.current() else { return };
        self.send(if tab.state.loading {
            TabCommand::Stop
        } else {
            TabCommand::Navigate(tab.state.pending_url.clone())
        });
    }

    fn change_zoom(&mut self, step: i32, cx: &mut Context<Self>) {
        const LEVELS: [f64; 11] = [0.5, 0.67, 0.8, 0.9, 1., 1.1, 1.25, 1.5, 1.75, 2., 2.5];
        let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == self.active) else {
            return;
        };
        let index = LEVELS
            .iter()
            .position(|zoom| *zoom == tab.zoom)
            .unwrap_or(4);
        tab.zoom = if step == 0 {
            1.
        } else {
            LEVELS[(index as i32 + step).clamp(0, LEVELS.len() as i32 - 1) as usize]
        };
        let zoom = tab.zoom;
        self.surface
            .update(cx, |surface, cx| surface.set_zoom(zoom, cx));
        cx.notify();
    }

    fn toggle_popup(&mut self, popup: Popup, cx: &mut Context<Self>) {
        self.popup = if self.popup.as_ref() == Some(&popup) {
            None
        } else {
            Some(popup)
        };
        cx.notify();
    }

    fn tab_strip(&self, colors: Palette, cx: &mut Context<Self>) -> impl IntoElement {
        let left_inset = if cfg!(target_os = "macos") { 80. } else { 8. };
        let tabs = self.tabs.iter().map(|tab| {
            let id = tab.id;
            let active = id == self.active;
            let title = SharedString::from(tab.title());
            let busy = tab.state.loading;
            div()
                .id(("tab", id as usize))
                .h(px(34.))
                .max_w(px(208.))
                .min_w(px(108.))
                .w(px(160.))
                .flex_shrink_1()
                .flex()
                .gap_2()
                .items_center()
                .px_2()
                .rounded_t_md()
                .border_t_2()
                .border_color(if active { colors.accent } else { colors.frame })
                .bg(if active { colors.toolbar } else { colors.frame })
                .hover_style(move |style| {
                    style.bg(if active { colors.toolbar } else { colors.hover })
                })
                .on_click(cx.listener(move |view, _, window, cx| view.select_tab(id, window, cx)))
                .on_mouse_up(
                    MouseButton::Middle,
                    cx.listener(move |view, _, window, cx| view.close_tab(id, window, cx)),
                )
                .on_drag(
                    DraggedTab {
                        id,
                        title: title.clone(),
                    },
                    |dragged, _, _, cx| {
                        cx.new(|_| DraggedTab {
                            id: dragged.id,
                            title: dragged.title.clone(),
                        })
                    },
                )
                .on_drop(
                    cx.listener(move |view, tab: &DraggedTab, _, cx| view.move_tab(tab.id, id, cx)),
                )
                .child(
                    match (busy, tab.connection.is_none(), tab.state.favicon.as_ref()) {
                        (true, _, _) => glyph(Icon::Loading, colors.accent).into_any_element(),
                        (false, true, _) => glyph(Icon::Download, colors.muted).into_any_element(),
                        (false, false, Some(icon)) => gpui::img(icon.clone())
                            .size(px(16.))
                            .flex_shrink_0()
                            .into_any_element(),
                        (false, false, None) => glyph(Icon::Page, colors.muted).into_any_element(),
                    },
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .child(title),
                )
                .child(
                    icon(
                        ("close", id as usize),
                        Icon::Close,
                        "Close tab",
                        true,
                        colors,
                    )
                    .size(px(22.))
                    .on_click(cx.listener(move |view, _, window, cx| {
                        cx.stop_propagation();
                        view.close_tab(id, window, cx);
                    })),
                )
        });
        div()
            .flex()
            .items_end()
            .h(px(40.))
            .pl(px(left_inset))
            .pr_2()
            .bg(colors.frame)
            .flex_shrink_0()
            .child(
                div()
                    .id("tabs")
                    .flex()
                    .items_end()
                    .min_w_0()
                    .gap_1()
                    .overflow_x_scroll()
                    .track_scroll(&self.tab_scroll)
                    .children(tabs),
            )
            .child(
                icon("new-tab", Icon::Add, "New tab", true, colors)
                    .mb(px(2.))
                    .on_click(cx.listener(|view, _, window, cx| view.add_tab(None, window, cx))),
            )
            .child(
                div()
                    .flex_1()
                    .when(cfg!(target_os = "macos"), |region| {
                        region.min_w(px(40.)).flex_shrink_0()
                    })
                    .h_full()
                    .on_mouse_down(MouseButton::Left, |event, window, _| {
                        if event.click_count == 2 {
                            window.zoom_window();
                        } else {
                            window.start_window_move();
                        }
                    }),
            )
    }

    fn toolbar(
        &self,
        colors: Palette,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let nav = self.current().and_then(|tab| tab.state.navigation.as_ref());
        let back = nav.is_some_and(|nav| nav.can_go_back);
        let forward = nav.is_some_and(|nav| nav.can_go_forward);
        let loading = self.current().is_some_and(|tab| tab.state.loading);
        let focused = self.address.read(cx).focus.is_focused(window);
        div()
            .h(px(45.))
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .bg(colors.toolbar)
            .border_b_1()
            .border_color(colors.border)
            .flex_shrink_0()
            .child(
                icon("back", Icon::Back, "Go back", back, colors).on_click(cx.listener(
                    move |view, _, _, _| {
                        if back {
                            view.send(TabCommand::History(-1));
                        }
                    },
                )),
            )
            .child(
                icon("forward", Icon::Forward, "Go forward", forward, colors).on_click(
                    cx.listener(move |view, _, _, _| {
                        if forward {
                            view.send(TabCommand::History(1));
                        }
                    }),
                ),
            )
            .child(
                icon(
                    "reload",
                    if loading { Icon::Stop } else { Icon::Reload },
                    if loading { "Stop loading" } else { "Reload" },
                    true,
                    colors,
                )
                .on_click(cx.listener(|view, _, _, _| view.reload_or_stop())),
            )
            .child(
                div()
                    .id("address-field")
                    .ml_1()
                    .flex_1()
                    .flex()
                    .gap_1()
                    .items_center()
                    .h(px(32.))
                    .min_w_0()
                    .rounded_md()
                    .border_1()
                    .border_color(if focused {
                        colors.accent
                    } else {
                        colors.border
                    })
                    .bg(colors.field)
                    .pr_2()
                    .child(
                        icon("site-info", Icon::Site, "Site information", true, colors)
                            .size(px(29.))
                            .on_click(
                                cx.listener(|view, _, _, cx| view.toggle_popup(Popup::Site, cx)),
                            ),
                    )
                    .child(self.address.clone()),
            )
            .child(
                icon("downloads", Icon::Download, "Downloads", true, colors)
                    .ml_1()
                    .on_click(cx.listener(|view, _, window, cx| view.show_downloads(window, cx))),
            )
            .child(
                icon("menu", Icon::Menu, "molio menu", true, colors)
                    .ml_1()
                    .on_click(cx.listener(|view, _, _, cx| view.toggle_popup(Popup::Menu, cx))),
            )
    }

    fn popup(&self, colors: Palette, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let active = self.popup.as_ref()?;
        let panel = div()
            .id("browser-popup")
            .absolute()
            .top(px(82.))
            .p_2()
            .w(px(244.))
            .bg(colors.toolbar)
            .border_1()
            .border_color(colors.border)
            .rounded_lg()
            .shadow_lg()
            .on_mouse_down_out(cx.listener(|view, _, _, cx| {
                view.popup = None;
                cx.notify();
            }));
        if *active == Popup::Site {
            let url = self.current().map_or("about:blank", Tab::url);
            let origin = url::Url::parse(url)
                .ok()
                .and_then(|url| url.host_str().map(str::to_owned))
                .unwrap_or_else(|| "Local page".to_owned());
            return Some(
                panel
                    .left(px(120.))
                    .px_3()
                    .py_3()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(origin))
                    .child(div().text_size(px(12.)).text_color(colors.muted).child(
                        if url.starts_with("https:") {
                            "HTTPS"
                        } else if url.starts_with("http:") {
                            "HTTP"
                        } else {
                            "Browser content"
                        },
                    )),
            );
        }
        let percentage = self
            .current()
            .map_or(100, |tab| (tab.zoom * 100.).round() as u32);
        Some(
            panel
                .right(px(8.))
                .child(
                    menu_row("new", "New tab", colors).on_click(
                        cx.listener(|view, _, window, cx| view.add_tab(None, window, cx)),
                    ),
                )
                .child(
                    menu_row("reopen", "Reopen closed tab", colors)
                        .on_click(cx.listener(|view, _, window, cx| view.reopen(window, cx))),
                )
                .child(
                    menu_row("open-downloads", "Downloads", colors).on_click(
                        cx.listener(|view, _, window, cx| view.show_downloads(window, cx)),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px_2()
                        .py_1()
                        .text_size(px(13.))
                        .child("Page zoom")
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    icon("zoom-out", Icon::Remove, "Zoom out", true, colors)
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.change_zoom(-1, cx)),
                                        ),
                                )
                                .child(
                                    button("zoom-reset", "Reset zoom", true, colors)
                                        .w(px(46.))
                                        .text_size(px(12.))
                                        .child(format!("{percentage}%"))
                                        .on_click(
                                            cx.listener(|view, _, _, cx| view.change_zoom(0, cx)),
                                        ),
                                )
                                .child(
                                    icon("zoom-in", Icon::Add, "Zoom in", true, colors).on_click(
                                        cx.listener(|view, _, _, cx| view.change_zoom(1, cx)),
                                    ),
                                ),
                        ),
                )
                .child(menu_row("quit", "Quit molio", colors).on_click(|_, _, cx| cx.quit())),
        )
    }

    fn address_suggestions(
        &self,
        colors: Palette,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        let address = self.address.read(cx);
        if !address.focus.is_focused(window) || self.popup.is_some() {
            return None;
        }
        let suggestions = address.suggestions();
        if suggestions.is_empty() {
            return None;
        }
        let highlighted = address.selected_suggestion;
        let rows = suggestions
            .into_iter()
            .enumerate()
            .map(|(index, entry)| (index, entry.url.clone(), entry.title.clone()))
            .collect::<Vec<_>>();
        Some(
            div()
                .id("address-suggestions")
                .absolute()
                .top(px(80.))
                .left(px(120.))
                .right(px(48.))
                .rounded_lg()
                .border_1()
                .border_color(colors.border)
                .bg(colors.toolbar)
                .shadow_lg()
                .p_1()
                .occlude()
                .children(rows.into_iter().map(|(index, url, title)| {
                    let destination = url.clone();
                    let label = if title.is_empty() { url.clone() } else { title };
                    div()
                        .id(("address-suggestion", index))
                        .flex()
                        .flex_col()
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .bg(if highlighted == Some(index) {
                            colors.selection
                        } else {
                            colors.toolbar
                        })
                        .hover_style(move |style| style.bg(colors.hover))
                        .cursor_pointer()
                        .child(div().text_sm().truncate().child(label))
                        .child(
                            div()
                                .text_xs()
                                .text_color(colors.muted)
                                .truncate()
                                .child(url),
                        )
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.open_address(&destination, window, cx);
                        }))
                })),
        )
    }
}

impl Render for BrowserWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Palette::for_window(window.appearance());
        let error = self.current().and_then(|tab| tab.state.error.clone());
        let show_new_tab_page = self
            .current()
            .is_some_and(|tab| tab.show_new_tab_page && !tab.state.loading);
        let showing_downloads = self.current().is_some_and(|tab| tab.connection.is_none());
        let surface = div().relative().flex_1().min_h_0().overflow_hidden();
        let surface = if showing_downloads {
            surface.child(self.downloads.clone())
        } else if let Some(error) = error {
            let surface = surface.child(self.surface.clone());
            surface.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(colors.toolbar)
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Unable to load this page"),
                    )
                    .child(
                        div()
                            .max_w(px(480.))
                            .px_4()
                            .text_sm()
                            .text_color(colors.muted)
                            .child(error),
                    )
                    .child(
                        menu_row("retry", "Try again", colors)
                            .border_1()
                            .border_color(colors.border)
                            .on_click(cx.listener(|view, _, _, _| view.reload_or_stop())),
                    ),
            )
        } else if show_new_tab_page {
            let surface = surface.child(self.surface.clone());
            surface.child(
                div()
                    .debug_selector(|| "new-tab-page".into())
                    .absolute()
                    .inset_0()
                    .bg(colors.toolbar)
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_4()
                    .child(
                        div()
                            .text_color(colors.accent)
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(54.))
                            .child("molio"),
                    )
                    .child(
                        menu_row("start-browsing", "Search or enter an address", colors)
                            .px_5()
                            .mx_4()
                            .bg(colors.field)
                            .border_1()
                            .border_color(colors.border)
                            .on_click(
                                cx.listener(|view, _, window, cx| view.focus_address(window, cx)),
                            ),
                    ),
            )
        } else {
            surface.child(self.surface.clone())
        };
        div()
            .id("browser-window")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .text_color(colors.text)
            .bg(colors.toolbar)
            .on_action(cx.listener(|view, _: &NewTab, window, cx| view.add_tab(None, window, cx)))
            .on_action(
                cx.listener(|view, _: &CloseTab, window, cx| {
                    view.close_tab(view.active, window, cx)
                }),
            )
            .on_action(cx.listener(|view, _: &ReopenTab, window, cx| view.reopen(window, cx)))
            .on_action(
                cx.listener(|view, _: &ShowDownloads, window, cx| view.show_downloads(window, cx)),
            )
            .on_action(
                cx.listener(|view, _: &FocusAddress, window, cx| view.focus_address(window, cx)),
            )
            .on_action(cx.listener(|view, _: &Reload, _, _| view.reload_or_stop()))
            .on_action(cx.listener(|view, _: &GoBack, _, _| view.send(TabCommand::History(-1))))
            .on_action(cx.listener(|view, _: &GoForward, _, _| view.send(TabCommand::History(1))))
            .on_action(cx.listener(|view, _: &NextTab, window, cx| view.cycle_tabs(1, window, cx)))
            .on_action(
                cx.listener(|view, _: &PreviousTab, window, cx| view.cycle_tabs(-1, window, cx)),
            )
            .on_action(cx.listener(|view, _: &ZoomIn, _, cx| view.change_zoom(1, cx)))
            .on_action(cx.listener(|view, _: &ZoomOut, _, cx| view.change_zoom(-1, cx)))
            .on_action(cx.listener(|view, _: &ResetZoom, _, cx| view.change_zoom(0, cx)))
            .child(self.tab_strip(colors, cx))
            .child(self.toolbar(colors, window, cx))
            .child(surface)
            .children(self.address_suggestions(colors, window, cx))
            .children(self.popup(colors, cx))
    }
}

fn icon(
    id: impl Into<gpui::ElementId>,
    symbol: Icon,
    label: &'static str,
    enabled: bool,
    colors: Palette,
) -> gpui::Stateful<gpui::Div> {
    button(id, label, enabled, colors).child(glyph(
        symbol,
        if enabled { colors.text } else { colors.muted },
    ))
}

struct Tooltip(&'static str);

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let colors = Palette::for_window(window.appearance());
        div()
            .bg(colors.toolbar)
            .text_color(colors.text)
            .border_1()
            .border_color(colors.border)
            .rounded_md()
            .text_xs()
            .px_2()
            .py_1()
            .shadow_sm()
            .child(self.0)
    }
}

fn button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    enabled: bool,
    colors: Palette,
) -> gpui::Stateful<gpui::Div> {
    let button = div()
        .id(id)
        .size(px(32.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .text_color(if enabled { colors.text } else { colors.muted })
        .tooltip(move |_, cx| cx.new(|_| Tooltip(label)).into());
    if enabled {
        button
            .cursor_pointer()
            .hover_style(move |style| style.bg(colors.hover))
    } else {
        button.opacity(0.45)
    }
}

fn menu_row(id: &'static str, label: &'static str, colors: Palette) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .rounded_md()
        .px_2()
        .py_2()
        .text_size(px(13.))
        .cursor_pointer()
        .hover_style(move |style| style.bg(colors.hover))
        .child(label)
}

pub fn bind_keys(cx: &mut App) {
    use gpui::{KeyBinding, Menu, MenuItem};
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{modifier}-t"), NewTab, None),
        KeyBinding::new(&format!("{modifier}-w"), CloseTab, None),
        KeyBinding::new(&format!("{modifier}-shift-t"), ReopenTab, None),
        KeyBinding::new(&format!("{modifier}-j"), ShowDownloads, None),
        KeyBinding::new(&format!("{modifier}-l"), FocusAddress, None),
        KeyBinding::new(&format!("{modifier}-r"), Reload, None),
        KeyBinding::new("alt-left", GoBack, None),
        KeyBinding::new("alt-right", GoForward, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
        KeyBinding::new(&format!("{modifier}-="), ZoomIn, None),
        KeyBinding::new(&format!("{modifier}--"), ZoomOut, None),
        KeyBinding::new(&format!("{modifier}-0"), ResetZoom, None),
        KeyBinding::new(&format!("{modifier}-q"), Quit, None),
    ]);
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.set_menus(vec![Menu::new("molio").items([
        MenuItem::action("Downloads", ShowDownloads),
        MenuItem::action("Quit molio", Quit),
    ])]);
}
#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use gpui::{TestAppContext, VisualTestContext, WindowHandle, size};

    use super::*;

    fn html_url(html: &str) -> String {
        let html: String = url::form_urlencoded::byte_serialize(html.as_bytes()).collect();
        format!("data:text/html,{}", html.replace('+', "%20"))
    }

    fn wait_for_browser(
        browser: &WindowHandle<BrowserWindow>,
        cx: &mut VisualTestContext,
        message: &str,
        condition: impl Fn(&BrowserWindow) -> bool,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            cx.run_until_parked();
            if browser.update(cx, |browser, _, _| condition(browser))? {
                return Ok(());
            }
            anyhow::ensure!(Instant::now() < deadline, "{message}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn display_callbacks_stop_at_idle_and_follow_demand_across_resize() -> anyhow::Result<()> {
        let (host, events) = BrowserHost::start(None, None)?;
        let mut app = TestAppContext::single();
        let browser = app.open_window(size(px(640.), px(480.)), |window, cx| {
            BrowserWindow::new(
                Rc::new(host),
                events,
                Some(html_url(concat!(
                    "<title>Animated</title><script>",
                    "requestAnimationFrame(function frame() { requestAnimationFrame(frame); });",
                    "</script>",
                ))),
                window,
                cx,
            )
        });
        let mut cx = VisualTestContext::from_window(browser.into(), &app);
        wait_for_browser(&browser, &mut cx, "animation did not load", |browser| {
            browser
                .current()
                .is_some_and(|tab| tab.title() == "Animated")
        })?;
        let (output, mut ticks) = browser.update(&mut cx, |browser, _, _| {
            let tab = browser.current().unwrap().connection.as_ref().unwrap();
            (
                tab.viewport.clone(),
                tab.display_ticks.as_ref().unwrap().subscribe(),
            )
        })?;
        let mut deliver_tick = |cx: &mut VisualTestContext, revision| -> anyhow::Result<()> {
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                cx.run_until_parked();
                cx.update(|window, cx| window.simulate_next_frame(cx));
                if ticks.borrow_and_update().0 == revision {
                    return Ok(());
                }
                anyhow::ensure!(
                    Instant::now() < deadline,
                    "no display tick for revision {revision}"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        cx.update(|window, cx| window.draw(cx).clear(cx));
        wait_for_browser(
            &browser,
            &mut cx,
            "animation did not request a display tick",
            |_| output.display_tick_request().is_some(),
        )?;
        let first = output.display_tick_request().unwrap();
        deliver_tick(&mut cx, first)?;

        cx.simulate_resize(size(px(740.), px(540.)));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        wait_for_browser(
            &browser,
            &mut cx,
            "resize did not request a new display revision",
            |_| {
                output
                    .display_tick_request()
                    .is_some_and(|revision| revision > first)
            },
        )?;
        deliver_tick(&mut cx, output.display_tick_request().unwrap())?;

        browser.update(&mut cx, |browser, window, cx| {
            browser.open_address(&html_url("<title>Static</title><p>Idle</p>"), window, cx);
        })?;
        wait_for_browser(
            &browser,
            &mut cx,
            "static document did not settle",
            |browser| {
                browser.current().is_some_and(|tab| {
                    tab.title() == "Static"
                        && output.display_tick_request().is_none()
                        && output
                            .read(|frame| Some(frame.document) == tab.state.document)
                            .unwrap_or(false)
                })
            },
        )?;
        for _ in 0..2 {
            cx.update(|window, cx| window.simulate_next_frame(cx));
            assert!(!ticks.has_changed()?);
        }
        Ok(())
    }

    #[test]
    fn blank_popup_content_is_not_covered_by_the_new_tab_page() -> anyhow::Result<()> {
        let (host, events) = BrowserHost::start(None, None)?;
        let mut app = TestAppContext::single();
        let browser = app.open_window(size(px(640.), px(480.)), |window, cx| {
            BrowserWindow::new(Rc::new(host), events, None, window, cx)
        });
        let mut cx = VisualTestContext::from_window(browser.into(), &app);
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("new-tab-page").is_some());

        let html = r#"<script>
            const report = window.open("", "report");
            report.document.title = "Invoice";
            report.document.body.innerHTML = "<h1>Invoice</h1>";
        </script>"#;
        let url = html_url(html);
        browser.update(&mut cx, |browser, window, cx| {
            browser.open_address(&url, window, cx);
        })?;

        wait_for_browser(&browser, &mut cx, "invoice popup did not open", |browser| {
            browser.tabs.len() == 2
                && browser
                    .current()
                    .is_some_and(|tab| tab.url() == "about:blank" && tab.title() == "Invoice")
        })?;

        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert!(cx.debug_bounds("new-tab-page").is_none());
        Ok(())
    }

    #[test]
    fn nested_popup_survives_parent_close_before_adoption() -> anyhow::Result<()> {
        let (host, events) = BrowserHost::start(None, None)?;
        let url = html_url(
            r#"<script>
                const parent = window.open("about:blank", "parent");
                const child = parent.eval('window.open("about:blank", "child")');
                child.document.title = "Nested child";
                parent.close();
                document.title = "Parent closed:" + parent.closed;
            </script>"#,
        );
        let mut app = TestAppContext::single();
        let browser = app.open_window(size(px(640.), px(480.)), |window, cx| {
            BrowserWindow::new(Rc::new(host), events, Some(url), window, cx)
        });
        let mut cx = VisualTestContext::from_window(browser.into(), &app);

        wait_for_browser(&browser, &mut cx, "parent never closed", |browser| {
            browser
                .tabs
                .iter()
                .any(|tab| tab.title() == "Parent closed:true")
        })?;
        wait_for_browser(
            &browser,
            &mut cx,
            "nested child popup did not open",
            |browser| {
                browser.tabs.len() == 2
                    && browser
                        .current()
                        .is_some_and(|tab| tab.title() == "Nested child")
            },
        )?;
        let next = html_url("<title>Child navigated</title>");
        let child = browser.update(&mut cx, |browser, window, cx| {
            // Tab IDs are allocated monotonically, including for tabs later closed.
            assert_eq!(browser.next_id, 3, "the closed parent was attached");
            let child = browser.active;
            browser.open_address(&next, window, cx);
            child
        })?;
        wait_for_browser(
            &browser,
            &mut cx,
            "nested child popup did not navigate",
            |browser| {
                browser.current().is_some_and(|tab| {
                    tab.id == child && tab.url() == next && tab.title() == "Child navigated"
                })
            },
        )?;
        Ok(())
    }
}
