mod address;
mod browser;
mod favicon;
mod host;
mod icons;
mod palette;
mod surface;
mod text_input;

use std::path::PathBuf;

use gpui::{
    App, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, prelude::*, px, size,
};

use crate::{browser::BrowserWindow, host::BrowserHost};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let mut profile = None;
    let mut initial = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "-h" | "--help" => {
                println!("Usage: molio [--profile-dir DIRECTORY] [URL]");
                return Ok(());
            }
            "--profile-dir" => {
                profile =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        anyhow::anyhow!("missing --profile-dir value")
                    })?));
            }
            _ if initial.is_none() => initial = Some(argument),
            _ => anyhow::bail!("unexpected argument {argument}"),
        }
    }
    let host = BrowserHost::start(profile)?;
    gpui_platform::application().run(move |cx: &mut App| {
        browser::bind_keys(cx);
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
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(480.), px(300.))),
                app_id: Some("org.lexmount.molio".into()),
                ..Default::default()
            },
            move |window, cx| cx.new(|cx| BrowserWindow::new(host, initial, window, cx)),
        )
        .expect("open browser window");
        cx.activate(true);
    });
    Ok(())
}
