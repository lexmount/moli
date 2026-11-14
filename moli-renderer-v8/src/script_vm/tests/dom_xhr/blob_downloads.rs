use super::*;
use crate::runtime::RendererDownloadBody;

#[test]
fn blob_download_activation_captures_binary_bytes_before_url_revocation() {
    for keyboard in [false, true] {
        let mut vm = new_parsed_test_vm(
            "https://blob-download.test/page",
            "<a id=link download=payload.bin>save</a>",
        );
        let url = vm.eval(r#"
            globalThis.url=URL.createObjectURL(new Blob([new Uint8Array([0,255,128,65,10])],{type:'application/octet-stream'}));
            link.href=url;url
        "#).unwrap();
        let download = if keyboard {
            vm.eval("link.focus()").unwrap();
            let outcome = vm
                .dispatch_key_event("keydown", "Enter", "Enter", "", 0, false, false)
                .unwrap();
            vm.eval("URL.revokeObjectURL(url)").unwrap();
            outcome.pending_download.unwrap()
        } else {
            vm.eval("link.click();URL.revokeObjectURL(url)").unwrap();
            let mut downloads = vm.take_pending_download_activations();
            assert_eq!(downloads.len(), 1);
            downloads.remove(0)
        };
        assert_eq!(download.url, url);
        assert_eq!(download.suggested_filename.as_deref(), Some("payload.bin"));
        let response = download
            .response
            .expect("blob download must carry its local body");
        assert_eq!(response.final_url, url);
        assert_eq!(response.status, 200);
        assert_eq!(
            response.body,
            RendererDownloadBody::Buffered(vec![0, 255, 128, 65, 10])
        );
        assert!(response.headers.iter().any(|(name,value)|name=="Content-Type" && value==b"application/octet-stream"));
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name == "Content-Length" && value == b"5")
        );
        assert!(vm.take_pending_location_navigation_with_seed().is_none());
    }
}

#[test]
fn revoked_blob_download_does_not_fall_back_to_a_network_download() {
    let mut vm = new_parsed_test_vm(
        "https://revoked-blob-download.test/",
        "<a id=link download=file.bin>save</a>",
    );
    vm.eval("const url=URL.createObjectURL(new Blob(['gone']));link.href=url;URL.revokeObjectURL(url);link.click()").unwrap();
    assert!(vm.take_pending_download_activations().is_empty());
    assert!(vm.take_pending_location_navigation_with_seed().is_none());
}
