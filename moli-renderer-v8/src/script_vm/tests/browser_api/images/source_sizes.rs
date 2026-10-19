use super::*;

#[tokio::test]
async fn responsive_image_sizes_preserve_decoding_and_canonical_zero_requests() {
    const BASE: &str = "https://image-source-sizes.test/";
    for (sizes, expected, natural_width) in [
        ("-0e-0px", "small.gif", Some(0)),
        ("calc(1px)", "small.gif", Some(1)),
        ("min(1px, calc(100vw + 20px)), 100vw", "small.gif", Some(1)),
        ("not (width: 0px) 1px, 100vw", "small.gif", Some(1)),
        ("1/**/px, 1px", "small.gif", Some(1)),
        ("all 1px, 100vw", "large.gif", None),
        ("10%, 100vw", "large.gif", None),
    ] {
        let loader = ResourceRequestClient::new(&moli_fetch::FetchConfig::default()).unwrap();
        loader.set_image_fetch_enabled(true);
        let mut vm = new_storage_page_task_executor_test_vm_with_loader(BASE, &loader);
        vm.set_layout_policy(moli_page_types::LayoutPolicy::Mock);
        vm.set_fetch_subresource_interception(
            true,
            Some(crate::types::SubresourceResourceType::Image),
        );
        vm.eval(&format!(
            r#"
            var image = document.createElement('img');
            var loads = 0, errors = 0, decodes = [];
            image.onload = () => loads++; image.onerror = () => errors++;
            image.sizes = {};
            image.srcset = 'small.gif 1w, large.gif 2w';
            document.body.append(image);
            image.decode().then(() => decodes.push('fulfilled'), error => decodes.push(error.name));
        "#,
            serde_json::to_string(sizes).unwrap()
        ))
        .unwrap();
        let requests = vm.take_pending_subresource_fetch_infos();
        assert_eq!(requests.len(), 1, "{sizes}");
        let expected = format!("{BASE}{expected}");
        assert_eq!(requests[0].url.as_str(), expected, "{sizes}");
        if sizes == "-0e-0px" {
            vm.eval("image.sizes = '0px'; image.sizes = 'calc(-0px)'")
                .unwrap();
            assert!(vm.take_pending_subresource_fetch_infos().is_empty());
            assert_eq!(vm.eval("decodes.length").unwrap(), "0");
        }
        vm.fulfill_pending_subresource_fetch(
            requests[0].internal_id,
            200,
            vec![("Content-Type".into(), b"image/gif".to_vec())],
            one_by_one_gif_response_body(),
        )
        .unwrap();
        run_next_image_event_task(&mut vm, &loader, "responsive image sizes").await;
        assert_eq!(
            vm.eval("[loads, errors, image.complete, decodes.join(':')].join('|')")
                .unwrap(),
            "1|0|true|fulfilled",
            "{sizes}"
        );
        assert_eq!(vm.eval("image.currentSrc").unwrap(), expected, "{sizes}");
        if let Some(width) = natural_width {
            assert_eq!(
                vm.eval("image.naturalWidth").unwrap(),
                width.to_string(),
                "{sizes}"
            );
        }
        assert!(vm.take_pending_subresource_fetch_infos().is_empty());
    }
}
