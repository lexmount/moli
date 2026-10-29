use super::*;
use crate::dom::native::{DomHost, NativeDom};

#[test]
fn dragged_image_file_keeps_original_encoding_and_checks_origin_after_resource_reuse() {
    let url = url::Url::parse("https://images.test/path/original%20image.bin").unwrap();
    let mut host = DomHost::from_dom(NativeDom::new(url.clone()));
    let document = host.document_handle();
    let other_document = host.create_document(url::Url::parse("https://foreign.test/").unwrap());
    let bytes = moli_image::encode_png(
        &moli_image::RgbaImage::try_new(1, 1, vec![255, 0, 0, 128]).unwrap(),
    )
    .unwrap()
    .bytes;
    let manager = moli_parkable_image::ParkableImageManager::default();
    let encoded = manager.from_frozen_bytes(bytes.clone());
    let descriptor =
        ImageResponseDescriptor::raster(moli_image::probe_raster_image(&bytes).unwrap());
    let response = crate::protocol_types::NavigationResponse::from_text_body(
        url.clone(),
        200,
        vec![],
        String::new(),
    );
    let source = DragImageSource::from_response(
        encoded.clone(),
        url.origin(),
        &response,
        false,
        Some(document),
    );
    let file = source
        .file(url.origin(), document, &url, descriptor)
        .unwrap();
    assert_eq!(file.name, "original image.bin");
    assert_eq!(file.mime_type, "image/png");
    assert_eq!(file.bytes, bytes);
    let foreign = url::Url::parse("https://foreign.test/image.png").unwrap();
    assert!(
        source
            .file(foreign.origin(), other_document, &url, descriptor)
            .is_none()
    );
    // A cached no-CORS response is not made readable by the original URL
    // being same-origin when its actual response came from another origin.
    let redirected = crate::protocol_types::NavigationResponse::from_text_body(
        foreign.clone(),
        200,
        vec![],
        String::new(),
    );
    let source = DragImageSource::from_response(
        encoded.clone(),
        url.origin(),
        &redirected,
        false,
        Some(document),
    );
    assert!(
        source
            .file(url.origin(), document, &url, descriptor)
            .is_none()
    );
    let cors = DragImageSource::from_response(
        encoded.clone(),
        url.origin(),
        &redirected,
        true,
        Some(document),
    );
    assert_eq!(
        cors.file(url.origin(), document, &url, descriptor)
            .unwrap()
            .bytes,
        bytes
    );
    assert!(
        cors.file(foreign.origin(), other_document, &url, descriptor)
            .is_none()
    );
    let opaque = DragImageSource {
        encoded,
        readable_origin: None,
        readable_document: Some(document),
    };
    assert_eq!(
        opaque
            .file(url::Origin::new_opaque(), document, &url, descriptor)
            .unwrap()
            .bytes,
        bytes
    );
    assert!(
        opaque
            .file(url::Origin::new_opaque(), other_document, &url, descriptor)
            .is_none()
    );
}
