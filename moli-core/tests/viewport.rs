use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use moli_core::{
    LayoutPolicy, RendererViewportFrame,
    page::{ViewportBuffers, ViewportFrames, ViewportOutput},
    protocol_types::{DocumentActivity, ViewportSurface},
    runtime::{Browser, BrowserConfig},
};

fn html_url(html: &str) -> String {
    let html: String = url::form_urlencoded::byte_serialize(html.as_bytes()).collect();
    format!("data:text/html;charset=utf-8,{}", html.replace('+', "%20"))
}

async fn frame(
    frames: &mut ViewportFrames,
    condition: impl Fn(&RendererViewportFrame) -> bool,
) -> Result<RendererViewportFrame> {
    tokio::time::timeout(
        Duration::from_secs(5),
        frames
            .updates
            .wait_for(|frame| frame.as_ref().is_some_and(&condition)),
    )
    .await??
    .clone()
    .context("viewport frame disappeared")
}

#[tokio::test]
async fn viewport_frames_follow_realtime_animation_and_document_replacement() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser.fetch(&html_url(r#"
        <title>First</title><div id="box" style="width:20px;height:20px;background:red"></div>
        <script>
          let count = 0;
          function update() { box.style.marginLeft = (++count % 100) + 'px'; requestAnimationFrame(update); }
          requestAnimationFrame(update);
        </script>
    "#)).await?;
    let surface = ViewportSurface {
        inner_width: 320,
        inner_height: 240,
        device_pixel_ratio: 2.,
        ..ViewportSurface::default()
    };
    let mut frames = page.subscribe_viewport_frames(surface, 7).await?;
    let first = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;
    assert_eq!(first.navigation.title, "First");
    assert_eq!(first.viewport_revision, 7);
    let first_surface = first.snapshot.as_ref().context("first paint")?.surface;
    assert_eq!(first_surface.css_width, 320.);
    assert_eq!(first_surface.css_height, 240.);
    assert_eq!(first_surface.device_scale, 2.);
    let next = frame(&mut frames, |frame| frame.sequence > first.sequence).await?;
    assert_eq!(first.document, next.document);
    assert_ne!(first.snapshot, next.snapshot);

    let next_url = html_url("<title>Second</title><p>Document replacement</p>");
    let pending = page.start_viewport_navigation(Some(next_url.clone()))?;
    assert!(page.finish_viewport_navigation(pending.wait().await?)?);
    let replacement = frame(&mut frames, |frame| frame.document != first.document).await?;
    assert_eq!(replacement.navigation.title, "Second");
    assert_eq!(replacement.navigation.url.as_str(), next_url);
    assert!(replacement.navigation.can_go_back);
    assert_eq!(replacement.viewport_revision, 7);
    assert!(page.traverse_viewport_history(-1).await?);
    let back = frame(&mut frames, |frame| frame.navigation.title == "First").await?;
    assert!(back.navigation.can_go_forward);

    page.set_document_activity_async(DocumentActivity::new(false, false))
        .await?;
    frames.updates.borrow_and_update();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), frames.updates.changed())
            .await
            .is_err()
    );
    page.set_document_activity_async(DocumentActivity::new(true, true))
        .await?;
    let resized = ViewportSurface {
        inner_width: 410,
        inner_height: 250,
        device_pixel_ratio: 1.25,
        ..surface
    };
    let mut resumed = page.subscribe_viewport_frames(resized, 8).await?;
    let resumed = frame(&mut resumed, |frame| frame.viewport_revision == 8).await?;
    let resumed_surface = resumed.snapshot.as_ref().context("resumed paint")?.surface;
    assert_eq!(resumed_surface.css_width, 410.);
    assert_eq!(resumed_surface.css_height, 250.);
    assert_eq!(resumed_surface.device_scale, 1.25);
    Ok(())
}

#[tokio::test]
async fn viewport_paints_into_caller_buffers_without_waiting_for_control_updates() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser.fetch(&html_url(r#"
        <style>html{background:red} #box{width:8px;height:8px;background:blue}</style><div id="box"></div>
        <script>
          let offset = 0;
          function update() { box.style.marginLeft = (++offset % 20) + 'px'; requestAnimationFrame(update); }
          requestAnimationFrame(update);
        </script>
    "#)).await?;
    let surface = ViewportSurface {
        inner_width: 40,
        inner_height: 30,
        device_pixel_ratio: 2.,
        ..ViewportSurface::default()
    };
    let layout = ViewportBuffers::layout_for(surface)?;
    let storage = std::array::from_fn(|_| Vec::with_capacity(layout.byte_len));
    let addresses = storage.each_ref().map(|bytes| bytes.as_ptr());
    let output = Arc::new(ViewportOutput::default());
    let mut ready = output.subscribe();
    let binding = page
        .subscribe_viewport_buffer(
            surface,
            7,
            ViewportBuffers::new(layout, storage)?,
            output.clone(),
        )
        .await?;

    let mut sequence = 0;
    tokio::time::timeout(Duration::from_secs(5), async {
        for _ in 0..5 {
            loop {
                if let Some(Some(next)) = output.read(|frame| {
                    let pixels = frame.pixels?;
                    if frame.sequence <= sequence {
                        return None;
                    }
                    assert_eq!(frame.viewport_revision, 7);
                    assert_eq!(frame.layout, layout);
                    assert!(addresses.contains(&pixels.as_ptr()));
                    assert_eq!(pixels[..4], [0, 0, 255, 255]);
                    Some(frame.sequence)
                }) {
                    sequence = next;
                    break;
                }
                ready.changed().await?;
            }
        }
        Ok::<_, tokio::sync::watch::error::RecvError>(())
    })
    .await??;
    drop(binding);
    assert!(output.read(|_| ()).is_none());
    Ok(())
}

#[tokio::test]
async fn viewport_ime_replaces_composition_and_reports_utf16_offsets() -> Result<()> {
    use moli_core::{RendererViewportEdit, RendererViewportEditAction, page::PageInputExt};

    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser.fetch(&html_url("<title>Editor</title><input id='editor' value='A😀' style='position:absolute;left:0;top:0;width:200px;height:40px'>")).await?;
    let mut frames = page
        .subscribe_viewport_frames(ViewportSurface::default(), 1)
        .await?;
    let first = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;
    page.dispatch_mouse_event_at_point_async(15., 10., "mousedown", 0, Some(1), 0., 0.)
        .await?;
    page.dispatch_mouse_event_at_point_async(15., 10., "mouseup", 0, Some(0), 0., 0.)
        .await?;
    let original = page.viewport_editor().await?.context("focused editor")?;
    let edit = |sequence, previous, revision, action| RendererViewportEdit {
        document: first.document,
        target: original.target,
        revision,
        sequence,
        previous,
        action,
    };
    let compose = page
        .edit_viewport(edit(
            1,
            None,
            original.revision,
            RendererViewportEditAction::Compose {
                range: Some(3..3),
                text: "ni".into(),
                selection: Some(2..2),
            },
        ))
        .await?;
    assert!(compose.applied);
    let composing = compose.editor.context("composing editor")?;
    assert_eq!(composing.text, "A😀ni");
    assert_eq!(composing.marked, Some(3..5));
    assert!(composing.caret_rect.is_some());
    let commit = page
        .edit_viewport(edit(
            2,
            Some(1),
            original.revision,
            RendererViewportEditAction::Commit {
                range: None,
                text: "你".into(),
            },
        ))
        .await?;
    assert!(commit.applied);
    let committed = commit.editor.context("committed editor")?;
    assert_eq!(committed.text, "A😀你");
    assert_eq!(committed.selection, 4..4);
    assert_eq!(committed.marked, None);
    let compose_again = page
        .edit_viewport(edit(
            3,
            None,
            committed.revision,
            RendererViewportEditAction::Compose {
                range: Some(1..3),
                text: "かな".into(),
                selection: None,
            },
        ))
        .await?;
    assert!(compose_again.applied);
    let cancel = page
        .edit_viewport(edit(
            4,
            Some(3),
            committed.revision,
            RendererViewportEditAction::Cancel,
        ))
        .await?;
    assert!(cancel.applied);
    let restored = cancel.editor.context("restored editor")?;
    assert_eq!(restored.text, "A😀你");
    page.evaluate_runtime_expression_async("editor.setSelectionRange(1, 4)")
        .await?;
    assert_eq!(
        page.viewport_selected_text(first.document)
            .await?
            .as_deref(),
        Some("😀你")
    );
    let selected = page.viewport_editor().await?.context("selected editor")?;
    let pasted = page
        .edit_viewport(edit(
            5,
            None,
            selected.revision,
            RendererViewportEditAction::Paste {
                text: "日本語".into(),
            },
        ))
        .await?;
    assert!(pasted.applied);
    assert_eq!(pasted.editor.context("pasted editor")?.text, "A日本語");
    Ok(())
}

#[tokio::test]
async fn viewport_ime_replaces_a_selection_across_contenteditable_nodes() -> Result<()> {
    use moli_core::{RendererViewportEdit, RendererViewportEditAction};

    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let html = "<div id='editor' contenteditable><b>A</b><span>😀</span><i>B</i></div>";
    let mut page = browser.fetch(&html_url(html)).await?;
    let mut frames = page
        .subscribe_viewport_frames(ViewportSurface::default(), 1)
        .await?;
    let document = frame(&mut frames, |frame| frame.snapshot.is_some())
        .await?
        .document;
    let selection = page
        .evaluate_runtime_expression_async(
            "(() => {
                const editor = document.getElementById('editor');
                editor.focus();
                const selection = window.getSelection();
                selection.setBaseAndExtent(editor.firstChild.firstChild, 1,
                    editor.lastChild.firstChild, 1);
                return selection.toString();
            })()",
        )
        .await?;
    let editor = page.viewport_editor().await?.context("contenteditable")?;
    assert_eq!(editor.text, "A😀B");
    assert_eq!(editor.selection, 1..4, "{selection}");
    let edit = |sequence, previous, action| RendererViewportEdit {
        document,
        target: editor.target,
        revision: editor.revision,
        sequence,
        previous,
        action,
    };
    let composing = page
        .edit_viewport(edit(
            1,
            None,
            RendererViewportEditAction::Compose {
                range: None,
                text: "ni".into(),
                selection: Some(2..2),
            },
        ))
        .await?;
    assert!(composing.applied);
    let composing = composing.editor.context("marked contenteditable")?;
    assert_eq!(composing.text, "Ani");
    assert_eq!(composing.marked, Some(1..3));
    let committed = page
        .edit_viewport(edit(
            2,
            Some(1),
            RendererViewportEditAction::Commit {
                range: None,
                text: "你".into(),
            },
        ))
        .await?;
    assert!(committed.applied);
    assert_eq!(
        committed.editor.context("committed contenteditable")?.text,
        "A你"
    );
    Ok(())
}

#[tokio::test]
async fn viewport_file_drop_and_link_activation_use_the_current_document() -> Result<()> {
    use moli_core::page::{PageInputExt, RendererDraggedFile};
    use moli_test_support::FixtureServer;

    let server = FixtureServer::spawn().await?;
    let next = server.url("/");
    let html = format!(
        r#"<title>Drop target</title><a href="{next}" style="display:block;width:180px;height:60px">
        Next</a><script>
        window.addEventListener("dragover", event => event.preventDefault());
        window.addEventListener("drop", event => {{
            event.preventDefault();
            document.title = event.dataTransfer.files[0].name;
        }});
        </script>"#
    );
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser.fetch(&html_url(&html)).await?;
    let mut frames = page
        .subscribe_viewport_frames(ViewportSurface::default(), 1)
        .await?;
    let first = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;
    let file = RendererDraggedFile {
        bytes: b"local drop fixture".to_vec(),
        mime_type: "text/plain".into(),
        name: "Dropped.txt".into(),
        last_modified: 0.,
    };
    assert!(
        page.drop_viewport_files(first.document, (16., 16.), vec![file.clone()], 0)
            .await?
    );
    frame(&mut frames, |frame| frame.navigation.title == file.name).await?;
    page.dispatch_mouse_event_at_point_async(16., 16., "mousedown", 0, Some(1), 0., 0.)
        .await?;
    page.dispatch_mouse_event_at_point_async(16., 16., "mouseup", 0, Some(0), 0., 0.)
        .await?;
    let navigated = frame(&mut frames, |frame| frame.document != first.document).await?;
    assert_eq!(navigated.navigation.url.as_str(), next);
    assert!(
        !page
            .drop_viewport_files(first.document, (16., 16.), vec![file], 0)
            .await?
    );
    Ok(())
}
