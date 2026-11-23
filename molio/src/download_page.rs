use gpui::{
    Context, Entity, FocusHandle, FontWeight, Render, Subscription, Task, Window, div, prelude::*,
    px, relative,
};

use crate::{
    address::Address,
    downloads::{Download, DownloadClient, DownloadCommand, DownloadState},
    icons::{Icon, glyph},
    palette::Palette,
};

pub const URL: &str = "molio://downloads/";

pub fn is_downloads_url(input: &str) -> bool {
    input
        .trim()
        .trim_end_matches('/')
        .eq_ignore_ascii_case("molio://downloads")
}

pub struct DownloadsPage {
    pub focus: FocusHandle,
    search: Entity<Address>,
    client: DownloadClient,
    entries: Vec<Download>,
    notice: Option<&'static str>,
    _search_updates: Subscription,
    _download_updates: Task<()>,
}

impl DownloadsPage {
    pub fn new(client: DownloadClient, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| Address::with_placeholder("Search downloads", cx));
        let search_updates = cx.observe(&search, |_, _, cx| cx.notify());
        let mut updates = client.entries.clone();
        let entries = updates.borrow_and_update().clone();
        let download_updates = cx.spawn(async move |view, cx| {
            while updates.changed().await.is_ok() {
                let entries = updates.borrow_and_update().clone();
                if view
                    .update(cx, |page, cx| {
                        page.entries = entries;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            focus: cx.focus_handle(),
            search,
            client,
            entries,
            notice: None,
            _search_updates: search_updates,
            _download_updates: download_updates,
        }
    }

    fn row(&self, entry: &Download, colors: Palette, cx: &mut Context<Self>) -> gpui::Div {
        let id = entry.id;
        let detail = match &entry.state {
            DownloadState::Downloading => entry.total.map_or_else(
                || format!("{} downloaded", format_size(entry.received)),
                |total| format!("{} of {}", format_size(entry.received), format_size(total)),
            ),
            DownloadState::Complete(_) => format_size(entry.received),
            DownloadState::Canceled => "Canceled".to_owned(),
            DownloadState::Failed(reason) => format!("Download failed · {reason}"),
        };
        let mut name = div().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD);
        let mut actions = div().flex().flex_wrap().gap_2();
        if let DownloadState::Complete(path) = &entry.state {
            let file = path.clone();
            name = name.child(
                div()
                    .id(("open-download", id as usize))
                    .truncate()
                    .text_color(colors.accent)
                    .cursor_pointer()
                    .hover_style(|style| style.underline())
                    .on_click(cx.listener(move |page, _, _, cx| {
                        page.notice = if file.is_file() {
                            cx.open_with_system(&file);
                            None
                        } else {
                            Some("This file was moved or deleted.")
                        };
                        cx.notify();
                    }))
                    .child(entry.name.clone()),
            );
            let file = path.clone();
            actions = actions.child(
                page_button(("show-download", id as usize), "Show in folder", colors).on_click(
                    cx.listener(move |page, _, _, cx| {
                        page.notice = if file.is_file() {
                            cx.reveal_path(&file);
                            None
                        } else {
                            Some("This file was moved or deleted.")
                        };
                        cx.notify();
                    }),
                ),
            );
        } else {
            name = name.child(entry.name.clone());
        }
        let (label, command) = if entry.is_active() {
            ("Cancel", DownloadCommand::Cancel(id))
        } else {
            ("Remove", DownloadCommand::Remove(id))
        };
        let client = self.client.clone();
        actions = actions.child(
            page_button(("manage-download", id as usize), label, colors)
                .on_click(move |_, _, _| client.send(command)),
        );
        let progress = entry
            .total
            .filter(|&total| total > 0)
            .map(|total| (entry.received as f64 / total as f64).clamp(0.0, 1.0) as f32);
        div()
            .flex()
            .items_start()
            .gap_4()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(colors.border)
            .bg(colors.toolbar)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(38.))
                    .rounded_md()
                    .bg(colors.field)
                    .flex_shrink_0()
                    .child(glyph(Icon::Page, colors.accent)),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(name)
                    .when(!entry.source.is_empty(), |row| {
                        row.child(
                            div()
                                .text_sm()
                                .text_color(colors.muted)
                                .truncate()
                                .child(entry.source.clone()),
                        )
                    })
                    .child(div().text_sm().text_color(colors.muted).child(detail))
                    .when(entry.is_active(), |row| {
                        row.when_some(progress, |row, progress| {
                            row.child(
                                div()
                                    .h(px(4.))
                                    .w_full()
                                    .rounded_full()
                                    .overflow_hidden()
                                    .bg(colors.field)
                                    .child(div().h_full().w(relative(progress)).bg(colors.accent)),
                            )
                        })
                    })
                    .child(actions),
            )
    }
}

impl Render for DownloadsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = Palette::for_window(window.appearance());
        let query = self.search.read(cx).value().trim().to_lowercase();
        let entries = self
            .entries
            .iter()
            .filter(|entry| {
                query.is_empty()
                    || entry.name.to_lowercase().contains(&query)
                    || entry.source.to_lowercase().contains(&query)
            })
            .collect::<Vec<_>>();
        let has_finished = self.entries.iter().any(|entry| !entry.is_active());
        let root = div()
            .id("downloads-page")
            .track_focus(&self.focus)
            .size_full()
            .min_h_0()
            .overflow_y_scroll()
            .bg(colors.field)
            .text_color(colors.text)
            .px_5()
            .py_8();
        root.child(div().max_w(px(820.)).mx_auto().flex().flex_col().gap_4()
            .child(div().flex().flex_wrap().gap_3().items_center().justify_between()
                .child(div().flex().items_center().gap_3().text_2xl().font_weight(FontWeight::BOLD)
                    .child(glyph(Icon::Download, colors.accent)).child("Downloads"))
                .child(div().flex().flex_wrap().gap_2()
                    .child(page_button("downloads-folder", "Open downloads folder", colors)
                        .on_click(cx.listener(|page, _, _, cx| {
                            if page.client.directory.is_dir() {
                                cx.open_with_system(&page.client.directory);
                                page.notice = None;
                            } else {
                                page.notice = Some("The downloads folder will be created when a download starts.");
                            }
                            cx.notify();
                        })))
                    .when(has_finished, |row| row.child(page_button("clear-downloads", "Clear history", colors)
                        .on_click(cx.listener(|page, _, _, _| page.client.send(DownloadCommand::Clear)))))))
            .child(div().flex().items_center().h(px(40.)).px_3().rounded_md()
                .border_1().border_color(colors.border).bg(colors.toolbar).child(self.search.clone()))
            .when_some(self.notice, |page, notice| page.child(div().text_sm().text_color(colors.muted).child(notice)))
            .when(entries.is_empty(), |page| page.child(div()
                .p_8().rounded_lg().border_1().border_color(colors.border).bg(colors.toolbar)
                .text_color(colors.muted).text_center().child(if query.is_empty() {
                    "No downloads yet"
                } else { "No matching downloads" })))
            .children(entries.into_iter().map(|entry| self.row(entry, colors, cx))))
    }
}

fn page_button(
    id: impl Into<gpui::ElementId>,
    label: &'static str,
    colors: Palette,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_md()
        .border_1()
        .border_color(colors.border)
        .bg(colors.toolbar)
        .text_sm()
        .cursor_pointer()
        .hover_style(move |style| style.bg(colors.hover))
        .child(label)
}

fn format_size(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..MIB => format!("{:.1} KiB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MiB", bytes as f64 / MIB as f64),
    }
}
