mod address;
mod address_target;
mod browser;
mod download_page;
mod downloads;
mod favicon;
mod host;
mod icons;
mod palette;
mod surface;
mod text_input;
mod viewport_edit_sequence;

use std::{future, path::PathBuf, rc::Rc};

use gpui::{
    App, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, prelude::*, px, size,
};

use crate::{browser::BrowserWindow, host::BrowserHost};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut profile = None;
    let mut initial = None;
    let mut max_fps = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-h" | "--help" => {
                println!(
                    "Usage: molio [--profile-dir DIRECTORY] [--max-fps HZ] [URL]\n\n--max-fps HZ   Set the maximum realtime frame rate (0 for unlimited).\n               Omit to follow the window display clock."
                );
                return Ok(());
            }
            "--profile-dir" => {
                profile =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("missing --profile-dir value")
                    })?));
            }
            "--max-fps" => {
                let value = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing --max-fps value"))?;
                max_fps = Some(
                    value
                        .parse::<u32>()
                        .map_err(|_| anyhow::anyhow!("--max-fps must be a nonnegative integer"))?,
                );
            }
            _ if initial.is_none() => initial = Some(argument),
            _ => anyhow::bail!("unexpected argument {argument}"),
        }
    }
    let (host, host_events) = BrowserHost::start(profile, max_fps)?;
    let host = Rc::new(host);
    gpui_platform::application().run(move |cx: &mut App| {
        browser::bind_keys(cx);
        let quitting_host = Rc::clone(&host);
        cx.on_app_quit(move |_| {
            quitting_host.shutdown();
            future::ready(())
        })
        .detach();
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let bounds = Bounds::centered(None, size(px(1200.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("molio".into()),
                    appears_transparent: cfg!(target_os = "macos"),
                    traffic_light_position: Some(point(px(12.), px(12.))),
                }),
                app_owns_titlebar_drag: cfg!(target_os = "macos"),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(480.), px(300.))),
                app_id: Some("org.lexmount.molio".into()),
                ..Default::default()
            },
            move |window, cx| {
                cx.new(|cx| BrowserWindow::new(host, host_events, initial, window, cx))
            },
        )
        .expect("open browser window");
        cx.activate(true);
    });
    Ok(())
}
