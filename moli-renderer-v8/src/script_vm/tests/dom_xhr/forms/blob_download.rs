use super::*;

#[test]
fn blob_download_activation_retains_bytes_after_revocation_and_creator_retirement() {
    for tag in ["a", "area"] {
        for child in [false, true] {
            let mut vm = new_parsed_test_vm(
                "https://blob-download.test/",
                "<!doctype html><body></body>",
            );
            let url = vm.eval(&format!(r#"(() => {{
                const frame = {child} ? document.body.appendChild(document.createElement('iframe')) : null;
                const w = frame ? frame.contentWindow : window;
                const url = w.URL.createObjectURL(new w.Blob([new w.Uint8Array([0, 255, 128, 97, 13, 10])], {{type:'application/octet-stream'}}));
                const link = w.document.body.appendChild(w.document.createElement({tag:?}));
                link.href = url; link.download = 'bytes.bin'; link.click();
                w.URL.revokeObjectURL(url);
                if (frame) frame.remove();
                return url;
            }})()"#)).unwrap();
            let mut activations = vm.take_pending_download_activations();
            assert_eq!(activations.len(), 1, "{tag} child={child}");
            let activation = activations.pop().unwrap();
            assert_eq!(activation.url, url);
            assert_eq!(activation.suggested_filename.as_deref(), Some("bytes.bin"));
            assert!(crate::blob::object_url_body_and_type(&url).is_none());
            let response = activation
                .response
                .expect("accepted Blob download must own its response");
            assert_eq!(response.final_url, url);
            assert_eq!(response.status, 200);
            assert!(
                response
                    .headers
                    .iter()
                    .any(|(name, value)| name.eq_ignore_ascii_case("Content-Type")
                        && value == b"application/octet-stream")
            );
            let crate::runtime::RendererDownloadBody::Blob(data) = response.body else {
                panic!("accepted Blob must retain its immutable backing");
            };
            assert_eq!(data.read_blocking().unwrap(), [0, 255, 128, 97, 13, 10]);
            assert!(vm.take_pending_location_navigation_with_seed().is_none());
        }
    }
}

#[test]
fn canceled_and_previously_revoked_blob_downloads_do_not_capture_a_response() {
    for cancel in [false, true] {
        let mut vm = new_parsed_test_vm(
            "https://blob-download-cancel.test/",
            "<!doctype html><body></body>",
        );
        vm.eval(&format!(
            r#"(() => {{
            const link = document.body.appendChild(document.createElement('a'));
            const url = URL.createObjectURL(new Blob(['not accepted']));
            link.href = url; link.download = 'bytes.bin';
            if ({cancel}) link.addEventListener('click', e => e.preventDefault());
            URL.revokeObjectURL(url); link.click();
        }})()"#
        ))
        .unwrap();
        let activations = vm.take_pending_download_activations();
        if cancel {
            assert!(activations.is_empty());
        } else {
            assert_eq!(activations.len(), 1);
            assert!(activations[0].response.is_none());
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn blob_download_authorizes_the_initiating_document_storage_key_and_partition() {
    let loader = static_http_loader(std::iter::empty::<String>());
    for foreign_partition in [false, true] {
        for foreign_origin in [false, true] {
            let creator_owner = crate::runtime::RendererBrowserContextRuntime::new();
            let other_owner = crate::runtime::RendererBrowserContextRuntime::new();
            let mut creator =
                crate::runtime::PageVmTaskExecutorTestHarness::new_with_browser_context_runtime(
                    Url::parse("https://blob-owner.test/").unwrap(),
                    &loader,
                    creator_owner.handle(),
                );
            let url = creator
                .eval("URL.createObjectURL(new Blob(['private payload']))")
                .unwrap();
            let target_url = if foreign_origin {
                "https://other-owner.test/"
            } else {
                "https://blob-owner.test/"
            };
            let owner = if foreign_partition {
                other_owner.handle()
            } else {
                creator_owner.handle()
            };
            let mut target =
                crate::runtime::PageVmTaskExecutorTestHarness::new_with_browser_context_runtime(
                    Url::parse(target_url).unwrap(),
                    &loader,
                    owner,
                );
            target
                .eval(&format!(
                    r#"(() => {{
                const a = document.createElement('a');
                a.href = {url:?}; a.download = 'private.txt'; a.click();
            }})()"#
                ))
                .unwrap();
            let mut downloads = target.take_pending_download_activations();
            assert_eq!(downloads.len(), 1);
            let download = downloads.pop().unwrap();
            assert_eq!(
                download.response.is_some(),
                !foreign_partition && !foreign_origin,
                "foreign partition={foreign_partition}, foreign origin={foreign_origin}"
            );
            assert!(
                target
                    .take_pending_location_navigation_with_seed()
                    .is_none()
            );
            creator
                .eval(&format!("URL.revokeObjectURL({url:?})"))
                .unwrap();
        }
    }
}

#[cfg(feature = "devtools")]
#[test]
fn keyboard_blob_download_retains_the_backing_before_revocation() {
    let mut vm = new_parsed_test_vm(
        "https://keyboard-blob.test/",
        "<a id=link download=payload.bin>save</a>",
    );
    vm.eval("globalThis.url = URL.createObjectURL(new Blob([new Uint8Array([0,255,128,65,10])], {type:'application/octet-stream'})); link.href=url; link.focus()").unwrap();
    let download = vm
        .dispatch_key_event("keydown", "Enter", "Enter", "", 0, false, false)
        .unwrap()
        .pending_download
        .unwrap();
    vm.eval("URL.revokeObjectURL(url)").unwrap();
    let response = download.response.unwrap();
    let crate::runtime::RendererDownloadBody::Blob(data) = response.body else {
        panic!("keyboard activation must retain the backing");
    };
    assert_eq!(data.read_blocking().unwrap(), [0, 255, 128, 65, 10]);
    assert!(
        response
            .headers
            .iter()
            .any(|(name, value)| name == "Content-Length" && value == b"5")
    );
}
