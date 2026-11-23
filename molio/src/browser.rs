use gpui::{
    App, Context, Entity, FontWeight, MouseButton, Render, ScrollHandle, SharedString,
    Subscription, Window, actions, div, prelude::*, px,
};

use crate::{
    address::{Address, AddressEvent},
    host::{BrowserHost, TabCommand, TabConnection, TabState},
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
    connection: TabConnection,
    state: TabState,
    zoom: f64,
    page_focus: bool,
}

impl Tab {
    fn title(&self) -> String {
        match self.state.navigation.as_ref() {
            Some(nav) if !nav.title.is_empty() => nav.title.clone(),
            Some(nav) if nav.url.as_str() != "about:blank" => {
                nav.url.host_str().unwrap_or(nav.url.as_str()).to_owned()
            }
            _ => "New tab".to_owned(),
        }
    }

    fn url(&self) -> &str {
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
    host: BrowserHost,
    surface: Entity<MoliSurface>,
    address: Entity<Address>,
    tabs: Vec<Tab>,
    active: TabId,
    next_id: TabId,
    closed: Vec<String>,
    popup: Option<Popup>,
    displayed_address: String,
    tab_scroll: ScrollHandle,
    _address_events: Subscription,
    _address_updates: Subscription,
}

impl BrowserWindow {
    pub fn new(
        host: BrowserHost,
        initial_url: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let address = cx.new(Address::new);
        let address_updates = cx.observe(&address, |_, _, cx| cx.notify());
        let surface = cx.new(|cx| MoliSurface::new(window, cx));
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
        if let Some(tab) = self.current() {
            let _ = tab.connection.commands.send(command);
        }
    }

    fn add_tab(&mut self, url: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let connection = match self.host.open_tab() {
            Ok(connection) => connection,
            Err(error) => {
                eprintln!("Could not open tab: {error}");
                return;
            }
        };
        let id = self.next_id;
        self.next_id += 1;
        let mut state = connection.state.clone();
        let mut frames = connection.frames.clone();
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
                                browser.present_frame(window, cx);
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
            connection,
            state: TabState::default(),
            zoom: 1.,
            page_focus: url.is_some(),
        });
        self.select_tab(id, window, cx);
        if let Some(url) = url {
            self.open_address(&url, window, cx);
        }
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
        let commands = tab.connection.commands.clone();
        let zoom = tab.zoom;
        let page_focus = tab.page_focus;
        let document = tab.state.document;
        self.active = id;
        self.popup = None;
        self.surface.update(cx, |surface, cx| {
            surface.select_tab(commands, zoom, document, window, cx)
        });
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
        self.closed.push(tab.url().to_owned());
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
            self.add_tab(Some(url), window, cx);
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
            self.present_frame(window, cx);
        }
        cx.notify();
    }

    fn present_frame(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.current() else { return };
        let frame = tab.connection.frames.borrow().clone();
        if let Some(frame) = frame.filter(|frame| Some(frame.document) == tab.state.document) {
            self.surface
                .update(cx, |surface, cx| surface.show_frame(&frame, window, cx));
        }
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
        if let Some(url) = address_target(text) {
            let url = url.to_string();
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
                .child(glyph(
                    if busy { Icon::Loading } else { Icon::Page },
                    if busy { colors.accent } else { colors.muted },
                ))
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
        let blank = self
            .current()
            .is_some_and(|tab| tab.url() == "about:blank" && !tab.state.loading);
        let surface = div()
            .relative()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(self.surface.clone());
        let surface = if let Some(error) = error {
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
        } else if blank {
            surface.child(
                div()
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
            surface
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
    cx.set_menus(vec![
        Menu::new("molio").items([MenuItem::action("Quit molio", Quit)]),
    ]);
}

fn address_target(input: &str) -> Option<url::Url> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }
    if let Ok(url) = url::Url::parse(input)
        && matches!(url.scheme(), "http" | "https" | "about" | "data" | "file")
    {
        return Some(url);
    }
    if !input.contains(char::is_whitespace) {
        let host = input.split('/').next().unwrap_or(input);
        let local = host == "localhost"
            || host.starts_with("localhost:")
            || host.starts_with("127.")
            || host.starts_with("[::1]");
        if (local || host.contains('.'))
            && let Ok(url) = url::Url::parse(&format!(
                "{}://{input}",
                if local { "http" } else { "https" }
            ))
        {
            return Some(url);
        }
    }
    let mut search = url::Url::parse("https://duckduckgo.com/").expect("search URL");
    search.query_pairs_mut().append_pair("q", input);
    Some(search)
}
