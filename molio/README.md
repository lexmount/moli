# molio desktop browser

From the workspace root, run the desktop app with a URL or leave the URL out to open a blank tab.

```sh
cargo run --locked -p molio -- https://example.org
```

Use `--profile-dir DIRECTORY` to persist your molio profile. The address field opens URLs and submits search terms to DuckDuckGo. Address suggestions use visits from the current session.

Files save to your system's Downloads folder. Open the Downloads tab from the toolbar, the browser menu, or with Ctrl+J (Cmd+J on macOS). Search by file name or source, cancel a transfer, open a finished file, or reveal it in its folder. With `--profile-dir`, Molio restores download history on startup.

The GPUI-CE window displays software frames from Moli's Realtime renderer. The raster worker writes into GPUI-allocated BGRA buffers, and the window uploads each completed frame by borrowing its pixels. The UI and platform crates are pinned to the same Git revision. Tabs have independent navigation history, zoom, and focus state. The page surface accepts keyboard, mouse, composition input, text clipboard shortcuts, and dropped files.

macOS uses GPUI-CE's Metal backend. Debian and Ubuntu workspace builds need `pkg-config`, `libfontconfig1-dev`, `libwayland-dev`, `libxcb1-dev`, `libxkbcommon-dev`, and `libxkbcommon-x11-dev` in addition to the workspace's regular build dependencies.

The desktop app currently runs the renderer on a dedicated thread in the same process. Process isolation and native input validation across macOS, Windows, X11, and Wayland are open.
