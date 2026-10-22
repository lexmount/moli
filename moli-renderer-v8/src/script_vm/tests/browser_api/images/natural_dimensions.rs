use super::*;

#[tokio::test]
async fn responsive_image_natural_dimensions_truncate_fractional_raster_css_pixels() {
    const BASE: &str = "https://image-natural-dimensions.test/";
    for (density, expected) in [(1.5, 0), (2.0, 0), (0.75, 1), (0.4, 2)] {
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
            var image = new Image();
            var loads = 0, errors = 0, decoded = false;
            image.onload = () => loads++; image.onerror = () => errors++;
            image.srcset = 'pixel.gif {density}x';
            image.decode().then(() => decoded = true);
            "#
        ))
        .unwrap();
        let requests = vm.take_pending_subresource_fetch_infos();
        assert_eq!(requests.len(), 1, "{density}");
        assert_eq!(requests[0].url.as_str(), format!("{BASE}pixel.gif"));
        vm.fulfill_pending_subresource_fetch(
            requests[0].internal_id,
            200,
            vec![("Content-Type".into(), b"image/gif".to_vec())],
            one_by_one_gif_response_body(),
        )
        .unwrap();
        run_next_image_event_task(&mut vm, &loader, "fractional natural dimensions").await;
        assert_eq!(
            vm.eval("[loads, errors, image.complete, decoded].join('|')")
                .unwrap(),
            "1|0|true|true",
            "{density}"
        );
        assert_eq!(
            vm.eval("[image.naturalWidth, image.naturalHeight].join('|')")
                .unwrap(),
            format!("{expected}|{expected}"),
            "{density}"
        );
    }
}
