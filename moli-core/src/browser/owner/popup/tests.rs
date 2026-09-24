use super::*;
use crate::browser::{
    BrowserContextHandle, BrowserContextStoragePartitionHandles, BrowserService, DocumentId,
    StoragePartitionKind, WebContentsHandle,
};
use crate::page::{
    RendererDocumentLifecycleIdentity, RendererDocumentToken, RendererFrameToken,
    RendererLifecycleEpoch,
};

fn context(browser: &BrowserHandle) -> BrowserContextHandle {
    browser
        .create_context(
            BrowserContextStoragePartitionHandles::memory(),
            StoragePartitionKind::Ephemeral,
            None,
            None,
        )
        .unwrap()
}

fn source(context: &BrowserContextHandle, page: u64) -> PopupSource {
    let (contents, _) = context
        .create_web_contents(WebContentsCreation::default())
        .unwrap();
    PopupSource {
        document: DocumentHandle::new(contents, DocumentId::from_raw_for_test(page)),
        renderer: RendererPageResidenceIdentity::from_parts(
            crate::RendererOwnerLocalHostId::new_for_testing(1),
            crate::PageId::new_for_testing(page),
        ),
        creator: Some(InitialDocumentCreator::new(
            contents.id(),
            "https://accepted.example".into(),
            "Secure".into(),
        )),
    }
}

fn request(
    source: &PopupSource,
    window: RendererWindowDocumentSource,
    popup_id: u64,
    name: &str,
) -> RendererPendingPopupActivation {
    let page = source.renderer.page_id();
    RendererPendingPopupActivation::window(
        RendererDocumentLifecycleIdentity {
            frame: RendererFrameToken { page_id: page },
            document: RendererDocumentToken::new_for_testing(page, 1),
            epoch: RendererLifecycleEpoch(1),
        },
        window,
        true,
        Some(popup_id),
        "about:blank".into(),
        name.into(),
        RendererPopupDisposition::Background,
    )
}

async fn admit(
    browser: &BrowserHandle,
    source: &PopupSource,
    requests: Vec<RendererPendingPopupActivation>,
) -> Vec<BrowserPopupAdmission> {
    let openings = requests
        .iter()
        .map(RendererPendingPopupActivation::opening)
        .collect::<Vec<_>>();
    let source = source.clone();
    browser
        .execute(move |browser| browser.admit_popups(&source, requests))
        .unwrap();
    let mut admitted = Vec::new();
    for opening in openings {
        admitted.push(browser.wait_for_renderer_popup(opening).await.unwrap());
    }
    admitted
}

#[tokio::test]
async fn accepted_popup_survives_removed_source_without_rebinding_to_selected_peer() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let context = context(&browser);
    let source = source(&context, 1);
    let (peer, _) = context
        .create_web_contents(WebContentsCreation::default())
        .unwrap();
    context
        .activate_web_contents(peer)
        .unwrap()
        .wait()
        .await
        .unwrap();
    let input = request(
        &source,
        RendererWindowDocumentSource::RootFrame,
        1,
        "accepted-tail",
    );
    context
        .close_web_contents(source.document.web_contents())
        .unwrap()
        .close_async()
        .await;
    let admitted = admit(&browser, &source, vec![input]).await;
    let popup = admitted[0].web_contents;
    assert_eq!(context.web_contents_opener(popup).unwrap(), None);
    assert_eq!(
        context
            .selected_web_contents_snapshot()
            .unwrap()
            .web_contents,
        peer
    );
    assert_eq!(
        browser
            .web_contents_snapshot(popup)
            .unwrap()
            .popup
            .unwrap()
            .source_document,
        source.document
    );
    assert_eq!(context.web_contents_count(), 2);
    service.shutdown();
}

#[tokio::test]
async fn accepted_popup_in_disposed_context_never_falls_back_to_a_peer_context() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let original = context(&browser);
    let source = source(&original, 1);
    let peer = context(&browser);
    let input = request(
        &source,
        RendererWindowDocumentSource::RootFrame,
        1,
        "removed-context",
    );
    let opening = input.opening();
    browser.remove_context(original.id()).unwrap();
    browser
        .execute(move |browser| browser.admit_popups(&source, vec![input]))
        .unwrap();
    assert_eq!(browser.wait_for_renderer_popup(opening).await, None);
    assert_eq!(peer.web_contents_count(), 0);
    service.shutdown();
}

#[tokio::test]
async fn popup_aliases_are_renderer_scoped_and_nested_admission_is_fifo() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let context = context(&browser);
    let sources = [source(&context, 1), source(&context, 2)];
    let mut firsts: Vec<WebContentsHandle> = Vec::new();
    for source in &sources {
        let admitted = admit(
            &browser,
            source,
            vec![
                request(source, RendererWindowDocumentSource::RootFrame, 1, "_blank"),
                request(
                    source,
                    RendererWindowDocumentSource::LightweightPopup {
                        popup_id: 1,
                        popup_document_id: 1,
                    },
                    2,
                    "_blank",
                ),
            ],
        )
        .await;
        let first = admitted[0].web_contents;
        let second = admitted[1].web_contents;
        assert_eq!(
            context.web_contents_for_renderer_popup(source.renderer, 1),
            Some(first)
        );
        assert_eq!(
            context.web_contents_opener(second).unwrap(),
            Some((first.id(), true))
        );
        assert_eq!(
            context
                .web_contents_initial_document_state(second)
                .unwrap()
                .unwrap()
                .creator()
                .unwrap()
                .web_contents_id(),
            first.id()
        );
        firsts.push(first);
    }
    assert_ne!(firsts[0], firsts[1]);
    context
        .close_web_contents(firsts[0])
        .unwrap()
        .close_async()
        .await;
    assert_eq!(
        context.web_contents_for_renderer_popup(sources[0].renderer, 1),
        None
    );
    assert_eq!(
        context.web_contents_for_renderer_popup(sources[1].renderer, 1),
        Some(firsts[1])
    );
    service.shutdown();
}

#[tokio::test]
async fn named_popup_reuse_preserves_native_creation_and_drops_dead_receipts() {
    let service = BrowserService::start().unwrap();
    let browser = service.handle();
    let context = context(&browser);
    let first_source = source(&context, 1);
    let second_source = source(&context, 2);
    let first = admit(
        &browser,
        &first_source,
        vec![request(
            &first_source,
            RendererWindowDocumentSource::RootFrame,
            1,
            "Report",
        )],
    )
    .await[0];
    let creation = browser
        .web_contents_snapshot(first.web_contents)
        .unwrap()
        .popup
        .unwrap();
    let second = admit(
        &browser,
        &second_source,
        vec![request(
            &second_source,
            RendererWindowDocumentSource::RootFrame,
            1,
            "Report",
        )],
    )
    .await[0];
    assert!(first.created);
    assert!(!second.created);
    assert_eq!(first.web_contents, second.web_contents);
    assert_eq!(
        browser
            .web_contents_snapshot(second.web_contents)
            .unwrap()
            .popup
            .unwrap()
            .request,
        creation.request
    );
    assert_eq!(
        context.web_contents_opener(second.web_contents).unwrap(),
        Some((first_source.document.web_contents().id(), true))
    );
    assert_eq!(
        browser
            .execute(|browser| browser.popup_admissions.records.len())
            .unwrap(),
        1
    );
    assert_eq!(context.web_contents_count(), 3);
    service.shutdown();
}
