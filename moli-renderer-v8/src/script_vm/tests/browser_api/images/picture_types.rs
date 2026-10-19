use super::*;

const BASE: &str = "https://picture-source-types.test/";

fn picture_type_vm(
    windowless: bool,
    hint: Option<&str>,
) -> (
    crate::runtime::PageVmTaskExecutorTestHarness,
    crate::network::ResourceRequestClientOwner,
) {
    let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
    loader.set_image_fetch_enabled(true);
    let mut vm = new_storage_page_task_executor_test_vm_with_loader(BASE, &loader);
    vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
    vm.set_fetch_subresource_interception(true, Some(crate::types::SubresourceResourceType::Image));
    let hint = serde_json::to_string(&hint).unwrap();
    vm.eval(&format!(r#"
        var imageDocument = {windowless} ? document.implementation.createHTMLDocument('') : document;
        var picture = imageDocument.createElement('picture');
        var source = imageDocument.createElement('source');
        source.srcset = '{BASE}hint.gif';
        if ({hint} !== null) source.type = {hint};
        var second = imageDocument.createElement('source');
        second.srcset = '{BASE}second.gif'; second.type = 'image/gif';
        var image = imageDocument.createElement('img'); image.src = '{BASE}fallback.gif';
        var loads = 0, errors = 0, decodes = [];
        image.onload = () => loads++; image.onerror = () => errors++;
        picture.append(source, second, image); imageDocument.body.append(picture);
    "#)).unwrap();
    if windowless {
        assert!(
            vm.take_pending_subresource_fetch_infos().is_empty(),
            "inactive Documents must not start picture image requests"
        );
        assert_eq!(
            vm.eval("imageDocument.defaultView === null && imageDocument.hidden")
                .unwrap(),
            "true"
        );
        // Adoption makes the image's node document fully active. Creation in a
        // windowless Document must not create a synthetic Window to load it.
        vm.eval("document.body.appendChild(picture)").unwrap();
        assert_eq!(vm.eval("image.ownerDocument === document").unwrap(), "true");
    }
    (vm, loader)
}

#[tokio::test]
async fn picture_source_type_hints_match_real_codec_capabilities() {
    for windowless in [false, true] {
        for (hint, accepted) in [
            (None, true),
            (Some(""), true),
            (Some(" \t\n\x0c\r"), true),
            (Some("image/gif"), true),
            (Some(" IMAGE/PNG; charset=ignored "), true),
            (Some("image/jpeg"), true),
            (Some("image/pjpeg"), true),
            (Some("image/x-png"), true),
            (Some("image/webp"), true),
            (Some("image/bmp"), true),
            (Some("image/svg+xml"), true),
            (Some("image/x-icon"), true),
            (Some("image/vnd.microsoft.icon"), true),
            (Some("image/gif;encodings"), true),
            (Some("text/plain"), false),
            (Some("text/xml"), false),
            (Some("video/mp4"), false),
            (Some("application/octet-stream"), false),
            (Some("image/unknown"), false),
            (Some("image/tiff"), false),
            (Some("image/*"), false),
            (Some("*/*"), false),
            (Some("image/gif, image/png"), false),
            (Some("image/gif image/png"), false),
            (Some("image\\gif"), false),
            (Some("gif"), false),
            (Some("\u{a0}"), false),
            (Some("\x0b"), false),
        ] {
            let (mut vm, loader) = picture_type_vm(windowless, hint);
            let expected = format!("{BASE}{}", if accepted { "hint.gif" } else { "second.gif" });
            let requests = vm.take_pending_subresource_fetch_infos();
            assert_eq!(requests.len(), 1, "{windowless}/{hint:?}");
            assert_eq!(requests[0].url.as_str(), expected, "{windowless}/{hint:?}");
            vm.eval("image.decode().then(() => decodes.push('fulfilled'), error => decodes.push(error.name))").unwrap();
            vm.fulfill_pending_subresource_fetch(
                requests[0].internal_id,
                200,
                vec![("Content-Type".into(), b"image/gif".to_vec())],
                one_by_one_gif_response_body(),
            )
            .unwrap();
            run_next_image_event_task(&mut vm, &loader, "picture source type").await;
            assert_eq!(vm.eval("[loads, errors, image.complete, image.naturalWidth, image.naturalHeight, decodes.join(':')].join('|')").unwrap(), "1|0|true|1|1|fulfilled", "{windowless}/{hint:?}");
            assert_eq!(vm.eval("image.currentSrc").unwrap(), expected);
            if windowless {
                assert_eq!(
                    vm.eval("imageDocument.defaultView === null && imageDocument.hidden")
                        .unwrap(),
                    "true"
                );
            }
            assert!(vm.take_pending_subresource_fetch_infos().is_empty());
        }
    }
}

#[tokio::test]
async fn picture_source_type_changes_reselect_requests_and_preserve_matching_loads() {
    for windowless in [false, true] {
        let (mut vm, loader) = picture_type_vm(windowless, Some("image/gif"));
        let initial = vm.take_pending_subresource_fetch_infos();
        assert_eq!(initial.len(), 1);
        assert_eq!(initial[0].url.as_str(), format!("{BASE}hint.gif"));
        vm.eval("image.decode().then(() => decodes.push('first-fulfilled'), error => decodes.push('first-' + error.name))").unwrap();
        vm.eval("source.type = ' IMAGE/PNG; ignored=parameter '")
            .unwrap();
        assert!(vm.take_pending_subresource_fetch_infos().is_empty());
        assert_eq!(vm.eval("decodes.length").unwrap(), "0");

        vm.eval("source.type = 'text/plain'").unwrap();
        let replacement = vm.take_pending_subresource_fetch_infos();
        assert_eq!(replacement.len(), 1);
        assert_eq!(replacement[0].url.as_str(), format!("{BASE}second.gif"));
        assert_eq!(
            vm.eval("decodes.includes('first-fulfilled')").unwrap(),
            "false"
        );
        vm.eval("image.decode().then(() => decodes.push('second-fulfilled'), error => decodes.push('second-' + error.name))").unwrap();
        vm.eval("source.type = 'image/unknown'").unwrap();
        assert!(vm.take_pending_subresource_fetch_infos().is_empty());
        vm.fulfill_pending_subresource_fetch(
            replacement[0].internal_id,
            200,
            vec![("Content-Type".into(), b"image/gif".to_vec())],
            one_by_one_gif_response_body(),
        )
        .unwrap();
        run_next_image_event_task(&mut vm, &loader, "picture type fallback").await;
        assert_eq!(
            vm.eval("[loads, errors, decodes.join(':')].join('|')")
                .unwrap(),
            "1|0|first-EncodingError:second-fulfilled"
        );

        vm.eval("source.removeAttribute('type')").unwrap();
        let restored = vm.take_pending_subresource_fetch_infos();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].url.as_str(), format!("{BASE}hint.gif"));
        vm.fulfill_pending_subresource_fetch(
            restored[0].internal_id,
            200,
            vec![("Content-Type".into(), b"image/gif".to_vec())],
            one_by_one_gif_response_body(),
        )
        .unwrap();
        run_next_image_event_task(&mut vm, &loader, "removed picture type").await;
        assert_eq!(vm.eval("[loads, errors].join('|')").unwrap(), "2|0");
        assert_eq!(
            vm.eval("image.currentSrc").unwrap(),
            format!("{BASE}hint.gif")
        );
        assert!(vm.take_pending_subresource_fetch_infos().is_empty());
    }
}

#[tokio::test]
async fn picture_source_type_ignores_namespaced_hints_and_falls_back_to_img() {
    let (mut vm, loader) = picture_type_vm(false, None);
    let initial = vm.take_pending_subresource_fetch_infos();
    assert_eq!(initial.len(), 1);
    vm.eval("source.setAttributeNS('urn:unrelated', 'type', 'text/plain')")
        .unwrap();
    assert!(vm.take_pending_subresource_fetch_infos().is_empty());
    vm.fulfill_pending_subresource_fetch(
        initial[0].internal_id,
        200,
        vec![("Content-Type".into(), b"image/gif".to_vec())],
        one_by_one_gif_response_body(),
    )
    .unwrap();
    run_next_image_event_task(&mut vm, &loader, "namespaced source type").await;
    assert_eq!(
        vm.eval("image.currentSrc").unwrap(),
        format!("{BASE}hint.gif")
    );

    vm.eval("source.setAttributeNS(null, 'type', 'text/plain'); second.type = 'video/mp4'")
        .unwrap();
    let fallback = vm.take_pending_subresource_fetch_infos();
    assert_eq!(fallback.len(), 1);
    assert_eq!(fallback[0].url.as_str(), format!("{BASE}fallback.gif"));
    vm.fulfill_pending_subresource_fetch(
        fallback[0].internal_id,
        200,
        vec![("Content-Type".into(), b"image/gif".to_vec())],
        one_by_one_gif_response_body(),
    )
    .unwrap();
    run_next_image_event_task(&mut vm, &loader, "img fallback").await;
    assert_eq!(
        vm.eval("[loads, errors, image.complete].join('|')")
            .unwrap(),
        "2|0|true"
    );
    assert_eq!(
        vm.eval("image.currentSrc").unwrap(),
        format!("{BASE}fallback.gif")
    );
}

#[tokio::test]
async fn picture_source_icon_types_decode_real_resources() {
    let png = moli_image::encode_png_rgba8(2, 1, &[255, 0, 0, 255, 0, 0, 255, 128]).unwrap();
    let mut icon = vec![0, 0, 1, 0, 1, 0, 2, 1, 0, 0, 1, 0, 32, 0];
    icon.extend_from_slice(&u32::try_from(png.bytes.len()).unwrap().to_le_bytes());
    icon.extend_from_slice(&22_u32.to_le_bytes());
    icon.extend_from_slice(&png.bytes);
    for windowless in [false, true] {
        for hint in ["image/x-icon", "image/vnd.microsoft.icon"] {
            let (mut vm, loader) = picture_type_vm(windowless, Some(hint));
            let requests = vm.take_pending_subresource_fetch_infos();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].url.as_str(), format!("{BASE}hint.gif"));
            vm.eval("image.decode().then(() => decodes.push('fulfilled'), error => decodes.push(error.name))").unwrap();
            vm.fulfill_pending_subresource_fetch(
                requests[0].internal_id,
                200,
                vec![("Content-Type".into(), hint.as_bytes().to_vec())],
                crate::runtime::RendererSyntheticResponseBody::from_bytes(icon.clone()),
            )
            .unwrap();
            run_next_image_event_task(&mut vm, &loader, "decoded ICO picture source").await;
            assert_eq!(vm.eval("[loads, errors, image.complete, image.naturalWidth, image.naturalHeight, decodes.join(':')].join('|')").unwrap(), "1|0|true|2|1|fulfilled", "{windowless}/{hint}");
        }
    }
}
