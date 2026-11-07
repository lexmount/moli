use super::*;

#[test]
fn ace_page_task_loading_reports_normal_pending_navigation() {
    let mut page = crate::runtime::page_vm::tests::test_page_vm();
    let host = AcePageTaskHost {
        page: &mut page,
        kind: AcePageTaskKind::ReadOnly,
        input_outcomes: Vec::new(),
    };
    assert!(!host.loading().pending_navigation);
    host.page
        .vm_mut()
        .eval("location.href = '/next-page'")
        .unwrap();
    assert!(host.loading().pending_navigation);
}

#[derive(Debug)]
struct DownloadThenKey {
    fail: bool,
}

impl AcePageTask for DownloadThenKey {
    fn kind(&self) -> AcePageTaskKind {
        AcePageTaskKind::Input
    }

    fn run(self: Box<Self>, host: &mut dyn PageExtensionHost) -> Result<AcePageTaskOutput> {
        for id in ["first", "second"] {
            let handle = host.dom().element_handle_by_id(id).unwrap();
            host.activate(handle, 0.0, 0.0, 0)?;
        }
        host.key("keyup", "Shift", "ShiftLeft", "", 0, false, false)?;
        anyhow::ensure!(!self.fail, "task failed after input");
        Ok(AcePageTaskOutput {
            value: serde_json::json!("done"),
        })
    }
}

#[test]
fn ace_page_task_preserves_all_input_effects_on_success_and_error() {
    for fail in [false, true] {
        let mut page = crate::runtime::page_vm::tests::test_page_vm();
        page.vm_mut().eval(
            r#"document.body.innerHTML = '<a id="first" download="first.txt" href="/first.txt">one</a><a id="second" download="second.txt" href="/second.txt">two</a>'"#,
        ).unwrap();
        let RendererPageReply::AcePageTask(execution) = page
            .execute_ace_page_task(Box::new(DownloadThenKey { fail }))
            .unwrap()
        else {
            panic!("expected task execution");
        };
        assert_eq!(execution.result.is_err(), fail);
        assert_eq!(execution.input_outcomes.len(), 3);
        let downloads = execution
            .input_outcomes
            .iter()
            .filter_map(|outcome| {
                outcome
                    .pending_download
                    .as_ref()
                    .and_then(|download| download.suggested_filename.as_deref())
            })
            .collect::<Vec<_>>();
        assert_eq!(downloads, ["first.txt", "second.txt"]);
        assert!(
            execution
                .input_outcomes
                .last()
                .unwrap()
                .pending_download
                .is_none()
        );
    }
}

#[derive(Debug)]
struct ReadOnlyClick;

impl AcePageTask for ReadOnlyClick {
    fn kind(&self) -> AcePageTaskKind {
        AcePageTaskKind::ReadOnly
    }

    fn run(self: Box<Self>, host: &mut dyn PageExtensionHost) -> Result<AcePageTaskOutput> {
        let handle = host.dom().element_handle_by_id("button").unwrap();
        host.activate(handle, 0.0, 0.0, 0)?;
        unreachable!("read-only task must not dispatch a click")
    }
}

#[test]
fn ace_page_task_read_only_rejects_input_before_page_code_runs() {
    let mut page = crate::runtime::page_vm::tests::test_page_vm();
    page.vm_mut().eval(
        r#"globalThis.clicks = 0; document.body.innerHTML = '<button id="button" onclick="clicks++">click</button>'"#,
    ).unwrap();
    let RendererPageReply::AcePageTask(execution) =
        page.execute_ace_page_task(Box::new(ReadOnlyClick)).unwrap()
    else {
        panic!("expected task execution");
    };
    assert!(execution.result.unwrap_err().contains("Read-only ACE task"));
    assert!(execution.input_outcomes.is_empty());
    assert_eq!(page.vm_mut().eval("clicks").unwrap(), "0");
}

#[test]
fn ace_page_task_screenshot_captures_visible_text_and_rejects_hidden_node() {
    let mut page = crate::runtime::page_vm::tests::test_page_vm();
    page.vm_mut().eval(
        r#"document.body.innerHTML = '<p id="visible">Hello screenshot</p><p id="hidden" style="display:none">hidden</p>'"#,
    ).unwrap();
    let mut host = AcePageTaskHost {
        page: &mut page,
        kind: AcePageTaskKind::ReadOnly,
        input_outcomes: Vec::new(),
    };
    let visible = host.dom().element_handle_by_id("visible").unwrap();
    let hidden = host.dom().element_handle_by_id("hidden").unwrap();
    let text = host
        .dom()
        .node(visible)
        .unwrap()
        .child_ids(host.dom().dom())
        .next()
        .unwrap();
    let clip = host.screenshot_clip(text).unwrap();
    assert!(clip.width > 0.0 && clip.height > 0.0);
    assert!(host.screenshot_clip(hidden).is_err());
    host.flush_screenshot().unwrap();
    let mut request = RendererCaptureScreenshotRequest::viewport_png();
    request.region = RendererScreenshotRegion::PageClip(clip);
    let RendererCaptureScreenshotReply::Captured(image) = host.capture(request).unwrap() else {
        panic!("expected screenshot");
    };
    assert!(image.width > 0 && image.height > 0);
    assert_eq!(&image.bytes[..8], b"\x89PNG\r\n\x1a\n");
}
