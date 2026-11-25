use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use moli_core::{
    LayoutPolicy, RendererPageNavigationState, RendererViewportCursor,
    RendererViewportCursorKind as Cursor, RendererViewportFrame, RendererViewportPointerInput,
    RendererViewportWheelInput, RendererViewportWheelPhase,
    page::{PageInputExt, ViewportBuffers, ViewportFrames, ViewportOutput},
    protocol_types::{DocumentActivity, ViewportSurface},
    runtime::{Browser, BrowserConfig},
};
use tokio::sync::watch;

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

async fn page_state(
    states: &mut watch::Receiver<Option<RendererPageNavigationState>>,
    condition: impl Fn(&RendererPageNavigationState) -> bool,
) -> Result<RendererPageNavigationState> {
    tokio::time::timeout(
        Duration::from_secs(5),
        states.wait_for(|state| state.as_ref().is_some_and(&condition)),
    )
    .await??
    .clone()
    .context("viewport page state disappeared")
}

#[tokio::test]
async fn background_popup_activity_is_set_before_exposure_and_preserved_by_adoption() -> Result<()>
{
    use moli_core::{
        RendererOutputItem, RendererOutputTransportMessage, RendererOwnerAction,
        page::RendererPopupDisposition,
    };

    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut output = browser.subscribe_viewport_actions();
    let mut opener = browser
        .fetch(&html_url(
            r#"<!doctype html><style>body { margin: 0; }</style>
                <button id="open">Open background tab</button>
                <script>
                    document.getElementById("open").onclick = () => {
                        const child = window.open("about:blank", "background-probe");
                        document.title = child.document.visibilityState + ":" + child.document.hasFocus();
                    };
                </script>"#,
        ))
        .await?;
    let mut frames = opener
        .subscribe_viewport_frames(ViewportSurface::default(), 1)
        .await?;
    frame(&mut frames, |frame| frame.snapshot.is_some()).await?;

    let new_tab_modifier = if cfg!(target_os = "macos") {
        0b100
    } else {
        0b010
    };
    for (event, buttons) in [("mousedown", 1), ("mouseup", 0)] {
        let pending = opener.start_dispatch_mouse_event_at_point_with_pointer_outcome(
            12.,
            10.,
            event,
            0,
            Some(buttons),
            1,
            0.,
            0.,
            Default::default(),
            new_tab_modifier,
            false,
        )?;
        opener.finish_dispatch_mouse_event_at_point_with_outcome(pending.wait().await?)?;
    }
    let popup = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(item) = output.recv().await {
            if let RendererOutputTransportMessage::Publication(publication) = item {
                for record in publication.into_records() {
                    if let RendererOutputItem::OwnerAction(RendererOwnerAction::Popup(popup)) =
                        record.into_parts().1
                    {
                        return Some(popup.into_parts());
                    }
                }
            }
        }
        None
    })
    .await?
    .context("background popup activation")?;
    assert_eq!(popup.disposition, RendererPopupDisposition::Background);
    let initial = opener
        .evaluate_runtime_expression_async("document.title")
        .await?;
    assert_eq!(initial["value"], "hidden:false", "synchronous opener read");

    let mut child = browser.adopt_viewport_page(&popup).await?;
    const ACTIVITY: &str = "document.visibilityState + ':' + document.hasFocus()";
    let adopted = child.evaluate_runtime_expression_async(ACTIVITY).await?;
    assert_eq!(adopted["value"], "hidden:false", "adopted background tab");

    child
        .set_document_activity_async(DocumentActivity::new(true, true))
        .await?;
    let selected = child.evaluate_runtime_expression_async(ACTIVITY).await?;
    assert_eq!(selected["value"], "visible:true", "selected tab");
    Ok(())
}

async fn cursor(frames: &mut ViewportFrames, kind: Cursor) -> Result<RendererViewportCursor> {
    tokio::time::timeout(
        Duration::from_secs(5),
        frames
            .cursor
            .wait_for(|cursor| cursor.is_some_and(|cursor| cursor.kind == kind)),
    )
    .await
    .with_context(|| format!("waiting for {kind:?} cursor"))??
    .context("viewport cursor disappeared")
}

#[tokio::test]
async fn viewport_cursor_uses_the_hit_element_text_and_computed_style() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser.fetch(&html_url(r##"<!doctype html><style>
        body { margin: 0; font: 20px/30px sans-serif; }
        .row { position: absolute; height: 30px; width: 100px; }
        #link { top: 0; }
        #overlay { top: 0; pointer-events: none; cursor: wait; }
        #input { top: 40px; }
        #hover { top: 80px; cursor: crosshair; }
        #hover:hover { cursor: grab; }
        #text { top: 120px; }
        #noselect { top: 160px; user-select: none; }
        #editable { top: 200px; }
        iframe { position: absolute; left: 120px; top: 0; border: 0; width: 80px; height: 50px; }
    </style>
    <a class="row" id="link" href="#link"><span>Link</span></a>
    <div class="row" id="overlay"></div>
    <input class="row" id="input" value="Text box">
    <div class="row" id="hover"><span>Hover</span></div>
    <div class="row" id="text">Read this</div>
    <div class="row" id="noselect"><span>Static</span></div>
    <div class="row" id="editable" contenteditable></div>
    <iframe srcdoc='<style>body { margin:0 } div { width:80px; height:50px; cursor:ew-resize }</style><div></div>'></iframe>
    <script>globalThis.moves = 0; addEventListener('mousemove', () => ++moves);</script>"##)).await?;
    let surface = ViewportSurface {
        inner_width: 240,
        inner_height: 280,
        ..ViewportSurface::default()
    };
    let mut frames = page.subscribe_viewport_frames(surface, 1).await?;
    frame(&mut frames, |frame| frame.snapshot.is_some()).await?;

    for (x, y, kind) in [
        (10., 15., Cursor::Pointer),
        (10., 55., Cursor::Text),
        (10., 95., Cursor::Grab),
        (10., 135., Cursor::Text),
        (10., 175., Cursor::Default),
        (80., 215., Cursor::Text),
        (140., 15., Cursor::EwResize),
        (200., 250., Cursor::Default),
        (10., 55., Cursor::Text),
    ] {
        page.dispatch_mouse_event_at_point_async(x, y, "mousemove", -1, Some(0), 0., 0.)
            .await?;
        cursor(&mut frames, kind)
            .await
            .with_context(|| format!("point ({x}, {y})"))?;
    }

    let before = page.evaluate_runtime_expression_async("moves").await?;
    page.evaluate_runtime_expression_async("input.style.cursor = 'url(missing.cur), crosshair'")
        .await?;
    cursor(&mut frames, Cursor::Crosshair).await?;
    assert_eq!(
        page.evaluate_runtime_expression_async("moves").await?["value"],
        before["value"]
    );
    Ok(())
}

#[tokio::test]
async fn viewport_cursor_uses_pseudo_and_slotted_text_styles() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser
        .fetch(&html_url(
            r#"<!doctype html><style>
                body { margin: 0; font: 20px/30px sans-serif; }
                #generated, #slotted { width: 150px; height: 30px; cursor: default; }
                #generated::before {
                    content: ""; display: block; width: 60px; height: 30px;
                    background: silver; cursor: text;
                }
                #generated:hover::before { cursor: crosshair; }
            </style><div id="generated"></div><div id="slotted">Slotted text</div>
            <script>
                const shadow = slotted.attachShadow({ mode: "open" });
                shadow.innerHTML =
                    '<style>slot { cursor: wait; }</style><slot></slot>';
                const slot = shadow.querySelector('slot');
                globalThis.pointerSamples = [];
                addEventListener('mousemove', event => {
                    pointerSamples.push(event.target.id + ':' + slot.matches(':hover'));
                });
            </script>"#,
        ))
        .await?;
    let surface = ViewportSurface {
        inner_width: 160,
        inner_height: 100,
        ..ViewportSurface::default()
    };
    let mut frames = page.subscribe_viewport_frames(surface, 1).await?;
    let initial = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;

    page.dispatch_mouse_event_at_point_async(20., 15., "mousemove", -1, Some(0), 0., 0.)
        .await?;
    cursor(&mut frames, Cursor::Crosshair).await?;
    let hovered = frame(&mut frames, |frame| frame.sequence > initial.sequence).await?;

    page.set_viewport_pointer_position(1, Some((100., 15.)))
        .await?;
    cursor(&mut frames, Cursor::Default).await?;
    assert_eq!(
        frames.updates.borrow().as_ref().unwrap().sequence,
        hovered.sequence
    );

    page.dispatch_mouse_event_at_point_async(20., 45., "mousemove", -1, Some(0), 0., 0.)
        .await?;
    cursor(&mut frames, Cursor::Wait).await?;

    page.set_viewport_pointer_position(1, None).await?;
    assert_eq!(*frames.cursor.borrow(), None);
    page.set_viewport_pointer_position(1, Some((20., 45.)))
        .await?;
    cursor(&mut frames, Cursor::Wait).await?;
    assert_eq!(
        page.evaluate_runtime_expression_async(
            "slotted.shadowRoot.querySelector('slot').matches(':hover')"
        )
        .await?["value"],
        true
    );
    assert_eq!(
        page.evaluate_runtime_expression_async("JSON.stringify(pointerSamples)")
            .await?["value"],
        r#"["generated:false","slotted:true"]"#
    );
    Ok(())
}

#[tokio::test]
async fn viewport_cursor_auto_respects_generated_text_selectability() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser
        .fetch(&html_url(
            r#"<!doctype html><style>
                body { margin: 0; font: 32px/48px monospace; }
                #generated::before, #generated::after {
                    content: "ICON"; display: block;
                    cursor: auto; user-select: auto;
                }
                #generated.explicit::before, #generated.explicit::after { cursor: text; }
                #generated.selectable::before, #generated.selectable::after { user-select: text; }
            </style><div id="generated"></div>"#,
        ))
        .await?;
    let surface = ViewportSurface {
        inner_width: 160,
        inner_height: 104,
        ..ViewportSurface::default()
    };
    let mut frames = page.subscribe_viewport_frames(surface, 1).await?;
    frame(&mut frames, |frame| frame.snapshot.is_some()).await?;

    for (y, pseudo) in [(24., "::before"), (72., "::after")] {
        page.evaluate_runtime_expression_async("generated.className = ''")
            .await?;
        page.set_viewport_pointer_position(1, Some((12., y)))
            .await?;
        let automatic = tokio::time::timeout(
            Duration::from_secs(5),
            frames.cursor.wait_for(Option::is_some),
        )
        .await??
        .context("generated text cursor disappeared")?;
        assert_eq!(automatic.kind, Cursor::Default, "{pseudo}");

        for (class, expected) in [
            ("explicit", Cursor::Text),
            ("", Cursor::Default),
            ("selectable", Cursor::Text),
        ] {
            page.evaluate_runtime_expression_async(&format!("generated.className = '{class}'"))
                .await?;
            cursor(&mut frames, expected)
                .await
                .with_context(|| format!("{pseudo} with class {class:?}"))?;
        }
        page.set_viewport_pointer_position(1, None).await?;
        assert_eq!(*frames.cursor.borrow(), None);
    }
    Ok(())
}

#[tokio::test]
async fn viewport_cursor_follows_scroll_document_and_viewport_without_a_mouse_move() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let html = r#"<!doctype html><style>
        body { margin: 0; }
        div { height: 100px; cursor: pointer; }
        div + div { cursor: crosshair; height: 500px; }
        div + div:hover { cursor: grab; }
    </style><div id="first"></div><div id="second"></div>
    <script>
        globalThis.moves = 0;
        addEventListener('mousemove', () => ++moves);
    </script>"#;
    let mut page = browser.fetch(&html_url(html)).await?;
    let surface = ViewportSurface {
        inner_width: 160,
        inner_height: 120,
        ..ViewportSurface::default()
    };
    let mut frames = page.subscribe_viewport_frames(surface, 7).await?;
    let initial = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;

    page.set_viewport_pointer_position(7, Some((40., 40.)))
        .await?;
    let first = cursor(&mut frames, Cursor::Pointer).await?;
    assert_eq!(
        (first.document, first.viewport_revision),
        (initial.document, 7)
    );
    assert_eq!(
        page.evaluate_runtime_expression_async("first.matches(':hover')")
            .await?["value"],
        true
    );
    page.set_viewport_pointer_position(6, None).await?;
    assert_eq!(*frames.cursor.borrow(), Some(first));

    page.evaluate_runtime_expression_async("scrollTo(0, 120)")
        .await?;
    cursor(&mut frames, Cursor::Grab).await?;
    assert_eq!(
        page.evaluate_runtime_expression_async(
            "JSON.stringify([first.matches(':hover'), second.matches(':hover'), moves])"
        )
        .await?["value"],
        "[false,true,0]"
    );
    let pending = page.start_viewport_navigation(Some(html_url(
        "<textarea style='width:100px;height:80px'></textarea>",
    )))?;
    assert!(page.finish_viewport_navigation(pending.wait().await?)?);
    let second = cursor(&mut frames, Cursor::Text).await?;
    assert_ne!(second.document, first.document);
    assert_eq!(second.viewport_revision, 7);

    let resized = ViewportSurface {
        inner_width: 200,
        ..surface
    };
    let mut resized_frames = page.subscribe_viewport_frames(resized, 8).await?;
    frame(&mut resized_frames, |frame| frame.snapshot.is_some()).await?;
    assert_eq!(*resized_frames.cursor.borrow(), None);
    page.set_viewport_pointer_position(8, Some((40., 40.)))
        .await?;
    let cursor = cursor(&mut resized_frames, Cursor::Text).await?;
    assert_eq!(
        (cursor.document, cursor.viewport_revision),
        (second.document, 8)
    );
    page.set_viewport_pointer_position(8, None).await?;
    assert_eq!(*resized_frames.cursor.borrow(), None);
    assert_eq!(
        page.evaluate_runtime_expression_async(
            "document.querySelector('textarea').matches(':hover')"
        )
        .await?["value"],
        false
    );
    Ok(())
}

#[tokio::test]
async fn queued_viewport_wheel_preserves_input_order_and_document_scope() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let html = r#"<!doctype html><style>body { height: 1000px; margin: 0; }</style>
        <script>
            globalThis.inputOrder = [];
            addEventListener('wheel', event => {
                inputOrder.push(event.deltaY);
                if (event.deltaY === 30) event.preventDefault();
            }, { passive: false });
            addEventListener('mousedown', () => inputOrder.push('down'));
        </script><p>Scroll test</p>"#;
    let mut page = browser.fetch(&html_url(html)).await?;
    let viewport = ViewportSurface {
        inner_width: 160,
        inner_height: 120,
        ..ViewportSurface::default()
    };
    let mut frames = page.subscribe_viewport_frames(viewport, 1).await?;
    let document = frame(&mut frames, |frame| frame.snapshot.is_some())
        .await?
        .document;
    let wheel = RendererViewportWheelInput {
        document,
        viewport_revision: 1,
        position: (16., 16.),
        buttons: 0,
        delta: (0., 20.),
        modifiers: 0,
        phase: RendererViewportWheelPhase::Moved,
        precise: true,
    };
    page.enqueue_viewport_wheel(wheel)?;
    page.dispatch_mouse_event_at_point_async(16., 16., "mousedown", 0, Some(1), 0., 0.)
        .await?;
    page.dispatch_mouse_event_at_point_async(16., 16., "mouseup", 0, Some(0), 0., 0.)
        .await?;
    page.enqueue_viewport_wheel(RendererViewportWheelInput {
        delta: (0., 30.),
        ..wheel
    })?;
    const SCROLL_AND_INPUT: &str = "`${scrollY}:${inputOrder.join(',')}`";
    let result = page
        .evaluate_runtime_expression_async(SCROLL_AND_INPUT)
        .await?;
    assert_eq!(result["value"], "20:20,down,30");

    let next_url = html_url(&format!("{html} "));
    let pending = page.start_viewport_navigation(Some(next_url))?;
    assert!(page.finish_viewport_navigation(pending.wait().await?)?);
    let replacement = frame(&mut frames, |frame| frame.document != document).await?;
    page.enqueue_viewport_wheel(wheel)?;
    page.enqueue_viewport_wheel(RendererViewportWheelInput {
        document: replacement.document,
        viewport_revision: 0,
        ..wheel
    })?;
    let result = page
        .evaluate_runtime_expression_async(SCROLL_AND_INPUT)
        .await?;
    assert_eq!(result["value"], "0:");
    Ok(())
}

#[tokio::test]
async fn viewport_wheel_zero_began_waits_for_a_scroll_direction() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let html = r#"<!doctype html><body style="height:1200px;margin:0">
        <div id="inner" style="width:100px;height:100px;overflow:auto">
            <div style="height:300px">Inner scroller</div>
        </div>
        <script>
            window.deltas = [];
            inner.addEventListener('wheel', event => deltas.push(event.deltaY));
        </script>"#;
    let mut page = browser.fetch(&html_url(html)).await?;
    let mut frames = page
        .subscribe_viewport_frames(
            ViewportSurface {
                inner_width: 160,
                inner_height: 120,
                ..Default::default()
            },
            1,
        )
        .await?;
    let document = frame(&mut frames, |frame| frame.snapshot.is_some())
        .await?
        .document;
    let began = RendererViewportWheelInput {
        document,
        viewport_revision: 1,
        position: (16., 16.),
        buttons: 0,
        delta: (0., 0.),
        modifiers: 0,
        phase: RendererViewportWheelPhase::Started,
        precise: true,
    };
    page.enqueue_viewport_wheel(began)?;
    page.enqueue_viewport_wheel(RendererViewportWheelInput {
        phase: RendererViewportWheelPhase::Moved,
        delta: (0., 40.),
        ..began
    })?;
    let result = page
        .evaluate_runtime_expression_async("JSON.stringify([inner.scrollTop,scrollY,deltas])")
        .await?;
    assert_eq!(result["value"], "[40,0,[0,40]]");
    Ok(())
}

#[tokio::test]
async fn viewport_pointer_requires_a_current_rendered_document() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let html = r#"<!doctype html><body style="margin:0">
        <div style="width:100px;height:100px">Pointer target</div>
        <script>window.moves = 0; addEventListener('mousemove', () => moves++);</script>"#;
    let mut page = browser.fetch(&html_url(html)).await?;
    let mut navigation = page.subscribe_navigation_state().await?;
    let document =
        tokio::time::timeout(Duration::from_secs(5), navigation.wait_for(Option::is_some))
            .await??
            .as_ref()
            .context("viewport document")?
            .document;
    let input = RendererViewportPointerInput {
        document,
        viewport_revision: 1,
        position: (16., 16.),
        event: "mousemove",
        button: -1,
        buttons: 0,
        click_count: 0,
        modifiers: 0,
    };
    assert!(page.dispatch_viewport_pointer(input).await?.is_none());

    let surface = ViewportSurface::default();
    let mut frames = page.subscribe_viewport_frames(surface, 1).await?;
    let initial = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;
    assert!(page.dispatch_viewport_pointer(input).await?.is_some());
    assert_eq!(
        page.evaluate_runtime_expression_async("moves").await?["value"],
        1
    );

    let mut frames = page.subscribe_viewport_frames(surface, 2).await?;
    assert!(page.dispatch_viewport_pointer(input).await?.is_none());
    let resized = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;
    let current_input = RendererViewportPointerInput {
        viewport_revision: resized.viewport_revision,
        ..input
    };
    page.evaluate_runtime_expression_async(
        "window.savedRoot = document.documentElement; savedRoot.remove();",
    )
    .await?;
    assert!(
        page.dispatch_viewport_pointer(current_input)
            .await?
            .is_none()
    );
    page.evaluate_runtime_expression_async("document.append(savedRoot)")
        .await?;
    frame(&mut frames, |frame| {
        frame.sequence > resized.sequence && frame.snapshot.is_some()
    })
    .await?;
    assert!(
        page.dispatch_viewport_pointer(current_input)
            .await?
            .is_some()
    );
    assert_eq!(
        page.evaluate_runtime_expression_async("moves").await?["value"],
        2
    );

    let next_url = html_url(&format!("{html} "));
    let pending = page.start_viewport_navigation(Some(next_url))?;
    assert!(page.finish_viewport_navigation(pending.wait().await?)?);
    let replacement = frame(&mut frames, |frame| {
        frame.document != initial.document && frame.snapshot.is_some()
    })
    .await?;
    assert!(
        page.dispatch_viewport_pointer(current_input)
            .await?
            .is_none()
    );
    assert_eq!(
        page.evaluate_runtime_expression_async("moves").await?["value"],
        0
    );
    assert!(
        page.dispatch_viewport_pointer(RendererViewportPointerInput {
            document: replacement.document,
            ..current_input
        })
        .await?
        .is_some()
    );
    assert_eq!(
        page.evaluate_runtime_expression_async("moves").await?["value"],
        1
    );
    Ok(())
}

#[tokio::test]
async fn viewport_pointer_events_keywords_restore_link_targeting() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let html = r##"<!doctype html><style>
        body { margin: 0; }
        #background, #sidebar { position: absolute; inset: 0; }
        #sidebar { pointer-events: none; }
        #next { display: block; width: 80px; height: 40px; pointer-events: POINTER_EVENTS_KEYWORD; }
        #title { display: block; height: 100%; }
    </style>
    <div id="background"></div>
    <div id="sidebar"><a id="next" href="#related"><span id="title">Next video</span></a></div>
    <div id="related"></div>
    <script>
        document.addEventListener('click', event => {
            window.clicked = [event.target.id, event.isTrusted];
        });
    </script>"##;
    for keyword in [
        "visiblePainted",
        "visibleFill",
        "visibleStroke",
        "visible",
        "painted",
        "fill",
        "stroke",
        "all",
    ] {
        let html = html.replace("POINTER_EVENTS_KEYWORD", keyword);
        let mut page = browser.fetch(&html_url(&html)).await?;
        let mut frames = page
            .subscribe_viewport_frames(ViewportSurface::default(), 1)
            .await?;
        let current = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;

        let hit = page
            .evaluate_runtime_expression_async(&format!(
                r#"[
                    CSS.supports('pointer-events', '{keyword}'),
                    getComputedStyle(document.getElementById('title')).pointerEvents,
                    document.elementFromPoint(12, 12).id,
                    document.elementFromPoint(120, 12).id
                ].join('|')"#,
            ))
            .await?;
        let expected_hit = format!("true|{}|title|background", keyword.to_ascii_lowercase());
        assert_eq!(hit["value"], expected_hit, "pointer-events: {keyword}");

        for (event, buttons) in [("mousedown", 1), ("mouseup", 0)] {
            assert!(
                page.dispatch_viewport_pointer(RendererViewportPointerInput {
                    document: current.document,
                    viewport_revision: current.viewport_revision,
                    position: (12., 12.),
                    event,
                    button: 0,
                    buttons,
                    click_count: 1,
                    modifiers: 0,
                })
                .await?
                .is_some(),
                "pointer-events: {keyword}"
            );
        }
        let result = page
            .evaluate_runtime_expression_async("JSON.stringify([window.clicked, location.hash])")
            .await?;
        assert_eq!(
            result["value"], r##"[["title",true],"#related"]"##,
            "pointer-events: {keyword}"
        );
        page.close_async().await?;
    }
    Ok(())
}

#[tokio::test]
async fn viewport_wheel_preserves_the_accepted_buttons_and_iframe_drag_route() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let html = r#"<!doctype html>
        <iframe id="child" style="position:absolute;left:40px;top:25px;width:120px;height:100px;border:0"
            srcdoc="<body style='margin:0'>
                <div style='width:100px;height:80px'></div>
                <script>
                    window.inputEvents = [];
                    addEventListener('wheel', event => {
                        inputEvents.push('wheel:' + event.buttons);
                        event.preventDefault();
                    }, {passive: false});
                    for (const type of ['mousedown', 'mousemove', 'mouseup']) {
                        addEventListener(type, () => inputEvents.push(type));
                    }
                </script>"></iframe>
        <script>
            window.parentMouseUps = 0;
            addEventListener('mouseup', () => parentMouseUps++);
        </script>"#;
    let mut page = browser.fetch(&html_url(html)).await?;
    let mut states = page.subscribe_navigation_state().await?;
    let document = page_state(&mut states, |_| true).await?.document;
    let press = RendererViewportPointerInput {
        document,
        viewport_revision: 1,
        position: (56., 41.),
        event: "mousedown",
        button: 0,
        buttons: 1,
        click_count: 1,
        modifiers: 0,
    };
    let release = RendererViewportPointerInput {
        event: "mouseup",
        buttons: 0,
        ..press
    };
    assert!(page.dispatch_viewport_pointer(press).await?.is_none());

    let mut frames = page
        .subscribe_viewport_frames(
            ViewportSurface {
                inner_width: 320,
                inner_height: 180,
                ..ViewportSurface::default()
            },
            press.viewport_revision,
        )
        .await?;
    let ready = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;
    assert_eq!(ready.document, document);
    let wheel = RendererViewportWheelInput {
        document,
        viewport_revision: press.viewport_revision,
        position: press.position,
        delta: (0., 12.),
        buttons: 1,
        modifiers: 0,
        phase: RendererViewportWheelPhase::Moved,
        precise: true,
    };
    page.enqueue_viewport_wheel(wheel)?;
    assert!(page.dispatch_viewport_pointer(release).await?.is_none());
    // Press again at the same point, before any idle mousemove can clear buttons.
    assert!(page.dispatch_viewport_pointer(press).await?.is_some());
    page.enqueue_viewport_wheel(RendererViewportWheelInput {
        buttons: 0,
        ..wheel
    })?;

    let outside = (220., 41.);
    assert!(
        page.dispatch_viewport_pointer(RendererViewportPointerInput {
            position: outside,
            event: "mousemove",
            button: -1,
            ..press
        })
        .await?
        .is_some()
    );
    assert!(
        page.dispatch_viewport_pointer(RendererViewportPointerInput {
            position: outside,
            ..release
        })
        .await?
        .is_some()
    );
    let events = page
        .evaluate_runtime_expression_async(
            "`${child.contentWindow.inputEvents.join(',')}|${parentMouseUps}`",
        )
        .await?;
    assert_eq!(
        events["value"],
        "wheel:0,mousedown,wheel:1,mousemove,mouseup|0"
    );
    Ok(())
}

#[tokio::test]
async fn viewport_pressed_buttons_expire_when_document_open_restarts_the_epoch() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser
        .fetch(&html_url(
            "<!doctype html><body style='margin:0'><div style='width:100px;height:80px'></div>",
        ))
        .await?;
    let mut frames = page
        .subscribe_viewport_frames(
            ViewportSurface {
                inner_width: 320,
                inner_height: 180,
                ..ViewportSurface::default()
            },
            1,
        )
        .await?;
    let first = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;
    let press = RendererViewportPointerInput {
        document: first.document,
        viewport_revision: first.viewport_revision,
        position: (16., 16.),
        event: "mousedown",
        button: 0,
        buttons: 1,
        click_count: 1,
        modifiers: 0,
    };
    assert!(page.dispatch_viewport_pointer(press).await?.is_some());

    page.evaluate_runtime_expression_async(
        "document.open(); document.write('<!doctype html><body style=margin:0>'); document.close();",
    )
    .await?;
    let loaded = page
        .evaluate_runtime_expression_with_await_async(
            r#"
            new Promise(resolve => {
                window.parentMouseUps = 0;
                addEventListener('mouseup', () => parentMouseUps++);
                const child = document.createElement('iframe');
                child.id = 'child';
                child.style.cssText = 'position:absolute;left:0;top:0;width:120px;height:100px;border:0';
                child.srcdoc = `<body style='margin:0'>
                    <div style='width:100px;height:80px'></div>
                    <script>
                        window.inputEvents = [];
                        addEventListener('wheel', event => {
                            inputEvents.push('wheel:' + event.buttons);
                            event.preventDefault();
                        }, {passive: false});
                        for (const type of ['mousedown', 'mousemove', 'mouseup']) {
                            addEventListener(type, () => inputEvents.push(type));
                        }
                    </script>`;
                child.addEventListener('load', () => resolve(true), {once: true});
                document.body.append(child);
            })
            "#,
            true,
        )
        .await?;
    assert_eq!(loaded["value"], true);

    let replacement = frame(&mut frames, |frame| {
        frame.document != first.document && frame.snapshot.is_some()
    })
    .await?;
    assert_eq!(replacement.document.frame, first.document.frame);
    assert_eq!(replacement.document.document, first.document.document);
    assert_ne!(replacement.document.epoch, first.document.epoch);
    assert_eq!(replacement.viewport_revision, first.viewport_revision);
    page.cancel_viewport_pointer(first.document, first.viewport_revision)
        .await?;
    page.enqueue_viewport_wheel(RendererViewportWheelInput {
        document: replacement.document,
        viewport_revision: replacement.viewport_revision,
        position: press.position,
        delta: (0., 12.),
        buttons: 1,
        modifiers: 0,
        phase: RendererViewportWheelPhase::Moved,
        precise: true,
    })?;

    let press = RendererViewportPointerInput {
        document: replacement.document,
        ..press
    };
    assert!(page.dispatch_viewport_pointer(press).await?.is_some());
    let outside = (220., 41.);
    assert!(
        page.dispatch_viewport_pointer(RendererViewportPointerInput {
            position: outside,
            event: "mousemove",
            button: -1,
            ..press
        })
        .await?
        .is_some()
    );
    assert!(
        page.dispatch_viewport_pointer(RendererViewportPointerInput {
            position: outside,
            event: "mouseup",
            buttons: 0,
            ..press
        })
        .await?
        .is_some()
    );
    let events = page
        .evaluate_runtime_expression_async(
            "child.contentWindow.inputEvents.join(',') + '|' + parentMouseUps",
        )
        .await?;
    assert_eq!(events["value"], "wheel:0,mousedown,mousemove,mouseup|0");
    Ok(())
}

#[tokio::test]
async fn viewport_resize_cancels_a_pressed_pointer_and_releases_capture() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let html = r#"<!doctype html><style>body { margin: 0; } #target { width: 120px; height: 100px; }</style>
        <div id="target"></div>
        <script>
            const target = document.getElementById('target');
            window.events = [];
            target.addEventListener('pointerdown', event => {
                window.pressedId = event.pointerId;
                target.setPointerCapture(pressedId);
            });
            for (const type of ['gotpointercapture', 'pointercancel', 'lostpointercapture', 'pointerup', 'click']) {
                target.addEventListener(type, () => events.push(type));
            }
            let dragging = false;
            target.addEventListener('mousedown', () => {
                dragging = true;
                events.push('mousedown');
            });
            addEventListener('mouseup', event => {
                dragging = false;
                events.push(event.target === window ? 'window-mouseup' : 'element-mouseup');
            });
            window.statusText = () => events.join(',') + ':' + target.hasPointerCapture(pressedId) + ':' + dragging;
        </script>"#;
    let mut page = browser.fetch(&html_url(html)).await?;
    let surface = ViewportSurface::default();
    let mut frames = page.subscribe_viewport_frames(surface, 1).await?;
    let document = frame(&mut frames, |frame| frame.snapshot.is_some())
        .await?
        .document;
    let mut press = RendererViewportPointerInput {
        document,
        viewport_revision: 1,
        position: (16., 16.),
        event: "mousedown",
        button: 0,
        buttons: 1,
        click_count: 1,
        modifiers: 0,
    };
    let capture_move = RendererViewportPointerInput {
        event: "mousemove",
        button: -1,
        ..press
    };
    assert!(page.dispatch_viewport_pointer(press).await?.is_some());
    assert!(
        page.dispatch_viewport_pointer(capture_move)
            .await?
            .is_some()
    );
    page.cancel_viewport_pointer(document, 0).await?;
    assert_eq!(
        page.evaluate_runtime_expression_async("statusText()")
            .await?["value"],
        "mousedown,gotpointercapture:true:true"
    );

    // The UI cancels before sending the new viewport, so this also works if
    // allocating the replacement frame buffers fails.
    page.cancel_viewport_pointer(document, 1).await?;
    let canceled =
        "mousedown,gotpointercapture,pointercancel,window-mouseup,lostpointercapture:false:false";
    assert_eq!(
        page.evaluate_runtime_expression_async("statusText()")
            .await?["value"],
        canceled
    );
    let release = RendererViewportPointerInput {
        event: "mouseup",
        buttons: 0,
        ..press
    };
    assert!(page.dispatch_viewport_pointer(release).await?.is_none());
    assert_eq!(
        page.evaluate_runtime_expression_async("statusText()")
            .await?["value"],
        canceled
    );

    // A renderer rebind also ends a press when the caller replaces its
    // frame subscription directly.
    page.evaluate_runtime_expression_async("events.length = 0")
        .await?;
    assert!(page.dispatch_viewport_pointer(press).await?.is_some());
    let mut resized = page
        .subscribe_viewport_frames(
            ViewportSurface {
                inner_width: surface.inner_width + 10,
                ..surface
            },
            2,
        )
        .await?;
    assert!(page.dispatch_viewport_pointer(release).await?.is_none());
    assert_eq!(
        page.evaluate_runtime_expression_async("statusText()")
            .await?["value"],
        canceled
    );

    frame(&mut resized, |frame| frame.snapshot.is_some()).await?;
    page.evaluate_runtime_expression_async("events.length = 0")
        .await?;
    press.viewport_revision = 2;
    assert!(page.dispatch_viewport_pointer(press).await?.is_some());
    assert!(
        page.dispatch_viewport_pointer(RendererViewportPointerInput {
            viewport_revision: 2,
            ..release
        })
        .await?
        .is_some()
    );
    assert_eq!(
        page.evaluate_runtime_expression_async("statusText()")
            .await?["value"],
        "mousedown,gotpointercapture,pointerup,element-mouseup,lostpointercapture,click:false:false"
    );
    Ok(())
}

#[tokio::test]
async fn viewport_replacement_releases_each_pressed_mouse_button() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser
        .fetch(&html_url(
            r#"<!doctype html><style>body { margin: 0; } div { width: 100px; height: 100px; }</style>
            <div></div><script>
                const pressed = new Set();
                const canceled = [];
                addEventListener('mousedown', event => pressed.add(event.button));
                addEventListener('mouseup', event => {
                    pressed.delete(event.button);
                    if (event.target === window) canceled.push(event.button + ':' + event.buttons);
                });
                globalThis.statusText = () => [...pressed].join(',') + '|' + canceled.join(',');
            </script>"#,
        ))
        .await?;
    let surface = ViewportSurface::default();
    let mut frames = page.subscribe_viewport_frames(surface, 1).await?;
    let document = frame(&mut frames, |frame| frame.snapshot.is_some())
        .await?
        .document;
    let left = RendererViewportPointerInput {
        document,
        viewport_revision: 1,
        position: (16., 16.),
        event: "mousedown",
        button: 0,
        buttons: 1,
        click_count: 1,
        modifiers: 0,
    };
    let right = RendererViewportPointerInput {
        button: 2,
        buttons: 3,
        ..left
    };
    assert!(page.dispatch_viewport_pointer(left).await?.is_some());
    assert!(page.dispatch_viewport_pointer(right).await?.is_some());
    assert_eq!(
        page.evaluate_runtime_expression_async("statusText()")
            .await?["value"],
        "0,2|"
    );

    let mut resized = page
        .subscribe_viewport_frames(
            ViewportSurface {
                inner_width: surface.inner_width + 10,
                ..surface
            },
            2,
        )
        .await?;
    assert_eq!(
        page.evaluate_runtime_expression_async("statusText()")
            .await?["value"],
        "|0:2,2:0"
    );

    frame(&mut resized, |frame| frame.snapshot.is_some()).await?;
    page.evaluate_runtime_expression_async("canceled.length = 0")
        .await?;
    let left = RendererViewportPointerInput {
        viewport_revision: 2,
        ..left
    };
    let right = RendererViewportPointerInput {
        viewport_revision: 2,
        ..right
    };
    assert!(page.dispatch_viewport_pointer(left).await?.is_some());
    assert!(page.dispatch_viewport_pointer(right).await?.is_some());
    assert!(
        page.dispatch_viewport_pointer(RendererViewportPointerInput {
            event: "mouseup",
            buttons: 1,
            ..right
        })
        .await?
        .is_some()
    );
    page.cancel_viewport_pointer(document, 2).await?;
    assert_eq!(
        page.evaluate_runtime_expression_async("statusText()")
            .await?["value"],
        "|0:0"
    );
    Ok(())
}

#[tokio::test]
async fn viewport_cancel_uses_the_press_window_and_respects_mouse_suppression() -> Result<()> {
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let html = r#"<!doctype html>
        <div id="top-target" style="position:absolute;left:0;top:0;width:25px;height:100px"></div>
        <iframe id="child" style="position:absolute;left:40px;top:25px;width:120px;height:100px;border:0"
            srcdoc="<body style='margin:0'>
                <div id='target' style='width:100px;height:80px'></div>
                <script>
                    window.inputEvents = [];
                    window.suppressMouse = true;
                    const target = document.getElementById('target');
                    target.addEventListener('pointerdown', event => {
                        inputEvents.push('pointerdown');
                        if (suppressMouse) event.preventDefault();
                    });
                    for (const type of ['pointercancel', 'mousedown', 'click']) {
                        target.addEventListener(type, () => inputEvents.push(type));
                    }
                    addEventListener('mouseup', event => {
                        const atWindow = event.target === window && event.view === window;
                        inputEvents.push(['mouseup', atWindow, event.isTrusted, event.clientX,
                            event.clientY, event.button, event.buttons].join('/'));
                    });
                </script>"></iframe>
        <script>
            window.topMouseUps = 0;
            document.getElementById('top-target').addEventListener('mousedown', event => {
                event.target.remove();
            });
            addEventListener('mouseup', () => topMouseUps++);
        </script>"#;
    let mut page = browser.fetch(&html_url(html)).await?;
    let mut frames = page
        .subscribe_viewport_frames(ViewportSurface::default(), 1)
        .await?;
    let document = frame(&mut frames, |frame| frame.snapshot.is_some())
        .await?
        .document;
    let press = RendererViewportPointerInput {
        document,
        viewport_revision: 1,
        position: (56., 41.),
        event: "mousedown",
        button: 0,
        buttons: 1,
        click_count: 1,
        modifiers: 0,
    };
    let release = RendererViewportPointerInput {
        event: "mouseup",
        buttons: 0,
        ..press
    };
    const LOG: &str = "`${child.contentWindow.inputEvents.join(',')}:${topMouseUps}`";

    assert!(page.dispatch_viewport_pointer(press).await?.is_some());
    page.cancel_viewport_pointer(document, 1).await?;
    assert!(page.dispatch_viewport_pointer(release).await?.is_none());
    assert_eq!(
        page.evaluate_runtime_expression_async(LOG).await?["value"],
        "pointerdown,pointercancel:0"
    );

    page.evaluate_runtime_expression_async(
        "child.contentWindow.suppressMouse = false; child.contentWindow.inputEvents.length = 0",
    )
    .await?;
    assert!(page.dispatch_viewport_pointer(press).await?.is_some());
    page.cancel_viewport_pointer(document, 1).await?;
    assert!(page.dispatch_viewport_pointer(release).await?.is_none());
    assert_eq!(
        page.evaluate_runtime_expression_async(LOG).await?["value"],
        "pointerdown,mousedown,pointercancel,mouseup/true/true/16/16/0/0:0"
    );

    // An uncaptured pointer can cross into the iframe after the element
    // pressed in the parent was removed. The parent Window owns the mouseup.
    page.evaluate_runtime_expression_async("child.contentWindow.inputEvents.length = 0")
        .await?;
    let parent_press = RendererViewportPointerInput {
        position: (10., 41.),
        ..press
    };
    let move_into_child = RendererViewportPointerInput {
        event: "mousemove",
        button: -1,
        ..press
    };
    assert!(
        page.dispatch_viewport_pointer(parent_press)
            .await?
            .is_some()
    );
    assert!(
        page.dispatch_viewport_pointer(move_into_child)
            .await?
            .is_some()
    );
    page.cancel_viewport_pointer(document, 1).await?;
    assert_eq!(
        page.evaluate_runtime_expression_async(LOG).await?["value"],
        "pointercancel:1"
    );
    Ok(())
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
    let mut states = page.subscribe_navigation_state().await?;
    let mut frames = page.subscribe_viewport_frames(surface, 7).await?;
    let first = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;
    let initial = page_state(&mut states, |state| state.document == first.document).await?;
    assert_eq!(initial.navigation.title, "First");
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
    let state = page_state(&mut states, |state| state.navigation.title == "Second").await?;
    assert_ne!(state.document, first.document);
    assert_eq!(state.navigation.url.as_str(), next_url);
    assert!(state.navigation.can_go_back);
    let replacement = frame(&mut frames, |frame| frame.document == state.document).await?;
    assert_eq!(replacement.viewport_revision, 7);
    assert!(page.traverse_viewport_history(-1).await?);
    let back = page_state(&mut states, |state| state.navigation.title == "First").await?;
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
async fn viewport_committed_document_uses_its_own_cookie_site() -> Result<()> {
    let server = moli_test_support::FixtureServer::spawn().await?;
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser.create_viewport_page(Default::default()).await?;
    let mut states = page.subscribe_navigation_state().await?;
    let pending = page.start_viewport_navigation(Some(server.url("/cookie-scope/check")))?;
    assert!(page.finish_viewport_navigation(pending.wait().await?)?);
    let first = page_state(&mut states, |state| {
        state.navigation.url.path() == "/cookie-scope/check"
    })
    .await?;
    browser
        .wait_for_selector(&mut page, "main", Duration::from_secs(5))
        .await?;
    let response = page
        .evaluate_runtime_expression_with_await_async(
            r#"
            new Promise((resolve, reject) => {
                const xhr = new XMLHttpRequest();
                xhr.open("GET", "/cookie-scope/set");
                xhr.onload = () => resolve(xhr.status);
                xhr.onerror = () => reject(new Error("cookie request failed"));
                xhr.send();
            })
            "#,
            true,
        )
        .await?;
    assert_eq!(response["value"], 200);
    page.evaluate_runtime_expression_async(
        "history.replaceState(null, '', '?return=1#status'); location.reload();",
    )
    .await?;
    let second = page_state(&mut states, |state| state.document != first.document).await?;
    assert_eq!(second.navigation.url.query(), Some("return=1"));
    assert_eq!(second.navigation.url.fragment(), Some("status"));
    browser
        .wait_for_selector(&mut page, "main", Duration::from_secs(5))
        .await?;
    let result = page
        .evaluate_runtime_expression_async("document.querySelector('main').textContent")
        .await?;
    assert_eq!(result["value"], "scoped-cookie=seen");
    Ok(())
}

#[tokio::test]
async fn viewport_history_survives_link_and_form_navigation() -> Result<()> {
    let server = moli_test_support::FixtureServer::spawn().await?;
    let destination = server.url("/net/echo");
    let redirect = server.url("/redirect");
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let style = "display:block;width:180px;height:60px";
    for (label, control, destination_path) in [
        (
            "link",
            format!(r#"<a href="{destination}" style="{style}">Next</a>"#),
            "/net/echo",
        ),
        (
            "redirected link",
            format!(r#"<a href="{redirect}" style="{style}">Next</a>"#),
            "/static",
        ),
        (
            "GET form",
            format!(r#"<form action="{destination}"><button style="{style}">Next</button></form>"#),
            "/net/echo",
        ),
        (
            "POST form",
            format!(
                r#"<form action="{destination}" method="post"><button style="{style}">Next</button></form>"#
            ),
            "/net/echo",
        ),
    ] {
        let original = html_url(&format!("<title>First</title>{control}"));
        let mut page = browser.fetch(&original).await?;
        let mut states = page.subscribe_navigation_state().await?;
        let mut frames = page
            .subscribe_viewport_frames(ViewportSurface::default(), 1)
            .await?;
        let initial = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;
        let first = page_state(&mut states, |state| state.document == initial.document).await?;
        assert!(!first.navigation.can_go_back, "{label}");

        page.dispatch_mouse_event_at_point_async(16., 16., "mousedown", 0, Some(1), 0., 0.)
            .await?;
        page.dispatch_mouse_event_at_point_async(16., 16., "mouseup", 0, Some(0), 0., 0.)
            .await?;
        let next = page_state(&mut states, |state| state.document != first.document).await?;
        assert_eq!(next.navigation.url.path(), destination_path, "{label}");
        assert!(next.navigation.can_go_back, "{label}");
        assert!(!next.navigation.can_go_forward, "{label}");
        let current_entry = page
            .evaluate_runtime_expression_async("navigation.currentEntry.url")
            .await?;
        assert_eq!(
            current_entry["value"],
            next.navigation.url.as_str(),
            "{label}"
        );

        assert!(page.traverse_viewport_history(-1).await?, "{label}");
        let back = page_state(&mut states, |state| {
            state.navigation.url == first.navigation.url
        })
        .await?;
        assert!(!back.navigation.can_go_back, "{label}");
        assert!(back.navigation.can_go_forward, "{label}");

        assert!(page.traverse_viewport_history(1).await?, "{label}");
        let forward = page_state(&mut states, |state| {
            state.navigation.url == next.navigation.url
        })
        .await?;
        assert!(forward.navigation.can_go_back, "{label}");
        assert!(!forward.navigation.can_go_forward, "{label}");
    }
    Ok(())
}

#[tokio::test]
async fn viewport_history_uses_the_link_source_origin_for_same_url_navigation() -> Result<()> {
    let origin = moli_test_support::FixtureServer::spawn().await?;
    let other_origin = moli_test_support::FixtureServer::spawn().await?;
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let previous = origin.url("/static?previous");
    let current = origin.url("/static?current");
    let style = "display:block;width:180px;height:60px";

    for (case, target, frame_origin, adds_entry) in [
        ("default target", "", None, false),
        ("explicit self", "_self", None, false),
        ("explicit top", "_top", None, false),
        ("explicit parent", "_parent", None, false),
        ("same-origin child", "_top", Some(&origin), false),
        ("cross-origin child", "_top", Some(&other_origin), true),
    ] {
        let mut page = browser.fetch(&previous).await?;
        page.evaluate_runtime_expression_async(&format!("location.assign({current:?})"))
            .await?;
        let mut states = page.subscribe_navigation_state().await?;
        let mut frames = page
            .subscribe_viewport_frames(ViewportSurface::default(), 1)
            .await?;
        let first = page_state(&mut states, |state| {
            state.navigation.url.as_str() == current
        })
        .await
        .with_context(|| format!("{case} initial state"))?;
        frame(&mut frames, |frame| {
            frame.document == first.document && frame.snapshot.is_some()
        })
        .await
        .with_context(|| format!("{case} initial frame"))?;
        assert!(first.navigation.can_go_back, "{case}");

        let anchor = format!(r#"<a href="{current}" target="{target}" style="{style}">Open</a>"#);
        if let Some(frame_origin) = frame_origin {
            let mut child =
                url::Url::parse(&frame_origin.url("/compat/child-dynamic-markup-document"))?;
            child.query_pairs_mut().append_pair(
                "markup",
                &format!(r#"<!doctype html><body style="margin:0">{anchor}"#),
            );
            let loaded = page
                .evaluate_runtime_expression_with_await_async(
                    &format!(
                        r#"
                        new Promise(resolve => {{
                            const child = document.createElement('iframe');
                            child.style.cssText = 'display:block;width:200px;height:80px;border:0';
                            child.onload = () => resolve(true);
                            child.src = {:?};
                            document.body.style.margin = '0';
                            document.body.replaceChildren(child);
                        }})
                        "#,
                        child.as_str(),
                    ),
                    true,
                )
                .await?;
            assert_eq!(loaded["value"], true, "{case} child load");
        } else {
            page.evaluate_runtime_expression_async(&format!(
                "document.body.style.margin = '0'; document.body.innerHTML = {anchor:?}"
            ))
            .await?;
        }
        let before = page
            .evaluate_runtime_expression_by_value_async(
                r#"
                sessionStorage.removeItem('link-navigation-type');
                navigation.addEventListener('navigate', event => {
                    sessionStorage.setItem('link-navigation-type', event.navigationType);
                });
                [history.length, navigation.currentEntry.key]
                "#,
            )
            .await?;
        let history_length = before["value"][0].as_u64().context("history.length")?;

        page.dispatch_mouse_event_at_point_async(16., 16., "mousedown", 0, Some(1), 0., 0.)
            .await?;
        page.dispatch_mouse_event_at_point_async(16., 16., "mouseup", 0, Some(0), 0., 0.)
            .await?;
        let next = page_state(&mut states, |state| state.document != first.document)
            .await
            .with_context(|| format!("{case} committed state"))?;
        assert_eq!(next.navigation.url.as_str(), current, "{case}");
        assert!(next.navigation.can_go_back, "{case}");
        let after = page
            .evaluate_runtime_expression_by_value_async(
                r#"
                [history.length, navigation.currentEntry.key, navigation.activation.navigationType,
                    sessionStorage.getItem('link-navigation-type')]
                "#,
            )
            .await?;
        let kind = if adds_entry { "push" } else { "replace" };
        assert_eq!(
            after["value"][0],
            history_length + u64::from(adds_entry),
            "{case}"
        );
        assert_eq!(
            after["value"][1] != before["value"][1],
            adds_entry,
            "{case}"
        );
        assert_eq!(after["value"][2], kind, "{case}");
        if !adds_entry || !after["value"][3].is_null() {
            assert_eq!(after["value"][3], kind, "{case} navigate event");
        }

        assert!(page.traverse_viewport_history(-1).await?, "{case}");
        let back = page_state(&mut states, |state| state.document != next.document)
            .await
            .with_context(|| format!("{case} back state"))?;
        assert_eq!(
            back.navigation.url.as_str(),
            if adds_entry { &current } else { &previous },
            "{case}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn viewport_history_keeps_the_page_before_scripted_form_submission() -> Result<()> {
    let server = moli_test_support::FixtureServer::spawn().await?;
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;

    for (method, source, destination) in [
        ("GET", "/static", "/net/echo?q=hello"),
        ("POST", "/static", "/net/echo"),
        ("GET", "/net/echo?q=hello", "/net/echo?q=hello"),
        ("POST", "/net/echo", "/net/echo"),
    ] {
        for submit in ["submit", "requestSubmit"] {
            let case = format!("{method} {submit} from {source}");
            let mut page = browser.fetch(&server.url(source)).await?;
            let mut states = page.subscribe_navigation_state().await?;
            let _frames = page
                .subscribe_viewport_frames(ViewportSurface::default(), 1)
                .await?;
            let first = page_state(&mut states, |_| true)
                .await
                .with_context(|| format!("{case} initial state"))?;
            assert!(!first.navigation.can_go_back, "{case}");

            let setup = format!(
                r#"
                document.body.innerHTML =
                    '<form action="/net/echo" method="{method}"><input name="q" value="hello"></form>';
                sessionStorage.removeItem('form-navigation');
                navigation.addEventListener('navigate', event => {{
                    sessionStorage.setItem('form-navigation',
                        event.navigationType + ':' + event.userInitiated);
                }});
                [document.readyState, history.length]
                "#
            );
            let before = page
                .evaluate_runtime_expression_by_value_async(&setup)
                .await?;
            assert_eq!(before["value"][0], "complete", "{case}");
            let history_length = before["value"][1].as_u64().context("history.length")?;
            page.evaluate_runtime_expression_async(&format!(
                "document.querySelector('form').{submit}()"
            ))
            .await?;
            let next = page_state(&mut states, |state| state.document != first.document)
                .await
                .with_context(|| format!("{case} submitted state"))?;
            assert_eq!(
                next.navigation.url.as_str(),
                server.url(destination),
                "{case}"
            );
            let after = page
                .evaluate_runtime_expression_by_value_async(
                    r#"[
                        sessionStorage.getItem('form-navigation') + ':' + navigation.activation.navigationType,
                        history.length,
                        navigation.canGoBack
                    ]"#,
                )
                .await?;
            assert_eq!(
                after["value"],
                serde_json::json!(["push:false:push", history_length + 1, true]),
                "{case}"
            );
            assert!(next.navigation.can_go_back, "{case}");
            assert!(page.traverse_viewport_history(-1).await?, "{case}");
            let back = page_state(&mut states, |state| {
                state.document != next.document && state.navigation.url == first.navigation.url
            })
            .await
            .with_context(|| format!("{case} back state"))?;
            assert!(back.navigation.can_go_forward, "{case}");
        }
    }
    Ok(())
}

#[tokio::test]
async fn viewport_form_submission_during_load_replaces_the_current_entry() -> Result<()> {
    let server = moli_test_support::FixtureServer::spawn().await?;
    let mut source = url::Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
    source.query_pairs_mut().append_pair(
        "markup",
        r#"<!doctype html><form action="/net/echo" method="post"></form>
        <script>
            addEventListener('load', () => {
                navigation.addEventListener('navigate', event => {
                    sessionStorage.setItem('form-navigation',
                        document.readyState + ':' + event.navigationType);
                });
                document.querySelector('form').requestSubmit();
            });
        </script>"#,
    );
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser.fetch(source.as_str()).await?;
    let mut states = page.subscribe_navigation_state().await?;
    let _frames = page
        .subscribe_viewport_frames(ViewportSurface::default(), 1)
        .await?;
    let next = page_state(&mut states, |state| {
        state.navigation.url.path() == "/net/echo"
    })
    .await?;
    let types = page
        .evaluate_runtime_expression_async(
            "sessionStorage.getItem('form-navigation') + ':' + navigation.activation.navigationType",
        )
        .await?;
    assert_eq!(types["value"], "complete:replace:replace");
    assert!(!next.navigation.can_go_back);
    Ok(())
}

#[tokio::test]
async fn viewport_history_honors_the_source_document_referrer_policy() -> Result<()> {
    let server = moli_test_support::FixtureServer::spawn().await?;
    let mut source = url::Url::parse(&server.url("/compat/child-dynamic-markup-document"))?;
    source
        .query_pairs_mut()
        .append_pair("private", "source-value")
        .append_pair(
            "markup",
            r#"<!doctype html><meta name="referrer" content="no-referrer">
        <a href="/static" style="display:block;width:180px;height:60px">Next</a>"#,
        );
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser.fetch(source.as_str()).await?;
    let mut states = page.subscribe_navigation_state().await?;
    let mut frames = page
        .subscribe_viewport_frames(ViewportSurface::default(), 1)
        .await?;
    let first = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;

    page.dispatch_mouse_event_at_point_async(16., 16., "mousedown", 0, Some(1), 0., 0.)
        .await?;
    page.dispatch_mouse_event_at_point_async(16., 16., "mouseup", 0, Some(0), 0., 0.)
        .await?;
    let next = page_state(&mut states, |state| state.document != first.document).await?;
    assert_eq!(next.navigation.url.path(), "/static");
    assert!(next.navigation.can_go_back);
    let urls = page
        .evaluate_runtime_expression_async(
            r#"
            (() => {
                const from = navigation.activation.from;
                const previous = navigation.entries().find(entry => entry.key === from.key);
                return JSON.stringify([from.url, previous.url]);
            })()
            "#,
        )
        .await?;
    assert_eq!(urls["value"], "[null,null]");
    Ok(())
}

#[tokio::test]
async fn viewport_navigation_activation_uses_the_final_redirect_origin() -> Result<()> {
    let origin = moli_test_support::FixtureServer::spawn().await?;
    let other_origin = moli_test_support::FixtureServer::spawn().await?;
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;

    for (case, redirect_at, destination_at, replace, no_referrer) in [
        ("return to origin", &other_origin, &origin, false, false),
        ("replace at origin", &other_origin, &origin, true, false),
        (
            "return with no-referrer",
            &other_origin,
            &origin,
            false,
            true,
        ),
        ("leave origin", &origin, &other_origin, false, false),
    ] {
        let destination = destination_at.url("/static");
        let mut redirect = url::Url::parse(&redirect_at.url("/redirect"))?;
        redirect.query_pairs_mut().append_pair("to", &destination);
        let mut source = url::Url::parse(&origin.url("/compat/child-dynamic-markup-document"))?;
        let policy = if no_referrer {
            r#"<meta name="referrer" content="no-referrer">"#
        } else {
            ""
        };
        source.query_pairs_mut().append_pair(
            "markup",
            &format!(r#"<!doctype html>{policy}<a href="{redirect}">Next</a>"#),
        );

        let mut page = browser.fetch(source.as_str()).await?;
        let mut states = page.subscribe_navigation_state().await?;
        let _frames = page
            .subscribe_viewport_frames(ViewportSurface::default(), 1)
            .await?;
        let first = page_state(&mut states, |_| true).await?;
        let previous = page
            .evaluate_runtime_expression_async("navigation.currentEntry.key")
            .await?;
        let previous_key = previous["value"].as_str().context("source history key")?;

        let navigate = if replace {
            format!("location.replace({:?})", redirect.as_str())
        } else {
            "document.querySelector('a').click()".to_owned()
        };
        page.evaluate_runtime_expression_async(&navigate).await?;
        let next = page_state(&mut states, |state| state.document != first.document).await?;
        assert_eq!(next.navigation.url.as_str(), destination, "{case}");
        assert_eq!(next.navigation.can_go_back, !replace, "{case}");

        let activation = page
            .evaluate_runtime_expression_by_value_async(
                r#"[
                    navigation.activation.navigationType,
                    navigation.activation.entry.url,
                    navigation.activation.from?.key ?? null,
                    navigation.activation.from?.url ?? null
                ]"#,
            )
            .await?;
        let same_origin = next.navigation.url.origin() == source.origin();
        assert_eq!(
            activation["value"],
            serde_json::json!([
                if replace { "replace" } else { "push" },
                destination,
                same_origin.then_some(previous_key),
                (same_origin && !no_referrer).then_some(source.as_str()),
            ]),
            "{case}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn navigation_state_updates_without_painting_a_static_document() -> Result<()> {
    let server = moli_test_support::FixtureServer::spawn().await?;
    let browser =
        Browser::new(BrowserConfig::default().with_layout_policy(LayoutPolicy::Realtime))?;
    let mut page = browser.fetch(&server.url("/static")).await?;
    let mut states = page.subscribe_navigation_state().await?;
    let mut frames = page
        .subscribe_viewport_frames(ViewportSurface::default(), 1)
        .await?;
    let initial_frame = frame(&mut frames, |frame| frame.snapshot.is_some()).await?;
    let first = page_state(&mut states, |state| {
        state.document == initial_frame.document
    })
    .await?;
    assert!(!first.navigation.can_go_back);

    page.evaluate_runtime_expression_async("history.pushState(null, '', '/pushed')")
        .await?;
    let pushed = page_state(&mut states, |state| {
        state.navigation.url.path() == "/pushed"
    })
    .await?;
    assert_eq!(pushed.document, first.document);
    assert!(pushed.navigation.can_go_back);
    assert!(!pushed.navigation.can_go_forward);
    assert!(!frames.updates.has_changed()?);

    assert!(page.traverse_viewport_history(-1).await?);
    let back = page_state(&mut states, |state| state.navigation.can_go_forward).await?;
    assert_eq!(back.document, first.document);
    assert_eq!(back.navigation.url, first.navigation.url);
    assert!(!back.navigation.can_go_back);
    assert!(!frames.updates.has_changed()?);

    page.evaluate_runtime_expression_async("document.body.style.backgroundColor = 'red'")
        .await?;
    let repainted = frame(&mut frames, |frame| frame.sequence > initial_frame.sequence).await?;
    assert_eq!(repainted.document, first.document);
    assert_ne!(repainted.snapshot, initial_frame.snapshot);
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
    use moli_core::page::RendererDraggedFile;
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
    let mut states = page.subscribe_navigation_state().await?;
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
    page_state(&mut states, |state| state.navigation.title == file.name).await?;
    let press = RendererViewportPointerInput {
        document: first.document,
        viewport_revision: first.viewport_revision,
        position: (16., 16.),
        event: "mousedown",
        button: 0,
        buttons: 1,
        click_count: 1,
        modifiers: 0,
    };
    assert!(page.dispatch_viewport_pointer(press).await?.is_some());
    let release = RendererViewportPointerInput {
        event: "mouseup",
        buttons: 0,
        ..press
    };
    assert!(page.dispatch_viewport_pointer(release).await?.is_some());
    let navigated = page_state(&mut states, |state| state.document != first.document).await?;
    assert_eq!(navigated.navigation.url.as_str(), next);
    assert!(
        !page
            .drop_viewport_files(first.document, (16., 16.), vec![file], 0)
            .await?
    );
    Ok(())
}
